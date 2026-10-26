use super::{
    agent_discord_gateway_runtime_plan_json, plan_with_discord_gateway_runtime_planner,
    DiscordGatewayRuntimePlanner,
};
use ait_core::json_support::{json, JsonValue};

const DISCORD_GUILD_MESSAGES_INTENT: i64 = 1 << 9;
const DISCORD_DIRECT_MESSAGES_INTENT: i64 = 1 << 12;
const DISCORD_MESSAGE_CONTENT_INTENT: i64 = 1 << 15;

#[test]
fn gateway_info_request_uses_bot_auth_and_default_user_agent() {
    let planned = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "gateway_info_request",
        "bot_token": "discord-bot-token",
        "discord_api_base_url": "https://discord.com/api/v10",
        "request_timeout_seconds": 20.0,
    }))
    .unwrap();

    assert_eq!(planned["stage"], "gateway_info_request");
    assert_eq!(planned["python_gateway_allowed"], false);
    assert_eq!(planned["request"]["method"], "GET");
    assert_eq!(
        planned["request"]["url"],
        "https://discord.com/api/v10/gateway/bot"
    );
    assert_eq!(
        planned["request"]["headers"]["Authorization"],
        "Bot discord-bot-token"
    );
    assert_eq!(planned["request"]["headers"]["User-Agent"], "curl/8.7.1");
    assert_eq!(planned["request"]["allow_retry"], true);
}

#[test]
fn gateway_url_reuses_resume_gateway_url_without_refetching() {
    let planned = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "gateway_url",
        "session_id": "discord-session-1",
        "resume_gateway_url": "wss://gateway.discord.gg",
    }))
    .unwrap();

    assert_eq!(planned["ok"], true);
    assert_eq!(planned["should_fetch_gateway_info"], false);
    assert_eq!(planned["gateway_source"], "resume_gateway_url");
    assert_eq!(planned["gateway_base_url"], "wss://gateway.discord.gg");
    assert_eq!(
        planned["gateway_socket_url"],
        "wss://gateway.discord.gg?v=10&encoding=json"
    );
    assert_eq!(planned["actions"][0]["kind"], "use_resume_gateway_url");
}

#[test]
fn gateway_url_fetches_gateway_info_without_resume_url() {
    let planned = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "gateway_url",
        "session_id": "discord-session-1",
        "gateway_info": {"url": "wss://gateway-us-east1.discord.gg"},
    }))
    .unwrap();

    assert_eq!(planned["ok"], true);
    assert_eq!(planned["should_fetch_gateway_info"], true);
    assert_eq!(planned["gateway_source"], "gateway_info");
    assert_eq!(
        planned["gateway_socket_url"],
        "wss://gateway-us-east1.discord.gg?v=10&encoding=json"
    );
    assert_eq!(planned["actions"][0]["kind"], "fetch_gateway_info");
    assert_eq!(planned["actions"][1]["kind"], "connect_gateway");
}

#[test]
fn handshake_identifies_after_hello_without_resume_state() {
    let intents = DISCORD_GUILD_MESSAGES_INTENT
        | DISCORD_DIRECT_MESSAGES_INTENT
        | DISCORD_MESSAGE_CONTENT_INTENT;
    let planned = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "handshake",
        "hello_payload": {"op": 10, "d": {"heartbeat_interval": 41250}},
        "bot_token": "discord-bot-token",
        "gateway_intents": intents,
        "platform": "linux",
        "now_monotonic_seconds": 100.0,
    }))
    .unwrap();

    assert_eq!(planned["heartbeat_interval_ms"], 41250);
    assert_eq!(planned["heartbeat_interval_seconds"], 41.25);
    assert_eq!(planned["next_heartbeat_at"], 141.25);
    assert_eq!(planned["should_identify"], true);
    assert_eq!(planned["should_resume"], false);
    assert_eq!(planned["outbound_payload"]["op"], 2);
    assert_eq!(
        planned["outbound_payload"]["d"]["token"],
        "discord-bot-token"
    );
    assert_eq!(planned["outbound_payload"]["d"]["intents"], intents);
    assert_eq!(
        planned["outbound_payload"]["d"]["properties"]["browser"],
        "ait-agent"
    );
}

#[test]
fn handshake_resumes_when_session_and_sequence_are_available() {
    let planned = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "handshake",
        "hello_payload": {"op": 10, "d": {"heartbeat_interval": 1000}},
        "bot_token": "discord-bot-token",
        "session_id": "discord-session-1",
        "sequence": 77,
    }))
    .unwrap();

    assert_eq!(planned["should_resume"], true);
    assert_eq!(planned["should_identify"], false);
    assert_eq!(planned["outbound_payload"]["op"], 6);
    assert_eq!(
        planned["outbound_payload"]["d"]["session_id"],
        "discord-session-1"
    );
    assert_eq!(planned["outbound_payload"]["d"]["seq"], 77);
}

#[test]
fn heartbeat_tick_sends_heartbeat_or_requests_reconnect_on_ack_timeout() {
    let heartbeat = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "tick",
        "now_monotonic_seconds": 10.0,
        "next_heartbeat_at": 10.0,
        "heartbeat_interval_seconds": 4.0,
        "heartbeat_acknowledged": true,
        "sequence": 42,
    }))
    .unwrap();

    assert_eq!(heartbeat["should_send_heartbeat"], true);
    assert_eq!(heartbeat["heartbeat_acknowledged"], false);
    assert_eq!(heartbeat["next_heartbeat_at"], 14.0);
    assert_eq!(heartbeat["outbound_payload"], json!({"op": 1, "d": 42}));

    let timeout = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "tick",
        "now_monotonic_seconds": 14.0,
        "next_heartbeat_at": 14.0,
        "heartbeat_interval_seconds": 4.0,
        "heartbeat_acknowledged": false,
        "sequence": 42,
    }))
    .unwrap();

    assert_eq!(timeout["ok"], false);
    assert_eq!(timeout["should_reconnect"], true);
    assert_eq!(timeout["reconnect_reason"], "heartbeat_ack_timeout");
    assert_eq!(timeout["actions"][0]["kind"], "reconnect_gateway");
}

#[test]
fn gateway_payload_handles_reconnect_and_invalid_session_reset() {
    let reconnect = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "payload",
        "payload": {"op": 7, "s": 91, "d": null},
        "sequence": 90,
    }))
    .unwrap();

    assert_eq!(reconnect["should_reconnect"], true);
    assert_eq!(reconnect["sequence"], 91);
    assert_eq!(reconnect["reconnect_reason"], "gateway_reconnect_requested");

    let invalid = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "payload",
        "payload": {"op": 9, "d": false},
        "session_id": "discord-session-1",
        "resume_gateway_url": "wss://gateway.discord.gg",
        "sequence": 91,
    }))
    .unwrap();

    assert_eq!(invalid["should_reconnect"], true);
    assert_eq!(invalid["can_resume"], false);
    assert_eq!(invalid["session_id"], JsonValue::Null);
    assert_eq!(invalid["resume_gateway_url"], JsonValue::Null);
    assert_eq!(invalid["sequence"], JsonValue::Null);
}

#[test]
fn error_recovery_drops_message_content_intent_only_for_disallowed_intent_error() {
    let intents = DISCORD_GUILD_MESSAGES_INTENT | DISCORD_MESSAGE_CONTENT_INTENT;
    let dropped = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "error_recovery",
        "error_message": "received 4014 (private use) Disallowed intent(s).",
        "gateway_intents": intents,
        "session_id": "discord-session-1",
        "resume_gateway_url": "wss://gateway.discord.gg",
        "sequence": 55,
    }))
    .unwrap();

    assert_eq!(dropped["should_drop_message_content_intent"], true);
    assert_eq!(
        dropped["new_gateway_intents"],
        DISCORD_GUILD_MESSAGES_INTENT
    );
    assert_eq!(dropped["session_id"], JsonValue::Null);
    assert_eq!(dropped["actions"][0]["kind"], "drop_message_content_intent");

    let without_intent = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "error_recovery",
        "error_message": "received 4014 (private use) Disallowed intent(s).",
        "gateway_intents": DISCORD_GUILD_MESSAGES_INTENT,
    }))
    .unwrap();
    assert_eq!(without_intent["should_drop_message_content_intent"], false);

    let auth_failure = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "error_recovery",
        "error_message": "received 4004 authentication failed",
        "gateway_intents": intents,
    }))
    .unwrap();
    assert_eq!(auth_failure["should_drop_message_content_intent"], false);
}

#[test]
fn discord_gateway_runtime_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl DiscordGatewayRuntimePlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "gateway_runtime_state": "substitute",
            }))
        }
    }

    let planned = plan_with_discord_gateway_runtime_planner(
        &SubstitutePlanner,
        &json!({ "stage": "gateway_url" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "gateway_url");
    assert_eq!(planned["gateway_runtime_state"], "substitute");
}
