use super::actor_plan::call;
use super::actor_registration::{register, request_for};
use super::*;
use ait_server_core::foundation::workflow_binary_v0::WorkflowBinaryV0Codec;

fn affected_bytes(state: &ServerState) -> BTreeMap<String, Option<Vec<u8>>> {
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    let task = db
        .resolve_record_path(&WorkflowBinaryV0Codec::task_file())
        .unwrap();
    let root = task.parent().unwrap();
    [
        "task.bin",
        "task_payload.bin",
        "change.bin",
        "change_payload.bin",
        "snapshot.bin",
        "snapshot_payload.bin",
        "land.bin",
        "snapshot_link.bin",
        "actor.bin",
        "actor_payload.bin",
        "patchset.bin",
        "patchset_summary_payload.bin",
        "plan.bin",
        "plan_revision.bin",
    ]
    .into_iter()
    .map(|name| {
        let bytes = match fs::read(root.join(name)) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("read {name}: {error}"),
        };
        (name.to_string(), bytes)
    })
    .collect()
}

#[tokio::test]
async fn actor_workflow_rejects_missing_and_invalid_actor_before_domain_effects() {
    let root = TestDirectory::new("actor-workflow-invalid");
    let state = fresh_state(root.path());
    let app = build_router_with_state(state.clone());
    assert_eq!(
        register(&app, request_for(&state, 1)).await.1["actor_key_index"],
        0
    );
    let base = "/v1/native/repository-authorities/1";
    let (status, task) = call(
        &app,
        "POST",
        &format!("{base}/tasks"),
        Some(json!({
            "actor_key_index":0,"title":"Actor task","intent":"Validate mutation admission"
        })),
    )
    .await;
    assert!(status.is_success(), "{task}");
    let task_id = task["task_id"].as_str().unwrap();
    let (status, change) = call(
        &app,
        "POST",
        &format!("{base}/changes"),
        Some(json!({
            "actor_key_index":0,"task_id":task_id,"title":"Actor change","base_line":"main"
        })),
    )
    .await;
    assert!(status.is_success(), "{change}");
    let change_path = format!(
        "{base}/changes/{}",
        change["change_ref"].as_str().unwrap().replace('/', "%2F")
    );
    let requests = [
        (
            format!("{base}/tasks"),
            json!({"title":"No effect","intent":"Invalid actor"}),
        ),
        (
            format!("{base}/tasks/{task_id}:close"),
            json!({"status":"abandoned"}),
        ),
        (
            format!("{base}/changes"),
            json!({"task_id":task_id,"title":"No effect","base_line":"main"}),
        ),
        (
            format!("{change_path}:close"),
            json!({"status":"abandoned"}),
        ),
        (
            format!("{change_path}:selectPatchset"),
            json!({"patchset_id":"not-selected"}),
        ),
        (
            format!("{change_path}/patchsets"),
            json!({"base_snapshot_id":"not-present","revision_snapshot_id":"not-present","summary":"No effect","author_mode":"ai_with_human_review"}),
        ),
        (
            format!("{change_path}:requestReview"),
            json!({"patchset_id":"not-selected","reviewer_groups":["Team"]}),
        ),
        (
            format!("{change_path}:submit"),
            json!({"target_line":"main","mode":"direct"}),
        ),
        (
            format!("{base}/task-land"),
            json!({"contract":"task-land-atomic/v2","task_or_change_ref":task_id,"idempotency_key":"invalid","target_line":"main","mode":"direct"}),
        ),
        (
            format!("{base}/history-promotion:prepare"),
            json!({"contract":"history-promotion-prepare/v3"}),
        ),
        (
            format!("{base}/remote-sync/zstd-bulk/commit"),
            json!({"contract":"ait.remote_sync.zstd_bulk.commit.v1","snapshots":[],"object_packs":[],"tree_packs":[],"blob_locators":[],"tree_locators":[]}),
        ),
        (
            format!("{base}/task-start"),
            json!({"contract":"task-start-atomic/v2","idempotency_key":"invalid","plan_item_ref":"actor/item","plan":{"action":"existing","plan_id":"PR-0","plan_revision_id":"plan-revision:0"},"task":{"title":"No effect","intent":"Invalid actor"},"change":null}),
        ),
    ];
    let before = affected_bytes(&state);
    for actor in [
        None,
        Some(JsonValue::Null),
        Some(json!(-1)),
        Some(json!("0")),
        Some(json!(1)),
        Some(json!(u32::MAX)),
    ] {
        for (path, payload) in &requests {
            let mut payload = payload.clone();
            if let Some(actor) = &actor {
                payload["actor_key_index"] = actor.clone();
            }
            let (status, response) = call(&app, "POST", path, Some(payload)).await;
            assert!(status.is_client_error(), "{path}: {status} {response}");
            assert!(
                response.to_string().contains("actor_key_index"),
                "{path}: {response}"
            );
            assert_eq!(
                affected_bytes(&state),
                before,
                "invalid Actor effects at {path}"
            );
        }
    }
}

#[tokio::test]
async fn actor_workflow_task_and_change_mutators_survive_noop_and_reopen() {
    let root = TestDirectory::new("actor-workflow-readback");
    let state = fresh_state(root.path());
    let app = build_router_with_state(state.clone());
    let alice = request_for(&state, 1);
    assert_eq!(register(&app, alice.clone()).await.1["actor_key_index"], 0);
    let mut bob = alice;
    bob["user_name"] = json!("Bob");
    bob["user_email"] = json!("bob@example.test");
    assert_eq!(register(&app, bob).await.1["actor_key_index"], 1);
    let base = "/v1/native/repository-authorities/1";
    let (status, task) = call(
        &app,
        "POST",
        &format!("{base}/tasks"),
        Some(json!({
            "actor_key_index":0,"title":"Attributed task","intent":"Persist the actual mutator"
        })),
    )
    .await;
    assert!(status.is_success(), "{task}");
    assert_eq!(task["actor_key_index"], 0);
    let task_id = task["task_id"].as_str().unwrap().to_string();
    let (status, change) = call(
        &app,
        "POST",
        &format!("{base}/changes"),
        Some(json!({
            "actor_key_index":0,"task_id":task_id,"title":"Attributed change","base_line":"main"
        })),
    )
    .await;
    assert!(status.is_success(), "{change}");
    assert_eq!(change["actor_key_index"], 0);
    let change_path = format!(
        "{base}/changes/{}",
        change["change_ref"].as_str().unwrap().replace('/', "%2F")
    );
    let (status, closed) = call(
        &app,
        "POST",
        &format!("{change_path}:close"),
        Some(json!({
            "actor_key_index":1,"status":"abandoned"
        })),
    )
    .await;
    assert!(status.is_success(), "{closed}");
    assert_eq!(closed["actor_key_index"], 1);
    let before = affected_bytes(&state);
    let (status, replayed) = call(
        &app,
        "POST",
        &format!("{change_path}:close"),
        Some(json!({
            "actor_key_index":0,"status":"abandoned"
        })),
    )
    .await;
    assert!(status.is_success(), "{replayed}");
    assert_eq!(replayed["actor_key_index"], 1);
    assert_eq!(affected_bytes(&state), before);
    let task_path = format!("{base}/tasks/{task_id}");
    let (status, closed) = call(
        &app,
        "POST",
        &format!("{task_path}:close"),
        Some(json!({
            "actor_key_index":1,"status":"abandoned"
        })),
    )
    .await;
    assert!(status.is_success(), "{closed}");
    assert_eq!(closed["actor_key_index"], 1);
    let before = affected_bytes(&state);
    let (status, replayed) = call(
        &app,
        "POST",
        &format!("{task_path}:close"),
        Some(json!({
            "actor_key_index":0,"status":"abandoned"
        })),
    )
    .await;
    assert!(status.is_success(), "{replayed}");
    assert_eq!(replayed["actor_key_index"], 1);
    assert_eq!(affected_bytes(&state), before);
    drop(app);
    drop(state);
    let reopened = OperationalBinaryRuntime::open_generation(
        root.path().join("generation"),
        root.path().join("runtime-worker-leases.bin"),
        60,
        15,
    )
    .unwrap();
    let app = build_router_with_state(state_from_operational(Arc::new(reopened)));
    for path in [task_path, change_path] {
        let (status, value) = call(&app, "GET", &path, None).await;
        assert!(status.is_success(), "{value}");
        assert_eq!(value["actor_key_index"], 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run explicitly with the frozen companion in AIT_CAPTURE_TEST_CLI"]
async fn actual_actor_workflow_cli_first_finish_replay_and_reopen() {
    actor_workflow_cli_pair(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run explicitly with the frozen companion in AIT_CAPTURE_TEST_CLI"]
async fn actual_actor_workflow_cli_capture_guarded_finish_replay_and_reopen() {
    actor_workflow_cli_pair(true).await;
}

async fn actor_workflow_cli_pair(guarded_api_first: bool) {
    use super::snapshot_capture::CaptureCli;
    use crate::binary_runtime::RoutedBinaryWorkflowStore;
    use std::sync::Mutex;
    let directory = TestDirectory::new("actor-finish-paired");
    let root = directory.path().canonicalize().unwrap();
    let state = fresh_state(&root);
    let landed_request = Arc::new(Mutex::new(None::<JsonValue>));
    let recorded = landed_request.clone();
    let router = build_router_with_state(state.clone()).layer(axum::middleware::from_fn(
        move |request: Request<Body>, next: axum::middleware::Next<Body>| {
            let recorded = recorded.clone();
            async move {
                if request.uri().path().ends_with("/task-land") && request.method() == "POST" {
                    let (parts, body) = request.into_parts();
                    let bytes = to_bytes(body).await.unwrap();
                    *recorded.lock().unwrap() = Some(serde_json::from_slice(&bytes).unwrap());
                    return next
                        .run(Request::from_parts(parts, Body::from(bytes)))
                        .await;
                }
                next.run(request).await
            }
        },
    ));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server_router = router.clone();
    let server = tokio::spawn(async move {
        axum::Server::from_tcp(listener)
            .unwrap()
            .serve(server_router.into_make_service())
            .await
            .unwrap();
    });
    let cli = CaptureCli {
        executable: PathBuf::from(env::var_os("AIT_CAPTURE_TEST_CLI").expect("frozen Core CLI")),
        native_home: root.join("native-home"),
    };
    fs::create_dir(&cli.native_home).unwrap();
    let client = root.join("actor-finish-client");
    fs::create_dir(&client).unwrap();
    cli.json(&client, &["init", "--json"]);
    cli.json(
        &client,
        &[
            "config",
            "set",
            "--sprint",
            "off",
            "--agent-teams",
            "disabled",
            "--id-namespace-prefix",
            "AF",
            "--user-name",
            "Alice",
            "--user-email",
            "alice@example.test",
            "--json",
        ],
    );
    cli.json(
        &client,
        &["remote", "add", "origin", &url, "--default", "--json"],
    );
    let edit = root.join("actor-edit");
    let started = cli.json(
        &client,
        &[
            "task",
            "start",
            "--title",
            "Actor guarded finish",
            "--intent",
            "Persist author and actual initiator",
            "--remote",
            "origin",
            "--edit-root",
            edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    let task = started["task_id"].as_str().unwrap();
    let index = started["repo_id"].as_str().unwrap().parse::<u32>().unwrap();
    let base = format!("/v1/native/repository-authorities/{index}");
    fs::write(edit.join("source.txt"), "first reviewed version\n").unwrap();
    let initial = cli.json(
        &edit,
        &[
            "snapshot",
            "create",
            task,
            "--message",
            "Initial author A",
            "--json",
            "--full",
        ],
    );
    cli.json(
        &edit,
        &[
            "patchset",
            "publish",
            task,
            "--summary",
            "Reviewed first version",
            "--remote",
            "origin",
            "--json",
        ],
    );
    let change = cli.json(
        &edit,
        &[
            "change",
            "create",
            task,
            "--title",
            "Actual correction",
            "--remote",
            "origin",
            "--json",
        ],
    );
    let change_ref = change["change_ref"].as_str().unwrap();
    fs::write(edit.join("source.txt"), "corrected reviewed version\n").unwrap();
    let captured = cli.json(
        &edit,
        &[
            "snapshot",
            "create",
            change_ref,
            "--message",
            "Correction author A",
            "--json",
            "--full",
        ],
    );
    let snapshot_id = captured["snapshot_id"].as_str().unwrap();
    let patchset = cli.json(
        &edit,
        &[
            "patchset",
            "publish",
            change_ref,
            "--summary",
            "Exact corrected version",
            "--remote",
            "origin",
            "--json",
        ],
    );
    let patchset_id = patchset["patchset_id"].as_str().unwrap();
    let owner_path = format!("{base}/changes/{}", change_ref.replace('/', "%2F"));
    // Exercise the real Review/CI/Attestation/Policy state transitions. The test
    // supplies a completed isolated CI result; acceptance itself stays guarded.
    let (status,review)=call(&router,"POST",&format!("{owner_path}/reviews"),
        Some(json!({"patchset_id":patchset_id,"reviewer":"Human reviewer","reviewer_kind":"user","action":"approve"}))).await;
    assert!(status.is_success(), "{review}");
    let (_, workflow) = state
        .workflow_service
        .as_any()
        .downcast_ref::<RoutedBinaryWorkflowStore>()
        .unwrap()
        .store_for_repo(&index.to_string())
        .unwrap();
    let run = workflow
        .run_patchset_ci(patchset_id, &json!({"trigger":"workflow_ready"}))
        .unwrap();
    workflow.complete_patchset_ci(patchset_id,&json!({"patchset_id":patchset_id,"ci_run_seq":run["ci_run_seq"],
        "selected_suite_count":1,"suite_result_count":1,"blocking_failure_count":0,"overall_status":"pass","tests_status":"pass","lint_status":"pass"})).unwrap();
    workflow.put_attestation(patchset_id,&json!({"verification_state":"pass","require_tests_pass":true,"require_lint_pass":true,"require_human_review":true})).unwrap();
    assert_eq!(
        workflow.evaluate_policy(patchset_id).unwrap()["decision"],
        "pass"
    );
    drop(workflow);
    cli.json(
        &client,
        &[
            "config",
            "set",
            "--user-name",
            "Bob",
            "--user-email",
            "bob@example.test",
            "--json",
        ],
    );
    let mut bob = request_for(&state, index);
    bob["user_name"] = json!("Bob");
    bob["user_email"] = json!("bob@example.test");
    assert_eq!(register(&router, bob).await.1["actor_key_index"], 2);
    let guarded_request = json!({"actor_key_index":2,"contract":"task-land-atomic-guarded/v2",
        "idempotency_key":"actor-paired-guarded-finish","task_or_change_ref":task,"target_line":"main","mode":"direct",
        "expected_identity":{"repository_index":index,"patchset_id":patchset_id,
            "base_snapshot_id":patchset["base_snapshot_id"],"revision_snapshot_id":snapshot_id}});
    if guarded_api_first {
        let (status, accepted) = call(
            &router,
            "POST",
            &format!("{base}/task-land"),
            Some(guarded_request.clone()),
        )
        .await;
        assert!(status.is_success(), "{accepted}");
        assert_eq!(accepted["land"]["actor_key_index"], 2);
    }
    cli.json(
        &edit,
        &[
            "task", "finish", task, "--remote", "origin", "--json", "--full",
        ],
    );
    let task_path = format!("{base}/tasks/{task}");
    let (_, finished) = call(&router, "GET", &task_path, None).await;
    let land = finished["successful_land"].clone();
    assert_eq!(finished["actor_key_index"], 2);
    assert_eq!(land["actor_key_index"], 2);
    let (_, snapshot) = call(
        &router,
        "GET",
        &format!("{base}/snapshots/{snapshot_id}"),
        None,
    )
    .await;
    assert_eq!(snapshot["actor_key_index"], 0);
    assert_eq!(
        landed_request
            .lock()
            .unwrap()
            .as_ref()
            .expect("actual CLI finish request")["actor_key_index"],
        2
    );
    let mut replay = if guarded_api_first {
        guarded_request
    } else {
        landed_request.lock().unwrap().clone().unwrap()
    };
    replay["actor_key_index"] = json!(0);
    let (status, replayed) =
        call(&router, "POST", &format!("{base}/task-land"), Some(replay)).await;
    assert!(status.is_success(), "{replayed}");
    assert_eq!(replayed["replayed"], true);
    assert_eq!(replayed["land"], land);
    let (_, db) = state.operational_binary.repository_db(index).unwrap();
    assert_eq!(
        fs::read(
            db.resolve_record_path(&WorkflowBinaryV0Codec::land_file())
                .unwrap()
        )
        .unwrap()
        .len(),
        4 + 52
    );
    server.abort();
    let _ = server.await;
    drop(router);
    drop(state);
    drop(db);
    let started = std::time::Instant::now();
    let reopened = OperationalBinaryRuntime::open_generation(
        root.join("generation"),
        root.join("actor-reopened-leases.bin"),
        60,
        15,
    )
    .unwrap();
    let runtime_seconds = started.elapsed().as_secs_f64();
    let router = build_router_with_state(state_from_operational(Arc::new(reopened)));
    for path in [task_path, owner_path] {
        let (status, value) = call(&router, "GET", &path, None).await;
        assert!(status.is_success(), "{value}");
        assert_eq!(value["actor_key_index"], 2);
    }
    let (_, snapshot) = call(
        &router,
        "GET",
        &format!("{base}/snapshots/{snapshot_id}"),
        None,
    )
    .await;
    assert_eq!(snapshot["actor_key_index"], 0);
    let (_, first) = call(
        &router,
        "GET",
        &format!(
            "{base}/snapshots/{}",
            initial["snapshot_id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(first["actor_key_index"], 0);
    println!("Actual frozen Actor pair ({guarded_api_first}) passed capture A, finish B, exact replay A and persisted Task/Change/Snapshot/Land readback; normal runtime open={runtime_seconds:.6}s");
}
