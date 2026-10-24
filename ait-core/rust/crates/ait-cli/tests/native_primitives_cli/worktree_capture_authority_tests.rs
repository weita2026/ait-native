const CAPTURE_TEST_URL: &str = "https://capture-user:capture-secret@example.test/private-token";

fn capture_authority_fixture() -> (TempDir, PathBuf) {
    let temp = init_repo(&(CAPTURE_TEST_URL.to_string() + "///"));
    let root = temp.path().canonicalize().unwrap();
    let worktree = init_registered_worktree(
        &root,
        "capture-selected",
        "feature/capture-selected",
        None,
        None,
        false,
        None,
    );
    (temp, worktree)
}

fn capture_test_pin() -> JsonValue {
    json!({
        "capture_remote_name": "origin",
        "capture_remote_url": CAPTURE_TEST_URL,
        "capture_repository_index": FIXTURE_REPOSITORY_INDEX,
    })
}

fn set_capture_test_pin(root: &Path, pin: &JsonValue) {
    let path = root.join(".ait/worktrees/capture-selected.json");
    let mut metadata = parse_json_file(&path);
    let object = metadata.as_object_mut().unwrap();
    for key in [
        "capture_remote_name",
        "capture_remote_url",
        "capture_repository_index",
    ] {
        object.remove(key);
    }
    object.extend(pin.as_object().unwrap().clone());
    write_file(&path, &encode_json_pretty(&metadata));
}

fn assert_capture_test_no_secrets(output: &Output) {
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for secret in [
        CAPTURE_TEST_URL,
        "capture-user",
        "capture-secret",
        "private-token",
        "capture_remote_url",
        "capture_remote_name",
        "capture_repository_index",
    ] {
        assert!(
            !text.contains(secret),
            "disclosed private capture authority: {secret}"
        );
    }
}

#[test]
fn native_worktree_capture_authority_show_ordinary_json_selects_named_and_current_worktree() {
    let (_temp, selected) = capture_authority_fixture();
    let root = selected.parent().unwrap();
    let other = init_registered_worktree(
        root,
        "capture-other",
        "feature/capture-other",
        None,
        None,
        false,
        None,
    );
    let other_overlay = other.join(".ait-worktree.json");
    let mut overlay = parse_json_file(&other_overlay);
    overlay["repository_index"] = json!(99);
    write_file(&other_overlay, &encode_json_pretty(&overlay));
    let config_path = root.join(".ait/config.json");
    let mut config = parse_json_file(&config_path);
    config["default_remote"] = json!("unrelated-default");
    write_file(&config_path, &encode_json_pretty(&config));
    set_capture_test_pin(root, &capture_test_pin());
    let before_snapshots = json_output(root, &["snapshot", "list", "--json"]);
    let before_lines = json_output(root, &["line", "list", "--all", "--json"]);
    let before_config = fs::read(root.join(".ait/config.json")).unwrap();
    for (cwd, args) in [
        (root, vec!["worktree", "show", "capture-selected", "--json"]),
        (selected.as_path(), vec!["worktree", "show", "--json"]),
        (
            other.as_path(),
            vec!["worktree", "show", "capture-selected", "--json"],
        ),
    ] {
        // Call the executable directly: no helper may add --full or bypass stdout rendering.
        let output = command_output_with_env(cwd, &args, &[]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_capture_test_no_secrets(&output);
        let payload = parse_json_bytes(&output.stdout);
        assert_eq!(payload["name"], json!("capture-selected"));
        assert_eq!(
            payload["capture_authority"],
            json!({
                "remote_name": "origin", "repository_index": FIXTURE_REPOSITORY_INDEX,
            })
        );
        assert_eq!(payload["current_line"], json!("feature/capture-selected"));
        assert_eq!(payload["head_snapshot_id"], json!(FIXTURE_BASE_SNAPSHOT_ID));
        assert!(payload.get("bound_task_id").is_some());
    }
    let other_payload = json_output(&other, &["worktree", "show", "--json"]);
    assert_eq!(other_payload["name"], json!("capture-other"));
    assert!(other_payload
        .as_object()
        .unwrap()
        .contains_key("capture_authority"));
    assert_eq!(other_payload["capture_authority"], JsonValue::Null);
    let list = json_output(root, &["worktree", "list", "--json"]);
    assert!(list
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row.get("capture_authority").is_none()));
    assert_eq!(
        json_output(root, &["snapshot", "list", "--json"]),
        before_snapshots
    );
    assert_eq!(
        json_output(root, &["line", "list", "--all", "--json"]),
        before_lines
    );
    assert_eq!(
        fs::read(root.join(".ait/config.json")).unwrap(),
        before_config
    );
    let metadata = parse_json_file(root.join(".ait/worktrees/capture-selected.json"));
    assert!(metadata.get("capture_pending").is_none());
}

#[test]
fn native_worktree_capture_authority_show_rejects_partial_and_malformed_pins() {
    let (temp, _) = capture_authority_fixture();
    let root = temp.path();
    let valid = capture_test_pin();
    let keys = [
        "capture_remote_name",
        "capture_remote_url",
        "capture_repository_index",
    ];
    let mut cases = Vec::new();
    for mask in 1..7 {
        let mut pin = json!({});
        for (bit, key) in keys.iter().enumerate() {
            if mask & (1 << bit) != 0 {
                pin[*key] = valid[*key].clone();
            }
        }
        cases.push(pin);
    }
    for key in keys {
        for malformed in [JsonValue::Null, json!(true), json!([]), json!({})] {
            let mut pin = valid.clone();
            pin[key] = malformed;
            cases.push(pin);
        }
    }
    for key in ["capture_remote_name", "capture_remote_url"] {
        for malformed in [json!(""), json!(" \t"), json!(7)] {
            let mut pin = valid.clone();
            pin[key] = malformed;
            cases.push(pin);
        }
    }
    for malformed in [
        json!(-1),
        json!(7.0),
        json!(7.5),
        json!("7"),
        json!(4_294_967_296_u64),
    ] {
        let mut pin = valid.clone();
        pin["capture_repository_index"] = malformed;
        cases.push(pin);
    }
    cases.push(json!({
        "capture_remote_name": null, "capture_remote_url": null, "capture_repository_index": null,
    }));
    for pin in cases {
        set_capture_test_pin(root, &pin);
        let output = command_output_with_env(
            root,
            &["worktree", "show", "capture-selected", "--json"],
            &[],
        );
        assert!(!output.status.success(), "accepted malformed pin: {pin}");
        assert!(output.stdout.is_empty());
        assert_capture_test_no_secrets(&output);
        assert!(String::from_utf8_lossy(&output.stderr).contains("Verify its pinned authority"));
        let stored = parse_json_file(root.join(".ait/worktrees/capture-selected.json"));
        for (key, value) in pin.as_object().unwrap() {
            assert_eq!(&stored[key], value, "read command repaired {key}");
        }
    }
}

#[test]
fn native_worktree_capture_authority_show_rejects_selected_identity_mismatch() {
    let (temp, selected) = capture_authority_fixture();
    set_capture_test_pin(temp.path(), &capture_test_pin());
    let overlay_path = selected.join(".ait-worktree.json");
    let mut overlay = parse_json_file(&overlay_path);
    overlay["worktree_name"] = json!("unrelated-worktree");
    write_file(&overlay_path, &encode_json_pretty(&overlay));
    let output = command_output_with_env(
        temp.path(),
        &["worktree", "show", "capture-selected", "--json"],
        &[],
    );
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Repository identity does not match"));
    assert_capture_test_no_secrets(&output);
}

#[test]
fn native_worktree_capture_authority_show_rejects_authority_mismatches_and_accepts_zero() {
    let (temp, selected) = capture_authority_fixture();
    let root = temp.path();
    for (key, mismatch) in [
        ("capture_remote_name", json!("missing")),
        ("capture_remote_name", json!(" origin ")),
        ("capture_remote_url", json!("https://other.test")),
        (
            "capture_remote_url",
            json!(CAPTURE_TEST_URL.to_string() + "/"),
        ),
        ("capture_repository_index", json!(8)),
    ] {
        let mut pin = capture_test_pin();
        pin[key] = mismatch;
        set_capture_test_pin(root, &pin);
        let output = command_output_with_env(&selected, &["worktree", "show", "--json"], &[]);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_capture_test_no_secrets(&output);
    }
    let config_path = root.join(".ait/config.json");
    let mut config = parse_json_file(&config_path);
    config["repository_index"] = json!(0);
    write_file(&config_path, &encode_json_pretty(&config));
    let mut pin = capture_test_pin();
    pin["capture_repository_index"] = json!(0);
    set_capture_test_pin(root, &pin);
    let payload = json_output(root, &["worktree", "show", "capture-selected", "--json"]);
    assert_eq!(
        payload["capture_authority"],
        json!({"remote_name": "origin", "repository_index": 0})
    );
    config.as_object_mut().unwrap().remove("repository_index");
    write_file(&config_path, &encode_json_pretty(&config));
    let output = command_output_with_env(
        root,
        &["worktree", "show", "capture-selected", "--json"],
        &[],
    );
    assert!(!output.status.success());
    assert_capture_test_no_secrets(&output);
}

#[test]
fn native_worktree_capture_authority_show_has_no_remote_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = Server::from_listener(listener, None).unwrap();
    let requested = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let worker_requested = Arc::clone(&requested);
    let worker_stopped = Arc::clone(&stopped);
    let worker = thread::spawn(move || {
        while !worker_stopped.load(Ordering::SeqCst) {
            if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(10)) {
                worker_requested.store(true, Ordering::SeqCst);
                request.respond(Response::empty(500)).unwrap();
            }
        }
    });
    let temp = init_repo(&url);
    let root = temp.path().canonicalize().unwrap();
    init_registered_worktree(
        &root,
        "capture-selected",
        "feature/capture-selected",
        None,
        None,
        false,
        None,
    );
    let mut pin = capture_test_pin();
    pin["capture_remote_url"] = json!(url);
    set_capture_test_pin(temp.path(), &pin);
    let output = command_output_with_env(
        temp.path(),
        &["worktree", "show", "capture-selected", "--json"],
        &[],
    );
    stopped.store(true, Ordering::SeqCst);
    worker.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !requested.load(Ordering::SeqCst),
        "show contacted remote authority"
    );
}
