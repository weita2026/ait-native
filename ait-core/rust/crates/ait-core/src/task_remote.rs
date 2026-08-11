use crate::json_support::{JsonMap as Map, JsonValue as Value};

fn normalize_optional_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub fn task_remote_change_lineage_payload(
    base_line: &str,
    line_row: Option<&Value>,
) -> Result<Value, String> {
    let normalized_base_line = {
        let text = base_line.trim();
        if text.is_empty() {
            return Err("Task remote lineage payload requires `base_line`.".to_string());
        }
        text.to_string()
    };
    let line_row_object = match line_row {
        Some(Value::Object(map)) => Some(map),
        Some(Value::Null) | None => None,
        Some(_) => {
            return Err("Task remote lineage line row must be an object when provided.".to_string())
        }
    };
    let mut payload = Map::new();
    payload.insert(
        "forked_from_line".to_string(),
        Value::String(normalized_base_line),
    );
    let fork_snapshot_id =
        line_row_object.and_then(|row| normalize_optional_text(row.get("head_snapshot_id")));
    payload.insert(
        "fork_snapshot_id".to_string(),
        fork_snapshot_id.map(Value::String).unwrap_or(Value::Null),
    );
    Ok(Value::Object(payload))
}

#[cfg(test)]
mod tests;
