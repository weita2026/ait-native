use std::sync::{Arc, Mutex};

use super::*;

#[derive(Clone)]
struct StubBindings(Result<Vec<JsonValue>, String>);

impl TelegramBackgroundSyncBindingReadPort for StubBindings {
    fn list_active_telegram_bindings(&self) -> Result<Vec<JsonValue>, String> {
        self.0.clone()
    }
}

#[derive(Default)]
struct StubSubmissions {
    calls: Mutex<Vec<(String, JsonValue)>>,
    reject: bool,
}

impl TelegramBackgroundSyncSubmissionPort for StubSubmissions {
    fn submit_background_sync_for_chat(
        &self,
        queue_key: &str,
        chat_id: &JsonValue,
    ) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push((queue_key.to_string(), chat_id.clone()));
        if self.reject {
            Err("private submission failure".to_string())
        } else {
            Ok(())
        }
    }
}

fn callback() -> JsonValue {
    json!({
        "callback_kind": "run_background_sync_once",
        "callback_group": "background_sync"
    })
}

fn binding(chat_id: &str) -> JsonValue {
    json!({
        "binding_id": format!("telegram:{chat_id}"),
        "transport": "telegram",
        "surface_id": chat_id,
        "status": "active",
        "conversation_key": format!("telegram:{chat_id}")
    })
}

fn run(bindings: Vec<JsonValue>) -> (Result<usize, String>, Arc<StubSubmissions>) {
    let submissions = Arc::new(StubSubmissions::default());
    let service = NativeTelegramBackgroundSyncServicePort::with_ports(
        StubBindings(Ok(bindings)),
        Arc::clone(&submissions),
    );
    (service.run_background_sync_once(&callback()), submissions)
}

#[test]
fn empty_and_idle_binding_snapshots_schedule_no_jobs() {
    for bindings in [Vec::new(), vec![binding("100")]] {
        let (result, submissions) = run(bindings);
        assert_eq!(result.expect("run"), 0);
        assert!(submissions.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn notification_work_uses_binding_order_and_chat_queue_keys() {
    let mut first = binding("101");
    first["workflow_notifications_enabled"] = json!(true);
    let mut second = binding("102");
    second["workflow_notifications_enabled"] = json!(true);

    let (result, submissions) = run(vec![first, second]);
    assert_eq!(result.expect("run"), 2);
    assert_eq!(
        submissions.calls.lock().unwrap().as_slice(),
        [
            ("chat-101".to_string(), json!("101")),
            ("chat-102".to_string(), json!("102"))
        ]
    );
}

#[test]
fn duplicate_or_malformed_bindings_fail_before_submission() {
    for bindings in [
        vec![binding("100"), binding("100")],
        vec![json!({"transport": "telegram", "surface_id": "100"})],
        vec![json!({
            "binding_id": "discord:100",
            "transport": "discord",
            "surface_id": "100"
        })],
    ] {
        let (result, submissions) = run(bindings);
        assert_eq!(
            result.expect_err("invalid binding"),
            "Telegram background sync binding contract is invalid."
        );
        assert!(submissions.calls.lock().unwrap().is_empty());
    }
}

#[test]
fn callback_and_binding_read_errors_are_stable() {
    let submissions = Arc::new(StubSubmissions::default());
    let service = NativeTelegramBackgroundSyncServicePort::with_ports(
        StubBindings(Err("private read failure".to_string())),
        Arc::clone(&submissions),
    );
    assert_eq!(
        service.run_background_sync_once(&callback()).unwrap_err(),
        "Telegram background sync binding read failed."
    );
    assert_eq!(
        service.run_background_sync_once(&json!({})).unwrap_err(),
        "Telegram background sync service request is invalid."
    );
}

#[test]
fn submission_failure_is_redacted_and_stops_the_pass() {
    let mut work = binding("100");
    work["workflow_notifications_enabled"] = json!(true);
    let submissions = Arc::new(StubSubmissions {
        calls: Mutex::new(Vec::new()),
        reject: true,
    });
    let service = NativeTelegramBackgroundSyncServicePort::with_ports(
        StubBindings(Ok(vec![work])),
        Arc::clone(&submissions),
    );
    assert_eq!(
        service.run_background_sync_once(&callback()).unwrap_err(),
        "Telegram background sync submission failed."
    );
    assert_eq!(submissions.calls.lock().unwrap().len(), 1);
}

#[test]
fn runtime_binding_reader_lists_only_active_telegram_bindings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = AgentRuntimeBindingStore::new(temp.path().join("bindings.json"));
    for (transport, surface_id, status) in [
        ("telegram", "100", "active"),
        ("telegram", "101", "inactive"),
        ("discord", "200", "active"),
    ] {
        store
            .execute(
                "upsert_binding",
                &json!({
                    "transport": transport,
                    "surface_id": surface_id,
                    "repo_name": "ait",
                    "status": status,
                    "updates": {
                        "conversation_key": format!("{transport}:{surface_id}")
                    }
                }),
            )
            .expect("upsert");
    }

    let reader =
        RuntimeBindingTelegramBackgroundSyncReadPort::new(temp.path().join("bindings.json"))
            .expect("reader");
    let bindings = reader.list_active_telegram_bindings().expect("bindings");
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0]["surface_id"], "100");
    assert_eq!(bindings[0]["conversation_key"], "telegram:100");
}
