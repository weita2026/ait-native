use super::{
    agent_transport_websocket_handshake_plan_json, plan_with_websocket_handshake_planner,
    WebSocketHandshakePlanner,
};
use ait_core::json_support::{json, JsonValue};

const SAMPLE_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const SAMPLE_ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

#[test]
fn websocket_handshake_plans_slack_upgrade_request_without_python() {
    let planned = agent_transport_websocket_handshake_plan_json(&json!({
        "stage": "upgrade_request",
        "url": "wss://wss-primary.slack.com/link/?ticket=abc",
        "sec_websocket_key": SAMPLE_KEY,
        "subprotocols": ["slack-socket-mode"],
        "additional_headers": {
            "Origin": "https://slack.com",
            "User-Agent": "ait-agent"
        }
    }))
    .unwrap();

    assert_eq!(planned["stage"], "upgrade_request");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_transport_websocket_handshake_boundary"
    );
    assert_eq!(
        planned["websocket_handshake_contract"],
        "ait_agent_core.transport.WebSocketHandshake.v1"
    );
    assert_eq!(
        planned["websocket_handshake_state"],
        "upgrade_request_planned"
    );
    assert_eq!(planned["python_websocket_handshake_allowed"], false);
    assert_eq!(planned["execute_connect"], false);
    assert_eq!(planned["execute_tls"], false);
    assert_eq!(planned["execute_upgrade_write"], false);
    assert_eq!(planned["secure"], true);
    assert_eq!(planned["host"], "wss-primary.slack.com");
    assert_eq!(planned["port"], 443);
    assert_eq!(planned["host_header"], "wss-primary.slack.com");
    assert_eq!(planned["path_and_query"], "/link/?ticket=abc");
    assert_eq!(planned["expected_sec_websocket_accept"], SAMPLE_ACCEPT);

    let request_text = planned["request_text"].as_str().unwrap();
    assert!(request_text.starts_with("GET /link/?ticket=abc HTTP/1.1\r\n"));
    assert!(request_text.contains("Host: wss-primary.slack.com\r\n"));
    assert!(request_text.contains("Upgrade: websocket\r\n"));
    assert!(request_text.contains("Connection: Upgrade\r\n"));
    assert!(request_text.contains("Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"));
    assert!(request_text.contains("Sec-WebSocket-Version: 13\r\n"));
    assert!(request_text.contains("Sec-WebSocket-Protocol: slack-socket-mode\r\n"));
    assert!(request_text.contains("Origin: https://slack.com\r\n"));
    assert!(request_text.ends_with("\r\n\r\n"));
    assert_eq!(
        planned["actions"][0]["kind"],
        "write_websocket_upgrade_request"
    );
    assert_eq!(planned["actions"][0]["execute_write"], false);
    assert_eq!(
        planned["actions"][1]["kind"],
        "await_websocket_upgrade_response"
    );
}

#[test]
fn websocket_handshake_parses_localhost_port_and_plain_ws_url() {
    let planned = agent_transport_websocket_handshake_plan_json(&json!({
        "stage": "request",
        "url": "ws://localhost:8080/socket",
        "sec_websocket_key": SAMPLE_KEY,
    }))
    .unwrap();

    assert_eq!(planned["scheme"], "ws");
    assert_eq!(planned["secure"], false);
    assert_eq!(planned["host"], "localhost");
    assert_eq!(planned["port"], 8080);
    assert_eq!(planned["explicit_port"], true);
    assert_eq!(planned["host_header"], "localhost:8080");
    assert_eq!(planned["path_and_query"], "/socket");
    assert!(planned["request_text"]
        .as_str()
        .unwrap()
        .contains("Host: localhost:8080\r\n"));
}

#[test]
fn websocket_handshake_validates_switching_protocols_response_and_registration_action() {
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\n\
         Upgrade: websocket\r\n\
         Connection: keep-alive, Upgrade\r\n\
         Sec-WebSocket-Accept: {SAMPLE_ACCEPT}\r\n\
         \r\n"
    );

    let planned = agent_transport_websocket_handshake_plan_json(&json!({
        "stage": "validate_response",
        "sec_websocket_key": SAMPLE_KEY,
        "response_text": response,
        "websocket_fd": 42,
        "event_loop_token": "slack:T1",
        "worker_key": "slack-worker",
    }))
    .unwrap();

    assert_eq!(planned["websocket_handshake_state"], "upgrade_accepted");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["status_code"], 101);
    assert_eq!(planned["expected_sec_websocket_accept"], SAMPLE_ACCEPT);
    assert_eq!(planned["actual_sec_websocket_accept"], SAMPLE_ACCEPT);
    assert_eq!(planned["upgrade_valid"], true);
    assert_eq!(planned["registration_ready"], true);
    assert_eq!(planned["should_register_websocket"], true);
    assert_eq!(planned["execute_registration"], false);
    assert_eq!(planned["actions"][0]["kind"], "complete_websocket_upgrade");
    assert_eq!(
        planned["actions"][1]["kind"],
        "register_websocket_after_upgrade"
    );
    assert_eq!(planned["actions"][1]["websocket_fd"], 42);
    assert_eq!(planned["actions"][1]["event_loop_token"], "slack:T1");
    assert_eq!(planned["actions"][1]["execute_registration"], false);
}

#[test]
fn websocket_handshake_rejects_wrong_accept_hash_without_python_fallback() {
    let response = "HTTP/1.1 101 Switching Protocols\r\n\
                    Upgrade: websocket\r\n\
                    Connection: Upgrade\r\n\
                    Sec-WebSocket-Accept: wrong\r\n\
                    \r\n";

    let planned = agent_transport_websocket_handshake_plan_json(&json!({
        "stage": "response",
        "sec_websocket_key": SAMPLE_KEY,
        "response_text": response,
    }))
    .unwrap();

    assert_eq!(planned["websocket_handshake_state"], "upgrade_rejected");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["upgrade_valid"], false);
    assert_eq!(planned["should_close_websocket"], true);
    assert_eq!(planned["should_register_websocket"], false);
    assert_eq!(planned["python_websocket_handshake_allowed"], false);
    assert!(planned["error"]
        .as_str()
        .unwrap()
        .contains("Sec-WebSocket-Accept"));
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_handshake_rejection"
    );
    assert_eq!(planned["actions"][1]["kind"], "close_websocket");
}

#[test]
fn websocket_handshake_rejects_non_101_or_missing_upgrade_headers() {
    let response = "HTTP/1.1 200 OK\r\n\
                    Connection: keep-alive\r\n\
                    Sec-WebSocket-Accept: wrong\r\n\
                    \r\n";

    let planned = agent_transport_websocket_handshake_plan_json(&json!({
        "stage": "validate_response",
        "sec_websocket_key": SAMPLE_KEY,
        "response_text": response,
    }))
    .unwrap();

    assert_eq!(planned["websocket_handshake_state"], "upgrade_rejected");
    assert_eq!(planned["status_code"], 200);
    assert_eq!(planned["validation_errors"].as_array().unwrap().len(), 4);
    assert_eq!(planned["should_close_websocket"], true);
}

#[test]
fn websocket_handshake_blocks_core_header_overrides() {
    let planned = agent_transport_websocket_handshake_plan_json(&json!({
        "stage": "upgrade_request",
        "url": "wss://gateway.discord.gg/?v=10&encoding=json",
        "sec_websocket_key": SAMPLE_KEY,
        "additional_headers": {
            "Connection": "close"
        }
    }))
    .unwrap();

    assert_eq!(planned["websocket_handshake_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert!(planned["error"].as_str().unwrap().contains("Connection"));
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_handshake_configuration_error"
    );
}

#[test]
fn websocket_handshake_rejects_unsupported_scheme_and_invalid_key() {
    let bad_scheme = agent_transport_websocket_handshake_plan_json(&json!({
        "stage": "request",
        "url": "https://example.test/socket",
        "sec_websocket_key": SAMPLE_KEY,
    }))
    .unwrap();
    assert_eq!(
        bad_scheme["websocket_handshake_state"],
        "configuration_error"
    );
    assert_eq!(
        bad_scheme["error"],
        "unsupported WebSocket URL scheme `https`."
    );

    let bad_key = agent_transport_websocket_handshake_plan_json(&json!({
        "stage": "request",
        "url": "wss://example.test/socket",
        "sec_websocket_key": "bad",
    }))
    .unwrap();
    assert_eq!(bad_key["websocket_handshake_state"], "configuration_error");
    assert_eq!(
        bad_key["error"],
        "WebSocket `sec_websocket_key` must be valid base64."
    );
}

#[test]
fn websocket_handshake_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl WebSocketHandshakePlanner for SubstitutePlanner {
        fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "websocket_handshake_state": "substitute",
                "python_websocket_handshake_allowed": false,
            }))
        }
    }

    let planned = plan_with_websocket_handshake_planner(&SubstitutePlanner, &json!({})).unwrap();
    assert_eq!(planned["websocket_handshake_state"], "substitute");
    assert_eq!(planned["python_websocket_handshake_allowed"], false);
}
