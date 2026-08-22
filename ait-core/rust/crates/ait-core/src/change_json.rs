use crate::json_support::{json, JsonMap, JsonValue};
use crate::land_json::LandJson;
use crate::plan_http_client::{
    build_plan_http_request_spec, configured_repository_authority_path_segment,
    encode_path_segment, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult,
    PlanHttpRequestSpec,
};
use crate::text_normalization::normalize_optional_text;
use reqwest::Method;

pub struct ChangeJson<S> {
    store: S,
}

impl<S> ChangeJson<S> {
    pub fn canonical_change_id(&self, value: &str) -> Result<String, String> {
        let _ = &self.store;
        let value = normalize_optional_text(Some(value))
            .ok_or_else(|| "change_id must not be empty.".to_string())?;
        let Some((_, child)) = value.rsplit_once('/') else {
            return Ok(value);
        };
        if is_short_change_id(child) {
            Ok(child.to_string())
        } else {
            Ok(value)
        }
    }

    pub fn normalize_remote_change_payload(
        &self,
        payload: &JsonValue,
        expected_task_id: Option<&str>,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        let mut object = require_object(Some(payload), "remote Change payload")?.clone();
        let expected_task_id = normalize_optional_text(expected_task_id);
        let payload_task_id = object
            .get("task_id")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_optional_text(Some(value)));
        if let (Some(expected), Some(actual)) =
            (expected_task_id.as_deref(), payload_task_id.as_deref())
        {
            if expected != actual {
                return Err(format!(
                    "Remote Change belongs to task `{actual}`, not expected task `{expected}`."
                ));
            }
        }
        let task_id = expected_task_id.or(payload_task_id);
        let raw_change_id = object
            .get("change_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "Remote Change payload is missing change_id.".to_string())?;
        if let (Some(task_id), Some((prefix, child))) =
            (task_id.as_deref(), raw_change_id.rsplit_once('/'))
        {
            if is_short_change_id(child) && prefix != task_id {
                return Err(format!(
                    "Remote Change id `{raw_change_id}` belongs to task `{prefix}`, not `{task_id}`."
                ));
            }
        }
        let canonical_change_id = self.canonical_change_id(raw_change_id)?;
        let change_ref = if is_short_change_id(&canonical_change_id) {
            let task_id = task_id.as_deref().ok_or_else(|| {
                format!(
                    "Remote Change `{canonical_change_id}` is missing task_id required to derive change_ref."
                )
            })?;
            format!("{task_id}/{canonical_change_id}")
        } else {
            object
                .get("change_ref")
                .and_then(JsonValue::as_str)
                .and_then(|value| normalize_optional_text(Some(value)))
                .unwrap_or_else(|| raw_change_id.to_string())
        };
        if let Some(provided_change_ref) = object
            .get("change_ref")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_optional_text(Some(value)))
        {
            if provided_change_ref != change_ref {
                return Err(format!(
                    "Remote Change change_ref `{provided_change_ref}` does not match derived `{change_ref}`."
                ));
            }
        }
        object.insert(
            "change_id".to_string(),
            JsonValue::String(canonical_change_id),
        );
        object.insert("change_ref".to_string(), JsonValue::String(change_ref));
        Ok(JsonValue::Object(object))
    }

    pub fn normalize_remote_change_detail_payload(
        &self,
        payload: &JsonValue,
        expected_task_id: Option<&str>,
    ) -> Result<JsonValue, String> {
        let mut object = require_object(Some(payload), "remote Change detail payload")?.clone();
        if object.get("change_id").is_some() {
            return self.normalize_remote_change_payload(payload, expected_task_id);
        }
        let change = object.get("change").ok_or_else(|| {
            "Remote Change detail payload is missing both change_id and nested change.".to_string()
        })?;
        let normalized_change = self.normalize_remote_change_payload(change, expected_task_id)?;
        object.insert("change".to_string(), normalized_change);
        Ok(JsonValue::Object(object))
    }

    pub fn normalize_remote_task_audit_payload(
        &self,
        payload: &JsonValue,
        expected_task_id: &str,
    ) -> Result<JsonValue, String> {
        let expected_task_id = normalize_optional_text(Some(expected_task_id))
            .ok_or_else(|| "Task audit task_id must not be empty.".to_string())?;
        let mut object = require_object(Some(payload), "remote Task audit payload")?.clone();
        if let Some(payload_task_id) = object
            .get("task_id")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_optional_text(Some(value)))
        {
            if payload_task_id != expected_task_id {
                return Err(format!(
                    "Remote Task audit belongs to task `{payload_task_id}`, not expected task `{expected_task_id}`."
                ));
            }
        }
        let Some(changes) = object.get("changes") else {
            return Ok(JsonValue::Object(object));
        };
        let changes = changes
            .as_array()
            .ok_or_else(|| "Remote Task audit changes must be an array.".to_string())?;
        let normalized_changes = changes
            .iter()
            .map(|change| {
                self.normalize_remote_change_detail_payload(change, Some(&expected_task_id))
            })
            .collect::<Result<Vec<_>, _>>()?;
        object.insert("changes".to_string(), JsonValue::Array(normalized_changes));
        Ok(JsonValue::Object(object))
    }

    pub fn rolling_server_change_id(
        &self,
        task_id: Option<&str>,
        change_id: &str,
    ) -> Result<String, String> {
        let _ = &self.store;
        let change_id = normalize_optional_text(Some(change_id))
            .ok_or_else(|| "change_id must not be empty.".to_string())?;
        if !is_short_change_id(&change_id) {
            return Ok(change_id);
        }
        let task_id = normalize_optional_text(task_id).ok_or_else(|| {
            format!(
                "Task context is required to resolve short change_id `{change_id}`; refusing an ambiguous repository-wide lookup."
            )
        })?;
        Ok(format!("{task_id}/{change_id}"))
    }

    pub fn new(store: S) -> Self {
        Self { store }
    }
}

fn is_short_change_id(value: &str) -> bool {
    let Some(ordinal) = value.strip_prefix("C-") else {
        return false;
    };
    !ordinal.is_empty() && ordinal.bytes().all(|byte| byte.is_ascii_digit())
}

impl ChangeJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> ChangeJson<S> {
    pub fn normalize_linked_change_lookup_payload(
        &self,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        let object = require_object(Some(payload), "linked change lookup payload")?;
        let change_links_by_task =
            normalize_change_links_by_task_rows(object.get("change_links_by_task"))?;
        let linked_change_count = match object.get("linked_change_count") {
            Some(value) => require_nonnegative_i64(Some(value), "linked_change_count")?,
            None => change_links_by_task
                .iter()
                .map(|row| {
                    row.get("changes")
                        .and_then(JsonValue::as_array)
                        .map(|items| items.len() as i64)
                        .unwrap_or(0)
                })
                .sum(),
        };
        Ok(JsonValue::Object(JsonMap::from_iter([
            (
                "change_links_by_task".to_string(),
                JsonValue::Array(change_links_by_task),
            ),
            (
                "linked_change_count".to_string(),
                JsonValue::Number(linked_change_count.into()),
            ),
        ])))
    }

    pub fn build_linked_change_lookup_payload(
        &self,
        change_links_by_task_rows: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        self.normalize_linked_change_lookup_payload(&json!({
            "change_links_by_task": change_links_by_task_rows.cloned().unwrap_or(JsonValue::Array(vec![]))
        }))
    }

    pub fn recover_land_submission_from_change_state(
        &self,
        change: &JsonValue,
        fallback_change_id: &str,
    ) -> Option<JsonValue> {
        let _ = &self.store;
        LandJson::stateless().recover_land_submission_from_change_state(change, fallback_change_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn build_create_change_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let body = self.build_create_change_body(
            task_id,
            title,
            base_line,
            change_id,
            fork_snapshot_id,
            forked_from_line,
        )?;
        let repository_index = configured_repository_authority_path_segment(config)?;
        build_plan_http_request_spec(
            config,
            Method::POST,
            &format!("/v1/native/repository-authorities/{repository_index}/changes"),
            Vec::new(),
            Some(body),
        )
    }

    pub fn build_list_changes_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        _repo_name: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!("/v1/native/repository-authorities/{repository_index}/changes");
        build_plan_http_request_spec(config, Method::GET, &path, Vec::new(), None)
    }

    pub fn build_get_change_detail_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let change_ref =
            encode_path_segment(&require_plan_http_non_empty_text(change_ref, "change_ref")?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path =
            format!("/v1/native/repository-authorities/{repository_index}/changes/{change_ref}");
        build_plan_http_request_spec(config, Method::GET, &path, Vec::new(), None)
    }

    pub fn build_get_change_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let change_ref =
            encode_path_segment(&require_plan_http_non_empty_text(change_ref, "change_ref")?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path =
            format!("/v1/native/repository-authorities/{repository_index}/changes/{change_ref}");
        build_plan_http_request_spec(config, Method::GET, &path, Vec::new(), None)
    }

    pub fn build_close_change_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        change_id: &str,
        status: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let change_id =
            encode_path_segment(&require_plan_http_non_empty_text(change_id, "change_id")?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/changes/{change_id}:close"
        );
        build_plan_http_request_spec(
            config,
            Method::POST,
            &path,
            Vec::new(),
            Some(self.build_close_change_body(status)?),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_create_change_body(
        &self,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> PlanHttpClientResult<JsonValue> {
        let task_id = require_plan_http_non_empty_text(task_id, "task_id")?;
        let title = require_plan_http_non_empty_text(title, "title")?;
        let base_line = require_plan_http_non_empty_text(base_line, "base_line")?;
        let mut body = JsonMap::new();
        body.insert("task_id".to_string(), JsonValue::String(task_id));
        body.insert("title".to_string(), JsonValue::String(title));
        body.insert("base_line".to_string(), JsonValue::String(base_line));
        insert_optional_string(&mut body, "change_id", change_id);
        insert_optional_string(&mut body, "fork_snapshot_id", fork_snapshot_id);
        insert_optional_string(&mut body, "forked_from_line", forked_from_line);
        Ok(JsonValue::Object(body))
    }

    fn build_close_change_body(&self, status: &str) -> PlanHttpClientResult<JsonValue> {
        let status = require_plan_http_non_empty_text(status, "status")?;
        Ok(json!({ "status": status }))
    }
}

fn normalize_change_links_by_task_rows(
    value: Option<&JsonValue>,
) -> Result<Vec<JsonValue>, String> {
    let rows = normalize_array(value, "change_links_by_task")?;
    rows.into_iter()
        .map(|row| {
            let object = require_object(Some(&row), "change_links_by_task row")?;
            let task_id = require_text(object.get("task_id"), "task_id")?;
            let changes = normalize_object_list(object.get("changes"), "changes")?;
            Ok(json!({
                "task_id": task_id,
                "changes": changes,
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

fn require_nonnegative_i64(value: Option<&JsonValue>, field_name: &str) -> Result<i64, String> {
    let Some(JsonValue::Number(number)) = value else {
        return Err(format!("`{field_name}` must be an integer."));
    };
    let Some(integer) = number.as_i64() else {
        return Err(format!("`{field_name}` must be an integer."));
    };
    if integer < 0 {
        return Err(format!("`{field_name}` must be non-negative."));
    }
    Ok(integer)
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

#[cfg(test)]
mod tests;
