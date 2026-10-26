use super::*;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::AtomicU8;
use std::sync::Mutex;

async fn post_capture_fixture(
    router: &Router,
    uri: String,
    value: &JsonValue,
) -> axum::response::Response {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(value).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}

pub(super) struct CaptureCli {
    pub(super) executable: PathBuf,
    pub(super) native_home: PathBuf,
}

impl CaptureCli {
    fn command(&self, root: &FsPath, args: &[&str]) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .current_dir(root)
            .env("CODEX_HOME", &self.native_home)
            .args(args);
        command
    }
    pub(super) fn run(&self, root: &FsPath, args: &[&str]) -> Output {
        self.command(root, args)
            .output()
            .expect("run actual capture CLI")
    }
    pub(super) fn run_bounded(&self, root: &FsPath, args: &[&str]) -> (Output, bool) {
        let mut child = self
            .command(root, args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let timed_out = loop {
            if child.try_wait().unwrap().is_some() {
                break false;
            }
            if std::time::Instant::now() >= deadline {
                child.kill().unwrap();
                eprintln!("Stopped bounded paired CLI after 10s: {args:?}");
                break true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        (child.wait_with_output().unwrap(), timed_out)
    }
    pub(super) fn json(&self, root: &FsPath, args: &[&str]) -> JsonValue {
        let result = self.run(root, args);
        assert!(
            result.status.success(),
            "{args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        serde_json::from_slice(&result.stdout).expect("CLI JSON")
    }
    pub(super) fn rejects(&self, root: &FsPath, args: &[&str], reason: &str) {
        let result = self.run(root, args);
        assert!(!result.status.success(), "unexpected success: {args:?}");
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains(reason), "{args:?}: {error}");
    }
}

async fn persisted(router: &Router, index: u32, id: &str) -> JsonValue {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/native/repository-authorities/{index}/snapshots/{id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

/// This test deliberately crosses processes and HTTP. A hand-made projection
/// cannot establish that the capture entrypoint persisted the ownership.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run explicitly with AIT_CAPTURE_TEST_CLI pointing to the exact companion CLI build"]
async fn actual_remote_capture_persists_task_then_exact_change_after_reopen() {
    let directory = TestDirectory::new("actual-remote-capture");
    let fixture_root = directory.path().canonicalize().unwrap();
    let executable = PathBuf::from(
        env::var_os("AIT_CAPTURE_TEST_CLI").expect("set exact companion CLI executable"),
    );
    assert!(executable.is_file());
    let cli = CaptureCli {
        executable,
        native_home: fixture_root.join("native-home"),
    };
    fs::create_dir_all(&cli.native_home).unwrap();
    let router = build_router_with_state(fresh_state(&fixture_root));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let fault = Arc::new(AtomicU8::new(0));
    let last_capture = Arc::new(Mutex::new(None::<JsonValue>));
    let capture_posts = Arc::new(Mutex::new(BTreeMap::<String, usize>::new()));
    let fault_in = fault.clone();
    let last_in = last_capture.clone();
    let posts_in = capture_posts.clone();
    let server_router = router.clone().layer(axum::middleware::from_fn(
        move |request: Request<Body>, next: axum::middleware::Next<Body>| {
            let fault = fault_in.clone();
            let last = last_in.clone();
            let posts = posts_in.clone();
            async move {
                let lose_ack = {
                    let saved = last.lock().unwrap();
                    fault.load(Ordering::SeqCst) == 6
                        && request.method() == "GET"
                        && saved
                            .as_ref()
                            .and_then(|v| v["capture_binding"]["snapshot_id"].as_str())
                            .is_some_and(|id| {
                                request.uri().path().ends_with(&format!("/snapshots/{id}"))
                            })
                };
                if lose_ack {
                    fault.store(0, Ordering::SeqCst);
                    return axum::response::IntoResponse::into_response((
                        StatusCode::SERVICE_UNAVAILABLE,
                        "lost first capture acknowledgment",
                    ));
                }
                if !request
                    .uri()
                    .path()
                    .ends_with("/remote-sync/zstd-bulk/commit")
                {
                    return next.run(request).await;
                }
                let (mut parts, body) = request.into_parts();
                let bytes = to_bytes(body).await.unwrap();
                let mut value: JsonValue = serde_json::from_slice(&bytes).unwrap();
                if value.get("capture_binding").is_some() {
                    *last.lock().unwrap() = Some(value.clone());
                    *posts
                        .lock()
                        .unwrap()
                        .entry(
                            value["capture_binding"]["snapshot_id"]
                                .as_str()
                                .unwrap()
                                .to_owned(),
                        )
                        .or_default() += 1;
                    match fault.load(Ordering::SeqCst) {
                        1 => value["capture_binding"]["repository_index"] = json!(u32::MAX),
                        2 => value["capture_binding"]["line_name"] = json!("feature/wrong-task"),
                        3 => {
                            value.as_object_mut().unwrap().remove("capture_binding");
                        }
                        4 => {
                            value.as_object_mut().unwrap().remove("capture_binding");
                            value["contract"] = json!("ait.remote_sync.zstd_bulk.v1");
                        }
                        _ => {}
                    }
                }
                parts.headers.remove("content-length");
                let lose_response =
                    fault.load(Ordering::SeqCst) == 5 && value.get("capture_binding").is_some();
                let response = next
                    .run(Request::from_parts(
                        parts,
                        Body::from(serde_json::to_vec(&value).unwrap()),
                    ))
                    .await;
                if lose_response {
                    assert!(response.status().is_success());
                    fault.store(6, Ordering::SeqCst);
                    axum::response::IntoResponse::into_response((
                        StatusCode::SERVICE_UNAVAILABLE,
                        "lost accepted capture response",
                    ))
                } else {
                    response
                }
            }
        },
    ));
    let server = tokio::spawn(async move {
        axum::Server::from_tcp(listener)
            .unwrap()
            .serve(server_router.into_make_service())
            .await
            .unwrap();
    });
    let root = fixture_root.join("capture-client");
    fs::create_dir(&root).unwrap();
    cli.json(&root, &["init", "--json"]);
    cli.json(
        &root,
        &[
            "config",
            "set",
            "--sprint",
            "off",
            "--agent-teams",
            "disabled",
            "--id-namespace-prefix",
            "CP",
            "--user-name",
            "capture-test",
            "--user-email",
            "capture@example.test",
            "--json",
        ],
    );
    cli.json(
        &root,
        &["remote", "add", "origin", &url, "--default", "--json"],
    );
    let edit = fixture_root.join("capture-edit");
    let started = cli.json(
        &root,
        &[
            "task",
            "start",
            "--title",
            "Capture ownership",
            "--intent",
            "Verify exact persisted remote capture ownership",
            "--remote",
            "origin",
            "--edit-root",
            edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    let task = started["task_id"].as_str().unwrap();
    let index: u32 = started["repo_id"].as_str().unwrap().parse().unwrap();
    let first_owner = task.to_string();
    let fork = started["worktree"]["fork_snapshot_id"].as_str().unwrap();
    fs::write(edit.join("source.txt"), "initial Task work\n").unwrap();
    let first = cli.json(
        &edit,
        &[
            "snapshot",
            "create",
            &first_owner,
            "--message",
            "first capture",
            "--json",
            "--full",
        ],
    );
    let first_id = first["snapshot_id"].as_str().unwrap();
    assert!(first["capture_binding"]["change_ref"].is_null());
    let saved_first = persisted(&router, index, first_id).await;
    assert_eq!(saved_first["capture_binding"], first["capture_binding"]);
    assert_eq!(saved_first["capture_binding"]["repository_index"], index);
    assert_eq!(saved_first["actor_key_index"], 0);
    let first_capture_request = last_capture.lock().unwrap().clone().unwrap();
    cli.json(
        &root,
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
    // Initial work has Task ownership and requires no automatic Change.
    let initial_candidate = cli.json(
        &edit,
        &[
            "patchset",
            "publish",
            &first_owner,
            "--summary",
            "Publish exact first capture",
            "--remote",
            "origin",
            "--json",
        ],
    );
    let second = cli.json(
        &edit,
        &[
            "change",
            "create",
            task,
            "--title",
            "Real correction Change",
            "--remote",
            "origin",
            "--json",
        ],
    );
    let second_change = second["change_ref"].as_str().unwrap();
    assert_eq!(second_change, format!("{task}/C-01"));
    assert_eq!(second["fork_snapshot_id"], first_id);
    fs::write(edit.join("sibling.txt"), "second Change\n").unwrap();
    let captured_second = cli.json(
        &edit,
        &[
            "snapshot",
            "create",
            second_change,
            "--message",
            "sibling capture",
            "--json",
            "--full",
        ],
    );
    let second_id = captured_second["snapshot_id"].as_str().unwrap();
    assert_eq!(
        persisted(&router, index, second_id).await["actor_key_index"],
        1
    );
    let mut retry_first = first_capture_request;
    retry_first["actor_key_index"] = json!(1);
    let retried = post_capture_fixture(
        &router,
        format!("/v1/native/repository-authorities/{index}/remote-sync/zstd-bulk/commit"),
        &retry_first,
    )
    .await;
    assert!(
        retried.status().is_success(),
        "{}",
        response_json(retried).await
    );
    assert_eq!(
        persisted(&router, index, first_id).await["actor_key_index"],
        0
    );
    assert_eq!(
        captured_second["capture_binding"]["change_ref"],
        second_change
    );
    assert_eq!(
        persisted(&router, index, second_id).await["capture_binding"],
        captured_second["capture_binding"]
    );
    // C01 is real server-owned correction data on the same Task and Line.
    let correction = cli.json(
        &edit,
        &[
            "patchset",
            "publish",
            &first_owner,
            "--summary",
            "Publish exact current correction",
            "--remote",
            "origin",
            "--json",
        ],
    );
    assert_eq!(correction["change_ref"], second_change, "{correction}");
    assert_eq!(correction["revision_snapshot_id"], second_id);
    let resumed = cli.json(
        &edit,
        &[
            "snapshot",
            "create",
            &first_owner,
            "--resume",
            second_id,
            "--json",
            "--full",
        ],
    );
    // Task input resolves its unique actual revision; it cannot alter the
    // already durable optional-Change association during resume.
    assert_eq!(resumed["snapshot_id"], second_id);
    assert_eq!(
        persisted(&router, index, second_id).await["capture_binding"],
        captured_second["capture_binding"]
    );
    // The legitimate old bootstrap Snapshot has no binding; resume must not claim it.
    cli.rejects(
        &edit,
        &[
            "snapshot",
            "create",
            &first_owner,
            "--resume",
            fork,
            "--json",
        ],
        "exact existing durable capture binding",
    );
    assert!(persisted(&router, index, fork).await["capture_binding"].is_null());
    cli.json(
        &edit,
        &[
            "snapshot",
            "create",
            second_change,
            "--resume",
            second_id,
            "--json",
        ],
    );
    // Hit the real numeric publication endpoint, bypassing all CLI guards.
    let foreign = post_capture_fixture(&router, format!("/v1/native/repository-authorities/{index}/changes/{}/patchsets",first_owner.replace('/',"%2F")),
        &json!({"actor_key_index":1,"base_snapshot_id":fork,"revision_snapshot_id":second_id,"summary":"Reject sibling at server","author_mode":"ai_with_human_review"})).await;
    assert_eq!(foreign.status(), StatusCode::BAD_REQUEST);
    assert!(response_json(foreign)
        .await
        .to_string()
        .contains("not owned"));
    cli.rejects(
        &edit,
        &[
            "snapshot",
            "create",
            "RCPT-9999/C-01",
            "--message",
            "wrong Task",
            "--json",
        ],
        "does not match",
    );
    // Corrupt only isolated fixture identity, then restore it.
    let identity_path = edit.join(".ait-worktree.json");
    let identity_bytes = fs::read(&identity_path).unwrap();
    let mut identity: JsonValue = serde_json::from_slice(&identity_bytes).unwrap();
    identity["current_line"] = json!("main");
    fs::write(&identity_path, serde_json::to_vec(&identity).unwrap()).unwrap();
    let wrong_line = cli.run(
        &edit,
        &[
            "snapshot",
            "create",
            second_change,
            "--message",
            "wrong Line",
            "--json",
        ],
    );
    assert!(!wrong_line.status.success());
    identity = serde_json::from_slice(&identity_bytes).unwrap();
    identity["worktree_name"] = json!("foreign-worktree");
    fs::write(&identity_path, serde_json::to_vec(&identity).unwrap()).unwrap();
    assert!(!cli
        .run(
            &edit,
            &[
                "snapshot",
                "create",
                second_change,
                "--message",
                "wrong worktree",
                "--json"
            ]
        )
        .status
        .success());
    fs::write(&identity_path, &identity_bytes).unwrap();
    let correction_retry = cli.json(
        &edit,
        &[
            "patchset",
            "publish",
            second_change,
            "--summary",
            "Publish exact correction capture",
            "--remote",
            "origin",
            "--json",
        ],
    );
    assert_eq!(correction_retry["patchset_id"], correction["patchset_id"]);
    let expected_identity = fixture_root.join("old-candidate.json");
    fs::write(
        &expected_identity,
        serde_json::to_vec(&json!({
            "repository_index":index,"patchset_id":initial_candidate["patchset_id"],
            "base_snapshot_id":fork,"revision_snapshot_id":first_id
        }))
        .unwrap(),
    )
    .unwrap();
    let before_drift =
        ait_server_core::foundation::server_workflow_upgrade::workflow_generation_fingerprint(
            &fixture_root.join("generation"),
        )
        .unwrap();
    let (stale, timed_out) = cli.run_bounded(
        &edit,
        &[
            "workflow",
            "ready",
            task,
            "--remote",
            "origin",
            "--apply",
            "--expected-identity",
            expected_identity.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!timed_out, "stale candidate must reject before CI");
    assert!(!stale.status.success());
    assert_eq!(
        ait_server_core::foundation::server_workflow_upgrade::workflow_generation_fingerprint(
            &fixture_root.join("generation")
        )
        .unwrap(),
        before_drift
    );
    let foreign_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    foreign_listener.set_nonblocking(true).unwrap();
    let foreign_url = format!("http://{}", foreign_listener.local_addr().unwrap());
    let foreign_router = router.clone();
    let foreign_server = tokio::spawn(async move {
        axum::Server::from_tcp(foreign_listener)
            .unwrap()
            .serve(foreign_router.into_make_service())
            .await
            .unwrap();
    });
    cli.json(&root, &["remote", "add", "foreign", &foreign_url, "--json"]);
    // A foreign refresh must validate the capture authority. Reusing an
    // already-published candidate does not need a capture/refresh operation.
    fs::write(
        edit.join("refresh.txt"),
        "new frozen revision for refresh guard\n",
    )
    .unwrap();
    let refresh = cli.json(
        &edit,
        &[
            "snapshot",
            "create",
            second_change,
            "--message",
            "Refresh guard revision",
            "--json",
            "--full",
        ],
    );
    let refresh_id = refresh["snapshot_id"].as_str().unwrap();
    cli.rejects(
        &edit,
        &[
            "patchset",
            "publish",
            second_change,
            "--summary",
            "wrong authority",
            "--remote",
            "foreign",
            "--json",
        ],
        "pinned capture authority",
    );
    let (foreign_ready, timed_out) = cli.run_bounded(
        &edit,
        &[
            "workflow",
            "ready",
            second_change,
            "--apply",
            "--remote",
            "foreign",
            "--json",
        ],
    );
    assert!(
        !timed_out && !foreign_ready.status.success(),
        "foreign refresh must reject before CI"
    );
    assert!(
        String::from_utf8_lossy(&foreign_ready.stderr).contains("pinned capture authority"),
        "{}",
        String::from_utf8_lossy(&foreign_ready.stderr)
    );
    foreign_server.abort();
    let _ = foreign_server.await;
    let before_head = cli.json(&edit, &["line", "show", "--json"]);
    assert_eq!(before_head["head_snapshot_id"], refresh_id);
    // Three failures exercise request validation and same-transaction rollback.
    for mode in 1..=3 {
        fault.store(mode, Ordering::SeqCst);
        fs::write(edit.join("failure.txt"), format!("failed capture {mode}\n")).unwrap();
        cli.rejects(
            &edit,
            &[
                "snapshot",
                "create",
                second_change,
                "--message",
                &format!("fault {mode}"),
                "--json",
            ],
            "SNAPSHOT_CAPTURE_INCOMPLETE",
        );
        let attempted = last_capture.lock().unwrap().clone().unwrap();
        let id = attempted["capture_binding"]["snapshot_id"]
            .as_str()
            .unwrap();
        let absent = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/native/repository-authorities/{index}/snapshots/{id}"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(absent.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            cli.json(&edit, &["line", "show", "--json"])["head_snapshot_id"],
            before_head["head_snapshot_id"]
        );
    }
    // A fresh Task capture cannot downgrade into ordinary publication.
    fault.store(4, Ordering::SeqCst);
    fs::write(edit.join("failure.txt"), "unbound fresh Task capture\n").unwrap();
    cli.rejects(
        &edit,
        &[
            "snapshot",
            "create",
            second_change,
            "--message",
            "unbound capture",
            "--json",
        ],
        "SNAPSHOT_CAPTURE_INCOMPLETE",
    );
    let unbound = last_capture.lock().unwrap().clone().unwrap();
    let unbound_id = unbound["capture_binding"]["snapshot_id"].as_str().unwrap();
    let absent = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/native/repository-authorities/{index}/snapshots/{unbound_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(absent.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        cli.json(&edit, &["line", "show", "--json"])["head_snapshot_id"],
        before_head["head_snapshot_id"]
    );
    fs::remove_file(edit.join("failure.txt")).unwrap();
    let base_edit = fixture_root.join("base-advance-edit");
    let base_started = cli.json(
        &root,
        &[
            "task",
            "start",
            "--title",
            "Advance base",
            "--intent",
            "Exercise ordinary base advance",
            "--local",
            "--edit-root",
            base_edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    let base_task = base_started["task_id"].as_str().unwrap();
    fs::write(base_edit.join("base.txt"), "new independent base file\n").unwrap();
    let base_capture = cli.json(
        &base_edit,
        &[
            "snapshot",
            "create",
            base_task,
            "--message",
            "Advanced independent base",
            "--json",
            "--full",
        ],
    );
    let base_id = base_capture["snapshot_id"].as_str().unwrap();
    cli.json(
        &root,
        &[
            "line",
            "create",
            "capture-base",
            "--from-snapshot",
            base_id,
            "--json",
        ],
    );
    cli.json(
        &root,
        &[
            "config",
            "set",
            "--user-name",
            "capture-test",
            "--user-email",
            "capture@example.test",
            "--json",
        ],
    );
    fault.store(1, Ordering::SeqCst);
    cli.rejects(
        &edit,
        &["worktree", "rebase", "--onto", "capture-base", "--json"],
        "SNAPSHOT_CAPTURE_INCOMPLETE",
    );
    assert_eq!(
        cli.json(&edit, &["line", "show", "--json"])["head_snapshot_id"],
        refresh_id
    );
    assert!(
        !edit.join("base.txt").exists(),
        "failed rebase must restore the verified old workspace"
    );
    let rejected_id = last_capture.lock().unwrap().as_ref().unwrap()["capture_binding"]
        ["snapshot_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let absent = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/native/repository-authorities/{index}/snapshots/{rejected_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(absent.status(), StatusCode::NOT_FOUND);
    cli.json(
        &root,
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
    fault.store(0, Ordering::SeqCst);
    println!("Rebase pair: local_base={base_id}, current_feature_head={refresh_id}, original_remote_fork={fork}, last_capture={}", last_capture.lock().unwrap().as_ref().unwrap()["capture_binding"]["snapshot_id"]);
    cli.json(
        &edit,
        &["worktree", "rebase", "--onto", "capture-base", "--json"],
    );
    let rebased = cli.json(&edit, &["line", "show", "--json"]);
    let rebased_id = rebased["head_snapshot_id"].as_str().unwrap();
    assert_ne!(rebased_id, refresh_id);
    assert_ne!(rebased_id, rejected_id);
    assert_eq!(
        persisted(&router, index, rebased_id).await["actor_key_index"],
        1
    );
    assert_eq!(
        fs::read_to_string(edit.join("base.txt")).unwrap(),
        "new independent base file\n"
    );
    assert_eq!(
        persisted(&router, index, rebased_id).await["capture_binding"]["change_ref"],
        second_change
    );
    fs::write(base_edit.join("base-two.txt"), "second independent base\n").unwrap();
    let base_two = cli.json(
        &base_edit,
        &[
            "snapshot",
            "create",
            base_task,
            "--message",
            "Second base",
            "--json",
            "--full",
        ],
    );
    cli.json(
        &root,
        &[
            "line",
            "create",
            "capture-base-two",
            "--from-snapshot",
            base_two["snapshot_id"].as_str().unwrap(),
            "--json",
        ],
    );
    cli.json(
        &root,
        &[
            "config",
            "set",
            "--user-name",
            "capture-test",
            "--user-email",
            "capture@example.test",
            "--json",
        ],
    );
    fault.store(5, Ordering::SeqCst);
    cli.rejects(
        &edit,
        &["worktree", "rebase", "--onto", "capture-base-two", "--json"],
        "SNAPSHOT_CAPTURE_INCOMPLETE",
    );
    let accepted_id = last_capture.lock().unwrap().as_ref().unwrap()["capture_binding"]
        ["snapshot_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        persisted(&router, index, &accepted_id).await["actor_key_index"],
        0
    );
    assert_eq!(
        cli.json(&edit, &["line", "show", "--json"])["head_snapshot_id"],
        rebased_id
    );
    cli.json(
        &root,
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
    cli.json(
        &edit,
        &["worktree", "rebase", "--onto", "capture-base-two", "--json"],
    );
    assert_eq!(
        cli.json(&edit, &["line", "show", "--json"])["head_snapshot_id"],
        accepted_id
    );
    assert_eq!(capture_posts.lock().unwrap().get(&accepted_id), Some(&1));
    assert_eq!(
        persisted(&router, index, &accepted_id).await["actor_key_index"],
        0
    );
    cli.json(
        &root,
        &[
            "line",
            "create",
            "capture-source",
            "--from-snapshot",
            second_id,
            "--json",
        ],
    );
    cli.rejects(
        &edit,
        &[
            "line",
            "merge",
            "capture-source",
            "--message",
            "Capture merge recovery",
            "--json",
        ],
        "remote_sync.snapshot_dag.v2",
    );
    assert_eq!(
        cli.json(&edit, &["line", "show", "--json"])["head_snapshot_id"],
        accepted_id
    );
    // The existing server does not advertise multi-parent upload. Refuse before
    // workspace/head/merge-state mutation rather than extending that protocol.
    cli.rejects(
        &edit,
        &["line", "merge", "--continue", "--json"],
        "no conflicted line merge",
    );
    // Reopening the CLI is exercised by each command; reopen the actual server authority too.
    server.abort();
    let _ = server.await;
    drop(router);
    let reopened = Arc::new(
        OperationalBinaryRuntime::open_generation(
            fixture_root.join("generation"),
            fixture_root.join("runtime-worker-leases-reopened.bin"),
            60,
            15,
        )
        .unwrap(),
    );
    let reopened_router = build_router_with_state(state_from_operational(reopened));
    assert_eq!(
        persisted(&reopened_router, index, first_id).await["capture_binding"],
        first["capture_binding"]
    );
    assert_eq!(
        persisted(&reopened_router, index, second_id).await["capture_binding"],
        captured_second["capture_binding"]
    );
    assert_eq!(
        persisted(&reopened_router, index, rebased_id).await["capture_binding"]["change_ref"],
        second_change
    );
    assert_eq!(
        persisted(&reopened_router, index, first_id).await["actor_key_index"],
        0
    );
    assert_eq!(
        persisted(&reopened_router, index, second_id).await["actor_key_index"],
        1
    );
    assert!(persisted(&reopened_router, index, fork).await["actor_key_index"].is_null());
    assert_eq!(
        persisted(&reopened_router, index, &accepted_id).await["actor_key_index"],
        0
    );
    println!("actual remote capture persisted and reopened: repo={index}, task={task}, initialTaskSnapshot={first_id}, correctionC01={second_id}, refresh={refresh_id}");
}
