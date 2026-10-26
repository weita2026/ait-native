use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{
    agent_slack_background_reply_transaction_execute_json, agent_slack_ingress_runtime_plan_json,
};
use crate::runtime::{agent_runtime_backend_execute_json, AgentRuntimeBindingStore};

const MIGRATION_STAGE: &str = "rust_agent_slack_command_job_transaction";
const SLACK_COMMAND_JOB_CONTRACT: &str = "ait_agent_core.event_loop.SlackCommandJob.v1";
const RECENT_COMMAND_KEY: &str = "slack_recent_request_ids";
const LAST_COMMAND_KEY: &str = "slack_last_request_id";
const RECENT_COMMAND_LIMIT: i64 = 64;
const DEFAULT_ACK_TEXT: &str = "ait is thinking...";
const DEFAULT_RESPONSE_TYPE: &str = "in_channel";
const REDACTED: &str = "[redacted]";

pub trait SlackCommandJobStatePort {
    fn execute_state(
        &self,
        path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String>;
}

pub trait SlackCommandJobBackendPort {
    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait SlackCommandJobDeliveryPort {
    fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackCommandJobStatePort;

impl SlackCommandJobStatePort for DefaultSlackCommandJobStatePort {
    fn execute_state(
        &self,
        path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String> {
        AgentRuntimeBindingStore::new(path).execute(operation, request)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackCommandJobBackendPort;

impl SlackCommandJobBackendPort for DefaultSlackCommandJobBackendPort {
    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_runtime_backend_execute_json(request)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackCommandJobDeliveryPort;

impl SlackCommandJobDeliveryPort for DefaultSlackCommandJobDeliveryPort {
    fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_slack_background_reply_transaction_execute_json(request)
    }
}

pub fn agent_slack_command_job_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    execute_with_slack_command_job_ports(
        &DefaultSlackCommandJobStatePort,
        &DefaultSlackCommandJobBackendPort,
        &DefaultSlackCommandJobDeliveryPort,
        request,
    )
}

pub fn execute_with_slack_command_job_ports<S, B, D>(
    state: &S,
    backend: &B,
    delivery: &D,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    S: SlackCommandJobStatePort + ?Sized,
    B: SlackCommandJobBackendPort + ?Sized,
    D: SlackCommandJobDeliveryPort + ?Sized,
{
    let request = request_object(request)?;
    let target = required_object(request.get("runtime_target"), "runtime_target")?;
    let mode = required_text(target.get("mode"), "runtime_target.mode")?;
    if !matches!(mode.as_str(), "remote" | "local") {
        return Err(
            "Slack command job runtime_target.mode must be `remote` or `local`.".to_string(),
        );
    }

    let input = CommandJobInput::parse(request, target)?;
    let initial_plan = plan_command(&input, None)?;
    let initial_plan = required_plan_object(&initial_plan)?;
    if !plan_bool(initial_plan, "should_submit_turn")? {
        return Ok(non_submitted_outcome(initial_plan)
            .redacted(&input)
            .payload());
    }

    let binding_request = input.binding_lookup_request();
    let binding = match state.execute_state(&input.state_path, "get_binding", &binding_request) {
        Ok(JsonValue::Null) => JsonValue::Null,
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => return Ok(state_failure("binding_read_failed").payload()),
    };
    let plan = if binding.is_object() {
        plan_command(&input, Some(&binding))?
    } else {
        JsonValue::Object(initial_plan.clone())
    };
    let plan_object = required_plan_object(&plan)?;
    if !plan_bool(plan_object, "should_submit_turn")? {
        return Ok(non_submitted_outcome(plan_object)
            .redacted(&input)
            .payload());
    }

    let conversation_key = slack_conversation_key(&input.channel_id, input.thread_id.as_deref());
    let binding_created = binding.is_null();
    let mut pending = match plan_object.get("pending_reply") {
        Some(value) if value.is_object() => value.clone(),
        _ => {
            return Ok(with_conversation(
                contract_failure("pending_reply_invalid"),
                conversation_key,
                binding_created,
            )
            .redacted(&input)
            .payload())
        }
    };
    let pending_object = pending
        .as_object_mut()
        .ok_or_else(|| "Slack pending reply must be an object.".to_string())?;
    pending_object.insert(
        "conversation_key".to_string(),
        JsonValue::String(conversation_key.clone()),
    );

    let upsert_request = binding_upsert_request(&input, plan_object, &conversation_key);
    let bound = match state.execute_state(&input.state_path, "upsert_binding", &upsert_request) {
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => {
            return Ok(with_conversation(
                state_failure("binding_write_failed"),
                conversation_key,
                binding_created,
            )
            .redacted(&input)
            .payload())
        }
    };
    let remember_request = remember_command_request(&input, plan_object);
    let remembered_binding = match state.execute_state(
        &input.state_path,
        "remember_recent_value",
        &remember_request,
    ) {
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => {
            return Ok(with_conversation(
                state_failure("command_remember_failed"),
                conversation_key,
                binding_created,
            )
            .redacted(&input)
            .payload())
        }
    };

    let turn_request = backend_request(
        &input,
        "create_turn",
        required_plan_text(plan_object, "actor_identity")?.as_str(),
        "slack_user",
        json!({
            "conversation_key": conversation_key,
            "provider_thread": bound
                .get("codex_thread_binding")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "payload": {
                "text": required_plan_text(plan_object, "text")?,
                "surface": "slack",
                "title": required_plan_text(plan_object, "channel_title")?,
                "actor_display_name": optional_string_json(clean_text(plan_object.get("actor_display_name")).as_deref()),
                "transport_envelope": plan_object.get("transport_envelope").cloned().unwrap_or(JsonValue::Null),
            },
        }),
    );
    let turn = match backend_payload(backend, &turn_request) {
        Ok(value)
            if value.get("ok").and_then(JsonValue::as_bool).is_some()
                && clean_text(value.get("conversation_key")).as_deref()
                    == Some(conversation_key.as_str()) =>
        {
            JsonValue::Object(value)
        }
        Ok(_) => {
            let delivered = deliver_background_error(
                delivery,
                &input,
                &pending,
                &remembered_binding,
                "Slack turn backend returned an invalid payload.",
            );
            let mut outcome = with_conversation(
                backend_contract_failure("turn_payload_invalid"),
                conversation_key,
                binding_created,
            );
            outcome.delivery_attempted = true;
            outcome.delivered = delivered;
            return Ok(outcome.redacted(&input).payload());
        }
        Err(_) => {
            let delivered = deliver_background_error(
                delivery,
                &input,
                &pending,
                &remembered_binding,
                "Slack turn backend failed.",
            );
            let mut outcome = with_conversation(
                backend_failure("turn_backend_failed"),
                conversation_key,
                binding_created,
            );
            outcome.delivery_attempted = true;
            outcome.delivered = delivered;
            return Ok(outcome.redacted(&input).payload());
        }
    };
    let turn_ok = turn.get("ok").and_then(JsonValue::as_bool).unwrap_or(false);
    let delivery_request =
        background_delivery_request(&input, &pending, &remembered_binding, Some(&turn), None);
    let delivery_result = match delivery.execute_delivery(&delivery_request) {
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => {
            let mut outcome = with_conversation(
                delivery_contract_failure("delivery_transaction_failed"),
                conversation_key,
                binding_created,
            );
            outcome.turn_ok = Some(turn_ok);
            outcome.delivery_attempted = true;
            return Ok(outcome.redacted(&input).payload());
        }
    };
    let delivery_ok = match delivery_result
        .get("delivery_ok")
        .and_then(JsonValue::as_bool)
    {
        Some(value) => value,
        None => {
            let mut outcome = with_conversation(
                delivery_contract_failure("delivery_contract_invalid"),
                conversation_key,
                binding_created,
            );
            outcome.turn_ok = Some(turn_ok);
            outcome.delivery_attempted = true;
            return Ok(outcome.redacted(&input).payload());
        }
    };
    let transaction_ok = delivery_result.get("ok").and_then(JsonValue::as_bool);
    let should_apply_patch = delivery_result
        .get("should_apply_state_patch")
        .and_then(JsonValue::as_bool);
    let should_send_response = delivery_result
        .get("should_send_response")
        .and_then(JsonValue::as_bool);
    let (Some(transaction_ok), Some(should_apply_patch), Some(should_send_response)) =
        (transaction_ok, should_apply_patch, should_send_response)
    else {
        let mut outcome = with_conversation(
            delivery_contract_failure("delivery_contract_invalid"),
            conversation_key,
            binding_created,
        );
        outcome.turn_ok = Some(turn_ok);
        outcome.delivery_attempted = true;
        return Ok(outcome.redacted(&input).payload());
    };
    if !transaction_ok || !delivery_ok {
        let mut outcome = with_conversation(
            delivery_failure("response_delivery_failed"),
            conversation_key,
            binding_created,
        );
        outcome.turn_ok = Some(turn_ok);
        outcome.delivery_attempted = true;
        return Ok(outcome.redacted(&input).payload());
    }

    let mut recorded = !should_apply_patch;
    let mut sequence = None;
    if should_apply_patch {
        let patch = delivery_result
            .get("remember_command_patch")
            .or_else(|| delivery_result.get("state_patch"));
        let Some(patch) = patch.filter(|value| value.is_object()) else {
            let mut outcome = with_conversation(
                delivery_contract_failure("state_patch_invalid"),
                conversation_key,
                binding_created,
            );
            outcome.turn_ok = Some(turn_ok);
            outcome.delivery_attempted = true;
            outcome.delivered = true;
            return Ok(outcome.redacted(&input).payload());
        };
        sequence = patch
            .get("last_synced_sequence")
            .and_then(JsonValue::as_i64);
        let patch_request = json!({
            "transport": "slack",
            "surface_id": input.channel_id,
            "thread_id": optional_string_json(input.thread_id.as_deref()),
            "updates": patch,
        });
        recorded = matches!(
            state.execute_state(&input.state_path, "patch_binding", &patch_request),
            Ok(value) if value.is_object()
        );
        if !recorded {
            let mut outcome = with_conversation(
                state_failure("delivered_state_patch_failed"),
                conversation_key,
                binding_created,
            );
            outcome.turn_ok = Some(turn_ok);
            outcome.delivery_attempted = true;
            outcome.delivered = true;
            outcome.sequence = sequence;
            return Ok(outcome.redacted(&input).payload());
        }
    }

    let mut outcome = with_conversation(
        JobOutcome::new(
            if turn_ok {
                "processed"
            } else {
                "turn_failed_reported"
            },
            true,
        ),
        conversation_key,
        binding_created,
    );
    outcome.processed = true;
    outcome.turn_ok = Some(turn_ok);
    outcome.delivery_attempted = should_send_response;
    outcome.delivered = delivery_ok && outcome.delivery_attempted;
    outcome.recorded = recorded;
    outcome.sequence = sequence;
    if !turn_ok {
        outcome.error_kind = Some("ait_reply".to_string());
        outcome.error = Some("Slack turn failed and the failure was reported.".to_string());
    }
    Ok(outcome.redacted(&input).payload())
}

fn plan_command(input: &CommandJobInput, binding: Option<&JsonValue>) -> Result<JsonValue, String> {
    let mut request = json!({
        "stage": "command",
        "payload": input.command_payload,
        "repo_name": input.repo_name,
        "defer_replies": true,
        "ack_text": input.ack_text,
        "response_type": input.response_type,
    });
    if let Some(occurred_at) = &input.occurred_at {
        request["occurred_at"] = JsonValue::String(occurred_at.clone());
    }
    if let Some(binding) = binding {
        request["binding"] = binding.clone();
    }
    agent_slack_ingress_runtime_plan_json(&request)
}

fn binding_upsert_request(
    input: &CommandJobInput,
    plan: &Map<String, JsonValue>,
    conversation_key: &str,
) -> JsonValue {
    let channel_title = clean_text(plan.get("channel_title")).unwrap_or_default();
    let channel_kind = clean_text(plan.get("channel_kind"));
    let source_user_id = clean_text(plan.get("source_user_id"));
    let team_id = clean_text(plan.get("team_id"));
    let reply_target = slack_reply_target(
        &input.channel_id,
        channel_kind.as_deref(),
        team_id.as_deref(),
        &input.response_url,
        input.thread_id.as_deref(),
        source_user_id.as_deref(),
    );
    json!({
        "transport": "slack",
        "surface_id": input.channel_id,
        "thread_id": optional_string_json(input.thread_id.as_deref()),
        "repo_name": input.repo_name,
        "surface_title": channel_title,
        "surface_kind": optional_string_json(channel_kind.as_deref()),
        "updates": {
            "conversation_key": conversation_key,
            "slack_source_user_id": optional_string_json(source_user_id.as_deref()),
            "slack_team_id": optional_string_json(team_id.as_deref()),
            "slack_reply_target": reply_target,
        },
    })
}

fn remember_command_request(input: &CommandJobInput, plan: &Map<String, JsonValue>) -> JsonValue {
    json!({
        "transport": "slack",
        "surface_id": input.channel_id,
        "thread_id": optional_string_json(input.thread_id.as_deref()),
        "value": clean_text(plan.get("request_id")).unwrap_or_default(),
        "recent_key": RECENT_COMMAND_KEY,
        "last_value_key": LAST_COMMAND_KEY,
        "limit": RECENT_COMMAND_LIMIT,
        "updates": {
            "slack_last_source_user_id": plan.get("source_user_id").cloned().unwrap_or(JsonValue::Null),
            "slack_last_team_id": plan.get("team_id").cloned().unwrap_or(JsonValue::Null),
            "slack_last_command_name": plan.get("command_name").cloned().unwrap_or(JsonValue::Null),
        },
    })
}

fn slack_reply_target(
    channel_id: &str,
    channel_kind: Option<&str>,
    team_id: Option<&str>,
    response_url: &str,
    thread_id: Option<&str>,
    source_user_id: Option<&str>,
) -> JsonValue {
    let mut target = Map::new();
    target.insert(
        "channel_id".to_string(),
        JsonValue::String(channel_id.to_string()),
    );
    target.insert(
        "response_url".to_string(),
        JsonValue::String(response_url.to_string()),
    );
    for (key, value) in [
        ("channel_kind", channel_kind),
        ("team_id", team_id),
        ("thread_id", thread_id),
        ("source_user_id", source_user_id),
    ] {
        if let Some(value) = value {
            target.insert(key.to_string(), JsonValue::String(value.to_string()));
        }
    }
    JsonValue::Object(target)
}

fn background_delivery_request(
    input: &CommandJobInput,
    pending: &JsonValue,
    binding: &JsonValue,
    turn: Option<&JsonValue>,
    error: Option<&str>,
) -> JsonValue {
    let mut request = json!({
        "stage": "background_result",
        "pending_reply": pending,
        "response_type": input.response_type,
        "existing_recent_request_ids": binding
            .get(RECENT_COMMAND_KEY)
            .cloned()
            .unwrap_or_else(|| json!([])),
        "last_synced_sequence": binding
            .get("last_synced_sequence")
            .cloned()
            .unwrap_or_else(|| json!(0)),
    });
    if let Some(timeout_seconds) = &input.timeout_seconds {
        request["timeout_seconds"] = timeout_seconds.clone();
    }
    if let Some(turn) = turn {
        request["turn"] = turn.clone();
    }
    if let Some(error) = error {
        request["error"] = JsonValue::String(error.to_string());
    }
    request
}

fn deliver_background_error<D>(
    delivery: &D,
    input: &CommandJobInput,
    pending: &JsonValue,
    binding: &JsonValue,
    error: &str,
) -> bool
where
    D: SlackCommandJobDeliveryPort + ?Sized,
{
    matches!(
        delivery.execute_delivery(&background_delivery_request(
            input,
            pending,
            binding,
            None,
            Some(error),
        )),
        Ok(value)
            if value.get("ok").and_then(JsonValue::as_bool) == Some(true)
                && value.get("delivery_ok").and_then(JsonValue::as_bool) == Some(true)
    )
}

fn backend_request(
    input: &CommandJobInput,
    operation: &str,
    actor_identity: &str,
    actor_type: &str,
    arguments: JsonValue,
) -> JsonValue {
    let mut request = json!({
        "operation": operation,
        "target": input.runtime_target,
        "actor": {
            "identity": actor_identity,
            "type": actor_type,
        },
        "arguments": arguments,
    });
    if let Some(timeout_seconds) = &input.timeout_seconds {
        request["timeout_seconds"] = timeout_seconds.clone();
    }
    if let Some(local_reply) = &input.local_reply {
        request["local_reply"] = local_reply.clone();
    }
    request
}

fn backend_payload<B>(backend: &B, request: &JsonValue) -> Result<Map<String, JsonValue>, ()>
where
    B: SlackCommandJobBackendPort + ?Sized,
{
    let response = backend.execute_backend(request).map_err(|_| ())?;
    if response.get("ok").and_then(JsonValue::as_bool) != Some(true) {
        return Err(());
    }
    response
        .get("payload")
        .and_then(JsonValue::as_object)
        .cloned()
        .ok_or(())
}

struct CommandJobInput {
    state_path: String,
    runtime_target: JsonValue,
    repo_name: String,
    command_payload: JsonValue,
    channel_id: String,
    thread_id: Option<String>,
    response_url: String,
    ack_text: String,
    response_type: String,
    occurred_at: Option<String>,
    timeout_seconds: Option<JsonValue>,
    local_reply: Option<JsonValue>,
}

impl CommandJobInput {
    fn parse(
        request: &Map<String, JsonValue>,
        target: &Map<String, JsonValue>,
    ) -> Result<Self, String> {
        let payload = request
            .get("command_payload")
            .or_else(|| request.get("payload"))
            .and_then(JsonValue::as_object)
            .ok_or_else(|| "Slack command job requires object `command_payload`.".to_string())?;
        let mode = required_text(target.get("mode"), "runtime_target.mode")?;
        let workflow_mode =
            required_text(target.get("workflow_mode"), "runtime_target.workflow_mode")?;
        if mode == "remote" {
            if !matches!(workflow_mode.as_str(), "solo_remote" | "team_remote") {
                return Err(
                    "Slack command job remote workflow_mode must be `solo_remote` or `team_remote`."
                        .to_string(),
                );
            }
            required_text(target.get("server_url"), "runtime_target.server_url")?;
        } else if workflow_mode != "solo_local" {
            return Err("Slack command job local workflow_mode must be `solo_local`.".to_string());
        }
        let local_reply = request.get("local_reply").filter(|value| !value.is_null());
        if local_reply.is_some_and(|value| !value.is_object()) {
            return Err("Slack command job local_reply must be an object.".to_string());
        }
        let timeout_seconds = request
            .get("timeout_seconds")
            .filter(|value| !value.is_null());
        if timeout_seconds.is_some_and(|value| !value.is_number())
            || timeout_seconds
                .and_then(JsonValue::as_f64)
                .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("Slack command job timeout_seconds must be positive.".to_string());
        }
        if request.get("defer_replies").and_then(JsonValue::as_bool) == Some(false) {
            return Err(
                "Slack command job requires deferred replies for response URL delivery."
                    .to_string(),
            );
        }
        Ok(Self {
            state_path: required_text(request.get("state_path"), "state_path")?,
            runtime_target: JsonValue::Object(target.clone()),
            repo_name: required_text(target.get("repo_name"), "runtime_target.repo_name")?,
            command_payload: JsonValue::Object(payload.clone()),
            channel_id: required_text(payload.get("channel_id"), "command_payload.channel_id")?,
            thread_id: clean_text(payload.get("thread_ts")),
            response_url: required_text(
                payload.get("response_url"),
                "command_payload.response_url",
            )?,
            ack_text: clean_text(request.get("ack_text"))
                .unwrap_or_else(|| DEFAULT_ACK_TEXT.to_string()),
            response_type: clean_text(request.get("response_type"))
                .unwrap_or_else(|| DEFAULT_RESPONSE_TYPE.to_string()),
            occurred_at: clean_text(request.get("occurred_at"))
                .or_else(|| clean_text(request.get("now_iso"))),
            timeout_seconds: timeout_seconds.cloned(),
            local_reply: local_reply.cloned(),
        })
    }

    fn binding_lookup_request(&self) -> JsonValue {
        json!({
            "transport": "slack",
            "surface_id": self.channel_id,
            "thread_id": optional_string_json(self.thread_id.as_deref()),
        })
    }
}

struct JobOutcome {
    state: &'static str,
    ok: bool,
    processed: bool,
    duplicate: bool,
    conversation_key: Option<String>,
    binding_created: bool,
    turn_ok: Option<bool>,
    delivery_attempted: bool,
    delivered: bool,
    recorded: bool,
    sequence: Option<i64>,
    error_kind: Option<String>,
    error: Option<String>,
}

impl JobOutcome {
    fn new(state: &'static str, ok: bool) -> Self {
        Self {
            state,
            ok,
            processed: false,
            duplicate: false,
            conversation_key: None,
            binding_created: false,
            turn_ok: None,
            delivery_attempted: false,
            delivered: false,
            recorded: false,
            sequence: None,
            error_kind: None,
            error: None,
        }
    }

    fn failure(state: &'static str, error_kind: &str, error: &str) -> Self {
        let mut outcome = Self::new(state, false);
        outcome.error_kind = Some(error_kind.to_string());
        outcome.error = Some(error.to_string());
        outcome
    }

    fn redacted(mut self, input: &CommandJobInput) -> Self {
        self.conversation_key = self
            .conversation_key
            .map(|value| redact_text(&value, &input.response_url));
        self.error = self
            .error
            .map(|value| redact_text(&value, &input.response_url));
        self
    }

    fn payload(self) -> JsonValue {
        json!({
            "contract": SLACK_COMMAND_JOB_CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "stage": "execute",
            "command_job_state": self.state,
            "ok": self.ok,
            "processed": self.processed,
            "duplicate": self.duplicate,
            "conversation_key": optional_string_json(self.conversation_key.as_deref()),
            "binding_created": self.binding_created,
            "turn_ok": self.turn_ok.map(JsonValue::Bool).unwrap_or(JsonValue::Null),
            "delivery_attempted": self.delivery_attempted,
            "delivered": self.delivered,
            "recorded": self.recorded,
            "sequence": self.sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "error_kind": optional_string_json(self.error_kind.as_deref()),
            "error": optional_string_json(self.error.as_deref()),
            "local_reply_generation_available": true,
            "python_state_mutation_allowed": false,
            "python_ait_runtime_allowed": false,
            "python_response_url_delivery_allowed": false,
        })
    }
}

fn non_submitted_outcome(plan: &Map<String, JsonValue>) -> JobOutcome {
    let duplicate = plan.get("duplicate").and_then(JsonValue::as_bool) == Some(true);
    let mut outcome = JobOutcome::new(if duplicate { "duplicate" } else { "ignored" }, true);
    outcome.duplicate = duplicate;
    outcome
}

fn state_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(state, "state", "Slack command state operation failed.")
}

fn backend_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "backend",
        "Slack command AIT backend operation failed.",
    )
}

fn backend_contract_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "backend_contract",
        "Slack command AIT backend returned an invalid payload.",
    )
}

fn delivery_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(state, "delivery", "Slack response delivery failed.")
}

fn delivery_contract_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "delivery_contract",
        "Slack response delivery returned an invalid payload.",
    )
}

fn contract_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "contract",
        "Slack command transaction returned an invalid payload.",
    )
}

fn with_conversation(
    mut outcome: JobOutcome,
    conversation_key: String,
    binding_created: bool,
) -> JobOutcome {
    outcome.conversation_key = Some(conversation_key);
    outcome.binding_created = binding_created;
    outcome
}

fn slack_conversation_key(channel_id: &str, thread_id: Option<&str>) -> String {
    format!("slack:{channel_id}:{}", thread_id.unwrap_or("root"))
}

fn redact_text(value: &str, response_url: &str) -> String {
    if response_url.is_empty() {
        value.to_string()
    } else {
        value.replace(response_url, REDACTED)
    }
}

fn required_plan_object(value: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| "Slack command ingress plan must be an object.".to_string())
}

fn plan_bool(plan: &Map<String, JsonValue>, key: &str) -> Result<bool, String> {
    plan.get(key)
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| format!("Slack command ingress plan requires boolean `{key}`."))
}

fn required_plan_text(plan: &Map<String, JsonValue>, key: &str) -> Result<String, String> {
    clean_text(plan.get(key))
        .ok_or_else(|| format!("Slack command ingress plan requires non-empty `{key}`."))
}

fn request_object(value: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| "Slack command job request must be an object.".to_string())
}

fn required_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Slack command job requires object `{field}`."))
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    clean_text(value).ok_or_else(|| format!("Slack command job requires non-empty `{field}`."))
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

#[cfg(test)]
mod tests;
