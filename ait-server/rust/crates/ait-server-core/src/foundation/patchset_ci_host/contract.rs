use super::helpers::optional_text;
use serde_json::{json, Value as JsonValue};

pub fn patchset_ci_contract_available_json(request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request.as_object().ok_or_else(|| {
        "patchset-ci contract-available payload must be a JSON object.".to_string()
    })?;
    let available = payload
        .get("snapshot_paths")
        .and_then(JsonValue::as_array)
        .map(|paths| {
            paths.iter().filter_map(optional_text).any(|path| {
                let normalized = path.replace('\\', "/");
                let parts: Vec<&str> = normalized
                    .split('/')
                    .filter(|part| !part.is_empty() && *part != ".")
                    .collect();
                parts == ["ci", "patch_ci.json"]
            })
        })
        .unwrap_or(false);
    Ok(json!({"available": available}))
}
