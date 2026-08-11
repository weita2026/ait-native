use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "arguments map directly to the stable Plan sync result payload"
)]
pub(super) fn plan_sync_payload(
    status: &str,
    request: &SyncRequest,
    sync_target: Option<&SyncTarget>,
    results: Vec<JsonValue>,
    adoptions: Vec<JsonValue>,
    publish_results: Vec<JsonValue>,
    artifact_results: Vec<JsonValue>,
    error: Option<String>,
) -> JsonValue {
    let mut payload = JsonMap::from_iter([
        ("status".to_string(), JsonValue::String(status.to_string())),
        (
            "target".to_string(),
            JsonValue::String(request.target.clone()),
        ),
        (
            "scope".to_string(),
            JsonValue::String(
                sync_target
                    .map(|target| target.scope.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
            ),
        ),
        (
            "mode".to_string(),
            JsonValue::String(if request.base_url.is_some() {
                "local_publish".to_string()
            } else {
                "local".to_string()
            }),
        ),
        ("results".to_string(), JsonValue::Array(results)),
        ("adoptions".to_string(), JsonValue::Array(adoptions)),
        (
            "publish_results".to_string(),
            JsonValue::Array(publish_results),
        ),
        (
            "artifact_results".to_string(),
            JsonValue::Array(artifact_results),
        ),
    ]);
    if let Some(message) = error {
        payload.insert(
            "error".to_string(),
            JsonValue::Object(JsonMap::from_iter([
                ("message".to_string(), JsonValue::String(message)),
                ("stage".to_string(), JsonValue::String("sync".to_string())),
            ])),
        );
    }
    JsonValue::Object(payload)
}

pub(super) fn plan_sync_result_row(
    action: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    plan: &JsonValue,
    continuity_match: Option<JsonValue>,
) -> JsonValue {
    let plan_id = value_get(plan, "plan_id")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let local_plan_ref = plan_id
        .as_str()
        .and_then(|value| LocalPlanId::from_raw(value).ok())
        .map(|value| JsonValue::String(value.reference()))
        .unwrap_or(JsonValue::Null);
    let mut payload = JsonMap::from_iter([
        ("action".to_string(), JsonValue::String(action.to_string())),
        (
            "artifact_path".to_string(),
            JsonValue::String(artifact_path.to_string()),
        ),
        (
            "artifact_selector".to_string(),
            artifact_selector
                .map(|value| JsonValue::String(value.to_string()))
                .unwrap_or(JsonValue::Null),
        ),
        ("plan_id".to_string(), plan_id),
        ("local_plan_ref".to_string(), local_plan_ref),
        (
            "plan_revision_id".to_string(),
            head_value(plan, "plan_revision_id")
                .or_else(|| value_get(plan, "head_revision_id").cloned())
                .unwrap_or(JsonValue::Null),
        ),
        (
            "status".to_string(),
            value_get(plan, "status")
                .cloned()
                .unwrap_or(JsonValue::Null),
        ),
    ]);
    if let Some(continuity_match) = continuity_match {
        payload.insert("continuity_match".to_string(), continuity_match);
    }
    JsonValue::Object(payload)
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments map directly to the stable Plan adoption result payload"
)]
pub(super) fn plan_sync_adoption_row(
    plan_id: JsonValue,
    remote_plan_id: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    remote_head_revision_id: Option<String>,
    local_head_revision_id: Option<String>,
    rekeyed_local_plan_id: Option<JsonValue>,
    replaced_local_plan_id: Option<JsonValue>,
) -> JsonValue {
    let local_plan_ref = plan_id
        .as_str()
        .and_then(|value| LocalPlanId::from_raw(value).ok())
        .map(|value| JsonValue::String(value.reference()))
        .unwrap_or(JsonValue::Null);
    let remote_plan_ref = RemotePlanId::from_raw(remote_plan_id)
        .map(|value| JsonValue::String(value.reference()))
        .unwrap_or(JsonValue::Null);
    let mut payload = JsonMap::from_iter([
        ("plan_id".to_string(), plan_id),
        ("local_plan_ref".to_string(), local_plan_ref),
        (
            "remote_plan_id".to_string(),
            JsonValue::String(remote_plan_id.to_string()),
        ),
        ("remote_plan_ref".to_string(), remote_plan_ref),
        (
            "artifact_path".to_string(),
            JsonValue::String(artifact_path.to_string()),
        ),
        (
            "artifact_selector".to_string(),
            artifact_selector
                .map(|value| JsonValue::String(value.to_string()))
                .unwrap_or(JsonValue::Null),
        ),
        (
            "remote_head_revision_id".to_string(),
            remote_head_revision_id
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "local_head_revision_id".to_string(),
            local_head_revision_id
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
    ]);
    if let Some(value) = rekeyed_local_plan_id {
        payload.insert("rekeyed_local_plan_id".to_string(), value);
    }
    if let Some(value) = replaced_local_plan_id {
        payload.insert("replaced_local_plan_id".to_string(), value);
    }
    JsonValue::Object(payload)
}

pub(super) fn artifact_to_json(artifact: &SyncArtifact) -> JsonValue {
    JsonValue::Object(JsonMap::from_iter([
        (
            "artifact_path".to_string(),
            JsonValue::String(artifact.artifact_path.clone()),
        ),
        (
            "artifact_selector".to_string(),
            artifact
                .artifact_selector
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "artifact_heading".to_string(),
            JsonValue::String(artifact.artifact_heading.clone()),
        ),
        (
            "items".to_string(),
            JsonValue::Array(artifact.items.clone()),
        ),
        (
            "artifact_body".to_string(),
            JsonValue::String(artifact.artifact_body.clone()),
        ),
        (
            "artifact_blob_id".to_string(),
            JsonValue::String(artifact.artifact_blob_id.clone()),
        ),
    ]))
}

pub(super) fn plan_item_to_json(item: &PlanItem) -> JsonValue {
    JsonValue::Object(JsonMap::from_iter([
        (
            "plan_item_ref".to_string(),
            JsonValue::String(item.plan_item_ref.clone()),
        ),
        ("text".to_string(), JsonValue::String(item.text.clone())),
        (
            "checkbox_state".to_string(),
            JsonValue::String(item.checkbox_state.as_str().to_string()),
        ),
        (
            "heading_path".to_string(),
            JsonValue::Array(
                item.heading_path
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        ),
        (
            "line_number".to_string(),
            JsonValue::Number(Number::from(item.line_number)),
        ),
    ]))
}

pub(super) fn default_markdown_artifact_heading(markdown: &str, artifact_path: &str) -> String {
    for raw_line in markdown.lines() {
        let trimmed = raw_line.trim();
        if let Some(rest) = trimmed.strip_prefix('#') {
            let heading = rest.trim_start_matches('#').trim();
            if !heading.is_empty() {
                return heading.to_string();
            }
        }
    }
    Path::new(artifact_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| value.replace(['_', '-'], " ").trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| artifact_path.to_string())
}

pub(super) fn current_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, false)
}

pub(super) fn rfc3339_timestamp_from_epoch_s(seconds: u64) -> Result<String, String> {
    let seconds_i64 = i64::try_from(seconds).map_err(|_| {
        format!("Binary DB epoch timestamp {seconds} cannot be represented as RFC 3339.")
    })?;
    chrono::DateTime::<Utc>::from_timestamp(seconds_i64, 0)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, false))
        .ok_or_else(|| format!("Binary DB epoch timestamp {seconds} is invalid."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_result_projects_local_plan_identity_without_changing_raw_id() {
        let row = plan_sync_result_row(
            "unchanged",
            "docs/plan.md",
            None,
            &json!({"plan_id": "PR-52"}),
            None,
        );

        assert_eq!(row["plan_id"], "PR-52");
        assert_eq!(row["local_plan_ref"], "LPR-52");
    }

    #[test]
    fn adoption_result_projects_distinct_local_and_remote_authorities() {
        let row = plan_sync_adoption_row(
            JsonValue::String("PR-150".to_string()),
            "PR-52",
            "docs/plan.md",
            None,
            Some("PRR-REMOTE".to_string()),
            Some("PRR-LOCAL".to_string()),
            None,
            None,
        );

        assert_eq!(row["plan_id"], "PR-150");
        assert_eq!(row["local_plan_ref"], "LPR-150");
        assert_eq!(row["remote_plan_id"], "PR-52");
        assert_eq!(row["remote_plan_ref"], "RPR-52");
    }
}
