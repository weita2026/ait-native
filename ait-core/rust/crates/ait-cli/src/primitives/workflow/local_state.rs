use super::*;
use ait_core::task_workflow_store::{
    close_task_with_task_workflow_task_store, get_change_with_task_workflow_change_store,
    get_task_with_task_workflow_task_store, land_change_with_task_workflow_change_store,
    list_changes_with_task_workflow_change_store,
};

pub(in crate::primitives) fn workflow_local_change_read_with_change_store<S>(
    change_store: &S,
    change_id: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowChangeReader + ?Sized,
{
    get_change_with_task_workflow_change_store(change_store, change_id)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_local_change_rows_with_change_store<S>(
    change_store: &S,
) -> Result<Vec<JsonValue>, String>
where
    S: TaskWorkflowChangeLister + ?Sized,
{
    list_changes_with_task_workflow_change_store(change_store).map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_local_change_land_with_change_store<S>(
    change_store: &S,
    change_id: &str,
    target_line: &str,
    landed_snapshot_id: &str,
    pre_land_target_snapshot_id: Option<&str>,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowChangeLander + ?Sized,
{
    land_change_with_task_workflow_change_store(
        change_store,
        change_id,
        target_line,
        landed_snapshot_id,
        pre_land_target_snapshot_id,
    )
    .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_local_task_read_with_task_store<S>(
    task_store: &S,
    task_id: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowTaskReader + ?Sized,
{
    get_task_with_task_workflow_task_store(task_store, task_id).map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_local_task_close_with_task_store<S>(
    task_store: &S,
    task_id: &str,
    status: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowTaskCloser + ?Sized,
{
    close_task_with_task_workflow_task_store(task_store, task_id, status)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_landing_summary_landed_snapshot_id(
    landing_summary: Option<&JsonValue>,
) -> Option<String> {
    landing_summary
        .and_then(JsonValue::as_object)
        .and_then(|summary| summary.get("result"))
        .and_then(JsonValue::as_object)
        .and_then(|result| workflow_json_text(result.get("landed_snapshot_id")))
}

pub(in crate::primitives) fn workflow_landing_summary_effectively_landed(
    landing_summary: Option<&JsonValue>,
) -> bool {
    let Some(summary) = landing_summary.and_then(JsonValue::as_object) else {
        return false;
    };
    let status = workflow_json_text(summary.get("status"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        status.as_str(),
        "succeeded" | "landed" | "complete" | "completed"
    ) && workflow_landing_summary_landed_snapshot_id(landing_summary).is_some()
}

pub(in crate::primitives) fn workflow_change_effectively_landed(
    change: &JsonValue,
    landing_summary: Option<&JsonValue>,
) -> bool {
    string_field(change, "status").as_deref() == Some("landed")
        || string_field(change, "landed_snapshot_id").is_some()
        || string_field(change, "landed_at").is_some()
        || workflow_landing_summary_effectively_landed(landing_summary)
}

pub(in crate::primitives) fn workflow_project_landed_change(
    change: &JsonValue,
    landing_summary: Option<&JsonValue>,
) -> JsonValue {
    let mut payload = change.clone();
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "status".to_string(),
            JsonValue::String("landed".to_string()),
        );
        if !object.contains_key("landed_snapshot_id") {
            if let Some(landed_snapshot_id) =
                workflow_landing_summary_landed_snapshot_id(landing_summary)
            {
                object.insert(
                    "landed_snapshot_id".to_string(),
                    JsonValue::String(landed_snapshot_id),
                );
            }
        }
    }
    payload
}

pub(in crate::primitives) fn workflow_projected_land_state(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    workflow_projected_land_state_with_workspace_mode(
        repo,
        change_id,
        remote_name,
        false,
        false,
        true,
    )
}

pub(in crate::primitives) fn workflow_projected_ready_state(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    workflow_projected_land_state_with_workspace_mode(
        repo,
        change_id,
        remote_name,
        false,
        false,
        false,
    )
}

pub(in crate::primitives) fn workflow_projected_ready_task_land_state(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    workflow_projected_land_state_with_workspace_mode(
        repo,
        change_id,
        remote_name,
        true,
        true,
        false,
    )
}

fn workflow_preserve_land_workspace_mode(
    mut projected: JsonValue,
    state: &JsonValue,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
) -> JsonValue {
    if !ignore_workspace_authoring && !patchset_is_authoritative {
        return projected;
    }
    let Some(projected_obj) = projected.as_object_mut() else {
        return projected;
    };
    if ignore_workspace_authoring {
        if let Some(workspace) = state.get("workspace") {
            projected_obj.insert("workspace".to_string(), workspace.clone());
        }
    }
    projected_obj.insert(
        "ignore_workspace_authoring".to_string(),
        JsonValue::Bool(ignore_workspace_authoring),
    );
    projected_obj.insert(
        "patchset_is_authoritative".to_string(),
        JsonValue::Bool(patchset_is_authoritative),
    );
    projected
}

fn workflow_projected_land_state_with_workspace_mode(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
    include_landing_detail: bool,
) -> Result<JsonValue, String> {
    let state = workflow_hydrate_land_state(
        repo,
        Some(change_id),
        None,
        remote_name,
        ignore_workspace_authoring,
        patchset_is_authoritative,
        include_landing_detail,
    )?;
    let change = state.get("change").cloned().unwrap_or(JsonValue::Null);
    let resolved_change_ref = workflow_projected_command_change_ref(&change, change_id);
    let task = state.get("task").cloned().unwrap_or(JsonValue::Null);
    let task_id = string_field(&task, "task_id").unwrap_or_default();
    let base_line_name =
        string_field(&state, "base_line_name").unwrap_or_else(|| "main".to_string());
    let target_line = string_field(&state, "target_line").unwrap_or_else(|| base_line_name.clone());
    let landed_facts = workflow_landed_facts(&state)?;
    if landed_facts
        .get("landed")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let command_hints = workflow_land_command_hints(
            repo,
            resolved_change_ref.as_str(),
            task_id.as_str(),
            state.get("patchset"),
            base_line_name.as_str(),
            target_line.as_str(),
            state.get("worktree_retarget"),
            0,
            false,
        );
        return project_workflow_landed_read_model(&landed_facts, &command_hints).map(
            |projected| {
                workflow_preserve_land_workspace_mode(
                    projected,
                    &state,
                    ignore_workspace_authoring,
                    patchset_is_authoritative,
                )
            },
        );
    }
    let facts = workflow_land_full_facts(&state)?;
    let command_hints = workflow_land_command_hints(
        repo,
        resolved_change_ref.as_str(),
        task_id.as_str(),
        state.get("patchset"),
        base_line_name.as_str(),
        target_line.as_str(),
        state.get("worktree_retarget"),
        facts
            .get("review_blocking")
            .and_then(JsonValue::as_i64)
            .unwrap_or_default(),
        facts
            .get("requires_code_review_summary")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
    );
    let task_review_config = workflow_effective_task_review(repo);
    project_workflow_land_full_read_model(&facts, &command_hints, &task_review_config, false).map(
        |projected| {
            workflow_preserve_land_workspace_mode(
                projected,
                &state,
                ignore_workspace_authoring,
                patchset_is_authoritative,
            )
        },
    )
}

fn workflow_projected_command_change_ref(change: &JsonValue, requested_id: &str) -> String {
    change_reference_from_payload(change, Some(requested_id))
        .unwrap_or_else(|_| requested_id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projected_commands_keep_the_task_qualified_change_reference() {
        let change = json!({
            "change_id": "C-01",
            "change_ref": "RT-2/C-01",
            "task_id": "RT-2",
        });
        assert_eq!(
            workflow_projected_command_change_ref(&change, "RT-2/C-01"),
            "RT-2/C-01"
        );
    }
}
