// Current Task-owned closeout fixture. Legacy Change routes remain separate.
// Successful receipts are recorded only when the simulated atomic mutation runs.
fn current_closeout_runner_ci(state: &CloseoutRecoveryRemoteState) -> Option<JsonValue> {
    let (marker, passes) = state.runner_ci_result.as_ref()?;
    let completed = marker.is_file();
    let status = if !completed {
        "pending"
    } else if *passes {
        "pass"
    } else {
        "fail"
    };
    let job = json!({
        "repository_index":7,"patchset_index":1,"worker_job_index":11,
        "job_type":"patchset.ci","state":if completed {"succeeded"} else {"queued"},
        "patchset_ci":{"ci_run_seq":1,"ci_worker_job_index":11}
    });
    Some(json!({
        "contract":"ait.server.patchset_ci.readiness.v2","projection":"readiness",
        "repo_name":"fixture-ait","recent_limit_applied":10,"available":true,
        "repository_index":7,"patchset_index":1,"task_id":"RT-1",
        "patchset_id":if state.revision_change {"RT-1/C-01/P-01"} else {"RT-1/P-01"},
        "change_id":state.revision_change.then_some("C-01"),
        "change_ref":state.revision_change.then_some("RT-1/C-01"),
        "ci_run_seq":1,"ci_completed_at_s":if completed {1} else {0},
        "overall_status":status,"tests_status":status,"lint_status":"none",
        "selected_suite_ids":["preflight"],"selected_suite_count":1,
        "suite_result_count":usize::from(completed),
        "blocking_failure_count":usize::from(completed && !passes),
        "has_runnable_evidence":true,"latest_job":job,"recent_jobs":[job]
    }))
}

fn current_closeout_patchset(state: &CloseoutRecoveryRemoteState) -> JsonValue {
    let mut patchset = json!({
        "task_id":"RT-1","change_id":state.revision_change.then_some("C-01"),
        "change_ref":state.revision_change.then_some("RT-1/C-01"),
        "patchset_id":if state.revision_change {"RT-1/C-01/P-01"} else {"RT-1/P-01"},
        "base_snapshot_id":FIXTURE_BASE_SNAPSHOT_ID,
        "revision_snapshot_id":state.patchset_revision_snapshot_id.as_deref().unwrap_or(FIXTURE_BASE_SNAPSHOT_ID),
        "author_mode":if state.ai_only_candidate {"ai_only_experimental"} else {"ai_with_human_review"},
        "publish_state":"published","evaluation_state":"pass",
        "ci_run_seq":1,"ci_completed_at_s":1_783_814_400_u64,
        "ci":{"run_seq":1,"completed_at_s":1_783_814_400_u64,"overall_status":"pass",
            "tests_status":"pass","lint_status":"none","selected_suite_count":1,"suite_result_count":1,"blocking_failure_count":0}
    });
    if state.ai_only_candidate {
        patchset["diff_stats"] = json!({"paths":{"modified":["src/lib.rs"]}});
    }
    if let Some(ci) = current_closeout_runner_ci(state) {
        patchset["ci_completed_at_s"] = ci["ci_completed_at_s"].clone();
        patchset["ci"] = ci;
    }
    patchset
}

fn current_closeout_task(state: &CloseoutRecoveryRemoteState) -> JsonValue {
    let mut task = if let Some(plan) = &state.plan_closeout {
        plan.bound_task(state.task_completed)
    } else {
        json!({"task_id":"RT-1","repo_name":"fixture-ait","title":"Closeout recovery task",
            "intent":"prove authoritative recovery","status":if state.task_completed {"completed"} else {"active"},
            "publication_state":"published","published_task_id":"RT-1"})
    };
    task["current_patchset_id"] = current_closeout_patchset(state)["patchset_id"].clone();
    task["successful_land"] = state
        .accepted
        .as_ref()
        .map(|receipt| receipt["land"].clone())
        .unwrap_or(JsonValue::Null);
    task
}

fn current_closeout_acceptance(
    request: &JsonValue,
    state: &CloseoutRecoveryRemoteState,
) -> JsonValue {
    let patchset = current_closeout_patchset(state);
    let mode = request["mode"].as_str().unwrap_or("direct");
    let target = request["target_line"].as_str().unwrap_or("main");
    let land = json!({"repository_index":7,"task_id":"RT-1","submission_id":"RT-1/L-01",
        "change_id":patchset["change_id"],"change_ref":patchset["change_ref"],"patchset_id":patchset["patchset_id"],"status":"succeeded",
        "base_snapshot_id":patchset["base_snapshot_id"],"revision_snapshot_id":patchset["revision_snapshot_id"],
        "target_line":target,"target_line_index":0,"pre_land_target_snapshot_id":FIXTURE_BASE_SNAPSHOT_ID,
        "landed_snapshot_id":patchset["revision_snapshot_id"],"mode":mode,
        "created_at":"2026-10-26T00:00:00Z","updated_at":"2026-10-26T00:00:01Z"});
    let mut task = current_closeout_task(state);
    task["status"] = json!("completed");
    task["successful_land"] = land.clone();
    json!({"contract":request["contract"],"repository_index":7,"repo_name":"fixture-ait",
        "idempotency_key":request["idempotency_key"],"expected_identity":request["expected_identity"],"replayed":false,
        "status":"succeeded","task_id":"RT-1","task_status":"completed","change_status":state.revision_change.then_some("landed"),"change":state.revision_change.then(|| json!({"task_id":"RT-1","change_id":"C-01","change_ref":"RT-1/C-01","status":"landed"})),"change_id":patchset["change_id"],"change_ref":patchset["change_ref"],
        "patchset_id":patchset["patchset_id"],"patchset":patchset,"target_line":target,"landed_snapshot_id":land["landed_snapshot_id"],
        "task":task,"land":land})
}

fn current_closeout_response(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<CloseoutRecoveryRemoteState>>,
) -> Option<Response<std::io::Cursor<Vec<u8>>>> {
    const PREFIX: &str = "/v1/native/repository-authorities/7/";
    let suffix = if url == "/v1/native/repository-authorities" {
        "register"
    } else {
        url.strip_prefix(PREFIX)?
    };
    let mut state = state.lock().unwrap();
    let patchset = current_closeout_patchset(&state);
    let patchset_id = patchset["patchset_id"].as_str().unwrap();
    let change = json!({"task_id":"RT-1","change_id":"C-01","change_ref":"RT-1/C-01", "base_line":"main",
        "fork_snapshot_id":FIXTURE_BASE_SNAPSHOT_ID,"status":if state.task_completed {"landed"} else {"approved"}});
    if (suffix.starts_with("patchsets/") || suffix.starts_with("read/patchsets/"))
        && suffix.contains("%2FC-01") != state.revision_change
    {
        return Some(json_response(404, &json!({"detail":"Patchset is absent"})));
    }
    if suffix == "changes/RT-1%2FC-01/reviews" && !state.revision_change {
        return Some(json_response(404, &json!({"detail":"Change is absent"})));
    }
    let payload = match (method, suffix) {
        ("GET", path) if path.starts_with("worker-jobs?") && state.runner_ci_result.is_some() => {
            json!({"jobs":[current_closeout_runner_ci(&state).unwrap()["latest_job"]]})
        }
        ("POST", "register") => {
            let mut authority = repository_payload("fixture-ait");
            authority["actor_key_index"] = json!(7);
            authority
        }
        ("GET", "tasks/RT-1") => current_closeout_task(&state),
        ("GET", "changes") => {
            if state.revision_change {
                json!([change])
            } else {
                json!([])
            }
        }
        ("GET", "changes/RT-1%2FC-01") if state.revision_change => change,
        ("GET", "patchsets/RT-1%2FP-01") | ("GET", "patchsets/RT-1%2FC-01%2FP-01") => {
            patchset.clone()
        }
        ("GET", "changes/RT-1/reviews") => {
            let mut reviews = state.recorded_reviews.clone();
            if state.task_review_recorded
                && !reviews
                    .iter()
                    .any(|review| review["action"] == "task_approve")
            {
                let mut review = json!({"reviewer":"Fixture User","reviewer_kind":state.existing_task_review_kind.as_deref().unwrap_or("user"),
                    "patchset_id":patchset_id,"action":"task_approve","blocking":false,"comment":"functional checks passed"});
                if state.omit_task_review_kind {
                    review.as_object_mut().unwrap().remove("reviewer_kind");
                }
                reviews.push(review);
            }
            if state.code_review_recorded
                && !reviews
                    .iter()
                    .any(|review| review["action"] == "code_review_summary")
            {
                reviews.push(json!({"review_id":format!("{patchset_id}/R-01"),"reviewer":"ait-cli","reviewer_kind":"bot","patchset_id":patchset_id,
                    "action":"code_review_summary","blocking":false,"comment":ACTING_REVIEW_SUMMARY}));
            }
            let approvals = reviews
                .iter()
                .filter(|review| review["action"] == "task_approve")
                .count();
            let humans = reviews
                .iter()
                .filter(|review| {
                    review["action"] == "task_approve" && review["reviewer_kind"] == "user"
                })
                .count();
            let reviewers = reviews
                .iter()
                .filter(|review| review["action"] == "code_review_summary")
                .map(|review| review["reviewer"].clone())
                .collect::<Vec<_>>();
            json!({"task_id":"RT-1","change_id":patchset["change_id"],"change_ref":patchset["change_ref"],"current_patchset_id":patchset_id,
                "approvals":approvals,"task_approvals":approvals,"team_approvals":0,"blocking":0,
                "human_approvals":humans,"human_task_approvals":humans,"independent_human_approvals":humans,
                "independent_task_approvals":approvals,"code_review_summaries":reviewers.len(),"code_review_summary_reviewers":reviewers,
                "review_requests":[],"reviews":reviews})
        }
        ("POST", "changes/RT-1/reviews") | ("POST", "changes/RT-1%2FC-01/reviews") => {
            if state.task_completed {
                return Some(json_response(400, &json!({"detail":"Task or explicit Change is terminal"})));
            }
            let mut review = parse_json(body);
            review["review_id"] = json!(format!("{patchset_id}/R-{:02}", state.recorded_reviews.len() + 2));
            assert_eq!(review["patchset_id"], patchset_id);
            review["task_id"] = json!("RT-1");
            review["change_id"] = patchset["change_id"].clone();
            review["change_ref"] = patchset["change_ref"].clone();
            if review.get("reviewer_kind").is_none() {
                review["reviewer_kind"] = json!("user");
            }
            if review["action"] == "code_review_summary" {
                state.code_review_recorded = true;
            }
            if review["action"] == "task_approve" {
                state.task_review_recorded = true;
            }
            state.recorded_reviews.push(review.clone());
            review
        }
        ("GET", "patchsets/RT-1%2FP-01/attestation")
        | ("GET", "patchsets/RT-1%2FC-01%2FP-01/attestation") => {
            json!({"attestation_id":"AT-1","task_id":"RT-1",
            "change_id":patchset["change_id"],"change_ref":patchset["change_ref"],"patchset_id":patchset_id,"author_mode":patchset["author_mode"],
            "verification_state":"pass","require_tests_pass":true,"require_human_review":false,
            "require_lint_pass":false,"revoked":false,"ci_backed":true})
        }
        ("POST", "patchsets/RT-1%2FP-01:evaluatePolicy")
        | ("POST", "patchsets/RT-1%2FC-01%2FP-01:evaluatePolicy")
        | ("GET", "patchsets/RT-1%2FP-01/policy")
        | ("GET", "patchsets/RT-1%2FC-01%2FP-01/policy") => {
            if method == "POST" {
                state.policy_evaluated = true;
            }
            let mut policy = json!({"policy_id":"PO-1","task_id":"RT-1","patchset_id":patchset_id,
                "decision":if state.policy_evaluated {"pass"} else {"pending"},"content_class":"code","author_class":"hybrid",
                "effective_requirements":{"require_tests":true,"require_code_review_summary":true},"evaluated_at":"2026-10-26T00:00:00Z",
                "checks":[{"name":"require_tests","status":"pass","message":"ok"}],"input_fingerprint":"abc"});
            if state.ai_only_candidate {
                policy.as_object_mut().unwrap().remove("effective_requirements");
            }
            policy
        }
        ("GET", path)
            if (path.starts_with("read/patchsets/RT-1%2FP-01/ci-status")
                || path.starts_with("read/patchsets/RT-1%2FC-01%2FP-01/ci-status")) =>
        {
            if let Some(ci) = current_closeout_runner_ci(&state) {
                return Some(json_response(200, &ci));
            }
            json!({
            "contract":"ait.server.patchset_ci.readiness.v2","task_id":"RT-1","patchset_id":patchset_id,
            "change_id":patchset["change_id"],"change_ref":patchset["change_ref"],"tests_status":"pass","has_runnable_evidence":true,"selected_suite_ids":["preflight"],
            "latest_job":{"job_id":"JOB-1","job_type":"patchset.ci","state":"succeeded"}})
        }
        _ => return None,
    };
    Some(json_response(200, &payload))
}
