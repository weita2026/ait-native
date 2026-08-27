#[test]
fn native_patchset_publish_recovers_from_broken_change_read_and_response_path() {
    let (base_url, log, state, handle) = spawn_publish_recovery_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"recovery\" }\n",
    );
    let snapshot = json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "recovery snapshot",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();

    let patchset = json_output(
        root,
        &[
            "patchset",
            "publish",
            "RC-1",
            "--summary",
            "Recovery publish",
            "--json",
        ],
    );

    assert_eq!(patchset["patchset"]["patchset_id"].as_str(), Some("RP-REC"));
    assert_eq!(
        patchset["patchset"]["revision_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(
        patchset["patchset"]["response_recovery"]["state"].as_str(),
        Some("recovered_from_remote_publish")
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(
        logged
            .iter()
            .any(|row| row.method == "GET"
                && row.url == "/v1/native/repository-authorities/7/changes")
    );
    assert!(logged.iter().any(|row| {
        row.method == "GET"
            && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets"
    }));
}

const CLOSEOUT_RECOVERY_ENV: [(&str, &str); 3] = [
    ("AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS", "0.01"),
    ("AIT_REMOTE_MUTATION_SETTLE_WINDOW_SECONDS", "0.25"),
    ("AIT_REMOTE_MUTATION_SETTLE_POLL_SECONDS", "0.002"),
];

fn closeout_recovery_json(root: &Path, fixture_seed: u64, phase: &str) -> JsonValue {
    let output = command_output_with_env(
        root,
        &["task", "finish", "RT-1", "--json", "--full"],
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
fn native_first_local_task_land_materializes_empty_default_line() {
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
    let worktree = PathBuf::from(
        started["worktree"]["open_path"]
            .as_str()
            .or_else(|| started["worktree"]["path"].as_str())
            .expect("first-land worktree path"),
    );
    write_file(&worktree.join("first-land.txt"), "landed from first Snapshot\n");
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
    assert_eq!(snapshot["parent_snapshot_id"], JsonValue::Null);

    let landed = json_output(
        &worktree,
        &[
            "task",
            "finish",
            task_id.as_str(),
            "--local",
            "--json",
        ],
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
    assert!(
        fs::read_to_string(&sprint_card)
            .expect("closed first-land sprint card")
            .contains("- [x] Land the first file. [ref: first-land/write]")
    );
    assert!(!worktree.exists(), "completed first-land worktree must be removed");
}

fn assert_closeout_mutated_once(
    state: &Arc<Mutex<CloseoutRecoveryRemoteState>>,
    fixture_seed: u64,
) {
    let guard = state.lock().unwrap();
    assert_eq!(guard.fixture_seed, fixture_seed, "fixture_seed={fixture_seed:#x}");
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
    write_file(&root.join("ci/patch_ci.json"), "{}\n");

    let payload = closeout_recovery_json(root, 0xA17_0001, "land-response-lost");

    assert_eq!(payload["apply_status"].as_str(), Some("done"));
    let land_result = action_result(&payload, "submit_land");
    assert_eq!(
        land_result["submission_id"].as_str(),
        Some("LAND-REC"),
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
        row.method == "GET"
            && row.url == "/v1/native/repository-authorities/7/read/changes/RC-1"
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
        assert_eq!(guard.land_submit_attempts, 2);
        assert!(guard.atomic_task_land_idempotency_key.is_some());
    }

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert_eq!(
        logged
            .iter()
            .filter(|row| {
                row.method == "POST"
                    && row.url == "/v1/native/repository-authorities/7/task-land"
            })
            .count(),
        2
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
        &["task", "finish", "RT-1", "--json", "--full"],
        &[
            ("AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS", "0.02"),
            ("AIT_REMOTE_MUTATION_SETTLE_WINDOW_SECONDS", "0.04"),
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
                row.method == "POST"
                    && row.url == "/v1/native/repository-authorities/7/task-land"
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
    assert!(!logged
        .iter()
        .any(|row| { row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks/RT-1:close" }));
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
        &["task", "finish", "RT-1", "--json", "--full"],
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
    assert_eq!(guard.land_submit_attempts, 2, "fixture_seed={fixture_seed:#x}");
    assert_eq!(guard.land_submit_mutations, 0, "fixture_seed={fixture_seed:#x}");
    assert!(!guard.land_submitted, "fixture_seed={fixture_seed:#x}");
    assert_eq!(guard.task_close_attempts, 0, "fixture_seed={fixture_seed:#x}");
    assert_eq!(guard.task_close_mutations, 0, "fixture_seed={fixture_seed:#x}");
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
    base_url: &str,
    state: &Arc<Mutex<CloseoutRecoveryRemoteState>>,
    fixture_seed: u64,
    land_delay: Duration,
    task_delay: Duration,
) -> PathBuf {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.lock().unwrap().reset_iteration(
            fixture_seed,
            CloseoutMutationBoundary::MutateBeforeResponse,
            land_delay,
            CloseoutMutationBoundary::MutateBeforeResponse,
            task_delay,
        );
        let temp = init_repo(base_url);
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
            payload["atomic_task_land"]["replayed"], true,
            "fixture_seed={fixture_seed:#x} fixture_root={}",
            root.display()
        );
        assert_closeout_mutated_once(state, fixture_seed);
        root
    }));
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
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let mut fixture_roots = BTreeSet::new();
    for repetition in 0..50_u64 {
        let fixture_seed = 0xA17_D37E_0000_0000_u64.wrapping_add(repetition);
        let root = run_seeded_closeout_repetition(
            &base_url,
            &state,
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
    handle.join().unwrap();
}

#[test]
fn native_task_land_recovery_passes_thirty_seeded_bounded_jitter_repetitions() {
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let mut fixture_roots = BTreeSet::new();
    for repetition in 0..30_u64 {
        let fixture_seed = 0xA17_5177_0000_0000_u64.wrapping_add(repetition);
        let land_delay = deterministic_recovery_jitter(fixture_seed, 13);
        let task_delay = deterministic_recovery_jitter(fixture_seed, 37);
        let root = run_seeded_closeout_repetition(
            &base_url,
            &state,
            fixture_seed,
            land_delay,
            task_delay,
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
    assert_eq!(fixture_roots.len(), 30);
    handle.join().unwrap();
}

#[test]
fn native_task_land_current_worktree_skips_unrelated_backlog_refresh() {
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
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
    let before: JsonValue =
        parse_json_file(&extra_metadata_path);
    assert!(before.get("workspace_status_cache").is_none());

    let payload = json_output_with_env(
        &worktree,
        &["task", "finish", "RT-1", "--json"],
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
        payload["automatic_reconciliation"]["sources"]["workspace_lock"]["metadata"]
            ["repo_root"]
            .as_str(),
        Some(root.canonicalize().unwrap().to_string_lossy().as_ref())
    );

    let after: JsonValue =
        parse_json_file(&extra_metadata_path);
    assert!(after.get("workspace_status_cache").is_none());
    handle.join().unwrap();
}

#[test]
fn native_workflow_finish_apply_delegates_final_closeout_to_atomic_task_land() {
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
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
    let before: JsonValue =
        parse_json_file(&backlog_metadata_path);
    assert!(before.get("workspace_status_cache").is_none());

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = workflow_land_apply(
        &repo,
        "RC-1",
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
        Some("task-land-atomic/v1")
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
    let after: JsonValue =
        parse_json_file(&backlog_metadata_path);
    assert!(after.get("workspace_status_cache").is_none());
    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert_eq!(
        logged
            .iter()
            .filter(|row| {
                row.method == "POST"
                    && row.url == "/v1/native/repository-authorities/7/task-land"
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
fn native_workflow_finish_cli_uses_default_remote_in_solo_local_before_atomic_task_land() {
    let (base_url, log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let config_path = temp.path().join(".ait/config.json");
    let mut config: JsonValue = parse_json_file(&config_path);
    config["workflow_mode"] = json!("solo_local");
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
            "RC-1",
            "--apply",
            "--review-message",
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
                && row.url == "/v1/native/repository-authorities/7/changes/RC-1/reviews"
                && row.body.contains("\"action\":\"code_review_summary\"")
        })
        .expect("workflow finish must record exact-Patchset code review");
    let task_review_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/changes/RC-1/reviews"
                && row.body.contains("\"action\":\"task_approve\"")
        })
        .expect("workflow finish must record configured Task approval");
    let policy_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url
                    == "/v1/native/repository-authorities/7/patchsets/RP-1:evaluatePolicy"
        })
        .expect("workflow finish must evaluate final Policy");
    let atomic_land_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/task-land"
        })
        .expect("workflow finish must delegate to atomic Task Land");
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
        &[
            "task",
            "audit",
            "RT-1",
            "--remote",
            "origin",
            "--json",
        ],
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
fn native_task_audit_cli_solo_remote_default_uses_remote_for_unpublished_local_draft() {
    let (base_url, log, _state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_local_draft_worktree_repo(&base_url);

    let payload = json_output(
        &worktree,
        &[
            "task",
            "audit",
            "RT-1",
            "--json",
        ],
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
    let (_temp, worktree, _started) =
        init_cli_local_draft_worktree_repo("http://127.0.0.1:1");

    let payload = json_output(
        &worktree,
        &[
            "task",
            "audit",
            "LT-0001",
            "--local",
            "--json",
        ],
    );

    assert_eq!(payload["audit_source"]["mode"].as_str(), Some("local"));
    assert_eq!(payload["task"]["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(
        payload["changes"][0]["change"]["change_id"].as_str(),
        Some("C-01")
    );
    assert_eq!(
        payload["changes"][0]["change"]["change_ref"].as_str(),
        Some("LT-0001/C-01")
    );
}

#[test]
fn native_task_audit_cli_solo_local_default_does_not_fallback_to_remote() {
    let (temp, worktree, _started) =
        init_cli_local_draft_worktree_repo("http://127.0.0.1:1");
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
        &[
            "task",
            "audit",
            "LT-0001",
            "--json",
        ],
    );

    assert_eq!(payload["audit_source"]["mode"].as_str(), Some("local"));
    assert_eq!(payload["task"]["task_id"].as_str(), Some("LT-0001"));
}

#[test]
fn native_task_land_unscoped_api_does_not_fallback_to_local_authority() {
    let (_temp, worktree) = init_local_draft_worktree_repo("http://127.0.0.1:1");

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let error = task_land_payload(&repo, "RT-1", None).unwrap_err();
    assert!(error.contains("as a remote task or change"));
}

#[test]
fn native_local_task_land_from_root_routes_to_exact_bound_worktree() {
    let (temp, worktree, started) =
        init_cli_local_draft_worktree_repo("http://127.0.0.1:1");
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
    let _unrelated_worktree = init_registered_worktree(
        root,
        "lt-other",
        "feature/lt-other",
        Some("LT-OTHER"),
        Some("C-01"),
        true,
        Some("after_remote_land"),
    );
    set_active_root_worktree(root, "lt-other");

    let main_before = json_output(root, &["line", "show", "main", "--json"]);
    assert_eq!(
        main_before["head_snapshot_id"].as_str(),
        started["worktree"]["fork_snapshot_id"].as_str()
    );

    let landed = json_output(
        root,
        &[
            "task",
            "finish",
            "LT-0001",
            "--local",
            "--json",
        ],
    );

    assert_eq!(landed["task_status"].as_str(), Some("completed"));
    assert_eq!(landed["change_status"].as_str(), Some("landed"));
    assert_eq!(landed["landed_snapshot_id"].as_str(), Some(snapshot_id.as_str()));
    assert_eq!(fs::read_to_string(root.join("src/lib.rs")).unwrap(), source);
    assert!(!worktree.exists());
    assert!(root.join("lt-other").exists());
}

#[test]
fn native_local_task_land_from_root_rejects_root_authoring_drift_before_mutation() {
    let (temp, worktree, started) =
        init_cli_local_draft_worktree_repo("http://127.0.0.1:1");
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
        json_output(root, &["task", "show", "LT-0001", "--local", "--json"])["status"]
            .as_str(),
        Some("active")
    );
    assert_eq!(
        json_output(
            root,
            &[
                "change",
                "show",
                "LT-0001/C-01",
                "--local",
                "--json",
            ],
        )["status"]
            .as_str(),
        Some("draft")
    );
    assert_eq!(
        json_output(root, &["line", "show", "main", "--json"])["head_snapshot_id"],
        started["worktree"]["fork_snapshot_id"]
    );
    assert_eq!(fs::read_to_string(worktree.join("src/lib.rs")).unwrap(), worktree_source);
    assert!(worktree.exists());
}

#[test]
fn native_task_land_local_apply_lands_snapshot_and_cleans_worktree() {
    let (temp, worktree, _started) =
        init_cli_local_draft_worktree_repo("http://127.0.0.1:1");
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
        &[
            "task",
            "finish",
            "LT-0001/C-01",
            "--local",
            "--json",
        ],
    );

    assert_eq!(payload["mode"].as_str(), Some("local"));
    assert_eq!(payload["apply_status"].as_str(), Some("done"));
    assert_eq!(payload["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(payload["change_id"].as_str(), Some("C-01"));
    assert_eq!(payload["change_ref"].as_str(), Some("LT-0001/C-01"));
    assert_eq!(
        payload["landed_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(payload["task_status"].as_str(), Some("completed"));
    assert_eq!(payload["change_status"].as_str(), Some("landed"));
    assert!(payload["auto_snapshot"].is_null());
    assert_eq!(fs::read_to_string(temp.path().join("src/lib.rs")).unwrap(), source);
    assert!(!metadata_path.exists());
    assert!(!worktree.exists());
}

#[test]
fn native_task_land_apply_force_removes_current_worktree_after_completion() {
    let (base_url, _log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
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
    assert_eq!(
        payload["workspace"]["evaluation"].as_str(),
        Some("skipped")
    );
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
    let patchset_revision_snapshot_id =
        seed_snapshot(&worktree, "task land main-seed promotion");
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
    assert_eq!(payload["main_seed_sync"]["status"].as_str(), Some("promoted"));
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
    assert!(payload["main_seed_sync"]["phase_timings_ms"]["land_seed_sync_total"]
        .as_f64()
        .is_some());

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
