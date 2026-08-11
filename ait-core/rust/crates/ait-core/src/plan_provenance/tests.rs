use super::*;
use crate::json_support::json;

fn assert_plan_provenance_codec<T: PlanProvenanceCodec>() {}

struct SubstitutePlanProvenanceCodec;

impl PlanProvenanceCodec for SubstitutePlanProvenanceCodec {
    fn normalize_revision_provenance_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "codec": "substitute",
            "operation": "normalize",
            "payload_json": payload_json,
        }))
    }

    fn build_revision_provenance_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "codec": "substitute",
            "operation": "build",
            "payload_json": payload_json,
        }))
    }
}

#[test]
fn plan_provenance_foundation_implements_neutral_trait() {
    assert_plan_provenance_codec::<PlanProvenanceFoundation>();
}

#[test]
fn revision_provenance_helpers_accept_substitute_codec() {
    let codec = SubstitutePlanProvenanceCodec;
    let request = json!({ "sample": true }).to_string();

    let normalized =
        normalize_plan_revision_provenance_with_plan_provenance_codec(&codec, &request).unwrap();
    let built =
        build_plan_revision_provenance_with_plan_provenance_codec(&codec, &request).unwrap();

    assert_eq!(normalized["operation"], "normalize");
    assert_eq!(built["operation"], "build");
    assert_eq!(normalized["payload_json"], request);
}

#[test]
fn revision_provenance_requires_both_identities() {
    assert_eq!(
        normalize_plan_revision_provenance_payload_json(
            &json!({ "plan_revision_id": "REV-1" }).to_string()
        )
        .unwrap_err(),
        "Plan provenance payload must include plan_id."
    );
    assert_eq!(
        normalize_plan_revision_provenance_payload_json(&json!({ "plan_id": "PR-1" }).to_string())
            .unwrap_err(),
        "Plan provenance payload must include plan_revision_id."
    );
}
