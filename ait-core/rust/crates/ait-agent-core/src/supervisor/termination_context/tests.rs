use std::fs;

use ait_core::json_support::json;
use tempfile::tempdir;

use super::*;

fn consume(path: Option<&Path>, expected_pid: i64, include_issuer_details: bool) -> JsonValue {
    consume_worker_termination_context_json(&json!({
        "path": path.map(|value| value.to_string_lossy().into_owned()),
        "expected_pid": expected_pid,
        "signal": 15,
        "include_issuer_details": include_issuer_details,
    }))
    .expect("consume result")
}

#[test]
fn termination_context_consumer_classifies_missing_and_absent_paths() {
    let temp = tempdir().expect("tempdir");
    let missing_path = consume(None, 42, false);
    assert_eq!(
        missing_path["contract"],
        AGENT_SUPERVISOR_TERMINATION_CONTEXT_CONTRACT
    );
    assert_eq!(missing_path["status"], "missing_path");
    assert_eq!(missing_path["consumed"], false);
    assert_eq!(missing_path["suffix"], "");

    let absent = consume(Some(&temp.path().join("missing.json")), 42, false);
    assert_eq!(absent["status"], "not_found");
    assert_eq!(absent["consumed"], false);
    assert_eq!(absent["removed"], false);
}

#[test]
fn termination_context_consumer_leaves_invalid_and_mismatched_files_untouched() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("termination.json");
    let cases = [
        ("{invalid", "invalid_json"),
        ("[]", "invalid_payload"),
        (r#"{"pid":"bad"}"#, "invalid_pid"),
        (r#"{"pid":41,"reason":"other"}"#, "pid_mismatch"),
    ];

    for (source, expected_status) in cases {
        fs::write(&path, source).expect("write context");
        let result = consume(Some(&path), 42, false);
        assert_eq!(result["status"], expected_status);
        assert_eq!(result["consumed"], false);
        assert_eq!(result["removed"], false);
        assert_eq!(
            fs::read_to_string(&path).expect("preserved context"),
            source
        );
    }
}

#[test]
fn termination_context_consumer_removes_matching_context_and_formats_short_suffix() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("termination.json");
    fs::write(
        &path,
        json!({
            "pid": "42",
            "reason": "cli_stop",
            "worker_name": "main",
            "issued_at": "2026-07-16T00:00:00Z",
            "issued_by_pid": 7,
        })
        .to_string(),
    )
    .expect("write context");

    let result = consume(Some(&path), 42, false);
    assert_eq!(result["status"], "consumed");
    assert_eq!(result["consumed"], true);
    assert_eq!(result["removed"], true);
    assert_eq!(
        result["suffix"],
        " (signal=15, reason=cli_stop, worker=main)"
    );
    assert_eq!(result["payload"]["issued_by_pid"], 7);
    assert!(!path.exists());

    let repeated = consume(Some(&path), 42, false);
    assert_eq!(repeated["status"], "not_found");
}

#[test]
fn termination_context_consumer_formats_opt_in_issuer_details() {
    let temp = tempdir().expect("tempdir");
    let path = temp.path().join("termination.json");
    fs::write(
        &path,
        json!({
            "pid": 42,
            "reason": "cli_stop",
            "worker_name": "telegram-main",
            "issued_at": "2026-07-16T00:00:00Z",
            "issued_by_pid": 7,
        })
        .to_string(),
    )
    .expect("write context");

    let result = consume(Some(&path), 42, true);
    assert_eq!(
        result["suffix"],
        " (signal=15, reason=cli_stop, worker=telegram-main, issued_at=2026-07-16T00:00:00Z, issued_by_pid=7)"
    );
}

#[test]
fn termination_context_consumer_validates_request_shape() {
    let invalid_pid = consume_worker_termination_context_json(&json!({
        "path": null,
        "expected_pid": 0,
        "signal": 15,
    }))
    .expect_err("invalid pid");
    assert!(invalid_pid.contains("expected_pid"));

    let invalid_flag = consume_worker_termination_context_json(&json!({
        "path": null,
        "expected_pid": 42,
        "signal": 15,
        "include_issuer_details": "yes",
    }))
    .expect_err("invalid flag");
    assert!(invalid_flag.contains("include_issuer_details"));
}

#[test]
fn termination_context_consumer_has_no_transport_or_environment_authority() {
    let source = include_str!("../termination_context.rs");
    for forbidden in [
        "std::env",
        "AIT_TELEGRAM",
        "AIT_DISCORD",
        "AIT_LINE",
        "AIT_SLACK",
        "TransportKind",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden dependency: {forbidden}"
        );
    }
}
