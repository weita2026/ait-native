use std::collections::BTreeMap;

use ait_agent_core::{
    agent_discord_ingress_runtime_plan_json, agent_discord_interaction_job_execute_json,
    AgentWorkerRuntimeConfig, DiscordWorkerConfig, TransportKind,
};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::diagnostic::{WorkerDiagnostic, EXIT_INVALID_CONFIGURATION, EXIT_INVALID_REQUEST};
use crate::paths::{resolve_worker_paths, WorkerPathInputs};
use crate::run::{resolve_worker_selection, validate_worker_name};

pub const DISCORD_INTERACTION_ONCE_CONTRACT: &str = "ait.agent.worker.discord-interaction-once.v1";
const DISCORD_INTERACTION_JOB_CONTRACT: &str = "ait_agent_core.event_loop.DiscordInteractionJob.v1";
const DISCORD_INTERACTION_JOB_MIGRATION_STAGE: &str =
    "rust_agent_discord_interaction_job_transaction";
const REDACTED: &str = "[redacted]";

pub struct DiscordInteractionOnceRequest {
    pub path_inputs: WorkerPathInputs,
    pub worker_name: String,
    pub process_env: BTreeMap<String, String>,
    pub raw_payload: String,
    pub signature: Option<String>,
    pub signature_timestamp: Option<String>,
}

pub trait DiscordInteractionOnceJobExecutor {
    fn execute_interaction_job(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDiscordInteractionOnceJobExecutor;

impl DiscordInteractionOnceJobExecutor for DefaultDiscordInteractionOnceJobExecutor {
    fn execute_interaction_job(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_discord_interaction_job_execute_json(request)
    }
}

pub fn execute_discord_interaction_once(
    request: &DiscordInteractionOnceRequest,
) -> Result<JsonValue, WorkerDiagnostic> {
    execute_discord_interaction_once_with_job_executor(
        &DefaultDiscordInteractionOnceJobExecutor,
        request,
    )
}

pub fn execute_discord_interaction_once_with_job_executor<J>(
    executor: &J,
    request: &DiscordInteractionOnceRequest,
) -> Result<JsonValue, WorkerDiagnostic>
where
    J: DiscordInteractionOnceJobExecutor + ?Sized,
{
    let worker_name = validate_worker_name(&request.worker_name)?;
    let paths = resolve_worker_paths(&request.path_inputs)?;
    let selection = resolve_worker_selection(
        &paths,
        TransportKind::Discord,
        &worker_name,
        request.process_env.clone(),
    )?;
    let AgentWorkerRuntimeConfig::Discord(config) = selection.config else {
        return Err(WorkerDiagnostic::new(
            "discord_interaction_config_invalid",
            "The selected worker did not resolve to typed Discord configuration.",
            EXIT_INVALID_CONFIGURATION,
        ));
    };
    let public_key = config
        .public_key
        .as_ref()
        .map(|secret| secret.expose().to_string())
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "discord_public_key_missing",
                "The selected Discord interaction worker requires a public key.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("worker", worker_name.clone())
        })?;

    let ingress = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction_http_request",
        "raw_payload": request.raw_payload,
        "signature": optional_string_json(request.signature.as_deref()),
        "signature_timestamp": optional_string_json(request.signature_timestamp.as_deref()),
        "public_key": public_key,
    }))
    .map_err(|_| {
        WorkerDiagnostic::new(
            "discord_interaction_ingress_failed",
            "Rust Discord interaction ingress planning failed.",
            EXIT_INVALID_REQUEST,
        )
    })?;
    if ingress
        .get("should_handle_interaction")
        .and_then(JsonValue::as_bool)
        != Some(true)
    {
        return Err(rejected_ingress_diagnostic(&ingress));
    }
    let interaction_payload = ingress
        .get("payload")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "discord_interaction_ingress_contract_invalid",
                "Rust Discord interaction ingress omitted the parsed payload.",
                EXIT_INVALID_REQUEST,
            )
        })?;
    let interaction_token = clean_text(interaction_payload.get("token"));
    let job = executor
        .execute_interaction_job(&interaction_job_request(&config, interaction_payload))
        .map_err(|_| {
            WorkerDiagnostic::new(
                "discord_interaction_job_failed",
                "Rust Discord interaction execution failed.",
                EXIT_INVALID_REQUEST,
            )
        })?;
    validate_interaction_job_contract(&job)?;
    let response = job
        .get("response")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(JsonValue::Null);
    let mut output = json!({
        "contract": DISCORD_INTERACTION_ONCE_CONTRACT,
        "binary": "ait-agent-worker",
        "transport": "discord",
        "worker": worker_name,
        "ok": job.get("ok").cloned().unwrap_or(JsonValue::Bool(false)),
        "response": response,
        "interaction_job": public_interaction_job_outcome(&job),
        "python_worker_execution_allowed": false,
        "python_signature_verification_allowed": false,
        "python_interaction_job_allowed": false,
    });
    let mut secrets = vec![
        public_key,
        request.signature.clone().unwrap_or_default(),
        interaction_token.unwrap_or_default(),
    ];
    if let Some(bot_token) = config.bot_token.as_ref() {
        secrets.push(bot_token.expose().to_string());
    }
    redact_value(&mut output, &secrets);
    Ok(output)
}

pub(crate) fn interaction_job_request(
    config: &DiscordWorkerConfig,
    interaction_payload: JsonValue,
) -> JsonValue {
    event_job_request(config, "interaction_payload", interaction_payload)
}

pub(crate) fn message_job_request(
    config: &DiscordWorkerConfig,
    message_payload: JsonValue,
) -> JsonValue {
    event_job_request(config, "message_payload", message_payload)
}

fn event_job_request(
    config: &DiscordWorkerConfig,
    payload_key: &str,
    payload: JsonValue,
) -> JsonValue {
    let target = &config.shared.runtime_target;
    let mut request = json!({
        "state_path": config.shared.paths.sync_state_path,
        "application_id": config.application_id.expose(),
        "runtime_target": {
            "mode": target.mode.as_str(),
            "workflow_mode": target.workflow_mode.as_str(),
            "repo_name": target.repo_name,
            "repo_root": target.repo_root.to_string_lossy().to_string(),
            "remote_name": optional_string_json(target.remote_name.as_deref()),
            "server_url": optional_string_json(target.server_url.as_deref()),
        },
        "timeout_seconds": config
            .turn_timeout_seconds
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
        "local_reply": config.shared.local_reply,
    });
    request[payload_key] = payload;
    request
}

fn rejected_ingress_diagnostic(ingress: &JsonValue) -> WorkerDiagnostic {
    let state = clean_text(ingress.get("interaction_http_ingress_state"))
        .unwrap_or_else(|| "rejected".to_string());
    let code = match state.as_str() {
        "missing_signature"
        | "missing_signature_timestamp"
        | "invalid_signature_encoding"
        | "invalid_signature" => "discord_interaction_signature_invalid",
        "missing_public_key" | "invalid_public_key" => "discord_interaction_configuration_rejected",
        "empty_payload" | "invalid_json" | "invalid_payload" | "missing_type" => {
            "discord_interaction_payload_invalid"
        }
        _ => "discord_interaction_request_rejected",
    };
    let mut diagnostic = WorkerDiagnostic::new(
        code,
        "Rust Discord interaction ingress rejected the request.",
        EXIT_INVALID_REQUEST,
    )
    .with_detail("ingress_state", state);
    if let Some(status) = ingress.get("http_status").and_then(JsonValue::as_i64) {
        diagnostic = diagnostic.with_detail("http_status", status);
    }
    diagnostic
}

pub(crate) fn validate_interaction_job_contract(job: &JsonValue) -> Result<(), WorkerDiagnostic> {
    let object = job
        .as_object()
        .ok_or_else(interaction_job_contract_diagnostic)?;
    if clean_text(object.get("contract")).as_deref() != Some(DISCORD_INTERACTION_JOB_CONTRACT)
        || clean_text(object.get("migration_stage")).as_deref()
            != Some(DISCORD_INTERACTION_JOB_MIGRATION_STAGE)
        || !clean_text(object.get("interaction_job_state"))
            .is_some_and(|value| safe_label(&value, 64))
        || [
            "ok",
            "processed",
            "duplicate",
            "binding_created",
            "recorded",
        ]
        .iter()
        .any(|key| object.get(*key).and_then(JsonValue::as_bool).is_none())
        || !object
            .get("turn_ok")
            .is_some_and(|value| value.is_null() || value.is_boolean())
        || !object
            .get("sequence")
            .is_some_and(|value| value.is_null() || value.as_i64().is_some())
        || !object.get("conversation_key").is_some_and(|value| {
            value.is_null()
                || clean_text(Some(value))
                    .is_some_and(|text| !text.is_empty() && text.len() <= 4_096)
        })
        || !object
            .get("response")
            .is_some_and(|value| value.is_null() || value.is_object())
        || !object
            .get("delivery_request")
            .is_some_and(valid_discord_delivery_request)
        || !object
            .get("recovery_request")
            .is_some_and(|value| value.is_null() || value.is_object())
        || !object.get("error_kind").is_some_and(|value| {
            value.is_null() || clean_text(Some(value)).is_some_and(|text| safe_label(&text, 32))
        })
    {
        return Err(interaction_job_contract_diagnostic());
    }
    Ok(())
}

fn valid_discord_delivery_request(value: &JsonValue) -> bool {
    if value.is_null() {
        return true;
    }
    let Some(request) = value.as_object() else {
        return false;
    };
    let Some(reply_mode) = clean_text(request.get("reply_mode")) else {
        return false;
    };
    request
        .get("operations")
        .and_then(JsonValue::as_array)
        .is_some_and(|operations| {
            !operations.is_empty()
                && operations.iter().all(|operation| {
                    let kind = clean_text(operation.get("kind"));
                    match reply_mode.as_str() {
                        "interaction" => matches!(
                            kind.as_deref(),
                            Some(
                                "edit_original_response"
                                    | "send_followup"
                                    | "send_followup_attachment"
                            )
                        ),
                        "channel_message" => matches!(
                            kind.as_deref(),
                            Some("send_channel_message" | "send_channel_attachment")
                        ),
                        _ => false,
                    }
                })
        })
}

fn interaction_job_contract_diagnostic() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_interaction_job_contract_invalid",
        "Rust Discord interaction execution returned an invalid contract.",
        EXIT_INVALID_REQUEST,
    )
}

fn public_interaction_job_outcome(job: &JsonValue) -> JsonValue {
    let object = job.as_object().expect("validated interaction job object");
    let mut public = Map::from_iter([
        (
            "contract".to_string(),
            JsonValue::String(DISCORD_INTERACTION_JOB_CONTRACT.to_string()),
        ),
        (
            "migration_stage".to_string(),
            JsonValue::String(DISCORD_INTERACTION_JOB_MIGRATION_STAGE.to_string()),
        ),
    ]);
    for key in [
        "interaction_job_state",
        "ok",
        "processed",
        "duplicate",
        "conversation_key",
        "binding_created",
        "turn_ok",
        "recorded",
        "sequence",
        "error_kind",
    ] {
        public.insert(
            key.to_string(),
            object.get(key).cloned().unwrap_or(JsonValue::Null),
        );
    }
    public.insert(
        "python_state_mutation_allowed".to_string(),
        JsonValue::Bool(false),
    );
    public.insert(
        "python_ait_runtime_allowed".to_string(),
        JsonValue::Bool(false),
    );
    public.insert(
        "python_interaction_execution_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(public)
}

fn redact_value(value: &mut JsonValue, secrets: &[String]) {
    match value {
        JsonValue::String(text) => {
            for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
                *text = text.replace(secret, REDACTED);
            }
        }
        JsonValue::Array(values) => {
            for value in values {
                redact_value(value, secrets);
            }
        }
        JsonValue::Object(object) => {
            for value in object.values_mut() {
                redact_value(value, secrets);
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {}
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    match value {
        JsonValue::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        JsonValue::Number(_) | JsonValue::Bool(_) => Some(value.to_string()),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}

fn safe_label(value: &str, max_length: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

#[cfg(test)]
mod tests;
