use super::*;

fn parse_row_json(field: &str, value: Option<&JsonValue>) -> Result<JsonValue, String> {
    let Some(value) = value else {
        return Ok(JsonValue::Object(JsonMap::new()));
    };
    match value {
        JsonValue::Null => Ok(JsonValue::Object(JsonMap::new())),
        JsonValue::String(text) => serde_json::from_str::<JsonValue>(text)
            .map_err(|err| format!("{field} must be valid JSON: {err}")),
        _ => Ok(value.clone()),
    }
}

fn row_i64(row: &JsonMap<String, JsonValue>, field: &str) -> i64 {
    row.get(field)
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
                .or_else(|| {
                    value
                        .as_str()
                        .and_then(|text| text.trim().parse::<i64>().ok())
                })
        })
        .unwrap_or(0)
}

pub fn row_to_job(row: &JsonMap<String, JsonValue>) -> Result<JsonMap<String, JsonValue>, String> {
    AsyncJobJson::stateless().row_to_job(row)
}

pub(crate) fn row_to_job_impl(
    row: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = row.clone();
    out.insert(
        "payload".to_string(),
        parse_row_json("payload_json", row.get("payload_json"))?,
    );
    out.insert(
        "result".to_string(),
        parse_row_json("result_json", row.get("result_json"))?,
    );

    let attempt_count = row_i64(row, "attempt_count");
    let max_attempts = row_i64(row, "max_attempts");
    let attempts_remaining = (max_attempts - attempt_count).max(0);
    let state = row
        .get("state")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();
    let last_error = row
        .get("last_error")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let retry_pending = state == "queued" && !last_error.is_empty() && attempts_remaining > 0;
    let attempts_exhausted = max_attempts > 0 && attempt_count >= max_attempts;
    let diagnostic_status = if state == "running" {
        "running"
    } else if state == "queued" && retry_pending {
        "retry_pending"
    } else if state == "failed" && attempts_exhausted {
        "exhausted_failed"
    } else if state == "failed" {
        "failed"
    } else if state.is_empty() {
        "unknown"
    } else {
        state.as_str()
    };

    out.insert(
        "attempt_count".to_string(),
        JsonValue::Number(attempt_count.into()),
    );
    out.insert(
        "max_attempts".to_string(),
        JsonValue::Number(max_attempts.into()),
    );
    out.insert(
        "attempts_remaining".to_string(),
        JsonValue::Number(attempts_remaining.into()),
    );
    out.insert(
        "attempts_exhausted".to_string(),
        JsonValue::Bool(attempts_exhausted),
    );
    out.insert("retry_pending".to_string(), JsonValue::Bool(retry_pending));
    out.insert(
        "next_retry_at".to_string(),
        if retry_pending {
            row.get("available_at").cloned().unwrap_or(JsonValue::Null)
        } else {
            JsonValue::Null
        },
    );
    out.insert(
        "diagnostic_status".to_string(),
        JsonValue::String(diagnostic_status.to_string()),
    );
    Ok(out)
}
