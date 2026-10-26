pub(crate) fn format_plan_list_text(
    payload: &JsonValue,
    show_all: bool,
    scope: &crate::binding_services::PlanScope,
) -> Result<String, String> {
    let rows = expect_array(payload)?;
    Ok(agent_list_text(
        &rows,
        &[
            "plan_id",
            "status",
            "publication_state",
            "head_match",
            "title",
        ],
        show_all,
        &["archived"],
        Some("active"),
        &scoped_all_command("ait plan list", scope.local, scope.remote_name.as_deref()),
    ))
}

pub(crate) fn format_plan_revisions_text(
    payload: &JsonValue,
    plan_id: &str,
    show_all: bool,
    scope: &crate::binding_services::PlanScope,
) -> Result<String, String> {
    let rows = expect_array(payload)?;
    let all_command = scoped_all_command(
        &format!("ait plan revisions {plan_id}"),
        scope.local,
        scope.remote_name.as_deref(),
    );
    let mut output = crate::render::key_values_text(
        &format!("ait plan revisions {plan_id}"),
        &[
            ("revisions", rows.len().to_string()),
            ("order", "newest first".to_string()),
        ],
    );
    output.push('\n');
    if !rows.is_empty() {
        output.push_str("\nhistory\n");
    }
    output.push_str(&bounded_evidence_text(
        &rows,
        &[
            "revision_number",
            "plan_revision_id",
            "title_snapshot",
            "summary",
            "created_at",
            "publication_state",
        ],
        show_all,
        DEFAULT_AGENT_TEXT_LIST_LIMIT,
        &all_command,
    ));
    Ok(output)
}
