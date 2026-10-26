use super::facade::DiagnosticsJson;
use crate::json_support::{JsonCodec, JsonEncodeOptions, JsonMap, JsonValue};

pub(super) const PLAN_DIAGNOSTICS_COMMAND_SURFACE: &[&str] = &[
    "ait plan list",
    "ait plan show",
    "ait plan revisions",
    "ait plan items",
    "ait plan candidates",
    "ait plan inspect",
    "ait plan sync",
];

const PLAN_CONFIG_RUNTIME_SELECTION_KEYS: &[&str] = &[
    "plan_core_backend",
    "plan_http_backend",
    "plan_filesystem_backend",
    "plan_blob_diff_backend",
    "plan_pack_substrate_backend",
    "workflow_primitives_backend",
    "plan_ports_protocols_backend",
    "plan_config_runtime_backend",
];

pub(super) const PLAN_AUTHORITY_CONTRACT_VERSION: &str = "plan-foundation-v7";
pub(super) const TASK_CONTRACT_VERSION: &str = "task-close-v1";
pub(super) const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(super) const PLAN_AUTHORITY_COMMAND_SURFACE: &[&str] =
    &["ait plan items", "ait plan candidates", "ait plan inspect"];

pub(super) const PLAN_AUTHORITY_REQUIRED_EXPORTS: &[&str] = &[
    "plan_items_payload",
    "plan_dispatch_summary",
    "plan_task_link_indexes",
    "compute_taskable_items",
    "validate_dispatch_legality",
    "plan_candidates_payload",
    "extract_plan_items",
    "extract_plan_section",
    "find_plan_item",
    "normalize_plan_items",
];

pub fn normalize_plan_diagnostics_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    DiagnosticsJson::stateless().normalize_diagnostics_request_payload_json(payload_json)
}

pub(super) fn normalize_plan_diagnostics_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    if let Some(field) = payload.keys().next() {
        return Err(format!(
            "Unsupported plan diagnostics request field `{field}`; diagnostics read effective configuration and accept no overrides."
        ));
    }
    Ok(JsonValue::Object(JsonMap::new()))
}

pub fn normalize_plan_backend_identity_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    DiagnosticsJson::stateless().normalize_backend_identity_payload_json(payload_json)
}

pub(super) fn normalize_plan_backend_identity_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    Ok(JsonValue::Object(normalize_backend_identity_map(&payload)?))
}

pub fn normalize_plan_diagnostics_compatibility_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    DiagnosticsJson::stateless().normalize_diagnostics_compatibility_payload_json(payload_json)
}

pub(super) fn normalize_plan_diagnostics_compatibility_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let selectors = normalize_selection_facts_map(require_object(
        payload.get("selectors"),
        "plan diagnostics compatibility selectors",
    )?)?;
    let backend_identity = normalize_backend_identity_map(require_object(
        payload.get("backend_identity"),
        "plan diagnostics compatibility backend_identity",
    )?)?;
    let compatible = require_bool(payload.get("compatible"), "compatible")?;
    let issues = normalize_string_list(payload.get("issues"), "issues")?;
    let surface_commands = normalize_command_surface(payload.get("surface_commands"))?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("selectors".to_string(), JsonValue::Object(selectors)),
        (
            "backend_identity".to_string(),
            JsonValue::Object(backend_identity),
        ),
        ("compatible".to_string(), JsonValue::Bool(compatible)),
        ("issues".to_string(), JsonValue::Array(issues)),
        (
            "surface_commands".to_string(),
            JsonValue::Array(surface_commands),
        ),
    ])))
}

pub fn normalize_plan_diagnostics_readiness_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    DiagnosticsJson::stateless().normalize_diagnostics_readiness_payload_json(payload_json)
}

pub(super) fn normalize_plan_diagnostics_readiness_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let selectors = normalize_selection_facts_map(require_object(
        payload.get("selectors"),
        "plan diagnostics readiness selectors",
    )?)?;
    let backend_identity = normalize_backend_identity_map(require_object(
        payload.get("backend_identity"),
        "plan diagnostics readiness backend_identity",
    )?)?;
    let storage_readiness = match payload.get("storage_readiness") {
        None | Some(JsonValue::Null) => JsonValue::Null,
        Some(_) => {
            return Err(
                "Plan diagnostics storage readiness must be null in this runtime.".to_string(),
            )
        }
    };
    let ready = require_bool(payload.get("ready"), "ready")?;
    let issues = normalize_string_list(payload.get("issues"), "issues")?;
    let surface_commands = normalize_command_surface(payload.get("surface_commands"))?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("selectors".to_string(), JsonValue::Object(selectors)),
        (
            "backend_identity".to_string(),
            JsonValue::Object(backend_identity),
        ),
        ("storage_readiness".to_string(), storage_readiness),
        ("ready".to_string(), JsonValue::Bool(ready)),
        ("issues".to_string(), JsonValue::Array(issues)),
        (
            "surface_commands".to_string(),
            JsonValue::Array(surface_commands),
        ),
    ])))
}

pub fn normalize_plan_diagnostics_doctor_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    DiagnosticsJson::stateless().normalize_diagnostics_doctor_payload_json(payload_json)
}

pub(super) fn normalize_plan_diagnostics_doctor_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let selectors = normalize_selection_facts_map(require_object(
        payload.get("selectors"),
        "plan diagnostics doctor selectors",
    )?)?;
    let backend_identity = normalize_backend_identity_map(require_object(
        payload.get("backend_identity"),
        "plan diagnostics doctor backend_identity",
    )?)?;
    let compatibility = match normalize_plan_diagnostics_compatibility_payload_map(
        require_object(
            payload.get("compatibility"),
            "plan diagnostics doctor compatibility",
        )?
        .clone(),
    )? {
        JsonValue::Object(map) => map,
        _ => {
            return Err(
                "Plan diagnostics doctor compatibility normalization must return an object."
                    .to_string(),
            )
        }
    };
    let readiness = match normalize_plan_diagnostics_readiness_payload_map(
        require_object(
            payload.get("readiness"),
            "plan diagnostics doctor readiness",
        )?
        .clone(),
    )? {
        JsonValue::Object(map) => map,
        _ => {
            return Err(
                "Plan diagnostics doctor readiness normalization must return an object."
                    .to_string(),
            )
        }
    };
    let env_map = normalize_env_snapshot_map(require_object(
        payload.get("env"),
        "plan diagnostics doctor env",
    )?)?;
    let explicit_readiness_only = require_bool(
        payload.get("explicit_readiness_only"),
        "explicit_readiness_only",
    )?;
    let surface_commands = normalize_command_surface(payload.get("surface_commands"))?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        ("selectors".to_string(), JsonValue::Object(selectors)),
        (
            "backend_identity".to_string(),
            JsonValue::Object(backend_identity),
        ),
        (
            "compatibility".to_string(),
            JsonValue::Object(compatibility),
        ),
        ("readiness".to_string(), JsonValue::Object(readiness)),
        ("env".to_string(), JsonValue::Object(env_map)),
        (
            "explicit_readiness_only".to_string(),
            JsonValue::Bool(explicit_readiness_only),
        ),
        (
            "surface_commands".to_string(),
            JsonValue::Array(surface_commands),
        ),
    ])))
}

pub(super) fn normalize_selection_facts_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut normalized = JsonMap::new();
    for key in PLAN_CONFIG_RUNTIME_SELECTION_KEYS {
        let entry = require_object(
            payload.get(*key),
            &format!("plan diagnostics selection facts entry `{key}`"),
        )?;
        let value = normalize_backend_name(Some(
            require_string(
                entry.get("value").ok_or_else(|| {
                    format!("Plan diagnostics selection facts entry `{key}` is missing value.")
                })?,
                &format!("{key}.value"),
            )?
            .as_str(),
        ))?;
        let source = require_string(
            entry.get("source").ok_or_else(|| {
                format!("Plan diagnostics selection facts entry `{key}` is missing source.")
            })?,
            &format!("{key}.source"),
        )?;
        if !matches!(source.as_str(), "default" | "env" | "explicit") {
            return Err(format!(
                "Unsupported backend selection source `{source}` for `{key}`."
            ));
        }
        normalized.insert(
            (*key).to_string(),
            JsonValue::Object(JsonMap::from_iter([
                ("value".to_string(), JsonValue::String(value)),
                ("source".to_string(), JsonValue::String(source)),
            ])),
        );
    }
    Ok(normalized)
}

fn normalize_backend_identity_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut normalized = payload.clone();
    let selected_backend = normalize_backend_name(Some(
        require_string(
            payload.get("selected_backend").ok_or_else(|| {
                "Plan diagnostics backend_identity is missing selected_backend.".to_string()
            })?,
            "backend_identity.selected_backend",
        )?
        .as_str(),
    ))?;
    let selected_backend_ready = require_bool(
        payload.get("selected_backend_ready"),
        "backend_identity.selected_backend_ready",
    )?;
    let rust_authority_ready = require_bool(
        payload.get("rust_authority_ready"),
        "backend_identity.rust_authority_ready",
    )?;
    let compatibility = require_nonempty_text(
        payload.get("compatibility"),
        "backend_identity.compatibility",
    )?;
    let extension_module = optional_text(payload.get("extension_module"))?;
    let extension_loaded = require_bool(
        payload.get("extension_loaded"),
        "backend_identity.extension_loaded",
    )?;
    let extension_path = optional_text(payload.get("extension_path"))?;
    let package_version = optional_text(payload.get("package_version"))?;
    let extension_package_version = optional_text(payload.get("extension_package_version"))?;
    let extension_task_contract_version =
        optional_text(payload.get("extension_task_contract_version"))?;
    let extension_plan_contract_version =
        optional_text(payload.get("extension_plan_contract_version"))?;
    let expected_plan_contract_version = require_nonempty_text(
        payload.get("expected_plan_contract_version"),
        "backend_identity.expected_plan_contract_version",
    )?;
    let surface_commands = normalize_command_surface(payload.get("surface_commands"))?;
    let required_exports =
        normalize_string_list(payload.get("required_exports"), "required_exports")?;
    let exports = normalize_bool_map(require_object(
        payload.get("exports"),
        "backend_identity.exports",
    )?)?;
    let missing_exports = normalize_string_list(payload.get("missing_exports"), "missing_exports")?;
    let issues = normalize_string_list(payload.get("issues"), "issues")?;
    let env =
        normalize_env_snapshot_map(require_object(payload.get("env"), "backend_identity.env")?)?;
    normalized.insert(
        "selected_backend".to_string(),
        JsonValue::String(selected_backend),
    );
    normalized.insert(
        "selected_backend_ready".to_string(),
        JsonValue::Bool(selected_backend_ready),
    );
    normalized.insert(
        "rust_authority_ready".to_string(),
        JsonValue::Bool(rust_authority_ready),
    );
    normalized.insert(
        "compatibility".to_string(),
        JsonValue::String(compatibility),
    );
    normalized.insert(
        "extension_module".to_string(),
        extension_module
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    normalized.insert(
        "extension_loaded".to_string(),
        JsonValue::Bool(extension_loaded),
    );
    normalized.insert(
        "extension_path".to_string(),
        extension_path
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    normalized.insert(
        "package_version".to_string(),
        package_version
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    normalized.insert(
        "extension_package_version".to_string(),
        extension_package_version
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    normalized.insert(
        "extension_task_contract_version".to_string(),
        extension_task_contract_version
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    normalized.insert(
        "extension_plan_contract_version".to_string(),
        extension_plan_contract_version
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    normalized.insert(
        "expected_plan_contract_version".to_string(),
        JsonValue::String(expected_plan_contract_version),
    );
    normalized.insert(
        "surface_commands".to_string(),
        JsonValue::Array(surface_commands),
    );
    normalized.insert(
        "required_exports".to_string(),
        JsonValue::Array(required_exports),
    );
    normalized.insert("exports".to_string(), JsonValue::Object(exports));
    normalized.insert(
        "missing_exports".to_string(),
        JsonValue::Array(missing_exports),
    );
    normalized.insert("issues".to_string(), JsonValue::Array(issues));
    normalized.insert("env".to_string(), JsonValue::Object(env));
    Ok(normalized)
}

fn normalize_env_snapshot_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut normalized = JsonMap::new();
    for (key, value) in payload {
        if key.trim().is_empty() {
            return Err("Plan diagnostics env keys must be non-empty strings.".to_string());
        }
        let normalized_value = match value {
            JsonValue::Null => JsonValue::Null,
            JsonValue::String(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    JsonValue::Null
                } else {
                    JsonValue::String(trimmed.to_string())
                }
            }
            _ => {
                return Err(format!(
                    "Plan diagnostics env entry `{key}` must be a string or null."
                ))
            }
        };
        normalized.insert(key.clone(), normalized_value);
    }
    Ok(normalized)
}

fn normalize_bool_map(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut normalized = JsonMap::new();
    for (key, value) in payload {
        if key.trim().is_empty() {
            return Err("Plan diagnostics bool-map keys must be non-empty strings.".to_string());
        }
        normalized.insert(
            key.clone(),
            JsonValue::Bool(require_bool(Some(value), &format!("bool_map.{key}"))?),
        );
    }
    Ok(normalized)
}

fn normalize_command_surface(value: Option<&JsonValue>) -> Result<Vec<JsonValue>, String> {
    let items = normalize_string_list(value, "surface_commands")?;
    if items.is_empty() {
        return Err(
            "Plan diagnostics payload must include a non-empty surface_commands list.".to_string(),
        );
    }
    Ok(items)
}

fn normalize_string_list(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<Vec<JsonValue>, String> {
    match value {
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(|item| {
                Ok(JsonValue::String(require_nonempty_text(
                    Some(item),
                    &format!("{field_name} entry"),
                )?))
            })
            .collect(),
        _ => Err(format!(
            "Plan diagnostics payload field `{field_name}` must be a list."
        )),
    }
}

pub(super) fn require_object<'a>(
    value: Option<&'a JsonValue>,
    field_name: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    match value {
        Some(JsonValue::Object(map)) => Ok(map),
        _ => Err(format!(
            "Plan diagnostics payload field `{field_name}` must be an object."
        )),
    }
}

fn require_string(value: &JsonValue, field_name: &str) -> Result<String, String> {
    match value {
        JsonValue::String(text) => Ok(text.clone()),
        _ => Err(format!(
            "Plan diagnostics payload field `{field_name}` must be a string."
        )),
    }
}

pub(super) fn require_nonempty_text(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<String, String> {
    let text = require_string(
        value
            .ok_or_else(|| format!("Plan diagnostics payload field `{field_name}` is missing."))?,
        field_name,
    )?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "Plan diagnostics payload field `{field_name}` must be non-empty."
        ));
    }
    Ok(trimmed.to_string())
}

pub(super) fn require_bool(value: Option<&JsonValue>, field_name: &str) -> Result<bool, String> {
    match value {
        Some(JsonValue::Bool(flag)) => Ok(*flag),
        _ => Err(format!(
            "Plan diagnostics payload field `{field_name}` must be a boolean."
        )),
    }
}

pub(super) fn optional_text(value: Option<&JsonValue>) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(text)) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        _ => Err("Plan diagnostics optional text fields must be strings.".to_string()),
    }
}

pub(super) fn normalize_backend_name(value: Option<&str>) -> Result<String, String> {
    let text = value.unwrap_or("python").trim().to_ascii_lowercase();
    match text.as_str() {
        "" | "python" => Ok("python".to_string()),
        "rust" => Ok("rust".to_string()),
        other => Err(format!("Unsupported core backend `{other}`.")),
    }
}

pub(super) fn serialize_json_value(value: &JsonValue) -> String {
    JsonCodec::encode_value(value, JsonEncodeOptions::compact())
        .expect("serializing validated plan diagnostics payload should not fail")
}
