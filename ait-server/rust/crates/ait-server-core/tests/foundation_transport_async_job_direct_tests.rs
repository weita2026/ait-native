use ait_server_core::foundation::async_job_json::AsyncJobJson;
use ait_server_core::foundation::transport::{
    elapsed_ms, land_freshness_result, land_request_json, land_snapshot_alignment,
    normalize_async_job_payload, retry_delay_seconds_for_job,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

fn row(value: JsonValue) -> JsonMap<String, JsonValue> {
    value.as_object().cloned().expect("row should be an object")
}

#[test]
fn normalize_async_job_payload_rejects_unsupported_job_type() {
    let payload = JsonMap::new();
    let error = normalize_async_job_payload("content.unknown", Some(&payload))
        .expect_err("unsupported job type should be rejected");
    assert!(error.starts_with("Unsupported async job type: content.unknown."));
}

#[test]
fn normalize_async_job_payload_rejects_missing_required_field() {
    let payload = JsonMap::new();
    let error = normalize_async_job_payload("content.gc", Some(&payload))
        .expect_err("missing required field should be rejected");
    assert_eq!(error, "content.gc requires payload field `repo_name`.");
}

#[test]
fn normalize_async_job_payload_rejects_unsupported_field() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), JsonValue::String("repo-alpha".into()));
    payload.insert("extra_field".into(), JsonValue::String("nope".into()));

    let error = normalize_async_job_payload("content.gc", Some(&payload))
        .expect_err("unsupported field should be rejected");
    assert_eq!(
        error,
        "content.gc payload has unsupported field(s): extra_field"
    );
}

#[test]
fn normalize_async_job_payload_rejects_invalid_bool_coercion() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), JsonValue::String("repo-alpha".into()));
    payload.insert(
        "prune_unreferenced".into(),
        JsonValue::String("maybe".into()),
    );

    let error = normalize_async_job_payload("content.gc", Some(&payload))
        .expect_err("invalid bool coercion should be rejected");
    assert_eq!(
        error,
        "content.gc payload field `prune_unreferenced` must be a boolean."
    );
}

#[test]
fn normalize_async_job_payload_rejects_invalid_positive_integer_coercion() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), JsonValue::String("repo-alpha".into()));
    payload.insert("max_members".into(), json!(0));

    let error = normalize_async_job_payload("content.pack", Some(&payload))
        .expect_err("invalid positive integer coercion should be rejected");
    assert_eq!(
        error,
        "content.pack payload field `max_members` must be greater than zero when set."
    );
}

#[test]
fn normalize_async_job_payload_applies_defaults() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), JsonValue::String("repo-alpha".into()));

    let normalized = normalize_async_job_payload("content.gc", Some(&payload))
        .expect("defaults should apply for omitted optional fields");

    assert_eq!(normalized.get("repo_name"), Some(&json!("repo-alpha")));
    assert_eq!(normalized.get("prune_unreferenced"), Some(&json!(true)));
    assert_eq!(normalized.get("prune_orphan_packs"), Some(&json!(true)));
}

#[test]
fn async_job_json_wrapper_preserves_contract_defaults_retry_and_result_shapes() {
    let json = AsyncJobJson::stateless();
    let contract = json.async_job_contract();
    let content_gc = contract
        .iter()
        .find(|entry| entry.get("job_type") == Some(&json!("content.gc")))
        .expect("content.gc contract");
    assert_eq!(content_gc["required"]["repo_name"], json!("str"));
    assert_eq!(
        content_gc["optional"]["prune_unreferenced"]["default"],
        json!(true)
    );
    assert_eq!(content_gc["max_attempts"], json!(3));
    assert_eq!(content_gc["retry_delay_seconds"], json!(3));

    let payload = row(json!({
        "repo_name": "housekeeper"
    }));
    let normalized = json
        .normalize_async_job_payload("content.gc", Some(&payload))
        .expect("normalized content.gc payload");
    assert_eq!(
        normalized,
        row(json!({
            "repo_name": "housekeeper",
            "prune_unreferenced": true,
            "prune_orphan_packs": true
        }))
    );
    assert_eq!(json.retry_delay_seconds_for_job("content.gc"), 3);
    assert_eq!(json.max_attempts_for_job("content.gc"), 3);

    let job = json
        .row_to_job(&row(json!({
            "job_id": 7,
            "repo_name": "housekeeper",
            "job_type": "content.gc",
            "state": "queued",
            "payload_json": "{\"repo_name\":\"housekeeper\"}",
            "result_json": "{\"status\":\"queued\"}",
            "attempt_count": 1,
            "max_attempts": 3,
            "available_at": "2026-06-27T13:10:00+08:00",
            "locked_at": null,
            "locked_by": null,
            "last_error": "transient failure"
        })))
        .expect("job payload");
    assert_eq!(job["payload"], json!({"repo_name": "housekeeper"}));
    assert_eq!(job["result"], json!({"status": "queued"}));
    assert_eq!(job["attempts_remaining"], json!(2));
    assert_eq!(job["retry_pending"], json!(true));
    assert_eq!(job["next_retry_at"], json!("2026-06-27T13:10:00+08:00"));
    assert_eq!(job["diagnostic_status"], json!("retry_pending"));

    let land_payload = json
        .land_request_payload(&row(json!({
            "priority": 99,
            "result_json": "{\"status\":\"pass\",\"landed\":true}"
        })))
        .expect("land request payload");
    assert!(land_payload.get("priority").is_none());
    assert_eq!(
        land_payload["result"],
        json!({"status": "pass", "landed": true})
    );

    let timings = json.phase_timings_from_result(Some(&json!({
        "phase_timings_ms": {
            "snapshot": 1.25,
            "publish": 2.5
        }
    })));
    assert_eq!(timings["snapshot"], json!(1.25));
    assert_eq!(timings["publish"], json!(2.5));
}

#[test]
fn async_job_json_wrapper_preserves_exact_error_text() {
    let json = AsyncJobJson::stateless();
    let payload = row(json!({
        "repo_name": "housekeeper",
        "prune_unreferenced": "maybe"
    }));

    assert_eq!(
        json.normalize_async_job_payload("content.gc", Some(&payload))
            .expect_err("invalid bool"),
        "content.gc payload field `prune_unreferenced` must be a boolean."
    );
    assert_eq!(
        json.land_request_payload(&row(json!({})))
            .expect_err("missing result json"),
        "result_json must be a JSON string"
    );
}

#[test]
fn land_request_payload_preserves_status_rows_and_phase_timings() {
    let json = AsyncJobJson::stateless();
    for (status, result) in [
        (
            "queued",
            json!({"freshness_preflight": {"base_is_fresh": true}}),
        ),
        ("running", json!({"phase": "process_land"})),
        (
            "blocked",
            json!({
                "blocker_class": "BASE_STALE",
                "freshness_preflight": {"base_is_fresh": false}
            }),
        ),
        (
            "succeeded",
            json!({
                "line_action": "already_at_selected_patchset_revision",
                "freshness_preflight": {"already_aligned_equivalent": true}
            }),
        ),
    ] {
        let land = json
            .land_request_payload(&row(json!({
                "submission_id": "RSEL-1",
                "priority": 99,
                "status": status,
                "result_json": serde_json::to_string(&result).expect("result JSON")
            })))
            .expect("land request payload");
        assert_eq!(land["status"], json!(status));
        assert_eq!(land["result"], result);
        assert!(land.get("priority").is_none());
    }

    assert_eq!(elapsed_ms(10.0, 10.1234567), 123.457);
    assert!(json
        .phase_timings_from_result(Some(&json!({"phase_timings_ms": []})))
        .is_empty());
}

#[test]
fn land_snapshot_alignment_matches_python_payload_policy() {
    let exact = land_snapshot_alignment(
        Some(" SNP-REV "),
        Some("SNP-REV"),
        Some("manifest-target"),
        Some("manifest-revision"),
        Some("TREE-TARGET"),
        Some("TREE-REVISION"),
    );

    assert_eq!(exact["target_line_head"], json!("SNP-REV"));
    assert_eq!(exact["revision_snapshot_id"], json!("SNP-REV"));
    assert_eq!(exact["target_matches_revision_snapshot"], json!(true));
    assert_eq!(exact["target_matches_revision_tree"], json!(true));
    assert_eq!(exact["already_aligned_equivalent"], json!(true));
    assert_eq!(exact["target_manifest_hash"], JsonValue::Null);
    assert_eq!(exact["target_root_tree_id"], JsonValue::Null);

    let equivalent_tree = land_snapshot_alignment(
        Some("SNP-LATER"),
        Some("SNP-REV"),
        Some("manifest-later"),
        Some("manifest-rev"),
        Some("TREE-SAME"),
        Some("TREE-SAME"),
    );
    assert_eq!(
        equivalent_tree["target_matches_revision_snapshot"],
        json!(false)
    );
    assert_eq!(equivalent_tree["target_matches_revision_tree"], json!(true));
    assert_eq!(equivalent_tree["already_aligned_equivalent"], json!(true));
    assert_eq!(
        equivalent_tree["target_manifest_hash"],
        json!("manifest-later")
    );
    assert_eq!(equivalent_tree["revision_root_tree_id"], json!("TREE-SAME"));

    let stale = land_snapshot_alignment(
        Some("SNP-OTHER"),
        Some("SNP-REV"),
        Some("manifest-other"),
        Some("manifest-rev"),
        Some("TREE-OTHER"),
        Some("TREE-REV"),
    );
    assert_eq!(stale["target_matches_revision_snapshot"], json!(false));
    assert_eq!(stale["target_matches_revision_tree"], json!(false));
    assert_eq!(stale["already_aligned_equivalent"], json!(false));
}

#[test]
fn land_freshness_result_covers_fresh_stale_and_already_aligned_states() {
    let patchset = row(json!({
        "base_snapshot_id": "SNP-BASE",
        "revision_snapshot_id": "SNP-REV"
    }));

    let accepted = land_freshness_result(
        "main",
        &patchset,
        Some(" SNP-BASE "),
        None,
        "2026-07-08T10:00:00Z",
    );
    assert_eq!(accepted["base_is_fresh"], json!(true));
    assert_eq!(accepted["already_aligned_equivalent"], json!(false));

    let stale = land_freshness_result(
        "main",
        &patchset,
        Some("SNP-OTHER"),
        None,
        "2026-07-08T10:00:01Z",
    );
    assert_eq!(stale["base_is_fresh"], json!(false));
    assert_eq!(stale["target_matches_revision_tree"], json!(false));
    assert_eq!(stale["already_aligned_equivalent"], json!(false));

    let exact_revision = land_freshness_result(
        "main",
        &patchset,
        Some("SNP-REV"),
        None,
        "2026-07-08T10:00:02Z",
    );
    assert_eq!(
        exact_revision["target_matches_revision_snapshot"],
        json!(true)
    );
    assert_eq!(exact_revision["already_aligned_equivalent"], json!(true));

    let equivalent_tree_alignment = land_snapshot_alignment(
        Some("SNP-LATER"),
        Some("SNP-REV"),
        Some("manifest-later"),
        Some("manifest-rev"),
        Some("TREE-SAME"),
        Some("TREE-SAME"),
    );
    let aligned = land_freshness_result(
        "main",
        &patchset,
        Some("SNP-LATER"),
        Some(&equivalent_tree_alignment),
        "2026-07-08T10:00:03Z",
    );
    assert_eq!(aligned["base_is_fresh"], json!(false));
    assert_eq!(aligned["target_matches_revision_snapshot"], json!(false));
    assert_eq!(aligned["target_matches_revision_tree"], json!(true));
    assert_eq!(aligned["already_aligned_equivalent"], json!(true));

    let nonempty_alignment_missing_bool_fields = row(json!({"target_manifest_hash": "mh"}));
    let python_compat_missing_field = land_freshness_result(
        "main",
        &patchset,
        Some("SNP-REV"),
        Some(&nonempty_alignment_missing_bool_fields),
        "2026-07-08T10:00:04Z",
    );
    assert_eq!(
        python_compat_missing_field["target_matches_revision_snapshot"],
        json!(false)
    );
    assert_eq!(
        python_compat_missing_field["already_aligned_equivalent"],
        json!(false)
    );
}

#[test]
fn land_request_json_exposes_contract_operations() {
    let contract =
        land_request_json("contract", &json!({})).expect("contract operation should work");
    assert_eq!(contract["contract"], json!("ait.server.land_request.v1"));
    assert_eq!(
        contract["reference_modules"],
        json!(["../ait/src/ait_native/server_api.py"])
    );
    assert_eq!(
        contract["migration_status"],
        json!("payload_wrapper_removed_rust_owned")
    );
    assert_eq!(contract["mutates_state"], json!(false));
    assert!(contract["operations"]
        .as_array()
        .expect("operations should be an array")
        .contains(&json!("snapshot-alignment")));

    let payload = land_request_json(
        "payload",
        &json!({
            "row": {
                "submission_id": "RSEL-1",
                "priority": 50,
                "status": "blocked",
                "result_json": "{\"blocker_class\":\"BASE_STALE\"}"
            }
        }),
    )
    .expect("payload operation should work");
    assert_eq!(payload["land_request"]["submission_id"], json!("RSEL-1"));
    assert_eq!(
        payload["land_request"]["result"]["blocker_class"],
        json!("BASE_STALE")
    );
    assert!(payload["land_request"].get("priority").is_none());
}

#[test]
fn normalize_async_job_payload_accepts_agent_recovered_turn_job() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), json!("ait"));
    payload.insert("idempotency_key".into(), json!("idem-1"));
    payload.insert("payload".into(), json!({"message": "hello"}));

    let normalized = normalize_async_job_payload("agent.turn.submit", Some(&payload))
        .expect("agent turn jobs should normalize");

    assert_eq!(normalized.get("repo_name"), Some(&json!("ait")));
    assert_eq!(normalized.get("idempotency_key"), Some(&json!("idem-1")));
    assert_eq!(
        normalized.get("payload"),
        Some(&json!({"message": "hello"}))
    );
    assert_eq!(normalized.get("transport"), Some(&JsonValue::Null));
}

#[test]
fn normalize_async_job_payload_accepts_patchset_ci_aggregate_job() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), json!("ait"));
    payload.insert("patchset_id".into(), json!("RP-1"));
    payload.insert(
        "suite_ids".into(),
        json!(["package_smoke", "preflight", "stable_smoke"]),
    );
    payload.insert("stage".into(), json!("ready_blocking"));
    payload.insert("revision_snapshot_id".into(), json!("SNP-1"));

    let normalized = normalize_async_job_payload("patchset.ci.aggregate", Some(&payload))
        .expect("patchset CI aggregation jobs should normalize");

    assert_eq!(normalized.get("repo_name"), Some(&json!("ait")));
    assert_eq!(normalized.get("patchset_id"), Some(&json!("RP-1")));
    assert_eq!(
        normalized.get("suite_ids"),
        Some(&json!(["package_smoke", "preflight", "stable_smoke"]))
    );
    assert_eq!(normalized.get("stage"), Some(&json!("ready_blocking")));
    assert_eq!(
        normalized.get("revision_snapshot_id"),
        Some(&json!("SNP-1"))
    );
}

#[test]
fn normalize_async_job_payload_accepts_main_seed_refresh_job() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), json!("ait-server"));
    payload.insert("snapshot_id".into(), json!("SNP-LANDED"));
    payload.insert("patchset_id".into(), json!("RSEP-1"));
    payload.insert("previous_snapshot_id".into(), json!("SNP-BASE"));
    payload.insert("target_line".into(), json!("main"));
    payload.insert("trigger".into(), json!("remote_land"));

    let normalized = normalize_async_job_payload("main-seed.refresh", Some(&payload))
        .expect("main seed refresh jobs should normalize");

    assert_eq!(normalized.get("repo_name"), Some(&json!("ait-server")));
    assert_eq!(normalized.get("snapshot_id"), Some(&json!("SNP-LANDED")));
    assert_eq!(normalized.get("patchset_id"), Some(&json!("RSEP-1")));
    assert_eq!(
        normalized.get("previous_snapshot_id"),
        Some(&json!("SNP-BASE"))
    );
    assert_eq!(normalized.get("target_line"), Some(&json!("main")));
    assert_eq!(normalized.get("trigger"), Some(&json!("remote_land")));
    assert_eq!(retry_delay_seconds_for_job("main-seed.refresh"), 3);
}

#[test]
fn normalize_async_job_payload_accepts_patchset_ci_profile_fields() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), json!("ait"));
    payload.insert("patchset_id".into(), json!("RP-1"));
    payload.insert("trigger".into(), json!("workflow_ready_apply"));
    payload.insert(
        "execution_profile".into(),
        json!("workflow_ready_foreground"),
    );
    payload.insert("suite_ids".into(), json!(["preflight", "full"]));

    let normalized = normalize_async_job_payload("patchset.ci", Some(&payload))
        .expect("patchset CI jobs should accept ready execution profile metadata");

    assert_eq!(normalized.get("repo_name"), Some(&json!("ait")));
    assert_eq!(normalized.get("patchset_id"), Some(&json!("RP-1")));
    assert_eq!(
        normalized.get("trigger"),
        Some(&json!("workflow_ready_apply"))
    );
    assert_eq!(
        normalized.get("execution_profile"),
        Some(&json!("workflow_ready_foreground"))
    );
    assert_eq!(
        normalized.get("suite_ids"),
        Some(&json!(["preflight", "full"]))
    );
    assert_eq!(normalized.get("runtime_payload"), Some(&JsonValue::Null));
}

#[test]
fn normalize_async_job_payload_accepts_ci_runtime_payloads() {
    let mut patchset_payload = JsonMap::new();
    patchset_payload.insert("repo_name".into(), json!("ait-server"));
    patchset_payload.insert("patchset_id".into(), json!("RSEP-1"));
    patchset_payload.insert(
        "runtime_payload".into(),
        json!({"prewarm_commands": ["./ci/prewarm.sh"]}),
    );

    let normalized_patchset = normalize_async_job_payload("patchset.ci", Some(&patchset_payload))
        .expect("patchset CI jobs should accept Rust runtime payload");
    assert_eq!(
        normalized_patchset.get("runtime_payload"),
        Some(&json!({"prewarm_commands": ["./ci/prewarm.sh"]}))
    );

    let mut repo_payload = JsonMap::new();
    repo_payload.insert("repo_name".into(), json!("ait-server"));
    repo_payload.insert(
        "runtime_payload".into(),
        json!({"prewarm_commands": ["./ci/prewarm.sh"]}),
    );

    let normalized_repo = normalize_async_job_payload("repo.ci", Some(&repo_payload))
        .expect("repo CI jobs should accept Rust runtime payload");
    assert_eq!(
        normalized_repo.get("runtime_payload"),
        Some(&json!({"prewarm_commands": ["./ci/prewarm.sh"]}))
    );
}

#[test]
fn normalize_async_job_payload_rejects_non_object_agent_payload() {
    let mut payload = JsonMap::new();
    payload.insert("repo_name".into(), json!("ait"));
    payload.insert("idempotency_key".into(), json!("idem-1"));
    payload.insert("payload".into(), json!("not-object"));

    let error = normalize_async_job_payload("agent.turn.submit", Some(&payload))
        .expect_err("agent payload should be an object");

    assert_eq!(
        error,
        "agent.turn.submit payload field `payload` must be a JSON object."
    );
}

#[test]
fn retry_delay_seconds_for_job_falls_back_when_type_is_unknown() {
    assert_eq!(retry_delay_seconds_for_job("content.gc"), 3);
    assert_eq!(retry_delay_seconds_for_job("agent.turn.submit"), 3);
    assert_eq!(retry_delay_seconds_for_job("content.unknown"), 3);
}
