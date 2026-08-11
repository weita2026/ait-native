use crate::json_support::{JsonMap, JsonValue};
use crate::plan_workflow_json::PlanWorkflowJson;
use crate::shared_foundation::PlanProvenanceCodec;

#[derive(Default)]
pub struct PlanProvenanceFoundation;

impl PlanProvenanceCodec for PlanProvenanceFoundation {
    fn normalize_revision_provenance_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        normalize_plan_revision_provenance_payload_json(payload_json)
    }

    fn build_revision_provenance_payload_json(
        &self,
        payload_json: &str,
    ) -> Result<JsonValue, String> {
        build_plan_revision_provenance_payload_json(payload_json)
    }
}

pub fn normalize_plan_revision_provenance_with_plan_provenance_codec<C>(
    codec: &C,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    C: PlanProvenanceCodec + ?Sized,
{
    codec.normalize_revision_provenance_payload_json(payload_json)
}

pub fn build_plan_revision_provenance_with_plan_provenance_codec<C>(
    codec: &C,
    payload_json: &str,
) -> Result<JsonValue, String>
where
    C: PlanProvenanceCodec + ?Sized,
{
    codec.build_revision_provenance_payload_json(payload_json)
}

pub fn normalize_plan_revision_provenance_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_revision_provenance_payload_json(payload_json)
}

pub(crate) fn normalize_plan_revision_provenance_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    Ok(JsonValue::Object(normalize_revision_provenance_map(
        &payload,
    )?))
}

pub fn build_plan_revision_provenance_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_revision_provenance_payload_json(payload_json)
}

pub(crate) fn build_plan_revision_provenance_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let plan = require_object(payload.get("plan"), "plan")?;
    let revision = require_object(payload.get("revision"), "revision")?;
    let plan_id = optional_text(revision.get("plan_id"))?
        .or_else(|| optional_text(plan.get("plan_id")).ok().flatten())
        .ok_or_else(|| "Plan revision provenance payload requires plan_id.".to_string())?;
    let plan_revision_id =
        require_nonempty_text(revision.get("plan_revision_id"), "plan_revision_id")?;
    normalize_plan_revision_provenance_payload_map(JsonMap::from_iter([
        ("plan_id".to_string(), JsonValue::String(plan_id)),
        (
            "plan_revision_id".to_string(),
            JsonValue::String(plan_revision_id),
        ),
        maybe_json_entry("source_kind", optional_text(revision.get("source_kind"))?),
        maybe_json_entry("created_by", optional_text(revision.get("created_by"))?),
        maybe_json_entry("actor_type", optional_text(revision.get("actor_type"))?),
        maybe_json_entry(
            "publication_state",
            optional_text(revision.get("publication_state"))?
                .or_else(|| optional_text(plan.get("publication_state")).ok().flatten()),
        ),
        maybe_json_entry(
            "published_plan_id",
            optional_text(plan.get("published_plan_id"))?,
        ),
        maybe_json_entry(
            "published_plan_revision_id",
            optional_text(revision.get("published_plan_revision_id"))?,
        ),
        maybe_json_entry("published_at", optional_text(revision.get("published_at"))?),
        maybe_json_entry("created_at", optional_text(revision.get("created_at"))?),
    ]))
}

fn normalize_revision_provenance_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    Ok(JsonMap::from_iter([
        (
            "plan_id".to_string(),
            JsonValue::String(require_nonempty_text(payload.get("plan_id"), "plan_id")?),
        ),
        (
            "plan_revision_id".to_string(),
            JsonValue::String(require_nonempty_text(
                payload.get("plan_revision_id"),
                "plan_revision_id",
            )?),
        ),
        maybe_json_entry("source_kind", optional_text(payload.get("source_kind"))?),
        maybe_json_entry("created_by", optional_text(payload.get("created_by"))?),
        maybe_json_entry("actor_type", optional_text(payload.get("actor_type"))?),
        maybe_json_entry(
            "publication_state",
            optional_text(payload.get("publication_state"))?,
        ),
        maybe_json_entry(
            "published_plan_id",
            optional_text(payload.get("published_plan_id"))?,
        ),
        maybe_json_entry(
            "published_plan_revision_id",
            optional_text(payload.get("published_plan_revision_id"))?,
        ),
        maybe_json_entry("published_at", optional_text(payload.get("published_at"))?),
        maybe_json_entry("created_at", optional_text(payload.get("created_at"))?),
    ]))
}

fn require_object<'a>(
    value: Option<&'a JsonValue>,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    let Some(value) = value else {
        return Err(format!("{label} must be an object."));
    };
    value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object."))
}

fn require_nonempty_text(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    optional_text(value)?
        .ok_or_else(|| format!("Plan provenance payload must include {field_name}."))
}

fn optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => {
            let trimmed = text.trim();
            Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
        }
        Some(_) => Err("Plan provenance text fields must be strings.".to_string()),
    }
}

fn maybe_json_entry(key: &str, value: Option<String>) -> (String, JsonValue) {
    (
        key.to_string(),
        value.map(JsonValue::String).unwrap_or(JsonValue::Null),
    )
}

#[cfg(test)]
mod tests;
