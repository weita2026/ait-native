use crate::json_support::{json, JsonMap, JsonValue};
use crate::plan_http_client::{
    build_plan_http_request_spec, configured_repository_authority_path_segment,
    encode_path_segment, PlanHttpClientConfig, PlanHttpClientError, PlanHttpClientResult,
    PlanHttpRequestSpec,
};
use crate::policy;
use crate::text_normalization::normalize_optional_text;
use reqwest::Method;
use serde::{Deserialize, Serialize};

/// The existing remote HTTP statement. This is not local database authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteVerificationState {
    #[default]
    Unknown,
    Pending,
    Pass,
    Fail,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteAttestationStatement {
    pub verification_state: RemoteVerificationState,
    pub require_tests_pass: bool,
    pub require_human_review: bool,
    pub require_lint_pass: bool,
    pub revoked: bool,
}

impl Default for RemoteAttestationStatement {
    fn default() -> Self {
        Self {
            verification_state: RemoteVerificationState::Unknown,
            require_tests_pass: true,
            require_human_review: false,
            require_lint_pass: false,
            revoked: false,
        }
    }
}

impl RemoteAttestationStatement {
    pub fn manual_verification(value: Option<&str>) -> Result<RemoteVerificationState, String> {
        match value.map(str::trim) {
            None => Ok(RemoteVerificationState::Unknown),
            Some("unknown") => Ok(RemoteVerificationState::Unknown),
            Some("pending") => Ok(RemoteVerificationState::Pending),
            Some("pass") => Ok(RemoteVerificationState::Pass),
            Some("fail") => Ok(RemoteVerificationState::Fail),
            Some(other) => Err(format!("Remote manual --tests verification cannot represent {other:?}; use unknown, pending, pass or fail. Manual verification never supplies CI results or disables required CI.")),
        }
    }

    pub fn validate_manual_inputs(
        tests: Option<&str>,
        lint: Option<&str>,
        security: Option<&str>,
        license: Option<&str>,
        explicit_model: Option<&str>,
    ) -> Result<(), String> {
        Self::manual_verification(tests)?;
        for (name, value) in [
            ("lint", lint),
            ("security", security),
            ("license", license),
            ("model", explicit_model),
        ] {
            if value.is_some() {
                return Err(format!("Explicit --{name} has no representation in the remote compact Attestation contract; no publication or mutation was performed."));
            }
        }
        Ok(())
    }

    pub fn from_readback(value: &JsonValue) -> Result<Self, String> {
        let statement: Self = serde_json::from_value(value.clone()).map_err(|error| {
            format!("Incompatible remote compact Attestation statement: {error}")
        })?;
        if value["ci_backed"].as_bool().is_none() {
            return Err("Remote compact Attestation requires derived boolean ci_backed".into());
        }
        Ok(statement)
    }

    pub fn validate_for_patchset(value: &JsonValue, patchset: &JsonValue) -> Result<Self, String> {
        let task = patchset["task_id"]
            .as_str()
            .ok_or("Patchset requires Task ownership")?;
        let expected_change =
            crate::workflow_expected_identity::validate_patchset_owner_fields(patchset, task)?;
        let actual_change =
            crate::workflow_expected_identity::validate_patchset_owner_fields(value, task)?;
        let author_mode = patchset["author_mode"]
            .as_str()
            .filter(|mode| !mode.trim().is_empty())
            .ok_or("Patchset requires immutable author_mode")?;
        if actual_change != expected_change || value["author_mode"].as_str() != Some(author_mode) {
            return Err(
                "Remote compact Attestation owner or author_mode differs from the exact Patchset"
                    .into(),
            );
        }
        crate::workflow_expected_identity::WorkflowExpectedIdentity {
            repository_index: 0,
            patchset_id: patchset["patchset_id"]
                .as_str()
                .ok_or("Patchset identity missing")?
                .into(),
            base_snapshot_id: patchset["base_snapshot_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
            revision_snapshot_id: patchset["revision_snapshot_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
        }
        .validate_patchset_reference(value)?;
        Self::from_readback(value)
    }

    pub fn retain_requirements_from(&mut self, previous: &Self) {
        self.require_tests_pass |= previous.require_tests_pass;
        self.require_human_review |= previous.require_human_review;
        self.require_lint_pass |= previous.require_lint_pass;
        self.revoked |= previous.revoked;
    }

    /// Only completed CI embedded in this exact Patchset can justify automatic verification.
    pub fn required_ci_passed(&self, patchset: &JsonValue) -> bool {
        let ci = &patchset["ci"];
        let run = patchset["ci_run_seq"]
            .as_u64()
            .or_else(|| ci["run_seq"].as_u64())
            .unwrap_or_default();
        let completed = patchset["ci_completed_at_s"]
            .as_u64()
            .or_else(|| ci["completed_at_s"].as_u64())
            .unwrap_or_default();
        let evidence = ci["suite_result_count"].as_u64().unwrap_or_default() > 0;
        let required = self.require_tests_pass || self.require_lint_pass;
        required
            && run > 0
            && completed > 0
            && evidence
            && ci["blocking_failure_count"].as_u64().unwrap_or_default() == 0
            && (!self.require_tests_pass || ci["tests_status"] == "pass")
            && (!self.require_lint_pass || ci["lint_status"] == "pass")
    }
}

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
        statement: &crate::attest_json::RemoteAttestationStatement,
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
            Some(self.build_put_attestation_body(author_mode, statement)?),
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
        statement: &crate::attest_json::RemoteAttestationStatement,
    ) -> PlanHttpClientResult<JsonValue> {
        let _ = &self.store;
        let author_mode = require_plan_http_non_empty_text(author_mode, "author_mode")?;
        let mut body = serde_json::to_value(statement)
            .map_err(|error| PlanHttpClientError::Invalid(error.to_string()))?;
        body["author_mode"] = json!(author_mode);
        Ok(body)
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

    pub fn normalize_attestation_payload(&self, payload: &JsonValue) -> Result<JsonValue, String> {
        let _ = &self.store;
        Ok(JsonValue::Object(
            require_object(Some(payload), "attestation payload")?.clone(),
        ))
    }

    pub fn optional_patchset_id(&self, attestation: &JsonValue) -> Option<String> {
        let _ = &self.store;
        normalize_optional_text(attestation.get("patchset_id").and_then(JsonValue::as_str))
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
