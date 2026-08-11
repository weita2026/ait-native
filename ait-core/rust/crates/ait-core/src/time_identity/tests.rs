use super::*;
use crate::json_support::json;

fn assert_time_identity_provider<T: TimeIdentityProvider>() {}

struct SubstituteTimeIdentityProvider;

impl SubstituteTimeIdentityProvider {
    fn payload(operation: &str, payload_json: &str) -> JsonValue {
        json!({
            "provider": "substitute",
            "operation": operation,
            "payload_json": payload_json,
        })
    }
}

impl TimeIdentityProvider for SubstituteTimeIdentityProvider {
    fn normalize_timestamp_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_timestamp_request_payload_json",
            payload_json,
        ))
    }

    fn normalize_timestamp_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_timestamp_payload_json",
            payload_json,
        ))
    }

    fn build_timestamp_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        Ok(Self::payload("build_timestamp_payload_json", payload_json))
    }

    fn normalize_sequence_identity_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_sequence_identity_request_payload_json",
            payload_json,
        ))
    }

    fn normalize_sequence_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_sequence_identity_payload_json",
            payload_json,
        ))
    }

    fn build_sequence_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "build_sequence_identity_payload_json",
            payload_json,
        ))
    }

    fn normalize_workflow_id_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_workflow_id_request_payload_json",
            payload_json,
        ))
    }

    fn normalize_workflow_id_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_workflow_id_payload_json",
            payload_json,
        ))
    }

    fn build_workflow_id_payload_json(&self, payload_json: &str) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "build_workflow_id_payload_json",
            payload_json,
        ))
    }
}

#[test]
fn plan_time_identity_foundation_implements_trait() {
    assert_time_identity_provider::<TimeIdentityFoundation>();
}

#[test]
fn time_identity_bound_helpers_accept_substitute_provider() {
    let provider = SubstituteTimeIdentityProvider;
    let request = json!({ "sample": true }).to_string();

    let cases = [
        (
            "normalize_timestamp_request_payload_json",
            normalize_plan_timestamp_request_with_time_identity_provider(&provider, &request)
                .expect("timestamp request"),
        ),
        (
            "normalize_timestamp_payload_json",
            normalize_plan_timestamp_with_time_identity_provider(&provider, &request)
                .expect("timestamp payload"),
        ),
        (
            "build_timestamp_payload_json",
            build_plan_timestamp_with_time_identity_provider(&provider, &request)
                .expect("timestamp build"),
        ),
        (
            "normalize_sequence_identity_request_payload_json",
            normalize_plan_sequence_identity_request_with_time_identity_provider(
                &provider, &request,
            )
            .expect("sequence request"),
        ),
        (
            "normalize_sequence_identity_payload_json",
            normalize_plan_sequence_identity_with_time_identity_provider(&provider, &request)
                .expect("sequence payload"),
        ),
        (
            "build_sequence_identity_payload_json",
            build_plan_sequence_identity_with_time_identity_provider(&provider, &request)
                .expect("sequence build"),
        ),
        (
            "normalize_workflow_id_request_payload_json",
            normalize_plan_workflow_id_request_with_time_identity_provider(&provider, &request)
                .expect("workflow id request"),
        ),
        (
            "normalize_workflow_id_payload_json",
            normalize_plan_workflow_id_with_time_identity_provider(&provider, &request)
                .expect("workflow id payload"),
        ),
        (
            "build_workflow_id_payload_json",
            build_plan_workflow_id_with_time_identity_provider(&provider, &request)
                .expect("workflow id build"),
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
fn plan_time_identity_foundation_build_timestamp_marks_default_source() {
    let foundation = TimeIdentityFoundation;
    let payload = foundation
        .build_timestamp_payload_json(&json!({}).to_string())
        .expect("timestamp payload");
    assert_eq!(payload["source"], JsonValue::String("system".to_string()));
}

#[test]
fn workflow_id_generation_preserves_empty_namespace_prefix() {
    let payload = build_plan_workflow_id_payload_json(
        &json!({
            "family": "PL",
            "namespace_prefix": "",
            "timestamp_ms": 1_706_000_000_000i64,
            "randomness_hex": "00000000000000000001"
        })
        .to_string(),
    )
    .expect("workflow id payload");

    assert_eq!(
        payload["namespace_prefix"],
        JsonValue::String(String::new())
    );
    assert!(payload["generated_id"]
        .as_str()
        .expect("generated id")
        .starts_with("PL-"));
    assert!(!payload["generated_id"]
        .as_str()
        .expect("generated id")
        .starts_with("AITPL-"));
}

#[test]
fn sequence_id_generation_preserves_empty_namespace_prefix() {
    let payload = build_plan_sequence_identity_payload_json(
        &json!({
            "family": "T",
            "number": 7,
            "namespace_prefix": "",
            "width": 4
        })
        .to_string(),
    )
    .expect("sequence id payload");

    assert_eq!(
        payload["namespace_prefix"],
        JsonValue::String(String::new())
    );
    assert_eq!(
        payload["generated_id"],
        JsonValue::String("T-0007".to_string())
    );
}
