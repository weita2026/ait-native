use serde_json::{json, Map as JsonMap, Value as JsonValue};

const RETIRED_RISK_LANE_KEYS: [&str; 2] = ["risk_tier", "lane"];

pub const IDENTITY_CONTRACT: &str = "ait.server.identity.v1";
pub const REPO_SCOPED_KEYS_REFERENCE_MODULE: &str =
    "rust/crates/ait-server-core/src/foundation/identity.rs";

pub fn identity_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "identity payload must be a JSON object.".to_string())?;
    match operation {
        "contract" => Ok(json!({
            "contract": IDENTITY_CONTRACT,
            "reference_modules": [
                REPO_SCOPED_KEYS_REFERENCE_MODULE
            ],
            "migration_status": "rust_owned_no_python_reference",
            "mutates_state": false,
            "excluded_reference_behaviors": [
                "control-plane repo_id backfill",
                "control-plane local key backfill",
                "repository lookup by repo_id"
            ],
            "operations": [
                "repo-scope-predicate",
                "local-id-after-first-dash",
                "sequence-after-first-dash",
                "sequence-after-last-dash",
                "repo-scoped-sequence-ref",
                "assert-repo-scope",
                "derive-patchset-id",
                "normalize-task-row",
                "normalize-change-row"
            ],
        })),
        "repo-scope-predicate" => Ok(json!({
            "contract": IDENTITY_CONTRACT,
            "predicate": repo_scope_predicate(optional_text_value(payload.get("alias")).as_deref()),
        })),
        "local-id-after-first-dash" => Ok(json!({
            "contract": IDENTITY_CONTRACT,
            "local_id": local_id_after_first_dash_value(payload.get("value")),
        })),
        "sequence-after-first-dash" => Ok(json!({
            "contract": IDENTITY_CONTRACT,
            "sequence": sequence_after_first_dash_value(payload.get("value")),
        })),
        "sequence-after-last-dash" => Ok(json!({
            "contract": IDENTITY_CONTRACT,
            "sequence": sequence_after_last_dash_value(payload.get("value")),
        })),
        "repo-scoped-sequence-ref" => Ok(json!({
            "contract": IDENTITY_CONTRACT,
            "sequence": repo_scoped_sequence_ref_value(payload.get("value")),
        })),
        "assert-repo-scope" => {
            let repo_name = required_text_value(payload.get("repo_name"), "repo_name")?;
            let resolved_repo_id =
                required_text_value(payload.get("resolved_repo_id"), "resolved_repo_id")?;
            let expected_repo_id = optional_text_value(payload.get("expected_repo_id"));
            Ok(json!({
                "contract": IDENTITY_CONTRACT,
                "repo_id": assert_repo_scope(&repo_name, &resolved_repo_id, expected_repo_id.as_deref())?,
            }))
        }
        "derive-patchset-id" => {
            let change_id = required_text_value(payload.get("change_id"), "change_id")?;
            let patchset_number =
                required_i64_value(payload.get("patchset_number"), "patchset_number")?;
            Ok(json!({
                "contract": IDENTITY_CONTRACT,
                "patchset_id": derive_patchset_id(&change_id, patchset_number),
            }))
        }
        "normalize-task-row" => {
            let row = required_object(payload.get("row"), "row")?;
            Ok(json!({
                "contract": IDENTITY_CONTRACT,
                "row": normalize_task_row(row),
            }))
        }
        "normalize-change-row" => {
            let row = required_object(payload.get("row"), "row")?;
            Ok(json!({
                "contract": IDENTITY_CONTRACT,
                "row": normalize_change_row(row),
            }))
        }
        other => Err(format!("Unsupported identity operation `{other}`.")),
    }
}

pub fn repo_scope_predicate(alias: Option<&str>) -> String {
    let prefix = alias.map(|value| format!("{value}.")).unwrap_or_default();
    format!("({prefix}repo_id = ? or ({prefix}repo_id is null and {prefix}repo_name = ?))")
}

pub fn local_id_after_first_dash(value: Option<&str>) -> Option<String> {
    let text = value.unwrap_or_default().trim();
    if text.is_empty() {
        return None;
    }
    if let Some((_, suffix)) = text.split_once('-') {
        let normalized_suffix = suffix.trim();
        if normalized_suffix.is_empty() {
            return Some(text.to_string());
        }
        return Some(normalized_suffix.to_string());
    }
    Some(text.to_string())
}

pub fn sequence_after_first_dash(value: Option<&str>) -> Option<i64> {
    local_id_after_first_dash(value)?.parse::<i64>().ok()
}

pub fn sequence_after_last_dash(value: Option<&str>) -> Option<i64> {
    let text = value.unwrap_or_default().trim();
    let (_, suffix) = text.rsplit_once('-')?;
    suffix.parse::<i64>().ok()
}

pub fn repo_scoped_sequence_ref(value: Option<&str>) -> Option<i64> {
    let text = value.unwrap_or_default().trim();
    if text.is_empty() || !text.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    text.parse::<i64>().ok()
}

pub fn assert_repo_scope(
    repo_name: &str,
    resolved_repo_id: &str,
    expected_repo_id: Option<&str>,
) -> Result<String, String> {
    let resolved = resolved_repo_id.trim();
    let expected = expected_repo_id.unwrap_or_default().trim();
    if !expected.is_empty() && expected != resolved {
        return Err(format!(
            "Repository scope mismatch for {}: repo_id {} does not match {}",
            repo_name.trim(),
            expected,
            resolved
        ));
    }
    Ok(resolved.to_string())
}

pub fn derive_patchset_id(change_id: &str, patchset_number: i64) -> String {
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

pub fn normalize_row(row: &JsonMap<String, JsonValue>) -> JsonMap<String, JsonValue> {
    let mut normalized = row.clone();
    for key in RETIRED_RISK_LANE_KEYS {
        normalized.remove(key);
    }
    normalized
}

pub fn normalize_task_row(row: &JsonMap<String, JsonValue>) -> JsonMap<String, JsonValue> {
    normalize_row(row)
}

pub fn normalize_change_row(row: &JsonMap<String, JsonValue>) -> JsonMap<String, JsonValue> {
    normalize_row(row)
}

fn local_id_after_first_dash_value(value: Option<&JsonValue>) -> Option<String> {
    let text = optional_text_value(value)?;
    if let Some((_, suffix)) = text.split_once('-') {
        let normalized_suffix = suffix.trim();
        if normalized_suffix.is_empty() {
            return Some(text);
        }
        return Some(normalized_suffix.to_string());
    }
    Some(text)
}

fn sequence_after_first_dash_value(value: Option<&JsonValue>) -> Option<i64> {
    local_id_after_first_dash_value(value)?.parse::<i64>().ok()
}

fn sequence_after_last_dash_value(value: Option<&JsonValue>) -> Option<i64> {
    let text = optional_text_value(value)?;
    let (_, suffix) = text.rsplit_once('-')?;
    suffix.parse::<i64>().ok()
}

fn repo_scoped_sequence_ref_value(value: Option<&JsonValue>) -> Option<i64> {
    let text = optional_text_value(value)?;
    if text.chars().all(|ch| ch.is_ascii_digit()) {
        text.parse::<i64>().ok()
    } else {
        None
    }
}

fn required_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Field `{field}` must be a JSON object."))
}

fn required_text_value(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text_value(value).ok_or_else(|| format!("Field `{field}` must be non-empty."))
}

fn required_i64_value(value: Option<&JsonValue>, field: &str) -> Result<i64, String> {
    match value {
        Some(JsonValue::Number(number)) => number
            .as_i64()
            .ok_or_else(|| format!("Field `{field}` must be an integer.")),
        Some(JsonValue::String(text)) => text
            .trim()
            .parse::<i64>()
            .map_err(|_| format!("Field `{field}` must be an integer.")),
        _ => Err(format!("Field `{field}` must be an integer.")),
    }
}

fn optional_text_value(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if !python_truthy(value) {
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

fn python_truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(value) => *value,
        JsonValue::Number(number) => number.as_f64().map(|value| value != 0.0).unwrap_or(true),
        JsonValue::String(value) => !value.is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
    }
}
