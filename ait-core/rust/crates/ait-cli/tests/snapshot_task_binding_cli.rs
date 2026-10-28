#[path = "support/cli_binary.rs"]
mod cli_binary;
#[path = "support/native_home.rs"]
mod native_home;
#[path = "support/workflow_process.rs"]
mod workflow_process;
use ait_cli::runtime::RepoRuntime;
use ait_core::task_store::TaskStore;
use ait_core::task_workflow_store_traits::TaskWorkflowChangeCreator;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(cli_binary::selected(env!("CARGO_BIN_EXE_ait-cli")))
        .env("CODEX_HOME", native_home::path())
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
    // These fixtures exercise explicit revision APIs. Ordinary Task-only work
    // below creates no optional revision record.
    let task_id = started["task_id"].as_str().unwrap();
    let edit = PathBuf::from(started["worktree"]["path"].as_str().unwrap());
    let revision = json(
        &edit,
        &[
            "change",
            "create",
            task_id,
            "--title",
            "Explicit revision fixture",
            "--local",
            "--json",
        ],
    );
    (
        task_id.to_string(),
        revision["change_id"].as_str().unwrap().to_string(),
        edit,
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
fn finish_uses_the_complete_task_head_once_with_optional_revision_provenance() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    seed_baseline(&parent, &root, &[("code.rs", "fn base() {}\n")]);
    let (task, change, edit) = start(&root, &parent.join("edit"), "complete-finish");
    fs::write(edit.join("code.rs"), "fn base() {}\nfn first() {}\n").unwrap();
    create(&edit, &task, &change, "first revision");
    json(
        &edit,
        &[
            "change",
            "create",
            &task,
            "--title",
            "Independent second revision",
            "--local",
            "--json",
        ],
    );
    fs::write(
        edit.join("code.rs"),
        "fn base() {}\nfn first() {}\nfn second() {}\n",
    )
    .unwrap();
    let final_snapshot = create(&edit, &task, "C-02", "complete final candidate");
    let finish = json(
        &edit,
        &[
            "task",
            "finish",
            &format!("{task}/C-01"),
            "--local",
            "--json",
            "--full",
        ],
    );
    assert_eq!(finish["landed_snapshot_id"], final_snapshot["snapshot_id"]);
    assert_eq!(finish["task_status"], "completed");
    assert_eq!(finish["change_id"], "C-02");
    assert!(!edit.exists());
    assert!(fs::read_to_string(root.join("code.rs"))
        .unwrap()
        .contains("second"));
    assert_eq!(
        json(
            &root,
            &[
                "change",
                "show",
                &format!("{task}/C-01"),
                "--local",
                "--json"
            ]
        )["status"],
        "archived"
    );
    let before = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    json(&root, &["task", "finish", &task, "--local", "--json"]);
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        before
    );
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
    assert!(worktrees[0]["bound_change_id"].is_null());
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
        &[
            "snapshot", "create", &task, "-m", "first", "--json", "--full",
        ],
    );
    assert_eq!(first["task_id"], task);
    assert_eq!(first["change_id"], first_change);
    fs::write(edit.join("code.rs"), "fn first() {}\nfn second() {}\n").unwrap();
    let committed = json(
        &edit,
        &["commit", &task, "-m", "second", "--json", "--full"],
    );
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
    assert!(String::from_utf8_lossy(&rejected.stderr)
        .contains("multiple internal writable work records"));
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
        &[
            "snapshot", "create", &task, "-m", "third", "--json", "--full",
        ],
    );
    assert_eq!(replacement["change_id"], second["change_id"]);
    assert_eq!(
        json(&edit, &["worktree", "show", "--json"])["bound_task_id"],
        task
    );
    let finished = json(
        &edit,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
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
fn ordinary_task_text_workflow_needs_no_change_and_creates_none() {
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
    fs::write(
        edit.join("code.rs"),
        "fn checkpoint() {}\nfn frozen_candidate() {}\n",
    )
    .unwrap();
    let frozen = json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "-m",
            "Reviewed Task-only candidate",
            "--json",
            "--full",
        ],
    );
    assert!(frozen["change_id"].is_null());
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
        0
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
    let task_record = json(&root, &["task", "show", &task, "--local", "--json"]);
    assert_eq!(
        task_record["local_finish"]["landed_snapshot_id"],
        frozen["snapshot_id"]
    );
    assert!(task_record["local_finish"]["change_ref"].is_null());
    let audit = json(&root, &["task", "audit", &task, "--local", "--json"]);
    assert_eq!(
        audit["local_closeout_evidence"]["receipt"]["status"],
        "done"
    );
    let blame = json(&root, &["blame", "code.rs", "--json"]);
    assert!(blame["lines"]
        .as_array()
        .unwrap()
        .iter()
        .all(|line| line["task_id"] == task && line["change_id"].is_null()));
    let raw = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    assert_eq!(raw.len(), 48);
    assert_eq!(&raw[..4], &2u32.to_le_bytes());
    assert_eq!(raw[6], 0);
    assert_eq!(&raw[8..12], &0u32.to_le_bytes());
    assert!(fs::read_to_string(root.join("docs/sprints/plain.md"))
        .unwrap()
        .contains("- [x]"));
    json(&root, &["task", "finish", &task, "--local", "--json"]);
    assert_eq!(fs::read(root.join(".ait/binary-db/land.bin")).unwrap(), raw);
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

fn legacy_receipt_fixture() -> (TempDir, PathBuf, PathBuf, Vec<u8>) {
    let temp = TempDir::new().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let root = parent.join("ait-core");
    fs::create_dir(&root).unwrap();
    json(&root, &["init", "--json"]);
    json(
        &root,
        &[
            "config",
            "set",
            "--id-namespace-prefix",
            "C",
            "--user-name",
            "legacy-fixture",
            "--json",
        ],
    );
    let (task, change, edit) = start(&root, &parent.join("legacy-edit"), "legacy-source");
    fs::write(edit.join("source.rs"), "fn first() {}\n").unwrap();
    create(&edit, &task, &change, "First legacy Snapshot");
    fs::write(edit.join("source.rs"), "fn retained_revision() {}\n").unwrap();
    create(&edit, &task, &change, "Retained revision Snapshot");
    fs::write(edit.join("source.rs"), "fn accepted() {}\n").unwrap();
    create(&edit, &task, &change, "Accepted legacy Snapshot");
    json(&edit, &["task", "finish", &task, "--local", "--json"]);
    let db = root.join(".ait/binary-db");
    let original_task = fs::read(db.join("task.bin")).unwrap()[4..].to_vec();
    let original_change = fs::read(db.join("change.bin")).unwrap()[4..].to_vec();
    let mut tasks = 1u32.to_le_bytes().to_vec();
    for index in 0..1113 {
        let mut row = original_task.clone();
        if index != 0 {
            row[0] = 0;
            row[1] = 0;
            row[12..24].fill(0);
            row[40..64].fill(0);
        }
        if index == 2 || index == 53 {
            row[1] = 1;
            row[20..24].copy_from_slice(&(if index == 2 { 981u32 } else { 787 }).to_le_bytes());
            row[48..56].copy_from_slice(&1u64.to_le_bytes());
        }
        if index == 620 {
            row[0] = 64;
        }
        tasks.extend_from_slice(&row);
    }
    let mut changes = 1u32.to_le_bytes().to_vec();
    let mut task_changes = vec![[0u8; 8]; 1113];
    let mut physical = vec![None; 1113];
    for task in 0..1113usize {
        if task == 362 || task == 376 {
            continue;
        }
        let index = ((changes.len() - 4) / 68) as u32;
        physical[task] = Some(index);
        let mut row = original_change.clone();
        row[0] = if task == 0 { 2 } else { 0 };
        row[1] = 0;
        row[4] = 0;
        row[5] = 0;
        row[16..20].copy_from_slice(&(task as u32).to_le_bytes());
        row[20..24].fill(0);
        row[28..32].fill(0);
        row[48..56].fill(0);
        row[60..68].fill(0);
        changes.extend_from_slice(&row);
        let owner = &mut task_changes[task];
        owner[..4].copy_from_slice(&(index + 1).to_le_bytes());
        owner[4] = 1;
        owner[6] = 1;
        if [2, 53, 620].contains(&task) {
            let mut revision = row.clone();
            revision[0] = if task == 620 { 2 } else { 3 };
            revision[4] = 1;
            revision[20..24].copy_from_slice(&(index + 1).to_le_bytes());
            if task != 620 {
                revision[1] = 1;
                revision[5] = 1;
                revision[28] = 1;
                revision[48..56].copy_from_slice(&1u64.to_le_bytes());
            }
            changes.extend_from_slice(&revision);
            owner[..4].copy_from_slice(&(index + 2).to_le_bytes());
            owner[4] = 2;
            owner[6] = 2;
        }
    }
    assert_eq!((changes.len() - 4) / 68, 1114);
    let index_bytes = |rows: Vec<[u8; 8]>| {
        let mut out = 1u32.to_le_bytes().to_vec();
        for row in rows {
            out.extend_from_slice(&row);
        }
        out
    };
    let mut lands = fs::read(db.join("land.bin")).unwrap();
    lands[..4].copy_from_slice(&1u32.to_le_bytes());
    lands[6] = 0;
    lands[8..12].copy_from_slice(&0u32.to_le_bytes());
    let current_links = fs::read(db.join("snapshot_link.bin")).unwrap();
    assert_eq!(current_links.len(), 4 + 3 * 22);
    let mut links = 1u32.to_le_bytes().to_vec();
    let mut legacy_payload = 1u32.to_le_bytes().to_vec();
    for (index, row) in current_links[4..].chunks_exact(22).enumerate() {
        let fields = [
            "legacy-edit",
            "feature/lct-0001",
            "LCT-0001",
            "C-01",
            "ai_with_human_review",
            "legacy-model",
        ];
        let mut bytes = Vec::new();
        for field in &fields[..5] {
            bytes.extend_from_slice(&(field.len() as u16).to_le_bytes());
        }
        for field in fields {
            bytes.extend_from_slice(field.as_bytes());
        }
        let mut old = vec![row[0] | 0x38, row[1]];
        old.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        old.extend_from_slice(&(legacy_payload.len() as u64).to_le_bytes());
        old.extend_from_slice(&row[2..14]);
        old.extend_from_slice(&(index as u32).to_le_bytes());
        old.extend_from_slice(&(index as u32).to_le_bytes());
        old.extend_from_slice(&row[14..22]);
        links.extend_from_slice(&old);
        legacy_payload.extend_from_slice(&bytes);
    }
    fs::write(db.join("snapshot_link_payload.bin"), legacy_payload).unwrap();

    assert_eq!(links.len(), 124);
    let first_snapshot = u32::from_le_bytes(links[24..28].try_into().unwrap());
    let retained_snapshot = u32::from_le_bytes(links[64..68].try_into().unwrap());
    let last_snapshot = u32::from_le_bytes(links[104..108].try_into().unwrap());
    let mut retained_receipt = lands[4..].to_vec();
    retained_receipt[2] = 1;
    retained_receipt[4..8].copy_from_slice(&621u32.to_le_bytes());
    retained_receipt[20..24].copy_from_slice(&(retained_snapshot + 1).to_le_bytes());
    lands.extend_from_slice(&retained_receipt);
    let mut task_lands = vec![[0u8; 8]; 1113];
    task_lands[0][0] = 1;
    task_lands[0][4] = 1;
    task_lands[620][0] = 2;
    task_lands[620][4] = 1;
    let mut change_lands = vec![[0u8; 8]; 1114];
    change_lands[0][0] = 1;
    change_lands[0][4] = 1;
    change_lands[0][6] = 1;
    change_lands[621][0] = 2;
    change_lands[621][4] = 1;
    change_lands[621][6] = 1;
    // Initial accepted Snapshot has no explicit link. A retained revision has
    // both fixed and typed ownership requiring exact physical/textual remapping.
    let mut retained_link = links[44..84].to_vec();
    retained_link[1] = 0;
    retained_link[12..16].copy_from_slice(&620u32.to_le_bytes());
    retained_link[16..20].copy_from_slice(&622u32.to_le_bytes());
    retained_link[24..32].fill(0);
    let mut payloads = fs::read(db.join("snapshot_link_payload.bin")).unwrap();
    let offset = u64::from_le_bytes(retained_link[4..12].try_into().unwrap()) as usize;
    let len = u16::from_le_bytes(retained_link[2..4].try_into().unwrap()) as usize;
    let mut payload = payloads[offset..offset + len].to_vec();
    let worktree_len = u16::from_le_bytes(payload[..2].try_into().unwrap()) as usize;
    let line_len = u16::from_le_bytes(payload[2..4].try_into().unwrap()) as usize;
    let task_offset = 10 + worktree_len + line_len;
    payload[task_offset..task_offset + 8].copy_from_slice(b"LCT-0621");
    payload[task_offset + 8..task_offset + 12].copy_from_slice(b"C-02");
    retained_link[4..12].copy_from_slice(&(payloads.len() as u64).to_le_bytes());
    payloads.extend_from_slice(&payload);
    let mut source_links = links[..44].to_vec();
    source_links.extend_from_slice(&retained_link);
    fs::write(db.join("snapshot_link.bin"), source_links).unwrap();
    fs::write(db.join("snapshot_link_payload.bin"), payloads).unwrap();
    let mut task_links = vec![[0u8; 8]; 1113];
    task_links[0][0] = 1;
    task_links[0][4] = 1;
    task_links[0][6] = 1;
    task_links[620][0] = 2;
    task_links[620][4] = 1;
    task_links[620][6] = 1;
    let mut change_links = vec![[0u8; 8]; 1114];
    change_links[0][0] = 1;
    change_links[0][4] = 1;
    change_links[621][0] = 2;
    change_links[621][4] = 1;
    for (name, bytes) in [
        ("task.bin", tasks),
        ("change.bin", changes),
        ("task_change_index.bin", index_bytes(task_changes)),
        ("task_land_index.bin", index_bytes(task_lands)),
        ("change_land_index.bin", index_bytes(change_lands)),
        ("task_snapshot_index.bin", index_bytes(task_links)),
        ("change_snapshot_index.bin", index_bytes(change_links)),
        ("land.bin", lands.clone()),
    ] {
        fs::write(db.join(name), bytes).unwrap();
    }
    let active = parent.join("active-1063");
    fs::create_dir_all(active.join(".ait-worktree")).unwrap();
    fs::write(active.join("keep.rs"), "fn protected_work() {}\n").unwrap();
    fs::write(active.join(".ait-worktree.json"),serde_json::to_vec(&serde_json::json!({"repo_root":root,"workspace_root":active,"worktree_name":"active-1063","current_line":"feature/lct-1063"})).unwrap()).unwrap();
    fs::create_dir_all(root.join(".ait/worktrees")).unwrap();
    fs::write(root.join(".ait/worktrees/active-1063.json"),serde_json::to_vec(&serde_json::json!({"repo_root":root,"path":active,"name":"active-1063","bound_task_id":"LCT-1063","bound_change_id":"C-01","bound_change_ref":"LCT-1063/C-01","line_name":"feature/lct-1063","auto_created_for_task":true})).unwrap()).unwrap();
    let mut cursor = vec![0; 32];
    cursor[..4].copy_from_slice(&1u32.to_le_bytes());
    cursor[4] = 7;
    cursor[8..12].copy_from_slice(&1062u32.to_le_bytes());
    cursor[12..16].copy_from_slice(&(physical[1062].unwrap() + 1).to_le_bytes());
    cursor[16..20].copy_from_slice(&(first_snapshot + 1).to_le_bytes());
    cursor[20..24].copy_from_slice(&(last_snapshot + 1).to_le_bytes());
    fs::write(active.join(".ait-worktree/cursor.bin"), cursor).unwrap();
    let retained = parent.join("retained-0003");
    fs::create_dir_all(retained.join(".ait-worktree")).unwrap();
    fs::write(retained.join(".ait-worktree.json"),serde_json::to_vec(&serde_json::json!({"repo_root":root,"workspace_root":retained,"worktree_name":"retained-0003","current_line":"feature/lct-0003"})).unwrap()).unwrap();
    fs::write(root.join(".ait/worktrees/retained-0003.json"),serde_json::to_vec(&serde_json::json!({"repo_root":root,"path":retained,"name":"retained-0003","bound_task_id":"LCT-0003","bound_change_id":"C-02","bound_change_ref":"LCT-0003/C-02"})).unwrap()).unwrap();
    let mut cursor = vec![0; 32];
    cursor[..4].copy_from_slice(&1u32.to_le_bytes());
    cursor[4] = 1;
    cursor[8..12].copy_from_slice(&2u32.to_le_bytes());
    cursor[12..16].copy_from_slice(&4u32.to_le_bytes());
    fs::write(retained.join(".ait-worktree/cursor.bin"), cursor).unwrap();
    let generation = root.join(".ait/local-task-receipts-layout2");
    (temp, root, generation, lands)
}
fn upgrade(root: &Path, _target: &Path, _activate: bool) -> Output {
    run(root, &["task", "list", "--local", "--json"])
}
#[test]
fn automatic_admission_compacts_exact_cohort_preserves_acceptance_and_cursors_once() {
    let (_temp, root, target, source) = legacy_receipt_fixture();
    let active = root.parent().unwrap().join("active-1063");
    let immutable = [
        "task.bin",
        "snapshot.bin",
        "snapshot_payload.bin",
        "tree.bin",
        "blob.bin",
    ]
    .map(|leaf| {
        (
            leaf,
            fs::read(root.join(".ait/binary-db").join(leaf)).unwrap(),
        )
    });
    let before = fs::read(active.join(".ait-worktree/cursor.bin")).unwrap();
    let admitted = upgrade(&root, &target, true);
    assert!(
        admitted.status.success(),
        "{}",
        String::from_utf8_lossy(&admitted.stderr)
    );
    let db = root.join(".ait/binary-db");
    let changes = fs::read(db.join("change.bin")).unwrap();
    assert_eq!(changes.len(), 4 + 3 * 68);
    for (index, task) in [2u32, 53, 620].iter().enumerate() {
        let row = &changes[4 + index * 68..][..68];
        assert_eq!(row[4], 0);
        assert_eq!(u32::from_le_bytes(row[16..20].try_into().unwrap()), *task);
        assert_eq!(row[1], 0);
        assert_eq!(row[28], 0);
        if index < 2 {
            assert_eq!(&row[48..56], &1u64.to_le_bytes());
        }
        assert_eq!(row[5], if index < 2 { 1 } else { 0 });
    }
    for (leaf, before) in immutable {
        assert_eq!(fs::read(db.join(leaf)).unwrap(), before, "{leaf}");
    }
    let retained_cursor = fs::read(
        root.parent()
            .unwrap()
            .join("retained-0003/.ait-worktree/cursor.bin"),
    )
    .unwrap();
    assert_eq!(&retained_cursor[12..16], &1u32.to_le_bytes());
    assert_eq!(retained_cursor[4], 1);
    let retained_metadata: Value =
        serde_json::from_slice(&fs::read(root.join(".ait/worktrees/retained-0003.json")).unwrap())
            .unwrap();
    assert_eq!(retained_metadata["bound_change_ref"], "LCT-0003/C-01");
    let lands = fs::read(db.join("land.bin")).unwrap();
    assert_eq!(lands.len(), source.len());
    assert_eq!(&lands[..4], &2u32.to_le_bytes());
    assert_eq!(lands[6], 0);
    assert_eq!(lands[50], 1);
    assert_eq!(&lands[52..56], &620u32.to_le_bytes());
    let after = fs::read(active.join(".ait-worktree/cursor.bin")).unwrap();
    let mut expected = before.clone();
    expected[4] = 6;
    expected[12..16].fill(0);
    assert_eq!(after, expected);
    assert_eq!(
        fs::read_to_string(active.join("keep.rs")).unwrap(),
        "fn protected_work() {}\n"
    );
    let metadata: Value =
        serde_json::from_slice(&fs::read(root.join(".ait/worktrees/active-1063.json")).unwrap())
            .unwrap();
    assert!(metadata["bound_change_id"].is_null());
    assert!(metadata["bound_change_ref"].is_null());
    assert!(!target.exists());
    assert!(fs::read_dir(root.join(".ait")).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".workflow-metadata-")));
    let links = fs::read(db.join("snapshot_link.bin")).unwrap();
    assert_eq!(links[4] & 1, 0);
    assert_eq!(&links[10..14], &0u32.to_le_bytes());
    assert!(!db.join("snapshot_link_payload.bin").exists());
    assert_eq!(links.len(), 48);
    assert_eq!(links[26] & 1, 1);
    assert_eq!(&links[32..36], &3u32.to_le_bytes());
    let retained = json(
        &root,
        &["change", "show", "LCT-0003/C-01", "--local", "--json"],
    );
    assert!(retained["published_change_id"].is_null());
    assert!(retained["published_remote_name"].is_null());
    assert_eq!(retained["publication_state"], "local_draft");
    assert_eq!(retained["published_at"], "1970-01-01T00:00:01Z");
    let published_task = json(&root, &["task", "show", "LCT-0003", "--local", "--json"]);
    assert_eq!(published_task["published_task_id"], "RCT-0982");
    let task = json(&root, &["task", "show", "LCT-0001", "--local", "--json"]);
    let accepted = task["local_finish"]["landed_snapshot_id"].as_str().unwrap();
    let blame = json(
        &root,
        &["blame", "source.rs", "--snapshot", accepted, "--json"],
    );
    assert_eq!(blame["lines"][0]["task_id"], "LCT-0001");
    assert!(blame["lines"][0]["change_id"].is_null());
    assert_eq!(
        blame["lines"][0]["provenance_confidence"],
        "derived_from_recorded_task_receipt"
    );
    assert!(!run(
        &root,
        &["change", "show", "LCT-0001/C-01", "--local", "--json"]
    )
    .status
    .success());
    let repo = RepoRuntime::discover_from_path(&root).unwrap();
    let owners = repo
        .task_store()
        .unwrap()
        .snapshot_ownership_rows(&[accepted.into()])
        .unwrap();
    assert_eq!(owners.len(), 1);
    assert_eq!(owners[0]["task_id"], "LCT-0001");
    assert!(owners[0]["change_id"].is_null());
    drop(repo);
    let runtime = RepoRuntime::discover_from_path(&root).unwrap();
    let changes = runtime.change_store().unwrap();
    let first = changes
        .create_change(
            "ait-core",
            "LCT-1112",
            "First actual revision",
            "main",
            Some("C"),
            None,
        )
        .unwrap();
    assert_eq!(first["change_id"], "C-01");
    json(&root, &["task", "list", "--local", "--json"]);
    let next = changes
        .create_change(
            "ait-core",
            "LCT-1112",
            "Separate revision",
            "main",
            Some("C"),
            None,
        )
        .unwrap();
    assert_eq!(next["change_id"], "C-02");
    let retained = changes
        .create_change(
            "ait-core",
            "LCT-0003",
            "Later retained revision",
            "main",
            Some("C"),
            None,
        )
        .unwrap();
    assert_eq!(retained["change_id"], "C-02");
    assert_eq!(fs::read(db.join("land.bin")).unwrap(), lands);
}

#[test]
fn common_repository_admission_upgrades_before_plan_and_agent_access() {
    for agent in [false, true] {
        let (_temp, root, _target, _source) = legacy_receipt_fixture();
        if agent {
            let result =
                ait_core::agent_local_workflow_backend::agent_local_current_workflow_execute_json(
                    &serde_json::json!({
                        "operation":"read_current_workflow", "target":{"mode":"local","repo_root":root,"repo_name":"ait-core"},"arguments":{}
                    }),
                );
            result.unwrap();
        } else {
            let output = run(&root, &["plan", "list", "--local", "--json"]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        for leaf in ["land.bin", "snapshot_link.bin"] {
            assert_eq!(
                &fs::read(root.join(".ait/binary-db").join(leaf)).unwrap()[..4],
                &2u32.to_le_bytes()
            );
        }
        assert!(!root
            .join(".ait/binary-db/snapshot_link_payload.bin")
            .exists());
        assert_eq!(
            (fs::metadata(root.join(".ait/binary-db/change.bin"))
                .unwrap()
                .len()
                - 4)
                / 68,
            3
        );
    }
}

#[test]
fn retained_revision_inspection_returns_old_c02_content_after_reopen() {
    let (_temp, root, _target, _source) = legacy_receipt_fixture();
    let db = root.join(".ait/binary-db");
    let mut changes = fs::read(db.join("change.bin")).unwrap();
    let mut payload = fs::read(db.join("change_payload.bin")).unwrap();
    let mut originals = Vec::new();
    for (index, row) in changes[4..].chunks_exact_mut(68).enumerate() {
        if row[4] != 1 {
            continue;
        }
        let task = u32::from_le_bytes(row[16..20].try_into().unwrap()) + 1;
        let title = format!("Retained actual revision for Task {task}");
        row[2..4].copy_from_slice(&(title.len() as u16).to_le_bytes());
        row[8..16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        payload.extend_from_slice(title.as_bytes());
        originals.push((task, title, row[24..28].to_vec(), index));
    }
    fs::write(db.join("change.bin"), changes).unwrap();
    fs::write(db.join("change_payload.bin"), payload).unwrap();
    assert_eq!(originals.len(), 3);
    for (position, (task, title, fork, _)) in originals.iter().enumerate() {
        let task_id = format!("LCT-{task:04}");
        let reference = format!("{task_id}/C-01");
        let explicit = json(&root, &["change", "show", &reference, "--local", "--json"]);
        let task_only = json(&root, &["change", "show", &task_id, "--local", "--json"]);
        assert_eq!(explicit, task_only);
        assert_eq!(explicit["change_ref"], reference);
        assert_eq!(explicit["title"], *title);
        assert_eq!(
            explicit["status"],
            if *task == 621 { "landed" } else { "canceled" }
        );
        let current = fs::read(db.join("change.bin")).unwrap();
        assert_eq!(
            &current[4 + position * 68 + 24..4 + position * 68 + 28],
            fork
        );
        if *task == 621 {
            let repo = RepoRuntime::discover_from_path(&root).unwrap();
            let accepted = explicit["landed_snapshot_id"].as_str().unwrap();
            let owners = repo
                .task_store()
                .unwrap()
                .snapshot_ownership_rows(&[accepted.into()])
                .unwrap();
            assert_eq!(owners[0]["task_id"], task_id);
            assert_eq!(owners[0]["change_id"], "C-01");
        }
        assert!(!run(
            &root,
            &[
                "change",
                "show",
                &format!("{task_id}/C-02"),
                "--local",
                "--json"
            ]
        )
        .status
        .success());
    }
    let no_change = run(&root, &["change", "show", "LCT-0001", "--local", "--json"]);
    assert!(!no_change.status.success());
    assert!(String::from_utf8_lossy(&no_change.stderr).contains("has no Change"));
}

#[test]
fn concurrent_repository_opens_share_one_metadata_upgrade_without_backup() {
    let (_temp, root, target, _source) = legacy_receipt_fixture();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let root = root.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            ait_core::binary_db_generation::admit_activated_binary_db_generation_for_runtime(
                &root,
                &root.join(".ait/binary-db"),
                "ait-core",
            )
            .map(|_| ())
        }));
    }
    barrier.wait();
    for thread in threads {
        thread.join().unwrap().unwrap();
    }
    assert!(!target.exists());
    assert_eq!(
        (fs::metadata(root.join(".ait/binary-db/change.bin"))
            .unwrap()
            .len()
            - 4)
            / 68,
        3
    );
    let retained = fs::read_dir(root.join(".ait"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".binary-db.activate-")
        })
        .collect::<Vec<_>>();
    assert!(retained.is_empty());
    upgrade(&root, &target, true);
    assert_eq!(
        fs::read_dir(root.join(".ait"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".binary-db.activate-"))
            .count(),
        0
    );
}
#[test]
fn automatic_admission_rejects_duplicates_and_unexpected_cohort_without_activation() {
    for case in 0..2 {
        let (_temp, root, target, source) = legacy_receipt_fixture();
        let path = root.join(".ait/binary-db/land.bin");
        if case == 0 {
            let mut duplicate = source.clone();
            duplicate.extend_from_slice(&source[4..48]);
            fs::write(&path, &duplicate).unwrap();
        } else {
            let change = root.join(".ait/binary-db/change.bin");
            let mut raw = fs::read(&change).unwrap();
            raw[8] = 2;
            fs::write(change, raw).unwrap();
        }
        let before = fs::read(&path).unwrap();
        let rejected = upgrade(&root, &target, true);
        assert!(!rejected.status.success());
        assert!(!target.exists());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

fn quick_initial(root: &Path, edit: &Path, intent: &str) -> String {
    let started = json(
        root,
        &[
            "task",
            "quick",
            "--local",
            "--intent",
            intent,
            "--edit-root",
            edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    assert!(started["change"].is_null());
    started["task_id"].as_str().unwrap().into()
}

#[test]
fn fixed_task_worktree_name_survives_recreate_at_an_explicit_physical_path() {
    let (temp, root) = initialized();
    let edit = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("custom-physical-path");
    let task = quick_initial(&root, &edit, "Verify fixed Task worktree naming");
    let name = task.to_ascii_lowercase();
    let registry = root.join(".ait/worktrees").join(format!("{name}.json"));
    assert!(registry.is_file());
    assert!(!root
        .join(".ait/worktrees")
        .join(format!("{name}-2.json"))
        .exists());
    let registered: Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    assert_eq!(registered["path"].as_str(), edit.to_str());
    fs::remove_dir_all(&edit).unwrap();
    let recreated = json(&root, &["worktree", "recreate", &name, "--json"]);
    assert!(edit.is_dir(), "{recreated}");
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&registry).unwrap()).unwrap()["name"],
        name
    );
    fs::write(edit.join("fixed.rs"), "fn retained_name() {}\n").unwrap();
    let captured = json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "--message",
            "Recreated fixed worktree",
            "--json",
            "--full",
        ],
    );
    assert_eq!(captured["task_id"], task);
    assert!(captured["change_id"].is_null());
}
#[test]
fn actual_revision_forks_reviewed_task_snapshot_and_reuses_c01_for_two_candidates() {
    let (temp, root) = initialized();
    let edit = temp.path().canonicalize().unwrap().join("revision");
    let task = quick_initial(&root, &edit, "Review initial Task work before revisions");
    fs::write(edit.join("code.rs"), "fn reviewed() {}\n").unwrap();
    let baseline = json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "--message",
            "Reviewed Task-only baseline",
            "--json",
            "--full",
        ],
    );
    assert!(baseline["change_id"].is_null());
    fs::write(
        edit.join("code.rs"),
        "fn reviewed() {}\nfn later_initial_work() {}\n",
    )
    .unwrap();
    let later = json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "--message",
            "Initial development while review runs",
            "--json",
            "--full",
        ],
    );
    assert_ne!(later["snapshot_id"], baseline["snapshot_id"]);
    let change = json(
        &edit,
        &[
            "change",
            "create",
            &task,
            "--title",
            "Repair reviewed behavior",
            "--fork-snapshot",
            baseline["snapshot_id"].as_str().unwrap(),
            "--local",
            "--json",
        ],
    );
    assert_eq!(change["change_id"], "C-01");
    assert_eq!(change["fork_snapshot_id"], baseline["snapshot_id"]);
    for (body, message) in [
        ("fn reviewed() {}\nfn first_fix() {}\n", "Resolve R01/F01"),
        (
            "fn reviewed() {}\nfn first_fix() {}\nfn second_fix() {}\n",
            "Resolve R02/F01",
        ),
    ] {
        fs::write(edit.join("code.rs"), body).unwrap();
        let snapshot = json(
            &edit,
            &[
                "snapshot",
                "create",
                &task,
                "--message",
                message,
                "--json",
                "--full",
            ],
        );
        assert_eq!(snapshot["change_id"], "C-01");
    }
    assert_eq!(
        json(&edit, &["change", "list", "--local", "--all", "--json"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let finish = json(
        &edit,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    assert_eq!(finish["local_finish"]["change_ref"], format!("{task}/C-01"));
}
#[test]
fn old_receipt_retry_preserves_the_newer_target_and_files() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    let first = parent.join("first");
    let task = quick_initial(&root, &first, "First complete Task");
    fs::write(first.join("code.rs"), "fn first() {}\n").unwrap();
    json(
        &first,
        &[
            "task",
            "finish",
            &task,
            "--local",
            "--message",
            "first",
            "--json",
        ],
    );
    let second = parent.join("second");
    let next = quick_initial(&root, &second, "Second complete Task");
    fs::write(second.join("code.rs"), "fn first() {}\nfn second() {}\n").unwrap();
    let newer = json(
        &second,
        &[
            "task",
            "finish",
            &next,
            "--local",
            "--message",
            "second",
            "--json",
            "--full",
        ],
    );
    let receipts = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    let retry = json(
        &root,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    assert_eq!(
        retry["repo_root_restore"]["reason"],
        "target_advanced_after_successful_apply"
    );
    assert!(fs::read_to_string(root.join("code.rs"))
        .unwrap()
        .contains("second"));
    assert_eq!(
        json(&root, &["line", "show", "main", "--json"])["head_snapshot_id"],
        newer["landed_snapshot_id"]
    );
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        receipts
    );
}
#[test]
fn parallel_same_task_attempts_create_only_one_successful_receipt() {
    use workflow_process::{reply_json, request, Fixture, Workers};
    if workflow_process::run_worker() {
        return;
    }
    let fixture = Fixture::new(false);
    let root = fixture.root.clone();
    let edit = fixture.edit("parallel");
    let task = quick_initial(&root, &edit, "Parallel exact finish attempts");
    fs::write(edit.join("code.rs"), "fn one_apply() {}\n").unwrap();
    let frozen = json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "--message",
            "Frozen candidate",
            "--json",
        ],
    );
    let mut workers = Workers::spawn(
        "parallel_same_task_attempts_create_only_one_successful_receipt",
        2,
    );
    // The stable canonical cwd avoids racing worktree deletion before spawn.
    let attempts = workers.together(
        (0..2)
            .map(|i| {
                (
                    i,
                    request(
                        &root,
                        &["task", "finish", &task, "--local", "--json", "--full"],
                    ),
                )
            })
            .collect(),
    );
    let mut succeeded = false;
    for (_, reply) in &attempts {
        if reply["ok"] == true {
            succeeded = true;
            assert_eq!(
                reply_json(reply)["landed_snapshot_id"],
                frozen["snapshot_id"]
            );
        } else {
            // The winner can remove the resolved execution root between the
            // other process's binding lookup and guarded execution. Only that
            // exact missing-root race is recoverable by the canonical retry.
            let error = reply["stderr"].as_str().unwrap();
            let expected = format!(
                "Error: Unable to verify execution root `{}`:",
                edit.display()
            );
            assert!(
                error.starts_with(&expected)
                    && (error.contains("(os error 2)")
                        || (cfg!(windows) && error.contains("(os error 3)"))),
                "{reply}"
            );
            assert!(
                !edit.exists(),
                "only disappearance after cleanup is accepted"
            );
        }
    }
    assert!(
        succeeded,
        "one concurrent attempt must finish the frozen candidate"
    );
    let completed = json(
        &root,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    assert_eq!(completed["landed_snapshot_id"], frozen["snapshot_id"]);
    assert!(!edit.exists());
    assert_eq!(
        fs::read_to_string(root.join("code.rs")).unwrap(),
        "fn one_apply() {}\n"
    );
    assert_eq!(
        json(&root, &["task", "show", &task, "--local", "--json"])["status"],
        "completed"
    );
    let receipts = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    assert_eq!(receipts.len(), 48);
    for (_, reply) in workers.together(
        (0..2)
            .map(|i| {
                (
                    i,
                    request(
                        &root,
                        &["task", "finish", &task, "--local", "--json", "--full"],
                    ),
                )
            })
            .collect(),
    ) {
        assert_eq!(
            reply_json(&reply)["landed_snapshot_id"],
            frozen["snapshot_id"]
        );
    }
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        receipts
    );
    assert_eq!(
        json(&root, &["line", "show", "main", "--json"])["head_snapshot_id"],
        frozen["snapshot_id"]
    );
    assert_eq!(
        json(&root, &["blame", "code.rs", "--json"])["lines"][0]["task_id"],
        task
    );
    workers.finish();
}
#[test]
fn unfinished_zero_change_remote_ready_has_no_publication_or_binding_side_effects() {
    let (temp, root) = initialized();
    let config_path = root.join(".ait/config.json");
    let mut config: Value = serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["repository_index"] = 7.into();
    config["remotes"] = serde_json::json!({"origin": {
        "remote_id": 1,
        "url": "http://127.0.0.1:1",
        "repo_name": config["repo_name"],
        "created_at": "2026-10-23T00:00:00Z"
    }});
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let edit = temp.path().canonicalize().unwrap().join("capability");
    let task = quick_initial(&root, &edit, "Unsupported remote contract preflight");
    fs::write(edit.join("code.rs"), "fn local_only() {}\n").unwrap();
    json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "--message",
            "Task-only candidate",
            "--json",
        ],
    );
    let before = json(&edit, &["task", "show", &task, "--local", "--json"]);
    let snapshots_before = snapshots(&root);
    let main_before = json(&root, &["line", "show", "main", "--json"])["head_snapshot_id"].clone();
    let rejected = run(
        &edit,
        &[
            "workflow", "ready", &task, "--apply", "--remote", "origin", "--json",
        ],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains(
            "Unfinished Local Task history requires a real successful receipt before package preparation"
        ),
        "{}",
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert_eq!(
        json(&edit, &["task", "show", &task, "--local", "--json"]),
        before
    );
    assert_eq!(snapshots(&root), snapshots_before);
    assert_eq!(
        json(&root, &["line", "show", "main", "--json"])["head_snapshot_id"],
        main_before
    );
    assert!(
        json(&edit, &["change", "list", "--local", "--all", "--json"])
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn upgrade_rejects_unknown_headers_bad_refs_and_earlier_widths_without_output() {
    for case in 0..4 {
        let (_temp, root, target, source) = legacy_receipt_fixture();
        let path = root.join(".ait/binary-db/land.bin");
        let mut invalid = source.clone();
        match case {
            0 => invalid[..4].copy_from_slice(&3u32.to_le_bytes()),
            1 => invalid[8..12].copy_from_slice(&u32::MAX.to_le_bytes()),
            2 => invalid[6] = 63,
            _ => {
                invalid.pop();
            }
        }
        fs::write(&path, &invalid).unwrap();
        let rejected = upgrade(&root, &target, true);
        assert!(!rejected.status.success(), "bad source case {case}");
        assert!(!target.exists());
        assert_eq!(fs::read(&path).unwrap(), invalid);
    }
}
#[test]
fn unrelated_old_staging_is_not_a_runtime_upgrade_input() {
    let (_temp, root, target, _source) = legacy_receipt_fixture();
    let partial = target.with_file_name(".upgraded.capture-staging-aborted");
    fs::create_dir_all(&partial).unwrap();
    fs::write(
        partial.join("incomplete"),
        b"retained interruption evidence",
    )
    .unwrap();
    let recovered = upgrade(&root, &target, true);
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stderr)
    );
    assert!(partial.join("incomplete").is_file());
    assert!(!target.exists());
}

#[test]
fn clean_noop_finish_and_cancellation_keep_distinct_terminal_outcomes() {
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap();
    let edit = parent.join("noop");
    let task = quick_initial(&root, &edit, "No-op complete Task");
    let finish = json(
        &edit,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    assert_eq!(finish["task_status"], "completed");
    assert!(!edit.exists());
    assert_eq!(
        finish["local_finish"]["pre_land_target_snapshot_id"],
        finish["local_finish"]["landed_snapshot_id"]
    );
    let canceled_edit = parent.join("canceled");
    let canceled = quick_initial(&root, &canceled_edit, "Canceled initial work");
    let abandoned = run(&root, &["task", "abandon", &canceled, "--local", "--json"]);
    assert!(!abandoned.status.success());
    assert!(String::from_utf8_lossy(&abandoned.stderr).contains("cleanup is incomplete"));
    assert!(canceled_edit.is_dir(), "explicit roots remain caller-owned");
    assert_eq!(
        json(&root, &["task", "show", &canceled, "--local", "--json"])["status"],
        "abandoned"
    );
    assert!(
        !run(&root, &["task", "finish", &canceled, "--local", "--json"])
            .status
            .success()
    );
    assert!(
        json(&root, &["task", "show", &canceled, "--local", "--json"])["local_finish"].is_null()
    );
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin"))
            .unwrap()
            .len(),
        48
    );
}

#[cfg(unix)]
#[test]
fn committed_apply_survives_task_close_failure_and_retry_closes_only_remaining_phases() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, root) = initialized();
    let edit = temp.path().canonicalize().unwrap().join("close-failure");
    let task = quick_initial(&root, &edit, "Recover Task status after committed apply");
    fs::write(edit.join("code.rs"), "fn durable_apply() {}\n").unwrap();
    json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "--message",
            "Frozen candidate",
            "--json",
        ],
    );
    let path = root.join(".ait/binary-db/task.bin");
    let original = fs::metadata(&path).unwrap().permissions();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    if fs::OpenOptions::new().write(true).open(&path).is_ok() {
        fs::set_permissions(&path, original).unwrap();
        return; // privileged test host
    }
    let attempted = run(
        &edit,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    fs::set_permissions(&path, original).unwrap();
    let partial: Value = serde_json::from_slice(&attempted.stdout).unwrap();
    assert_eq!(partial["apply_status"], "partial");
    assert_eq!(partial["task_status"], "active");
    assert_eq!(partial["task_closeout"]["status"], "failed");
    assert!(edit.exists());
    let receipt = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    assert_eq!(receipt.len(), 48);
    let recovered = json(
        &edit,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    assert_eq!(recovered["task_status"], "completed");
    assert!(!edit.exists());
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        receipt
    );
}

#[cfg(unix)]
#[test]
fn plan_closeout_failure_retains_successful_receipt_and_recovers_without_reapply() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, root) = initialized();
    let edit = temp.path().canonicalize().unwrap().join("plan-failure");
    let task = quick_initial(&root, &edit, "Recover Plan closeout after successful apply");
    fs::write(edit.join("code.rs"), "fn saved_before_plan_closeout() {}\n").unwrap();
    json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "--message",
            "Frozen candidate",
            "--json",
        ],
    );
    let directory = root.join("docs/sprints");
    let original = fs::metadata(&directory).unwrap().permissions();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = directory.join("permission-probe");
    if fs::write(&probe, b"probe").is_ok() {
        fs::remove_file(probe).unwrap();
        fs::set_permissions(&directory, original).unwrap();
        return;
    }
    let attempted = run(
        &edit,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    fs::set_permissions(&directory, original).unwrap();
    let partial: Value = serde_json::from_slice(&attempted.stdout).unwrap();
    assert!(partial.to_string().contains("failed"), "{partial}");
    let receipt = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    assert_eq!(receipt.len(), 48);
    json(
        &root,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        receipt
    );
    assert_eq!(
        json(&root, &["task", "show", &task, "--local", "--json"])["status"],
        "completed"
    );
}

#[test]
fn upgrade_rejects_output_inside_source_authority_before_writing() {
    let (_temp, root, _target, source) = legacy_receipt_fixture();
    for child in [".ait/binary-db/upgrade-child", ".ait/objects/upgrade-child"] {
        let target = root.join(child);
        let rejected = ait_core::binary_db_generation::stage_local_land_upgrade(
            ait_core::binary_db_generation::StageLocalLandUpgradeOptions {
                repo_root: root.clone(),
                output_root: target.clone(),
                source_layout: "local-u64-workflow-v1".into(),
            },
        );
        assert!(rejected.is_err());
        assert!(!target.exists());
        assert_eq!(
            fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
            source
        );
    }
}

#[cfg(unix)]
#[test]
fn failed_worktree_removal_recovers_from_the_receipt_without_another_apply() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, root) = initialized();
    let parent = temp.path().canonicalize().unwrap().join("cleanup-parent");
    fs::create_dir(&parent).unwrap();
    let edit = parent.join("edit");
    let task = quick_initial(&root, &edit, "Recover failed worktree cleanup after apply");
    fs::write(edit.join("code.rs"), "fn applied_once() {}\n").unwrap();
    json(
        &edit,
        &[
            "snapshot",
            "create",
            &task,
            "--message",
            "Frozen candidate",
            "--json",
        ],
    );
    let original = fs::metadata(&parent).unwrap().permissions();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();
    let probe = parent.join("permission-probe");
    if fs::write(&probe, b"probe").is_ok() {
        fs::remove_file(probe).unwrap();
        fs::set_permissions(&parent, original).unwrap();
        return;
    }
    let attempted = run(
        &edit,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    fs::set_permissions(&parent, original).unwrap();
    let partial: Value = serde_json::from_slice(&attempted.stdout).unwrap();
    let receipt = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    assert_eq!(receipt.len(), 48);
    assert!(partial.to_string().contains("failed"), "{partial}");
    assert!(edit.exists());
    let recovered = json(
        &root,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    assert_eq!(
        recovered["bound_worktree_cleanup"]["status"], "removed",
        "{recovered}"
    );
    assert!(!edit.exists(), "{recovered}");
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        receipt
    );
    assert_eq!(
        fs::read_to_string(root.join("code.rs")).unwrap(),
        "fn applied_once() {}\n"
    );
}

#[test]
fn automatic_admission_preserves_stale_stage_and_requires_writer_coordination() {
    use ait_core::binary_db::BinaryDbReadLockSet;
    use ait_core::binary_db_generation::*;
    let (_temp, root, target, source) = legacy_receipt_fixture();
    let guard = BinaryDbReadLockSet::try_acquire(&binary_db_activation_lock_root(&root)).unwrap();
    let blocked = upgrade(&root, &target, true);
    assert!(!blocked.status.success());
    assert!(!target.exists());
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        source
    );
    drop(guard);
    stage_local_land_upgrade(StageLocalLandUpgradeOptions {
        repo_root: root.clone(),
        output_root: target.clone(),
        source_layout: LOCAL_LAND_V1_SOURCE_SELECTOR.into(),
    })
    .unwrap();
    let payload = root.join(".ait/binary-db/task_payload.bin");
    let mut bytes = fs::read(&payload).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(payload, bytes).unwrap();
    let stale = stage_local_land_upgrade(StageLocalLandUpgradeOptions {
        repo_root: root.clone(),
        output_root: target.clone(),
        source_layout: LOCAL_LAND_V1_SOURCE_SELECTOR.into(),
    });
    assert!(stale.is_err());
    assert!(target.is_dir());
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        source
    );
}
#[test]
fn legacy_upgrade_subcommand_is_absent() {
    let (temp, root) = initialized();
    let output = run(&root, &["binary-db", "--help"]);
    drop(temp);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("upgrade-local-land"));
}

#[test]
fn publication_disposition_metadata_is_exact_on_retry_without_content_copy() {
    use ait_core::binary_db_generation::*;
    let (_temp, root, target, _) = legacy_receipt_fixture();
    let before = fs::read(root.join(".ait/binary-db/change.bin")).unwrap();
    let stage = || {
        stage_local_land_upgrade(StageLocalLandUpgradeOptions {
            repo_root: root.clone(),
            output_root: target.clone(),
            source_layout: LOCAL_LAND_V1_SOURCE_SELECTOR.into(),
        })
    };
    let first = stage().unwrap();
    let final_bytes = fs::read(target.join("local/change.bin")).unwrap();
    assert_eq!(final_bytes[5], 0);
    assert_eq!(final_bytes[32], 0);
    assert_eq!(final_bytes[73], 0);
    assert_eq!(final_bytes[100], 0);
    assert_eq!(
        fs::read(root.join(".ait/binary-db/change.bin")).unwrap(),
        before
    );
    assert!(!target.join("source-workflow").exists());
    assert!(!target.join(".ait").exists());
    assert!(!target.join("client-manifest.json").exists());
    let retry = stage().unwrap();
    assert_eq!(retry.status, "staged_retry");
    assert_eq!(retry.content_fingerprint, first.content_fingerprint);
    // A previous preserve-only target is not the approved metadata output.
    let mut preserved = final_bytes.clone();
    for at in [5, 32, 73, 100] {
        preserved[at] = 1;
    }
    fs::write(target.join("local/change.bin"), &preserved).unwrap();
    let error = stage().unwrap_err();
    assert!(
        error.contains("not the complete approved conversion: change.bin"),
        "{error}"
    );
    assert_eq!(
        fs::read(target.join("local/change.bin")).unwrap(),
        preserved
    );
    assert_eq!(
        fs::read(root.join(".ait/binary-db/change.bin")).unwrap(),
        before
    );
    fs::write(target.join("local/change.bin"), final_bytes).unwrap();
    fs::write(target.join("local/undeclared.bin"), 1u32.to_le_bytes()).unwrap();
    assert!(stage().unwrap_err().contains("undeclared schema path"));
}

#[test]
fn publication_disposition_rejects_partial_legacy_edges_without_publishing() {
    for offset in [1, 28] {
        let (_temp, root, target, _) = legacy_receipt_fixture();
        let path = root.join(".ait/binary-db/change.bin");
        let mut before = fs::read(&path).unwrap();
        before[4 + 3 * 68 + offset] = 0;
        fs::write(&path, &before).unwrap();
        assert!(!upgrade(&root, &target, true).status.success());
        assert!(!target.exists());
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn admission_rejects_incomplete_cursors_after_metadata_switch_without_repair() {
    use ait_core::binary_db_generation::*;
    let (_temp, root, target, _source) = legacy_receipt_fixture();
    let cursor = root
        .parent()
        .unwrap()
        .join("active-1063/.ait-worktree/cursor.bin");
    let old = fs::read(&cursor).unwrap();
    stage_local_land_upgrade(StageLocalLandUpgradeOptions {
        repo_root: root.clone(),
        output_root: target.clone(),
        source_layout: LOCAL_LAND_V1_SOURCE_SELECTOR.into(),
    })
    .unwrap();
    // Simulate interruption after the metadata exchange and before cursor writes.
    let discarded = root.join(".ait/discarded-metadata");
    fs::rename(root.join(".ait/binary-db"), &discarded).unwrap();
    fs::rename(target.join("local"), root.join(".ait/binary-db")).unwrap();
    let metadata = fs::read(root.join(".ait/binary-db/change.bin")).unwrap();
    for _ in 0..2 {
        let rejected = run(&root, &["task", "list", "--local", "--json"]);
        assert!(!rejected.status.success());
        assert_eq!(fs::read(&cursor).unwrap(), old);
        assert_eq!(
            fs::read(root.join(".ait/binary-db/change.bin")).unwrap(),
            metadata
        );
        assert!(!target.join("worktree-before-images").exists());
    }
}

#[test]
fn legacy_bootstrap_can_add_an_initial_only_task_without_fixed_total_assumptions() {
    let (_temp, root, target, _source) = legacy_receipt_fixture();
    let db = root.join(".ait/binary-db");
    let mut tasks = fs::read(db.join("task.bin")).unwrap();
    let mut task = tasks[4..68].to_vec();
    task[0] = 0;
    task[1] = 0;
    task[12..24].fill(0);
    task[40..64].fill(0);
    tasks.extend_from_slice(&task);
    let payload = fs::read(db.join("task_payload.bin")).unwrap();
    let offset = u64::from_le_bytes(task[4..12].try_into().unwrap()) as usize;
    let title_len = u16::from_le_bytes(payload[offset..offset + 2].try_into().unwrap()) as usize;
    let title = &payload[offset + 2..offset + 2 + title_len];
    let mut changes = fs::read(db.join("change.bin")).unwrap();
    let index = ((changes.len() - 4) / 68) as u32;
    let mut change = changes[4..72].to_vec();
    change[0] = 0;
    change[16..20].copy_from_slice(&1113u32.to_le_bytes());
    change[20..24].fill(0);
    let mut titles = fs::read(db.join("change_payload.bin")).unwrap();
    change[2..4].copy_from_slice(&(title.len() as u16).to_le_bytes());
    change[8..16].copy_from_slice(&(titles.len() as u64).to_le_bytes());
    titles.extend_from_slice(title);
    changes.extend_from_slice(&change);
    fs::write(db.join("task.bin"), tasks).unwrap();
    fs::write(db.join("change.bin"), changes).unwrap();
    fs::write(db.join("change_payload.bin"), titles).unwrap();
    for name in [
        "task_change_index.bin",
        "task_land_index.bin",
        "change_land_index.bin",
        "task_snapshot_index.bin",
        "change_snapshot_index.bin",
    ] {
        let mut rows = fs::read(db.join(name)).unwrap();
        let mut row = [0u8; 8];
        if name == "task_change_index.bin" {
            row[..4].copy_from_slice(&(index + 1).to_le_bytes());
            row[4] = 1;
            row[6] = 1;
        }
        rows.extend_from_slice(&row);
        fs::write(db.join(name), rows).unwrap();
    }
    let admitted = upgrade(&root, &target, true);
    assert!(
        admitted.status.success(),
        "{}",
        String::from_utf8_lossy(&admitted.stderr)
    );
    assert_eq!(
        (fs::metadata(db.join("task.bin")).unwrap().len() - 4) / 64,
        1114
    );
    assert_eq!(
        (fs::metadata(db.join("change.bin")).unwrap().len() - 4) / 68,
        3
    );
    let task = json(&root, &["task", "show", "LCT-1114", "--local", "--json"]);
    assert_eq!(task["status"], "active");
    let runtime = RepoRuntime::discover_from_path(&root).unwrap();
    assert!(runtime
        .task_store()
        .unwrap()
        .changes_for_task("LCT-1114")
        .unwrap()
        .is_empty());
}
#[test]
fn unexpected_valid_legacy_revision_or_changed_remote_mapping_blocks_cleanup() {
    for unexpected_revision in [true, false] {
        let (_temp, root, target, _source) = legacy_receipt_fixture();
        let db = root.join(".ait/binary-db");
        let mut changes = fs::read(db.join("change.bin")).unwrap();
        if unexpected_revision {
            let old = changes[4..]
                .chunks_exact(68)
                .position(|row| u32::from_le_bytes(row[16..20].try_into().unwrap()) == 1062)
                .unwrap();
            let mut row = changes[4 + old * 68..][..68].to_vec();
            row[4] = 1;
            row[20..24].copy_from_slice(&(old as u32 + 1).to_le_bytes());
            let index = ((changes.len() - 4) / 68) as u32;
            changes.extend_from_slice(&row);
            let mut owners = fs::read(db.join("task_change_index.bin")).unwrap();
            let owner = &mut owners[4 + 1062 * 8..][..8];
            owner[..4].copy_from_slice(&(index + 1).to_le_bytes());
            owner[4] = 2;
            owner[6] = 2;
            fs::write(db.join("task_change_index.bin"), owners).unwrap();
            let mut lands = fs::read(db.join("change_land_index.bin")).unwrap();
            lands.extend_from_slice(&[0; 8]);
            fs::write(db.join("change_land_index.bin"), lands).unwrap();
        } else {
            changes[4 + 3 * 68 + 28] = 2;
        }
        fs::write(db.join("change.bin"), changes.clone()).unwrap();
        let denied = upgrade(&root, &target, true);
        assert!(!denied.status.success());
        let message = String::from_utf8_lossy(&denied.stderr);
        assert!(
            message.contains(if unexpected_revision {
                "Unexpected legacy revision"
            } else {
                "Remote mapping changed"
            }),
            "{message}"
        );
        assert!(!target.exists());
        assert_eq!(fs::read(db.join("change.bin")).unwrap(), changes);
    }
}

#[test]
fn automatic_admission_upgrades_known_empty_format_without_repository_approval() {
    let (_temp, root) = initialized();
    let path = root.join(".ait/binary-db/land.bin");
    assert_eq!(fs::metadata(&path).unwrap().len(), 4);
    fs::write(&path, 1u32.to_le_bytes()).unwrap();
    fs::write(
        root.join(".ait/binary-db/snapshot_link.bin"),
        1u32.to_le_bytes(),
    )
    .unwrap();
    let admitted = run(&root, &["task", "list", "--local", "--json"]);
    assert!(
        admitted.status.success(),
        "{}",
        String::from_utf8_lossy(&admitted.stderr)
    );
    assert_eq!(fs::read(&path).unwrap(), 2u32.to_le_bytes());
    assert_eq!(
        fs::read(root.join(".ait/binary-db/snapshot_link.bin")).unwrap(),
        2u32.to_le_bytes()
    );
    assert!(!root.join(".ait/local-task-receipts-layout2").exists());
}

#[test]
fn admission_does_not_restore_a_missing_authority_from_old_staging() {
    use ait_core::binary_db_generation::*;
    let (_temp, root, target, source) = legacy_receipt_fixture();
    stage_local_land_upgrade(StageLocalLandUpgradeOptions {
        repo_root: root.clone(),
        output_root: target.clone(),
        source_layout: LOCAL_LAND_V1_SOURCE_SELECTOR.into(),
    })
    .unwrap();
    let retired = root.join(".ait/.binary-db.retired-interrupted");
    fs::rename(root.join(".ait/binary-db"), &retired).unwrap();
    let rejected = run(&root, &["task", "list", "--local", "--json"]);
    assert!(!rejected.status.success());
    assert!(!root.join(".ait/binary-db").exists());
    assert_eq!(fs::read(retired.join("land.bin")).unwrap(), source);
}

#[test]
fn concurrent_admission_waits_through_a_coordinated_missing_authority_gap() {
    use ait_core::binary_db::{BinaryDbCommandLockSet, BinaryDbCommandScope};
    use ait_core::binary_db_generation::{
        admit_activated_binary_db_generation_for_runtime, binary_db_activation_lock_root,
    };
    let (_temp, root) = initialized();
    let authority = root.join(".ait/binary-db");
    let coordination = BinaryDbCommandLockSet::try_acquire(
        &binary_db_activation_lock_root(&root),
        BinaryDbCommandScope::General,
    )
    .unwrap()
    .unwrap();
    let withheld = root.join(".ait/withheld-metadata-test");
    fs::rename(&authority, &withheld).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let opened = root.clone();
    let thread = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = admit_activated_binary_db_generation_for_runtime(
            &opened,
            &opened.join(".ait/binary-db"),
            "repo",
        )
        .map(|_| ());
        finished_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    let premature = finished_rx.recv_timeout(std::time::Duration::from_millis(100));
    // Restore the complete profile before releasing coordination, even if the
    // assertion would fail, so the worker is always joined without a lock leak.
    fs::rename(&withheld, &authority).unwrap();
    drop(coordination);
    thread.join().unwrap();
    assert!(
        matches!(premature, Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
        "{premature:?}"
    );
    finished_rx.recv().unwrap().unwrap();
}

#[test]
fn current_profile_admission_waits_for_an_in_progress_metadata_transaction() {
    use ait_core::binary_db::{
        AuthorityId, BinaryDbCommandScope, BinaryFileId, LocalBinaryDbFs, LocalStateScope,
        REPOSITORY_BINARY_DB_BIN_PATHS, REPOSITORY_BINARY_DB_INDEX_PATHS,
    };
    use ait_core::binary_db_generation::admit_activated_binary_db_generation_for_runtime;
    let (_temp, root, _target, _source) = legacy_receipt_fixture();
    let authority = root.join(".ait/binary-db");
    drop(admit_activated_binary_db_generation_for_runtime(&root, &authority, "ait-core").unwrap());
    let before = fs::read(authority.join("task.bin")).unwrap();
    let db = LocalBinaryDbFs::new(
        authority.clone(),
        root.clone(),
        AuthorityId::new("admission-writer-test"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
    let mut transaction = db.begin_write_txn(BinaryDbCommandScope::General).unwrap();
    transaction
        .append_record(
            BinaryFileId::new("task.bin", 1, 64),
            &before[before.len() - 64..],
        )
        .unwrap();
    // A real writer has appended the Task but has not yet appended its indexes.
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();
    let opened = root.clone();
    let thread = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = admit_activated_binary_db_generation_for_runtime(
            &opened,
            &opened.join(".ait/binary-db"),
            "ait-core",
        )
        .map(|_| ());
        finished_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    let premature = finished_rx.recv_timeout(std::time::Duration::from_millis(150));
    transaction.abort().unwrap();
    drop(transaction);
    thread.join().unwrap();
    assert!(
        matches!(premature, Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
        "{premature:?}"
    );
    finished_rx.recv().unwrap().unwrap();
    assert_eq!(fs::read(authority.join("task.bin")).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn current_profile_admission_rejects_symlink_without_external_lock_creation() {
    use ait_core::binary_db_generation::admit_activated_binary_db_generation_for_runtime;
    use std::os::unix::fs::symlink;
    let (_temp, root) = initialized();
    let authority = root.join(".ait/binary-db");
    let external = root.parent().unwrap().join("external-authority");
    fs::rename(&authority, &external).unwrap();
    // Start with clean current metadata and no authority lock directory.
    if external.join(".locks").exists() {
        fs::remove_dir_all(external.join(".locks")).unwrap();
    }
    let before = fs::read_dir(&external)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    symlink(&external, &authority).unwrap();
    let rejected = admit_activated_binary_db_generation_for_runtime(&root, &authority, "repo")
        .map(|_| ())
        .expect_err("indirect authority must reject");
    assert!(
        rejected.contains("symbolic links are forbidden"),
        "{rejected}"
    );
    let after = fs::read_dir(&external)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), fs::read(entry.path()).unwrap())
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(after, before);
    assert!(!external.join(".locks").exists());
    assert!(fs::symlink_metadata(&authority)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
fn current_profile_admission_rejects_aliased_lock_paths_before_writes() {
    use ait_core::binary_db_generation::admit_activated_binary_db_generation_for_runtime;
    use std::os::unix::fs::symlink;
    for relative in [
        ".locks",
        ".locks/binary-db",
        ".locks/binary-db/global.write.lock",
    ] {
        let (_temp, root) = initialized();
        let authority = root.join(".ait/binary-db");
        fs::remove_dir_all(authority.join(".locks")).unwrap();
        let external = root.parent().unwrap().join("external-lock-target");
        let alias = authority.join(relative);
        fs::create_dir_all(alias.parent().unwrap()).unwrap();
        if relative.ends_with(".lock") {
            fs::write(&external, b"unchanged").unwrap();
        } else {
            fs::create_dir(&external).unwrap();
        }
        symlink(&external, &alias).unwrap();
        assert!(
            admit_activated_binary_db_generation_for_runtime(&root, &authority, "repo").is_err()
        );
        if relative.ends_with(".lock") {
            assert_eq!(fs::read(&external).unwrap(), b"unchanged");
            assert_eq!(fs::read_dir(alias.parent().unwrap()).unwrap().count(), 1);
        } else {
            assert_eq!(fs::read_dir(&external).unwrap().count(), 0);
        }
        assert!(fs::symlink_metadata(alias)
            .unwrap()
            .file_type()
            .is_symlink());
    }
}
