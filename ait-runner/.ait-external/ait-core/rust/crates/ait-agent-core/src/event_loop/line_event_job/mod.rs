use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use chrono::{SecondsFormat, Utc};

use super::agent_line_reply_delivery_execute_json;
use crate::runtime::{agent_runtime_backend_execute_json, AgentRuntimeBindingStore};

const MIGRATION_STAGE: &str = "rust_agent_line_event_job_transaction";
const LINE_EVENT_JOB_CONTRACT: &str = "ait_agent_core.event_loop.LineEventJob.v1";
const RECENT_EVENT_KEY: &str = "line_recent_webhook_event_ids";
const LAST_EVENT_KEY: &str = "line_last_webhook_event_id";
const RECENT_EVENT_LIMIT: i64 = 64;

pub trait LineEventJobStatePort {
    fn execute_state(
        &self,
        path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String>;
}

pub trait LineEventJobBackendPort {
    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait LineEventJobDeliveryPort {
    fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultLineEventJobStatePort;

impl LineEventJobStatePort for DefaultLineEventJobStatePort {
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
pub struct DefaultLineEventJobBackendPort;

impl LineEventJobBackendPort for DefaultLineEventJobBackendPort {
    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_runtime_backend_execute_json(request)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultLineEventJobDeliveryPort;

impl LineEventJobDeliveryPort for DefaultLineEventJobDeliveryPort {
    fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_line_reply_delivery_execute_json(request)
    }
}

pub fn agent_line_event_job_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    execute_with_line_event_job_ports(
        &DefaultLineEventJobStatePort,
        &DefaultLineEventJobBackendPort,
        &DefaultLineEventJobDeliveryPort,
        request,
    )
}

pub fn execute_with_line_event_job_ports<S, B, D>(
    state: &S,
    backend: &B,
    delivery: &D,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    S: LineEventJobStatePort + ?Sized,
    B: LineEventJobBackendPort + ?Sized,
    D: LineEventJobDeliveryPort + ?Sized,
{
    let request = request_object(request)?;
    let event_plan = required_object(request.get("event_plan"), "event_plan")?;
    let should_submit_turn = required_bool(
        event_plan.get("should_submit_turn"),
        "event_plan.should_submit_turn",
    )?;
    if !should_submit_turn {
        return Ok(JobOutcome::new("ignored", true).payload());
    }

    let target = required_object(request.get("runtime_target"), "runtime_target")?;
    let mode = required_text(target.get("mode"), "runtime_target.mode")?;
    if !matches!(mode.as_str(), "remote" | "local") {
        return Err("LINE event job runtime_target.mode must be `remote` or `local`.".to_string());
    }

    let input = EventJobInput::parse(request, event_plan, target)?;
    Ok(execute_job(state, backend, delivery, &input)
        .redacted(&input)
        .payload())
}

fn execute_job<S, B, D>(state: &S, backend: &B, delivery: &D, input: &EventJobInput) -> JobOutcome
where
    S: LineEventJobStatePort + ?Sized,
    B: LineEventJobBackendPort + ?Sized,
    D: LineEventJobDeliveryPort + ?Sized,
{
    let binding_request = json!({
        "transport": "line",
        "surface_id": input.channel_id,
    });
    let duplicate_request = json!({
        "transport": "line",
        "surface_id": input.channel_id,
        "value": input.webhook_event_id,
        "recent_key": RECENT_EVENT_KEY,
    });
    let duplicate =
        match state.execute_state(&input.state_path, "has_recent_value", &duplicate_request) {
            Ok(value) => match value.as_bool() {
                Some(value) => value,
                None => return state_failure("duplicate_check_failed"),
            },
            Err(_) => return state_failure("duplicate_check_failed"),
        };
    if duplicate {
        let mut outcome = JobOutcome::new("duplicate", true);
        outcome.duplicate = true;
        return outcome;
    }

    let binding = match state.execute_state(&input.state_path, "get_binding", &binding_request) {
        Ok(JsonValue::Null) => JsonValue::Null,
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => return state_failure("binding_read_failed"),
    };

    let conversation_key = line_conversation_key(&input.channel_id);
    let binding_created = binding.is_null();
    let upsert_request = binding_upsert_request(input, &conversation_key);
    let updated_binding =
        match state.execute_state(&input.state_path, "upsert_binding", &upsert_request) {
            Ok(value) if value.is_object() => value,
            Ok(_) | Err(_) => {
                return with_conversation(
                    state_failure("binding_write_failed"),
                    conversation_key,
                    binding_created,
                )
            }
        };

    let turn_request = backend_request(
        input,
        "create_turn",
        &input.actor_identity,
        "line_user",
        json!({
            "conversation_key": conversation_key,
            "provider_thread": updated_binding
                .get("codex_thread_binding")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "payload": {
                "text": input.text,
                "surface": "line",
                "title": input.channel_title,
                "actor_display_name": optional_string_json(input.actor_display_name.as_deref()),
                "transport_envelope": input.transport_envelope,
            },
        }),
    );
    let turn = match backend_payload(backend, &turn_request) {
        Ok(value) => value,
        Err(_) => {
            return with_conversation(
                backend_failure("turn_backend_failed"),
                conversation_key,
                binding_created,
            )
        }
    };
    let turn_ok = match turn.get("ok").and_then(JsonValue::as_bool) {
        Some(value) => value,
        None => {
            return with_conversation(
                backend_contract_failure("turn_payload_invalid"),
                conversation_key,
                binding_created,
            )
        }
    };
    if clean_text(turn.get("conversation_key")).as_deref() != Some(conversation_key.as_str()) {
        return with_conversation(
            backend_contract_failure("turn_conversation_mismatch"),
            conversation_key,
            binding_created,
        );
    }

    if turn_ok {
        execute_successful_turn(
            state,
            delivery,
            input,
            &updated_binding,
            &conversation_key,
            binding_created,
            &turn,
        )
    } else {
        execute_failed_turn(
            state,
            delivery,
            input,
            &updated_binding,
            &conversation_key,
            binding_created,
            &turn,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_successful_turn<S, D>(
    state: &S,
    delivery: &D,
    input: &EventJobInput,
    binding: &JsonValue,
    conversation_key: &str,
    binding_created: bool,
    turn: &Map<String, JsonValue>,
) -> JobOutcome
where
    S: LineEventJobStatePort + ?Sized,
    D: LineEventJobDeliveryPort + ?Sized,
{
    let sequence = next_delivery_sequence(binding);
    let reply_text = clean_text(turn.get("reply_text"));
    let mut delivered = false;
    let mut delivery_attempted = false;
    if let Some(reply_text) = reply_text {
        delivery_attempted = true;
        if !delivery_succeeded(delivery, &delivery_request(input, &reply_text)) {
            let mut outcome = JobOutcome::failure(
                "reply_delivery_failed",
                "delivery",
                "LINE reply delivery failed.",
            );
            outcome.conversation_key = Some(conversation_key.to_string());
            outcome.binding_created = binding_created;
            outcome.turn_ok = Some(true);
            outcome.delivery_attempted = true;
            outcome.sequence = Some(sequence);
            return outcome;
        }
        delivered = true;
    }

    if !remember_processed_event(state, input, binding, turn, sequence) {
        let mut outcome = state_failure("processed_state_write_failed");
        outcome.conversation_key = Some(conversation_key.to_string());
        outcome.binding_created = binding_created;
        outcome.turn_ok = Some(true);
        outcome.delivery_attempted = delivery_attempted;
        outcome.delivered = delivered;
        outcome.sequence = Some(sequence);
        return outcome;
    }

    let mut outcome = JobOutcome::new("processed", true);
    outcome.processed = true;
    outcome.conversation_key = Some(conversation_key.to_string());
    outcome.binding_created = binding_created;
    outcome.turn_ok = Some(true);
    outcome.delivery_attempted = delivery_attempted;
    outcome.delivered = delivered;
    outcome.recorded = true;
    outcome.sequence = Some(sequence);
    outcome
}

#[allow(clippy::too_many_arguments)]
fn execute_failed_turn<S, D>(
    state: &S,
    delivery: &D,
    input: &EventJobInput,
    binding: &JsonValue,
    conversation_key: &str,
    binding_created: bool,
    turn: &Map<String, JsonValue>,
) -> JobOutcome
where
    S: LineEventJobStatePort + ?Sized,
    D: LineEventJobDeliveryPort + ?Sized,
{
    let sequence = next_delivery_sequence(binding);
    let error_text =
        clean_text(turn.get("error")).unwrap_or_else(|| "Unknown backend reply error.".to_string());
    if !remember_processed_event(state, input, binding, turn, sequence) {
        let mut outcome = state_failure("failed_turn_state_write_failed");
        outcome.conversation_key = Some(conversation_key.to_string());
        outcome.binding_created = binding_created;
        outcome.turn_ok = Some(false);
        outcome.sequence = Some(sequence);
        return outcome;
    }

    let reply_text = format!("The AI reply failed.\n{error_text}");
    let delivered = delivery_succeeded(delivery, &delivery_request(input, &reply_text));
    let mut outcome = if delivered {
        JobOutcome::new("turn_failed_reported", true)
    } else {
        JobOutcome::failure(
            "turn_failed_delivery_failed",
            "delivery",
            "LINE failed-turn notification delivery failed.",
        )
    };
    outcome.processed = true;
    outcome.conversation_key = Some(conversation_key.to_string());
    outcome.binding_created = binding_created;
    outcome.turn_ok = Some(false);
    outcome.delivery_attempted = true;
    outcome.delivered = delivered;
    outcome.recorded = true;
    outcome.sequence = Some(sequence);
    if delivered {
        outcome.error_kind = Some("ait_reply".to_string());
        outcome.error = Some(sanitize_public_text(&error_text, input));
    }
    outcome
}

fn remember_processed_event<S>(
    state: &S,
    input: &EventJobInput,
    binding: &JsonValue,
    turn: &Map<String, JsonValue>,
    sequence: i64,
) -> bool
where
    S: LineEventJobStatePort + ?Sized,
{
    let reply_seen_at = if input.reply_token.is_some() {
        JsonValue::String(Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true))
    } else {
        binding
            .get("line_last_reply_token_seen_at")
            .cloned()
            .unwrap_or(JsonValue::Null)
    };
    let request = json!({
        "transport": "line",
        "surface_id": input.channel_id,
        "value": input.webhook_event_id,
        "recent_key": RECENT_EVENT_KEY,
        "last_value_key": LAST_EVENT_KEY,
        "last_synced_sequence": sequence,
        "limit": RECENT_EVENT_LIMIT,
        "updates": {
            "line_last_message_id": optional_string_json(input.message_id.as_deref()),
            "line_last_source_user_id": optional_string_json(input.source_user_id.as_deref()),
            "line_last_reply_token_seen_at": reply_seen_at,
            "codex_thread_binding": turn
                .get("provider_thread")
                .filter(|value| value.is_object())
                .cloned()
                .unwrap_or(JsonValue::Null),
        },
    });
    matches!(
        state.execute_state(&input.state_path, "remember_recent_value", &request),
        Ok(value) if value.is_object()
    )
}

fn binding_upsert_request(input: &EventJobInput, conversation_key: &str) -> JsonValue {
    let reply_target = reply_target(input);
    json!({
        "transport": "line",
        "surface_id": input.channel_id,
        "repo_name": input.repo_name,
        "surface_title": input.channel_title,
        "surface_kind": optional_string_json(input.channel_kind.as_deref()),
        "updates": {
            "conversation_key": conversation_key,
            "line_channel_id": input.channel_id,
            "line_channel_title": input.channel_title,
            "line_channel_kind": optional_string_json(input.channel_kind.as_deref()),
            "line_source_user_id": optional_string_json(input.source_user_id.as_deref()),
            "line_reply_target": reply_target,
        },
    })
}

fn reply_target(input: &EventJobInput) -> JsonValue {
    let mut target = Map::new();
    target.insert(
        "channel_id".to_string(),
        JsonValue::String(input.channel_id.clone()),
    );
    if let Some(channel_kind) = &input.channel_kind {
        target.insert(
            "channel_kind".to_string(),
            JsonValue::String(channel_kind.clone()),
        );
    }
    if let Some(source_user_id) = &input.source_user_id {
        target.insert(
            "source_user_id".to_string(),
            JsonValue::String(source_user_id.clone()),
        );
    }
    JsonValue::Object(target)
}

fn backend_request(
    input: &EventJobInput,
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
    B: LineEventJobBackendPort + ?Sized,
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

fn delivery_request(input: &EventJobInput, text: &str) -> JsonValue {
    let mut request = json!({
        "channel_id": input.channel_id,
        "channel_access_token": input.channel_access_token,
        "text": text,
        "reply_token": optional_string_json(input.reply_token.as_deref()),
    });
    if let Some(api_base_url) = &input.api_base_url {
        request["api_base_url"] = JsonValue::String(api_base_url.clone());
    }
    if let Some(timeout_seconds) = &input.timeout_seconds {
        request["timeout_seconds"] = timeout_seconds.clone();
    }
    request
}

fn delivery_succeeded<D>(delivery: &D, request: &JsonValue) -> bool
where
    D: LineEventJobDeliveryPort + ?Sized,
{
    matches!(
        delivery.execute_delivery(request),
        Ok(value)
            if value.get("ok").and_then(JsonValue::as_bool) == Some(true)
                && value.get("delivered").and_then(JsonValue::as_bool) == Some(true)
    )
}

fn next_delivery_sequence(binding: &JsonValue) -> i64 {
    binding
        .get("last_synced_sequence")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        .saturating_add(1)
}

fn line_conversation_key(channel_id: &str) -> String {
    format!("line:{channel_id}")
}

struct EventJobInput {
    state_path: String,
    runtime_target: JsonValue,
    repo_name: String,
    channel_access_token: String,
    api_base_url: Option<String>,
    timeout_seconds: Option<JsonValue>,
    local_reply: Option<JsonValue>,
    channel_id: String,
    channel_title: String,
    channel_kind: Option<String>,
    source_user_id: Option<String>,
    message_id: Option<String>,
    reply_token: Option<String>,
    webhook_event_id: String,
    actor_identity: String,
    actor_display_name: Option<String>,
    text: String,
    transport_envelope: JsonValue,
}

impl EventJobInput {
    fn parse(
        request: &Map<String, JsonValue>,
        event: &Map<String, JsonValue>,
        target: &Map<String, JsonValue>,
    ) -> Result<Self, String> {
        let channel_id = required_text(event.get("channel_id"), "event_plan.channel_id")?;
        let channel_title =
            clean_text(event.get("channel_title")).unwrap_or_else(|| channel_id.clone());
        let channel_kind = clean_text(event.get("channel_kind"));
        let source_user_id = clean_text(event.get("source_user_id"));
        let transport_envelope = event
            .get("transport_envelope")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| {
                "LINE event job requires object `event_plan.transport_envelope`.".to_string()
            })?;
        let timeout_seconds = request
            .get("timeout_seconds")
            .filter(|value| !value.is_null());
        if timeout_seconds.is_some_and(|value| !value.is_number()) {
            return Err("LINE event job timeout_seconds must be a number.".to_string());
        }
        if timeout_seconds
            .and_then(JsonValue::as_f64)
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("LINE event job timeout_seconds must be positive.".to_string());
        }
        let mode = required_text(target.get("mode"), "runtime_target.mode")?;
        let workflow_mode =
            required_text(target.get("workflow_mode"), "runtime_target.workflow_mode")?;
        if mode == "remote" {
            if !matches!(workflow_mode.as_str(), "solo_remote" | "team_remote") {
                return Err(
                    "LINE event job remote workflow_mode must be `solo_remote` or `team_remote`."
                        .to_string(),
                );
            }
            required_text(target.get("server_url"), "runtime_target.server_url")?;
        } else if workflow_mode != "solo_local" {
            return Err("LINE event job local workflow_mode must be `solo_local`.".to_string());
        }
        let local_reply = request.get("local_reply").filter(|value| !value.is_null());
        if local_reply.is_some_and(|value| !value.is_object()) {
            return Err("LINE event job local_reply must be an object.".to_string());
        }
        Ok(Self {
            state_path: required_text(request.get("state_path"), "state_path")?,
            runtime_target: JsonValue::Object(target.clone()),
            repo_name: required_text(target.get("repo_name"), "runtime_target.repo_name")?,
            channel_access_token: required_text(
                request.get("channel_access_token"),
                "channel_access_token",
            )?,
            api_base_url: clean_text(request.get("api_base_url")),
            timeout_seconds: timeout_seconds.cloned(),
            local_reply: local_reply.cloned(),
            channel_id,
            channel_title,
            channel_kind,
            source_user_id,
            message_id: scalar_text(event.get("message_id")),
            reply_token: clean_text(event.get("reply_token")),
            webhook_event_id: required_text(
                event.get("webhook_event_id"),
                "event_plan.webhook_event_id",
            )?,
            actor_identity: required_text(
                event.get("actor_identity"),
                "event_plan.actor_identity",
            )?,
            actor_display_name: clean_text(event.get("actor_display_name")),
            text: required_text(event.get("text"), "event_plan.text")?,
            transport_envelope,
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

    fn redacted(mut self, input: &EventJobInput) -> Self {
        self.conversation_key = self
            .conversation_key
            .map(|value| sanitize_public_text(&value, input));
        self.error = self.error.map(|value| sanitize_public_text(&value, input));
        self
    }

    fn payload(self) -> JsonValue {
        json!({
            "contract": LINE_EVENT_JOB_CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "stage": "execute",
            "event_job_state": self.state,
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
            "python_line_api_allowed": false,
        })
    }
}

fn state_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(state, "state", "LINE event state operation failed.")
}

fn backend_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(state, "backend", "LINE event AIT backend operation failed.")
}

fn backend_contract_failure(state: &'static str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "backend_contract",
        "LINE event AIT backend returned an invalid payload.",
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

fn sanitize_public_text(value: &str, input: &EventJobInput) -> String {
    let mut sanitized = value.replace(&input.channel_access_token, "[redacted]");
    if let Some(reply_token) = &input.reply_token {
        sanitized = sanitized.replace(reply_token, "[redacted]");
    }
    sanitized
}

fn request_object(value: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| "LINE event job request must be an object.".to_string())
}

fn required_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("LINE event job requires object `{field}`."))
}

fn required_bool(value: Option<&JsonValue>, field: &str) -> Result<bool, String> {
    value
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| format!("LINE event job requires boolean `{field}`."))
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    clean_text(value).ok_or_else(|| format!("LINE event job requires non-empty `{field}`."))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn scalar_text(value: Option<&JsonValue>) -> Option<String> {
    clean_text(value).or_else(|| {
        value
            .filter(|value| value.is_number())
            .map(JsonValue::to_string)
    })
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}

#[cfg(test)]
mod tests;
