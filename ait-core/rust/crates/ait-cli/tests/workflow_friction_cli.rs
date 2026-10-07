//! End-to-end coverage for the workflow friction repairs: `hooks.post_finish`,
//! `ait task quick`, Plan-backed Markdown materialization after a local
//! finish, and exact repair commands for deleted Plan-backed Markdown.

use ait_core::json_support::{JsonCodec, JsonValue};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ait-cli"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn json(root: &Path, args: &[&str]) -> JsonValue {
    let output = run(root, args);
    assert!(output.status.success(), "{args:?}\n{}", text(&output));
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid CLI JSON").unwrap()
}

/// Symlink-free base for explicit edit roots (macOS `/var` -> `/private/var`).
fn scratch(temp: &TempDir) -> PathBuf {
    temp.path().canonicalize().unwrap()
}

fn initialized() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap().join("repo");
    fs::create_dir(&root).unwrap();
    json(&root, &["init", "--json"]);
    json(
        &root,
        &[
            "config",
            "set",
            "--user-name",
            "friction-regression",
            "--json",
        ],
    );
    (temp, root)
}

fn quick_start(root: &Path, edit_root: &Path, title: &str, intent: &str) -> (String, JsonValue) {
    let started = json(
        root,
        &[
            "task",
            "quick",
            "--local",
            "--title",
            title,
            "--intent",
            intent,
            "--edit-root",
            edit_root.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    (started["task_id"].as_str().unwrap().to_string(), started)
}

fn finish_local(edit_root: &Path, task_id: &str, message: &str) -> Output {
    run(
        edit_root,
        &[
            "task",
            "finish",
            task_id,
            "--message",
            message,
            "--local",
            "--json",
            "--full",
        ],
    )
}

fn finished_payload(output: &Output) -> JsonValue {
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid finish JSON").unwrap()
}

#[test]
fn task_quick_generates_a_bound_card_and_finishes_through_the_normal_path() {
    let (temp, root) = initialized();
    let edit_root = scratch(&temp).join("quick-edit");
    let (task_id, started) = quick_start(
        &root,
        &edit_root,
        "Refresh the release badge",
        "Point the README badge at the current release",
    );
    assert_eq!(
        started["quick_card"]["artifact_path"].as_str(),
        Some("docs/sprints/quick-refresh-the-release-badge.md")
    );
    assert_eq!(
        started["quick_card"]["item_ref"].as_str(),
        Some("quick-refresh-the-release-badge/task")
    );
    assert_eq!(
        started["quick_card"]["reused_existing_card"],
        JsonValue::Bool(false)
    );
    let card =
        fs::read_to_string(root.join("docs/sprints/quick-refresh-the-release-badge.md")).unwrap();
    assert!(
        card.contains("[plan-ref: quick-refresh-the-release-badge/root]"),
        "{card}"
    );
    assert!(
        card.contains(
            "- [ ] Refresh the release badge [ref: quick-refresh-the-release-badge/task]"
        ),
        "{card}"
    );
    assert!(
        card.contains("Scope: Point the README badge at the current release"),
        "{card}"
    );
    assert_eq!(started["worktree"]["path"].as_str(), edit_root.to_str());

    let compact = json(&root, &["task", "show", &task_id, "--local", "--json"]);
    assert_eq!(compact["title"].as_str(), Some("Refresh the release badge"));

    fs::write(edit_root.join("badge.txt"), "1.1.4\n").unwrap();
    let finished = finish_local(&edit_root, &task_id, "Refresh badge");
    assert!(finished.status.success(), "{}", text(&finished));
    let payload = finished_payload(&finished);
    assert_eq!(payload["task_status"].as_str(), Some("completed"));
    assert_eq!(
        payload["plan_checklist_closeout"]["status"].as_str(),
        Some("synced"),
        "{}",
        payload["plan_checklist_closeout"]
    );
    let card =
        fs::read_to_string(root.join("docs/sprints/quick-refresh-the-release-badge.md")).unwrap();
    assert!(card.contains("- [x] Refresh the release badge"), "{card}");
    assert_eq!(
        payload["post_finish_hooks"]["status"].as_str(),
        Some("skipped")
    );
    assert_eq!(
        payload["plan_markdown_materialization"]["status"].as_str(),
        Some("in_sync"),
        "{}",
        payload["plan_markdown_materialization"]
    );
}

#[test]
fn task_quick_text_output_names_the_generated_card_and_reuses_it_on_rerun() {
    let (temp, root) = initialized();
    let edit_root = scratch(&temp).join("quick-text");
    let output = run(
        &root,
        &[
            "task",
            "quick",
            "--local",
            "--intent",
            "Tighten the doctor output wording",
            "--edit-root",
            edit_root.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{}", text(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("card: docs/sprints/quick-tighten-the-doctor-output-wording.md"),
        "{stdout}"
    );
    assert!(
        stdout.contains("title: Tighten the doctor output wording"),
        "{stdout}"
    );
    let task_id = stdout
        .lines()
        .find_map(|line| line.strip_prefix("task: "))
        .unwrap()
        .trim()
        .to_string();
    json(&root, &["task", "abandon", &task_id, "--local", "--json"]);
    // The card is Plan-tracked and byte-identical, so a rerun reuses it.
    let second_root = scratch(&temp).join("quick-text-2");
    let (_, started) = quick_start(
        &root,
        &second_root,
        "Tighten the doctor output wording",
        "Tighten the doctor output wording",
    );
    assert_eq!(
        started["quick_card"]["reused_existing_card"],
        JsonValue::Bool(true)
    );
}

#[test]
fn task_quick_requires_sprint_mode() {
    let (temp, root) = initialized();
    json(&root, &["config", "set", "--sprint", "off", "--json"]);
    let output = run(
        &root,
        &[
            "task",
            "quick",
            "--intent",
            "Anything",
            "--edit-root",
            scratch(&temp).join("never").to_str().unwrap(),
        ],
    );
    assert!(!output.status.success());
    let rendered = text(&output);
    assert!(rendered.contains("requires sprint mode"), "{rendered}");
    assert!(rendered.contains("ait task start --title"), "{rendered}");
    let generated = fs::read_dir(root.join("docs/sprints"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_name().to_string_lossy().starts_with("quick-"))
                .count()
        })
        .unwrap_or(0);
    assert_eq!(
        generated, 0,
        "no card may be generated when sprint mode is off"
    );
}

#[cfg(unix)]
#[test]
fn post_finish_hooks_run_in_the_repository_root_after_a_local_finish() {
    let (temp, root) = initialized();
    let marker = scratch(&temp).join("hook-marker.txt");
    let hooks = format!(
        r#"[{{"run":["sh","-c","printf '%s|%s|%s' \"$AIT_TASK_ID\" \"$AIT_TARGET_LINE\" \"$(pwd)\" > {}"]}}]"#,
        marker.display()
    );
    let shown = json(
        &root,
        &["config", "set", "--post-finish-hooks", &hooks, "--json"],
    );
    assert_eq!(shown["hooks"]["post_finish_count"], JsonValue::from(1u64));
    assert_eq!(
        shown["hooks"]["post_finish"][0]["on_failure"].as_str(),
        Some("warn")
    );
    let config_text = run(&root, &["config", "set", "--post-finish-hooks", &hooks]);
    assert!(
        text(&config_text).contains("post-finish-hooks"),
        "{}",
        text(&config_text)
    );

    let edit_root = scratch(&temp).join("hook-edit");
    let (task_id, _) = quick_start(&root, &edit_root, "Exercise post finish hooks", "hooks");
    fs::write(edit_root.join("hooked.txt"), "yes\n").unwrap();
    let finished = finish_local(&edit_root, &task_id, "Hooked finish");
    assert!(finished.status.success(), "{}", text(&finished));
    let payload = finished_payload(&finished);
    assert_eq!(
        payload["post_finish_hooks"]["status"].as_str(),
        Some("complete"),
        "{}",
        payload["post_finish_hooks"]
    );
    assert_eq!(
        payload["post_finish_hooks"]["hooks"][0]["exit_code"],
        JsonValue::from(0i64)
    );
    let recorded = fs::read_to_string(&marker).unwrap();
    let parts = recorded.split('|').collect::<Vec<_>>();
    assert_eq!(parts[0], task_id);
    assert_eq!(parts[1], "main");
    assert_eq!(
        fs::canonicalize(parts[2]).unwrap(),
        fs::canonicalize(&root).unwrap(),
        "hooks must run in the canonical repository root"
    );

    // The compact contract also reports the hook outcome.
    let compact_output = run(&root, &["task", "finish", &task_id, "--local", "--json"]);
    let compact = finished_payload(&compact_output);
    assert_eq!(compact["contract"].as_str(), Some("ait-agent-action/v1"));
    assert!(compact.get("post_finish_hooks").is_some());

    let cleared = json(&root, &["config", "unset", "post-finish-hooks", "--json"]);
    assert_eq!(cleared["hooks"]["post_finish_count"], JsonValue::from(0u64));
    assert_eq!(cleared["hooks"]["source"].as_str(), Some("unset"));
}

#[cfg(unix)]
#[test]
fn post_finish_hook_fail_policy_keeps_the_finish_but_changes_the_exit_code() {
    let (temp, root) = initialized();
    json(
        &root,
        &[
            "config",
            "set",
            "--post-finish-hooks",
            r#"[{"run":["sh","-c","echo hook exploded >&2; exit 9"],"on_failure":"fail"}]"#,
            "--json",
        ],
    );
    let edit_root = scratch(&temp).join("fail-edit");
    let (task_id, _) = quick_start(&root, &edit_root, "Exercise failing hook", "fail hook");
    fs::write(edit_root.join("failing.txt"), "yes\n").unwrap();
    let finished = run(
        &edit_root,
        &[
            "task",
            "finish",
            &task_id,
            "--message",
            "Failing hook",
            "--local",
        ],
    );
    assert_eq!(finished.status.code(), Some(3), "{}", text(&finished));
    let stdout = String::from_utf8_lossy(&finished.stdout);
    assert!(
        stdout.contains("hooks: failed (1 configured, 1 failed)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("hook failed (fail): sh -c echo hook exploded >&2; exit 9 -> exit 9"),
        "{stdout}"
    );
    assert!(stdout.contains("hook output: hook exploded"), "{stdout}");
    assert!(
        stdout.starts_with(&format!("finished: {task_id} -> main @ SNP-")),
        "{stdout}"
    );
    let shown = json(&root, &["task", "show", &task_id, "--local", "--json"]);
    assert_eq!(shown["status"].as_str(), Some("completed"));
    assert!(!edit_root.exists(), "worktree cleanup must still happen");
}

#[test]
fn invalid_post_finish_hooks_are_rejected_at_config_time() {
    let (_temp, root) = initialized();
    for (value, expected) in [
        (r#"{"run":["x"]}"#, "must be a JSON array"),
        (r#"[{"run":[]}]"#, "non-empty argv array"),
        (
            r#"[{"run":["x"],"on_failure":"retry"}]"#,
            "`warn` or `fail`",
        ),
        ("not json", "--post-finish-hooks"),
    ] {
        let output = run(&root, &["config", "set", "--post-finish-hooks", value]);
        assert!(!output.status.success(), "{value}");
        let rendered = text(&output);
        assert!(rendered.contains(expected), "{value}: {rendered}");
    }
    let shown = json(&root, &["config", "show", "--json"]);
    assert_eq!(shown["hooks"]["post_finish_count"], JsonValue::from(0u64));
}

#[test]
fn local_finish_materializes_plan_markdown_synced_from_the_worktree() {
    let (temp, root) = initialized();
    fs::create_dir_all(root.join("notes")).unwrap();
    let notes = root.join("notes/guide.md");
    fs::write(
        &notes,
        "# Operator notes [plan-ref: notes/root]\n\nFirst version.\n\n- [ ] Keep the notes current [ref: notes/root/keep]\n",
    )
    .unwrap();
    json(
        &root,
        &["plan", "sync", "notes/guide.md", "--local", "--json"],
    );

    let edit_root = scratch(&temp).join("notes-edit");
    let (task_id, _) = quick_start(&root, &edit_root, "Update operator notes", "notes");
    fs::write(edit_root.join("src.rs"), "fn notes() {}\n").unwrap();
    let edited = "# Operator notes [plan-ref: notes/root]\n\nSecond version written in the worktree.\n\n- [ ] Keep the notes current [ref: notes/root/keep]\n";
    fs::create_dir_all(edit_root.join("notes")).unwrap();
    fs::write(edit_root.join("notes/guide.md"), edited).unwrap();
    json(
        &edit_root,
        &["plan", "sync", "notes/guide.md", "--local", "--json"],
    );
    assert!(
        fs::read_to_string(&notes)
            .unwrap()
            .contains("First version."),
        "the canonical root copy is untouched before finish"
    );

    let finished = finish_local(&edit_root, &task_id, "Update notes");
    assert!(finished.status.success(), "{}", text(&finished));
    let payload = finished_payload(&finished);
    let materialization = &payload["plan_markdown_materialization"];
    assert_eq!(
        materialization["status"].as_str(),
        Some("complete"),
        "{materialization}"
    );
    assert_eq!(
        materialization["materialized"][0]["artifact_path"].as_str(),
        Some("notes/guide.md")
    );
    assert_eq!(
        materialization["materialized"][0]["previous_state"].as_str(),
        Some("earlier_revision")
    );
    assert_eq!(fs::read_to_string(&notes).unwrap(), edited);
    assert!(!edit_root.exists());

    let rendered = run(&root, &["task", "finish", &task_id, "--local"]);
    assert!(rendered.status.success(), "{}", text(&rendered));
}

#[test]
fn local_finish_reports_root_markdown_drift_instead_of_overwriting_it() {
    let (temp, root) = initialized();
    fs::create_dir_all(root.join("notes")).unwrap();
    let notes = root.join("notes/guarded.md");
    fs::write(
        &notes,
        "# Guarded notes [plan-ref: guarded/root]\n\nSynced body.\n\n- [ ] Guard the notes [ref: guarded/root/guard]\n",
    )
    .unwrap();
    json(
        &root,
        &["plan", "sync", "notes/guarded.md", "--local", "--json"],
    );
    let edit_root = scratch(&temp).join("guarded-edit");
    let (task_id, _) = quick_start(&root, &edit_root, "Guard root drift", "guard");
    fs::write(edit_root.join("src.rs"), "fn guard() {}\n").unwrap();
    fs::create_dir_all(edit_root.join("notes")).unwrap();
    fs::write(
        edit_root.join("notes/guarded.md"),
        "# Guarded notes [plan-ref: guarded/root]\n\nWorktree body.\n\n- [ ] Guard the notes [ref: guarded/root/guard]\n",
    )
    .unwrap();
    json(
        &edit_root,
        &["plan", "sync", "notes/guarded.md", "--local", "--json"],
    );
    // Meanwhile someone edits the root copy without syncing it.
    let root_edit = "# Guarded notes [plan-ref: guarded/root]\n\nUnsynced root edit.\n\n- [ ] Guard the notes [ref: guarded/root/guard]\n";
    fs::write(&notes, root_edit).unwrap();

    let finished = finish_local(&edit_root, &task_id, "Guarded finish");
    assert!(finished.status.success(), "{}", text(&finished));
    let payload = finished_payload(&finished);
    let materialization = &payload["plan_markdown_materialization"];
    assert_eq!(
        materialization["status"].as_str(),
        Some("partial"),
        "{materialization}"
    );
    assert_eq!(
        materialization["drift"][0]["artifact_path"].as_str(),
        Some("notes/guarded.md")
    );
    assert_eq!(
        materialization["drift"][0]["command"].as_str(),
        Some("ait plan sync notes/guarded.md --local")
    );
    assert_eq!(
        fs::read_to_string(&notes).unwrap(),
        root_edit,
        "unsynced root drift is never overwritten"
    );
}

#[test]
fn deleted_plan_backed_markdown_reports_the_exact_prune_command() {
    let (temp, root) = initialized();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(
        root.join("docs/retired.md"),
        "# Retired notes [plan-ref: retired/root]\n\nGone soon.\n\n- [ ] Retire the notes [ref: retired/root/retire]\n",
    )
    .unwrap();
    json(
        &root,
        &["plan", "sync", "docs/retired.md", "--local", "--json"],
    );
    fs::remove_file(root.join("docs/retired.md")).unwrap();

    let plain_sync = run(&root, &["plan", "sync", "docs/retired.md", "--local"]);
    assert!(!plain_sync.status.success());
    let rendered = text(&plain_sync);
    assert!(
        rendered.contains("ait plan sync docs/retired.md --prune --local"),
        "{rendered}"
    );

    let blocked = run(
        &root,
        &[
            "task",
            "quick",
            "--local",
            "--intent",
            "Blocked by deleted markdown",
            "--edit-root",
            scratch(&temp).join("blocked").to_str().unwrap(),
        ],
    );
    assert!(!blocked.status.success());
    let rendered = text(&blocked);
    assert!(rendered.contains("authored Markdown drift"), "{rendered}");
    assert!(
        rendered.contains("`ait plan sync docs/retired.md --prune --local` (file was deleted)"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Planning-only paths: docs/retired.md"),
        "{rendered}"
    );

    json(
        &root,
        &[
            "plan",
            "sync",
            "docs/retired.md",
            "--prune",
            "--local",
            "--json",
        ],
    );
    let edit_root = scratch(&temp).join("unblocked");
    let (task_id, _) = quick_start(&root, &edit_root, "Unblocked after prune", "prune");
    json(&root, &["task", "abandon", &task_id, "--local", "--json"]);
}

#[test]
fn modified_plan_backed_markdown_reports_a_plain_sync_command() {
    let (temp, root) = initialized();
    fs::create_dir_all(root.join("docs")).unwrap();
    let path = root.join("docs/edited.md");
    fs::write(
        &path,
        "# Edited notes [plan-ref: edited/root]\n\nOriginal.\n\n- [ ] Edit the notes [ref: edited/root/edit]\n",
    )
    .unwrap();
    json(
        &root,
        &["plan", "sync", "docs/edited.md", "--local", "--json"],
    );
    fs::write(
        &path,
        "# Edited notes [plan-ref: edited/root]\n\nChanged.\n\n- [ ] Edit the notes [ref: edited/root/edit]\n",
    )
    .unwrap();
    let blocked = run(
        &root,
        &[
            "task",
            "quick",
            "--local",
            "--intent",
            "Blocked by edited markdown",
            "--edit-root",
            scratch(&temp).join("blocked").to_str().unwrap(),
        ],
    );
    assert!(!blocked.status.success());
    let rendered = text(&blocked);
    assert!(
        rendered.contains("`ait plan sync docs/edited.md --local`"),
        "{rendered}"
    );
    assert!(!rendered.contains("--prune"), "{rendered}");
}

#[test]
fn plan_sync_from_a_worktree_accepts_the_shared_docs_link() {
    let (temp, root) = initialized();
    let edit_root = scratch(&temp).join("docs-sync-edit");
    let (task_id, started) = quick_start(&root, &edit_root, "Sync docs from worktree", "docs sync");
    let card = started["quick_card"]["artifact_path"]
        .as_str()
        .unwrap()
        .to_string();
    let card_in_worktree = edit_root.join(&card);
    assert!(
        card_in_worktree.exists(),
        "the shared docs link exposes the card"
    );
    let mut markdown = fs::read_to_string(&card_in_worktree).unwrap();
    markdown.push_str("\nExtra note written from the worktree.\n");
    fs::write(&card_in_worktree, &markdown).unwrap();
    let synced = json(&edit_root, &["plan", "sync", &card, "--local", "--json"]);
    assert_eq!(synced["status"].as_str(), Some("ok"), "{synced}");
    assert_eq!(
        synced["results"][0]["action"].as_str(),
        Some("updated"),
        "{synced}"
    );
    assert_eq!(fs::read_to_string(root.join(&card)).unwrap(), markdown);
    fs::write(edit_root.join("code.rs"), "fn docs_sync() {}\n").unwrap();
    let finished = finish_local(&edit_root, &task_id, "Docs synced from worktree");
    assert!(finished.status.success(), "{}", text(&finished));
    let payload = finished_payload(&finished);
    assert_eq!(
        payload["plan_checklist_closeout"]["status"].as_str(),
        Some("synced"),
        "{}",
        payload["plan_checklist_closeout"]
    );
}

#[test]
fn task_quick_names_plan_backed_markdown_and_plan_list_reports_head_match() {
    let (temp, root) = initialized();
    fs::create_dir_all(root.join("notes")).unwrap();
    fs::write(
        root.join("notes/tracked.md"),
        "# Tracked notes [plan-ref: tracked/root]\n\nBody.\n\n- [ ] Track [ref: tracked/root/track]\n",
    )
    .unwrap();
    json(
        &root,
        &["plan", "sync", "notes/tracked.md", "--local", "--json"],
    );
    let edit_root = scratch(&temp).join("plan-backed-edit");
    let output = run(
        &root,
        &[
            "task",
            "quick",
            "--local",
            "--intent",
            "Disclose plan-backed markdown at start",
            "--edit-root",
            edit_root.to_str().unwrap(),
        ],
    );
    assert!(output.status.success(), "{}", text(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("plan-backed: docs/ is a shared link to the repository root"),
        "{stdout}"
    );
    assert!(
        stdout.contains("sync with `ait plan sync <path> --local` before finish"),
        "{stdout}"
    );
    assert!(
        stdout.contains("docs/sprints/quick-disclose-plan-backed-markdown-at-start.md"),
        "{stdout}"
    );
    let task_id = stdout
        .lines()
        .find_map(|line| line.strip_prefix("task: "))
        .unwrap()
        .trim()
        .to_string();
    let started = json(&root, &["task", "show", &task_id, "--local", "--json"]);
    assert_eq!(started["task_id"].as_str(), Some(task_id.as_str()));

    let plans = json(&root, &["plan", "list", "--local", "--json"]);
    let tracked = plans
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["head_artifact_path"].as_str() == Some("notes/tracked.md"))
        .expect("tracked plan row");
    assert_eq!(
        tracked["head_artifact_matches_workspace"],
        JsonValue::Bool(true)
    );
    assert_eq!(tracked["head_match"].as_str(), Some("yes"));
    assert!(tracked["workspace_artifact_blob_id"]
        .as_str()
        .unwrap()
        .starts_with("BLB-"));
    fs::write(root.join("notes/tracked.md"), "# Tracked notes [plan-ref: tracked/root]\n\nEdited.\n\n- [ ] Track [ref: tracked/root/track]\n").unwrap();
    let plans = json(&root, &["plan", "list", "--local", "--json"]);
    let tracked = plans
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["head_artifact_path"].as_str() == Some("notes/tracked.md"))
        .unwrap();
    assert_eq!(tracked["head_match"].as_str(), Some("no"));
    let listed = run(&root, &["plan", "list", "--local"]);
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains("head_match"),
        "{}",
        text(&listed)
    );
    json(
        &root,
        &["plan", "sync", "notes/tracked.md", "--local", "--json"],
    );
    json(&root, &["task", "abandon", &task_id, "--local", "--json"]);
}

#[test]
fn compact_payloads_carry_read_only_evidence_commands() {
    let (temp, root) = initialized();
    let status = json(&root, &["status", "--json"]);
    assert_eq!(
        status["evidence"]["command"].as_str(),
        Some("ait status --json --full")
    );
    let edit_root = scratch(&temp).join("evidence-edit");
    let started = json(
        &root,
        &[
            "task",
            "quick",
            "--local",
            "--intent",
            "Carry evidence commands",
            "--edit-root",
            edit_root.to_str().unwrap(),
            "--json",
        ],
    );
    let task_id = started["task_id"].as_str().unwrap().to_string();
    assert_eq!(
        started["evidence"]["command"].as_str(),
        Some(format!("ait task show {task_id} --json").as_str())
    );
    fs::write(edit_root.join("evidence.txt"), "x\n").unwrap();
    let snapshot = json(
        &edit_root,
        &[
            "snapshot",
            "create",
            &task_id,
            "--message",
            "evidence",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap();
    assert_eq!(
        snapshot["evidence"]["command"].as_str(),
        Some(format!("ait snapshot show {snapshot_id} --json").as_str())
    );
    let finished = run(
        &edit_root,
        &["task", "finish", &task_id, "--local", "--json"],
    );
    assert!(finished.status.success(), "{}", text(&finished));
    let finished = finished_payload(&finished);
    assert_eq!(
        finished["evidence"]["command"].as_str(),
        Some(format!("ait task audit {task_id} --json").as_str())
    );
}

#[test]
fn line_show_remote_is_read_only_and_reports_unreachable_remotes() {
    let (_temp, root) = initialized();
    let output = run(
        &root,
        &["line", "show", "main", "--remote", "origin", "--json"],
    );
    assert!(!output.status.success(), "{}", text(&output));
    let local = json(&root, &["line", "show", "main", "--json"]);
    assert_eq!(local["line_name"].as_str(), Some("main"));
    assert!(local.get("remote").is_none());
}

#[test]
fn finish_sync_plan_markdown_syncs_reachable_drift_and_reports_it() {
    let (temp, root) = initialized();
    fs::create_dir_all(root.join("notes")).unwrap();
    fs::write(
        root.join("notes/presync.md"),
        "# Presync notes [plan-ref: presync/root]\n\nFirst.\n\n- [ ] Keep [ref: presync/root/keep]\n",
    )
    .unwrap();
    json(
        &root,
        &["plan", "sync", "notes/presync.md", "--local", "--json"],
    );
    let edit_root = scratch(&temp).join("presync-edit");
    let (task_id, started) = quick_start(&root, &edit_root, "Presync plan markdown", "presync");
    let card = started["quick_card"]["artifact_path"]
        .as_str()
        .unwrap()
        .to_string();
    fs::write(edit_root.join("code.rs"), "fn presync() {}\n").unwrap();
    // Regular Plan-backed file edited in the worktree, plus the shared docs card.
    fs::create_dir_all(edit_root.join("notes")).unwrap();
    let edited_notes = "# Presync notes [plan-ref: presync/root]\n\nSecond, from the worktree.\n\n- [ ] Keep [ref: presync/root/keep]\n";
    fs::write(edit_root.join("notes/presync.md"), edited_notes).unwrap();
    let mut card_markdown = fs::read_to_string(edit_root.join(&card)).unwrap();
    card_markdown.push_str("\nNote added from the worktree.\n");
    fs::write(edit_root.join(&card), &card_markdown).unwrap();

    let refused = run(
        &edit_root,
        &[
            "task",
            "finish",
            &task_id,
            "--message",
            "no flag",
            "--local",
        ],
    );
    assert!(!refused.status.success());
    assert!(
        text(&refused).contains("authored Markdown drift"),
        "{}",
        text(&refused)
    );

    let remote_refused = run(
        &edit_root,
        &[
            "task",
            "finish",
            &task_id,
            "--remote",
            "origin",
            "--sync-plan-markdown",
        ],
    );
    assert!(
        !remote_refused.status.success(),
        "{}",
        text(&remote_refused)
    );

    let finished = run(
        &edit_root,
        &[
            "task",
            "finish",
            &task_id,
            "--message",
            "presync",
            "--local",
            "--sync-plan-markdown",
            "--json",
            "--full",
        ],
    );
    assert!(finished.status.success(), "{}", text(&finished));
    let payload = finished_payload(&finished);
    let presync = &payload["plan_markdown_presync"];
    assert_eq!(presync["status"].as_str(), Some("synced"), "{presync}");
    assert_eq!(presync["count"], JsonValue::from(2u64), "{presync}");
    let synced = presync["synced"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["artifact_path"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(synced.contains(&card), "{synced:?}");
    assert!(
        synced.contains(&"notes/presync.md".to_string()),
        "{synced:?}"
    );
    assert_eq!(payload["task_status"].as_str(), Some("completed"));
    assert_eq!(
        fs::read_to_string(root.join("notes/presync.md")).unwrap(),
        edited_notes
    );
    assert!(fs::read_to_string(root.join(&card))
        .unwrap()
        .contains("Note added from the worktree."));
}

#[test]
fn finish_closes_nested_checkboxes_under_the_bound_item_only() {
    let (temp, root) = initialized();
    fs::create_dir_all(root.join("docs/sprints")).unwrap();
    let card = "docs/sprints/nested.md";
    fs::write(
        root.join(card),
        "# Nested card [plan-ref: nested/root]\n\nScope: nested steps.\n\n- [ ] Ship nested work [ref: nested/ship]\n  - [ ] step one\n  - [ ] step two\n- [ ] Unrelated follow-up [ref: nested/later]\n",
    )
    .unwrap();
    let edit_root = scratch(&temp).join("nested-edit");
    let started = json(
        &root,
        &[
            "task",
            "start",
            "--local",
            "--from",
            "docs/sprints/nested.md#nested/ship",
            "--intent",
            "Exercise nested checkbox closeout",
            "--edit-root",
            edit_root.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    let task_id = started["task_id"].as_str().unwrap().to_string();
    fs::write(edit_root.join("nested.rs"), "fn nested() {}\n").unwrap();
    let finished = finish_local(&edit_root, &task_id, "Nested closeout");
    assert!(finished.status.success(), "{}", text(&finished));
    let payload = finished_payload(&finished);
    assert_eq!(
        payload["plan_checklist_closeout"]["status"].as_str(),
        Some("synced"),
        "{}",
        payload["plan_checklist_closeout"]
    );
    assert_eq!(
        payload["plan_checklist_closeout"]["nested_closed_count"],
        JsonValue::from(2u64)
    );
    let markdown = fs::read_to_string(root.join(card)).unwrap();
    assert!(
        markdown.contains(
            "- [x] Ship nested work [ref: nested/ship]\n  - [x] step one\n  - [x] step two\n"
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains("- [ ] Unrelated follow-up [ref: nested/later]"),
        "{markdown}"
    );
}

#[cfg(unix)]
#[test]
fn tracked_pre_finish_hooks_gate_local_finish() {
    let (temp, root) = initialized();
    fs::write(
        root.join(".ait-hooks.json"),
        r#"{"pre_finish": [{"run": ["sh", "-c", "test -f gate-ok.txt || { echo gate missing >&2; exit 4; }"]}]}"#,
    )
    .unwrap();
    let shown = json(&root, &["config", "show", "--json"]);
    assert_eq!(
        shown["hooks"]["pre_finish_count"],
        JsonValue::from(1u64),
        "{}",
        shown["hooks"]
    );
    assert_eq!(
        shown["hooks"]["pre_finish"][0]["on_failure"].as_str(),
        Some("fail")
    );

    let edit_root = scratch(&temp).join("gate-edit");
    let (task_id, _) = quick_start(&root, &edit_root, "Exercise pre finish gate", "gate");
    assert!(
        edit_root.join(".ait-hooks.json").is_file(),
        "the tracked hooks file travels into the worktree"
    );
    fs::write(edit_root.join("code.rs"), "fn gated() {}\n").unwrap();

    let refused = run(
        &edit_root,
        &["task", "finish", &task_id, "--message", "gated", "--local"],
    );
    assert!(!refused.status.success());
    let rendered = text(&refused);
    assert!(rendered.contains("a pre-finish hook from"), "{rendered}");
    assert!(rendered.contains("exit 4"), "{rendered}");
    assert!(rendered.contains("hook output: gate missing"), "{rendered}");
    assert!(
        rendered.contains(&format!("`ait task finish {task_id} --local`")),
        "{rendered}"
    );
    let shown = json(&root, &["task", "show", &task_id, "--local", "--json"]);
    assert_eq!(
        shown["status"].as_str(),
        Some("active"),
        "nothing may be applied when the gate fails"
    );

    fs::write(edit_root.join("gate-ok.txt"), "ok\n").unwrap();
    let finished = finish_local(&edit_root, &task_id, "gated");
    assert!(finished.status.success(), "{}", text(&finished));
    let payload = finished_payload(&finished);
    assert_eq!(
        payload["pre_finish_hooks"]["status"].as_str(),
        Some("complete"),
        "{}",
        payload["pre_finish_hooks"]
    );
    assert_eq!(
        payload["pre_finish_hooks"]["phase"].as_str(),
        Some("pre_finish")
    );
    assert_eq!(payload["task_status"].as_str(), Some("completed"));
}

#[test]
fn queue_summary_reports_stale_task_count() {
    let (_temp, root) = initialized();
    let summary = json(&root, &["queue", "summary", "--json"]);
    assert_eq!(
        summary["summary"]["stale_task_count"],
        JsonValue::from(0u64)
    );
    assert_eq!(
        summary["local"]["summary"]["stale_task_days"],
        JsonValue::from(30i64)
    );
    let rendered = run(&root, &["queue", "summary"]);
    assert!(
        text(&rendered).contains("stale tasks: 0"),
        "{}",
        text(&rendered)
    );
}
