use ait_server_core::foundation::workflow_async_runtime::workflow_async_runtime_json;
use serde_json::json;

fn run(operation: &str, payload: serde_json::Value) -> serde_json::Value {
    workflow_async_runtime_json(operation, &payload).expect("workflow async runtime should succeed")
}

#[test]
fn queue_mode_normalizes_to_supported_values() {
    assert_eq!(run("queue-mode", json!({}))["queue_mode"], json!("inline"));
    assert_eq!(
        run("queue-mode", json!({"queue_mode": "async"}))["queue_mode"],
        json!("async")
    );
    assert_eq!(
        run("queue-mode", json!({"queue_mode": "bogus"}))["queue_mode"],
        json!("inline")
    );
}

#[test]
fn patchset_ci_execution_profile_fails_closed_for_unknown_values() {
    assert_eq!(
        run("normalize-patchset-ci-execution-profile", json!({}))["execution_profile"],
        json!("full")
    );
    assert_eq!(
        run(
            "normalize-patchset-ci-execution-profile",
            json!({"execution_profile": "workflow_ready_foreground"})
        )["execution_profile"],
        json!("workflow_ready_foreground")
    );
    let error = workflow_async_runtime_json(
        "normalize-patchset-ci-execution-profile",
        &json!({"execution_profile": "fast"}),
    )
    .expect_err("unknown profile should fail");
    assert_eq!(error, "Unsupported patchset CI execution_profile `fast`.");
}

#[test]
fn job_payloads_are_shaped_from_patchset_change_and_land_rows() {
    let patchset = json!({
        "patchset_id": "RP-1",
        "repo_id": "repo-id-1",
        "change_id": "RC-1",
        "patchset_number": 2
    });
    let change = json!({
        "repo_name": "ait",
        "repo_id": "repo-id-fallback",
        "change_id": "RC-1",
        "change_seq": 7
    });
    let policy = run(
        "policy-job-payload",
        json!({"patchset": patchset, "change": change}),
    );
    assert_eq!(policy["payload"]["patchset_id"], json!("RP-1"));
    assert_eq!(policy["payload"]["repo_name"], json!("ait"));
    assert_eq!(policy["payload"]["repo_id"], json!("repo-id-1"));
    assert_eq!(policy["payload"]["change_seq"], json!(7));
    assert_eq!(policy["payload"]["patchset_number"], json!(2));

    let patchset_ci = run(
        "patchset-ci-job-payload",
        json!({
            "patchset": {
                "patchset_id": "RP-1",
                "repo_id": "repo-id-1",
                "patchset_number": 2
            },
            "change": {
                "repo_name": "ait",
                "change_id": "RC-1",
                "change_seq": 7
            },
            "trigger": "queued_rerun",
            "execution_profile": "workflow_ready_foreground"
        }),
    );
    assert_eq!(patchset_ci["payload"]["trigger"], json!("queued_rerun"));
    assert_eq!(
        patchset_ci["payload"]["execution_profile"],
        json!("workflow_ready_foreground")
    );

    let land = run(
        "land-job-payload",
        json!({
            "land": {
                "submission_id": "LAND-1",
                "change_id": "RC-1",
                "patchset_id": "RP-1",
                "land_seq": 3
            },
            "change": {
                "repo_name": "ait",
                "repo_id": "repo-id-1",
                "change_id": "RC-1",
                "change_seq": 7
            }
        }),
    );
    assert_eq!(land["payload"]["submission_id"], json!("LAND-1"));
    assert_eq!(land["payload"]["patchset_id"], json!("RP-1"));
    assert_eq!(land["payload"]["land_seq"], json!(3));
}

#[test]
fn patchset_publish_policy_followup_matches_inline_and_async_contracts() {
    let inline = run(
        "patchset-publish-policy-followup",
        json!({"patchset_id": "RP-1", "queue_mode": "inline"}),
    );
    assert_eq!(inline["policy_followup"]["state"], json!("deferred"));
    assert_eq!(inline["policy_followup"]["queue_mode"], json!("inline"));
    assert_eq!(
        inline["policy_followup"]["command"],
        json!("ait policy eval RP-1")
    );

    let async_payload = run(
        "patchset-publish-policy-followup",
        json!({"patchset_id": "RP-1", "queue_mode": "async"}),
    );
    assert_eq!(
        async_payload["policy_followup"]["queue_mode"],
        json!("async")
    );
    assert!(async_payload["policy_followup"].get("command").is_none());
}

#[test]
fn patchset_ci_start_plan_reports_unavailable_active_or_enqueue() {
    let unavailable = run(
        "patchset-ci-start-plan",
        json!({"patchset_id": "RP-1", "contract_available": false}),
    );
    assert_eq!(unavailable["state"], json!("unavailable"));

    let active = run(
        "patchset-ci-start-plan",
        json!({
            "patchset_id": "RP-1",
            "contract_available": true,
            "active_state": {"queued": true}
        }),
    );
    assert_eq!(active["state"], json!("reuse_active"));
    assert_eq!(active["result"]["queued"], json!(true));

    let enqueue = run(
        "patchset-ci-start-plan",
        json!({
            "patchset_id": "RP-1",
            "trigger": "patchset_select",
            "contract_available": true,
            "queue_mode": "async"
        }),
    );
    assert_eq!(enqueue["state"], json!("enqueue"));
    assert_eq!(enqueue["delivery"], json!("async_queue"));
    assert_eq!(enqueue["enqueue"]["job_type"], json!("patchset.ci"));
    assert_eq!(enqueue["mark_pending"]["job_state"], json!("queued"));
}
