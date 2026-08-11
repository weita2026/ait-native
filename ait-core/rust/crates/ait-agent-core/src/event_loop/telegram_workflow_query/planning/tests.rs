use super::{
    agent_telegram_workflow_query_plan_json, plan_with_telegram_workflow_query_planner,
    DefaultTelegramWorkflowQueryPlanner, TelegramWorkflowQueryPlanner, MIGRATION_STAGE,
    WORKFLOW_QUERY_CONTRACT,
};
use ait_core::json_support::{json, JsonValue};

struct SubstituteWorkflowQueryPlanner;

impl TelegramWorkflowQueryPlanner for SubstituteWorkflowQueryPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        Ok(json!({
            "kind": request.get("kind").cloned().unwrap_or(JsonValue::Null),
            "substitute": true,
            "transport": "telegram",
        }))
    }
}

#[test]
fn parse_command_accepts_bot_mention_target() {
    let planned = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "parse_command",
        "text": "/status@ait_test_bot now",
        "username": "ait_test_bot",
    }))
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_telegram_workflow_query"
    );
    assert_eq!(
        planned["workflow_query_contract"],
        "ait_agent_core.event_loop.TelegramWorkflowQuery.v1"
    );
    assert_eq!(planned["python_workflow_query_allowed"], false);
    assert_eq!(planned["matched"], true);
    assert_eq!(planned["command_name"], "status");
    assert_eq!(planned["command_args"], "now");
    assert_eq!(planned["command"], json!(["status", "now"]));
}

#[test]
fn parse_command_rejects_other_bot_target() {
    let planned = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "parse_command",
        "text": "/status@other_bot",
        "username": "ait_test_bot",
    }))
    .unwrap();

    assert_eq!(planned["matched"], false);
    assert!(planned["command"].is_null());
}

#[test]
fn detect_workflow_queries_and_ids() {
    for (text, kind, reference) in [
        ("queue", "queue", None),
        ("what needs attention", "attention", None),
        ("what can land", "ready", None),
        ("task aitt-0010", "task", Some("AITT-0010")),
        ("audit aitt-0010", "audit", Some("AITT-0010")),
        ("change aitc-0011", "change", Some("AITC-0011")),
        ("land aitc-0011", "land", Some("AITC-0011")),
        ("任務 rt-2446", "task", Some("RT-2446")),
        ("變更 rc-2290", "change", Some("RC-2290")),
    ] {
        let planned = agent_telegram_workflow_query_plan_json(&json!({
            "kind": "detect_workflow_query",
            "text": text,
        }))
        .unwrap();
        assert_eq!(planned["matched"], true, "{text}");
        assert_eq!(planned["query_kind"], kind, "{text}");
        match reference {
            Some(reference) => assert_eq!(planned["query_ref"], reference, "{text}"),
            None => assert!(planned["query_ref"].is_null(), "{text}"),
        }
    }
}

#[test]
fn title_identity_and_display_name_match_python_contract() {
    let chat_title = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "chat_title",
        "chat": {"id": 123, "first_name": "Wei", "last_name": "Ta"},
    }))
    .unwrap();
    assert_eq!(chat_title["text"], "Wei Ta");

    let actor = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "actor_identity",
        "chat_id": 123,
        "from_user": {"id": 456, "username": "weita"},
    }))
    .unwrap();
    assert_eq!(actor["text"], "telegram:456:@weita");

    let display = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "user_display_name",
        "from_user": {"username": "weita"},
    }))
    .unwrap();
    assert_eq!(display["text"], "@weita");
}

#[test]
fn message_entrypoint_dispatches_slash_command() {
    let planned = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "message_entrypoint",
        "chat": {"id": 123, "title": "ops"},
        "raw_text": "/task lt-1860",
        "normalized_text": "/task lt-1860",
        "username": "ait_test_bot",
    }))
    .unwrap();

    assert_eq!(planned["kind"], "message_entrypoint");
    assert_eq!(planned["chat_title"], "ops");
    assert_eq!(planned["action_kind"], "dispatch_command");
    assert_eq!(planned["command"], json!(["task", "lt-1860"]));
    assert_eq!(planned["dispatch_command_name"], "task");
    assert_eq!(planned["dispatch_command_args"], "lt-1860");
    assert!(planned["workflow_query"].is_null());
}

#[test]
fn message_entrypoint_dispatches_workflow_query() {
    let planned = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "message_entrypoint",
        "chat": {"id": 123, "first_name": "Wei", "last_name": "Ta"},
        "raw_text": "task lt-1860",
        "normalized_text": "task lt-1860",
        "username": "ait_test_bot",
    }))
    .unwrap();

    assert_eq!(planned["chat_title"], "Wei Ta");
    assert_eq!(planned["action_kind"], "dispatch_command");
    assert_eq!(planned["workflow_query"], json!(["task", "LT-1860"]));
    assert_eq!(planned["dispatch_command_name"], "task");
    assert_eq!(planned["dispatch_command_args"], "LT-1860");
}

#[test]
fn message_entrypoint_does_not_parse_commands_when_attachments_are_present() {
    let planned = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "message_entrypoint",
        "chat": {"id": 123},
        "raw_text": "/queue",
        "normalized_text": "Telegram attachment upload:\n- demo.txt",
        "username": "ait_test_bot",
        "attachments": [{"kind": "document", "file_name": "demo.txt"}],
    }))
    .unwrap();

    assert_eq!(planned["attachments_present"], true);
    assert_eq!(planned["action_kind"], "normal_text_turn");
    assert!(planned["command"].is_null());
    assert!(planned["workflow_query"].is_null());
}

#[test]
fn message_entrypoint_sends_empty_text_help() {
    let planned = agent_telegram_workflow_query_plan_json(&json!({
        "kind": "message_entrypoint",
        "chat": {"id": 123},
        "raw_text": "   ",
        "username": "ait_test_bot",
    }))
    .unwrap();

    assert_eq!(planned["action_kind"], "send_empty_text_help");
    assert_eq!(
        planned["message_text"],
        "Send a message after the bot mention, or use /help."
    );
    assert_eq!(planned["normalized_text"], "");
}

#[test]
fn workflow_query_defaults_stage_alias_and_error_contract() {
    let defaulted = agent_telegram_workflow_query_plan_json(&json!({
        "text": "queue",
    }))
    .unwrap();
    assert_eq!(defaulted["kind"], "detect_workflow_query");
    assert_eq!(defaulted["matched"], true);
    assert_eq!(defaulted["query_kind"], "queue");

    let stage_alias = agent_telegram_workflow_query_plan_json(&json!({
        "stage": "parse_command",
        "text": "/status"
    }))
    .unwrap();
    assert_eq!(stage_alias["kind"], "parse_command");
    assert_eq!(stage_alias["command_name"], "status");

    assert_eq!(
        agent_telegram_workflow_query_plan_json(&json!("bad request")).unwrap_err(),
        "request must be a JSON object"
    );
    assert_eq!(
        agent_telegram_workflow_query_plan_json(&json!({"kind": "unknown"})).unwrap_err(),
        "unsupported Telegram workflow query plan kind `unknown`"
    );
}

#[test]
fn workflow_query_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramWorkflowQueryPlanner = &DefaultTelegramWorkflowQueryPlanner;
    let planned = planner
        .plan_json(&json!({
            "kind": "chat_title",
            "chat": {"username": "ait_core"}
        }))
        .unwrap();
    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(planned["workflow_query_contract"], WORKFLOW_QUERY_CONTRACT);
    assert_eq!(planned["text"], "@ait_core");
}

#[test]
fn workflow_query_bound_entrypoint_accepts_substitute_planner() {
    let planner = SubstituteWorkflowQueryPlanner;
    let planned = plan_with_telegram_workflow_query_planner(
        &planner,
        &json!({"kind": "detect_workflow_query"}),
    )
    .unwrap();
    assert_eq!(planned["kind"], "detect_workflow_query");
    assert_eq!(planned["substitute"], true);
    assert_eq!(planned["transport"], "telegram");
}
