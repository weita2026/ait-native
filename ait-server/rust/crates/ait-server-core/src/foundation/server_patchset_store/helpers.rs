use super::*;

pub(super) fn schema_table(schema: &str, table: &str) -> String {
    format!("\"{schema}\".\"{table}\"")
}

pub(super) fn ensure_change_mutable(
    change: &JsonMap<String, JsonValue>,
    action: &str,
) -> Result<(), String> {
    let status = optional_text(change.get("status")).unwrap_or_default();
    let change_id = optional_text(change.get("change_id")).unwrap_or_default();
    if status == "archived" {
        return Err(format!(
            "Change {change_id} is archived and cannot {action}"
        ));
    }
    if status == "landed" {
        return Err(format!("Change {change_id} is landed and cannot {action}"));
    }
    Ok(())
}

pub(super) fn derive_patchset_id(
    change_id: &str,
    patchset_number: i64,
    _namespace: Option<&str>,
) -> String {
    if let Some((token, rest)) = change_id.trim().split_once('-') {
        let normalized_token = token.trim().to_ascii_uppercase();
        if normalized_token == "C" {
            return format!("P-{rest}-{patchset_number}");
        }
        if (normalized_token.starts_with('L') || normalized_token.starts_with('R'))
            && normalized_token.ends_with('C')
        {
            let mut patch_token = normalized_token;
            patch_token.pop();
            patch_token.push('P');
            return format!("{patch_token}-{rest}-{patchset_number}");
        }
    }
    format!("P-{change_id}-{patchset_number}")
}

pub(super) fn normalize_author_mode(value: Option<&str>) -> Result<String, String> {
    let value = value.unwrap_or("ai_with_human_review").trim();
    match value {
        "human_only" | "human_with_ai_assist" | "ai_with_human_review" | "ai_only_experimental" => {
            Ok(value.to_string())
        }
        "" => Ok("ai_with_human_review".to_string()),
        other => Err(format!("Unsupported author_mode `{other}`.")),
    }
}

pub(super) fn repo_scoped_sequence_ref(value: &str) -> Option<i64> {
    let text = value.trim();
    if text.is_empty() || !text.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>().ok()
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

pub(super) fn payload_object(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .cloned()
        .ok_or_else(|| format!("{field} must be a JSON object."))
}

pub(super) fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value)
        .ok_or_else(|| format!("patchset-store payload requires text field `{field}`."))
}

pub(super) fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if !truthy(value) {
        return None;
    }
    let text = match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => String::new(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
        JsonValue::Null => String::new(),
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

pub(super) fn truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(value) => *value,
        JsonValue::Number(number) => number.as_f64().map(|value| value != 0.0).unwrap_or(true),
        JsonValue::String(text) => !text.trim().is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
    }
}

pub(super) fn int_value(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        JsonValue::Bool(value) => Some(if *value { 1 } else { 0 }),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

pub(super) fn utc_now() -> String {
    Utc::now().to_rfc3339()
}
