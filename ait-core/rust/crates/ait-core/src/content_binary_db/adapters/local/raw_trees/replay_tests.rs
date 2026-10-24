use super::*;
use crate::line_store::LineStore;
use crate::snapshot_rebase::{plan_snapshot_history_rebase, SnapshotHistoryBoundary};
use crate::task_store::TaskStore;
use crate::workflow_binary_db::{BinaryDbWorkflowStore, SnapshotWorkflowBinding};
use tempfile::TempDir;

#[test]
fn completed_replay_materializes_blob_trees_without_workspace_changes_and_reuses_exact_capture_on_retry(
) {
    let temp = TempDir::new().unwrap();
    let content = LocalContentBinaryDb::<1>::new(
        temp.path().join(".ait/binary-db"),
        temp.path(),
        AuthorityId::new("local:fixture"),
        LocalStateScope::Repository,
    );
    let lines = crate::line_binary_db::BinaryDbLineStore::<_, 1>::new(content.db().clone());
    lines
        .create_line("main", None, "2026-10-20T00:00:00Z")
        .unwrap();
    std::fs::write(temp.path().join("base.txt"), "base\n").unwrap();
    let capture = |line: &str,
                   parents: &[String],
                   message: &str,
                   binding: Option<&SnapshotWorkflowBinding>| {
        content
            .create_snapshot_content_with_workflow_binding(
                "fixture",
                line,
                parents,
                Some(message),
                binding.is_some(),
                SnapshotAuthoringOptions::default(),
                binding.map(|owner| (owner, "C")),
            )
            .unwrap()["snapshot_id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let base = capture("main", &[], "base", None);
    lines
        .set_line_head("main", Some(&base), "2026-10-20T00:00:00Z")
        .unwrap();
    lines
        .create_line("feature", Some(&base), "2026-10-20T00:00:00Z")
        .unwrap();
    let workflow =
        BinaryDbWorkflowStore::<_, 1>::new_with_namespace(content.db().clone(), "fixture", "C");
    let task = TaskStore::create_task(
        &workflow,
        "fixture",
        "Feature",
        "Preserve original receipt",
        Some("C"),
        None,
        None,
        None,
    )
    .unwrap();
    let owner = SnapshotWorkflowBinding {
        task_id: task["task_id"].as_str().unwrap().into(),
        change_id: String::new(),
        line_name: "feature".into(),
    };
    std::fs::write(temp.path().join("feature.txt"), "feature\n").unwrap();
    let source = capture(
        "feature",
        std::slice::from_ref(&base),
        "feature",
        Some(&owner),
    );
    workflow
        .finish_local_task(&owner.task_id, &source, "main", Some(&base), None, None)
        .unwrap();
    let before = TaskStore::get_task(&workflow, &owner.task_id).unwrap();
    std::fs::remove_file(temp.path().join("feature.txt")).unwrap();
    std::fs::write(temp.path().join("remote.txt"), "remote\n").unwrap();
    let target = capture("main", std::slice::from_ref(&base), "remote", None);
    let file_map = |id: &str| {
        content
            .snapshot_tree_file_rows(Some(id))
            .unwrap()
            .into_iter()
            .map(|row| (row.path.clone(), row))
            .collect()
    };
    let boundaries = [SnapshotHistoryBoundary {
        base_snapshot_id: base.clone(),
        snapshot_id: source,
        owner: owner.clone(),
        base_files: file_map(&base),
        files: file_map(
            before["local_finish"]["landed_snapshot_id"]
                .as_str()
                .unwrap(),
        ),
        metadata: (),
    }];
    let mut steps = plan_snapshot_history_rebase(&target, &file_map(&target), &boundaries).unwrap();
    let before_count = content.snapshots().list_line_snapshots().unwrap().len();
    steps[0].reuse_snapshot_id = Some(&boundaries[0].snapshot_id);
    assert!(content
        .replay_completed_snapshot_history("fixture", "C", &steps)
        .unwrap_err()
        .contains("immutable source ancestry or content"));
    assert_eq!(
        content.snapshots().list_line_snapshots().unwrap().len(),
        before_count
    );
    steps[0].reuse_snapshot_id = None;
    let first = content
        .replay_completed_snapshot_history("fixture", "C", &steps)
        .unwrap();
    let rows = content
        .snapshot_tree_file_rows(Some(&first[0].snapshot_id))
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(),
        ["base.txt", "feature.txt", "remote.txt"]
    );
    assert!(!temp.path().join("feature.txt").exists());
    assert_eq!(
        std::fs::read_to_string(temp.path().join("remote.txt")).unwrap(),
        "remote\n"
    );
    assert_eq!(
        TaskStore::get_task(&workflow, &owner.task_id).unwrap(),
        before
    );
    let count = content.snapshots().list_line_snapshots().unwrap().len();
    let second = content
        .replay_completed_snapshot_history("fixture", "C", &steps)
        .unwrap();
    assert_eq!(second, first);
    assert_eq!(
        content.snapshots().list_line_snapshots().unwrap().len(),
        count
    );
    let owner_row = workflow
        .snapshot_ownership_rows(std::slice::from_ref(&first[0].snapshot_id))
        .unwrap();
    assert_eq!(owner_row[0]["task_id"], owner.task_id);
    assert_eq!(
        lines
            .line_by_name("main")
            .unwrap()
            .unwrap()
            .head_snapshot_id,
        before["local_finish"]["landed_snapshot_id"]
            .as_str()
            .map(str::to_string)
    );
}
