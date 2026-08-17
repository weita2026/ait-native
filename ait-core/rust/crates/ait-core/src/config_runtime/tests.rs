use super::*;
use crate::json_support::json;
use crate::shared_foundation::ConfigProvider;

fn assert_config_provider<T: ConfigProvider>() {}

struct SubstituteConfigProvider;

impl SubstituteConfigProvider {
    fn payload(operation: &str, payload_json: &str) -> JsonValue {
        json!({
            "provider": "substitute",
            "operation": operation,
            "payload_json": payload_json,
        })
    }
}

impl ConfigProvider for SubstituteConfigProvider {
    fn normalize_runtime_selection_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_runtime_selection_request_payload_json",
            payload_json,
        ))
    }

    fn build_runtime_selection_facts_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "build_runtime_selection_facts_json",
            payload_json,
        ))
    }

    fn normalize_runtime_selection_facts_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_runtime_selection_facts_payload_json",
            payload_json,
        ))
    }

    fn normalize_runtime_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_runtime_compatibility_payload_json",
            payload_json,
        ))
    }

    fn normalize_runtime_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_runtime_readiness_payload_json",
            payload_json,
        ))
    }

    fn normalize_runtime_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_runtime_doctor_payload_json",
            payload_json,
        ))
    }
}

#[test]
fn plan_runtime_config_foundation_implements_config_provider() {
    assert_config_provider::<RuntimeConfigFoundation>();
}

#[test]
fn config_runtime_bound_helpers_accept_substitute_provider() {
    let provider = SubstituteConfigProvider;
    let request = json!({ "sample": true }).to_string();

    let cases = [
        (
            "normalize_runtime_selection_request_payload_json",
            normalize_plan_runtime_selection_request_with_config_provider(&provider, &request)
                .expect("selection request"),
        ),
        (
            "build_runtime_selection_facts_json",
            build_plan_runtime_selection_facts_with_config_provider(&provider, &request)
                .expect("selection facts"),
        ),
        (
            "normalize_runtime_selection_facts_payload_json",
            normalize_plan_runtime_selection_facts_with_config_provider(&provider, &request)
                .expect("selection facts normalize"),
        ),
        (
            "normalize_runtime_compatibility_payload_json",
            normalize_plan_runtime_compatibility_with_config_provider(&provider, &request)
                .expect("compatibility"),
        ),
        (
            "normalize_runtime_readiness_payload_json",
            normalize_plan_runtime_readiness_with_config_provider(&provider, &request)
                .expect("readiness"),
        ),
        (
            "normalize_runtime_doctor_payload_json",
            normalize_plan_runtime_doctor_with_config_provider(&provider, &request)
                .expect("doctor"),
        ),
    ];

    for (operation, payload) in cases {
        assert_eq!(
            payload["provider"],
            JsonValue::String("substitute".to_string())
        );
        assert_eq!(
            payload["operation"],
            JsonValue::String(operation.to_string())
        );
        assert_eq!(payload["payload_json"], JsonValue::String(request.clone()));
    }
}

#[test]
fn plan_runtime_config_foundation_delegates_core_normalizer() {
    let foundation = RuntimeConfigFoundation;
    let request_json = r#"{"overrides":{}}"#;
    assert_eq!(
        foundation
            .normalize_runtime_selection_request_payload_json(request_json)
            .unwrap(),
        normalize_plan_runtime_selection_request_payload_json(request_json).unwrap()
    );
    assert_eq!(
        foundation
            .build_runtime_selection_facts_json(request_json)
            .unwrap(),
        build_plan_runtime_selection_facts_json(request_json).unwrap()
    );
}

#[test]
fn selection_facts_use_typed_overrides_or_the_fixed_rust_default() {
    let request_json = json!({
        "overrides": {
            "plan_http_backend": "rust"
        }
    })
    .to_string();

    let facts = build_plan_runtime_selection_facts_json(&request_json).expect("selection facts");

    assert_eq!(facts["plan_core_backend"]["value"], "rust");
    assert_eq!(facts["plan_core_backend"]["source"], "default");
    assert_eq!(facts["plan_http_backend"]["value"], "rust");
    assert_eq!(facts["plan_http_backend"]["source"], "explicit");
    assert_eq!(facts["plan_filesystem_backend"]["value"], "rust");
    assert_eq!(facts["plan_filesystem_backend"]["source"], "default");
}

#[test]
fn backend_gate_is_fixed_to_rust_without_process_environment() {
    assert_eq!(
        resolve_runtime_backend_selection("rust", "plan core backend activation")
            .expect("Rust authority"),
        "rust"
    );
    assert!(resolve_runtime_backend_selection("python", "plan core backend activation").is_err());
    assert!(resolve_runtime_backend_selection("other", "plan core backend activation").is_err());
}
