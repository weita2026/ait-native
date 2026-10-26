use crate::json_support::JsonValue;

pub trait ConfigProvider {
    fn normalize_runtime_selection_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn build_runtime_selection_facts_json(&self, payload_json: &str) -> Result<JsonValue, String>;
    fn normalize_runtime_selection_facts_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_runtime_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_runtime_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_runtime_doctor_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
}
