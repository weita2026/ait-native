use assert_cmd::Command;
use predicates::prelude::*;

fn evidence_manifest(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evidence")
        .join("stage4-2026-07-19")
        .join(name)
}

fn agent_token_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

#[test]
fn protocol_command_exposes_compiled_versioned_contract() {
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .arg("protocol")
        .assert()
        .success()
        .stdout(predicate::str::contains("ait-vcs-benchmark-protocol/v1"))
        .stdout(predicate::str::contains(
            "minimum_measured_local_iterations",
        ));
}

#[test]
fn agent_token_protocol_and_solo_local_template_validate() {
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "protocol"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "ait-agent-token-benchmark-protocol/v1",
        ))
        .stdout(predicate::str::contains(
            "\"workflow_mode\": \"solo_local\"",
        ))
        .stdout(predicate::str::contains("\"core_sprint_mode\": \"off\""))
        .stdout(predicate::str::contains(
            "\"ait_server_connection_allowed\": false",
        ));

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "validate", "--manifest"])
        .arg(agent_token_path(
            "campaigns/agent-token-game-v1/smoke-steady-state.json",
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"workflow_mode\": \"solo_local\"",
        ))
        .stdout(predicate::str::contains("\"sprint_mode\": \"off\""))
        .stdout(predicate::str::contains("\"ait_server_connected\": false"));
}

#[test]
fn agent_token_fixture_materialization_and_browser_contract_are_cli_visible() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("candidate");
    let receipt = temp.path().join("receipt.json");
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "fixture", "--manifest"])
        .arg(agent_token_path(
            "fixtures/agent-token-game-v1/manifest.json",
        ))
        .args(["--workload", "GD-01", "--output-dir"])
        .arg(&output)
        .arg("--receipt")
        .arg(&receipt)
        .assert()
        .success()
        .stdout(predicate::str::contains("GD-01"))
        .stdout(predicate::str::contains("browser-check.mjs"));
    assert!(output.join("TASK.txt").is_file());
    assert!(receipt.is_file());
}

#[test]
fn agent_token_usage_import_does_not_double_count_cached_input() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("codex.jsonl");
    std::fs::write(
        &source,
        concat!(
            "{\"type\":\"turn.completed\",\"usage\":{",
            "\"input_tokens\":100,\"cached_input_tokens\":60,",
            "\"cache_write_input_tokens\":0,\"output_tokens\":25,",
            "\"reasoning_output_tokens\":5}}\n"
        ),
    )
    .unwrap();
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "import-usage", "--source"])
        .arg(&source)
        .args([
            "--run-id",
            "run-1",
            "--workload",
            "GD-01",
            "--mode",
            "git_linear_single_session",
            "--profile",
            "steady_state_task_cost",
            "--model-provider",
            "openai",
            "--model-id",
            "test",
            "--model-revision",
            "test",
            "--reasoning-effort",
            "medium",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"provider_total_tokens\": 125"));
}

#[test]
fn agent_token_schedule_is_frozen_and_create_new() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("schedule.json");
    let manifest = agent_token_path("campaigns/agent-token-game-v1/smoke-steady-state.json");
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "schedule", "--manifest"])
        .arg(&manifest)
        .arg("--output")
        .arg(&output)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"entry_count\": 2"));
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "schedule", "--manifest"])
        .arg(&manifest)
        .arg("--output")
        .arg(&output)
        .assert()
        .failure()
        .stderr(predicate::str::contains("without overwriting"));
}

#[test]
fn fixture_digest_excludes_subject_specific_vcs_metadata() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("payload.txt"), "payload").unwrap();
    std::fs::create_dir(root.path().join(".git")).unwrap();
    std::fs::write(root.path().join(".git/config"), "metadata").unwrap();
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["fixture", "digest", "--root"])
        .arg(root.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("sha256:"));
}

#[test]
fn plain_fixture_digest_is_probe_safe() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("payload.txt"), "payload").unwrap();
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["fixture", "digest", "--plain", "--root"])
        .arg(root.path())
        .assert()
        .success()
        .stdout(predicate::str::starts_with("sha256:"))
        .stdout(predicate::str::is_match("^[^\\n]+\\n$").unwrap());
}

#[test]
fn portable_validation_accepts_committed_manifest_and_lists_bindings() {
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["validate", "--portable", "--manifest"])
        .arg(evidence_manifest("candidate-manifest.json"))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"portable\": true"))
        .stdout(predicate::str::contains("candidate-ait-workspace"))
        .stdout(predicate::str::contains("ait-benchmark"));
}

#[test]
fn run_fails_before_writing_when_a_required_binding_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let output = temp.path().join("raw.jsonl");
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["run", "--smoke", "--manifest"])
        .arg(evidence_manifest("candidate-manifest.json"))
        .arg("--raw-jsonl")
        .arg(&output)
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing runtime binding"));
    assert!(!output.exists());
}

#[test]
fn normalize_redacts_runtime_values_and_refuses_overwrite() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("runtime.json");
    let output = temp.path().join("portable.json");
    let mut manifest =
        std::fs::read_to_string(evidence_manifest("candidate-manifest.json")).unwrap();
    let bindings = [
        ("candidate-ait-workspace", "/private/campaign/ait"),
        ("candidate-git-workspace", "/private/campaign/git"),
        ("candidate-ait-cli", "/private/bin/ait-cli"),
        ("ait-benchmark", "/private/bin/ait-benchmark"),
        ("git", "/private/bin/git"),
    ];
    for (name, value) in bindings {
        manifest = manifest.replace(&format!("{{binding:{name}}}"), value);
    }
    std::fs::write(&source, manifest).unwrap();

    let mut command = Command::cargo_bin("ait-benchmark").unwrap();
    command
        .args(["normalize", "--manifest"])
        .arg(&source)
        .arg("--output")
        .arg(&output);
    for (name, value) in bindings {
        command.arg("--bind").arg(format!("{name}={value}"));
    }
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("\"portable\": true"));

    let normalized = std::fs::read_to_string(&output).unwrap();
    assert!(!normalized.contains("/private/"));
    for (name, _) in bindings {
        assert!(normalized.contains(&format!("{{binding:{name}}}")));
    }

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["validate", "--portable", "--manifest"])
        .arg(&output)
        .assert()
        .success();

    let mut overwrite = Command::cargo_bin("ait-benchmark").unwrap();
    overwrite
        .args(["normalize", "--manifest"])
        .arg(&source)
        .arg("--output")
        .arg(&output);
    for (name, value) in bindings {
        overwrite.arg("--bind").arg(format!("{name}={value}"));
    }
    overwrite
        .assert()
        .failure()
        .stderr(predicate::str::contains("without overwriting"));
}

#[cfg(unix)]
#[test]
fn bound_portable_manifest_executes_a_complete_smoke_block() {
    use ait_benchmark::digest_workspace;

    let temp = tempfile::tempdir().unwrap();
    let ait_workspace = temp.path().join("ait-workspace");
    let git_workspace = temp.path().join("git-workspace");
    std::fs::create_dir_all(&ait_workspace).unwrap();
    std::fs::create_dir_all(&git_workspace).unwrap();
    std::fs::write(ait_workspace.join("payload.txt"), "payload").unwrap();
    std::fs::write(git_workspace.join("payload.txt"), "payload").unwrap();
    let digest =
        digest_workspace(&ait_workspace, &[".ait".to_string(), ".git".to_string()]).unwrap();
    let command = serde_json::json!({
        "program": "{binding:shell}",
        "args": ["-c", "exit 0"],
        "cwd": "{workspace}",
        "env": {},
        "expected_exit_codes": [0]
    });
    let history = serde_json::json!({
        "program": "{binding:echo}",
        "args": ["1"],
        "cwd": "{workspace}",
        "env": {},
        "expected_exit_codes": [0]
    });
    let outcome = serde_json::json!({
        "program": "{binding:echo}",
        "args": ["same"],
        "cwd": "{workspace}",
        "env": {},
        "expected_exit_codes": [0]
    });
    let subject = |id: &str, role: &str, workspace_binding: &str| {
        serde_json::json!({
            "subject_id": id,
            "role": role,
            "workspace_root": format!("{{binding:{workspace_binding}}}"),
            "metadata_excludes": [".ait", ".git"],
            "command": command.clone(),
            "reset_commands": [],
            "prepare_commands": [],
            "cleanup_commands": [],
            "history_node_probe": history.clone(),
            "outcome_probe": outcome.clone(),
            "metrics_json_path": null
        })
    };
    let features = [
        "small_text",
        "large_binary",
        "deep_directories",
        "ignored_files",
        "renames",
        "branches",
        "merge_history",
    ];
    let fixture = |id: &str, kind: &str| {
        serde_json::json!({
            "fixture_id": id,
            "revision": "1",
            "scale": "small",
            "kind": kind,
            "source": format!("fixture://{id}"),
            "redistribution": "generated for CLI test",
            "content_digest": digest,
            "file_count": 1,
            "total_bytes": 7,
            "history_nodes": 1,
            "features": features
        })
    };
    let manifest = serde_json::json!({
        "contract": "ait-vcs-benchmark-manifest/v1",
        "benchmark_id": "portable-cli-smoke",
        "protocol_revision": "vcs-performance-test",
        "campaign_scope": "focused_slice",
        "seed": 42,
        "sampling": {
            "warmup_iterations": 5,
            "measured_local_iterations": 50,
            "measured_cold_iterations": 30
        },
        "environment": {
            "captured_at": "2026-08-22T00:00:00Z",
            "os": "test",
            "architecture": "test",
            "filesystem": "test",
            "storage_medium": "test",
            "cpu": "test",
            "memory_bytes": 1,
            "rust_version": "test",
            "git_version": "test",
            "ait_version": "test",
            "repository_snapshot": "test",
            "server_revision": "test",
            "network_profile": "test",
            "cache_drop_method": "test",
            "command_options": {}
        },
        "fixtures": [
            fixture("small-synthetic", "synthetic"),
            fixture("small-real", "real")
        ],
        "cells": [{
            "cell_id": "small-status-clean-warm-local",
            "fixture_id": "small-synthetic",
            "operation": "status_clean",
            "temperature": "warm",
            "sample_class": "local",
            "subjects": [
                subject("ait-test", "ait", "ait-workspace"),
                subject("git-test", "git", "git-workspace")
            ]
        }],
        "bootstrap_resamples": 1000,
        "limitations": ["CLI smoke only"]
    });
    let manifest_path = temp.path().join("manifest.json");
    let raw_path = temp.path().join("raw.jsonl");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["run", "--smoke", "--manifest"])
        .arg(&manifest_path)
        .arg("--raw-jsonl")
        .arg(&raw_path)
        .arg("--bind")
        .arg("shell=/bin/sh")
        .arg("--bind")
        .arg("echo=/bin/echo")
        .arg("--bind")
        .arg(format!("ait-workspace={}", ait_workspace.display()))
        .arg("--bind")
        .arg(format!("git-workspace={}", git_workspace.display()))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"sample_count\": 6"))
        .stdout(predicate::str::contains("\"failure_count\": 0"));
    assert!(raw_path.is_file());
}
