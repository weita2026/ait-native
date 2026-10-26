use super::*;

#[test]
fn default_registry_contains_rust_contract_flags() {
    let planned = agent_telegram_event_trigger_plan_json(&json!({"stage": "default_registry"}))
        .expect("plan");
    assert_eq!(
        planned["event_trigger_contract"],
        "ait_agent_core.event_loop.TelegramEventTrigger.v1"
    );
    assert_eq!(planned["python_event_trigger_allowed"], false);
    assert_eq!(
        planned["registry"]["fresh_topic"]["clear"]["phrases"],
        json!(["換個話題", "換個主題"])
    );
}

#[test]
fn event_trigger_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramEventTriggerPlanner = &DefaultTelegramEventTriggerPlanner;

    let planned = planner
        .plan_json(&json!({"stage": "default_registry"}))
        .expect("plan");

    assert_eq!(planned["stage"], "default_registry");
    assert_eq!(
        planned["event_trigger_contract"],
        "ait_agent_core.event_loop.TelegramEventTrigger.v1"
    );
}

#[test]
fn event_trigger_bound_entrypoint_accepts_substitute_planner() {
    struct StubEventTriggerPlanner;

    impl TelegramEventTriggerPlanner for StubEventTriggerPlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": "stubbed",
                "observed_stage": request.get("stage").cloned().unwrap_or(JsonValue::Null),
            }))
        }
    }

    let planned = plan_with_telegram_event_trigger_planner(
        &StubEventTriggerPlanner,
        &json!({"stage": "default_registry"}),
    )
    .unwrap();

    assert_eq!(planned["stage"], "stubbed");
    assert_eq!(planned["observed_stage"], "default_registry");
}

#[test]
fn event_trigger_defaults_aliases_and_error_contract_are_stable() {
    let default_plan = agent_telegram_event_trigger_plan_json(&json!({})).expect("default");
    assert_eq!(default_plan["stage"], "normalize_registry");
    assert_eq!(default_plan["transport"], "telegram");
    assert_eq!(default_plan["rust_event_loop_required"], true);
    assert_eq!(default_plan["python_event_trigger_allowed"], false);
    assert_eq!(default_plan["registry"]["telegram_operational"], json!([]));

    let alias_plan = agent_telegram_event_trigger_plan_json(&json!({"kind": "default_registry"}))
        .expect("kind alias");
    assert_eq!(alias_plan["stage"], "default_registry");
    assert_eq!(
        alias_plan["registry"]["planning_mode"]["phrases"],
        json!(["進行計劃", "進行計畫", "進行计划"])
    );

    let invalid = agent_telegram_event_trigger_plan_json(&json!("bad"));
    assert_eq!(
        invalid.unwrap_err(),
        "Telegram event trigger request must be an object"
    );

    let unsupported = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "unknown"
    }));
    assert_eq!(
        unsupported.unwrap_err(),
        "unsupported Telegram event trigger plan stage `unknown`"
    );

    let bad_operational = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "normalize_operational_trigger",
        "payload": {
            "kind": "telegram_operational_trigger",
            "id": "missing_handler",
            "match": {"phrases": ["go"]}
        }
    }))
    .expect("bad operational trigger is reported as absent");
    assert_eq!(bad_operational["ok"], false);
    assert!(bad_operational["trigger"].is_null());
}

#[test]
fn fresh_topic_matching_handles_clear_and_topic() {
    let clear = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "fresh_topic_match",
        "text": "換個話題！",
    }))
    .expect("clear plan");
    assert_eq!(clear["matched"], true);
    assert_eq!(clear["trigger"]["mode"], "clear");

    let topic = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "fresh_topic_match",
        "text": "換個話題跟 remote land 有關。",
    }))
    .expect("topic plan");
    assert_eq!(topic["matched"], true);
    assert_eq!(topic["trigger"]["mode"], "topic");
    assert_eq!(topic["trigger"]["topic"], "remote land");
}

#[test]
fn fresh_topic_confirmation_text_is_rust_owned_callback_output() {
    let clear = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "fresh_topic_confirmation_text",
        "fresh_topic": {"mode": "clear", "display_trigger": "換個話題"},
    }))
    .expect("confirmation plan");

    assert_eq!(clear["callback_group"], "fresh_topic_confirmation");
    assert_eq!(clear["python_event_trigger_allowed"], false);
    assert_eq!(
        clear["text"],
        "Started a fresh Telegram conversation.\nTrigger: 換個話題."
    );

    let topic = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "fresh_topic_confirmation_text",
        "fresh_topic": {
            "mode": "topic",
            "topic": "remote land",
            "display_trigger": "換個話題跟…有關"
        },
    }))
    .expect("topic confirmation plan");

    assert_eq!(
        topic["confirmation_text"],
        "Started a fresh Telegram conversation.\nTrigger: 換個話題跟…有關.\nTopic hint: remote land"
    );
}

#[test]
fn planning_mode_matching_uses_config_payload() {
    let planned = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "planning_mode_match",
        "text": "計畫一下",
        "config": {
            "phrases": ["計畫一下"],
            "display_trigger": "計畫一下",
            "allow_trailing_punctuation": true,
        },
    }))
    .expect("planning plan");
    assert_eq!(planned["matched"], true);
    assert_eq!(planned["trigger"]["mode"], "planning");
}

#[test]
fn operational_config_normalization_sorts_and_dedupes() {
    let planned = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "normalize_registry",
        "operational_triggers": [
            {
                "source_path": "docs/event_trigger/b.md",
                "payload": {
                    "kind": "telegram_operational_trigger",
                    "id": "b",
                    "handlerCommand": "python handler.py",
                    "match": {"phrases": ["go", "go"]},
                    "priority": 1
                }
            },
            {
                "source_path": "docs/event_trigger/a.md",
                "payload": {
                    "kind": "telegram_operational_trigger",
                    "id": "a",
                    "handlerCommand": ["python", "handler.py"],
                    "match": {"commands": ["/Route", "route"]},
                    "priority": 5
                }
            }
        ]
    }))
    .expect("normalize");
    let triggers = planned["registry"]["telegram_operational"]
        .as_array()
        .expect("triggers");
    assert_eq!(triggers[0]["trigger_id"], "a");
    assert_eq!(triggers[0]["match"]["commands"], json!(["route"]));
    assert_eq!(
        triggers[1]["handler_command"],
        json!(["python", "handler.py"])
    );
    assert_eq!(triggers[1]["match"]["phrases"], json!(["go"]));
}

#[test]
fn operational_matching_supports_command_phrase_and_pattern() {
    let config = json!({
        "trigger_id": "router",
        "display_trigger": "route",
        "source_path": "docs/event_trigger/router.md",
        "handler_command": ["python", "handler.py"],
        "match": {
            "phrases": ["加入黑名單"],
            "commands": ["routerblock"],
            "pattern": "^(?P<code>[A-Za-z0-9]{4})$",
            "allow_trailing_punctuation": true,
            "reply_only": false,
            "case_sensitive": false
        },
        "priority": 0
    });
    let command = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": config,
        "command": ["routerblock", "abc"],
    }))
    .expect("command match");
    assert_eq!(command["trigger"]["mode"], "command");
    assert_eq!(command["trigger"]["command_args"], "abc");

    let phrase = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": config,
        "normalized_text": "加入黑名單！",
    }))
    .expect("phrase match");
    assert_eq!(phrase["trigger"]["mode"], "phrase");

    let pattern = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": config,
        "raw_text": "AB12",
        "normalized_text": "AB12",
    }))
    .expect("pattern match");
    assert_eq!(pattern["trigger"]["mode"], "pattern");
    assert_eq!(pattern["trigger"]["groupdict"]["code"], "AB12");

    let unanchored_config = json!({
        "trigger_id": "router",
        "display_trigger": "route",
        "source_path": "docs/event_trigger/router.md",
        "handler_command": ["python", "handler.py"],
        "match": {
            "phrases": [],
            "commands": [],
            "pattern": "AB12",
            "allow_trailing_punctuation": true,
            "reply_only": false,
            "case_sensitive": false
        },
        "priority": 0
    });
    let no_search = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": unanchored_config,
        "raw_text": "xxAB12",
        "normalized_text": "xxAB12",
    }))
    .expect("pattern non-match");
    assert_eq!(no_search["matched"], false);
}

#[test]
fn operational_dispatch_selects_first_sorted_match() {
    let planned = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_dispatch",
        "normalized_text": "route!",
        "operational_triggers": [
            {
                "trigger_id": "low",
                "display_trigger": "low",
                "handler_command": ["python", "low.py"],
                "source_path": "docs/event_trigger/z.md",
                "match": {
                    "phrases": ["route"],
                    "allow_trailing_punctuation": true
                },
                "priority": 1
            },
            {
                "trigger_id": "high",
                "display_trigger": "high",
                "handler_command": ["python", "high.py"],
                "source_path": "docs/event_trigger/a.md",
                "match": {
                    "phrases": ["route"],
                    "allow_trailing_punctuation": true
                },
                "priority": 9
            }
        ]
    }))
    .expect("dispatch plan");

    assert_eq!(planned["stage"], "operational_dispatch");
    assert_eq!(planned["matched"], true);
    assert_eq!(planned["handled"], true);
    assert_eq!(planned["trigger"]["trigger_id"], "high");
    assert_eq!(planned["match_payload"]["mode"], "phrase");
    assert_eq!(planned["match_payload"]["matched_text"], "route");
    assert_eq!(planned["python_event_trigger_allowed"], false);
}

#[test]
fn operational_dispatch_handles_no_match_and_reply_only() {
    let reply_only = json!({
        "trigger_id": "reply",
        "display_trigger": "reply",
        "handler_command": ["python", "reply.py"],
        "source_path": "docs/event_trigger/reply.md",
        "match": {
            "phrases": ["reply now"],
            "reply_only": true,
            "allow_trailing_punctuation": true
        },
        "priority": 0
    });

    let no_reply = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_dispatch",
        "normalized_text": "reply now",
        "telegram_operational": [reply_only.clone()]
    }))
    .expect("no reply dispatch");
    assert_eq!(no_reply["matched"], false);
    assert!(no_reply["trigger"].is_null());
    assert!(no_reply["match_payload"].is_null());

    let with_reply = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_dispatch",
        "normalized_text": "reply now!",
        "reply_to_message_id": 42,
        "telegram_operational": [reply_only]
    }))
    .expect("reply dispatch");
    assert_eq!(with_reply["matched"], true);
    assert_eq!(with_reply["trigger"]["trigger_id"], "reply");
    assert_eq!(with_reply["match_payload"]["mode"], "phrase");
}

#[test]
fn operational_matching_edge_cases_are_stable() {
    let missing_config = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match"
    }));
    assert_eq!(
        missing_config.unwrap_err(),
        "operational_match requires config"
    );

    let non_object_config = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": "bad",
    }));
    assert_eq!(
        non_object_config.unwrap_err(),
        "operational_match config must be an object"
    );

    let invalid_pattern = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": {
            "display_trigger": "bad",
            "match": {
                "pattern": "[",
                "case_sensitive": false
            }
        },
        "raw_text": "anything",
        "normalized_text": "anything"
    }));
    assert!(
        invalid_pattern
            .unwrap_err()
            .starts_with("invalid Telegram operational trigger pattern:"),
        "invalid pattern error should keep its stable prefix"
    );

    let reply_only_config = json!({
        "display_trigger": "reply",
        "match": {
            "phrases": ["reply now"],
            "reply_only": true,
            "allow_trailing_punctuation": true,
            "case_sensitive": false
        }
    });
    let no_reply = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": reply_only_config,
        "normalized_text": "reply now",
    }))
    .expect("reply-only non-match");
    assert_eq!(no_reply["matched"], false);

    let with_reply = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": reply_only_config,
        "normalized_text": "reply now!",
        "reply_to_message_id": "42",
    }))
    .expect("reply-only match");
    assert_eq!(with_reply["matched"], true);
    assert_eq!(with_reply["trigger"]["mode"], "phrase");

    let case_sensitive_config = json!({
        "display_trigger": "Case",
        "match": {
            "phrases": ["Route"],
            "case_sensitive": true
        }
    });
    let lower_case = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": case_sensitive_config,
        "normalized_text": "route",
    }))
    .expect("case-sensitive non-match");
    assert_eq!(lower_case["matched"], false);

    let exact_case = agent_telegram_event_trigger_plan_json(&json!({
        "stage": "operational_match",
        "config": case_sensitive_config,
        "normalized_text": "Route",
    }))
    .expect("case-sensitive match");
    assert_eq!(exact_case["matched"], true);
    assert_eq!(exact_case["trigger"]["matched_text"], "Route");
}
