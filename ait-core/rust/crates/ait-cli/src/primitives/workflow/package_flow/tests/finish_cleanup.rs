use super::*;

#[derive(Clone, Copy)]
enum Reply {
    Confirmed,
    Pending,
    UnknownOwner,
    Lost,
    Retired,
    WrongIdentity,
    MissingMapping,
}

#[test]
fn committed_cleanup_replay_skips_review_ci_and_preserves_idempotent_closeout() {
    finish_script(true, &[Reply::Confirmed, Reply::Confirmed, Reply::Retired]);
}

#[test]
fn initial_postcommit_pending_only_retries_cleanup_on_the_next_explicit_invocation() {
    finish_script(false, &[Reply::Pending, Reply::Confirmed]);
}

#[test]
fn committed_pending_unknown_lost_and_retired_results_never_infer_full_mappings() {
    for reply in [
        Reply::Pending,
        Reply::UnknownOwner,
        Reply::Lost,
        Reply::Retired,
    ] {
        finish_script(true, &[reply]);
    }
}

#[test]
fn committed_replay_preserves_exact_identity_and_entire_batch_validation() {
    for reply in [Reply::WrongIdentity, Reply::MissingMapping] {
        finish_script(true, &[reply]);
    }
}

fn finish_script(initially_committed: bool, replies: &[Reply]) {
    let mut fixture = fixture();
    let identity = pinned(&fixture);
    let original = TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    configure_remote(
        &mut fixture.repo,
        &format!("http://{}", server.server_addr()),
    );
    let expected = identity.clone();
    let local = fixture.id.clone();
    let script = replies.to_vec();
    let handle = std::thread::spawn(move || {
        let mut committed = initially_committed;
        let mut publications = usize::from(initially_committed);
        let mut finishes = 0;
        let mut reviews = 0;
        let mut original_request = None;
        loop {
            let mut request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            let method = request.method().as_str().to_string();
            let path = request.url().to_string();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let body: JsonValue = if body.is_empty() {
                JsonValue::Null
            } else {
                serde_json::from_str(&body).unwrap()
            };
            let (status, response) = if path == "/healthz" {
                assert_eq!(method, "GET");
                (
                    200,
                    json!({"ready":true,"phase":null,"processed":null,"total":null,"last_progress_at":null}),
                )
            } else if path == "/v1/native/repository-authorities/0" {
                assert_eq!(method, "GET");
                (
                    200,
                    json!({"repository_index":0,"repository_name":"fixture","namespace":"","policy_flags":0,"tombstoned":false}),
                )
            } else if path == "/v1/native/repository-authorities" {
                assert_eq!(method, "POST");
                (200, json!({"repository_index":0,"actor_key_index":0}))
            } else if path.ends_with("/worker-jobs/0") && method == "GET" {
                (
                    200,
                    json!({"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":if committed {json!(9)} else {JsonValue::Null}},"package_execution":if committed {JsonValue::Null} else {json!({"expected_identity":expected,"ci_state":"passed","availability":"available","recoverable":true})}}),
                )
            } else if path.contains("/snapshots/") {
                assert_eq!(method, "GET");
                // The formal GET cannot prove complete sealed membership.
                (
                    404,
                    json!({"detail":"Original full source proof is unavailable through GET"}),
                )
            } else if path.ends_with("/worker-jobs/0") && method == "POST" {
                assert!(
                    !committed,
                    "committed retries cannot mutate review/approval"
                );
                assert_eq!(body["operation"], "review");
                reviews += 1;
                (
                    200,
                    json!({"expected_identity":expected,"operation":"review","accepted":true,"recorded_at_s":123}),
                )
            } else if path.ends_with("/task-land") {
                assert_eq!(method, "POST");
                assert_eq!(body["contract"], "task-land-package-guarded/v1");
                assert_eq!(body["expected_identity"], json!(expected));
                assert_eq!(body.as_object().unwrap().len(), 6);
                if let Some(original) = &original_request {
                    assert_eq!(&body, original, "cleanup replay retains the exact request");
                } else {
                    original_request = Some(body);
                }
                if !committed {
                    publications += 1;
                    committed = true;
                }
                let reply = script[finishes];
                finishes += 1;
                let mut result = json!({"outcome":"confirmed","expected_identity":expected,"patchset_id":"RT-0001/P-01","submission_id":"RT-0001/L-01","entries":[{"local_task_id":local,"remote_task_id":"RT-0001","changes":[]}]});
                match reply {
                    Reply::Pending => (
                        503,
                        json!({"detail":"PACKAGE_FINISH_CLEANUP_PENDING: publication committed; source reader is held"}),
                    ),
                    Reply::UnknownOwner => (
                        409,
                        json!({"detail":"PACKAGE_FINISH_CLEANUP_UNKNOWN: publication committed; retained source ownership is unknown"}),
                    ),
                    Reply::Retired => (
                        200,
                        json!({"outcome":"unknown","expected_identity":expected,"patchset_id":null,"submission_id":null,"entries":[]}),
                    ),
                    Reply::WrongIdentity => {
                        result["expected_identity"]["worker_job_index"] = json!(1);
                        (200, result)
                    }
                    Reply::MissingMapping => {
                        result["entries"] = json!([]);
                        (200, result)
                    }
                    Reply::Confirmed => (200, result),
                    Reply::Lost => {
                        request
                            .respond(tiny_http::Response::from_string("{"))
                            .unwrap();
                        if finishes == script.len() {
                            break;
                        }
                        continue;
                    }
                }
            } else {
                panic!("Unexpected cleanup request {method} {path}");
            };
            request
                .respond(
                    tiny_http::Response::from_string(response.to_string()).with_status_code(status),
                )
                .unwrap();
            if finishes == script.len() {
                break;
            }
        }
        assert_eq!(publications, 1);
        assert_eq!(reviews, if initially_committed { 0 } else { 2 });
        assert_eq!(finishes, script.len());
    });
    let message = "1. Reviewed files\nPackage cleanup.\n2. Findings\nExact recovery.\n3. Risks\nServer pairing.\n4. Tests\nNative fixture.\n5. Recommendation\nPass.";
    let mut previous = original.clone();
    for (index, reply) in replies.iter().enumerate() {
        let fresh = !initially_committed && index == 0;
        let result = workflow_package_finish(
            &fixture.repo,
            &fixture.id,
            Some("origin"),
            &identity,
            true,
            fresh.then_some("Einstein"),
            fresh.then_some(message),
        )
        .unwrap();
        assert_eq!(result["expected_identity"], json!(identity));
        let after = TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap();
        let code = match reply {
            Reply::Confirmed => "done",
            Reply::Pending => "finish_incomplete",
            _ => "completion_unknown",
        };
        assert_eq!(result["next_action"]["code"], code, "{result}");
        if matches!(reply, Reply::Confirmed) {
            assert_eq!(result["closeout_status"], "complete");
            assert_eq!(after["published_task_id"], "RT-0001");
            if !previous["published_task_id"].is_null() {
                assert_eq!(after, previous);
            }
        } else {
            assert_ne!(result["apply_status"], "done");
            assert!(result["closeout_status"].is_null());
            assert_eq!(after, previous, "unconfirmed cleanup cannot write mappings");
            if matches!(reply, Reply::Pending | Reply::UnknownOwner) {
                let summary = result["next_action"]["summary"].as_str().unwrap();
                assert!(summary.contains("Server reported unresolved cleanup"));
                assert!(summary.contains(
                    "Formal acceptance and complete mappings remain independently unverified"
                ));
                assert!(summary.contains("same finish"));
            }
        }
        assert_eq!(after["local_finish"], original["local_finish"]);
        previous = after;
    }
    handle.join().unwrap();
}
