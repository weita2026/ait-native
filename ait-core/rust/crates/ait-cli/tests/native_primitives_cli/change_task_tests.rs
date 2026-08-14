#[test]
fn native_patchset_namespace_supports_list_show_and_select() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);

    let listed = json_output(
        &worktree,
        &[
            "patchset",
            "list",
            "--change",
            "RC-1",
            "--repo",
            "fixture-ait",
            "--json",
        ],
    );
    let listed_rows = listed.as_array().unwrap();
    assert_eq!(listed_rows[0]["patchset_id"].as_str(), Some("RP-1"));
    assert_eq!(listed_rows[0]["patchset_number"].as_i64(), Some(1));

    let shown = json_output(
        &worktree,
        &[
            "patchset",
            "show",
            "1",
            "--repo",
            "fixture-ait",
            "--change",
            "RC-1",
            "--json",
        ],
    );
    assert_eq!(shown["patchset_id"].as_str(), Some("RP-1"));
    assert_eq!(shown["change_id"].as_str(), Some("RC-1"));
    assert_eq!(shown["summary"].as_str(), Some("Native Rust patchset"));

    let selected = json_output(
        &worktree,
        &["patchset", "select", "RP-1", "--change", "RC-1", "--json"],
    );
    assert_eq!(selected["change_id"].as_str(), Some("RC-1"));
    assert_eq!(selected["selected_patchset_id"].as_str(), Some("RP-1"));

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets"));
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url == "/v1/native/repository-authorities/7/patchsets/1?change_ref=RC-1"));
    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/changes/RC-1:selectPatchset"
        && row.body.contains("\"patchset_id\":\"RP-1\"")));
}

#[test]
fn native_change_namespace_supports_local_and_remote_scopes() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let repo = init_repo(&base_url);
    let root = repo.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let local_started = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Native local change",
            "--intent",
            "create local change for change namespace coverage",
            "--base-line",
            "main",
            "--json",
        ],
    );
    assert_eq!(local_started["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(local_started["change"]["change_id"].as_str(), Some("C-01"));
    assert_eq!(
        local_started["change"]["publication_state"].as_str(),
        Some("local_draft")
    );
    let local_show = json_output(
        root,
        &["change", "show", "C-01", "--local", "--json"],
    );
    assert_eq!(local_show["change_id"].as_str(), Some("C-01"));
    assert_eq!(local_show["task_id"].as_str(), Some("LT-0001"));
    assert!(log.lock().unwrap().is_empty());

    let remote_show = json_output(root, &["change", "show", "RC-1", "--json"]);
    assert_eq!(remote_show["change_id"].as_str(), Some("RC-1"));

    let remote_closed = json_output(root, &["change", "close", "RC-1", "--json"]);
    assert_eq!(remote_closed["change_id"].as_str(), Some("RC-1"));
    assert_eq!(remote_closed["status"].as_str(), Some("archived"));

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged
        .iter()
        .any(|row| row.method == "GET"
            && row.url == "/v1/native/repository-authorities/7/changes/RC-1"));
    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/changes/RC-1:close"
        && row.body.contains("\"status\":\"archived\"")));
}

#[test]
fn native_change_namespace_supports_revert_and_replay() {
    let (base_url, _log, state, handle) = spawn_fake_remote();
    let (_repo, worktree) = init_worktree_repo(&base_url);
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "change checkpoint",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();

    let reverted = json_output(
        &worktree,
        &["change", "revert", "RC-1", "--remote", "origin", "--json"],
    );
    assert_eq!(reverted["change_id"].as_str(), Some("RC-1"));
    assert_eq!(
        reverted["latest_change_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(
        reverted["fork_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(reverted["applied"].as_bool(), Some(true));

    let replayed = json_output(
        &worktree,
        &[
            "change",
            "replay",
            "RC-1",
            "--onto",
            "feature/rt-1",
            "--remote",
            "origin",
            "--force",
            "--json",
        ],
    );
    assert_eq!(replayed["change_id"].as_str(), Some("RC-1"));
    assert_eq!(
        replayed["latest_change_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(replayed["onto_line"].as_str(), Some("feature/rt-1"));
    assert_eq!(replayed["applied"].as_bool(), Some(true));

    handle.join().unwrap();
}

#[test]
fn native_task_complete_local_cli_surface_is_removed() {
    let temp = init_repo("https://example.test");
    let root = temp.path();

    let output = cargo_bin()
        .current_dir(root)
        .args(["task", "complete", "RT-LOCAL", "--local", "--json"])
        .env_remove("AIT_JSON_MODE")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand 'complete'"));
}

#[test]
fn native_task_start_local_scope_creates_authoritative_rows_and_worktree() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "default_remote": "origin",
  "workflow_default_scope": "local",
  "task_default_scope": "local",
  "sprint": "off",
  "plan_task_binding": {"mode": "off"},
  "task_tracking": "on",
  "user_name": "Fixture User",
  "user_email": "fixture@example.com"
}"#,
    );

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap local workflow",
            "--intent",
            "create draft task and change together",
            "--base-line",
            "main",
            "--json",
        ],
    );

    assert_eq!(payload["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(payload["publication_state"].as_str(), Some("local_draft"));
    assert_eq!(payload["change"]["change_id"].as_str(), Some("C-01"));
    assert_eq!(payload["change"]["publication_state"].as_str(), Some("local_draft"));
    assert_eq!(payload["worktree"]["exists"].as_bool(), Some(true));
    assert!(root.join(".ait/worktrees/lt-0001.json").exists());
    assert!(payload["worktree"]["cargo_target_dir"].is_null());
    assert!(payload["worktree"]["cargo_build_dir"].is_null());
    assert!(!payload["worktree"]["shell_command"]
        .as_str()
        .unwrap()
        .contains("CARGO_"));
    let worktree_path = PathBuf::from(payload["worktree"]["path"].as_str().unwrap());
    assert!(!root.join(".ait/cargo-build").exists());
    assert!(!root.join(".ait/cargo-target").exists());
    assert!(!worktree_path.join(".cargo/config.toml").exists());

    let task = json_output(root, &["task", "show", "LT-0001", "--local", "--json"]);
    assert_eq!(task["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(task["status"].as_str(), Some("active"));
    let change = json_output(
        root,
        &["change", "show", "C-01", "--local", "--json"],
    );
    assert_eq!(change["change_id"].as_str(), Some("C-01"));
    assert_eq!(change["status"].as_str(), Some("draft"));

    let second = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Second local workflow",
            "--intent",
            "prove Change ordinals are Task-local",
            "--base-line",
            "main",
            "--json",
        ],
    );
    assert_eq!(second["task_id"].as_str(), Some("LT-0002"));
    assert_eq!(second["change"]["change_id"].as_str(), Some("C-01"));
    assert_eq!(
        second["change"]["change_ref"].as_str(),
        Some("LT-0002/C-01")
    );

    let ambiguous = cargo_bin()
        .current_dir(root)
        .args(["change", "show", "C-01", "--local", "--json"])
        .env_remove("AIT_JSON_MODE")
        .output()
        .unwrap();
    assert!(!ambiguous.status.success());
    assert!(String::from_utf8_lossy(&ambiguous.stderr)
        .contains("Task context is required to resolve ambiguous Change ID C-01"));

    let contextual = json_output(
        root,
        &[
            "change",
            "show",
            "LT-0002/C-01",
            "--local",
            "--json",
        ],
    );
    assert_eq!(contextual["change_id"].as_str(), Some("C-01"));
    assert_eq!(contextual["change_ref"].as_str(), Some("LT-0002/C-01"));

    let audit = json_output(root, &["task", "audit", "LT-0002", "--json"]);
    assert_eq!(audit["task"]["task_id"].as_str(), Some("LT-0002"));
    assert_eq!(audit["audit_source"]["mode"].as_str(), Some("local_draft"));
    assert_eq!(
        audit["audit_source"]["remote_task_missing"].as_bool(),
        Some(false)
    );
    assert_eq!(audit["workflow"]["state"].as_str(), Some("in_progress"));
    assert_eq!(
        audit["changes"][0]["change"]["change_ref"].as_str(),
        Some("LT-0002/C-01")
    );
    assert_eq!(
        audit["changes"][0]["target_state"].as_str(),
        Some("local_change_not_landed")
    );

    let land_preview = json_output(
        root,
        &[
            "task", "land", "LT-0002", "--local", "--preview", "--json",
        ],
    );
    assert_eq!(land_preview["status"].as_str(), Some("ready"));
    assert_eq!(land_preview["change_id"].as_str(), Some("C-01"));
    assert_eq!(
        land_preview["change_ref"].as_str(),
        Some("LT-0002/C-01")
    );
    assert_eq!(
        land_preview["change"]["task_id"].as_str(),
        Some("LT-0002")
    );
    assert_eq!(
        land_preview["change"]["change_ref"].as_str(),
        Some("LT-0002/C-01")
    );
}

#[test]
fn native_task_start_enforces_sprint_specific_public_forms() {
    let temp = init_repo("https://example.test");
    let root = temp.path();

    let source_while_off = cargo_bin()
        .current_dir(root)
        .args([
            "task",
            "start",
            "--from",
            "docs/sprints/card.md#card/item",
            "--intent",
            "Reject Plan source while sprint mode is off",
            "--local",
        ])
        .output()
        .unwrap();
    assert!(!source_while_off.status.success());
    assert!(String::from_utf8_lossy(&source_while_off.stderr)
        .contains("`ait task start --from` is unavailable while sprint mode is off"));

    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "workflow_default_scope": "local",
  "task_default_scope": "local",
  "change_default_scope": "local",
  "sprint": "on",
  "plan_task_binding": {"mode": "required"}
}"#,
    );
    let manual_while_on = cargo_bin()
        .current_dir(root)
        .args([
            "task",
            "start",
            "--title",
            "Unbound sprint task",
            "--intent",
            "Reject manual title while sprint mode is on",
            "--local",
        ])
        .output()
        .unwrap();
    assert!(!manual_while_on.status.success());
    let stderr = String::from_utf8_lossy(&manual_while_on.stderr);
    assert!(stderr.contains("`ait task start --title` is unavailable while sprint mode is on"));
    assert!(stderr.contains("task start --from <markdown-path>#<item-ref>"));

    let tasks = json_output(root, &["task", "list", "--local", "--json"]);
    assert_eq!(tasks.as_array().map(Vec::len), Some(0));
}

#[test]
fn native_task_start_from_emits_one_json_document_and_ordered_human_progress() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(
        &root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "workflow_default_scope": "local",
  "task_default_scope": "local",
  "sprint": "on",
  "plan_task_binding": {"mode": "required"},
  "task_tracking": "on",
  "user_name": "Fixture User",
  "user_email": "fixture@example.com"
}"#,
    );
    write_file(
        &root.join("docs/sprints/source-start.md"),
        r#"# Source Start [plan-ref: source-start]

- [ ] Add JSON Plan-derived start. [ref: source-start/json]
- [ ] Add human Plan-derived start. [ref: source-start/human]
- [ ] Add compact Agent Plan-derived start. [ref: source-start/compact]
"#,
    );

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/source-start.md#source-start/json",
            "--intent",
            "Prove JSON output remains one valid document",
            "--local",
            "--json",
        ],
    );
    assert_eq!(payload["title"], "Add JSON Plan-derived start");
    assert_eq!(payload["title_source"], "plan_item");
    assert_eq!(payload["plan_source"]["scope"], "local");
    assert_eq!(
        payload["plan_source"]["plan_item_ref"],
        "source-start/json"
    );
    assert_eq!(payload["cd_command"], payload["worktree"]["cd_command"]);

    let output = cargo_bin()
        .current_dir(root)
        .args([
            "task",
            "start",
            "--from",
            "docs/sprints/source-start.md#source-start/human",
            "--intent",
            "Prove human progress follows the durable phase order",
            "--local",
            "--verbose",
        ])
        .env_remove("AIT_JSON_MODE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let sync_index = stdout.find("synchronizing Plan source:").unwrap();
    let validation_index = stdout.find("Plan item taskable:").unwrap();
    let task_index = stdout.find("task created:").unwrap();
    let ready_index = stdout.find("worktree ready:").unwrap();
    assert!(sync_index < validation_index);
    assert!(validation_index < task_index);
    assert!(task_index < ready_index);
    assert!(stdout.contains("ait task start"));
    assert!(stdout.contains("next: cd "));
    assert!(!stdout.contains("Your current shell has not been switched automatically."));

    let compact_output = cargo_bin()
        .current_dir(root)
        .args([
            "task",
            "start",
            "--from",
            "docs/sprints/source-start.md#source-start/compact",
            "--intent",
            "Prove default Agent output contains only decision facts",
            "--local",
        ])
        .env_remove("AIT_JSON_MODE")
        .output()
        .unwrap();
    assert!(compact_output.status.success());
    let compact_stdout = String::from_utf8_lossy(&compact_output.stdout);
    assert!(compact_stdout.contains("ait task start"));
    assert!(compact_stdout.contains("task: LT-"));
    assert!(compact_stdout.contains("change: LT-"));
    assert!(compact_stdout.contains("/C-01"));
    assert!(compact_stdout.contains("next: cd "));
    assert!(!compact_stdout.contains("synchronizing Plan source:"));
    assert!(!compact_stdout.contains("worktree ready:"));
    assert!(!compact_stdout.contains("ait-cli"));

    let tasks = json_output(root, &["task", "list", "--local", "--json"]);
    let compact_task_id = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["title"] == "Add compact Agent Plan-derived start")
        .and_then(|task| task["task_id"].as_str())
        .unwrap();
    let audit = cargo_bin()
        .current_dir(root)
        .args(["task", "audit", compact_task_id])
        .env_remove("AIT_JSON_MODE")
        .output()
        .unwrap();
    assert!(audit.status.success());
    let audit_stdout = String::from_utf8_lossy(&audit.stdout);
    assert!(audit_stdout.contains(&format!("{compact_task_id}/C-01")));
    assert!(audit_stdout.contains("target_state"));
    assert!(!audit_stdout.contains("{\"archived_at\""));

    let shown = cargo_bin()
        .current_dir(root)
        .args(["task", "show", compact_task_id, "--local"])
        .env_remove("AIT_JSON_MODE")
        .output()
        .unwrap();
    assert!(shown.status.success());
    let shown_stdout = String::from_utf8_lossy(&shown.stdout);
    assert!(shown_stdout.starts_with("ait task show\n"));
    assert!(!shown_stdout.contains("published_task_id:"));
    assert!(!shown_stdout.contains("ait-cli"));
}

#[test]
fn native_remote_task_start_from_uses_one_atomic_mutation_and_no_legacy_posts() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    let config_path = root.join(".ait/config.json");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace(r#""sprint": "off""#, r#""sprint": "on""#)
        .replace(
            r#""plan_task_binding": {"mode": "off"}"#,
            r#""plan_task_binding": {"mode": "required"}"#,
        );
    write_file(&config_path, &config);
    write_file(
        &root.join("docs/sprints/atomic-start.md"),
        r#"# Atomic Start [plan-ref: atomic-start]

- [ ] Start the remote Task atomically. [ref: atomic-start/implement]
"#,
    );

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/atomic-start.md#atomic-start/implement",
            "--intent",
            "Publish the Plan head and create Task plus Change in one mutation",
            "--base-line",
            "main",
            "--json",
        ],
    );

    assert_eq!(payload["task_id"], "RT-ATOMIC");
    assert_eq!(payload["change"]["change_id"], "C-01");
    assert_eq!(
        payload["plan_source"]["transport_contract"],
        "task-start-atomic/v1"
    );
    assert!(payload["phase_timings_ms"]["atomic_remote_start"]
        .as_f64()
        .is_some());
    assert!(payload["phase_timings_ms"]["worktree_bootstrap"]
        .as_f64()
        .is_some());

    let replay = json_output(
        root,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/atomic-start.md#atomic-start/implement",
            "--intent",
            "Publish the Plan head and create Task plus Change in one mutation",
            "--base-line",
            "main",
            "--json",
        ],
    );
    assert_eq!(replay["task_id"], payload["task_id"]);
    assert_eq!(replay["change"]["change_id"], payload["change"]["change_id"]);
    assert_eq!(
        replay["worktree"]["open_path"],
        payload["worktree"]["open_path"]
    );
    assert_eq!(replay["worktree_reused"], true);

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let atomic = logged
        .iter()
        .filter(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/task-start"
        })
        .collect::<Vec<_>>();
    assert_eq!(atomic.len(), 2);
    let atomic_body: JsonValue = parse_json(&atomic[0].body);
    assert_eq!(atomic_body["contract"], "task-start-atomic/v1");
    assert_eq!(atomic_body["plan"]["action"], "create");
    assert!(atomic_body["task"].get("plan_id").is_none());
    assert!(atomic_body["change"].get("task_id").is_none());
    let replay_body: JsonValue = parse_json(&atomic[1].body);
    assert_eq!(replay_body["plan"]["action"], "existing");
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
    }));
    let commit_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url
                    == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
        })
        .unwrap();
    let atomic_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/task-start"
        })
        .unwrap();
    assert!(commit_index < atomic_index);
}

#[test]
fn native_remote_task_start_attaches_task_change_only_to_final_plan_revision() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    let config_path = root.join(".ait/config.json");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace(r#""sprint": "off""#, r#""sprint": "on""#)
        .replace(
            r#""plan_task_binding": {"mode": "off"}"#,
            r#""plan_task_binding": {"mode": "required"}"#,
        );
    write_file(&config_path, &config);
    let card_path = root.join("docs/sprints/atomic-revise.md");
    write_file(
        &card_path,
        r#"# Atomic Revise [plan-ref: atomic-revise]

- [ ] Start from the final revised Plan head. [ref: atomic-revise/implement]
"#,
    );
    let initial_sync = json_output(
        root,
        &[
            "plan",
            "sync",
            "docs/sprints/atomic-revise.md",
            "--remote",
            "origin",
            "--json",
        ],
    );
    assert_eq!(initial_sync["status"], "ok");
    write_file(
        &card_path,
        r#"# Atomic Revise [plan-ref: atomic-revise]

- [ ] Start from the final revised Plan head. [ref: atomic-revise/implement]
  - Verify the composite mutation owns this new head.
"#,
    );

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/atomic-revise.md#atomic-revise/implement",
            "--intent",
            "Attach workflow lineage only after the final Plan pack is committed",
            "--base-line",
            "main",
            "--json",
        ],
    );
    assert_eq!(payload["plan_source"]["plan_id"], "PR-42");
    assert_eq!(
        payload["plan_source"]["plan_revision_id"],
        "plan-revision:43"
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let atomic = logged
        .iter()
        .find(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/task-start"
        })
        .unwrap();
    let body: JsonValue = parse_json(&atomic.body);
    assert_eq!(body["plan"]["action"], "revise");
    assert_eq!(body["plan"]["plan_id"], "PR-42");
    assert_eq!(
        body["plan"]["expected_head_revision_id"],
        "plan-revision:42"
    );
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
    }));
}

#[test]
fn native_remote_task_start_atomic_failure_never_falls_back_to_legacy_posts() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.fail_atomic_task_start = true;
    }
    let config_path = root.join(".ait/config.json");
    let config = fs::read_to_string(&config_path)
        .unwrap()
        .replace(r#""sprint": "off""#, r#""sprint": "on""#)
        .replace(
            r#""plan_task_binding": {"mode": "off"}"#,
            r#""plan_task_binding": {"mode": "required"}"#,
        );
    write_file(&config_path, &config);
    write_file(
        &root.join("docs/sprints/atomic-failure.md"),
        r#"# Atomic Failure [plan-ref: atomic-failure]

- [ ] Fail without partial workflow lineage. [ref: atomic-failure/implement]
"#,
    );

    let output = cargo_bin()
        .current_dir(root)
        .args([
            "task",
            "start",
            "--from",
            "docs/sprints/atomic-failure.md#atomic-failure/implement",
            "--intent",
            "Prove transport failure does not use legacy mutation endpoints",
            "--base-line",
            "main",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("injected atomic task-start transaction failure"));
    assert!(stderr.contains("exact-replay safe"));

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert_eq!(
        logged
            .iter()
            .filter(|row| {
                row.method == "POST"
                    && row.url == "/v1/native/repository-authorities/7/task-start"
            })
            .count(),
        1
    );
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
    }));
    assert!(
        fs::read_dir(root.join(".ait/worktrees"))
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
    );
}

#[test]
fn native_task_start_uses_remote_task_payload_contract() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--title",
            "Bootstrap remote workflow",
            "--intent",
            "create remote task without task tracking payload",
            "--base-line",
            "main",
            "--json",
        ],
    );

    assert_eq!(payload["task_id"].as_str(), Some("RT-REMOTE"));
    assert!(payload.get("tracking").is_none());
    let mut payload_keys = payload
        .as_object()
        .expect("task start payload")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    payload_keys.sort();
    assert_eq!(
        payload_keys,
        vec![
            "automatic_reconciliation",
            "change",
            "intent",
            "phase_timings_ms",
            "published_task_id",
            "repo_name",
            "task_id",
            "title",
            "worktree",
        ]
    );
    assert_eq!(
        payload["automatic_reconciliation"]["automatic_trigger"]["contract"],
        json!("workflow-automatic-reconciliation/v1")
    );
    assert_eq!(
        payload["automatic_reconciliation"]["automatic_trigger"]["trigger"],
        json!("pre_task_start")
    );
    assert_eq!(
        payload["automatic_reconciliation"]["safe_only"],
        json!(true)
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let task_create = logged
        .iter()
        .find(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks")
        .unwrap();
    let body: JsonValue = parse_json(&task_create.body);
    assert!(body.get("tracking").is_none());
}

#[test]
fn native_task_start_local_scope_bootstraps_main_seed() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    let ephemeral_root = root.join("ephemeral-root");
    write_file(
        &root.join(".ait/config.json"),
        &format!(
            r#"{{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "default_remote": "origin",
  "workflow_default_scope": "local",
  "task_default_scope": "local",
  "change_default_scope": "local",
  "sprint": "off",
  "plan_task_binding": {{"mode": "off"}},
  "task_worktree": {{
    "ephemeral_root": "{}"
  }}
}}"#,
            ephemeral_root.display()
        ),
    );

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap main seed",
            "--intent",
            "exercise fresh main-seed bootstrap",
            "--base-line",
            "main",
            "--json",
        ],
    );

    assert_eq!(payload["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(
        payload["worktree"]["main_seed"]["status"].as_str(),
        Some("refreshed")
    );
    assert_eq!(
        payload["worktree"]["main_seed"]["seed_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert!(ephemeral_root.exists());
}

#[test]
fn native_task_start_ignores_disjoint_server_seed_root() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    let client_runtime_root = root.join("client-runtime");
    let server_runtime_root = root.join("separate-server-runtime");
    let server_seed = server_runtime_root
        .join("main-seeds")
        .join("fixture-ait")
        .join("main-seed");
    write_file(
        &root.join(".ait/config.json"),
        &format!(
            r#"{{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "default_remote": "origin",
  "workflow_default_scope": "local",
  "task_default_scope": "local",
  "change_default_scope": "local",
  "sprint": "off",
  "plan_task_binding": {{"mode": "off"}},
  "task_worktree": {{
    "ephemeral_root": "{}"
  }}
}}"#,
            client_runtime_root.display()
        ),
    );
    write_file(
        &root.join(".cargo/config.toml"),
        "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/workspaces/{workspace-path-hash}\"\n\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n",
    );
    let server_seed_snapshot_id = seed_snapshot(root, "server seed source Cargo policy");
    fs::create_dir_all(server_seed.join("src")).unwrap();
    fs::copy(
        root.join("src/lib.rs"),
        server_seed.join("src/lib.rs"),
    )
    .unwrap();
    write_file(
        &server_seed.join("server-only-marker.txt"),
        "must never cross the host boundary\n",
    );
    let server_runtime_root_text = server_runtime_root.to_string_lossy().to_string();

    let payload = json_output_with_env(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Ignore server land seed",
            "--intent",
            "prove the CLI refreshes only its own local seed",
            "--base-line",
            "main",
            "--json",
        ],
        &[("AIT_NATIVE_SERVER_CI_RAM_ROOT", &server_runtime_root_text)],
    );

    assert_eq!(
        payload["worktree"]["main_seed"]["status"].as_str(),
        Some("refreshed")
    );
    let client_seed = PathBuf::from(
        payload["worktree"]["main_seed"]["path"]
            .as_str()
            .unwrap(),
    );
    let _seed_cleanup = WritableTreeOnDrop::new(client_seed.clone());
    let canonical_client_seed = client_seed.canonicalize().unwrap();
    let canonical_client_runtime_root = client_runtime_root.canonicalize().unwrap();
    assert_eq!(
        payload["worktree"]["main_seed"]["path"].as_str(),
        Some(canonical_client_seed.to_string_lossy().as_ref())
    );
    assert!(canonical_client_seed.starts_with(&canonical_client_runtime_root));
    assert!(canonical_client_seed.ends_with("fixture-ait/.ait-internal/main-seed"));
    assert_ne!(
        payload["worktree"]["main_seed"]["path"].as_str(),
        Some(server_seed.to_string_lossy().as_ref())
    );
    assert_eq!(
        payload["worktree"]["main_seed"]["seed_snapshot_id"].as_str(),
        Some(server_seed_snapshot_id.as_str())
    );
    assert!(payload
        .pointer("/phase_timings_ms/worktree_bootstrap")
        .and_then(JsonValue::as_f64)
        .is_some());
    let worktree = PathBuf::from(payload["worktree"]["open_path"].as_str().unwrap());
    let projected_cargo_config = fs::read_to_string(worktree.join(".cargo/config.toml")).unwrap();
    assert!(projected_cargo_config
        .starts_with("# Managed by ait: stable final artifacts, workspace-isolated intermediates."));
    assert!(projected_cargo_config
        .contains("[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n"));
    assert!(payload["worktree"]["cargo_target_dir"].is_string());
    assert!(payload["worktree"]["cargo_build_dir"].is_string());
    assert!(payload["worktree"]["shell_command"]
        .as_str()
        .unwrap()
        .contains("CARGO_TARGET_DIR"));
    let status = json_output(&worktree, &["status", "--json"]);
    assert_eq!(status["workspace_status"].as_str(), Some("clean"));
    assert!(
        client_seed.is_dir(),
        "task start must materialize the CLI-owned seed"
    );
    assert!(
        !client_seed.join("server-only-marker.txt").exists(),
        "the CLI must never copy content from the server-owned seed path"
    );
}

#[test]
fn native_task_start_uses_remote_base_when_local_line_is_ahead() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"local ahead\" }\n",
    );
    let local_snapshot_id = seed_snapshot(root, "local ahead");

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--title",
            "Use authoritative remote base",
            "--intent",
            "start shared work without pushing the local-only head",
            "--base-line",
            "main",
            "--json",
        ],
    );

    assert_eq!(payload["task_id"].as_str(), Some("RT-REMOTE"));
    assert_eq!(
        payload["change"]["fork_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        local_line_head(root, "main").as_deref(),
        Some(local_snapshot_id.as_str()),
        "Remote Task start must not move the local Line"
    );
    let worktree = PathBuf::from(payload["worktree"]["open_path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        "pub fn example() -> &'static str { \"ok\" }\n",
        "the worktree must materialize the Remote head, not the local-only head"
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged
        .iter()
        .any(|row| row.method == "GET"
            && row.url == "/v1/native/repository-authorities/7/lines/main"));
    assert!(logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"));
    let change_create = logged
        .iter()
        .find(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
        })
        .unwrap();
    assert_eq!(
        parse_json(&change_create.body)["fork_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
}

#[test]
fn native_task_start_uses_remote_base_when_remote_line_is_ahead() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"remote ahead\" }\n",
    );
    let remote_snapshot_id = seed_snapshot(root, "remote ahead");
    seed_binary_line(root, "main", FIXTURE_BASE_SNAPSHOT_ID);
    state.lock().unwrap().remote_head_snapshot_id = Some(remote_snapshot_id.clone());

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--title",
            "Use imported remote base",
            "--intent",
            "start shared work from the imported Remote head",
            "--base-line",
            "main",
            "--json",
        ],
    );

    assert_eq!(payload["task_id"].as_str(), Some("RT-REMOTE"));
    assert_eq!(
        payload["change"]["fork_snapshot_id"].as_str(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(
        local_line_head(root, "main").as_deref(),
        Some(FIXTURE_BASE_SNAPSHOT_ID),
        "Remote Task start must not fast-forward the local Line"
    );
    let worktree = PathBuf::from(payload["worktree"]["open_path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        "pub fn example() -> &'static str { \"remote ahead\" }\n",
        "the worktree must materialize the imported Remote head"
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged
        .iter()
        .any(|row| row.method == "GET"
            && row.url == "/v1/native/repository-authorities/7/lines/main"));
    assert!(logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"));
    let change_create = logged
        .iter()
        .find(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
        })
        .unwrap();
    assert_eq!(
        parse_json(&change_create.body)["fork_snapshot_id"].as_str(),
        Some(remote_snapshot_id.as_str())
    );
}

#[test]
fn native_task_start_from_existing_worktree_performs_zero_remote_mutations() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    let worktree = init_registered_worktree(
        root,
        "rt-existing",
        "feature/rt-existing",
        Some("RT-EXISTING"),
        Some("RT-EXISTING/C-01"),
        true,
        Some("after_remote_land"),
    );

    let output = cargo_bin()
        .current_dir(&worktree)
        .args([
            "task",
            "start",
            "--title",
            "Must not mutate",
            "--intent",
            "Reject before remote task create",
            "--base-line",
            "main",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Refusing to run `ait task start` inside existing worktree"));
    assert!(stderr.contains(root.to_string_lossy().as_ref()));

    let _ = json_output(root, &["task", "list", "--json"]);
    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
    }));
}

#[test]
fn native_change_create_recovers_existing_remote_task_with_contextual_id() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let created = json_output(
        root,
        &[
            "change",
            "create",
            "RT-REMOTE",
            "--title",
            "Recover remote task",
            "--base-line",
            "main",
            "--json",
        ],
    );
    assert_eq!(created["change_id"], json!("C-01"));
    assert_eq!(created["change_ref"], json!("RT-REMOTE/C-01"));
    assert_eq!(created["task_id"], json!("RT-REMOTE"));
    assert_eq!(created["base_line"], json!("main"));

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| {
        row.method == "GET" && row.url == "/v1/native/repository-authorities/7/lines/main"
    }));
    assert!(logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"
    }));
}

#[test]
fn native_task_namespace_reads_local_and_remote_scopes() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    let local_started = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Local namespace task",
            "--intent",
            "verify local task reads",
            "--base-line",
            "main",
            "--json",
        ],
    );
    assert_eq!(local_started["task_id"].as_str(), Some("LT-0001"));
    let local_listed = json_output(root, &["task", "list", "--local", "--json"]);
    assert_eq!(local_listed.as_array().unwrap().len(), 1);
    assert_eq!(local_listed[0]["task_id"].as_str(), Some("LT-0001"));
    let local_shown = json_output(
        root,
        &["task", "show", "LT-0001", "--local", "--json"],
    );
    assert_eq!(local_shown["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(local_shown["publication_state"].as_str(), Some("local_draft"));
    assert!(log.lock().unwrap().is_empty());

    let remote_listed = json_output(root, &["task", "list", "--remote", "origin", "--json"]);
    assert!(remote_listed
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row.get("tracking").is_none()));
    let mut remote_listed_keys = remote_listed[0]
        .as_object()
        .expect("remote task row")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    remote_listed_keys.sort();
    assert_eq!(
        remote_listed_keys,
        vec!["publication_state", "status", "task_id", "title"]
    );

    let remote_shown = json_output(
        root,
        &["task", "show", "RT-1", "--remote", "origin", "--json"],
    );
    assert_eq!(remote_shown["task_id"].as_str(), Some("RT-1"));
    assert!(remote_shown.get("tracking").is_none());
    let remote_show_keys = remote_shown
        .as_object()
        .expect("remote task")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(remote_show_keys, vec!["task_id"]);

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().all(|row| row.method == "GET"));
}
