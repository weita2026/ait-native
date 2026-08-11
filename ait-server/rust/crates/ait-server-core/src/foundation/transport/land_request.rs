use super::*;

pub const LAND_REQUEST_CONTRACT: &str = "ait.server.land_request.v1";
pub const LAND_REQUEST_LANDS_REFERENCE_MODULE: &str = "../ait/src/ait_native/server_api.py";

pub fn land_request_payload(
    row: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    AsyncJobJson::stateless().land_request_payload(row)
}

pub(crate) fn land_request_payload_impl(
    row: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = row.clone();
    out.remove("priority");
    let result_json = out
        .get("result_json")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "result_json must be a JSON string".to_string())?;
    let parsed = serde_json::from_str::<JsonValue>(result_json).map_err(|err| err.to_string())?;
    out.insert("result".to_string(), parsed);
    Ok(out)
}

pub fn land_request_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "land-request payload must be a JSON object.".to_string())?;
    match operation {
        "contract" => Ok(serde_json::json!({
            "contract": LAND_REQUEST_CONTRACT,
            "reference_modules": [LAND_REQUEST_LANDS_REFERENCE_MODULE],
            "migration_status": "payload_wrapper_removed_rust_owned",
            "mutates_state": false,
            "excluded_reference_behaviors": [
                "land request persistence",
                "target line ref reads",
                "snapshot manifest/root tree lookup",
                "line archive/application",
                "queue/retry orchestration"
            ],
            "operations": [
                "payload",
                "phase-timings",
                "freshness-result",
                "snapshot-alignment"
            ],
        })),
        "payload" => {
            let row = payload
                .get("row")
                .and_then(JsonValue::as_object)
                .unwrap_or(payload);
            Ok(serde_json::json!({
                "contract": LAND_REQUEST_CONTRACT,
                "land_request": land_request_payload(row)?,
            }))
        }
        "phase-timings" => {
            let result = payload.get("result").unwrap_or(request);
            Ok(serde_json::json!({
                "contract": LAND_REQUEST_CONTRACT,
                "phase_timings_ms": phase_timings_from_result(Some(result)),
            }))
        }
        "freshness-result" => {
            let target_line = required_text_field(payload, "target_line")?;
            let patchset = payload
                .get("patchset")
                .and_then(JsonValue::as_object)
                .unwrap_or(payload);
            let target_line_head = optional_text_field(payload, "target_line_head");
            let alignment = payload.get("alignment").and_then(JsonValue::as_object);
            let checked_at = required_text_field(payload, "checked_at")?;
            Ok(serde_json::json!({
                "contract": LAND_REQUEST_CONTRACT,
                "freshness": land_freshness_result(
                    &target_line,
                    patchset,
                    target_line_head.as_deref(),
                    alignment,
                    &checked_at,
                ),
            }))
        }
        "snapshot-alignment" => Ok(serde_json::json!({
            "contract": LAND_REQUEST_CONTRACT,
            "alignment": land_snapshot_alignment(
                optional_text_field(payload, "target_line_head").as_deref(),
                optional_text_field(payload, "revision_snapshot_id").as_deref(),
                optional_text_field(payload, "target_manifest_hash").as_deref(),
                optional_text_field(payload, "revision_manifest_hash").as_deref(),
                optional_text_field(payload, "target_root_tree_id").as_deref(),
                optional_text_field(payload, "revision_root_tree_id").as_deref(),
            ),
        })),
        other => Err(format!("Unsupported land-request operation `{other}`.")),
    }
}

pub fn elapsed_ms(start: f64, end: f64) -> f64 {
    let raw = (end - start) * 1000.0;
    (raw * 1000.0).round() / 1000.0
}

pub fn phase_timings_from_result(result: Option<&JsonValue>) -> JsonMap<String, JsonValue> {
    AsyncJobJson::stateless().phase_timings_from_result(result)
}

pub(crate) fn phase_timings_from_result_impl(
    result: Option<&JsonValue>,
) -> JsonMap<String, JsonValue> {
    let Some(JsonValue::Object(result_map)) = result else {
        return JsonMap::new();
    };
    let Some(JsonValue::Object(phase_timings)) = result_map.get("phase_timings_ms") else {
        return JsonMap::new();
    };
    phase_timings.clone()
}
