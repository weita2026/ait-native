#[test]
fn native_patchset_publish_recovers_busy_response_from_exact_task_candidate() {
    let (base_url, log, state, handle) = spawn_publish_recovery_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    let snapshot = checkpoint_bound_worktree_fixture(&worktree, "recovery snapshot");
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    let args = [
        "patchset",
        "publish",
        "RT-1",
        "--summary",
        "Recovery publish",
        "--json",
    ];
    let patchset = json_output(&worktree, &args);
    assert_eq!(patchset["patchset"]["patchset_id"], "RT-1/P-01");
    assert_eq!(patchset["patchset"]["revision_snapshot_id"], snapshot_id);
    assert!(patchset["patchset"]["change_id"].is_null());
    assert!(patchset["patchset"]["response_recovery"].is_null());
    let replay = json_output(&worktree, &args);
    assert_eq!(replay["patchset"], patchset["patchset"]);
    handle.join().unwrap();
    let requests = log.lock().unwrap();
    let publish = requests
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/changes/RT-1/patchsets"
        })
        .unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|row| row.method == "POST" && row.url.ends_with("/patchsets"))
            .count(),
        1
    );
    assert!(requests[publish + 1..]
        .iter()
        .any(|row| row.method == "GET" && row.url.ends_with("/tasks/RT-1")));
    assert!(requests[publish + 1..]
        .iter()
        .any(|row| row.method == "GET" && row.url.ends_with("/patchsets/RT-1%2FP-01")));
}

const CLOSEOUT_RECOVERY_ENV: [(&str, &str); 3] = [
    ("AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS", "0.01"),
    ("AIT_REMOTE_MUTATION_SETTLE_WINDOW_SECONDS", "0.25"),
    ("AIT_REMOTE_MUTATION_SETTLE_POLL_SECONDS", "0.002"),
];

fn closeout_recovery_json(root: &Path, fixture_seed: u64, phase: &str) -> JsonValue {
    let output = command_output_with_env(
        root,
        &[
            "task", "finish", "RT-1", "--remote", "origin", "--json", "--full",
        ],
        &CLOSEOUT_RECOVERY_ENV,
    );
    assert!(
        output.status.success(),
        "closeout recovery failed: phase={phase} fixture_seed={fixture_seed:#x} fixture_root={}\nstdout:\n{}\n\nstderr:\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json_bytes(&output.stdout)
}

#[test]
fn native_first_local_task_finish_descends_from_task_start_baseline() {
    let temp = TempDir::new().expect("first-land repository tempdir");
    let root = temp.path();
    initialize_repo(&InitRequest {
        root: root.to_path_buf(),
        name: Some("first-land-fixture".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("initialize empty first-land repository");
    let sprint_card = root.join("docs/sprints/first_land.md");
    write_file(
        &sprint_card,
        concat!(
            "# First Land [plan-ref: first-land/root]\n\n",
            "## Work\n\n",
            "- [ ] Land the first file. [ref: first-land/write]\n",
        ),
    );

    let started = json_output(
        root,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/first_land.md#first-land/write",
            "--intent",
            "Prove the empty default Line first-land boundary",
            "--json",
        ],
    );
    let task_id = started["task_id"]
        .as_str()
        .expect("first-land Task ID")
        .to_string();
    let baseline_snapshot_id = started["worktree"]["head_snapshot_id"].clone();
    assert!(baseline_snapshot_id.is_string());
    assert!(started["change"].is_null());
    assert_eq!(
        started["worktree"]["fork_snapshot_id"],
        baseline_snapshot_id
    );
    let worktree = PathBuf::from(
        started["worktree"]["open_path"]
            .as_str()
            .or_else(|| started["worktree"]["path"].as_str())
            .expect("first-land worktree path"),
    );
    write_file(
        &worktree.join("first-land.txt"),
        "landed from first Snapshot\n",
    );
    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "First local Task Snapshot",
            "--json",
        ],
    );
    assert_eq!(snapshot["parent_snapshot_id"], baseline_snapshot_id);

    let landed = json_output(
        &worktree,
        &["task", "finish", task_id.as_str(), "--local", "--json"],
    );

    assert_eq!(landed["task_status"].as_str(), Some("completed"));
    assert_eq!(landed["closeout_status"].as_str(), Some("complete"));
    assert_eq!(
        landed["repo_root_restore"]["landed_diff_paths"],
        json!(["first-land.txt"])
    );
    assert_eq!(
        landed["repo_root_restore"]["plan"]["write_paths"],
        json!(["first-land.txt"])
    );
    assert_eq!(
        landed["plan_checklist_closeout"]["status"].as_str(),
        Some("synced")
    );
    assert_eq!(
        fs::read_to_string(root.join("first-land.txt")).expect("canonical first-land file"),
        "landed from first Snapshot\n"
    );
    assert!(fs::read_to_string(&sprint_card)
        .expect("closed first-land sprint card")
        .contains("- [x] Land the first file. [ref: first-land/write]"));
    assert!(
        !worktree.exists(),
        "completed first-land worktree must be removed"
    );
}

fn assert_closeout_mutated_once(
    state: &Arc<Mutex<CloseoutRecoveryRemoteState>>,
    fixture_seed: u64,
) {
    let guard = state.lock().unwrap();
    assert_eq!(
        guard.fixture_seed, fixture_seed,
        "fixture_seed={fixture_seed:#x}"
    );
    assert!(guard.land_submitted, "fixture_seed={fixture_seed:#x}");
    assert!(guard.task_completed, "fixture_seed={fixture_seed:#x}");
    assert!(
        guard.land_submit_attempts >= 1,
        "atomic Task Land must be attempted; fixture_seed={fixture_seed:#x}"
    );
    assert_eq!(
        guard.land_submit_mutations, 1,
        "land must mutate exactly once; fixture_seed={fixture_seed:#x}"
    );
    assert_eq!(
        guard.task_close_attempts, 1,
        "Task close must not be retried after authoritative recovery; fixture_seed={fixture_seed:#x}"
    );
    assert_eq!(
        guard.task_close_mutations, 1,
        "Task must mutate exactly once; fixture_seed={fixture_seed:#x}"
    );
}

#[test]
fn native_task_land_recovers_land_submit_from_authoritative_land_state() {
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    state.lock().unwrap().reset_iteration(
        0xA17_0001,
        CloseoutMutationBoundary::MutateBeforeResponse,
        Duration::from_millis(80),
        CloseoutMutationBoundary::MutateBeforeResponse,
        Duration::ZERO,
    );
    let temp = init_repo(&base_url);
    let root = temp.path();

    let payload = closeout_recovery_json(root, 0xA17_0001, "land-response-lost");

    assert_eq!(payload["apply_status"].as_str(), Some("done"));
    let land_result = action_result(&payload, "submit_land");
    assert_eq!(
        land_result["submission_id"].as_str(),
        Some("RT-1/L-01"),
        "{}",
        encode_json_pretty(land_result)
    );
    assert_eq!(land_result["status"].as_str(), Some("succeeded"));
    assert_eq!(payload["atomic_task_land"]["replayed"], true);
    assert_closeout_mutated_once(&state, 0xA17_0001);

    let retried = closeout_recovery_json(root, 0xA17_0001, "land-authoritative-retry");
    assert_eq!(retried["apply_status"].as_str(), Some("done"));
    assert_closeout_mutated_once(&state, 0xA17_0001);

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes/RC-1:submit"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST"
            && row.url == "/v1/native/repository-authorities/7/changes/RC-1/reviews"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "GET" && row.url == "/v1/native/repository-authorities/7/read/changes/RC-1"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "GET"
            && row
                .url
                .starts_with("/v1/native/repository-authorities/7/read/patchsets/RP-1/ci-status")
    }));
}

#[test]
fn native_task_land_resumes_retryable_busy_with_same_idempotency_key() {
    let fixture_seed = 0xA17_0004;
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    state.lock().unwrap().reset_iteration(
        fixture_seed,
        CloseoutMutationBoundary::RetryableBusyAfterMutation,
        Duration::ZERO,
        CloseoutMutationBoundary::MutateBeforeResponse,
        Duration::ZERO,
    );
    let temp = init_repo(&base_url);
    let root = temp.path();

    let payload = closeout_recovery_json(root, fixture_seed, "retryable-busy-after-mutation");

    assert_eq!(payload["apply_status"].as_str(), Some("done"));
    assert_eq!(payload["atomic_task_land"]["replayed"], true);
    assert_closeout_mutated_once(&state, fixture_seed);
    {
        let guard = state.lock().unwrap();
        assert!(guard.land_submit_attempts >= 2);
        assert!(guard.atomic_task_land_idempotency_key.is_some());
    }

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(
        logged
            .iter()
            .filter(|row| {
                row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
            })
            .count()
            >= 2
    );
}

#[test]
fn native_task_land_waits_for_timed_out_in_flight_mutation_then_replays() {
    let fixture_seed = 0xA17_0005;
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    state.lock().unwrap().reset_iteration(
        fixture_seed,
        CloseoutMutationBoundary::MutationInFlight,
        Duration::from_millis(70),
        CloseoutMutationBoundary::MutateBeforeResponse,
        Duration::ZERO,
    );
    let temp = init_repo(&base_url);
    let root = temp.path();
    let started = Instant::now();
    let output = command_output_with_env(
        root,
        &[
            "task", "finish", "RT-1", "--remote", "origin", "--json", "--full",
        ],
        &[
            ("AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS", "0.02"),
            // Preserve headroom above the 70 ms fixture mutation when the
            // integration suite schedules many mock servers in parallel.
            ("AIT_REMOTE_MUTATION_SETTLE_WINDOW_SECONDS", "0.20"),
            ("AIT_REMOTE_MUTATION_SETTLE_POLL_SECONDS", "0.002"),
        ],
    );
    let elapsed = started.elapsed();

    assert!(
        output.status.success(),
        "in-flight atomic Task Land recovery failed: fixture_seed={fixture_seed:#x} fixture_root={} elapsed={elapsed:?}\nstdout:\n{}\n\nstderr:\n{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = parse_json_bytes(&output.stdout);
    assert_eq!(payload["apply_status"].as_str(), Some("done"));
    assert_eq!(payload["atomic_task_land"]["replayed"], true);
    assert!(elapsed < Duration::from_secs(2));
    assert_closeout_mutated_once(&state, fixture_seed);
    {
        let guard = state.lock().unwrap();
        assert!(guard.land_submit_attempts > 2);
        assert!(!guard.land_mutation_in_flight);
        assert!(guard.atomic_task_land_idempotency_key.is_some());
    }

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(
        logged
            .iter()
            .filter(|row| {
                row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
            })
            .count()
            > 2
    );
}

#[test]
fn native_task_land_recovers_task_completion_from_authoritative_task_state() {
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    state.lock().unwrap().reset_iteration(
        0xA17_0002,
        CloseoutMutationBoundary::MutateBeforeResponse,
        Duration::ZERO,
        CloseoutMutationBoundary::MutateBeforeResponse,
        Duration::from_millis(80),
    );
    let temp = init_repo(&base_url);
    let root = temp.path();

    let payload = closeout_recovery_json(root, 0xA17_0002, "task-response-lost");

    assert_eq!(payload["apply_status"].as_str(), Some("done"));
    let complete_result = action_result(&payload, "complete_task");
    assert_eq!(complete_result["task_id"].as_str(), Some("RT-1"));
    assert_eq!(complete_result["status"].as_str(), Some("completed"));
    assert_eq!(payload["atomic_task_land"]["replayed"], true);
    assert_closeout_mutated_once(&state, 0xA17_0002);

    let retried = closeout_recovery_json(root, 0xA17_0002, "task-authoritative-retry");
    assert_eq!(retried["apply_status"].as_str(), Some("done"));
    assert_closeout_mutated_once(&state, 0xA17_0002);

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks/RT-1:close"
    }));
}

#[test]
fn native_task_land_timeout_before_remote_mutation_stays_failed_and_bounded() {
    let fixture_seed = 0xA17_0003;
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    state.lock().unwrap().reset_iteration(
        fixture_seed,
        CloseoutMutationBoundary::TimeoutBeforeMutation,
        Duration::from_millis(40),
        CloseoutMutationBoundary::MutateBeforeResponse,
        Duration::ZERO,
    );
    let temp = init_repo(&base_url);
    let root = temp.path();
    let started = Instant::now();
    let output = command_output_with_env(
        root,
        &[
            "task", "finish", "RT-1", "--remote", "origin", "--json", "--full",
        ],
        &[
            ("AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS", "0.01"),
            ("AIT_REMOTE_MUTATION_SETTLE_WINDOW_SECONDS", "0.05"),
            ("AIT_REMOTE_MUTATION_SETTLE_POLL_SECONDS", "0.002"),
        ],
    );
    let elapsed = started.elapsed();

    assert!(
        !output.status.success(),
        "timeout before mutation must not be reported as recovered; fixture_seed={fixture_seed:#x} fixture_root={} stdout={} stderr={}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "recovery exceeded its bounded monotonic deadline: elapsed={elapsed:?} fixture_seed={fixture_seed:#x}"
    );
    let guard = state.lock().unwrap();
    assert!(
        guard.land_submit_attempts >= 2,
        "fixture_seed={fixture_seed:#x}"
    );
    assert_eq!(
        guard.land_submit_mutations, 0,
        "fixture_seed={fixture_seed:#x}"
    );
    assert!(!guard.land_submitted, "fixture_seed={fixture_seed:#x}");
    assert_eq!(
        guard.task_close_attempts, 0,
        "fixture_seed={fixture_seed:#x}"
    );
    assert_eq!(
        guard.task_close_mutations, 0,
        "fixture_seed={fixture_seed:#x}"
    );
    assert!(!guard.task_completed, "fixture_seed={fixture_seed:#x}");
    drop(guard);
    handle.join().unwrap();
}

fn panic_payload_text(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    "non-string panic payload".to_string()
}

fn run_seeded_closeout_repetition(
    fixture_seed: u64,
    land_delay: Duration,
    task_delay: Duration,
) -> PathBuf {
    // Each iteration owns its server and state, including delayed POST workers.
    let (base_url, _log, state, handle) = spawn_closeout_iteration(
        fixture_seed,
        CloseoutMutationBoundary::MutateBeforeResponse,
        land_delay,
        task_delay,
    );
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let temp = init_repo(&base_url);
        let root = fs::canonicalize(temp.path()).unwrap();
        let started = Instant::now();
        let payload = closeout_recovery_json(&root, fixture_seed, "seeded-repetition");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "bounded recovery exceeded two seconds: elapsed={elapsed:?} fixture_seed={fixture_seed:#x} fixture_root={}",
            root.display()
        );
        assert_eq!(
            payload["atomic_task_land"]["replayed"],
            true,
            "fixture_seed={fixture_seed:#x} fixture_root={}",
            root.display()
        );
        assert_closeout_mutated_once(&state, fixture_seed);
        root
    }));
    handle.join().unwrap();
    match result {
        Ok(root) => root,
        Err(payload) => panic!(
            "seeded closeout recovery failed: fixture_seed={fixture_seed:#x} land_delay={land_delay:?} task_delay={task_delay:?}: {}",
            panic_payload_text(payload)
        ),
    }
}

fn deterministic_recovery_jitter(seed: u64, stream: u32) -> Duration {
    let mixed = seed
        .rotate_left(stream)
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    Duration::from_millis(25 + (mixed % 31))
}

#[test]
fn native_task_land_recovery_passes_fifty_deterministic_isolated_repetitions() {
    let mut fixture_roots = BTreeSet::new();
    for repetition in 0..50_u64 {
        let fixture_seed = 0xA17_D37E_0000_0000_u64.wrapping_add(repetition);
        let root = run_seeded_closeout_repetition(
            fixture_seed,
            Duration::from_millis(30),
            Duration::from_millis(30),
        );
        assert!(
            fixture_roots.insert(root.clone()),
            "fixture root reused: fixture_seed={fixture_seed:#x} fixture_root={}",
            root.display()
        );
        assert!(
            !root.exists(),
            "fixture root was not cleaned: fixture_seed={fixture_seed:#x} fixture_root={}",
            root.display()
        );
    }
    assert_eq!(fixture_roots.len(), 50);
}

#[test]
fn native_task_land_recovery_passes_thirty_seeded_bounded_jitter_repetitions() {
    let mut fixture_roots = BTreeSet::new();
    for repetition in 0..30_u64 {
        let fixture_seed = 0xA17_5177_0000_0000_u64.wrapping_add(repetition);
        let land_delay = deterministic_recovery_jitter(fixture_seed, 13);
        let task_delay = deterministic_recovery_jitter(fixture_seed, 37);
        let root = run_seeded_closeout_repetition(fixture_seed, land_delay, task_delay);
        assert!(
            fixture_roots.insert(root.clone()),
            "fixture root reused: fixture_seed={fixture_seed:#x} fixture_root={}",
            root.display()
        );
        assert!(
            !root.exists(),
            "fixture root was not cleaned: fixture_seed={fixture_seed:#x} fixture_root={}",
            root.display()
        );
    }
    assert_eq!(fixture_roots.len(), 30);
}

#[test]
fn native_closeout_fixture_isolates_delayed_response_and_in_flight_iterations() {
    fn atomic_reply(
        base_url: &str,
        request: &JsonValue,
    ) -> (u16, JsonValue) {
        use std::io::Read;
        let mut stream = std::net::TcpStream::connect(base_url.strip_prefix("http://").unwrap()).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
        let body = encode_json(request);
        write!(stream,
            "POST /v1/native/repository-authorities/7/task-land HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(), body).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let separator = response.windows(4).position(|part| part == b"\r\n\r\n").unwrap();
        let header = std::str::from_utf8(&response[..separator]).unwrap();
        let status = header.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, parse_json_bytes(&response[separator + 4..]))
    }

    for boundary in [
        CloseoutMutationBoundary::MutateBeforeResponse,
        CloseoutMutationBoundary::MutationInFlight,
    ] {
        let (base_a, _log_a, state_a, server_a) =
            spawn_closeout_iteration(0xa, boundary, Duration::ZERO, Duration::ZERO);
        let gate = Arc::new(CloseoutResponseGate::default());
        {
            let mut state = state_a.lock().unwrap();
            state.patchset_revision_snapshot_id = Some(FIXTURE_REVISION_SNAPSHOT_ID.into());
            state.response_gate = Some(Arc::clone(&gate));
        }
        let request_a = json!({
            "contract":"task-land-atomic/v2", "task_or_change_ref":"RT-1",
            "idempotency_key":"iteration-a"
        });
        let original = {
            let base = base_a.clone();
            let request = request_a.clone();
            thread::Builder::new()
                .name("delayed-iteration-0xa".into())
                .spawn(move || atomic_reply(&base, &request))
                .unwrap()
        };
        gate.wait_until_entered();
        let (status, replay) = atomic_reply(&base_a, &request_a);
        if boundary == CloseoutMutationBoundary::MutateBeforeResponse {
            assert_eq!(status, 200);
            assert_eq!(replay["replayed"], true);
            assert_closeout_mutated_once(&state_a, 0xa);
        } else {
            assert_eq!(status, 503);
            assert!(state_a.lock().unwrap().accepted.is_none());
        }

        // B starts while A's worker is still paused, using the same ownership
        // boundary as every deterministic/jitter iteration.
        let (base_b, _log_b, state_b, server_b) = spawn_closeout_iteration(
            0xb,
            CloseoutMutationBoundary::MutateBeforeResponse,
            Duration::ZERO,
            Duration::ZERO,
        );
        assert_ne!(base_a, base_b);
        assert!(!Arc::ptr_eq(&state_a, &state_b));
        {
            let mut state = state_b.lock().unwrap();
            state.patchset_revision_snapshot_id = Some(FIXTURE_FINISHED_SNAPSHOT_ID.into());
        }
        let request_b = json!({
            "contract":"task-land-atomic/v2", "task_or_change_ref":"RT-1",
            "idempotency_key":"iteration-b"
        });
        let (status, response_b) = atomic_reply(&base_b, &request_b);
        assert_eq!(status, 200);
        let accepted_b = state_b.lock().unwrap().accepted.clone();

        gate.release();
        let (status, response_a) = original.join().unwrap();
        assert_eq!(status, 200);
        assert_eq!(response_a["idempotency_key"], "iteration-a");
        assert_eq!(response_a["landed_snapshot_id"], FIXTURE_REVISION_SNAPSHOT_ID);
        assert_eq!(response_b["idempotency_key"], "iteration-b");
        assert_eq!(response_b["landed_snapshot_id"], FIXTURE_FINISHED_SNAPSHOT_ID);
        assert_eq!(state_b.lock().unwrap().accepted, accepted_b);
        assert_closeout_mutated_once(&state_a, 0xa);
        assert_closeout_mutated_once(&state_b, 0xb);
        let (status, replay) = atomic_reply(&base_a, &request_a);
        assert_eq!(status, 200);
        assert_eq!(replay["replayed"], true);
        assert_eq!(replay["land"], response_a["land"]);
        assert_closeout_mutated_once(&state_a, 0xa);
        server_a.join().unwrap();
        server_b.join().unwrap();
    }
}

#[test]
fn native_task_land_dirty_canonical_delta_preserves_head_bytes_and_worktree() {
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    let root_source = fs::read(root.join("src/lib.rs")).unwrap();
    let head = local_line_head(root, "main");
    state.lock().unwrap().patchset_revision_snapshot_id =
        Some(seed_snapshot(&worktree, "accepted Remote source"));
    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = task_land_apply(
        &repo, "RT-1", None, None::<fn(&JsonValue) -> Result<(), String>>,
    ).unwrap();
    assert_eq!(payload["apply_status"], "done");
    assert_eq!(payload["local_line_sync"]["status"], "failed");
    assert_eq!(payload["local_line_sync"]["reason"], "local_reconciliation_required");
    assert_eq!(payload["local_line_sync"]["detail"], "Canonical workspace has unsaved changes in the Remote delta.");
    assert_eq!(payload["bound_worktree_cleanup"]["status"], "deferred");
    assert_eq!(local_line_head(root, "main"), head);
    assert_eq!(fs::read(root.join("src/lib.rs")).unwrap(), root_source);
    assert!(worktree.exists());
    assert!(root.join(".ait/worktrees/rt-1.json").exists());
    assert_closeout_mutated_once(&state, 0);
    handle.join().unwrap();
}

#[test]
fn native_task_land_same_head_dirty_canonical_preserves_head_and_bytes() {
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    let head = local_line_head(root, "main");
    let source = fs::read(root.join("src/lib.rs")).unwrap();
    let untracked = root.join("ci/patch_ci.json");
    write_file(&untracked, "{}\n");
    let repo = RepoRuntime::discover_from_path(root).unwrap();
    let payload = task_land_apply(
        &repo, "RT-1", None, None::<fn(&JsonValue) -> Result<(), String>>,
    ).unwrap();
    assert_eq!(payload["apply_status"], "done");
    assert_eq!(payload["local_line_sync"]["status"], "failed");
    assert_eq!(payload["local_line_sync"]["same_head"], true);
    assert_eq!(payload["local_line_sync"]["reason"], "local_reconciliation_required");
    assert_eq!(payload["bound_worktree_cleanup"]["status"], "deferred");
    assert_eq!(local_line_head(root, "main"), head);
    assert_eq!(fs::read(root.join("src/lib.rs")).unwrap(), source);
    assert_eq!(fs::read_to_string(untracked).unwrap(), "{}\n");
    assert_closeout_mutated_once(&state, 0);
    handle.join().unwrap();
}

#[test]
fn native_task_land_current_worktree_skips_unrelated_backlog_refresh() {
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    restore_fixture_canonical_base(root);
    let patchset_revision_snapshot_id =
        seed_snapshot(&worktree, "task land clean current worktree");
    state.lock().unwrap().patchset_revision_snapshot_id = Some(patchset_revision_snapshot_id);
    let extra_metadata_path = root.join(".ait/worktrees/rt-extra.json");
    init_registered_worktree(
        root,
        "rt-extra",
        "feature/rt-extra",
        None,
        None,
        false,
        Some("manual_only"),
    );
    let before: JsonValue = parse_json_file(&extra_metadata_path);
    assert!(before.get("workspace_status_cache").is_none());

    let payload = json_output_with_env(
        &worktree,
        &["task", "finish", "RT-1", "--remote", "origin", "--json"],
        &[
            ("AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS", "0.05"),
            ("AIT_REMOTE_MUTATION_SETTLE_WINDOW_SECONDS", "1.0"),
            ("AIT_REMOTE_MUTATION_SETTLE_POLL_SECONDS", "0.01"),
        ],
    );

    assert_eq!(payload["apply_status"].as_str(), Some("done"));
    assert_eq!(
        action_result(&payload, "complete_task")["status"].as_str(),
        Some("completed")
    );
    assert!(state.lock().unwrap().task_completed);
    assert!(
        payload["automatic_reconciliation"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|finding| finding["code"].as_str() != Some("remote.inventory_unavailable")),
        "{}",
        encode_json_pretty(&payload["automatic_reconciliation"])
    );
    assert_eq!(
        payload["automatic_reconciliation"]["lease"]["path"].as_str(),
        Some(
            root.canonicalize()
                .unwrap()
                .join(".ait/reconciliation/v1/reconcile.lock")
                .to_string_lossy()
                .as_ref()
        )
    );
    assert_eq!(
        payload["automatic_reconciliation"]["sources"]["workspace_lock"]["metadata"]["repo_root"]
            .as_str(),
        Some(root.canonicalize().unwrap().to_string_lossy().as_ref())
    );

    let after: JsonValue = parse_json_file(&extra_metadata_path);
    assert!(after.get("workspace_status_cache").is_none());
    handle.join().unwrap();
}

#[test]
fn native_workflow_finish_apply_delegates_final_closeout_to_atomic_task_land() {
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    // Happy-path cleanup requires the canonical root to match its captured base.
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"ok\" }\n",
    );
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"root\" }\n",
    );
    let patchset_revision_snapshot_id = seed_snapshot(&worktree, "workflow finish clean worktree");
    state.lock().unwrap().patchset_revision_snapshot_id = Some(patchset_revision_snapshot_id);
    let backlog_path = init_registered_worktree(
        root,
        "rt-backlog",
        "feature/rt-backlog",
        Some("RT-BACKLOG"),
        Some("RC-BACKLOG"),
        true,
        Some("after_remote_land"),
    );
    let backlog_metadata_path = root.join(".ait/worktrees/rt-backlog.json");
    let before: JsonValue = parse_json_file(&backlog_metadata_path);
    assert!(before.get("workspace_status_cache").is_none());

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = workflow_land_apply(
        &repo,
        "RT-1",
        Some("Reviewed files: src/lib.rs; Findings: no blocking findings; Risks: low; Tests: cargo test passed; Recommendation: land."),
        None,
        None::<fn(&JsonValue) -> Result<(), String>>,
    )
    .unwrap();

    assert_eq!(
        payload["apply_status"].as_str(),
        Some("done"),
        "{}",
        encode_json_pretty(&payload)
    );
    assert!(state.lock().unwrap().land_submitted);
    assert!(state.lock().unwrap().task_completed);
    assert_eq!(
        payload["reviewer_workflow"]["contract"].as_str(),
        Some("workflow-land-reviewer-atomic-closeout/v1")
    );
    assert_eq!(
        payload["reviewer_workflow"]["finalizer"].as_str(),
        Some("task-land-atomic/v2")
    );
    assert_eq!(
        payload["atomic_task_land"]["remote_mutation_count"].as_u64(),
        Some(1)
    );
    let cleanup = &payload["bound_worktree_cleanup"];
    assert_eq!(cleanup["status"].as_str(), Some("removed"));
    assert!(matches!(
        cleanup["reason"].as_str(),
        Some("promoted_to_cli_main_seed" | "task_land_force_close")
    ));
    assert!(backlog_path.exists());
    let after: JsonValue = parse_json_file(&backlog_metadata_path);
    assert!(after.get("workspace_status_cache").is_none());
    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert_eq!(
        logged
            .iter()
            .filter(|row| {
                row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
            })
            .count(),
        1
    );
    assert!(!logged.iter().any(|row| {
        row.method == "POST"
            && matches!(
                row.url.as_str(),
                "/v1/native/repository-authorities/7/changes/RC-1:submit"
                    | "/v1/native/repository-authorities/7/tasks/RT-1:close"
            )
    }));
}

#[test]
fn native_workflow_finish_cli_uses_default_remote_with_local_defaults_before_atomic_task_land() {
    assert_workflow_finish_cli_preserves_review_gates("RT-1/C-01");
}

#[test]
fn native_workflow_finish_task_input_preserves_exact_review_policy_and_atomic_closeout() {
    assert_workflow_finish_cli_preserves_review_gates("RT-1");
}

#[test]
fn guarded_remote_finish_recovers_plan_sync_with_original_apply_receipt() {
    assert_guarded_remote_finish_recovers_plan_sync(false, false);
}

#[test]
fn guarded_ai_only_team_finish_records_or_reuses_code_review_before_apply() {
    for existing_summary in [false, true] {
        assert_guarded_remote_finish_recovers_plan_sync(true, existing_summary);
    }
}

fn assert_guarded_remote_finish_recovers_plan_sync(teams: bool, existing_summary: bool) {
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    write_file(&root.join("src/lib.rs"), "pub fn example() -> &'static str { \"ok\" }\n");
    let config_path = root.join(".ait/config.json");
    let mut config = parse_json_file(&config_path);
    config["agent_teams"] = json!(teams);
    if teams {
        config["sprint"] = json!("on");
        bind_fixture_reviewers(root);
    }
    write_file(&config_path, &encode_json_pretty(&config));
    let artifact = "docs/sprints/remote-closeout.md";
    let original = "# Finish [plan-ref: finish/root]\n\n- [ ] Ship [ref: finish/ship]\n- [ ] Other [ref: finish/other]\n";
    write_file(&root.join(artifact), original);
    let synced = json_output(root, &["plan", "sync", artifact, "--local", "--json"]);
    let plan_id = synced["results"][0]["plan_id"].as_str().unwrap();
    let mut plan = json_output(root, &["plan", "show", plan_id, "--local", "--json"]);
    for item in plan["head_revision"]["items"].as_array_mut().unwrap() {
        item.as_object_mut().unwrap().remove("taskable_hint");
    }
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"reviewed\" }\n",
    );
    let revision_id = seed_snapshot(&worktree, "reviewed closeout candidate");
    {
        let mut state = state.lock().unwrap();
        state.plan_closeout = Some(CloseoutPlanFixture {
            revisions: vec![plan["head_revision"].clone()],
            plan,
            fail_sync: true,
        });
        state.patchset_revision_snapshot_id = Some(revision_id.clone());
        state.guarded_base_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        state.enforce_reviewer_workflow = !teams;
        state.ai_only_candidate = teams;
        state.code_review_recorded = existing_summary;
        state.task_review_recorded = false;
        state.policy_evaluated = false;
    }
    let frozen = TempDir::new().unwrap();
    let expected_path = frozen.path().join("expected.json");
    let expected = json!({
        "repository_index":FIXTURE_REPOSITORY_INDEX,
        "patchset_id":"RT-1/P-01",
        "revision_snapshot_id":revision_id,
        "base_snapshot_id":FIXTURE_BASE_SNAPSHOT_ID
    });
    write_file(&expected_path, &encode_json_pretty(&expected));
    let environment = if teams {
        vec![("CODEX_THREAD_ID", "einstein-session"), ("CODEX_SESSION_ID", "einstein-session")]
    } else { Vec::new() };
    let guarded_finish = |root: &Path, identity: &Path| {
        command_output_with_env(root, &[
        "workflow", "finish", "RT-1", "--apply", "--remote", "origin",
        "--expected-identity", identity.to_str().unwrap(),
        "--reviewer", "Einstein", "--review-message", "Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: cargo test passed; Recommendation: finish.",
        "--json", "--full"
    ], &environment)
    };
    for (field, value) in [
        ("repository_index", json!(8)),
        ("patchset_id", json!("RT-1/P-02")),
        ("revision_snapshot_id", json!("SNP-000000000098")),
        ("base_snapshot_id", json!("SNP-000000000099")),
    ] {
        let mut mismatch = expected.clone();
        mismatch[field] = value;
        let path = frozen.path().join(format!("wrong-{field}.json"));
        write_file(&path, &encode_json_pretty(&mismatch));
        let rejected = guarded_finish(&worktree, &path);
        assert!(!rejected.status.success(), "{field}");
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("EXPECTED_CANDIDATE_MISMATCH"),
            "{field}: {}",
            String::from_utf8_lossy(&rejected.stderr)
        );
    }
    assert!(log.lock().unwrap().iter().all(|r| r.method == "GET"));
    // The final locked authority may change after client preflight has passed.
    state.lock().unwrap().guarded_base_snapshot_id = Some("SNP-000000000099".to_string());
    let rejected = guarded_finish(&worktree, &expected_path);
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("EXPECTED_CANDIDATE_MISMATCH"),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    {
        let mut state = state.lock().unwrap();
        assert_eq!(state.land_submit_mutations, 0);
        assert_eq!(state.task_close_mutations, 0);
        assert_eq!(state.plan_closeout.as_ref().unwrap().revisions.len(), 1);
        state.guarded_base_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        state.recorded_reviews.clear();
        state.code_review_recorded = existing_summary;
        state.task_review_recorded = false;
        state.policy_evaluated = false;
    }
    assert_eq!(fs::read_to_string(root.join(artifact)).unwrap(), original);
    let successful_attempt_start = log.lock().unwrap().len();
    let first_output = guarded_finish(&worktree, &expected_path);
    assert_eq!(
        first_output.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&first_output.stdout),
        String::from_utf8_lossy(&first_output.stderr)
    );
    let first = parse_json_bytes(&first_output.stdout);
    assert_eq!(first["apply_status"], "done", "{first}");
    assert_eq!(
        first["atomic_task_land"]["contract"],
        ait_core::workflow_expected_identity::GUARDED_TASK_LAND_CONTRACT
    );
    assert_eq!(
        first["reviewer_workflow"]["finalizer"],
        ait_core::workflow_expected_identity::GUARDED_TASK_LAND_CONTRACT
    );
    assert_eq!(first["closeout_status"], "partial", "{first}");
    let recovery_command = first["closeout_recovery"]["command"].as_str().unwrap();
    assert!(recovery_command.starts_with("ait workflow finish "));
    assert!(recovery_command.contains("--expected-identity"));
    assert!(recovery_command.contains(expected_path.to_str().unwrap()));
    assert!(recovery_command.contains("--remote origin"));
    assert!(
        first["plan_checklist_closeout"]["error"]
            .as_str()
            .unwrap()
            .contains("injected Plan sync failure"),
        "{first}"
    );
    assert_eq!(first["landed_snapshot_id"], revision_id);
    assert_eq!(fs::read_to_string(root.join(artifact)).unwrap(), original);
    assert_closeout_mutated_once(&state, 0);
    let receipt = state
        .lock()
        .unwrap()
        .atomic_task_land_idempotency_key
        .clone();
    let before = json_output(root, &["snapshot", "list", "--all", "--json"]);
    for field in [
        "repository_index",
        "patchset_id",
        "revision_snapshot_id",
        "base_snapshot_id",
    ] {
        let rejected = guarded_finish(root, &frozen.path().join(format!("wrong-{field}.json")));
        assert!(
            !rejected.status.success(),
            "receipt recovery must preserve {field}"
        );
        assert!(
            String::from_utf8_lossy(&rejected.stderr).contains("EXPECTED_CANDIDATE_MISMATCH"),
            "{field}: {}",
            String::from_utf8_lossy(&rejected.stderr)
        );
    }
    assert_closeout_mutated_once(&state, 0);
    assert_eq!(fs::read_to_string(root.join(artifact)).unwrap(), original);
    state
        .lock()
        .unwrap()
        .plan_closeout
        .as_mut()
        .unwrap()
        .fail_sync = false;
    let recovery_output = guarded_finish(root, &expected_path);
    assert!(
        recovery_output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&recovery_output.stdout),
        String::from_utf8_lossy(&recovery_output.stderr)
    );
    let recovered = parse_json_bytes(&recovery_output.stdout);
    assert_eq!(recovered["apply_status"], "done", "{recovered}");
    assert_eq!(recovered["closeout_status"], "complete", "{recovered}");
    assert_eq!(recovered["plan_checklist_closeout"]["status"], "synced");
    assert_eq!(recovered["atomic_task_land"]["replayed"], true);
    assert_eq!(recovered["landed_snapshot_id"], first["landed_snapshot_id"]);
    assert_eq!(
        state.lock().unwrap().atomic_task_land_idempotency_key,
        receipt
    );
    assert_closeout_mutated_once(&state, 0);
    assert_eq!(
        json_output(root, &["snapshot", "list", "--all", "--json"]),
        before
    );
    let mut expected_markdown = original.replacen("- [ ] Ship", "- [x] Ship", 1);
    if teams {
        assert_eq!(recovered["team_finish_log"]["status"], "synced", "{recovered}");
        let review_id = recovered["team_finish_log"]["review_id"].as_str().unwrap();
        expected_markdown.push_str(&format!("\n- Reviewed and remote finished: RT-1/P-01 | ref: finish/ship | review: {review_id}\n"));
    }
    assert_eq!(fs::read_to_string(root.join(artifact)).unwrap(), expected_markdown);
    let state = state.lock().unwrap();
    assert_eq!(state.plan_closeout.as_ref().unwrap().revisions.len(), if teams {5} else {4},
        "original, failed closure, restored local Plan, successful closure remain in native history");
    drop(state);
    let requests = log.lock().unwrap();
    let apply_index = successful_attempt_start + requests[successful_attempt_start..]
        .iter()
        .position(|r| r.url.ends_with("/task-land"))
        .unwrap();
    for action in ["code_review_summary", "task_approve"] {
        let expected = if action == "code_review_summary" && existing_summary {0} else {1};
        let matching = |r: &&RecordedRequest| r.method == "POST" && r.url.ends_with("/reviews") && r.body.contains(action);
        assert_eq!(requests[successful_attempt_start..apply_index].iter().filter(matching).count(), expected, "{action}");
        assert_eq!(requests[successful_attempt_start..].iter().filter(matching).count(), expected, "receipt retry must reuse {action}");
    }
    assert!(requests[..apply_index]
        .iter()
        .any(|r| r.url.ends_with(":evaluatePolicy")));
    let atomic = requests
        .iter()
        .filter(|r| r.method == "POST" && r.url.ends_with("/task-land"))
        .collect::<Vec<_>>();
    assert!(atomic.len() >= 3, "mismatched authority, original apply and receipt recovery must reach the atomic entrypoint");
    for request in atomic {
        let body = parse_json(&request.body);
        assert_eq!(
            body["contract"],
            ait_core::workflow_expected_identity::GUARDED_TASK_LAND_CONTRACT
        );
        assert_eq!(body["expected_identity"], expected);
    }
    assert_eq!(parse_json_file(&expected_path), expected);
    drop(requests);
    handle.join().unwrap();
}

fn assert_workflow_finish_cli_preserves_review_gates(reference: &str) {
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    restore_fixture_canonical_base(temp.path());
    let config_path = temp.path().join(".ait/config.json");
    let mut config: JsonValue = parse_json_file(&config_path);
    config["workflow_default_scope"] = json!("local");
    config["task_default_scope"] = json!("local");
    config["change_default_scope"] = json!("local");
    write_file(&config_path, &(encode_json_pretty(&config) + "\n"));
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"reviewed\" }\n",
    );
    let patchset_revision_snapshot_id = seed_snapshot(&worktree, "reviewer workflow finish");
    {
        let mut guard = state.lock().unwrap();
        guard.revision_change = reference.contains("/C-");
        guard.patchset_revision_snapshot_id = Some(patchset_revision_snapshot_id);
        guard.enforce_reviewer_workflow = true;
        guard.code_review_recorded = false;
        guard.task_review_recorded = false;
        guard.policy_evaluated = false;
    }

    let output = command_output_with_env(
        &worktree,
        &[
            "workflow",
            "finish",
            reference,
            "--apply",
            "--reviewer", "Einstein", "--review-message",
            "Reviewed files: src/lib.rs; Findings: no blocking findings; Risks: low; Tests: cargo test passed; Recommendation: land.",
        ],
        &[],
    );
    assert!(
        output.status.success(),
        "workflow finish failed\nstdout:\n{}\n\nstderr:\n{}\nrequests:\n{:#?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        log.lock().unwrap().clone()
    );
    assert!(state.lock().unwrap().land_submitted);
    assert!(state.lock().unwrap().task_completed);

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let code_review_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url
                    == if reference.contains("/C-") {
                        "/v1/native/repository-authorities/7/changes/RT-1%2FC-01/reviews"
                    } else {
                        "/v1/native/repository-authorities/7/changes/RT-1/reviews"
                    }
                && row.body.contains("\"action\":\"code_review_summary\"")
        })
        .expect("workflow finish must record exact-Patchset code review");
    let task_review_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url
                    == if reference.contains("/C-") {
                        "/v1/native/repository-authorities/7/changes/RT-1%2FC-01/reviews"
                    } else {
                        "/v1/native/repository-authorities/7/changes/RT-1/reviews"
                    }
                && row.body.contains("\"action\":\"task_approve\"")
        })
        .expect("workflow finish must record configured Task approval");
    let policy_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url == if reference.contains("/C-") {"/v1/native/repository-authorities/7/patchsets/RT-1%2FC-01%2FP-01:evaluatePolicy"} else {"/v1/native/repository-authorities/7/patchsets/RT-1%2FP-01:evaluatePolicy"}
        })
        .expect("workflow finish must evaluate final Policy");
    let atomic_land_index = logged
        .iter()
        .position(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
        })
        .expect("workflow finish must delegate to atomic Task Land");
    for index in [code_review_index, task_review_index] {
        let review = parse_json(&logged[index].body);
        assert_eq!(review["reviewer"], "Einstein");
        assert_eq!(review["reviewer_kind"], "bot");
    }
    assert!(code_review_index < task_review_index);
    assert!(task_review_index < policy_index);
    assert!(policy_index < atomic_land_index);
}

#[test]
fn native_task_audit_remote_task_reference_does_not_probe_remote_change() {
    let (base_url, log, _state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let config_path = temp.path().join(".ait/config.json");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace(
            "\"workflow_default_scope\": \"remote\"",
            "\"workflow_default_scope\": \"local\"",
        )
        .replace(
            "\"task_default_scope\": \"remote\"",
            "\"task_default_scope\": \"local\"",
        );
    write_file(&config_path, &config);
    let payload = json_output(
        &worktree,
        &["task", "audit", "RT-1", "--remote", "origin", "--json"],
    );

    assert_eq!(
        payload["changes"][0]["change"]["change_id"].as_str(),
        Some("RC-1"),
        "{}",
        encode_json_pretty(&payload)
    );
    assert_eq!(payload["task"]["task_id"].as_str(), Some("RT-1"));
    assert!(log.lock().unwrap().iter().all(|row| {
        !(row.method == "GET" && row.url == "/v1/native/repository-authorities/7/changes/RT-1")
    }));
    drop(temp);
    handle.join().unwrap();
}

#[test]
fn native_task_audit_cli_explicit_remote_reads_shared_task_for_unpublished_local_draft() {
    let (base_url, log, _state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_local_draft_worktree_repo(&base_url);

    let payload = json_output(
        &worktree,
        &["task", "audit", "RT-1", "--remote", "origin", "--json"],
    );

    assert_eq!(
        payload["changes"][0]["change"]["change_id"].as_str(),
        Some("RC-1")
    );
    assert_eq!(payload["task"]["task_id"].as_str(), Some("RT-1"));
    assert!(log.lock().unwrap().iter().any(|row| {
        row.method == "GET"
            && row.url.starts_with(
                "/v1/native/repository-authorities/7/read/tasks/RT-1/audit?target_line=main",
            )
    }));
    drop(temp);
    handle.join().unwrap();
}

#[test]
fn native_task_audit_cli_local_override_reads_local_draft() {
    let (_temp, worktree, _started) = init_cli_local_draft_worktree_repo("http://127.0.0.1:1");

    let payload = json_output(
        &worktree,
        &["task", "audit", "LT-0001", "--local", "--json"],
    );

    assert_eq!(payload["audit_source"]["mode"].as_str(), Some("local"));
    assert_eq!(payload["task"]["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(payload["changes"], json!([]));
}

#[test]
fn native_task_audit_cli_local_default_does_not_fallback_to_remote() {
    let (temp, worktree, _started) = init_cli_local_draft_worktree_repo("http://127.0.0.1:1");
    let config_path = temp.path().join(".ait/config.json");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace(
            "\"workflow_default_scope\": \"remote\"",
            "\"workflow_default_scope\": \"local\"",
        )
        .replace(
            "\"task_default_scope\": \"remote\"",
            "\"task_default_scope\": \"local\"",
        );
    write_file(&config_path, &config);

    let payload = json_output(&worktree, &["task", "audit", "LT-0001", "--json"]);

    assert_eq!(payload["audit_source"]["mode"].as_str(), Some("local"));
    assert_eq!(payload["task"]["task_id"].as_str(), Some("LT-0001"));
}

#[test]
fn native_task_land_unscoped_api_does_not_fallback_to_local_authority() {
    let (_temp, worktree) = init_local_draft_worktree_repo("http://127.0.0.1:1");

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let error = task_land_payload(&repo, "RT-1", None).unwrap_err();
    assert!(error.contains("Task `RT-1`"), "{error}");
}

#[test]
fn native_local_task_land_from_root_routes_to_exact_bound_worktree() {
    let (temp, worktree, started) = init_cli_local_draft_worktree_repo("http://127.0.0.1:1");
    let root = temp.path();
    let source = "pub fn example() -> &'static str { \"guarded local land\" }\n";
    write_file(&worktree.join("src/lib.rs"), source);
    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "guarded local land snapshot",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    let unrelated = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Unrelated active Task",
            "--intent",
            "Keep this Task open during exact routing",
            "--json",
        ],
    );
    assert_eq!(unrelated["task_id"], "LT-0002");
    let unrelated_worktree = PathBuf::from(unrelated["worktree"]["path"].as_str().unwrap());
    set_active_root_worktree(root, "lt-0002");

    let main_before = json_output(root, &["line", "show", "main", "--json"]);
    assert_eq!(
        main_before["head_snapshot_id"].as_str(),
        started["worktree"]["fork_snapshot_id"].as_str()
    );

    let landed = json_output(root, &["task", "finish", "LT-0001", "--local", "--json"]);

    assert_eq!(landed["task_status"].as_str(), Some("completed"));
    assert!(landed["change_status"].is_null());
    assert_eq!(
        landed["landed_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), source);
    assert!(!worktree.exists());
    assert!(unrelated_worktree.exists());
}

#[test]
fn native_local_task_land_from_root_rejects_root_authoring_drift_before_mutation() {
    let (temp, worktree, started) = init_cli_local_draft_worktree_repo("http://127.0.0.1:1");
    let root = temp.path();
    let worktree_source = "pub fn example() -> &'static str { \"target worktree only\" }\n";
    write_file(&worktree.join("src/lib.rs"), worktree_source);
    write_file(
        &root.join("src/root_only.rs"),
        "pub fn must_not_land() {}\n",
    );

    let output = command_output_with_env(
        root,
        &[
            "task",
            "finish",
            "LT-0001",
            "--local",
            "--message",
            "must not snapshot root drift",
            "--json",
        ],
        &[],
    );

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("code/workspace drift"));
    assert!(stderr.contains("src/root_only.rs"));
    assert_eq!(
        json_output(root, &["task", "show", "LT-0001", "--local", "--json"])["status"].as_str(),
        Some("active")
    );
    assert_eq!(
        json_output(root, &["change", "list", "--local", "--json"]),
        json!([])
    );
    assert_eq!(
        json_output(root, &["line", "show", "main", "--json"])["head_snapshot_id"],
        started["worktree"]["fork_snapshot_id"]
    );
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        worktree_source
    );
    assert!(worktree.exists());
}

#[test]
fn native_task_land_local_apply_lands_snapshot_and_cleans_worktree() {
    let (temp, worktree, _started) = init_cli_local_draft_worktree_repo("http://127.0.0.1:1");
    let metadata_path = temp.path().join(".ait/worktrees/lt-0001.json");
    let source = "pub fn example() -> &'static str { \"local landed\" }\n";
    write_file(&worktree.join("src/lib.rs"), source);
    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "local land integration snapshot",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();

    let payload = json_output(
        &worktree,
        &["task", "finish", "LT-0001", "--local", "--json"],
    );

    assert_eq!(payload["mode"].as_str(), Some("local"));
    assert_eq!(payload["apply_status"].as_str(), Some("done"));
    assert_eq!(payload["task_id"].as_str(), Some("LT-0001"));
    assert!(payload["change_id"].is_null());
    assert!(payload["change_ref"].is_null());
    assert_eq!(
        payload["landed_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(payload["task_status"].as_str(), Some("completed"));
    assert!(payload["change_status"].is_null());
    assert!(payload["auto_snapshot"].is_null());
    assert_eq!(
        fs::read_to_string(temp.path().join("src/lib.rs")).unwrap(),
        source
    );
    assert!(!metadata_path.exists());
    assert!(!worktree.exists());
}

#[test]
fn native_task_land_apply_force_removes_current_worktree_after_completion() {
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    restore_fixture_canonical_base(root);
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"root\" }\n",
    );
    let patchset_revision_snapshot_id = seed_snapshot(&worktree, "task land clean worktree");
    state.lock().unwrap().patchset_revision_snapshot_id = Some(patchset_revision_snapshot_id);
    let metadata_path = root.join(".ait/worktrees/rt-1.json");
    assert!(metadata_path.exists());

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = task_land_apply(
        &repo,
        "RT-1",
        None,
        None::<fn(&JsonValue) -> Result<(), String>>,
    )
    .unwrap();

    assert_eq!(
        payload["apply_status"].as_str(),
        Some("done"),
        "{}",
        encode_json_pretty(&payload)
    );
    assert!(state.lock().unwrap().land_submitted);
    assert!(state.lock().unwrap().task_completed);
    assert!(payload["workspace"]["clean"].is_null());
    assert_eq!(payload["workspace"]["evaluation"].as_str(), Some("skipped"));
    assert_eq!(
        payload["workspace"]["reason"].as_str(),
        Some("ready_patchset_is_authoritative")
    );
    assert_eq!(
        payload["workspace"]["read_scope"].as_str(),
        Some("line_and_bound_worktree_metadata_only")
    );
    let cleanup = &payload["bound_worktree_cleanup"];
    assert_eq!(cleanup["status"].as_str(), Some("removed"));
    assert_eq!(cleanup["reason"].as_str(), Some("task_land_force_close"));
    assert_eq!(cleanup["worktree"]["name"].as_str(), Some("rt-1"));
    assert_eq!(
        cleanup["worktree"]["workspace_status"].as_str(),
        Some("not_evaluated")
    );
    assert_eq!(
        cleanup["worktree"]["workspace_status_evaluation"].as_str(),
        Some("skipped")
    );
    assert_eq!(
        cleanup["worktree"]["workspace_status_reason"].as_str(),
        Some("ready_remote_task_land_is_authoritative")
    );
    assert_eq!(
        cleanup["worktree"]["workspace_read_scope"].as_str(),
        Some("bound_worktree_metadata_only")
    );
    assert!(!metadata_path.exists());
    assert!(!worktree.exists());

    let actions = payload["applied_actions"].as_array().unwrap();
    let complete_cleanup = actions
        .iter()
        .find(|row| row["code"].as_str() == Some("complete_task"))
        .and_then(|row| row.get("result"))
        .and_then(|result| result.get("bound_worktree_cleanup"))
        .unwrap();
    assert_eq!(complete_cleanup["status"].as_str(), Some("removed"));
    assert_eq!(
        complete_cleanup["reason"].as_str(),
        Some("task_land_force_close")
    );
    handle.join().unwrap();
}

#[test]
fn native_task_land_promotes_clean_completed_worktree_to_cli_main_seed() {
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    restore_fixture_canonical_base(root);
    let client_runtime_root = root.join("client-runtime");
    let config_path = root.join(".ait/config.json");
    let mut config = parse_json_file(&config_path);
    config
        .as_object_mut()
        .expect("fixture config must be an object")
        .insert(
            "task_worktree".to_string(),
            json!({
                "ephemeral_root": client_runtime_root.to_string_lossy().to_string(),
            }),
        );
    write_file(&config_path, &(encode_json_pretty(&config) + "\n"));
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"root\" }\n",
    );
    let patchset_revision_snapshot_id = seed_snapshot(&worktree, "task land main-seed promotion");
    state.lock().unwrap().patchset_revision_snapshot_id =
        Some(patchset_revision_snapshot_id.clone());

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = task_land_apply(
        &repo,
        "RT-1",
        None,
        None::<fn(&JsonValue) -> Result<(), String>>,
    )
    .unwrap();

    assert_eq!(
        payload["apply_status"].as_str(),
        Some("done"),
        "{}",
        encode_json_pretty(&payload)
    );
    assert_eq!(
        payload["main_seed_sync"]["status"].as_str(),
        Some("promoted")
    );
    assert_eq!(
        payload["main_seed_sync"]["refresh_strategy"].as_str(),
        Some("validated_task_worktree_atomic_promotion")
    );
    assert_eq!(
        payload["main_seed_sync"]["seed_snapshot_id"].as_str(),
        Some(patchset_revision_snapshot_id.as_str())
    );
    assert_eq!(
        payload["bound_worktree_cleanup"]["reason"].as_str(),
        Some("already_removed_by_workflow_land")
    );
    assert_eq!(
        payload["bound_worktree_cleanup"]["worktree"]["reason"].as_str(),
        Some("promoted_to_cli_main_seed")
    );
    let seed_path = PathBuf::from(payload["main_seed_sync"]["path"].as_str().unwrap());
    let _seed_cleanup = WritableTreeOnDrop::new(seed_path.clone());
    let canonical_seed_path = seed_path.canonicalize().unwrap();
    let canonical_client_runtime_root = client_runtime_root.canonicalize().unwrap();
    assert_eq!(
        payload["main_seed_sync"]["path"].as_str(),
        Some(canonical_seed_path.to_string_lossy().as_ref())
    );
    assert!(canonical_seed_path.starts_with(&canonical_client_runtime_root));
    assert!(canonical_seed_path.ends_with("fixture-ait/.ait-internal/main-seed"));
    assert!(seed_path.is_dir());
    assert_eq!(
        fs::read_to_string(seed_path.join("src/lib.rs")).unwrap(),
        "pub fn repo_root_version() -> &'static str { \"root\" }\n"
    );
    assert!(!root.join(".ait/worktrees/rt-1.json").exists());
    assert!(!worktree.exists());
    assert!(
        payload["main_seed_sync"]["phase_timings_ms"]["land_seed_sync_total"]
            .as_f64()
            .is_some()
    );

    handle.join().unwrap();
}

#[test]
fn native_task_land_missing_patchset_fails_fast_without_publish_sync_or_ci() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    state.lock().unwrap().force_no_selected_patchset = true;

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let started = Instant::now();
    let error = task_land_apply(
        &repo,
        "RT-1",
        None,
        None::<fn(&JsonValue) -> Result<(), String>>,
    )
    .expect_err("task land must require an existing ready patchset");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "task land precondition took {:?}",
        started.elapsed()
    );
    assert!(error.contains("requires an existing selected remote patchset"));
    assert!(error.contains("currently has no selected patchset"));
    assert!(error.contains("does not publish or synchronize content"));
    assert!(error.contains("does not start or wait for CI"));

    drop(temp);
    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().all(|row| {
        !(row.method == "POST"
            && (row.url.contains("/patchsets")
                || row.url.contains(":runCi")
                || row.url.contains("/ci")))
    }));
}
