use std::cell::{Cell, RefCell};

use ait_core::json_support::{json, JsonValue};

use super::*;

struct StubDispatch {
    fail_call: Cell<Option<usize>>,
    requests: RefCell<Vec<JsonValue>>,
}

impl StubDispatch {
    fn new() -> Self {
        Self {
            fail_call: Cell::new(None),
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramWebhookTransactionDispatchPort for StubDispatch {
    fn dispatch_update(&self, request: &JsonValue) -> Result<(), String> {
        let call = self.requests.borrow().len() + 1;
        self.requests.borrow_mut().push(request.clone());
        if self.fail_call.get() == Some(call) {
            return Err("dispatch-port-secret private-update-text".to_string());
        }
        Ok(())
    }
}

struct StubIngress {
    result: RefCell<Option<Result<JsonValue, String>>>,
    requests: RefCell<Vec<JsonValue>>,
}

impl StubIngress {
    fn new(result: Result<JsonValue, String>) -> Self {
        Self {
            result: RefCell::new(Some(result)),
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramWebhookTransactionIngressPort for StubIngress {
    fn plan_ingress(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.borrow_mut().push(request.clone());
        self.result
            .borrow_mut()
            .take()
            .unwrap_or_else(|| Err("unexpected-ingress-secret".to_string()))
    }
}

fn execute_default(dispatch: &StubDispatch, request: &JsonValue) -> JsonValue {
    execute_with_telegram_webhook_transaction_ports(
        &DefaultTelegramWebhookTransactionIngressPort,
        dispatch,
        request,
    )
    .expect("Telegram webhook transaction")
}

#[test]
fn single_update_is_planned_and_dispatched_without_echoing_payload() {
    let dispatch = StubDispatch::new();
    let outcome = execute_default(
        &dispatch,
        &json!({
            "raw_payload": r#"{"update_id":42,"message":{"message_id":7,"chat":{"id":99},"text":"private-update-text"}}"#,
            "fallback_update_key_prefix": "webhook-main",
            "ignored_secret": "must-not-reach-ingress",
        }),
    );

    assert_eq!(outcome["contract"], CONTRACT);
    assert_eq!(outcome["transaction_state"], "completed");
    assert_eq!(outcome["http_status"], 200);
    assert_eq!(outcome["planned_update_count"], 1);
    assert_eq!(outcome["attempted_update_count"], 1);
    assert_eq!(outcome["dispatched_update_count"], 1);
    assert_eq!(outcome["last_update_id_observed"], 42);
    assert_eq!(outcome["cursor_mutated"], false);
    assert_eq!(outcome["python_service_entry_loop_allowed"], false);
    let request = &dispatch.requests.borrow()[0];
    assert_eq!(request["index"], 0);
    assert_eq!(request["dispatch_key"], "chat-99");
    assert_eq!(request["queue_key"], "chat-99");
    assert_eq!(request["update_key"], "update-42");
    assert_eq!(request["fallback_update_key"], "webhook-main-42");
    assert_eq!(request["update"]["message"]["text"], "private-update-text");
    assert!(!outcome.to_string().contains("private-update-text"));
    assert!(!outcome.to_string().contains("must-not-reach-ingress"));
}

#[test]
fn update_array_dispatches_in_planner_order_with_fallback_keys() {
    let dispatch = StubDispatch::new();
    let outcome = execute_default(
        &dispatch,
        &json!({
            "raw_payload": r#"[{"message":{"message_id":10}},{"update_id":2,"callback_query":{"id":"cb"}}]"#,
        }),
    );

    assert_eq!(outcome["dispatched_update_count"], 2);
    assert_eq!(outcome["last_update_id_observed"], 2);
    assert_eq!(dispatch.requests.borrow()[0]["update_key"], "message-10");
    assert_eq!(
        dispatch.requests.borrow()[0]["fallback_update_key"],
        "webhook-0"
    );
    assert_eq!(dispatch.requests.borrow()[1]["update_key"], "update-2");
    assert_eq!(
        dispatch.requests.borrow()[1]["fallback_update_key"],
        "webhook-2"
    );
    assert_eq!(dispatch.requests.borrow()[0]["index"], 0);
    assert_eq!(dispatch.requests.borrow()[1]["index"], 1);
}

#[test]
fn parser_rejections_are_http_400_and_never_dispatch() {
    for raw_payload in ["", "5", "[1]", "{"] {
        let dispatch = StubDispatch::new();
        let outcome = execute_default(&dispatch, &json!({"raw_payload": raw_payload}));
        assert_eq!(outcome["transaction_state"], "ingress_failed");
        assert_eq!(outcome["http_status"], 400);
        assert_eq!(outcome["retryable"], false);
        assert_eq!(outcome["attempted_update_count"], 0);
        assert!(dispatch.requests.borrow().is_empty());
    }

    let dispatch = StubDispatch::new();
    let secret = execute_default(&dispatch, &json!({"raw_payload": "{private-parser-secret"}));
    assert!(!secret.to_string().contains("private-parser-secret"));
}

#[test]
fn ingress_port_error_is_generic_and_secret_safe() {
    let ingress = StubIngress::new(Err("ingress-port-secret".to_string()));
    let dispatch = StubDispatch::new();
    let outcome = execute_with_telegram_webhook_transaction_ports(
        &ingress,
        &dispatch,
        &json!({"raw_payload": "{}"}),
    )
    .unwrap();

    assert_eq!(outcome["transaction_state"], "ingress_failed");
    assert!(!outcome.to_string().contains("ingress-port-secret"));
    assert!(dispatch.requests.borrow().is_empty());
}

#[test]
fn complete_ingress_contract_is_validated_before_any_dispatch() {
    let mut invalid = agent_telegram_webhook_ingress_plan_json(&json!({
        "raw_payload": r#"[{"update_id":1},{"update_id":2}]"#,
    }))
    .unwrap();
    invalid["python_ingress_allowed"] = JsonValue::Bool(true);
    invalid["updates"][0]["secret"] = json!("contract-secret");
    let ingress = StubIngress::new(Ok(invalid));
    let dispatch = StubDispatch::new();

    let outcome = execute_with_telegram_webhook_transaction_ports(
        &ingress,
        &dispatch,
        &json!({"raw_payload": "ignored"}),
    )
    .unwrap();

    assert_eq!(outcome["transaction_state"], "ingress_contract_invalid");
    assert_eq!(outcome["ingress_state"], "invalid");
    assert_eq!(outcome["http_status"], 500);
    assert_eq!(outcome["retryable"], true);
    assert!(dispatch.requests.borrow().is_empty());
    assert!(!outcome.to_string().contains("contract-secret"));
}

#[test]
fn dispatch_failure_stops_remaining_updates_and_is_secret_safe() {
    let dispatch = StubDispatch::new();
    dispatch.fail_call.set(Some(2));
    let outcome = execute_default(
        &dispatch,
        &json!({
            "raw_payload": r#"[{"update_id":1},{"update_id":2,"message":{"text":"private-update-text"}},{"update_id":3}]"#,
        }),
    );

    assert_eq!(outcome["transaction_state"], "dispatch_failed");
    assert_eq!(outcome["http_status"], 500);
    assert_eq!(outcome["retryable"], true);
    assert_eq!(outcome["planned_update_count"], 3);
    assert_eq!(outcome["attempted_update_count"], 2);
    assert_eq!(outcome["dispatched_update_count"], 1);
    assert_eq!(outcome["failed_update_count"], 1);
    assert_eq!(outcome["unattempted_update_count"], 1);
    assert_eq!(outcome["remaining_update_count"], 2);
    assert_eq!(dispatch.requests.borrow().len(), 2);
    assert!(!outcome.to_string().contains("dispatch-port-secret"));
    assert!(!outcome.to_string().contains("private-update-text"));
}

#[test]
fn mismatched_dispatch_index_and_counts_fail_before_dispatch() {
    let base = agent_telegram_webhook_ingress_plan_json(&json!({
        "raw_payload": r#"[{"update_id":1},{"update_id":2}]"#,
    }))
    .unwrap();
    for invalid in [
        {
            let mut invalid = base.clone();
            invalid["dispatch_items"][1]["index"] = json!(0);
            invalid
        },
        {
            let mut invalid = base.clone();
            invalid["dispatch_count"] = json!(1);
            invalid
        },
        {
            let mut invalid = base.clone();
            invalid["dispatch_items"][0]["update_key"] = json!("wrong-update-key");
            invalid
        },
        {
            let mut invalid = base.clone();
            invalid["last_update_id"] = json!(99);
            invalid
        },
    ] {
        let ingress = StubIngress::new(Ok(invalid));
        let dispatch = StubDispatch::new();
        let outcome = execute_with_telegram_webhook_transaction_ports(
            &ingress,
            &dispatch,
            &json!({"raw_payload": "ignored"}),
        )
        .unwrap();
        assert_eq!(outcome["transaction_state"], "ingress_contract_invalid");
        assert!(dispatch.requests.borrow().is_empty());
    }
}

#[test]
fn public_input_shape_is_rejected_before_ports() {
    let ingress = StubIngress::new(Err("must not run".to_string()));
    let dispatch = StubDispatch::new();
    let error =
        execute_with_telegram_webhook_transaction_ports(&ingress, &dispatch, &json!("bad request"))
            .unwrap_err();

    assert!(error.contains("must be an object"));
    assert!(ingress.requests.borrow().is_empty());
    assert!(dispatch.requests.borrow().is_empty());
}
