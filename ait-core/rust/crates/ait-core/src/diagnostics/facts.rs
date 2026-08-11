use super::normalization::*;
use crate::config_runtime::build_plan_runtime_selection_facts_json;
use crate::file_io::{FileIoStore, FilesystemFileIoStore};
use crate::json_support::{json, JsonMap, JsonValue};
use std::env;
use std::io::{Cursor, Read};
use std::path::Path;
use zip::ZipArchive;

pub fn build_plan_backend_identity_facts_json(payload_json: &str) -> Result<JsonValue, String> {
    let context = diagnostics_context(payload_json)?;
    let core_backend = selector_backend(&context.selectors, "plan_core_backend")?;
    let payload = build_backend_identity_payload(&core_backend)?;
    normalize_plan_backend_identity_payload_json(&serialize_json_value(&payload))
}

pub fn build_plan_wheel_status_facts_json(payload_json: &str) -> Result<JsonValue, String> {
    let request = normalized_request_map(payload_json)?;
    let wheel_path = optional_text(request.get("wheel_path"))?;
    let repack_installed = require_bool(request.get("repack_installed"), "repack_installed")?;
    let smoke = require_bool(request.get("smoke"), "smoke")?;
    let payload = build_wheel_status_payload_with_file_io_store(
        &FilesystemFileIoStore,
        wheel_path.as_deref(),
        repack_installed,
        smoke,
    )?;
    normalize_plan_wheel_status_payload_json(&serialize_json_value(&payload))
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
    let wheel_status = build_wheel_status_for_request(&context.request)?;
    let payload =
        build_compatibility_payload(&context.selectors, &backend_identity, &wheel_status)?;
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
    let wheel_status = build_wheel_status_for_request(&context.request)?;
    let compatibility =
        build_compatibility_payload(&context.selectors, &backend_identity, &wheel_status)?;
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
        ("wheel_status".to_string(), wheel_status),
        ("compatibility".to_string(), compatibility),
        ("readiness".to_string(), readiness),
        ("env".to_string(), JsonValue::Object(doctor_env_snapshot())),
        ("explicit_readiness_only".to_string(), JsonValue::Bool(true)),
        (
            "surface_commands".to_string(),
            command_surface_value(PLAN_DIAGNOSTICS_COMMAND_SURFACE),
        ),
    ]));
    normalize_plan_diagnostics_doctor_payload_json(&serialize_json_value(&payload))
}

struct DiagnosticsContext {
    request: JsonMap<String, JsonValue>,
    selectors: JsonMap<String, JsonValue>,
}

fn diagnostics_context(payload_json: &str) -> Result<DiagnosticsContext, String> {
    let request = normalized_request_map(payload_json)?;
    let selectors = build_selection_facts_for_request(&request)?;
    Ok(DiagnosticsContext { request, selectors })
}

fn normalized_request_map(payload_json: &str) -> Result<JsonMap<String, JsonValue>, String> {
    match normalize_plan_diagnostics_request_payload_json(payload_json)? {
        JsonValue::Object(map) => Ok(map),
        _ => Err("Plan diagnostics request normalization must return an object.".to_string()),
    }
}

fn build_selection_facts_for_request(
    request: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let payload = json!({
        "overrides": request
            .get("overrides")
            .cloned()
            .unwrap_or_else(|| JsonValue::Object(JsonMap::new())),
    });
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
            JsonValue::String(extension_module_name()),
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
        (
            "env".to_string(),
            JsonValue::Object(backend_identity_env_snapshot()),
        ),
    ]));
    Ok(payload)
}

fn build_wheel_status_for_request(
    request: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    build_wheel_status_for_request_with_file_io_store(&FilesystemFileIoStore, request)
}

fn build_wheel_status_for_request_with_file_io_store<S>(
    store: &S,
    request: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String>
where
    S: FileIoStore + ?Sized,
{
    let wheel_path = optional_text(request.get("wheel_path"))?;
    let repack_installed = require_bool(request.get("repack_installed"), "repack_installed")?;
    let smoke = require_bool(request.get("smoke"), "smoke")?;
    build_wheel_status_payload_with_file_io_store(
        store,
        wheel_path.as_deref(),
        repack_installed,
        smoke,
    )
}

pub(super) fn build_wheel_status_payload_with_file_io_store<S>(
    store: &S,
    wheel_path: Option<&str>,
    repack_installed: bool,
    smoke: bool,
) -> Result<JsonValue, String>
where
    S: FileIoStore + ?Sized,
{
    let current_target = current_plan_authority_target();
    let mut issues = Vec::<String>::new();
    if current_target.is_none() {
        issues.push(
            "Current platform target is outside the supported plan authority wheel matrix."
                .to_string(),
        );
    }
    let supported_targets = SUPPORTED_WHEEL_MATRIX
        .iter()
        .map(|(target, os, arch)| {
            json!({
                "target": target,
                "os": os,
                "arch": arch,
            })
        })
        .collect::<Vec<_>>();
    let mut payload = JsonMap::from_iter([
        (
            "supported_matrix".to_string(),
            JsonValue::Array(supported_targets),
        ),
        (
            "current_target".to_string(),
            current_target
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "current_supported".to_string(),
            JsonValue::Bool(current_target.is_some()),
        ),
        ("installed_wheel_tag".to_string(), JsonValue::Null),
        (
            "installed_wheel_tag_source".to_string(),
            JsonValue::String("unavailable_without_python_runtime".to_string()),
        ),
        ("wheel_path".to_string(), JsonValue::Null),
        ("wheel_filename".to_string(), JsonValue::Null),
        ("wheel_source".to_string(), JsonValue::Null),
        ("wheel_tag".to_string(), JsonValue::Null),
        ("wheel_target".to_string(), JsonValue::Null),
        ("wheel_target_supported".to_string(), JsonValue::Null),
        ("wheel_matches_current_target".to_string(), JsonValue::Null),
        ("repack_supported".to_string(), JsonValue::Bool(false)),
        (
            "repack_requested".to_string(),
            JsonValue::Bool(repack_installed),
        ),
        ("smoke_supported".to_string(), JsonValue::Bool(false)),
        ("smoke_requested".to_string(), JsonValue::Bool(smoke)),
        ("smoke".to_string(), JsonValue::Null),
    ]);
    if repack_installed {
        issues.push(
            "Rust ait-core does not repack installed Python extension wheels; build or pass an explicit wheel artifact from the integration layer."
                .to_string(),
        );
    } else if let Some(path_text) = wheel_path {
        let path = Path::new(path_text);
        payload.insert(
            "wheel_path".to_string(),
            JsonValue::String(path_text.to_string()),
        );
        payload.insert(
            "wheel_source".to_string(),
            JsonValue::String("provided".to_string()),
        );
        payload.insert(
            "wheel_filename".to_string(),
            path.file_name()
                .and_then(|value| value.to_str())
                .map(|value| JsonValue::String(value.to_string()))
                .unwrap_or(JsonValue::Null),
        );
        if store.path_exists(path) {
            match inspect_plan_authority_wheel_tag_with_file_io_store(store, path) {
                Ok(wheel_tag) => {
                    let wheel_target = wheel_target_from_tag(wheel_tag.as_deref());
                    let wheel_target_supported = wheel_target.as_deref().map(|target| {
                        SUPPORTED_WHEEL_MATRIX
                            .iter()
                            .any(|(candidate, _, _)| *candidate == target)
                    });
                    let wheel_matches_current_target =
                        match (wheel_target.as_deref(), current_target.as_deref()) {
                            (Some(wheel_target), Some(current_target)) => {
                                Some(wheel_target == current_target)
                            }
                            _ => None,
                        };
                    payload.insert(
                        "wheel_tag".to_string(),
                        wheel_tag
                            .clone()
                            .map(JsonValue::String)
                            .unwrap_or(JsonValue::Null),
                    );
                    payload.insert(
                        "wheel_target".to_string(),
                        wheel_target
                            .clone()
                            .map(JsonValue::String)
                            .unwrap_or(JsonValue::Null),
                    );
                    payload.insert(
                        "wheel_target_supported".to_string(),
                        wheel_target_supported
                            .map(JsonValue::Bool)
                            .unwrap_or(JsonValue::Null),
                    );
                    payload.insert(
                        "wheel_matches_current_target".to_string(),
                        wheel_matches_current_target
                            .map(JsonValue::Bool)
                            .unwrap_or(JsonValue::Null),
                    );
                    if wheel_target.is_none() {
                        issues.push(format!(
                            "Could not derive a supported platform target from wheel tag `{}`.",
                            wheel_tag.as_deref().unwrap_or("missing")
                        ));
                    } else if wheel_target_supported == Some(false) {
                        issues.push(format!(
                            "Wheel tag `{}` is outside the supported plan authority wheel matrix.",
                            wheel_tag.as_deref().unwrap_or("missing")
                        ));
                    }
                }
                Err(err) => issues.push(err),
            }
        } else {
            issues.push(format!("Wheel path does not exist: {path_text}"));
        }
    }
    if smoke {
        payload.insert(
            "smoke".to_string(),
            json!({
                "status": "unsupported",
                "reason": "Rust ait-core does not create Python virtualenvs or import ait_py; run wheel smoke from the external integration layer.",
                "wheel_path": wheel_path.unwrap_or_default(),
                "expected_plan_contract_version": PLAN_AUTHORITY_CONTRACT_VERSION,
                "issues": [],
            }),
        );
        issues.push(
            "Plan authority wheel smoke is unavailable inside repo-pure Rust ait-core.".to_string(),
        );
    }
    payload.insert(
        "issues".to_string(),
        JsonValue::Array(issues.into_iter().map(JsonValue::String).collect()),
    );
    Ok(JsonValue::Object(payload))
}

fn build_storage_readiness_for_context(
    _context: &DiagnosticsContext,
) -> Result<Option<JsonValue>, String> {
    Ok(None)
}

fn build_compatibility_payload(
    selectors: &JsonMap<String, JsonValue>,
    backend_identity: &JsonValue,
    wheel_status: &JsonValue,
) -> Result<JsonValue, String> {
    let core_backend = selector_backend(selectors, "plan_core_backend")?;
    let backend_object = require_object(Some(backend_identity), "backend_identity")?;
    let wheel_object = require_object(Some(wheel_status), "wheel_status")?;
    let mut issues = json_array_strings(backend_object.get("issues"), "backend_identity.issues")?;
    if core_backend == "rust" {
        issues.extend(json_array_strings(
            wheel_object.get("issues"),
            "wheel_status.issues",
        )?);
    }
    let selected_backend_ready = require_bool(
        backend_object.get("selected_backend_ready"),
        "selected_backend_ready",
    )?;
    let current_supported =
        require_bool(wheel_object.get("current_supported"), "current_supported")?;
    Ok(JsonValue::Object(JsonMap::from_iter([
        (
            "selectors".to_string(),
            JsonValue::Object(selectors.clone()),
        ),
        ("backend_identity".to_string(), backend_identity.clone()),
        ("wheel_status".to_string(), wheel_status.clone()),
        (
            "compatible".to_string(),
            JsonValue::Bool(
                selected_backend_ready && (core_backend != "rust" || current_supported),
            ),
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

fn backend_identity_env_snapshot() -> JsonMap<String, JsonValue> {
    env_snapshot(&[
        "AIT_PLAN_BACKEND",
        "AIT_PLAN_CORE_BACKEND",
        "AIT_CORE_BACKEND",
        "AIT_ALLOW_RUST_BACKEND_EXPERIMENTS",
        "AIT_RUST_EXT_MODULE",
    ])
}

fn doctor_env_snapshot() -> JsonMap<String, JsonValue> {
    env_snapshot(&[
        "AIT_PLAN_BACKEND",
        "AIT_PLAN_CORE_BACKEND",
        "AIT_PLAN_HTTP_BACKEND",
        "AIT_PLAN_FILESYSTEM_BACKEND",
        "AIT_PLAN_BLOB_DIFF_BACKEND",
        "AIT_PLAN_PACK_SUBSTRATE_BACKEND",
        "AIT_PLAN_PORTS_PROTOCOLS_BACKEND",
        "AIT_PLAN_CONFIG_RUNTIME_BACKEND",
        "AIT_PLAN_DIAGNOSTICS_BACKEND",
        "AIT_CORE_BACKEND",
        "AIT_ALLOW_RUST_BACKEND_EXPERIMENTS",
        "AIT_RUST_EXT_MODULE",
    ])
}

fn env_snapshot(names: &[&str]) -> JsonMap<String, JsonValue> {
    names
        .iter()
        .map(|name| {
            (
                (*name).to_string(),
                env::var(name)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            )
        })
        .collect()
}

fn extension_module_name() -> String {
    env::var("AIT_RUST_EXT_MODULE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "ait_py".to_string())
}

fn current_plan_authority_target() -> Option<String> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "x86_64") => Some("macos-x86_64".to_string()),
        ("macos", "aarch64") => Some("macos-arm64".to_string()),
        ("linux", "x86_64") => Some("linux-x86_64".to_string()),
        ("linux", "aarch64") => Some("linux-aarch64".to_string()),
        ("windows", "x86_64") => Some("windows-x86_64".to_string()),
        ("windows", "aarch64") => Some("windows-arm64".to_string()),
        _ => None,
    }
}

fn inspect_plan_authority_wheel_tag_with_file_io_store<S>(
    store: &S,
    path: &Path,
) -> Result<Option<String>, String>
where
    S: FileIoStore + ?Sized,
{
    let bytes = store
        .read_bytes(path)
        .map_err(|err| format!("Could not open wheel {}: {err}", path.display()))?;
    let file = Cursor::new(bytes);
    let mut zip = ZipArchive::new(file)
        .map_err(|err| format!("Could not read wheel {}: {err}", path.display()))?;
    for index in 0..zip.len() {
        let mut file = zip
            .by_index(index)
            .map_err(|err| format!("Could not read wheel member {index}: {err}"))?;
        if !file.name().ends_with(".dist-info/WHEEL") {
            continue;
        }
        let mut text = String::new();
        file.read_to_string(&mut text)
            .map_err(|err| format!("Could not decode wheel WHEEL metadata: {err}"))?;
        for line in text.lines() {
            if let Some(tag) = line.strip_prefix("Tag:") {
                let tag = tag.trim();
                if !tag.is_empty() {
                    return Ok(Some(tag.to_string()));
                }
            }
        }
        return Ok(None);
    }
    Ok(None)
}

fn wheel_target_from_tag(tag: Option<&str>) -> Option<String> {
    let normalized = tag?.trim().to_ascii_lowercase();
    if normalized.contains("macosx") {
        if normalized.ends_with("_x86_64") {
            return Some("macos-x86_64".to_string());
        }
        if normalized.ends_with("_arm64") {
            return Some("macos-arm64".to_string());
        }
        return None;
    }
    if normalized.contains("manylinux") || normalized.contains("linux") {
        if normalized.ends_with("_x86_64") {
            return Some("linux-x86_64".to_string());
        }
        if normalized.ends_with("_aarch64") {
            return Some("linux-aarch64".to_string());
        }
        return None;
    }
    if normalized.contains("win_amd64") {
        return Some("windows-x86_64".to_string());
    }
    if normalized.contains("win_arm64") {
        return Some("windows-arm64".to_string());
    }
    None
}
