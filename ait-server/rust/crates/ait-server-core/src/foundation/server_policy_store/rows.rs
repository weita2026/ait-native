use super::*;

pub(super) fn review_row_json(row: &Row) -> JsonMap<String, JsonValue> {
    let mut out = JsonMap::new();
    insert_i64(&mut out, "review_id", row_i64(row, "review_id"));
    insert_text(&mut out, "reviewer", row_text(row, "reviewer"));
    insert_text(&mut out, "action", row_text(row, "action"));
    out.insert(
        "blocking".to_string(),
        row_bool(row, "blocking").map_or(JsonValue::Null, JsonValue::Bool),
    );
    insert_text(&mut out, "comment", row_text(row, "comment"));
    insert_text(&mut out, "created_at", row_text(row, "created_at"));
    insert_text(&mut out, "patchset_id", row_text(row, "patchset_id"));
    out
}

pub(super) fn parse_json_object(raw: &str) -> Result<JsonMap<String, JsonValue>, String> {
    let parsed = serde_json::from_str::<JsonValue>(raw).map_err(|exc| exc.to_string())?;
    Ok(parsed.as_object().cloned().unwrap_or_default())
}

pub(super) fn json_object(
    value: Option<&JsonMap<String, JsonValue>>,
    field: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    value
        .cloned()
        .ok_or_else(|| format!("{field} must be a JSON object."))
}

pub(super) fn row_text(row: &Row, name: &str) -> Option<String> {
    row.try_get::<_, Option<String>>(name).ok().flatten()
}

pub(super) fn row_i64(row: &Row, name: &str) -> Option<i64> {
    row.try_get::<_, Option<i64>>(name).ok().flatten()
}

pub(super) fn row_bool(row: &Row, name: &str) -> Option<bool> {
    row.try_get::<_, Option<bool>>(name).ok().flatten()
}

pub(super) fn insert_text(out: &mut JsonMap<String, JsonValue>, key: &str, value: Option<String>) {
    out.insert(
        key.to_string(),
        value.map(JsonValue::String).unwrap_or(JsonValue::Null),
    );
}

pub(super) fn insert_i64(out: &mut JsonMap<String, JsonValue>, key: &str, value: Option<i64>) {
    out.insert(
        key.to_string(),
        value.map_or(JsonValue::Null, JsonValue::from),
    );
}
