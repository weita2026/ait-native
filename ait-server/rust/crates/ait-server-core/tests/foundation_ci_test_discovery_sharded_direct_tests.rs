use ait_server_core::foundation::ci_test_discovery_sharded::ci_test_discovery_sharded_run_json;
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    env::temp_dir().join(format!(
        "ait-server-ci-test-discovery-{label}-{}-{nanos}",
        std::process::id()
    ))
}

#[cfg(unix)]
fn make_executable(path: &PathBuf) {
    let mut permissions = fs::metadata(path)
        .expect("script metadata should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("script should be executable");
}

#[cfg(not(unix))]
fn make_executable(_path: &PathBuf) {}

fn write_script(path: &PathBuf, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("script parent should exist");
    }
    fs::write(path, body).expect("script should be written");
    make_executable(path);
}

fn run(payload: JsonValue) -> JsonValue {
    ci_test_discovery_sharded_run_json(&payload).expect("discovery sharded runner should run")
}

#[test]
fn cargo_adapter_builds_once_and_shards_discovered_test_cases() {
    let root = temp_root("cargo-shards");
    let workspace = root.join("workspace");
    let rust_dir = workspace.join("rust");
    let output = root.join("output");
    let bin_dir = root.join("bin");
    fs::create_dir_all(&rust_dir).expect("rust dir should exist");
    fs::write(rust_dir.join("Cargo.toml"), "[workspace]\n").expect("manifest should exist");
    fs::write(workspace.join("README.md"), "test\n").expect("readme should exist");

    let marker = root.join("shards.txt");
    let fmt_marker = root.join("fmt.txt");
    let exe_a = bin_dir.join("test-a");
    let exe_b = bin_dir.join("test-b");
    let exe_c = bin_dir.join("test-c");
    let ordinary_bin = bin_dir.join("ordinary-bin");
    for (exe, case_a, case_b) in [
        (&exe_a, "alpha::case_one", "alpha::case_two"),
        (&exe_b, "beta::case_one", "beta::case_two"),
        (&exe_c, "gamma::case_one", "gamma::case_two"),
    ] {
        write_script(
            exe,
            r#"#!/bin/sh
case " $* " in
  *" --list "*)
    cat <<'TESTS'
__CASE_A__: test
__CASE_B__: test
TESTS
    exit 0
    ;;
esac
if [ "${RUST_TEST_THREADS}" != "1" ]; then
  echo "expected RUST_TEST_THREADS=1, got ${RUST_TEST_THREADS}" >&2
  exit 21
fi
if [ "${AIT_CI_TEST_SHARDING}" != "test_case" ]; then
  echo "missing sharding marker" >&2
  exit 22
fi
if [ "$1" != "--exact" ]; then
  echo "expected --exact test case execution, got $*" >&2
  exit 23
fi
shift
for case_name in "$@"; do
  echo "${AIT_CI_SHARD_ID}:${case_name}" >> "${MARKER}"
done
exit 0
	"#
            .replace("__CASE_A__", case_a)
            .replace("__CASE_B__", case_b)
            .as_str(),
        );
    }
    write_script(
        &ordinary_bin,
        r#"#!/bin/sh
echo "ordinary bin artifact should not be executed" >&2
exit 91
"#,
    );

    let fake_cargo = bin_dir.join("cargo");
    write_script(
        &fake_cargo,
        &format!(
            r#"#!/bin/sh
case " $* " in
  *" fmt "*)
    echo fmt > "${{FMT_MARKER}}"
    ;;
  *" --no-run "*)
    if [ "${{CARGO_BUILD_JOBS}}" != "3" ]; then
      echo "expected CARGO_BUILD_JOBS=3, got ${{CARGO_BUILD_JOBS}}" >&2
      exit 31
    fi
    case " $* " in
      *" --profile ait-ci "*) ;;
      *)
        echo "expected --profile ait-ci in cargo discovery args" >&2
        exit 33
        ;;
    esac
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":false}},"target":{{"name":"ordinary-bin","kind":["bin"]}},"executable":"{}"}}'
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":true}},"target":{{"name":"test-a","kind":["test"]}},"executable":"{}"}}'
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":true}},"target":{{"name":"test-b","kind":["test"]}},"executable":"{}"}}'
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":true}},"target":{{"name":"test-c","kind":["test"]}},"executable":"{}"}}'
    ;;
  *" --doc "*)
    case " $* " in
      *" --profile ait-ci "*) ;;
      *)
        echo "expected --profile ait-ci in cargo doc test args" >&2
        exit 34
        ;;
    esac
    echo "doc tests pass"
    ;;
  *)
    echo "unexpected fake cargo args: $*" >&2
    exit 32
    ;;
esac
"#,
            ordinary_bin.to_string_lossy(),
            exe_a.to_string_lossy(),
            exe_b.to_string_lossy(),
            exe_c.to_string_lossy()
        ),
    );

    let result = run(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output.to_string_lossy(),
        "suite_id": "rust_core",
        "runner_parallelism": 3,
        "env": {
            "MARKER": marker.to_string_lossy(),
            "FMT_MARKER": fmt_marker.to_string_lossy()
        },
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "cargo",
            "cargo_binary": fake_cargo.to_string_lossy(),
            "manifest_path": "rust/Cargo.toml",
            "workspace": true,
            "doc_tests": true,
            "timeout_seconds": 7,
            "checks": [{
                "kind": "cargo_fmt",
                "check_id": "rustfmt"
            }, {
                "kind": "forbid_files",
                "check_id": "no_python_boundary",
                "file_name_suffix": ".py",
                "exclude_dirs": [".git", ".ait", ".cargo", "target"]
            }]
        }
    }));

    assert_eq!(result["status"], json!("pass"));
    assert_eq!(result["runner"]["kind"], json!("test_discovery_sharded"));
    assert_eq!(result["runner"]["adapter"], json!("cargo"));
    assert_eq!(result["runner"]["shard_by"], json!("test_case"));
    assert_eq!(result["runner"]["timeout_seconds"], json!(7));
    assert_eq!(result["checks"]["reports"][0]["kind"], json!("cargo_fmt"));
    assert_eq!(result["checks"]["reports"][0]["status"], json!("pass"));
    assert_eq!(result["checks"]["reports"][0]["timeout_seconds"], json!(7));
    assert_eq!(
        result["discovery"]["build_report"]["timeout_seconds"],
        json!(7)
    );
    assert_eq!(result["discovery"]["executable_count"], json!(3));
    assert_eq!(result["discovery"]["test_case_count"], json!(6));
    assert_eq!(result["test_shards"]["shard_by"], json!("test_case"));
    assert_eq!(result["test_shards"]["shard_count"], json!(3));
    for shard in result["test_shards"]["shards"].as_array().unwrap() {
        assert_eq!(shard["test_case_count"], json!(2));
    }
    assert_eq!(result["doc_tests"]["status"], json!("pass"));
    assert_eq!(result["diagnostics"]["cargo_compiles_once"], json!(true));
    assert_eq!(result["diagnostics"]["test_case_shards"], json!(true));
    assert!(result["discovery"]["build_report"]["command"]
        .as_str()
        .unwrap_or_default()
        .contains("--profile ait-ci"));
    assert!(result["doc_tests"]["command"]
        .as_str()
        .unwrap_or_default()
        .contains("--profile ait-ci"));
    assert_eq!(result["environment"]["cargo_build_jobs_env"], json!("3"));
    assert_eq!(result["environment"]["rust_test_threads_env"], json!("1"));
    assert_eq!(
        result["environment"]["process_policy"]["policy"],
        json!("safe_ambient_allowlist_with_explicit_overrides")
    );
    let marker_text = fs::read_to_string(&marker).expect("shards should append marker");
    assert_eq!(marker_text.lines().count(), 6);
    assert!(marker_text.contains("shard-0"));
    assert!(marker_text.contains("shard-1"));
    assert!(marker_text.contains("shard-2"));
    assert_eq!(
        fs::read_to_string(&fmt_marker).expect("fmt marker should exist"),
        "fmt\n"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn discovery_sharded_rejects_invalid_timeout_before_cargo_runs() {
    let root = temp_root("invalid-timeout");
    let workspace = root.join("workspace");
    let rust_dir = workspace.join("rust");
    fs::create_dir_all(&rust_dir).expect("rust dir should exist");
    fs::write(rust_dir.join("Cargo.toml"), "[workspace]\n").expect("manifest should exist");

    for invalid in [json!(0), json!(-1), json!(86_401), json!("1")] {
        let error = ci_test_discovery_sharded_run_json(&json!({
            "workspace_path": workspace.to_string_lossy(),
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "cargo",
                "cargo_binary": "/bin/false",
                "manifest_path": "rust/Cargo.toml",
                "timeout_seconds": invalid
            }
        }))
        .expect_err("invalid timeout should fail closed");
        assert!(error.contains("timeout_seconds"), "{error}");
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cargo_adapter_excludes_configured_patchset_only_test_cases() {
    let root = temp_root("cargo-exclude-test-cases");
    let workspace = root.join("workspace");
    let rust_dir = workspace.join("rust");
    let output = root.join("output");
    let bin_dir = root.join("bin");
    fs::create_dir_all(&rust_dir).expect("rust dir should exist");
    fs::write(rust_dir.join("Cargo.toml"), "[workspace]\n").expect("manifest should exist");

    let marker = root.join("executed.txt");
    let test_exe = bin_dir.join("test-a");
    write_script(
        &test_exe,
        r#"#!/bin/sh
case " $* " in
  *" --list "*)
    cat <<'TESTS'
fast::unit_case: test
slow::contract_case: test
TESTS
    exit 0
    ;;
esac
for arg in "$@"; do
  if [ "$arg" = "slow::contract_case" ]; then
    echo "excluded contract case must not run in patchset CI" >&2
    exit 51
  fi
  if [ "$arg" = "fast::unit_case" ]; then
    echo "$arg" >> "${MARKER}"
  fi
done
exit 0
"#,
    );

    let fake_cargo = bin_dir.join("cargo");
    write_script(
        &fake_cargo,
        &format!(
            r#"#!/bin/sh
case " $* " in
  *" --no-run "*)
    echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":true}},"target":{{"name":"test-a","kind":["test"]}},"executable":"{}"}}'
    ;;
  *)
    echo "unexpected fake cargo args: $*" >&2
    exit 52
    ;;
esac
"#,
            test_exe.to_string_lossy()
        ),
    );

    let result = run(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output.to_string_lossy(),
        "runner_parallelism": 2,
        "env": {
            "MARKER": marker.to_string_lossy()
        },
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "cargo",
            "cargo_binary": fake_cargo.to_string_lossy(),
            "manifest_path": "rust/Cargo.toml",
            "exclude_test_cases": ["slow::contract_case"]
        }
    }));

    assert_eq!(result["status"], json!("pass"));
    assert_eq!(result["discovery"]["test_case_count"], json!(1));
    assert_eq!(result["discovery"]["excluded_test_case_count"], json!(1));
    assert_eq!(
        result["discovery"]["excluded_test_cases"],
        json!(["slow::contract_case"])
    );
    assert_eq!(
        result["discovery"]["test_case_discovery"]["reports"][0]["excluded_test_cases"],
        json!([{"kind": "test", "name": "slow::contract_case"}])
    );
    assert_eq!(result["test_shards"]["test_case_count"], json!(1));
    assert_eq!(
        fs::read_to_string(&marker).expect("fast case should run"),
        "fast::unit_case\n"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cargo_adapter_rejects_stale_excluded_test_case_names() {
    let root = temp_root("cargo-stale-exclude-test-case");
    let workspace = root.join("workspace");
    let rust_dir = workspace.join("rust");
    let output = root.join("output");
    let bin_dir = root.join("bin");
    fs::create_dir_all(&rust_dir).expect("rust dir should exist");
    fs::write(rust_dir.join("Cargo.toml"), "[workspace]\n").expect("manifest should exist");

    let test_exe = bin_dir.join("test-a");
    write_script(
        &test_exe,
        r#"#!/bin/sh
case " $* " in
  *" --list "*)
    echo "current::contract_case: test"
    exit 0
    ;;
esac
exit 0
"#,
    );
    let fake_cargo = bin_dir.join("cargo");
    write_script(
        &fake_cargo,
        &format!(
            r#"#!/bin/sh
echo '{{"reason":"compiler-artifact","package_id":"pkg 0.1.0","profile":{{"test":true}},"target":{{"name":"test-a","kind":["test"]}},"executable":"{}"}}'
"#,
            test_exe.to_string_lossy()
        ),
    );

    let error = ci_test_discovery_sharded_run_json(&json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output.to_string_lossy(),
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "cargo",
            "cargo_binary": fake_cargo.to_string_lossy(),
            "manifest_path": "rust/Cargo.toml",
            "exclude_test_cases": ["stale::contract_case"]
        }
    }))
    .expect_err("stale exclusion must fail closed");

    assert!(error.contains("stale::contract_case"));
    assert!(error.contains("did not match discovered tests"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn forbidden_file_check_fails_before_cargo_discovery_runs() {
    let root = temp_root("forbid-before-cargo");
    let workspace = root.join("workspace");
    let output = root.join("output");
    let bin_dir = root.join("bin");
    fs::create_dir_all(workspace.join("rust")).expect("workspace should exist");
    fs::write(workspace.join("rust/Cargo.toml"), "[workspace]\n").expect("manifest should exist");
    fs::write(workspace.join("bad.py"), "print('no')\n").expect("bad file should exist");
    let cargo_called = root.join("cargo-called");
    let fake_cargo = bin_dir.join("cargo");
    write_script(
        &fake_cargo,
        &format!(
            r#"#!/bin/sh
echo called > "{}"
exit 88
"#,
            cargo_called.to_string_lossy()
        ),
    );

    let result = run(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output.to_string_lossy(),
        "runner_parallelism": 3,
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "cargo",
            "cargo_binary": fake_cargo.to_string_lossy(),
            "manifest_path": "rust/Cargo.toml",
            "checks": [{
                "kind": "forbid_files",
                "check_id": "no_python_boundary",
                "file_name_suffix": ".py",
                "exclude_dirs": [".git", ".ait", ".cargo", "target"]
            }]
        }
    }));

    assert_eq!(result["status"], json!("fail"));
    assert_eq!(result["checks"]["reports"][0]["match_count"], json!(1));
    assert_eq!(result["checks"]["reports"][0]["matches"], json!(["bad.py"]));
    assert!(
        !cargo_called.exists(),
        "cargo discovery must not run after a failed check"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn cargo_fmt_check_fails_before_cargo_discovery_runs() {
    let root = temp_root("fmt-before-cargo");
    let workspace = root.join("workspace");
    let output = root.join("output");
    let bin_dir = root.join("bin");
    fs::create_dir_all(workspace.join("rust")).expect("workspace should exist");
    fs::write(workspace.join("rust/Cargo.toml"), "[workspace]\n").expect("manifest should exist");
    let cargo_called = root.join("cargo-called");
    let fake_cargo = bin_dir.join("cargo");
    write_script(
        &fake_cargo,
        &format!(
            r#"#!/bin/sh
case " $* " in
  *" fmt "*)
    echo "formatting drift" >&2
    exit 44
    ;;
  *" --no-run "*)
    case " $* " in
      *" --profile ait-ci "*) ;;
      *)
        echo "expected --profile ait-ci in cargo discovery args" >&2
        exit 46
        ;;
    esac
    echo discovery > "{}"
    exit 0
    ;;
  *)
    echo "unexpected fake cargo args: $*" >&2
    exit 45
    ;;
esac
"#,
            cargo_called.to_string_lossy()
        ),
    );

    let result = run(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output.to_string_lossy(),
        "runner_parallelism": 3,
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "cargo",
            "cargo_binary": fake_cargo.to_string_lossy(),
            "manifest_path": "rust/Cargo.toml",
            "checks": [{
                "kind": "cargo_fmt",
                "check_id": "rustfmt"
            }]
        }
    }));

    assert_eq!(result["status"], json!("fail"));
    assert_eq!(result["checks"]["reports"][0]["kind"], json!("cargo_fmt"));
    assert_eq!(result["checks"]["reports"][0]["status"], json!("fail"));
    assert_eq!(result["checks"]["reports"][0]["exit_code"], json!(44));
    assert_eq!(result["discovery"]["status"], JsonValue::Null);
    assert!(
        !cargo_called.exists(),
        "cargo discovery must not run after a failed fmt check"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_adapter_discovers_once_and_runs_bounded_json_array_shards() {
    let root = temp_root("command-json-shards");
    let workspace = root.join("workspace");
    let output = root.join("output");
    let bin_dir = root.join("bin");
    let discovery_marker = root.join("discovery.txt");
    let shard_marker = root.join("shards.txt");
    fs::create_dir_all(&workspace).expect("workspace should exist");

    let discovery = bin_dir.join("discover");
    write_script(
        &discovery,
        r#"#!/bin/sh
test "$CUSTOM_LANGUAGE_HOME" = "/workspace/custom" || exit 61
printf 'discover\n' >> "$DISCOVERY_MARKER"
printf '%s\n' '["suite::alpha","suite::beta","suite::gamma","suite::delta","suite::epsilon","suite::zeta"]'
"#,
    );
    let shard_runner = bin_dir.join("run-tests");
    write_script(
        &shard_runner,
        r#"#!/bin/sh
test "$CUSTOM_LANGUAGE_HOME" = "/workspace/custom" || exit 62
test "$AIT_CI_TEST_DISCOVERY_ADAPTER" = "command" || exit 63
test "$AIT_CI_TEST_SHARDING" = "test_case" || exit 64
test "$AIT_SHARD_ID" = "$AIT_CI_SHARD_ID" || exit 65
test "$AIT_CI_TEST_CASE_COUNT" = "2" || exit 66
test "$#" = "2" || exit 67
test -n "$AIT_TEST_ITEMS" || exit 68
test -n "$AIT_TEST_ITEMS_JSON" || exit 69
test "$AIT_SHARD_REPO_DIR" = "$AIT_REPO_ROOT" || exit 70
test -d "$AIT_SHARD_OUTPUT_DIR" || exit 71
printf '%s|%s|%s\n' "$AIT_CI_SHARD_ID" "$1" "$2" >> "$SHARD_MARKER"
"#,
    );

    let result = run(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output.to_string_lossy(),
        "runner_parallelism": 3,
        "env": {
            "CUSTOM_LANGUAGE_HOME": "/workspace/custom",
            "DISCOVERY_MARKER": discovery_marker.to_string_lossy(),
            "SHARD_MARKER": shard_marker.to_string_lossy()
        },
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "command",
            "discovery_program": discovery.to_string_lossy(),
            "discovery_args": [],
            "discovery_output_format": "json_array",
            "run_program": shard_runner.to_string_lossy(),
            "run_args": [],
            "append_test_items": true,
            "timeout_seconds": 7
        }
    }));

    assert_eq!(result["status"], json!("pass"));
    assert_eq!(result["runner"]["adapter"], json!("command"));
    assert_eq!(result["runner"]["discovery_phase"], json!("command_once"));
    assert_eq!(result["runner"]["append_test_items"], json!(true));
    assert_eq!(result["discovery"]["executable_count"], json!(0));
    assert_eq!(result["discovery"]["test_case_count"], json!(6));
    assert_eq!(
        result["discovery"]["test_case_discovery"]["status"],
        json!("pass")
    );
    assert_eq!(result["test_shards"]["shard_count"], json!(3));
    assert_eq!(result["test_shards"]["test_case_count"], json!(6));
    for shard in result["test_shards"]["shards"].as_array().unwrap() {
        assert_eq!(shard["status"], json!("pass"));
        assert_eq!(shard["test_case_count"], json!(2));
        assert_eq!(shard["reports"][0]["timeout_seconds"], json!(7));
    }
    assert_eq!(
        result["environment"]["process_policy"]["policy"],
        json!("safe_ambient_allowlist_with_explicit_overrides")
    );
    assert_eq!(
        result["diagnostics"]["language_neutral_command_adapter"],
        json!(true)
    );
    assert_eq!(
        fs::read_to_string(&discovery_marker).expect("discovery marker should exist"),
        "discover\n",
        "the command adapter must discover exactly once"
    );
    let mut shard_lines = fs::read_to_string(&shard_marker)
        .expect("shards should append marker")
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    shard_lines.sort();
    assert_eq!(
        shard_lines,
        vec![
            "shard-0|suite::alpha|suite::delta",
            "shard-1|suite::beta|suite::epsilon",
            "shard-2|suite::gamma|suite::zeta",
        ]
    );
    assert_eq!(result["artifacts"]["summary_json"]["exists"], json!(true));
    assert_eq!(result["artifacts"]["log_path"]["exists"], json!(true));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_adapter_accepts_lines_and_applies_exact_exclusions() {
    let root = temp_root("command-lines");
    let workspace = root.join("workspace");
    let output = root.join("output");
    let bin_dir = root.join("bin");
    let shard_marker = root.join("shards.txt");
    let working_directory = workspace.join("app");
    fs::create_dir_all(&working_directory).expect("working directory should exist");
    let expected_working_directory =
        fs::canonicalize(&working_directory).expect("working directory should canonicalize");

    let discovery = bin_dir.join("discover");
    write_script(
        &discovery,
        r#"#!/bin/sh
test "$PWD" = "$EXPECTED_WORKING_DIRECTORY" || exit 72
printf 'case-a\nskip-me\ncase-b\n'
"#,
    );
    let shard_runner = bin_dir.join("run-tests");
    write_script(
        &shard_runner,
        r#"#!/bin/sh
test "$#" = "0" || exit 71
test "$PWD" = "$EXPECTED_WORKING_DIRECTORY" || exit 72
printf '%s|%s\n' "$AIT_CI_SHARD_ID" "$AIT_TEST_ITEMS_JSON" >> "$SHARD_MARKER"
"#,
    );

    let result = run(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output.to_string_lossy(),
        "runner_parallelism": 2,
        "env": {
            "SHARD_MARKER": shard_marker.to_string_lossy(),
            "EXPECTED_WORKING_DIRECTORY": expected_working_directory.to_string_lossy()
        },
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "command",
            "discovery_program": discovery.to_string_lossy(),
            "discovery_output_format": "lines",
            "run_program": shard_runner.to_string_lossy(),
            "append_test_items": false,
            "working_directory": "app",
            "exclude_test_cases": ["skip-me"]
        }
    }));

    assert_eq!(result["status"], json!("pass"));
    assert_eq!(
        result["discovery"]["excluded_test_cases"],
        json!(["skip-me"])
    );
    assert_eq!(result["discovery"]["test_case_count"], json!(2));
    assert_eq!(result["test_shards"]["shard_count"], json!(2));
    assert_eq!(result["runner"]["working_directory"], json!("app"));
    let marker = fs::read_to_string(&shard_marker).expect("shards should append marker");
    assert!(marker.contains("shard-0|[\"case-a\"]"));
    assert!(marker.contains("shard-1|[\"case-b\"]"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_adapter_rejects_invalid_discovery_before_any_shard_runs() {
    let root = temp_root("command-invalid-discovery");
    let workspace = root.join("workspace");
    let bin_dir = root.join("bin");
    let inventory = root.join("inventory.txt");
    fs::create_dir_all(&workspace).expect("workspace should exist");
    let shard_runner = bin_dir.join("run-tests");
    write_script(
        &shard_runner,
        r#"#!/bin/sh
printf 'unexpected\n' >> "$SHARD_MARKER"
"#,
    );

    let oversized = (0..=100_000)
        .map(|index| format!("case-{index}\n"))
        .collect::<String>();
    let invalid_cases = vec![
        ("malformed", "json_array", "{not-json".to_string(), "JSON"),
        (
            "non-string",
            "json_array",
            "[\"case-a\",7]".to_string(),
            "only strings",
        ),
        (
            "blank-json",
            "json_array",
            "[\"case-a\",\" \"]".to_string(),
            "blank test identifier",
        ),
        (
            "duplicate",
            "json_array",
            "[\"case-a\",\"case-a\"]".to_string(),
            "duplicate test identifier",
        ),
        ("empty", "json_array", "[]".to_string(), "no test cases"),
        (
            "blank-line",
            "lines",
            "case-a\n\ncase-b\n".to_string(),
            "blank test identifier",
        ),
        ("oversized", "lines", oversized, "maximum is 100000"),
        (
            "identifier-too-large",
            "json_array",
            format!("[\"{}\"]", "x".repeat(4_097)),
            "maximum is 4096",
        ),
    ];

    for (label, output_format, contents, expected) in invalid_cases {
        fs::write(&inventory, contents).expect("inventory should be written");
        let output = root.join(format!("output-{label}"));
        let shard_marker = root.join(format!("shards-{label}.txt"));
        let result = run(json!({
            "workspace_path": workspace.to_string_lossy(),
            "output_dir": output.to_string_lossy(),
            "runner_parallelism": 4,
            "env": {
                "SHARD_MARKER": shard_marker.to_string_lossy()
            },
            "runner": {
                "kind": "test_discovery_sharded",
                "adapter": "command",
                "discovery_program": "/bin/cat",
                "discovery_args": [inventory.to_string_lossy()],
                "discovery_output_format": output_format,
                "run_program": shard_runner.to_string_lossy(),
                "append_test_items": true
            }
        }));

        assert_eq!(result["status"], json!("fail"), "{label}");
        assert_eq!(
            result["failure"]["stage"],
            json!("command_discovery_output"),
            "{label}"
        );
        assert!(
            result["failure"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains(expected),
            "{label}: {}",
            result["failure"]["message"]
        );
        assert!(
            !shard_marker.exists(),
            "{label}: invalid discovery must fail before shard execution"
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_adapter_rejects_malformed_configuration() {
    let root = temp_root("command-invalid-config");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace should exist");
    let base = json!({
        "workspace_path": workspace.to_string_lossy(),
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "command",
            "run_program": "/bin/true"
        }
    });
    let error = ci_test_discovery_sharded_run_json(&base)
        .expect_err("missing discovery program should fail closed");
    assert!(error.contains("discovery_program"), "{error}");

    let error = ci_test_discovery_sharded_run_json(&json!({
        "workspace_path": workspace.to_string_lossy(),
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "command",
            "discovery_program": "/bin/true",
            "run_program": "/bin/true",
            "discovery_output_format": "xml"
        }
    }))
    .expect_err("unsupported output format should fail closed");
    assert!(error.contains("discovery_output_format"), "{error}");

    let error = ci_test_discovery_sharded_run_json(&json!({
        "workspace_path": workspace.to_string_lossy(),
        "runner": {
            "kind": "test_discovery_sharded",
            "adapter": "command",
            "discovery_program": "/bin/true",
            "run_program": "/bin/true",
            "doc_tests": true
        }
    }))
    .expect_err("command doc tests should fail closed");
    assert!(error.contains("cargo adapter"), "{error}");

    let _ = fs::remove_dir_all(root);
}
