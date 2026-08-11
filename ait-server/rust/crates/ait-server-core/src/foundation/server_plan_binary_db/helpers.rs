use super::*;

pub(super) fn server_plan_ref(plan_index: u32) -> String {
    format!("PR-{plan_index}")
}

pub(super) fn server_revision_ref(revision_index: u32) -> String {
    format!("plan-revision:{revision_index}")
}

pub(super) fn parse_server_plan_ref(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if let Some(raw) = value.strip_prefix("PR-") {
        return parse_u32_ref(raw, value, "plan");
    }
    if value.is_empty() {
        return Err("Plan Binary DB plan ref must not be empty.".to_string());
    }
    Err(format!(
        "Plan Binary DB plan ref `{value}` is not canonical; use `PR-<index>`."
    ))
}

pub(super) fn parse_server_revision_ref(value: &str) -> Result<u32, String> {
    let value = value.trim();
    if let Some(raw) = value
        .strip_prefix("plan-revision:")
        .or_else(|| value.strip_prefix("revision:"))
    {
        return parse_u32_ref(raw, value, "revision");
    }
    if let Ok(revision_index) = value.parse::<u32>() {
        return Ok(revision_index);
    }
    if value.is_empty() {
        return Err("Plan Binary DB revision ref must not be empty.".to_string());
    }
    Err(format!(
        "Plan Binary DB revision ref `{value}` is not canonical; use `plan-revision:<index>`."
    ))
}

pub(super) fn parse_u32_ref(raw: &str, value: &str, label: &str) -> Result<u32, String> {
    raw.parse::<u32>()
        .map_err(|_| format!("Plan Binary DB {label} ref `{value}` must contain a u32 index."))
}

pub(super) fn item_record_payload(
    item: &JsonValue,
) -> Result<(PlanItemRecord, PlanItemPayload), String> {
    let object = item
        .as_object()
        .ok_or_else(|| "Binary DB plan item must be a JSON object.".to_string())?;
    let plan_item_ref = optional_text(object, "plan_item_ref").unwrap_or_default();
    let text = optional_text(object, "text").unwrap_or_default();
    let checkbox_state = optional_text(object, "checkbox_state").unwrap_or_else(|| "none".into());
    let mut item_meta = match checkbox_state.as_str() {
        "none" => 0,
        "open" => ITEM_STATE_OPEN_META,
        "done" => ITEM_STATE_DONE_META,
        other => {
            return Err(format!(
                "Unsupported Binary DB plan checkbox_state `{other}`."
            ))
        }
    };
    if !plan_item_ref.trim().is_empty() {
        item_meta |= ITEM_HAS_REF_META | ITEM_TASKABLE_HINT_META;
    }
    let heading_path = object
        .get("heading_path")
        .and_then(JsonValue::as_array)
        .map(|path| {
            path.iter()
                .map(|part| {
                    part.as_str().map(ToString::to_string).ok_or_else(|| {
                        "Binary DB plan heading_path entries must be strings.".to_string()
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let line_number = object
        .get("line_number")
        .and_then(json_i64_value)
        .unwrap_or(0);
    Ok((
        PlanItemRecord {
            item_meta,
            reserved0: 0,
            payload_len: 0,
            payload_offset: 0,
            line_number: u32::try_from(line_number)
                .map_err(|_| format!("Binary DB plan line_number is outside u32: {line_number}"))?,
        },
        PlanItemPayload {
            plan_item_ref,
            text,
            heading_path,
        },
    ))
}

pub(super) fn plan_item_view_from_compact_record(
    record: &PlanItemRecord,
    payload: PlanItemPayload,
) -> Result<JsonValue, String> {
    let mut item = JsonMap::new();
    if !payload.plan_item_ref.trim().is_empty() {
        item.insert(
            "plan_item_ref".to_string(),
            json!(payload.plan_item_ref.trim()),
        );
    }
    if !payload.text.trim().is_empty() {
        item.insert("text".to_string(), json!(payload.text.trim()));
    }
    let checkbox_state = match record.item_meta & (ITEM_STATE_OPEN_META | ITEM_STATE_DONE_META) {
        ITEM_STATE_OPEN_META => "open",
        ITEM_STATE_DONE_META => "done",
        0 => "none",
        _ => {
            return Err(format!(
                "Unsupported Binary DB plan item_meta checkbox bits: {}",
                record.item_meta
            ))
        }
    };
    if checkbox_state != "none" {
        item.insert("checkbox_state".to_string(), json!(checkbox_state));
    }
    if !payload.heading_path.is_empty() {
        item.insert(
            "heading_path".to_string(),
            JsonValue::Array(
                payload
                    .heading_path
                    .into_iter()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
    }
    if record.line_number != 0 {
        item.insert("line_number".to_string(), json!(record.line_number));
    }
    Ok(JsonValue::Object(item))
}

pub(super) fn normalize_plan_status(value: Option<&str>) -> Result<String, String> {
    let status = clean_optional_text(value).unwrap_or_else(|| "draft".to_string());
    match status.as_str() {
        "active" | "archived" | "draft" | "superseded" => Ok(status),
        _ => Err(format!("Unsupported plan status: {status}")),
    }
}

pub(super) fn plan_meta_for_status(status: &str) -> Result<u8, String> {
    Ok(match status {
        "active" | "draft" => PLAN_STATE_DRAFT_META,
        "archived" => PLAN_STATE_ARCHIVED_META,
        "superseded" => PLAN_STATE_SUPERSEDED_META,
        other => return Err(format!("Unsupported Binary DB plan status `{other}`.")),
    })
}

pub(super) fn plan_meta_with_status(current: u8, status: &str) -> Result<u8, String> {
    Ok((current & !PLAN_STATE_MASK) | plan_meta_for_status(status)?)
}

pub(super) fn plan_status_from_record(record: &PlanRecord) -> Result<String, String> {
    match record.plan_meta & PLAN_STATE_MASK {
        PLAN_STATE_DRAFT_META => Ok("draft".to_string()),
        PLAN_STATE_ARCHIVED_META => Ok("archived".to_string()),
        PLAN_STATE_SUPERSEDED_META => Ok("superseded".to_string()),
        other => Err(format!("Unsupported Binary DB plan_meta `{other}`.")),
    }
}

pub(super) fn revision_index_plus1(revision_index: u32) -> Result<u32, String> {
    revision_index
        .checked_add(1)
        .ok_or_else(|| "Binary DB revision index overflow".to_string())
}

pub(super) fn required_text(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<String, String> {
    match obj.get(field) {
        Some(JsonValue::String(value)) if !value.trim().is_empty() => Ok(value.trim().to_string()),
        Some(value) => {
            let text = value.to_string();
            if text.trim().is_empty() {
                Err(format!("Field `{field}` must be non-empty."))
            } else {
                Ok(text)
            }
        }
        None => Err(format!("Missing required field `{field}`.")),
    }
}

pub(super) fn optional_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::String(value)) => {
            let text = value.trim();
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        }
        Some(value) => {
            let text = value.to_string();
            if text.trim().is_empty() {
                None
            } else {
                Some(text)
            }
        }
    }
}

pub(super) fn exact_optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err("Expected optional text field to be a JSON string.".to_string()),
    }
}

pub(super) fn json_i64_value(value: &JsonValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

pub(super) fn clean_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn utc_now_string() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(super) fn timestamp_s(raw: &str) -> Result<u64, String> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|err| format!("Binary DB plan timestamp `{raw}` is invalid: {err}"))?
        .with_timezone(&Utc);
    u64::try_from(parsed.timestamp())
        .map_err(|_| format!("Binary DB plan timestamp `{raw}` precedes the Unix epoch."))
}

pub(super) fn timestamp_string(timestamp_s: u64) -> Result<String, String> {
    let timestamp = i64::try_from(timestamp_s)
        .map_err(|_| format!("Binary DB plan timestamp `{timestamp_s}` exceeds RFC 3339 range"))?;
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
        .ok_or_else(|| format!("Binary DB plan timestamp `{timestamp_s}` is invalid"))
}

#[cfg(test)]
pub(super) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex_lower(&hasher.finalize())
}

pub(super) fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(format!("{byte:02x}").as_str());
    }
    out
}

pub(super) fn u16_len(len: usize, field: &str) -> Result<u16, String> {
    u16::try_from(len).map_err(|_| format!("{field} length exceeds u16::MAX: {len}"))
}

pub(super) fn binary_error(err: BinaryDbError) -> String {
    binary_db_runtime_error("Binary DB plan runtime failed", err)
}
