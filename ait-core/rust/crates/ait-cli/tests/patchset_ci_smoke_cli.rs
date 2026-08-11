use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::{Builder as TempDirBuilder, TempDir};

#[path = "../../../test_support.rs"]
mod workspace_test_support;

const PACKAGE_SMOKE_CONCURRENT_WORKERS: usize = 4;
const PACKAGE_SMOKE_REPETITIONS_PER_WORKER: usize = 3;

fn cargo_bin() -> Command {
    Command::cargo_bin("ait-cli").unwrap()
}

fn run_ok(root: &Path, args: &[&str]) -> JsonValue {
    let output = cargo_bin().current_dir(root).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid CLI JSON").unwrap()
}

fn run_ok_with_env(root: &Path, args: &[&str], envs: &[(&str, &Path)]) -> JsonValue {
    let mut command = cargo_bin();
    command.current_dir(root).args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid CLI JSON").unwrap()
}

fn init_repo(temp: &TempDir) -> std::path::PathBuf {
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let output = cargo_bin()
        .current_dir(&root)
        .args(["init", "--name", "fixture", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    root
}

fn tempdir_outside_current_repo_with_prefix(prefix: &str) -> Result<TempDir, String> {
    let root = env::var_os("AIT_TEST_OUTSIDE_REPO_TMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let unix_tmp = PathBuf::from("/tmp");
            if unix_tmp.is_dir() {
                unix_tmp
            } else {
                env::temp_dir()
            }
        });
    fs::create_dir_all(&root).map_err(|error| {
        format!(
            "failed to create isolated test fixture base {}: {error}",
            root.display()
        )
    })?;
    let canonical_root = fs::canonicalize(&root).map_err(|error| {
        format!(
            "failed to canonicalize isolated test fixture base {}: {error}",
            root.display()
        )
    })?;
    TempDirBuilder::new()
        .prefix(prefix)
        .tempdir_in(&canonical_root)
        .map_err(|error| {
            format!(
                "failed to allocate exclusive test fixture under {}: {error}",
                canonical_root.display()
            )
        })
}

fn tempdir_outside_current_repo() -> TempDir {
    tempdir_outside_current_repo_with_prefix("ait-patchset-ci-")
        .expect("exclusive patchset CI fixture")
}

struct PackageSmokeFixture {
    _temp: TempDir,
    fixture_id: String,
    scope_root: PathBuf,
    repo_root: PathBuf,
    ownership_markers: Vec<PathBuf>,
}

impl PackageSmokeFixture {
    fn new(fixture_id: impl Into<String>) -> Result<Self, String> {
        let fixture_id = fixture_id.into();
        let temp = tempdir_outside_current_repo_with_prefix("ait-package-smoke-")?;
        let scope_root = fs::canonicalize(temp.path()).map_err(|error| {
            format!(
                "package-smoke fixture `{fixture_id}` phase `allocate` could not canonicalize {}: {error}",
                temp.path().display()
            )
        })?;
        let repo_root = scope_root.join("repo");
        let ownership_markers = [
            scope_root.join("current-source-cache/owner"),
            scope_root.join("release-smoke/owner"),
            scope_root.join("sibling-test/owner"),
        ]
        .into_iter()
        .collect::<Vec<_>>();

        fs::create_dir_all(&repo_root).map_err(|error| {
            format!(
                "package-smoke fixture `{fixture_id}` phase `allocate` could not create repository root {}: {error}",
                repo_root.display()
            )
        })?;
        for marker in &ownership_markers {
            let parent = marker.parent().ok_or_else(|| {
                format!(
                    "package-smoke fixture `{fixture_id}` phase `allocate` has no parent for ownership marker {}",
                    marker.display()
                )
            })?;
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "package-smoke fixture `{fixture_id}` phase `allocate` could not create owned scratch root {}: {error}",
                    parent.display()
                )
            })?;
            fs::write(marker, &fixture_id).map_err(|error| {
                format!(
                    "package-smoke fixture `{fixture_id}` phase `allocate` could not write ownership marker {}: {error}",
                    marker.display()
                )
            })?;
        }

        let output = cargo_bin()
            .current_dir(&repo_root)
            .env("AIT_REPO_ROOT", &repo_root)
            .args(["init", "--name", "package-smoke-fixture", "--json"])
            .output()
            .map_err(|error| {
                format!(
                    "package-smoke fixture `{fixture_id}` phase `ait-init` could not launch ait-cli in {}: {error}",
                    repo_root.display()
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "package-smoke fixture `{fixture_id}` phase `ait-init` failed in {}\nstdout:\n{}\nstderr:\n{}",
                repo_root.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let repo_root = fs::canonicalize(&repo_root).map_err(|error| {
            format!(
                "package-smoke fixture `{fixture_id}` phase `ait-init` removed or invalidated repository root {}: {error}",
                repo_root.display()
            )
        })?;
        if !repo_root.starts_with(&scope_root) || repo_root == scope_root {
            return Err(format!(
                "package-smoke fixture `{fixture_id}` repository root {} escaped exclusive scope {}",
                repo_root.display(),
                scope_root.display()
            ));
        }

        let fixture = Self {
            _temp: temp,
            fixture_id,
            scope_root,
            repo_root,
            ownership_markers,
        };
        fixture.assert_owned_paths_intact("ait-init")?;
        Ok(fixture)
    }

    fn write(&self, relative_path: &str, content: &str, phase: &str) -> Result<(), String> {
        let relative = Path::new(relative_path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "package-smoke fixture `{}` phase `{phase}` rejected non-contained path `{relative_path}`",
                self.fixture_id
            ));
        }

        let target = self.repo_root.join(relative);
        let parent = target.parent().ok_or_else(|| {
            format!(
                "package-smoke fixture `{}` phase `{phase}` has no parent for {}",
                self.fixture_id,
                target.display()
            )
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "package-smoke fixture `{}` phase `{phase}` failed to create parent {} for {}: {error}",
                self.fixture_id,
                parent.display(),
                target.display()
            )
        })?;
        let canonical_parent = fs::canonicalize(parent).map_err(|error| {
            format!(
                "package-smoke fixture `{}` phase `{phase}` could not canonicalize parent {} for {}: {error}",
                self.fixture_id,
                parent.display(),
                target.display()
            )
        })?;
        if !canonical_parent.starts_with(&self.repo_root) {
            return Err(format!(
                "package-smoke fixture `{}` phase `{phase}` parent {} escaped repository root {}",
                self.fixture_id,
                canonical_parent.display(),
                self.repo_root.display()
            ));
        }
        fs::write(&target, content).map_err(|error| {
            format!(
                "package-smoke fixture `{}` phase `{phase}` failed to write {} (parent: {}, parent_exists: {}): {error}",
                self.fixture_id,
                target.display(),
                canonical_parent.display(),
                canonical_parent.is_dir()
            )
        })
    }

    fn run_public_package_smoke(&self) -> Result<JsonValue, String> {
        let output = cargo_bin()
            .current_dir(&self.repo_root)
            .env("AIT_REPO_ROOT", &self.repo_root)
            .args(["test", "patchset-ci", "package-smoke", "--json"])
            .output()
            .map_err(|error| {
                format!(
                    "package-smoke fixture `{}` phase `public-command` could not launch ait-cli in {}: {error}",
                    self.fixture_id,
                    self.repo_root.display()
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "package-smoke fixture `{}` phase `public-command` failed in {}\nstdout:\n{}\nstderr:\n{}",
                self.fixture_id,
                self.repo_root.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let payload = JsonCodec::parse_slice_with_error_prefix(
            &output.stdout,
            "Invalid package-smoke CLI JSON",
        )
        .map_err(|error| {
            format!(
                "package-smoke fixture `{}` phase `public-command` returned invalid JSON: {error}",
                self.fixture_id
            )
        })?;
        let reported_root = payload["workspace_root"].as_str().ok_or_else(|| {
            format!(
                "package-smoke fixture `{}` phase `public-command` omitted workspace_root",
                self.fixture_id
            )
        })?;
        if Path::new(reported_root) != self.repo_root {
            return Err(format!(
                "package-smoke fixture `{}` phase `public-command` inspected workspace {} instead of owned repository {}",
                self.fixture_id,
                reported_root,
                self.repo_root.display()
            ));
        }
        self.assert_owned_paths_intact("public-command")?;
        Ok(payload)
    }

    fn assert_owned_paths_intact(&self, phase: &str) -> Result<(), String> {
        if fs::canonicalize(&self.scope_root).ok().as_deref() != Some(self.scope_root.as_path()) {
            return Err(format!(
                "package-smoke fixture `{}` phase `{phase}` lost canonical scope root {}",
                self.fixture_id,
                self.scope_root.display()
            ));
        }
        for marker in &self.ownership_markers {
            let owner = fs::read_to_string(marker).map_err(|error| {
                format!(
                    "package-smoke fixture `{}` phase `{phase}` lost ownership marker {}: {error}",
                    self.fixture_id,
                    marker.display()
                )
            })?;
            if owner != self.fixture_id {
                return Err(format!(
                    "package-smoke fixture `{}` phase `{phase}` ownership marker {} was replaced by `{owner}`",
                    self.fixture_id,
                    marker.display()
                ));
            }
        }
        Ok(())
    }
}

fn populate_package_smoke_fixture(fixture: &PackageSmokeFixture) -> Result<(), String> {
    for (path, content, phase) in [
        (
            "src/ait/cli/app.py",
            "def main():\n    return 'ok'\n",
            "public-app",
        ),
        (
            "pyproject.toml",
            "[project.scripts]\nait = \"ait.cli_entrypoint:main\"\n",
            "console-script",
        ),
        (
            "src/ait/cli_entrypoint.py",
            "NATIVE = {\"release\"}\nos.execvpe(binary, argv, env)\nfrom .cli import app\n",
            "native-entrypoint",
        ),
        (
            "src/ait/cli/app_surfaces.py",
            "# no release route\n",
            "python-surface",
        ),
        (
            "src/ait/cli/native_namespace_command.py",
            "NATIVE_WORKFLOW_GATE_NAMESPACES = set()\n",
            "native-namespace",
        ),
        (
            "src/ait/cli/commands/bootstrap.py",
            "PRIMARY_COMMAND_MODULES = {}\n",
            "command-bootstrap",
        ),
        (
            "ci/patch_ci.json",
            "AIT_SHARED_CARGO_TARGET_DIR/debug/ait-cli test patchset-ci release-artifact-smoke --json\n",
            "native-release-smoke",
        ),
        (
            "docs/sprints/README.md",
            "directory note only\nshould not become\nprimary entry surface\n",
            "sprint-directory-note",
        ),
        (
            "docs/sprint_artifact_routing.md",
            "Do not treat `docs/sprints/README.md`\nauthority layer\n",
            "sprint-routing",
        ),
        (
            "docs/ait_native_quickstart.md",
            "must not create\ndocs/sprints/README.md\nsprint entry surface\n",
            "native-quickstart",
        ),
    ] {
        fixture.write(path, content, phase)?;
    }
    fixture.assert_owned_paths_intact("populate")
}

fn run_isolated_package_smoke(fixture_id: String) -> Result<PathBuf, String> {
    let fixture = PackageSmokeFixture::new(fixture_id)?;
    populate_package_smoke_fixture(&fixture)?;
    let payload = fixture.run_public_package_smoke()?;
    if payload["status"].as_str() != Some("pass")
        || payload["runner"].as_str() != Some("ait-cli")
        || payload["rust_only"].as_bool() != Some(true)
        || payload["suite_id"].as_str() != Some("package-smoke")
        || payload["contract"].as_str() != Some("AT.patchset_ci.package_smoke.v1")
    {
        return Err(format!(
            "package-smoke fixture `{}` phase `contract` returned unexpected payload: {payload:?}",
            fixture.fixture_id
        ));
    }
    Ok(fixture.scope_root.clone())
}

fn repo_root_for_patch_ci_manifest() -> PathBuf {
    let mut candidates = Vec::new();
    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir);
    }
    candidates.push(workspace_test_support::crate_root("ait-cli"));

    for candidate in candidates {
        for ancestor in candidate.ancestors() {
            if ancestor.join("ci/patch_ci.json").is_file()
                && ancestor.join("rust/Cargo.toml").is_file()
            {
                let marker_path = ancestor.join(".ait-worktree.json");
                if !marker_path.is_file() {
                    return ancestor.to_path_buf();
                }

                let marker = fs::read_to_string(&marker_path).unwrap_or_else(|error| {
                    panic!(
                        "failed to read task-worktree marker {}: {error}",
                        marker_path.display()
                    )
                });
                let marker: JsonValue = JsonCodec::parse_value_with_error_prefix(
                    &marker,
                    "Invalid task-worktree marker",
                )
                .unwrap();
                let source_root = marker["repo_root"]
                    .as_str()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        panic!(
                            "task-worktree marker {} is missing repo_root",
                            marker_path.display()
                        )
                    });
                let source_root = fs::canonicalize(&source_root).unwrap_or_else(|error| {
                    panic!(
                        "failed to canonicalize task-worktree source root {}: {error}",
                        source_root.display()
                    )
                });
                assert!(
                    source_root.join("ci/patch_ci.json").is_file()
                        && source_root.join("rust/Cargo.toml").is_file(),
                    "task-worktree marker {} points at a non-repository source root {}",
                    marker_path.display(),
                    source_root.display()
                );
                return source_root;
            }
        }
    }
    panic!("could not locate repo root containing ci/patch_ci.json");
}

#[test]
fn repo_patch_ci_manifest_uses_sharded_runner_without_command_bundle() {
    let manifest_path = repo_root_for_patch_ci_manifest().join("ci/patch_ci.json");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let cargo_config =
        fs::read_to_string(repo_root_for_patch_ci_manifest().join(".cargo/config.toml")).unwrap();
    let manifest_json: JsonValue =
        JsonCodec::parse_value_with_error_prefix(&manifest, "Invalid patch CI manifest").unwrap();
    let runner = &manifest_json["suites"][0]["runner"];
    let checks = runner["checks"].as_array().unwrap();

    assert!(
        !manifest.contains("CARGO_TARGET_DIR=\"$(mktemp"),
        "patch CI must preserve the scheduler-provided shared Cargo target dir"
    );
    assert!(
        cargo_config.contains("target-dir = \".ait/cargo-target\"")
            && cargo_config.contains(
                "build-dir = \".ait/cargo-build/workspaces/{workspace-path-hash}\""
            ),
        "raw Cargo must keep final artifacts stable and intermediates repository-shared but workspace-isolated"
    );
    assert!(
        manifest_json.get("prewarm").is_none(),
        "patch CI must not use a shell prewarm block; the server runner builds once"
    );
    assert_eq!(runner["kind"].as_str(), Some("test_discovery_sharded"));
    assert_eq!(runner["adapter"].as_str(), Some("cargo"));
    assert_eq!(runner["manifest_path"].as_str(), Some("rust/Cargo.toml"));
    assert_eq!(runner["workspace"].as_bool(), Some(false));
    assert_eq!(runner["doc_tests"].as_bool(), Some(false));
    let build_args = runner["build_args"].as_array().unwrap();
    let build_args = build_args
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        build_args,
        vec!["ait-core-patch-ci-build", "--message-format=json"],
        "patch CI and local builds should share one Cargo alias"
    );
    assert!(
        cargo_config.contains("\"ait-agent-core\"")
            && cargo_config.contains("\"ait-py\"")
            && cargo_config.contains("\"ait-cli\""),
        "patch CI should keep focused package coverage in the native Cargo adapter"
    );
    assert!(
        cargo_config.contains("\"--lib\"")
            && cargo_config.contains("\"--bin\"")
            && cargo_config.contains("\"--test\"")
            && cargo_config.contains("\"patchset_ci_smoke_cli\"")
            && cargo_config.contains("\"--profile\"")
            && cargo_config.contains("\"ait-ci\"")
            && !cargo_config.contains("\"--release\""),
        "patch CI should keep ait-cli lib, binary, and patchset smoke coverage without full integration tests"
    );
    assert!(
        !cargo_config.contains("\"--workspace\""),
        "patch CI should not run the full ait-core workspace integration suite"
    );
    assert_eq!(
        checks
            .iter()
            .filter(|check| check["kind"] == "cargo_fmt")
            .count(),
        1,
        "patch CI should preserve rustfmt as a native runner check"
    );
    assert_eq!(
        checks
            .iter()
            .filter(|check| check["kind"] == "forbid_files"
                && check["check_id"] == "no_python_boundary")
            .count(),
        1,
        "patch CI should keep the no-Python repository boundary check"
    );
    assert!(
        runner.get("commands").is_none(),
        "patch CI must not use shell command bundles"
    );
    assert!(
        !manifest.contains("command_bundle"),
        "patch CI should use the generic sharded runner"
    );
    assert!(
        !manifest.contains("cargo test"),
        "patch CI must not encode shell Cargo commands"
    );
}

#[test]
fn repo_full_test_manifests_require_only_existing_main_seed_paths() {
    let root = repo_root_for_patch_ci_manifest();
    let manifest = fs::read_to_string(root.join("ci/patch_ci.json")).unwrap();
    let manifest_json: JsonValue =
        JsonCodec::parse_value_with_error_prefix(&manifest, "Invalid patch CI manifest").unwrap();
    let suites = manifest_json["suites"].as_array().unwrap();
    let full_suites = suites
        .iter()
        .filter(|suite| suite["runner"]["kind"].as_str() == Some("test_shard"))
        .collect::<Vec<_>>();

    assert_eq!(full_suites.len(), 2);
    for suite in full_suites {
        let suite_id = suite["suite_id"].as_str().unwrap();
        assert!(matches!(suite_id, "full_repo" | "full_repo_zstd_only"));
        assert_eq!(suite["plane"].as_str(), Some("nightly"));
        let required_paths = suite["main_seed_prewarm"]["required_paths"]
            .as_array()
            .unwrap();
        assert!(!required_paths.is_empty());
        for required_path in required_paths {
            let required_path = required_path.as_str().unwrap();
            assert!(
                root.join(required_path).exists(),
                "suite {suite_id} requires missing main-seed path {required_path}"
            );
        }
    }
}

#[test]
fn hidden_patchset_ci_accepts_materialized_workspace_without_ait_metadata() {
    let temp = tempdir_outside_current_repo();
    let unrelated_repo = temp.path().join("unrelated-repo");
    let runner = unrelated_repo.join("runner");
    let workspace = unrelated_repo.join("workspace");
    fs::create_dir_all(unrelated_repo.join(".ait")).unwrap();
    fs::create_dir_all(&runner).unwrap();
    fs::create_dir_all(workspace.join("docs")).unwrap();
    fs::write(workspace.join("docs/plan.md"), "# Plan\n\nNo links here.\n").unwrap();

    let payload = run_ok_with_env(
        &runner,
        &["test", "patchset-ci", "preflight", "--json"],
        &[("AIT_REPO_ROOT", &workspace)],
    );
    assert_eq!(payload["status"].as_str(), Some("pass"));
    let expected_workspace = fs::canonicalize(&workspace).unwrap();
    assert_eq!(
        payload["workspace_root"].as_str(),
        Some(expected_workspace.to_string_lossy().as_ref())
    );
}

#[test]
fn hidden_patchset_ci_rejects_missing_explicit_workspace_authority() {
    let temp = tempdir_outside_current_repo();
    let runner = temp.path().join("runner");
    let missing_workspace = temp.path().join("missing-workspace");
    fs::create_dir_all(&runner).unwrap();

    let output = cargo_bin()
        .current_dir(&runner)
        .args(["test", "patchset-ci", "preflight", "--json"])
        .env("AIT_REPO_ROOT", &missing_workspace)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Patchset CI workspace authority `AIT_REPO_ROOT`"));
    assert!(stderr.contains("missing-workspace"));
}

#[test]
fn hidden_patchset_preflight_command_passes_and_identifies_rust_runner() {
    let temp = TempDir::new().unwrap();
    let root = init_repo(&temp);

    let payload = run_ok(&root, &["test", "patchset-ci", "preflight", "--json"]);
    assert_eq!(payload["status"].as_str(), Some("pass"));
    assert_eq!(payload["runner"].as_str(), Some("ait-cli"));
    assert_eq!(payload["rust_only"].as_bool(), Some(true));
    assert_eq!(payload["suite_id"].as_str(), Some("preflight"));
    assert_eq!(
        payload["contract"].as_str(),
        Some("AT.patchset_ci.preflight.v1")
    );
}

#[test]
fn hidden_patchset_package_smoke_command_passes() {
    run_isolated_package_smoke("isolated-single".to_string())
        .expect("isolated public package-smoke fixture");
}

#[test]
fn hidden_patchset_package_smoke_is_repeated_and_concurrent_without_shared_paths() {
    let start = Arc::new(Barrier::new(PACKAGE_SMOKE_CONCURRENT_WORKERS));
    let handles = (0..PACKAGE_SMOKE_CONCURRENT_WORKERS)
        .map(|worker| {
            let start = Arc::clone(&start);
            thread::spawn(move || -> Result<Vec<PathBuf>, String> {
                start.wait();
                let mut roots = Vec::new();
                for repetition in 0..PACKAGE_SMOKE_REPETITIONS_PER_WORKER {
                    roots.push(run_isolated_package_smoke(format!(
                        "worker-{worker}-repetition-{repetition}"
                    ))?);
                }
                Ok(roots)
            })
        })
        .collect::<Vec<_>>();

    let mut roots = Vec::new();
    for (worker, handle) in handles.into_iter().enumerate() {
        let worker_roots = handle
            .join()
            .unwrap_or_else(|_| panic!("package-smoke worker {worker} panicked"))
            .unwrap_or_else(|error| panic!("package-smoke worker {worker} failed: {error}"));
        roots.extend(worker_roots);
    }

    assert_eq!(
        roots.len(),
        PACKAGE_SMOKE_CONCURRENT_WORKERS * PACKAGE_SMOKE_REPETITIONS_PER_WORKER
    );
    assert_eq!(
        roots.iter().cloned().collect::<BTreeSet<_>>().len(),
        roots.len(),
        "every repeated package-smoke execution must own a unique canonical fixture root"
    );
    for (index, root) in roots.iter().enumerate() {
        for other in roots.iter().skip(index + 1) {
            assert!(
                !root.starts_with(other) && !other.starts_with(root),
                "package-smoke fixture roots must not contain each other: {} vs {}",
                root.display(),
                other.display()
            );
        }
    }
}

#[test]
fn hidden_patchset_stable_smoke_command_passes() {
    let temp = TempDir::new().unwrap();
    let root = init_repo(&temp);

    let payload = run_ok(&root, &["test", "patchset-ci", "stable-smoke", "--json"]);
    assert_eq!(payload["status"].as_str(), Some("pass"));
    assert!(payload["request_count"].as_u64().unwrap_or(0) > 0);
    assert_eq!(payload["runner"].as_str(), Some("ait-cli"));
    assert_eq!(payload["rust_only"].as_bool(), Some(true));
    assert_eq!(payload["suite_id"].as_str(), Some("stable-smoke"));
    assert_eq!(
        payload["contract"].as_str(),
        Some("AT.patchset_ci.stable_smoke.v1")
    );
}

#[test]
fn hidden_patchset_tg1_required_command_runs_native_case_ids() {
    let temp = TempDir::new().unwrap();
    let root = init_repo(&temp);

    let payload = run_ok(
        &root,
        &["test", "patchset-ci", "tg1-required", "--json", "26"],
    );
    assert_eq!(payload["status"].as_str(), Some("pass"));
    assert_eq!(payload["rust_only"].as_bool(), Some(true));
    assert_eq!(payload["case_indices"][0].as_i64(), Some(26));
}

#[test]
fn hidden_patchset_tg1_required_accepts_current_contract_corpus_node_ids() {
    let temp = TempDir::new().unwrap();
    let root = init_repo(&temp);

    let payload = run_ok(
        &root,
        &[
            "test",
            "patchset-ci",
            "tg1-required",
            "--json",
            "corpora/ait/full_repo/tests/cli/test_land_workflow.py::test_remote_land_excludes_non_docs_root_markdown",
        ],
    );
    assert_eq!(payload["status"].as_str(), Some("pass"));
    assert_eq!(payload["case_indices"][0].as_i64(), Some(15));
}
