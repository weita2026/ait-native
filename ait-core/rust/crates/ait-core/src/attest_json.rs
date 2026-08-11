use crate::json_support::{json, JsonCodec, JsonMap, JsonValue};
use crate::plan_http_client::{
    build_plan_http_request_spec, configured_repository_authority_path_segment,
    encode_path_segment, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult,
    PlanHttpRequestSpec,
};
use crate::policy;
use crate::text_normalization::normalize_optional_text;
use reqwest::Method;

pub struct AttestJson<S> {
    store: S,
}

impl<S> AttestJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl AttestJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

impl<S> AttestJson<S> {
    pub fn build_put_attestation_request_spec(
        &self,
        config: &PlanHttpClientConfig,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &JsonValue,
        provenance_summary: &JsonValue,
        detail: &JsonValue,
    ) -> PlanHttpClientResult<PlanHttpRequestSpec> {
        let _ = &self.store;
        let patchset_id = encode_path_segment(&require_plan_http_non_empty_text(
            patchset_id,
            "patchset_id",
        )?);
        let repository_index = configured_repository_authority_path_segment(config)?;
        let path = format!(
            "/v1/native/repository-authorities/{repository_index}/patchsets/{patchset_id}/attestation"
        );
        build_plan_http_request_spec(
            config,
            Method::PUT,
            &path,
            Vec::new(),
            Some(self.build_put_attestation_body(
                author_mode,
                evaluation_summary,
                provenance_summary,
                detail,
            )?),
        )
    }

    pub fn build_get_attestation_request_spec(
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
            "/v1/native/repository-authorities/{repository_index}/patchsets/{patchset_id}/attestation"
        );
        build_plan_http_request_spec(config, Method::GET, &path, Vec::new(), None)
    }

    pub fn build_put_attestation_body(
        &self,
        author_mode: &str,
        evaluation_summary: &JsonValue,
        provenance_summary: &JsonValue,
        detail: &JsonValue,
    ) -> PlanHttpClientResult<JsonValue> {
        let _ = &self.store;
        let author_mode = require_plan_http_non_empty_text(author_mode, "author_mode")?;
        Ok(json!({
            "author_mode": author_mode,
            "evaluation_summary": evaluation_summary,
            "provenance_summary": provenance_summary,
            "detail": detail,
        }))
    }

    pub fn build_evaluation_summary(
        &self,
        tests: Option<&str>,
        lint: Option<&str>,
        security: Option<&str>,
        license: Option<&str>,
    ) -> JsonValue {
        let _ = &self.store;
        let mut evaluation = JsonMap::new();
        insert_optional_string(&mut evaluation, "tests", tests);
        insert_optional_string(&mut evaluation, "lint", lint);
        insert_optional_string(&mut evaluation, "security_scan", security);
        insert_optional_string(&mut evaluation, "license_scan", license);
        JsonValue::Object(evaluation)
    }

    pub fn build_minimum_provenance(
        &self,
        author_mode: &str,
        model_name: Option<&str>,
    ) -> Result<(JsonValue, JsonValue), String> {
        let _ = &self.store;
        policy::build_minimum_provenance(author_mode, model_name)
    }

    pub fn normalize_attestation_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        let payload = self.parse_object_payload(payload_json, "attestation payload")?;
        self.normalize_attestation_payload(&JsonValue::Object(payload))
    }

    pub fn normalize_attestation_payload(&self, payload: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "attestation payload")?.clone(),
        ))
    }

    pub fn normalize_evaluation_summary_payload(
        &self,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "evaluation summary payload")?.clone(),
        ))
    }

    pub fn normalize_provenance_summary_payload(
        &self,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "provenance summary payload")?.clone(),
        ))
    }

    pub fn normalize_detail_payload(&self, payload: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "attestation detail payload")?.clone(),
        ))
    }

    pub fn optional_attestation_id(&self, attestation: &JsonValue) -> Option<String> {
        let _ = &self.store;
        normalize_optional_text(
            attestation
                .get("attestation_id")
                .and_then(JsonValue::as_str),
        )
    }

    pub fn optional_patchset_id(&self, attestation: &JsonValue) -> Option<String> {
        let _ = &self.store;
        normalize_optional_text(attestation.get("patchset_id").and_then(JsonValue::as_str))
    }

    pub fn optional_author_mode(&self, attestation: &JsonValue) -> Option<String> {
        let _ = &self.store;
        normalize_optional_text(attestation.get("author_mode").and_then(JsonValue::as_str))
    }

    pub fn tests_state_from_attestation(&self, attestation: Option<&JsonValue>) -> Option<String> {
        let _ = &self.store;
        attestation
            .and_then(JsonValue::as_object)
            .and_then(|attestation| attestation.get("evaluation_summary"))
            .and_then(JsonValue::as_object)
            .and_then(|summary| summary.get("tests"))
            .and_then(JsonValue::as_str)
            .and_then(|value| normalize_optional_text(Some(value)))
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

fn insert_optional_string(body: &mut JsonMap<String, JsonValue>, key: &str, value: Option<&str>) {
    if let Some(value) = normalize_optional_text(value) {
        body.insert(key.to_string(), JsonValue::String(value));
    }
}

fn require_plan_http_non_empty_text(value: &str, label: &str) -> PlanHttpClientResult<String> {
    normalize_optional_text(Some(value)).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!("Plan HTTP `{label}` must be non-empty."))
    })
}

fn require_object<'a>(
    value: Option<&'a JsonValue>,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("{label} must be an object."))
}
