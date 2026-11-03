use super::*;

fn row(path: &str, content: &str, mode: &str) -> SnapshotFileRow {
    SnapshotFileRow {
        path: path.into(),
        blob_id: content.into(),
        size_bytes: content.len() as i64,
        mode: mode.into(),
        sha256: content.into(),
    }
}

fn tree(entries: &[(&str, &str)]) -> SnapshotFileMap {
    entries
        .iter()
        .map(|(path, content)| (path.to_string(), row(path, content, "100644")))
        .collect()
}

fn history() -> Vec<SnapshotHistoryBoundary<String>> {
    let mut files = tree(&[("base", "original")]);
    let mut base = "base".to_string();
    (1..=5)
        .map(|index| {
            let before = files.clone();
            let path = format!("task-{index}");
            files.insert(path.clone(), row(&path, &index.to_string(), "100644"));
            let snapshot_id = format!("source-{index}");
            let boundary = SnapshotHistoryBoundary {
                base_snapshot_id: base.clone(),
                snapshot_id: snapshot_id.clone(),
                owner: SnapshotWorkflowBinding {
                    task_id: format!("local-task-{index}"),
                    change_id: if index % 2 == 0 {
                        "C-01".into()
                    } else {
                        String::new()
                    },
                    line_name: format!("feature-{index}"),
                },
                base_files: before,
                files: files.clone(),
                metadata: format!("unchanged published mapping {index}"),
            };
            base = snapshot_id;
            boundary
        })
        .collect()
}

#[test]
fn tree_replay_preserves_target_edits_deletions_modes_and_symlinks() {
    let base = tree(&[("remove", "old"), ("mode", "same"), ("link", "old-target")]);
    let mut feature = base.clone();
    feature.remove("remove");
    feature.get_mut("mode").unwrap().mode = "100755".into();
    feature.insert("link".into(), row("link", "new-target", "120000"));
    let mut target = base.clone();
    target.insert("remote".into(), row("remote", "retained", "100644"));
    let plan = rebase_snapshot_files(&base, &feature, &target);
    assert!(plan.conflict_paths.is_empty());
    assert_eq!(plan.remove_paths, ["remove"]);
    assert_eq!(plan.write_paths, ["link", "mode"]);
    let merged = plan.merged_files.unwrap();
    assert!(!merged.contains_key("remove"));
    assert_eq!(merged["mode"].mode, "100755");
    assert_eq!(merged["link"].mode, "120000");
    assert_eq!(merged["remote"], target["remote"]);
    assert_eq!(
        plan.paths
            .iter()
            .map(|path| path.path.as_str())
            .collect::<Vec<_>>(),
        ["link", "mode", "remote", "remove"]
    );
}

#[test]
fn divergent_changes_never_expose_a_partial_tree() {
    let base = tree(&[("delete-modify", "old"), ("mode", "same")]);
    let mut feature = base.clone();
    feature.remove("delete-modify");
    feature.get_mut("mode").unwrap().mode = "100755".into();
    let mut target = tree(&[("delete-modify", "different"), ("mode", "same")]);
    target.get_mut("mode").unwrap().mode = "120000".into();
    let plan = rebase_snapshot_files(&base, &feature, &target);
    assert_eq!(plan.conflict_paths, ["delete-modify", "mode"]);
    assert_eq!(plan.merged_files, None);
    let duplicate = rebase_snapshot_files(&base, &feature, &feature);
    assert!(duplicate.write_paths.is_empty());
    assert!(duplicate.remove_paths.is_empty());
    assert!(duplicate
        .paths
        .iter()
        .all(|path| path.resolution == SnapshotRebaseResolution::SameResult));
}

#[test]
fn five_tasks_replay_on_one_chain_and_keep_source_metadata_and_owners() {
    let source = history();
    let mut target = source[0].base_files.clone();
    target.insert("remote".into(), row("remote", "new-head", "100644"));
    let steps = plan_snapshot_history_rebase("remote-head", &target, &source).unwrap();
    assert_eq!(steps.len(), 5);
    for (index, step) in steps.iter().enumerate() {
        assert!(std::ptr::eq(step.source, &source[index]));
        assert_eq!(step.source.owner, source[index].owner);
        assert_eq!(
            step.source.metadata,
            format!("unchanged published mapping {}", index + 1)
        );
        assert_eq!(step.files["remote"], target["remote"]);
        assert_eq!(step.files.len(), index + 3);
        assert_eq!(step.reuse_snapshot_id, None);
        assert_eq!(
            step.parent,
            if index == 0 {
                SnapshotReplayParent::Existing("remote-head".into())
            } else {
                SnapshotReplayParent::Replayed(index - 1)
            }
        );
    }
}

#[test]
fn unchanged_replay_reuses_every_snapshot_but_changed_parent_requires_capture() {
    let source = history();
    let steps = plan_snapshot_history_rebase("base", &source[0].base_files, &source).unwrap();
    for (step, original) in steps.iter().zip(&source) {
        assert_eq!(step.reuse_snapshot_id, Some(original.snapshot_id.as_str()));
        assert_eq!(
            step.parent,
            SnapshotReplayParent::Existing(original.base_snapshot_id.clone())
        );
    }
    // An upstream Snapshot with an equal tree still has a different identity.
    let steps =
        plan_snapshot_history_rebase("different-parent", &source[0].base_files, &source).unwrap();
    assert!(steps.iter().all(|step| step.reuse_snapshot_id.is_none()));
}

#[test]
fn later_task_conflict_stops_the_batch_and_resolved_retry_replays_all_tasks() {
    let source = history();
    let mut target = source[0].base_files.clone();
    target.insert("task-3".into(), row("task-3", "remote-conflict", "100644"));
    assert_eq!(
        plan_snapshot_history_rebase("remote-head", &target, &source).unwrap_err(),
        SnapshotHistoryRebaseError::Conflict {
            task_id: "local-task-3".into(),
            snapshot_id: "source-3".into(),
            paths: vec!["task-3".into()],
        }
    );
    target.insert("task-3".into(), source[2].files["task-3"].clone());
    let resolved = plan_snapshot_history_rebase("resolved-head", &target, &source).unwrap();
    assert_eq!(resolved.len(), 5);
    assert_eq!(resolved.last().unwrap().files, source.last().unwrap().files);
}

#[test]
fn invalid_history_rejects_gaps_cycles_tree_aliases_and_task_reentry() {
    let check = |source: &[SnapshotHistoryBoundary<String>]| {
        assert!(matches!(
            plan_snapshot_history_rebase("remote", &source[0].base_files, source),
            Err(SnapshotHistoryRebaseError::InvalidHistory(_))
        ));
    };
    let mut source = history();
    source[2].base_snapshot_id = "gap".into();
    check(&source);
    let mut source = history();
    source[4].snapshot_id = "base".into();
    check(&source);
    let mut source = history();
    source[2].base_files.clear();
    check(&source);
    let mut source = history();
    source[2].owner.task_id = source[0].owner.task_id.clone();
    check(&source);
    let source = history();
    assert!(matches!(
        plan_snapshot_history_rebase("base", &SnapshotFileMap::new(), &source),
        Err(SnapshotHistoryRebaseError::InvalidHistory(_))
    ));
}
