use super::*;
use crate::workspace_test_support;
use ait_core::json_support::json;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn core_release_profiles_reject_the_retired_mixed_license_surface() {
    let profile = require_profile("local-cli").expect("Apache-only local profile remains valid");
    assert_eq!(profile.id, "local-cli");
    assert_eq!(profile.license, "Apache-2.0");
    assert!(!profile
        .license_files
        .iter()
        .any(|path| path.contains("AGPL") || path.contains("Commercial")));

    let error = require_profile("public-self-hosted-core")
        .expect_err("combined public releases must use the family path");
    assert!(
        error.contains("Combined public packages use the separately licensed family release path")
    );
}

#[test]
fn native_release_artifact_smoke_has_no_subprocess_escape_hatch() {
    let source = include_str!("build_orchestration.rs");
    let start = source
        .find("pub fn release_artifact_smoke")
        .expect("native release artifact smoke exists");
    let end = source[start..]
        .find("pub(super) fn release_source_bundle")
        .map(|offset| start + offset)
        .expect("release source bundling follows native smoke");
    let smoke = &source[start..end];

    assert!(smoke.contains("release_candidate_create"));
    assert!(smoke.contains("release_check_with_compileall_policy"));
    assert!(smoke.contains("release_build"));
    assert!(smoke.contains("NATIVE_RELEASE_SMOKE_COMPILEALL_SKIP_REASON"));
    assert!(!smoke.contains("Command::new"));
    assert!(!smoke.contains("python3"));
    assert!(!smoke.contains("sh\""));
}

#[test]
fn native_bundle_only_orchestration_does_not_build_foreign_packages_or_direct_worker_artifacts() {
    let source = include_str!("build_orchestration.rs");
    let start = source
        .find("pub fn release_native_bundle")
        .expect("native bundle-only orchestration exists");
    let end = source[start..]
        .find("pub fn release_artifact_smoke")
        .map(|offset| start + offset)
        .expect("native artifact smoke follows native bundle orchestration");
    let orchestration = &source[start..end];

    assert!(orchestration.contains("build_native_distribution"));
    assert!(orchestration.contains("release_source_bundle"));
    assert!(orchestration.contains("apply_release_notes"));
    assert!(orchestration.contains("update_release"));
    assert!(orchestration.contains("ait_rust_native_bundle_only"));
    assert!(orchestration.contains("python_distribution_built\": false"));
    assert!(orchestration.contains("public_publish\": false"));
    assert!(!orchestration.contains("build_sdist"));
    assert!(!orchestration.contains("build_wheel"));
    assert!(!orchestration.contains("resolve_native_worker_command_source_dir"));
    assert!(!orchestration.contains("copy_native_worker_command_artifacts_from_dir"));
    assert!(!orchestration.contains("Command::new"));
}

#[test]
fn native_bundle_only_artifact_projection_replaces_native_rows_and_preserves_others() {
    let record = json!({
        "artifacts": [
            {"kind": "wheel", "path": "dist/ait-1.2.3.whl", "sha256": "wheel"},
            {"kind": "native-bundle", "target": "stale-target", "path": "dist/stale.tar.gz"},
            {"kind": "provenance", "path": "dist/provenance.json", "sha256": "provenance"}
        ]
    });
    let projected = replace_native_bundle_artifacts(
        &record,
        vec![
            json!({
                "kind": "native-bundle",
                "target": "x86_64-unknown-linux-gnu",
                "path": "dist/ait-native-1.2.3-x86_64-unknown-linux-gnu.tar.gz"
            }),
            json!({
                "kind": "native-bundle",
                "target": "aarch64-apple-darwin",
                "path": "dist/ait-native-1.2.3-aarch64-apple-darwin.tar.gz"
            }),
        ],
    );

    assert_eq!(
        projected
            .iter()
            .filter(|artifact| string_field(artifact, "kind").as_deref() == Some("native-bundle"))
            .count(),
        2
    );
    assert!(!projected
        .iter()
        .any(|artifact| { string_field(artifact, "target").as_deref() == Some("stale-target") }));
    assert!(projected.iter().any(|artifact| {
        string_field(artifact, "kind").as_deref() == Some("wheel")
            && string_field(artifact, "path").as_deref() == Some("dist/ait-1.2.3.whl")
    }));
    assert!(projected
        .iter()
        .any(|artifact| { string_field(artifact, "kind").as_deref() == Some("provenance") }));
    let targets = projected
        .iter()
        .filter(|artifact| string_field(artifact, "kind").as_deref() == Some("native-bundle"))
        .filter_map(|artifact| string_field(artifact, "target"))
        .collect::<Vec<_>>();
    assert_eq!(
        targets,
        vec![
            "aarch64-apple-darwin".to_string(),
            "x86_64-unknown-linux-gnu".to_string()
        ]
    );
}

#[test]
fn supplemental_release_files_fall_back_to_authoritative_repo_root() {
    let temp = tempfile::TempDir::new().unwrap();
    let workspace = temp.path().join("worktree");
    let authoritative = temp.path().join("repo");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(authoritative.join("release/guides")).unwrap();
    fs::write(
        authoritative.join("release/guides/LOCAL_QUICKSTART.md"),
        "# Quickstart\n",
    )
    .unwrap();
    let repo = RepoRuntime {
        root: workspace.clone(),
        ait_dir: workspace.join(".ait"),
        config: JsonMap::from_iter([
            ("repo_name".to_string(), json!("ait")),
            (
                "repo_root".to_string(),
                json!(authoritative.to_string_lossy().to_string()),
            ),
        ]),
        worktree_config_path: Some(workspace.join(".ait-worktree.json")),
    };

    let resolved = supplemental_source_file_path(&repo, "release/guides/LOCAL_QUICKSTART.md")
        .expect("authoritative release guide fallback");

    assert_eq!(
        resolved,
        authoritative.join("release/guides/LOCAL_QUICKSTART.md")
    );
}

#[test]
fn materialized_release_bundle_uses_raii_system_tempdir() {
    let bundle = ReleaseBundle {
        raw: json!({"created_at": "1785761092"}),
        files: BTreeMap::from([(
            "pyproject.toml".to_string(),
            BundleEntry {
                path: "pyproject.toml".to_string(),
                data: b"[project]\nname = \"fixture\"\nversion = \"1.0.0\"\n".to_vec(),
                mode: "0644".to_string(),
            },
        )]),
    };
    let workspace = tempfile::TempDir::new().unwrap();
    let materialized = materialize_bundle_to_temp(&bundle, "ait-release-raii-test-").unwrap();
    let source_dir = materialized.source_dir().to_path_buf();

    assert!(source_dir.join("pyproject.toml").is_file());
    assert!(!source_dir.starts_with(workspace.path()));

    drop(materialized);
    assert!(!source_dir.exists());
}

#[test]
fn release_epoch_accepts_current_and_legacy_snapshot_time_encodings() {
    let unix_seconds = 1_785_761_092_i64;

    assert_eq!(
        release_epoch(&json!({"created_at": "1785761092"})).unwrap(),
        unix_seconds
    );
    assert_eq!(
        release_epoch(&json!({"created_at": 1785761092_u64})).unwrap(),
        unix_seconds
    );
    assert_eq!(
        release_epoch(&json!({"created_at": "2026-08-03T12:44:52Z"})).unwrap(),
        unix_seconds
    );
}

#[test]
fn release_epoch_rejects_missing_or_malformed_snapshot_time() {
    let invalid = [
        json!({}),
        json!({"created_at": -1}),
        json!({"created_at": "-1"}),
        json!({"created_at": 1.5}),
        json!({"created_at": u64::MAX}),
        json!({"created_at": "18446744073709551616"}),
        json!({"created_at": "not-a-time"}),
        json!({"created_at": null}),
    ];

    for bundle in invalid {
        let first = release_epoch(&bundle).unwrap_err();
        let repeated = release_epoch(&bundle).unwrap_err();
        assert_eq!(first, repeated);
        assert!(first.contains("created_at"));
    }
}

#[test]
fn rust_build_profile_contracts_enforce_release_and_lean_ci_without_debug() {
    let release_contract = rust_release_profile_contract();
    assert_eq!(release_contract["cargo_profile"].as_str(), Some("release"));
    assert_eq!(release_contract["opt_level"].as_u64(), Some(3));
    assert_eq!(release_contract["debug"].as_u64(), Some(0));
    assert_eq!(release_contract["debug_assertions"].as_bool(), Some(false));
    assert_eq!(release_contract["overflow_checks"].as_bool(), Some(false));
    assert_eq!(release_contract["incremental"].as_bool(), Some(false));
    assert_eq!(
        release_contract["rustc_opt_level_flag"].as_str(),
        Some("-C opt-level=3")
    );
    assert!(release_contract["diagnostic_role"]
        .as_str()
        .unwrap_or_default()
        .contains("not intended for single-step debugging"));

    let ci_contract = rust_ci_profile_contract();
    assert_eq!(ci_contract["cargo_profile"].as_str(), Some("ait-ci"));
    assert_eq!(ci_contract["opt_level"].as_u64(), Some(0));
    assert_eq!(ci_contract["debug"].as_u64(), Some(0));
    assert_eq!(ci_contract["debug_assertions"].as_bool(), Some(true));
    assert_eq!(ci_contract["overflow_checks"].as_bool(), Some(true));
    assert_eq!(ci_contract["incremental"].as_bool(), Some(false));
    assert!(ci_contract["diagnostic_role"]
        .as_str()
        .unwrap_or_default()
        .contains("debug_assert!"));

    let runtime_dir = std::env::current_dir().expect("current test working directory");
    let manifest_dir = workspace_test_support::crate_root("ait-cli");
    let (manifest_path, manifest) =
        workspace_release_manifest_candidates(&runtime_dir, &manifest_dir)
            .into_iter()
            .find_map(|candidate| {
                let manifest = fs::read_to_string(&candidate).ok()?;
                if manifest.contains("[workspace]") && manifest.contains("[profile.release]") {
                    Some((candidate, manifest))
                } else {
                    None
                }
            })
            .expect("workspace Cargo.toml with release profile");
    let repo_root = manifest_path
        .parent()
        .and_then(Path::parent)
        .expect("workspace manifest beneath repository rust directory");
    let source_repo_root = RepoRuntime::discover_from_path(repo_root)
        .map(|repo| repo.authoritative_repo_root())
        .unwrap_or_else(|_| repo_root.to_path_buf());
    assert!(manifest.contains("[profile.release]"));
    assert!(manifest.contains("[profile.ait-ci]"));
    assert!(manifest.contains("opt-level = 3"));
    assert!(manifest.contains("debug = 0"));
    assert!(manifest.contains("debug-assertions = false"));
    assert!(manifest.contains("overflow-checks = false"));
    assert_eq!(manifest.matches("incremental = false").count(), 2);
    assert!(!manifest.contains("incremental = true"));
    assert!(!manifest.contains("[profile.dev]"));

    let wrapper = fs::read_to_string(source_repo_root.join("ait.sh")).expect("AIT core wrapper");
    assert!(wrapper.contains("debug/dev profile is forbidden"));
    assert!(wrapper.contains(
        "run_cargo test --manifest-path \"${ROOT_DIR}/rust/Cargo.toml\" --workspace --profile ait-ci"
    ));
    assert!(!wrapper.contains(
        "run_cargo test --manifest-path \"${ROOT_DIR}/rust/Cargo.toml\" --workspace --release"
    ));

    let cargo_config = fs::read_to_string(source_repo_root.join(".cargo/config.toml"))
        .expect("source Cargo config");
    assert!(cargo_config.contains("\"--profile\",\n  \"ait-ci\","));
    assert!(!cargo_config.contains("\"--release\","));
}

fn workspace_release_manifest_candidates(runtime_dir: &Path, manifest_dir: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for start in [runtime_dir, manifest_dir] {
        for ancestor in start.ancestors() {
            candidates.push(ancestor.join("Cargo.toml"));
            candidates.push(ancestor.join("rust").join("Cargo.toml"));
        }
    }
    candidates
}

#[test]
fn release_external_readiness_check_skips_repos_without_manifest() {
    let temp = tempfile::TempDir::new().unwrap();
    let repo = release_test_repo(temp.path());

    let check = release_external_readiness_check(&repo).unwrap();

    assert!(check.is_none());
}

#[test]
fn release_external_readiness_check_blocks_missing_lock_and_materialization() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::write(
        temp.path().join("ait-external.toml"),
        r#"
[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "SNP-DB-DIRECT"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
version = "0.1.0"

[external.bindings.rust]
kind = "cargo-path"
path = "rust/crates/ait-db"
package = "ait-db"
"#,
    )
    .unwrap();
    let repo = release_test_repo(temp.path());

    let check = release_external_readiness_check(&repo).unwrap().unwrap();
    let blockers = check["external_readiness"]["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|blocker| blocker["code"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(check["check_id"], json!("external_readiness"));
    assert_eq!(check["status"], json!("fail"));
    assert_eq!(check["blocking"], json!(true));
    assert!(check["details"]
        .as_str()
        .unwrap()
        .contains("external_lock_missing ait-db ait-external.lock"));
    assert!(blockers.contains(&"external_lock_missing"));
    assert!(blockers.contains(&"external_materialization_missing"));
}

#[test]
fn release_external_readiness_gate_skips_repos_without_manifest() {
    let temp = tempfile::TempDir::new().unwrap();
    let repo = release_test_repo(temp.path());

    release_require_external_readiness(&repo).unwrap();
}

#[test]
fn release_external_readiness_gate_accepts_valid_external_materialization() {
    let temp = tempfile::TempDir::new().unwrap();
    write_external_manifest(temp.path(), "SNP-DB-DIRECT");
    write_external_lock(temp.path(), "SNP-DB-DIRECT");
    write_external_marker(temp.path(), "SNP-DB-DIRECT");
    let repo = release_test_repo(temp.path());

    release_require_external_readiness(&repo).unwrap();
}

#[test]
fn release_external_readiness_gate_blocks_missing_lock_before_candidate_create() {
    let temp = tempfile::TempDir::new().unwrap();
    write_external_manifest(temp.path(), "SNP-DB-DIRECT");
    let repo = release_test_repo(temp.path());

    let error = release_require_external_readiness(&repo).unwrap_err();

    assert!(error.contains("Cannot create release candidate"));
    assert!(error.contains("ait external update --locked"));
    assert!(error.contains("external_lock_missing"));
    assert!(error.contains("external_materialization_missing"));
}

#[test]
fn release_external_readiness_gate_blocks_active_local_links() {
    let temp = tempfile::TempDir::new().unwrap();
    write_external_manifest(temp.path(), "SNP-DB-DIRECT");
    write_external_lock(temp.path(), "SNP-DB-DIRECT");
    write_external_marker(temp.path(), "SNP-DB-DIRECT");
    fs::write(
        temp.path().join("ait-external.links.toml"),
        r#"
[[link]]
name = "ait-db"
path = "../ait-db"
"#,
    )
    .unwrap();
    let repo = release_test_repo(temp.path());

    let error = release_require_external_readiness(&repo).unwrap_err();

    assert!(error.contains("external_local_link_active"));
    assert!(error.contains("ait-db"));
}

#[test]
fn release_external_readiness_gate_blocks_dirty_materialization() {
    let temp = tempfile::TempDir::new().unwrap();
    write_external_manifest(temp.path(), "SNP-DB-DIRECT");
    write_external_lock(temp.path(), "SNP-DB-DIRECT");
    fs::create_dir_all(temp.path().join(".ait-external").join("ait-db")).unwrap();
    let repo = release_test_repo(temp.path());

    let error = release_require_external_readiness(&repo).unwrap_err();

    assert!(error.contains("external_materialization_dirty"));
    assert!(error.contains("external materialization is dirty or not generated by AIT"));
}

#[test]
fn release_external_readiness_gate_blocks_lock_drift() {
    let temp = tempfile::TempDir::new().unwrap();
    write_external_manifest(temp.path(), "SNP-DB-MANIFEST");
    write_external_lock(temp.path(), "SNP-DB-LOCK");
    write_external_marker(temp.path(), "SNP-DB-LOCK");
    let repo = release_test_repo(temp.path());

    let error = release_require_external_readiness(&repo).unwrap_err();

    assert!(error.contains("external_lock_drift"));
    assert!(error.contains("field snapshot"));
}

#[test]
fn release_external_closure_from_bundle_ignores_missing_lockfile() {
    let bundle = ReleaseBundle {
        raw: json!({}),
        files: std::collections::BTreeMap::new(),
    };

    assert!(release_external_closure_from_bundle(&bundle)
        .unwrap()
        .is_none());
}

#[test]
fn release_external_closure_from_bundle_records_lockfile_dag() {
    let lockfile = r#"
format = "ait.external.lock"

[[node]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 11
remote = "origin"
line = "main"
snapshot = "SNP-DB-RECURSIVE"
parent_path = ""
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
version = "0.1.0"

[[node]]
name = "ait-codec"
repo_name = "ait-codec"
repository_index = 12
remote = "origin"
line = "main"
snapshot = "SNP-CODEC-RECURSIVE"
parent_path = ".ait-external/ait-db"
materialize_to = ".ait-external/ait-db/.ait-external/ait-codec"
license = "Apache-2.0"

[[node.binding]]
language = "rust"
kind = "cargo-path"
path = "rust/crates/ait-codec"
package = "ait-codec"
"#;
    let bundle = ReleaseBundle {
        raw: json!({}),
        files: std::collections::BTreeMap::from([(
            EXTERNAL_LOCKFILE_PATH.to_string(),
            BundleEntry {
                path: EXTERNAL_LOCKFILE_PATH.to_string(),
                data: lockfile.as_bytes().to_vec(),
                mode: "0644".to_string(),
            },
        )]),
    };

    let closure = release_external_closure_from_bundle(&bundle)
        .unwrap()
        .unwrap();

    assert_eq!(closure["source"], json!(EXTERNAL_LOCKFILE_PATH));
    assert_eq!(closure["summary"]["node_count"], json!(2));
    assert_eq!(closure["summary"]["root_count"], json!(1));
    assert_eq!(closure["summary"]["version_label_count"], json!(1));
    assert_eq!(closure["closure"]["summary"]["node_count"], json!(2));
    assert_eq!(
        closure["canonical_snapshots"][0]["identity"],
        json!("ait-db")
    );
    assert_eq!(
        closure["canonical_snapshots"][0]["snapshot"],
        json!("SNP-DB-RECURSIVE")
    );
    assert_eq!(
        closure["canonical_snapshots"][1]["identity"],
        json!(".ait-external/ait-db:ait-codec")
    );
    assert_eq!(
        closure["canonical_snapshots"][1]["snapshot"],
        json!("SNP-CODEC-RECURSIVE")
    );
    assert_eq!(closure["version_labels"][0]["version"], json!("0.1.0"));
    assert_eq!(
        closure["version_labels"][0]["snapshot"],
        json!("SNP-DB-RECURSIVE")
    );
    assert!(closure.get("canonical_versions").is_none());
}

#[test]
fn release_candidate_record_json_preserves_external_closure_metadata() {
    let record = WorkflowReleaseRecord {
        release_id: "REL-test".to_string(),
        repo_name: "ait-core".to_string(),
        version: "1.2.3".to_string(),
        line_name: "main".to_string(),
        snapshot_id: "SNP-ROOT".to_string(),
        manifest_hash: "manifest-hash".to_string(),
        profile: "public".to_string(),
        package_name: Some("ait".to_string()),
        package_version: Some("1.2.3".to_string()),
        package_requires_python: Some(">=3.11".to_string()),
        status: "candidate".to_string(),
        checks_json: "[]".to_string(),
        artifacts_json: "[]".to_string(),
        formula_json: "{}".to_string(),
        metadata_json: json!({
            "external_closure": {
                "source": EXTERNAL_LOCKFILE_PATH,
                "canonical_snapshots": [{"identity": "ait-db", "snapshot": "SNP-DB"}],
                "version_labels": [{"identity": "ait-db", "version": "0.1.0", "snapshot": "SNP-DB"}],
            }
        })
        .to_string(),
        created_at: "2026-07-05T00:00:00Z".to_string(),
        updated_at: "2026-07-05T00:00:00Z".to_string(),
    };

    let payload = release_candidate_record_json(&record);

    assert_eq!(
        payload["metadata"]["external_closure"]["source"],
        json!(EXTERNAL_LOCKFILE_PATH)
    );
    assert_eq!(
        payload["metadata"]["external_closure"]["canonical_snapshots"][0]["snapshot"],
        json!("SNP-DB")
    );
    assert_eq!(
        payload["metadata"]["external_closure"]["version_labels"][0]["version"],
        json!("0.1.0")
    );
}

fn release_test_repo(root: &Path) -> RepoRuntime {
    RepoRuntime {
        root: root.to_path_buf(),
        ait_dir: root.join(".ait"),
        config: JsonMap::from_iter([("repo_name".to_string(), json!("ait-core"))]),
        worktree_config_path: None,
    }
}

fn write_external_manifest(root: &Path, snapshot: &str) {
    fs::write(
        root.join("ait-external.toml"),
        format!(
            r#"
[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "{snapshot}"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
"#
        ),
    )
    .unwrap();
}

fn write_external_lock(root: &Path, snapshot: &str) {
    fs::write(
        root.join("ait-external.lock"),
        format!(
            r#"
format = "ait.external.lock"

[[node]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 0
remote = "origin"
line = "main"
snapshot = "{snapshot}"
parent_path = ""
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
"#
        ),
    )
    .unwrap();
}

fn write_external_marker(root: &Path, snapshot: &str) {
    let target = root.join(".ait-external").join("ait-db");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join(".ait-external-marker.json"),
        format!(
            r#"{{
  "format": "ait.external.materialized",
  "version": 3,
  "line": "main",
  "materialize_to": ".ait-external/ait-db",
  "name": "ait-db",
  "parent_path": "",
  "remote": "origin",
  "repo_name": "ait-db",
  "repository_index": 0,
  "snapshot": "{snapshot}",
  "files": []
}}"#
        ),
    )
    .unwrap();
}

fn native_command_row(command: &str) -> JsonValue {
    json!({
        "kind": "native-command",
        "command": command,
        "runtime_authority": "rust",
        "python_fallback": false,
        "cargo_profile": "release",
        "path": format!("dist/{command}-1.0.0-test-target")
    })
}

#[test]
fn native_worker_artifact_names_include_version_and_target() {
    assert_eq!(
        native_worker_artifact_filename("ait-agent-worker", "1.2.3"),
        format!(
            "ait-agent-worker-1.2.3-{}-{}{}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::env::consts::EXE_SUFFIX
        )
    );
}

#[test]
fn native_worker_artifact_is_copied_and_projected_as_a_rust_command() {
    let temp = tempfile::TempDir::new().unwrap();
    let source = temp.path().join("release");
    let dist = temp.path().join("dist");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&dist).unwrap();
    for command in REQUIRED_NATIVE_WORKER_COMMANDS {
        let path = source.join(format!("{command}{}", std::env::consts::EXE_SUFFIX));
        fs::write(&path, format!("native-{command}")).unwrap();
        set_filesystem_mode(&path, 0o755).unwrap();
    }
    let repo = release_test_repo(temp.path());

    let artifacts =
        copy_native_worker_command_artifacts_from_dir(&repo, &source, &dist, "1.2.3").unwrap();

    assert_native_worker_artifact(&artifacts).unwrap();
    assert_eq!(artifacts.len(), 1);
    for artifact in &artifacts {
        assert_eq!(artifact["kind"], json!("native-command"));
        assert_eq!(artifact["runtime_authority"], json!("rust"));
        assert_eq!(artifact["python_fallback"], json!(false));
        assert_eq!(artifact["cargo_profile"], json!("release"));
        let destination = PathBuf::from(artifact["absolute_path"].as_str().unwrap());
        assert!(destination.is_file());
        assert_ne!(
            filesystem_mode(&fs::metadata(destination).unwrap(), 0o755) & 0o111,
            0
        );
    }
}

#[test]
fn native_worker_artifact_rejects_missing_duplicate_and_python_fallback() {
    let missing = Vec::new();
    assert!(assert_native_worker_artifact(&missing)
        .unwrap_err()
        .contains("missing: ait-agent-worker"));

    let duplicate = vec![
        native_command_row("ait-agent-worker"),
        native_command_row("ait-agent-worker"),
    ];
    assert!(assert_native_worker_artifact(&duplicate)
        .unwrap_err()
        .contains("duplicate `ait-agent-worker`"));

    let mut fallback = native_command_row("ait-agent-worker");
    fallback["python_fallback"] = json!(true);
    let fallback = vec![fallback];
    assert!(assert_native_worker_artifact(&fallback)
        .unwrap_err()
        .contains("Python fallback disabled"));
}

#[test]
fn native_worker_artifact_copy_rejects_missing_or_debug_artifacts() {
    let temp = tempfile::TempDir::new().unwrap();
    let source = temp.path().join("release");
    let dist = temp.path().join("dist");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&dist).unwrap();
    let repo = release_test_repo(temp.path());
    assert!(
        copy_native_worker_command_artifacts_from_dir(&repo, &source, &dist, "1.0.0")
            .unwrap_err()
            .contains("ait-agent-worker")
    );

    let debug_source = temp.path().join("target/debug");
    fs::create_dir_all(&debug_source).unwrap();
    assert!(
        copy_native_worker_command_artifacts_from_dir(&repo, &debug_source, &dist, "1.0.0")
            .unwrap_err()
            .contains("Refusing debug-profile")
    );
}

#[test]
fn publish_ready_requires_build_profile_contract_metadata() {
    let record = json!({
        "release_id": "REL-test",
        "checks": [{"id": "fixture", "blocking": false}],
        "artifacts": [
            {"kind": "sdist", "path": "dist/ait-1.0.0.tar.gz"},
            {"kind": "wheel", "path": "dist/ait-1.0.0-py3-none-any.whl"},
            {"kind": "manifest", "path": "dist/ait-release-1.0.0.manifest.json"},
            {"kind": "checksum", "path": "dist/ait-release-1.0.0.sha256"}
        ],
        "metadata": {"build": {"builder": "ait_rust_internal_sdist_and_wheel"}}
    });

    let error = assert_publish_ready(&record).unwrap_err();
    assert!(error.contains("missing the Rust release build-profile contract"));
}

#[test]
fn publish_ready_requires_lean_ci_profile_metadata() {
    let record = json!({
        "release_id": "REL-test",
        "checks": [{"id": "fixture", "blocking": false}],
        "artifacts": [
            {"kind": "sdist", "path": "dist/ait-1.0.0.tar.gz"},
            {"kind": "wheel", "path": "dist/ait-1.0.0-py3-none-any.whl"},
            {"kind": "manifest", "path": "dist/ait-release-1.0.0.manifest.json"},
            {"kind": "checksum", "path": "dist/ait-release-1.0.0.sha256"}
        ],
        "metadata": {"build": {"rust_release_profile": rust_release_profile_contract()}}
    });

    let error = assert_publish_ready(&record).unwrap_err();
    assert!(error.contains("missing the Rust lean-CI build-profile contract"));
}

#[test]
fn publish_ready_rejects_debug_cargo_target_artifact_paths() {
    let record = json!({
        "release_id": "REL-test",
        "checks": [{"id": "fixture", "blocking": false}],
        "artifacts": [
            {"kind": "sdist", "path": "dist/ait-1.0.0.tar.gz"},
            {"kind": "wheel", "path": "target/debug/ait-cli"},
            {"kind": "manifest", "path": "dist/ait-release-1.0.0.manifest.json"},
            {"kind": "checksum", "path": "dist/ait-release-1.0.0.sha256"}
        ],
        "metadata": {"build": {
            "rust_release_profile": rust_release_profile_contract(),
            "rust_ci_profile": rust_ci_profile_contract()
        }}
    });

    let error = assert_publish_ready(&record).unwrap_err();
    assert!(error.contains("must not reference debug Cargo target outputs"));
}

#[test]
fn publish_ready_requires_the_native_worker_command() {
    let record = json!({
        "release_id": "REL-test",
        "checks": [{"id": "fixture", "blocking": false}],
        "artifacts": [
            {"kind": "sdist", "path": "dist/ait-1.0.0.tar.gz"},
            {"kind": "wheel", "path": "dist/ait-1.0.0-py3-none-any.whl"},
            {"kind": "manifest", "path": "dist/ait-release-1.0.0.manifest.json"},
            {"kind": "checksum", "path": "dist/ait-release-1.0.0.sha256"}
        ],
        "metadata": {"build": {
            "rust_release_profile": rust_release_profile_contract(),
            "rust_ci_profile": rust_ci_profile_contract()
        }}
    });

    let error = assert_publish_ready(&record).unwrap_err();
    assert!(error.contains("missing: ait-agent-worker"));
}

#[test]
fn release_publish_metadata_preserves_build_profile_contracts() {
    let record = json!({
        "metadata": {
            "build": {
                "built_at": "2026-06-29T00:00:00Z",
                "source_date_epoch": 123,
                "builder": "ait_rust_internal_sdist_and_wheel",
                "rust_release_profile": rust_release_profile_contract(),
                "rust_ci_profile": rust_ci_profile_contract()
            }
        }
    });

    let metadata = release_publish_metadata(&record);
    assert_eq!(
        metadata["build"]["rust_release_profile"],
        rust_release_profile_contract()
    );
    assert_eq!(
        metadata["build"]["rust_ci_profile"],
        rust_ci_profile_contract()
    );
}

#[test]
fn workflow_release_local_crud_fails_closed_without_workflow_authority() {
    let temp = tempfile::TempDir::new().unwrap();
    let repo = release_test_repo(temp.path());

    let error = create_workflow_release_explicit(
        &repo,
        " R-1 ",
        " ait-core ",
        " 1.2.3 ",
        " main ",
        " SNP-1 ",
        " abc123 ",
        " public ",
        Some("ait"),
        Some("1.2.3"),
        Some(">=3.11"),
        None,
        "[]",
        "[]",
        "{}",
        "{\"package\":{\"name\":\"ait\"}}",
    )
    .unwrap_err();
    assert_eq!(
        error,
        ait_core::agent_local_workflow_backend::LOCAL_WORKFLOW_AUTHORITY_ERROR
    );
    assert_eq!(
        list_workflow_releases(&repo).unwrap_err(),
        ait_core::agent_local_workflow_backend::LOCAL_WORKFLOW_AUTHORITY_ERROR
    );
    assert!(!temp
        .path()
        .join(".ait/binary-db/workflow_record.bin")
        .exists());
    assert!(!temp
        .path()
        .join(".ait/binary-db/workflow_record_payload.bin")
        .exists());
}
