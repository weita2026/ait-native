use std::fs;
use std::sync::Mutex;

use tempfile::tempdir;

use super::*;

struct RecordingOperationalExecutor {
    result: Result<JsonValue, String>,
    requests: Mutex<Vec<JsonValue>>,
}

impl RecordingOperationalExecutor {
    fn returning(result: Result<JsonValue, String>) -> Self {
        Self {
            result,
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl TelegramUpdateOperationalExecutor for RecordingOperationalExecutor {
    fn execute_operational_trigger(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.lock().unwrap().push(request.clone());
        self.result.clone()
    }
}

struct RecordingReplyExecutor {
    result: Result<JsonValue, String>,
    requests: Mutex<Vec<JsonValue>>,
}

impl RecordingReplyExecutor {
    fn returning(result: Result<JsonValue, String>) -> Self {
        Self {
            result,
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl TelegramUpdateAssistantReplyExecutor for RecordingReplyExecutor {
    fn execute_assistant_reply(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.lock().unwrap().push(request.clone());
        self.result.clone()
    }
}

#[derive(Default)]
struct RecordingMessage {
    messages: Mutex<Vec<(JsonValue, String)>>,
    fail: bool,
}

impl TelegramUpdateOperationalMessagePort for RecordingMessage {
    fn send_operational_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String> {
        self.messages
            .lock()
            .unwrap()
            .push((chat_id.clone(), text.to_string()));
        if self.fail {
            Err("private-message-secret".to_string())
        } else {
            Ok(())
        }
    }
}

fn operational_request(command: Option<(&str, &str)>) -> TelegramUpdateOperationalRequest {
    TelegramUpdateOperationalRequest {
        chat_id: json!(42),
        chat: json!({"id": 42, "type": "private"}),
        from_user: json!({"id": 7, "username": "owner"}),
        chat_title: "Ada".to_string(),
        raw_text: "/router now".to_string(),
        normalized_text: "/router now".to_string(),
        command: command.map(|(name, args)| (name.to_string(), args.to_string())),
        telegram_message_id: Some(17),
        telegram_message_ids: vec![17, 18],
        reply_to_message: Some(json!({"message_id": 9})),
        attachments: vec![json!({"kind": "document", "telegram_file_id": "file-id"})],
        actor_identity: Some("telegram:7".to_string()),
        message: json!({"message_id": 17, "private": "request-secret"}),
    }
}

fn operational_outcome(
    ok: bool,
    matched: bool,
    handled: bool,
    assistant_event_sent: bool,
) -> JsonValue {
    let operation_count = usize::from(matched);
    json!({
        "contract": OPERATIONAL_CONTRACT,
        "migration_stage": OPERATIONAL_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "ok": ok,
        "completed": true,
        "matched": matched,
        "handled": handled,
        "operation_count": operation_count,
        "completed_operation_count": if ok { operation_count } else { 0 },
        "result_callback_planned": matched && ok,
        "assistant_event_sent": assistant_event_sent,
        "failure_message_sent": matched && !ok,
        "failure_kind": if matched && !ok { json!("operation") } else { JsonValue::Null },
        "python_executor_allowed": false,
    })
}

fn reply_outcome() -> JsonValue {
    json!({
        "contract": REPLY_CONTRACT,
        "migration_stage": REPLY_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "ok": true,
        "completed": true,
        "reply_delivery_state": "completed",
        "decision": "completed",
        "delivered": true,
        "operation_count": 1,
        "attempted_operation_count": 1,
        "delivered_operation_count": 1,
        "failed_operation_count": 0,
        "failed_operation_index": JsonValue::Null,
        "failed_operation_kind": JsonValue::Null,
        "error_kind": JsonValue::Null,
        "error": JsonValue::Null,
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
        "local_path_exposed": false,
    })
}

#[test]
fn typed_adapter_translates_complete_context_and_preserves_handled_decision() {
    for (outcome, expected) in [
        (operational_outcome(true, false, false, false), false),
        (operational_outcome(true, true, false, false), false),
        (operational_outcome(true, true, true, true), true),
        (operational_outcome(false, true, true, false), true),
    ] {
        let port = NativeTelegramUpdateOperationalPort::with_executor(
            RecordingOperationalExecutor::returning(Ok(outcome)),
        );
        assert_eq!(
            port.handle_operational_trigger(&operational_request(Some(("router", "now"))))
                .unwrap(),
            expected
        );
        let requests = port.executor.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["chat_id"], 42);
        assert_eq!(requests[0]["chat"]["type"], "private");
        assert_eq!(requests[0]["from_user"]["id"], 7);
        assert_eq!(requests[0]["chat_title"], "Ada");
        assert_eq!(requests[0]["context"]["raw_text"], "/router now");
        assert_eq!(requests[0]["context"]["normalized_text"], "/router now");
        assert_eq!(requests[0]["context"]["command"], json!(["router", "now"]));
        assert_eq!(requests[0]["context"]["telegram_message_id"], 17);
        assert_eq!(
            requests[0]["context"]["telegram_message_ids"],
            json!([17, 18])
        );
        assert_eq!(requests[0]["context"]["reply_to_message"]["message_id"], 9);
        assert_eq!(
            requests[0]["context"]["attachments"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(requests[0]["context"]["actor_identity"], "telegram:7");
        assert_eq!(requests[0]["context"]["message"]["message_id"], 17);
    }

    let port = NativeTelegramUpdateOperationalPort::with_executor(
        RecordingOperationalExecutor::returning(Ok(operational_outcome(true, false, false, false))),
    );
    port.handle_operational_trigger(&operational_request(None))
        .unwrap();
    assert!(port.executor.requests.lock().unwrap()[0]["context"]["command"].is_null());
}

#[test]
fn typed_adapter_rejects_invalid_requests_before_executor_invocation() {
    let mut invalid = Vec::new();
    let mut request = operational_request(None);
    request.chat_id = json!({"private": "secret"});
    invalid.push(request);
    let mut request = operational_request(None);
    request.chat = json!([]);
    invalid.push(request);
    let mut request = operational_request(None);
    request.command = Some((String::new(), "secret".to_string()));
    invalid.push(request);
    let mut request = operational_request(None);
    request.telegram_message_id = Some(-1);
    invalid.push(request);
    let mut request = operational_request(None);
    request.reply_to_message = Some(json!(["secret"]));
    invalid.push(request);
    let mut request = operational_request(None);
    request.attachments = vec![json!("secret")];
    invalid.push(request);
    let mut request = operational_request(None);
    request.actor_identity = Some("secret\ractor".to_string());
    invalid.push(request);
    let mut request = operational_request(None);
    request.message = json!(["secret"]);
    invalid.push(request);

    for request in invalid {
        let port = NativeTelegramUpdateOperationalPort::with_executor(
            RecordingOperationalExecutor::returning(Ok(operational_outcome(
                true, false, false, false,
            ))),
        );
        let failure = port.handle_operational_trigger(&request).unwrap_err();
        assert_eq!(
            failure.to_string(),
            "Telegram update execution port failed."
        );
        assert!(port.executor.requests.lock().unwrap().is_empty());
    }
}

#[test]
fn typed_adapter_rejects_executor_errors_and_every_corrupt_outcome_generically() {
    for (field, value) in [
        ("contract", json!("private-corrupt-secret")),
        ("python_executor_allowed", json!(true)),
        ("completed_operation_count", json!(99)),
        ("failure_kind", json!("private-kind-secret")),
    ] {
        let mut corrupt = operational_outcome(false, true, true, false);
        corrupt[field] = value;
        let port = NativeTelegramUpdateOperationalPort::with_executor(
            RecordingOperationalExecutor::returning(Ok(corrupt)),
        );
        let failure = port
            .handle_operational_trigger(&operational_request(None))
            .unwrap_err();
        assert_eq!(
            failure.to_string(),
            "Telegram update execution port failed."
        );
        assert!(!failure.to_string().contains("secret"));
    }

    let port = NativeTelegramUpdateOperationalPort::with_executor(
        RecordingOperationalExecutor::returning(Err("private-executor-secret".to_string())),
    );
    let failure = port
        .handle_operational_trigger(&operational_request(None))
        .unwrap_err();
    assert_eq!(
        failure.to_string(),
        "Telegram update execution port failed."
    );
    assert!(!failure.to_string().contains("secret"));
}

#[test]
fn native_delivery_propagates_transport_configuration_and_validates_outcomes() {
    let reply = RecordingReplyExecutor::returning(Ok(reply_outcome()));
    let message = RecordingMessage::default();
    let delivery = NativeTelegramOperationalTriggerDeliveryPort::with_ports(
        "123:private-bot-secret",
        Some(12.5),
        true,
        reply,
        message,
    )
    .unwrap();
    let event = json!({
        "event_type": "assistant.reply",
        "payload": {"text": "private reply", "transport_reply_envelope": {"message": {}}},
    });
    delivery
        .send_assistant_event_reply(&json!(42), &event)
        .unwrap();
    let requests = delivery.reply.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["chat_id"], 42);
    assert_eq!(requests[0]["assistant_event"], event);
    assert_eq!(requests[0]["bot_token"], "123:private-bot-secret");
    assert_eq!(requests[0]["request_timeout_seconds"], 12.5);
    assert_eq!(requests[0]["reply_markdown_enabled"], true);
    assert_eq!(requests[0]["should_execute"], true);
    drop(requests);

    delivery
        .send_failure_message(&json!(42), "generic failure")
        .unwrap();
    assert_eq!(
        *delivery.message.messages.lock().unwrap(),
        vec![(json!(42), "generic failure".to_string())]
    );
    let debug = format!("{delivery:?}");
    assert!(!debug.contains("private-bot-secret"));
}

#[test]
fn native_delivery_rejects_corruption_and_port_failures_without_secrets() {
    let mut corrupt = reply_outcome();
    corrupt["delivered_operation_count"] = json!(0);
    let delivery = NativeTelegramOperationalTriggerDeliveryPort::with_ports(
        "token",
        Some(5.0),
        false,
        RecordingReplyExecutor::returning(Ok(corrupt)),
        RecordingMessage::default(),
    )
    .unwrap();
    let failure = delivery
        .send_assistant_event_reply(
            &json!(42),
            &json!({"event_type": "assistant.reply", "payload": {}}),
        )
        .unwrap_err();
    assert_eq!(failure, operational_delivery_error());

    let delivery = NativeTelegramOperationalTriggerDeliveryPort::with_ports(
        "token",
        Some(5.0),
        false,
        RecordingReplyExecutor::returning(Err("private-reply-secret".to_string())),
        RecordingMessage {
            fail: true,
            ..RecordingMessage::default()
        },
    )
    .unwrap();
    for failure in [
        delivery
            .send_assistant_event_reply(
                &json!(42),
                &json!({"event_type": "assistant.reply", "payload": {}}),
            )
            .unwrap_err(),
        delivery
            .send_failure_message(&json!(42), "private-message-secret")
            .unwrap_err(),
    ] {
        assert_eq!(failure, operational_delivery_error());
        assert!(!failure.contains("secret"));
    }
}

#[test]
fn production_port_loads_native_registry_and_unmatched_request_has_no_side_effects() {
    let temp = tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".ait")).unwrap();
    let port = NativeTelegramUpdateOperationalPort::new(
        "private-repo-secret",
        temp.path(),
        temp.path().join(".ait/private-state-secret.json"),
        "123:private-token-secret",
        Some(5.0),
        true,
    )
    .unwrap();
    assert!(!port
        .handle_operational_trigger(&operational_request(None))
        .unwrap());
    let debug = format!("{port:?}");
    for secret in [
        "private-repo-secret",
        "private-state-secret",
        "private-token-secret",
        temp.path().to_string_lossy().as_ref(),
    ] {
        assert!(!debug.contains(secret));
    }

    let failure = NativeTelegramUpdateOperationalPort::new(
        "repo",
        temp.path(),
        temp.path().join(".ait/state.json"),
        "private-token-secret",
        Some(-1.0),
        false,
    )
    .unwrap_err();
    assert_eq!(failure, operational_configuration_error());
    assert!(!failure.contains("secret"));
}

#[test]
#[cfg(unix)]
fn production_port_loads_markdown_and_executes_native_unhandled_handler() {
    let temp = tempdir().unwrap();
    let trigger_directory = temp.path().join("docs/event_trigger");
    fs::create_dir_all(&trigger_directory).unwrap();
    fs::create_dir_all(temp.path().join(".ait")).unwrap();
    let marker = temp.path().join("native-handler-marker");
    let script = format!(
        "printf handled > '{}'; printf '{{}}'",
        marker.to_string_lossy()
    );
    let payload = json!({
        "kind": "telegram_operational_trigger",
        "id": "router",
        "handlerCommand": ["/bin/sh", "-c", script],
        "match": {"commands": ["router"]},
    });
    fs::write(
        trigger_directory.join("router.md"),
        format!("# Router\n\n```json\n{payload}\n```\n"),
    )
    .unwrap();

    let port = NativeTelegramUpdateOperationalPort::new(
        "fixture",
        temp.path(),
        temp.path().join(".ait/state.json"),
        "token-not-used",
        Some(5.0),
        false,
    )
    .unwrap();
    assert!(!port
        .handle_operational_trigger(&operational_request(Some(("router", "now"))))
        .unwrap());
    assert_eq!(fs::read_to_string(marker).unwrap(), "handled");
}
