use std::path::Path;

use ait_core::agent_local_workflow_backend::{
    agent_local_workflow_backend_execute_json, AGENT_LOCAL_CURRENT_WORKFLOW_CONTRACT,
    AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION, AGENT_LOCAL_WORKFLOW_BACKEND_CONTRACT,
};
use ait_core::json_support::{JsonMap as Map, JsonValue};

use super::{
    agent_gateway_reply_runtime_execute_json, AgentRuntimeBackend, RemoteAitRuntimeBackend,
    AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT, AGENT_REMOTE_RUNTIME_BACKEND_CONTRACT,
};

pub const AGENT_RUNTIME_BACKEND_CONTRACT: &str = "ait.agent.runtime_backend.v1";

const LOCAL_WORKFLOW_OPERATIONS: &[&str] = &[
    AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION,
    "read_task_queue",
    "read_task",
    "read_change",
    "read_task_audit",
];
const LOCAL_REPLY_OPERATIONS: &[&str] = &["create_turn", "create_telegram_turn"];

pub trait AgentLocalRuntimeBackend {
    fn execute_workflow(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn execute_reply(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeAgentLocalRuntimeBackend;

impl AgentLocalRuntimeBackend for NativeAgentLocalRuntimeBackend {
    fn execute_workflow(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_local_workflow_backend_execute_json(request)
    }

    fn execute_reply(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_gateway_reply_runtime_execute_json(request)
    }
}

#[derive(Debug, Clone)]
pub struct SelectedAitRuntimeBackend<
    R = RemoteAitRuntimeBackend,
    L = NativeAgentLocalRuntimeBackend,
> {
    remote: R,
    local: L,
}

impl<R, L> SelectedAitRuntimeBackend<R, L> {
    pub fn new(remote: R, local: L) -> Self {
        Self { remote, local }
    }
}

impl Default for SelectedAitRuntimeBackend {
    fn default() -> Self {
        Self::new(
            RemoteAitRuntimeBackend::default(),
            NativeAgentLocalRuntimeBackend,
        )
    }
}

impl<R, L> AgentRuntimeBackend for SelectedAitRuntimeBackend<R, L>
where
    R: AgentRuntimeBackend,
    L: AgentLocalRuntimeBackend,
{
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let request_object = required_object(request, "runtime backend request")?;
        let operation = required_text(request_object.get("operation"), "operation")?;
        let target = required_object(
            request_object
                .get("target")
                .ok_or_else(|| "runtime backend request field `target` is required".to_string())?,
            "target",
        )?;
        let mode = required_text(target.get("mode"), "target.mode")?;

        match mode.as_str() {
            "remote" if LOCAL_REPLY_OPERATIONS.contains(&operation.as_str()) => {
                wrap_backend_response(
                    self.local.execute_reply(request)?,
                    &operation,
                    "gateway",
                    AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT,
                )
            }
            "remote" => wrap_backend_response(
                self.remote.execute(request)?,
                &operation,
                "remote",
                AGENT_REMOTE_RUNTIME_BACKEND_CONTRACT,
            ),
            "local" => self.execute_local(request, target, &operation),
            _ => Err("runtime backend target mode must be `local` or `remote`".to_string()),
        }
    }
}

impl<R, L> SelectedAitRuntimeBackend<R, L>
where
    R: AgentRuntimeBackend,
    L: AgentLocalRuntimeBackend,
{
    fn execute_local(
        &self,
        request: &JsonValue,
        target: &Map<String, JsonValue>,
        operation: &str,
    ) -> Result<JsonValue, String> {
        validate_local_target(target)?;
        if LOCAL_WORKFLOW_OPERATIONS.contains(&operation) {
            let expected_contract = if operation == AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION {
                AGENT_LOCAL_CURRENT_WORKFLOW_CONTRACT
            } else {
                AGENT_LOCAL_WORKFLOW_BACKEND_CONTRACT
            };
            return wrap_backend_response(
                self.local.execute_workflow(request)?,
                operation,
                "local",
                expected_contract,
            );
        }
        if LOCAL_REPLY_OPERATIONS.contains(&operation) {
            return wrap_backend_response(
                self.local.execute_reply(request)?,
                operation,
                "gateway",
                AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT,
            );
        }
        Err(format!(
            "unsupported local runtime backend operation `{operation}`"
        ))
    }
}

pub fn agent_runtime_backend_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    SelectedAitRuntimeBackend::default().execute(request)
}

fn validate_local_target(target: &Map<String, JsonValue>) -> Result<(), String> {
    let workflow_mode = required_text(target.get("workflow_mode"), "target.workflow_mode")?;
    if workflow_mode != "solo_local" {
        return Err("local runtime backend workflow_mode must be `solo_local`".to_string());
    }
    required_text(target.get("repo_name"), "target.repo_name")?;
    let repo_root = required_text(target.get("repo_root"), "target.repo_root")?;
    let repo_root = Path::new(&repo_root);
    if !repo_root.is_dir() {
        return Err(format!(
            "Local runtime backend repository root '{}' is not a directory.",
            repo_root.display()
        ));
    }
    if !repo_root.join(".ait").is_dir() {
        return Err(format!(
            "Local runtime backend repository root '{}' does not contain .ait.",
            repo_root.display()
        ));
    }
    Ok(())
}

fn wrap_backend_response(
    response: JsonValue,
    operation: &str,
    backend: &str,
    expected_contract: &str,
) -> Result<JsonValue, String> {
    let response = response
        .as_object()
        .ok_or_else(|| format!("Rust {backend} runtime backend returned a non-object response"))?;
    if response.get("contract").and_then(JsonValue::as_str) != Some(expected_contract) {
        return Err(format!(
            "Rust {backend} runtime backend returned an unsupported contract"
        ));
    }
    if response.get("operation").and_then(JsonValue::as_str) != Some(operation) {
        return Err(format!(
            "Rust {backend} runtime backend returned an operation mismatch"
        ));
    }
    let ok = response
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| format!("Rust {backend} runtime backend omitted boolean `ok`"))?;
    if ok && !response.contains_key("payload") {
        return Err(format!(
            "Rust {backend} runtime backend omitted the successful payload"
        ));
    }

    let mut selected = response.clone();
    selected.insert(
        "backend_contract".to_string(),
        JsonValue::String(expected_contract.to_string()),
    );
    selected.insert(
        "contract".to_string(),
        JsonValue::String(AGENT_RUNTIME_BACKEND_CONTRACT.to_string()),
    );
    selected.insert(
        "backend".to_string(),
        JsonValue::String(backend.to_string()),
    );
    for field in ["retryable", "retry_exhausted"] {
        selected
            .entry(field.to_string())
            .or_insert(JsonValue::Bool(false));
    }
    selected.insert(
        "python_backend_selection_allowed".to_string(),
        JsonValue::Bool(false),
    );
    Ok(JsonValue::Object(selected))
}

fn required_object<'a>(
    value: &'a JsonValue,
    field: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("runtime backend field `{field}` must be an object"))
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("runtime backend field `{field}` must be a non-empty string"))
}
