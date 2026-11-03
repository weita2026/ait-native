// Current HTTP fixture for the Snapshot/remote primitive integration test.
// Legacy Change fixtures stay separate. This fixture has no Change and records
// reviews, attestation and acceptance only after their explicit test requests.
#[derive(Default)]
struct DurableTaskFixture {
    capture: JsonValue,
    patchset: Option<JsonValue>,
    reviews: Vec<JsonValue>,
    attestation: Option<JsonValue>,
    policy: Option<JsonValue>,
    accepted: Option<JsonValue>,
}

fn durable_task_response(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<FakeRemoteState>>,
) -> Option<Response<std::io::Cursor<Vec<u8>>>> {
    const PREFIX: &str = "/v1/native/repository-authorities/7/";
    const PATCHSET: &str = "RT-1/P-02";
    let suffix = if url == "/v1/native/repository-authorities" {
        "register"
    } else if url == PREFIX.trim_end_matches('/') {
        ""
    } else {
        url.strip_prefix(PREFIX)?
    };
    let mut guard = state.lock().unwrap();
    let head = guard.remote_head_snapshot_id.clone();
    let fixture = guard.durable_task.as_mut()?;
    let payload = match (method, suffix) {
        ("GET", "") | ("POST", "register") => {
            let mut payload = repository_payload("fixture-ait");
            payload["repository"]["snapshot_capture_binding_contract"] =
                json!(ait_core::repository_pack_json::SNAPSHOT_CAPTURE_BINDING_CONTRACT);
            payload["actor_key_index"] = json!(7);
            payload
        }
        ("GET", "tasks/RT-1") => json!({
            "task_id":"RT-1", "status":if fixture.accepted.is_some() {"completed"} else {"active"},
            "base_line":"main", "successful_land":fixture.accepted.as_ref().map(|value| &value["land"]),
            "current_patchset_id":fixture.patchset.as_ref().map(|value| &value["patchset_id"])
        }),
        ("GET", path) if path.starts_with("snapshots/") => {
            let id = path
                .trim_start_matches("snapshots/")
                .split('?')
                .next()
                .unwrap();
            if fixture.capture["snapshot_id"].as_str() != Some(id) {
                return Some(json_response(404, &json!({"detail":"Snapshot is absent"})));
            }
            fixture.capture.clone()
        }
        ("POST", "snapshots:exists") => {
            let request = parse_json(body);
            let mut present = Vec::new();
            let mut missing = Vec::new();
            for id in request["snapshot_ids"].as_array().unwrap() {
                if id.as_str() == head.as_deref() || *id == fixture.capture["snapshot_id"] {
                    present.push(id.clone());
                } else {
                    missing.push(id.clone());
                }
            }
            json!({"present":present,"missing":missing})
        }
        ("POST", "changes/RT-1/patchsets") => {
            let request = parse_json(body);
            assert_eq!(
                request["revision_snapshot_id"],
                fixture.capture["snapshot_id"]
            );
            let patchset = json!({
                "task_id":"RT-1","change_id":null,"change_ref":null,"patchset_id":PATCHSET,
                "patchset_number":2,"base_snapshot_id":request["base_snapshot_id"],
                "revision_snapshot_id":request["revision_snapshot_id"],"summary":request["summary"],
                "author_mode":request["author_mode"],"publish_state":"published","evaluation_state":"pending",
                "ci_run_seq":1,"ci_completed_run_seq":1,
                "ci":{"run_seq":1,"suite_result_count":1,"blocking_failure_count":0,"tests_status":"pass","lint_status":"pass"}
            });
            fixture.patchset = Some(patchset.clone());
            patchset
        }
        ("GET", "changes/RT-1/patchsets") => json!(fixture.patchset.iter().collect::<Vec<_>>()),
        ("GET", "patchsets/RT-1%2FP-02") => fixture.patchset.clone().expect("published candidate"),
        ("GET", path) if path.starts_with("read/patchsets/RT-1%2FP-02/ci-status?") => {
            let recent_limit = path.split('?').nth(1).unwrap_or("").split('&')
                .find_map(|item| item.strip_prefix("recent_limit=").and_then(|value| value.parse::<i64>().ok()))
                .unwrap_or(10).clamp(1, 20);
            json!({"contract":"ait.server.patchset_ci.readiness.v2","projection":"readiness","repo_name":"fixture-ait",
                "task_id":"RT-1","change_id":null,"change_ref":null,"patchset_id":PATCHSET,
                "available":true,"tests_status":"pass","has_runnable_evidence":true,"selected_suite_ids":["preflight"],
                "ci_run_seq":1,"ci_completed_at_s":1_783_814_400_u64,"suite_result_count":1,"blocking_failure_count":0,
                "recent_limit_applied":recent_limit,"recent_jobs":[],
                "latest_job":{"job_id":"JOB-1","job_type":"patchset.ci","state":"succeeded"}})
        }
        ("POST", "patchsets/RT-1%2FP-02:runCi") => {
            let request = parse_json(body);
            json!({"patchset_id":PATCHSET,"queued":true,"trigger":request["trigger"]})
        }
        ("POST", "changes/RT-1/reviews") => {
            let mut review = parse_json(body);
            assert_eq!(review["patchset_id"], PATCHSET);
            review["task_id"] = json!("RT-1");
            review["change_id"] = JsonValue::Null;
            review["change_ref"] = JsonValue::Null;
            fixture.reviews.push(review.clone());
            review
        }
        ("GET", "changes/RT-1/reviews") => {
            let count = |action: &str| {
                fixture
                    .reviews
                    .iter()
                    .filter(|row| row["action"] == action)
                    .count()
            };
            let humans = |action: &str| {
                fixture
                    .reviews
                    .iter()
                    .filter(|row| row["action"] == action && row["reviewer_kind"] == "user")
                    .count()
            };
            json!({"task_id":"RT-1","change_id":null,"change_ref":null,"current_patchset_id":PATCHSET,
                "approvals":count("approve"),"team_approvals":count("approve"),"task_approvals":count("task_approve"),
                "human_approvals":humans("approve"),"human_task_approvals":humans("task_approve"),
                "blocking":0,"code_review_summaries":count("code_review_summary"),"reviews":fixture.reviews})
        }
        ("PUT", "patchsets/RT-1%2FP-02/attestation") => {
            let mut attestation = parse_json(body);
            attestation["attestation_id"] = json!("AT-1");
            attestation["task_id"] = json!("RT-1");
            attestation["change_id"] = JsonValue::Null;
            attestation["change_ref"] = JsonValue::Null;
            attestation["patchset_id"] = json!(PATCHSET);
            attestation["ci_backed"] = json!(true);
            fixture.attestation = Some(attestation.clone());
            attestation
        }
        ("GET", "patchsets/RT-1%2FP-02/attestation") => {
            let Some(attestation) = fixture.attestation.clone() else {
                return Some(json_response(
                    404,
                    &json!({"detail":"Attestation is absent"}),
                ));
            };
            attestation
        }
        ("POST", "patchsets/RT-1%2FP-02:evaluatePolicy") => {
            let decision = if fixture.attestation.is_some() {
                "pass"
            } else {
                "pending"
            };
            let policy = json!({"task_id":"RT-1","patchset_id":PATCHSET,"decision":decision,
                "checks":[{"name":"tests","status":"pass"}],"evaluated_at":"2026-10-25T00:00:00Z"});
            fixture.policy = Some(policy.clone());
            policy
        }
        ("GET", "patchsets/RT-1%2FP-02/policy") => {
            match fixture.policy.clone() {
                Some(policy) => policy,
                None => return Some(json_response(404, &json!({"detail":"Policy is absent"}))),
            }
        }
        ("POST", "task-land") => {
            let request = parse_json(body);
            assert_eq!(request["task_or_change_ref"], "RT-1");
            assert!(fixture
                .reviews
                .iter()
                .any(|row| row["action"] == "task_approve"));
            assert!(fixture
                .reviews
                .iter()
                .any(|row| row["action"] == "code_review_summary"));
            assert!(fixture.policy.is_some());
            let patchset = fixture.patchset.clone().unwrap();
            let land = json!({"repository_index":7,"task_id":"RT-1","submission_id":"RT-1/L-01",
                "change_id":null,"change_ref":null,"patchset_id":PATCHSET,"status":"succeeded",
                "base_snapshot_id":patchset["base_snapshot_id"],"revision_snapshot_id":patchset["revision_snapshot_id"],
                "target_line":"main","target_line_index":0,"pre_land_target_snapshot_id":head,
                "landed_snapshot_id":patchset["revision_snapshot_id"],"mode":request["mode"],
                "created_at":"2026-10-25T00:00:00Z","updated_at":"2026-10-25T00:00:01Z"});
            let response = json!({"contract":request["contract"],"repository_index":7,"repo_name":"fixture-ait",
                "idempotency_key":request["idempotency_key"],"expected_identity":request["expected_identity"],"replayed":false,
                "status":"succeeded","task_id":"RT-1","task_status":"completed","change":null,"change_id":null,"change_ref":null,
                "patchset_id":PATCHSET,"patchset":patchset,"target_line":"main","landed_snapshot_id":land["landed_snapshot_id"],
                "task":{"task_id":"RT-1","status":"completed","current_patchset_id":PATCHSET,"successful_land":land},"land":land});
            fixture.accepted = Some(response.clone());
            guard.remote_head_snapshot_id =
                response["landed_snapshot_id"].as_str().map(str::to_string);
            response
        }
        _ => return None,
    };
    Some(json_response(200, &payload))
}
