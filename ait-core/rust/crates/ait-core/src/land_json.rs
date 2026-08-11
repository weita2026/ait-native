use crate::json_support::{json, JsonCodec, JsonMap, JsonValue};
use crate::plan_http_client::{
    build_plan_http_request_spec, configured_repository_authority_path_segment,
    encode_path_segment, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult,
    PlanHttpRequestSpec,
};
use crate::text_normalization::normalize_optional_text;
use reqwest::Method;

pub struct LandJson<S> {
    store: S,
}

impl<S> LandJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl LandJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> LandJson<S> {
    pub fn build_submit_task_land_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        task_or_change_ref: &str,
        target_line: Option<&str>,
        mode: &str,
        idempotency_key: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let task_or_change_ref =
            require_plan_http_non_empty_text(task_or_change_ref, "task_or_change_ref")?;
        let mode = require_plan_http_non_empty_text(mode, "mode")?;
        let idempotency_key = require_plan_http_non_empty_text(idempotency_key, "idempotency_key")?;
        if idempotency_key.len() > 256 {
            return Err(PlanHttpClientError::Invalid(
                "Plan HTTP idempotency_key must not exceed 256 bytes.".to_string(),
            ));
        }
        let mut body = JsonMap::new();
        body.insert(
            "contract".to_string(),
            JsonValue::String("task-land-atomic/v1".to_string()),
        );
        body.insert(
            "idempotency_key".to_string(),
            JsonValue::String(idempotency_key),
        );
        body.insert(
            "task_or_change_ref".to_string(),
            JsonValue::String(task_or_change_ref),
        );
        body.insert("mode".to_string(), JsonValue::String(mode));
        if let Some(target_line) = normalize_optional_text(target_line) {
            body.insert("target_line".to_string(), JsonValue::String(target_line));
        }
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!("/v1/native/repository-authorities/{repository_index}/task-land");
        build_plan_http_request_spec(
            config,
            Method::POST,
            &path,
            Vec::new(),
            Some(JsonValue::Object(body)),
        )
    }

    pub fn build_submit_land_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let change_id =
            encode_path_segment(&require_plan_http_non_empty_text(change_id, "change_id")?);
        let body = self.build_submit_land_body(patchset_id, target_line, mode)?;
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/changes/{change_id}:submit"
        );
        build_plan_http_request_spec(config, Method::POST, &path, Vec::new(), Some(body))
    }

    pub fn build_get_land_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let submission_id = encode_path_segment(&require_plan_http_non_empty_text(
            submission_id,
            "submission_id",
        )?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path =
            format!("/v1/native/repository-authorities/{repository_index}/lands/{submission_id}");
        build_plan_http_request_spec(config, Method::GET, &path, Vec::new(), None)
    }

    pub fn build_retry_land_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let _ = repo_name;
        let submission_id = encode_path_segment(&require_plan_http_non_empty_text(
            submission_id,
            "submission_id",
        )?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/lands/{submission_id}:retry"
        );
        build_plan_http_request_spec(
            config,
            Method::POST,
            &path,
            Vec::new(),
            Some(self.build_retry_land_body(reason)),
        )
    }

    pub fn build_submit_land_body(
        &self,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
    ) -> PlanHttpClientResult<JsonValue> {
        let _ = &self.store;
        let target_line = require_plan_http_non_empty_text(target_line, "target_line")?;
        let mode = require_plan_http_non_empty_text(mode, "mode")?;
        Ok(json!({
            "patchset_id": optional_json_string_value(patchset_id),
            "target_line": target_line,
            "mode": mode,
        }))
    }

    pub fn build_retry_land_body(&self, reason: Option<&str>) -> JsonValue {
        let _ = &self.store;
        json!({ "reason": optional_json_string_value(reason) })
    }

    pub fn normalize_land_submission_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "land submission payload")?;
        self.normalize_land_submission_payload(&JsonValue::Object(payload))
    }

    pub fn normalize_land_submission_payload(
        &self,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "land submission payload")?.clone(),
        ))
    }

    pub fn normalize_landing_summary_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "landing summary payload")?;
        self.normalize_landing_summary_payload(&JsonValue::Object(payload))
    }

    pub fn normalize_landing_summary_payload(
        &self,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "landing summary payload")?.clone(),
        ))
    }

    pub fn optional_submission_id(&self, payload: &JsonValue) -> Option<String> {
        let _ = &self.store;
        optional_json_text(payload.get("submission_id"))
    }

    pub fn optional_land_status(&self, payload: &JsonValue) -> Option<String> {
        let _ = &self.store;
        optional_json_text(payload.get("status"))
    }

    pub fn optional_landed_snapshot_id(&self, payload: &JsonValue) -> Option<String> {
        let _ = &self.store;
        optional_json_text(payload.get("landed_snapshot_id")).or_else(|| {
            payload
                .get("result")
                .and_then(JsonValue::as_object)
                .and_then(|result| optional_json_text(result.get("landed_snapshot_id")))
        })
    }

    pub fn optional_landed_at(&self, payload: &JsonValue) -> Option<String> {
        let _ = &self.store;
        optional_json_text(payload.get("landed_at"))
    }

    pub fn landing_result(&self, payload: &JsonValue) -> JsonMap<String, JsonValue> {
        let _ = &self.store;
        payload
            .get("result")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default()
    }

    pub fn optional_blocker_class(&self, payload: &JsonValue) -> Option<String> {
        let _ = &self.store;
        optional_json_text(payload.get("blocker_class")).or_else(|| {
            payload
                .get("result")
                .and_then(JsonValue::as_object)
                .and_then(|result| optional_json_text(result.get("blocker_class")))
        })
    }

    pub fn landing_policy(&self, payload: &JsonValue) -> Option<JsonValue> {
        let _ = &self.store;
        payload
            .get("policy")
            .or_else(|| {
                payload
                    .get("result")
                    .and_then(JsonValue::as_object)
                    .and_then(|result| result.get("policy"))
            })
            .and_then(JsonValue::as_object)
            .cloned()
            .map(JsonValue::Object)
    }

    pub fn land_status_is_pending(&self, status: &str) -> bool {
        let _ = &self.store;
        matches!(
            status.trim().to_ascii_lowercase().as_str(),
            "queued" | "running"
        )
    }

    pub fn land_status_is_success(&self, status: &str) -> bool {
        let _ = &self.store;
        matches!(
            status.trim().to_ascii_lowercase().as_str(),
            "succeeded" | "landed" | "complete" | "completed"
        )
    }

    pub fn land_status_has_landing_evidence(&self, status: &str) -> bool {
        let _ = &self.store;
        self.land_status_is_success(status) || status.trim().eq_ignore_ascii_case("applied")
    }

    pub fn land_status_is_blocked(&self, status: &str) -> bool {
        let _ = &self.store;
        status.trim().eq_ignore_ascii_case("blocked")
    }

    pub fn change_effectively_landed(
        &self,
        change: &JsonMap<String, JsonValue>,
        landing_summary: Option<&JsonMap<String, JsonValue>>,
    ) -> bool {
        let _ = &self.store;
        if optional_json_text(change.get("status")).as_deref() == Some("landed")
            || optional_json_text(change.get("landed_snapshot_id")).is_some()
            || optional_json_text(change.get("landed_at")).is_some()
        {
            return true;
        }
        let Some(landing_summary) = landing_summary else {
            return false;
        };
        let landing_status = self.landing_summary_status(Some(landing_summary));
        self.land_status_is_success(&landing_status)
            && self
                .landing_summary_result(Some(landing_summary))
                .get("landed_snapshot_id")
                .and_then(|value| optional_json_text(Some(value)))
                .is_some()
    }

    pub fn change_has_landed_status(&self, change: &JsonValue) -> bool {
        let _ = &self.store;
        change_payload_text(change, "status")
            .is_some_and(|status| status.eq_ignore_ascii_case("landed"))
    }

    pub fn change_has_landing_evidence(&self, change: &JsonValue) -> bool {
        let _ = &self.store;
        if self.change_has_landed_status(change) {
            return true;
        }
        if change_payload_text(change, "landed_snapshot_id").is_some()
            || change_payload_text(change, "landed_at").is_some()
        {
            return true;
        }
        let Some(landing_summary) = change_payload_object(change, "landing_summary") else {
            return false;
        };
        let landing_status = self.landing_summary_status(Some(landing_summary));
        if self.land_status_has_landing_evidence(&landing_status) {
            return true;
        }
        let landing_result = self.landing_summary_result(Some(landing_summary));
        optional_json_text(landing_result.get("landed_snapshot_id")).is_some()
            || optional_json_text(landing_result.get("target_line_head_snapshot_id")).is_some()
    }

    pub fn landing_summary_status(
        &self,
        landing_summary: Option<&JsonMap<String, JsonValue>>,
    ) -> String {
        let _ = &self.store;
        landing_summary
            .and_then(|value| optional_json_text(value.get("status")))
            .unwrap_or_default()
            .to_ascii_lowercase()
    }

    pub fn landing_summary_submission_id(
        &self,
        landing_summary: Option<&JsonMap<String, JsonValue>>,
    ) -> Option<String> {
        let _ = &self.store;
        landing_summary.and_then(|value| optional_json_text(value.get("submission_id")))
    }

    pub fn landing_summary_result(
        &self,
        landing_summary: Option<&JsonMap<String, JsonValue>>,
    ) -> JsonMap<String, JsonValue> {
        let _ = &self.store;
        landing_summary
            .and_then(|value| value.get("result"))
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default()
    }

    pub fn landing_result_blocker_class(&self, result: &JsonMap<String, JsonValue>) -> String {
        let _ = &self.store;
        optional_json_text(result.get("blocker_class"))
            .unwrap_or_default()
            .to_ascii_uppercase()
    }

    pub fn landing_summary_blocker_class(
        &self,
        landing_summary: Option<&JsonMap<String, JsonValue>>,
    ) -> String {
        let _ = &self.store;
        let result = self.landing_summary_result(landing_summary);
        self.landing_result_blocker_class(&result)
    }

    pub fn stale_policy_blocker_cleared(
        &self,
        landing_status: &str,
        landing_blocker_class: &str,
        policy_decision: &str,
    ) -> bool {
        let _ = &self.store;
        self.land_status_is_blocked(landing_status)
            && landing_blocker_class.eq_ignore_ascii_case("POLICY_BLOCKED")
            && policy_decision == "pass"
    }

    pub fn recover_land_submission_from_change_state(
        &self,
        change: &JsonValue,
        fallback_change_id: &str,
    ) -> Option<JsonValue> {
        let _ = &self.store;
        let change_map = change.as_object()?;
        let landing_summary = change_map
            .get("landing_summary")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let submission_id = optional_json_text(
            landing_summary
                .get("submission_id")
                .or_else(|| change_map.get("landing_submission_id")),
        );
        let mut status = optional_json_text(landing_summary.get("status"));
        if status.is_none()
            && optional_json_text(change_map.get("status"))
                .map(|value| value.eq_ignore_ascii_case("landed"))
                .unwrap_or(false)
        {
            status = Some("succeeded".to_string());
        }
        let status = status?;
        let change_id = optional_json_text(change_map.get("change_id"))
            .unwrap_or_else(|| fallback_change_id.to_string());
        let landing_result = landing_summary
            .get("result")
            .and_then(JsonValue::as_object)
            .cloned();
        let mut recovered = JsonMap::new();
        recovered.insert("status".to_string(), JsonValue::String(status));
        if let Some(submission_id) = &submission_id {
            recovered.insert(
                "submission_id".to_string(),
                JsonValue::String(submission_id.clone()),
            );
        }
        if let Some(result) = landing_result {
            recovered.insert("result".to_string(), JsonValue::Object(result));
        }
        recovered.insert(
            "response_recovery".to_string(),
            json!({
                "action": "submit_land",
                "state": "recovered_from_remote_land_state",
                "submission_id": submission_id,
                "change_id": change_id,
            }),
        );
        Some(JsonValue::Object(recovered))
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let _ = &self.store;
        JsonCodec::parse_object_with_error_prefix(
            payload_json,
            &format!("{label} invalid JSON"),
            &format!("{label} must be an object."),
        )
        .map_err(|err| err.to_string())
    }
}

fn require_plan_http_non_empty_text(value: &str, field: &str) -> PlanHttpClientResult<String> {
    normalize_optional_text(Some(value)).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!("Plan HTTP {field} must not be empty."))
    })
}

fn optional_json_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_optional_text(Some(value)))
}

fn optional_json_string_value(value: Option<&str>) -> JsonValue {
    match normalize_optional_text(value) {
        Some(value) => JsonValue::String(value),
        None => JsonValue::Null,
    }
}

fn require_object<'a>(
    value: Option<&'a JsonValue>,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("{label} must be an object."))
}

fn change_payload_field<'a>(change: &'a JsonValue, field: &str) -> Option<&'a JsonValue> {
    change.get(field).or_else(|| {
        change
            .get("change")
            .and_then(JsonValue::as_object)
            .and_then(|object| object.get(field))
    })
}

fn change_payload_text(change: &JsonValue, field: &str) -> Option<String> {
    change_payload_field(change, field)
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_optional_text(Some(value)))
}

fn change_payload_object<'a>(
    change: &'a JsonValue,
    field: &str,
) -> Option<&'a JsonMap<String, JsonValue>> {
    change_payload_field(change, field).and_then(JsonValue::as_object)
}

#[cfg(test)]
mod tests;
