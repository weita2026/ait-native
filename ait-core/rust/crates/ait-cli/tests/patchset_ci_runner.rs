use ait_cli::patchset_ci_smoke::{
    run_package_smoke, run_preflight, run_stable_smoke, run_tg1_required,
};
use ait_cli::runtime::RepoRuntime;
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

fn ait_cli_executable() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ait-cli"))
}

fn init_repo(temp: &TempDir) -> std::path::PathBuf {
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).unwrap();
    let output = cargo_bin()
        .current_dir(&root)
        .args(["init", "--json"])
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
    let unix_tmp = PathBuf::from("/tmp");
    let root = if unix_tmp.is_dir() {
        unix_tmp
    } else {
        env::temp_dir()
    };
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
            .args(["init", "--json"])
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

    fn run_internal_package_smoke(&self) -> Result<JsonValue, String> {
        let repo = RepoRuntime::discover_from_path(&self.repo_root).map_err(|error| {
            format!(
                "package-smoke fixture `{}` phase `internal-runner` could not open {}: {error}",
                self.fixture_id,
                self.repo_root.display()
            )
        })?;
        let payload = run_package_smoke(&repo, None, &ait_cli_executable()).map_err(|error| {
            format!(
                "package-smoke fixture `{}` phase `internal-runner` failed in {}: {error}",
                self.fixture_id,
                self.repo_root.display()
            )
        })?;
        let reported_root = payload["workspace_root"].as_str().ok_or_else(|| {
            format!(
                "package-smoke fixture `{}` phase `internal-runner` omitted workspace_root",
                self.fixture_id
            )
        })?;
        if Path::new(reported_root) != self.repo_root {
            return Err(format!(
                "package-smoke fixture `{}` phase `internal-runner` inspected workspace {} instead of owned repository {}",
                self.fixture_id,
                reported_root,
                self.repo_root.display()
            ));
        }
        self.assert_owned_paths_intact("internal-runner")?;
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
            r#"{"schema_version":1,"suites":[{"runner":{"kind":"test_discovery_sharded","build_args":["test","--test","patchset_ci_runner","--no-run"]}}]}"#,
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
    let payload = fixture.run_internal_package_smoke()?;
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
                return ancestor.to_path_buf();
            }
        }
    }
    panic!("could not locate repo root containing ci/patch_ci.json");
}

#[test]
fn repo_patch_ci_manifest_uses_sharded_runner_without_command_bundle() {
    let manifest_path = repo_root_for_patch_ci_manifest().join("ci/patch_ci.json");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let manifest_json: JsonValue =
        JsonCodec::parse_value_with_error_prefix(&manifest, "Invalid patch CI manifest").unwrap();
    let runner = &manifest_json["suites"][0]["runner"];
    let checks = runner["checks"].as_array().unwrap();

    assert!(
        !manifest.contains("CARGO_TARGET_DIR=\"$(mktemp"),
        "patch CI must preserve the scheduler-provided shared Cargo target dir"
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
        vec![
            "test",
            "--profile",
            "ait-ci",
            "-p",
            "ait-core",
            "-p",
            "ait-cli",
            "-p",
            "ait-agent-core",
            "-p",
            "ait-py",
            "--lib",
            "--test",
            "server_source_ownership",
            "--test",
            "patchset_ci_runner",
            "--no-run",
            "--message-format=json",
        ],
        "Snapshot-only patch CI discovery must build every remotely runnable harness without requiring lineage-only Markdown"
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

fn expected_native_patchset_cargo_invocations() -> Vec<Vec<String>> {
    vec![
        vec![
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "--profile",
            "ait-ci",
            "-p",
            "ait-core",
            "-p",
            "ait-cli",
            "-p",
            "ait-agent-core",
            "-p",
            "ait-py",
            "--lib",
            "--test",
            "server_source_ownership",
            "--test",
            "patchset_ci_runner",
            "--no-run",
        ],
        vec![
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "--profile",
            "ait-ci",
            "-p",
            "ait-core",
            "--lib",
            "--test",
            "server_source_ownership",
        ],
        vec![
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "--profile",
            "ait-ci",
            "-p",
            "ait-core",
            "--test",
            "binary_db_schema_authority",
        ],
        vec![
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "--profile",
            "ait-ci",
            "-p",
            "ait-cli",
            "--lib",
        ],
        vec![
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "--profile",
            "ait-ci",
            "-p",
            "ait-agent-core",
            "--lib",
        ],
        vec![
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "--profile",
            "ait-ci",
            "-p",
            "ait-py",
            "--lib",
        ],
        vec![
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "--profile",
            "ait-ci",
            "-p",
            "ait-cli",
            "--test",
            "patchset_ci_runner",
        ],
    ]
    .into_iter()
    .map(|arguments| arguments.into_iter().map(str::to_owned).collect())
    .collect()
}

fn shell_cargo_test_invocations(script: &str) -> Vec<Vec<String>> {
    script
        .replace("\\\n", " ")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("cargo "))
        .filter(|line| line.starts_with("test "))
        .map(|line| line.split_whitespace().map(str::to_owned).collect())
        .collect()
}

fn powershell_cargo_test_invocations(script: &str) -> Vec<Vec<String>> {
    script
        .split("Invoke-Checked \"cargo\" @(")
        .skip(1)
        .filter_map(|tail| {
            let mut end = 0;
            for line in tail.split_inclusive('\n') {
                if line.trim() == ")" {
                    return Some(&tail[..end]);
                }
                end += line.len();
            }
            None
        })
        .map(|arguments| {
            arguments
                .split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|arguments| arguments.first().map(String::as_str) == Some("test"))
        .collect()
}

fn script_region_between<'a>(script: &'a str, start: &str, end: &str) -> &'a str {
    let start_index = script
        .find(start)
        .unwrap_or_else(|| panic!("script is missing guard start `{start}`"));
    let guarded = &script[start_index..];
    let end_index = guarded
        .find(end)
        .unwrap_or_else(|| panic!("script guard `{start}` is missing end `{end}`"));
    &guarded[..end_index + end.len()]
}

#[test]
fn repo_native_patchset_entrypoints_guard_lineage_schema_beyond_discovered_harnesses() {
    let root = repo_root_for_patch_ci_manifest();
    let unix = fs::read_to_string(root.join("ci/run.sh")).unwrap();
    let windows = fs::read_to_string(root.join("ci/run.ps1")).unwrap();
    let expected = expected_native_patchset_cargo_invocations();

    assert_eq!(
        shell_cargo_test_invocations(&unix),
        expected,
        "Unix Patchset CI must build every Snapshot-runnable harness and conditionally execute the local schema authority harness"
    );
    assert_eq!(
        powershell_cargo_test_invocations(&windows),
        expected,
        "Windows Patchset CI must build every Snapshot-runnable harness and conditionally execute the local schema authority harness"
    );

    let expected_schema_invocation = vec![expected[2].clone()];
    let unix_schema_guard = script_region_between(
        &unix,
        "if [ -f \"$repo_root/docs/binary_db_v0.md\" ]; then",
        "\nfi",
    );
    assert_eq!(
        shell_cargo_test_invocations(unix_schema_guard),
        expected_schema_invocation,
        "Unix must execute the schema-authority harness only while its lineage-only Markdown source exists"
    );
    let windows_schema_guard = script_region_between(
        &windows,
        "if (Test-Path -LiteralPath (Join-Path $repoRoot \"docs/binary_db_v0.md\") -PathType Leaf) {",
        "\n    } else {",
    );
    assert_eq!(
        powershell_cargo_test_invocations(windows_schema_guard),
        vec![expected[2].clone()],
        "Windows must execute the schema-authority harness only while its lineage-only Markdown source exists"
    );
}

#[test]
fn internal_patchset_preflight_runner_passes() {
    let temp = TempDir::new().unwrap();
    let root = init_repo(&temp);
    let repo = RepoRuntime::discover_from_path(&root).unwrap();

    let payload = run_preflight(&repo).unwrap();
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
fn internal_patchset_package_smoke_runner_passes() {
    run_isolated_package_smoke("isolated-single".to_string())
        .expect("isolated internal package-smoke fixture");
}

#[test]
fn internal_patchset_package_smoke_is_repeated_and_concurrent_without_shared_paths() {
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
fn internal_patchset_stable_smoke_runner_passes() {
    let temp = TempDir::new().unwrap();
    let root = init_repo(&temp);
    let repo = RepoRuntime::discover_from_path(&root).unwrap();

    let payload = run_stable_smoke(&repo, &ait_cli_executable()).unwrap();
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
fn internal_patchset_tg1_runner_runs_native_case_ids() {
    let temp = TempDir::new().unwrap();
    let root = init_repo(&temp);
    let repo = RepoRuntime::discover_from_path(&root).unwrap();

    let payload =
        run_tg1_required(&repo, &["26".to_string()], None, &ait_cli_executable()).unwrap();
    assert_eq!(payload["status"].as_str(), Some("pass"));
    assert_eq!(payload["rust_only"].as_bool(), Some(true));
    assert_eq!(payload["case_indices"][0].as_i64(), Some(26));
}

#[test]
fn internal_patchset_tg1_runner_accepts_current_contract_corpus_node_ids() {
    let temp = TempDir::new().unwrap();
    let root = init_repo(&temp);
    let repo = RepoRuntime::discover_from_path(&root).unwrap();

    let payload = run_tg1_required(
        &repo,
        &["corpora/ait/full_repo/tests/cli/test_land_workflow.py::test_remote_land_excludes_non_docs_root_markdown".to_string()],
        None,
        &ait_cli_executable(),
    )
    .unwrap();
    assert_eq!(payload["status"].as_str(), Some("pass"));
    assert_eq!(payload["case_indices"][0].as_i64(), Some(15));
}
