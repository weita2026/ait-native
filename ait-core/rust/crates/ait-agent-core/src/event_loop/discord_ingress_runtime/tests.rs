use super::{
    agent_discord_ingress_runtime_plan_json, plan_with_discord_ingress_runtime_planner,
    DiscordIngressRuntimePlanner,
};
use ait_core::json_support::{json, JsonValue};

fn interaction_payload() -> JsonValue {
    json!({
        "id": "112233445566778899",
        "type": 2,
        "token": "discord-token-1",
        "application_id": "123456789012345678",
        "channel_id": "998877665544332211",
        "guild_id": "556677889900112233",
        "data": {
            "id": "887766554433221100",
            "name": "ask",
            "type": 1,
            "options": [
                {
                    "name": "text",
                    "type": 3,
                    "value": "Hello from Discord"
                }
            ]
        },
        "member": {
            "user": {
                "id": "U-discord-1",
                "username": "weita",
                "global_name": "WeiTa"
            }
        }
    })
}

fn message_payload() -> JsonValue {
    json!({
        "id": "998899887777666655",
        "type": 0,
        "channel_id": "998877665544332211",
        "guild_id": "556677889900112233",
        "content": "Hello from Discord chat",
        "author": {
            "id": "U-discord-1",
            "username": "weita",
            "global_name": "WeiTa",
            "bot": false
        }
    })
}

const DISCORD_PUBLIC_KEY: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const DISCORD_SIGNATURE_TIMESTAMP: &str = "1714990000";
const DISCORD_RAW_INTERACTION_PAYLOAD: &str = r#"{"id":"112233445566778899","type":2,"token":"discord-token-1","application_id":"123456789012345678","channel_id":"998877665544332211","guild_id":"556677889900112233","data":{"id":"887766554433221100","name":"ask","type":1,"options":[{"name":"text","type":3,"value":"Hello from Discord"}]},"member":{"user":{"id":"U-discord-1","username":"weita","global_name":"WeiTa"}}}"#;
const DISCORD_VALID_SIGNATURE: &str =
    "cdd61b985c0507f54f261a6fb4415dd5db603c78387c7176ea311202c61578f67cbdf9e596dfc0c039c7e80f2ced117804740076680db3075ffd55818605ce00";
const DISCORD_EMPTY_OBJECT_SIGNATURE: &str =
    "cbfe0f23c0477d870a8019242e7e054c8d05e85386ff783dfb8f7ea19ce3106f8e146b344c7dc34849d9da6e22a475734674931484b2b895cfe7b5db3899f20b";

#[test]
fn interaction_http_request_verifies_signature_and_parses_payload_without_python() {
    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction_http_request",
        "raw_payload": DISCORD_RAW_INTERACTION_PAYLOAD,
        "signature": DISCORD_VALID_SIGNATURE,
        "signature_timestamp": DISCORD_SIGNATURE_TIMESTAMP,
        "public_key": DISCORD_PUBLIC_KEY,
    }))
    .unwrap();

    assert_eq!(planned["stage"], "interaction_http_request");
    assert_eq!(planned["ingress_runtime_state"], "payload_valid");
    assert_eq!(planned["interaction_http_ingress_state"], "payload_valid");
    assert_eq!(planned["python_ingress_allowed"], false);
    assert_eq!(planned["python_signature_verification_allowed"], false);
    assert_eq!(planned["python_json_parsing_allowed"], false);
    assert_eq!(planned["rust_event_loop_required"], true);
    assert_eq!(planned["signature_verified"], true);
    assert_eq!(planned["should_handle_interaction"], true);
    assert_eq!(planned["should_parse_payload"], true);
    assert_eq!(planned["http_status"], 200);
    assert_eq!(planned["payload"]["id"], "112233445566778899");
    assert_eq!(planned["payload"]["type"], 2);
    assert_eq!(planned["interaction_type"], 2);
    assert_eq!(planned["next_ingress_request"]["stage"], "interaction");
    assert_eq!(
        planned["next_ingress_request"]["payload"]["data"]["options"][0]["value"],
        "Hello from Discord"
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "verify_interaction_signature"
    );
    assert_eq!(planned["actions"][1]["kind"], "parse_interaction_payload");
}

#[test]
fn interaction_http_request_fails_closed_for_signature_and_payload_errors() {
    let missing_signature = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction_http_request",
        "raw_payload": DISCORD_RAW_INTERACTION_PAYLOAD,
        "signature_timestamp": DISCORD_SIGNATURE_TIMESTAMP,
        "public_key": DISCORD_PUBLIC_KEY,
    }))
    .unwrap();
    assert_eq!(
        missing_signature["interaction_http_ingress_state"],
        "missing_signature"
    );
    assert_eq!(missing_signature["http_status"], 401);
    assert_eq!(missing_signature["should_handle_interaction"], false);
    assert_eq!(missing_signature["signature_verified"], false);
    assert_eq!(
        missing_signature["response"]["error"],
        "Missing Discord interaction signature header."
    );

    let invalid_signature = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction_http_request",
        "raw_payload": DISCORD_RAW_INTERACTION_PAYLOAD,
        "signature": "00".repeat(64),
        "signature_timestamp": DISCORD_SIGNATURE_TIMESTAMP,
        "public_key": DISCORD_PUBLIC_KEY,
    }))
    .unwrap();
    assert_eq!(
        invalid_signature["interaction_http_ingress_state"],
        "invalid_signature"
    );
    assert_eq!(invalid_signature["http_status"], 401);
    assert_eq!(invalid_signature["should_handle_interaction"], false);
    assert_eq!(invalid_signature["signature_verified"], false);

    let missing_type = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction_http_request",
        "raw_payload": "{}",
        "signature": DISCORD_EMPTY_OBJECT_SIGNATURE,
        "signature_timestamp": DISCORD_SIGNATURE_TIMESTAMP,
        "public_key": DISCORD_PUBLIC_KEY,
    }))
    .unwrap();
    assert_eq!(
        missing_type["interaction_http_ingress_state"],
        "missing_type"
    );
    assert_eq!(missing_type["http_status"], 400);
    assert_eq!(missing_type["error_kind"], "invalid_payload");
    assert_eq!(missing_type["should_handle_interaction"], false);
    assert_eq!(missing_type["signature_verified"], true);
    assert_eq!(
        missing_type["response"]["error"],
        "Discord interaction payload must include a numeric type."
    );
}

#[test]
fn parse_interaction_payload_requires_numeric_type() {
    let error = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "parse_interaction_payload",
        "raw_payload": "{}",
    }))
    .unwrap_err();

    assert_eq!(
        error,
        "Discord interaction payload must include a numeric type."
    );
}

#[test]
fn interaction_ping_returns_discord_pong_without_python_ingress() {
    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction",
        "payload": {"type": 1},
    }))
    .unwrap();

    assert_eq!(planned["stage"], "interaction");
    assert_eq!(planned["python_ingress_allowed"], false);
    assert_eq!(planned["rust_event_loop_required"], true);
    assert_eq!(planned["response"], json!({"type": 1}));
    assert_eq!(planned["should_submit_turn"], false);
    assert_eq!(planned["actions"][0]["kind"], "send_interaction_pong");
}

#[test]
fn interaction_plans_deferred_reply_transport_envelope_and_pending_reply() {
    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction",
        "payload": interaction_payload(),
        "defer_replies": true,
        "occurred_at": "2026-07-02T10:00:00Z",
    }))
    .unwrap();

    assert_eq!(planned["ingress_runtime_state"], "turn_submission_planned");
    assert_eq!(planned["response"], json!({"type": 5}));
    assert_eq!(planned["should_submit_turn"], true);
    assert_eq!(planned["should_start_background_reply"], true);
    assert_eq!(planned["event_id"], "112233445566778899");
    assert_eq!(planned["channel_id"], "998877665544332211");
    assert_eq!(planned["actor_identity"], "discord:U-discord-1");
    assert_eq!(planned["actor_display_name"], "WeiTa");
    assert_eq!(planned["text"], "Hello from Discord");
    assert_eq!(planned["transport_envelope"]["transport"], "discord");
    assert_eq!(
        planned["transport_envelope"]["event_id"],
        "112233445566778899"
    );
    assert_eq!(
        planned["transport_envelope"]["channel"]["channel_id"],
        "998877665544332211"
    );
    assert_eq!(
        planned["transport_envelope"]["metadata"]["command_name"],
        "ask"
    );
    assert_eq!(
        planned["pending_reply"]["conversation_key"],
        "discord:998877665544332211"
    );
    assert_eq!(planned["pending_reply"]["reply_mode"], "interaction");
    assert_eq!(
        planned["pending_reply"]["interaction_token"],
        "discord-token-1"
    );
    assert_eq!(planned["watch_spec"]["event_kind"], "interaction");
    assert_eq!(planned["actions"][0]["kind"], "upsert_binding");
    assert_eq!(planned["actions"][1]["kind"], "remember_interaction");
    assert_eq!(planned["actions"][2]["kind"], "start_background_reply");
}

#[test]
fn interaction_duplicate_returns_channel_message_response_without_turn() {
    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction",
        "payload": interaction_payload(),
        "duplicate": true,
    }))
    .unwrap();

    assert_eq!(planned["ingress_runtime_state"], "duplicate_ignored");
    assert_eq!(planned["duplicate"], true);
    assert_eq!(planned["accepted"], false);
    assert_eq!(planned["should_submit_turn"], false);
    assert_eq!(planned["response"]["type"], 4);
    assert_eq!(
        planned["response"]["data"]["content"],
        "Duplicate Discord interaction ignored."
    );
    assert_eq!(planned["actions"], json!([]));
}

#[test]
fn interaction_fresh_topic_creates_conversation_without_turn() {
    let mut payload = interaction_payload();
    payload["id"] = json!("112233445566778900");
    payload["token"] = json!("discord-token-2");
    payload["data"]["options"][0]["value"] = json!("換個話題");

    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "interaction",
        "payload": payload,
    }))
    .unwrap();

    assert_eq!(
        planned["ingress_runtime_state"],
        "fresh_topic_conversation_planned"
    );
    assert_eq!(planned["fresh_topic"], true);
    assert_eq!(planned["should_submit_turn"], false);
    assert_eq!(planned["should_start_background_reply"], false);
    assert_eq!(planned["response"]["type"], 4);
    assert_eq!(
        planned["response"]["data"]["content"],
        "Started a fresh Discord conversation.\nTrigger: 換個話題."
    );
    assert_eq!(planned["actions"][0]["kind"], "create_fresh_binding");
    assert_eq!(
        planned["conversation_key"],
        "discord:998877665544332211:topic:112233445566778900"
    );
    assert_eq!(
        planned["actions"][0]["rotation_reason"],
        "fresh_topic_event_trigger"
    );
    assert_eq!(planned["actions"][1]["kind"], "remember_interaction");
    assert_eq!(planned["actions"][2]["kind"], "respond_to_interaction");
}

#[test]
fn message_plans_background_reply_transport_envelope_and_pending_reply() {
    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "message",
        "payload": message_payload(),
        "config_application_id": "123456789012345678",
        "occurred_at": "2026-07-02T10:00:00Z",
    }))
    .unwrap();

    assert_eq!(planned["ingress_runtime_state"], "turn_submission_planned");
    assert_eq!(planned["accepted"], true);
    assert_eq!(planned["should_submit_turn"], true);
    assert_eq!(planned["should_start_background_reply"], true);
    assert_eq!(planned["event_id"], "998899887777666655");
    assert_eq!(planned["actor_identity"], "discord:U-discord-1");
    assert_eq!(planned["transport_envelope"]["transport"], "discord");
    assert_eq!(
        planned["transport_envelope"]["event_id"],
        "998899887777666655"
    );
    assert_eq!(
        planned["transport_envelope"]["channel"]["channel_id"],
        "998877665544332211"
    );
    assert_eq!(planned["transport_envelope"]["metadata"]["message_type"], 0);
    assert_eq!(
        planned["pending_reply"]["conversation_key"],
        "discord:998877665544332211"
    );
    assert_eq!(planned["pending_reply"]["reply_mode"], "channel_message");
    assert_eq!(
        planned["pending_reply"]["interaction_token"],
        JsonValue::Null
    );
    assert_eq!(planned["watch_spec"]["event_kind"], "message");
    assert_eq!(planned["actions"][0]["kind"], "upsert_binding");
    assert_eq!(planned["actions"][1]["kind"], "remember_message");
    assert_eq!(planned["actions"][2]["kind"], "start_background_reply");
}

#[test]
fn message_preserves_discord_attachment_metadata_and_accepts_attachment_only_events() {
    let mut payload = message_payload();
    payload["content"] = json!("");
    payload["attachments"] = json!([{
        "id": "778899001122334455",
        "filename": "question.txt",
        "content_type": "text/plain",
        "size": 42,
        "description": "fixture question",
        "url": "https://cdn.discordapp.com/attachments/question.txt",
    }]);

    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "message",
        "payload": payload,
        "config_application_id": "123456789012345678",
    }))
    .unwrap();

    assert_eq!(planned["accepted"], true);
    assert_eq!(planned["should_submit_turn"], true);
    assert_eq!(
        planned["text"],
        "Shared Discord attachment(s): question.txt"
    );
    assert_eq!(
        planned["transport_envelope"]["message"]["attachments"][0]["kind"],
        "document"
    );
    assert_eq!(
        planned["transport_envelope"]["message"]["attachments"][0]["file_name"],
        "question.txt"
    );
    assert_eq!(
        planned["transport_envelope"]["message"]["attachments"][0]["mime_type"],
        "text/plain"
    );
    assert_eq!(
        planned["transport_envelope"]["message"]["attachments"][0]["file_size_bytes"],
        42
    );
    assert_eq!(
        planned["transport_envelope"]["message"]["attachments"][0]["url"],
        "https://cdn.discordapp.com/attachments/question.txt"
    );
    assert_eq!(
        planned["transport_envelope"]["metadata"]["attachment_count"],
        1
    );
}

#[test]
fn message_ignores_bot_webhook_empty_and_duplicate_messages() {
    let bot = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "message",
        "payload": {
            "author": {"id": "U-discord-bot", "bot": true},
            "content": "ignored",
        },
    }))
    .unwrap();
    assert_eq!(bot["ignored"], true);
    assert_eq!(bot["ignore_reason"], "bot_author");
    assert_eq!(bot["should_submit_turn"], false);

    let mut webhook_payload = message_payload();
    webhook_payload["webhook_id"] = json!("webhook-1");
    let webhook = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "message",
        "payload": webhook_payload,
    }))
    .unwrap();
    assert_eq!(webhook["ignore_reason"], "webhook_message");

    let mut empty_payload = message_payload();
    empty_payload["content"] = json!("  ");
    let empty = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "message",
        "payload": empty_payload,
    }))
    .unwrap();
    assert_eq!(empty["ignore_reason"], "empty_text");

    let duplicate = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "message",
        "payload": message_payload(),
        "duplicate": true,
    }))
    .unwrap();
    assert_eq!(duplicate["ingress_runtime_state"], "duplicate_ignored");
    assert_eq!(duplicate["duplicate"], true);
    assert_eq!(duplicate["accepted"], false);
    assert_eq!(duplicate["actions"], json!([]));
}

#[test]
fn message_fresh_topic_sends_confirmation_without_turn() {
    let mut payload = message_payload();
    payload["id"] = json!("998899887777666656");
    payload["content"] = json!("換個話題");

    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "message",
        "payload": payload,
        "config_application_id": "123456789012345678",
    }))
    .unwrap();

    assert_eq!(planned["accepted"], true);
    assert_eq!(planned["fresh_topic"], true);
    assert_eq!(planned["should_submit_turn"], false);
    assert_eq!(planned["should_start_background_reply"], false);
    assert_eq!(
        planned["send_channel_message"],
        json!({
            "channel_id": "998877665544332211",
            "text": "Started a fresh Discord conversation.\nTrigger: 換個話題."
        })
    );
    assert_eq!(planned["actions"][0]["kind"], "create_fresh_binding");
    assert_eq!(planned["actions"][1]["kind"], "remember_message");
    assert_eq!(planned["actions"][2]["kind"], "send_channel_message");
}

#[test]
fn message_fresh_topic_uses_rust_registry_matching_without_python_trigger() {
    let mut payload = message_payload();
    payload["id"] = json!("998899887777666657");
    payload["content"] = json!("切新話題");

    let planned = agent_discord_ingress_runtime_plan_json(&json!({
        "stage": "message",
        "payload": payload,
        "config_application_id": "123456789012345678",
        "fresh_topic_config": {
            "clear": {
                "phrases": ["切新話題"],
                "display_trigger": "切新話題",
                "allow_trailing_punctuation": true
            },
            "topic": {
                "lead_phrases": ["切新話題"],
                "joiners": ["跟"],
                "tail": "有關",
                "display_trigger": "切新話題跟…有關",
                "allow_trailing_punctuation": true
            }
        },
    }))
    .unwrap();

    assert_eq!(
        planned["ingress_runtime_state"],
        "fresh_topic_conversation_planned"
    );
    assert_eq!(planned["fresh_topic"], true);
    assert_eq!(planned["should_submit_turn"], false);
    assert_eq!(
        planned["send_channel_message"],
        json!({
            "channel_id": "998877665544332211",
            "text": "Started a fresh Discord conversation.\nTrigger: 切新話題."
        })
    );
    assert_eq!(
        planned["actions"][0]["rotation_reason"],
        "fresh_topic_event_trigger"
    );
}

#[test]
fn discord_ingress_runtime_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl DiscordIngressRuntimePlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "ingress_runtime_state": "substitute",
            }))
        }
    }

    let planned = plan_with_discord_ingress_runtime_planner(
        &SubstitutePlanner,
        &json!({ "stage": "interaction" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "interaction");
    assert_eq!(planned["ingress_runtime_state"], "substitute");
}
