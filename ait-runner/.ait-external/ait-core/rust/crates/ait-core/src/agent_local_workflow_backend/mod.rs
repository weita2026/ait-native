use std::path::Path;

use crate::json_support::{json, JsonMap, JsonValue};

mod current;

pub use current::{
    agent_local_current_workflow_execute_json, AGENT_LOCAL_CURRENT_WORKFLOW_CONTRACT,
    AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION,
};

pub const AGENT_LOCAL_WORKFLOW_BACKEND_CONTRACT: &str = "ait.agent.local_workflow_backend.v1";
pub const LOCAL_WORKFLOW_AUTHORITY_ERROR: &str = "Local workflow event/release/ownership authority is unsupported; Task and Change use the explicit local Binary DB authority.";
pub const AGENT_LOCAL_WORKFLOW_BACKEND_OPERATIONS: &[&str] = &[
    AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION,
    "read_task_queue",
    "read_task",
    "read_change",
    "read_task_audit",
];

pub fn agent_local_workflow_backend_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "local workflow backend request must be an object".to_string())?;
    let operation = required_text(request.get("operation"), "operation")?;
    if !AGENT_LOCAL_WORKFLOW_BACKEND_OPERATIONS.contains(&operation.as_str()) {
        return Err(format!(
            "Unsupported local workflow backend operation `{operation}`."
        ));
    }
    let target = request
        .get("target")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "local workflow backend request field `target` is required".to_string())?;
    validate_target(target)?;
    let arguments = optional_object(request.get("arguments"), "arguments")?;
    validate_arguments(&operation, arguments)?;

    if operation == AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION {
        return agent_local_current_workflow_execute_json(&JsonValue::Object(request.clone()));
    }

    Ok(json!({
        "contract": AGENT_LOCAL_WORKFLOW_BACKEND_CONTRACT,
        "ok": false,
        "operation": operation,
        "retryable": false,
        "message": LOCAL_WORKFLOW_AUTHORITY_ERROR,
        "error": {
            "kind": "unsupported_authority",
            "message": LOCAL_WORKFLOW_AUTHORITY_ERROR,
        },
    }))
}

fn validate_target(target: &JsonMap<String, JsonValue>) -> Result<(), String> {
    let mode = required_text(target.get("mode"), "target.mode")?;
    if mode != "local" {
        return Err("local workflow backend target mode must be `local`".to_string());
    }
    let workflow_mode = required_text(target.get("workflow_mode"), "target.workflow_mode")?;
    if workflow_mode != "solo_local" {
        return Err("local workflow backend target workflow_mode must be `solo_local`".to_string());
    }
    let repo_root = required_text(target.get("repo_root"), "target.repo_root")?;
    required_text(target.get("repo_name"), "target.repo_name")?;
    let repo_root = Path::new(&repo_root);
    if !repo_root.is_dir() {
        return Err(format!(
            "Local workflow backend repository root '{}' is not a directory.",
            repo_root.display()
        ));
    }
    if !repo_root.join(".ait").is_dir() {
        return Err(format!(
            "Local workflow backend repository root '{}' does not contain .ait.",
            repo_root.display()
        ));
    }
    Ok(())
}

fn validate_arguments(
    operation: &str,
    arguments: Option<&JsonMap<String, JsonValue>>,
) -> Result<(), String> {
    let empty = JsonMap::new();
    let arguments = arguments.unwrap_or(&empty);
    match operation {
        AGENT_LOCAL_CURRENT_WORKFLOW_OPERATION => {
            if arguments.is_empty() {
                Ok(())
            } else {
                Err("local current-workflow arguments must be empty".to_string())
            }
        }
        "read_task_queue" => Ok(()),
        "read_task" => {
            required_text(arguments.get("task_id"), "arguments.task_id")?;
            Ok(())
        }
        "read_change" => {
            required_text(arguments.get("change_id"), "arguments.change_id")?;
            Ok(())
        }
        "read_task_audit" => {
            required_text(arguments.get("task_id"), "arguments.task_id")?;
            if arguments.contains_key("target_line") {
                required_text(arguments.get("target_line"), "arguments.target_line")?;
            }
            Ok(())
        }
        _ => Err(format!(
            "Unsupported local workflow backend operation `{operation}`."
        )),
    }
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            format!("local workflow backend request field `{field}` must be a non-empty string")
        })
}

fn optional_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<Option<&'a JsonMap<String, JsonValue>>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(value)) => Ok(Some(value)),
        Some(_) => Err(format!(
            "local workflow backend request field `{field}` must be an object or null"
        )),
    }
}

#[cfg(test)]
mod tests;
