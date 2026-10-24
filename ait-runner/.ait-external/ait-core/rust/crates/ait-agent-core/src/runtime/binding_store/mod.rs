use std::path::{Path, PathBuf};

use ait_core::file_io::FilesystemFileIoStore;
use ait_core::json_support::{json, JsonValue};

mod operations;
mod persistence;

pub use operations::agent_runtime_binding_projection_json;

pub const AGENT_RUNTIME_BINDING_STORE_CONTRACT: &str = "ait.agent.runtime_binding_store.v2";

#[derive(Debug, Clone)]
pub struct AgentRuntimeBindingStore {
    path: PathBuf,
    io: FilesystemFileIoStore,
}

impl AgentRuntimeBindingStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            io: FilesystemFileIoStore,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lock_path(&self) -> PathBuf {
        let mut value = self.path.as_os_str().to_os_string();
        value.push(".lock");
        PathBuf::from(value)
    }
}

pub fn agent_runtime_binding_store_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "ait-agent runtime binding store request must be an object".to_string())?;
    let path = object
        .get("path")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ait-agent runtime binding store request requires path".to_string())?;
    let operation = object
        .get("operation")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ait-agent runtime binding store request requires operation".to_string())?;
    let result = AgentRuntimeBindingStore::new(path).execute(operation, request)?;
    Ok(json!({
        "contract": AGENT_RUNTIME_BINDING_STORE_CONTRACT,
        "operation": operation,
        "result": result,
        "python_file_mutation_allowed": false,
    }))
}

#[cfg(test)]
mod tests;
