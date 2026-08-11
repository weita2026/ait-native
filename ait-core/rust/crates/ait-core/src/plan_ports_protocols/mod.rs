use crate::json_support::{JsonMap, JsonValue};

use crate::plan_workflow_json::PlanWorkflowJson;
use crate::task_json::TaskJson;

const PLAN_STORE_READ_OPERATIONS: &[&str] = &[
    "list_plans",
    "get_plan",
    "list_revisions",
    "get_revision",
    "get_revision_by_id",
    "inspect_storage_readiness",
];

const PLAN_REMOTE_OPERATIONS: &[&str] = &[
    "list_plans",
    "get_plan",
    "list_revisions",
    "get_revision",
    "create_plan",
    "revise_plan",
    "update_plan_status",
    "put_plan_revision_artifacts",
];

const ARTIFACT_RESOLVER_OPERATIONS: &[&str] = &[
    "list_visible_workspace_paths",
    "list_visible_markdown_artifact_paths",
    "read_utf8_text_file",
    "read_json_file",
    "read_binary_file",
    "resolve_repo_artifact_path",
    "zip_archive_has_member",
    "read_zip_archive_member",
];

const CONFIG_RUNTIME_KEYS: &[&str] = &[
    "plan_core_backend",
    "plan_http_backend",
    "plan_filesystem_backend",
    "plan_blob_diff_backend",
    "plan_pack_substrate_backend",
    "workflow_primitives_backend",
    "plan_ports_protocols_backend",
];

pub fn normalize_plan_store_read_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_store_read_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_store_read_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let operation = require_named_operation(
        payload.get("operation"),
        PLAN_STORE_READ_OPERATIONS,
        "plan store",
    )?;
    let plan_storage = require_object(payload.get("plan_storage"), "plan_storage")?.clone();
    require_text(plan_storage.get("mode"), "plan_storage.mode")?;
    let plan_id = optional_text(payload.get("plan_id"))?;
    let plan_revision_id = optional_text(payload.get("plan_revision_id"))?;
    if matches!(
        operation.as_str(),
        "get_plan" | "list_revisions" | "get_revision"
    ) && plan_id.is_none()
    {
        return Err(format!(
            "Plan store request `{operation}` must include plan_id."
        ));
    }
    if matches!(operation.as_str(), "get_revision" | "get_revision_by_id")
        && plan_revision_id.is_none()
    {
        return Err(format!(
            "Plan store request `{operation}` must include plan_revision_id."
        ));
    }
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("operation".to_string(), JsonValue::String(operation)),
        ("plan_storage".to_string(), JsonValue::Object(plan_storage)),
        (
            "plan_id".to_string(),
            plan_id.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        (
            "plan_revision_id".to_string(),
            plan_revision_id
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
    ])))
}

pub fn normalize_plan_remote_transport_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_remote_transport_payload_json(payload_json)
}

pub(crate) fn normalize_plan_remote_transport_payload_object(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_remote_transport_payload_map(&payload)
}

pub fn normalize_plan_remote_request_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_remote_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_remote_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let operation = require_named_operation(
        payload.get("operation"),
        PLAN_REMOTE_OPERATIONS,
        "plan remote",
    )?;
    let transport = normalize_plan_remote_transport_payload_map(require_object(
        payload.get("transport"),
        "plan remote transport",
    )?)?;
    let repo_name = optional_text(payload.get("repo_name"))?;
    let plan_id = optional_text(payload.get("plan_id"))?;
    let plan_revision_id = optional_text(payload.get("plan_revision_id"))?;
    let artifact_path = optional_text(payload.get("artifact_path"))?;
    let artifact_selector = optional_text(payload.get("artifact_selector"))?;
    let artifact_heading = optional_text(payload.get("artifact_heading"))?;
    let title = optional_text(payload.get("title"))?;
    let summary = optional_text(payload.get("summary"))?;
    let status = optional_text(payload.get("status"))?;
    let source_kind = optional_text(payload.get("source_kind"))?;
    let artifact_body = optional_exact_string(payload.get("artifact_body"), "artifact_body")?;
    let expected_head_revision_id = optional_text(payload.get("expected_head_revision_id"))?;
    let items = normalize_object_list(payload.get("items"), "items")?;
    let artifacts = normalize_object_list(payload.get("artifacts"), "artifacts")?;

    if matches!(operation.as_str(), "list_plans" | "create_plan") && repo_name.is_none() {
        return Err(format!(
            "Plan remote request `{operation}` must include repo_name."
        ));
    }
    if matches!(
        operation.as_str(),
        "get_plan"
            | "list_revisions"
            | "get_revision"
            | "revise_plan"
            | "update_plan_status"
            | "put_plan_revision_artifacts"
    ) && plan_id.is_none()
    {
        return Err(format!(
            "Plan remote request `{operation}` must include plan_id."
        ));
    }
    if matches!(
        operation.as_str(),
        "get_revision" | "put_plan_revision_artifacts"
    ) && plan_revision_id.is_none()
    {
        return Err(format!(
            "Plan remote request `{operation}` must include plan_revision_id."
        ));
    }
    if matches!(operation.as_str(), "create_plan" | "revise_plan") {
        if artifact_path.is_none() {
            return Err(format!(
                "Plan remote request `{operation}` must include artifact_path."
            ));
        }
        if artifact_heading.is_none() {
            return Err(format!(
                "Plan remote request `{operation}` must include artifact_heading."
            ));
        }
        if items.is_empty() {
            return Err(format!(
                "Plan remote request `{operation}` must include items."
            ));
        }
    }
    if operation == "update_plan_status" && status.is_none() {
        return Err("Plan remote request `update_plan_status` must include status.".to_string());
    }

    Ok(JsonValue::Object(JsonMap::from_iter([
        ("operation".to_string(), JsonValue::String(operation)),
        ("transport".to_string(), transport),
        (
            "repo_name".to_string(),
            repo_name.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        (
            "plan_id".to_string(),
            plan_id.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        (
            "plan_revision_id".to_string(),
            plan_revision_id
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "artifact_path".to_string(),
            artifact_path
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "artifact_selector".to_string(),
            artifact_selector
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "artifact_heading".to_string(),
            artifact_heading
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "title".to_string(),
            title.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        (
            "summary".to_string(),
            summary.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        (
            "status".to_string(),
            status.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        (
            "source_kind".to_string(),
            source_kind
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "artifact_body".to_string(),
            artifact_body
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "expected_head_revision_id".to_string(),
            expected_head_revision_id
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        ("items".to_string(), JsonValue::Array(items)),
        ("artifacts".to_string(), JsonValue::Array(artifacts)),
    ])))
}

pub fn normalize_artifact_resolver_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_artifact_resolver_request_payload_json(payload_json)
}

pub(crate) fn normalize_artifact_resolver_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let operation = require_named_operation(
        payload.get("operation"),
        ARTIFACT_RESOLVER_OPERATIONS,
        "artifact resolver",
    )?;
    let repo_root = optional_text(payload.get("repo_root"))?;
    let path = optional_text(payload.get("path"))?;
    let runtime_root = optional_text(payload.get("runtime_root"))?;
    let ignore_rules_text = optional_text(payload.get("ignore_rules_text"))?;
    let entry_name = optional_text(payload.get("entry_name"))?;
    let allow_missing = optional_bool(payload.get("allow_missing"))?.unwrap_or(false);

    if matches!(
        operation.as_str(),
        "list_visible_workspace_paths" | "list_visible_markdown_artifact_paths"
    ) && repo_root.is_none()
    {
        return Err(format!(
            "Artifact resolver request `{operation}` must include repo_root."
        ));
    }
    if matches!(
        operation.as_str(),
        "read_utf8_text_file" | "read_json_file" | "read_binary_file"
    ) && path.is_none()
    {
        return Err(format!(
            "Artifact resolver request `{operation}` must include path."
        ));
    }
    if operation == "resolve_repo_artifact_path" && (repo_root.is_none() || path.is_none()) {
        return Err(
            "Artifact resolver request `resolve_repo_artifact_path` must include repo_root and path.".to_string(),
        );
    }
    if matches!(
        operation.as_str(),
        "zip_archive_has_member" | "read_zip_archive_member"
    ) && (path.is_none() || entry_name.is_none())
    {
        return Err(format!(
            "Artifact resolver request `{operation}` must include path and entry_name."
        ));
    }

    Ok(JsonValue::Object(JsonMap::from_iter([
        ("operation".to_string(), JsonValue::String(operation)),
        (
            "repo_root".to_string(),
            repo_root.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        (
            "path".to_string(),
            path.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        (
            "runtime_root".to_string(),
            runtime_root
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "ignore_rules_text".to_string(),
            ignore_rules_text
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "entry_name".to_string(),
            entry_name.map(JsonValue::String).unwrap_or(JsonValue::Null),
        ),
        ("allow_missing".to_string(), JsonValue::Bool(allow_missing)),
    ])))
}

pub fn normalize_artifact_publish_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_artifact_publish_request_payload_json(payload_json)
}

pub(crate) fn normalize_artifact_publish_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let plan_id = require_text(payload.get("plan_id"), "plan_id")?;
    let plan_revision_id = require_text(payload.get("plan_revision_id"), "plan_revision_id")?;
    let artifacts = normalize_object_list(payload.get("artifacts"), "artifacts")?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("plan_id".to_string(), JsonValue::String(plan_id)),
        (
            "plan_revision_id".to_string(),
            JsonValue::String(plan_revision_id),
        ),
        ("artifacts".to_string(), JsonValue::Array(artifacts)),
    ])))
}

pub fn normalize_linked_task_lookup_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    TaskJson::stateless().normalize_linked_task_lookup_payload_json(payload_json)
}

pub(crate) fn normalize_linked_task_lookup_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let task_links_by_item = normalize_linked_item_rows(payload.get("task_links_by_item"))?;
    let tasks_by_plan = normalize_tasks_by_plan_rows(payload.get("tasks_by_plan"))?;
    let linked_task_count = match payload.get("linked_task_count") {
        Some(value) => require_nonnegative_i64(Some(value), "linked_task_count")?,
        None => tasks_by_plan
            .iter()
            .map(|row| {
                row.get("tasks")
                    .and_then(JsonValue::as_array)
                    .map(|items| items.len() as i64)
                    .unwrap_or(0)
            })
            .sum(),
    };
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "task_links_by_item".to_string(),
            JsonValue::Array(task_links_by_item),
        ),
        ("tasks_by_plan".to_string(), JsonValue::Array(tasks_by_plan)),
        (
            "linked_task_count".to_string(),
            JsonValue::Number(linked_task_count.into()),
        ),
    ])))
}

pub fn normalize_plan_config_runtime_facts_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_config_runtime_facts_payload_json(payload_json)
}

pub(crate) fn normalize_plan_config_runtime_facts_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let mut normalized = JsonMap::new();
    for key in CONFIG_RUNTIME_KEYS {
        let entry = require_object(payload.get(*key), &format!("config runtime entry `{key}`"))?;
        let value =
            normalize_backend_name(require_text(entry.get("value"), &format!("{key}.value"))?)?;
        let source = require_text(entry.get("source"), &format!("{key}.source"))?;
        if !matches!(source.as_str(), "default" | "env" | "explicit") {
            return Err(format!(
                "Unsupported backend selection source `{source}` for `{key}`."
            ));
        }
        normalized.insert(
            key.to_string(),
            JsonValue::Object(JsonMap::from_iter([
                ("value".to_string(), JsonValue::String(value)),
                ("source".to_string(), JsonValue::String(source)),
            ])),
        );
    }
    Ok(JsonValue::Object(normalized))
}

pub fn normalize_plan_connection_manager_stats_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_connection_manager_stats_payload_json(payload_json)
}

pub(crate) fn normalize_plan_connection_manager_stats_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let active_write_lease = require_bool(payload.get("active_write_lease"), "active_write_lease")?;
    let closed = require_bool(payload.get("closed"), "closed")?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "store_id".to_string(),
            JsonValue::String(require_text(payload.get("store_id"), "store_id")?),
        ),
        (
            "max_size".to_string(),
            JsonValue::Number(require_positive_i64(payload.get("max_size"), "max_size")?.into()),
        ),
        (
            "total_connections".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(payload.get("total_connections"), "total_connections")?
                    .into(),
            ),
        ),
        (
            "idle_connections".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(payload.get("idle_connections"), "idle_connections")?
                    .into(),
            ),
        ),
        (
            "active_leases".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(payload.get("active_leases"), "active_leases")?.into(),
            ),
        ),
        (
            "active_read_leases".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(payload.get("active_read_leases"), "active_read_leases")?
                    .into(),
            ),
        ),
        (
            "active_write_lease".to_string(),
            JsonValue::Bool(active_write_lease),
        ),
        ("closed".to_string(), JsonValue::Bool(closed)),
        (
            "busy_timeout_ms".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(payload.get("busy_timeout_ms"), "busy_timeout_ms")?.into(),
            ),
        ),
        (
            "retry_attempts".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(payload.get("retry_attempts"), "retry_attempts")?.into(),
            ),
        ),
        (
            "retry_backoff_ms".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(payload.get("retry_backoff_ms"), "retry_backoff_ms")?
                    .into(),
            ),
        ),
        (
            "pool_exhaustion_count".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(
                    payload.get("pool_exhaustion_count"),
                    "pool_exhaustion_count",
                )?
                .into(),
            ),
        ),
        (
            "concurrency_rejection_count".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(
                    payload.get("concurrency_rejection_count"),
                    "concurrency_rejection_count",
                )?
                .into(),
            ),
        ),
        (
            "busy_retry_count".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(payload.get("busy_retry_count"), "busy_retry_count")?
                    .into(),
            ),
        ),
    ])))
}

fn normalize_plan_remote_transport_payload_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let base_url = require_text(payload.get("base_url"), "base_url")?;
    let headers = normalize_header_map(payload.get("headers"))?;
    let timeout_ms = optional_nonnegative_i64(payload.get("timeout_ms"), "timeout_ms")?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("base_url".to_string(), JsonValue::String(base_url)),
        ("headers".to_string(), JsonValue::Object(headers)),
        (
            "timeout_ms".to_string(),
            timeout_ms
                .map(|value| JsonValue::Number(value.into()))
                .unwrap_or(JsonValue::Null),
        ),
        (
            "retry_attempts".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(
                    payload.get("retry_attempts").or(Some(&JsonValue::from(0))),
                    "retry_attempts",
                )?
                .into(),
            ),
        ),
        (
            "retry_backoff_ms".to_string(),
            JsonValue::Number(
                require_nonnegative_i64(
                    payload
                        .get("retry_backoff_ms")
                        .or(Some(&JsonValue::from(0))),
                    "retry_backoff_ms",
                )?
                .into(),
            ),
        ),
        (
            "pool_max_idle_per_host".to_string(),
            JsonValue::Number(
                require_positive_i64(
                    payload
                        .get("pool_max_idle_per_host")
                        .or(Some(&JsonValue::from(1))),
                    "pool_max_idle_per_host",
                )?
                .into(),
            ),
        ),
    ])))
}

fn normalize_header_map(value: Option<&JsonValue>) -> Result<JsonMap<String, JsonValue>, String> {
    let Some(value) = value else {
        return Ok(JsonMap::new());
    };
    let object = require_object(Some(value), "header payload")?;
    let mut normalized = JsonMap::new();
    for (key, raw_value) in object {
        let normalized_key = optional_text(Some(&JsonValue::String(key.clone())))?;
        let normalized_value = optional_text(Some(raw_value))?;
        let Some(header_key) = normalized_key else {
            return Err("Header payloads must contain non-empty string keys.".to_string());
        };
        let Some(header_value) = normalized_value else {
            return Err("Header payloads must contain non-empty string values.".to_string());
        };
        normalized.insert(header_key, JsonValue::String(header_value));
    }
    Ok(normalized)
}

fn normalize_linked_item_rows(value: Option<&JsonValue>) -> Result<Vec<JsonValue>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let rows = value
        .as_array()
        .ok_or_else(|| "Linked task item rows must be a list.".to_string())?;
    let mut normalized = Vec::new();
    for row in rows {
        let object = require_object(Some(row), "linked task item row")?;
        let plan_id = require_text(object.get("plan_id"), "plan_id")?;
        let plan_item_ref = require_text(object.get("plan_item_ref"), "plan_item_ref")?;
        let tasks = normalize_object_list(object.get("tasks"), "tasks")?;
        normalized.push(JsonValue::Object(JsonMap::from_iter([
            ("plan_id".to_string(), JsonValue::String(plan_id)),
            (
                "plan_item_ref".to_string(),
                JsonValue::String(plan_item_ref),
            ),
            ("tasks".to_string(), JsonValue::Array(tasks)),
        ])));
    }
    Ok(normalized)
}

fn normalize_tasks_by_plan_rows(value: Option<&JsonValue>) -> Result<Vec<JsonValue>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let rows = value
        .as_array()
        .ok_or_else(|| "Tasks-by-plan rows must be a list.".to_string())?;
    let mut normalized = Vec::new();
    for row in rows {
        let object = require_object(Some(row), "tasks by plan row")?;
        let plan_id = require_text(object.get("plan_id"), "plan_id")?;
        let tasks = normalize_object_list(object.get("tasks"), "tasks")?;
        normalized.push(JsonValue::Object(JsonMap::from_iter([
            ("plan_id".to_string(), JsonValue::String(plan_id)),
            ("tasks".to_string(), JsonValue::Array(tasks)),
        ])));
    }
    Ok(normalized)
}

fn normalize_object_list(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<Vec<JsonValue>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| format!("Payload field `{field_name}` must be a list."))?;
    let mut normalized = Vec::new();
    for entry in items {
        let object = require_object(Some(entry), field_name)?;
        normalized.push(JsonValue::Object(object.clone()));
    }
    Ok(normalized)
}

fn require_object<'a>(
    value: Option<&'a JsonValue>,
    field_name: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    match value {
        Some(JsonValue::Object(values)) => Ok(values),
        _ => Err(format!("{field_name} must be an object.")),
    }
}

fn require_named_operation(
    value: Option<&JsonValue>,
    allowed: &[&str],
    label: &str,
) -> Result<String, String> {
    let operation = require_text(value, &format!("{label} operation"))?;
    if !allowed.contains(&operation.as_str()) {
        return Err(format!(
            "Unsupported {label} operation `{operation}`. Expected one of: {}.",
            allowed.join(", ")
        ));
    }
    Ok(operation)
}

fn normalize_backend_name(value: String) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    if matches!(normalized.as_str(), "python" | "rust") {
        Ok(normalized)
    } else {
        Err(format!("Unsupported core backend: {value}"))
    }
}

fn require_text(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    optional_text(value)?
        .ok_or_else(|| format!("Payload field `{field_name}` must be a non-empty string."))
}

fn optional_exact_string(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => Ok(Some(text.clone())),
        _ => Err(format!(
            "Payload field `{field_name}` must be a string when provided."
        )),
    }
}

fn optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        _ => Err("Text payload fields must be strings.".to_string()),
    }
}

fn require_bool(value: Option<&JsonValue>, field_name: &str) -> Result<bool, String> {
    match value {
        Some(JsonValue::Bool(result)) => Ok(*result),
        _ => Err(format!("Payload field `{field_name}` must be a boolean.")),
    }
}

fn optional_bool(value: Option<&JsonValue>) -> Result<Option<bool>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(result)) => Ok(Some(*result)),
        _ => Err("Boolean payload fields must be booleans.".to_string()),
    }
}

fn optional_nonnegative_i64(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<Option<i64>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(_) => Ok(Some(require_nonnegative_i64(value, field_name)?)),
    }
}

fn require_nonnegative_i64(value: Option<&JsonValue>, field_name: &str) -> Result<i64, String> {
    let Some(value) = value else {
        return Err(format!("Payload field `{field_name}` must be an integer."));
    };
    let normalized = match value {
        JsonValue::Number(number) => number
            .as_i64()
            .ok_or_else(|| format!("Payload field `{field_name}` must be an integer."))?,
        _ => return Err(format!("Payload field `{field_name}` must be an integer.")),
    };
    if normalized < 0 {
        return Err(format!("Payload field `{field_name}` must be >= 0."));
    }
    Ok(normalized)
}

fn require_positive_i64(value: Option<&JsonValue>, field_name: &str) -> Result<i64, String> {
    let normalized = require_nonnegative_i64(value, field_name)?;
    if normalized < 1 {
        return Err(format!("Payload field `{field_name}` must be >= 1."));
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests;
