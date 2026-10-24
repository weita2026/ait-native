use crate::json_support::JsonValue;

pub trait TimeIdentityProvider {
    fn normalize_timestamp_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_timestamp_payload_json(&self, payload_json: &str) -> Result<JsonValue, String>;
    fn build_timestamp_payload_json(&self, payload_json: &str) -> Result<JsonValue, String>;
    fn normalize_sequence_identity_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_sequence_identity_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn build_sequence_identity_payload_json(&self, payload_json: &str)
        -> Result<JsonValue, String>;
    fn normalize_workflow_id_request_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn normalize_workflow_id_payload_json(&self, payload_json: &str) -> Result<JsonValue, String>;
    fn build_workflow_id_payload_json(&self, payload_json: &str) -> Result<JsonValue, String>;
}
