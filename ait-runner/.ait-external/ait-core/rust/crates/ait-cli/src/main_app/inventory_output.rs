const TASK_TERMINAL_STATUSES: &[&str] = &[
    "completed",
    "abandoned",
    "canceled",
    "later_promotion_excluded",
];

const CHANGE_TERMINAL_STATUSES: &[&str] = &["landed", "archived", "abandoned", "canceled"];

pub(crate) fn project_change_inventory(payload: &JsonValue, all: bool) -> JsonValue {
    agent_list_json_payload(payload, all, CHANGE_TERMINAL_STATUSES)
}

pub(crate) fn format_change_inventory(
    payload: &JsonValue,
    all: bool,
    scope: &crate::binding_services::PlanScope,
) -> Result<String, String> {
    let rows = payload
        .as_array()
        .ok_or("Change list payload must decode to an array.")?;
    let columns: &[&str] = if rows
        .iter()
        .any(|row| row.get("publication_state").is_some())
    {
        &["task_id", "status", "publication_state", "title"]
    } else {
        &["task_id", "status", "title"]
    };
    Ok(agent_list_text(
        rows,
        columns,
        all,
        CHANGE_TERMINAL_STATUSES,
        Some("open"),
        &scoped_all_command("ait change list", scope.local, scope.remote_name.as_deref()),
    ))
}

pub(crate) fn project_snapshot_inventory(payload: &JsonValue, all: bool) -> JsonValue {
    bounded_ordered_json_payload(payload, all, DEFAULT_AGENT_TEXT_LIST_LIMIT)
}

pub(crate) fn format_snapshot_inventory(payload: &JsonValue, all: bool) -> Result<String, String> {
    let rows = payload
        .as_array()
        .ok_or("Snapshot list payload must decode to an array.")?;
    Ok(bounded_evidence_text(
        rows,
        &["snapshot_id", "line_name", "message"],
        all,
        DEFAULT_AGENT_TEXT_LIST_LIMIT,
        "ait snapshot list --all",
    ))
}

pub(crate) fn project_task_inventory(payload: &JsonValue, all: bool) -> JsonValue {
    agent_list_json_payload(payload, all, TASK_TERMINAL_STATUSES)
}

pub(crate) fn format_task_inventory(
    payload: &JsonValue,
    all: bool,
    scope: &crate::binding_services::PlanScope,
) -> Result<String, String> {
    let rows = payload
        .as_array()
        .ok_or("task list payload must decode to a JSON array.")?;
    let columns: &[&str] = if rows
        .iter()
        .any(|row| row.get("publication_state").is_some())
    {
        &["task_id", "status", "publication_state", "title"]
    } else {
        &["task_id", "status", "title"]
    };
    Ok(agent_list_text(
        rows,
        columns,
        all,
        TASK_TERMINAL_STATUSES,
        Some("open"),
        &scoped_all_command("ait task list", scope.local, scope.remote_name.as_deref()),
    ))
}
