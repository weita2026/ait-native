use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value as JsonValue;

fn evidence_manifest(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evidence")
        .join("stage4-2026-07-19")
        .join(name)
}

fn agent_token_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn repository_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

#[test]
fn agent_token_publication_bundle_is_sanitized_checksummed_and_release_bound() {
    let campaign = "game-v1-g56s-max-complete200-fx27-20260826";
    let bundle = repository_root().join("release/benchmarks").join(campaign);
    let result: JsonValue =
        serde_json::from_slice(&std::fs::read(bundle.join("result.json")).unwrap()).unwrap();
    let runs: JsonValue =
        serde_json::from_slice(&std::fs::read(bundle.join("runs.json")).unwrap()).unwrap();

    assert_eq!(
        result["contract"],
        "ait-agent-token-benchmark-publication/v1"
    );
    assert_eq!(result["release_version"], "1.1.0");
    assert_eq!(result["scheduled_run_count"], 200);
    assert_eq!(result["observed_run_count"], 200);
    assert_eq!(result["executed_evidence_run_count"], 201);
    assert_eq!(result["statistically_excluded_run_count"], 1);
    assert_eq!(result["valid_run_count"], 200);
    assert_eq!(result["invalid_run_count"], 0);
    assert_eq!(result["accepted_run_count"], 200);
    assert_eq!(result["accepted_by_mode"]["ait_linear_single_session"], 100);
    assert_eq!(result["accepted_by_mode"]["git_linear_single_session"], 100);
    assert_eq!(result["source_protocol_claim_eligible"], false);
    assert_eq!(result["claim_eligible"], true);
    assert_eq!(result["claim_blockers"], serde_json::json!([]));
    assert_eq!(result["retained_failures"].as_array().unwrap().len(), 0);
    assert_eq!(
        result["statistically_excluded_failures"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        result["statistically_excluded_failures"][0]["evaluator_score"],
        50
    );
    assert_eq!(
        result["replacement_policy_revision"],
        "game-development-2026-08-27.29"
    );
    assert_eq!(
        result["statistical_replacements"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        result["statistical_replacements"][0]["replacement_runner_sha256"],
        "sha256:89046039ffa7554b8791e5b2c2c75eaa42ac8c719bd93e831d1fc6ba2814d71d"
    );
    assert_eq!(
        result["source_sha256"]["replacement-runner"],
        "89046039ffa7554b8791e5b2c2c75eaa42ac8c719bd93e831d1fc6ba2814d71d"
    );
    assert_eq!(
        result["workload_results"][4]["acceptance_rate_deficit_percentage_points"],
        0.0
    );
    assert_eq!(runs["contract"], "ait-agent-token-benchmark-public-runs/v1");
    assert_eq!(runs["executed_evidence_run_count"], 201);
    assert_eq!(runs["statistically_excluded_run_count"], 1);
    let rows = runs["runs"].as_array().unwrap();
    assert_eq!(rows.len(), 200);
    assert_eq!(
        rows.iter()
            .filter(|row| row["valid_attempt"] == false)
            .count(),
        0
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row["accepted_equivalent"] == false)
            .count(),
        0
    );
    let excluded_rows = runs["excluded_runs"].as_array().unwrap();
    assert_eq!(excluded_rows.len(), 1);
    assert_eq!(excluded_rows[0]["accepted_equivalent"], false);
    assert_eq!(excluded_rows[0]["evaluator_score"], 50);

    let checksum_text = std::fs::read_to_string(bundle.join("SHA256SUMS")).unwrap();
    for line in checksum_text.lines() {
        let (expected, name) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(bundle.join(name)).unwrap();
        let observed = ait_benchmark::sha256_digest(&bytes);
        assert_eq!(observed.strip_prefix("sha256:").unwrap(), expected);
    }
    for name in ["summary.txt", "result.json", "runs.json", "SHA256SUMS"] {
        let text = std::fs::read_to_string(bundle.join(name)).unwrap();
        for private_marker in ["/Users/", ".ait-runtime", "private/", "codex-events.raw"] {
            assert!(
                !text.contains(private_marker),
                "{name} contains {private_marker}"
            );
        }
    }

    let preparer =
        std::fs::read_to_string(repository_root().join("ci/release_endpoint_publication.sh"))
            .unwrap();
    let remote =
        std::fs::read_to_string(repository_root().join("ci/release_endpoint_remote.sh")).unwrap();
    assert!(preparer.contains("ait-agent-token-benchmark-publication/v1"));
    assert!(preparer.contains("ait-agent-token-benchmark-${benchmark_campaign}.runs.json"));
    assert!(remote.contains("## AIT vs Git benchmark"));
    assert!(remote.contains("replacement-qualified result is claim-eligible"));

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "replace", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--campaign-dir"))
        .stdout(predicate::str::contains("--source-run-id"));

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "publish", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--campaign-dir"))
        .stdout(predicate::str::contains("--output-dir"))
        .stdout(predicate::str::contains("--release-version"))
        .stdout(predicate::str::contains("--measured-product-snapshot"))
        .stdout(predicate::str::contains("--measured-ait-sha256"))
        .stdout(predicate::str::contains("--campaign-runner-sha256"));
}

#[test]
fn sprint_on_replication_publication_preserves_current_policy_boundary() {
    let campaign = "game-v1-g56s-max-sprint-on-natural-complete200-20260828";
    let bundle = repository_root().join("release/benchmarks").join(campaign);
    let result: JsonValue =
        serde_json::from_slice(&std::fs::read(bundle.join("result.json")).unwrap()).unwrap();
    let runs: JsonValue =
        serde_json::from_slice(&std::fs::read(bundle.join("runs.json")).unwrap()).unwrap();

    assert_eq!(result["scheduled_run_count"], 200);
    assert_eq!(result["observed_run_count"], 200);
    assert_eq!(result["executed_evidence_run_count"], 203);
    assert_eq!(result["statistically_excluded_run_count"], 3);
    assert_eq!(result["valid_run_count"], 200);
    assert_eq!(result["invalid_run_count"], 0);
    assert_eq!(result["accepted_run_count"], 200);
    assert_eq!(result["accepted_by_mode"]["ait_linear_single_session"], 100);
    assert_eq!(result["accepted_by_mode"]["git_linear_single_session"], 100);
    assert_eq!(result["source_protocol_claim_eligible"], false);
    assert_eq!(
        result["current_policy_revision"],
        "game-development-2026-08-29.36"
    );
    assert_eq!(
        result["current_policy_evaluation_mode"],
        "owner_authorized_recovery_adjudication_and_statistical_replacement"
    );
    assert_eq!(result["current_policy_criteria_met"], true);
    assert_eq!(result["current_policy_blockers"], serde_json::json!([]));
    assert_eq!(result["claim_eligible"], true);
    assert_eq!(result["claim_blockers"], serde_json::json!([]));
    assert_eq!(runs["runs"].as_array().unwrap().len(), 200);
    assert_eq!(runs["excluded_runs"].as_array().unwrap().len(), 2);
    assert_eq!(
        result["host_shutdown_pair_recoveries"][0]["interrupted_run_id"],
        "game-v1-g56s-max-sprint-on-natural-complete200-20260828-b009-gd-05-git"
    );

    let summary = std::fs::read_to_string(bundle.join("summary.txt")).unwrap();
    assert!(summary.contains("Source-protocol claim eligible: **false**"));
    assert!(summary.contains("criteria met: **true**"));
    assert!(summary.contains("Effective recovery-policy-qualified claim eligible: **true**"));
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
        .stdout(predicate::str::contains(
            "\"core_sprint_mode\": \"manifest-pinned off for the primary campaign; separately reported sprint-on smoke and complete campaigns are admitted\"",
        ))
        .stdout(predicate::str::contains(
            "\"ait_server_connection_allowed\": false",
        ))
        .stdout(predicate::str::contains(
            "\"protocol_revision\": \"game-development-2026-08-31.48\"",
        ))
        .stdout(predicate::str::contains(
            "\"contract\": \"ait-agent-token-statistical-replacement/v1\"",
        ))
        .stdout(predicate::str::contains("\"measured_ait_treatment\""))
        .stdout(predicate::str::contains(
            "\"measured_ait_allowed_commands\"",
        ))
        .stdout(predicate::str::contains("\"measured_git_treatment\""))
        .stdout(predicate::str::contains(
            "\"measured_git_required_commands\"",
        ))
        .stdout(predicate::str::contains(
            "\"cross_mode_repository_command_exclusion\"",
        ))
        .stdout(predicate::str::contains(
            "\"weighted_composite_score_allowed\": false",
        ))
        .stdout(predicate::str::contains(
            "\"workflow_metric_asymmetry_policy\": \"retain the pair and report the difference\"",
        ))
        .stdout(predicate::str::contains(
            "\"zero_git_baseline_encoding\": \"null reduction with both raw mode values retained\"",
        ))
        .stdout(predicate::str::contains("executor-feature-override-sets"))
        .stdout(predicate::str::contains("executor-program-and-version"))
        .stdout(predicate::str::contains("\"claude_code_local_tools\""))
        .stdout(predicate::str::contains(
            "\"required_separate_read_only_shell_commands\": 30",
        ))
        .stdout(predicate::str::contains(
            "\"required_started_command_items\": 30",
        ))
        .stdout(predicate::str::contains(
            "\"unexpected_non_command_tool_items_allowed\": 0",
        ))
        .stdout(predicate::str::contains(
            "\"git_worktree_permission_probe_required\": true",
        ))
        .stdout(predicate::str::contains(
            "\"name\": \"ait_benchmark_local_v1\"",
        ))
        .stdout(predicate::str::contains(
            "\"danger_full_access_allowed\": false",
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

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "validate", "--manifest"])
        .arg(agent_token_path(
            "campaigns/agent-token-game-v1/smoke-sprint-on.json",
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"sprint_mode\": \"on\""))
        .stdout(predicate::str::contains("\"ait_server_connected\": false"));

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "validate", "--manifest"])
        .arg(agent_token_path(
            "campaigns/agent-token-game-v1/smoke-steady-state-claude.json",
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"executor\": \"claude\""))
        .stdout(predicate::str::contains(
            "\"tool_policy\": \"claude_code_local_tools\"",
        ))
        .stdout(predicate::str::contains("\"ait_server_connected\": false"));

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "validate", "--manifest"])
        .arg(agent_token_path(
            "campaigns/agent-token-game-v1/fable-max-sprint-on-smoke10.json",
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"model_id\": \"claude-fable-5\""))
        .stdout(predicate::str::contains(
            "\"executor_version\": \"2.1.235 (Claude Code)\"",
        ))
        .stdout(predicate::str::contains("\"ait_version\": \"ait 1.1.0\""))
        .stdout(predicate::str::contains("\"reasoning_effort\": \"max\""))
        .stdout(predicate::str::contains("\"scheduled_run_count\": 10"))
        .stdout(predicate::str::contains(
            "\"functional_replacement_policy\": \"none\"",
        ));

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "validate", "--manifest"])
        .arg(agent_token_path(
            "campaigns/agent-token-game-v1/fable-max-sprint-on-complete200.json",
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"campaign_scope\": \"complete\""))
        .stdout(predicate::str::contains("\"sprint_mode\": \"on\""))
        .stdout(predicate::str::contains("\"scheduled_run_count\": 200"))
        .stdout(predicate::str::contains(
            "\"functional_replacement_policy\": \"first_valid_unaccepted_lane_once\"",
        ));

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "validate", "--manifest"])
        .arg(agent_token_path(
            "campaigns/agent-token-game-v1/sol-max-codex-managed-complete200.json",
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"campaign_scope\": \"complete\""))
        .stdout(predicate::str::contains("\"scheduled_run_count\": 200"))
        .stdout(predicate::str::contains("\"model_id\": \"gpt-5.6-sol\""))
        .stdout(predicate::str::contains(
            "\"git_worktree_mode\": \"codex_app_equivalent_managed\"",
        ));
}

#[test]
fn agent_token_run_exposes_pair_slicing_and_rejects_retired_run_slicing() {
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "run", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--max-pairs"))
        .stdout(predicate::str::contains("--max-runs").not());

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args([
            "agent-token",
            "run",
            "--manifest",
            "missing.json",
            "--output-dir",
            "missing-output",
            "--max-runs",
            "2",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unexpected argument '--max-runs'"));

    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["agent-token", "resume", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--campaign-dir"))
        .stdout(predicate::str::contains("--max-pairs"))
        .stdout(predicate::str::contains("--adjudicate-transcripts"))
        .stdout(predicate::str::contains("--adjudicate-recovered-spawn"))
        .stdout(predicate::str::contains("--recover-infrastructure-pair"))
        .stdout(predicate::str::contains("--recover-host-shutdown-pair"))
        .stdout(predicate::str::contains("--max-runs").not());
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

    for (manifest_name, output_name, expected_count) in [
        (
            "fable-max-sprint-on-smoke10.json",
            "fable-smoke-schedule.json",
            10,
        ),
        (
            "fable-max-sprint-on-complete200.json",
            "fable-complete-schedule.json",
            200,
        ),
        (
            "sol-max-codex-managed-complete200.json",
            "sol-managed-complete-schedule.json",
            200,
        ),
    ] {
        let manifest_relative = format!("campaigns/agent-token-game-v1/{manifest_name}");
        Command::cargo_bin("ait-benchmark")
            .unwrap()
            .args(["agent-token", "schedule", "--manifest"])
            .arg(agent_token_path(&manifest_relative))
            .arg("--output")
            .arg(temp.path().join(output_name))
            .assert()
            .success()
            .stdout(predicate::str::contains(format!(
                "\"entry_count\": {expected_count}"
            )));
    }
    let smoke_schedule: JsonValue = serde_json::from_slice(
        &std::fs::read(temp.path().join("fable-smoke-schedule.json")).unwrap(),
    )
    .unwrap();
    let complete_schedule: JsonValue = serde_json::from_slice(
        &std::fs::read(temp.path().join("fable-complete-schedule.json")).unwrap(),
    )
    .unwrap();
    for (smoke, complete) in smoke_schedule["entries"]
        .as_array()
        .unwrap()
        .iter()
        .zip(complete_schedule["entries"].as_array().unwrap())
    {
        for field in [
            "workload_id",
            "mode",
            "attempt",
            "block_index",
            "randomized_order",
        ] {
            assert_eq!(smoke[field], complete[field]);
        }
    }
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
