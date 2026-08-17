use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn release_receipt_packages_the_complete_agpl_license() {
    let repo_root = repo_root_from_manifest_dir();
    let packaged =
        fs::read(repo_root.join("LICENSE")).expect("root release LICENSE should be readable");
    let canonical = fs::read(repo_root.join("LICENSES/AGPL-3.0-only.txt"))
        .expect("canonical AGPL-3.0-only text should be readable");
    assert_eq!(
        packaged, canonical,
        "root LICENSE must remain the complete canonical AGPL-3.0-only text"
    );
    assert!(
        packaged.len() > 30_000,
        "release LICENSE must not regress to a short pointer"
    );
    let text = String::from_utf8(packaged).expect("AGPL license should be UTF-8 text");
    assert!(text.contains("GNU AFFERO GENERAL PUBLIC LICENSE"));
    assert!(text.contains("13. Remote Network Interaction"));

    let adapter = fs::read_to_string(repo_root.join("ait-release.json"))
        .expect("release adapter should be readable");
    let adapter: serde_json::Value =
        serde_json::from_str(&adapter).expect("release adapter should parse");
    assert_eq!(
        adapter["package"]["license_files"],
        serde_json::json!([
            {"path": "LICENSE", "role": "license"},
            {"path": "NOTICE", "role": "notice"}
        ]),
        "server receipts must freeze the complete root LICENSE and repository NOTICE"
    );
}

#[test]
fn release_notice_covers_the_complete_locked_rust_dependency_inventory() {
    let repo_root = repo_root_from_manifest_dir();
    let notice_path = repo_root.join("NOTICE");
    let notice = fs::read_to_string(&notice_path)
        .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", notice_path.display()));
    let marker = "----- BEGIN GENERATED THIRD-PARTY NOTICES -----";
    assert_eq!(
        notice.matches(marker).count(),
        1,
        "NOTICE must contain one deterministic generated section"
    );
    assert!(notice.contains("Third-party dependency notices for ait-server"));
    assert!(notice.contains("It contains no build-host paths."));
    assert!(
        !notice.contains("/.cargo/registry/"),
        "NOTICE must not expose a Cargo registry path"
    );
    assert!(
        !notice.contains(repo_root.to_string_lossy().as_ref()),
        "NOTICE must not expose the repository build path"
    );

    let lock_path = repo_root.join("rust/Cargo.lock");
    let lock = fs::read_to_string(&lock_path)
        .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", lock_path.display()));
    let mut external_packages = 0usize;
    for package in lock.split("[[package]]").skip(1) {
        if cargo_lock_string(package, "source").is_none() {
            continue;
        }
        let name = cargo_lock_string(package, "name").expect("locked package must have a name");
        let version =
            cargo_lock_string(package, "version").expect("locked package must have a version");
        let inventory_row = format!("{name}\t{version}\t");
        assert!(
            notice.contains(&inventory_row),
            "NOTICE is missing locked external package {name} {version}"
        );
        external_packages += 1;
    }
    assert!(
        external_packages > 100,
        "server NOTICE inventory unexpectedly covered only {external_packages} external packages"
    );

    let generator_path = repo_root.join("ci/generate_rust_notice.sh");
    let generator = fs::read_to_string(&generator_path)
        .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", generator_path.display()));
    assert!(generator.contains("cargo metadata --manifest-path \"$manifest\" --locked"));
    assert!(generator.contains("select(.source != null)"));
    assert!(generator.contains("Complete deduplicated upstream legal texts"));
    assert!(generator.contains("cmp -s \"$generated\" \"$notice\""));
}

#[test]
fn ait_server_build_has_no_ait_core_external_dependency() {
    let repo_root = repo_root_from_manifest_dir();

    for relative in [
        "rust/Cargo.toml",
        "rust/Cargo.lock",
        "rust/crates/ait-server/Cargo.toml",
    ] {
        let path = repo_root.join(relative);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", path.display()));
        assert!(
            !text.contains(".ait-external/ait-core"),
            "{} must not resolve ait-core from an external source tree",
            path.display()
        );
        assert!(
            !text.contains("name = \"ait-core\""),
            "{} must not contain the ait-core package",
            path.display()
        );
        assert!(
            !text.contains("ait-core ="),
            "{} must not declare an ait-core dependency",
            path.display()
        );
    }

    assert!(
        !repo_root.join("ait-external.toml").exists(),
        "ait-server must not carry an ait-core external manifest"
    );
    assert!(
        !repo_root.join("ait-external.lock").exists(),
        "ait-server must not carry an ait-core external lock"
    );
    assert!(
        repo_root
            .join("rust/crates/ait-server-core/src/foundation/server_binary_lifecycle.rs",)
            .is_file(),
        "server Binary DB activation lifecycle must be owned by ait-server"
    );

    let server_source_root = repo_root.join("rust/crates/ait-server/src");
    let mut pending = vec![server_source_root];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", directory.display()))
        {
            let path = entry
                .expect("server source entry should be readable")
                .path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", path.display()));
            assert!(
                !source.contains("ait_core::"),
                "{} must not import the ait-core crate",
                path.display()
            );
        }
    }
}

#[test]
fn workspace_release_and_ci_profiles_forbid_debug_and_incremental_state() {
    let manifest = workspace_manifest_candidates()
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .find(|manifest| manifest.contains("[profile.release]"))
        .expect("workspace Cargo.toml with [profile.release] should be readable");

    assert!(manifest.contains("[profile.release]"));
    assert!(manifest.contains("opt-level = 3"));
    assert!(manifest.contains("debug = 0"));
    assert!(manifest.contains("incremental = false"));
    assert!(manifest.contains("[profile.ait-ci]"));
    assert!(manifest.contains("inherits = \"test\""));
    assert!(manifest.contains("opt-level = 0"));
    assert!(manifest.contains("debug-assertions = true"));
    assert!(manifest.contains("overflow-checks = true"));
    assert!(!manifest.contains("incremental = true"));
}

#[test]
fn ci_prewarm_uses_managed_cargo_alias() {
    let script = prewarm_script_candidates()
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .find(|script| script.contains("cargo patch-ci-build"))
        .expect("ci/prewarm.sh should be readable");

    assert!(script.contains("cd \"$ROOT_DIR/rust\""));
    assert!(script.contains("cargo patch-ci-build"));
}

#[test]
fn patchset_ci_and_prewarm_share_lean_ci_build_contract_without_alias_expansion() {
    let repo_root = repo_root_from_manifest_dir();
    let cargo_config = canonical_cargo_source_policy(&repo_root);
    let patch_ci = fs::read_to_string(repo_root.join("ci/patch_ci.json"))
        .expect("patchset CI catalog should be readable");

    assert!(cargo_config.contains("patch-ci-build = ["));
    assert!(cargo_config.contains("target-dir = \".ait/cargo-target\""));
    assert!(
        cargo_config.contains("build-dir = \".ait/cargo-build/workspaces/{workspace-path-hash}\"")
    );
    assert!(cargo_config.contains("\"--profile\""));
    assert!(cargo_config.contains("\"ait-ci\""));
    assert!(!cargo_config.contains("\"--release\""));
    assert!(cargo_config.contains("\"--lib\""));
    assert!(cargo_config.contains("\"seam_contract_direct_tests\""));
    assert!(cargo_config.contains("\"ait-server-core/patch-ci-harness\""));
    let patch_ci: serde_json::Value =
        serde_json::from_str(&patch_ci).expect("patchset CI catalog should parse");
    let build_args = patch_ci["suites"][0]["runner"]["build_args"]
        .as_array()
        .expect("patchset CI build args should be an array");
    assert_eq!(
        patch_ci["suites"][0]["runner"]["doc_tests"], false,
        "documentation tests belong to full workspace validation, not the patchset gate"
    );
    assert_eq!(build_args[0], "test");
    assert!(build_args
        .windows(2)
        .any(|args| args[0] == "--profile" && args[1] == "ait-ci"));
    assert!(!build_args.iter().any(|arg| arg == "--release"));
    assert!(build_args.iter().any(|arg| arg == "--lib"));
    assert!(build_args
        .iter()
        .any(|arg| arg == "seam_contract_direct_tests"));
    assert!(build_args
        .iter()
        .any(|arg| arg == "ait-server-core/patch-ci-harness"));
    assert!(!build_args.iter().any(|arg| arg == "patch-ci-build"));
    assert!(!build_args.iter().any(|arg| arg == "--bins"));
}

#[test]
fn ci_test_discovery_defaults_use_lean_ci_profile() {
    let source = discovery_runner_source_candidates()
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .find(|text| text.contains("default_cargo_build_args"))
        .expect("ci_test_discovery_sharded/cargo.rs should be readable");

    assert!(source.contains("\"--profile\".to_string()"));
    assert!(source.matches("\"ait-ci\".to_string()").count() >= 2);
    assert!(!source.contains("\"--release\".to_string()"));
    assert!(source.contains("\"--doc\".to_string()"));
}

#[test]
fn patchset_ci_harness_covers_every_top_level_integration_test() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let harness_path = manifest_dir.join("tests/patchset_integration_harness.rs");
    let harness = fs::read_to_string(&harness_path)
        .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", harness_path.display()));
    let mut missing = fs::read_dir(manifest_dir.join("tests"))
        .expect("integration test directory should be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("rs"))
        .filter(|path| {
            !matches!(
                path.file_name().and_then(|value| value.to_str()),
                Some("patchset_integration_harness.rs" | "seam_contract_direct_tests.rs")
            )
        })
        .filter_map(|path| {
            let file_name = path.file_name()?.to_str()?;
            let module_name = path.file_stem()?.to_str()?;
            let path_marker = format!("#[path = \"{file_name}\"]");
            let module_marker = format!("mod {module_name};");
            (!harness.contains(&path_marker) || !harness.contains(&module_marker))
                .then(|| file_name.to_string())
        })
        .collect::<Vec<_>>();
    missing.sort();

    assert!(
        missing.is_empty(),
        "patchset integration harness is missing top-level tests: {}",
        missing.join(", ")
    );
    assert!(
        manifest_dir
            .join("tests/seam_contract_direct_tests.rs")
            .is_file(),
        "seam contract must remain an explicit patchset CI target"
    );
}

#[test]
fn ait_server_launcher_uses_release_binary_only() {
    let script = ait_script_candidates()
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .find(|script| script.contains("server_binary_path"))
        .expect("ait.sh should be readable");

    assert!(script.contains("release/ait-server"));
    assert!(script.contains("-p ait-server --release"));
    assert!(script.contains("--workspace --release"));
    assert!(script.contains(
        "cargo test --manifest-path \"${ROOT_DIR}/rust/Cargo.toml\" --workspace --profile ait-ci"
    ));
    assert!(script.contains("export CARGO_INCREMENTAL=0"));
    assert!(script.contains("reject_debug_server_binary"));
    assert!(script.contains("*/[Dd][Ee][Bb][Uu][Gg]/*"));
    assert!(!script.contains("debug/ait-server"));
    assert!(!script.contains("AIT_NATIVE_SERVER_DB_BACKEND"));
    assert!(!script.contains("AIT_NATIVE_SERVER_POSTGRES_DSN"));
    assert!(script.contains("Unknown server start argument"));
    assert!(script.contains(r#"exec "$1" run --data "$2" --listen "$3""#));
    assert!(!script.contains("shift 3"));
}

#[test]
fn ait_server_launcher_isolates_managed_task_cargo_directories() {
    let source = ait_script_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("ait.sh should be readable");
    let fixture = tempfile::tempdir().expect("launcher fixture");
    let script = fixture.path().join("ait.sh");
    fs::copy(&source, &script).expect("copy launcher");
    fs::create_dir_all(
        fixture
            .path()
            .join(".ait/cargo-target/task-workspaces/lset-fixture"),
    )
    .expect("Task Cargo target");
    fs::create_dir_all(
        fixture
            .path()
            .join(".ait/cargo-build/task-workspaces/lset-fixture"),
    )
    .expect("Task Cargo build");
    fs::write(
        fixture.path().join(".ait-worktree.json"),
        r#"{"worktree_name":"lset-fixture"}"#,
    )
    .expect("Task worktree marker");

    let selected_paths = || {
        let output = Command::new("bash")
            .args([
                "-c",
                "source \"$1\"; printf '%s\\n%s\\n' \"$(cargo_target_dir)\" \"$(cargo_build_dir)\"",
                "_",
            ])
            .arg(&script)
            .env_remove("AIT_SHARED_CARGO_TARGET_DIR")
            .env_remove("AIT_SHARED_CARGO_BUILD_DIR")
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_BUILD_BUILD_DIR")
            .output()
            .expect("source launcher");
        assert!(
            output.status.success(),
            "launcher selection failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("launcher paths should be UTF-8")
            .lines()
            .map(str::to_string)
            .collect::<Vec<_>>()
    };

    let task_paths = selected_paths();
    assert_eq!(task_paths.len(), 2);
    assert!(task_paths[0].ends_with("cargo-target/task-workspaces/lset-fixture"));
    assert!(task_paths[1].ends_with("cargo-build/task-workspaces/lset-fixture"));
    assert_ne!(task_paths[0], task_paths[1]);

    fs::remove_file(fixture.path().join(".ait-worktree.json")).expect("remove Task marker");
    let canonical_paths = selected_paths();
    assert_eq!(canonical_paths.len(), 2);
    assert!(canonical_paths[0].ends_with(".ait/cargo-target"));
    assert!(canonical_paths[1].ends_with(".ait/cargo-build/canonical"));
    assert_ne!(task_paths, canonical_paths);
}

#[test]
fn tg1_native_runner_uses_release_ait_cli_fallback() {
    let source = patchset_runtime_tg1_source_candidates()
        .into_iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .find(|text| text.contains("native_ait_cli_binary_name"))
        .expect("patchset_ci_runtime/tg1.rs should be readable");

    assert!(source.contains(".join(\"release\").join(native_ait_cli_binary_name())"));
    assert!(!source.contains(".join(\"debug\").join(native_ait_cli_binary_name())"));
}

#[test]
fn generic_ci_tool_processes_use_shared_bounded_streaming() {
    let repo_root = repo_root_from_manifest_dir();
    for relative in [
        "rust/crates/ait-server-core/src/foundation/main_seed_prewarm/steps.rs",
        "rust/crates/ait-server-core/src/foundation/repo_ci_runtime/full_test.rs",
        "rust/crates/ait-server-core/src/foundation/test_shard_runner.rs",
    ] {
        let path = repo_root.join(relative);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", path.display()));
        assert!(
            source.contains("run_streamed_command"),
            "{} must use the shared bounded CI process runner",
            path.display()
        );
        assert!(
            !source.contains(".output()"),
            "{} must not retain complete child stdout/stderr",
            path.display()
        );
    }
}

#[test]
fn generic_ci_tool_processes_use_the_shared_clean_environment_policy() {
    let repo_root = repo_root_from_manifest_dir();
    for relative in [
        "rust/crates/ait-server-core/src/foundation/ci_command_bundle.rs",
        "rust/crates/ait-server-core/src/foundation/ci_test_discovery_sharded/process.rs",
        "rust/crates/ait-server-core/src/foundation/main_seed_prewarm/steps.rs",
        "rust/crates/ait-server-core/src/foundation/repo_ci_runtime/full_test.rs",
        "rust/crates/ait-server-core/src/foundation/test_shard_runner.rs",
    ] {
        let path = repo_root.join(relative);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", path.display()));
        assert!(
            source.contains("apply_clean_ci_process_env"),
            "{} must clear and rebuild each generic child environment",
            path.display()
        );
        assert!(
            !source.contains("std::env::vars().collect"),
            "{} must not collect the complete ambient server environment",
            path.display()
        );
    }

    let collection_source = fs::read_to_string(
        repo_root.join("rust/crates/ait-server-core/src/foundation/repo_ci_runtime/full_test.rs"),
    )
    .expect("full-test collection source should read");
    assert!(
        !collection_source.contains(".or_else(|| env::var(variable).ok())"),
        "collection executable expansion must not fall back to arbitrary ambient variables"
    );
}

#[test]
fn command_discovery_adapter_uses_shared_bounded_process_contracts() {
    let repo_root = repo_root_from_manifest_dir();
    let path = repo_root
        .join("rust/crates/ait-server-core/src/foundation/ci_test_discovery_sharded/command.rs");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", path.display()));

    assert!(source.contains("run_process_with_output"));
    assert!(source.contains("run_process("));
    assert!(source.contains("command_env("));
    assert!(source.contains("MAX_COMMAND_DISCOVERED_TEST_CASES"));
    assert!(source.contains("stable_round_robin_by_test_case"));
    assert!(!source.contains(".output()"));
    assert!(!source.contains("python3"));
    assert!(!source.contains("\".py\""));
    assert!(!source.contains("sqlite3"));
}

fn workspace_manifest_candidates() -> Vec<PathBuf> {
    let repo_root = repo_root_from_manifest_dir();
    vec![
        repo_root.join("rust/Cargo.toml"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml"),
        PathBuf::from("../../Cargo.toml"),
        PathBuf::from("Cargo.toml"),
    ]
}

fn ait_script_candidates() -> Vec<PathBuf> {
    let repo_root = repo_root_from_manifest_dir();
    vec![
        repo_root.join("ait.sh"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ait.sh"),
        PathBuf::from("ait.sh"),
    ]
}

fn prewarm_script_candidates() -> Vec<PathBuf> {
    let repo_root = repo_root_from_manifest_dir();
    vec![
        repo_root.join("ci/prewarm.sh"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ci/prewarm.sh"),
        PathBuf::from("ci/prewarm.sh"),
    ]
}

fn patchset_runtime_tg1_source_candidates() -> Vec<PathBuf> {
    let repo_root = repo_root_from_manifest_dir();
    vec![
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/foundation/patchset_ci_runtime/tg1.rs"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/foundation/patchset_ci_runtime.rs"),
        repo_root.join("rust/crates/ait-server-core/src/foundation/patchset_ci_runtime/tg1.rs"),
        repo_root.join("rust/crates/ait-server-core/src/foundation/patchset_ci_runtime.rs"),
        PathBuf::from("rust/crates/ait-server-core/src/foundation/patchset_ci_runtime/tg1.rs"),
        PathBuf::from("rust/crates/ait-server-core/src/foundation/patchset_ci_runtime.rs"),
    ]
}

fn discovery_runner_source_candidates() -> Vec<PathBuf> {
    let repo_root = repo_root_from_manifest_dir();
    vec![
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/foundation/ci_test_discovery_sharded/cargo.rs"),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/foundation/ci_test_discovery_sharded.rs"),
        repo_root
            .join("rust/crates/ait-server-core/src/foundation/ci_test_discovery_sharded/cargo.rs"),
        repo_root.join("rust/crates/ait-server-core/src/foundation/ci_test_discovery_sharded.rs"),
        PathBuf::from(
            "rust/crates/ait-server-core/src/foundation/ci_test_discovery_sharded/cargo.rs",
        ),
        PathBuf::from("rust/crates/ait-server-core/src/foundation/ci_test_discovery_sharded.rs"),
    ]
}

fn repo_root_from_manifest_dir() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .ancestors()
        .find(|path| path.join("rust/Cargo.toml").is_file() && path.join("ci").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest_dir.join("../../.."))
}

fn canonical_cargo_source_policy(repo_root: &Path) -> String {
    let config_path = repo_root.join(".cargo/config.toml");
    let projected = fs::read_to_string(&config_path)
        .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", config_path.display()));
    if !projected.contains("Managed by ait:") {
        return projected;
    }

    let worktree_path = repo_root.join(".ait-worktree.json");
    let worktree: serde_json::Value = serde_json::from_slice(
        &fs::read(&worktree_path)
            .unwrap_or_else(|exc| panic!("failed to read {}: {exc}", worktree_path.display())),
    )
    .unwrap_or_else(|exc| panic!("failed to parse {}: {exc}", worktree_path.display()));
    let source_root = worktree
        .get("repo_root")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .expect("AIT task worktree metadata must name its canonical repo_root");
    let source_config = source_root.join(".cargo/config.toml");
    fs::read_to_string(&source_config).unwrap_or_else(|exc| {
        panic!(
            "failed to read canonical Cargo source policy {}: {exc}",
            source_config.display()
        )
    })
}

fn cargo_lock_string<'a>(package: &'a str, field: &str) -> Option<&'a str> {
    let prefix = format!("{field} = \"");
    package.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix('"'))
    })
}
