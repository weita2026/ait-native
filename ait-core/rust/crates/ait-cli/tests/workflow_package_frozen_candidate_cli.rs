//! Frozen package identity across actual ready/finish CLI processes.
#[path = "support/workflow_package_fixture.rs"]
mod package_fixture;
use package_fixture::*;
use std::fs;

#[test]
fn sealed_batch_survives_later_local_finish_and_explicit_rebase_then_next_batch_is_independent() {
    for explicit_rebase in [false, true] {
        let fixture = Fixture::new();
        let (initial, original_final) = fixture.initial_batch();
        let selected = initial.last().unwrap();
        let before = fixture.snapshots();
        let ready = fixture.ready(selected, None);
        assert_eq!(ready["next_action"]["code"], "done", "{ready}");
        assert_eq!(fixture.snapshots(), before);
        let identity = ready["expected_identity"].clone();
        assert_eq!(identity["revision_snapshot_id"], original_final);
        let path = fixture.identity(&identity, "original.identity.json");
        let (later, edit) = fixture.start("later");
        fs::write(edit.join("later.txt"), "later stays Local\n").unwrap();
        let mut next = Vec::new();
        if explicit_rebase {
            cli(
                &edit,
                &[
                    "snapshot",
                    "create",
                    &later,
                    "--message",
                    "Before explicit rebase",
                    "--json",
                ],
            );
            let (other, other_edit) = fixture.start("other");
            fs::write(other_edit.join("other.txt"), "explicit target\n").unwrap();
            fixture.finish_local(&other, &other_edit);
            next.push(other);
            cli(&edit, &["worktree", "rebase", "--onto", "main", "--json"]);
        }
        let later_final = fixture.finish_local(&later, &edit);
        next.push(later.clone());
        let later_before = next.iter().map(|id| fixture.task(id)).collect::<Vec<_>>();
        let before = fixture.snapshots();
        cli(
            &fixture.root,
            &[
                "config",
                "set",
                "--default-author-mode",
                "human_only",
                "--json",
            ],
        );
        let resumed = fixture.ready(selected, Some(&path));
        assert_eq!(resumed["expected_identity"], identity);
        let sealed = fixture.state.lock().unwrap().batches[0].clone();
        assert_eq!(
            sealed.manifest["history_entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["local_task_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            initial.iter().map(String::as_str).collect::<Vec<_>>()
        );
        let result = fixture.finish(selected, &path);
        assert_eq!(result["next_action"]["code"], "done", "{result}");
        assert_eq!(result["closeout_status"], "complete");
        assert_eq!(fixture.snapshots(), before);
        assert_eq!(fixture.head(), later_final);
        assert_eq!(
            fs::read_to_string(fixture.root.join("later.txt")).unwrap(),
            "later stays Local\n"
        );
        assert_eq!(
            next.iter().map(|id| fixture.task(id)).collect::<Vec<_>>(),
            later_before
        );
        for id in &initial {
            assert_eq!(fixture.task(id)["publication_state"], "published");
        }
        {
            let state = fixture.state.lock().unwrap();
            assert_eq!(state.batches.len(), 1);
            assert_eq!(state.batches[0].identity, identity);
            assert_eq!(state.batches[0].bytes, sealed.bytes);
            assert_eq!(state.batches[0].manifest, sealed.manifest);
            assert_eq!(state.head, original_final);
        }
        let earlier_mappings = fixture.state.lock().unwrap().mappings.clone();
        let ready_next = fixture.ready(&later, None);
        let next_identity = ready_next["expected_identity"].clone();
        assert_eq!(next_identity["base_snapshot_id"], original_final);
        assert_eq!(next_identity["revision_snapshot_id"], later_final);
        assert_ne!(
            next_identity["worker_job_index"],
            identity["worker_job_index"]
        );
        let next_path = fixture.identity(&next_identity, "next.identity.json");
        let result = fixture.finish(&later, &next_path);
        assert_eq!(result["closeout_status"], "complete", "{result}");
        assert_eq!(fixture.snapshots(), before);
        assert_eq!(fixture.head(), later_final);
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.batches.len(), 2);
        assert_eq!(
            state.batches[1].manifest["history_entries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["local_task_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            next.iter().map(String::as_str).collect::<Vec<_>>()
        );
        for (local, remote) in earlier_mappings {
            assert_eq!(state.mappings[&local], remote);
        }
    }
}

#[test]
fn fresh_incompatible_target_and_competing_finish_never_rebase_or_replace_ci() {
    for fresh in [true, false] {
        let fixture = Fixture::new();
        let (tasks, final_id) = fixture.initial_batch();
        let selected = tasks.last().unwrap();
        let identity = if fresh {
            None
        } else {
            Some(fixture.ready(selected, None)["expected_identity"].clone())
        };
        let foreign = fixture.foreign();
        fixture.state.lock().unwrap().head = foreign.clone();
        let before = fixture.snapshots();
        let before_tasks = tasks.iter().map(|id| fixture.task(id)).collect::<Vec<_>>();
        if let Some(identity) = identity {
            let path = fixture.identity(&identity, "stale.identity.json");
            let output = fixture.finish_output(selected, &path);
            assert!(!output.status.success());
            let result = output_json(&output);
            assert_ne!(result["next_action"]["code"], "done");
            assert!(result.to_string().contains("BASE_STALE"), "{result}");
            let state = fixture.state.lock().unwrap();
            assert_eq!(state.batches.len(), 1);
            assert_eq!(state.batches[0].identity, identity);
            assert!(state.batches[0].accepted.is_none());
            assert_eq!(state.finishes.len(), 1);
        } else {
            let output = run(
                &fixture.root,
                &[
                    "workflow",
                    "ready",
                    selected,
                    "--apply",
                    "--remote",
                    "origin",
                    "--author-mode",
                    MODE,
                    "--json",
                ],
            );
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("BASE_STALE"));
            let state = fixture.state.lock().unwrap();
            assert!(state.batches.is_empty());
            assert!(state.requests.iter().all(|(method, _)| method == "GET"));
        }
        assert_eq!(fixture.snapshots(), before);
        assert_eq!(fixture.head(), final_id);
        assert_eq!(
            tasks.iter().map(|id| fixture.task(id)).collect::<Vec<_>>(),
            before_tasks
        );
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.head, foreign);
        assert!(state.mappings.is_empty());
    }
}
