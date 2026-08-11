use super::*;

#[test]
fn task_workflow_snapshot_remote_helpers_accept_narrow_trait_object() {
    let mut remote = FakeSnapshotRemote;
    let remote_port: &mut dyn TaskWorkflowSnapshotRemote = &mut remote;

    assert_eq!(
        get_remote_snapshot_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "SNP-5",
            true,
            Some("a.txt"),
        )
        .unwrap()["path"],
        "a.txt"
    );
    assert_eq!(
        get_remote_zstd_object_pack_with_task_workflow_task_remote(remote_port, "repo", "OPK-1")
            .unwrap(),
        b"repo:object:OPK-1"
    );
    assert_eq!(
        get_remote_zstd_tree_pack_with_task_workflow_task_remote(remote_port, "repo", "TPK-1")
            .unwrap(),
        b"repo:tree:TPK-1"
    );
    assert_eq!(
        get_remote_zstd_import_manifest_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "SNP-7"
        )
        .unwrap()
        .snapshot_id,
        "SNP-7"
    );
    assert_eq!(
        get_remote_snapshots_existence_with_task_workflow_task_remote(
            remote_port,
            "repo",
            &["SNP-7".to_string()],
        )
        .unwrap()["snapshot_ids"][0],
        "SNP-7"
    );
}

#[test]
fn task_workflow_snapshot_helpers_accept_single_capability_ports() {
    struct FakeSnapshotMetadataReader;
    impl TaskWorkflowSnapshotMetadataReader for FakeSnapshotMetadataReader {
        fn get_remote_snapshot(
            &mut self,
            repo_name: &str,
            snapshot_id: &str,
            include_content: bool,
            path: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            Ok(json!({
                "repo_name": repo_name,
                "snapshot_id": snapshot_id,
                "include_content": include_content,
                "path": path,
            }))
        }
    }

    struct FakeSnapshotExistenceReader;
    impl TaskWorkflowSnapshotExistenceReader for FakeSnapshotExistenceReader {
        fn get_remote_snapshots_existence(
            &mut self,
            repo_name: &str,
            snapshot_ids: &[String],
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            Ok(json!({
                "repo_name": repo_name,
                "present": snapshot_ids,
            }))
        }
    }

    assert_eq!(
        get_remote_snapshot_with_task_workflow_task_remote(
            &mut FakeSnapshotMetadataReader,
            "repo",
            "SNP-3",
            false,
            None,
        )
        .unwrap()["snapshot_id"],
        "SNP-3"
    );
    assert_eq!(
        get_remote_snapshots_existence_with_task_workflow_task_remote(
            &mut FakeSnapshotExistenceReader,
            "repo",
            &["SNP-5".to_string()],
        )
        .unwrap()["present"][0],
        "SNP-5"
    );
}
