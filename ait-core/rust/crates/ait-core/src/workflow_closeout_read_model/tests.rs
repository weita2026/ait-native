use super::*;
use crate::json_support::json;

#[test]
fn workflow_ready_blocks_external_readiness_before_ci() {
    let facts = json!({
        "change": {"change_id": "LCC-1", "base_line": "main"},
        "task": {"task_id": "LCT-1", "status": "active"},
        "patchset": {"patchset_id": "LCP-1"},
        "workspace": {
            "clean": true,
            "current_line": "feature/lct-1",
            "workspace_matches_patchset": true
        },
        "freshness": {"base_is_fresh": true},
        "attestation": null,
        "policy": null,
        "external_readiness": {
            "ready": false,
            "blockers": [{
                "code": "external_lock_missing",
                "name": "ait-db",
                "path": "ait-external.lock",
                "message": "ait-external.lock is missing direct external \"ait-db\""
            }]
        },
        "payload_seed": {}
    });
    let commands = json!({
        "apply_command": "ait workflow ready LCC-1 --apply",
        "publish_command": "ait patchset publish LCC-1",
        "patchset_ci_command": "ait patchset rerun-ci LCP-1",
        "attestation_command": "ait attest put LCP-1",
    });

    let model = project_workflow_ready_read_model(&facts, &commands, false, false, false).unwrap();
    let external_step = model["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["code"] == "external")
        .unwrap();

    assert_eq!(
        model["next_action"]["code"],
        json!("external_readiness_blocked")
    );
    assert_eq!(external_step["status"], json!("blocked"));
    assert!(external_step["detail"]
        .as_str()
        .unwrap()
        .contains("external_lock_missing ait-db ait-external.lock"));
    assert_eq!(model["suggested_commands"], json!(["ait external doctor"]));
}

#[test]
fn completed_local_ready_keeps_selected_patchset_authoritative_over_dirty_workspace() {
    let facts = json!({
        "change": {"change_id": "RCC-1", "base_line": "main", "status": "active"},
        "task": {"task_id": "RCT-1", "status": "active"},
        "patchset": {
            "patchset_id": "RCP-1",
            "base_snapshot_id": "SNP-BASE",
            "revision_snapshot_id": "SNP-LANDED"
        },
        "workspace": {
            "clean": false,
            "changed_count": 1,
            "changed_paths": ["unrelated.log"],
            "workspace_matches_patchset": false
        },
        "freshness": {"base_is_fresh": true},
        "attestation": null,
        "policy": null,
        "external_readiness": null,
        "payload_seed": {
            "ignore_workspace_authoring": true,
            "patchset_is_authoritative": true
        }
    });
    let commands = json!({
        "apply_command": "ait workflow ready RCC-1 --apply",
        "publish_command": "ait patchset publish RCC-1",
        "patchset_ci_command": "ait patchset rerun-ci RCP-1",
        "attestation_command": "ait attest put RCP-1",
    });

    let model = project_workflow_ready_read_model(&facts, &commands, true, true, false).unwrap();

    assert_ne!(model["next_action"]["code"], json!("snapshot_create"));
    assert_ne!(model["next_action"]["code"], json!("refresh_patchset"));
    assert_eq!(model["ignore_workspace_authoring"], json!(true));
    assert_eq!(model["patchset_is_authoritative"], json!(true));
    let snapshot_step = model["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["code"] == "snapshot")
        .expect("snapshot step");
    assert_eq!(snapshot_step["status"], json!("done"));
    assert!(snapshot_step["command"].is_null());
}

#[test]
fn ancestry_recovery_does_not_offer_a_patchset_republish_command() {
    let facts = json!({
        "change": {"change_id": "RCC-1", "base_line": "main", "status": "active"},
        "task": {"task_id": "RCT-1", "status": "active"},
        "patchset": {"patchset_id": "RCP-1"},
        "workspace": {"clean": true, "workspace_matches_patchset": false},
        "freshness": {"base_is_fresh": true},
        "patchset_refresh": {
            "reason_code": "current_head_behind_patchset",
            "republish_allowed": false,
            "summary": "Restore the selected Patchset revision before continuing.",
            "detail": "The current head is older than the selected revision; do not republish it."
        },
        "attestation": null,
        "policy": null,
        "external_readiness": null,
        "payload_seed": {}
    });
    let commands = json!({
        "apply_command": "ait workflow ready RCC-1 --apply",
        "publish_command": "ait patchset publish RCC-1",
        "attestation_command": "ait attest put RCP-1"
    });

    let model = project_workflow_ready_read_model(&facts, &commands, false, false, true).unwrap();
    let patchset_step = model["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["code"] == "patchset")
        .unwrap();

    assert_eq!(
        model["next_action"]["code"],
        json!("patchset_recovery_required")
    );
    assert!(model["next_action"]["command"].is_null());
    assert!(patchset_step["command"].is_null());
    assert_eq!(model["suggested_commands"], json!([]));
}

#[test]
fn workflow_land_full_read_model_preserves_patchset_ci_status() {
    let facts = json!({
        "change": {"change_id": "RCC-1", "base_line": "main"},
        "task": {"task_id": "RCT-1", "status": "active"},
        "patchset": {"patchset_id": "RCP-1"},
        "workspace": {"clean": true, "changed_count": 0},
        "current_line_name": "feature/rct-1",
        "revision_snapshot_id": "SNP-REV",
        "base_line_name": "main",
        "target_line": "main",
        "remote_base_snapshot_id": "SNP-BASE",
        "review_summary": [],
        "attestation": {
            "attestation_id": "ATT-1",
            "evaluation_summary": {"tests": "pass"}
        },
        "patchset_ci_status": {
            "available": true,
            "tests_status": "pass",
            "latest_job": {
                "job_id": 1,
                "job_type": "patchset.ci",
                "state": "succeeded"
            },
            "selected_suite_ids": ["rust_core"],
            "suite_results": [{"suite_id": "rust_core", "status": "pass"}]
        },
        "policy": null,
        "landing_summary": null,
        "tests_state": "pass",
        "patchset_base_snapshot_id": "SNP-BASE",
        "patchset_revision_snapshot_id": "SNP-REV",
        "base_is_fresh": true,
        "workspace_matches_patchset": true,
        "review_blocking": 0,
        "review_approvals": 0,
        "task_review_approvals": 1,
        "team_review_approvals": 0,
        "code_review_summary_count": 0,
        "policy_decision": "pass",
        "requires_code_review_summary": false,
        "landing_status": "",
        "ignore_workspace_authoring": false,
        "patchset_is_authoritative": false,
        "resolved_change_id": "RCC-1"
    });
    let commands = json!({
        "ready_command": "ait workflow ready RCC-1 --apply",
        "patchset_ci_command": "ait patchset rerun-ci RCP-1",
        "land_command": "ait workflow finish RCC-1 --apply"
    });

    let model =
        project_workflow_land_full_read_model(&facts, &commands, &json!({"value": true}), false)
            .unwrap();

    assert_eq!(
        model["patchset_ci_status"]["latest_job"]["job_type"],
        json!("patchset.ci")
    );
    assert_eq!(
        model["patchset_ci_status"]["selected_suite_ids"],
        json!(["rust_core"])
    );
    assert_eq!(model["review"]["task_approvals"], json!(1));
}
