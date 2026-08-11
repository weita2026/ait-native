use super::*;

pub(super) fn storage_validation_view(data: &JsonValue) -> JsonValue {
    let validation = data
        .get("validation_summary")
        .and_then(JsonValue::as_object);
    let efficiency = data
        .get("efficiency_summary")
        .and_then(JsonValue::as_object);
    let optimization = data
        .get("optimization_summary")
        .and_then(JsonValue::as_object);
    json!({
        "state": get_obj(validation, "state"),
        "recommended_action": get_obj(validation, "recommended_action"),
        "next_actions": get_obj(validation, "next_actions").unwrap_or_else(|| JsonValue::Array(vec![])),
        "reasons": get_obj(validation, "reasons").unwrap_or_else(|| JsonValue::Array(vec![])),
        "issues": get_obj(validation, "issues").unwrap_or_else(|| JsonValue::Array(vec![])),
        "needs_attention": get_obj(validation, "needs_attention").and_then(|v| v.as_bool().map(JsonValue::Bool)).unwrap_or(JsonValue::Bool(false)),
        "has_pack_optimization": get_obj(validation, "has_pack_optimization").and_then(|v| v.as_bool().map(JsonValue::Bool)).unwrap_or(JsonValue::Bool(false)),
        "has_delta_optimization": get_obj(validation, "has_delta_optimization").and_then(|v| v.as_bool().map(JsonValue::Bool)).unwrap_or(JsonValue::Bool(false)),
        "tracked_blob_count": get_obj(optimization, "tracked_blob_count").unwrap_or_else(|| JsonValue::Number(Number::from(0))),
        "packed_blob_count": data.get("packed_blob_count").cloned().unwrap_or_else(|| JsonValue::Number(Number::from(0))),
        "packed_delta_blob_count": data.get("packed_delta_blob_count").cloned().unwrap_or_else(|| JsonValue::Number(Number::from(0))),
        "pack_count": data.get("pack_count").cloned().unwrap_or_else(|| JsonValue::Number(Number::from(0))),
        "storage_savings_ratio": get_obj(efficiency, "storage_savings_ratio").unwrap_or_else(|| json!(0.0)),
        "delta_pre_archive_savings_ratio": get_obj(efficiency, "delta_pre_archive_savings_ratio").unwrap_or_else(|| json!(0.0)),
    })
}
