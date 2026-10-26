use crate::json_support::JsonValue;

pub trait PlanProvenanceCodec {
    fn normalize_revision_provenance_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
    fn build_revision_provenance_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String>;
}
