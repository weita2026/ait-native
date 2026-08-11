use super::*;

pub(super) const REQUIRED_APPROVALS: i64 = 1;
pub(super) const FAKE_POSTGRES_PREFIX: &str = "fake-postgres://";

pub(super) fn schema_table(schema: &str, table: &str) -> String {
    format!("\"{schema}\".\"{table}\"")
}

pub(super) fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value)
        .ok_or_else(|| format!("policy-store payload requires text field `{field}`."))
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

pub(super) fn utc_now() -> String {
    Utc::now().to_rfc3339()
}
