//! Native CLI regressions for candidate diagnostics and integrated Task recovery.
#[path = "support/cli_binary.rs"]
mod cli_binary;
#[path = "support/native_home.rs"]
mod native_home;
#[path = "support/workflow_process.rs"]
mod workflow_process;

use ait_core::json_support::{JsonCodec, JsonValue};
use serde_json::{json as fixture_json, Value as FixtureValue};
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

struct GitlessFinishContext<'a> {
    task: &'a str,
    root: &'a Path,
    edit: &'a Path,
    baseline: &'a FixtureValue,
    checkpoint: &'a FixtureValue,
}

struct GitlessFinishIdentity {
    snapshot: FixtureValue,
    receipt: FixtureValue,
}

fn finish_fields(value: &FixtureValue, fields: &[(&str, FixtureValue)]) -> Result<(), String> {
    for (field, expected) in fields {
        if value.pointer(field) != Some(expected) {
            return Err(format!(
                "finish {field}: expected {expected}, got {:?}",
                value.pointer(field)
            ));
        }
    }
    Ok(())
}

fn finish_common(
    context: &GitlessFinishContext<'_>,
    identity: &GitlessFinishIdentity,
    value: &FixtureValue,
    recovery: bool,
) -> Result<(), String> {
    finish_fields(
        value,
        &[
            ("/task_id", fixture_json!(context.task)),
            ("/task/task_id", fixture_json!(context.task)),
            ("/task_status", fixture_json!("completed")),
            ("/task/status", fixture_json!("completed")),
            ("/apply_status", fixture_json!("done")),
            (
                "/execution_status",
                fixture_json!(if recovery {
                    "already_landed"
                } else {
                    "applied"
                }),
            ),
            ("/mode", fixture_json!("local")),
            ("/target_line", fixture_json!("main")),
            ("/current_line", fixture_json!("main")),
            ("/line_name", fixture_json!("main")),
            ("/change", FixtureValue::Null),
            ("/change_id", FixtureValue::Null),
            ("/change_ref", FixtureValue::Null),
            ("/change_status", FixtureValue::Null),
            ("/landed_snapshot_id", identity.snapshot.clone()),
            ("/local_finish", identity.receipt.clone()),
            ("/task/local_finish", identity.receipt.clone()),
            ("/local_finish/status", fixture_json!("succeeded")),
            ("/local_finish/task_id", fixture_json!(context.task)),
            ("/local_finish/target_line", fixture_json!("main")),
            ("/local_finish/change_ref", FixtureValue::Null),
            (
                "/local_finish/landed_snapshot_id",
                identity.snapshot.clone(),
            ),
            (
                "/local_finish/pre_land_target_snapshot_id",
                context.baseline.clone(),
            ),
            (
                "/previous_target_head_snapshot_id",
                context.baseline.clone(),
            ),
            ("/workspace_action", fixture_json!("restored")),
            ("/repo_root_restore/status", fixture_json!("restored")),
            ("/repo_root_restore/applied", fixture_json!(true)),
            (
                "/repo_root_restore/workspace_root",
                fixture_json!(context.root),
            ),
            (
                "/repo_root_restore/target_snapshot_id",
                identity.snapshot.clone(),
            ),
            ("/repo_root_restore/dirty_outside_paths", fixture_json!([])),
            ("/plan_checklist_closeout/status", fixture_json!("synced")),
            (
                "/plan_markdown_materialization/status",
                fixture_json!("in_sync"),
            ),
            ("/plan_markdown_materialization/drift", fixture_json!([])),
            ("/task_land_contract/scope", fixture_json!("local")),
            (
                "/task_land_contract/version",
                fixture_json!("task-land-plan-closeout/v1"),
            ),
            (
                "/task_land_contract/recovery_policy",
                fixture_json!("idempotent_phase_resume"),
            ),
        ],
    )?;
    if identity.receipt["land_ordinal"].as_u64().is_none() {
        return Err("finish receipt has no valid ordinal".into());
    }
    if !recovery || value.get("post_finish_hooks").is_some() {
        finish_fields(
            value,
            &[
                ("/post_finish_hooks/status", fixture_json!("skipped")),
                (
                    "/post_finish_hooks/reason",
                    fixture_json!("no_hooks_configured"),
                ),
            ],
        )?;
    }
    if recovery {
        finish_fields(
            value,
            &[
                ("/auto_snapshot", FixtureValue::Null),
                ("/repo_root_restore/plan/write_count", fixture_json!(0)),
                ("/repo_root_restore/plan/remove_count", fixture_json!(0)),
            ],
        )
    } else {
        finish_fields(
            value,
            &[
                ("/auto_snapshot/snapshot_id", identity.snapshot.clone()),
                ("/auto_snapshot/task_id", fixture_json!(context.task)),
                ("/auto_snapshot/change_id", FixtureValue::Null),
                ("/auto_snapshot/newly_created", fixture_json!(true)),
                (
                    "/auto_snapshot/parent_snapshot_id",
                    context.checkpoint.clone(),
                ),
            ],
        )
    }
}

fn finish_other_phases(value: &FixtureValue, allowed: &[&str]) -> Result<(), String> {
    for (phase, component) in value.as_object().ok_or("finish is not an object")? {
        if !allowed.contains(&phase.as_str())
            && matches!(
                component["status"].as_str(),
                Some("failed" | "blocked" | "partial" | "pending" | "deferred")
            )
        {
            return Err(format!("unrelated finish failure: {phase}"));
        }
    }
    Ok(())
}

fn finish_windows_partial(
    context: &GitlessFinishContext<'_>,
    identity: &GitlessFinishIdentity,
    value: &FixtureValue,
) -> Result<(), String> {
    finish_common(context, identity, value, false)?;
    finish_fields(
        value,
        &[
            ("/closeout_status", fixture_json!("partial")),
            ("/bound_worktree_cleanup/status", fixture_json!("failed")),
            (
                "/bound_worktree_cleanup/task_id",
                fixture_json!(context.task),
            ),
            ("/bound_line_closeout/status", fixture_json!("failed")),
            (
                "/bound_line_closeout/reason",
                fixture_json!("feature_line_closeout_failed"),
            ),
            (
                "/bound_line_closeout/error",
                fixture_json!("Bound worktree cleanup failed before Line closeout."),
            ),
            ("/bound_line_closeout/task_id", fixture_json!(context.task)),
            (
                "/bound_line_closeout/task_status",
                fixture_json!("completed"),
            ),
            ("/main_seed_sync/status", fixture_json!("pending")),
            (
                "/main_seed_sync/reason",
                fixture_json!("task_closeout_recovery_pending"),
            ),
            ("/main_seed_sync/line_name", fixture_json!("main")),
            ("/automatic_reconciliation/status", fixture_json!("skipped")),
            (
                "/automatic_reconciliation/reason",
                fixture_json!("task_closeout_recovery_pending"),
            ),
            (
                "/closeout_recovery/code",
                fixture_json!("resume_task_land_closeout"),
            ),
            (
                "/closeout_recovery/command",
                fixture_json!(format!("ait task finish {} --local", context.task)),
            ),
            ("/closeout_recovery/idempotent", fixture_json!(true)),
        ],
    )?;
    if !value["bound_worktree_cleanup"]["error"]
        .as_str()
        .is_some_and(|error| error.ends_with("(os error 32)"))
    {
        return Err("finish cleanup is not exactly Windows sharing violation32".into());
    }
    finish_other_phases(
        value,
        &[
            "bound_worktree_cleanup",
            "bound_line_closeout",
            "main_seed_sync",
        ],
    )
}

fn finish_complete(
    context: &GitlessFinishContext<'_>,
    identity: &GitlessFinishIdentity,
    value: &FixtureValue,
    recovery: bool,
) -> Result<(), String> {
    finish_common(context, identity, value, recovery)?;
    finish_fields(
        value,
        &[
            ("/closeout_status", fixture_json!("complete")),
            ("/bound_worktree_cleanup/status", fixture_json!("removed")),
            (
                "/bound_worktree_cleanup/task_id",
                fixture_json!(context.task),
            ),
            (
                "/bound_worktree_cleanup/worktree/removed",
                fixture_json!(true),
            ),
            (
                "/bound_worktree_cleanup/worktree/deleted_path",
                fixture_json!(true),
            ),
            (
                "/bound_worktree_cleanup/worktree/path",
                fixture_json!(context.edit),
            ),
            ("/bound_line_closeout/status", fixture_json!("archived")),
            ("/bound_line_closeout/task_id", fixture_json!(context.task)),
            (
                "/bound_line_closeout/task_status",
                fixture_json!("completed"),
            ),
            ("/bound_line_closeout/target_line", fixture_json!("main")),
            (
                "/bound_line_closeout/expected_revision_snapshot_id",
                identity.snapshot.clone(),
            ),
            (
                "/bound_line_closeout/local/line/status",
                fixture_json!("archived"),
            ),
            (
                "/bound_line_closeout/local/line/head_snapshot_id",
                identity.snapshot.clone(),
            ),
            (
                "/automatic_reconciliation/status",
                fixture_json!("completed"),
            ),
        ],
    )?;
    if !value
        .get("closeout_recovery")
        .is_none_or(FixtureValue::is_null)
        || value["main_seed_sync"]["status"].as_str().is_none()
    {
        return Err("finish still requires closeout recovery".into());
    }
    finish_other_phases(value, &[])
}

fn finish_initial(
    context: &GitlessFinishContext<'_>,
    value: &FixtureValue,
    code: Option<i32>,
    windows: bool,
) -> Result<GitlessFinishIdentity, String> {
    let snapshot = value["auto_snapshot"]["snapshot_id"].clone();
    if !snapshot.as_str().is_some_and(|id| {
        id.len() == 16
            && id.starts_with("SNP-")
            && id.as_bytes()[4..]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(b))
    }) {
        return Err("finish did not create the exact Task Snapshot".into());
    }
    let identity = GitlessFinishIdentity {
        snapshot,
        receipt: value["local_finish"].clone(),
    };
    match code {
        Some(0) => finish_complete(context, &identity, value, false)?,
        Some(2) if windows => finish_windows_partial(context, &identity, value)?,
        _ => return Err(format!("unexpected first finish exit {code:?}")),
    }
    Ok(identity)
}

fn finish_no_reapply(
    before: &[u8],
    after: &[u8],
    count: &FixtureValue,
    recovered_count: &FixtureValue,
) -> Result<(), String> {
    if before != after || count.as_u64().is_none() || count != recovered_count {
        Err("finish recovery repeated apply or created an extra Snapshot".into())
    } else {
        Ok(())
    }
}

fn finish_recovered(
    context: &GitlessFinishContext<'_>,
    identity: &GitlessFinishIdentity,
    value: &FixtureValue,
    code: Option<i32>,
) -> Result<(), String> {
    if code != Some(0) {
        return Err(format!("root finish recovery failed with {code:?}"));
    }
    finish_complete(context, identity, value, true)
}

fn finish_guard_controls(
    context: &GitlessFinishContext<'_>,
    identity: &GitlessFinishIdentity,
    first: &FixtureValue,
) {
    // Exercise response guards on every host; only a real Windows run can
    // prove the actual partial-closeout/root-resume path below.
    let mut partial = first.clone();
    partial["closeout_status"] = fixture_json!("partial");
    partial["bound_worktree_cleanup"] = fixture_json!({
        "status":"failed", "task_id":context.task,
        "error":"The process cannot access the file because it is being used by another process. (os error 32)"
    });
    partial["bound_line_closeout"] = fixture_json!({
        "status":"failed", "reason":"feature_line_closeout_failed",
        "error":"Bound worktree cleanup failed before Line closeout.",
        "task_id":context.task, "task_status":"completed"
    });
    partial["main_seed_sync"] = fixture_json!({
        "status":"pending", "reason":"task_closeout_recovery_pending", "line_name":"main"
    });
    partial["automatic_reconciliation"] = fixture_json!({
        "status":"skipped", "reason":"task_closeout_recovery_pending"
    });
    partial["closeout_recovery"] = fixture_json!({
        "code":"resume_task_land_closeout", "idempotent":true,
        "command":format!("ait task finish {} --local", context.task)
    });
    finish_initial(context, &partial, Some(2), true).unwrap();
    assert!(finish_initial(context, &partial, Some(2), false).is_err());
    assert!(finish_initial(context, &partial, Some(1), true).is_err());
    assert!(finish_initial(context, &partial, Some(0), true).is_err());
    let common_drift = [
        ("/task_id", fixture_json!("LT-9999")),
        ("/landed_snapshot_id", fixture_json!("wrong-snapshot")),
        (
            "/local_finish/land_ordinal",
            fixture_json!(identity.receipt["land_ordinal"].as_u64().unwrap() + 1),
        ),
        ("/task/local_finish/target_line", fixture_json!("other")),
        ("/target_line", fixture_json!("other")),
        ("/apply_status", fixture_json!("failed")),
        ("/task_status", fixture_json!("active")),
        ("/task/status", fixture_json!("active")),
        ("/plan_checklist_closeout/status", fixture_json!("failed")),
        (
            "/plan_markdown_materialization/status",
            fixture_json!("failed"),
        ),
        ("/repo_root_restore/status", fixture_json!("failed")),
    ];
    for (field, replacement) in &common_drift {
        let mut invalid = partial.clone();
        *invalid.pointer_mut(field).unwrap() = replacement.clone();
        assert!(
            finish_windows_partial(context, identity, &invalid).is_err(),
            "accepted {field}"
        );
    }
    for (field, replacement) in [
        (
            "/bound_worktree_cleanup/error",
            fixture_json!("sharing violation (os error 320)"),
        ),
        (
            "/bound_worktree_cleanup/error",
            fixture_json!("Permission denied (os error 13)"),
        ),
        ("/bound_line_closeout/reason", fixture_json!("unrelated")),
        ("/main_seed_sync/status", fixture_json!("failed")),
        (
            "/automatic_reconciliation/reason",
            fixture_json!("unrelated"),
        ),
        ("/post_finish_hooks/status", fixture_json!("failed")),
        (
            "/closeout_recovery/command",
            fixture_json!("ait task finish LT-9999 --local"),
        ),
        ("/closeout_recovery/idempotent", fixture_json!(false)),
    ] {
        let mut invalid = partial.clone();
        *invalid.pointer_mut(field).unwrap() = replacement;
        assert!(
            finish_windows_partial(context, identity, &invalid).is_err(),
            "accepted {field}"
        );
    }
    let mut unrelated = partial.clone();
    unrelated["other_mandatory_step"] = fixture_json!({"status":"failed"});
    assert!(finish_windows_partial(context, identity, &unrelated).is_err());

    let mut recovered = first.clone();
    recovered["execution_status"] = fixture_json!("already_landed");
    recovered["auto_snapshot"] = FixtureValue::Null;
    recovered["closeout_status"] = fixture_json!("complete");
    recovered["closeout_recovery"] = FixtureValue::Null;
    recovered["bound_worktree_cleanup"] = fixture_json!({
        "status":"removed", "task_id":context.task,
        "worktree":{"removed":true, "deleted_path":true, "path":context.edit}
    });
    recovered["bound_line_closeout"] = fixture_json!({
        "status":"archived", "task_id":context.task, "task_status":"completed",
        "target_line":"main", "expected_revision_snapshot_id":identity.snapshot,
        "local":{"line":{"status":"archived", "head_snapshot_id":identity.snapshot}}
    });
    recovered["main_seed_sync"] = fixture_json!({"status":"refreshed"});
    recovered["automatic_reconciliation"] = fixture_json!({"status":"completed"});
    recovered["repo_root_restore"]["plan"]["write_count"] = fixture_json!(0);
    recovered["repo_root_restore"]["plan"]["remove_count"] = fixture_json!(0);
    finish_recovered(context, identity, &recovered, Some(0)).unwrap();
    assert!(finish_recovered(context, identity, &recovered, Some(2)).is_err());
    for (field, replacement) in common_drift.iter().cloned().chain([
        ("/execution_status", fixture_json!("applied")),
        ("/auto_snapshot", fixture_json!({"newly_created":true})),
        ("/closeout_status", fixture_json!("partial")),
        ("/bound_worktree_cleanup/status", fixture_json!("failed")),
        (
            "/bound_worktree_cleanup/worktree/removed",
            fixture_json!(false),
        ),
        (
            "/bound_worktree_cleanup/worktree/path",
            fixture_json!("other-worktree"),
        ),
        ("/bound_line_closeout/status", fixture_json!("active")),
        ("/main_seed_sync/status", fixture_json!("pending")),
        ("/repo_root_restore/plan/write_count", fixture_json!(1)),
        ("/repo_root_restore/plan/remove_count", fixture_json!(1)),
    ]) {
        let mut invalid = recovered.clone();
        *invalid.pointer_mut(field).unwrap() = replacement;
        assert!(
            finish_recovered(context, identity, &invalid, Some(0)).is_err(),
            "accepted {field}"
        );
    }
    assert!(
        finish_no_reapply(b"receipt", b"receipt", &fixture_json!(3), &fixture_json!(3)).is_ok()
    );
    assert!(finish_no_reapply(
        b"receipt",
        b"duplicate",
        &fixture_json!(3),
        &fixture_json!(3)
    )
    .is_err());
    assert!(
        finish_no_reapply(b"receipt", b"receipt", &fixture_json!(3), &fixture_json!(4)).is_err()
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
    let context = GitlessFinishContext {
        task,
        root,
        edit: &edit,
        baseline: &baseline,
        checkpoint: &checkpoint["snapshot_id"],
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
    // One recovery only. The original already-complete retry at the end of
    // this case still checks that later retries preserve the same receipt.
    let finished = if windows_recovery {
        let receipts = fs::read(root.join(".ait/binary-db/land.bin")).unwrap();
        let count = fixture.json(root, &["status", "--json", "--full"])["snapshot_count"].clone();
        let (recovered, ()) = workflow_process::checked_cli_json(
            root,
            &["task", "finish", task, "--local", "--json", "--full"],
            Some(path),
            |output, value| {
                finish_recovered(&context, &identity, value, output.status.code())?;
                let after = fs::read(root.join(".ait/binary-db/land.bin"))
                    .map_err(|error| error.to_string())?;
                let state = fixture.json(root, &["status", "--json", "--full"]);
                finish_no_reapply(&receipts, &after, &count, &state["snapshot_count"])?;
                if state["head_snapshot_id"] != identity.snapshot || edit.exists() {
                    return Err("root recovery changed target or left the worktree".into());
                }
                Ok(())
            },
        );
        recovered
    } else {
        first
    };
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
        assert_eq!(reply_json(&reply)["task_id"], tasks[i]);
    }
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
    let mut accepted = Vec::new();
    let mut pending = Vec::new();
    for (i, reply) in attempts {
        if reply["ok"] == true {
            accepted.push(i);
        } else {
            assert!(
                reply["stderr"]
                    .as_str()
                    .unwrap()
                    .contains("ait worktree rebase --onto main"),
                "worker {i}, Task {}, execution root {}: {reply}",
                tasks[i],
                edits[i].display()
            );
            assert_eq!(
                fs::read_to_string(edits[i].join(&files[i])).unwrap(),
                format!("accepted author {i}\n")
            );
            pending.push(i);
        }
    }
    assert_eq!(
        accepted.len(),
        1,
        "one common-base candidate may advance main"
    );
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
        let newer = reply_json(&results[0].1);
        reply_json(&results[1].1);
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
