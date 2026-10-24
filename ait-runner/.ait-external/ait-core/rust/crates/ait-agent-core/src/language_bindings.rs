use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use ait_core::environment_contract::names;
use ait_core::json_support::{json, JsonMap, JsonValue};

use crate::{AgentManagementRuntime, TransportKind};

pub const LANGUAGE_BINDING_CONTRACT: &str = "ait.language.binding.v1";

pub fn language_binding_info_json() -> JsonValue {
    json!({
        "contract": LANGUAGE_BINDING_CONTRACT,
        "version": env!("CARGO_PKG_VERSION"),
        "runtime_authority": "rust",
        "python_binding": "pyo3",
        "node_binding": "napi",
        "process_transport_allowed": false,
        "supported_surfaces": [
            "ait-core",
            "ait-agent",
            "ait-agent-worker",
        ],
    })
}

pub fn agent_management_binding_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "ait-agent binding request must be an object".to_string())?;
    let operation = required_text(object.get("operation"), "operation")?;
    let runtime = management_runtime(object)?;

    match operation.as_str() {
        "add" => {
            let worker = object
                .get("worker")
                .filter(|value| value.is_object())
                .cloned()
                .ok_or_else(|| "ait-agent add requires a worker object".to_string())?;
            runtime.add_worker(worker)
        }
        "list" => Ok(JsonValue::Array(
            runtime.list_workers(required_transport(object)?)?,
        )),
        "status" => runtime.status_workers(
            required_transport(object)?,
            optional_text(object.get("name"), "name")?.as_deref(),
        ),
        "start" => runtime.start_worker(
            required_transport(object)?,
            &required_text(object.get("name"), "name")?,
        ),
        "stop" => runtime.stop_worker(
            required_transport(object)?,
            &required_text(object.get("name"), "name")?,
        ),
        "restart" => runtime.restart_worker(
            required_transport(object)?,
            &required_text(object.get("name"), "name")?,
        ),
        "remove" => runtime.remove_worker(
            required_transport(object)?,
            &required_text(object.get("name"), "name")?,
        ),
        "logs" => runtime.worker_logs(
            required_transport(object)?,
            &required_text(object.get("name"), "name")?,
            optional_lines(object.get("lines"))?,
        ),
        other => Err(format!(
            "unsupported ait-agent binding operation `{other}`; expected add, list, status, start, stop, restart, remove, or logs"
        )),
    }
}

fn management_runtime(
    object: &JsonMap<String, JsonValue>,
) -> Result<AgentManagementRuntime, String> {
    let process_current_dir =
        env::current_dir().map_err(|error| format!("failed to read current directory: {error}"))?;
    let current_dir = optional_text(object.get("cwd"), "cwd")?
        .map(PathBuf::from)
        .map(|path| absolute_path(&process_current_dir, &path))
        .unwrap_or(process_current_dir);
    let repo_root = optional_text(object.get("repo_root"), "repo_root")?
        .map(PathBuf::from)
        .or_else(process_repo_root)
        .map(|path| absolute_path(&current_dir, &path))
        .unwrap_or_else(|| current_dir.clone());
    let manifest_path = optional_text(object.get("manifest_path"), "manifest_path")?
        .map(PathBuf::from)
        .or_else(|| env::var_os(names::AIT_AGENT_CONFIG_PATH).map(PathBuf::from))
        .map(|path| absolute_path(&current_dir, &path))
        .unwrap_or_else(|| repo_root.join(".ait/agent-workers.json"));
    let worker_binary = optional_text(object.get("worker_binary"), "worker_binary")?
        .or_else(process_worker_binary)
        .unwrap_or_else(|| "ait-agent-worker".to_string());
    let parent_env = binding_environment(object.get("env"))?;

    Ok(AgentManagementRuntime::filesystem(
        repo_root,
        manifest_path,
        worker_binary,
        parent_env,
    ))
}

fn process_repo_root() -> Option<PathBuf> {
    [names::AIT_REPO_ROOT]
        .iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty())
}

fn process_worker_binary() -> Option<String> {
    if let Ok(value) = env::var(names::AIT_AGENT_RUST_WORKER_BINARY) {
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|parent| parent.join(format!("ait-agent-worker{}", env::consts::EXE_SUFFIX)))
        .filter(|candidate| candidate.is_file())
        .map(|candidate| candidate.to_string_lossy().into_owned())
}

fn binding_environment(value: Option<&JsonValue>) -> Result<BTreeMap<String, String>, String> {
    let mut environment = env::vars().collect::<BTreeMap<_, _>>();
    let Some(value) = value else {
        return Ok(environment);
    };
    if value.is_null() {
        return Ok(environment);
    }
    let object = value
        .as_object()
        .ok_or_else(|| "ait-agent binding env must be an object".to_string())?;
    for (name, value) in object {
        if name.is_empty() {
            return Err("ait-agent binding env names must not be empty".to_string());
        }
        match value {
            JsonValue::Null => {
                environment.remove(name);
            }
            JsonValue::String(value) => {
                environment.insert(name.clone(), value.clone());
            }
            _ => {
                return Err(format!(
                    "ait-agent binding env field `{name}` must be text or null"
                ));
            }
        }
    }
    Ok(environment)
}

fn required_transport(object: &JsonMap<String, JsonValue>) -> Result<TransportKind, String> {
    TransportKind::from_str(&required_text(object.get("transport"), "transport")?)
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| format!("ait-agent binding field `{field}` must be non-empty text"))
}

fn optional_text(value: Option<&JsonValue>, field: &str) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| format!("ait-agent binding field `{field}` must be text or null"))
}

fn optional_lines(value: Option<&JsonValue>) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(200);
    };
    let value = value.as_u64().ok_or_else(|| {
        "ait-agent binding field `lines` must be a non-negative integer".to_string()
    })?;
    usize::try_from(value)
        .map_err(|_| "ait-agent binding field `lines` exceeds this platform".to_string())
}

fn absolute_path(current_dir: &Path, path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    };
    absolute.canonicalize().unwrap_or(absolute)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn binding_info_identifies_direct_language_interfaces() {
        let payload = language_binding_info_json();

        assert_eq!(payload["contract"], LANGUAGE_BINDING_CONTRACT);
        assert_eq!(payload["python_binding"], "pyo3");
        assert_eq!(payload["node_binding"], "napi");
        assert_eq!(payload["process_transport_allowed"], false);
    }

    #[test]
    fn management_binding_lists_empty_manifest_without_cli_process() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".ait")).expect("ait directory");

        let payload = agent_management_binding_json(&json!({
            "operation": "list",
            "transport": "telegram",
            "repo_root": temp.path().to_string_lossy(),
        }))
        .expect("empty list");

        assert_eq!(payload, JsonValue::Array(Vec::new()));
    }

    #[test]
    fn management_binding_rejects_unknown_operations() {
        let error = agent_management_binding_json(&json!({
            "operation": "shell",
        }))
        .expect_err("unknown operation");

        assert!(error.contains("unsupported ait-agent binding operation"));
    }

    #[test]
    fn relative_binding_cwd_is_resolved_once_from_the_process_directory() {
        let process_current_dir = env::current_dir().expect("current directory");
        let relative = Path::new("binding-relative-cwd");

        assert_eq!(
            absolute_path(&process_current_dir, relative),
            process_current_dir.join(relative)
        );
    }
}
