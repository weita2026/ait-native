use crate::json_support::{json, JsonValue};

use crate::workflow_closeout_decision::{
    change_effectively_landed, workflow_land_next_action, workflow_ready_next_action,
};
use crate::workflow_closeout_model_support::{
    bool_field, clone_field, clone_obj_field, field_obj, insert_json, int_field,
    optional_string_field,
};
use crate::workflow_closeout_projection::{
    workflow_land_full_steps, workflow_land_phase_steps, workflow_land_suggested_commands,
    workflow_landed_steps_and_suggested_commands, workflow_ready_steps,
    workflow_ready_suggested_commands,
};

fn projected_change_with_effective_land(facts: &JsonValue) -> JsonValue {
    let mut change = field_obj(facts, "change");
    let landing_summary =
        crate::workflow_closeout_model_support::optional_obj_field(facts, "landing_summary");
    if !change_effectively_landed(&change, landing_summary.as_ref()) {
        return JsonValue::Object(change);
    }
    change.insert(
        "status".to_string(),
        JsonValue::String("landed".to_string()),
    );
    if optional_string_field(&change, "landed_snapshot_id").is_none() {
        if let Some(landed_snapshot_id) = landing_summary
            .as_ref()
            .and_then(|summary| summary.get("result"))
            .and_then(JsonValue::as_object)
            .and_then(|result| optional_string_field(result, "landed_snapshot_id"))
        {
            change.insert(
                "landed_snapshot_id".to_string(),
                JsonValue::String(landed_snapshot_id),
            );
        }
    }
    JsonValue::Object(change)
}

fn projected_review_with_lane_counts(facts: &JsonValue) -> JsonValue {
    let mut review = field_obj(facts, "review_summary");
    for (field, fact_field) in [
        ("approvals", "review_approvals"),
        ("blocking", "review_blocking"),
        ("task_approvals", "task_review_approvals"),
        ("team_approvals", "team_review_approvals"),
    ] {
        review.insert(
            field.to_string(),
            JsonValue::from(int_field(facts, fact_field)),
        );
    }
    JsonValue::Object(review)
}

pub fn project_workflow_landed_read_model(
    facts: &JsonValue,
    command_hints: &JsonValue,
) -> Result<JsonValue, String> {
    let next_action = workflow_land_next_action(
        facts,
        command_hints,
        false,
        false,
        true,
        true,
        None,
        "pass",
        None,
        true,
    );
    let steps = workflow_landed_steps_and_suggested_commands(facts, &next_action);
    Ok(json!({
        "change": projected_change_with_effective_land(facts),
        "task": clone_field(facts, "task"),
        "patchset": clone_field(facts, "patchset"),
        "patchset_source": clone_field(facts, "patchset_source"),
        "workspace": {
            "clean": bool_field(&field_obj(facts, "workspace"), "clean"),
            "changed_count": crate::workflow_closeout_model_support::int_field(&field_obj(facts, "workspace"), "changed_count"),
            "current_line": crate::workflow_closeout_model_support::string_field(facts, "current_line_name"),
            "head_snapshot_id": optional_string_field(facts, "revision_snapshot_id"),
            "workspace_status": if bool_field(&field_obj(facts, "workspace"), "clean") { "clean" } else { "dirty" },
            "workspace_matches_patchset": true,
        },
        "base_line": {
            "line_name": crate::workflow_closeout_model_support::string_field(facts, "target_line"),
            "head_snapshot_id": JsonValue::Null,
        },
        "review": {
            "approvals": 1,
            "blocking": 0,
            "task_approvals": 1,
            "team_approvals": 0,
            "current_patchset_id": crate::workflow_closeout_model_support::optional_nonempty_string(facts, "patchset_label"),
            "reviews": [],
        },
        "attestation": JsonValue::Null,
        "policy": {
            "decision": "pass",
            "checks": [],
            "status_source": "landed_fast_path",
        },
        "landing_summary": clone_field(facts, "landing_summary"),
        "freshness": {
            "base_is_fresh": true,
            "preflight_state": "not_required",
            "recovery_required": false,
            "worktree_needs_retarget": false,
            "rebase_state": "idle",
            "remote_base_snapshot_id": JsonValue::Null,
            "patchset_base_snapshot_id": JsonValue::Null,
            "patchset_revision_snapshot_id": clone_field(facts, "patchset_revision_snapshot_id"),
        },
        "steps": steps.0,
        "next_action": next_action,
        "suggested_commands": steps.1,
    }))
}

pub fn project_workflow_land_full_read_model(
    facts: &JsonValue,
    command_hints: &JsonValue,
    task_review_config: &JsonValue,
    apply_owned_continuation: bool,
) -> Result<JsonValue, String> {
    let task_review_enabled = bool_field(task_review_config, "value");
    let auto_review_reviewer = if !task_review_enabled {
        crate::workflow_closeout_model_support::command_hint(command_hints, "auto_review_reviewer")
    } else {
        None
    };
    let next_action = workflow_land_next_action(
        facts,
        command_hints,
        apply_owned_continuation,
        false,
        bool_field(facts, "base_is_fresh"),
        crate::workflow_closeout_model_support::optional_bool_field(
            facts,
            "workspace_matches_patchset",
        )
        .unwrap_or(false),
        crate::workflow_closeout_model_support::optional_obj_field(facts, "policy"),
        crate::workflow_closeout_model_support::string_field(facts, "policy_decision").as_str(),
        crate::workflow_closeout_model_support::optional_obj_field(facts, "landing_summary"),
        task_review_enabled,
    );
    let suggested_commands = workflow_land_suggested_commands(
        facts,
        command_hints,
        &next_action,
        apply_owned_continuation,
    );
    let steps = workflow_land_full_steps(
        facts,
        command_hints,
        task_review_enabled,
        auto_review_reviewer.as_deref(),
        apply_owned_continuation,
    );
    let workspace = field_obj(facts, "workspace");
    let patchset = crate::workflow_closeout_model_support::optional_obj_field(facts, "patchset");
    let worktree_retarget =
        crate::workflow_closeout_model_support::optional_obj_field(facts, "worktree_retarget");
    Ok(json!({
        "change": projected_change_with_effective_land(facts),
        "task": clone_field(facts, "task"),
        "patchset": clone_field(facts, "patchset"),
        "patchset_refresh": clone_field(facts, "patchset_refresh"),
        "patchset_source": if patchset.is_some() { clone_field(facts, "patchset_source") } else { JsonValue::Null },
        "workspace": {
            "clean": bool_field(&workspace, "clean"),
            "changed_count": crate::workflow_closeout_model_support::int_field(&workspace, "changed_count"),
            "current_line": crate::workflow_closeout_model_support::string_field(facts, "current_line_name"),
            "head_snapshot_id": clone_field(facts, "revision_snapshot_id"),
            "workspace_status": if bool_field(&workspace, "clean") { "clean" } else { "dirty" },
            "workspace_matches_patchset": clone_field(facts, "workspace_matches_patchset"),
        },
        "base_line": {
            "line_name": crate::workflow_closeout_model_support::string_field(facts, "base_line_name"),
            "head_snapshot_id": clone_field(facts, "remote_base_snapshot_id"),
        },
        "review": projected_review_with_lane_counts(facts),
        "attestation": clone_field(facts, "attestation"),
        "patchset_ci_status": clone_field(facts, "patchset_ci_status"),
        "policy": clone_field(facts, "policy"),
        "task_review": task_review_config.clone(),
        "landing_summary": clone_field(facts, "landing_summary"),
        "worktree_retarget": clone_field(facts, "worktree_retarget"),
        "freshness": {
            "base_is_fresh": bool_field(facts, "base_is_fresh"),
            "preflight_state": if bool_field(facts, "base_is_fresh") { "fresh" } else { "stale" },
            "recovery_required": patchset.is_some() && !bool_field(facts, "base_is_fresh"),
            "worktree_needs_retarget": worktree_retarget
                .as_ref()
                .map(|value| bool_field(value, "needs_retarget"))
                .unwrap_or(false),
            "rebase_state": worktree_retarget
                .as_ref()
                .and_then(|value| optional_string_field(value, "rebase_state"))
                .unwrap_or_else(|| "idle".to_string()),
            "remote_base_snapshot_id": clone_field(facts, "remote_base_snapshot_id"),
            "patchset_base_snapshot_id": clone_field(facts, "patchset_base_snapshot_id"),
            "patchset_revision_snapshot_id": clone_field(facts, "patchset_revision_snapshot_id"),
        },
        "steps": steps,
        "next_action": next_action,
        "suggested_commands": suggested_commands,
    }))
}

pub fn project_workflow_ready_read_model(
    facts: &JsonValue,
    command_hints: &JsonValue,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
    apply_owned_continuation: bool,
) -> Result<JsonValue, String> {
    let next_action = workflow_ready_next_action(
        facts,
        command_hints,
        ignore_workspace_authoring,
        patchset_is_authoritative,
        apply_owned_continuation,
    );
    let mut result = clone_obj_field(facts, "payload_seed");
    insert_json(
        &mut result,
        "change",
        projected_change_with_effective_land(facts),
    );
    insert_json(
        &mut result,
        "steps",
        JsonValue::Array(workflow_ready_steps(
            facts,
            command_hints,
            ignore_workspace_authoring,
            patchset_is_authoritative,
        )),
    );
    insert_json(&mut result, "next_action", next_action.clone());
    insert_json(
        &mut result,
        "suggested_commands",
        JsonValue::Array(workflow_ready_suggested_commands(
            command_hints,
            &next_action,
            apply_owned_continuation,
        )),
    );
    insert_json(
        &mut result,
        "workflow_phase",
        JsonValue::String("ready".to_string()),
    );
    Ok(JsonValue::Object(result))
}

pub fn project_workflow_land_phase_read_model(facts: &JsonValue) -> Result<JsonValue, String> {
    let ready_done = bool_field(facts, "ready_done");
    let mut result = clone_obj_field(facts, "payload_seed");
    insert_json(
        &mut result,
        "steps",
        JsonValue::Array(workflow_land_phase_steps(facts)),
    );
    insert_json(
        &mut result,
        "workflow_phase",
        JsonValue::String("land".to_string()),
    );
    if !ready_done {
        let ready_next_action = field_obj(facts, "ready_next_action");
        let detail = optional_string_field(&ready_next_action, "detail")
            .or_else(|| optional_string_field(&ready_next_action, "summary"))
            .unwrap_or_else(|| {
                "The change still needs a ready patchset and attestation before land can continue."
                    .to_string()
            });
        insert_json(
            &mut result,
            "next_action",
            json!({
                "code": "workflow_ready",
                "summary": "Run workflow ready before review or land.",
                "detail": detail,
                "command": clone_field(facts, "ready_command"),
            }),
        );
        insert_json(
            &mut result,
            "suggested_commands",
            JsonValue::Array(vec![clone_field(facts, "ready_command")]),
        );
    }
    Ok(JsonValue::Object(result))
}

#[cfg(test)]
mod tests;
