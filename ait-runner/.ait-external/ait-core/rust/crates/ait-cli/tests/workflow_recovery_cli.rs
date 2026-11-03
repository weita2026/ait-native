//! Native CLI regressions for candidate diagnostics and integrated Task recovery.
#[path = "support/cli_binary.rs"]
mod cli_binary;
#[path = "support/native_home.rs"]
mod native_home;
#[path = "support/workflow_process.rs"]
mod workflow_process;

use ait_core::json_support::{JsonCodec, JsonValue};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use workflow_process::{finish_closeout, finish_guard_controls, finish_initial, FinishContext};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(cli_binary::selected(env!("CARGO_BIN_EXE_ait-cli")))
        .env("CODEX_HOME", native_home::path())
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
            "recovery-regression",
            "--json",
        ],
    );
    let seed = temp.path().canonicalize().unwrap().join("seed");
    let task = start(&root, &seed, "Fixture baseline");
    fs::write(seed.join("shared.txt"), "base\n").unwrap();
    let accepted = finish(&seed, &task, Some("fixture baseline"));
    assert!(accepted.status.success(), "{}", text(&accepted));
    (temp, root)
}

fn start(root: &Path, edit: &Path, title: &str) -> String {
    json(
        root,
        &[
            "task",
            "quick",
            "--local",
            "--title",
            title,
            "--intent",
            title,
            "--edit-root",
            edit.to_str().unwrap(),
            "--json",
        ],
    )["task_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn finish(edit: &Path, task: &str, message: Option<&str>) -> Output {
    let mut args = vec!["task", "finish", task, "--local", "--json", "--full"];
    if let Some(message) = message {
        args.extend(["--message", message]);
    }
    run(edit, &args)
}

#[test]
fn local_finish_diagnostic_identifies_candidate_and_heads_without_losing_edits() {
    let (temp, root) = initialized();
    let a = temp.path().canonicalize().unwrap().join("a");
    let b = temp.path().canonicalize().unwrap().join("b");
    let task_a = start(&root, &a, "Author A");
    let task_b = start(&root, &b, "Author B");
    fs::write(a.join("shared.txt"), "base\na\n").unwrap();
    let accepted = finish(&a, &task_a, Some("A change"));
    assert!(accepted.status.success(), "{}", text(&accepted));
    let accepted: JsonValue =
        JsonCodec::parse_slice_with_error_prefix(&accepted.stdout, "finish JSON").unwrap();
    let target = accepted["landed_snapshot_id"].as_str().unwrap();
    fs::write(b.join("shared.txt"), "base\nb\n").unwrap();
    let rejected = finish(&b, &task_b, Some("B change"));
    assert!(!rejected.status.success());
    let message = text(&rejected);
    assert!(
        message.contains("Candidate selection: new workspace Snapshot"),
        "{message}"
    );
    assert!(message.contains("Current Line `feature/"), "{message}");
    assert!(
        message.contains(&format!("Target Line `main` head: {target}")),
        "{message}"
    );
    assert!(
        message.contains("ait worktree rebase --onto main"),
        "{message}"
    );
    assert_eq!(
        fs::read_to_string(b.join("shared.txt")).unwrap(),
        "base\nb\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("shared.txt")).unwrap(),
        "base\na\n"
    );
    println!("diagnostic after: {message}");
}

fn integrated_clean_rebase_fixture(
    include_b: bool,
) -> (TempDir, PathBuf, PathBuf, String, String, String) {
    let (temp, root) = initialized();
    let a = temp.path().canonicalize().unwrap().join("integrator");
    let b = temp.path().canonicalize().unwrap().join("author");
    let task_a = start(&root, &a, "Integrate accepted B change");
    let task_b = start(&root, &b, "Submit B change");
    fs::write(b.join("shared.txt"), "base\nb\n").unwrap();
    let submitted = json(
        &b,
        &[
            "snapshot",
            "create",
            &task_b,
            "--message",
            "B submitted change",
            "--json",
        ],
    );
    let old = submitted["snapshot_id"].as_str().unwrap().to_string();
    let line_b = json(&b, &["status", "--json"])["line_name"]
        .as_str()
        .unwrap()
        .to_string();
    fs::write(a.join("shared.txt"), "base\na\n").unwrap();
    json(
        &a,
        &[
            "snapshot",
            "create",
            &task_a,
            "--message",
            "A change",
            "--json",
        ],
    );
    if include_b {
        let merged = json(
            &a,
            &[
                "line",
                "merge",
                &line_b,
                "--message",
                "Integrate B",
                "--json",
            ],
        );
        assert_eq!(merged["status"].as_str(), Some("conflicted"), "{merged}");
        fs::write(a.join("shared.txt"), "base\na\nb\n").unwrap();
        json(
            &a,
            &[
                "line",
                "merge",
                "--continue",
                "--message",
                "Preserve both authors",
                "--json",
            ],
        );
    }
    let accepted = finish(&a, &task_a, None);
    assert!(accepted.status.success(), "{}", text(&accepted));
    let accepted: JsonValue =
        JsonCodec::parse_slice_with_error_prefix(&accepted.stdout, "finish JSON").unwrap();
    let target = accepted["landed_snapshot_id"].as_str().unwrap().to_string();
    let rebased = json(&b, &["worktree", "rebase", "--onto", "main", "--json"]);
    assert_eq!(
        rebased["rebase"]["status"].as_str(),
        Some("conflicted"),
        "{rebased}"
    );
    json(
        &b,
        &[
            "worktree",
            "restore",
            "--snapshot",
            &target,
            "--path",
            "shared.txt",
            "--force",
            "--json",
        ],
    );
    let continued = json(&b, &["worktree", "rebase", "--continue", "--json"]);
    assert_eq!(continued["clean"], JsonValue::Bool(true), "{continued}");
    if include_b {
        let receipt = continued["head_snapshot_id"].as_str().unwrap();
        assert_ne!(
            receipt,
            target.as_str(),
            "integrated rebase must retain Task-owned lineage"
        );
        let snapshot = json(&b, &["snapshot", "show", receipt, "--json"]);
        assert_eq!(
            snapshot["parent_snapshot_id"].as_str(),
            Some(target.as_str()),
            "{snapshot}"
        );
        let repeated_rebase = json(&b, &["worktree", "rebase", "--onto", "main", "--json"]);
        assert_eq!(
            repeated_rebase["head_snapshot_id"].as_str(),
            Some(receipt),
            "{repeated_rebase}"
        );
        let unchanged = run(
            &b,
            &[
                "snapshot",
                "create",
                &task_b,
                "--message",
                "ordinary unchanged checkpoint",
                "--json",
            ],
        );
        assert!(!unchanged.status.success());
        assert!(
            text(&unchanged).contains("workspace tree is unchanged"),
            "{}",
            text(&unchanged)
        );
    } else {
        assert_eq!(
            continued["head_snapshot_id"].as_str(),
            Some(target.as_str())
        );
    }
    assert_eq!(
        continued["fork_snapshot_id"].as_str(),
        Some(target.as_str())
    );
    assert_eq!(
        fs::read_to_string(b.join("shared.txt")).unwrap(),
        if include_b {
            "base\na\nb\n"
        } else {
            "base\na\n"
        }
    );
    assert_eq!(
        fs::read(root.join("shared.txt")).unwrap(),
        fs::read(b.join("shared.txt")).unwrap()
    );
    (temp, root, b, task_b, old, target)
}

#[test]
fn integrated_clean_rebase_finishes_once_without_reintegrating_or_losing_edits() {
    let (_temp, root, b, task, old, target) = integrated_clean_rebase_fixture(true);
    let receipt = json(&b, &["status", "--json"])["head_snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    let finished = finish(&b, &task, None);
    assert!(finished.status.success(), "{}", text(&finished));
    let result: JsonValue =
        JsonCodec::parse_slice_with_error_prefix(&finished.stdout, "finish JSON").unwrap();
    assert_eq!(
        result["landed_snapshot_id"].as_str(),
        Some(receipt.as_str()),
        "{result}"
    );
    assert!(
        !b.exists(),
        "successful native finish must clean its worktree"
    );
    assert_eq!(
        json(&root, &["status", "--json"])["head_snapshot_id"].as_str(),
        Some(receipt.as_str())
    );
    assert_eq!(
        fs::read_to_string(root.join("shared.txt")).unwrap(),
        "base\na\nb\n"
    );
    assert_eq!(
        json(&root, &["task", "show", &task, "--local", "--json"])["status"].as_str(),
        Some("completed")
    );
    // Preserve the immutable submitted Snapshot and make repeated finish safe.
    json(&root, &["snapshot", "show", &old, "--json"]);
    let repeated = finish(&root, &task, None);
    assert!(repeated.status.success(), "{}", text(&repeated));
    assert_eq!(
        json(&root, &["status", "--json"])["head_snapshot_id"].as_str(),
        Some(receipt.as_str())
    );
    println!("repair after: Task {task} completed at Task-owned {receipt}, parent={target}, unchanged integrated content; submitted {old} preserved; repeated finish leaves main unchanged");
}

#[test]
fn clean_rebase_refuses_closeout_when_target_does_not_include_submitted_change() {
    let (_temp, root, b, task, old, target) = integrated_clean_rebase_fixture(false);
    let rejected = finish(&b, &task, None);
    assert!(!rejected.status.success(), "{}", text(&rejected));
    let message = text(&rejected);
    assert!(
        message.contains(&format!("selected revision `{old}`")),
        "{message}"
    );
    assert!(
        message.contains("current Line already matches the target"),
        "{message}"
    );
    assert!(
        !message.contains("Run `ait worktree rebase --onto main`"),
        "{message}"
    );
    assert_eq!(
        json(&root, &["task", "show", &task, "--local", "--json"])["status"].as_str(),
        Some("active")
    );
    assert_eq!(
        json(&root, &["status", "--json"])["head_snapshot_id"].as_str(),
        Some(target.as_str())
    );
    json(&root, &["snapshot", "show", &old, "--json"]);
}

#[test]
fn independent_authors_finish_after_rebase_with_all_changes_preserved() {
    let (temp, root) = initialized();
    let a = temp.path().canonicalize().unwrap().join("a");
    let b = temp.path().canonicalize().unwrap().join("b");
    let task_a = start(&root, &a, "Independent A");
    let task_b = start(&root, &b, "Independent B");
    fs::write(a.join("a.txt"), "A change\n").unwrap();
    let first = finish(&a, &task_a, Some("A change"));
    assert!(first.status.success(), "{}", text(&first));
    fs::write(b.join("b.txt"), "B change\n").unwrap();
    let stale = finish(&b, &task_b, Some("B change"));
    assert!(!stale.status.success(), "{}", text(&stale));
    let rebased = json(&b, &["worktree", "rebase", "--onto", "main", "--json"]);
    assert_eq!(rebased["clean"], JsonValue::Bool(true), "{rebased}");
    let second = finish(&b, &task_b, None);
    assert!(second.status.success(), "{}", text(&second));
    assert_eq!(
        fs::read_to_string(root.join("a.txt")).unwrap(),
        "A change\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("b.txt")).unwrap(),
        "B change\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("shared.txt")).unwrap(),
        "base\n"
    );
}

#[test]
fn native_lifecycle_preserves_content_history_and_cleanup_without_git() {
    use workflow_process::Fixture;
    if workflow_process::gitless_worker(
        "native_lifecycle_preserves_content_history_and_cleanup_without_git",
    ) {
        return;
    }
    let fixture = Fixture::new(true);
    let root = &fixture.root;
    let path = fixture.search_path.as_ref().unwrap();
    let unavailable = Command::new("git")
        .env("PATH", path)
        .current_dir(root)
        .output();
    assert_eq!(
        unavailable.unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    let assert_no_git = |path: &Path| {
        assert_eq!(
            fs::symlink_metadata(path.join(".git")).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    };
    assert_no_git(root);
    let edit = fixture.edit("native-only");
    let started = fixture.json(
        root,
        &[
            "task",
            "quick",
            "--local",
            "--title",
            "Git-free native lifecycle",
            "--intent",
            "Verify native content and history",
            "--edit-root",
            edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    let task = started["task_id"].as_str().unwrap();
    let card = started["quick_card"]["artifact_path"].as_str().unwrap();
    assert_no_git(&edit);
    fs::write(edit.join("native.txt"), "first checkpoint\n").unwrap();
    let checkpoint = fixture.json(
        &edit,
        &[
            "snapshot",
            "create",
            task,
            "--message",
            "first checkpoint",
            "--json",
            "--full",
        ],
    );
    assert_eq!(checkpoint["task_id"], task);
    assert!(checkpoint["change_id"].is_null());
    fs::write(edit.join("native.txt"), "first checkpoint\nfinal change\n").unwrap();
    let diff = workflow_process::cli(&edit, &["diff"], Some(path));
    assert!(diff.status.success(), "{}", text(&diff));
    let diff = String::from_utf8_lossy(&diff.stdout);
    assert!(
        diff.contains("native.txt") && diff.contains("+final change"),
        "{diff}"
    );
    assert_no_git(root);
    assert_no_git(&edit);
    let baseline =
        fixture.json(root, &["line", "show", "main", "--json"])["head_snapshot_id"].clone();
    let context = FinishContext {
        task,
        root,
        edit: &edit,
        baseline: &baseline,
        checkpoint: &checkpoint["snapshot_id"],
        dirty: true,
    };
    let (first, identity) = workflow_process::checked_cli_json(
        &edit,
        &[
            "task",
            "finish",
            task,
            "--local",
            "--message",
            "final change",
            "--json",
            "--full",
        ],
        Some(path),
        |output, value| finish_initial(&context, value, output.status.code(), cfg!(windows)),
    );
    finish_guard_controls(&context, &identity, &first);
    let windows_recovery = cfg!(windows) && first["closeout_status"] == "partial";
    let finished = finish_closeout(&context, first, &identity, Some(path));
    assert_eq!(
        fs::read_to_string(root.join("native.txt")).unwrap(),
        "first checkpoint\nfinal change\n"
    );
    assert!(!edit.exists());
    assert_no_git(root);
    assert_eq!(
        fixture.json(root, &["task", "show", task, "--local", "--json"])["status"],
        "completed"
    );
    assert!(fs::read_to_string(root.join(card))
        .unwrap()
        .contains("- [x]"));
    let blame = fixture.json(root, &["blame", "native.txt", "--json"]);
    let lines = blame["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["snapshot_id"], checkpoint["snapshot_id"]);
    assert_eq!(lines[1]["snapshot_id"], finished["landed_snapshot_id"]);
    for line in lines {
        assert_eq!(line["task_id"], task);
    }
    fixture.json(
        root,
        &[
            "snapshot",
            "show",
            checkpoint["snapshot_id"].as_str().unwrap(),
            "--json",
        ],
    );
    let receipts = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    fixture.json(root, &["task", "finish", task, "--local", "--json"]);
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        receipts
    );
    assert_eq!(
        fixture.json(root, &["line", "show", "main", "--json"])["head_snapshot_id"],
        finished["landed_snapshot_id"]
    );
    if windows_recovery {
        eprintln!("Windows os32 root recovery completed with unchanged receipt and full cleanup");
    }
    workflow_process::gitless_worker_complete();
}

#[test]
fn four_process_tasks_preserve_ownership_through_overlapping_finish_and_retry() {
    use workflow_process::{cli_json, reply_json, request, write_request, Fixture, Workers};
    if workflow_process::run_worker() {
        return;
    }
    let fixture = Fixture::new(false);
    let root = &fixture.root;
    let edits = (0..4)
        .map(|i| fixture.edit(&format!("author-{i}")))
        .collect::<Vec<_>>();
    let tasks = edits
        .iter()
        .enumerate()
        .map(|(i, edit)| {
            let title = format!("Independent author {i}");
            cli_json(
                root,
                &[
                    "task",
                    "quick",
                    "--local",
                    "--title",
                    &title,
                    "--intent",
                    &title,
                    "--edit-root",
                    edit.to_str().unwrap(),
                    "--json",
                ],
            )["task_id"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tasks
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
    let mut workers = Workers::spawn(
        "four_process_tasks_preserve_ownership_through_overlapping_finish_and_retry",
        4,
    );
    let files = (0..4)
        .map(|i| format!("author-{i}.txt"))
        .collect::<Vec<_>>();
    for (_, reply) in workers.together(
        (0..4)
            .map(|i| {
                (
                    i,
                    write_request(
                        &edits[i].join(&files[i]),
                        &format!("accepted author {i}\n"),
                        false,
                    ),
                )
            })
            .collect(),
    ) {
        assert_eq!(reply["ok"], true);
    }
    let mut checkpoints = vec![serde_json::Value::Null; 4];
    for (i, reply) in workers.together(
        (0..4)
            .map(|i| {
                (
                    i,
                    request(
                        &edits[i],
                        &[
                            "snapshot",
                            "create",
                            &tasks[i],
                            "--message",
                            "author checkpoint",
                            "--json",
                        ],
                    ),
                )
            })
            .collect(),
    ) {
        let checkpoint = reply_json(&reply);
        assert_eq!(checkpoint["task_id"], tasks[i]);
        checkpoints[i] = checkpoint["snapshot_id"].clone();
    }
    let baseline = cli_json(root, &["line", "show", "main", "--json"])["head_snapshot_id"].clone();
    let attempts = workers.together(
        (0..4)
            .map(|i| {
                (
                    i,
                    request(
                        &edits[i],
                        &["task", "finish", &tasks[i], "--local", "--json", "--full"],
                    ),
                )
            })
            .collect(),
    );
    let mut applied = Vec::new();
    let mut pending = Vec::new();
    // All four acknowledged replies exist before classification or recovery.
    for (i, reply) in attempts {
        let context = FinishContext {
            task: &tasks[i],
            root,
            edit: &edits[i],
            baseline: &baseline,
            checkpoint: &checkpoints[i],
            dirty: false,
        };
        if let Some((value, identity)) = workflow_process::finish_reply(&context, &reply) {
            applied.push((i, value, identity));
        } else {
            assert_eq!(
                fs::read_to_string(edits[i].join(&files[i])).unwrap(),
                format!("accepted author {i}\n")
            );
            pending.push(i);
        }
    }
    assert_eq!(
        applied.len(),
        1,
        "one common-base candidate may advance main"
    );
    let (winner, value, identity) = applied.pop().unwrap();
    finish_closeout(
        &FinishContext {
            task: &tasks[winner],
            root,
            edit: &edits[winner],
            baseline: &baseline,
            checkpoint: &checkpoints[winner],
            dirty: false,
        },
        value,
        &identity,
        None,
    );
    let mut accepted = vec![winner];
    for i in pending {
        let rebased = reply_json(&workers.one(
            i,
            request(
                &edits[i],
                &["worktree", "rebase", "--onto", "main", "--json"],
            ),
        ));
        assert_eq!(rebased["clean"], true, "{rebased}");
        let older = accepted[0];
        // Race a newly rebased different Task against a retry of an older receipt.
        let results = workers.together(vec![
            (
                i,
                request(
                    &edits[i],
                    &["task", "finish", &tasks[i], "--local", "--json", "--full"],
                ),
            ),
            (
                older,
                request(
                    root,
                    &[
                        "task",
                        "finish",
                        &tasks[older],
                        "--local",
                        "--json",
                        "--full",
                    ],
                ),
            ),
        ]);
        let context = FinishContext {
            task: &tasks[i],
            root,
            edit: &edits[i],
            baseline: &rebased["rebase"]["new_base_snapshot_id"],
            checkpoint: &rebased["rebase"]["new_head_snapshot_id"],
            dirty: false,
        };
        let (value, identity) = workflow_process::finish_reply(&context, &results[0].1)
            .expect("rebased Task must apply");
        // This completed older receipt may validly observe a newer target.
        reply_json(&results[1].1);
        let newer = finish_closeout(&context, value, &identity, None);
        accepted.push(i);
        assert_eq!(
            cli_json(root, &["line", "show", "main", "--json"])["head_snapshot_id"],
            newer["landed_snapshot_id"]
        );
        for &owner in &accepted {
            assert_eq!(
                fs::read_to_string(root.join(&files[owner])).unwrap(),
                format!("accepted author {owner}\n")
            );
        }
    }
    let head = cli_json(root, &["line", "show", "main", "--json"])["head_snapshot_id"].clone();
    let receipts = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
    for (_, reply) in workers.together(
        (0..4)
            .map(|i| {
                (
                    i,
                    request(
                        root,
                        &["task", "finish", &tasks[i], "--local", "--json", "--full"],
                    ),
                )
            })
            .collect(),
    ) {
        reply_json(&reply);
    }
    assert_eq!(
        fs::read(root.join(".ait/binary-db/land.bin")).unwrap(),
        receipts
    );
    assert_eq!(
        cli_json(root, &["line", "show", "main", "--json"])["head_snapshot_id"],
        head
    );
    for i in 0..4 {
        assert!(!edits[i].exists());
        assert_eq!(
            cli_json(root, &["task", "show", &tasks[i], "--local", "--json"])["status"],
            "completed"
        );
        assert_eq!(
            fs::read_to_string(root.join(&files[i])).unwrap(),
            format!("accepted author {i}\n")
        );
        let blame = cli_json(root, &["blame", &files[i], "--json"]);
        let lines = blame["lines"].as_array().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["task_id"], tasks[i]);
    }
    workers.finish();
}
