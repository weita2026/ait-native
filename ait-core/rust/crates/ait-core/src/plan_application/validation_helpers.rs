use super::*;

const PLAN_APPLICATION_SCOPE_VALUES: &[&str] = &["local", "remote"];

pub(super) fn normalize_query_request(
    payload: &JsonMap<String, JsonValue>,
    _label: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    let scope = require_scope(payload.get("scope"), "scope")?;
    let repo_name = require_nonempty_text(payload.get("repo_name"), "repo_name")?;
    Ok(JsonMap::from_iter([
        ("scope".to_string(), JsonValue::String(scope)),
        (
            "remote".to_string(),
            optional_text(payload.get("remote"))?
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        ("repo_name".to_string(), JsonValue::String(repo_name)),
    ]))
}

pub(super) fn require_scope(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    let scope = require_nonempty_text(value, field_name)?;
    if PLAN_APPLICATION_SCOPE_VALUES.contains(&scope.as_str()) {
        Ok(scope)
    } else {
        Err(format!(
            "Plan application payload field `{field_name}` must be one of: {}.",
            PLAN_APPLICATION_SCOPE_VALUES.join(", ")
        ))
    }
}

pub(super) fn parse_dispatch_tasks_from_value(
    value: Option<&JsonValue>,
) -> Result<Vec<DispatchTaskInput>, String> {
    require_array(value, "tasks")?
        .iter()
        .map(parse_dispatch_task_from_value)
        .collect()
}

pub(super) fn parse_dispatch_task_from_value(
    value: &JsonValue,
) -> Result<DispatchTaskInput, String> {
    let payload = require_object(Some(value), "plan dispatch task")?;
    Ok(DispatchTaskInput {
        task_id: optional_text(payload.get("task_id"))?,
        title: optional_text(payload.get("title"))?,
        status: optional_text(payload.get("status"))?,
        planning_state: optional_text(payload.get("planning_state"))?,
        origin_plan_revision_id: optional_text(payload.get("origin_plan_revision_id"))?,
        plan_drift_state: optional_text(payload.get("plan_drift_state"))?,
        plan_id: optional_text(payload.get("plan_id"))?,
        plan_item_ref: optional_text(payload.get("plan_item_ref"))?,
    })
}

pub(super) fn parse_dispatch_plan_from_value(
    value: &JsonValue,
) -> Result<DispatchPlanInput, String> {
    let payload = require_object(Some(value), "plan dispatch plan")?;
    let head_revision = payload
        .get("head_revision")
        .filter(|value| !value.is_null())
        .map(parse_dispatch_revision_from_value)
        .transpose()?;
    Ok(DispatchPlanInput {
        plan_id: optional_text(payload.get("plan_id"))?,
        title: optional_text(payload.get("title"))?,
        status: optional_text(payload.get("status"))?,
        repo_name: optional_text(payload.get("repo_name"))?,
        publication_state: optional_text(payload.get("publication_state"))?,
        published_plan_id: optional_text(payload.get("published_plan_id"))?,
        published_head_revision_id: optional_text(payload.get("published_head_revision_id"))?,
        head_revision_id: optional_text(payload.get("head_revision_id"))?,
        head_revision,
    })
}

pub(super) fn parse_dispatch_revision_from_value(
    value: &JsonValue,
) -> Result<DispatchRevisionInput, String> {
    let payload = require_object(Some(value), "plan dispatch revision")?;
    let items = require_array(payload.get("items"), "revision items")?
        .iter()
        .map(parse_dispatch_plan_item_from_value)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(DispatchRevisionInput {
        plan_revision_id: optional_text(payload.get("plan_revision_id"))?,
        revision_number: optional_i64(payload.get("revision_number"), "revision_number")?,
        artifact_path: optional_text(payload.get("artifact_path"))?,
        artifact_selector: optional_text(payload.get("artifact_selector"))?,
        artifact_heading: optional_text(payload.get("artifact_heading"))?,
        publication_state: optional_text(payload.get("publication_state"))?,
        items,
    })
}

pub(super) fn parse_dispatch_plan_item_from_value(
    value: &JsonValue,
) -> Result<DispatchPlanItemInput, String> {
    let payload = require_object(Some(value), "plan dispatch item")?;
    let heading_path = match payload.get("heading_path") {
        None | Some(JsonValue::Null) => Vec::new(),
        Some(value) => require_array(Some(value), "heading_path")?
            .iter()
            .filter_map(|entry| normalize_optional_text(entry.as_str()))
            .collect(),
    };
    Ok(DispatchPlanItemInput {
        plan_item_ref: optional_text(payload.get("plan_item_ref"))?,
        text: payload
            .get("text")
            .map(|value| match value {
                JsonValue::String(text) => text.trim().to_string(),
                JsonValue::Null => String::new(),
                other => other.to_string(),
            })
            .unwrap_or_default(),
        checkbox_state: optional_text(payload.get("checkbox_state"))?.unwrap_or_default(),
        heading_path,
        line_number: optional_i64(payload.get("line_number"), "line_number")?.unwrap_or(0),
    })
}

pub(super) fn parse_local_plan_publish_shadow_from_value(
    value: &JsonValue,
) -> Result<LocalPlanPublishShadow, String> {
    let payload = require_object(Some(value), "local plan publish shadow")?;
    Ok(LocalPlanPublishShadow {
        plan_id: optional_text(payload.get("plan_id"))?,
        publication_state: optional_text(payload.get("publication_state"))?,
        head_publication_state: optional_text(payload.get("head_publication_state"))?,
        head_revision_id: optional_text(payload.get("head_revision_id"))?,
        head_revision_number: optional_i64(
            payload.get("head_revision_number"),
            "head_revision_number",
        )?,
        published_plan_id: optional_text(payload.get("published_plan_id"))?,
        published_head_revision_id: optional_text(payload.get("published_head_revision_id"))?,
        unpublished_head: optional_bool_with_default(
            payload.get("unpublished_head"),
            false,
            "unpublished_head",
        )?,
    })
}

pub(super) fn object_payload_map_from_value(
    value: JsonValue,
    label: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    match value {
        JsonValue::Object(map) => Ok(map),
        _ => Err(format!("{label} payload must be an object.")),
    }
}

pub(super) fn require_object<'a>(
    value: Option<&'a JsonValue>,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    match value {
        Some(JsonValue::Object(map)) => Ok(map),
        Some(_) => Err(format!("{label} must be an object.")),
        None => Err(format!("{label} is required.")),
    }
}

pub(super) fn require_array<'a>(
    value: Option<&'a JsonValue>,
    label: &str,
) -> Result<&'a Vec<JsonValue>, String> {
    match value {
        Some(JsonValue::Array(entries)) => Ok(entries),
        Some(_) => Err(format!("{label} must be a list.")),
        None => Err(format!("{label} is required.")),
    }
}

pub(super) fn normalize_object_list(
    value: Option<&JsonValue>,
    label: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        JsonValue::Array(entries) => entries
            .iter()
            .map(|entry| require_object(Some(entry), label).cloned())
            .collect(),
        _ => Err(format!("{label} must be a list.")),
    }
}

pub(super) fn normalize_plan_application_object_list(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let JsonValue::Array(entries) = value else {
        return Err(format!(
            "Plan application payload field `{field_name}` must be a list."
        ));
    };
    entries
        .iter()
        .map(|entry| match entry {
            JsonValue::Object(object) => Ok(object.clone()),
            _ => Err(format!(
                "Plan application payload field `{field_name}` must contain objects."
            )),
        })
        .collect()
}

pub(super) fn normalize_text_list(
    value: Option<&JsonValue>,
    label: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    match value {
        JsonValue::Array(entries) => {
            let mut normalized = Vec::new();
            for entry in entries {
                let Some(text) = normalize_optional_text(entry.as_str()) else {
                    return Err(format!("{label} must contain non-empty strings."));
                };
                if !normalized.contains(&text) {
                    normalized.push(text);
                }
            }
            Ok(normalized)
        }
        _ => Err(format!("{label} must be a list.")),
    }
}

pub(super) fn require_object_list(
    value: Option<&JsonValue>,
    label: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    require_array(value, label)?
        .iter()
        .map(|entry| require_object(Some(entry), label).cloned())
        .collect()
}

pub(super) fn optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(normalize_optional_text(Some(text.as_str()))),
        Some(other) => Err(format!("Expected string or null, got `{other}`.")),
    }
}

pub(super) fn require_nonempty_text(
    value: Option<&JsonValue>,
    label: &str,
) -> Result<String, String> {
    normalize_optional_text(optional_text(value)?.as_deref())
        .ok_or_else(|| format!("Plan application payload field `{label}` must be non-empty."))
}

pub(super) fn require_plan_sync_text(
    payload: &JsonMap<String, JsonValue>,
    field_name: &str,
) -> Result<String, String> {
    payload
        .get(field_name)
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_optional_text(Some(value)))
        .ok_or_else(|| format!("Plan sync service request must include {field_name}."))
}

pub(super) fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub(super) fn optional_i64(value: Option<&JsonValue>, label: &str) -> Result<Option<i64>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => number
            .as_i64()
            .ok_or_else(|| format!("{label} must be an integer."))
            .map(Some),
        Some(other) => Err(format!("{label} must be an integer, got `{other}`.")),
    }
}

pub(super) fn require_bool(value: Option<&JsonValue>, label: &str) -> Result<bool, String> {
    match value {
        Some(JsonValue::Bool(flag)) => Ok(*flag),
        Some(other) => Err(format!("{label} must be a boolean, got `{other}`.")),
        None => Err(format!("{label} is required.")),
    }
}

pub(super) fn optional_bool_with_default(
    value: Option<&JsonValue>,
    default: bool,
    label: &str,
) -> Result<bool, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(default),
        Some(JsonValue::Bool(flag)) => Ok(*flag),
        Some(other) => Err(format!("{label} must be a boolean, got `{other}`.")),
    }
}
