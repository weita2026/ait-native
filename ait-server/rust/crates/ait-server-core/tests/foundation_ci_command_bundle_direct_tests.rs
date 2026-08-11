use ait_server_core::foundation::ci_command_bundle::ci_command_bundle_run_json;
use ait_server_core::foundation::ci_runtime_json::CommandBundleRunJson;
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    env::temp_dir().join(format!(
        "ait-server-ci-command-bundle-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn run_bundle(payload: JsonValue) -> JsonValue {
    ci_command_bundle_run_json(&payload).expect("command bundle should run")
}

#[test]
fn command_bundle_run_json_wrapper_preserves_failure_scheduler_and_artifact_shapes() {
    let root = temp_root("wrapper-failure-scheduler");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    fs::create_dir_all(&workspace).expect("workspace should be created");

    let value = CommandBundleRunJson::stateless()
        .run(&json!({
            "workspace_path": workspace.to_string_lossy(),
            "output_dir": output_dir.to_string_lossy(),
            "runner_parallelism": 2,
            "runner": {
                "kind": "command_bundle",
                "commands": [
                    "printf 'FIRST-LINE\\n'; printf 'LAST-LINE\\n'; exit 7"
                ]
            }
        }))
        .expect("wrapper should run command bundle");

    assert_eq!(
        value["contract"],
        json!("ait.server.ci_command_bundle_run.v1")
    );
    assert_eq!(value["status"], json!("fail"));
    assert_eq!(value["failure"]["stage"], json!("command"));
    assert_eq!(value["failure"]["exit_code"], json!(7));
    assert!(value["failure"]["combined_tail"]
        .as_str()
        .expect("failure tail should be text")
        .contains("LAST-LINE"));
    assert_eq!(value["environment"]["runner_parallelism"], json!(2));
    assert_eq!(
        value["environment"]["process_policy"]["policy"],
        json!("safe_ambient_allowlist_with_explicit_overrides")
    );
    assert_eq!(
        value["environment"]["process_policy"]["ambient_secret_forwarding"],
        json!(false)
    );
    assert_eq!(
        value["diagnostics"]["scheduler_parallelism_controls_command_environment"],
        json!(true)
    );
    assert!(PathBuf::from(value["artifacts"]["summary_json"]["path"].as_str().unwrap()).is_file());
    assert!(PathBuf::from(value["artifacts"]["log_path"]["path"].as_str().unwrap()).is_file());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_bundle_runs_prewarm_and_commands_with_shared_environment_and_durable_logs() {
    let root = temp_root("shared-env");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let target_dir = root.join("target");
    let build_dir = root.join("build");
    let temp_dir = root.join("tmp");
    fs::create_dir_all(&workspace).expect("workspace should be created");

    let value = run_bundle(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "shared_cargo_target_dir": target_dir.to_string_lossy(),
        "shared_cargo_build_dir": build_dir.to_string_lossy(),
        "temp_dir": temp_dir.to_string_lossy(),
        "env": {"AIT_TEST_MARKER": "shared"},
        "runner": {
            "kind": "command_bundle",
            "prewarm_commands": [
                "test \"$AIT_TEST_MARKER\" = shared && test \"$CARGO_TARGET_DIR\" = \"$AIT_SHARED_CARGO_TARGET_DIR\" && test \"$CARGO_BUILD_BUILD_DIR\" = \"$AIT_SHARED_CARGO_BUILD_DIR\" && printf '%s\\n' \"$AIT_CI_WORKSPACE_PATH\" > prewarm-marker.txt"
            ],
            "commands": [
                "test \"$AIT_CI_PREWARM_COMPLETE\" = 1 && test -f prewarm-marker.txt && printf run > \"$AIT_CI_COMMAND_BUNDLE_OUTPUT_DIR/run-marker.txt\""
            ]
        }
    }));

    assert_eq!(
        value["contract"],
        json!("ait.server.ci_command_bundle_run.v1")
    );
    assert_eq!(value["status"], json!("pass"));
    assert_eq!(value["prewarm"]["status"], json!("pass"));
    assert_eq!(value["prewarm"]["reports"].as_array().unwrap().len(), 1);
    assert_eq!(value["command_reports"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["environment"]["shared_cargo_target_dir"],
        json!(target_dir.to_string_lossy())
    );
    assert_eq!(
        value["environment"]["shared_cargo_build_dir"],
        json!(build_dir.to_string_lossy())
    );
    assert!(output_dir.join("run-marker.txt").is_file());

    let command_log = PathBuf::from(
        value["command_reports"][0]["log_path"]
            .as_str()
            .expect("command log path should be text"),
    );
    let merged_log = PathBuf::from(
        value["artifacts"]["log_path"]["path"]
            .as_str()
            .expect("merged log path should be text"),
    );
    fs::remove_dir_all(&workspace).expect("workspace cleanup should succeed");
    assert!(
        command_log.is_file(),
        "per-command log must outlive workspace"
    );
    assert!(merged_log.is_file(), "merged log must outlive workspace");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_bundle_uses_scheduler_parallelism_for_cargo_and_libtest_env() {
    let root = temp_root("scheduler-parallelism");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    fs::create_dir_all(&workspace).expect("workspace should be created");

    let value = run_bundle(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "runner_parallelism": 3,
        "env": {
            "CARGO_BUILD_JOBS": "99",
            "RUST_TEST_THREADS": "99"
        },
        "runner": {
            "kind": "command_bundle",
            "commands": [
                "test \"$AIT_RUNNER_PARALLELISM\" = 3 && test \"$AIT_CI_RUNNER_PARALLELISM\" = 3 && test \"$AIT_CI_ADMITTED_CPU_TOKENS\" = 3 && test \"$CARGO_BUILD_JOBS\" = 3 && test \"$RUST_TEST_THREADS\" = 3"
            ]
        }
    }));

    assert_eq!(value["status"], json!("pass"));
    assert_eq!(value["environment"]["runner_parallelism"], json!(3));
    assert_eq!(value["environment"]["admitted_cpu_tokens"], json!(3));
    assert_eq!(
        value["environment"]["parallelism_source"],
        json!("scheduler")
    );
    assert_eq!(value["environment"]["cargo_build_jobs_env"], json!("3"));
    assert_eq!(value["environment"]["rust_test_threads_env"], json!("3"));
    assert_eq!(
        value["diagnostics"]["scheduler_parallelism_controls_command_environment"],
        json!(true)
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_bundle_failure_json_keeps_tail_while_full_log_keeps_entire_output() {
    let root = temp_root("failure-tail");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    fs::create_dir_all(&workspace).expect("workspace should be created");

    let value = run_bundle(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "runner": {
            "kind": "command_bundle",
            "commands": [
                "i=1; while [ $i -le 1400 ]; do printf 'prefix-%04d\\n' \"$i\"; i=$((i + 1)); done; printf 'LAST-LINE\\n'; exit 7"
            ]
        }
    }));

    assert_eq!(value["status"], json!("fail"));
    assert_eq!(value["failure"]["exit_code"], json!(7));
    let tail = value["failure"]["combined_tail"]
        .as_str()
        .expect("failure tail should be text");
    assert!(tail.contains("LAST-LINE"));
    assert!(!tail.contains("prefix-0001"));

    let command_log = PathBuf::from(
        value["command_reports"][0]["log_path"]
            .as_str()
            .expect("command log path should be text"),
    );
    let log_text = fs::read_to_string(&command_log).expect("full command log should read");
    assert!(log_text.contains("prefix-0001"));
    assert!(log_text.contains("LAST-LINE"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_bundle_streams_multi_megabyte_output_with_bounded_json_tails() {
    let root = temp_root("large-streamed-output");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    fs::create_dir_all(&workspace).expect("workspace should be created");

    let value = run_bundle(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "runner": {
            "kind": "command_bundle",
            "commands": [
                "head -c 4194304 /dev/zero | tr '\\000' x; printf stderr-marker >&2"
            ]
        }
    }));

    assert_eq!(value["status"], json!("pass"));
    assert_eq!(
        value["command_reports"][0]["stdout_bytes"],
        json!(4_194_304)
    );
    assert_eq!(
        value["command_reports"][0]["stderr_bytes"],
        json!("stderr-marker".len())
    );
    assert!(
        value["command_reports"][0]["combined_tail"]
            .as_str()
            .unwrap()
            .len()
            <= 12_000
    );
    let log_path = PathBuf::from(value["command_reports"][0]["log_path"].as_str().unwrap());
    assert!(fs::metadata(log_path).unwrap().len() > 4_194_304);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_bundle_reports_configured_timeout_with_complete_log() {
    let root = temp_root("timeout");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    fs::create_dir_all(&workspace).expect("workspace should be created");

    let value = run_bundle(json!({
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "runner": {
            "kind": "command_bundle",
            "timeout_seconds": 1,
            "commands": ["printf timeout-started; sleep 30"]
        }
    }));

    assert_eq!(value["status"], json!("fail"));
    assert_eq!(value["runner"]["timeout_seconds"], json!(1));
    assert_eq!(value["failure"]["timed_out"], json!(true));
    assert_eq!(value["failure"]["timeout_seconds"], json!(1));
    assert_eq!(value["command_reports"][0]["timed_out"], json!(true));
    let log_path = PathBuf::from(
        value["command_reports"][0]["log_path"]
            .as_str()
            .expect("timeout log path should be text"),
    );
    let log = fs::read_to_string(log_path).expect("timeout log should be retained");
    assert!(log.contains("timed_out=true"));
    assert!(log.contains("timeout-started"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn command_bundle_rejects_invalid_timeout_before_execution() {
    let root = temp_root("invalid-timeout");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("workspace should be created");

    for invalid in [json!(0), json!(-1), json!(86_401), json!("1")] {
        let error = ci_command_bundle_run_json(&json!({
            "workspace_path": workspace.to_string_lossy(),
            "runner": {
                "kind": "command_bundle",
                "timeout_seconds": invalid,
                "commands": ["printf must-not-run > marker.txt"]
            }
        }))
        .expect_err("invalid timeout should fail closed");
        assert!(error.contains("timeout_seconds"), "{error}");
        assert!(!workspace.join("marker.txt").exists());
    }

    let _ = fs::remove_dir_all(root);
}
