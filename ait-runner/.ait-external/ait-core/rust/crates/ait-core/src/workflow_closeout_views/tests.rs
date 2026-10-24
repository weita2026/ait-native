use crate::json_support::json;

use super::{
    workflow_applied_action_summary, workflow_apply_phase_payload, workflow_apply_phase_summary,
    workflow_mutation_receipt_summary,
};

#[test]
fn apply_phase_payload_and_summary_match_reference() {
    let payload =
        workflow_apply_phase_payload("authoritative_resume", "done", Some("ready"), true).unwrap();
    assert_eq!(
        payload,
        json!({
            "phase":"authoritative_resume",
            "code":"done",
            "detail":"ready",
            "resumed_from_authoritative_state":true
        })
    );
    assert_eq!(
        workflow_apply_phase_summary(&payload).unwrap().as_deref(),
        Some("ready")
    );
}

#[test]
fn mutation_receipt_summary_matches_reference_shape() {
    let summary = workflow_mutation_receipt_summary(&json!({
        "action":"submit_land",
        "delivery":"response_recovery",
        "submission_id":"LAND-1",
        "status":"queued",
        "queued": true
    }))
    .unwrap();
    assert!(summary.contains("submit_land"));
    assert!(summary.contains("authoritative recovery"));
}

#[test]
fn applied_action_summary_matches_auto_rebase_and_cleanup_reference() {
    let publish = workflow_applied_action_summary(&json!({
        "code":"publish_patchset",
        "result":{
            "patchset_id":"RP-1",
            "auto_rebase":{"rebase":{"status":"applied"}}
        }
    }))
    .unwrap();
    assert!(publish.contains("auto-rebase"));

    let land = workflow_applied_action_summary(&json!({
        "code":"submit_land",
        "result":{
            "submission_id":"LAND-1",
            "status":"landed",
            "bound_worktree_cleanup":{"status":"removed","worktree":{"name":"rt-1"}}
        }
    }))
    .unwrap();
    assert!(land.contains("removed bound worktree"));
}
