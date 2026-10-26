use crate::json_support::{json, JsonCodec, JsonMap, JsonValue};
use crate::plan_http_client::{
    build_plan_http_request_spec, configured_repository_authority_path_segment,
    encode_path_segment, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult,
    PlanHttpRequestSpec,
};
use crate::plan_ports_protocols;
use crate::text_normalization::normalize_optional_text;
use reqwest::Method;

pub struct TaskJson<S> {
    store: S,
}

impl<S> TaskJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl TaskJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> TaskJson<S> {
    pub fn normalize_linked_task_lookup_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "linked task lookup payload")?;
        plan_ports_protocols::normalize_linked_task_lookup_payload_map(payload)
    }

    pub fn build_linked_task_lookup_payload(
        &self,
        task_links_by_item_rows: Option<&JsonValue>,
        tasks_by_plan_rows: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        let task_links_by_item = normalize_linked_item_rows(task_links_by_item_rows)?;
        let tasks_by_plan = normalize_tasks_by_plan_rows(tasks_by_plan_rows)?;
        let linked_task_count: i64 = tasks_by_plan
            .iter()
            .map(|row| {
                row.get("tasks")
                    .and_then(JsonValue::as_array)
                    .map(|items| items.len() as i64)
                    .unwrap_or(0)
            })
            .sum();
        self.normalize_linked_task_lookup_payload_json(
            &json!({
                "task_links_by_item": task_links_by_item,
                "tasks_by_plan": tasks_by_plan,
                "linked_task_count": linked_task_count,
            })
            .to_string(),
        )
    }

    pub fn build_task_tracking_title_payload(&self, task: &JsonValue) -> Result<JsonValue, String> {
        let object = require_object(Some(task), "task")?;
        let task_id = optional_text_field(object, "task_id");
        let title = optional_text_field(object, "title");
        let resolved_title = match (task_id.as_deref(), title.as_deref()) {
            (Some(task_id), Some(title)) => format!("{task_id}: {title}"),
            (_, Some(title)) => title.to_string(),
            (Some(task_id), _) => task_id.to_string(),
            _ => "Tracked task".to_string(),
        };
        Ok(json!({ "title": resolved_title }))
    }

    pub fn build_task_tracking_metadata_payload(
        &self,
        task: &JsonValue,
        author_mode: &str,
        tracking_policy: &str,
    ) -> Result<JsonValue, String> {
        let object = require_object(Some(task), "task")?;
        let author_mode = normalize_optional_text(Some(author_mode))
            .ok_or_else(|| "`author_mode` must be non-empty.".to_string())?;
        let tracking_policy = normalize_optional_text(Some(tracking_policy))
            .ok_or_else(|| "`tracking_policy` must be non-empty.".to_string())?;
        let mut metadata = JsonMap::new();
        metadata.insert("author_mode".to_string(), JsonValue::String(author_mode));
        metadata.insert(
            "tracking_policy".to_string(),
            JsonValue::String(tracking_policy),
        );
        if let Some(task_id) = optional_text_field(object, "task_id") {
            metadata.insert("task_id".to_string(), JsonValue::String(task_id));
        }
        if let Some(intent) = optional_text_field(object, "intent") {
            metadata.insert("objective".to_string(), JsonValue::String(intent));
        }
        Ok(JsonValue::Object(metadata))
    }

    pub fn build_task_audit_verdict_payload(
        &self,
        task: &JsonValue,
        change_rows: &JsonValue,
        target_line: &str,
    ) -> Result<JsonValue, String> {
        let task = require_object(Some(task), "task")?;
        let change_rows = require_array(Some(change_rows), "change_rows")?;
        let target_line = require_nonempty_text(
            Some(&JsonValue::String(target_line.to_string())),
            "target_line",
        )?;

        let open_change_count = count_change_rows(change_rows, |row| {
            !change_status_is_terminal(change_status(row).as_deref())
        });
        let landed_change_count = count_change_rows(change_rows, |row| {
            matches!(change_status(row).as_deref(), Some("landed"))
        });
        let effective_on_target_count =
            count_change_rows(change_rows, |row| bool_field(row, "effective_on_target"));
        let open_on_target_count = count_change_rows(change_rows, |row| {
            bool_field(row, "effective_on_target")
                && !change_status_is_terminal(change_status(row).as_deref())
        });
        let stale_workflow_count =
            count_change_rows(change_rows, |row| bool_field(row, "stale_workflow_record"));
        let ambiguous_line_count = count_change_rows(change_rows, |row| {
            string_field(row, "target_state").as_deref() == Some("ambiguous_line_candidates")
        });
        let line_evidence_count = count_change_rows(change_rows, |row| {
            row.get("preferred_line").is_some()
                && !row.get("preferred_line").is_some_and(JsonValue::is_null)
        });
        let effectively_complete_on_target = !change_rows.is_empty()
            && change_rows.iter().all(|row| {
                matches!(
                    change_status_from_value(row.get("change")).as_deref(),
                    Some("archived")
                ) || bool_field_from_value(row.get("effective_on_target"))
            })
            && change_rows
                .iter()
                .any(|row| bool_field_from_value(row.get("effective_on_target")));

        let task_status = optional_text(task.get("status"))?.unwrap_or_default();

        let (verdict, workflow_state, workflow_reason, recommended_action) = if task_status
            == "completed"
        {
            (
                "task_completed",
                "task_completed",
                "The local task is already completed.".to_string(),
                action(
                    "none",
                    "No action required",
                    "The local task is already completed.",
                    None,
                ),
            )
        } else if matches!(task_status.as_str(), "abandoned" | "canceled") {
            (
                "task_abandoned",
                "task_abandoned",
                "The local task is already abandoned.".to_string(),
                action(
                    "none",
                    "No action required",
                    "The local task is already abandoned.",
                    None,
                ),
            )
        } else if task_status == "later_promotion_excluded" {
            (
                "task_later_promotion_excluded",
                "task_later_promotion_excluded",
                "The local task has already been excluded from later promotion.".to_string(),
                action(
                    "none",
                    "No action required",
                    "The local task has already been excluded from later promotion.",
                    None,
                ),
            )
        } else if change_rows.is_empty() {
            (
                "no_changes",
                "planning",
                "No linked local changes exist yet.".to_string(),
                action(
                    "create_change",
                    "Create a change",
                    "This task has no linked local changes yet.",
                    None,
                ),
            )
        } else if effectively_complete_on_target {
            (
                    "workflow_missing_on_target",
                    "workflow_missing_on_target",
                    format!(
                        "Linked changes already appear on {target_line}, but the remote task record is missing."
                    ),
                    action(
                        "reconcile_workflow_records",
                        "Reconcile workflow records",
                        &format!(
                            "The remote task record is missing, but local line evidence indicates this task is already absorbed into {target_line}."
                        ),
                        None,
                    ),
                )
        } else if ambiguous_line_count > 0 {
            (
                    "needs_line_inspection",
                    "needs_line_inspection",
                    "Task audit found multiple candidate lines and could not safely infer closure from the preferred line alone.".to_string(),
                    action(
                        "inspect_candidate_lines",
                        "Inspect candidate lines",
                        "Review the inferred local line candidates before deciding whether to reconcile or continue the task.",
                        None,
                    ),
                )
        } else if open_on_target_count > 0 {
            (
                    "partially_on_target",
                    "partially_on_target",
                    format!(
                        "Some inferred local line heads already appear on {target_line}, while other linked changes still need work."
                    ),
                    action(
                        "inspect_stale_workflow",
                        "Inspect stale workflow state",
                        "At least one inferred local line head is already on target, but the task is not fully absorbed.",
                        None,
                    ),
                )
        } else {
            let open_change = change_rows.iter().find_map(|row| {
                let row = row.as_object()?;
                if change_status_is_terminal(change_status(row).as_deref()) {
                    None
                } else {
                    row.get("change")
                        .and_then(JsonValue::as_object)
                        .and_then(|change| optional_text(change.get("change_id")).ok().flatten())
                }
            });
            (
                    "not_landed_on_target",
                    "in_progress",
                    format!(
                        "No inferred local line head for this task is reachable from {target_line}."
                    ),
                    action(
                        "continue_task_work",
                        "Continue task work",
                        "Keep working on the task and publish or repair workflow records once reviewable work exists.",
                        open_change.as_deref(),
                    ),
                )
        };

        Ok(json!({
            "workflow": {
                "state": workflow_state,
                "reason": workflow_reason,
            },
            "recommended_action": recommended_action,
            "summary": {
                "change_count": change_rows.len(),
                "open_change_count": open_change_count,
                "landed_change_count": landed_change_count,
                "patchset_count": 0,
                "effective_on_target_change_count": effective_on_target_count,
                "open_on_target_change_count": open_on_target_count,
                "stale_workflow_change_count": stale_workflow_count,
                "ready_to_complete": false,
                "effectively_complete_on_target": effectively_complete_on_target,
                "stale_workflow_records": stale_workflow_count > 0,
                "missing_remote_change_count": count_change_rows(change_rows, |row| {
                    bool_field(row, "missing_remote_record")
                }),
                "line_evidence_change_count": line_evidence_count,
                "ambiguous_line_change_count": ambiguous_line_count,
                "verdict": verdict,
            },
        }))
    }

    pub fn recover_closed_task_from_state(
        &self,
        task: &JsonValue,
        fallback_task_id: &str,
    ) -> Option<JsonValue> {
        let task_map = task.as_object()?;
        let status = normalize_optional_text_value(task_map.get("status"))?;
        if !status.eq_ignore_ascii_case("completed") {
            return None;
        }
        let task_id = self.resolved_task_id_from_task_payload(task, fallback_task_id);
        Some(json!({
            "task_id": task_id,
            "status": "completed",
            "response_recovery": {
                "action": "complete_task",
                "state": "recovered_from_remote_task_status",
                "task_id": task_id,
            }
        }))
    }

    pub fn resolved_task_id_from_task_payload(
        &self,
        task: &JsonValue,
        fallback_task_id: &str,
    ) -> String {
        task.get("task_id")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|task_id| !task_id.is_empty())
            .unwrap_or(fallback_task_id)
            .to_string()
    }

    pub fn build_list_tasks_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let repository_index = configured_repository_authority_path_segment(config)?;
        build_plan_http_request_spec(
            config,
            Method::GET,
            &format!("/v1/native/repository-authorities/{repository_index}/tasks"),
            Vec::new(),
            None,
        )
    }

    pub fn build_get_task_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let task_id = encode_path_segment(&require_plan_http_non_empty_text(task_id, "task_id")?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!("/v1/native/repository-authorities/{repository_index}/tasks/{task_id}");
        build_plan_http_request_spec(config, Method::GET, &path, Vec::new(), None)
    }

    pub fn build_read_task_audit_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let repository_index = configured_repository_authority_path_segment(config)?;
        let task_id = encode_path_segment(&require_plan_http_non_empty_text(task_id, "task_id")?);
        let target_line = require_plan_http_non_empty_text(target_line, "target_line")?;
        build_plan_http_request_spec(
            config,
            Method::GET,
            &format!(
                "/v1/native/repository-authorities/{repository_index}/read/tasks/{task_id}/audit"
            ),
            vec![("target_line".to_string(), target_line)],
            None,
        )
    }

    pub fn build_read_task_queue_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
        status: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let repository_index = configured_repository_authority_path_segment(config)?;
        let mut query_pairs = Vec::new();
        if let Some(status_value) = normalize_optional_text(status) {
            query_pairs.push(("status".to_string(), status_value));
        }
        build_plan_http_request_spec(
            config,
            Method::GET,
            &format!("/v1/native/repository-authorities/{repository_index}/read/task-queue"),
            query_pairs,
            None,
        )
    }

    pub fn build_read_reviewer_inbox_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let repository_index = configured_repository_authority_path_segment(config)?;
        build_plan_http_request_spec(
            config,
            Method::GET,
            &format!("/v1/native/repository-authorities/{repository_index}/read/reviewer-inbox"),
            Vec::new(),
            None,
        )
    }

    pub fn build_read_queue_summary_bundle_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
        status: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let repository_index = configured_repository_authority_path_segment(config)?;
        let mut query_pairs = Vec::new();
        if let Some(status_value) = normalize_optional_text(status) {
            query_pairs.push(("status".to_string(), status_value));
        }
        build_plan_http_request_spec(
            config,
            Method::GET,
            &format!("/v1/native/repository-authorities/{repository_index}/read/queue-summary"),
            query_pairs,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_create_task_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let body = self.build_create_task_body(
            title,
            intent,
            task_id,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        )?;
        let repository_index = configured_repository_authority_path_segment(config)?;
        build_plan_http_request_spec(
            config,
            Method::POST,
            &format!("/v1/native/repository-authorities/{repository_index}/tasks"),
            Vec::new(),
            Some(body),
        )
    }

    pub fn build_close_task_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        task_id: &str,
        status: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let task_id = encode_path_segment(&require_plan_http_non_empty_text(task_id, "task_id")?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path =
            format!("/v1/native/repository-authorities/{repository_index}/tasks/{task_id}:close");
        build_plan_http_request_spec(
            config,
            Method::POST,
            &path,
            Vec::new(),
            Some(self.build_close_task_body(status)?),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_create_task_body(
        &self,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanHttpClientResult<JsonValue> {
        let title = require_plan_http_non_empty_text(title, "title")?;
        let intent = require_plan_http_non_empty_text(intent, "intent")?;
        let mut body = JsonMap::new();
        body.insert("title".to_string(), JsonValue::String(title));
        body.insert("intent".to_string(), JsonValue::String(intent));
        insert_optional_string(&mut body, "task_id", task_id);
        insert_optional_string(&mut body, "plan_id", plan_id);
        insert_optional_string(
            &mut body,
            "origin_plan_revision_id",
            origin_plan_revision_id,
        );
        insert_optional_string(&mut body, "plan_item_ref", plan_item_ref);
        Ok(JsonValue::Object(body))
    }

    fn build_close_task_body(&self, status: &str) -> PlanHttpClientResult<JsonValue> {
        let status = require_plan_http_non_empty_text(status, "status")?;
        Ok(json!({ "status": status }))
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        JsonCodec::parse_object_with_error_prefix(
            payload_json,
            &format!("Failed to parse {label} JSON"),
            &format!("{label} payload must decode to an object."),
        )
        .map_err(String::from)
    }
}

fn normalize_linked_item_rows(value: Option<&JsonValue>) -> Result<Vec<JsonValue>, String> {
    let rows = normalize_array(value, "task_links_by_item")?;
    rows.into_iter()
        .map(|row| {
            let object = require_object(Some(&row), "linked task row")?;
            let plan_id = require_text(object.get("plan_id"), "plan_id")?;
            let plan_item_ref = require_text(object.get("plan_item_ref"), "plan_item_ref")?;
            let tasks = normalize_object_list(object.get("tasks"), "tasks")?;
            Ok(json!({
                "plan_id": plan_id,
                "plan_item_ref": plan_item_ref,
                "tasks": tasks,
            }))
        })
        .collect()
}

fn normalize_tasks_by_plan_rows(value: Option<&JsonValue>) -> Result<Vec<JsonValue>, String> {
    let rows = normalize_array(value, "tasks_by_plan")?;
    rows.into_iter()
        .map(|row| {
            let object = require_object(Some(&row), "tasks_by_plan row")?;
            let plan_id = require_text(object.get("plan_id"), "plan_id")?;
            let tasks = normalize_object_list(object.get("tasks"), "tasks")?;
            Ok(json!({
                "plan_id": plan_id,
                "tasks": tasks,
            }))
        })
        .collect()
}

fn normalize_object_list(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<Vec<JsonValue>, String> {
    normalize_array(value, field_name)?
        .into_iter()
        .map(|entry| {
            let object = require_object(Some(&entry), field_name)?.clone();
            Ok(JsonValue::Object(object))
        })
        .collect()
}

fn normalize_array(value: Option<&JsonValue>, field_name: &str) -> Result<Vec<JsonValue>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(items)) => Ok(items.clone()),
        Some(_) => Err(format!("`{field_name}` must be an array when provided.")),
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

fn require_array<'a>(
    value: Option<&'a JsonValue>,
    field_name: &str,
) -> Result<&'a Vec<JsonValue>, String> {
    match value {
        Some(JsonValue::Array(items)) => Ok(items),
        _ => Err(format!("`{field_name}` must be an array.")),
    }
}

fn require_text(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    let Some(JsonValue::String(text)) = value else {
        return Err(format!("`{field_name}` must be a string."));
    };
    let normalized = text.trim();
    if normalized.is_empty() {
        return Err(format!("`{field_name}` must be non-empty."));
    }
    Ok(normalized.to_string())
}

fn require_nonempty_text(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    let Some(JsonValue::String(text)) = value else {
        return Err(format!("`{field_name}` must be a string."));
    };
    let normalized = text.trim();
    if normalized.is_empty() {
        return Err(format!("`{field_name}` must be non-empty."));
    }
    Ok(normalized.to_string())
}

fn require_plan_http_non_empty_text(value: &str, field: &str) -> PlanHttpClientResult<String> {
    normalize_optional_text(Some(value)).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!("Plan HTTP {field} must not be empty."))
    })
}

fn insert_optional_string(body: &mut JsonMap<String, JsonValue>, key: &str, value: Option<&str>) {
    if let Some(text) = normalize_optional_text(value) {
        body.insert(key.to_string(), JsonValue::String(text));
    }
}

fn action(code: &str, label: &str, detail: &str, change_id: Option<&str>) -> JsonValue {
    json!({
        "code": code,
        "label": label,
        "detail": detail,
        "change_id": change_id,
    })
}

fn count_change_rows<F>(rows: &[JsonValue], predicate: F) -> i64
where
    F: Fn(&JsonMap<String, JsonValue>) -> bool,
{
    rows.iter()
        .filter_map(JsonValue::as_object)
        .filter(|row| predicate(row))
        .count() as i64
}

fn change_status(row: &JsonMap<String, JsonValue>) -> Option<String> {
    change_status_from_value(row.get("change"))
}

fn change_status_from_value(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_object)
        .and_then(|change| optional_text(change.get("status")).ok().flatten())
}

fn change_status_is_terminal(status: Option<&str>) -> bool {
    matches!(
        status,
        Some(
            "landed"
                | "closed"
                | "archived"
                | "canceled"
                | "cancelled"
                | "abandoned"
                | "superseded"
        )
    )
}

fn bool_field(row: &JsonMap<String, JsonValue>, field_name: &str) -> bool {
    bool_field_from_value(row.get(field_name))
}

fn bool_field_from_value(value: Option<&JsonValue>) -> bool {
    value.and_then(JsonValue::as_bool).unwrap_or(false)
}

fn string_field(row: &JsonMap<String, JsonValue>, field_name: &str) -> Option<String> {
    optional_text(row.get(field_name)).ok().flatten()
}

fn optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => {
            let normalized = text.trim();
            if normalized.is_empty() {
                Ok(None)
            } else {
                Ok(Some(normalized.to_string()))
            }
        }
        Some(_) => Err("Optional text field must be a string when provided.".to_string()),
    }
}

fn optional_text_field(object: &JsonMap<String, JsonValue>, field_name: &str) -> Option<String> {
    object
        .get(field_name)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_optional_text_value(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests;
