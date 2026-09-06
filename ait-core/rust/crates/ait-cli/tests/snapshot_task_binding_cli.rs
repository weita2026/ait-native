use ait_cli::runtime::RepoRuntime;
use ait_core::task_store::TaskStore;
use serde_json::Value;
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

fn json(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    assert!(
        output.status.success(),
        "{args:?}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
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
            "snapshot-regression",
            "--json",
        ],
    );
    (temp, root)
}

fn start(root: &Path, edit_root: &Path, name: &str) -> (String, String, PathBuf) {
    let card = format!("docs/sprints/{name}.md");
    fs::create_dir_all(root.join("docs/sprints")).unwrap();
    fs::write(root.join(&card), format!(
        "# Snapshot ownership regression [plan-ref: test/{name}]\n\nVerify explicit authoring and durable provenance.\n\n- [ ] Complete the ownership regression fixture [ref: test/{name}/run]\n"
    )).unwrap();
    let from = format!("{card}#test/{name}/run");
    let started = json(
        root,
        &[
            "task",
            "start",
            "--local",
            "--from",
            &from,
            "--intent",
            "Verify Snapshot ownership",
            "--edit-root",
            edit_root.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    (
        started["task_id"].as_str().unwrap().to_string(),
        started["change"]["change_id"].as_str().unwrap().to_string(),
        PathBuf::from(started["worktree"]["path"].as_str().unwrap()),
    )
}

fn create(root: &Path, task: &str, change: &str, message: &str) -> Value {
    json(
        root,
        &[
            "snapshot",
            "create",
            &format!("{task}/{change}"),
            "--message",
            message,
            "--json",
            "--full",
        ],
    )
}

fn snapshots(root: &Path) -> Value {
    json(root, &["snapshot", "list", "--all", "--json"])
}

fn seed_baseline(parent: &Path, root: &Path, files: &[(&str, &str)]) {
    let (task, change, edit) = start(root, &parent.join("baseline"), "baseline");
    for (path, content) in files {
        fs::write(edit.join(path), content).unwrap();
    }
    json(
        &edit,
        &[
            "task",
            "finish",
            &format!("{task}/{change}"),
            "--local",
            "--message",
            "baseline",
            "--json",
        ],
    );
}

#[test]
fn sibling_changes_share_one_task_worktree_and_keep_exact_snapshot_history() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    seed_baseline(&parent, &root, &[("code.rs", "fn baseline() {}\n")]);
    let (task, first_change, edit) = start(&root, &parent.join("edit"), "siblings");
    fs::write(edit.join("code.rs"), "fn baseline() {}\nfn first() {}\n").unwrap();
    let first = create(&edit, &task, &first_change, "first Change");
    let second_change = json(
        &edit,
        &[
            "change",
            "create",
            &task,
            "--title",
            "Second Change",
            "--local",
            "--json",
        ],
    );
    let second_ref = second_change["change_ref"].as_str().unwrap();
    fs::write(
        edit.join("code.rs"),
        "fn baseline() {}\nfn first() {}\nfn second() {}\n",
    )
    .unwrap();
    let second = create(&edit, &task, "C-02", "second Change");
    fs::write(
        edit.join("code.rs"),
        "fn baseline() {}\nfn first() {}\nfn second() {}\nfn first_again() {}\n",
    )
    .unwrap();
    let first_again = create(&edit, &task, &first_change, "first Change again");
    let worktrees = json(&root, &["worktree", "list", "--json"]);
    assert_eq!(worktrees.as_array().unwrap().len(), 1);
    assert_eq!(worktrees[0]["path"], edit.to_str().unwrap());
    for (reference, expected) in [
        (format!("{task}/{first_change}"), &first_again),
        (second_ref.to_string(), &second),
    ] {
        let preview = json(
            &root,
            &[
                "change",
                "replay",
                &reference,
                "--local",
                "--dry-run",
                "--json",
            ],
        );
        assert_eq!(
            preview["latest_change_snapshot_id"],
            expected["snapshot_id"]
        );
    }
    json(
        &edit,
        &[
            "change",
            "close",
            &format!("{task}/{first_change}"),
            "--local",
            "--json",
        ],
    );
    fs::write(edit.join("code.rs"), "fn baseline() {}\nfn first() {}\nfn second() {}\nfn first_again() {}\nfn final_second() {}\n").unwrap();
    let finish = json(
        &edit,
        &[
            "task",
            "finish",
            second_ref,
            "--local",
            "--message",
            "finish second Change",
            "--json",
        ],
    );
    assert_eq!(finish["closeout"]["task_status"], "completed");
    assert!(!edit.exists());
    let blame = json(&root, &["blame", "code.rs", "--json"]);
    for (index, change, snapshot) in [
        (1, first_change.as_str(), &first["snapshot_id"]),
        (2, "C-02", &second["snapshot_id"]),
        (3, first_change.as_str(), &first_again["snapshot_id"]),
        (4, "C-02", &finish["landed_snapshot_id"]),
    ] {
        assert_eq!(blame["lines"][index]["task_id"], task);
        assert_eq!(blame["lines"][index]["change_id"], change);
        assert_eq!(&blame["lines"][index]["snapshot_id"], snapshot);
    }
}

#[test]
fn change_replay_preserves_later_saved_edits_and_rejects_conflicts_before_writes() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    seed_baseline(
        &parent,
        &root,
        &[("a.txt", "old\n"), ("b.txt", "alpha=0\nbeta=0\n")],
    );
    let (source_task, source_change, source_edit) = start(&root, &parent.join("source"), "source");
    fs::write(source_edit.join("a.txt"), "new\n").unwrap();
    fs::write(source_edit.join("b.txt"), "alpha=1\nbeta=0\n").unwrap();
    let source_ref = format!("{source_task}/{source_change}");
    json(
        &source_edit,
        &[
            "task",
            "finish",
            &source_ref,
            "--local",
            "--message",
            "source delta",
            "--json",
        ],
    );
    let (task, change, edit) = start(&root, &parent.join("target"), "target");
    fs::write(edit.join("b.txt"), "alpha=1\nbeta=2\n").unwrap();
    create(&edit, &task, &change, "later saved edit");
    let before = snapshots(&root);
    let heads = json(&root, &["line", "list", "--all", "--json"]);
    for _ in 0..2 {
        json(
            &edit,
            &["change", "replay", &source_ref, "--local", "--json"],
        );
        assert_eq!(
            fs::read_to_string(edit.join("b.txt")).unwrap(),
            "alpha=1\nbeta=2\n"
        );
    }
    assert_eq!(snapshots(&root), before);
    assert_eq!(json(&root, &["line", "list", "--all", "--json"]), heads);
    fs::write(edit.join("a.txt"), "old\n").unwrap();
    fs::write(edit.join("b.txt"), "alpha=2\nbeta=2\n").unwrap();
    create(&edit, &task, &change, "later conflicting edit");
    let before = snapshots(&root);
    let heads = json(&root, &["line", "list", "--all", "--json"]);
    let conflict = run(
        &edit,
        &["change", "replay", &source_ref, "--local", "--json"],
    );
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr)
        .to_lowercase()
        .contains("conflict"));
    assert_eq!(fs::read_to_string(edit.join("a.txt")).unwrap(), "old\n");
    assert_eq!(
        fs::read_to_string(edit.join("b.txt")).unwrap(),
        "alpha=2\nbeta=2\n"
    );
    assert_eq!(snapshots(&root), before);
    assert_eq!(json(&root, &["line", "list", "--all", "--json"]), heads);
}

#[test]
fn finishing_one_change_keeps_its_sibling_worktree_and_uses_the_selected_revision() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    seed_baseline(&parent, &root, &[("code.rs", "fn base() {}\n")]);
    let (task, change, edit) = start(&root, &parent.join("edit"), "partial-finish");
    fs::write(edit.join("code.rs"), "fn base() {}\nfn first() {}\n").unwrap();
    let first = create(&edit, &task, &change, "first");
    json(
        &edit,
        &[
            "change", "create", &task, "--title", "Second", "--local", "--json",
        ],
    );
    fs::write(
        edit.join("code.rs"),
        "fn base() {}\nfn first() {}\nfn second() {}\n",
    )
    .unwrap();
    let second = create(&edit, &task, "C-02", "second");
    let owned = json(
        &edit,
        &["worktree", "restore-owned-head", "--dry-run", "--json"],
    );
    assert_eq!(
        owned["restore_owned_head"]["restored_snapshot_id"],
        second["snapshot_id"]
    );
    let first_finish = json(
        &root,
        &[
            "task",
            "finish",
            &format!("{task}/C-01"),
            "--local",
            "--json",
        ],
    );
    assert_eq!(first_finish["landed_snapshot_id"], first["snapshot_id"]);
    assert_eq!(
        json(&root, &["task", "show", &task, "--local", "--json"])["status"],
        "active"
    );
    assert!(edit.exists());
    assert!(!fs::read_to_string(root.join("code.rs"))
        .unwrap()
        .contains("second"));
    let before = snapshots(&root);
    assert!(
        !run(&edit, &["snapshot", "create", &format!("{task}/C-01")])
            .status
            .success()
    );
    assert_eq!(snapshots(&root), before);
    // A Task reference now selects the sole open Change despite an accepted sibling.
    let second_finish = json(&root, &["task", "finish", &task, "--local", "--json"]);
    assert_eq!(second_finish["landed_snapshot_id"], second["snapshot_id"]);
    assert_eq!(second_finish["closeout"]["task_status"], "completed");
    assert!(!edit.exists());
    assert!(fs::read_to_string(root.join("code.rs"))
        .unwrap()
        .contains("second"));
}

#[test]
fn replay_and_revert_merge_add_delete_and_dirty_paths_without_mutating_history() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    seed_baseline(
        &parent,
        &root,
        &[
            ("value.txt", "alpha=0\nbeta=0\n"),
            ("deleted.txt", "original\n"),
        ],
    );
    let (source_task, source_change, source) = start(&root, &parent.join("source"), "source-delta");
    fs::write(source.join("value.txt"), "alpha=1\nbeta=0\n").unwrap();
    fs::write(source.join("added.txt"), "added\n").unwrap();
    fs::remove_file(source.join("deleted.txt")).unwrap();
    create(&source, &source_task, &source_change, "source delta");
    let reference = format!("{source_task}/{source_change}");
    let (task, change, edit) = start(&root, &parent.join("target"), "target-delta");
    fs::write(edit.join("value.txt"), "alpha=0\nbeta=2\n").unwrap();
    create(&edit, &task, &change, "compatible later change");
    fs::write(edit.join("outside.txt"), "unsaved outside\n").unwrap();
    json(
        &edit,
        &["change", "replay", &reference, "--local", "--json"],
    );
    assert_eq!(
        fs::read_to_string(edit.join("value.txt")).unwrap(),
        "alpha=1\nbeta=2\n"
    );
    json(
        &edit,
        &[
            "change", "revert", &reference, "--local", "--force", "--json",
        ],
    );
    fs::write(edit.join("value.txt"), "alpha=0\nbeta=3\n").unwrap();
    let inventory = snapshots(&root);
    let heads = json(&root, &["line", "list", "--all", "--json"]);
    let preview = json(
        &edit,
        &[
            "change",
            "replay",
            &reference,
            "--local",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["applied"], false);
    assert_eq!(
        preview["dirty_selected_paths"],
        serde_json::json!(["value.txt"])
    );
    assert_eq!(
        preview["dirty_outside_paths"],
        serde_json::json!(["outside.txt"])
    );
    assert!(!run(&edit, &["change", "replay", &reference, "--local"])
        .status
        .success());
    assert!(!edit.join("added.txt").exists());
    assert!(edit.join("deleted.txt").exists());
    for _ in 0..2 {
        json(
            &edit,
            &[
                "change", "replay", &reference, "--local", "--force", "--json",
            ],
        );
        assert_eq!(
            fs::read_to_string(edit.join("value.txt")).unwrap(),
            "alpha=1\nbeta=3\n"
        );
        assert_eq!(
            fs::read_to_string(edit.join("added.txt")).unwrap(),
            "added\n"
        );
        assert!(!edit.join("deleted.txt").exists());
    }
    for _ in 0..2 {
        json(
            &edit,
            &[
                "change", "revert", &reference, "--local", "--force", "--json",
            ],
        );
        assert_eq!(
            fs::read_to_string(edit.join("value.txt")).unwrap(),
            "alpha=0\nbeta=3\n"
        );
        assert_eq!(
            fs::read_to_string(edit.join("deleted.txt")).unwrap(),
            "original\n"
        );
        assert!(!edit.join("added.txt").exists());
    }
    assert_eq!(
        fs::read_to_string(edit.join("outside.txt")).unwrap(),
        "unsaved outside\n"
    );
    // Addition conflict must also protect otherwise compatible paths and removals.
    fs::write(edit.join("added.txt"), "independent addition\n").unwrap();
    let preview = json(
        &edit,
        &[
            "change",
            "replay",
            &reference,
            "--local",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["conflict_kinds"]["added.txt"], "add_add");
    assert!(!run(
        &edit,
        &["change", "replay", &reference, "--local", "--force"]
    )
    .status
    .success());
    assert_eq!(
        fs::read_to_string(edit.join("value.txt")).unwrap(),
        "alpha=0\nbeta=3\n"
    );
    assert_eq!(
        fs::read_to_string(edit.join("added.txt")).unwrap(),
        "independent addition\n"
    );
    assert!(edit.join("deleted.txt").exists());
    fs::remove_file(edit.join("added.txt")).unwrap();
    fs::create_dir(edit.join("added.txt")).unwrap();
    assert!(!run(
        &edit,
        &["change", "replay", &reference, "--local", "--force"]
    )
    .status
    .success());
    assert!(edit.join("deleted.txt").exists());
    assert_eq!(snapshots(&root), inventory);
    assert_eq!(json(&root, &["line", "list", "--all", "--json"]), heads);
}

#[test]
fn rebase_continues_the_second_change_without_rebinding_its_task_worktree() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    seed_baseline(&parent, &root, &[("base.txt", "base\n")]);
    let (task, _, edit) = start(&root, &parent.join("edit"), "rebase-siblings");
    json(
        &edit,
        &[
            "change", "create", &task, "--title", "Second", "--local", "--json",
        ],
    );
    fs::write(edit.join("feature.txt"), "second Change\n").unwrap();
    create(&edit, &task, "C-02", "second Change before rebase");
    let (other_task, other_change, other) = start(&root, &parent.join("other"), "advance-main");
    fs::write(other.join("unrelated.txt"), "new main\n").unwrap();
    json(
        &other,
        &[
            "task",
            "finish",
            &format!("{other_task}/{other_change}"),
            "--local",
            "--message",
            "advance main",
            "--json",
        ],
    );
    let rebased = json(&edit, &["worktree", "rebase", "--onto", "main", "--json"]);
    let revision = rebased["rebase"]["new_head_snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    let runtime = RepoRuntime::discover_from_path(&edit).unwrap();
    let owners = runtime
        .task_store()
        .unwrap()
        .snapshot_ownership_rows(std::slice::from_ref(&revision))
        .unwrap();
    assert_eq!(owners[0]["task_id"], task);
    assert_eq!(owners[0]["change_id"], "C-02");
    let worktrees = json(&root, &["worktree", "list", "--json"]);
    assert_eq!(worktrees.as_array().unwrap().len(), 1);
    assert_eq!(worktrees[0]["bound_change_id"], "C-01");
    assert_eq!(
        fs::read_to_string(edit.join("feature.txt")).unwrap(),
        "second Change\n"
    );
    assert_eq!(
        fs::read_to_string(edit.join("unrelated.txt")).unwrap(),
        "new main\n"
    );
    let preview = json(
        &root,
        &[
            "change",
            "replay",
            &format!("{task}/C-02"),
            "--local",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["latest_change_snapshot_id"], revision);
}

#[test]
fn duplicate_task_registration_is_rejected_for_sibling_snapshot_before_mutation() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    let (task, _, edit) = start(&root, &parent.join("edit"), "duplicate");
    json(
        &edit,
        &[
            "change", "create", &task, "--title", "Second", "--local", "--json",
        ],
    );
    fs::write(edit.join("code.rs"), "fn pending() {}\n").unwrap();
    let registry = root.join(".ait/worktrees");
    let original = fs::read_dir(&registry)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    fs::copy(original, registry.join("duplicate.json")).unwrap();
    let inventory = snapshots(&root);
    let heads = json(&root, &["line", "list", "--all", "--json"]);
    let failure = run(&edit, &["snapshot", "create", &format!("{task}/C-02")]);
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("exactly one registered worktree"));
    assert_eq!(snapshots(&root), inventory);
    assert_eq!(json(&root, &["line", "list", "--all", "--json"]), heads);
    assert_eq!(
        fs::read_to_string(edit.join("code.rs")).unwrap(),
        "fn pending() {}\n"
    );
}

#[test]
fn task_snapshot_and_commit_keep_exact_identity_and_reject_ambiguous_siblings() {
    let (temp, root) = initialized();
    let (task, first_change, edit) = start(
        &root,
        &temp.path().canonicalize().unwrap().join("edit"),
        "task-input",
    );
    fs::write(edit.join("code.rs"), "fn first() {}\n").unwrap();
    let first = json(
        &edit,
        &["snapshot", "create", &task, "-m", "first", "--json"],
    );
    assert_eq!(first["task_id"], task);
    assert_eq!(first["change_id"], first_change);
    fs::write(edit.join("code.rs"), "fn first() {}\nfn second() {}\n").unwrap();
    let committed = json(&edit, &["commit", &task, "-m", "second", "--json"]);
    assert_eq!(committed["change_id"], first_change);
    let second = json(
        &edit,
        &[
            "change",
            "create",
            &task,
            "--title",
            "Replacement",
            "--local",
            "--json",
        ],
    );
    let second_ref = second["change_ref"].as_str().unwrap();
    fs::write(
        edit.join("code.rs"),
        "fn first() {}\nfn second() {}\nfn third() {}\n",
    )
    .unwrap();
    let before = snapshots(&root);
    let heads = json(&root, &["line", "list", "--all", "--json"]);
    let rejected = run(&edit, &["snapshot", "create", &task, "-m", "ambiguous"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("multiple writable changes"));
    assert_eq!(snapshots(&root), before);
    assert_eq!(json(&root, &["line", "list", "--all", "--json"]), heads);
    // Closing C-01 must select the sole C-02 without rebinding the worktree.
    json(
        &edit,
        &[
            "change",
            "close",
            &format!("{task}/{first_change}"),
            "--local",
            "--json",
        ],
    );
    let replacement = json(
        &edit,
        &["snapshot", "create", &task, "-m", "third", "--json"],
    );
    assert_eq!(replacement["change_id"], second["change_id"]);
    assert_eq!(
        json(&edit, &["worktree", "show", "--json"])["bound_task_id"],
        task
    );
    let finished = json(&edit, &["task", "finish", &task, "--local", "--json"]);
    assert_eq!(finished["change_ref"], second_ref);
    assert!(!edit.exists());
    let blame = json(&root, &["blame", "code.rs", "--line", "3", "--json"]);
    assert_eq!(blame["lines"][0]["task_id"], task);
    assert_eq!(blame["lines"][0]["change_id"], "C-02");
    assert_eq!(blame["lines"][0]["snapshot_id"], replacement["snapshot_id"]);
    let text = run(&root, &["blame", "code.rs", "--line", "3"]);
    assert!(text.status.success());
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains(&task), "{text}");
    assert!(!text.contains("C-02"), "{text}");
}

#[test]
fn ordinary_task_text_workflow_needs_no_change_id_and_creates_only_one_change() {
    let (temp, root) = initialized();
    let edit = temp.path().canonicalize().unwrap().join("plain-task");
    fs::create_dir_all(root.join("docs/sprints")).unwrap();
    fs::write(
        root.join("docs/sprints/plain.md"),
        "# Plain Task [plan-ref: test/plain]\n\n- [ ] Preserve Task title [ref: test/plain/run]\n",
    )
    .unwrap();
    let started = run(
        &root,
        &[
            "task",
            "start",
            "--from",
            "docs/sprints/plain.md#test/plain/run",
            "--intent",
            "Task input workflow",
            "--edit-root",
            edit.to_str().unwrap(),
        ],
    );
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let output = String::from_utf8_lossy(&started.stdout);
    assert!(output.contains("Preserve Task title"), "{output}");
    assert!(!output.contains("C-01"), "{output}");
    let task = json(&edit, &["worktree", "show", "--json"])["bound_task_id"]
        .as_str()
        .unwrap()
        .to_string();
    fs::write(edit.join("code.rs"), "fn checkpoint() {}\n").unwrap();
    let snapshot = run(
        &edit,
        &["snapshot", "create", &task, "-m", "原樣保存 checkpoint"],
    );
    assert!(
        snapshot.status.success(),
        "{}",
        String::from_utf8_lossy(&snapshot.stderr)
    );
    let output = String::from_utf8_lossy(&snapshot.stdout);
    assert!(
        output.contains(&task) && output.contains("原樣保存 checkpoint"),
        "{output}"
    );
    assert!(!output.contains("C-01"), "{output}");
    for args in [
        vec!["task", "audit", task.as_str(), "--local"],
        vec!["worktree", "show"],
    ] {
        let inspected = run(&edit, &args);
        assert!(inspected.status.success());
        assert!(!String::from_utf8_lossy(&inspected.stdout).contains("C-01"));
    }
    assert_eq!(
        json(&edit, &["change", "list", "--local", "--all", "--json"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let finished = run(&edit, &["task", "finish", &task, "--local"]);
    assert!(
        finished.status.success(),
        "{}",
        String::from_utf8_lossy(&finished.stderr)
    );
    let output = String::from_utf8_lossy(&finished.stdout);
    assert!(output.contains(&task), "{output}");
    assert!(!output.contains("C-01"), "{output}");
    assert!(!edit.exists());
}

#[test]
fn snapshot_and_commit_require_one_change_reference_and_reject_a_fresh_repository() {
    let (_temp, root) = initialized();
    fs::write(root.join("code.rs"), "fn regression() {}\n").unwrap();
    let before = snapshots(&root);
    for prefix in [vec!["snapshot", "create"], vec!["commit"]] {
        for suffix in [
            vec!["--message", "missing reference"],
            vec!["LT-0001", "--message", "no Task"],
            vec!["LT-0001/C-01", "--message", "no Task"],
            vec!["--task-id", "LT-0001", "--message", "missing Change"],
            vec!["--change-id", "C-01", "--message", "missing Task"],
            vec![
                "--task-id",
                "LT-0001",
                "--change-id",
                "C-01",
                "--message",
                "no Task",
            ],
        ] {
            let args = prefix.iter().chain(&suffix).copied().collect::<Vec<_>>();
            assert!(!run(&root, &args).status.success(), "{args:?}");
            assert_eq!(snapshots(&root), before);
        }
    }
}

#[test]
fn snapshot_blame_preserves_earlier_and_final_ownership_after_finish_and_cleanup() {
    let (temp, root) = initialized();
    let (task, change, worktree) = start(
        &root,
        &temp.path().canonicalize().unwrap().join("edit"),
        "durable",
    );
    fs::write(worktree.join("code.rs"), "fn first_checkpoint() {}\n").unwrap();
    let first = create(&worktree, &task, &change, "first checkpoint");
    assert_eq!(first["task_id"], task);
    assert_eq!(first["change_id"], change);
    fs::write(
        worktree.join("code.rs"),
        "fn first_checkpoint() {}\nfn second_checkpoint() {}\n",
    )
    .unwrap();
    let change_ref = format!("{task}/{change}");
    let second = json(
        &worktree,
        &[
            "commit",
            &change_ref,
            "-m",
            "second checkpoint",
            "--json",
            "--full",
        ],
    );
    assert_eq!(second["task_id"], task);
    assert_eq!(second["change_id"], change);
    let selected = json(
        &worktree,
        &["change", "show", &change_ref, "--local", "--json"],
    );
    assert_eq!(selected["task_id"], task);
    assert_eq!(selected["change_id"], change);
    fs::write(
        worktree.join("code.rs"),
        "fn first_checkpoint() {}\nfn second_checkpoint() {}\nfn finish_checkpoint() {}\n",
    )
    .unwrap();
    json(
        &worktree,
        &[
            "task",
            "finish",
            &change_ref,
            "--local",
            "--message",
            "finish checkpoint",
            "--json",
            "--full",
        ],
    );
    assert!(!worktree.exists(), "Task finish must clean up its worktree");
    let completed = json(&root, &["task", "show", &task, "--local", "--json"]);
    assert_eq!(completed["status"], "completed");
    let blame = json(&root, &["blame", "code.rs", "--json"]);
    let lines = blame["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    for line in lines {
        assert_eq!(line["task_id"], task);
        assert_eq!(line["change_id"], change);
        assert_eq!(line["provenance_confidence"], "durable_snapshot_binding");
    }
    assert_eq!(lines[0]["snapshot_id"], first["snapshot_id"]);
    assert_eq!(lines[1]["snapshot_id"], second["snapshot_id"]);
    let explicit = json(
        &root,
        &[
            "blame",
            "code.rs",
            "--snapshot",
            first["snapshot_id"].as_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(explicit["lines"][0]["task_id"], task);
    // Historical inspection remains available without starting another Task.
    json(
        &root,
        &[
            "snapshot",
            "show",
            first["snapshot_id"].as_str().unwrap(),
            "--json",
        ],
    );
    let before = snapshots(&root);
    assert!(
        !run(&root, &["snapshot", "create", &format!("{task}/{change}")])
            .status
            .success()
    );
    assert_eq!(snapshots(&root), before);
}

#[test]
fn wrong_task_change_and_terminal_task_are_rejected_before_snapshot_creation() {
    let (temp, root) = initialized();
    let (task, change, worktree) = start(
        &root,
        &temp.path().canonicalize().unwrap().join("edit"),
        "mismatch",
    );
    fs::write(worktree.join("code.rs"), "fn pending() {}\n").unwrap();
    let before = snapshots(&root);
    let heads_before = json(&root, &["line", "list", "--all", "--json"]);
    for prefix in [vec!["snapshot", "create"], vec!["commit"]] {
        for reference in [
            format!("LT-9999/{change}"),
            format!("{task}/C-02"),
            "LT-9999".to_string(),
            change.clone(),
            String::new(),
            format!("{task}/{task}/{change}"),
            format!("{task}//{change}"),
            format!("/{change}"),
            format!("{task}/"),
            format!("{task}/SNP-123"),
            format!("{task}/C-x"),
        ] {
            let mut args = prefix.clone();
            args.push(&reference);
            let result = run(&worktree, &args);
            assert!(!result.status.success(), "{args:?}");
            assert_eq!(snapshots(&root), before);
            assert_eq!(
                json(&root, &["line", "list", "--all", "--json"]),
                heads_before
            );
            assert_eq!(
                fs::read_to_string(worktree.join("code.rs")).unwrap(),
                "fn pending() {}\n"
            );
        }
    }
    // Model a stale worktree left behind after a terminal Task transition.
    let repo = RepoRuntime::discover_from_path(&root).unwrap();
    repo.task_store()
        .unwrap()
        .close_task(&task, "completed")
        .unwrap();
    for reference in [task.clone(), format!("{task}/{change}")] {
        let result = run(&worktree, &["snapshot", "create", &reference]);
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("active Task"));
        assert_eq!(snapshots(&root), before);
    }
}

#[test]
fn two_active_tasks_cannot_snapshot_each_others_workspace_or_line() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    let (first, change, edit) = start(&root, &parent.join("first"), "first");
    let (second, _, _) = start(&root, &parent.join("second"), "second");
    fs::write(edit.join("private.rs"), "fn belongs_to_first() {}\n").unwrap();
    let before = snapshots(&root);
    let heads = json(&root, &["line", "list", "--all", "--json"]);
    for reference in [second.clone(), format!("{second}/{change}")] {
        let denied = run(&edit, &["snapshot", "create", &reference]);
        assert!(!denied.status.success());
        assert!(String::from_utf8_lossy(&denied.stderr).contains("does not match"));
    }
    let overlay = edit.join(".ait-worktree.json");
    let bytes = fs::read(&overlay).unwrap();
    let mut config: Value = serde_json::from_slice(&bytes).unwrap();
    config["current_line"] = Value::String("main".into());
    fs::write(&overlay, serde_json::to_vec(&config).unwrap()).unwrap();
    assert!(
        !run(&edit, &["snapshot", "create", &format!("{first}/{change}")])
            .status
            .success()
    );
    assert!(!run(&edit, &["snapshot", "create", &first]).status.success());
    fs::write(&overlay, bytes).unwrap();
    assert_eq!(snapshots(&root), before);
    assert_eq!(json(&root, &["line", "list", "--all", "--json"]), heads);
    create(&edit, &first, &change, "correct workspace");
}
