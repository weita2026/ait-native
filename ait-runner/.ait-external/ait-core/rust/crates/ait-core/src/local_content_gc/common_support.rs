use super::*;

pub(super) fn ratio(numerator: i64, denominator: i64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    ((numerator as f64 / denominator as f64) * 10_000.0).round() / 10_000.0
}

pub(super) fn json_i64(value: &JsonValue, field: &str) -> Result<i64, String> {
    value
        .get(field)
        .and_then(JsonValue::as_i64)
        .or_else(|| {
            value
                .get(field)
                .and_then(JsonValue::as_u64)
                .map(|value| value as i64)
        })
        .ok_or_else(|| format!("payload missing integer field: {field}"))
}

pub(super) fn get_obj(obj: Option<&JsonMap<String, JsonValue>>, field: &str) -> Option<JsonValue> {
    obj.and_then(|value| value.get(field).cloned())
}

pub(super) fn path_to_string(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(ToString::to_string)
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}
