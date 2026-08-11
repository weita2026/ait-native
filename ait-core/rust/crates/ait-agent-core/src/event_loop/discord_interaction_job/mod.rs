use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{
    agent_discord_ingress_runtime_plan_json, agent_discord_reply_delivery_execution_plan_json,
};
use crate::runtime::{agent_runtime_backend_execute_json, AgentRuntimeBindingStore};

const MIGRATION_STAGE: &str = "rust_agent_discord_interaction_job_transaction";
const DISCORD_INTERACTION_JOB_CONTRACT: &str = "ait_agent_core.event_loop.DiscordInteractionJob.v1";
const RECENT_INTERACTION_KEY: &str = "discord_recent_interaction_ids";
const LAST_INTERACTION_KEY: &str = "discord_last_interaction_id";
const RECENT_MESSAGE_KEY: &str = "discord_recent_message_ids";
const LAST_MESSAGE_KEY: &str = "discord_last_message_id";
const RECENT_INTERACTION_LIMIT: i64 = 64;

pub trait DiscordInteractionJobStatePort {
    fn execute_state(
        &self,
        path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String>;
}

pub trait DiscordInteractionJobBackendPort {
    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDiscordInteractionJobStatePort;

impl DiscordInteractionJobStatePort for DefaultDiscordInteractionJobStatePort {
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
pub struct DefaultDiscordInteractionJobBackendPort;

impl DiscordInteractionJobBackendPort for DefaultDiscordInteractionJobBackendPort {
    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_runtime_backend_execute_json(request)
    }
}

pub fn agent_discord_interaction_job_execute_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    execute_with_discord_interaction_job_ports(
        &DefaultDiscordInteractionJobStatePort,
        &DefaultDiscordInteractionJobBackendPort,
        request,
    )
}

pub fn execute_with_discord_interaction_job_ports<S, B>(
    state: &S,
    backend: &B,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    S: DiscordInteractionJobStatePort + ?Sized,
    B: DiscordInteractionJobBackendPort + ?Sized,
{
    let request = request_object(request)?;
    let target = required_object(request.get("runtime_target"), "runtime_target")?;
    let input = InteractionJobInput::parse(request, target)?;
    let initial_plan = plan_event(&input, false, None)?;
    let initial_plan = required_plan_object(&initial_plan, input.ingress_kind)?;

    let fresh_topic = initial_plan
        .get("fresh_topic")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if !plan_bool(initial_plan, "should_submit_turn")? && !fresh_topic {
        return Ok(
            with_planned_delivery(non_submitted_outcome(initial_plan), &input, None).payload(),
        );
    }
    let channel_id = required_plan_text(initial_plan, "channel_id")?;
    let event_id = required_plan_text(initial_plan, "event_id")?;
    let lookup_request = binding_lookup_request(&channel_id);
    let binding = match state.execute_state(&input.state_path, "get_binding", &lookup_request) {
        Ok(JsonValue::Null) => JsonValue::Null,
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => return Ok(state_failure("binding_read_failed").payload()),
    };

    if binding_has_recent_event(&binding, &event_id, input.ingress_kind) {
        let duplicate_plan = plan_event(&input, true, None)?;
        let duplicate_plan = required_plan_object(&duplicate_plan, input.ingress_kind)?;
        return Ok(
            with_planned_delivery(non_submitted_outcome(duplicate_plan), &input, None).payload(),
        );
    }

    if fresh_topic {
        return execute_fresh_topic(
            state,
            backend,
            &input,
            initial_plan,
            &binding,
            &channel_id,
            &event_id,
        );
    }

    let conversation_key = clean_text(binding.get("conversation_key"))
        .unwrap_or_else(|| discord_conversation_key(&channel_id, None));
    let binding_created = binding.is_null();
    let resolved_plan = initial_plan;

    if !upsert_binding(state, &input, resolved_plan, &conversation_key, None, None) {
        return Ok(with_conversation(
            state_failure("binding_write_failed"),
            conversation_key,
            binding_created,
        )
        .payload());
    }
    if !remember_event(state, &input, resolved_plan, None, None) {
        return Ok(with_conversation(
            state_failure("interaction_remember_failed"),
            conversation_key,
            binding_created,
        )
        .payload());
    }

    let turn_request = backend_request(
        &input,
        "create_turn",
        required_plan_text(resolved_plan, "actor_identity")?.as_str(),
        "discord_user",
        json!({
            "conversation_key": conversation_key,
            "provider_thread": binding
                .get("codex_thread_binding")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "payload": {
                "text": required_plan_text(resolved_plan, "text")?,
                "surface": "discord",
                "title": required_plan_text(resolved_plan, "channel_title")?,
                "actor_display_name": optional_string_json(
                    clean_text(resolved_plan.get("actor_display_name")).as_deref()
                ),
                "transport_envelope": resolved_plan
                    .get("transport_envelope")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            },
        }),
    );
    let turn = match backend_payload(backend, &turn_request) {
        Ok(value)
            if value.get("ok").and_then(JsonValue::as_bool).is_some()
                && clean_text(value.get("conversation_key")).as_deref()
                    == Some(conversation_key.as_str()) =>
        {
            value
        }
        Ok(_) => {
            return Ok(with_recovery_context(
                with_conversation(
                    backend_contract_failure("turn_payload_invalid"),
                    conversation_key,
                    binding_created,
                ),
                &input,
                resolved_plan,
            )
            .payload())
        }
        Err(_) => {
            return Ok(with_recovery_context(
                with_conversation(
                    backend_failure("turn_backend_failed"),
                    conversation_key,
                    binding_created,
                ),
                &input,
                resolved_plan,
            )
            .payload())
        }
    };
    let turn_ok = turn.get("ok").and_then(JsonValue::as_bool).unwrap_or(false);
    let sequence = next_delivery_sequence(&binding);
    let reply_text = turn_reply_text(&turn);
    let response_plan = plan_event(&input, false, Some(&reply_text))?;
    let response_plan = required_plan_object(&response_plan, input.ingress_kind)?;
    let response = match input.ingress_kind {
        DiscordIngressKind::Interaction => match response_plan.get("response") {
            Some(response) if response.is_object() => response.clone(),
            _ => {
                return Ok(with_conversation(
                    contract_failure("interaction_response_invalid"),
                    conversation_key,
                    binding_created,
                )
                .payload())
            }
        },
        DiscordIngressKind::Message => json!({
            "type": 4,
            "data": {"content": reply_text},
        }),
    };
    let recorded = remember_event(state, &input, response_plan, Some(sequence), Some(&turn));
    if !recorded {
        let mut outcome = with_conversation(
            state_failure("interaction_sequence_record_failed"),
            conversation_key,
            binding_created,
        );
        outcome.turn_ok = Some(turn_ok);
        outcome.sequence = Some(sequence);
        outcome.response = Some(response);
        return Ok(with_recovery_context(
            with_planned_delivery(outcome, &input, Some(sequence)),
            &input,
            resolved_plan,
        )
        .payload());
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
    outcome.recorded = true;
    outcome.sequence = Some(sequence);
    outcome.response = Some(response);
    if !turn_ok {
        outcome.error_kind = Some("ait_reply".to_string());
        outcome.error = Some("Discord turn failed and the failure was reported.".to_string());
    }
    Ok(with_recovery_context(
        with_planned_delivery(outcome, &input, Some(sequence)),
        &input,
        resolved_plan,
    )
    .payload())
}

#[allow(clippy::too_many_arguments)]
fn execute_fresh_topic<S, B>(
    state: &S,
    _backend: &B,
    input: &InteractionJobInput,
    initial_plan: &Map<String, JsonValue>,
    binding: &JsonValue,
    channel_id: &str,
    event_id: &str,
) -> Result<JsonValue, String>
where
    S: DiscordInteractionJobStatePort + ?Sized,
    B: DiscordInteractionJobBackendPort + ?Sized,
{
    let previous_conversation_key = clean_text(binding.get("conversation_key"));
    let conversation_key = discord_conversation_key(channel_id, Some(event_id));
    if initial_plan.get("fresh_topic").and_then(JsonValue::as_bool) != Some(true)
        || clean_text(initial_plan.get("channel_id")).as_deref() != Some(channel_id)
    {
        return Ok(with_conversation(
            contract_failure("fresh_topic_plan_invalid"),
            conversation_key,
            true,
        )
        .payload());
    }
    if !upsert_binding(
        state,
        input,
        initial_plan,
        &conversation_key,
        previous_conversation_key.as_deref(),
        Some("fresh_topic_event_trigger"),
    ) {
        return Ok(with_conversation(
            state_failure("fresh_binding_write_failed"),
            conversation_key,
            true,
        )
        .payload());
    }
    if !remember_event(state, input, initial_plan, None, None) {
        return Ok(with_conversation(
            state_failure("fresh_interaction_remember_failed"),
            conversation_key,
            true,
        )
        .payload());
    }
    let response = match input.ingress_kind {
        DiscordIngressKind::Interaction => initial_plan
            .get("response")
            .filter(|response| response.is_object())
            .cloned(),
        DiscordIngressKind::Message => initial_plan
            .get("send_channel_message")
            .and_then(|message| clean_text(message.get("text")))
            .map(|content| json!({"type": 4, "data": {"content": content}})),
    };
    let Some(response) = response else {
        return Ok(with_conversation(
            contract_failure("fresh_response_invalid"),
            conversation_key,
            true,
        )
        .payload());
    };
    let mut outcome = with_conversation(
        JobOutcome::new("fresh_conversation_started", true),
        conversation_key,
        true,
    );
    outcome.processed = true;
    outcome.recorded = true;
    outcome.response = Some(response);
    Ok(with_planned_delivery(outcome, input, None).payload())
}

fn plan_event(
    input: &InteractionJobInput,
    duplicate: bool,
    reply_text: Option<&str>,
) -> Result<JsonValue, String> {
    let mut request = json!({
        "stage": input.ingress_kind.stage(),
        "payload": input.event_payload,
        "config_application_id": input.application_id,
        "defer_replies": false,
    });
    if duplicate {
        request["duplicate"] = JsonValue::Bool(true);
    }
    if let Some(occurred_at) = &input.occurred_at {
        request["occurred_at"] = JsonValue::String(occurred_at.clone());
    }
    if let Some(reply_text) = reply_text {
        request["reply_text"] = JsonValue::String(reply_text.to_string());
    }
    agent_discord_ingress_runtime_plan_json(&request)
}

fn upsert_binding<S>(
    state: &S,
    input: &InteractionJobInput,
    plan: &Map<String, JsonValue>,
    conversation_key: &str,
    previous_conversation_key: Option<&str>,
    rotation_reason: Option<&str>,
) -> bool
where
    S: DiscordInteractionJobStatePort + ?Sized,
{
    let channel_id = match clean_text(plan.get("channel_id")) {
        Some(value) => value,
        None => return false,
    };
    let channel_title = clean_text(plan.get("channel_title")).unwrap_or_else(|| channel_id.clone());
    let channel_kind = clean_text(plan.get("channel_kind"));
    let source_user_id = clean_text(plan.get("source_user_id"));
    let guild_id = clean_text(plan.get("guild_id"));
    let application_id =
        clean_text(plan.get("application_id")).unwrap_or_else(|| input.application_id.clone());
    let request = json!({
        "transport": "discord",
        "surface_id": channel_id,
        "repo_name": input.repo_name,
        "surface_title": channel_title,
        "surface_kind": optional_string_json(channel_kind.as_deref()),
        "updates": {
            "conversation_key": conversation_key,
            "discord_source_user_id": optional_string_json(source_user_id.as_deref()),
            "discord_guild_id": optional_string_json(guild_id.as_deref()),
            "discord_application_id": application_id,
            "discord_reply_target": discord_reply_target(
                &channel_id,
                channel_kind.as_deref(),
                guild_id.as_deref(),
                &application_id,
                source_user_id.as_deref(),
            ),
            "previous_conversation_key": optional_string_json(previous_conversation_key),
            "rotation_reason": optional_string_json(rotation_reason),
        },
    });
    matches!(
        state.execute_state(&input.state_path, "upsert_binding", &request),
        Ok(value) if value.is_object()
    )
}

fn remember_event<S>(
    state: &S,
    input: &InteractionJobInput,
    plan: &Map<String, JsonValue>,
    last_synced_sequence: Option<i64>,
    turn: Option<&Map<String, JsonValue>>,
) -> bool
where
    S: DiscordInteractionJobStatePort + ?Sized,
{
    let Some(channel_id) = clean_text(plan.get("channel_id")) else {
        return false;
    };
    let Some(event_id) = clean_text(plan.get("event_id")) else {
        return false;
    };
    let (recent_key, last_value_key) = input.ingress_kind.dedup_keys();
    let mut request = json!({
        "transport": "discord",
        "surface_id": channel_id,
        "value": event_id,
        "recent_key": recent_key,
        "last_value_key": last_value_key,
        "limit": RECENT_INTERACTION_LIMIT,
        "updates": {
            "discord_last_source_user_id": plan
                .get("source_user_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "discord_last_guild_id": plan
                .get("guild_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "discord_last_command_name": input
                .ingress_kind
                .is_interaction()
                .then(|| command_name(&input.event_payload))
                .flatten()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
            "codex_thread_binding": turn
                .and_then(|turn| turn.get("provider_thread"))
                .filter(|value| value.is_object())
                .cloned()
                .unwrap_or(JsonValue::Null),
        },
    });
    if let Some(last_synced_sequence) = last_synced_sequence {
        request["last_synced_sequence"] = JsonValue::from(last_synced_sequence.max(0));
    }
    matches!(
        state.execute_state(&input.state_path, "remember_recent_value", &request),
        Ok(value) if value.is_object()
    )
}

fn binding_lookup_request(channel_id: &str) -> JsonValue {
    json!({
        "transport": "discord",
        "surface_id": channel_id,
    })
}

fn binding_has_recent_event(
    binding: &JsonValue,
    event_id: &str,
    ingress_kind: DiscordIngressKind,
) -> bool {
    let (recent_key, _) = ingress_kind.dedup_keys();
    binding
        .get(recent_key)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| clean_text(Some(value)))
        .any(|value| value == event_id)
}

fn discord_reply_target(
    channel_id: &str,
    channel_kind: Option<&str>,
    guild_id: Option<&str>,
    application_id: &str,
    source_user_id: Option<&str>,
) -> JsonValue {
    let mut target = Map::new();
    target.insert(
        "channel_id".to_string(),
        JsonValue::String(channel_id.to_string()),
    );
    target.insert(
        "application_id".to_string(),
        JsonValue::String(application_id.to_string()),
    );
    for (key, value) in [
        ("channel_kind", channel_kind),
        ("guild_id", guild_id),
        ("source_user_id", source_user_id),
    ] {
        if let Some(value) = value {
            target.insert(key.to_string(), JsonValue::String(value.to_string()));
        }
    }
    JsonValue::Object(target)
}

fn backend_request(
    input: &InteractionJobInput,
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
    B: DiscordInteractionJobBackendPort + ?Sized,
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

fn turn_reply_text(turn: &Map<String, JsonValue>) -> String {
    if turn.get("ok").and_then(JsonValue::as_bool) == Some(true) {
        return clean_text(turn.get("reply_text"))
            .unwrap_or_else(|| "ait completed the Discord turn without reply text.".to_string());
    }
    let error =
        clean_text(turn.get("error")).unwrap_or_else(|| "Unknown backend reply error.".to_string());
    format!("The AI reply failed.\n{error}")
}

fn next_delivery_sequence(binding: &JsonValue) -> i64 {
    binding
        .get("last_synced_sequence")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        .saturating_add(1)
}

fn discord_conversation_key(channel_id: &str, fresh_event_id: Option<&str>) -> String {
    match fresh_event_id {
        Some(event_id) => format!("discord:{channel_id}:topic:{event_id}"),
        None => format!("discord:{channel_id}"),
    }
}

fn command_name(payload: &JsonValue) -> Option<String> {
    payload
        .get("data")
        .and_then(|value| clean_text(value.get("name")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiscordIngressKind {
    Interaction,
    Message,
}

impl DiscordIngressKind {
    fn stage(self) -> &'static str {
        match self {
            Self::Interaction => "interaction",
            Self::Message => "message",
        }
    }

    fn is_interaction(self) -> bool {
        self == Self::Interaction
    }

    fn dedup_keys(self) -> (&'static str, &'static str) {
        match self {
            Self::Interaction => (RECENT_INTERACTION_KEY, LAST_INTERACTION_KEY),
            Self::Message => (RECENT_MESSAGE_KEY, LAST_MESSAGE_KEY),
        }
    }
}

struct InteractionJobInput {
    state_path: String,
    runtime_target: JsonValue,
    repo_name: String,
    application_id: String,
    ingress_kind: DiscordIngressKind,
    event_payload: JsonValue,
    occurred_at: Option<String>,
    timeout_seconds: Option<JsonValue>,
    local_reply: Option<JsonValue>,
}

impl InteractionJobInput {
    fn parse(
        request: &Map<String, JsonValue>,
        target: &Map<String, JsonValue>,
    ) -> Result<Self, String> {
        let mode = required_text(target.get("mode"), "runtime_target.mode")?;
        if !matches!(mode.as_str(), "remote" | "local") {
            return Err(
                "Discord interaction job runtime_target.mode must be `remote` or `local`."
                    .to_string(),
            );
        }
        let workflow_mode =
            required_text(target.get("workflow_mode"), "runtime_target.workflow_mode")?;
        if mode == "remote" {
            if !matches!(workflow_mode.as_str(), "solo_remote" | "team_remote") {
                return Err(
                    "Discord interaction job remote workflow_mode must be `solo_remote` or `team_remote`."
                        .to_string(),
                );
            }
            required_text(target.get("server_url"), "runtime_target.server_url")?;
        } else if workflow_mode != "solo_local" {
            return Err(
                "Discord interaction job local workflow_mode must be `solo_local`.".to_string(),
            );
        }
        let local_reply = request.get("local_reply").filter(|value| !value.is_null());
        if local_reply.is_some_and(|value| !value.is_object()) {
            return Err("Discord interaction job local_reply must be an object.".to_string());
        }
        let timeout_seconds = request
            .get("timeout_seconds")
            .filter(|value| !value.is_null());
        if timeout_seconds.is_some_and(|value| !value.is_number())
            || timeout_seconds
                .and_then(JsonValue::as_f64)
                .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err("Discord interaction job timeout_seconds must be positive.".to_string());
        }
        let interaction_payload = request
            .get("interaction_payload")
            .and_then(JsonValue::as_object);
        let message_payload = request
            .get("message_payload")
            .and_then(JsonValue::as_object);
        if interaction_payload.is_some() && message_payload.is_some() {
            return Err(
                "Discord event job accepts exactly one interaction or message payload.".to_string(),
            );
        }
        let (ingress_kind, payload) = match (interaction_payload, message_payload) {
            (Some(payload), None) => (DiscordIngressKind::Interaction, payload),
            (None, Some(payload)) => (DiscordIngressKind::Message, payload),
            (None, None) => {
                let payload = request
                    .get("payload")
                    .and_then(JsonValue::as_object)
                    .ok_or_else(|| {
                        "Discord event job requires an object interaction_payload or message_payload."
                            .to_string()
                    })?;
                let ingress_kind = match clean_text(request.get("ingress_kind")).as_deref() {
                    Some("message") => DiscordIngressKind::Message,
                    _ => DiscordIngressKind::Interaction,
                };
                (ingress_kind, payload)
            }
            (Some(_), Some(_)) => unreachable!(),
        };
        Ok(Self {
            state_path: required_text(request.get("state_path"), "state_path")?,
            runtime_target: JsonValue::Object(target.clone()),
            repo_name: required_text(target.get("repo_name"), "runtime_target.repo_name")?,
            application_id: required_text(request.get("application_id"), "application_id")?,
            ingress_kind,
            event_payload: JsonValue::Object(payload.clone()),
            occurred_at: clean_text(request.get("occurred_at"))
                .or_else(|| clean_text(request.get("now_iso"))),
            timeout_seconds: timeout_seconds.cloned(),
            local_reply: local_reply.cloned(),
        })
    }
}

struct JobOutcome {
    state: String,
    ok: bool,
    processed: bool,
    duplicate: bool,
    conversation_key: Option<String>,
    binding_created: bool,
    turn_ok: Option<bool>,
    recorded: bool,
    sequence: Option<i64>,
    response: Option<JsonValue>,
    delivery_request: Option<JsonValue>,
    recovery_request: Option<JsonValue>,
    error_kind: Option<String>,
    error: Option<String>,
}

impl JobOutcome {
    fn new(state: impl Into<String>, ok: bool) -> Self {
        Self {
            state: state.into(),
            ok,
            processed: false,
            duplicate: false,
            conversation_key: None,
            binding_created: false,
            turn_ok: None,
            recorded: false,
            sequence: None,
            response: None,
            delivery_request: None,
            recovery_request: None,
            error_kind: None,
            error: None,
        }
    }

    fn failure(state: impl Into<String>, error_kind: &str, error: &str) -> Self {
        let mut outcome = Self::new(state, false);
        outcome.error_kind = Some(error_kind.to_string());
        outcome.error = Some(error.to_string());
        outcome
    }

    fn payload(self) -> JsonValue {
        json!({
            "contract": DISCORD_INTERACTION_JOB_CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "stage": "execute",
            "interaction_job_state": self.state,
            "ok": self.ok,
            "processed": self.processed,
            "duplicate": self.duplicate,
            "conversation_key": optional_string_json(self.conversation_key.as_deref()),
            "binding_created": self.binding_created,
            "turn_ok": self.turn_ok.map(JsonValue::Bool).unwrap_or(JsonValue::Null),
            "recorded": self.recorded,
            "sequence": self.sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "response": self.response.unwrap_or(JsonValue::Null),
            "delivery_request": self.delivery_request.unwrap_or(JsonValue::Null),
            "recovery_request": self.recovery_request.unwrap_or(JsonValue::Null),
            "error_kind": optional_string_json(self.error_kind.as_deref()),
            "error": optional_string_json(self.error.as_deref()),
            "local_reply_generation_available": true,
            "python_state_mutation_allowed": false,
            "python_ait_runtime_allowed": false,
            "python_interaction_execution_allowed": false,
        })
    }
}

fn with_recovery_context(
    mut outcome: JobOutcome,
    _input: &InteractionJobInput,
    plan: &Map<String, JsonValue>,
) -> JobOutcome {
    let Some(conversation_key) = outcome.conversation_key.clone() else {
        return outcome;
    };
    let Some(pending_reply) = plan.get("pending_reply").filter(|value| value.is_object()) else {
        return outcome;
    };
    let watch_spec = plan.get("watch_spec").filter(|value| value.is_object());
    outcome.recovery_request = Some(json!({
        "conversation_key": conversation_key,
        "channel_id": plan.get("channel_id").cloned().unwrap_or(JsonValue::Null),
        "delivery_request": outcome.delivery_request.clone().unwrap_or(JsonValue::Null),
        "pending_reply": {
            "event_id": pending_reply
                .get("event_id")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "event_kind": pending_reply
                .get("event_kind")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "reply_mode": pending_reply
                .get("reply_mode")
                .cloned()
                .unwrap_or(JsonValue::Null),
        },
        "watch_spec": {
            "event_kind": watch_spec
                .and_then(|spec| spec.get("event_kind"))
                .cloned()
                .unwrap_or(JsonValue::Null),
            "recovery_attempts": watch_spec
                .and_then(|spec| spec.get("recovery_attempts"))
                .cloned()
                .unwrap_or(JsonValue::Null),
            "recovery_delay_seconds": watch_spec
                .and_then(|spec| spec.get("recovery_delay_seconds"))
                .cloned()
                .unwrap_or(JsonValue::Null),
        },
        "event_limit": 200,
    }));
    outcome
}

fn with_planned_delivery(
    mut outcome: JobOutcome,
    input: &InteractionJobInput,
    sequence: Option<i64>,
) -> JobOutcome {
    let Some(reply_text) = outcome
        .response
        .as_ref()
        .and_then(interaction_message_response_text)
    else {
        return outcome;
    };
    outcome.delivery_request = plan_interaction_delivery_request(input, &reply_text, sequence).ok();
    outcome
}

fn plan_interaction_delivery_request(
    input: &InteractionJobInput,
    reply_text: &str,
    sequence: Option<i64>,
) -> Result<JsonValue, String> {
    let interaction_token = if input.ingress_kind.is_interaction() {
        required_text(
            input.event_payload.get("token"),
            "interaction_payload.token",
        )?
    } else {
        String::new()
    };
    let channel_id = if input.ingress_kind == DiscordIngressKind::Message {
        required_text(
            input.event_payload.get("channel_id"),
            "message_payload.channel_id",
        )?
    } else {
        String::new()
    };
    let sequence = sequence.unwrap_or(0).max(0);
    let planned = agent_discord_reply_delivery_execution_plan_json(&json!({
        "stage": "request",
        "execution_request": {
            "reply_mode": if input.ingress_kind.is_interaction() {
                "interaction"
            } else {
                "channel_message"
            },
            "channel_id": channel_id,
            "application_id": input.application_id,
            "interaction_token": interaction_token,
            "reply_text": reply_text,
            "assistant_event": {},
            "assistant_sequence": sequence,
            "through_sequence": sequence,
        },
    }))?;
    if planned.get("should_execute").and_then(JsonValue::as_bool) != Some(true) {
        return Err("Discord interaction delivery planning did not admit execution.".to_string());
    }
    let request = planned
        .get("request")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "Discord interaction delivery planning omitted its request.".to_string())?;
    if request
        .get("operations")
        .and_then(JsonValue::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Err("Discord interaction delivery planning produced no operations.".to_string());
    }
    Ok(request)
}

fn interaction_message_response_text(response: &JsonValue) -> Option<String> {
    (response.get("type").and_then(JsonValue::as_i64) == Some(4))
        .then(|| {
            response
                .get("data")
                .and_then(|data| clean_text(data.get("content")))
        })
        .flatten()
}

fn non_submitted_outcome(plan: &Map<String, JsonValue>) -> JobOutcome {
    let duplicate = plan.get("duplicate").and_then(JsonValue::as_bool) == Some(true);
    let state = clean_text(plan.get("ingress_runtime_state"))
        .unwrap_or_else(|| if duplicate { "duplicate" } else { "ignored" }.to_string());
    let response = plan
        .get("response")
        .filter(|value| value.is_object())
        .cloned();
    let ok = plan
        .get("ok")
        .and_then(JsonValue::as_bool)
        .unwrap_or_else(|| response.is_some());
    let mut outcome = JobOutcome::new(state, ok);
    outcome.duplicate = duplicate;
    outcome.response = response;
    outcome
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

fn state_failure(state: &str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "state",
        "Discord interaction state operation failed.",
    )
}

fn backend_failure(state: &str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "backend",
        "Discord interaction AIT backend operation failed.",
    )
}

fn backend_contract_failure(state: &str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "backend_contract",
        "Discord interaction AIT backend returned an invalid payload.",
    )
}

fn contract_failure(state: &str) -> JobOutcome {
    JobOutcome::failure(
        state,
        "contract",
        "Discord interaction transaction contract validation failed.",
    )
}

fn request_object(value: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| "Discord interaction job request must be an object.".to_string())
}

fn required_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Discord interaction job requires object `{field}`."))
}

fn required_plan_object(
    value: &JsonValue,
    ingress_kind: DiscordIngressKind,
) -> Result<&Map<String, JsonValue>, String> {
    let plan = value
        .as_object()
        .ok_or_else(|| "Discord interaction planner returned a non-object payload.".to_string())?;
    if clean_text(plan.get("migration_stage")).as_deref()
        != Some("rust_agent_discord_ingress_runtime")
        || clean_text(plan.get("stage")).as_deref() != Some(ingress_kind.stage())
    {
        return Err("Discord event planner returned an invalid contract.".to_string());
    }
    Ok(plan)
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    clean_text(value).ok_or_else(|| format!("Discord interaction job requires `{field}`."))
}

fn required_plan_text(plan: &Map<String, JsonValue>, field: &str) -> Result<String, String> {
    clean_text(plan.get(field))
        .ok_or_else(|| format!("Discord interaction plan requires `{field}`."))
}

fn plan_bool(plan: &Map<String, JsonValue>, field: &str) -> Result<bool, String> {
    plan.get(field)
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| format!("Discord interaction plan requires boolean `{field}`."))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}

#[cfg(test)]
mod tests;
