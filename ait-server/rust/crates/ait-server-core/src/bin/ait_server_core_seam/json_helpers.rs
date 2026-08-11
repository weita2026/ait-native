use super::*;

pub(super) fn required_arg(name: &str, value: Option<String>) -> Result<String, String> {
    value.ok_or_else(|| format!("Missing required argument `{name}`."))
}

pub(super) fn payload_arg(name: &str, value: Option<String>) -> Result<String, String> {
    let value = required_arg(name, value)?;
    if value == "-" {
        let mut payload = String::new();
        io::stdin()
            .read_to_string(&mut payload)
            .map_err(|exc| format!("Failed to read `{name}` from stdin: {exc}"))?;
        return Ok(payload);
    }

    if let Some(path) = value.strip_prefix('@') {
        if path.trim().is_empty() {
            return Err(format!(
                "Payload file marker for `{name}` must include a path."
            ));
        }
        return fs::read_to_string(path)
            .map_err(|exc| format!("Failed to read `{name}` from file `{path}`: {exc}"));
    }

    Ok(value)
}

pub(super) fn required_array<'a>(
    obj: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<&'a [JsonValue], String> {
    optional_array(obj, field)?.ok_or_else(|| format!("Missing required field `{field}`."))
}

pub(super) fn optional_array<'a>(
    obj: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<&'a [JsonValue]>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Array(values)) => Ok(Some(values.as_slice())),
        Some(_) => Err(format!("Field `{field}` must be a JSON array.")),
    }
}

pub(super) fn required_object<'a>(
    obj: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    obj.get(field)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Field `{field}` must be a JSON object."))
}

pub(super) fn required_value<'a>(
    obj: &'a serde_json::Map<String, JsonValue>,
    field: &str,
) -> Result<&'a JsonValue, String> {
    obj.get(field)
        .ok_or_else(|| format!("Missing required field `{field}`."))
}

pub(super) fn required_text(
    obj: &serde_json::Map<String, JsonValue>,
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

pub(super) fn optional_text(
    obj: &serde_json::Map<String, JsonValue>,
    field: &str,
) -> Option<String> {
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

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) fn optional_i64(
    obj: &serde_json::Map<String, JsonValue>,
    field: &str,
) -> Result<Option<i64>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_i64()
            .ok_or_else(|| format!("Field `{field}` must be an integer."))
            .map(Some),
        Some(JsonValue::String(value)) if value.trim().is_empty() => Ok(None),
        Some(JsonValue::String(value)) => value
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| format!("Field `{field}` must be an integer.")),
        Some(_) => Err(format!("Field `{field}` must be an integer.")),
    }
}

pub(super) fn optional_usize(
    obj: &serde_json::Map<String, JsonValue>,
    field: &str,
) -> Result<Option<usize>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_u64()
            .map(|item| item as usize)
            .ok_or_else(|| format!("Field `{field}` must be a non-negative integer."))
            .map(Some),
        Some(JsonValue::String(value)) if value.trim().is_empty() => Ok(None),
        Some(JsonValue::String(value)) => value
            .trim()
            .parse::<usize>()
            .map(Some)
            .map_err(|_| format!("Field `{field}` must be a non-negative integer.")),
        Some(_) => Err(format!("Field `{field}` must be a non-negative integer.")),
    }
}

pub(super) fn optional_f64(
    obj: &serde_json::Map<String, JsonValue>,
    field: &str,
) -> Result<Option<f64>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_f64()
            .ok_or_else(|| format!("Field `{field}` must be a number."))
            .map(Some),
        Some(JsonValue::String(value)) if value.trim().is_empty() => Ok(None),
        Some(JsonValue::String(value)) => value
            .trim()
            .parse::<f64>()
            .map(Some)
            .map_err(|_| format!("Field `{field}` must be a number.")),
        Some(_) => Err(format!("Field `{field}` must be a number.")),
    }
}

pub(super) fn bytes_map(
    value: Option<&JsonValue>,
) -> Result<Option<std::collections::BTreeMap<String, Vec<u8>>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let obj = value
        .as_object()
        .ok_or_else(|| "resolve_base_blob_map must be a JSON object.".to_string())?;
    let mut out = std::collections::BTreeMap::new();
    for (key, value) in obj {
        out.insert(key.clone(), bytes_value(value)?);
    }
    Ok(Some(out))
}

fn bytes_value(value: &JsonValue) -> Result<Vec<u8>, String> {
    if let Some(text) = value.as_str() {
        return Ok(text.as_bytes().to_vec());
    }
    let array = value
        .as_array()
        .ok_or_else(|| "bytes values must be strings or byte arrays.".to_string())?;
    array
        .iter()
        .map(|item| {
            item.as_u64()
                .filter(|value| *value <= 255)
                .map(|value| value as u8)
                .ok_or_else(|| "byte arrays must contain integers in 0..=255.".to_string())
        })
        .collect()
}

pub(super) fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    let text = serde_json::to_string(value)
        .map_err(|exc| format!("Failed to serialize ait-server-core seam response: {exc}"))?;
    println!("{text}");
    Ok(())
}
