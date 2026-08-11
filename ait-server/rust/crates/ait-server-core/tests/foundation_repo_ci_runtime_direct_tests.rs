use ait_server_core::foundation::ci_runtime_json::RepoCiRunJson;
use ait_server_core::foundation::repo_ci_runtime::repo_ci_run_json;
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
        "ait-server-repo-ci-runtime-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn run_repo_ci(payload: JsonValue) -> JsonValue {
    repo_ci_run_json(&payload).expect("repo CI runtime should run")
}

#[test]
fn repo_ci_run_json_wrapper_preserves_prewarm_suite_cleanup_and_gate_shapes() {
    let root = temp_root("wrapper-prewarm-suite");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = RepoCiRunJson::stateless()
        .run(&json!({
            "repo_name": "ait",
            "repo_id": "REPO-1",
            "snapshot_id": "SNP-WRAPPER",
            "target_line": "main",
            "plane": "nightly",
            "workspace_path": workspace.to_string_lossy(),
            "output_dir": output_dir.to_string_lossy(),
            "cleanup_workspace": true,
            "ci_config": {
                "nightly_suites": ["nightly_smoke"]
            },
            "materialized_files": [
                {"path": "src/lib.rs", "content": "pub fn wrapper_repo_ci() -> bool { true }\\n"}
            ],
            "prewarm_commands": [
                "printf warm > .repo-ci-prewarm"
            ],
            "suites": [{
                "suite_id": "nightly_smoke",
                "plane": "nightly",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["test -f src/lib.rs && test -f .repo-ci-prewarm"]
                }
            }]
        }))
        .expect("wrapper repo CI should run");

    assert_eq!(value["contract"], json!("ait.server.repo_ci.run.v1"));
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["selected_suite_ids"], json!(["nightly_smoke"]));
    assert_eq!(value["native_prewarm"]["status"], json!("pass"));
    assert_eq!(
        value["suite_results"][0]["runner_kind"],
        json!("rust_repo_ci")
    );
    assert_eq!(
        value["repo_ci_detail"]["server_ci_gate"]["rust_repo_ci_runtime"],
        json!(true)
    );
    assert_eq!(value["cleanup"]["status"], json!("cleaned"));
    assert!(
        !workspace.exists(),
        "wrapper run should preserve cleanup ownership"
    );

    let _ = fs::remove_dir_all(root);
}

fn runtime_base_from_log(value: &JsonValue) -> PathBuf {
    let log_path = PathBuf::from(
        value["suite_results"][0]["artifacts"]["log_path"]["path"]
            .as_str()
            .expect("suite log path should be text"),
    );
    log_path
        .parent()
        .and_then(|path| path.parent())
        .and_then(|path| path.parent())
        .expect("log path should live under base/output/suite")
        .to_path_buf()
}

#[test]
fn repo_ci_runtime_can_create_rust_owned_runtime_paths() {
    let root = temp_root("rust-owned-paths");
    let runtime_root = root.join("runtime");
    let server_data_root = root.join("server-data");
    fs::create_dir_all(&server_data_root).expect("server data root should be created");

    let value = run_repo_ci(json!({
        "repo_name": "ait",
        "repo_id": "REPO-1",
        "snapshot_id": "SNP-1",
        "target_line": "main",
        "plane": "nightly",
        "server_data_root": server_data_root.to_string_lossy(),
        "ci_temp_root": runtime_root.to_string_lossy(),
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn rust_owned_repo_ci() -> bool { true }\\n"}
        ],
        "suites": [{
            "suite_id": "nightly_smoke",
            "plane": "nightly",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": ["test -f src/lib.rs"]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    let runtime_base = runtime_base_from_log(&value);
    assert!(runtime_base.starts_with(&runtime_root));
    assert!(
        !runtime_base.exists(),
        "Rust-owned run base, including outputs and manifest, should be cleaned on terminal state"
    );
    assert_eq!(value["cleanup"]["removed_scope"], json!("managed_run_base"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_runtime_rejects_retired_inline_binary_files() {
    let root = temp_root("retired-binary-materialized-file");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");
    let retired_binary_key = ["content", "base64"].join("_");
    let mut materialized_file = json!({
        "path": "src/lib.rs"
    });
    materialized_file[retired_binary_key.as_str()] = json!("AAECA/8=");

    let error = repo_ci_run_json(&json!({
        "repo_name": "ait",
        "repo_id": "REPO-1",
        "snapshot_id": "SNP-1",
        "target_line": "main",
        "plane": "nightly",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "materialized_files": [materialized_file],
        "suites": [{
            "suite_id": "nightly_smoke",
            "plane": "nightly",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": ["test -f src/lib.rs"]
            }
        }]
    }))
    .expect_err("retired inline binary materialization should fail closed");

    assert!(error.contains("pack-backed materialization"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_runtime_cleans_workspace_when_suite_errors() {
    let root = temp_root("error-cleanup");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let error = repo_ci_run_json(&json!({
        "repo_name": "ait",
        "repo_id": "REPO-1",
        "snapshot_id": "SNP-1",
        "target_line": "main",
        "plane": "nightly",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn should_cleanup() {}\\n"}
        ],
        "suites": [{
            "suite_id": "nightly_smoke",
            "plane": "nightly",
            "mode": "gate",
            "default_blocking": true,
            "runner": {"kind": "unsupported"}
        }]
    }))
    .expect_err("unsupported runner should fail the runtime");

    assert!(error.contains("Unsupported repo CI runner kind"));
    assert!(
        !workspace.exists(),
        "workspace must be cleaned even when suite execution returns Err"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_runtime_selects_configured_nightly_suites_runs_one_prewarm_and_keeps_logs_after_cleanup()
{
    let root = temp_root("nightly-prewarm");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = run_repo_ci(json!({
        "repo_name": "ait",
        "repo_id": "REPO-1",
        "snapshot_id": "SNP-1",
        "target_line": "main",
        "plane": "nightly",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "ci_config": {
            "nightly_suites": ["nightly_b", "nightly_a"]
        },
        "materialized_files": [
            {"path": "src/lib.rs", "content": "pub fn repo_ci() -> bool { true }\\n"}
        ],
        "prewarm_commands": [
            "test ! -f .repo-ci-prewarm && printf warm > .repo-ci-prewarm"
        ],
        "suites": [
            {
                "suite_id": "nightly_a",
                "plane": "nightly",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["test -f src/lib.rs && test -f .repo-ci-prewarm && printf a > suite-a.txt"]
                }
            },
            {
                "suite_id": "nightly_b",
                "plane": "nightly",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["test -f src/lib.rs && test -f .repo-ci-prewarm && printf b > suite-b.txt"]
                }
            },
            {
                "suite_id": "release_only",
                "plane": "release",
                "mode": "gate",
                "default_blocking": true,
                "runner": {
                    "kind": "command_bundle",
                    "commands": ["exit 99"]
                }
            }
        ]
    }));

    assert_eq!(value["contract"], json!("ait.server.repo_ci.run.v1"));
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["suite_status"], json!("pass"));
    assert_eq!(value["suite_failures"], json!([]));
    assert_eq!(
        value["selected_suite_ids"],
        json!(["nightly_a", "nightly_b"])
    );
    assert_eq!(value["native_prewarm"]["status"], json!("pass"));
    assert_eq!(
        value["native_prewarm"]["reports"]
            .as_array()
            .expect("prewarm reports should be an array")
            .len(),
        1
    );
    assert!(value["suite_results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|suite| suite["runner_kind"] == json!("rust_repo_ci")));
    assert_eq!(
        value["server_ci_gate"]["python_server_ci_executor"],
        json!(false)
    );
    assert_eq!(value["server_ci_gate"]["rust_repo_ci_runtime"], json!(true));
    assert!(!workspace.exists(), "cleanup should remove dirty workspace");

    for suite in value["suite_results"].as_array().unwrap() {
        let log_path = PathBuf::from(
            suite["artifacts"]["log_path"]["path"]
                .as_str()
                .expect("suite log path should be text"),
        );
        assert!(log_path.is_file(), "suite log must outlive workspace");
    }

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn repo_ci_runtime_cleanup_removes_readonly_suite_artifacts() {
    let root = temp_root("readonly-cleanup");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = run_repo_ci(json!({
        "repo_name": "ait",
        "snapshot_id": "SNP-READONLY",
        "target_line": "main",
        "plane": "nightly",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "cleanup_workspace": true,
        "ci_config": {
            "nightly_suites": ["nightly_smoke"]
        },
        "suites": [{
            "suite_id": "nightly_smoke",
            "plane": "nightly",
            "mode": "gate",
            "default_blocking": true,
            "runner": {
                "kind": "command_bundle",
                "commands": [
                    "mkdir -p readonly/child && printf locked > readonly/child/file.txt && chmod 0555 readonly/child readonly"
                ]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["cleanup"]["status"], json!("cleaned"));
    assert!(
        !workspace.exists(),
        "cleanup should remove readonly directories created by suite commands"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_runtime_attaches_release_gate_evidence_and_reports_missing_keys() {
    let root = temp_root("release-gate");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let value = run_repo_ci(json!({
        "repo_name": "ait",
        "snapshot_id": "SNP-REL",
        "target_line": "release",
        "plane": "release",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "dependency_evidence": ["lockfile"],
        "compliance_evidence": [],
        "ci_config": {
            "release_suites": ["release_smoke"],
            "rollout": {
                "release_evidence": {
                    "dependency_keys": ["lockfile", "sbom"],
                    "compliance_keys": ["license"]
                }
            }
        },
        "suites": [{
            "suite_id": "release_smoke",
            "plane": "release",
            "mode": "gate",
            "default_blocking": true,
            "release_gate_evidence": {
                "dependency_keys": ["wheel"],
                "compliance_keys": ["notice"]
            },
            "runner": {
                "kind": "command_bundle",
                "commands": ["printf release > release.txt"]
            }
        }]
    }));

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["suite_status"], json!("pass"));
    let gate = &value["suite_results"][0]["release_gate_evidence"];
    assert_eq!(
        gate["dependency_keys"],
        json!(["lockfile", "sbom", "wheel"])
    );
    assert_eq!(gate["compliance_keys"], json!(["license", "notice"]));
    assert_eq!(gate["attached_dependency_evidence"], json!(["lockfile"]));
    assert_eq!(gate["missing_dependency_keys"], json!(["sbom", "wheel"]));
    assert_eq!(
        gate["missing_compliance_keys"],
        json!(["license", "notice"])
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_non_full_legacy_runner_fails_closed_without_execution() {
    let root = temp_root("repo-non-full-legacy-runner-rejected");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let error = repo_ci_run_json(&json!({
        "repo_name": "ait",
        "snapshot_id": "SNP-PREVIEW",
        "target_line": "main",
        "plane": "nightly",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "ci_config": {
            "nightly_suites": ["postgres_preview"]
        },
        "materialized_files": [],
        "suites": [{
            "suite_id": "postgres_preview",
            "plane": "nightly",
            "mode": "diagnostic",
            "default_blocking": false,
            "runner": {
                "kind": "pytest",
                "args": ["--version"]
            }
        }]
    }))
    .expect_err("legacy runner must fail closed");

    assert!(error.contains("runner kind `pytest` is not supported"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn repo_ci_task_batch_requires_explicit_rust_inputs_and_can_fail_blocking_suite() {
    let root = temp_root("task-batch");
    let workspace = root.join("workspace");
    let output_dir = root.join("output");

    let missing = repo_ci_run_json(&json!({
        "repo_name": "ait",
        "snapshot_id": "SNP-TASK",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "suites": [{
            "suite_id": "task_batch",
            "plane": "nightly",
            "mode": "gate",
            "default_blocking": true,
            "runner": {"kind": "task_batch"}
        }]
    }))
    .expect_err("task_batch without explicit inputs should fail closed");
    assert!(missing.contains("requires explicit task_batch_inputs"));

    let value = run_repo_ci(json!({
        "repo_name": "ait",
        "snapshot_id": "SNP-TASK",
        "workspace_path": workspace.to_string_lossy(),
        "output_dir": output_dir.to_string_lossy(),
        "task_batch_inputs": {
            "task_batch": {
                "status": "fail",
                "selector": "recent_remote_landed",
                "selected_tasks": [{"task_id": "RT-1"}],
                "behavior_regressions": {
                    "status": "fail",
                    "failing_suite_ids": ["behavior"]
                }
            }
        },
        "suites": [{
            "suite_id": "task_batch",
            "plane": "nightly",
            "mode": "gate",
            "default_blocking": true,
            "runner": {"kind": "task_batch"}
        }]
    }));

    assert_eq!(value["tests_status"], json!("fail"));
    assert_eq!(value["blocking_failures"], json!(["task_batch"]));
    assert_eq!(value["suite_status"], json!("fail"));
    assert_eq!(value["suite_failures"], json!(["task_batch"]));
    assert_eq!(
        value["suite_results"][0]["runner_kind"],
        json!("rust_task_batch")
    );
    assert_eq!(
        value["suite_results"][0]["task_batch_summary"]["selected_tasks"][0]["task_id"],
        json!("RT-1")
    );
    assert!(PathBuf::from(
        value["suite_results"][0]["artifacts"]["summary_json"]["path"]
            .as_str()
            .unwrap()
    )
    .is_file());

    let _ = fs::remove_dir_all(root);
}
