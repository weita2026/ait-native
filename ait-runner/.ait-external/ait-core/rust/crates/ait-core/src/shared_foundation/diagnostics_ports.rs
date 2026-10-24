use crate::json_support::JsonValue;

pub trait DiagnosticsProbe {
    fn normalize_diagnostics_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_backend_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_diagnostics_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_diagnostics_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_diagnostics_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
}
