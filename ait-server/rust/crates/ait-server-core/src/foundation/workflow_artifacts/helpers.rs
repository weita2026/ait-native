use super::*;

pub(super) fn json_loads_or_default(value: Option<&JsonValue>, default: JsonValue) -> JsonValue {
    match value {
        Some(JsonValue::String(text)) if !text.is_empty() => {
            serde_json::from_str(text).unwrap_or(default)
        }
        _ => default,
    }
}

pub(super) fn required_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Field `{field}` must be a JSON object."))
}

pub(super) fn optional_object(value: Option<&JsonValue>) -> Option<&JsonMap<String, JsonValue>> {
    value.and_then(JsonValue::as_object)
}

pub(super) fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value).ok_or_else(|| format!("Field `{field}` must be non-empty."))
}

pub(super) fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if !truthy(Some(value)) {
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

pub(super) fn optional_i64(value: Option<&JsonValue>) -> Result<Option<i64>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) if !truthy(Some(value)) => Ok(None),
        Some(JsonValue::Number(number)) => number
            .as_i64()
            .ok_or_else(|| "value must be an integer.".to_string())
            .map(Some),
        Some(JsonValue::String(text)) => text
            .trim()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| "value must be an integer.".to_string()),
        Some(_) => Err("value must be an integer.".to_string()),
    }
}

pub(super) fn truthy(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(value)) => *value,
        Some(JsonValue::Number(number)) => {
            number.as_f64().map(|value| value != 0.0).unwrap_or(true)
        }
        Some(JsonValue::String(value)) => !value.is_empty(),
        Some(JsonValue::Array(values)) => !values.is_empty(),
        Some(JsonValue::Object(values)) => !values.is_empty(),
    }
}

pub(super) fn path_file_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

pub(super) fn component_name(component: Component<'_>) -> Option<String> {
    match component {
        Component::RootDir => Some("/".to_string()),
        Component::Normal(value) => value.to_str().map(str::to_string),
        Component::Prefix(value) => Some(value.as_os_str().to_string_lossy().to_string()),
        Component::CurDir => Some(".".to_string()),
        Component::ParentDir => Some("..".to_string()),
    }
}
