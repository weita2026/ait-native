use super::*;
use crate::json_support::{json, JsonValue};
use crate::shared_foundation::DiagnosticsProbe;

struct SubstituteDiagnosticsProbe;

impl SubstituteDiagnosticsProbe {
    fn payload(operation: &str, payload_json: &str) -> JsonValue {
        json!({
            "probe": "substitute",
            "operation": operation,
            "payload_json": payload_json,
        })
    }
}

impl DiagnosticsProbe for SubstituteDiagnosticsProbe {
    fn normalize_diagnostics_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_diagnostics_request_payload_json",
            payload_json,
        ))
    }

    fn normalize_backend_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_backend_identity_payload_json",
            payload_json,
        ))
    }

    fn normalize_diagnostics_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_diagnostics_compatibility_payload_json",
            payload_json,
        ))
    }

    fn normalize_diagnostics_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_diagnostics_readiness_payload_json",
            payload_json,
        ))
    }

    fn normalize_diagnostics_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(Self::payload(
            "normalize_diagnostics_doctor_payload_json",
            payload_json,
        ))
    }
}

#[test]
fn diagnostics_bound_helpers_accept_substitute_probe() {
    let probe = SubstituteDiagnosticsProbe;
    let request = json!({ "sample": true }).to_string();

    let cases = [
        (
            "normalize_diagnostics_request_payload_json",
            normalize_plan_diagnostics_request_with_diagnostics_probe(&probe, &request)
                .expect("diagnostics request"),
        ),
        (
            "normalize_backend_identity_payload_json",
            normalize_plan_backend_identity_with_diagnostics_probe(&probe, &request)
                .expect("backend identity"),
        ),
        (
            "normalize_diagnostics_compatibility_payload_json",
            normalize_plan_diagnostics_compatibility_with_diagnostics_probe(&probe, &request)
                .expect("diagnostics compatibility"),
        ),
        (
            "normalize_diagnostics_readiness_payload_json",
            normalize_plan_diagnostics_readiness_with_diagnostics_probe(&probe, &request)
                .expect("diagnostics readiness"),
        ),
        (
            "normalize_diagnostics_doctor_payload_json",
            normalize_plan_diagnostics_doctor_with_diagnostics_probe(&probe, &request)
                .expect("diagnostics doctor"),
        ),
    ];

    for (operation, payload) in cases {
        assert_eq!(
            payload["probe"],
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
fn plan_diagnostics_foundation_delegates_backend_identity_normalizer() {
    let foundation = DiagnosticsFoundation;
    let payload = r#"{"selected_backend":"python","selected_backend_ready":false,"rust_authority_ready":false,"compatibility":"plan","extension_loaded":true,"extension_module":null,"extension_path":null,"extension_task_contract_version":null,"extension_plan_contract_version":null,"expected_plan_contract_version":"1.0.0","extension_package_version":null,"package_version":null,"required_exports":[],"surface_commands":["ait plan list"],"issues":[],"env":{},"exports":{},"missing_exports":[]}"#;
    assert_eq!(
        foundation
            .normalize_backend_identity_payload_json(payload)
            .unwrap(),
        normalize_plan_backend_identity_payload_json(payload).unwrap()
    );
}

#[test]
fn diagnostics_request_rejects_retired_overrides_and_package_inputs() {
    assert_eq!(
        normalize_plan_diagnostics_request_payload_json("{}").unwrap(),
        json!({})
    );

    for payload in [
        r#"{"overrides":{}}"#,
        r#"{"wheel_path":"/tmp/example.whl"}"#,
        r#"{"repack_installed":true}"#,
        r#"{"smoke":true}"#,
    ] {
        let error = normalize_plan_diagnostics_request_payload_json(payload).unwrap_err();
        assert!(error.contains("accept no overrides"), "{error}");
    }
}

#[test]
fn doctor_and_compatibility_facts_are_package_format_neutral() {
    let doctor = build_plan_diagnostics_doctor_facts_json("{}").unwrap();
    assert!(doctor.get("wheel_status").is_none());
    assert!(doctor["compatibility"].get("wheel_status").is_none());

    let compatibility = build_plan_diagnostics_compatibility_status_json("{}").unwrap();
    assert!(compatibility.get("wheel_status").is_none());
}
