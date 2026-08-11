use crate::attest_json::AttestJson;
use crate::json_support::{json, JsonMap, JsonValue};

use crate::workflow_closeout_command_hints::workflow_ready_apply_command;

pub fn workflow_ready_tests_state(
    attestation: Option<&JsonValue>,
    policy: Option<&JsonValue>,
) -> String {
    let mut tests_state = AttestJson::stateless()
        .tests_state_from_attestation(attestation)
        .unwrap_or_default();
    if tests_state.is_empty() {
        if let Some(JsonValue::Object(policy)) = policy {
            if let Some(JsonValue::Array(checks)) = policy.get("checks") {
                for check in checks {
                    let JsonValue::Object(check) = check else {
                        continue;
                    };
                    if optional_string(check.get("name")).as_deref() != Some("tests") {
                        continue;
                    }
                    tests_state = optional_string(check.get("status")).unwrap_or_default();
                    break;
                }
            }
        }
    }
    tests_state
}

pub fn workflow_landed_facts(state: &JsonValue) -> Result<JsonValue, String> {
    let state = require_object(Some(state), "state")?;
    let change = payload_dict(state.get("change"));
    let task = payload_dict(state.get("task"));
    let patchset = state
        .get("patchset")
        .and_then(JsonValue::as_object)
        .cloned()
        .map(JsonValue::Object)
        .unwrap_or(JsonValue::Null);
    let workspace = payload_dict(state.get("workspace"));
    Ok(json!({
        "landed": state.get("landed").and_then(JsonValue::as_bool).unwrap_or(false),
        "change": JsonValue::Object(change.clone()),
        "task": JsonValue::Object(task.clone()),
        "patchset": patchset,
        "patchset_source": optional_string(state.get("patchset_source")),
        "landing_summary": clone_if_object(state.get("landing_summary")),
        "workspace": JsonValue::Object(workspace.clone()),
        "current_line_name": optional_string(state.get("current_line_name"))
            .or_else(|| optional_string(workspace.get("current_line")))
            .unwrap_or_else(|| "unknown".to_string()),
        "revision_snapshot_id": optional_string(state.get("revision_snapshot_id"))
            .or_else(|| optional_string(workspace.get("baseline_snapshot_id"))),
        "target_line": optional_string(state.get("target_line"))
            .or_else(|| optional_string(change.get("base_line")))
            .unwrap_or_else(|| "main".to_string()),
        "patchset_label": state
            .get("patchset")
            .and_then(JsonValue::as_object)
            .and_then(|patchset| optional_string(patchset.get("patchset_id")))
            .unwrap_or_default(),
        "patchset_revision_snapshot_id": state
            .get("patchset")
            .and_then(JsonValue::as_object)
            .and_then(|patchset| optional_string(patchset.get("revision_snapshot_id"))),
        "ignore_workspace_authoring": state
            .get("ignore_workspace_authoring")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
    }))
}

pub fn workflow_land_full_facts(state: &JsonValue) -> Result<JsonValue, String> {
    let state = require_object(Some(state), "state")?;
    let change = payload_dict(state.get("change"));
    let task = payload_dict(state.get("task"));
    let workspace = payload_dict(state.get("workspace"));
    Ok(json!({
        "change": JsonValue::Object(change.clone()),
        "task": JsonValue::Object(task),
        "patchset": clone_if_object(state.get("patchset")),
        "patchset_source": optional_string(state.get("patchset_source")),
        "workspace": JsonValue::Object(workspace.clone()),
        "current_line_name": optional_string(state.get("current_line_name"))
            .or_else(|| optional_string(workspace.get("current_line")))
            .unwrap_or_else(|| "unknown".to_string()),
        "revision_snapshot_id": optional_string(state.get("revision_snapshot_id"))
            .or_else(|| optional_string(workspace.get("baseline_snapshot_id"))),
        "base_line_name": optional_string(state.get("base_line_name"))
            .or_else(|| optional_string(change.get("base_line")))
            .unwrap_or_else(|| "main".to_string()),
        "target_line": optional_string(state.get("target_line"))
            .or_else(|| optional_string(change.get("base_line")))
            .unwrap_or_else(|| "main".to_string()),
        "remote_base_snapshot_id": optional_string(state.get("remote_base_snapshot_id")),
        "review_summary": JsonValue::Object(payload_dict(state.get("review_summary"))),
        "attestation": clone_if_object(state.get("attestation")),
        "patchset_ci_status": clone_if_object(state.get("patchset_ci_status")),
        "policy": clone_if_object(state.get("policy")),
        "landing_summary": clone_if_object(state.get("landing_summary")),
        "tests_state": optional_string(state.get("tests_state")).unwrap_or_default(),
        "patchset_base_snapshot_id": optional_string(state.get("patchset_base_snapshot_id")),
        "patchset_revision_snapshot_id": optional_string(state.get("patchset_revision_snapshot_id")),
        "base_is_fresh": state.get("base_is_fresh").and_then(JsonValue::as_bool).unwrap_or(false),
        "workspace_matches_patchset": state.get("workspace_matches_patchset").cloned().unwrap_or(JsonValue::Null),
        "review_blocking": state.get("review_blocking").and_then(JsonValue::as_i64).unwrap_or(0),
        "review_approvals": state.get("review_approvals").and_then(JsonValue::as_i64).unwrap_or(0),
        "task_review_approvals": state.get("task_review_approvals").and_then(JsonValue::as_i64).unwrap_or(0),
        "team_review_approvals": state.get("team_review_approvals").and_then(JsonValue::as_i64).unwrap_or(0),
        "code_review_summary_count": state.get("code_review_summary_count").and_then(JsonValue::as_i64).unwrap_or(0),
        "policy_decision": optional_string(state.get("policy_decision")).unwrap_or_else(|| "pending".to_string()),
        "requires_code_review_summary": state.get("requires_code_review_summary").and_then(JsonValue::as_bool).unwrap_or(false),
        "landing_status": optional_string(state.get("landing_status")).unwrap_or_default().to_lowercase(),
        "landing_submission_id": optional_string(state.get("landing_submission_id")),
        "landing_result": JsonValue::Object(payload_dict(state.get("landing_result"))),
        "landing_blocker_class": optional_string(state.get("landing_blocker_class")).unwrap_or_default().to_uppercase(),
        "stale_policy_blocker_cleared": state.get("stale_policy_blocker_cleared").and_then(JsonValue::as_bool).unwrap_or(false),
        "worktree_retarget": clone_if_object(state.get("worktree_retarget")),
        "patchset_refresh": clone_if_object(state.get("patchset_refresh")),
        "resolved_change_id": optional_string(state.get("resolved_change_id"))
            .or_else(|| optional_string(change.get("change_id")))
            .unwrap_or_default(),
        "ignore_workspace_authoring": state.get("ignore_workspace_authoring").and_then(JsonValue::as_bool).unwrap_or(false),
        "patchset_is_authoritative": state.get("patchset_is_authoritative").and_then(JsonValue::as_bool).unwrap_or(false),
    }))
}

pub fn workflow_ready_facts(state: &JsonValue) -> Result<JsonValue, String> {
    let state = require_object(Some(state), "state")?;
    let change = payload_dict(state.get("change"));
    let attestation = clone_if_object(state.get("attestation"));
    let policy = clone_if_object(state.get("policy"));
    let base_line = payload_dict(state.get("base_line"));
    let review = payload_dict(state.get("review"));
    let task_review = payload_dict(state.get("task_review"));
    Ok(json!({
        "change": JsonValue::Object(change.clone()),
        "task": JsonValue::Object(payload_dict(state.get("task"))),
        "patchset": clone_if_object(state.get("patchset")),
        "workspace": JsonValue::Object(payload_dict(state.get("workspace"))),
        "freshness": JsonValue::Object(payload_dict(state.get("freshness"))),
        "attestation": attestation.clone(),
        "patchset_ci_status": clone_if_object(state.get("patchset_ci_status")),
        "policy": policy.clone(),
        "review_blocking": review.get("blocking").and_then(JsonValue::as_i64).unwrap_or(0),
        "task_review_approvals": review.get("task_approvals").and_then(JsonValue::as_i64).unwrap_or(0),
        "task_review_enabled": task_review.get("value").and_then(JsonValue::as_bool).unwrap_or(false),
        "external_readiness": clone_if_object(state.get("external_readiness")),
        "worktree_retarget": clone_if_object(state.get("worktree_retarget")),
        "patchset_refresh": clone_if_object(state.get("patchset_refresh")),
        "base_line": JsonValue::Object(base_line.clone()),
        "base_line_name": optional_string(base_line.get("line_name"))
            .or_else(|| optional_string(change.get("base_line")))
            .unwrap_or_else(|| "main".to_string()),
        "tests_state": workflow_ready_tests_state(
            json_ref_if_object(&attestation),
            json_ref_if_object(&policy),
        ),
        "payload_seed": JsonValue::Object(state.clone()),
    }))
}

pub fn workflow_land_phase_facts(
    state: &JsonValue,
    ready_state: &JsonValue,
) -> Result<JsonValue, String> {
    let state = require_object(Some(state), "state")?;
    let ready_state = require_object(Some(ready_state), "ready_state")?;
    let change = payload_dict(state.get("change"));
    let task = payload_dict(state.get("task"));
    let ready_next_action = payload_dict(ready_state.get("next_action"));
    let ready_command = optional_string(ready_next_action.get("command")).unwrap_or_else(|| {
        workflow_ready_apply_command(optional_string(change.get("change_id")).as_deref())
    });
    let full_steps = match state.get("steps") {
        Some(JsonValue::Array(steps)) => {
            JsonValue::Object(JsonMap::from_iter(steps.iter().filter_map(|step| {
                let step = step.as_object()?;
                let code = optional_string(step.get("code"))?;
                Some((code, JsonValue::Object(step.clone())))
            })))
        }
        _ => JsonValue::Object(JsonMap::new()),
    };
    Ok(json!({
        "change": JsonValue::Object(change.clone()),
        "task": JsonValue::Object(task),
        "change_id": optional_string(change.get("change_id")).unwrap_or_default(),
        "ready_next_action": JsonValue::Object(ready_next_action.clone()),
        "ready_done": optional_string(ready_next_action.get("code")).as_deref() == Some("done"),
        "ready_command": ready_command,
        "full_steps": full_steps,
        "payload_seed": JsonValue::Object(state.clone()),
        "state_next_action_command": optional_string(state.get("next_action").and_then(JsonValue::as_object).and_then(|next| next.get("command"))).unwrap_or_default(),
    }))
}

fn clone_if_object(value: Option<&JsonValue>) -> JsonValue {
    value
        .and_then(JsonValue::as_object)
        .cloned()
        .map(JsonValue::Object)
        .unwrap_or(JsonValue::Null)
}

fn json_ref_if_object(value: &JsonValue) -> Option<&JsonValue> {
    value.as_object().map(|_| value)
}

fn payload_dict(value: Option<&JsonValue>) -> JsonMap<String, JsonValue> {
    value
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

fn optional_string(value: Option<&JsonValue>) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn require_object<'a>(
    value: Option<&'a JsonValue>,
    field_name: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    match value {
        Some(JsonValue::Object(map)) => Ok(map),
        _ => Err(format!("`{field_name}` must be an object.")),
    }
}
