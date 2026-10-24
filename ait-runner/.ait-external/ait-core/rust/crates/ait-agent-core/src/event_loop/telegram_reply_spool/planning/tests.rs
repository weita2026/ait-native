use super::{
    agent_telegram_reply_spool_execution_plan_json, plan_with_telegram_reply_spool_planner,
    DefaultTelegramReplySpoolPlanner, TelegramReplySpoolPlanner,
};
use ait_core::json_support::{json, JsonValue};

#[test]
fn key_stage_prefers_transport_event_id() {
    let planned = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "key",
        "pending_turn": {
            "conversation_key": "S-1",
            "chat_id": 123,
            "text": "hello",
            "telegram_message_id": 10,
            "transport_envelope": {"event_id": " evt-1 "}
        }
    }))
    .expect("plan");

    assert_eq!(planned["spool_key"], "evt-1");
    assert_eq!(planned["result"]["transport_event_id"], "evt-1");
    assert_eq!(planned["result"]["telegram_message_ids"], json!([10]));
}

#[test]
fn reply_spool_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramReplySpoolPlanner = &DefaultTelegramReplySpoolPlanner;

    let planned = planner
        .plan_json(&json!({
            "stage": "key",
            "pending_turn": {
                "conversation_key": "S-1",
                "chat_id": 123,
                "text": "hello"
            }
        }))
        .expect("plan");

    assert_eq!(planned["stage"], "key");
    assert_eq!(planned["execution_kind"], "telegram_reply_spool");
}

#[test]
fn reply_spool_bound_entrypoint_accepts_substitute_planner() {
    struct StubReplySpoolPlanner;

    impl TelegramReplySpoolPlanner for StubReplySpoolPlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": "stubbed",
                "observed_stage": request.get("stage").cloned().unwrap_or(JsonValue::Null),
            }))
        }
    }

    let planned = plan_with_telegram_reply_spool_planner(
        &StubReplySpoolPlanner,
        &json!({
            "stage": "entries",
            "link": {"telegram_reply_spool": []},
        }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "stubbed");
    assert_eq!(planned["observed_stage"], "entries");
}

#[test]
fn key_stage_falls_back_to_message_ids_then_text() {
    let message_key = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "key",
        "pending_turn": {
            "conversation_key": "S-1",
            "chat_id": 123,
            "text": " hello ",
            "telegram_message_id": null,
            "telegram_message_ids": [0, "11", "bad", 12]
        }
    }))
    .expect("plan");
    assert_eq!(message_key["spool_key"], "telegram:123:messages:11,12");

    let text_key = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "key",
        "pending_turn": {
            "conversation_key": "S-1",
            "chat_id": "chat-a",
            "text": " hello ",
            "telegram_message_id": null,
            "telegram_message_ids": []
        }
    }))
    .expect("plan");
    assert_eq!(
        text_key["spool_key"],
        "telegram:chat-a:conversation:S-1:text:hello"
    );
}

#[test]
fn reply_spool_defaults_aliases_and_error_contract_are_stable() {
    let default_plan = agent_telegram_reply_spool_execution_plan_json(&json!({
        "pending_turn": {
            "conversation_key": "S-1",
            "chat_id": 123,
            "text": "hello"
        }
    }))
    .expect("default stage");
    assert_eq!(default_plan["stage"], "key");
    assert_eq!(default_plan["execution_kind"], "telegram_reply_spool");
    assert_eq!(
        default_plan["spool_key"],
        "telegram:123:conversation:S-1:text:hello"
    );
    assert_eq!(
        default_plan["result"]["execution_kind"],
        "telegram_reply_spool"
    );

    let invalid = agent_telegram_reply_spool_execution_plan_json(&json!("bad"));
    assert_eq!(invalid.unwrap_err(), "request must be a JSON object");

    let unsupported = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "unknown"
    }));
    assert_eq!(
        unsupported.unwrap_err(),
        "unsupported Telegram reply spool execution stage `unknown`"
    );

    let entries_from_execution_request = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "entries",
        "execution_request": {
            "current_link": {
                "telegram_reply_spool": [{"spool_key": "nested"}]
            }
        }
    }))
    .expect("entries from execution_request");
    assert_eq!(entries_from_execution_request["entry_count"], 1);
    assert_eq!(
        entries_from_execution_request["result"]["entries"][0]["spool_key"],
        "nested"
    );
}

#[test]
fn reply_spool_nested_request_and_no_patch_contracts_are_stable() {
    let missing_link = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "remember",
        "request": {
            "pending_turn": {
                "conversation_key": "S-1",
                "chat_id": 123,
                "text": "hello"
            },
            "status": "queued"
        }
    }))
    .expect("missing current link");
    assert_eq!(missing_link["stage"], "remember");
    assert_eq!(missing_link["patch_required"], false);
    assert_eq!(missing_link["spool_key"], JsonValue::Null);
    assert_eq!(missing_link["result"]["reason"], "missing_current_link");
    assert_eq!(missing_link["result"]["patch_payload"], JsonValue::Null);

    let nested_request = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "remember",
        "request": {
            "pending_turn": {
                "conversation_key": "S-1",
                "chat_id": 123,
                "text": "hello",
                "telegram_message_id": 9
            },
            "current_link": {
                "conversation_key": "S-1",
                "telegram_reply_spool": []
            },
            "status": "failed",
            "attempt_increment": "yes",
            "now_iso": "2026-07-02T00:00:00Z"
        }
    }))
    .expect("nested request");
    assert_eq!(nested_request["patch_required"], true);
    assert_eq!(
        nested_request["entry"]["spool_key"],
        "telegram:123:messages:9"
    );
    assert_eq!(nested_request["entry"]["status"], "failed");
    assert_eq!(nested_request["entry"]["attempt_count"], 1);
    assert_eq!(
        nested_request["entry"]["last_attempt_at"],
        "2026-07-02T00:00:00Z"
    );
}

#[test]
fn entries_stage_filters_non_mapping_items() {
    let planned = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "entries",
        "link": {"telegram_reply_spool": [{"spool_key": "a"}, 3, null, {"spool_key": "b"}]}
    }))
    .expect("plan");

    assert_eq!(planned["entry_count"], 2);
    assert_eq!(planned["entries"][0]["spool_key"], "a");
    assert_eq!(planned["entries"][1]["spool_key"], "b");
}

#[test]
fn remember_stage_normalizes_entry_and_preserves_existing_timestamps() {
    let planned = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "remember",
        "pending_turn": {
            "conversation_key": "S-1",
            "chat_id": 123,
            "chat_title": "Wei",
            "chat_type": "private",
            "actor_identity": "telegram:456",
            "text": "Hello",
            "telegram_message_id": 10,
            "telegram_message_ids": [10, 11],
            "transport_envelope": {"event_id": "evt-1"},
            "watch_spec": {"identity": {"watch_id": "W-1"}}
        },
        "current_link": {
            "conversation_key": " S-1 ",
            "telegram_reply_spool": [
                {"spool_key": "old", "status": "queued"},
                {
                    "spool_key": "evt-1",
                    "queued_at": "queued-before",
                    "last_attempt_at": "attempt-before",
                    "attempt_count": 2
                }
            ]
        },
        "status": "attempting",
        "attempt_increment": true,
        "last_error": " failed once ",
        "user_event": {"sequence": 7},
        "assistant_event": {"sequence": 8},
        "now_iso": "2026-07-01T00:00:00Z",
        "spool_limit": 5
    }))
    .expect("plan");

    assert_eq!(planned["patch_required"], true);
    let entry = &planned["entry"];
    assert_eq!(entry["spool_key"], "evt-1");
    assert_eq!(entry["status"], "attempting");
    assert_eq!(entry["chat_id"], "123");
    assert_eq!(entry["transport_event_id"], "evt-1");
    assert_eq!(entry["telegram_message_ids"], json!([10, 10, 11]));
    assert_eq!(entry["queued_at"], "queued-before");
    assert_eq!(entry["last_attempt_at"], "2026-07-01T00:00:00Z");
    assert_eq!(entry["attempt_count"], 3);
    assert_eq!(entry["last_error"], "failed once");
    assert_eq!(entry["last_user_sequence"], 7);
    assert_eq!(entry["last_assistant_sequence"], 8);
    assert_eq!(entry["watch_spec"]["identity"]["watch_id"], "W-1");
    assert_eq!(planned["entries"][0]["spool_key"], "old");
    assert_eq!(planned["entries"][1]["spool_key"], "evt-1");
    assert_eq!(
        planned["patch_payload"]["telegram_reply_spool"],
        planned["entries"]
    );
}

#[test]
fn remember_stage_ignores_conversation_mismatch_and_limits_spool() {
    let mismatch = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "remember",
        "pending_turn": {"conversation_key": "S-1", "chat_id": 123, "text": "Hello"},
        "current_link": {"conversation_key": "S-2", "telegram_reply_spool": []},
        "status": "queued",
        "now_iso": "now"
    }))
    .expect("plan");
    assert_eq!(mismatch["patch_required"], false);
    assert_eq!(mismatch["result"]["reason"], "conversation_mismatch");

    let limited = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "remember",
        "pending_turn": {
            "conversation_key": "S-1",
            "chat_id": 123,
            "text": "third",
            "telegram_message_id": null
        },
        "current_link": {
            "conversation_key": "S-1",
            "telegram_reply_spool": [
                {"spool_key": "first"},
                {"spool_key": "second"}
            ]
        },
        "status": "queued",
        "now_iso": "now",
        "spool_limit": 2
    }))
    .expect("plan");
    assert_eq!(limited["entries"].as_array().unwrap().len(), 2);
    assert_eq!(limited["entries"][0]["spool_key"], "second");
    assert_eq!(
        limited["entries"][1]["spool_key"],
        "telegram:123:conversation:S-1:text:third"
    );
}

#[test]
fn clear_stage_removes_matching_entry_without_session_guard() {
    let planned = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "clear",
        "pending_turn": {
            "conversation_key": "S-1",
            "chat_id": 123,
            "text": "Hello",
            "telegram_message_id": 10
        },
        "current_link": {
            "conversation_key": "S-2",
            "telegram_reply_spool": [
                {"spool_key": "telegram:123:messages:10"},
                {"spool_key": "keep"}
            ]
        }
    }))
    .expect("plan");

    assert_eq!(planned["patch_required"], true);
    assert_eq!(planned["entries"], json!([{"spool_key": "keep"}]));

    let missing = agent_telegram_reply_spool_execution_plan_json(&json!({
        "stage": "clear",
        "pending_turn": {"conversation_key": "S-1", "chat_id": 123, "text": "Hello"}
    }))
    .expect("plan");
    assert_eq!(missing["patch_required"], false);
    assert_eq!(missing["patch_payload"], JsonValue::Null);
}
