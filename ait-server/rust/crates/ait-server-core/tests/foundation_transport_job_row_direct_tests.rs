use ait_server_core::foundation::transport::row_to_job;
use serde_json::{json, Map as JsonMap, Value as JsonValue};

fn job_row(row: JsonValue) -> JsonMap<String, JsonValue> {
    row.as_object().cloned().expect("row should be an object")
}

#[test]
fn payload_and_result_json_is_decoded_in_row_to_job() {
    let row = job_row(json!({
        "job_id": 7,
        "state": "succeeded",
        "payload_json": "{\"repo_name\": \"repo-a\", \"repack\": false}",
        "result_json": "{\"ok\": true, \"count\": 3}",
        "attempt_count": 1,
        "max_attempts": 3,
        "available_at": "2026-06-14T08:00:00+00:00",
        "last_error": null,
    }));

    let shaped = row_to_job(&row).expect("row should shape");
    assert_eq!(
        shaped.get("payload"),
        Some(&json!({"repo_name": "repo-a", "repack": false}))
    );
    assert_eq!(shaped.get("result"), Some(&json!({"ok": true, "count": 3})));
}

#[test]
fn attempts_remaining_is_computed_from_attempts_and_max_attempts() {
    let row = job_row(json!({
        "job_id": 8,
        "state": "queued",
        "payload_json": "{}",
        "result_json": "{}",
        "attempt_count": 2,
        "max_attempts": 5,
        "available_at": "2026-06-14T08:00:00+00:00",
        "last_error": "transient",
    }));

    let shaped = row_to_job(&row).expect("row should shape");
    assert_eq!(shaped.get("attempt_count"), Some(&json!(2)));
    assert_eq!(shaped.get("max_attempts"), Some(&json!(5)));
    assert_eq!(shaped.get("attempts_remaining"), Some(&json!(3)));
}

#[test]
fn retry_pending_and_next_retry_at_are_computed_for_queued_failed_rows() {
    let row = job_row(json!({
        "job_id": 9,
        "state": "queued",
        "payload_json": "{}",
        "result_json": "{}",
        "attempt_count": 1,
        "max_attempts": 3,
        "available_at": "2026-06-14T08:01:00+00:00",
        "last_error": "retry me",
    }));

    let shaped = row_to_job(&row).expect("row should shape");
    assert_eq!(shaped.get("retry_pending"), Some(&json!(true)));
    assert_eq!(
        shaped.get("next_retry_at"),
        Some(&json!("2026-06-14T08:01:00+00:00"))
    );
    assert_eq!(
        shaped.get("diagnostic_status"),
        Some(&json!("retry_pending"))
    );
}

#[test]
fn attempts_exhausted_is_reflected_in_row_to_job_shape() {
    let row = job_row(json!({
        "job_id": 10,
        "state": "failed",
        "payload_json": "{}",
        "result_json": "{}",
        "attempt_count": 3,
        "max_attempts": 3,
        "available_at": "2026-06-14T08:02:00+00:00",
        "last_error": "boom",
    }));

    let shaped = row_to_job(&row).expect("row should shape");
    assert_eq!(shaped.get("attempts_exhausted"), Some(&json!(true)));
    assert_eq!(
        shaped.get("diagnostic_status"),
        Some(&json!("exhausted_failed"))
    );
}

#[test]
fn diagnostic_status_routing_branches_are_covered() {
    let running = row_to_job(&job_row(json!({
        "state": "running",
        "payload_json": "{}",
        "result_json": "{}",
        "attempt_count": 1,
        "max_attempts": 3,
    })))
    .expect("running row should shape");
    assert_eq!(running.get("diagnostic_status"), Some(&json!("running")));

    let failed = row_to_job(&job_row(json!({
        "state": "failed",
        "payload_json": "{}",
        "result_json": "{}",
        "attempt_count": 1,
        "max_attempts": 3,
        "last_error": "boom",
    })))
    .expect("failed row should shape");
    assert_eq!(failed.get("diagnostic_status"), Some(&json!("failed")));

    let fallback = row_to_job(&job_row(json!({
        "state": "",
        "payload_json": "{}",
        "result_json": "{}",
        "attempt_count": 0,
        "max_attempts": 0,
    })))
    .expect("fallback row should shape");
    assert_eq!(fallback.get("diagnostic_status"), Some(&json!("unknown")));
}
