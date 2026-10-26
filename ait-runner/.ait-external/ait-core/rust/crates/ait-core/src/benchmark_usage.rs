use crate::json_support::JsonCodec;
use crate::json_support::{json, JsonMap as Map, JsonValue};
use std::fs;
use std::path::Path;

pub fn extract_codex_usage_jsonl(path: &Path, usage_scope: &str) -> Result<JsonValue, String> {
    let contents = fs::read_to_string(path)
        .map_err(|err| format!("Codex usage JSONL not found: {}: {err}", path.display()))?;
    let normalized_scope = normalize_usage_scope(usage_scope)?;
    let mut latest_info: Option<JsonValue> = None;
    let mut latest_turn_completed_usage: Option<JsonValue> = None;
    let mut event_count: u64 = 0;

    for (index, line) in contents.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record = JsonCodec::parse_value_with_error_prefix(
            line,
            &format!(
                "Codex usage JSONL is not valid JSONL: {}:{}",
                path.display(),
                index + 1
            ),
        )
        .map_err(String::from)?;
        let Some(record_obj) = record.as_object() else {
            continue;
        };
        if record_obj.get("type").and_then(JsonValue::as_str) == Some("turn.completed") {
            if let Some(usage) = record_obj.get("usage").filter(|value| value.is_object()) {
                latest_turn_completed_usage = Some(usage.clone());
                event_count += 1;
            }
            continue;
        }
        let Some(payload) = record_obj.get("payload").and_then(JsonValue::as_object) else {
            continue;
        };
        if payload.get("type").and_then(JsonValue::as_str) != Some("token_count") {
            continue;
        }
        let Some(info) = payload.get("info").filter(|value| value.is_object()) else {
            continue;
        };
        if !info
            .get("total_token_usage")
            .is_some_and(JsonValue::is_object)
        {
            continue;
        }
        latest_info = Some(info.clone());
        event_count += 1;
    }

    let (last_usage, total_usage, selected_usage, usage_source) = if let Some(info) = latest_info {
        let last_usage = extract_codex_usage_payload(
            info.get("last_token_usage").and_then(JsonValue::as_object),
        );
        let total_usage = extract_codex_usage_payload(
            info.get("total_token_usage").and_then(JsonValue::as_object),
        );
        let selected_usage = if normalized_scope == "total" {
            total_usage.clone()
        } else {
            last_usage.clone()
        };
        let usage_source = if normalized_scope == "total" {
            "codex_jsonl_total_token_usage"
        } else {
            "codex_jsonl_last_token_usage"
        };
        (last_usage, total_usage, selected_usage, usage_source)
    } else if let Some(usage) = latest_turn_completed_usage {
        let usage = extract_codex_usage_payload(usage.as_object());
        (
            usage.clone(),
            usage.clone(),
            usage,
            "codex_exec_turn_completed_usage",
        )
    } else {
        return Err(format!(
            "No Codex token_count usage found in: {}",
            path.display()
        ));
    };

    let manifest_usage = codex_usage_for_manifest(&selected_usage)?;
    Ok(json!({
        "usage_jsonl_path": path.to_string_lossy(),
        "token_event_count": event_count,
        "usage_source": usage_source,
        "usage_scope": normalized_scope,
        "last_token_usage": last_usage,
        "total_token_usage": total_usage,
        "manifest_usage": manifest_usage,
    }))
}

pub fn extract_codex_usage_bundle_jsonl(
    paths: &[String],
    roles: Option<&[String]>,
    usage_scope: &str,
) -> Result<JsonValue, String> {
    if paths.is_empty() {
        return Err("At least one Codex usage JSONL is required.".to_string());
    }
    if let Some(roles) = roles {
        if roles.len() != paths.len() {
            return Err(
                "usage_roles must match usage_jsonl_paths length when provided.".to_string(),
            );
        }
    }
    let normalized_scope = normalize_usage_scope(usage_scope)?;
    let normalized_roles: Vec<String> = roles
        .unwrap_or(&[])
        .iter()
        .map(|role| normalize_role(role))
        .collect();

    let mut total_event_count = 0_u64;
    let mut usage_files = Vec::new();
    for (index, raw_path) in paths.iter().enumerate() {
        let payload = extract_codex_usage_jsonl(Path::new(raw_path), &normalized_scope)?;
        let usage = payload
            .get("manifest_usage")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let role = normalized_roles
            .get(index)
            .cloned()
            .unwrap_or_else(|| "unclassified".to_string());
        let token_event_count = payload
            .get("token_event_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        total_event_count += token_event_count;
        usage_files.push(json!({
            "usage_jsonl_path": payload.get("usage_jsonl_path").cloned().unwrap_or_else(|| JsonValue::String(raw_path.clone())),
            "token_event_count": token_event_count,
            "role": role,
            "usage_source": payload.get("usage_source").cloned().unwrap_or(JsonValue::Null),
            "usage": usage,
        }));
    }

    let role_breakdown = role_breakdown(&usage_files);
    let usage_source = usage_files
        .first()
        .and_then(|row| row.get("usage_source"))
        .and_then(JsonValue::as_str)
        .map(|source| format!("{source}_sum"))
        .unwrap_or_else(|| "codex_jsonl_total_token_usage_sum".to_string());

    Ok(json!({
        "usage_jsonl_paths": paths,
        "usage_file_count": paths.len(),
        "token_event_count": total_event_count,
        "usage_source": usage_source,
        "usage_scope": normalized_scope,
        "usage_files": usage_files,
        "role_breakdown": role_breakdown,
        "manifest_usage": summed_manifest_usage(
            role_breakdown
                .as_object()
                .map(|roles| roles.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default()
                .iter()
                .collect::<Vec<_>>()
                .as_slice()
        ),
    }))
}

fn normalize_usage_scope(value: &str) -> Result<String, String> {
    let normalized = value.trim().to_ascii_lowercase();
    let normalized = if normalized.is_empty() {
        "total".to_string()
    } else {
        normalized
    };
    if normalized == "total" || normalized == "last" {
        Ok(normalized)
    } else {
        Err(format!("Unsupported Codex usage scope: {value}"))
    }
}

fn normalize_role(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        "unclassified".to_string()
    } else {
        normalized
    }
}

fn extract_codex_usage_payload(payload: Option<&Map<String, JsonValue>>) -> JsonValue {
    let input_tokens = first_int(payload, "input_tokens");
    let output_tokens = first_int(payload, "output_tokens");
    let mut total_tokens = first_int(payload, "total_tokens");
    if total_tokens.is_none() && (input_tokens.is_some() || output_tokens.is_some()) {
        total_tokens = Some(input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0));
    }
    json!({
        "input_tokens": json_int(input_tokens),
        "cached_input_tokens": json_int(first_int(payload, "cached_input_tokens")),
        "output_tokens": json_int(output_tokens),
        "reasoning_output_tokens": json_int(first_int(payload, "reasoning_output_tokens")),
        "total_tokens": json_int(total_tokens),
    })
}

fn codex_usage_for_manifest(total_usage: &JsonValue) -> Result<JsonValue, String> {
    let prompt_tokens = total_usage.get("input_tokens").and_then(JsonValue::as_i64);
    let completion_tokens = total_usage.get("output_tokens").and_then(JsonValue::as_i64);
    if prompt_tokens.is_none() && completion_tokens.is_none() {
        return Err("Codex total_token_usage is missing input_tokens/output_tokens.".to_string());
    }
    Ok(json!({
        "prompt_tokens": json_int(prompt_tokens),
        "completion_tokens": json_int(completion_tokens),
        "total_tokens": total_usage.get("total_tokens").cloned().unwrap_or(JsonValue::Null),
        "cached_input_tokens": total_usage.get("cached_input_tokens").cloned().unwrap_or(JsonValue::Null),
        "reasoning_output_tokens": total_usage.get("reasoning_output_tokens").cloned().unwrap_or(JsonValue::Null),
    }))
}

fn first_int(payload: Option<&Map<String, JsonValue>>, key: &str) -> Option<i64> {
    let value = payload?.get(key)?;
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    if let Some(number) = value.as_u64() {
        return i64::try_from(number).ok();
    }
    if let Some(number) = value.as_f64() {
        if number.is_finite() && number.fract() == 0.0 {
            return i64::try_from(number as i128).ok();
        }
    }
    if let Some(text) = value.as_str() {
        return text.trim().parse::<i64>().ok();
    }
    None
}

fn json_int(value: Option<i64>) -> JsonValue {
    value.map(JsonValue::from).unwrap_or(JsonValue::Null)
}

fn role_breakdown(usage_files: &[JsonValue]) -> JsonValue {
    let mut roles: Vec<String> = usage_files
        .iter()
        .filter_map(|row| row.get("role").and_then(JsonValue::as_str))
        .map(str::to_string)
        .collect();
    roles.sort();
    roles.dedup();

    let mut out = Map::new();
    for role in roles {
        let rows: Vec<&JsonValue> = usage_files
            .iter()
            .filter(|row| row.get("role").and_then(JsonValue::as_str) == Some(role.as_str()))
            .collect();
        let usage_jsonl_paths: Vec<JsonValue> = rows
            .iter()
            .filter_map(|row| row.get("usage_jsonl_path").cloned())
            .collect();
        let token_event_count: u64 = rows
            .iter()
            .map(|row| {
                row.get("token_event_count")
                    .and_then(JsonValue::as_u64)
                    .unwrap_or(0)
            })
            .sum();
        out.insert(
            role,
            json!({
                "usage_file_count": rows.len(),
                "token_event_count": token_event_count,
                "usage_jsonl_paths": usage_jsonl_paths,
                "usage": summed_manifest_usage(&rows),
            }),
        );
    }
    JsonValue::Object(out)
}

fn summed_manifest_usage(rows: &[&JsonValue]) -> JsonValue {
    let mut sums: Map<String, JsonValue> = Map::new();
    for key in [
        "prompt_tokens",
        "completion_tokens",
        "total_tokens",
        "cached_input_tokens",
        "reasoning_output_tokens",
    ] {
        let mut total = 0_i64;
        let mut saw = false;
        for row in rows {
            let usage = row.get("usage").unwrap_or(*row);
            if let Some(value) = usage.get(key).and_then(JsonValue::as_i64) {
                total += value;
                saw = true;
            }
        }
        sums.insert(key.to_string(), json_int(saw.then_some(total)));
    }
    JsonValue::Object(sums)
}

#[cfg(test)]
mod tests;
