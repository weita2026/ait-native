use super::actor_plan::call;
use super::snapshot_capture::CaptureCli;
use super::*;
use ait_server_core::foundation::workflow_binary_v0::WorkflowBinaryV0Codec;
use std::sync::Mutex;

fn metadata(state: &ServerState, index: u32) -> BTreeMap<String, Vec<u8>> {
    let (_, db) = state.operational_binary.repository_db(index).unwrap();
    let actor = db
        .resolve_record_path(&WorkflowBinaryV0Codec::actor_file())
        .unwrap();
    fs::read_dir(actor.parent().unwrap())
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| {
            entry.file_type().unwrap().is_file()
                && matches!(
                    entry.path().extension().and_then(|value| value.to_str()),
                    Some("bin" | "idx")
                )
        })
        .map(|entry| {
            (
                entry.file_name().into_string().unwrap(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run explicitly against the frozen companion with AIT_CAPTURE_TEST_CLI"]
async fn actual_actor_plan_cli_registration_publication_and_atomic_task_start() {
    let directory = TestDirectory::new("actor-plan-paired-cli");
    let root = directory.path().canonicalize().unwrap();
    let state = fresh_state(&root);
    let requests = Arc::new(Mutex::new(Vec::<(String, JsonValue)>::new()));
    let requests_in = requests.clone();
    let app = build_router_with_state(state.clone()).layer(axum::middleware::from_fn(
        move |request: Request<Body>, next: axum::middleware::Next<Body>| {
            let requests = requests_in.clone();
            async move {
                let path = request.uri().path().to_string();
                if request.method() == "POST"
                    && (path.ends_with("/repository-authorities")
                        || path.contains("/sprints")
                        || path.ends_with("/task-start"))
                {
                    let (parts, body) = request.into_parts();
                    let bytes = to_bytes(body).await.unwrap();
                    let value: JsonValue = serde_json::from_slice(&bytes).unwrap();
                    requests.lock().unwrap().push((path, value));
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
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::Server::from_tcp(listener)
            .unwrap()
            .serve(server_app.into_make_service())
            .await
            .unwrap();
    });
    let cli = CaptureCli {
        executable: PathBuf::from(
            env::var_os("AIT_CAPTURE_TEST_CLI").expect("exact frozen Core CLI"),
        ),
        native_home: root.join("native-home"),
    };
    assert!(cli.executable.is_file());
    fs::create_dir(&cli.native_home).unwrap();
    let client = root.join("actor-client");
    fs::create_dir(&client).unwrap();
    cli.json(&client, &["init", "--json"]);
    cli.json(
        &client,
        &[
            "config",
            "set",
            "--sprint",
            "on",
            "--agent-teams",
            "disabled",
            "--id-namespace-prefix",
            "AP",
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
    let index = 4;
    let base = format!("/v1/native/repository-authorities/{index}/sprints");
    let card = "docs/sprints/actor.md";
    fs::create_dir_all(client.join("docs/sprints")).unwrap();
    let original = "# Paired Actor [plan-ref: paired/actor]\n\n- [ ] First work. [ref: paired/first]\n- [ ] Second work. [ref: paired/second]\n";
    fs::write(client.join(card), original).unwrap();
    let sync = ["plan", "sync", card, "--remote", "origin", "--json"];
    assert_eq!(cli.json(&client, &sync)["status"], "ok");
    let (status, plans) = call(&app, "GET", &base, None).await;
    assert!(status.is_success(), "{plans}");
    let first = plans
        .as_array()
        .unwrap()
        .iter()
        .find(|plan| plan["head_artifact_path"] == card)
        .unwrap();
    assert_eq!(first["actor_key_index"], 0);
    assert_eq!(first["head_revision"]["actor_key_index"], 0);
    let plan_path = format!("{base}/{}", first["plan_id"].as_str().unwrap());
    let first_revision = first["head_revision_id"].as_str().unwrap();
    let (_, detail) = call(
        &app,
        "GET",
        &format!("{plan_path}/revisions/{first_revision}"),
        None,
    )
    .await;
    assert_eq!(detail["artifact_body"], original);
    assert!(detail["artifact_blob_id"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    let before = metadata(&state, index);
    assert_eq!(cli.json(&client, &sync)["status"], "ok");
    let after = metadata(&state, index);
    for (path, bytes) in &before {
        if path != "actor.bin" {
            assert_eq!(&after[path], bytes, "unchanged publication {path}");
        }
    }
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
    let revised_markdown = original.replace("First work.", "First revised work.");
    fs::write(client.join(card), &revised_markdown).unwrap();
    assert_eq!(cli.json(&client, &sync)["status"], "ok");
    let (_, revised) = call(&app, "GET", &plan_path, None).await;
    assert_ne!(revised["head_revision_id"], first["head_revision_id"]);
    assert_eq!(revised["actor_key_index"], 1);
    assert_eq!(revised["head_revision"]["actor_key_index"], 1);
    let (_, history) = call(
        &app,
        "GET",
        &format!("{plan_path}/revisions/{first_revision}"),
        None,
    )
    .await;
    assert_eq!(history["actor_key_index"], 0);
    let (_, detail) = call(
        &app,
        "GET",
        &format!(
            "{plan_path}/revisions/{}",
            revised["head_revision_id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(detail["artifact_body"], revised_markdown);

    // The actual CLI must resolve the new identity even when reusing a head.
    cli.json(
        &client,
        &[
            "config",
            "set",
            "--user-name",
            "Alice",
            "--user-email",
            "alice@example.test",
            "--json",
        ],
    );
    assert_eq!(cli.json(&client, &sync)["status"], "ok");
    let (_, reused) = call(&app, "GET", &plan_path, None).await;
    assert_eq!(reused["actor_key_index"], 1);
    assert_eq!(reused["head_revision"]["actor_key_index"], 1);
    let before = metadata(&state, index);
    cli.json(&client, &["config", "unset", "user-email", "--json"]);
    let missing_identity = cli.run(&client, &sync);
    assert!(!missing_identity.status.success());
    assert!(format!(
        "{}{}",
        String::from_utf8_lossy(&missing_identity.stdout),
        String::from_utf8_lossy(&missing_identity.stderr)
    )
    .contains("user_email"));
    assert_eq!(metadata(&state, index), before);
    cli.json(
        &client,
        &[
            "config",
            "set",
            "--user-email",
            "alice@example.test",
            "--json",
        ],
    );

    let from = format!("{card}#paired/first");
    let edit = root.join("first-edit");
    let existing = cli.json(
        &client,
        &[
            "task",
            "start",
            "--from",
            &from,
            "--intent",
            "Pair existing Plan Actor",
            "--remote",
            "origin",
            "--edit-root",
            edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    assert!(existing["task_id"].as_str().is_some());
    let atomic_card = "docs/sprints/atomic.md";
    fs::write(client.join(atomic_card), "# Atomic Actor [plan-ref: paired/atomic]\n\n- [ ] Create work. [ref: paired/create]\n- [ ] Revise work. [ref: paired/revise]\n").unwrap();
    for (reference, identity, email, label) in [
        ("paired/create", "Alice", "alice@example.test", "create"),
        ("paired/revise", "Bob", "bob@example.test", "revise"),
    ] {
        cli.json(
            &client,
            &[
                "config",
                "set",
                "--user-name",
                identity,
                "--user-email",
                email,
                "--json",
            ],
        );
        if label == "revise" {
            let body = fs::read_to_string(client.join(atomic_card))
                .unwrap()
                .replace("Revise work.", "Revised second work.");
            fs::write(client.join(atomic_card), body).unwrap();
        }
        let from = format!("{atomic_card}#{reference}");
        let edit = root.join(format!("{label}-edit"));
        let started = cli.json(
            &client,
            &[
                "task",
                "start",
                "--from",
                &from,
                "--intent",
                "Pair atomic Plan Actor",
                "--remote",
                "origin",
                "--edit-root",
                edit.to_str().unwrap(),
                "--json",
                "--full",
            ],
        );
        assert!(started["task_id"].as_str().is_some());
    }
    let recorded = requests.lock().unwrap().clone();
    let registrations = recorded
        .iter()
        .filter(|(path, _)| path.ends_with("/repository-authorities"))
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    assert!(registrations
        .iter()
        .any(|body| body["repository_index"] == index));
    for body in registrations
        .iter()
        .filter(|body| body.get("repository_index").is_some())
    {
        assert_eq!(body["repository_index"], index);
        assert_eq!(body["repository_name"], "actor-client");
        assert_eq!(body["namespace"], "AP");
        assert!(body["policy_flags"].as_u64().is_some());
    }
    let starts = recorded
        .iter()
        .filter(|(path, _)| path.ends_with("/task-start"))
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    for (action, actor) in [("existing", 0), ("create", 0), ("revise", 1)] {
        let request = starts
            .iter()
            .find(|request| request["plan"]["action"] == action)
            .unwrap();
        assert_eq!(request["contract"], "task-start-atomic/v2");
        assert_eq!(request["actor_key_index"], actor);
        assert!(request["plan"].get("actor_key_index").is_none());
        assert!(request["plan"]["payload"].get("actor_key_index").is_none());
    }
    let before = metadata(&state, index);
    assert_eq!(&before["task.bin"][..4], &2u32.to_le_bytes());
    assert_eq!(before["task.bin"].len(), 4 + 3 * 64);
    for (row, actor) in before["task.bin"][4..].chunks_exact(64).zip([1u32, 1, 2]) {
        assert_eq!(&row[60..], &actor.to_le_bytes());
    }
    let artifact_path = format!(
        "{plan_path}/revisions/{}/artifacts",
        revised["head_revision_id"].as_str().unwrap()
    );
    for actor in [JsonValue::Null, json!(u32::MAX), json!(2), json!(1)] {
        let (status, body) = call(&app, "PUT", &artifact_path,
            Some(json!({"actor_key_index":actor,"artifacts":[{"artifact_path":"docs/extra.json","role":"supporting","body":"{}","metadata":{}}]}))).await;
        assert!(!status.is_success(), "{body}");
        assert_eq!(metadata(&state, index), before);
    }
    println!("Frozen Core CLI pair passed: exact registration, packed Markdown create/revise/reuse, immutable authors, missing identity, atomic existing/create/revise, Task64/layout2 and unsupported supporting-artifact failures without effects");
    server.abort();
}
