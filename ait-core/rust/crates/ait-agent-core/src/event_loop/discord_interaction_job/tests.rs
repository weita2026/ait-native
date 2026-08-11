use std::cell::{Cell, RefCell};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{
    execute_with_discord_interaction_job_ports, DiscordInteractionJobBackendPort,
    DiscordInteractionJobStatePort,
};

const CHANNEL_ID: &str = "998877665544332211";
const CONVERSATION_KEY: &str = "discord:998877665544332211";

fn interaction_payload() -> JsonValue {
    json!({
        "id": "112233445566778899",
        "type": 2,
        "token": "discord-token-secret",
        "application_id": "123456789012345678",
        "channel_id": CHANNEL_ID,
        "guild_id": "556677889900112233",
        "data": {
            "name": "ask",
            "type": 1,
            "options": [{"name": "text", "type": 3, "value": "Hello from Discord"}],
        },
        "member": {
            "user": {
                "id": "U-discord-1",
                "username": "weita",
                "global_name": "WeiTa",
            },
        },
    })
}

fn message_payload() -> JsonValue {
    json!({
        "id": "998899887777666655",
        "type": 0,
        "channel_id": CHANNEL_ID,
        "guild_id": "556677889900112233",
        "content": "Hello from Discord Gateway",
        "author": {
            "id": "U-discord-1",
            "username": "weita",
            "global_name": "WeiTa",
            "bot": false,
        },
        "attachments": [{
            "id": "778899001122334455",
            "filename": "question.txt",
            "content_type": "text/plain",
            "size": 42,
            "url": "https://cdn.discordapp.com/attachments/question.txt",
        }],
    })
}

fn request(payload: JsonValue) -> JsonValue {
    json!({
        "interaction_payload": payload,
        "state_path": "/tmp/discord-state.json",
        "application_id": "123456789012345678",
        "runtime_target": {
            "mode": "remote",
            "workflow_mode": "solo_remote",
            "repo_name": "ait",
            "remote_name": "origin",
            "server_url": "http://127.0.0.1:1",
        },
        "timeout_seconds": 30.0,
        "occurred_at": "2026-07-17T00:00:00Z",
    })
}

fn message_request(payload: JsonValue) -> JsonValue {
    let mut request = request(payload.clone());
    request
        .as_object_mut()
        .expect("message request")
        .remove("interaction_payload");
    request["message_payload"] = payload;
    request
}

fn binding(recent_interactions: Vec<&str>, recent_messages: Vec<&str>) -> JsonValue {
    json!({
        "transport": "discord",
        "surface_id": CHANNEL_ID,
        "conversation_key": CONVERSATION_KEY,
        "discord_recent_interaction_ids": recent_interactions,
        "discord_recent_message_ids": recent_messages,
        "last_synced_sequence": 12,
        "codex_thread_binding": {"thread_id": "codex-discord-1"},
    })
}

struct StubState {
    binding: RefCell<JsonValue>,
    calls: RefCell<Vec<(String, JsonValue)>>,
    fail_operation: RefCell<Option<String>>,
}

impl StubState {
    fn empty() -> Self {
        Self {
            binding: RefCell::new(JsonValue::Null),
            calls: RefCell::new(Vec::new()),
            fail_operation: RefCell::new(None),
        }
    }

    fn with_binding(binding: JsonValue) -> Self {
        Self {
            binding: RefCell::new(binding),
            calls: RefCell::new(Vec::new()),
            fail_operation: RefCell::new(None),
        }
    }

    fn operation_count(&self, operation: &str) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|(name, _)| name == operation)
            .count()
    }
}

fn merge_object(target: &mut Map<String, JsonValue>, source: &Map<String, JsonValue>) {
    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

impl DiscordInteractionJobStatePort for StubState {
    fn execute_state(
        &self,
        _path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.calls
            .borrow_mut()
            .push((operation.to_string(), request.clone()));
        if self.fail_operation.borrow().as_deref() == Some(operation) {
            return Err("state failed".to_string());
        }
        match operation {
            "get_binding" => Ok(self.binding.borrow().clone()),
            "upsert_binding" => {
                let mut binding = self
                    .binding
                    .borrow()
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                if let Some(object) = request.as_object() {
                    for key in [
                        "transport",
                        "surface_id",
                        "repo_name",
                        "surface_title",
                        "surface_kind",
                    ] {
                        if let Some(value) = object.get(key) {
                            binding.insert(key.to_string(), value.clone());
                        }
                    }
                }
                if let Some(updates) = request.get("updates").and_then(JsonValue::as_object) {
                    merge_object(&mut binding, updates);
                }
                binding
                    .entry("discord_recent_interaction_ids".to_string())
                    .or_insert_with(|| json!([]));
                binding
                    .entry("discord_recent_message_ids".to_string())
                    .or_insert_with(|| json!([]));
                let binding = JsonValue::Object(binding);
                *self.binding.borrow_mut() = binding.clone();
                Ok(binding)
            }
            "remember_recent_value" => {
                let mut binding = self
                    .binding
                    .borrow()
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                let recent_key = request["recent_key"].as_str().expect("recent value key");
                binding.insert(recent_key.to_string(), json!([request["value"].clone()]));
                if let Some(last_value_key) = request["last_value_key"].as_str() {
                    binding.insert(last_value_key.to_string(), request["value"].clone());
                }
                if let Some(sequence) = request.get("last_synced_sequence") {
                    binding.insert("last_synced_sequence".to_string(), sequence.clone());
                }
                if let Some(updates) = request.get("updates").and_then(JsonValue::as_object) {
                    merge_object(&mut binding, updates);
                }
                let binding = JsonValue::Object(binding);
                *self.binding.borrow_mut() = binding.clone();
                Ok(binding)
            }
            _ => Err(format!("unexpected state operation {operation}")),
        }
    }
}

struct StubBackend {
    calls: RefCell<Vec<JsonValue>>,
    malformed_turn: Cell<bool>,
    fail_turn: Cell<bool>,
}

impl StubBackend {
    fn new() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            malformed_turn: Cell::new(false),
            fail_turn: Cell::new(false),
        }
    }
}

impl DiscordInteractionJobBackendPort for StubBackend {
    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.borrow_mut().push(request.clone());
        if request["operation"] != "create_turn" {
            return Err("session backend operation is forbidden".to_string());
        }
        if self.fail_turn.get() {
            return Err("gateway unavailable".to_string());
        }
        if self.malformed_turn.get() {
            return Ok(json!({"ok": true, "payload": {"ok": true}}));
        }
        Ok(json!({
            "ok": true,
            "payload": {
                "ok": true,
                "conversation_key": request["arguments"]["conversation_key"].clone(),
                "reply_text": "Rust Discord reply",
                "provider_thread": {"thread_id": "codex-discord-2"},
                "turn_telemetry": {"ait_cli_commands": {"attempted": 1}},
            },
        }))
    }
}

#[test]
fn ping_returns_pong_without_state_or_backend_calls() {
    let state = StubState::empty();
    let backend = StubBackend::new();
    let outcome =
        execute_with_discord_interaction_job_ports(&state, &backend, &request(json!({"type": 1})))
            .unwrap();
    assert_eq!(outcome["interaction_job_state"], "pong");
    assert_eq!(outcome["response"], json!({"type": 1}));
    assert!(state.calls.borrow().is_empty());
    assert!(backend.calls.borrow().is_empty());
}

#[test]
fn new_interaction_uses_direct_gateway_and_records_local_delivery_cursor() {
    let state = StubState::empty();
    let backend = StubBackend::new();
    let outcome = execute_with_discord_interaction_job_ports(
        &state,
        &backend,
        &request(interaction_payload()),
    )
    .unwrap();

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["processed"], true);
    assert_eq!(outcome["conversation_key"], CONVERSATION_KEY);
    assert_eq!(outcome["binding_created"], true);
    assert_eq!(outcome["sequence"], 1);
    assert_eq!(outcome["response"]["data"]["content"], "Rust Discord reply");
    assert_eq!(outcome["delivery_request"]["reply_mode"], "interaction");
    assert_eq!(outcome["delivery_request"]["assistant_sequence"], 1);
    assert_eq!(
        outcome["delivery_request"]["operations"][0]["kind"],
        "edit_original_response"
    );
    assert_eq!(
        outcome["recovery_request"]["conversation_key"],
        CONVERSATION_KEY
    );
    assert_eq!(
        outcome["recovery_request"]["delivery_request"],
        outcome["delivery_request"]
    );
    assert_eq!(backend.calls.borrow().len(), 1);
    let turn = &backend.calls.borrow()[0];
    assert_eq!(turn["operation"], "create_turn");
    assert_eq!(turn["arguments"]["conversation_key"], CONVERSATION_KEY);
    assert!(!turn.to_string().contains("session_id"));
    assert_eq!(state.operation_count("remember_recent_value"), 2);
}

#[test]
fn new_gateway_message_plans_channel_delivery_without_server_access() {
    let state = StubState::empty();
    let backend = StubBackend::new();
    let outcome = execute_with_discord_interaction_job_ports(
        &state,
        &backend,
        &message_request(message_payload()),
    )
    .unwrap();
    assert_eq!(outcome["processed"], true);
    assert_eq!(outcome["conversation_key"], CONVERSATION_KEY);
    assert_eq!(outcome["delivery_request"]["reply_mode"], "channel_message");
    assert_eq!(
        outcome["delivery_request"]["operations"][0]["kind"],
        "send_channel_message"
    );
    let turn = &backend.calls.borrow()[0];
    assert_eq!(
        turn["arguments"]["payload"]["transport_envelope"]["message"]["attachments"][0]
            ["file_name"],
        "question.txt"
    );
    let binding = state.binding.borrow();
    assert_eq!(
        binding["discord_recent_message_ids"],
        json!(["998899887777666655"])
    );
    assert_eq!(
        binding["codex_thread_binding"]["thread_id"],
        "codex-discord-2"
    );
}

#[test]
fn duplicates_and_bot_or_webhook_messages_never_call_the_gateway() {
    let state = StubState::with_binding(binding(vec![], vec!["998899887777666655"]));
    let backend = StubBackend::new();
    let duplicate = execute_with_discord_interaction_job_ports(
        &state,
        &backend,
        &message_request(message_payload()),
    )
    .unwrap();
    assert_eq!(duplicate["duplicate"], true);
    assert!(backend.calls.borrow().is_empty());

    for mutation in ["bot", "webhook"] {
        let state = StubState::empty();
        let backend = StubBackend::new();
        let mut payload = message_payload();
        if mutation == "bot" {
            payload["author"]["bot"] = json!(true);
        } else {
            payload["webhook_id"] = json!("webhook-1");
        }
        let outcome =
            execute_with_discord_interaction_job_ports(&state, &backend, &message_request(payload))
                .unwrap();
        assert_eq!(outcome["processed"], false);
        assert!(state.calls.borrow().is_empty());
        assert!(backend.calls.borrow().is_empty());
    }
}

#[test]
fn existing_binding_resumes_its_codex_thread_without_server_validation() {
    let state = StubState::with_binding(binding(vec![], vec![]));
    let backend = StubBackend::new();
    let outcome = execute_with_discord_interaction_job_ports(
        &state,
        &backend,
        &request(interaction_payload()),
    )
    .unwrap();
    assert_eq!(outcome["conversation_key"], CONVERSATION_KEY);
    assert_eq!(outcome["binding_created"], false);
    assert_eq!(outcome["sequence"], 13);
    let turn = &backend.calls.borrow()[0];
    assert_eq!(
        turn["arguments"]["provider_thread"]["thread_id"],
        "codex-discord-1"
    );
    assert_eq!(backend.calls.borrow().len(), 1);
}

#[test]
fn duplicate_interaction_does_not_submit_another_turn() {
    let state = StubState::with_binding(binding(vec!["112233445566778899"], vec![]));
    let backend = StubBackend::new();
    let outcome = execute_with_discord_interaction_job_ports(
        &state,
        &backend,
        &request(interaction_payload()),
    )
    .unwrap();
    assert_eq!(outcome["duplicate"], true);
    assert_eq!(
        outcome["response"]["data"]["content"],
        "Duplicate Discord interaction ignored."
    );
    assert!(backend.calls.borrow().is_empty());
}

#[test]
fn fresh_topic_rotates_the_conversation_key_without_a_backend_turn() {
    let state = StubState::with_binding(binding(vec![], vec![]));
    let backend = StubBackend::new();
    let mut payload = interaction_payload();
    payload["id"] = json!("112233445566778900");
    payload["data"]["options"][0]["value"] = json!("換個話題");
    let outcome =
        execute_with_discord_interaction_job_ports(&state, &backend, &request(payload)).unwrap();
    assert_eq!(
        outcome["interaction_job_state"],
        "fresh_conversation_started"
    );
    assert_eq!(
        outcome["conversation_key"],
        "discord:998877665544332211:topic:112233445566778900"
    );
    assert_eq!(outcome["binding_created"], true);
    assert_eq!(backend.calls.borrow().len(), 0);
    let binding = state.binding.borrow();
    assert_eq!(binding["previous_conversation_key"], CONVERSATION_KEY);
    assert_eq!(binding["rotation_reason"], "fresh_topic_event_trigger");
}

#[test]
fn state_and_gateway_contract_failures_are_closed_and_generic() {
    let state = StubState::empty();
    *state.fail_operation.borrow_mut() = Some("get_binding".to_string());
    let backend = StubBackend::new();
    let state_failure = execute_with_discord_interaction_job_ports(
        &state,
        &backend,
        &request(interaction_payload()),
    )
    .unwrap();
    assert_eq!(
        state_failure["interaction_job_state"],
        "binding_read_failed"
    );
    assert_eq!(state_failure["error_kind"], "state");

    let state = StubState::empty();
    let backend = StubBackend::new();
    backend.malformed_turn.set(true);
    let backend_failure = execute_with_discord_interaction_job_ports(
        &state,
        &backend,
        &request(interaction_payload()),
    )
    .unwrap();
    assert_eq!(
        backend_failure["interaction_job_state"],
        "turn_payload_invalid"
    );
    assert_eq!(backend_failure["error_kind"], "backend_contract");
    assert_eq!(
        backend_failure["recovery_request"]["conversation_key"],
        CONVERSATION_KEY
    );
    assert!(backend_failure["recovery_request"]["delivery_request"].is_null());
    assert!(!backend_failure.to_string().contains("discord-token-secret"));
}

#[test]
fn local_application_command_forwards_provider_configuration() {
    let state = StubState::empty();
    let backend = StubBackend::new();
    let mut command = request(interaction_payload());
    command["runtime_target"]["mode"] = json!("local");
    command["runtime_target"]["workflow_mode"] = json!("solo_local");
    command["runtime_target"]["repo_root"] = json!("/tmp/discord-repo");
    command["runtime_target"]["server_url"] = JsonValue::Null;
    command["local_reply"] = json!({
        "program": "/usr/bin/codex",
        "args": ["exec", "--json"],
    });
    let outcome = execute_with_discord_interaction_job_ports(&state, &backend, &command).unwrap();
    assert_eq!(outcome["interaction_job_state"], "processed");
    let turn = &backend.calls.borrow()[0];
    assert_eq!(turn["target"]["mode"], "local");
    assert_eq!(turn["local_reply"]["program"], "/usr/bin/codex");
}
