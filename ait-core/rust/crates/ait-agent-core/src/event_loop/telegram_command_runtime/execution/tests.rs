use std::sync::Mutex;

use super::*;
use crate::event_loop::{DefaultTelegramCommandRuntimePlanner, TelegramWorkflowQueryPlanner};
use ait_core::json_support::json;

#[derive(Default)]
struct Ports {
    delivered: Mutex<Vec<String>>,
}

impl TelegramCommandRuntimeReadPort for Ports {
    fn read_task_queue(&self) -> Result<JsonValue, String> {
        Err("unexpected read".to_string())
    }

    fn read_task(&self, _target_ref: &str) -> Result<JsonValue, String> {
        Err("unexpected read".to_string())
    }

    fn read_task_audit(&self, _target_ref: &str) -> Result<JsonValue, String> {
        Err("unexpected read".to_string())
    }

    fn read_change(&self, _target_ref: &str) -> Result<JsonValue, String> {
        Err("unexpected read".to_string())
    }
}

impl TelegramCommandRuntimeStatePort for Ports {
    fn load_binding(&self, _chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        Ok(Some(json!({
            "conversation_key": "telegram:123",
            "workflow_notifications_enabled": false
        })))
    }

    fn patch_chat(&self, _chat_id: &JsonValue, _patch: &JsonValue) -> Result<(), String> {
        Err("unexpected patch".to_string())
    }
}

impl TelegramCommandRuntimeClockPort for Ports {
    fn now_iso(&self) -> Result<String, String> {
        Ok("2026-07-19T00:00:00Z".to_string())
    }
}

impl TelegramCommandRuntimeDeliveryPort for Ports {
    fn send_message(&self, _chat_id: &JsonValue, text: &str) -> Result<(), String> {
        self.delivered.lock().unwrap().push(text.to_string());
        Ok(())
    }
}

struct NoWorkflowQuery;

impl TelegramWorkflowQueryPlanner for NoWorkflowQuery {
    fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Err("unexpected workflow query".to_string())
    }
}

fn request(name: &str) -> JsonValue {
    json!({
        "chat_id": 123,
        "chat": {"id": 123, "type": "private"},
        "from_user": {"id": 456},
        "chat_title": "Wei",
        "name": name,
        "args": ""
    })
}

fn config() -> JsonValue {
    json!({
        "repo_name": "ait",
        "runtime_mode": "local",
        "background_sync_enabled": false
    })
}

#[test]
fn ping_executes_without_session_state() {
    let ports = Ports::default();
    let result = execute_with_telegram_command_runtime_ports(
        &DefaultTelegramCommandRuntimePlanner,
        &NoWorkflowQuery,
        &ports,
        &ports,
        &ports,
        &ports,
        &config(),
        &request("ping"),
    )
    .expect("execute ping");

    assert_eq!(result["decision"], "message_delivered");
    assert_eq!(ports.delivered.lock().unwrap().as_slice(), ["pong"]);
    assert!(!result.to_string().contains("session"));
}

#[test]
fn retired_session_command_is_delivered_as_unknown() {
    let ports = Ports::default();
    let result = execute_with_telegram_command_runtime_ports(
        &DefaultTelegramCommandRuntimePlanner,
        &NoWorkflowQuery,
        &ports,
        &ports,
        &ports,
        &ports,
        &config(),
        &request("session"),
    )
    .expect("execute unknown command");

    assert_eq!(result["decision"], "message_delivered");
    assert_eq!(result["binding_loaded"], false);
}
