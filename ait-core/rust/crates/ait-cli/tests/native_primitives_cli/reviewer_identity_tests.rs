const ACTING_REVIEW_SUMMARY: &str = "Reviewed files: src/lib.rs; Findings: none after inspection; Risks: low; Tests: cargo test passed; Recommendation: pass";

fn spawn_reviewer_remote() -> SpawnedFakeRemote<FakeRemoteState> {
    let (url, log, state, handle) = spawn_fake_remote();
    state.lock().unwrap().selected_patchset_id = Some("RT-1/P-01".to_string());
    (url, log, state, handle)
}

fn acting_review_args() -> Vec<&'static str> {
    vec![
        "review",
        "code",
        "submit",
        "RT-1",
        "--patchset",
        "RT-1/P-01",
        "--message",
        ACTING_REVIEW_SUMMARY,
        "--json",
    ]
}

fn reviewer_config(root: &Path, teams: bool, human: bool, required: bool) -> Vec<u8> {
    let path = root.join(".ait/config.json");
    let mut config = parse_json_file(&path);
    config["agent_teams"] = json!(teams);
    config["task_review"] = json!(required);
    config["user_name"] = if human {
        json!("Human Approver")
    } else {
        JsonValue::Null
    };
    config["user_email"] = if human {
        json!("human@example.com")
    } else {
        JsonValue::Null
    };
    write_file(&path, &encode_json_pretty(&config));
    fs::read(path).unwrap()
}

fn bind_fixture_reviewers(root: &Path) {
    use ait_core::agent_binary_db::single_team;
    let repo = RepoRuntime::discover_from_path(root).unwrap();
    let db = repo.binary_db_stores::<1>().local_db();
    single_team::ensure_pool(&db).unwrap();
    for (index, role, session) in [
        (1, 2, "einstein-session"),
        (5, 2, "faraday-session"),
        (4, 1, "developer-session"),
    ] {
        let agents = single_team::read(&db).unwrap();
        single_team::assign(&db, index, &agents[index as usize], role, session).unwrap();
    }
}

#[test]
fn native_acting_reviewers_require_actor_registration_and_interleave_without_team_manager_dependency(
) {
    for (teams, human, bound) in [
        (false, false, false),
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        let (url, log, state, handle) = spawn_reviewer_remote();
        let temp = init_repo(&url);
        let root = temp.path();
        let config = reviewer_config(root, teams, human, false);
        if bound {
            bind_fixture_reviewers(root);
        }
        if !human {
            let mut args = acting_review_args();
            args.extend(["--reviewer", "Einstein"]);
            let output = command_output_with_env(
                root,
                &args,
                &[
                    ("CODEX_THREAD_ID", "einstein-session"),
                    ("CODEX_SESSION_ID", "einstein-session"),
                ],
            );
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr)
                .contains("Actor-aware Remote mutation requires effective AIT config user_name"));
            assert!(state.lock().unwrap().recorded_actor_reviews.is_empty());
            assert!(log
                .lock()
                .unwrap()
                .iter()
                .all(|request| request.method == "GET"));
            assert_eq!(fs::read(root.join(".ait/config.json")).unwrap(), config);
            handle.join().unwrap();
            continue;
        }
        for (name, session) in [
            ("Einstein", "einstein-session"),
            ("Faraday", "faraday-session"),
            ("Einstein", "einstein-session"),
        ] {
            let mut args = acting_review_args();
            if !bound {
                args.extend(["--reviewer", name]);
            }
            let result = json_output_with_env(
                root,
                &args,
                &[("CODEX_THREAD_ID", session), ("CODEX_SESSION_ID", session)],
            );
            assert_eq!(result["reviewer"], name);
            assert_eq!(result["reviewer_kind"], "bot");
            assert_eq!(result["task_review"]["reviewer"], name);
            assert_eq!(result["task_review"]["result"]["reviewer_kind"], "bot");
            assert_eq!(fs::read(root.join(".ait/config.json")).unwrap(), config);
        }
        let records = state.lock().unwrap().recorded_actor_reviews.clone();
        assert_eq!(records.len(), 6);
        for (pair, name) in records
            .chunks_exact(2)
            .zip(["Einstein", "Faraday", "Einstein"])
        {
            assert_eq!(pair[0]["action"], "code_review_summary");
            assert_eq!(pair[1]["action"], "task_approve");
            for record in pair {
                assert_eq!(record["reviewer"], name);
                assert_eq!(record["reviewer_kind"], "bot");
            }
        }
        assert!(log
            .lock()
            .unwrap()
            .iter()
            .all(|request| !request.url.contains("worker-jobs")
                && !request.url.ends_with(":runCi")
                && !request.url.ends_with("/attestation")));
        handle.join().unwrap();
    }
}

#[test]
fn native_acting_review_rejects_missing_conflicting_and_developer_callers_before_review_write() {
    let (url, log, _state, handle) = spawn_reviewer_remote();
    let temp = init_repo(&url);
    let root = temp.path();
    let config = reviewer_config(root, false, true, false);
    bind_fixture_reviewers(root);
    for (session, requested, reason) in [
        ("external-session", None, "--reviewer"),
        ("einstein-session", Some("Faraday"), "conflicts"),
        ("developer-session", Some("Einstein"), "Reviewer role"),
        ("external-session", Some(""), "1 to 255"),
    ] {
        let mut args = acting_review_args();
        if let Some(name) = requested {
            args.extend(["--reviewer", name]);
        }
        let output = command_output_with_env(
            root,
            &args,
            &[("CODEX_THREAD_ID", session), ("CODEX_SESSION_ID", session)],
        );
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(reason),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|request| request.method == "GET"));
    assert_eq!(fs::read(root.join(".ait/config.json")).unwrap(), config);
    handle.join().unwrap();
}

#[test]
fn native_acting_review_preflights_capability_and_stops_on_untruthful_readback() {
    let (url, log, state, handle) = spawn_reviewer_remote();
    let temp = init_repo(&url);
    let root = temp.path();
    reviewer_config(root, false, true, false);
    let mut args = acting_review_args();
    args.extend(["--reviewer", "Einstein"]);
    state.lock().unwrap().unsupported_review_actor_identity = true;
    let output = command_output_with_env(root, &args, &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("review_actor_identity_v1"));
    assert!(log.lock().unwrap().iter().all(
        |request| request.method == "GET" || request.url == "/v1/native/repository-authorities"
    ));
    {
        let mut state = state.lock().unwrap();
        state.unsupported_review_actor_identity = false;
        state.review_readback_kind_override = Some("user".into());
    }
    let output = command_output_with_env(root, &args, &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("reconciliation"));
    let records = state.lock().unwrap().recorded_actor_reviews.clone();
    assert_eq!(
        records.len(),
        1,
        "no automatic Task approval after mismatched code review readback"
    );
    assert_eq!(records[0]["action"], "code_review_summary");
    handle.join().unwrap();
}

#[test]
fn native_acting_review_keeps_explicit_human_approval_separate() {
    let (url, _log, state, handle) = spawn_reviewer_remote();
    let temp = init_repo(&url);
    let root = temp.path();
    let config = reviewer_config(root, false, false, true);
    let mut args = acting_review_args();
    args.extend(["--reviewer", "Einstein"]);
    let rejected = command_output_with_env(root, &args, &[]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("user_name"));
    assert!(state.lock().unwrap().recorded_actor_reviews.is_empty());
    let human_args = [
        "review",
        "task",
        "approve",
        "RT-1",
        "--patchset",
        "RT-1/P-01",
        "--message",
        "functional checks passed",
        "--json",
    ];
    let output = command_output_with_env(root, &human_args, &[]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("user_name"));
    assert_eq!(fs::read(root.join(".ait/config.json")).unwrap(), config);
    let config = reviewer_config(root, false, true, true);
    let result = json_output(root, &args);
    assert_eq!(result["reviewer_kind"], "bot");
    assert_eq!(result["task_review"]["status"], "pending");
    assert_eq!(state.lock().unwrap().recorded_actor_reviews.len(), 1);
    let approval = json_output(root, &human_args);
    assert_eq!(approval["reviewer"], "Human Approver");
    assert_eq!(approval["reviewer_kind"], "user");
    assert_eq!(fs::read(root.join(".ait/config.json")).unwrap(), config);
    handle.join().unwrap();
}

#[test]
fn native_acting_reviewer_workflow_stops_for_explicit_human_task_approval() {
    let (url, log, state, handle) = spawn_closeout_recovery_remote();
    let (temp, worktree) = init_worktree_repo(&url);
    reviewer_config(temp.path(), false, true, true);
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn reviewed() -> bool { true }\n",
    );
    let revision = seed_snapshot(&worktree, "exact reviewed candidate");
    {
        let mut guard = state.lock().unwrap();
        guard.patchset_revision_snapshot_id = Some(revision);
        guard.enforce_reviewer_workflow = true;
        guard.code_review_recorded = false;
        // A previous automatic bot approval cannot satisfy a subsequently
        // selected explicit human-review policy.
        guard.task_review_recorded = true;
        guard.existing_task_review_kind = Some("bot".into());
        guard.policy_evaluated = false;
    }
    let output = command_output_with_env(
        &worktree,
        &[
            "workflow",
            "finish",
            "RT-1",
            "--apply",
            "--remote",
            "origin",
            "--reviewer",
            "Einstein",
            "--review-message",
            ACTING_REVIEW_SUMMARY,
            "--json",
            "--full",
        ],
        &[],
    );
    let result = parse_json_bytes(&output.stdout);
    assert_eq!(result["apply_status"], "stopped", "{result}");
    assert!(result["apply_stopped_reason"]
        .as_str()
        .unwrap()
        .contains("Task review is required"));
    let requests = log.lock().unwrap();
    let reviews = requests
        .iter()
        .filter(|request| request.method == "POST" && request.url.ends_with("/reviews"))
        .collect::<Vec<_>>();
    assert_eq!(reviews.len(), 1);
    let review = parse_json(&reviews[0].body);
    assert_eq!(review["reviewer"], "Einstein");
    assert_eq!(review["reviewer_kind"], "bot");
    assert_eq!(review["action"], "code_review_summary");
    assert!(!requests
        .iter()
        .any(|request| request.url.ends_with("/task-land")));
    drop(requests);
    assert!(!state.lock().unwrap().land_submitted);
    handle.join().unwrap();
}

#[test]
fn native_required_human_workflow_consumes_only_positive_user_kind_evidence() {
    for (required, kind, omit, expected) in [
        (true, "user", true, 0),
        (true, "unknown", false, 0),
        (true, "bot", false, 0),
        (true, "user", false, 1),
        (false, "user", true, 1),
    ] {
        let (url, log, state, handle) = spawn_closeout_recovery_remote();
        let (temp, worktree) = init_worktree_repo(&url);
        reviewer_config(temp.path(), false, false, required);
        write_file(&worktree.join("source.txt"), "Exact reviewed candidate\n");
        let revision = seed_snapshot(&worktree, "positive user-kind fixture");
        {
            let mut guard = state.lock().unwrap();
            guard.patchset_revision_snapshot_id = Some(revision);
            guard.enforce_reviewer_workflow = true;
            guard.existing_task_review_kind = Some(kind.into());
            guard.omit_task_review_kind = omit;
        }
        let result = json_output(
            &worktree,
            &[
                "workflow", "finish", "RT-1", "--remote", "origin", "--json", "--full",
            ],
        );
        assert_eq!(
            result["review"]["task_approvals"], expected,
            "required={required} kind={kind} omit={omit}: {result}"
        );
        assert_eq!(
            result["review"]["human_task_approvals"],
            i64::from(kind == "user" && !omit),
            "{result}"
        );
        if expected == 0 {
            assert_eq!(result["next_action"]["code"], "record_review", "{result}");
        } else {
            assert_ne!(result["next_action"]["code"], "record_review", "{result}");
        }
        assert!(log
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.method == "GET"));
        handle.join().unwrap();
    }
}
