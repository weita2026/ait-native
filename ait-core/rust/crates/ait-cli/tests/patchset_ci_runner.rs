#[path = "support/native_home.rs"]
mod native_home;
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
    let mut c = Command::cargo_bin("ait-cli").unwrap();
    c.env("CODEX_HOME", native_home::path());
    c
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
    assert_eq!(runner["workspace"].as_bool(), Some(true));
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
            "--locked",
            "--all-features",
            "-p",
            "ait-core",
            "-p",
            "ait-cli",
            "-p",
            "ait-agent-core",
            "-p",
            "ait-agent-worker",
            "-p",
            "ait-benchmark",
            "-p",
            "ait-napi",
            "-p",
            "ait-py",
            "--lib",
            "--test",
            "server_source_ownership",
            "--test",
            "patchset_ci_runner",
            "--test",
            "workflow_recovery_cli",
            "--test",
            "snapshot_task_binding_cli",
            "--test",
            "workflow_friction_cli",
            "--test",
            "plan_cli",
            "--test",
            "workflow_package_frozen_candidate_cli",
            "--test",
            "workflow_package_accepted_retry_cli",
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

fn expected_native_ci_test_invocations() -> Vec<Vec<String>> {
    let base = vec![
        "test",
        "--manifest-path",
        "rust/Cargo.toml",
        "--profile",
        "ait-ci",
    ];
    let mut libraries = base.clone();
    libraries.extend(["--locked", "--all-features"]);
    for package in [
        "ait-core",
        "ait-cli",
        "ait-agent-core",
        "ait-agent-worker",
        "ait-benchmark",
        "ait-napi",
        "ait-py",
    ] {
        libraries.extend(["-p", package]);
    }
    libraries.push("--lib");
    let mut integrations = base.clone();
    integrations.extend([
        "--locked",
        "--all-features",
        "-p",
        "ait-core",
        "-p",
        "ait-cli",
    ]);
    let mut targets = Vec::new();
    for target in [
        "server_source_ownership",
        "patchset_ci_runner",
        "workflow_recovery_cli",
        "snapshot_task_binding_cli",
        "workflow_friction_cli",
        "plan_cli",
        "workflow_package_frozen_candidate_cli",
        "workflow_package_accepted_retry_cli",
    ] {
        targets.extend(["--test", target]);
    }
    integrations.extend(targets.clone());
    let mut build = libraries.clone();
    build.extend(targets);
    build.extend(["--test", "team_session_cli", "--no-run"]);
    libraries.push("--no-fail-fast");
    integrations.push("--no-fail-fast");
    let mut commands = vec![build, libraries, integrations];
    for case in [
        "concurrent_creation_then_retry_does_not_create_duplicate",
        "concurrent_missing_recovery_then_retry_does_not_create_duplicate",
    ] {
        let mut team = base.clone();
        team.extend([
            "--locked",
            "--all-features",
            "-p",
            "ait-cli",
            "--test",
            "team_session_cli",
            case,
            "--",
            "--exact",
        ]);
        commands.push(team);
    }
    let mut schema = base.clone();
    schema.extend([
        "--locked",
        "-p",
        "ait-core",
        "--test",
        "binary_db_schema_authority",
    ]);
    let mut schema_build = schema.clone();
    schema_build.push("--no-run");
    commands.extend([schema_build, schema]);
    let mut repo = base;
    repo.extend(["--workspace", "--all-targets", "--all-features", "--locked"]);
    let mut repo_build = repo.clone();
    repo_build.push("--no-run");
    repo.push("--no-fail-fast");
    commands.extend([repo_build, repo]);
    commands
        .into_iter()
        .map(|arguments| arguments.into_iter().map(str::to_owned).collect())
        .collect()
}

fn shell_cargo_test_invocations(script: &str) -> Vec<Vec<String>> {
    script
        .replace("\\\n", " ")
        .lines()
        .filter_map(|line| line.split_once(" cargo ").map(|(_, arguments)| arguments))
        .filter(|line| line.starts_with("test "))
        .map(|line| line.split_whitespace().map(str::to_owned).collect())
        .collect()
}

fn powershell_cargo_test_invocations(script: &str) -> Vec<Vec<String>> {
    script
        .split("\"cargo\" @(")
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

#[cfg(unix)]
mod owned_ci_cleanup {
    use super::*;
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    struct Fixture {
        temp: TempDir,
        bin: PathBuf,
        external: PathBuf,
        record: PathBuf,
        cargo_log: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let uid = Command::new("id").arg("-u").output().unwrap();
            assert!(uid.status.success());
            assert_ne!(
                String::from_utf8_lossy(&uid.stdout).trim(),
                "0",
                "cleanup acceptance requires a non-root process"
            );
            let temp = TempDir::new().unwrap();
            let bin = temp.path().join("bin");
            fs::create_dir(&bin).unwrap();
            let external = temp.path().join("external seed");
            fs::create_dir_all(external.join("nested")).unwrap();
            fs::write(external.join("nested/marker"), b"external seed bytes\n").unwrap();
            fs::set_permissions(
                external.join("nested/marker"),
                fs::Permissions::from_mode(0o444),
            )
            .unwrap();
            for path in [&external, &external.join("nested")] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o555)).unwrap();
            }
            let fixture = Self {
                record: temp.path().join("owned-root"),
                cargo_log: temp.path().join("cargo-calls"),
                temp,
                bin,
                external,
            };
            fixture.executable(
                "cargo",
                r#"#!/bin/sh
set -eu
stage=$1
case " $* " in
  *" --no-run "*) stage=build ;;
esac
printf '%s\n' "$stage" >> "$CI_CLEANUP_CARGO_LOG"
prepare_readonly_tree() {
  printf '%s\n' "${TMPDIR%/tmp}" > "$CI_CLEANUP_RECORD"
  mkdir -p "$TMPDIR/main-seed/nested"
  printf 'readonly fixture\n' > "$TMPDIR/main-seed/nested/marker"
  /bin/chmod 0444 "$TMPDIR/main-seed/nested/marker"
  ln -s "$CI_CLEANUP_EXTERNAL" "$TMPDIR/external-link"
  /bin/chmod 0555 "$TMPDIR/main-seed/nested" "$TMPDIR/main-seed" "$TMPDIR" "${TMPDIR%/tmp}"
}
if [ "$stage" = fmt ] && [ -n "${CI_CLEANUP_SIGNAL_READY:-}" ]; then
  prepare_readonly_tree
  : > "$CI_CLEANUP_SIGNAL_READY"
  while :; do sleep 10; done
fi
if [ "$stage" = fmt ] && [ -n "${CI_CLEANUP_FORMAT_STATUS:-}" ]; then
  prepare_readonly_tree
  exit "$CI_CLEANUP_FORMAT_STATUS"
fi
# Compilation creates no readonly runtime fixture; execution owns that setup.
if [ "$stage" = test ]; then
  prepare_readonly_tree
  exit "$CI_CLEANUP_TEST_STATUS"
fi
"#,
            );
            fixture
        }

        fn executable(&self, name: &str, contents: &str) {
            let path = self.bin.join(name);
            fs::write(&path, contents).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        fn command(&self, supplied_root: Option<&Path>, test_status: u8) -> Command {
            let mut paths = vec![self.bin.clone()];
            paths.extend(env::split_paths(&env::var_os("PATH").unwrap()));
            let mut command = Command::new("sh");
            command
                .arg(repo_root_for_patch_ci_manifest().join("ci/run.sh"))
                .arg("all")
                .env("PATH", env::join_paths(paths).unwrap())
                .env("TMPDIR", self.temp.path())
                .env("CI_CLEANUP_RECORD", &self.record)
                .env("CI_CLEANUP_CARGO_LOG", &self.cargo_log)
                .env("CI_CLEANUP_EXTERNAL", &self.external)
                .env("CI_CLEANUP_TEST_STATUS", test_status.to_string())
                .env_remove("AIT_RUNNER_ATTEMPT_ROOT");
            if let Some(root) = supplied_root {
                command.env("AIT_RUNNER_ATTEMPT_ROOT", root);
            }
            command
        }

        fn run(&self, supplied_root: Option<&Path>, test_status: u8) -> std::process::Output {
            self.command(supplied_root, test_status).output().unwrap()
        }

        fn terminated(&self, supplied_root: Option<&Path>) -> std::process::Output {
            let ready = self.temp.path().join("signal-ready");
            let stdout = self.temp.path().join("signal-stdout");
            let stderr = self.temp.path().join("signal-stderr");
            let mut child = self
                .command(supplied_root, 0)
                .env("CI_CLEANUP_SIGNAL_READY", &ready)
                .stdout(Stdio::from(fs::File::create(&stdout).unwrap()))
                .stderr(Stdio::from(fs::File::create(&stderr).unwrap()))
                .process_group(0)
                .spawn()
                .unwrap();
            let group = format!("-{}", child.id());
            let deadline = Instant::now() + Duration::from_secs(10);
            while !ready.is_file() {
                assert!(
                    child.try_wait().unwrap().is_none(),
                    "driver exited before signal readiness"
                );
                if Instant::now() >= deadline {
                    let _ = Command::new("kill")
                        .args(["-s", "KILL", "--", &group])
                        .status();
                    let _ = child.wait();
                    panic!("driver signal fixture did not become ready");
                }
                thread::sleep(Duration::from_millis(20));
            }
            assert!(Command::new("kill")
                .args(["-s", "TERM", "--", &group])
                .status()
                .unwrap()
                .success());
            let deadline = Instant::now() + Duration::from_secs(10);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() >= deadline {
                    let _ = Command::new("kill")
                        .args(["-s", "KILL", "--", &group])
                        .status();
                    let _ = child.wait();
                    panic!("cancelled driver did not exit");
                }
                thread::sleep(Duration::from_millis(20));
            };
            std::process::Output {
                status,
                stdout: fs::read(stdout).unwrap(),
                stderr: fs::read(stderr).unwrap(),
            }
        }

        fn owned_root(&self) -> PathBuf {
            PathBuf::from(fs::read_to_string(&self.record).unwrap().trim())
        }

        fn assert_external_unchanged(&self) {
            assert_eq!(
                fs::read(self.external.join("nested/marker")).unwrap(),
                b"external seed bytes\n"
            );
            assert_eq!(
                fs::metadata(&self.external).unwrap().permissions().mode() & 0o777,
                0o555
            );
            assert_eq!(
                fs::metadata(self.external.join("nested"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o555
            );
            assert_eq!(
                fs::metadata(self.external.join("nested/marker"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o444
            );
        }

        fn restore_owned_directories(path: &Path) {
            let metadata = fs::symlink_metadata(path).unwrap();
            if !metadata.is_dir() {
                return;
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            for entry in fs::read_dir(path).unwrap() {
                Self::restore_owned_directories(&entry.unwrap().path());
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            Self::restore_owned_directories(self.temp.path());
        }
    }

    #[test]
    fn self_owned_readonly_tree_is_removed_without_touching_external_symlink_target() {
        let fixture = Fixture::new();
        let output = fixture.run(None, 0);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let root = fixture.owned_root();
        assert_eq!(
            root.parent().unwrap(),
            fs::canonicalize(fixture.temp.path()).unwrap()
        );
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("ait-core-ci."));
        assert!(!root.exists());
        assert_eq!(
            fs::read_to_string(&fixture.cargo_log).unwrap(),
            "fmt\nbuild\ntest\nclippy\n"
        );
        fixture.assert_external_unchanged();
    }

    #[test]
    fn supplied_root_permissions_and_cleanup_remain_with_outer_owner() {
        let fixture = Fixture::new();
        let root = fixture.temp.path().join("runner-owned");
        let output = fixture.run(Some(&root), 0);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(root.exists());
        for path in [&root, &root.join("tmp"), &root.join("tmp/main-seed/nested")] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o555
            );
        }
        assert_eq!(
            fs::read(root.join("tmp/main-seed/nested/marker")).unwrap(),
            b"readonly fixture\n"
        );
        assert!(fs::symlink_metadata(root.join("tmp/external-link"))
            .unwrap()
            .file_type()
            .is_symlink());
        fixture.assert_external_unchanged();
        Fixture::restore_owned_directories(&root);
        fs::remove_dir_all(&root).unwrap();
        assert!(!root.exists());
        fixture.assert_external_unchanged();
    }

    #[test]
    fn failed_tests_stay_failed_after_successful_self_owned_cleanup() {
        let fixture = Fixture::new();
        let output = fixture.run(None, 23);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("CI FAIL repo-tests (exit 23)"));
        assert!(!fixture.owned_root().exists());
        assert_eq!(
            fs::read_to_string(&fixture.cargo_log).unwrap(),
            "fmt\nbuild\ntest\nclippy\n"
        );
        fixture.assert_external_unchanged();
    }

    #[test]
    fn failed_cleanup_remains_failed_after_successful_checks() {
        for command in ["chmod", "rm"] {
            let fixture = Fixture::new();
            fixture.executable(
                command,
                "#!/bin/sh\nprintf 'injected cleanup failure\\n' >&2\nexit 42\n",
            );
            let output = fixture.run(None, 0);
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("injected cleanup failure"));
            assert!(fixture.owned_root().exists());
            assert_eq!(
                fs::read_to_string(&fixture.cargo_log).unwrap(),
                "fmt\nbuild\ntest\nclippy\n"
            );
            fixture.assert_external_unchanged();
        }
    }
    #[test]
    fn termination_during_format_stops_later_stages_and_respects_root_ownership() {
        for supplied in [false, true] {
            let fixture = Fixture::new();
            let root = fixture.temp.path().join("runner-owned");
            let output = fixture.terminated(supplied.then_some(root.as_path()));
            assert_eq!(output.status.code(), Some(143));
            assert_eq!(fs::read_to_string(&fixture.cargo_log).unwrap(), "fmt\n");
            assert!(!String::from_utf8_lossy(&output.stdout).contains("CI PASS"));
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("CI BLOCKED remaining Cargo stages: cancelled"));
            assert_eq!(fixture.owned_root().exists(), supplied);
            if supplied {
                assert_eq!(
                    fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                    0o555
                );
                assert_eq!(
                    fs::read(root.join("tmp/main-seed/nested/marker")).unwrap(),
                    b"readonly fixture\n"
                );
                assert!(fs::symlink_metadata(root.join("tmp/external-link"))
                    .unwrap()
                    .file_type()
                    .is_symlink());
            }
            fixture.assert_external_unchanged();
        }
    }

    #[test]
    fn signal_exit_from_cargo_stops_later_stages() {
        let fixture = Fixture::new();
        let output = fixture
            .command(None, 0)
            .env("CI_CLEANUP_FORMAT_STATUS", "143")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(143));
        assert_eq!(fs::read_to_string(&fixture.cargo_log).unwrap(), "fmt\n");
        assert!(!fixture.owned_root().exists());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("CI FAIL format (exit 143)"));
        assert!(stderr.contains("CI BLOCKED remaining Cargo stages: cancelled"));
        fixture.assert_external_unchanged();
    }
}

#[test]
fn repo_native_ci_entrypoints_cover_workspace_and_guard_lineage_schema() {
    let root = repo_root_for_patch_ci_manifest();
    let unix = fs::read_to_string(root.join("ci/run.sh")).unwrap();
    let windows = fs::read_to_string(root.join("ci/run.ps1")).unwrap();
    let schema_authority_test =
        fs::read_to_string(root.join("rust/crates/ait-core/tests/binary_db_schema_authority.rs"))
            .unwrap();
    let expected = expected_native_ci_test_invocations();

    assert!(
        !schema_authority_test.contains("include_bytes!"),
        "Snapshot-runnable all-targets CI must not require lineage-only Markdown at compile time"
    );
    for required in [
        "std::fs::read(&path)",
        "ErrorKind::NotFound",
        "binary_db_v0_bytes_or_skip!()",
    ] {
        assert!(
            schema_authority_test.contains(required),
            "schema authority test is missing the bounded runtime lineage guard `{required}`"
        );
    }

    assert_eq!(
        shell_cargo_test_invocations(&unix),
        expected,
        "Unix CI must cover every Snapshot-runnable harness, the complete repository, and the conditional local schema authority harness"
    );
    assert_eq!(
        powershell_cargo_test_invocations(&windows),
        expected,
        "Windows CI must cover every Snapshot-runnable harness, the complete repository, and the conditional local schema authority harness"
    );

    let expected_schema_invocation = expected[5..7].to_vec();
    let unix_schema_guard = script_region_between(
        &unix,
        "if [ -f \"$repo_root/docs/binary_db_v0.md\" ]; then",
        "\n  fi",
    );
    assert_eq!(
        shell_cargo_test_invocations(unix_schema_guard),
        expected_schema_invocation,
        "Unix must execute the schema-authority harness only while its lineage-only Markdown source exists"
    );
    let windows_schema_guard = script_region_between(
        &windows,
        "if (Test-Path -LiteralPath (Join-Path $repoRoot \"docs/binary_db_v0.md\") -PathType Leaf) {",
        "\n        } else {",
    );
    assert_eq!(
        powershell_cargo_test_invocations(windows_schema_guard),
        expected[5..7].to_vec(),
        "Windows must execute the schema-authority harness only while its lineage-only Markdown source exists"
    );

    assert!(
        unix.contains("find \"$owned_root\" -type d -exec chmod u+rwx {} +"),
        "exclusive-root cleanup must retain physical directory permission recovery"
    );
    for script in [&unix, &windows] {
        assert!(
            script.contains("--workspace")
                && script.contains("--all-targets")
                && script.contains("--all-features")
                && script.contains("--locked"),
            "repo/all modes must cover the complete locked Rust workspace"
        );
        assert!(
            script.contains("clippy") && script.contains("warnings"),
            "all mode must enforce a warning-free clippy baseline"
        );
    }
    assert!(
        unix.contains("repo)\n    run_repo_tests")
            && unix.contains("all)\n    run_repo_tests\n    run_clippy"),
        "Unix repo and all mode routing must remain distinct"
    );
    assert!(
        windows.contains("\"repo\" { Invoke-RepoTests }")
            && windows
                .contains("\"all\" {\n            Invoke-RepoTests\n            Invoke-Clippy"),
        "Windows repo and all mode routing must remain distinct"
    );
}

#[test]
fn repository_quality_workflow_and_binding_metadata_are_release_complete() {
    let root = repo_root_for_patch_ci_manifest();
    let workflow =
        fs::read_to_string(root.join(".github/workflows/ait-agent-worker-platforms.yml")).unwrap();
    for required in [
        "cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked -- -D warnings",
        "cargo test --manifest-path rust/Cargo.toml --workspace --all-targets --all-features --locked",
        "rustsec/audit-check@v2.0.0",
        "working-directory: rust",
    ] {
        assert!(
            workflow.contains(required),
            "repository workflow is missing required contract `{required}`"
        );
    }

    let py_manifest = fs::read_to_string(root.join("rust/crates/ait-py/Cargo.toml")).unwrap();
    let napi_manifest = fs::read_to_string(root.join("rust/crates/ait-napi/Cargo.toml")).unwrap();
    for (crate_name, manifest) in [("ait-py", py_manifest), ("ait-napi", napi_manifest)] {
        assert!(
            manifest
                .lines()
                .any(|line| line.trim() == "version.workspace = true"),
            "{crate_name} must inherit the workspace release version"
        );
        assert!(
            manifest
                .lines()
                .any(|line| line.trim() == "license.workspace = true"),
            "{crate_name} must inherit the workspace license"
        );
    }

    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|line| line.trim() == "error.log"),
        "local runtime diagnostics must remain excluded from raw Git tooling"
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

// These small Rust workspaces exercise the real drivers and Cargo target
// continuation. They validate collection control, not product/binding behavior.
struct CiControlFixture {
    _temp: TempDir,
    root: PathBuf,
    attempt: PathBuf,
    markers: PathBuf,
}

impl CiControlFixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("driver fixture with spaces");
        let attempt = temp.path().join("owned attempt");
        let markers = temp.path().join("executed.txt");
        fs::create_dir_all(root.join("ci")).unwrap();
        for name in ["run.sh", "run.ps1"] {
            fs::copy(
                repo_root_for_patch_ci_manifest().join("ci").join(name),
                root.join("ci").join(name),
            )
            .unwrap();
        }
        let packages = [
            "ait-core",
            "ait-cli",
            "ait-agent-core",
            "ait-agent-worker",
            "ait-benchmark",
            "ait-napi",
            "ait-py",
        ];
        fs::create_dir_all(root.join("rust")).unwrap();
        fs::write(root.join("rust/Cargo.toml"), format!(
            "[workspace]\nresolver = \"2\"\nmembers = {:?}\n[profile.ait-ci]\ninherits = \"dev\"\ndebug = 0\n", packages
        )).unwrap();
        for package in packages {
            let crate_root = root.join("rust").join(package);
            fs::create_dir_all(crate_root.join("src")).unwrap();
            fs::write(
                crate_root.join("Cargo.toml"),
                format!("[package]\nname = {package:?}\nversion = \"0.0.0\"\nedition = \"2021\"\n"),
            )
            .unwrap();
            fs::write(crate_root.join("src/lib.rs"), format!("#[cfg(test)]\nmod tests {{\n{}\n#[test] fn inventory() {{ control(\"lib\"); }}\n}}\n", ci_control_test_body())).unwrap();
        }
        for target in [
            "server_source_ownership",
            "patchset_ci_runner",
            "workflow_recovery_cli",
            "snapshot_task_binding_cli",
            "workflow_friction_cli",
            "plan_cli",
            "workflow_package_frozen_candidate_cli",
            "workflow_package_accepted_retry_cli",
            "team_session_cli",
        ] {
            let tests = root.join("rust/ait-cli/tests");
            fs::create_dir_all(&tests).unwrap();
            let functions = if target == "team_session_cli" {
                "#[test] fn concurrent_creation_then_retry_does_not_create_duplicate() { control(\"team-create\"); }\n#[test] fn concurrent_missing_recovery_then_retry_does_not_create_duplicate() { control(\"team-recovery\"); }".to_string()
            } else {
                format!("#[test] fn inventory() {{ control({target:?}); }}")
            };
            fs::write(
                tests.join(format!("{target}.rs")),
                format!("{}\n{functions}\n", ci_control_test_body()),
            )
            .unwrap();
        }
        fs::create_dir_all(root.join("rust/ait-core/tests")).unwrap();
        // Copy the protected source unchanged; this synthetic harness only
        // records the conditional invocation, not a schema-validation result.
        let authority = repo_root_for_patch_ci_manifest().join("docs/binary_db_v0.md");
        if authority.is_file() {
            fs::create_dir_all(root.join("docs")).unwrap();
            fs::copy(authority, root.join("docs/binary_db_v0.md")).unwrap();
        }
        fs::write(
            root.join("rust/ait-core/tests/binary_db_schema_authority.rs"),
            format!(
                "{}\n#[test] fn inventory() {{ control(\"schema\"); }}\n",
                ci_control_test_body()
            ),
        )
        .unwrap();
        let fixture = Self {
            _temp: temp,
            root,
            attempt,
            markers,
        };
        for arguments in [vec!["generate-lockfile", "--offline"], vec!["fmt", "--all"]] {
            let output = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
                .current_dir(fixture.root.join("rust"))
                .env(
                    "CARGO_TARGET_DIR",
                    fixture._temp.path().join("prepare-target"),
                )
                .env(
                    "CARGO_BUILD_BUILD_DIR",
                    fixture._temp.path().join("prepare-build"),
                )
                .args(arguments)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "fixture preparation: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fixture
    }

    fn run(&self, powershell: bool, mode: &str, failure: &str) -> std::process::Output {
        let mut command = if powershell {
            let program = env::var_os("AIT_CI_TEST_POWERSHELL").unwrap_or_else(|| "pwsh".into());
            let mut command = Command::new(program);
            command
                .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
                .arg(self.root.join("ci/run.ps1"))
                .args(["-Mode", mode]);
            command
        } else {
            let mut command = Command::new("bash");
            command.arg(self.root.join("ci/run.sh")).arg(mode);
            command
        };
        command
            .current_dir(&self.root)
            .env("AIT_RUNNER_ATTEMPT_ROOT", &self.attempt)
            .env("AIT_CI_CONTROL_MARKERS", &self.markers)
            .env("AIT_CI_CONTROL_FAILURE", failure)
            .env("PYO3_BUILD_EXTENSION_MODULE", "inherited-negative-control")
            .env("CARGO_INCREMENTAL", "0")
            .output()
            .unwrap()
    }

    fn executed(&self) -> BTreeSet<String> {
        fs::read_to_string(&self.markers)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn ci_control_test_body() -> &'static str {
    r#"fn control(target: &str) {
    use std::io::Write;
    assert!(std::env::var_os("PYO3_BUILD_EXTENSION_MODULE").is_none());
    let label = format!("{}/{}", env!("CARGO_PKG_NAME"), target);
    let path = std::env::var_os("AIT_CI_CONTROL_MARKERS").unwrap();
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    file.write_all(format!("{label}\n").as_bytes()).unwrap();
    let failure = std::env::var("AIT_CI_CONTROL_FAILURE").unwrap_or_default();
    if failure == "targets" && (label == "ait-agent-core/lib" || label == "ait-cli/server_source_ownership") {
        println!("CONTROL stdout {label}");
        eprintln!("CONTROL stderr {label}");
        panic!("injected ordinary target failure {label}");
    }
}"#
}

fn exercise_ci_driver_collection(powershell: bool) {
    let mut failures = Vec::new();
    for case in [
        "pass",
        "repo-pass",
        "format",
        "targets",
        "compile",
        "all-targets",
        "all-compile",
        "unsafe",
        "boundary",
    ] {
        let fixture = CiControlFixture::new();
        if case == "format" {
            let path = fixture.root.join("rust/ait-core/src/lib.rs");
            let mut source = fs::read_to_string(&path).unwrap();
            source.push_str("pub fn unformatted( )->u8{1}\n");
            fs::write(path, source).unwrap();
        }
        if case == "compile" || case == "all-compile" {
            let warm = fixture.run(powershell, "patchset", "");
            if !warm.status.success() {
                failures.push(format!("compile warm-up failed: {warm:?}"));
            }
            let _ = fs::remove_file(&fixture.markers);
            fs::write(
                fixture.root.join("rust/ait-core/src/lib.rs"),
                "compile_error!(\"injected build prerequisite failure\");\n",
            )
            .unwrap();
        }
        if case == "unsafe" {
            fs::write(&fixture.attempt, "owned-file-sentinel").unwrap();
        }
        if case == "boundary" {
            fs::write(fixture.root.join("forbidden.py"), "never execute\n").unwrap();
        }
        let mode = if case == "all-targets" || case == "all-compile" {
            "all"
        } else if case == "repo-pass" {
            "repo"
        } else {
            "patchset"
        };
        let injected = if case == "targets" || case == "all-targets" {
            "targets"
        } else {
            ""
        };
        let output = fixture.run(powershell, mode, injected);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let executed = fixture.executed();
        let mut missing = Vec::new();
        if output.status.success() != matches!(case, "pass" | "repo-pass") {
            missing.push("strict aggregate exit".to_string());
        }
        if ["pass", "repo-pass", "format", "targets", "all-targets"].contains(&case) {
            for package in [
                "ait-core",
                "ait-cli",
                "ait-agent-core",
                "ait-agent-worker",
                "ait-benchmark",
                "ait-napi",
                "ait-py",
            ] {
                if !executed.contains(&format!("{package}/lib")) {
                    missing.push(format!("library {package}"));
                }
            }
            for target in [
                "server_source_ownership",
                "patchset_ci_runner",
                "workflow_recovery_cli",
                "snapshot_task_binding_cli",
                "workflow_friction_cli",
                "plan_cli",
                "workflow_package_frozen_candidate_cli",
                "workflow_package_accepted_retry_cli",
                "team-create",
                "team-recovery",
            ] {
                if !executed.contains(&format!("ait-cli/{target}")) {
                    missing.push(format!("integration {target}"));
                }
            }
            if fixture.root.join("docs/binary_db_v0.md").is_file()
                && !executed.contains("ait-core/schema")
            {
                missing.push("conditional lineage harness".into());
            }
        } else {
            if !executed.is_empty() {
                missing.push("dependent binaries ran despite blocked prerequisites".into());
            }
            if !stdout.contains("BLOCKED") && !stderr.contains("BLOCKED") {
                missing.push("explicit BLOCKED inventory".into());
            }
        }
        if (case == "all-targets" || case == "all-compile") && !stdout.contains("clippy") {
            missing.push("independent clippy after target failure".into());
        }
        if case == "targets"
            && (!stdout.contains("CONTROL stdout") || !stdout.contains("CONTROL stderr"))
        {
            missing.push("original failed-target streams".into());
        }
        if case == "unsafe"
            && fs::read_to_string(&fixture.attempt).unwrap() != "owned-file-sentinel"
        {
            missing.push("filesystem sentinel changed".into());
        }
        if !missing.is_empty() {
            failures.push(format!("case={case} missing={missing:?}\nstatus={:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", output.status));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[cfg(unix)]
#[test]
fn unix_ci_driver_collects_failures_and_blocks_unsafe_dependencies() {
    exercise_ci_driver_collection(false);
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "requires an explicit PowerShell runtime; Windows runs this normally"
)]
fn powershell_ci_driver_collects_failures_and_blocks_unsafe_dependencies() {
    exercise_ci_driver_collection(true);
}
