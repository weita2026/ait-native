use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

use ait_agent_core::{
    agent_discord_ingress_runtime_plan_json, agent_discord_interaction_job_execute_json,
    agent_discord_reply_delivery_execution_plan_json, agent_discord_rest_delivery_execute_json,
    agent_runtime_backend_execute_json, AgentRuntimeBindingStore, AgentWorkerRuntimeConfig,
    DiscordWorkerConfig,
};
use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};

use crate::discord_interaction_once::{interaction_job_request, validate_interaction_job_contract};
use crate::{
    run_worker_host, BoundedWorkerJobExecutor, WorkerDiagnostic, WorkerHttpCompletion,
    WorkerHttpDispatch, WorkerHttpHandler, WorkerHttpHostConfig, WorkerHttpHostRuntime,
    WorkerHttpRequest, WorkerHttpResponse, WorkerJobExecutorConfig, WorkerRunContext,
    EXIT_INVALID_CONFIGURATION, EXIT_RUNTIME_UNAVAILABLE,
};

const DEFAULT_DISCORD_MAX_INFLIGHT_JOBS: usize = 4;
const DISCORD_HTTP_REQUEST_DEADLINE: Duration = Duration::from_secs(3);
const DISCORD_INTERACTION_JOB_KIND: &str = "discord.interaction_job";
const DISCORD_DELIVERY_SWEEP_JOB_KIND: &str = "discord.delivery_sweep";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const DISCORD_REST_DELIVERY_CONTRACT: &str =
    "ait_agent_core.event_loop.DiscordRestDeliveryExecution.v1";
const DISCORD_REST_DELIVERY_MIGRATION_STAGE: &str = "rust_agent_discord_rest_delivery_execution";
const DISCORD_BACKGROUND_FAILURE_TEXT: &str =
    "The Discord command could not be completed. Please try again.";
const DISCORD_DELIVERED_SEQUENCE_KEY: &str = "discord_live_delivered_sequences";
const DISCORD_LAST_DELIVERED_SEQUENCE_KEY: &str = "discord_last_delivered_sequence";
const DISCORD_DELIVERED_SEQUENCE_LIMIT: i64 = 128;
const DISCORD_SWEEP_BINDING_LIMIT: usize = 100;
const DISCORD_PENDING_DELIVERY_KEY: &str = "discord_pending_delivery";

pub trait DiscordHttpInteractionJobExecutor: Clone + Send + Sync + 'static {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn execute_state(
        &self,
        path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String>;

    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDiscordHttpInteractionJobExecutor;

impl DiscordHttpInteractionJobExecutor for DefaultDiscordHttpInteractionJobExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_discord_interaction_job_execute_json(request)
    }

    fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_discord_rest_delivery_execute_json(request)
    }

    fn execute_state(
        &self,
        path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String> {
        AgentRuntimeBindingStore::new(path).execute(operation, request)
    }

    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_runtime_backend_execute_json(request)
    }
}

pub struct DiscordWorkerHttpHandler<E = DefaultDiscordHttpInteractionJobExecutor> {
    config: DiscordWorkerConfig,
    public_key: String,
    interaction_executor: E,
    jobs: BoundedWorkerJobExecutor<WorkerHttpResponse>,
}

impl<E> DiscordWorkerHttpHandler<E>
where
    E: DiscordHttpInteractionJobExecutor,
{
    pub fn new(
        config: &DiscordWorkerConfig,
        interaction_executor: E,
        max_inflight_jobs: usize,
    ) -> Result<Self, WorkerDiagnostic> {
        let public_key = config
            .public_key
            .as_ref()
            .map(|key| key.expose().trim().to_string())
            .filter(|key| !key.is_empty())
            .ok_or_else(discord_public_key_missing)?;
        let mut jobs = BoundedWorkerJobExecutor::new(WorkerJobExecutorConfig {
            max_inflight: max_inflight_jobs,
        })?;
        if config.bot_token.is_some() {
            let sweep_config = config.clone();
            let sweep_executor = interaction_executor.clone();
            jobs.submit(DISCORD_DELIVERY_SWEEP_JOB_KIND, move || {
                execute_discord_delivery_sweep(&sweep_executor, &sweep_config)
            })?;
        }
        Ok(Self {
            config: config.clone(),
            public_key,
            interaction_executor,
            jobs,
        })
    }

    fn ingress_request(&self, request: WorkerHttpRequest) -> Result<JsonValue, WorkerHttpResponse> {
        if request.path != self.config.interaction_path {
            return Err(WorkerHttpResponse::new(404, Vec::new()));
        }
        let raw_payload = String::from_utf8(request.body)
            .map_err(|_| discord_public_error(400, "Discord interaction payload must be UTF-8."))?;
        Ok(json!({
            "stage": "interaction_http_request",
            "raw_payload": raw_payload,
            "signature": optional_string_json(request.headers.get("x-signature-ed25519")),
            "signature_timestamp": optional_string_json(
                request.headers.get("x-signature-timestamp")
            ),
            "public_key": self.public_key,
        }))
    }
}

impl<E> WorkerHttpHandler for DiscordWorkerHttpHandler<E>
where
    E: DiscordHttpInteractionJobExecutor,
{
    fn handle(
        &mut self,
        request: WorkerHttpRequest,
    ) -> Result<WorkerHttpDispatch, WorkerDiagnostic> {
        let request = match self.ingress_request(request) {
            Ok(request) => request,
            Err(response) => return Ok(WorkerHttpDispatch::Immediate(response)),
        };
        let ingress = agent_discord_ingress_runtime_plan_json(&request)
            .map_err(|_| discord_ingress_failure())?;
        if ingress
            .get("should_handle_interaction")
            .and_then(JsonValue::as_bool)
            != Some(true)
        {
            return discord_ingress_response(&ingress).map(WorkerHttpDispatch::Immediate);
        }

        let interaction_payload = ingress
            .get("payload")
            .filter(|payload| payload.is_object())
            .cloned()
            .ok_or_else(discord_ingress_contract_failure)?;
        let interaction_plan = agent_discord_ingress_runtime_plan_json(&json!({
            "stage": "interaction",
            "payload": interaction_payload,
            "config_application_id": self.config.application_id.expose(),
            "defer_replies": true,
        }))
        .map_err(|_| discord_ingress_failure())?;
        let response = discord_interaction_plan_response(&interaction_plan)?;
        let should_execute_job = interaction_plan
            .get("should_submit_turn")
            .and_then(JsonValue::as_bool)
            == Some(true)
            || interaction_plan
                .get("fresh_topic")
                .and_then(JsonValue::as_bool)
                == Some(true);
        if !should_execute_job {
            return Ok(WorkerHttpDispatch::Immediate(response));
        }

        let job_request = interaction_job_request(&self.config, interaction_payload.clone());
        let config = self.config.clone();
        let executor = self.interaction_executor.clone();
        match self.jobs.submit(DISCORD_INTERACTION_JOB_KIND, move || {
            execute_discord_background_interaction(
                &executor,
                &config,
                &interaction_payload,
                &job_request,
            )
        }) {
            Ok(_) => Ok(WorkerHttpDispatch::Immediate(response)),
            Err(error)
                if matches!(
                    error.code,
                    "worker_job_capacity_exhausted" | "worker_job_executor_closed"
                ) =>
            {
                Ok(WorkerHttpDispatch::Immediate(discord_public_error(
                    503,
                    "Discord interaction worker is busy.",
                )))
            }
            Err(error) => Err(error),
        }
    }

    fn poll_completed(&mut self) -> Vec<WorkerHttpCompletion> {
        self.jobs
            .poll_completed()
            .into_iter()
            .map(|completion| WorkerHttpCompletion {
                job_id: completion.job_id,
                result: completion.result,
            })
            .collect()
    }

    fn close_admission(&mut self) {
        self.jobs.close_admission();
    }

    fn inflight_work_count(&self) -> usize {
        self.jobs.inflight_count()
    }

    fn finish_shutdown(&mut self) -> Result<(), WorkerDiagnostic> {
        if self.jobs.inflight_count() == 0 {
            Ok(())
        } else {
            Err(WorkerDiagnostic::new(
                "discord_worker_jobs_still_inflight",
                "Rust Discord interaction jobs remain in flight during graceful shutdown.",
                EXIT_RUNTIME_UNAVAILABLE,
            ))
        }
    }

    fn force_shutdown(&mut self) -> Result<(), WorkerDiagnostic> {
        self.jobs.close_admission();
        self.jobs.force_detach();
        Ok(())
    }
}

pub fn run_discord_transport(context: &WorkerRunContext) -> Result<(), WorkerDiagnostic> {
    let AgentWorkerRuntimeConfig::Discord(config) = &context.config else {
        return Err(WorkerDiagnostic::new(
            "discord_worker_config_mismatch",
            "The Rust Discord runner received a non-Discord worker configuration.",
            EXIT_INVALID_CONFIGURATION,
        ));
    };
    if config.bot_token.is_some() {
        return crate::discord_gateway::run_discord_gateway_transport(context, config);
    }
    let bind_addr = resolve_discord_bind_addr(config)?;
    let handler = DiscordWorkerHttpHandler::new(
        config,
        DefaultDiscordHttpInteractionJobExecutor,
        DEFAULT_DISCORD_MAX_INFLIGHT_JOBS,
    )?;
    let mut runtime = WorkerHttpHostRuntime::new(
        WorkerHttpHostConfig {
            bind_addr,
            expected_method: "POST".to_string(),
            expected_path: config.interaction_path.clone(),
            enforce_expected_path: false,
            request_timeout: DISCORD_HTTP_REQUEST_DEADLINE,
            ..WorkerHttpHostConfig::default()
        },
        handler,
    );
    run_worker_host(context, &mut runtime)
}

fn resolve_discord_bind_addr(config: &DiscordWorkerConfig) -> Result<SocketAddr, WorkerDiagnostic> {
    let port = u16::try_from(config.bind_port)
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "discord_worker_bind_port_invalid",
                "The Rust Discord worker bind port must be between 1 and 65535.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_port", config.bind_port)
        })?;
    (config.bind_host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| {
            WorkerDiagnostic::new(
                "discord_worker_bind_address_invalid",
                format!(
                    "Cannot resolve the Rust Discord worker bind host `{}`: {error}",
                    config.bind_host
                ),
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_host", config.bind_host.clone())
        })?
        .next()
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "discord_worker_bind_address_invalid",
                "The Rust Discord worker bind host did not resolve to an address.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_host", config.bind_host.clone())
        })
}

fn discord_ingress_response(ingress: &JsonValue) -> Result<WorkerHttpResponse, WorkerDiagnostic> {
    let object = ingress
        .as_object()
        .ok_or_else(discord_ingress_contract_failure)?;
    let status_code = object
        .get("http_status")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| (100..=599).contains(value))
        .ok_or_else(discord_ingress_contract_failure)?;
    let write_json_response = object
        .get("write_json_response")
        .and_then(JsonValue::as_bool)
        .ok_or_else(discord_ingress_contract_failure)?;
    if !write_json_response {
        return Ok(WorkerHttpResponse::new(status_code, Vec::new()));
    }
    let response = object
        .get("response")
        .ok_or_else(discord_ingress_contract_failure)?;
    json_response(
        status_code,
        response,
        "Failed to encode Discord ingress response",
    )
}

fn discord_interaction_plan_response(
    plan: &JsonValue,
) -> Result<WorkerHttpResponse, WorkerDiagnostic> {
    let response = plan
        .get("response")
        .filter(|response| response.is_object())
        .ok_or_else(discord_interaction_response_failure)?;
    if !matches!(
        response.get("type").and_then(JsonValue::as_i64),
        Some(1 | 4 | 5)
    ) {
        return Err(discord_interaction_response_failure());
    }
    json_response(
        200,
        response,
        "Failed to encode Discord interaction response",
    )
}

fn execute_discord_background_interaction<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    interaction_payload: &JsonValue,
    job_request: &JsonValue,
) -> Result<WorkerHttpResponse, WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let outcome = executor
        .execute(job_request)
        .map_err(|_| discord_interaction_job_failure())?;
    validate_interaction_job_contract(&outcome)?;
    let channel_id = clean_text(interaction_payload.get("channel_id"))
        .ok_or_else(discord_delivery_contract_failure)?;
    if let Some(delivery_request) = outcome
        .get("delivery_request")
        .filter(|request| delivery_request_has_operations(request))
    {
        let delivery_request = delivery_request_with_outcome_sequence(delivery_request, &outcome)?;
        execute_discord_delivery_lifecycle(
            executor,
            config,
            &delivery_request,
            DiscordDeliveryTarget::Interaction(interaction_payload),
            &channel_id,
        )?;
        return Ok(WorkerHttpResponse::new(204, Vec::new()));
    }
    if let Some(recovery_request) = outcome
        .get("recovery_request")
        .filter(|request| request.is_object())
    {
        execute_discord_interaction_recovery(
            executor,
            config,
            interaction_payload,
            recovery_request,
        )?;
        return Ok(WorkerHttpResponse::new(204, Vec::new()));
    }
    let delivery_request = fallback_interaction_delivery_request(&outcome);
    execute_discord_delivery_lifecycle(
        executor,
        config,
        &delivery_request,
        DiscordDeliveryTarget::Interaction(interaction_payload),
        &channel_id,
    )?;
    Ok(WorkerHttpResponse::new(204, Vec::new()))
}

pub(crate) fn execute_discord_background_message<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    message_payload: &JsonValue,
    job_request: &JsonValue,
) -> Result<(), WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let outcome = executor
        .execute(job_request)
        .map_err(|_| discord_interaction_job_failure())?;
    validate_interaction_job_contract(&outcome)?;
    let channel_id = clean_text(message_payload.get("channel_id"))
        .ok_or_else(discord_delivery_contract_failure)?;
    if let Some(delivery_request) = outcome
        .get("delivery_request")
        .filter(|request| delivery_request_has_operations(request))
    {
        let delivery_request = delivery_request_with_outcome_sequence(delivery_request, &outcome)?;
        execute_discord_delivery_lifecycle(
            executor,
            config,
            &delivery_request,
            DiscordDeliveryTarget::Channel(&channel_id),
            &channel_id,
        )?;
        return Ok(());
    }
    if let Some(recovery_request) = outcome
        .get("recovery_request")
        .filter(|request| request.is_object())
    {
        execute_discord_channel_recovery(executor, config, &channel_id, recovery_request)?;
        return Ok(());
    }
    if outcome.get("ok").and_then(JsonValue::as_bool) == Some(true) {
        return Ok(());
    }
    let fallback = fallback_channel_delivery_request(&outcome);
    execute_discord_delivery_lifecycle(
        executor,
        config,
        &fallback,
        DiscordDeliveryTarget::Channel(&channel_id),
        &channel_id,
    )?;
    Ok(())
}

fn delivery_request_has_operations(request: &JsonValue) -> bool {
    request
        .get("operations")
        .and_then(JsonValue::as_array)
        .is_some_and(|operations| !operations.is_empty())
}

fn delivery_request_with_outcome_sequence(
    request: &JsonValue,
    outcome: &JsonValue,
) -> Result<JsonValue, WorkerDiagnostic> {
    let mut request = request
        .as_object()
        .cloned()
        .ok_or_else(discord_delivery_contract_failure)?;
    let sequence = json_i64(outcome.get("sequence")).unwrap_or(0).max(0);
    if sequence > 0 {
        request.insert("assistant_sequence".to_string(), JsonValue::from(sequence));
        request.insert("through_sequence".to_string(), JsonValue::from(sequence));
    } else {
        request
            .entry("assistant_sequence".to_string())
            .or_insert_with(|| JsonValue::from(0));
        request
            .entry("through_sequence".to_string())
            .or_insert_with(|| JsonValue::from(0));
    }
    Ok(JsonValue::Object(request))
}

fn fallback_interaction_delivery_request(outcome: &JsonValue) -> JsonValue {
    let sequence = json_i64(outcome.get("sequence")).unwrap_or(0).max(0);
    let reply_text = outcome
        .get("response")
        .filter(|_| outcome.get("ok").and_then(JsonValue::as_bool) == Some(true))
        .and_then(|response| response.get("data"))
        .and_then(|data| clean_text(data.get("content")))
        .unwrap_or_else(|| DISCORD_BACKGROUND_FAILURE_TEXT.to_string());
    json!({
        "reply_mode": "interaction",
        "assistant_sequence": sequence,
        "through_sequence": sequence,
        "reply_text": reply_text,
        "attachments": [],
        "operations": [{
            "kind": "edit_original_response",
            "text": reply_text,
        }],
        "operation_count": 1,
    })
}

fn fallback_channel_delivery_request(outcome: &JsonValue) -> JsonValue {
    let sequence = json_i64(outcome.get("sequence")).unwrap_or(0).max(0);
    json!({
        "reply_mode": "channel_message",
        "assistant_sequence": sequence,
        "through_sequence": sequence,
        "reply_text": DISCORD_BACKGROUND_FAILURE_TEXT,
        "attachments": [],
        "operations": [{
            "kind": "send_channel_message",
            "text": DISCORD_BACKGROUND_FAILURE_TEXT,
        }],
        "operation_count": 1,
    })
}

#[derive(Clone, Copy)]
enum DiscordDeliveryTarget<'a> {
    Interaction(&'a JsonValue),
    Channel(&'a str),
}

fn execute_discord_delivery_lifecycle<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    delivery_request: &JsonValue,
    target: DiscordDeliveryTarget<'_>,
    channel_id: &str,
) -> Result<JsonValue, WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let request = delivery_request
        .as_object()
        .cloned()
        .ok_or_else(discord_delivery_contract_failure)?;
    let operations = request
        .get("operations")
        .and_then(JsonValue::as_array)
        .filter(|operations| !operations.is_empty())
        .cloned()
        .ok_or_else(discord_delivery_contract_failure)?;
    let assistant_sequence = json_i64(request.get("assistant_sequence"))
        .unwrap_or(0)
        .max(0);
    let pending_delivery = discord_channel_recovery_request(&JsonValue::Object(request.clone()))?;
    if assistant_sequence > 0 {
        persist_discord_pending_delivery(executor, config, channel_id, &pending_delivery)?;
    }
    if assistant_sequence > 0
        && discord_sequence_already_delivered(executor, config, channel_id, assistant_sequence)?
    {
        clear_discord_pending_delivery(executor, config, channel_id)?;
        return Ok(json!({
            "ok": true,
            "delivered": true,
            "duplicate": true,
            "assistant_sequence": assistant_sequence,
            "through_sequence": json_i64(request.get("through_sequence"))
                .unwrap_or(assistant_sequence)
                .max(assistant_sequence),
            "message_ids": [],
        }));
    }

    let operation_results =
        execute_discord_delivery_operations(executor, config, target, operations.as_slice())?;
    let mut callback = request;
    callback.insert(
        "operation_count".to_string(),
        JsonValue::from(operation_results.len() as i64),
    );
    callback.insert(
        "operation_results".to_string(),
        JsonValue::Array(operation_results),
    );
    callback.insert(
        "post_operation_results".to_string(),
        JsonValue::Array(Vec::new()),
    );
    let mut plan = discord_delivery_result_plan(&callback)?;
    if plan
        .get("requires_post_operations")
        .and_then(JsonValue::as_bool)
        == Some(true)
    {
        let post_operations = plan
            .get("result")
            .and_then(|result| result.get("post_operations"))
            .and_then(JsonValue::as_array)
            .filter(|operations| !operations.is_empty())
            .cloned()
            .ok_or_else(discord_delivery_contract_failure)?;
        let post_results = execute_discord_delivery_operations(
            executor,
            config,
            target,
            post_operations.as_slice(),
        )?;
        callback.insert(
            "post_operation_results".to_string(),
            JsonValue::Array(post_results),
        );
        plan = discord_delivery_result_plan(&callback)?;
    }
    let result = plan
        .get("result")
        .filter(|result| result.is_object())
        .cloned()
        .ok_or_else(discord_delivery_contract_failure)?;
    if result.get("delivered").and_then(JsonValue::as_bool) != Some(true)
        || result.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || plan.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || plan
            .get("requires_post_operations")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(discord_delivery_failure());
    }
    record_discord_delivery(executor, config, channel_id, &result)?;
    if assistant_sequence > 0 {
        clear_discord_pending_delivery(executor, config, channel_id)?;
    }
    Ok(result)
}

fn discord_channel_recovery_request(
    delivery_request: &JsonValue,
) -> Result<JsonValue, WorkerDiagnostic> {
    let request = delivery_request
        .as_object()
        .ok_or_else(discord_delivery_contract_failure)?;
    let operations = request
        .get("operations")
        .and_then(JsonValue::as_array)
        .filter(|operations| !operations.is_empty())
        .ok_or_else(discord_delivery_contract_failure)?;
    let mut channel_operations = Vec::with_capacity(operations.len());
    for operation in operations {
        let mut operation = operation
            .as_object()
            .cloned()
            .ok_or_else(discord_delivery_contract_failure)?;
        let kind =
            clean_text(operation.get("kind")).ok_or_else(discord_delivery_contract_failure)?;
        let channel_kind = match kind.as_str() {
            "edit_original_response" | "send_followup" | "send_channel_message" => {
                "send_channel_message"
            }
            "send_followup_attachment" | "send_channel_attachment" => "send_channel_attachment",
            _ => return Err(discord_delivery_contract_failure()),
        };
        for key in [
            "application_id",
            "interaction_token",
            "channel_id",
            "api_base_url",
            "bot_token",
            "http_user_agent",
            "timeout_seconds",
            "repo_root",
        ] {
            operation.remove(key);
        }
        operation.insert(
            "kind".to_string(),
            JsonValue::String(channel_kind.to_string()),
        );
        channel_operations.push(JsonValue::Object(operation));
    }
    Ok(json!({
        "reply_mode": "channel_message",
        "assistant_sequence": request
            .get("assistant_sequence")
            .cloned()
            .unwrap_or_else(|| JsonValue::from(0)),
        "through_sequence": request
            .get("through_sequence")
            .cloned()
            .unwrap_or_else(|| JsonValue::from(0)),
        "reply_text": request.get("reply_text").cloned().unwrap_or(JsonValue::Null),
        "attachments": request
            .get("attachments")
            .cloned()
            .unwrap_or_else(|| JsonValue::Array(Vec::new())),
        "operations": channel_operations,
        "operation_count": channel_operations.len(),
    }))
}

fn persist_discord_pending_delivery<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    channel_id: &str,
    delivery_request: &JsonValue,
) -> Result<(), WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let stored = executor
        .execute_state(
            &config.shared.paths.sync_state_path,
            "patch_binding",
            &json!({
                "transport": "discord",
                "surface_id": channel_id,
                "updates": {DISCORD_PENDING_DELIVERY_KEY: delivery_request},
            }),
        )
        .map_err(|_| discord_delivery_state_failure())?;
    stored
        .is_object()
        .then_some(())
        .ok_or_else(discord_delivery_state_failure)
}

fn clear_discord_pending_delivery<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    channel_id: &str,
) -> Result<(), WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let stored = executor
        .execute_state(
            &config.shared.paths.sync_state_path,
            "patch_binding",
            &json!({
                "transport": "discord",
                "surface_id": channel_id,
                "updates": {DISCORD_PENDING_DELIVERY_KEY: JsonValue::Null},
            }),
        )
        .map_err(|_| discord_delivery_state_failure())?;
    stored
        .is_object()
        .then_some(())
        .ok_or_else(discord_delivery_state_failure)
}

fn execute_discord_delivery_operations<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    target: DiscordDeliveryTarget<'_>,
    operations: &[JsonValue],
) -> Result<Vec<JsonValue>, WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let mut results = Vec::with_capacity(operations.len());
    for operation in operations {
        let operation = trusted_discord_delivery_operation(config, target, operation)?;
        let expected_kind =
            clean_text(operation.get("kind")).ok_or_else(discord_delivery_contract_failure)?;
        let request = discord_delivery_execution_request(config, operation.clone());
        match executor.execute_delivery(&request) {
            Ok(result) => {
                validate_discord_delivery_result_contract(&result, &expected_kind)?;
                results.push(result);
            }
            Err(_) => results.push(discord_delivery_executor_failure_result(
                &operation,
                &expected_kind,
            )),
        }
    }
    Ok(results)
}

fn trusted_discord_delivery_operation(
    config: &DiscordWorkerConfig,
    target: DiscordDeliveryTarget<'_>,
    operation: &JsonValue,
) -> Result<JsonValue, WorkerDiagnostic> {
    let mut operation = operation
        .as_object()
        .cloned()
        .ok_or_else(discord_delivery_contract_failure)?;
    let kind = clean_text(operation.get("kind")).ok_or_else(discord_delivery_contract_failure)?;
    for key in [
        "api_base_url",
        "bot_token",
        "http_user_agent",
        "timeout_seconds",
        "repo_root",
    ] {
        operation.remove(key);
    }
    match target {
        DiscordDeliveryTarget::Interaction(interaction_payload) => {
            if !matches!(
                kind.as_str(),
                "edit_original_response" | "send_followup" | "send_followup_attachment"
            ) {
                return Err(discord_delivery_contract_failure());
            }
            let interaction_token = clean_text(interaction_payload.get("token"))
                .ok_or_else(discord_delivery_contract_failure)?;
            operation.remove("channel_id");
            operation.insert(
                "application_id".to_string(),
                JsonValue::String(config.application_id.expose().to_string()),
            );
            operation.insert(
                "interaction_token".to_string(),
                JsonValue::String(interaction_token),
            );
        }
        DiscordDeliveryTarget::Channel(channel_id) => {
            if !matches!(
                kind.as_str(),
                "send_channel_message" | "send_channel_attachment" | "list_channel_messages"
            ) {
                return Err(discord_delivery_contract_failure());
            }
            operation.remove("application_id");
            operation.remove("interaction_token");
            operation.insert(
                "channel_id".to_string(),
                JsonValue::String(channel_id.to_string()),
            );
        }
    }
    Ok(JsonValue::Object(operation))
}

fn discord_delivery_executor_failure_result(operation: &JsonValue, kind: &str) -> JsonValue {
    json!({
        "kind": kind,
        "ok": false,
        "delivered": false,
        "completed": false,
        "delivery_execution_state": "executor_failed",
        "attachment_index": operation
            .get("attachment_index")
            .cloned()
            .unwrap_or(JsonValue::Null),
        "attachment": operation
            .get("attachment")
            .cloned()
            .unwrap_or(JsonValue::Null),
        "message_ids": [],
        "error": "The Rust Discord REST delivery executor failed.",
    })
}

fn discord_delivery_result_plan(
    callback: &ait_core::json_support::JsonMap<String, JsonValue>,
) -> Result<JsonValue, WorkerDiagnostic> {
    agent_discord_reply_delivery_execution_plan_json(&json!({
        "stage": "result",
        "callback_result": JsonValue::Object(callback.clone()),
    }))
    .map_err(|_| discord_delivery_contract_failure())
}

fn discord_sequence_already_delivered<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    channel_id: &str,
    assistant_sequence: i64,
) -> Result<bool, WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let result = executor
        .execute_state(
            &config.shared.paths.sync_state_path,
            "has_recent_value",
            &json!({
                "transport": "discord",
                "surface_id": channel_id,
                "value": assistant_sequence,
                "recent_key": DISCORD_DELIVERED_SEQUENCE_KEY,
            }),
        )
        .map_err(|_| discord_delivery_state_failure())?;
    result.as_bool().ok_or_else(discord_delivery_state_failure)
}

fn record_discord_delivery<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    channel_id: &str,
    result: &JsonValue,
) -> Result<(), WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let assistant_sequence = json_i64(result.get("assistant_sequence"))
        .unwrap_or(0)
        .max(0);
    if assistant_sequence == 0 {
        return Ok(());
    }
    let through_sequence = json_i64(result.get("through_sequence"))
        .unwrap_or(assistant_sequence)
        .max(assistant_sequence);
    let stored = executor
        .execute_state(
            &config.shared.paths.sync_state_path,
            "remember_recent_value",
            &json!({
                "transport": "discord",
                "surface_id": channel_id,
                "value": assistant_sequence,
                "recent_key": DISCORD_DELIVERED_SEQUENCE_KEY,
                "last_value_key": DISCORD_LAST_DELIVERED_SEQUENCE_KEY,
                "limit": DISCORD_DELIVERED_SEQUENCE_LIMIT,
                "last_synced_sequence": through_sequence,
                "updates": {
                    "discord_last_delivery_mode": result
                        .get("reply_mode")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                    "discord_last_delivery_message_ids": result
                        .get("message_ids")
                        .cloned()
                        .unwrap_or_else(|| JsonValue::Array(Vec::new())),
                },
            }),
        )
        .map_err(|_| discord_delivery_state_failure())?;
    if stored.is_object() {
        Ok(())
    } else {
        Err(discord_delivery_state_failure())
    }
}

fn discord_delivery_execution_request(
    config: &DiscordWorkerConfig,
    operation: JsonValue,
) -> JsonValue {
    json!({
        "api_base_url": config.api_base_url,
        "bot_token": config
            .bot_token
            .as_ref()
            .map(|token| JsonValue::String(token.expose().to_string()))
            .unwrap_or(JsonValue::Null),
        "http_user_agent": config.http_user_agent,
        "timeout_seconds": config
            .shared
            .request_timeout_seconds
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
        "repo_root": config.shared.runtime_target.repo_root.to_string_lossy(),
        "operation": operation,
    })
}

fn validate_discord_delivery_result_contract(
    result: &JsonValue,
    expected_kind: &str,
) -> Result<(), WorkerDiagnostic> {
    let object = result
        .as_object()
        .ok_or_else(discord_delivery_contract_failure)?;
    if clean_text(object.get("contract")).as_deref() != Some(DISCORD_REST_DELIVERY_CONTRACT)
        || clean_text(object.get("migration_stage")).as_deref()
            != Some(DISCORD_REST_DELIVERY_MIGRATION_STAGE)
        || clean_text(object.get("transport")).as_deref() != Some("discord")
        || clean_text(object.get("kind")).as_deref() != Some(expected_kind)
        || object
            .get("python_discord_api_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object
            .get("python_file_read_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object.get("ok").and_then(JsonValue::as_bool).is_none()
        || object
            .get("delivered")
            .and_then(JsonValue::as_bool)
            .is_none()
        || object
            .get("completed")
            .and_then(JsonValue::as_bool)
            .is_none()
        || clean_text(object.get("delivery_execution_state")).is_none()
    {
        return Err(discord_delivery_contract_failure());
    }
    Ok(())
}

fn execute_discord_interaction_recovery<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    interaction_payload: &JsonValue,
    recovery_request: &JsonValue,
) -> Result<JsonValue, WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let channel_id = clean_text(interaction_payload.get("channel_id"))
        .ok_or_else(discord_recovery_contract_failure)?;
    let interaction_token = clean_text(interaction_payload.get("token"))
        .ok_or_else(discord_recovery_contract_failure)?;
    execute_discord_recovery(
        executor,
        config,
        &channel_id,
        Some(&interaction_token),
        DiscordDeliveryTarget::Interaction(interaction_payload),
        recovery_request,
    )
}

fn execute_discord_channel_recovery<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    channel_id: &str,
    recovery_request: &JsonValue,
) -> Result<JsonValue, WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    execute_discord_recovery(
        executor,
        config,
        channel_id,
        None,
        DiscordDeliveryTarget::Channel(channel_id),
        recovery_request,
    )
}

fn execute_discord_recovery<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
    channel_id: &str,
    _interaction_token: Option<&str>,
    target: DiscordDeliveryTarget<'_>,
    recovery_request: &JsonValue,
) -> Result<JsonValue, WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let recovery = recovery_request
        .as_object()
        .ok_or_else(discord_recovery_contract_failure)?;
    clean_text(recovery.get("conversation_key")).ok_or_else(discord_recovery_contract_failure)?;
    let delivery_request = recovery
        .get("delivery_request")
        .filter(|request| delivery_request_has_operations(request))
        .ok_or_else(discord_recovery_exhausted)?;
    execute_discord_delivery_lifecycle(executor, config, delivery_request, target, channel_id)
}

fn execute_discord_delivery_sweep<E>(
    executor: &E,
    config: &DiscordWorkerConfig,
) -> Result<WorkerHttpResponse, WorkerDiagnostic>
where
    E: DiscordHttpInteractionJobExecutor,
{
    if config.bot_token.is_none() {
        return Ok(WorkerHttpResponse::new(204, Vec::new()));
    }
    let bindings = executor
        .execute_state(
            &config.shared.paths.sync_state_path,
            "list_bindings",
            &json!({
                "repo_name": config.shared.runtime_target.repo_name,
                "transport": "discord",
                "include_inactive": false,
            }),
        )
        .map_err(|_| discord_delivery_sweep_failure())?;
    let bindings = bindings
        .as_array()
        .ok_or_else(discord_delivery_sweep_failure)?;
    for binding in bindings.iter().take(DISCORD_SWEEP_BINDING_LIMIT) {
        let Some(channel_id) = discord_binding_channel_id(binding) else {
            continue;
        };
        let Some(delivery_request) = binding
            .get(DISCORD_PENDING_DELIVERY_KEY)
            .filter(|request| delivery_request_has_operations(request))
        else {
            continue;
        };
        execute_discord_delivery_lifecycle(
            executor,
            config,
            delivery_request,
            DiscordDeliveryTarget::Channel(&channel_id),
            &channel_id,
        )?;
    }
    Ok(WorkerHttpResponse::new(204, Vec::new()))
}

fn discord_binding_channel_id(binding: &JsonValue) -> Option<String> {
    clean_text(binding.get("surface_id"))
        .or_else(|| clean_text(binding.get("discord_channel_id")))
        .or_else(|| clean_text(binding.get("transport_channel_id")))
        .or_else(|| {
            binding
                .get("discord_reply_target")
                .and_then(|target| clean_text(target.get("channel_id")))
        })
}

fn json_response(
    status_code: u16,
    payload: &JsonValue,
    error_prefix: &str,
) -> Result<WorkerHttpResponse, WorkerDiagnostic> {
    let body = JsonCodec::encode_value_to_vec_with_error_prefix(
        payload,
        JsonEncodeOptions::compact(),
        error_prefix,
    )
    .map_err(|_| discord_interaction_response_failure())?;
    Ok(WorkerHttpResponse::new(status_code, body).with_header("Content-Type", JSON_CONTENT_TYPE))
}

fn discord_public_error(status_code: u16, message: &str) -> WorkerHttpResponse {
    let body = JsonCodec::encode_value_to_vec(
        &json!({"ok": false, "error": message}),
        JsonEncodeOptions::compact(),
    )
    .unwrap_or_else(|_| b"{\"ok\":false,\"error\":\"Discord interaction failed.\"}".to_vec());
    WorkerHttpResponse::new(status_code, body).with_header("Content-Type", JSON_CONTENT_TYPE)
}

fn optional_string_json(value: Option<&String>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.clone()))
        .unwrap_or(JsonValue::Null)
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn json_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(value) => value.as_i64(),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn discord_public_key_missing() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_public_key_missing",
        "The selected Discord interaction worker requires a public key.",
        EXIT_INVALID_CONFIGURATION,
    )
}

fn discord_ingress_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_interaction_ingress_failed",
        "The Rust Discord interaction ingress transaction failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_ingress_contract_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_interaction_ingress_contract_invalid",
        "The Rust Discord interaction ingress returned an invalid response contract.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_interaction_job_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_interaction_job_failed",
        "The Rust Discord interaction job failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_interaction_response_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_interaction_response_invalid",
        "The Rust Discord interaction job returned an invalid response.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_delivery_contract_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_rest_delivery_contract_invalid",
        "The Rust Discord REST delivery executor returned an invalid contract.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_delivery_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_rest_delivery_failed",
        "The Rust Discord background reply could not be delivered.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_delivery_state_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_delivery_state_update_failed",
        "The Rust Discord delivery receipt state could not be updated.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_recovery_contract_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_interaction_recovery_contract_invalid",
        "The Rust Discord interaction recovery transaction returned an invalid contract.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_recovery_exhausted() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_interaction_recovery_exhausted",
        "The Rust Discord interaction recovery attempts were exhausted.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_delivery_sweep_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_delivery_sweep_failed",
        "The bounded Rust Discord delivery sweep failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::net::SocketAddr;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use ait_agent_core::{
        resolve_agent_worker_config, AgentWorkerConfigInput, AgentWorkerRuntimeConfig,
    };
    use ait_core::json_support::{json, JsonCodec};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use tempfile::tempdir;

    use super::*;

    const PUBLIC_KEY: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
    const APPLICATION_ID: &str = "123456789012345678";
    const INTERACTION_TOKEN: &str = "discord-product-runner-token";
    const COMMAND_INTERACTION: &str = r#"{"id":"112233445566778899","type":2,"token":"discord-product-runner-token","application_id":"123456789012345678","channel_id":"998877665544332211","data":{"name":"ask","options":[{"name":"text","type":3,"value":"hello from rust"}]},"member":{"user":{"id":"discord-user-1","username":"weita","global_name":"WeiTa"}}}"#;

    #[derive(Clone)]
    struct StubExecutor {
        calls: Arc<Mutex<Vec<JsonValue>>>,
        results: Arc<Mutex<VecDeque<Result<JsonValue, String>>>>,
        delivery_calls: Arc<Mutex<Vec<JsonValue>>>,
        delivery_results: Arc<Mutex<VecDeque<Result<JsonValue, String>>>>,
        state_calls: Arc<Mutex<Vec<(String, String, JsonValue)>>>,
        state_results: Arc<Mutex<VecDeque<Result<JsonValue, String>>>>,
        backend_calls: Arc<Mutex<Vec<JsonValue>>>,
        backend_results: Arc<Mutex<VecDeque<Result<JsonValue, String>>>>,
        gate: Option<Arc<(Mutex<bool>, Condvar)>>,
    }

    impl StubExecutor {
        fn new(results: Vec<Result<JsonValue, String>>) -> Self {
            Self {
                calls: Arc::default(),
                results: Arc::new(Mutex::new(results.into())),
                delivery_calls: Arc::default(),
                delivery_results: Arc::default(),
                state_calls: Arc::default(),
                state_results: Arc::default(),
                backend_calls: Arc::default(),
                backend_results: Arc::default(),
                gate: None,
            }
        }

        fn with_delivery_results(
            results: Vec<Result<JsonValue, String>>,
            delivery_results: Vec<Result<JsonValue, String>>,
        ) -> Self {
            Self {
                calls: Arc::default(),
                results: Arc::new(Mutex::new(results.into())),
                delivery_calls: Arc::default(),
                delivery_results: Arc::new(Mutex::new(delivery_results.into())),
                state_calls: Arc::default(),
                state_results: Arc::default(),
                backend_calls: Arc::default(),
                backend_results: Arc::default(),
                gate: None,
            }
        }

        fn with_all_results(
            results: Vec<Result<JsonValue, String>>,
            delivery_results: Vec<Result<JsonValue, String>>,
            state_results: Vec<Result<JsonValue, String>>,
            backend_results: Vec<Result<JsonValue, String>>,
        ) -> Self {
            Self {
                calls: Arc::default(),
                results: Arc::new(Mutex::new(results.into())),
                delivery_calls: Arc::default(),
                delivery_results: Arc::new(Mutex::new(delivery_results.into())),
                state_calls: Arc::default(),
                state_results: Arc::new(Mutex::new(state_results.into())),
                backend_calls: Arc::default(),
                backend_results: Arc::new(Mutex::new(backend_results.into())),
                gate: None,
            }
        }

        fn blocked(results: Vec<Result<JsonValue, String>>) -> (Self, Arc<(Mutex<bool>, Condvar)>) {
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            (
                Self {
                    calls: Arc::default(),
                    results: Arc::new(Mutex::new(results.into())),
                    delivery_calls: Arc::default(),
                    delivery_results: Arc::default(),
                    state_calls: Arc::default(),
                    state_results: Arc::default(),
                    backend_calls: Arc::default(),
                    backend_results: Arc::default(),
                    gate: Some(gate.clone()),
                },
                gate,
            )
        }
    }

    impl DiscordHttpInteractionJobExecutor for StubExecutor {
        fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
            self.calls.lock().expect("calls").push(request.clone());
            if let Some(gate) = &self.gate {
                let (lock, ready) = &**gate;
                let released = lock.lock().expect("gate lock");
                drop(
                    ready
                        .wait_while(released, |released| !*released)
                        .expect("gate wait"),
                );
            }
            self.results
                .lock()
                .expect("results")
                .pop_front()
                .expect("stub result")
        }

        fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String> {
            self.delivery_calls
                .lock()
                .expect("delivery calls")
                .push(request.clone());
            self.delivery_results
                .lock()
                .expect("delivery results")
                .pop_front()
                .unwrap_or_else(|| Ok(delivered_result(request)))
        }

        fn execute_state(
            &self,
            path: &str,
            operation: &str,
            request: &JsonValue,
        ) -> Result<JsonValue, String> {
            self.state_calls.lock().expect("state calls").push((
                path.to_string(),
                operation.to_string(),
                request.clone(),
            ));
            if let Some(result) = self
                .state_results
                .lock()
                .expect("state results")
                .pop_front()
            {
                return result;
            }
            match operation {
                "has_recent_value" => Ok(JsonValue::Bool(false)),
                "list_bindings" => Ok(JsonValue::Array(Vec::new())),
                _ => Ok(json!({"transport": "discord", "surface_id": "stub"})),
            }
        }

        fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
            self.backend_calls
                .lock()
                .expect("backend calls")
                .push(request.clone());
            self.backend_results
                .lock()
                .expect("backend results")
                .pop_front()
                .unwrap_or_else(|| Ok(json!({"ok": true, "payload": []})))
        }
    }

    fn discord_config(
        public_key: Option<&str>,
        environment: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> AgentWorkerRuntimeConfig {
        let temp = tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
        std::fs::write(
            temp.path().join(".ait/config.json"),
            r#"{"repo_name":"fixture","workflow_mode":"solo_remote","default_remote":"origin","remotes":{"origin":{"url":"http://127.0.0.1:8088"}}}"#,
        )
        .expect("repo config");
        let mut worker = json!({
            "kind": "discord",
            "name": "main",
            "application_id": APPLICATION_ID,
        });
        if let Some(public_key) = public_key {
            worker["public_key"] = JsonValue::String(public_key.to_string());
        }
        let mut process_env = BTreeMap::from([(
            "AIT_DISCORD_INTERACTION_PATH".to_string(),
            "/interactions".to_string(),
        )]);
        process_env.extend(
            environment
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string())),
        );
        resolve_agent_worker_config(AgentWorkerConfigInput {
            repo_root: temp.path().to_path_buf(),
            worker_key: "discord/main".to_string(),
            worker,
            process_env,
        })
        .expect("Discord config")
    }

    fn handler<E: DiscordHttpInteractionJobExecutor>(
        executor: E,
        max_inflight: usize,
    ) -> DiscordWorkerHttpHandler<E> {
        let config = discord_config(Some(PUBLIC_KEY), []);
        let AgentWorkerRuntimeConfig::Discord(config) = config else {
            panic!("Discord config");
        };
        DiscordWorkerHttpHandler::new(&config, executor, max_inflight).expect("Discord handler")
    }

    fn key_pair() -> Ed25519KeyPair {
        let seed = (0u8..32).collect::<Vec<_>>();
        let pair = Ed25519KeyPair::from_seed_unchecked(&seed).expect("Ed25519 seed");
        assert_eq!(hex(pair.public_key().as_ref()), PUBLIC_KEY);
        pair
    }

    fn signed_request(body: Vec<u8>) -> WorkerHttpRequest {
        let timestamp = "1714990000";
        let mut message = timestamp.as_bytes().to_vec();
        message.extend_from_slice(&body);
        let signature = hex(key_pair().sign(&message).as_ref());
        WorkerHttpRequest {
            method: "POST".to_string(),
            path: "/interactions".to_string(),
            version: "HTTP/1.1".to_string(),
            headers: BTreeMap::from([
                ("x-signature-ed25519".to_string(), signature),
                ("x-signature-timestamp".to_string(), timestamp.to_string()),
            ]),
            body,
            peer_addr: SocketAddr::from(([127, 0, 0, 1], 40000)),
        }
    }

    fn processed_job(response: JsonValue) -> JsonValue {
        let delivery_request = response
            .get("data")
            .and_then(|data| data.get("content"))
            .and_then(JsonValue::as_str)
            .map(|content| {
                json!({
                    "reply_mode": "interaction",
                    "operations": [{
                        "kind": "edit_original_response",
                        "application_id": APPLICATION_ID,
                        "interaction_token": INTERACTION_TOKEN,
                        "text": content,
                    }],
                })
            })
            .unwrap_or(JsonValue::Null);
        json!({
            "contract": "ait_agent_core.event_loop.DiscordInteractionJob.v1",
            "migration_stage": "rust_agent_discord_interaction_job_transaction",
            "interaction_job_state": "processed",
            "ok": true,
            "processed": true,
            "duplicate": false,
            "conversation_key": "discord:channel:998877665544332211",
            "binding_created": false,
            "turn_ok": true,
            "recorded": true,
            "sequence": 7,
            "response": response,
            "delivery_request": delivery_request,
            "recovery_request": null,
            "error_kind": null,
        })
    }

    fn delivered_result(request: &JsonValue) -> JsonValue {
        let kind = request["operation"]["kind"].clone();
        json!({
            "contract": DISCORD_REST_DELIVERY_CONTRACT,
            "migration_stage": DISCORD_REST_DELIVERY_MIGRATION_STAGE,
            "transport": "discord",
            "kind": kind,
            "ok": true,
            "delivered": true,
            "completed": true,
            "delivery_execution_state": "delivered",
            "message_ids": ["message-1"],
            "operation_results": [],
            "python_discord_api_allowed": false,
            "python_file_read_allowed": false,
        })
    }

    fn failed_delivery_result(kind: &str, attachment_index: Option<i64>) -> JsonValue {
        json!({
            "contract": DISCORD_REST_DELIVERY_CONTRACT,
            "migration_stage": DISCORD_REST_DELIVERY_MIGRATION_STAGE,
            "transport": "discord",
            "kind": kind,
            "ok": false,
            "delivered": false,
            "completed": false,
            "delivery_execution_state": "delivery_failed",
            "attachment_index": attachment_index
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
            "message_ids": [],
            "error": "missing local file",
            "python_discord_api_allowed": false,
            "python_file_read_allowed": false,
        })
    }

    fn recovery_job() -> JsonValue {
        let mut job = processed_job(JsonValue::Null);
        job["interaction_job_state"] = json!("turn_backend_failed");
        job["ok"] = JsonValue::Bool(false);
        job["processed"] = JsonValue::Bool(false);
        job["turn_ok"] = JsonValue::Null;
        job["recorded"] = JsonValue::Bool(false);
        job["sequence"] = JsonValue::Null;
        job["response"] = JsonValue::Null;
        job["delivery_request"] = JsonValue::Null;
        job["recovery_request"] = json!({
            "conversation_key": "discord:channel:998877665544332211",
            "delivery_request": {
                "reply_mode": "interaction",
                "assistant_sequence": 6,
                "through_sequence": 6,
                "reply_text": "Recovered Discord reply",
                "attachments": [],
                "operations": [{
                    "kind": "edit_original_response",
                    "text": "Recovered Discord reply",
                }],
            },
        });
        job["error_kind"] = json!("gateway_reply");
        job
    }

    fn wait_completion<E: DiscordHttpInteractionJobExecutor>(
        handler: &mut DiscordWorkerHttpHandler<E>,
    ) -> WorkerHttpCompletion {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(completion) = handler.poll_completed().into_iter().next() {
                return completion;
            }
            assert!(
                Instant::now() < deadline,
                "Discord job completion timed out"
            );
            thread::yield_now();
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn signed_command_is_acknowledged_before_delivery_and_ping_is_immediate_job_free() {
        let executor = StubExecutor::new(vec![Ok(processed_job(json!({
            "type": 4,
            "data": {"content": "Rust Discord reply"},
        })))]);
        let calls = executor.calls.clone();
        let delivery_calls = executor.delivery_calls.clone();
        let mut handler = handler(executor, 2);

        let WorkerHttpDispatch::Immediate(ack) = handler
            .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
            .expect("command dispatch")
        else {
            panic!("immediate command acknowledgement");
        };
        assert_eq!(ack.status_code, 200);
        assert_eq!(ack.headers["Content-Type"], JSON_CONTENT_TYPE);
        assert_eq!(
            JsonCodec::parse_slice_with_error_prefix(&ack.body, "Discord acknowledgement")
                .expect("Discord acknowledgement"),
            json!({"type": 5})
        );
        assert!(!String::from_utf8_lossy(&ack.body).contains(INTERACTION_TOKEN));
        let completion = wait_completion(&mut handler)
            .result
            .expect("background delivery");
        assert_eq!(completion.status_code, 204);

        let WorkerHttpDispatch::Immediate(pong) = handler
            .handle(signed_request(br#"{"type":1}"#.to_vec()))
            .expect("ping dispatch")
        else {
            panic!("immediate Discord pong");
        };
        assert_eq!(
            JsonCodec::parse_slice_with_error_prefix(&pong.body, "Discord pong")
                .expect("Discord pong"),
            json!({"type": 1})
        );
        assert!(handler.poll_completed().is_empty());

        let calls = calls.lock().expect("calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["interaction_payload"]["data"]["name"], "ask");
        assert_eq!(calls[0]["runtime_target"]["mode"], "remote");
        assert_eq!(calls[0]["runtime_target"]["repo_name"], "fixture");
        assert!(calls[0]["state_path"].as_str().is_some());
        assert!(!calls[0].to_string().contains(PUBLIC_KEY));
        let delivery_calls = delivery_calls.lock().expect("delivery calls");
        assert_eq!(delivery_calls.len(), 1);
        assert_eq!(
            delivery_calls[0]["operation"]["kind"],
            "edit_original_response"
        );
        assert_eq!(
            delivery_calls[0]["operation"]["interaction_token"],
            INTERACTION_TOKEN
        );
        assert_eq!(delivery_calls[0]["operation"]["text"], "Rust Discord reply");
    }

    #[test]
    fn invalid_requests_are_rejected_before_jobs_start() {
        let executor = StubExecutor::new(Vec::new());
        let calls = executor.calls.clone();
        let mut handler = handler(executor, 1);

        let mut invalid_signature = signed_request(COMMAND_INTERACTION.as_bytes().to_vec());
        invalid_signature
            .headers
            .insert("x-signature-ed25519".to_string(), "00".repeat(64));
        let WorkerHttpDispatch::Immediate(invalid_signature) = handler
            .handle(invalid_signature)
            .expect("invalid signature response")
        else {
            panic!("immediate invalid signature response");
        };
        assert_eq!(invalid_signature.status_code, 401);
        assert!(String::from_utf8_lossy(&invalid_signature.body).contains("Invalid Discord"));

        let mut bad_path = signed_request(COMMAND_INTERACTION.as_bytes().to_vec());
        bad_path.path = "/wrong".to_string();
        let WorkerHttpDispatch::Immediate(bad_path) =
            handler.handle(bad_path).expect("bad path response")
        else {
            panic!("immediate bad path response");
        };
        assert_eq!(bad_path.status_code, 404);
        assert!(bad_path.body.is_empty());

        let WorkerHttpDispatch::Immediate(invalid_utf8) = handler
            .handle(signed_request(vec![0xff]))
            .expect("invalid UTF-8 response")
        else {
            panic!("immediate invalid UTF-8 response");
        };
        assert_eq!(invalid_utf8.status_code, 400);
        assert!(calls.lock().expect("calls").is_empty());
    }

    #[test]
    fn bounded_capacity_returns_503_and_recovers_after_reap() {
        let result = Ok(processed_job(json!({
            "type": 4,
            "data": {"content": "background reply"},
        })));
        let (executor, gate) = StubExecutor::blocked(vec![result.clone(), result]);
        let mut handler = handler(executor, 1);

        assert!(matches!(
            handler
                .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
                .expect("first interaction"),
            WorkerHttpDispatch::Immediate(WorkerHttpResponse {
                status_code: 200,
                ..
            })
        ));
        let WorkerHttpDispatch::Immediate(busy) = handler
            .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
            .expect("busy response")
        else {
            panic!("immediate capacity response");
        };
        assert_eq!(busy.status_code, 503);

        let (lock, ready) = &*gate;
        *lock.lock().expect("gate lock") = true;
        ready.notify_all();
        assert!(wait_completion(&mut handler).result.is_ok());
        assert_eq!(handler.inflight_work_count(), 0);

        assert!(matches!(
            handler
                .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
                .expect("recovered interaction"),
            WorkerHttpDispatch::Immediate(WorkerHttpResponse {
                status_code: 200,
                ..
            })
        ));
        assert!(wait_completion(&mut handler).result.is_ok());
    }

    #[test]
    fn background_delivery_executes_text_then_attachment_with_trusted_credentials() {
        let mut job = processed_job(json!({
            "type": 4,
            "data": {"content": "Rust Discord reply"},
        }));
        job["delivery_request"] = json!({
            "reply_mode": "interaction",
            "operations": [
                {
                    "kind": "edit_original_response",
                    "application_id": "untrusted-app",
                    "interaction_token": "untrusted-token",
                    "api_base_url": "https://untrusted.example.test",
                    "text": "Rust Discord reply",
                },
                {
                    "kind": "send_followup_attachment",
                    "attachment_index": 0,
                    "attachment": {
                        "kind": "document",
                        "local_path": "artifacts/report.md",
                        "file_name": "report.md",
                        "mime_type": "text/markdown",
                    },
                },
            ],
        });
        let executor = StubExecutor::new(vec![Ok(job)]);
        let delivery_calls = executor.delivery_calls.clone();
        let mut handler = handler(executor, 1);

        let WorkerHttpDispatch::Immediate(ack) = handler
            .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
            .expect("command acknowledgement")
        else {
            panic!("immediate acknowledgement");
        };
        assert_eq!(ack.status_code, 200);
        assert!(wait_completion(&mut handler).result.is_ok());

        let calls = delivery_calls.lock().expect("delivery calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["operation"]["kind"], "edit_original_response");
        assert_eq!(calls[1]["operation"]["kind"], "send_followup_attachment");
        for call in calls.iter() {
            assert_eq!(call["operation"]["application_id"], APPLICATION_ID);
            assert_eq!(call["operation"]["interaction_token"], INTERACTION_TOKEN);
            assert!(call["operation"].get("api_base_url").is_none());
            assert_eq!(call["api_base_url"], "https://discord.com/api/v10");
            assert_eq!(call["http_user_agent"], "curl/8.7.1");
            assert!(call["repo_root"].as_str().is_some());
        }
        assert_eq!(
            calls[1]["operation"]["attachment"]["local_path"],
            "artifacts/report.md"
        );
    }

    #[test]
    fn successful_delivery_records_sequence_and_duplicate_sequence_is_suppressed() {
        let executor = StubExecutor::new(vec![Ok(processed_job(json!({
            "type": 4,
            "data": {"content": "recorded reply"},
        })))]);
        let state_calls = executor.state_calls.clone();
        let mut first_handler = handler(executor, 1);
        assert!(matches!(
            first_handler
                .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
                .expect("recording dispatch"),
            WorkerHttpDispatch::Immediate(_)
        ));
        assert!(wait_completion(&mut first_handler).result.is_ok());
        let state_calls = state_calls.lock().expect("state calls");
        assert_eq!(state_calls.len(), 4);
        assert_eq!(state_calls[0].1, "patch_binding");
        assert_eq!(state_calls[1].1, "has_recent_value");
        assert_eq!(state_calls[2].1, "remember_recent_value");
        assert_eq!(state_calls[3].1, "patch_binding");
        assert_eq!(state_calls[2].2["value"], 7);
        assert_eq!(state_calls[2].2["last_synced_sequence"], 7);
        assert_eq!(
            state_calls[2].2["recent_key"],
            DISCORD_DELIVERED_SEQUENCE_KEY
        );
        assert_eq!(
            state_calls[0].2["updates"][DISCORD_PENDING_DELIVERY_KEY]["operations"][0]["kind"],
            "send_channel_message"
        );
        assert!(state_calls[0].2["updates"][DISCORD_PENDING_DELIVERY_KEY]
            .to_string()
            .find(INTERACTION_TOKEN)
            .is_none());
        assert!(state_calls[3].2["updates"][DISCORD_PENDING_DELIVERY_KEY].is_null());

        let duplicate = StubExecutor::with_all_results(
            vec![Ok(processed_job(json!({
                "type": 4,
                "data": {"content": "must not be sent twice"},
            })))],
            Vec::new(),
            vec![
                Ok(json!({"transport": "discord", "surface_id": "stub"})),
                Ok(JsonValue::Bool(true)),
                Ok(json!({"transport": "discord", "surface_id": "stub"})),
            ],
            Vec::new(),
        );
        let delivery_calls = duplicate.delivery_calls.clone();
        let state_calls = duplicate.state_calls.clone();
        let mut handler = handler(duplicate, 1);
        assert!(matches!(
            handler
                .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
                .expect("duplicate dispatch"),
            WorkerHttpDispatch::Immediate(_)
        ));
        assert!(wait_completion(&mut handler).result.is_ok());
        assert!(delivery_calls.lock().expect("delivery calls").is_empty());
        let state_calls = state_calls.lock().expect("state calls");
        assert_eq!(state_calls.len(), 3);
        assert_eq!(state_calls[0].1, "patch_binding");
        assert_eq!(state_calls[1].1, "has_recent_value");
        assert_eq!(state_calls[2].1, "patch_binding");
    }

    #[test]
    fn attachment_failure_executes_planned_text_fallback_before_recording() {
        let mut job = processed_job(json!({
            "type": 4,
            "data": {"content": "reply with attachment"},
        }));
        job["sequence"] = JsonValue::from(8);
        job["delivery_request"] = json!({
            "reply_mode": "interaction",
            "assistant_sequence": 8,
            "through_sequence": 8,
            "reply_text": "reply with attachment",
            "attachments": [{
                "kind": "document",
                "local_path": "artifacts/report.md",
                "file_name": "report.md",
            }],
            "operations": [
                {"kind": "edit_original_response", "text": "reply with attachment"},
                {
                    "kind": "send_followup_attachment",
                    "attachment_index": 0,
                    "attachment": {
                        "kind": "document",
                        "local_path": "artifacts/report.md",
                        "file_name": "report.md",
                    },
                },
            ],
        });
        let executor = StubExecutor::with_delivery_results(
            vec![Ok(job)],
            vec![
                Ok(delivered_result(&json!({
                    "operation": {"kind": "edit_original_response"}
                }))),
                Ok(failed_delivery_result("send_followup_attachment", Some(0))),
                Ok(delivered_result(&json!({
                    "operation": {"kind": "send_followup"}
                }))),
            ],
        );
        let delivery_calls = executor.delivery_calls.clone();
        let state_calls = executor.state_calls.clone();
        let mut handler = handler(executor, 1);
        assert!(matches!(
            handler
                .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
                .expect("attachment dispatch"),
            WorkerHttpDispatch::Immediate(_)
        ));
        assert!(wait_completion(&mut handler).result.is_ok());
        let calls = delivery_calls.lock().expect("delivery calls");
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0]["operation"]["kind"], "edit_original_response");
        assert_eq!(calls[1]["operation"]["kind"], "send_followup_attachment");
        assert_eq!(calls[2]["operation"]["kind"], "send_followup");
        assert!(calls[2]["operation"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("report.md") && text.contains("missing local file")));
        let state_calls = state_calls.lock().expect("state calls");
        let remember = state_calls
            .iter()
            .find(|call| call.1 == "remember_recent_value")
            .expect("remember call");
        assert_eq!(remember.2["value"], 8);
        assert_eq!(
            state_calls.last().map(|call| call.1.as_str()),
            Some("patch_binding")
        );
    }

    #[test]
    fn pending_interaction_uses_concrete_recovery_request_without_session_backend() {
        let executor = StubExecutor::with_all_results(
            vec![Ok(recovery_job())],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let backend_calls = executor.backend_calls.clone();
        let delivery_calls = executor.delivery_calls.clone();
        let state_calls = executor.state_calls.clone();
        let mut recovered_handler = handler(executor, 1);
        assert!(matches!(
            recovered_handler
                .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
                .expect("recovery dispatch"),
            WorkerHttpDispatch::Immediate(_)
        ));
        assert!(wait_completion(&mut recovered_handler).result.is_ok());
        assert!(backend_calls.lock().expect("backend calls").is_empty());
        let delivery_calls = delivery_calls.lock().expect("delivery calls");
        assert_eq!(delivery_calls.len(), 1);
        assert_eq!(
            delivery_calls[0]["operation"]["text"],
            "Recovered Discord reply"
        );
        assert_eq!(
            delivery_calls[0]["operation"]["interaction_token"],
            INTERACTION_TOKEN
        );
        let state_calls = state_calls.lock().expect("state calls");
        assert_eq!(
            state_calls
                .iter()
                .map(|call| call.1.as_str())
                .collect::<Vec<_>>(),
            vec![
                "patch_binding",
                "has_recent_value",
                "remember_recent_value",
                "patch_binding",
            ]
        );
        let persisted = &state_calls[0].2["updates"][DISCORD_PENDING_DELIVERY_KEY];
        assert_eq!(persisted["operations"][0]["kind"], "send_channel_message");
        assert_eq!(
            persisted["operations"][0]["text"],
            "Recovered Discord reply"
        );
        assert!(!persisted.to_string().contains(INTERACTION_TOKEN));
        assert_eq!(state_calls[2].2["value"], 6);
        assert!(state_calls[3].2["updates"][DISCORD_PENDING_DELIVERY_KEY].is_null());

        let mut malformed = recovery_job();
        malformed["recovery_request"]["delivery_request"] = JsonValue::Null;
        let malformed = StubExecutor::new(vec![Ok(malformed)]);
        let backend_calls = malformed.backend_calls.clone();
        let delivery_calls = malformed.delivery_calls.clone();
        let mut handler = handler(malformed, 1);
        assert!(matches!(
            handler
                .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
                .expect("malformed recovery dispatch"),
            WorkerHttpDispatch::Immediate(_)
        ));
        let error = wait_completion(&mut handler)
            .result
            .expect_err("recovery should exhaust");
        assert_eq!(error.code, "discord_interaction_recovery_exhausted");
        assert!(backend_calls.lock().expect("backend calls").is_empty());
        assert!(delivery_calls.lock().expect("delivery calls").is_empty());
        assert!(!error.render_json().contains(INTERACTION_TOKEN));
    }

    #[test]
    fn startup_sweep_ignores_bindings_without_pending_delivery() {
        let binding = json!({
            "transport": "discord",
            "surface_id": "998877665544332211",
            "conversation_key": "discord:channel:998877665544332211",
            "repo_name": "fixture",
            "discord_live_delivered_sequences": [],
        });
        let executor = StubExecutor::with_all_results(
            Vec::new(),
            Vec::new(),
            vec![Ok(JsonValue::Array(vec![binding]))],
            Vec::new(),
        );
        let delivery_calls = executor.delivery_calls.clone();
        let state_calls = executor.state_calls.clone();
        let backend_calls = executor.backend_calls.clone();
        let config = discord_config(
            Some(PUBLIC_KEY),
            [("AIT_DISCORD_BOT_TOKEN", "trusted-bot-token")],
        );
        let AgentWorkerRuntimeConfig::Discord(config) = config else {
            panic!("Discord config");
        };
        let mut handler = DiscordWorkerHttpHandler::new(&config, executor, 1)
            .expect("Discord handler with startup sweep");
        assert!(wait_completion(&mut handler).result.is_ok());
        assert!(delivery_calls.lock().expect("delivery calls").is_empty());
        assert!(backend_calls.lock().expect("backend calls").is_empty());
        let state_calls = state_calls.lock().expect("state calls");
        assert_eq!(state_calls.len(), 1);
        assert_eq!(state_calls[0].1, "list_bindings");
    }

    #[test]
    fn startup_sweep_delivers_persisted_channel_request_without_session_backend() {
        let binding = json!({
            "transport": "discord",
            "surface_id": "998877665544332211",
            "conversation_key": "discord:channel:998877665544332211",
            "repo_name": "fixture",
            "discord_live_delivered_sequences": [],
            "discord_pending_delivery": {
                "reply_mode": "channel_message",
                "assistant_sequence": 14,
                "through_sequence": 14,
                "reply_text": "unseen text reply",
                "attachments": [{
                    "kind": "document",
                    "local_path": "artifacts/report.md",
                    "file_name": "report.md",
                }],
                "operations": [
                    {
                        "kind": "send_channel_message",
                        "text": "unseen text reply",
                    },
                    {
                        "kind": "send_channel_attachment",
                        "attachment_index": 0,
                        "attachment": {
                            "kind": "document",
                            "local_path": "artifacts/report.md",
                            "file_name": "report.md",
                        },
                    },
                ],
                "operation_count": 2,
            },
        });
        let executor = StubExecutor::with_all_results(
            Vec::new(),
            Vec::new(),
            vec![Ok(JsonValue::Array(vec![binding]))],
            Vec::new(),
        );
        let delivery_calls = executor.delivery_calls.clone();
        let state_calls = executor.state_calls.clone();
        let backend_calls = executor.backend_calls.clone();
        let config = discord_config(
            Some(PUBLIC_KEY),
            [("AIT_DISCORD_BOT_TOKEN", "trusted-bot-token")],
        );
        let AgentWorkerRuntimeConfig::Discord(config) = config else {
            panic!("Discord config");
        };
        let mut handler = DiscordWorkerHttpHandler::new(&config, executor, 1)
            .expect("Discord handler with startup sweep");
        assert!(wait_completion(&mut handler).result.is_ok());
        assert!(backend_calls.lock().expect("backend calls").is_empty());
        let calls = delivery_calls.lock().expect("delivery calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["operation"]["kind"], "send_channel_message");
        assert_eq!(calls[0]["operation"]["text"], "unseen text reply");
        assert_eq!(calls[1]["operation"]["kind"], "send_channel_attachment");
        assert_eq!(
            calls[1]["operation"]["attachment"]["file_name"],
            "report.md"
        );
        assert!(calls
            .iter()
            .all(|call| call["bot_token"] == "trusted-bot-token"));
        let state_calls = state_calls.lock().expect("state calls");
        assert_eq!(
            state_calls
                .iter()
                .map(|call| call.1.as_str())
                .collect::<Vec<_>>(),
            vec![
                "list_bindings",
                "patch_binding",
                "has_recent_value",
                "remember_recent_value",
                "patch_binding",
            ]
        );
        assert_eq!(state_calls[3].2["value"], 14);
        assert!(state_calls[4].2["updates"][DISCORD_PENDING_DELIVERY_KEY].is_null());
    }

    #[test]
    fn background_delivery_failure_is_bounded_and_secret_safe() {
        let failed_delivery = json!({
            "contract": DISCORD_REST_DELIVERY_CONTRACT,
            "migration_stage": DISCORD_REST_DELIVERY_MIGRATION_STAGE,
            "transport": "discord",
            "kind": "edit_original_response",
            "ok": false,
            "delivered": false,
            "completed": false,
            "delivery_execution_state": "delivery_failed",
            "error": format!("failed {PUBLIC_KEY} {INTERACTION_TOKEN}"),
            "python_discord_api_allowed": false,
            "python_file_read_allowed": false,
        });
        let executor = StubExecutor::with_delivery_results(
            vec![Ok(processed_job(json!({
                "type": 4,
                "data": {"content": "Rust Discord reply"},
            })))],
            vec![Ok(failed_delivery)],
        );
        let mut handler = handler(executor, 1);

        let WorkerHttpDispatch::Immediate(ack) = handler
            .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
            .expect("command acknowledgement")
        else {
            panic!("immediate acknowledgement");
        };
        assert_eq!(ack.status_code, 200);
        let error = wait_completion(&mut handler)
            .result
            .expect_err("delivery failure");
        assert_eq!(error.code, "discord_rest_delivery_failed");
        let public = error.render_json();
        assert!(!public.contains(PUBLIC_KEY));
        assert!(!public.contains(INTERACTION_TOKEN));
    }

    #[test]
    fn job_failures_and_shutdown_are_secret_safe_and_bounded() {
        let mut handler = handler(
            StubExecutor::new(vec![Err(format!(
                "{PUBLIC_KEY} {INTERACTION_TOKEN} backend failure"
            ))]),
            1,
        );
        let WorkerHttpDispatch::Immediate(ack) = handler
            .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
            .expect("dispatch")
        else {
            panic!("immediate acknowledgement");
        };
        assert_eq!(ack.status_code, 200);
        let error = wait_completion(&mut handler)
            .result
            .expect_err("job failure");
        assert_eq!(error.code, "discord_interaction_job_failed");
        assert!(!error.render_json().contains(PUBLIC_KEY));
        assert!(!error.render_json().contains(INTERACTION_TOKEN));

        handler.close_admission();
        let WorkerHttpDispatch::Immediate(closed) = handler
            .handle(signed_request(COMMAND_INTERACTION.as_bytes().to_vec()))
            .expect("closed response")
        else {
            panic!("immediate closed response");
        };
        assert_eq!(closed.status_code, 503);
        let WorkerHttpDispatch::Immediate(pong) = handler
            .handle(signed_request(br#"{"type":1}"#.to_vec()))
            .expect("pong during drain")
        else {
            panic!("immediate pong during drain");
        };
        assert_eq!(pong.status_code, 200);
        assert!(handler.finish_shutdown().is_ok());
        handler.force_shutdown().expect("forced shutdown");
        assert_eq!(handler.inflight_work_count(), 0);
    }

    #[test]
    fn typed_configuration_rejects_missing_key_and_invalid_bind_address() {
        let missing = discord_config(None, []);
        let AgentWorkerRuntimeConfig::Discord(missing) = missing else {
            panic!("Discord config");
        };
        let error = DiscordWorkerHttpHandler::new(&missing, StubExecutor::new(Vec::new()), 1)
            .err()
            .expect("missing public key");
        assert_eq!(error.code, "discord_public_key_missing");

        let invalid_port = discord_config(Some(PUBLIC_KEY), [("AIT_DISCORD_BIND_PORT", "70000")]);
        let AgentWorkerRuntimeConfig::Discord(invalid_port) = invalid_port else {
            panic!("Discord config");
        };
        let error = resolve_discord_bind_addr(&invalid_port).expect_err("invalid port");
        assert_eq!(error.code, "discord_worker_bind_port_invalid");
        assert!(!error.render_json().contains(PUBLIC_KEY));
    }
}
