use super::{
    agent_transport_retry_default_errnos_json, agent_transport_retry_default_markers_json,
    agent_transport_retry_default_server_read_markers_json, agent_transport_retry_delay_seconds,
    agent_transport_retry_is_loopback_url,
    agent_transport_retry_is_retryable_server_read_error_json,
    agent_transport_retry_is_retryable_transport_error_json, agent_transport_retry_timeout_phrase,
    agent_transport_retry_timeout_value,
};
use ait_core::json_support::json;

#[test]
fn retry_policy_preserves_default_constants_and_scalar_helpers() {
    assert_eq!(
        agent_transport_retry_default_errnos_json(),
        json!([54, 60, 61, 104, 110, 111])
    );
    assert_eq!(
        agent_transport_retry_default_markers_json(),
        json!([
            "timed out",
            "connection reset by peer",
            "remote end closed connection without response",
            "temporarily unavailable",
            "connection aborted",
            "broken pipe",
            "network is unreachable"
        ])
    );
    assert_eq!(
        agent_transport_retry_default_server_read_markers_json(),
        json!([
            "500 internal server error",
            "502 bad gateway",
            "503 service unavailable",
            "504 gateway timeout"
        ])
    );
    assert_eq!(
        agent_transport_retry_timeout_value(None, Some(5.0)),
        Some(5.0)
    );
    assert_eq!(
        agent_transport_retry_timeout_value(Some(2.0), Some(5.0)),
        Some(5.0)
    );
    assert_eq!(
        agent_transport_retry_timeout_value(Some(7.0), Some(5.0)),
        Some(7.0)
    );
    assert_eq!(agent_transport_retry_timeout_phrase(None), "");
    assert_eq!(
        agent_transport_retry_timeout_phrase(Some(7.0)),
        " after 7 seconds"
    );
    assert_eq!(
        agent_transport_retry_timeout_phrase(Some(0.00001)),
        " after 1e-05 seconds"
    );
    assert_eq!(agent_transport_retry_delay_seconds(0.5, 3), 4.0);
    assert_eq!(agent_transport_retry_delay_seconds(-5.0, 3), 0.0);
}

#[test]
fn retry_policy_detects_loopback_url_hosts_only() {
    assert!(agent_transport_retry_is_loopback_url(
        "http://localhost:8000/v1"
    ));
    assert!(agent_transport_retry_is_loopback_url("https://127.0.0.1"));
    assert!(agent_transport_retry_is_loopback_url(
        "http://[::1]:8080/path"
    ));
    assert!(agent_transport_retry_is_loopback_url("//localhost/path"));
    assert!(!agent_transport_retry_is_loopback_url("localhost:8000"));
    assert!(!agent_transport_retry_is_loopback_url("http://example.com"));
    assert!(!agent_transport_retry_is_loopback_url("http://[::1"));
}

#[test]
fn retry_policy_classifies_transport_errors_from_normalized_exception_chain() {
    assert!(
        agent_transport_retry_is_retryable_transport_error_json(&json!({
            "chain": [
                {"class_names": ["urllib.error.URLError"], "text": "<urlopen error timed out>"},
                {"class_names": ["TimeoutError", "OSError"], "errno": 60, "text": "timed out"}
            ]
        }))
        .expect("classification")
    );

    assert!(agent_transport_retry_is_retryable_transport_error_json(&json!({
        "chain": [{"class_names": ["ConnectionRefusedError", "OSError"], "errno": 61, "text": "connection refused"}]
    }))
    .expect("classification"));

    assert!(agent_transport_retry_is_retryable_transport_error_json(&json!({
        "chain": [{"class_names": ["RuntimeError"], "text": "Remote end closed connection without response"}]
    }))
    .expect("classification"));

    assert!(
        !agent_transport_retry_is_retryable_transport_error_json(&json!({
            "chain": [{"class_names": ["RuntimeError"], "errno": 61, "text": "not an os error"}]
        }))
        .expect("classification")
    );
}

#[test]
fn retry_policy_classifies_server_read_markers_on_root_error() {
    assert!(
        agent_transport_retry_is_retryable_server_read_error_json(&json!({
            "chain": [{"class_names": ["RuntimeError"], "text": "503 Service Unavailable"}]
        }))
        .expect("classification")
    );

    assert!(
        agent_transport_retry_is_retryable_server_read_error_json(&json!({
            "chain": [{"class_names": ["RemoteDisconnected"], "text": "closed"}]
        }))
        .expect("classification")
    );

    assert!(
        !agent_transport_retry_is_retryable_server_read_error_json(&json!({
            "chain": [
                {"class_names": ["RuntimeError"], "text": "400 bad request"},
                {"class_names": ["RuntimeError"], "text": "503 service unavailable"}
            ]
        }))
        .expect("classification")
    );
}
