use super::*;
use ait_core::json_support::json;

#[test]
fn command_runtime_reports_missing_conversation_binding() {
    let planned = agent_telegram_command_runtime_plan_json(&json!({
        "stage": "missing_binding_text"
    }))
    .expect("plan");

    assert_eq!(planned["text_kind"], "missing_binding");
    assert_eq!(planned["text"], MISSING_BINDING_TEXT);
    assert!(!planned.to_string().contains("session"));
}

#[test]
fn command_specs_are_sessionless() {
    let planned =
        agent_telegram_command_runtime_plan_json(&json!({"stage": "command_specs"})).expect("plan");

    assert_eq!(planned["ok"], true);
    assert_eq!(planned["command_specs"][0]["name"], "help");
    assert!(planned["command_specs"]
        .as_array()
        .expect("commands")
        .iter()
        .all(|command| command["name"] != "session"));
    assert!(!planned.to_string().contains("session_id"));
}

#[test]
fn help_dispatch_reads_only_an_existing_binding() {
    let planned = agent_telegram_command_runtime_plan_json(&json!({
        "stage": "dispatch_command",
        "chat_id": 123,
        "name": "start",
        "args": ""
    }))
    .expect("plan");

    assert_eq!(planned["command_name"], "help");
    assert_eq!(planned["actions"][0]["stage"], "help_text");
    assert_eq!(planned["actions"][0]["binding_policy"], "read_existing");
    assert!(planned["actions"][0].get("session_request").is_none());
}

#[test]
fn ping_dispatches_direct_message() {
    let planned = agent_telegram_command_runtime_plan_json(&json!({
        "stage": "dispatch_command",
        "chat_id": 123,
        "name": "ping",
        "args": ""
    }))
    .expect("plan");

    assert_eq!(planned["mode"], "ping");
    assert_eq!(planned["actions"][0]["kind"], "send_message");
    assert_eq!(planned["actions"][0]["message_text"], "pong");
}

#[test]
fn unknown_command_does_not_create_conversation_state() {
    let planned = agent_telegram_command_runtime_plan_json(&json!({
        "stage": "dispatch_command",
        "chat_id": 123,
        "name": "obsolete",
        "args": ""
    }))
    .expect("plan");

    assert_eq!(planned["mode"], "unknown");
    assert_eq!(planned["actions"][0]["kind"], "send_message");
    assert!(!planned.to_string().contains("create_session"));
}

#[test]
fn unsupported_stage_fails_closed() {
    let error = agent_telegram_command_runtime_plan_json(&json!({"stage": "session_command"}))
        .expect_err("retired session stage must fail");
    assert!(error.contains("unsupported Telegram command runtime stage"));
}
