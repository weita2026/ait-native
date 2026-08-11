use super::{
    agent_telegram_owner_bootstrap_plan_json, plan_with_telegram_owner_bootstrap_planner,
    DefaultTelegramOwnerBootstrapPlanner, TelegramOwnerBootstrapPlanner,
};
use ait_core::json_support::json;

fn base_request() -> ait_core::json_support::JsonValue {
    json!({
        "kind": "handle",
        "owner_bootstrap_enabled": true,
        "chat_id": 123,
        "chat_title": "Wei",
        "chat": {"id": 123, "type": "private", "first_name": "Wei"},
        "from_user": {"id": 456, "username": "weita", "first_name": "Wei"},
        "expected_password": "ait",
        "now_iso": "2026-07-02T00:00:00Z",
    })
}

#[test]
fn owner_bootstrap_start_prompts_for_password() {
    let mut request = base_request();
    request["command_name"] = json!("start");
    request["auth_state"] = json!({
        "audit_marker": "keep",
        "pending_user_id": "456",
        "pending_started_at": "2026-07-01T00:00:00Z",
    });

    let planned = agent_telegram_owner_bootstrap_plan_json(&request).unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_telegram_owner_bootstrap"
    );
    assert_eq!(
        planned["owner_bootstrap_contract"],
        "ait_agent_core.event_loop.TelegramOwnerBootstrap.v1"
    );
    assert_eq!(planned["python_owner_bootstrap_allowed"], false);
    assert_eq!(planned["handled"], true);
    assert_eq!(planned["decision"], "prompt_start");
    assert_eq!(planned["save_auth_state"]["pending_user_id"], "456");
    assert_eq!(planned["save_auth_state"]["audit_marker"], "keep");
    assert_eq!(
        planned["save_auth_state"]["pending_started_at"],
        "2026-07-01T00:00:00Z"
    );
    assert_eq!(
        planned["send_message_text"],
        "Telegram bootstrap is locked. Send the repository-name password as plain text."
    );
}

#[test]
fn owner_bootstrap_success_claims_owner() {
    let mut request = base_request();
    request["auth_state"] = json!({
        "pending_user_id": "456",
        "failed_attempts": {"456": 1},
    });
    request["raw_text"] = json!("ait");

    let planned = agent_telegram_owner_bootstrap_plan_json(&request).unwrap();

    assert_eq!(planned["decision"], "owner_verified");
    assert_eq!(planned["handled"], true);
    assert_eq!(planned["save_auth_state"]["owner_user_id"], "456");
    assert_eq!(planned["save_auth_state"]["owner_username"], "weita");
    assert_eq!(planned["save_auth_state"]["owner_display_name"], "Wei");
    assert!(planned["save_auth_state"].get("failed_attempts").is_none());
    assert_eq!(
        planned["send_message_text"],
        "Owner verified. Telegram access is now bound to this user id. Send /help or a normal message to continue."
    );
}

#[test]
fn owner_bootstrap_adopts_existing_private_binding() {
    let mut request = base_request();
    request["raw_text"] = json!("hello");
    request["existing_binding"] = json!({
        "conversation_key": "AITS-TEST-1",
        "chat_type": "private",
        "binding_role": "primary_shared",
    });

    let planned = agent_telegram_owner_bootstrap_plan_json(&request).unwrap();

    assert_eq!(planned["decision"], "adopt_existing_private_binding");
    assert_eq!(planned["handled"], false);
    assert_eq!(planned["adopted_owner"], true);
    assert_eq!(planned["save_auth_state"]["owner_user_id"], "456");
    assert_eq!(
        planned["save_auth_state"]["owner_claim_reason"],
        "existing_private_conversation_binding"
    );
    assert!(planned["send_message_text"].is_null());
}

#[test]
fn owner_bootstrap_state_dependencies_plan_existing_binding_reads() {
    let mut auth_required = base_request();
    auth_required["kind"] = json!("state_dependencies");
    auth_required.as_object_mut().unwrap().remove("auth_state");
    let auth_required = agent_telegram_owner_bootstrap_plan_json(&auth_required).unwrap();
    assert_eq!(auth_required["kind"], "state_dependencies");
    assert_eq!(auth_required["decision"], "auth_state_required");
    assert_eq!(auth_required["load_auth_state"], true);
    assert_eq!(auth_required["load_existing_binding"], false);

    let mut private_candidate = base_request();
    private_candidate["kind"] = json!("state_dependencies");
    private_candidate["auth_state"] = json!({});
    let private_candidate = agent_telegram_owner_bootstrap_plan_json(&private_candidate).unwrap();
    assert_eq!(
        private_candidate["decision"],
        "existing_private_binding_candidate"
    );
    assert_eq!(private_candidate["load_auth_state"], true);
    assert_eq!(private_candidate["load_existing_binding"], true);

    for auth_state in [
        json!({"owner_user_id": "456"}),
        json!({"pending_user_id": "456"}),
        json!({"blacklist": {"456": {"attempt_count": 3}}}),
    ] {
        let mut request = base_request();
        request["kind"] = json!("state_dependencies");
        request["auth_state"] = auth_state;
        let planned = agent_telegram_owner_bootstrap_plan_json(&request).unwrap();
        assert_eq!(planned["decision"], "existing_private_binding_not_needed");
        assert_eq!(planned["load_auth_state"], true);
        assert_eq!(planned["load_existing_binding"], false);
    }

    let mut group_chat = base_request();
    group_chat["kind"] = json!("state_dependencies");
    group_chat["chat"] = json!({"id": -100, "type": "supergroup", "title": "Team"});
    group_chat["auth_state"] = json!({});
    let group_chat = agent_telegram_owner_bootstrap_plan_json(&group_chat).unwrap();
    assert_eq!(
        group_chat["decision"],
        "existing_private_binding_not_needed"
    );
    assert_eq!(group_chat["load_existing_binding"], false);
}

#[test]
fn owner_bootstrap_blacklists_after_three_failures() {
    let mut request = base_request();
    request["auth_state"] = json!({
        "pending_user_id": "456",
        "failed_attempts": {"456": 2},
    });
    request["raw_text"] = json!("wrong");

    let planned = agent_telegram_owner_bootstrap_plan_json(&request).unwrap();

    assert_eq!(planned["decision"], "blacklist_after_failures");
    assert_eq!(planned["handled"], true);
    assert_eq!(
        planned["save_auth_state"]["blacklist"]["456"]["attempt_count"],
        3
    );
    assert!(planned["save_auth_state"].get("pending_user_id").is_none());
    assert_eq!(
        planned["send_message_text"],
        "Incorrect password. This Telegram user id is now blocked until local reset clears the runtime auth state."
    );
}

#[test]
fn owner_bootstrap_preserves_unknown_state_on_incorrect_password() {
    let mut request = base_request();
    request["auth_state"] = json!({
        "audit_marker": "keep",
        "pending_user_id": "456",
        "pending_started_at": "2026-07-01T00:00:00Z",
        "failed_attempts": {"456": 1},
    });
    request["raw_text"] = json!("wrong");

    let planned = agent_telegram_owner_bootstrap_plan_json(&request).unwrap();

    assert_eq!(planned["decision"], "incorrect_password");
    assert_eq!(planned["handled"], true);
    assert_eq!(planned["save_auth_state"]["audit_marker"], "keep");
    assert_eq!(
        planned["save_auth_state"]["pending_started_at"],
        "2026-07-01T00:00:00Z"
    );
    assert_eq!(planned["save_auth_state"]["failed_attempts"]["456"], 2);
    assert_eq!(planned["remaining_attempts"], 1);
}

#[test]
fn owner_bootstrap_blocks_non_owner_after_claim() {
    let mut request = base_request();
    request["auth_state"] = json!({"owner_user_id": "456"});
    request["from_user"] = json!({"id": 789, "username": "mallory"});

    let planned = agent_telegram_owner_bootstrap_plan_json(&request).unwrap();

    assert_eq!(planned["decision"], "owner_mismatch");
    assert_eq!(planned["handled"], true);
    assert_eq!(planned["blocked"], true);
    assert!(planned["save_auth_state"].is_null());
    assert!(planned["send_message_text"].is_null());
}

#[test]
fn owner_bootstrap_defaults_aliases_and_error_contract_are_stable() {
    let mut default_request = base_request();
    default_request.as_object_mut().unwrap().remove("kind");
    default_request["command_name"] = json!("start");

    let default_plan = agent_telegram_owner_bootstrap_plan_json(&default_request).unwrap();
    assert_eq!(default_plan["kind"], "handle");
    assert_eq!(default_plan["transport"], "telegram");
    assert_eq!(default_plan["rust_event_loop_required"], true);
    assert_eq!(default_plan["python_owner_bootstrap_allowed"], false);
    assert_eq!(default_plan["decision"], "prompt_start");

    let alias = agent_telegram_owner_bootstrap_plan_json(&json!({
        "stage": "gate",
        "owner_bootstrap_enabled": false,
    }))
    .unwrap();
    assert_eq!(alias["kind"], "handle");
    assert_eq!(alias["decision"], "disabled");
    assert_eq!(alias["handled"], false);
    assert_eq!(alias["blocked"], false);

    let invalid = agent_telegram_owner_bootstrap_plan_json(&json!("bad"));
    assert_eq!(invalid.unwrap_err(), "request must be a JSON object");

    let unsupported = agent_telegram_owner_bootstrap_plan_json(&json!({
        "kind": "unknown"
    }));
    assert_eq!(
        unsupported.unwrap_err(),
        "unsupported Telegram owner bootstrap plan kind `unknown`"
    );
}

#[test]
fn owner_bootstrap_gate_branches_are_stable() {
    let mut missing_user = base_request();
    missing_user["from_user"] = json!({"username": "weita"});
    let missing_user = agent_telegram_owner_bootstrap_plan_json(&missing_user).unwrap();
    assert_eq!(missing_user["decision"], "missing_user_id");
    assert_eq!(missing_user["handled"], true);
    assert_eq!(missing_user["blocked"], true);

    let mut pending_other = base_request();
    pending_other["auth_state"] = json!({"pending_user_id": "999"});
    let pending_other = agent_telegram_owner_bootstrap_plan_json(&pending_other).unwrap();
    assert_eq!(pending_other["decision"], "pending_other_user");
    assert_eq!(pending_other["pending_user_id"], "999");
    assert_eq!(pending_other["blocked"], true);

    let awaiting_start = agent_telegram_owner_bootstrap_plan_json(&base_request()).unwrap();
    assert_eq!(awaiting_start["decision"], "awaiting_start");
    assert_eq!(awaiting_start["handled"], true);
    assert_eq!(awaiting_start["blocked"], true);

    let mut command_instead_of_plain_text = base_request();
    command_instead_of_plain_text["auth_state"] = json!({"pending_user_id": "456"});
    command_instead_of_plain_text["command"] = json!(["help"]);
    let plain_text_required =
        agent_telegram_owner_bootstrap_plan_json(&command_instead_of_plain_text).unwrap();
    assert_eq!(plain_text_required["decision"], "plain_text_required");
    assert_eq!(
        plain_text_required["send_message_text"],
        "Send the bootstrap password as plain text."
    );

    let mut config_password = base_request();
    config_password
        .as_object_mut()
        .unwrap()
        .remove("expected_password");
    config_password["auth_state"] = json!({"pending_user_id": "456"});
    config_password["config"] = json!({"repo_name": "ait-core"});
    config_password["raw_text"] = json!("ait-core");
    let config_password = agent_telegram_owner_bootstrap_plan_json(&config_password).unwrap();
    assert_eq!(config_password["decision"], "owner_verified");
    assert_eq!(config_password["save_auth_state"]["owner_user_id"], "456");
}

#[test]
fn owner_bootstrap_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramOwnerBootstrapPlanner = &DefaultTelegramOwnerBootstrapPlanner;
    let mut request = base_request();
    request["command_name"] = json!("start");

    let planned = planner.plan_json(&request).unwrap();

    assert_eq!(planned["kind"], "handle");
    assert_eq!(planned["decision"], "prompt_start");
}

#[test]
fn owner_bootstrap_bound_entrypoint_accepts_substitute_planner() {
    struct StubOwnerBootstrapPlanner;

    impl TelegramOwnerBootstrapPlanner for StubOwnerBootstrapPlanner {
        fn plan_json(
            &self,
            request: &ait_core::json_support::JsonValue,
        ) -> Result<ait_core::json_support::JsonValue, String> {
            Ok(json!({
                "kind": "stubbed",
                "observed_kind": request.get("kind").cloned().unwrap_or(ait_core::json_support::JsonValue::Null),
            }))
        }
    }

    let planned = plan_with_telegram_owner_bootstrap_planner(
        &StubOwnerBootstrapPlanner,
        &json!({
            "kind": "handle",
            "owner_bootstrap_enabled": true,
        }),
    )
    .unwrap();

    assert_eq!(planned["kind"], "stubbed");
    assert_eq!(planned["observed_kind"], "handle");
}
