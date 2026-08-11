use std::collections::BTreeMap;

use ait_agent_core::{
    agent_slack_command_http_transaction_plan_json, agent_slack_command_job_execute_json,
    AgentWorkerRuntimeConfig, SlackWorkerConfig, TransportKind,
};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::diagnostic::{WorkerDiagnostic, EXIT_INVALID_CONFIGURATION, EXIT_INVALID_REQUEST};
use crate::paths::{resolve_worker_paths, WorkerPathInputs};
use crate::run::{resolve_worker_selection, validate_worker_name};

pub const SLACK_COMMAND_ONCE_CONTRACT: &str = "ait.agent.worker.slack-command-once.v1";
const SLACK_COMMAND_JOB_CONTRACT: &str = "ait_agent_core.event_loop.SlackCommandJob.v1";
const SLACK_COMMAND_JOB_MIGRATION_STAGE: &str = "rust_agent_slack_command_job_transaction";
const REDACTED: &str = "[redacted]";

pub struct SlackCommandOnceRequest {
    pub path_inputs: WorkerPathInputs,
    pub worker_name: String,
    pub process_env: BTreeMap<String, String>,
    pub raw_payload: String,
    pub signature: Option<String>,
    pub signature_timestamp: Option<String>,
    pub now_unix_seconds: Option<i64>,
}

pub trait SlackCommandOnceJobExecutor {
    fn execute_command_job(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackCommandOnceJobExecutor;

impl SlackCommandOnceJobExecutor for DefaultSlackCommandOnceJobExecutor {
    fn execute_command_job(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_slack_command_job_execute_json(request)
    }
}

pub fn execute_slack_command_once(
    request: &SlackCommandOnceRequest,
) -> Result<JsonValue, WorkerDiagnostic> {
    execute_slack_command_once_with_job_executor(&DefaultSlackCommandOnceJobExecutor, request)
}

pub fn execute_slack_command_once_with_job_executor<J>(
    executor: &J,
    request: &SlackCommandOnceRequest,
) -> Result<JsonValue, WorkerDiagnostic>
where
    J: SlackCommandOnceJobExecutor + ?Sized,
{
    let worker_name = validate_worker_name(&request.worker_name)?;
    let paths = resolve_worker_paths(&request.path_inputs)?;
    let selection = resolve_worker_selection(
        &paths,
        TransportKind::Slack,
        &worker_name,
        request.process_env.clone(),
    )?;
    let AgentWorkerRuntimeConfig::Slack(config) = selection.config else {
        return Err(WorkerDiagnostic::new(
            "slack_command_config_invalid",
            "The selected worker did not resolve to typed Slack configuration.",
            EXIT_INVALID_CONFIGURATION,
        ));
    };
    let signing_secret = config
        .signing_secret
        .as_ref()
        .map(|secret| secret.expose().to_string())
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "slack_signing_secret_missing",
                "The selected Slack command worker requires a signing secret.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("worker", worker_name.clone())
        })?;

    let ingress_request = command_ingress_request(request, &config, &signing_secret);
    let transaction =
        agent_slack_command_http_transaction_plan_json(&ingress_request).map_err(|_| {
            WorkerDiagnostic::new(
                "slack_command_ingress_failed",
                "Rust Slack command ingress planning failed.",
                EXIT_INVALID_REQUEST,
            )
        })?;
    if transaction
        .get("should_handle_command")
        .and_then(JsonValue::as_bool)
        != Some(true)
    {
        return Err(rejected_ingress_diagnostic(&transaction));
    }

    let command_payload = transaction
        .get("http_ingress_plan")
        .and_then(|value| value.get("command_payload"))
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "slack_command_ingress_contract_invalid",
                "Rust Slack command ingress omitted the parsed command payload.",
                EXIT_INVALID_REQUEST,
            )
        })?;
    let mut response = transaction
        .get("response")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "slack_command_ingress_contract_invalid",
                "Rust Slack command ingress omitted its command response.",
                EXIT_INVALID_REQUEST,
            )
        })?;
    let should_submit_turn = transaction
        .get("should_submit_turn")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);

    let command_job = if should_submit_turn {
        let job_request = command_job_request(&config, command_payload.clone());
        let job = executor.execute_command_job(&job_request).map_err(|_| {
            WorkerDiagnostic::new(
                "slack_command_job_failed",
                "Rust Slack command execution failed.",
                EXIT_INVALID_REQUEST,
            )
        })?;
        validate_command_job_contract(&job)?;
        if job.get("duplicate").and_then(JsonValue::as_bool) == Some(true) {
            response = json!({
                "response_type": "ephemeral",
                "text": "Duplicate Slack command ignored.",
            });
        } else if clean_text(job.get("command_job_state")).as_deref() == Some("ignored") {
            response = json!({
                "response_type": "ephemeral",
                "text": "Slack command was not accepted.",
            });
        }
        public_command_job_outcome(&job)
    } else {
        JsonValue::Null
    };

    let job_ok = command_job.get("ok").and_then(JsonValue::as_bool);
    let mut output = json!({
        "contract": SLACK_COMMAND_ONCE_CONTRACT,
        "binary": "ait-agent-worker",
        "transport": "slack",
        "worker": worker_name,
        "ok": job_ok.unwrap_or_else(|| transaction.get("ok").and_then(JsonValue::as_bool).unwrap_or(true)),
        "accepted": transaction.get("accepted").cloned().unwrap_or(JsonValue::Bool(true)),
        "command_job_dispatched": should_submit_turn,
        "response": response,
        "command_job": command_job,
        "python_worker_execution_allowed": false,
        "python_signature_verification_allowed": false,
        "python_command_job_allowed": false,
    });
    let response_url = clean_text(command_payload.get("response_url"));
    redact_value(
        &mut output,
        &[signing_secret, response_url.unwrap_or_default()],
    );
    Ok(output)
}

fn command_ingress_request(
    request: &SlackCommandOnceRequest,
    config: &SlackWorkerConfig,
    signing_secret: &str,
) -> JsonValue {
    let mut ingress = json!({
        "request_path": config.command_path,
        "command_path": config.command_path,
        "raw_payload": request.raw_payload,
        "signature": optional_string_json(request.signature.as_deref()),
        "signature_timestamp": optional_string_json(request.signature_timestamp.as_deref()),
        "signing_secret": signing_secret,
        "repo_name": config.shared.runtime_target.repo_name,
        "defer_replies": true,
        "ack_text": config.ack_text,
        "response_type": config.response_type,
    });
    if let Some(now_unix_seconds) = request.now_unix_seconds {
        ingress["now_unix_seconds"] = JsonValue::from(now_unix_seconds);
    }
    ingress
}

pub(crate) fn command_job_request(
    config: &SlackWorkerConfig,
    command_payload: JsonValue,
) -> JsonValue {
    let target = &config.shared.runtime_target;
    json!({
        "command_payload": command_payload,
        "state_path": config.shared.paths.sync_state_path,
        "runtime_target": {
            "mode": target.mode.as_str(),
            "workflow_mode": target.workflow_mode.as_str(),
            "repo_name": target.repo_name,
            "repo_root": target.repo_root.to_string_lossy().to_string(),
            "remote_name": optional_string_json(target.remote_name.as_deref()),
            "server_url": optional_string_json(target.server_url.as_deref()),
        },
        "ack_text": config.ack_text,
        "response_type": config.response_type,
        "defer_replies": true,
        "timeout_seconds": config.shared.request_timeout_seconds.map(JsonValue::from).unwrap_or(JsonValue::Null),
    })
}

fn rejected_ingress_diagnostic(transaction: &JsonValue) -> WorkerDiagnostic {
    let error_kind = clean_text(transaction.get("error_kind")).unwrap_or_else(|| "request".into());
    let code = match error_kind.as_str() {
        "invalid_signature" => "slack_command_signature_invalid",
        "config_error" => "slack_command_configuration_rejected",
        "invalid_payload" | "invalid_command_payload" => "slack_command_payload_invalid",
        "not_found" => "slack_command_path_not_found",
        _ => "slack_command_request_rejected",
    };
    let mut diagnostic = WorkerDiagnostic::new(
        code,
        "Rust Slack command ingress rejected the request.",
        EXIT_INVALID_REQUEST,
    )
    .with_detail("error_kind", error_kind);
    if let Some(status) = transaction.get("http_status").and_then(JsonValue::as_i64) {
        diagnostic = diagnostic.with_detail("http_status", status);
    }
    diagnostic
}

pub(crate) fn validate_command_job_contract(job: &JsonValue) -> Result<(), WorkerDiagnostic> {
    let object = job
        .as_object()
        .ok_or_else(command_job_contract_diagnostic)?;
    if clean_text(object.get("contract")).as_deref() != Some(SLACK_COMMAND_JOB_CONTRACT)
        || clean_text(object.get("migration_stage")).as_deref()
            != Some(SLACK_COMMAND_JOB_MIGRATION_STAGE)
        || !clean_text(object.get("command_job_state")).is_some_and(|value| safe_label(&value, 64))
        || [
            "ok",
            "processed",
            "duplicate",
            "binding_created",
            "delivery_attempted",
            "delivered",
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
        || !object.get("error_kind").is_some_and(|value| {
            value.is_null() || clean_text(Some(value)).is_some_and(|text| safe_label(&text, 32))
        })
    {
        return Err(command_job_contract_diagnostic());
    }
    Ok(())
}

fn command_job_contract_diagnostic() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_command_job_contract_invalid",
        "Rust Slack command execution returned an invalid contract.",
        EXIT_INVALID_REQUEST,
    )
}

fn public_command_job_outcome(job: &JsonValue) -> JsonValue {
    let object = job.as_object().expect("validated command job object");
    let mut public = Map::from_iter([
        (
            "contract".to_string(),
            JsonValue::String(SLACK_COMMAND_JOB_CONTRACT.to_string()),
        ),
        (
            "migration_stage".to_string(),
            JsonValue::String(SLACK_COMMAND_JOB_MIGRATION_STAGE.to_string()),
        ),
    ]);
    for key in [
        "command_job_state",
        "ok",
        "processed",
        "duplicate",
        "conversation_key",
        "binding_created",
        "turn_ok",
        "delivery_attempted",
        "delivered",
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
        "python_response_url_delivery_allowed".to_string(),
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
