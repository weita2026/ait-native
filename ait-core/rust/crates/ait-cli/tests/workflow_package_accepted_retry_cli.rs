//! Actual CLI accepted retries never prepare or publish another candidate.
#[path = "support/workflow_package_fixture.rs"]
mod package_fixture;
use ait_core::json_support::{json, JsonValue};
use ait_core::local_snapshot::LocalSnapshotTreeReadStore;
use package_fixture::*;
use std::fs;
use std::path::Path;
use std::process::Output;

#[path = "support/workflow_package_routed_resume.rs"]
mod routed_resume;

#[test]
fn fixture_local_finish_closes_the_worktree_with_one_apply_receipt() {
    let fixture = Fixture::new();
    let (task, edit) = fixture.start("local-closeout");
    fs::write(edit.join("proof.txt"), "one accepted apply\n").unwrap();
    let snapshot = fixture.finish_local(&task, &edit);
    let receipt = fixture.task(&task)["local_finish"].clone();
    let snapshots = fixture.snapshots();

    let repeated = cli(
        &fixture.root,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );

    assert_eq!(repeated["local_finish"], receipt);
    assert_eq!(repeated["landed_snapshot_id"], snapshot);
    assert!(repeated["auto_snapshot"].is_null());
    assert_eq!(fixture.snapshots(), snapshots);
    assert!(!edit.exists());
}

#[cfg(windows)]
#[test]
fn fixture_local_finish_preserves_nonempty_failed_cleanup() {
    use std::os::windows::fs::OpenOptionsExt;

    let fixture = Fixture::new();
    let (task, edit) = fixture.start("blocked-local-closeout");
    let proof = edit.join("proof.txt");
    fs::write(&proof, "preserve the blocked file\n").unwrap();
    // Permit Snapshot reads while withholding FILE_SHARE_DELETE from cleanup.
    const FILE_SHARE_READ: u32 = 1;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(&proof)
        .unwrap();

    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fixture.finish_local(&task, &edit)
    }));
    assert!(failed.is_err());
    let receipt = fixture.task(&task)["local_finish"].clone();
    assert_eq!(receipt["status"], "succeeded");
    let snapshots = fixture.snapshots();
    drop(held);

    let retry = run(
        &fixture.root,
        &["task", "finish", &task, "--local", "--json", "--full"],
    );
    assert!(!retry.status.success());
    let retry = output_json(&retry);
    assert_eq!(retry["local_finish"], receipt);
    assert_eq!(retry["closeout_status"], "partial");
    assert_eq!(fixture.snapshots(), snapshots);
    assert_eq!(fs::read(&proof).unwrap(), b"preserve the blocked file\n");
}

#[test]
#[ignore = "requires exact reviewed Server and native Runner artifacts plus a CI RAM root"]
fn reviewed_routed_server_preserves_sealed_mode_across_real_cli_processes() {
    routed_resume::run_pair();
}

#[test]
#[ignore = "requires reviewed retained-source Server and pinned native Runner artifacts"]
fn reviewed_routed_server_restart_reprepares_the_original_sealed_candidate() {
    routed_resume::run_reset_pair();
}

fn defaults(fixture: &Fixture, teams: bool, mode: &str) {
    let path = fixture.root.join(".ait/config.json");
    let mut value: JsonValue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["agent_teams"] = json!(teams);
    value["default_author_mode"] = json!(mode);
    fs::write(path, value.to_string()).unwrap();
}

fn ready(fixture: &Fixture, task: &str, pin: Option<&Path>, mode: Option<&str>) -> Output {
    let mut args = vec![
        "workflow", "ready", task, "--apply", "--remote", "origin", "--json", "--full",
    ];
    if let Some(pin) = pin {
        args.extend(["--expected-identity", pin.to_str().unwrap()]);
    }
    if let Some(mode) = mode {
        args.extend(["--author-mode", mode]);
    }
    run(&fixture.root, &args)
}

fn finish(fixture: &Fixture, task: &str, pin: Option<&Path>) -> Output {
    let mut args = vec![
        "workflow", "finish", task, "--apply", "--remote", "origin", "--json", "--full",
    ];
    if let Some(pin) = pin {
        args.extend(["--expected-identity", pin.to_str().unwrap()]);
    }
    run(&fixture.root, &args)
}

fn accepted(output: &Output, identity: &JsonValue) -> JsonValue {
    let value = output_json(output);
    assert_eq!(value["expected_identity"], *identity, "{value}");
    assert_eq!(
        value["completion"]["expected_identity"], *identity,
        "{value}"
    );
    assert_eq!(value["completion"]["outcome"], "incomplete", "{value}");
    assert!(value["completion"]["patchset_id"].as_str().is_some());
    assert!(value["completion"]["submission_id"].as_str().is_some());
    assert_eq!(value["completion"]["entries"], json!([]));
    assert_eq!(value["apply_status"], "partial");
    assert_eq!(value["closeout_status"], "partial");
    assert_eq!(value["next_action"]["code"], "restore_publication_mappings");
    value
}

fn seal(fixture: &Fixture, task: &str) -> (JsonValue, std::path::PathBuf) {
    let output = ready(fixture, task, None, None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = output_json(&output);
    assert_eq!(value["next_action"]["code"], "done", "{value}");
    let identity = value["expected_identity"].clone();
    let pin = fixture.identity(&identity, "original.identity.json");
    (identity, pin)
}

#[test]
fn accepted_ready_and_finish_keep_the_original_batch_without_pin_after_newer_head_and_restart() {
    let fixture = Fixture::new();
    let (tasks, original_final) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    defaults(&fixture, true, "ai_with_human_review");
    let (identity, pin) = seal(&fixture, selected);
    fixture.finish(selected, &pin);
    let (later, edit) = fixture.start("later");
    fs::write(edit.join("later.txt"), "later content remains Local").unwrap();
    let later_head = fixture.finish_local(&later, &edit);
    let before = tasks
        .iter()
        .chain([&later])
        .map(|id| fixture.task(id))
        .collect::<Vec<_>>();
    let snapshots = fixture.snapshots();
    defaults(&fixture, false, "human_only");
    let (request_start, batch) = {
        let mut state = fixture.state.lock().unwrap();
        state.archive_missing = true;
        state.omit_membership = true;
        (state.requests.len(), state.batches[0].clone())
    };
    for explicit in [false, true] {
        let selected_pin = explicit.then_some(pin.as_path());
        let ready_result = accepted(&ready(&fixture, selected, selected_pin, None), &identity);
        let finish_result = accepted(&finish(&fixture, selected, selected_pin), &identity);
        assert_eq!(ready_result["completion"], finish_result["completion"]);
        for field in ["patchset_id", "submission_id"] {
            assert_eq!(
                ready_result["completion"][field],
                batch.accepted.as_ref().unwrap()[field]
            );
        }
    }
    let compact = run(
        &fixture.root,
        &[
            "workflow", "ready", selected, "--apply", "--remote", "origin", "--json",
        ],
    );
    accepted(&compact, &identity);
    let text = run(
        &fixture.root,
        &[
            "workflow", "finish", selected, "--apply", "--remote", "origin",
        ],
    );
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("accepted"));
    assert!(text.contains(
        batch.accepted.as_ref().unwrap()["patchset_id"]
            .as_str()
            .unwrap()
    ));
    assert_eq!(fixture.head(), later_head);
    assert_eq!(fixture.snapshots(), snapshots);
    assert_eq!(
        tasks
            .iter()
            .chain([&later])
            .map(|id| fixture.task(id))
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("later.txt")).unwrap(),
        "later content remains Local"
    );
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.head, original_final);
    assert_eq!(state.batches.len(), 1);
    assert_eq!(state.finishes.len(), 1);
    assert_eq!(state.batches[0].manifest, batch.manifest);
    assert_eq!(state.batches[0].bytes, batch.bytes);
    assert_eq!(state.batches[0].evidence, batch.evidence);
    assert!(state.requests[request_start..]
        .iter()
        .all(|(method, _)| method == "GET"));
}

#[test]
fn accepted_partial_and_lost_responses_resolve_the_original_candidate_without_local_mappings() {
    for lost in [false, true] {
        let fixture = Fixture::new();
        let (tasks, _) = fixture.initial_batch();
        let selected = tasks.last().unwrap();
        let (identity, pin) = seal(&fixture, selected);
        {
            let mut state = fixture.state.lock().unwrap();
            state.incomplete_response = !lost;
            state.lose_response = lost;
        }
        accepted(&fixture.finish_output(selected, &pin), &identity);
        let before = tasks.iter().map(|id| fixture.task(id)).collect::<Vec<_>>();
        assert!(before.iter().all(|row| row["published_task_id"].is_null()));
        let snapshots = fixture.snapshots();
        let request_start = {
            let mut state = fixture.state.lock().unwrap();
            state.archive_missing = true;
            state.omit_membership = true;
            state.requests.len()
        };
        accepted(&ready(&fixture, selected, None, None), &identity);
        accepted(&finish(&fixture, selected, None), &identity);
        assert_eq!(fixture.snapshots(), snapshots);
        assert_eq!(
            tasks.iter().map(|id| fixture.task(id)).collect::<Vec<_>>(),
            before
        );
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.batches.len(), 1);
        assert_eq!(state.finishes.len(), 1);
        assert!(state.requests[request_start..]
            .iter()
            .all(|(method, _)| method == "GET"));
    }
}

#[test]
fn conflicting_identity_or_failed_formal_reads_hold_without_borrowing_success_or_starting_ci() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    let (identity, pin) = seal(&fixture, selected);
    fixture.finish(selected, &pin);
    let before = tasks.iter().map(|id| fixture.task(id)).collect::<Vec<_>>();
    let snapshots = fixture.snapshots();
    let request_start = fixture.state.lock().unwrap().requests.len();
    for field in [
        "package_hash",
        "worker_job_index",
        "base_snapshot_id",
        "revision_snapshot_id",
    ] {
        let mut wrong = identity.clone();
        wrong[field] = match field {
            "package_hash" => json!("00".repeat(32)),
            "worker_job_index" => json!(99),
            _ => json!("SNP-FFFFFFFFFFFF"),
        };
        let path = fixture.identity(&wrong, "wrong.identity.json");
        let value = output_json(&finish(&fixture, selected, Some(&path)));
        assert_eq!(
            value["next_action"]["code"], "completion_unknown",
            "{value}"
        );
        assert!(value["completion"]["patchset_id"].is_null());
    }
    fixture.state.lock().unwrap().fail_formal_reads = true;
    for output in [
        ready(&fixture, selected, None, None),
        finish(&fixture, selected, None),
    ] {
        let value = output_json(&output);
        assert_eq!(
            value["next_action"]["code"], "completion_unknown",
            "{value}"
        );
        assert!(value["completion"]["patchset_id"].is_null());
    }
    fixture.state.lock().unwrap().fail_formal_reads = false;
    fixture.state.lock().unwrap().corrupt_key = true;
    let value = output_json(&finish(&fixture, selected, Some(&pin)));
    assert_eq!(
        value["next_action"]["code"], "completion_unknown",
        "{value}"
    );
    assert!(value["completion"]["patchset_id"].is_null());
    assert_eq!(fixture.snapshots(), snapshots);
    assert_eq!(
        tasks.iter().map(|id| fixture.task(id)).collect::<Vec<_>>(),
        before
    );
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.batches.len(), 1);
    assert_eq!(state.finishes.len(), 1);
    assert!(state.requests[request_start..]
        .iter()
        .all(|(method, _)| method == "GET"));
}

#[test]
fn accepted_mode_is_immutable_and_verified_precommit_mode_ignores_changed_defaults() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    defaults(&fixture, true, "ai_with_human_review");
    let (identity, pin) = seal(&fixture, selected);
    defaults(&fixture, false, "human_only");
    let original = fixture.state.lock().unwrap().batches[0].clone();
    let before = fixture.state.lock().unwrap().requests.len();
    let conflict = ready(&fixture, selected, Some(&pin), Some("human_only"));
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("sealed package"));
    {
        let mut state = fixture.state.lock().unwrap();
        assert!(state.requests[before..]
            .iter()
            .all(|(method, _)| method == "GET"));
        state.pending_execution_reads = 2;
    }
    let pending = output_json(&ready(&fixture, selected, Some(&pin), None));
    assert_eq!(pending["next_action"]["code"], "done", "{pending}");
    assert_eq!(pending["expected_identity"], identity);
    assert_eq!(pending["package_execution"]["author_mode"], MODE);
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.batches.len(), 1);
        assert!(state.finishes.is_empty());
        assert_eq!(state.batches[0].bytes, original.bytes);
        assert_eq!(state.batches[0].manifest, original.manifest);
        assert_eq!(state.batches[0].evidence, original.evidence);
        assert!(
            state.requests[before..].iter().all(|(method, path)| {
                method == "GET"
                    || (method == "POST"
                        && (path.ends_with("/worker-jobs/0")
                            || path == "/v1/native/repository-authorities"))
            }),
            "{:?}",
            &state.requests[before..]
        );
    }
    fixture.finish(selected, &pin);
    let before = {
        let mut state = fixture.state.lock().unwrap();
        state.archive_missing = true;
        state.requests.len()
    };
    accepted(&ready(&fixture, selected, None, None), &identity);
    let conflict = ready(&fixture, selected, None, Some("human_only"));
    assert!(!conflict.status.success());
    assert!(String::from_utf8_lossy(&conflict.stderr).contains("sealed mode"));
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.batches.len(), 1);
    assert_eq!(state.batches[0].manifest["author_mode"], MODE);
    assert!(state.requests[before..]
        .iter()
        .all(|(method, _)| method == "GET"));
}

#[test]
fn preformal_mode_read_holds_without_exact_authority_even_with_an_explicit_mode() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    let (identity, pin) = seal(&fixture, selected);
    defaults(&fixture, false, "human_only");
    let original = fixture.state.lock().unwrap().batches[0].clone();
    let before_tasks = tasks
        .iter()
        .map(|task| fixture.task(task))
        .collect::<Vec<_>>();
    let snapshots = fixture.snapshots();
    let head = fixture.head();
    let request_start = fixture.state.lock().unwrap().requests.len();
    let execution = json!({"expected_identity":identity,"ci_state":"passed","availability":"available","recoverable":true,"author_mode":MODE});
    let mut missing = execution.clone();
    missing.as_object_mut().unwrap().remove("author_mode");
    let mut variants = vec![missing, JsonValue::Null];
    for mode in [
        JsonValue::Null,
        json!("invalid"),
        json!(" human_only "),
        json!(true),
    ] {
        let mut value = execution.clone();
        value["author_mode"] = mode;
        variants.push(value);
    }
    let mut unavailable = execution.clone();
    unavailable["availability"] = json!("unavailable");
    unavailable["recoverable"] = json!(false);
    unavailable["author_mode"] = JsonValue::Null;
    variants.push(unavailable);
    for (field, wrong) in [
        ("repository_index", json!(1)),
        ("worker_job_index", json!(1)),
        ("package_hash", json!("00".repeat(32))),
        ("base_snapshot_id", json!("SNP-FFFFFFFFFFFF")),
        ("revision_snapshot_id", json!("SNP-FFFFFFFFFFFF")),
    ] {
        let mut value = execution.clone();
        value["expected_identity"][field] = wrong;
        variants.push(value);
    }
    for value in variants {
        fixture.state.lock().unwrap().package_execution_override = Some(value);
        for explicit in [None, Some(MODE)] {
            let result = output_json(&ready(&fixture, selected, Some(&pin), explicit));
            assert_ne!(result["next_action"]["code"], "done", "{result}");
            assert_ne!(result["next_action"]["code"], "waiting_for_ci", "{result}");
        }
    }
    fixture.state.lock().unwrap().package_execution_override = None;
    for wrong in [
        json!({"repository_index":1,"worker_job_index":0,"patchset_index":null,"tombstoned":false}),
        json!({"repository_index":0,"worker_job_index":1,"patchset_index":null,"tombstoned":false}),
    ] {
        fixture.state.lock().unwrap().job_override = Some(wrong);
        let result = output_json(&ready(&fixture, selected, Some(&pin), Some(MODE)));
        assert_ne!(result["next_action"]["code"], "done", "{result}");
    }
    fixture.state.lock().unwrap().job_override = None;
    for status in [401, 403, 404, 503] {
        fixture.state.lock().unwrap().job_read_status = Some(status);
        let result = output_json(&ready(&fixture, selected, Some(&pin), Some(MODE)));
        assert_ne!(result["next_action"]["code"], "done", "{result}");
    }
    assert_eq!(fixture.head(), head);
    assert_eq!(fixture.snapshots(), snapshots);
    assert_eq!(
        tasks
            .iter()
            .map(|task| fixture.task(task))
            .collect::<Vec<_>>(),
        before_tasks
    );
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.batches.len(), 1);
    assert!(state.finishes.is_empty());
    assert_eq!(state.batches[0].bytes, original.bytes);
    assert_eq!(state.batches[0].evidence, original.evidence);
    // Null execution allows only a Server-validated explicit retained request;
    // this fixture has no confirmed reset, so its one attempt was rejected.
    assert_eq!(state.reprepare_requests, vec![0]);
    assert!(state.requests[request_start..]
        .iter()
        .all(|(method, path)| {
            method == "GET"
                || (method == "POST"
                    && (path == "/v1/native/repository-authorities"
                        || path.ends_with("/remote-sync/zstd-bulk/commit")))
        }));
}

#[test]
fn retained_reset_reprepare_preserves_source_and_joins_ack_loss_and_passed_replays() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    defaults(&fixture, true, "ai_with_human_review");
    let initial = output_json(&run(
        &fixture.root,
        &[
            "workflow",
            "ready",
            selected,
            "--apply",
            "--remote",
            "origin",
            "--summary",
            "original sealed summary",
            "--json",
            "--full",
        ],
    ));
    assert_eq!(initial["next_action"]["code"], "done", "{initial}");
    let original_identity = initial["expected_identity"].clone();
    let original_pin = fixture.identity(&original_identity, "retired.identity.json");
    let original = fixture.state.lock().unwrap().batches[0].clone();
    let (later, edit) = fixture.start("later-reprepare");
    fs::write(edit.join("later.txt"), "later Local content").unwrap();
    let later_head = fixture.finish_local(&later, &edit);
    defaults(&fixture, false, "human_only");
    let snapshots = fixture.snapshots();
    {
        let mut state = fixture.state.lock().unwrap();
        state.reset_jobs.insert(0);
        state.lose_reprepare_response = true;
    }
    let before = fixture.state.lock().unwrap().requests.len();
    let readonly = output_json(&run(
        &fixture.root,
        &[
            "workflow",
            "ready",
            selected,
            "--remote",
            "origin",
            "--expected-identity",
            original_pin.to_str().unwrap(),
            "--json",
            "--full",
        ],
    ));
    assert_ne!(readonly["next_action"]["code"], "done");
    let held = output_json(&finish(&fixture, selected, Some(&original_pin)));
    assert_ne!(held["next_action"]["code"], "done");
    let explicit = output_json(&ready(&fixture, selected, Some(&original_pin), Some(MODE)));
    assert_eq!(
        explicit["next_action"]["code"],
        "author_mode_required_action"
    );
    assert!(fixture.state.lock().unwrap().requests[before..]
        .iter()
        .all(|(method, _)| method == "GET"));
    let mut wrong_hash = original_identity.clone();
    wrong_hash["package_hash"] = json!("00".repeat(32));
    let wrong_pin = fixture.identity(&wrong_hash, "foreign-hash.identity.json");
    let rejected = output_json(&ready(&fixture, selected, Some(&wrong_pin), None));
    assert_eq!(rejected["next_action"]["code"], "execution_unknown");
    assert_eq!(fixture.state.lock().unwrap().batches.len(), 1);
    let lost = output_json(&ready(&fixture, selected, Some(&original_pin), None));
    assert_eq!(lost["next_action"]["code"], "execution_unknown", "{lost}");
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.batches.len(), 2);
        assert_eq!(state.reprepare_requests, vec![0, 0]);
        assert!(state.batches[1].evidence.is_empty());
    }
    let resumed = output_json(&ready(&fixture, selected, Some(&original_pin), None));
    assert_eq!(resumed["next_action"]["code"], "done", "{resumed}");
    let replacement = resumed["expected_identity"].clone();
    assert_eq!(replacement["worker_job_index"], 1);
    for field in [
        "repository_index",
        "package_hash",
        "base_snapshot_id",
        "revision_snapshot_id",
    ] {
        assert_eq!(replacement[field], original_identity[field]);
    }
    let replay = output_json(&ready(&fixture, selected, Some(&original_pin), None));
    assert_eq!(replay["expected_identity"], replacement);
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.batches.len(), 2);
        assert_eq!(state.uploads.len(), 1);
        assert_eq!(state.reprepare_requests, vec![0, 0, 0, 0]);
        assert_eq!(state.batches[1].bytes, original.bytes);
        assert_eq!(state.batches[1].manifest, original.manifest);
        assert_eq!(state.batches[1].manifest["author_mode"], MODE);
        assert_eq!(
            state.batches[1].manifest["summary"],
            "original sealed summary"
        );
        assert_eq!(state.batches[1].evidence.len(), 1);
        assert!(state.finishes.is_empty());
    }
    assert_eq!(fixture.snapshots(), snapshots);
    assert_eq!(fixture.head(), later_head);
    let new_pin = fixture.identity(&replacement, "replacement.identity.json");
    fixture.finish(selected, &new_pin);
    accepted(&ready(&fixture, selected, None, None), &replacement);
    let old_finish = output_json(&finish(&fixture, selected, Some(&original_pin)));
    assert_ne!(old_finish["next_action"]["code"], "done");
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.finishes.len(), 1);
    assert_eq!(state.batches.len(), 2);
}

#[test]
fn retained_reprepare_requires_matching_known_mode_and_validates_every_returned_identity_field() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    let (identity, pin) = seal(&fixture, selected);
    {
        let mut state = fixture.state.lock().unwrap();
        state.reset_jobs.insert(0);
        state.reset_mode_available = true;
    }
    let conflict = ready(&fixture, selected, Some(&pin), Some("human_only"));
    assert!(!conflict.status.success());
    assert!(fixture.state.lock().unwrap().reprepare_requests.is_empty());
    let mut replacement = identity.clone();
    replacement["worker_job_index"] = json!(1);
    for (field, wrong) in [
        ("repository_index", json!(1)),
        ("package_hash", json!("00".repeat(32))),
        ("base_snapshot_id", json!("SNP-FFFFFFFFFFFF")),
        ("revision_snapshot_id", json!("SNP-FFFFFFFFFFFF")),
        ("worker_job_index", json!(0)),
    ] {
        let mut bad = replacement.clone();
        bad[field] = wrong;
        fixture.state.lock().unwrap().reprepare_response_override =
            Some(json!({"expected_identity":bad,"ci_state":"passed","reused":true}));
        let output = ready(&fixture, selected, Some(&pin), Some(MODE));
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("same sealed"));
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.batches.len(), 2);
        assert!(state.batches[1].evidence.is_empty());
        assert!(state.finishes.is_empty());
    }
    fixture.state.lock().unwrap().reprepare_response_override = None;
    let resumed = output_json(&ready(&fixture, selected, Some(&pin), Some(MODE)));
    assert_eq!(resumed["next_action"]["code"], "done", "{resumed}");
    assert_eq!(resumed["expected_identity"], replacement);
    assert_eq!(fixture.state.lock().unwrap().batches.len(), 2);
}

#[test]
fn fresh_team_default_uses_independent_review_and_real_policy_still_blocks_human_approval() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    defaults(&fixture, true, "ai_with_human_review");
    let (identity, pin) = seal(&fixture, selected);
    let config: JsonValue =
        serde_json::from_slice(&fs::read(fixture.root.join(".ait/config.json")).unwrap()).unwrap();
    assert_eq!(config["default_author_mode"], "ai_with_human_review");
    let missing = finish(&fixture, selected, Some(&pin));
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("independent Reviewer assignment"));
    {
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.batches[0].manifest["author_mode"], MODE);
        assert!(state.finishes.is_empty());
        assert!(state.batches[0]
            .evidence
            .values()
            .all(|row| row["operation"] == "attestation"));
    }
    fixture.state.lock().unwrap().policy_requires_human = true;
    let blocked = fixture.finish_output(selected, &pin);
    let value = output_json(&blocked);
    assert_eq!(value["next_action"]["code"], "finish_incomplete", "{value}");
    assert_eq!(value["expected_identity"], identity);
    assert!(value.to_string().contains("PACKAGE_HUMAN_REVIEW_REQUIRED"));
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.batches.len(), 1);
    assert_eq!(state.finishes.len(), 1);
    assert!(state.batches[0].accepted.is_none());
    assert!(state.mappings.is_empty());
    assert!(state.batches[0]
        .evidence
        .values()
        .all(|row| row["operation"] == "attestation" || row["payload"]["reviewer_kind"] == "bot"));
}

#[test]
fn cleanup_pending_and_unknown_never_erase_independently_proven_acceptance() {
    for code in [
        "PACKAGE_FINISH_CLEANUP_PENDING",
        "PACKAGE_FINISH_CLEANUP_UNKNOWN",
    ] {
        let fixture = Fixture::new();
        let (tasks, _) = fixture.initial_batch();
        let selected = tasks.last().unwrap();
        let (identity, pin) = seal(&fixture, selected);
        fixture.state.lock().unwrap().cleanup_error = Some(code);
        let value = accepted(&fixture.finish_output(selected, &pin), &identity);
        assert!(value.to_string().contains(code), "{value}");
        let request_start = fixture.state.lock().unwrap().requests.len();
        accepted(&finish(&fixture, selected, None), &identity);
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.finishes.len(), 1);
        assert_eq!(state.batches.len(), 1);
        assert!(state.requests[request_start..]
            .iter()
            .all(|(method, _)| method == "GET"));
    }
}

#[test]
fn accepted_retry_does_not_require_materializable_local_source() {
    let fixture = Fixture::new();
    let (tasks, original_final) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    let (identity, pin) = seal(&fixture, selected);
    fixture.state.lock().unwrap().incomplete_response = true;
    accepted(&fixture.finish_output(selected, &pin), &identity);
    let before = tasks.iter().map(|id| fixture.task(id)).collect::<Vec<_>>();
    let snapshots = fixture.snapshots();
    let pack = snapshots
        .iter()
        .find(|row| row.snapshot_id == original_final)
        .unwrap()
        .root_tree_pack_id
        .as_ref()
        .unwrap();
    let repo = fixture.repo();
    let stores = repo.binary_db_stores::<1>();
    let path = stores.content().tree_packs().repo_root().as_path().join(
        ait_core::pack_substrate::default_tree_pack_relative_path(pack),
    );
    assert!(path.starts_with(&fixture.scratch));
    fs::remove_file(&path).unwrap();
    assert!(repo
        .local_snapshot_operation_store::<1>(&fixture.root)
        .unwrap()
        .snapshot_tree_file_rows(Some(&original_final))
        .is_err());
    let request_start = fixture.state.lock().unwrap().requests.len();
    accepted(&ready(&fixture, selected, None, None), &identity);
    accepted(&finish(&fixture, selected, Some(&pin)), &identity);
    assert!(!path.exists());
    assert_eq!(fixture.snapshots(), snapshots);
    assert_eq!(
        tasks.iter().map(|id| fixture.task(id)).collect::<Vec<_>>(),
        before
    );
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.finishes.len(), 1);
    assert_eq!(state.batches.len(), 1);
    assert!(state.requests[request_start..]
        .iter()
        .all(|(method, _)| method == "GET"));
}

#[test]
fn unavailable_first_publication_evidence_does_not_become_a_fresh_job() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    fixture.state.lock().unwrap().fail_formal_reads = true;
    let value = output_json(&ready(&fixture, selected, None, None));
    assert_eq!(
        value["next_action"]["code"], "completion_unknown",
        "{value}"
    );
    let state = fixture.state.lock().unwrap();
    assert!(state.batches.is_empty());
    assert!(state.uploads.is_empty());
    assert!(state.finishes.is_empty());
    assert!(state.requests.iter().all(|(method, _)| method == "GET"));
}

#[test]
fn explicit_fresh_mode_takes_precedence_without_overwriting_the_solo_default() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    defaults(&fixture, true, "human_only");
    let output = ready(&fixture, selected, None, Some("human_with_ai_assist"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = output_json(&output);
    assert_eq!(value["next_action"]["code"], "done", "{value}");
    let config: JsonValue =
        serde_json::from_slice(&fs::read(fixture.root.join(".ait/config.json")).unwrap()).unwrap();
    assert_eq!(config["default_author_mode"], "human_only");
    let state = fixture.state.lock().unwrap();
    assert_eq!(
        state.batches[0].manifest["author_mode"],
        "human_with_ai_assist"
    );
}

#[test]
fn overlapping_task_numbers_on_another_authority_cannot_supply_the_original_candidate() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    let (_, pin) = seal(&fixture, selected);
    fixture.finish(selected, &pin);
    let before = fixture.task(selected);
    let snapshots = fixture.snapshots();

    let foreign = Fixture::new();
    let (a, edit) = foreign.start("other source");
    fs::write(
        edit.join("foreign.txt"),
        "another authority's immutable content",
    )
    .unwrap();
    foreign.finish_local(&a, &edit);
    let (b, edit) = foreign.start("other final owner");
    fs::write(edit.join("other.txt"), "another authority's final source").unwrap();
    foreign.finish_local(&b, &edit);
    let (_, foreign_pin) = seal(&foreign, &b);
    foreign.finish(&b, &foreign_pin);
    let request_start = {
        let other = foreign.state.lock().unwrap();
        let mut state = fixture.state.lock().unwrap();
        assert_eq!(state.mappings[selected], other.mappings[&b]);
        assert_ne!(state.batches[0].identity, other.batches[0].identity);
        state.head = other.head.clone();
        state.batches = other.batches.clone();
        state.mappings = other.mappings.clone();
        state.requests.len()
    };
    let value = output_json(&finish(&fixture, selected, None));
    assert_eq!(
        value["next_action"]["code"], "completion_unknown",
        "{value}"
    );
    assert!(value["completion"]["patchset_id"].is_null());
    assert_eq!(fixture.task(selected), before);
    assert_eq!(fixture.snapshots(), snapshots);
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.finishes.len(), 1);
    assert!(state.requests[request_start..]
        .iter()
        .all(|(method, _)| method == "GET"));
}

#[test]
fn conflicting_original_snapshot_metadata_holds_an_unpinned_retry() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    let (_, pin) = seal(&fixture, selected);
    fixture.finish(selected, &pin);
    let before = fixture.task(selected);
    let request_start = {
        let mut state = fixture.state.lock().unwrap();
        state.conflicting_snapshot_metadata = true;
        state.requests.len()
    };
    let value = output_json(&finish(&fixture, selected, None));
    assert_eq!(
        value["next_action"]["code"], "completion_unknown",
        "{value}"
    );
    assert!(value["completion"]["patchset_id"].is_null());
    assert_eq!(fixture.task(selected), before);
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.finishes.len(), 1);
    assert_eq!(state.batches.len(), 1);
    assert!(state.requests[request_start..]
        .iter()
        .all(|(method, _)| method == "GET"));
}

#[test]
fn wrapped_repository_authority_keeps_all_selected_authority_guards() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    let (_, pin) = seal(&fixture, selected);
    fixture.finish(selected, &pin);
    let before = fixture.task(selected);
    let snapshots = fixture.snapshots();
    let request_start = fixture.state.lock().unwrap().requests.len();
    for (field, wrong) in [
        ("repository_index", json!(1)),
        ("repository_name", json!("another-repository")),
        ("tombstoned", json!(true)),
    ] {
        let mut row = json!({"repository_index":0,"repository_name":"fixture","namespace":"","policy_flags":0,"tombstoned":false});
        row[field] = wrong;
        fixture.state.lock().unwrap().repository_override = Some(row);
        for output in [
            ready(&fixture, selected, None, None),
            ready(&fixture, selected, Some(&pin), None),
            finish(&fixture, selected, None),
            finish(&fixture, selected, Some(&pin)),
        ] {
            let value = output_json(&output);
            assert_eq!(
                value["next_action"]["code"], "completion_unknown",
                "{value}"
            );
            assert!(value["completion"]["patchset_id"].is_null());
        }
    }
    fixture.state.lock().unwrap().repository_override = Some(JsonValue::Null);
    let value = output_json(&finish(&fixture, selected, Some(&pin)));
    assert_eq!(
        value["next_action"]["code"], "completion_unknown",
        "{value}"
    );
    assert_eq!(fixture.task(selected), before);
    assert_eq!(fixture.snapshots(), snapshots);
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.finishes.len(), 1);
    assert_eq!(state.batches.len(), 1);
    assert!(state.requests[request_start..]
        .iter()
        .all(|(method, _)| method == "GET"));
}

#[test]
fn routed_snapshot_projection_keeps_selected_repository_index_and_name_guards() {
    let fixture = Fixture::new();
    let (tasks, _) = fixture.initial_batch();
    let selected = tasks.last().unwrap();
    let (_, pin) = seal(&fixture, selected);
    fixture.finish(selected, &pin);
    let before = fixture.task(selected);
    let snapshots = fixture.snapshots();
    let request_start = fixture.state.lock().unwrap().requests.len();
    for wrong in [
        json!({"repo_name":"another-repository"}),
        json!({"repo_name":null}),
        json!({"repository_index":1}),
        json!({"repository_index":null}),
        json!({"repository_index":"0"}),
    ] {
        fixture.state.lock().unwrap().snapshot_authority_override = Some(wrong);
        for output in [
            ready(&fixture, selected, None, None),
            ready(&fixture, selected, Some(&pin), None),
            finish(&fixture, selected, None),
            finish(&fixture, selected, Some(&pin)),
        ] {
            let value = output_json(&output);
            assert_eq!(
                value["next_action"]["code"], "completion_unknown",
                "{value}"
            );
            assert!(value["completion"]["patchset_id"].is_null());
        }
    }
    assert_eq!(fixture.task(selected), before);
    assert_eq!(fixture.snapshots(), snapshots);
    let state = fixture.state.lock().unwrap();
    assert_eq!(state.finishes.len(), 1);
    assert_eq!(state.batches.len(), 1);
    assert!(state.requests[request_start..]
        .iter()
        .all(|(method, _)| method == "GET"));
}
