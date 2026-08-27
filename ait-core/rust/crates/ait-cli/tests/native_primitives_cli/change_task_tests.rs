#[test]
fn native_patchset_namespace_supports_list_show_and_select() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);

    let listed = json_output(
        &worktree,
        &["patchset", "list", "RC-1", "--json"],
    );
    let listed_rows = listed.as_array().unwrap();
    assert_eq!(listed_rows[0]["patchset_id"].as_str(), Some("RP-1"));
    assert_eq!(listed_rows[0]["patchset_number"].as_i64(), Some(1));

    let shown = json_output(
        &worktree,
        &[
            "patchset",
            "show",
            "RP-1",
            "--json",
        ],
    );
    assert_eq!(shown["patchset_id"].as_str(), Some("RP-1"));
    assert_eq!(shown["change_id"].as_str(), Some("RC-1"));
    assert_eq!(shown["summary"].as_str(), Some("Native Rust patchset"));

    let selected = json_output(
        &worktree,
        &["patchset", "select", "RP-1", "--json"],
    );
    assert_eq!(selected["change_id"].as_str(), Some("RC-1"));
    assert_eq!(selected["selected_patchset_id"].as_str(), Some("RP-1"));

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets"));
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url == "/v1/native/repository-authorities/7/patchsets/RP-1"));
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
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand 'complete'"));
}

#[test]
fn compact_agent_action_json_drives_the_complete_local_task_loop() {
    let temp = init_repo("https://example.test");
    let root = temp.path();

    let started = compact_json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Compact agent action",
            "--intent",
            "exercise compact JSON through local land",
            "--json",
        ],
    );
    assert_eq!(started["contract"], "ait-agent-action/v1");
    assert_eq!(started["command"], "task.start");
    assert_eq!(started["task_id"], "LT-0001");
    assert_eq!(started["change_ref"], "LT-0001/C-01");
    let edit_root = PathBuf::from(
        started["edit_root"]
            .as_str()
            .expect("compact Task start physical edit root"),
    );
    assert!(edit_root.is_dir());
    assert!(started["next_action"]["command"]
        .as_str()
        .is_some_and(|command| command.contains(edit_root.to_string_lossy().as_ref())));
    assert!(encode_json_pretty(&started).len() < 1_500);

    write_file(
        &edit_root.join("src/compact_agent_action.rs"),
        "pub fn compact_agent_action() -> bool { true }\n",
    );
    let snapshot = compact_json_output(
        &edit_root,
        &[
            "snapshot",
            "create",
            "--message",
            "compact action snapshot",
            "--json",
        ],
    );
    assert_eq!(snapshot["contract"], "ait-agent-action/v1");
    assert_eq!(snapshot["command"], "snapshot.create");
    assert!(snapshot["snapshot_id"].as_str().is_some());
    assert!(snapshot.get("files").is_none());
    assert!(encode_json_pretty(&snapshot).len() < 900);

    let landed = compact_json_output(
        &edit_root,
        &["task", "finish", "LT-0001", "--local", "--json"],
    );
    assert_eq!(landed["contract"], "ait-agent-action/v1");
    assert_eq!(landed["command"], "task.finish");
    assert_eq!(landed["ok"], true);
    assert_eq!(landed["task_id"], "LT-0001");
    assert_eq!(landed["change_ref"], "LT-0001/C-01");
    assert_eq!(landed["closeout"]["status"], "complete_unbound");
    assert!(landed["next_action"].is_null());
    assert!(landed.get("task").is_none());
    assert!(encode_json_pretty(&landed).len() < 1_500);

    let status = compact_json_output(root, &["status", "--json"]);
    assert_eq!(status["contract"], "ait-agent-action/v1");
    assert_eq!(status["command"], "status");
    assert_eq!(status["line_name"], "main");
    assert_eq!(status["workspace"]["dirty"], false);
    assert!(status.get("snapshot_count").is_none());
    assert!(encode_json_pretty(&status).len() < 1_200);

    let full_status = json_output(root, &["status", "--json"]);
    assert_eq!(full_status["current_line"], "main");
    assert!(full_status["snapshot_count"].as_u64().is_some());
    assert!(full_status.get("contract").is_none());
}

#[test]
fn task_finish_creates_one_final_snapshot_for_dirty_local_work_and_reuses_it_on_retry() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    let started = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Integrated final Snapshot",
            "--intent",
            "prove Task finish creates exactly one final Snapshot",
            "--json",
        ],
    );
    let edit_root = PathBuf::from(started["worktree"]["path"].as_str().unwrap());
    write_file(
        &edit_root.join("src/task_finish_snapshot.rs"),
        "pub fn task_finish_snapshot() -> bool { true }\n",
    );

    let before_count = json_output(&edit_root, &["snapshot", "list", "--json"])
        .as_array()
        .unwrap()
        .len();
    let finished = json_output(
        &edit_root,
        &[
            "task",
            "finish",
            "LT-0001",
            "--message",
            "Create the final Snapshot in Task finish",
            "--local",
            "--json",
            "--full",
        ],
    );
    let created_snapshot_id = finished["auto_snapshot"]["snapshot_id"]
        .as_str()
        .expect("dirty Task finish must expose its created Snapshot")
        .to_string();
    assert_eq!(
        finished["auto_snapshot"]["message"].as_str(),
        Some("Create the final Snapshot in Task finish")
    );
    assert_eq!(
        finished["landed_snapshot_id"].as_str(),
        Some(created_snapshot_id.as_str())
    );
    assert!(!edit_root.exists());

    let after_count = json_output(root, &["snapshot", "list", "--json"])
        .as_array()
        .unwrap()
        .len();
    assert_eq!(after_count, before_count + 1);

    let retried = json_output(
        root,
        &[
            "task",
            "finish",
            "LT-0001/C-01",
            "--local",
            "--json",
            "--full",
        ],
    );
    assert_eq!(retried["execution_status"].as_str(), Some("already_landed"));
    assert_eq!(
        retried["landed_snapshot_id"].as_str(),
        Some(created_snapshot_id.as_str())
    );
    let retry_count = json_output(root, &["snapshot", "list", "--json"])
        .as_array()
        .unwrap()
        .len();
    assert_eq!(retry_count, after_count);
}

#[test]
fn task_finish_rejects_snapshot_message_before_remote_contact() {
    let temp = init_repo("http://127.0.0.1:1");
    let output = cargo_bin()
        .current_dir(temp.path())
        .args([
            "task",
            "finish",
            "RT-1",
            "--message",
            "must stay local",
            "--remote",
            "origin",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(
        "`--message` is available only for local `ait task finish`; remote finish consumes an already-ready selected Patchset."
    ));
}

#[test]
fn task_finish_retry_after_post_snapshot_failure_does_not_duplicate_snapshot() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    let base_snapshot_id = local_line_head(root, "main").expect("initialized main head");
    let original = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"divergent target\" }\n",
    );
    let divergent_target_snapshot_id = seed_snapshot(root, "Divergent target Snapshot");
    write_file(&root.join("src/lib.rs"), &original);
    seed_binary_line(root, "main", &base_snapshot_id);
    assert_eq!(
        json_output(root, &["status", "--json"])["workspace_status"].as_str(),
        Some("clean")
    );

    let started = json_output(
        root,
        &[
            "task",
            "start",
            "--local",
            "--title",
            "Post-Snapshot retry",
            "--intent",
            "prove retry reuses the final Snapshot after a later failure",
            "--json",
        ],
    );
    let worktree = PathBuf::from(started["worktree"]["path"].as_str().unwrap());
    write_file(
        &worktree.join("src/task_finish_retry.rs"),
        "pub fn task_finish_retry() -> bool { true }\n",
    );
    let before_count = json_output(&worktree, &["snapshot", "list", "--json"])
        .as_array()
        .unwrap()
        .len();

    seed_binary_line(root, "main", &divergent_target_snapshot_id);
    let first = cargo_bin()
        .current_dir(&worktree)
        .args([
            "task",
            "finish",
            "LT-0001",
            "--message",
            "Snapshot before injected Line failure",
            "--local",
            "--json",
            "--full",
        ])
        .output()
        .unwrap();
    assert!(!first.status.success());
    assert!(String::from_utf8_lossy(&first.stderr).contains("does not descend"));

    let after_first_count = json_output(&worktree, &["snapshot", "list", "--json"])
        .as_array()
        .unwrap()
        .len();
    assert_eq!(after_first_count, before_count + 1);
    let created = json_output(&worktree, &["snapshot", "list", "--json"])
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            row["message"].as_str() == Some("Snapshot before injected Line failure")
        })
        .count();
    assert_eq!(created, 1);

    let second = cargo_bin()
        .current_dir(&worktree)
        .args([
            "task",
            "finish",
            "LT-0001/C-01",
            "--message",
            "Snapshot before injected Line failure",
            "--local",
            "--json",
            "--full",
        ])
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("does not descend"));
    let after_retry_count = json_output(&worktree, &["snapshot", "list", "--json"])
        .as_array()
        .unwrap()
        .len();
    assert_eq!(after_retry_count, after_first_count);
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
    assert_eq!(audit["audit_source"]["mode"].as_str(), Some("local"));
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
fn native_task_start_from_emits_one_json_document_and_compact_text_output() {
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
            "Prove non-TTY output stays compact without a verbosity option",
            "--local",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ait task start"));
    assert!(stdout.contains("task: LT-"));
    assert!(stdout.contains("next: cd "));
    assert!(!stdout.contains("synchronizing Plan source:"));
    assert!(!stdout.contains("worktree ready:"));
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
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    write_file(
        &remote_root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"atomic remote base\" }\n",
    );
    let remote_snapshot_id = seed_snapshot(remote_root, "atomic remote base");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_snapshot_id);
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    let local_head_before = local_line_head(root, "main");
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(remote_snapshot_id.clone());
        guard.zstd_import_fixture = Some(remote_zstd);
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
            "--json",
        ],
    );

    assert_eq!(payload["task_id"], "RT-ATOMIC");
    assert_eq!(payload["change"]["change_id"], "C-01");
    assert_eq!(
        payload["change"]["fork_snapshot_id"].as_str(),
        Some(remote_snapshot_id.as_str()),
    );
    assert_eq!(local_line_head(root, "main"), local_head_before);
    let worktree = PathBuf::from(payload["worktree"]["open_path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        "pub fn example() -> &'static str { \"atomic remote base\" }\n",
    );
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
    assert_zstd_snapshot_download_logged(&logged, &remote_snapshot_id);
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
fn native_remote_task_start_from_seeds_local_main_before_atomic_task_change() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    let local_head_before = local_line_head(root, "main");
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
        &root.join("docs/sprints/atomic-empty-start.md"),
        r#"# Atomic Local Seed Start [plan-ref: atomic-empty-start]

- [ ] Start the first remote Task from the existing local main. [ref: atomic-empty-start/implement]
"#,
    );

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/atomic-empty-start.md#atomic-empty-start/implement",
            "--intent",
            "Seed the null remote Line from local main before atomic Task and Change creation",
            "--json",
        ],
    );

    let seed_snapshot_id = payload["change"]["fork_snapshot_id"]
        .as_str()
        .expect("atomic Change must fork from the local seed");
    assert_eq!(payload["task_id"], "RT-ATOMIC");
    assert_eq!(Some(seed_snapshot_id), local_head_before.as_deref());
    assert_eq!(
        payload["worktree"]["fork_snapshot_id"].as_str(),
        Some(seed_snapshot_id)
    );
    assert_eq!(
        payload["worktree"]["head_snapshot_id"].as_str(),
        Some(seed_snapshot_id)
    );
    assert_eq!(local_line_head(root, "main"), local_head_before);
    assert_eq!(
        state.lock().unwrap().remote_head_snapshot_id.as_deref(),
        Some(seed_snapshot_id)
    );
    let worktree = PathBuf::from(payload["worktree"]["open_path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        "pub fn example() -> &'static str { \"ok\" }\n"
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let commit_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url
                    == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
        })
        .expect("expected local-seed zstd commit");
    let atomic_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/task-start"
        })
        .expect("expected atomic Task start");
    assert!(commit_index < atomic_index);
    let commit = parse_json(&logged[commit_index].body);
    assert_eq!(
        commit["line_update"]["head_snapshot_id"].as_str(),
        Some(seed_snapshot_id)
    );
    assert!(commit["line_update"]["expected_head_snapshot_id"].is_null());
    assert_eq!(
        commit["snapshots"][0]["snapshot_id"].as_str(),
        Some(seed_snapshot_id)
    );
    assert!(commit["snapshots"][0]["file_count"].as_i64().unwrap() > 0);
    assert_eq!(commit["snapshots"][0]["parent_snapshot_ids"], json!([]));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
    }));
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
        "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n",
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
        .starts_with("# Managed by ait: workspace-isolated final artifacts and intermediates."));
    assert!(projected_cargo_config
        .contains("[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n"));
    assert!(payload["worktree"]["cargo_target_dir"].is_string());
    assert!(payload["worktree"]["cargo_build_dir"].is_string());
    let cargo_target_dir = payload["worktree"]["cargo_target_dir"]
        .as_str()
        .expect("Task Cargo target dir");
    let cargo_build_dir = payload["worktree"]["cargo_build_dir"]
        .as_str()
        .expect("Task Cargo build dir");
    assert!(cargo_target_dir.contains("/cargo-target/task-workspaces/"));
    assert!(cargo_build_dir.contains("/cargo-build/task-workspaces/"));
    assert_ne!(cargo_target_dir, cargo_build_dir);
    assert!(projected_cargo_config.contains(&format!(
        "target-dir = \"{cargo_target_dir}\""
    )));
    assert!(projected_cargo_config.contains(&format!(
        "build-dir = \"{cargo_build_dir}\""
    )));
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
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    write_file(
        &remote_root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"remote ahead\" }\n",
    );
    let remote_snapshot_id = seed_snapshot(remote_root, "remote ahead");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_snapshot_id);
    let (base_url, log, state, handle) = spawn_fake_remote();
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(remote_snapshot_id.clone());
        guard.zstd_import_fixture = Some(remote_zstd);
    }
    let temp = init_repo(&base_url);
    let root = temp.path();
    let local_head_before = local_line_head(root, "main");

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--title",
            "Use imported remote base",
            "--intent",
            "start shared work from the imported Remote head",
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
        local_head_before.as_deref(),
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
    assert_zstd_snapshot_download_logged(&logged, &remote_snapshot_id);
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
fn native_task_start_seeds_a_null_remote_base_from_local_main() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    let local_head_before = local_line_head(root, "main");

    let payload = json_output(
        root,
        &[
            "task",
            "start",
            "--title",
            "Use existing local main",
            "--intent",
            "prevent a fresh Remote Line from creating independent ancestry",
            "--json",
        ],
    );

    let seed_snapshot_id = payload["change"]["fork_snapshot_id"]
        .as_str()
        .expect("remote Change must fork from the local seed");
    assert_eq!(Some(seed_snapshot_id), local_head_before.as_deref());
    assert_eq!(
        payload["worktree"]["fork_snapshot_id"].as_str(),
        Some(seed_snapshot_id)
    );
    assert_eq!(
        payload["worktree"]["head_snapshot_id"].as_str(),
        Some(seed_snapshot_id)
    );
    assert_eq!(
        local_line_head(root, "main"),
        local_head_before,
        "local-main seeding must preserve the local Line",
    );
    let worktree = PathBuf::from(payload["worktree"]["open_path"].as_str().unwrap());
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        "pub fn example() -> &'static str { \"ok\" }\n",
        "the Remote Task must materialize the existing local main",
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let zstd_commit_index = logged
        .iter()
        .position(|row| {
            row.method == "POST"
                && row.url
                    == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
        })
        .expect("expected atomic local-seed zstd commit");
    let task_create_index = logged
        .iter()
        .position(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks"
        })
        .expect("expected Task create");
    let change_create_index = logged
        .iter()
        .position(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
        })
        .expect("expected Change create");
    assert!(zstd_commit_index < task_create_index);
    assert!(zstd_commit_index < change_create_index);
    let zstd_commit = parse_json(&logged[zstd_commit_index].body);
    assert_eq!(
        zstd_commit["line_update"]["line_name"].as_str(),
        Some("main")
    );
    assert_eq!(
        zstd_commit["line_update"]["head_snapshot_id"].as_str(),
        Some(seed_snapshot_id)
    );
    assert!(zstd_commit["line_update"]["expected_head_snapshot_id"].is_null());
    let committed_seed = zstd_commit["snapshots"]
        .as_array()
        .and_then(|rows| rows.first())
        .expect("local seed Snapshot commit row");
    assert_eq!(
        committed_seed["snapshot_id"].as_str(),
        Some(seed_snapshot_id)
    );
    assert!(committed_seed["file_count"].as_i64().unwrap() > 0);
    assert!(committed_seed["total_bytes"].as_i64().unwrap() > 0);
    assert_eq!(committed_seed["parent_snapshot_ids"], json!([]));
    let change_create = logged
        .iter()
        .find(|row| {
            row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"
        })
        .unwrap();
    assert_eq!(
        parse_json(&change_create.body)["fork_snapshot_id"].as_str(),
        Some(seed_snapshot_id)
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
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`ait task start` cannot run inside existing worktree"));
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

    let abandoned = json_output(
        root,
        &["task", "abandon", "LT-0001", "--local", "--json"],
    );
    assert_eq!(abandoned["status"].as_str(), Some("abandoned"));
    let bounded_local = json_output(root, &["task", "list", "--local", "--json"]);
    assert!(bounded_local.as_array().unwrap().is_empty());
    let all_local = json_output(
        root,
        &["task", "list", "--local", "--all", "--json"],
    );
    assert_eq!(all_local.as_array().unwrap().len(), 1);
    assert_eq!(all_local[0]["task_id"].as_str(), Some("LT-0001"));
    assert_eq!(all_local[0]["status"].as_str(), Some("abandoned"));
    let bounded_text = cargo_bin()
        .current_dir(root)
        .args(["task", "list", "--local"])
        .output()
        .unwrap();
    assert!(bounded_text.status.success());
    assert!(!String::from_utf8_lossy(&bounded_text.stdout).contains("LT-0001"));
    let all_text = cargo_bin()
        .current_dir(root)
        .args(["task", "list", "--local", "--all"])
        .output()
        .unwrap();
    assert!(all_text.status.success());
    assert!(String::from_utf8_lossy(&all_text.stdout).contains("LT-0001"));
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
