use crate::json_support::{json, JsonMap as Map, JsonNumber as Number, JsonValue};
use chrono::{SecondsFormat, Utc};

use crate::json_support::required_object_value;

pub const WORKER_MANIFEST_IR_VERSION: &str = "ait.worker_manifest.v1";

fn clean_optional_str(value: Option<&JsonValue>) -> Option<String> {
    match value {
        Some(JsonValue::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => None,
    }
}

fn json_string(value: String) -> JsonValue {
    JsonValue::String(value)
}

fn json_optional_string(value: Option<String>) -> JsonValue {
    match value {
        Some(text) => JsonValue::String(text),
        None => JsonValue::Null,
    }
}

fn object_or_empty(value: Option<&JsonValue>) -> Map<String, JsonValue> {
    value
        .and_then(|value| required_object_value(value, "worker manifest object").ok())
        .cloned()
        .unwrap_or_default()
}

fn default_config_map() -> Map<String, JsonValue> {
    Map::from_iter([
        ("version".to_string(), JsonValue::Number(Number::from(1))),
        ("workers".to_string(), JsonValue::Object(Map::new())),
    ])
}

fn coerce_config_version(
    value: Option<&JsonValue>,
    path: Option<&str>,
    issues: &mut Vec<JsonValue>,
) -> i64 {
    match value {
        None | Some(JsonValue::Bool(_)) | Some(JsonValue::Null) => 1,
        Some(JsonValue::Number(number)) => {
            if let Some(parsed) = number.as_i64() {
                parsed
            } else if let Some(parsed) = number.as_u64() {
                i64::try_from(parsed).unwrap_or(1)
            } else {
                issues.push(json_string(format!(
                    "Worker config at {} has invalid version value {:?}; defaulting to 1.",
                    path.unwrap_or("<memory>"),
                    value
                )));
                1
            }
        }
        Some(JsonValue::String(text)) => match text.parse::<i64>() {
            Ok(parsed) => parsed,
            Err(_) => {
                issues.push(json_string(format!(
                    "Worker config at {} has invalid version value {:?}; defaulting to 1.",
                    path.unwrap_or("<memory>"),
                    text
                )));
                1
            }
        },
        Some(other) => {
            issues.push(json_string(format!(
                "Worker config at {} has invalid version value {:?}; defaulting to 1.",
                path.unwrap_or("<memory>"),
                other
            )));
            1
        }
    }
}

fn coerce_optional_str(
    value: Option<&JsonValue>,
    path: Option<&str>,
    issues: &mut Vec<JsonValue>,
    field_name: &str,
    allow_null: bool,
) -> Option<String> {
    match value {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::String(text)) => Some(text.clone()),
        Some(other) => {
            let type_name = match other {
                JsonValue::Bool(_) => "bool",
                JsonValue::Number(_) => "number",
                JsonValue::Array(_) => "array",
                JsonValue::Object(_) => "object",
                JsonValue::Null => "null",
                JsonValue::String(_) => "string",
            };
            if !allow_null {
                issues.push(json_string(format!(
                    "Worker config at {} has invalid non-null value type for {}: {}. Expected string.",
                    path.unwrap_or("<memory>"),
                    field_name,
                    type_name
                )));
            } else {
                issues.push(json_string(format!(
                    "Worker config at {} has invalid value type for {}: {}. Expected string.",
                    path.unwrap_or("<memory>"),
                    field_name,
                    type_name
                )));
            }
            None
        }
    }
}

fn normalize_worker_entry(
    key: &str,
    worker: &JsonValue,
    path: Option<&str>,
    issues: &mut Vec<JsonValue>,
) -> Option<JsonValue> {
    let Ok(worker_map) = required_object_value(worker, "worker") else {
        issues.push(json_string(format!(
            "Worker {:?} must be an object; skipping invalid entry.",
            key
        )));
        return None;
    };
    if !key.contains('/') {
        issues.push(json_string(format!(
            "Worker key {:?} does not match expected `<kind>/<name>` form; skipping invalid entry.",
            key
        )));
        return None;
    }
    let mut key_parts = key.splitn(2, '/');
    let kind = key_parts.next().unwrap_or("").trim().to_string();
    let name = key_parts.next().unwrap_or("").trim().to_string();
    if kind.is_empty() || name.is_empty() {
        issues.push(json_string(format!(
            "Worker key {:?} must include both non-empty kind and name; skipping invalid entry.",
            key
        )));
        return None;
    }

    let mut normalized = worker_map.clone();
    let default_kind = JsonValue::String(kind.clone());
    let kind_seed = match worker_map.get("kind") {
        Some(JsonValue::Null) => &default_kind,
        Some(JsonValue::String(text)) if text.is_empty() => &default_kind,
        None => &default_kind,
        Some(value) => value,
    };
    normalized.insert(
        "kind".to_string(),
        json_optional_string(coerce_optional_str(
            Some(kind_seed),
            path,
            issues,
            &format!("{key}.kind"),
            true,
        )),
    );
    if clean_optional_str(normalized.get("kind")) != Some(kind.clone()) {
        issues.push(json_string(format!(
            "Worker {:?} has kind {:?}; normalized to {:?}.",
            key,
            clean_optional_str(normalized.get("kind")),
            kind
        )));
        normalized.insert("kind".to_string(), json_string(kind.clone()));
    }
    let default_name = JsonValue::String(name.clone());
    let name_seed = match worker_map.get("name") {
        Some(JsonValue::Null) => &default_name,
        Some(JsonValue::String(text)) if text.is_empty() => &default_name,
        None => &default_name,
        Some(value) => value,
    };
    normalized.insert(
        "name".to_string(),
        json_optional_string(coerce_optional_str(
            Some(name_seed),
            path,
            issues,
            &format!("{key}.name"),
            true,
        )),
    );

    for field in ["token", "secret", "app_token", "bot_token"] {
        let value = coerce_optional_str(
            worker_map.get(field),
            path,
            issues,
            &format!("{key}.{field}"),
            true,
        );
        if value.is_none()
            && worker_map.contains_key(field)
            && !matches!(worker_map.get(field), Some(JsonValue::Null))
        {
            issues.push(json_string(format!(
                "Worker {:?} has non-string {} {:?}; value removed to require reconfiguration.",
                key,
                field,
                worker_map.get(field).unwrap_or(&JsonValue::Null)
            )));
        }
        normalized.insert(field.to_string(), json_optional_string(value));
    }

    for field in ["username", "application_id", "public_key"] {
        normalized.insert(
            field.to_string(),
            json_optional_string(coerce_optional_str(
                worker_map.get(field),
                path,
                issues,
                &format!("{key}.{field}"),
                true,
            )),
        );
    }

    for field in [
        "sync_state_path",
        "pid_file",
        "log_file",
        "env_path",
        "termination_context_path",
        "created_at",
        "updated_at",
    ] {
        if normalized.contains_key(field) {
            normalized.insert(
                field.to_string(),
                json_optional_string(coerce_optional_str(
                    normalized.get(field),
                    path,
                    issues,
                    &format!("{key}.{field}"),
                    true,
                )),
            );
        }
    }

    Some(JsonValue::Object(normalized))
}

pub struct WorkerManifestJson<S> {
    _store: S,
}

impl<S> WorkerManifestJson<S> {
    pub fn new(store: S) -> Self {
        Self { _store: store }
    }

    pub fn ir_version(&self) -> &'static str {
        WORKER_MANIFEST_IR_VERSION
    }

    pub fn schema_json(&self) -> JsonValue {
        worker_manifest_schema_json_value()
    }

    pub fn default_config_json(&self) -> JsonValue {
        default_worker_manifest_config_json_value()
    }

    pub fn normalize_document_json(&self, payload: &JsonValue, path: Option<&str>) -> JsonValue {
        normalize_worker_manifest_document_json_value(payload, path)
    }

    pub fn upsert_worker_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        upsert_worker_manifest_worker_json_value(request)
    }

    pub fn select_telegram_worker(
        &self,
        config: &JsonValue,
        requested_name: Option<&str>,
    ) -> JsonValue {
        select_telegram_worker_json_value(config, requested_name)
    }
}

impl WorkerManifestJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

pub fn worker_manifest_ir_version() -> &'static str {
    WorkerManifestJson::stateless().ir_version()
}

pub fn worker_manifest_schema_json() -> JsonValue {
    WorkerManifestJson::stateless().schema_json()
}

pub fn default_worker_manifest_config_json() -> JsonValue {
    WorkerManifestJson::stateless().default_config_json()
}

pub fn normalize_worker_manifest_document_json(
    payload: &JsonValue,
    path: Option<&str>,
) -> JsonValue {
    WorkerManifestJson::stateless().normalize_document_json(payload, path)
}

pub fn upsert_worker_manifest_worker_json(request: &JsonValue) -> Result<JsonValue, String> {
    WorkerManifestJson::stateless().upsert_worker_json(request)
}

pub fn select_telegram_worker_json(config: &JsonValue, requested_name: Option<&str>) -> JsonValue {
    WorkerManifestJson::stateless().select_telegram_worker(config, requested_name)
}

fn worker_manifest_schema_json_value() -> JsonValue {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://ait.dev/schema/ait.worker_manifest.v1.schema.json",
        "title": "AitWorkerManifestSchema",
        "type": "object",
        "additionalProperties": false,
        "required": ["ir_version", "config"],
        "properties": {
            "ir_version": {"const": WORKER_MANIFEST_IR_VERSION},
            "config": {
                "type": "object",
                "required": ["version", "workers"],
                "additionalProperties": true,
                "properties": {
                    "version": {"type": "integer"},
                    "workers": {
                        "type": "object",
                        "additionalProperties": {"type": "object"}
                    }
                }
            },
            "issues": {
                "type": "array",
                "items": {"type": "string"}
            }
        }
    })
}

fn default_worker_manifest_config_json_value() -> JsonValue {
    JsonValue::Object(default_config_map())
}

fn normalize_worker_manifest_document_json_value(
    payload: &JsonValue,
    path: Option<&str>,
) -> JsonValue {
    let config_value = match payload {
        JsonValue::Object(map) => map.get("config").unwrap_or(payload),
        _ => payload,
    };
    let mut issues = Vec::new();
    let config_source = match config_value {
        JsonValue::Object(map) => map.clone(),
        JsonValue::Null => default_config_map(),
        _ => {
            issues.push(json_string(format!(
                "Worker config root must be a JSON object at {}",
                path.unwrap_or("<memory>")
            )));
            default_config_map()
        }
    };

    let mut config = Map::new();
    config.insert(
        "version".to_string(),
        JsonValue::Number(Number::from(coerce_config_version(
            config_source.get("version"),
            path,
            &mut issues,
        ))),
    );

    let mut workers = Map::new();
    match config_source.get("workers") {
        Some(JsonValue::Object(raw_workers)) => {
            for (key, worker) in raw_workers {
                if let Some(normalized) = normalize_worker_entry(key, worker, path, &mut issues) {
                    workers.insert(key.clone(), normalized);
                }
            }
        }
        _ => {
            issues.push(json_string(format!(
                "Worker config at {} is missing or has invalid `workers` map.",
                path.unwrap_or("<memory>")
            )));
        }
    }
    config.insert("workers".to_string(), JsonValue::Object(workers));

    json!({
        "ir_version": WORKER_MANIFEST_IR_VERSION,
        "config": JsonValue::Object(config),
        "issues": JsonValue::Array(issues)
    })
}

fn upsert_worker_manifest_worker_json_value(request: &JsonValue) -> Result<JsonValue, String> {
    let request_map = required_object_value(request, "worker manifest upsert request")
        .map_err(|_| "worker manifest upsert request must be an object".to_string())?;
    let path = request_map.get("path").and_then(JsonValue::as_str);
    let config_seed = request_map
        .get("config")
        .ok_or_else(|| "worker manifest upsert request is missing config".to_string())?;
    let worker_seed = request_map
        .get("worker")
        .and_then(|value| required_object_value(value, "worker").ok())
        .ok_or_else(|| "worker manifest upsert request is missing worker object".to_string())?;

    let normalized = normalize_worker_manifest_document_json(config_seed, path);
    let mut issues = normalized
        .get("issues")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mut config = normalized
        .get("config")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_else(default_config_map);
    let mut workers = object_or_empty(config.get("workers"));

    let kind =
        normalize_worker_key_part(worker_seed.get("kind"), "worker.kind")?.to_ascii_lowercase();
    let name = normalize_worker_key_part(worker_seed.get("name"), "worker.name")?;
    let key = format!("{kind}/{name}");
    let existing = workers.get(&key).and_then(JsonValue::as_object).cloned();
    let now =
        clean_optional_str(request_map.get("updated_at")).unwrap_or_else(current_utc_timestamp);
    let created_at = clean_optional_str(worker_seed.get("created_at"))
        .or_else(|| {
            existing
                .as_ref()
                .and_then(|worker| clean_optional_str(worker.get("created_at")))
        })
        .unwrap_or_else(|| now.clone());

    let mut worker = worker_seed.clone();
    worker.insert("kind".to_string(), JsonValue::String(kind));
    worker.insert("name".to_string(), JsonValue::String(name));
    worker.insert("created_at".to_string(), JsonValue::String(created_at));
    worker.insert("updated_at".to_string(), JsonValue::String(now));
    workers.insert(key.clone(), JsonValue::Object(worker));
    config.insert("workers".to_string(), JsonValue::Object(workers));

    let normalized_after =
        normalize_worker_manifest_document_json(&JsonValue::Object(config), path);
    issues.extend(
        normalized_after
            .get("issues")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default(),
    );
    let config = normalized_after
        .get("config")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_else(default_config_map);
    let worker = config
        .get("workers")
        .and_then(JsonValue::as_object)
        .and_then(|workers| workers.get(&key))
        .cloned()
        .unwrap_or(JsonValue::Null);

    Ok(json!({
        "ir_version": WORKER_MANIFEST_IR_VERSION,
        "config": JsonValue::Object(config),
        "worker_key": key,
        "worker": worker,
        "issues": JsonValue::Array(issues),
        "python_worker_execution_allowed": false,
        "migration_stage": "rust_agent_worker_manifest_upsert_contract"
    }))
}

fn select_telegram_worker_json_value(
    config: &JsonValue,
    requested_name: Option<&str>,
) -> JsonValue {
    let config_map = match config {
        JsonValue::Object(map) => {
            if let Some(JsonValue::Object(inner)) = map.get("config") {
                inner.clone()
            } else {
                map.clone()
            }
        }
        _ => Map::new(),
    };
    let workers = object_or_empty(config_map.get("workers"));
    let requested = requested_name.unwrap_or("main").trim();
    if !requested.is_empty() {
        let key = format!("telegram/{requested}");
        if let Some(JsonValue::Object(worker)) = workers.get(&key) {
            return JsonValue::Object(worker.clone());
        }
    }
    let mut telegram_workers: Vec<(String, Map<String, JsonValue>)> = workers
        .into_iter()
        .filter_map(|(key, worker)| match worker {
            JsonValue::Object(map) if key.starts_with("telegram/") => Some((key, map)),
            _ => None,
        })
        .collect();
    telegram_workers.sort_by(|left, right| left.0.cmp(&right.0));
    if telegram_workers.len() == 1 {
        JsonValue::Object(telegram_workers.remove(0).1)
    } else {
        JsonValue::Null
    }
}

fn normalize_worker_key_part(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<String, String> {
    let Some(text) = clean_optional_str(value) else {
        return Err(format!("{field_name} must not be empty"));
    };
    if text.contains('/') {
        return Err(format!("{field_name} must not contain `/`"));
    }
    Ok(text)
}

fn current_utc_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests;
