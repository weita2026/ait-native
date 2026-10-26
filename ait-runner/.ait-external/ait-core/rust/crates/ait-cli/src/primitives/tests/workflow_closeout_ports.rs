use super::*;

#[test]
fn workflow_closeout_action_helpers_accept_closeout_remote_trait() {
    let patchset = json!({
        "patchset_id": "RCP-CLOSEOUT-1",
        "change_id": "RCC-CLOSEOUT",
    });
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();
    closeout_remote
        .patchsets
        .insert("RCP-CLOSEOUT-1".to_string(), patchset.clone());

    let ci = workflow_run_patchset_ci_action_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        "fixture-ait",
        "missing patchset",
        "workflow_custom_apply",
        Some("foreground"),
    )
    .expect("run shared workflow CI helper through closeout remote trait");
    assert_eq!(ci["result"]["patchset_id"], json!("RCP-CLOSEOUT-1"));
    assert_eq!(ci["result"]["trigger"], json!("workflow_custom_apply"));
    assert_eq!(ci["result"]["execution_profile"], json!("foreground"));
    assert_eq!(ci["result"]["repo_name"], json!("fixture-ait"));
    assert_eq!(closeout_remote.ci_runs.len(), 1);

    let attestation = workflow_record_attestation_action_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        None,
        Some("fmt"),
        None,
        None,
        "ai_with_human_review",
        Some("gpt-5".to_string()),
        "fixture-ait",
        "missing patchset",
        Some("pass"),
    )
    .expect("record shared workflow attestation helper through closeout remote trait");
    assert_eq!(
        attestation["result"]["evaluation_summary"]["tests"],
        json!("pass")
    );
    assert_eq!(
        attestation["result"]["evaluation_summary"]["lint"],
        json!("fmt")
    );
    assert_eq!(
        attestation["result"]["author_mode"],
        json!("ai_with_human_review")
    );
    assert_eq!(attestation["result"]["repo_name"], json!("fixture-ait"));
    assert_eq!(closeout_remote.attestations.len(), 1);
    assert!(closeout_remote.attestations.contains_key("RCP-CLOSEOUT-1"));
}

#[test]
fn workflow_review_action_helper_accepts_closeout_remote_trait() {
    let patchset = json!({
        "patchset_id": "RCP-REVIEW-1",
        "change_id": "RCC-REVIEW",
    });
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();
    closeout_remote
        .patchsets
        .insert("RCP-REVIEW-1".to_string(), patchset.clone());

    let task_review = workflow_land_record_task_review_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        "RCC-REVIEW",
        "Reviewer <reviewer@example.com>",
        "fixture-ait",
    )
    .expect("record task review through shared closeout remote trait helper");
    assert_eq!(task_review["result"]["action"], json!("task_approve"));
    assert_eq!(
        task_review["result"]["policy_refresh"]["decision"],
        json!("pass")
    );
    assert_eq!(closeout_remote.policy_evaluations.len(), 1);

    let code_review = workflow_record_review_action_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        "RCC-REVIEW",
        "Agent Reviewer <agent@example.com>",
        "code_review_summary",
        Some("Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: cargo test; Recommendation: land"),
        false,
        "fixture-ait",
        "missing patchset",
    )
    .expect("record code review through shared closeout remote trait helper");
    assert_eq!(
        code_review["result"]["action"],
        json!("code_review_summary")
    );
    assert_eq!(
        code_review["result"]["comment"],
        json!("Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: cargo test; Recommendation: land")
    );
    assert!(code_review["result"].get("policy_refresh").is_none());
    assert_eq!(closeout_remote.policy_evaluations.len(), 1);
    assert_eq!(
        closeout_remote.reviews.get("RCC-REVIEW").map(Vec::len),
        Some(2)
    );
}

#[test]
fn workflow_ready_remote_actions_accept_closeout_remote_trait() {
    let patchset = json!({
        "patchset_id": "RCP-READY-1",
        "change_id": "RCC-READY",
    });
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();
    closeout_remote
        .patchsets
        .insert("RCP-READY-1".to_string(), patchset.clone());

    let ci = workflow_ready_run_patchset_ci_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        "fixture-ait",
    )
    .expect("run ready CI through closeout remote trait");

    assert_eq!(ci["result"]["patchset_id"], json!("RCP-READY-1"));
    assert_eq!(ci["result"]["trigger"], json!("workflow_ready_apply"));
    assert_eq!(
        ci["result"]["execution_profile"],
        json!(PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND)
    );
    assert_eq!(ci["result"]["repo_name"], json!("fixture-ait"));
    assert_eq!(closeout_remote.ci_runs.len(), 1);

    let attestation = workflow_ready_record_attestation_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        None,
        Some("fmt"),
        None,
        None,
        "ai_with_human_review",
        Some("gpt-5".to_string()),
        "fixture-ait",
    )
    .expect("record ready attestation through closeout remote trait");

    assert_eq!(
        attestation["result"]["evaluation_summary"]["tests"],
        json!("pass")
    );
    assert_eq!(
        attestation["result"]["evaluation_summary"]["lint"],
        json!("fmt")
    );
    assert_eq!(
        attestation["result"]["author_mode"],
        json!("ai_with_human_review")
    );
    assert_eq!(attestation["result"]["repo_name"], json!("fixture-ait"));
    assert!(attestation["result"]["detail"].get("patchset_ci").is_none());
    assert_eq!(closeout_remote.attestations.len(), 1);
    assert!(closeout_remote.attestations.contains_key("RCP-READY-1"));
}

#[test]
fn workflow_land_closeout_actions_accept_closeout_remote_trait() {
    let patchset = json!({
        "patchset_id": "RCP-LAND-1",
        "change_id": "RCC-LAND",
    });
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();
    closeout_remote
        .patchsets
        .insert("RCP-LAND-1".to_string(), patchset.clone());

    let attestation = workflow_land_record_attestation_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        Some("pass"),
        Some("fmt"),
        None,
        None,
        "ai_with_human_review",
        Some("gpt-5".to_string()),
        "fixture-ait",
    )
    .expect("record land attestation through closeout remote trait");
    assert_eq!(
        attestation["result"]["evaluation_summary"]["tests"],
        json!("pass")
    );
    assert_eq!(
        attestation["result"]["evaluation_summary"]["lint"],
        json!("fmt")
    );

    let review = workflow_land_record_task_review_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        "RCC-LAND",
        "Reviewer <reviewer@example.com>",
        "fixture-ait",
    )
    .expect("record land task approval through closeout remote trait");
    assert_eq!(review["result"]["action"], json!("task_approve"));
    assert_eq!(
        review["result"]["policy_refresh"]["decision"],
        json!("pass")
    );
    assert_eq!(closeout_remote.policy_evaluations.len(), 1);

    let code_review = workflow_land_record_code_review_summary_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        "RCC-LAND",
        "Agent Reviewer <agent@example.com>",
        "Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: cargo test; Recommendation: land",
        "fixture-ait",
    )
    .expect("record land code review through closeout remote trait");
    assert_eq!(
        code_review["result"]["action"],
        json!("code_review_summary")
    );
    assert_eq!(
        code_review["result"]["comment"],
        json!(
            "Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: cargo test; Recommendation: land"
        )
    );

    let policy = workflow_land_evaluate_policy_with_closeout_remote(
        &mut closeout_remote,
        &patchset,
        "fixture-ait",
    )
    .expect("evaluate land policy through closeout remote trait");
    assert_eq!(policy["result"]["decision"], json!("pass"));
    assert_eq!(closeout_remote.policy_evaluations.len(), 2);
    assert_eq!(
        closeout_remote.reviews.get("RCC-LAND").map(Vec::len),
        Some(2)
    );
}

#[test]
fn workflow_apply_support_helpers_preserve_json_contracts() {
    let state = json!({
        "status": " review ",
        "change": {"change_id": " RCC-APPLY "},
        "patchset": {"patchset_id": " RCP-APPLY "},
        "next_action": {
            "code": " waiting_for_ci ",
            "detail": " CI pending "
        }
    });

    assert_eq!(
        workflow_root_text(&state, "status").as_deref(),
        Some("review")
    );
    assert_eq!(
        workflow_nested_text(&state, "next_action", "code").as_deref(),
        Some("waiting_for_ci")
    );
    assert_eq!(
        workflow_current_ids(&state),
        (Some("RCC-APPLY".to_string()), Some("RCP-APPLY".to_string()))
    );
    assert_eq!(
        workflow_json_text(Some(&json!(" done "))).as_deref(),
        Some("done")
    );
    assert_eq!(workflow_json_text(Some(&json!("   "))), None);
    assert_eq!(workflow_json_text(Some(&json!(17))), None);

    assert_eq!(
        workflow_apply_phase_payload_json(
            " pending_gate ",
            " waiting_for_ci ",
            Some(" CI pending "),
            true,
        ),
        json!({
            "phase": "pending_gate",
            "code": "waiting_for_ci",
            "detail": "CI pending",
            "resumed_from_authoritative_state": true
        })
    );
    assert_eq!(
        workflow_apply_phase_payload_json("stopped", "waiting_for_ci", Some("   "), false),
        json!({
            "phase": "stopped",
            "code": "waiting_for_ci"
        })
    );

    let estimated_wait = json!({"seconds": 2});
    let mut progress_events = Vec::new();
    {
        let mut progress = Some(|event: &JsonValue| {
            progress_events.push(event.clone());
            Ok(())
        });
        workflow_progress_emit(
            &mut progress,
            "applied",
            "run_patchset_ci",
            Some(" RCC-APPLY "),
            Some("   "),
            Some(2),
            Some(" queued "),
            Some(" action "),
            Some("   "),
            Some(&estimated_wait),
            Some(" done "),
        )
        .expect("emit progress event");
    }
    assert_eq!(
        progress_events,
        vec![json!({
            "status": "applied",
            "code": "run_patchset_ci",
            "change_id": "RCC-APPLY",
            "step_number": 2,
            "detail": "queued",
            "phase": "action",
            "estimated_wait": {"seconds": 2},
            "summary": "done"
        })]
    );

    type WorkflowProgressCallback = fn(&JsonValue) -> Result<(), String>;
    let mut no_progress: Option<WorkflowProgressCallback> = None;
    workflow_progress_emit(
        &mut no_progress,
        "applied",
        "run_patchset_ci",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .expect("missing progress callback is a no-op");
}
