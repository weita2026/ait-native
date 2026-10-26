use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::*;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
struct StubIngress {
    calls: Rc<RefCell<Vec<JsonValue>>>,
    result: Rc<RefCell<Option<Result<JsonValue, String>>>>,
}

impl StubIngress {
    fn new(result: Result<JsonValue, String>) -> Self {
        Self {
            calls: Rc::new(RefCell::new(Vec::new())),
            result: Rc::new(RefCell::new(Some(result))),
        }
    }
}

impl LineHttpTransactionIngressPort for StubIngress {
    fn plan_ingress(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.borrow_mut().push(request.clone());
        self.result
            .borrow_mut()
            .take()
            .expect("unexpected ingress call")
    }
}

#[derive(Clone)]
struct StubEventJobs {
    calls: Rc<RefCell<Vec<JsonValue>>>,
    results: Rc<RefCell<VecDeque<Result<JsonValue, String>>>>,
}

impl StubEventJobs {
    fn new(results: Vec<Result<JsonValue, String>>) -> Self {
        Self {
            calls: Rc::new(RefCell::new(Vec::new())),
            results: Rc::new(RefCell::new(results.into())),
        }
    }
}

impl LineHttpTransactionEventJobPort for StubEventJobs {
    fn execute_event_job(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.borrow_mut().push(request.clone());
        self.results
            .borrow_mut()
            .pop_front()
            .expect("unexpected event-job call")
    }
}

fn sign(raw_payload: &str, secret: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(raw_payload.as_bytes());
    STANDARD.encode(mac.finalize().into_bytes())
}

fn signed_request(raw_payload: &str) -> JsonValue {
    json!({
        "raw_payload": raw_payload,
        "signature": sign(raw_payload, "line-channel-secret"),
        "channel_secret": "line-channel-secret",
        "request_path": "/callback",
        "webhook_path": "/callback",
        "state_path": "/tmp/line-state.json",
        "runtime_target": {
            "mode": "remote",
            "workflow_mode": "solo_remote",
            "repo_name": "demo-repo",
            "remote_name": "origin",
            "server_url": "http://127.0.0.1:8088",
        },
        "channel_access_token": "line-access-secret",
        "api_base_url": "https://api.line.example",
        "timeout_seconds": 20,
    })
}

fn planned_events(events: JsonValue) -> JsonValue {
    let count = events.as_array().map(Vec::len).unwrap_or_default();
    json!({
        "should_handle_webhook": true,
        "http_status": 200,
        "write_json_response": true,
        "response": {"ok": true, "processed_events": count},
        "event_plans": events,
    })
}

fn ready_event(id: &str) -> JsonValue {
    json!({
        "should_submit_turn": true,
        "webhook_event_id": id,
    })
}

fn ignored_event() -> JsonValue {
    json!({"should_submit_turn": false})
}

fn processed_result() -> Result<JsonValue, String> {
    Ok(json!({
        "ok": true,
        "processed": true,
        "duplicate": false,
        "event_job_state": "processed",
    }))
}

fn duplicate_result() -> Result<JsonValue, String> {
    Ok(json!({
        "ok": true,
        "processed": false,
        "duplicate": true,
        "event_job_state": "duplicate",
    }))
}

#[test]
fn real_signed_ingress_preserves_event_envelope_for_event_job_port() {
    let raw_payload = r#"{"events":[{"type":"message","replyToken":"reply-token-1","webhookEventId":"event-1","source":{"type":"user","userId":"U-1"},"message":{"id":"101","type":"text","text":"hello"}}]}"#;
    let event_jobs = StubEventJobs::new(vec![processed_result()]);

    let result = execute_with_line_http_transaction_ports(
        &DefaultLineHttpTransactionIngressPort,
        &event_jobs,
        &signed_request(raw_payload),
    )
    .unwrap();

    assert_eq!(result["transaction_state"], "completed");
    assert_eq!(result["http_status"], 200);
    assert_eq!(result["processed_events"], 1);
    assert_eq!(
        result["response"],
        json!({"ok": true, "processed_events": 1})
    );
    let calls = event_jobs.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["event_plan"]["webhook_event_id"], "event-1");
    assert_eq!(
        calls[0]["event_plan"]["transport_envelope"]["message"]["text"],
        "hello"
    );
    assert!(calls[0].get("channel_secret").is_none());
    assert!(calls[0].get("signature").is_none());
}

#[test]
fn mixed_events_count_only_processed_jobs_and_preserve_order() {
    let ingress = StubIngress::new(Ok(planned_events(json!([
        ignored_event(),
        ready_event("event-duplicate"),
        ready_event("event-processed"),
    ]))));
    let event_jobs = StubEventJobs::new(vec![duplicate_result(), processed_result()]);

    let result =
        execute_with_line_http_transaction_ports(&ingress, &event_jobs, &signed_request("{}"))
            .unwrap();

    assert_eq!(result["planned_event_count"], 3);
    assert_eq!(result["submitted_event_count"], 2);
    assert_eq!(result["attempted_event_count"], 2);
    assert_eq!(result["ignored_event_count"], 1);
    assert_eq!(result["duplicate_event_count"], 1);
    assert_eq!(result["processed_events"], 1);
    assert_eq!(
        event_jobs.calls.borrow()[0]["event_plan"]["webhook_event_id"],
        "event-duplicate"
    );
    assert_eq!(
        event_jobs.calls.borrow()[1]["event_plan"]["webhook_event_id"],
        "event-processed"
    );
}

#[test]
fn ignored_only_payload_does_not_require_event_runtime_configuration() {
    let ingress = StubIngress::new(Ok(planned_events(json!([
        ignored_event(),
        ignored_event()
    ]))));
    let event_jobs = StubEventJobs::new(vec![]);

    let result = execute_with_line_http_transaction_ports(
        &ingress,
        &event_jobs,
        &json!({"raw_payload": "{}"}),
    )
    .unwrap();

    assert_eq!(result["ok"], true);
    assert_eq!(result["ignored_event_count"], 2);
    assert_eq!(result["processed_events"], 0);
    assert!(event_jobs.calls.borrow().is_empty());
}

#[test]
fn ingress_rejections_preserve_status_and_do_not_execute_events() {
    let event_jobs = StubEventJobs::new(vec![]);
    let mut invalid_signature = signed_request(r#"{"events":[]}"#);
    invalid_signature["signature"] = json!("bad-signature");

    let rejected = execute_with_line_http_transaction_ports(
        &DefaultLineHttpTransactionIngressPort,
        &event_jobs,
        &invalid_signature,
    )
    .unwrap();
    assert_eq!(rejected["transaction_state"], "ingress_rejected");
    assert_eq!(rejected["http_status"], 401);
    assert_eq!(rejected["response"]["ok"], false);
    assert!(event_jobs.calls.borrow().is_empty());

    let wrong_path = execute_with_line_http_transaction_ports(
        &DefaultLineHttpTransactionIngressPort,
        &event_jobs,
        &json!({"request_path": "/wrong", "webhook_path": "/callback"}),
    )
    .unwrap();
    assert_eq!(wrong_path["http_status"], 404);
    assert_eq!(wrong_path["write_json_response"], false);
    assert_eq!(wrong_path["response"], JsonValue::Null);
}

#[test]
fn malformed_payload_rejection_uses_ingress_owned_http_contract() {
    let event_jobs = StubEventJobs::new(vec![]);
    let request = signed_request("{}");

    let result = execute_with_line_http_transaction_ports(
        &DefaultLineHttpTransactionIngressPort,
        &event_jobs,
        &request,
    )
    .unwrap();

    assert_eq!(result["http_status"], 400);
    assert_eq!(result["ingress_state"], "missing_events");
    assert_eq!(
        result["response"]["error"],
        "LINE webhook payload must include an events list."
    );
}

#[test]
fn event_job_failure_stops_later_events_and_returns_stable_error() {
    let ingress = StubIngress::new(Ok(planned_events(json!([
        ready_event("event-1"),
        ready_event("event-2"),
        ready_event("event-3"),
    ]))));
    let event_jobs = StubEventJobs::new(vec![
        processed_result(),
        Ok(json!({
            "ok": false,
            "processed": false,
            "duplicate": false,
            "event_job_state": "reply_delivery_failed-line-access-secret",
            "error": "line-reply-secret",
        })),
    ]);

    let result =
        execute_with_line_http_transaction_ports(&ingress, &event_jobs, &signed_request("{}"))
            .unwrap();

    assert_eq!(result["transaction_state"], "event_job_failed");
    assert_eq!(result["http_status"], 400);
    assert_eq!(result["attempted_event_count"], 2);
    assert_eq!(result["processed_events"], 1);
    assert_eq!(event_jobs.calls.borrow().len(), 2);
    let public = result.to_string();
    assert!(!public.contains("line-access-secret"));
    assert!(!public.contains("line-reply-secret"));
}

#[test]
fn event_job_executor_and_contract_failures_are_distinct() {
    let ingress = StubIngress::new(Ok(planned_events(json!([ready_event("event-1")]))));
    let executor_failure = StubEventJobs::new(vec![Err("port secret".to_string())]);
    let failed = execute_with_line_http_transaction_ports(
        &ingress,
        &executor_failure,
        &signed_request("{}"),
    )
    .unwrap();
    assert_eq!(failed["transaction_state"], "event_job_failed");
    assert_eq!(failed["error_kind"], "event_job");
    assert!(!failed.to_string().contains("port secret"));

    let malformed_ingress = StubIngress::new(Ok(json!([])));
    let no_jobs = StubEventJobs::new(vec![]);
    let invalid = execute_with_line_http_transaction_ports(
        &malformed_ingress,
        &no_jobs,
        &signed_request("{}"),
    )
    .unwrap();
    assert_eq!(invalid["transaction_state"], "ingress_contract_invalid");
    assert_eq!(invalid["http_status"], 500);
}

#[test]
fn ingress_public_response_is_recursively_redacted() {
    let ingress = StubIngress::new(Ok(json!({
        "should_handle_webhook": false,
        "http_status": 400,
        "write_json_response": true,
        "webhook_ingress_state": "rejected",
        "error_kind": "config",
        "error": "line-channel-secret line-access-secret bad-signature",
        "response": {
            "ok": false,
            "error": "line-channel-secret line-access-secret bad-signature",
            "channel_access_token": "nested-secret",
        },
    })));
    let no_jobs = StubEventJobs::new(vec![]);
    let mut request = signed_request("{}");
    request["signature"] = json!("bad-signature");

    let result = execute_with_line_http_transaction_ports(&ingress, &no_jobs, &request).unwrap();

    let public = result.to_string();
    assert!(!public.contains("line-channel-secret"));
    assert!(!public.contains("line-access-secret"));
    assert!(!public.contains("bad-signature"));
    assert!(!public.contains("nested-secret"));
    assert_eq!(result["response"]["channel_access_token"], REDACTED);
}

#[test]
fn production_event_job_rejects_an_incomplete_local_target() {
    let raw_payload = r#"{"events":[{"type":"message","webhookEventId":"event-local","source":{"type":"user","userId":"U-local"},"message":{"id":"1","type":"text","text":"hello"}}]}"#;
    let mut request = signed_request(raw_payload);
    request["runtime_target"] = json!({"mode": "local"});

    let result = agent_line_http_transaction_execute_json(&request).unwrap();

    assert_eq!(result["transaction_state"], "event_job_failed");
    assert_eq!(result["http_status"], 400);
    assert_eq!(result["attempted_event_count"], 1);
    assert_eq!(result["processed_events"], 0);
}

#[test]
fn public_entrypoint_rejects_non_object_request() {
    assert_eq!(
        agent_line_http_transaction_execute_json(&json!([])).unwrap_err(),
        "LINE HTTP transaction request must be an object."
    );
}
