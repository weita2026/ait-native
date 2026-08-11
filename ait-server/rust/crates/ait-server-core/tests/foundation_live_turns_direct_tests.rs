use ait_server_core::foundation::live_turns::{
    live_turns_contract, live_turns_json_with_registry, LiveTurnRegistry,
    LIVE_TURNS_CONTRACT_VERSION,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

fn object(payload: JsonValue) -> JsonMap<String, JsonValue> {
    payload.as_object().expect("object").clone()
}

#[test]
fn live_turns_contract_names_reference_and_snapshot_fields() {
    let contract = live_turns_contract();
    assert_eq!(contract["contract"], json!(LIVE_TURNS_CONTRACT_VERSION));
    assert_eq!(contract["reference_modules"], json!([]));
    assert_eq!(
        contract["migration_status"],
        json!("python_wrapper_removed_rust_owned")
    );
    assert_eq!(contract["state"]["storage"], json!("in_memory"));
    assert!(contract["snapshot_fields"]
        .as_array()
        .expect("fields")
        .iter()
        .any(|field| field == "recent_completed_p95_seconds"));
    assert_eq!(
        contract["compatibility_notes"]["task_dag"],
        json!("Task DAG is retired; live turns expose no graph-progress injection.")
    );
}

#[test]
fn live_turns_rejects_invalid_limits_and_blank_repo_names() {
    assert_eq!(
        LiveTurnRegistry::new(0).expect_err("zero limit"),
        "recent_completed_limit must be greater than zero"
    );
    let mut registry = LiveTurnRegistry::new(20).expect("registry");
    let error =
        live_turns_json_with_registry(&mut registry, "start", &object(json!({"repo_name": "   "})))
            .expect_err("blank repo");
    assert_eq!(error, "repo_name is required");
}

#[test]
fn live_turns_start_finish_and_snapshot_match_contract_shape() {
    let mut registry = LiveTurnRegistry::new(20).expect("registry");
    let started = live_turns_json_with_registry(
        &mut registry,
        "start",
        &object(json!({
            "repo_name": " ait-server ",
            "surface": " session ",
            "started_at_epoch_seconds": 10.0,
            "turn_token": "turn-a",
            "metadata": {"request_id": "req-1"},
            "extra_metadata": {"model": "gpt-test"}
        })),
    )
    .expect("start");
    assert_eq!(started["turn_token"], json!("turn-a"));

    let active_snapshot =
        live_turns_json_with_registry(&mut registry, "snapshot", &object(json!({"now": 12.5})))
            .expect("snapshot");
    let active = &active_snapshot["snapshot"];
    assert_eq!(active["active_turns"], json!(1));
    assert_eq!(active["active_turn_count"], json!(1));
    assert_eq!(active["active_repositories"], json!({"ait-server": 1}));
    assert_eq!(active["active_turns_by_repo"], json!({"ait-server": 1}));
    assert_eq!(active["oldest_active_turn_started_at"], json!(10.0));
    assert_eq!(active["oldest_active_turn_age_seconds"], json!(2.5));

    let finished = live_turns_json_with_registry(
        &mut registry,
        "finish",
        &object(json!({
            "turn_token": "turn-a",
            "finished_at_epoch_seconds": 15.0,
            "completion_metadata": {
                "ok": true,
                "metadata": {"tokens": 42},
                "latency_bucket": "fast"
            }
        })),
    )
    .expect("finish");
    let turn = &finished["turn"];
    assert_eq!(turn["turn_token"], json!("turn-a"));
    assert_eq!(turn["repo_name"], json!("ait-server"));
    assert_eq!(turn["surface"], json!("session"));
    assert_eq!(turn["duration_seconds"], json!(5.0));
    assert_eq!(turn["outcome"], json!("ok"));
    assert_eq!(turn["failed"], json!(false));
    assert_eq!(
        turn["metadata"],
        json!({"request_id": "req-1", "model": "gpt-test"})
    );
    assert_eq!(
        turn["completion_metadata"],
        json!({"tokens": 42, "latency_bucket": "fast"})
    );

    let complete_snapshot =
        live_turns_json_with_registry(&mut registry, "snapshot", &object(json!({"now": 16.0})))
            .expect("snapshot");
    let complete = &complete_snapshot["snapshot"];
    assert_eq!(complete["active_turn_count"], json!(0));
    assert_eq!(complete["recent_completed_turn_count"], json!(1));
    assert_eq!(complete["recent_failed_turn_count"], json!(0));
    assert_eq!(complete["recent_completed_p95_seconds"], json!(5.0));
}

#[test]
fn live_turns_finish_missing_token_returns_empty_payload() {
    let mut registry = LiveTurnRegistry::new(20).expect("registry");
    let blank = live_turns_json_with_registry(
        &mut registry,
        "finish",
        &object(json!({"turn_token": " ", "completion_metadata": {"ok": false}})),
    )
    .expect("finish");
    assert_eq!(blank["turn"], json!({}));
    let missing = live_turns_json_with_registry(
        &mut registry,
        "finish",
        &object(json!({"turn_token": "unknown", "completion_metadata": {"ok": false}})),
    )
    .expect("finish");
    assert_eq!(missing["turn"], json!({}));
}

#[test]
fn live_turns_derives_failed_outcomes_and_error_fields() {
    let mut registry = LiveTurnRegistry::new(20).expect("registry");
    live_turns_json_with_registry(
        &mut registry,
        "start",
        &object(json!({
            "repo_name": "ait-server",
            "started_at_epoch_seconds": 1.0,
            "turn_token": "turn-fail"
        })),
    )
    .expect("start");
    let failed = live_turns_json_with_registry(
        &mut registry,
        "finish",
        &object(json!({
            "turn_token": "turn-fail",
            "finished_at_epoch_seconds": 0.5,
            "completion_metadata": {
                "error": "boom",
                "detail": "visible"
            }
        })),
    )
    .expect("finish");
    let turn = &failed["turn"];
    assert_eq!(turn["duration_seconds"], json!(0.0));
    assert_eq!(turn["outcome"], json!("failed"));
    assert_eq!(turn["failed"], json!(true));
    assert_eq!(turn["error"], json!("boom"));
    assert_eq!(turn["completion_metadata"], json!({"detail": "visible"}));
}

#[test]
fn live_turns_snapshot_recent_limit_and_p95_follow_python_order() {
    let mut registry = LiveTurnRegistry::new(20).expect("registry");
    for index in 1..=4 {
        let token = format!("turn-{index}");
        live_turns_json_with_registry(
            &mut registry,
            "start",
            &object(json!({
                "repo_name": "ait-server",
                "started_at_epoch_seconds": 0.0,
                "turn_token": token
            })),
        )
        .expect("start");
        live_turns_json_with_registry(
            &mut registry,
            "finish",
            &object(json!({
                "turn_token": format!("turn-{index}"),
                "finished_at_epoch_seconds": index,
                "completion_metadata": {"status": "completed"}
            })),
        )
        .expect("finish");
    }

    let limited = live_turns_json_with_registry(
        &mut registry,
        "snapshot",
        &object(json!({"now": 10.0, "recent_completed_limit": 3})),
    )
    .expect("snapshot");
    let snapshot = &limited["snapshot"];
    assert_eq!(snapshot["recent_completed_turn_count"], json!(3));
    assert_eq!(snapshot["recent_completed_p95_seconds"], json!(4.0));
    assert_eq!(
        snapshot["recent_completed_turns"][0]["turn_token"],
        json!("turn-4")
    );
    assert_eq!(
        snapshot["recent_completed_turns"][2]["turn_token"],
        json!("turn-2")
    );

    let zero = live_turns_json_with_registry(
        &mut registry,
        "snapshot",
        &object(json!({"now": 10.0, "recent_completed_limit": 0})),
    )
    .expect("snapshot");
    assert_eq!(zero["snapshot"]["recent_completed_turn_count"], json!(0));

    let error = live_turns_json_with_registry(
        &mut registry,
        "snapshot",
        &object(json!({"now": 10.0, "recent_completed_limit": -1})),
    )
    .expect_err("negative limit");
    assert_eq!(
        error,
        "recent_completed_limit must be greater than or equal to zero"
    );
}
