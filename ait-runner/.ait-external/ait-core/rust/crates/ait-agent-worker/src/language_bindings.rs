use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

use ait_core::environment_contract::names;
use ait_core::json_support::{JsonCodec, JsonEncodeOptions, JsonMap, JsonValue};

use crate::{
    compiled_worker_capabilities, execute_discord_interaction_once, execute_native_reply_provider,
    execute_slack_command_once, render_capabilities_json, DiscordInteractionOnceRequest,
    SlackCommandOnceRequest, WorkerPathInputs,
};

pub fn agent_worker_capabilities_binding_json() -> Result<JsonValue, String> {
    let rendered = render_capabilities_json(&compiled_worker_capabilities())?;
    JsonCodec::parse_value_with_error_prefix(
        &rendered,
        "failed to decode ait-agent-worker capability binding payload",
    )
    .map_err(String::from)
}

pub fn agent_worker_transaction_binding_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "ait-agent-worker binding request must be an object".to_string())?;
    let operation = required_text(object.get("operation"), "operation")?;
    let payload = object
        .get("payload")
        .cloned()
        .ok_or_else(|| "ait-agent-worker binding field `payload` is required".to_string())?;
    let worker_name =
        optional_text(object.get("worker"), "worker")?.unwrap_or_else(|| "main".to_string());
    let path_inputs = binding_path_inputs(object)?;
    let process_env = binding_environment(object.get("env"))?;

    match operation.as_str() {
        "slack-command" => execute_slack_command_once(&SlackCommandOnceRequest {
            path_inputs,
            worker_name,
            process_env: process_env.clone(),
            raw_payload: payload_text(&payload)?,
            signature: optional_text(object.get("signature"), "signature")?,
            signature_timestamp: optional_text(
                object.get("signature_timestamp"),
                "signature_timestamp",
            )?,
            now_unix_seconds: optional_i64(object.get("now_unix_seconds"), "now_unix_seconds")?,
        })
        .map_err(|diagnostic| diagnostic.render_json()),
        "discord-interaction" => {
            execute_discord_interaction_once(&DiscordInteractionOnceRequest {
                path_inputs,
                worker_name,
                process_env: process_env.clone(),
                raw_payload: payload_text(&payload)?,
                signature: optional_text(object.get("signature"), "signature")?,
                signature_timestamp: optional_text(
                    object.get("signature_timestamp"),
                    "signature_timestamp",
                )?,
            })
            .map_err(|diagnostic| diagnostic.render_json())
        }
        "reply-provider" => Ok(execute_native_reply_provider(
            &payload_text(&payload)?,
            &process_env,
        )),
        other => Err(format!(
            "unsupported ait-agent-worker binding operation `{other}`; expected slack-command, discord-interaction, or reply-provider"
        )),
    }
}

fn binding_path_inputs(object: &JsonMap<String, JsonValue>) -> Result<WorkerPathInputs, String> {
    let current_dir = optional_text(object.get("cwd"), "cwd")?
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let repo_root_override = optional_text(object.get("repo_root"), "repo_root")?
        .map(PathBuf::from)
        .or_else(process_repo_root);
    let manifest_path_override = optional_text(object.get("manifest_path"), "manifest_path")?
        .map(PathBuf::from)
        .or_else(|| env::var_os(names::AIT_AGENT_CONFIG_PATH).map(PathBuf::from));
    Ok(WorkerPathInputs {
        current_dir,
        repo_root_override,
        manifest_path_override,
    })
}

fn process_repo_root() -> Option<PathBuf> {
    [names::AIT_REPO_ROOT]
        .iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .find(|path| !path.as_os_str().is_empty())
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
        .ok_or_else(|| "ait-agent-worker binding env must be an object".to_string())?;
    for (name, value) in object {
        if name.is_empty() {
            return Err("ait-agent-worker binding env names must not be empty".to_string());
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
                    "ait-agent-worker binding env field `{name}` must be text or null"
                ));
            }
        }
    }
    Ok(environment)
}

fn payload_text(payload: &JsonValue) -> Result<String, String> {
    if let Some(value) = payload.as_str() {
        return Ok(value.to_string());
    }
    JsonCodec::encode_value(payload, JsonEncodeOptions::compact()).map_err(String::from)
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| format!("ait-agent-worker binding field `{field}` must be non-empty text"))
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
        .ok_or_else(|| format!("ait-agent-worker binding field `{field}` must be text or null"))
}

fn optional_i64(value: Option<&JsonValue>, field: &str) -> Result<Option<i64>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_i64()
        .map(Some)
        .ok_or_else(|| format!("ait-agent-worker binding field `{field}` must be an integer"))
}

#[cfg(test)]
mod tests {
    use ait_core::json_support::json;

    use super::*;

    #[test]
    fn capability_binding_returns_the_native_worker_contract() {
        let payload = agent_worker_capabilities_binding_json().expect("capabilities");

        assert_eq!(payload["contract"], "ait.agent.worker.capabilities.v1");
        assert_eq!(payload["binary"], "ait-agent-worker");
        assert_eq!(payload["python_worker_execution_allowed"], false);
    }

    #[test]
    fn transaction_binding_rejects_cli_shaped_unknown_operations() {
        let error = agent_worker_transaction_binding_json(&json!({
            "operation": "run-command",
            "payload": {},
        }))
        .expect_err("unknown operation");

        assert!(error.contains("unsupported ait-agent-worker binding operation"));
    }
}
