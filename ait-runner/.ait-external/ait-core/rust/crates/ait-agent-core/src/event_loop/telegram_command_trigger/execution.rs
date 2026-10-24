use crate::json_support::encode_value;
use ait_core::environment_contract::names;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use std::io::Write;
use std::process::{Command, Stdio};

pub trait TelegramCommandTriggerOperationExecutor {
    fn execute_operation_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramCommandTriggerOperationExecutor;

impl TelegramCommandTriggerOperationExecutor for DefaultTelegramCommandTriggerOperationExecutor {
    fn execute_operation_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        execute_telegram_command_trigger_operation_json(request)
    }
}

pub fn agent_telegram_command_trigger_execute_operation_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    execute_with_telegram_command_trigger_operation_executor(
        &DefaultTelegramCommandTriggerOperationExecutor,
        request,
    )
}

pub fn execute_with_telegram_command_trigger_operation_executor<E>(
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: TelegramCommandTriggerOperationExecutor + ?Sized,
{
    executor.execute_operation_json(request)
}

fn execute_telegram_command_trigger_operation_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let operation = operation_object(request)?;
    let kind = clean_text(operation.get("kind")).unwrap_or_default();
    if kind != "run_handler" {
        return Ok(operation_failure(
            &kind,
            format!(
                "Unsupported Telegram command trigger operation: {}.",
                if kind.is_empty() { "<missing>" } else { &kind }
            ),
        ));
    }
    let handler_command = text_array_field(operation, "handler_command");
    if handler_command.is_empty() {
        return Ok(operation_failure(
            &kind,
            "Operational trigger handler command is empty.",
        ));
    }

    let cwd = clean_text(operation.get("cwd"));
    let repo_root = clean_text(operation.get("repo_root"));
    let stdin_json = operation
        .get("stdin_json")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let stdin_payload = encode_value(&stdin_json, "failed to encode handler stdin JSON")?;

    let mut command = Command::new(&handler_command[0]);
    command
        .args(&handler_command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd.as_deref().filter(|value| !value.is_empty()) {
        command.current_dir(cwd);
    }
    apply_operation_env(&mut command, operation, repo_root.as_deref());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(exc) => {
            return Ok(json!({
                "kind": kind,
                "method": "std::process::Command",
                "ok": false,
                "returncode": -1,
                "stdout": "",
                "stderr": exc.to_string(),
                "error": exc.to_string(),
            }));
        }
    };

    let stdin_write_error = child.stdin.take().and_then(|mut stdin| {
        stdin
            .write_all(stdin_payload.as_bytes())
            .err()
            .filter(|exc| exc.kind() != std::io::ErrorKind::BrokenPipe)
    });

    let output = child
        .wait_with_output()
        .map_err(|exc| format!("failed to wait for Telegram command trigger handler: {exc}"))?;
    if let Some(exc) = stdin_write_error {
        return Ok(json!({
            "kind": kind,
            "method": "std::process::Command",
            "ok": false,
            "returncode": -1,
            "stdout": "",
            "stderr": exc.to_string(),
            "error": exc.to_string(),
        }));
    }
    let returncode = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let ok = returncode == 0;
    let error = if ok {
        JsonValue::Null
    } else if !stderr.trim().is_empty() {
        JsonValue::String(stderr.trim().to_string())
    } else if !stdout.trim().is_empty() {
        JsonValue::String(stdout.trim().to_string())
    } else {
        JsonValue::String(format!("exit code {returncode}"))
    };

    Ok(json!({
        "kind": kind,
        "method": "std::process::Command",
        "ok": ok,
        "returncode": returncode,
        "stdout": stdout,
        "stderr": stderr,
        "error": error,
    }))
}

fn operation_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .and_then(|object| {
            object
                .get("operation")
                .and_then(JsonValue::as_object)
                .or(Some(object))
        })
        .ok_or_else(|| {
            "Telegram command trigger operation request must be a JSON object.".to_string()
        })
}

fn apply_operation_env(
    command: &mut Command,
    operation: &Map<String, JsonValue>,
    repo_root: Option<&str>,
) {
    let env_overrides = operation
        .get("env_overrides")
        .and_then(JsonValue::as_object);
    let mut has_repo_root = false;
    if let Some(env_overrides) = env_overrides {
        for (key, value) in env_overrides {
            if key == names::AIT_REPO_ROOT {
                has_repo_root = true;
            }
            command.env(key, pythonish_text(value));
        }
    }
    if !has_repo_root {
        if let Some(repo_root) = repo_root.filter(|value| !value.is_empty()) {
            command.env(names::AIT_REPO_ROOT, repo_root);
        }
    }
    if let Some(pythonpath_repo_src) = clean_text(operation.get("pythonpath_repo_src"))
        .or_else(|| repo_root.map(|root| format!("{root}/src")))
    {
        let existing = std::env::var("PYTHONPATH").unwrap_or_default();
        let pythonpath = if existing.trim().is_empty() {
            pythonpath_repo_src
        } else {
            format!("{pythonpath_repo_src}:{existing}")
        };
        command.env("PYTHONPATH", pythonpath);
    }
}

fn operation_failure(kind: &str, error: impl Into<String>) -> JsonValue {
    let error = error.into();
    json!({
        "kind": kind,
        "method": "std::process::Command",
        "ok": false,
        "returncode": -1,
        "stdout": "",
        "stderr": error,
        "error": error,
    })
}

fn text_array_field(object: &Map<String, JsonValue>, key: &str) -> Vec<String> {
    object
        .get(key)
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| clean_text(Some(value)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if matches!(value, JsonValue::Null) {
        return None;
    }
    let text = pythonish_text(value).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn pythonish_text(value: &JsonValue) -> String {
    match value {
        JsonValue::String(value) => value.clone(),
        JsonValue::Bool(value) => {
            if *value {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        JsonValue::Number(value) => value.to_string(),
        JsonValue::Null => String::new(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests;
