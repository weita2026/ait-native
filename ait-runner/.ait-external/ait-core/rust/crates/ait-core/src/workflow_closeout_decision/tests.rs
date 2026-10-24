use super::*;

fn commands_with_patchset_ci() -> JsonValue {
    json!({
        "apply_command": "ait workflow ready RCC-1 --apply",
        "patchset_ci_command": "ait patchset rerun-ci RCP-1",
        "land_command": "ait workflow finish RCC-1 --apply",
    })
}

fn attestation_pass() -> JsonValue {
    json!({
        "attestation_id": "AT-RCP-1",
        "evaluation_summary": {
            "tests": "pass"
        }
    })
}

fn pending_without_ci_job() -> JsonValue {
    json!({
        "available": true,
        "tests_status": "pending",
        "latest_job": {
            "job_id": 42,
            "job_type": "main-seed.refresh",
            "state": "succeeded"
        },
        "recent_jobs": [{
            "job_id": 42,
            "job_type": "main-seed.refresh",
            "state": "succeeded"
        }],
        "selected_suite_ids": [],
        "suite_results": []
    })
}

fn passing_ci_status() -> JsonValue {
    json!({
        "available": true,
        "ci_run_seq": 1,
        "ci_completed_at_s": 1_783_814_400_u64,
        "tests_status": "pass",
        "has_runnable_evidence": true,
        "suite_result_count": 1,
        "blocking_failure_count": 0,
        "latest_job": {
            "job_id": 43,
            "job_type": "patchset.ci",
            "state": "succeeded"
        },
        "selected_suite_ids": ["rust_core"],
        "suite_results": [{
            "suite_id": "rust_core",
            "status": "pass"
        }]
    })
}

fn running_ci_status_with_previous_pass() -> JsonValue {
    json!({
        "available": true,
        "ci_run_seq": 2,
        "ci_completed_at_s": null,
        "tests_status": "pending",
        "latest_job": {
            "job_id": 44,
            "job_type": "patchset.ci",
            "state": "running",
            "diagnostic_status": "running"
        },
        "recent_jobs": [{
            "job_id": 44,
            "job_type": "patchset.ci",
            "state": "running",
            "diagnostic_status": "running"
        }, {
            "job_id": 43,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "diagnostic_status": "succeeded"
        }],
        "selected_suite_ids": ["rust_core"],
        "suite_results": [{
            "suite_id": "rust_core",
            "status": "pass"
        }]
    })
}

fn passing_ci_status_with_stale_queued_previous_job() -> JsonValue {
    json!({
        "available": true,
        "ci_run_seq": 2,
        "ci_completed_at_s": 1_783_814_500_u64,
        "tests_status": "pass",
        "has_runnable_evidence": true,
        "suite_result_count": 1,
        "blocking_failure_count": 0,
        "latest_job": {
            "job_id": 45,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "diagnostic_status": "succeeded"
        },
        "recent_jobs": [{
            "job_id": 44,
            "job_type": "patchset.ci",
            "state": "queued",
            "diagnostic_status": "queued"
        }, {
            "job_id": 45,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "diagnostic_status": "succeeded"
        }],
        "selected_suite_ids": ["rust_core"],
        "suite_results": [{
            "suite_id": "rust_core",
            "status": "pass"
        }]
    })
}

fn ready_facts(patchset_ci_status: JsonValue) -> JsonValue {
    json!({
        "change": {
            "change_id": "RCC-1",
            "status": "draft"
        },
        "task": {
            "status": "active"
        },
        "workspace": {
            "clean": true
        },
        "patchset": {
            "patchset_id": "RCP-1"
        },
        "freshness": {
            "base_is_fresh": true
        },
        "attestation": attestation_pass(),
        "patchset_ci_status": patchset_ci_status,
        "tests_state": "pass",
        "review_blocking": 0,
        "task_review_approvals": 1,
        "task_review_enabled": false,
        "policy": {
            "decision": "pass"
        }
    })
}

#[test]
fn workflow_ready_hands_pending_review_and_policy_to_workflow_land() {
    let mut facts = ready_facts(passing_ci_status());
    facts["requires_code_review_summary"] = json!(true);
    facts["code_review_summary_count"] = json!(0);
    facts["task_review_approvals"] = json!(0);
    facts["policy"] = json!({"decision": "pending"});
    let commands = json!({
        "apply_command": "ait workflow ready RCC-1 --apply",
        "auto_review_reviewer": "alice",
        "review_command": "ait workflow finish RCC-1 --apply",
        "land_command": "ait workflow finish RCC-1 --apply"
    });

    let action = workflow_ready_next_action(&facts, &commands, false, false, true);

    assert_eq!(action["code"], json!("done"));
    assert_eq!(
        action["command"],
        json!("ait workflow finish RCC-1 --apply")
    );
    assert!(action["detail"]
        .as_str()
        .unwrap()
        .contains("reviewer-owned `workflow finish`"));
}

#[test]
fn workflow_ready_does_not_claim_ai_code_review() {
    let mut facts = ready_facts(passing_ci_status());
    facts["requires_code_review_summary"] = json!(true);
    facts["code_review_summary_count"] = json!(0);
    let commands = json!({
        "apply_command": "ait workflow ready RCC-1 --apply",
        "land_command": "ait workflow finish RCC-1 --apply"
    });

    let action = workflow_ready_next_action(&facts, &commands, false, false, true);

    assert_eq!(action["code"], json!("done"));
    assert_eq!(
        action["command"],
        json!("ait workflow finish RCC-1 --apply")
    );
}

#[test]
fn workflow_ready_does_not_claim_final_policy_evaluation() {
    let mut facts = ready_facts(passing_ci_status());
    facts["policy"] = json!({"decision": "pending"});
    let commands = json!({
        "apply_command": "ait workflow ready RCC-1 --apply",
        "policy_command": "ait policy eval RCP-1",
        "land_command": "ait workflow finish RCC-1 --apply"
    });

    let action = workflow_ready_next_action(&facts, &commands, false, false, true);

    assert_eq!(action["code"], json!("done"));
    assert_eq!(
        action["command"],
        json!("ait workflow finish RCC-1 --apply")
    );
}

#[test]
fn workflow_ready_fails_closed_when_current_head_is_not_a_proven_newer_revision() {
    let mut facts = ready_facts(passing_ci_status());
    facts["workspace"]["workspace_matches_patchset"] = json!(false);
    facts["patchset_refresh"] = json!({
        "reason_code": "current_head_behind_patchset",
        "republish_allowed": false,
        "summary": "Restore the selected Patchset revision before continuing.",
        "detail": "The current head is an ancestor of the selected Patchset revision; do not republish it."
    });
    let commands = json!({
        "apply_command": "ait workflow ready RCC-1 --apply",
        "publish_command": "ait patchset publish RCC-1",
        "land_command": "ait workflow finish RCC-1 --apply"
    });

    let action = workflow_ready_next_action(&facts, &commands, false, false, true);

    assert_eq!(action["code"], json!("patchset_recovery_required"));
    assert!(action["command"].is_null());
    assert_eq!(action["refresh_context"]["republish_allowed"], json!(false));
    assert!(action["detail"]
        .as_str()
        .unwrap()
        .contains("do not republish"));
}

fn land_facts(patchset_ci_status: JsonValue) -> JsonValue {
    json!({
        "change": {
            "change_id": "RCC-1",
            "status": "draft"
        },
        "task": {
            "status": "active"
        },
        "workspace": {
            "clean": true
        },
        "patchset": {
            "patchset_id": "RCP-1"
        },
        "attestation": attestation_pass(),
        "patchset_ci_status": patchset_ci_status,
        "tests_state": "pass",
        "review_blocking": 0,
        "target_line": "main",
        "ignore_workspace_authoring": false,
        "policy_decision": "pass",
        "task_review_approvals": 1,
        "landing_status": ""
    })
}

#[test]
fn workflow_land_owns_exact_patchset_ai_review_before_task_land() {
    let mut facts = land_facts(passing_ci_status());
    facts["requires_code_review_summary"] = json!(true);
    facts["code_review_summary_count"] = json!(0);
    facts["task_review_approvals"] = json!(0);
    let commands = json!({
        "apply_command": "ait workflow finish RCC-1 --apply",
        "ready_command": "ait workflow ready RCC-1 --apply",
        "code_review_summary_command": "ait workflow finish RCC-1 --apply --review-message \"structured summary\"",
        "auto_review_reviewer": "alice",
        "review_command": "ait workflow finish RCC-1 --apply",
        "land_command": "ait task finish RCC-1"
    });

    let action = workflow_land_next_action(
        &facts, &commands, true, false, true, true, None, "pass", None, false,
    );

    assert_eq!(action["code"], json!("record_code_review_summary"));
    assert_eq!(
        action["command"],
        json!("ait workflow finish RCC-1 --apply --review-message \"structured summary\"")
    );
}

#[test]
fn workflow_land_owns_automatic_task_approval() {
    let mut facts = land_facts(passing_ci_status());
    facts["task_review_approvals"] = json!(0);
    let commands = json!({
        "apply_command": "ait workflow finish RCC-1 --apply",
        "ready_command": "ait workflow ready RCC-1 --apply",
        "auto_review_reviewer": "alice",
        "review_command": "ait workflow finish RCC-1 --apply",
        "land_command": "ait task finish RCC-1"
    });

    let action = workflow_land_next_action(
        &facts, &commands, true, false, true, true, None, "pass", None, false,
    );

    assert_eq!(action["code"], json!("record_review"));
    assert_eq!(
        action["command"],
        json!("ait workflow finish RCC-1 --apply")
    );
}

#[test]
fn workflow_land_owns_pending_policy_evaluation() {
    let facts = land_facts(passing_ci_status());
    let pending_policy = json!({"decision": "pending"});
    let commands = json!({
        "apply_command": "ait workflow finish RCC-1 --apply",
        "ready_command": "ait workflow ready RCC-1 --apply",
        "policy_command": "ait policy eval RCP-1",
        "land_command": "ait task finish RCC-1"
    });

    let action = workflow_land_next_action(
        &facts,
        &commands,
        true,
        false,
        true,
        true,
        pending_policy.as_object().cloned(),
        "pending",
        None,
        false,
    );

    assert_eq!(action["code"], json!("evaluate_policy"));
    assert_eq!(
        action["command"],
        json!("ait workflow finish RCC-1 --apply")
    );
}

#[test]
fn workflow_ready_requires_remote_ci_evidence_despite_attestation_pass() {
    let action = workflow_ready_next_action(
        &ready_facts(pending_without_ci_job()),
        &commands_with_patchset_ci(),
        false,
        false,
        false,
    );

    assert_eq!(action["code"], json!("run_patchset_ci"));
    assert!(action["detail"]
        .as_str()
        .unwrap()
        .contains("embedded in the selected Patchset"));
}

#[test]
fn workflow_ready_accepts_completed_patchset_ci_with_compact_attestation() {
    let action = workflow_ready_next_action(
        &ready_facts(passing_ci_status()),
        &commands_with_patchset_ci(),
        false,
        false,
        false,
    );

    assert_eq!(action["code"], json!("done"));
}

#[test]
fn workflow_ready_rejects_postgres_pass_without_completed_patchset_state() {
    let mut status = passing_ci_status();
    status["ci_completed_at_s"] = JsonValue::Null;
    let action = workflow_ready_next_action(
        &ready_facts(status),
        &commands_with_patchset_ci(),
        false,
        false,
        false,
    );

    assert_eq!(action["code"], json!("run_patchset_ci"));
}

#[test]
fn workflow_ready_records_compact_attestation_after_patchset_ci() {
    let mut facts = ready_facts(passing_ci_status());
    facts["attestation"] = JsonValue::Null;

    let action =
        workflow_ready_next_action(&facts, &commands_with_patchset_ci(), false, false, false);

    assert_eq!(action["code"], json!("record_attestation"));
    assert_eq!(action["command"], json!("ait workflow ready RCC-1 --apply"));
}

#[test]
fn workflow_ready_without_ci_contract_keeps_manual_attestation_command() {
    let mut facts = ready_facts(JsonValue::Null);
    facts["attestation"] = JsonValue::Null;
    facts["tests_state"] = json!("");
    let commands = json!({
        "apply_command": "ait workflow ready RCC-1 --apply",
        "attest_command": "ait attest put RCP-1 --tests pass",
        "land_command": "ait task finish RCC-1",
    });

    let action = workflow_ready_next_action(&facts, &commands, false, false, false);

    assert_eq!(action["code"], json!("record_attestation"));
    assert_eq!(
        action["command"],
        json!("ait attest put RCP-1 --tests pass")
    );
}

#[test]
fn workflow_ci_gate_accepts_explicit_bounded_readiness_evidence() {
    let status = json!({
        "ci_run_seq": 1,
        "ci_completed_at_s": 1_783_814_400_u64,
        "tests_status": "pass",
        "has_runnable_evidence": true,
        "selected_suite_ids": [],
        "latest_job": null,
        "recent_jobs": []
    });

    assert_eq!(
        patchset_ci_gate_state(status.as_object()),
        PatchsetCiGateState::Pass
    );
}

#[test]
fn workflow_ci_gate_honors_explicit_missing_readiness_evidence() {
    let status = json!({
        "ci_run_seq": 1,
        "ci_completed_at_s": 1_783_814_400_u64,
        "tests_status": "pass",
        "has_runnable_evidence": false,
        "selected_suite_ids": ["legacy-suite"],
        "latest_job": null,
        "recent_jobs": []
    });

    assert_eq!(
        patchset_ci_gate_state(status.as_object()),
        PatchsetCiGateState::NeedsRun
    );
}

#[test]
fn workflow_ready_waits_for_running_ci_even_when_previous_pass_exists() {
    let action = workflow_ready_next_action(
        &ready_facts(running_ci_status_with_previous_pass()),
        &commands_with_patchset_ci(),
        false,
        false,
        false,
    );

    assert_eq!(action["code"], json!("waiting_for_ci"));
}

#[test]
fn workflow_ready_uses_latest_pass_when_previous_job_remains_queued() {
    let action = workflow_ready_next_action(
        &ready_facts(passing_ci_status_with_stale_queued_previous_job()),
        &commands_with_patchset_ci(),
        false,
        false,
        false,
    );

    assert_eq!(action["code"], json!("done"));
}

#[test]
fn workflow_land_blocks_attestation_only_pass_without_remote_ci() {
    let action = workflow_land_next_action(
        &land_facts(pending_without_ci_job()),
        &commands_with_patchset_ci(),
        false,
        false,
        true,
        true,
        None,
        "pass",
        None,
        true,
    );

    assert_eq!(action["code"], json!("workflow_ready"));
    assert!(action["detail"]
        .as_str()
        .unwrap()
        .contains("Remote patchset CI evidence is missing"));
}
