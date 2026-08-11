use crate::json_support::{json, JsonCodec, JsonMap, JsonValue};
use crate::plan_http_client::{
    build_plan_http_request_spec, configured_repository_authority_path_segment,
    encode_path_segment, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult,
    PlanHttpRequestSpec,
};
use crate::policy;
use crate::text_normalization::normalize_optional_text;
use reqwest::Method;

pub struct PolicyJson<S> {
    store: S,
}

impl<S> PolicyJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl PolicyJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> PolicyJson<S> {
    pub fn build_evaluate_policy_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        patchset_id: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let patchset_id = encode_path_segment(&require_plan_http_non_empty_text(
            patchset_id,
            "patchset_id",
        )?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/patchsets/{patchset_id}:evaluatePolicy"
        );
        build_plan_http_request_spec(
            config,
            Method::POST,
            &path,
            Vec::new(),
            Some(self.build_evaluate_policy_body()),
        )
    }

    pub fn build_get_policy_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        patchset_id: &str,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let patchset_id = encode_path_segment(&require_plan_http_non_empty_text(
            patchset_id,
            "patchset_id",
        )?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/patchsets/{patchset_id}/policy"
        );
        build_plan_http_request_spec(config, Method::GET, &path, Vec::new(), None)
    }

    pub fn build_create_waiver_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let patchset_id = encode_path_segment(&require_plan_http_non_empty_text(
            patchset_id,
            "patchset_id",
        )?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        build_plan_http_request_spec(
            config,
            Method::POST,
            &format!(
                "/v1/native/repository-authorities/{repository_index}/patchsets/{patchset_id}/waivers"
            ),
            Vec::new(),
            Some(self.build_create_waiver_body(rule_name, reason, expires_at)?),
        )
    }

    pub fn build_evaluate_policy_body(&self) -> JsonValue {
        let _ = &self.store;
        JsonValue::Object(JsonMap::new())
    }

    pub fn build_create_waiver_body(
        &self,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
    ) -> PlanHttpClientResult<JsonValue> {
        let _ = &self.store;
        let rule_name = require_plan_http_non_empty_text(rule_name, "rule_name")?;
        let reason = require_plan_http_non_empty_text(reason, "reason")?;
        Ok(json!({
            "rule_name": rule_name,
            "reason": reason,
            "expires_at": optional_json_string(expires_at),
        }))
    }

    pub fn policy_profile(&self, name: &str) -> Result<JsonValue, String> {
        let _ = &self.store;
        policy::policy_profile(name)
    }

    pub fn normalize_policy(
        &self,
        policy: Option<&JsonValue>,
        fallback_profile: &str,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        policy::normalize_policy(policy, fallback_profile)
    }

    pub fn resolve_effective_policy(
        &self,
        policy: Option<&JsonValue>,
        content_class: Option<&str>,
        author_class: Option<&str>,
        fallback_profile: &str,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        policy::resolve_effective_policy(policy, content_class, author_class, fallback_profile)
    }

    pub fn policy_to_yaml(
        &self,
        policy: Option<&JsonValue>,
        fallback_profile: &str,
    ) -> Result<String, String> {
        let _ = &self.store;
        policy::policy_to_yaml(policy, fallback_profile)
    }

    pub fn parse_policy_yaml(
        &self,
        text: &str,
        fallback_profile: &str,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        policy::parse_policy_yaml(text, fallback_profile)
    }

    pub fn policy_profile_names(&self) -> Vec<String> {
        let _ = &self.store;
        policy::policy_profile_names()
    }

    pub fn author_mode_values(&self) -> Vec<String> {
        let _ = &self.store;
        policy::author_mode_values()
    }

    pub fn policy_content_class_values(&self) -> Vec<String> {
        let _ = &self.store;
        policy::policy_content_class_values()
    }

    pub fn policy_author_class_values(&self) -> Vec<String> {
        let _ = &self.store;
        policy::policy_author_class_values()
    }

    pub fn normalize_author_mode(&self, value: &str) -> Result<String, String> {
        let _ = &self.store;
        policy::normalize_author_mode(value)
    }

    pub fn derive_policy_content_class(&self, changed_paths: Option<&JsonValue>) -> String {
        let _ = &self.store;
        policy::derive_policy_content_class(changed_paths)
    }

    pub fn derive_policy_author_class(&self, author_mode: Option<&str>) -> Option<String> {
        let _ = &self.store;
        policy::derive_policy_author_class(author_mode)
    }

    pub fn missing_code_review_summary_sections(&self, value: Option<&str>) -> Vec<String> {
        let _ = &self.store;
        policy::missing_code_review_summary_sections(value)
    }

    pub fn is_structured_code_review_summary(&self, value: Option<&str>) -> bool {
        let _ = &self.store;
        policy::is_structured_code_review_summary(value)
    }

    pub fn render_code_review_summary_template(
        &self,
        style: Option<&str>,
    ) -> Result<&'static str, String> {
        let _ = &self.store;
        policy::render_code_review_summary_template(style)
    }

    pub fn code_review_summary_requirement_text(&self, value: Option<&str>) -> String {
        let _ = &self.store;
        policy::code_review_summary_requirement_text(value)
    }

    pub fn normalize_policy_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "policy payload")?;
        self.normalize_policy_payload(&JsonValue::Object(payload))
    }

    pub fn normalize_policy_payload(&self, payload: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "policy payload")?.clone(),
        ))
    }

    pub fn normalize_policy_eval_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "policy eval payload")?;
        self.normalize_policy_eval_payload(&JsonValue::Object(payload))
    }

    pub fn normalize_policy_eval_payload(&self, payload: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "policy eval payload")?.clone(),
        ))
    }

    pub fn normalize_waiver_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "policy waiver payload")?;
        self.normalize_waiver_payload(&JsonValue::Object(payload))
    }

    pub fn normalize_waiver_payload(&self, payload: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "policy waiver payload")?.clone(),
        ))
    }

    pub fn optional_policy_id(&self, policy: &JsonValue) -> Option<String> {
        let _ = &self.store;
        normalize_optional_text(policy.get("policy_id").and_then(JsonValue::as_str))
    }

    pub fn optional_decision(&self, policy: &JsonValue) -> Option<String> {
        let _ = &self.store;
        normalize_optional_text(policy.get("decision").and_then(JsonValue::as_str))
    }

    pub fn policy_checks(&self, policy: Option<&JsonMap<String, JsonValue>>) -> Vec<JsonValue> {
        let _ = &self.store;
        policy
            .and_then(|value| value.get("checks"))
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default()
    }

    pub fn policy_has_checks(&self, policy: Option<&JsonMap<String, JsonValue>>) -> bool {
        let _ = &self.store;
        self.policy_checks(policy).iter().any(JsonValue::is_object)
    }

    pub fn blocking_policy_check_labels(
        &self,
        policy: Option<&JsonMap<String, JsonValue>>,
    ) -> Vec<String> {
        let _ = &self.store;
        self.policy_checks(policy)
            .iter()
            .filter_map(blocking_policy_check_label)
            .collect()
    }

    pub fn policy_check_statuses(
        &self,
        policy: Option<&JsonMap<String, JsonValue>>,
    ) -> Vec<String> {
        let _ = &self.store;
        self.policy_checks(policy)
            .iter()
            .filter_map(|check| {
                check
                    .as_object()
                    .and_then(|obj| obj.get("status"))
                    .and_then(JsonValue::as_str)
                    .and_then(|value| normalize_optional_text(Some(value)))
            })
            .collect()
    }

    pub fn policy_decision_or(
        &self,
        policy: Option<&JsonMap<String, JsonValue>>,
        fallback_decision: Option<&str>,
    ) -> String {
        let _ = &self.store;
        policy
            .and_then(|value| value.get("decision"))
            .and_then(JsonValue::as_str)
            .or(fallback_decision)
            .unwrap_or("pending")
            .trim()
            .to_string()
    }

    pub fn workflow_land_policy_has_checks(
        &self,
        policy: Option<&JsonMap<String, JsonValue>>,
    ) -> bool {
        self.policy_has_checks(policy)
    }

    pub fn workflow_land_policy_blocker_detail(
        &self,
        policy: Option<&JsonMap<String, JsonValue>>,
        landing_submission_id: Option<&str>,
        fallback_decision: Option<&str>,
    ) -> String {
        let _ = &self.store;
        let blocker_labels = self.blocking_policy_check_labels(policy);
        if !blocker_labels.is_empty() {
            let mut summary = blocker_labels
                .into_iter()
                .take(3)
                .collect::<Vec<_>>()
                .join(", ");
            if summary.split(", ").count() == 3 {
                summary.push_str(", ...");
            }
            return if let Some(submission_id) = landing_submission_id {
                format!(
                    "Remote land submission `{submission_id}` is blocked by policy requirements: {summary}."
                )
            } else {
                format!("Land preflight is blocked by policy requirements: {summary}.")
            };
        }
        let decision = self.policy_decision_or(policy, fallback_decision);
        if let Some(submission_id) = landing_submission_id {
            format!(
                "Remote land submission `{submission_id}` is blocked because policy is currently `{decision}`."
            )
        } else {
            format!("Land preflight is currently blocked because policy is `{decision}`.")
        }
    }

    pub fn build_policy_action_result(&self, result: JsonValue) -> JsonValue {
        let _ = &self.store;
        json!({ "result": result })
    }

    pub fn policy_refresh_from_review_result(&self, result: &JsonValue) -> Option<JsonValue> {
        let _ = &self.store;
        result
            .as_object()
            .and_then(|result| result.get("policy_refresh"))
            .cloned()
    }

    pub fn policy_refresh_recovery(&self, policy_refresh: Option<&JsonValue>) -> Option<JsonValue> {
        let _ = &self.store;
        policy_refresh
            .and_then(JsonValue::as_object)
            .and_then(|policy_refresh| policy_refresh.get("response_recovery"))
            .and_then(JsonValue::as_object)
            .cloned()
            .map(JsonValue::Object)
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

fn blocking_policy_check_label(check: &JsonValue) -> Option<String> {
    let obj = check.as_object()?;
    let status = obj
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim()
        .to_lowercase();
    if !matches!(
        status.as_str(),
        "pending" | "hard_fail" | "soft_fail" | "waived"
    ) {
        return None;
    }
    obj.get("label")
        .or_else(|| obj.get("name"))
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_optional_text(Some(value)))
}

fn require_plan_http_non_empty_text(value: &str, field: &str) -> PlanHttpClientResult<String> {
    normalize_optional_text(Some(value)).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!("Plan HTTP {field} must not be empty."))
    })
}

fn optional_json_string(value: Option<&str>) -> JsonValue {
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
