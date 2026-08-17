use super::normalization::*;
use crate::config_runtime::build_plan_runtime_selection_facts_json;
use crate::json_support::{json, JsonMap, JsonValue};

pub fn build_plan_backend_identity_facts_json(payload_json: &str) -> Result<JsonValue, String> {
    let context = diagnostics_context(payload_json)?;
    let core_backend = selector_backend(&context.selectors, "plan_core_backend")?;
    let payload = build_backend_identity_payload(&core_backend)?;
    normalize_plan_backend_identity_payload_json(&serialize_json_value(&payload))
}

pub fn build_plan_storage_readiness_facts_json(payload_json: &str) -> Result<JsonValue, String> {
    let _ = diagnostics_context(payload_json)?;
    Ok(JsonValue::Null)
}

pub fn build_plan_diagnostics_compatibility_status_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    let context = diagnostics_context(payload_json)?;
    let backend_identity = build_backend_identity_for_context(&context)?;
    let payload = build_compatibility_payload(&context.selectors, &backend_identity)?;
    normalize_plan_diagnostics_compatibility_payload_json(&serialize_json_value(&payload))
}

pub fn build_plan_diagnostics_readiness_status_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    let context = diagnostics_context(payload_json)?;
    let backend_identity = build_backend_identity_for_context(&context)?;
    let storage_readiness = build_storage_readiness_for_context(&context)?;
    let payload = build_readiness_payload(
        &context.selectors,
        &backend_identity,
        storage_readiness.as_ref(),
    )?;
    normalize_plan_diagnostics_readiness_payload_json(&serialize_json_value(&payload))
}

pub fn build_plan_diagnostics_doctor_facts_json(payload_json: &str) -> Result<JsonValue, String> {
    let context = diagnostics_context(payload_json)?;
    let backend_identity = build_backend_identity_for_context(&context)?;
    let compatibility = build_compatibility_payload(&context.selectors, &backend_identity)?;
    let storage_readiness = build_storage_readiness_for_context(&context)?;
    let readiness = build_readiness_payload(
        &context.selectors,
        &backend_identity,
        storage_readiness.as_ref(),
    )?;
    let payload = JsonValue::Object(JsonMap::from_iter([
        (
            "selectors".to_string(),
            JsonValue::Object(context.selectors.clone()),
        ),
        ("backend_identity".to_string(), backend_identity),
        ("compatibility".to_string(), compatibility),
        ("readiness".to_string(), readiness),
        ("env".to_string(), JsonValue::Object(JsonMap::new())),
        ("explicit_readiness_only".to_string(), JsonValue::Bool(true)),
        (
            "surface_commands".to_string(),
            command_surface_value(PLAN_DIAGNOSTICS_COMMAND_SURFACE),
        ),
    ]));
    normalize_plan_diagnostics_doctor_payload_json(&serialize_json_value(&payload))
}

struct DiagnosticsContext {
    selectors: JsonMap<String, JsonValue>,
}

fn diagnostics_context(payload_json: &str) -> Result<DiagnosticsContext, String> {
    let _request = normalized_request_map(payload_json)?;
    let selectors = build_selection_facts()?;
    Ok(DiagnosticsContext { selectors })
}

fn normalized_request_map(payload_json: &str) -> Result<JsonMap<String, JsonValue>, String> {
    match normalize_plan_diagnostics_request_payload_json(payload_json)? {
        JsonValue::Object(map) => Ok(map),
        _ => Err("Plan diagnostics request normalization must return an object.".to_string()),
    }
}

fn build_selection_facts() -> Result<JsonMap<String, JsonValue>, String> {
    let payload = json!({"overrides": {}});
    match build_plan_runtime_selection_facts_json(&serialize_json_value(&payload))? {
        JsonValue::Object(map) => normalize_selection_facts_map(&map),
        _ => Err(
            "Plan diagnostics selection facts builder returned a non-object payload.".to_string(),
        ),
    }
}

fn selector_backend(selectors: &JsonMap<String, JsonValue>, key: &str) -> Result<String, String> {
    let entry = require_object(
        selectors.get(key),
        &format!("plan diagnostics selection facts entry `{key}`"),
    )?;
    require_nonempty_text(entry.get("value"), &format!("{key}.value"))
}

fn build_backend_identity_for_context(context: &DiagnosticsContext) -> Result<JsonValue, String> {
    let core_backend = selector_backend(&context.selectors, "plan_core_backend")?;
    build_backend_identity_payload(&core_backend)
}

fn build_backend_identity_payload(selected_backend: &str) -> Result<JsonValue, String> {
    let selected_backend = normalize_backend_name(Some(selected_backend))?;
    let rust_ready = selected_backend == "rust";
    let exports = PLAN_AUTHORITY_REQUIRED_EXPORTS
        .iter()
        .map(|name| ((*name).to_string(), JsonValue::Bool(rust_ready)))
        .collect::<JsonMap<String, JsonValue>>();
    let missing_exports = if rust_ready {
        Vec::new()
    } else {
        PLAN_AUTHORITY_REQUIRED_EXPORTS
            .iter()
            .map(|name| JsonValue::String((*name).to_string()))
            .collect::<Vec<_>>()
    };
    let payload = JsonValue::Object(JsonMap::from_iter([
        (
            "selected_backend".to_string(),
            JsonValue::String(selected_backend.clone()),
        ),
        (
            "selected_backend_ready".to_string(),
            JsonValue::Bool(rust_ready),
        ),
        (
            "rust_authority_ready".to_string(),
            JsonValue::Bool(rust_ready),
        ),
        (
            "compatibility".to_string(),
            JsonValue::String(if rust_ready { "compatible" } else { "inactive" }.to_string()),
        ),
        (
            "authority_source".to_string(),
            JsonValue::String("ait-core-rust".to_string()),
        ),
        (
            "extension_module".to_string(),
            JsonValue::String("ait_py".to_string()),
        ),
        ("extension_loaded".to_string(), JsonValue::Bool(rust_ready)),
        ("extension_path".to_string(), JsonValue::Null),
        (
            "package_version".to_string(),
            JsonValue::String(PACKAGE_VERSION.to_string()),
        ),
        (
            "extension_package_version".to_string(),
            if rust_ready {
                JsonValue::String(PACKAGE_VERSION.to_string())
            } else {
                JsonValue::Null
            },
        ),
        (
            "extension_task_contract_version".to_string(),
            if rust_ready {
                JsonValue::String(TASK_CONTRACT_VERSION.to_string())
            } else {
                JsonValue::Null
            },
        ),
        (
            "extension_plan_contract_version".to_string(),
            if rust_ready {
                JsonValue::String(PLAN_AUTHORITY_CONTRACT_VERSION.to_string())
            } else {
                JsonValue::Null
            },
        ),
        (
            "expected_plan_contract_version".to_string(),
            JsonValue::String(PLAN_AUTHORITY_CONTRACT_VERSION.to_string()),
        ),
        (
            "surface_commands".to_string(),
            command_surface_value(PLAN_AUTHORITY_COMMAND_SURFACE),
        ),
        (
            "required_exports".to_string(),
            command_surface_value(PLAN_AUTHORITY_REQUIRED_EXPORTS),
        ),
        ("exports".to_string(), JsonValue::Object(exports)),
        (
            "missing_exports".to_string(),
            JsonValue::Array(missing_exports),
        ),
        ("issues".to_string(), JsonValue::Array(Vec::new())),
        ("env".to_string(), JsonValue::Object(JsonMap::new())),
    ]));
    Ok(payload)
}

fn build_storage_readiness_for_context(
    _context: &DiagnosticsContext,
) -> Result<Option<JsonValue>, String> {
    Ok(None)
}

fn build_compatibility_payload(
    selectors: &JsonMap<String, JsonValue>,
    backend_identity: &JsonValue,
) -> Result<JsonValue, String> {
    let backend_object = require_object(Some(backend_identity), "backend_identity")?;
    let issues = json_array_strings(backend_object.get("issues"), "backend_identity.issues")?;
    let selected_backend_ready = require_bool(
        backend_object.get("selected_backend_ready"),
        "selected_backend_ready",
    )?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "selectors".to_string(),
            JsonValue::Object(selectors.clone()),
        ),
        ("backend_identity".to_string(), backend_identity.clone()),
        (
            "compatible".to_string(),
            JsonValue::Bool(selected_backend_ready),
        ),
        ("issues".to_string(), JsonValue::Array(issues)),
        (
            "surface_commands".to_string(),
            command_surface_value(PLAN_DIAGNOSTICS_COMMAND_SURFACE),
        ),
    ])))
}

fn build_readiness_payload(
    selectors: &JsonMap<String, JsonValue>,
    backend_identity: &JsonValue,
    storage_readiness: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    let backend_object = require_object(Some(backend_identity), "backend_identity")?;
    let mut issues = json_array_strings(backend_object.get("issues"), "backend_identity.issues")?;
    let schema_ready = match storage_readiness {
        None | Some(JsonValue::Null) => true,
        Some(JsonValue::Object(map)) => {
            let ready = require_bool(map.get("ready"), "storage_readiness.ready")?;
            if !ready {
                issues.push(JsonValue::String(
                    "Legacy Plan schema import readiness is not ready.".to_string(),
                ));
            }
            ready
        }
        Some(_) => {
            return Err("Plan diagnostics schema readiness must be an object or null.".to_string())
        }
    };
    let selected_backend_ready = require_bool(
        backend_object.get("selected_backend_ready"),
        "selected_backend_ready",
    )?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "selectors".to_string(),
            JsonValue::Object(selectors.clone()),
        ),
        ("backend_identity".to_string(), backend_identity.clone()),
        (
            "storage_readiness".to_string(),
            storage_readiness.cloned().unwrap_or(JsonValue::Null),
        ),
        (
            "ready".to_string(),
            JsonValue::Bool(selected_backend_ready && schema_ready),
        ),
        ("issues".to_string(), JsonValue::Array(issues)),
        (
            "surface_commands".to_string(),
            command_surface_value(PLAN_DIAGNOSTICS_COMMAND_SURFACE),
        ),
    ])))
}

fn json_array_strings(
    value: Option<&JsonValue>,
    field_name: &str,
) -> Result<Vec<JsonValue>, String> {
    match value {
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(|value| {
                Ok(JsonValue::String(require_nonempty_text(
                    Some(value),
                    field_name,
                )?))
            })
            .collect(),
        _ => Err(format!(
            "Plan diagnostics payload field `{field_name}` must be a list."
        )),
    }
}

fn command_surface_value(values: &[&str]) -> JsonValue {
    JsonValue::Array(
        values
            .iter()
            .map(|value| JsonValue::String((*value).to_string()))
            .collect(),
    )
}
