use std::sync::Mutex;

use super::*;
use tempfile::tempdir;

#[derive(Default)]
struct SessionlessRuntime {
    binding_requests: Mutex<Vec<JsonValue>>,
    backend_requests: Mutex<Vec<JsonValue>>,
    spool_requests: Mutex<Vec<JsonValue>>,
    delivery_requests: Mutex<Vec<JsonValue>>,
}

impl TelegramUpdateLifecycleRuntimePort for SessionlessRuntime {
    fn ensure_conversation_binding(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.binding_requests.lock().unwrap().push(request.clone());
        Ok(json!({"conversation_key": "telegram:7"}))
    }

    fn execute_turn_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.backend_requests.lock().unwrap().push(request.clone());
        Ok(json!({
            "contract": AGENT_RUNTIME_BACKEND_CONTRACT,
            "backend_contract": AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT,
            "operation": "create_telegram_turn",
            "backend": "gateway",
            "ok": true,
            "python_backend_selection_allowed": false,
            "payload": {
                "ok": true,
                "conversation_key": "telegram:7",
                "reply_text": "hello from Codex",
                "provider_thread": {"thread_id": "codex-thread-1"}
            }
        }))
    }

    fn mutate_reply_spool(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.spool_requests.lock().unwrap().push(request.clone());
        Ok(json!({
            "contract": REPLY_SPOOL_CONTRACT,
            "migration_stage": REPLY_SPOOL_STAGE,
            "stage": request["stage"],
            "completed": true,
            "ok": true,
            "applied": true,
            "python_reply_spool_allowed": false
        }))
    }

    fn deliver_assistant_reply(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.delivery_requests.lock().unwrap().push(request.clone());
        Ok(json!({
            "contract": REPLY_DELIVERY_CONTRACT,
            "migration_stage": REPLY_DELIVERY_STAGE,
            "stage": "execute",
            "reply_delivery_state": "completed",
            "ok": true,
            "completed": true,
            "delivered": true,
            "python_reply_delivery_allowed": false,
            "python_message_delivery_allowed": false,
            "python_attachment_delivery_allowed": false,
            "raw_planner_result_exposed": false,
            "raw_executor_result_exposed": false,
            "bot_token_exposed": false,
            "chat_id_exposed": false,
            "reply_text_exposed": false,
            "attachment_exposed": false,
            "telegram_description_exposed": false,
            "local_path_exposed": false
        }))
    }

    fn send_failure_message(&self, _chat_id: &JsonValue, _text: &str) -> Result<(), String> {
        Err("unexpected failure message".to_string())
    }

    fn execute_background_sync(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Err("unexpected background sync".to_string())
    }
}

fn normal_request() -> JsonValue {
    json!({
        "operation": "normal_turn",
        "chat_id": 7,
        "chat": {"id": 7, "type": "private", "title": "Private chat"},
        "from_user": {
            "id": 9,
            "username": "weita",
            "first_name": "Wei",
            "last_name": "Ta",
            "is_bot": false
        },
        "chat_title": "Private chat",
        "text": "private user message",
        "telegram_message_id": 44,
        "telegram_message_ids": [44, 45],
        "attachments": [],
        "actor_identity": "telegram:9:@weita",
        "defer_reply": true
    })
}

#[test]
fn normal_turn_uses_conversation_binding_and_direct_gateway_reply() {
    let runtime = SessionlessRuntime::default();
    let result = execute_with_telegram_update_lifecycle_runtime(
        &runtime,
        &DefaultTelegramWorkflowQueryPlanner,
        &normal_request(),
    )
    .expect("normal turn");

    assert_eq!(result["lifecycle_state"], "completed");
    let backend = runtime.backend_requests.lock().unwrap();
    assert_eq!(backend.len(), 1);
    assert_eq!(backend[0]["arguments"]["conversation_key"], "telegram:7");
    assert!(backend[0]["arguments"].get("session_id").is_none());
    assert!(backend[0]["arguments"].get("session").is_none());
    assert_eq!(runtime.delivery_requests.lock().unwrap().len(), 1);

    let spool = runtime.spool_requests.lock().unwrap();
    assert!(spool
        .iter()
        .any(|request| request.to_string().contains("conversation_key")));
    assert!(spool
        .iter()
        .all(|request| !request.to_string().contains("session_id")));
}

#[test]
fn wait_for_idle_is_local_and_sessionless() {
    let runtime = SessionlessRuntime::default();
    let result = execute_with_telegram_update_lifecycle_runtime(
        &runtime,
        &DefaultTelegramWorkflowQueryPlanner,
        &json!({"operation": "wait_for_idle", "timeout_seconds": 0.25}),
    )
    .expect("wait");

    assert_eq!(result["lifecycle_state"], "idle");
    assert_eq!(result["idle"], true);
    assert!(runtime.backend_requests.lock().unwrap().is_empty());
}

#[test]
fn native_runtime_projects_manifest_local_reply_into_the_gateway_request() {
    let temp = tempdir().expect("tempdir");
    let runtime = NativeTelegramUpdateLifecycleRuntime::new(
        temp.path().join("state.json"),
        AgentRuntimeTarget {
            mode: AgentRuntimeMode::Local,
            workflow_mode: crate::transport_config::AgentWorkflowMode::SoloLocal,
            repo_name: "fixture".to_string(),
            repo_root: temp.path().to_path_buf(),
            remote_name: None,
            server_url: None,
        },
        Some(20.0),
        Some(json!({"model": "fixture-model", "sandbox": "workspace-write"})),
        "telegram-token",
        None,
        true,
    )
    .expect("native runtime");

    let request = runtime.turn_backend_request(&json!({"operation": "create_telegram_turn"}));

    assert_eq!(request["local_reply"]["model"], "fixture-model");
    assert_eq!(request["local_reply"]["sandbox"], "workspace-write");
    assert_eq!(request["target"]["mode"], "local");
    assert_eq!(request["timeout_seconds"], 20.0);
}

#[test]
fn retired_session_operation_fails_closed() {
    let runtime = SessionlessRuntime::default();
    assert!(execute_with_telegram_update_lifecycle_runtime(
        &runtime,
        &DefaultTelegramWorkflowQueryPlanner,
        &json!({"operation": "sync_session", "session_id": "retired"}),
    )
    .is_err());
}
