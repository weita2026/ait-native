use super::*;

pub fn build_plan_list_service_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_list_service_payload_json(payload_json)
}

pub(crate) fn build_plan_list_service_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_list_service_request_payload_map(payload)
}

pub fn build_plan_show_service_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_show_service_payload_json(payload_json)
}

pub(crate) fn build_plan_show_service_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_show_service_request_payload_map(payload)
}

pub fn build_plan_revisions_service_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_revisions_service_payload_json(payload_json)
}

pub(crate) fn build_plan_revisions_service_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_revisions_service_request_payload_map(payload)
}

pub fn build_plan_items_service_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_items_service_payload_json(payload_json)
}

pub(crate) fn build_plan_items_service_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = object_payload_map_from_value(
        normalize_plan_items_service_request_payload_map(payload)?,
        "plan items service request",
    )?;
    let plan = parse_dispatch_plan_from_value(
        request
            .get("plan")
            .ok_or_else(|| "Plan items service request is missing plan.".to_string())?,
    )?;
    let revision = request
        .get("revision")
        .filter(|value| !value.is_null())
        .map(parse_dispatch_revision_from_value)
        .transpose()?;
    let payload = plan_items_payload(&plan, revision.as_ref());
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "scope".to_string(),
            JsonValue::String(require_scope(request.get("scope"), "scope")?),
        ),
        (
            "remote".to_string(),
            optional_text(request.get("remote"))?
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "repo_name".to_string(),
            JsonValue::String(require_nonempty_text(
                request.get("repo_name"),
                "repo_name",
            )?),
        ),
        ("plan".to_string(), render_plan_items_payload_json(&payload)),
    ])))
}

pub fn build_plan_candidates_service_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_candidates_service_payload_json(payload_json)
}

pub(crate) fn build_plan_candidates_service_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = object_payload_map_from_value(
        normalize_plan_candidates_service_request_payload_map(payload)?,
        "plan candidates service request",
    )?;
    let tasks = parse_dispatch_tasks_from_value(request.get("tasks"))?;
    let local_shadow_index = require_object(
        request.get("local_shadow_index"),
        "plan candidates service request local_shadow_index",
    )?;
    let contains_terms = normalize_text_list(request.get("contains_terms"), "contains_terms")?;
    let mut summaries = Vec::new();
    for value in require_array(
        request.get("plans"),
        "plan candidates service request plans",
    )? {
        let plan = parse_dispatch_plan_from_value(value)?;
        let local_shadow = plan
            .plan_id
            .as_ref()
            .and_then(|plan_id| local_shadow_index.get(plan_id))
            .map(parse_local_plan_publish_shadow_from_value)
            .transpose()?;
        summaries.push(plan_dispatch_summary(
            &plan,
            &tasks,
            None,
            local_shadow.as_ref(),
        ));
    }
    if !contains_terms.is_empty() {
        summaries.retain(|summary| summary_matches_contains_terms(summary, &contains_terms));
    }
    let payload = plan_candidates_payload(
        &summaries,
        Some(require_scope(request.get("scope"), "scope")?.as_str()),
        Some(require_nonempty_text(request.get("repo_name"), "repo_name")?.as_str()),
        optional_text(request.get("remote"))?.as_deref(),
        require_bool(request.get("include_all"), "include_all")?,
    );
    Ok(render_plan_candidates_payload_json(&payload))
}

pub fn build_plan_inspect_service_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_inspect_service_payload_json(payload_json)
}

pub(crate) fn build_plan_inspect_service_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = object_payload_map_from_value(
        normalize_plan_inspect_service_request_payload_map(payload)?,
        "plan inspect service request",
    )?;
    let plan = parse_dispatch_plan_from_value(
        request
            .get("plan")
            .ok_or_else(|| "Plan inspect service request is missing plan.".to_string())?,
    )?;
    let tasks = parse_dispatch_tasks_from_value(request.get("tasks"))?;
    let revision = request
        .get("revision")
        .filter(|value| !value.is_null())
        .map(parse_dispatch_revision_from_value)
        .transpose()?;
    let local_shadow = request
        .get("local_shadow")
        .filter(|value| !value.is_null())
        .map(parse_local_plan_publish_shadow_from_value)
        .transpose()?;
    let summary = plan_dispatch_summary(&plan, &tasks, revision.as_ref(), local_shadow.as_ref());
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "scope".to_string(),
            JsonValue::String(require_scope(request.get("scope"), "scope")?),
        ),
        (
            "remote".to_string(),
            optional_text(request.get("remote"))?
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "repo_name".to_string(),
            JsonValue::String(require_nonempty_text(
                request.get("repo_name"),
                "repo_name",
            )?),
        ),
        (
            "plan".to_string(),
            render_plan_dispatch_summary_json(&summary),
        ),
    ])))
}

pub fn build_plan_sync_service_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_sync_service_payload_json(payload_json)
}

pub(crate) fn build_plan_sync_service_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let request = object_payload_map_from_value(
        normalize_plan_sync_service_request_payload_map(payload)?,
        "plan sync service request",
    )?;
    let results = normalize_object_list(request.get("results"), "results")?;
    let adoptions = normalize_object_list(request.get("adoptions"), "adoptions")?;
    let publish_results = normalize_object_list(request.get("publish_results"), "publish_results")?;
    let artifact_results =
        normalize_object_list(request.get("artifact_results"), "artifact_results")?;
    let summary = JsonValue::Object(JsonMap::from_iter([
        (
            "created_count".to_string(),
            json_usize(sync_action_count(&results, "created")),
        ),
        (
            "updated_count".to_string(),
            json_usize(sync_action_count(&results, "updated")),
        ),
        (
            "unchanged_count".to_string(),
            json_usize(sync_action_count(&results, "unchanged")),
        ),
        (
            "pruned_count".to_string(),
            json_usize(sync_action_count(&results, "pruned")),
        ),
        ("adopted_count".to_string(), json_usize(adoptions.len())),
        ("processed_count".to_string(), json_usize(results.len())),
        (
            "published_count".to_string(),
            json_usize(publish_results.len()),
        ),
        (
            "artifact_count".to_string(),
            json_usize(artifact_results.len()),
        ),
    ]));
    let payload = JsonMap::from_iter([
        (
            "status".to_string(),
            JsonValue::String(require_nonempty_text(request.get("status"), "status")?),
        ),
        (
            "target".to_string(),
            JsonValue::String(require_nonempty_text(request.get("target"), "target")?),
        ),
        (
            "scope".to_string(),
            JsonValue::String(require_nonempty_text(request.get("scope"), "scope")?),
        ),
        (
            "mode".to_string(),
            JsonValue::String(require_nonempty_text(request.get("mode"), "mode")?),
        ),
        (
            "results".to_string(),
            JsonValue::Array(results.into_iter().map(JsonValue::Object).collect()),
        ),
        (
            "adoptions".to_string(),
            JsonValue::Array(adoptions.into_iter().map(JsonValue::Object).collect()),
        ),
        (
            "publish_results".to_string(),
            JsonValue::Array(publish_results.into_iter().map(JsonValue::Object).collect()),
        ),
        (
            "artifact_results".to_string(),
            JsonValue::Array(
                artifact_results
                    .into_iter()
                    .map(JsonValue::Object)
                    .collect(),
            ),
        ),
        ("summary".to_string(), summary),
        (
            "task_start_advisory".to_string(),
            request.get("advisory").cloned().unwrap_or(JsonValue::Null),
        ),
        (
            "error".to_string(),
            request.get("error").cloned().unwrap_or(JsonValue::Null),
        ),
    ]);
    Ok(JsonValue::Object(payload))
}
