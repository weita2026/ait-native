use super::*;

pub(super) fn plan_record_base_payload(record: &PlanRecord) -> Map<String, Value> {
    Map::from_iter([
        ("plan_id".to_string(), Value::String(record.plan_id.clone())),
        (
            "repo_name".to_string(),
            Value::String(record.repo_name.clone()),
        ),
        ("title".to_string(), Value::String(record.title.clone())),
        ("status".to_string(), Value::String(record.status.clone())),
        (
            "head_revision_id".to_string(),
            json_optional_string_value(record.head_revision_id.as_deref()),
        ),
        (
            "publication_state".to_string(),
            Value::String(record.publication_state.clone()),
        ),
        (
            "published_remote_name".to_string(),
            json_optional_string_value(record.published_remote_name.as_deref()),
        ),
        (
            "published_plan_id".to_string(),
            json_optional_string_value(record.published_plan_id.as_deref()),
        ),
        (
            "published_head_revision_id".to_string(),
            json_optional_string_value(record.published_head_revision_id.as_deref()),
        ),
        (
            "published_at".to_string(),
            json_optional_string_value(record.published_at.as_deref()),
        ),
        (
            "created_by".to_string(),
            json_optional_string_value(record.created_by.as_deref()),
        ),
        (
            "created_at".to_string(),
            Value::String(record.created_at.clone()),
        ),
        (
            "updated_at".to_string(),
            Value::String(record.updated_at.clone()),
        ),
    ])
}

pub(super) fn plan_record_list_payload(record: &PlanRecord) -> Value {
    let mut payload = plan_record_base_payload(record);
    let head = record.head_summary.as_ref();
    payload.insert(
        "head_revision_number".to_string(),
        json_optional_i64_value(head.and_then(|value| value.head_revision_number)),
    );
    payload.insert(
        "head_revision_summary".to_string(),
        json_optional_string_value(head.and_then(|value| value.head_revision_summary.as_deref())),
    );
    payload.insert(
        "head_artifact_path".to_string(),
        json_optional_string_value(head.and_then(|value| value.head_artifact_path.as_deref())),
    );
    payload.insert(
        "head_artifact_selector".to_string(),
        json_optional_string_value(head.and_then(|value| value.head_artifact_selector.as_deref())),
    );
    payload.insert(
        "head_artifact_heading".to_string(),
        json_optional_string_value(head.and_then(|value| value.head_artifact_heading.as_deref())),
    );
    payload.insert(
        "head_artifact_blob_id".to_string(),
        json_optional_string_value(head.and_then(|value| value.head_artifact_blob_id.as_deref())),
    );
    payload.insert(
        "head_revision_created_at".to_string(),
        json_optional_string_value(
            head.and_then(|value| value.head_revision_created_at.as_deref()),
        ),
    );
    Value::Object(payload)
}

pub(crate) fn plan_record_detail_payload(record: &PlanRecord) -> Value {
    let mut payload = plan_record_base_payload(record);
    payload.insert(
        "head_revision".to_string(),
        record
            .head_revision
            .as_ref()
            .map(plan_revision_record_payload)
            .unwrap_or(Value::Null),
    );
    Value::Object(payload)
}

pub(crate) fn plan_revision_record_payload(record: &PlanRevisionRecord) -> Value {
    Value::Object(Map::from_iter([
        (
            "plan_revision_id".to_string(),
            Value::String(record.plan_revision_id.clone()),
        ),
        ("plan_id".to_string(), Value::String(record.plan_id.clone())),
        (
            "revision_number".to_string(),
            Value::Number(record.revision_number.into()),
        ),
        (
            "parent_plan_revision_id".to_string(),
            json_optional_string_value(record.parent_plan_revision_id.as_deref()),
        ),
        (
            "title_snapshot".to_string(),
            Value::String(record.title_snapshot.clone()),
        ),
        (
            "summary".to_string(),
            json_optional_string_value(record.summary.as_deref()),
        ),
        (
            "artifact_path".to_string(),
            Value::String(record.artifact_path.clone()),
        ),
        (
            "artifact_selector".to_string(),
            json_optional_string_value(record.artifact_selector.as_deref()),
        ),
        (
            "artifact_heading".to_string(),
            Value::String(record.artifact_heading.clone()),
        ),
        (
            "artifact_blob_id".to_string(),
            json_optional_string_value(record.artifact_blob_id.as_deref()),
        ),
        (
            "items".to_string(),
            Value::Array(
                record
                    .items
                    .iter()
                    .map(|item| Value::Object(item.payload.clone()))
                    .collect(),
            ),
        ),
        (
            "source_kind".to_string(),
            Value::String(record.source_kind.clone()),
        ),
        (
            "created_by".to_string(),
            json_optional_string_value(record.created_by.as_deref()),
        ),
        (
            "actor_type".to_string(),
            Value::String(record.actor_type.clone()),
        ),
        (
            "publication_state".to_string(),
            Value::String(record.publication_state.clone()),
        ),
        (
            "published_plan_revision_id".to_string(),
            json_optional_string_value(record.published_plan_revision_id.as_deref()),
        ),
        (
            "published_at".to_string(),
            json_optional_string_value(record.published_at.as_deref()),
        ),
        (
            "created_at".to_string(),
            Value::String(record.created_at.clone()),
        ),
    ]))
}

pub(super) fn json_optional_string_value(value: Option<&str>) -> Value {
    value
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null)
}

pub(super) fn json_optional_i64_value(value: Option<i64>) -> Value {
    value
        .map(|value| Value::Number(value.into()))
        .unwrap_or(Value::Null)
}
