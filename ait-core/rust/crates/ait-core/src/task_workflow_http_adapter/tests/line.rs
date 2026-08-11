use super::*;

#[derive(Default)]
struct FakeLineLifecycleRemote {
    receipts: BTreeMap<String, JsonValue>,
}

impl TaskWorkflowLineRenamer for FakeLineLifecycleRemote {
    fn rename_remote_line(
        &mut self,
        repo_name: &str,
        old_line_name: &str,
        new_line_name: &str,
        expected_line_id: &str,
        expected_head_snapshot_id: Option<&str>,
        idempotency_key: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(self
            .receipts
            .entry(idempotency_key.to_string())
            .or_insert_with(|| {
                json!({
                    "contract": "line-lifecycle/v1",
                    "operation": "rename",
                    "repo_name": repo_name,
                    "old_line_name": old_line_name,
                    "new_line_name": new_line_name,
                    "line_id": expected_line_id,
                    "head_snapshot_id": expected_head_snapshot_id,
                    "idempotency_key": idempotency_key,
                    "receipt_id": "LLR-RENAME-1",
                })
            })
            .clone())
    }
}

impl TaskWorkflowLineDeleter for FakeLineLifecycleRemote {
    fn delete_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        expected_line_id: &str,
        expected_head_snapshot_id: Option<&str>,
        idempotency_key: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(self
            .receipts
            .entry(idempotency_key.to_string())
            .or_insert_with(|| {
                json!({
                    "contract": "line-lifecycle/v1",
                    "operation": "delete",
                    "repo_name": repo_name,
                    "line_name": line_name,
                    "line_id": expected_line_id,
                    "head_snapshot_id": expected_head_snapshot_id,
                    "idempotency_key": idempotency_key,
                    "tombstone": true,
                    "receipt_id": "LLR-DELETE-1",
                })
            })
            .clone())
    }
}

#[test]
fn remote_line_lifecycle_ports_preserve_cas_inputs_and_replay_receipts() {
    let mut remote = FakeLineLifecycleRemote::default();
    let rename_first = rename_remote_line_with_task_workflow_task_remote(
        &mut remote,
        "repo",
        "topic/old",
        "topic/new",
        "LNE-0000002A",
        Some("SNP-HEAD"),
        "idem-rename",
    )
    .unwrap();
    let rename_replay = rename_remote_line_with_task_workflow_task_remote(
        &mut remote,
        "repo",
        "topic/old",
        "topic/new",
        "LNE-0000002A",
        Some("SNP-HEAD"),
        "idem-rename",
    )
    .unwrap();
    assert_eq!(rename_first, rename_replay);
    assert_eq!(rename_first["line_id"], "LNE-0000002A");
    assert_eq!(rename_first["head_snapshot_id"], "SNP-HEAD");

    let delete_first = delete_remote_line_with_task_workflow_task_remote(
        &mut remote,
        "repo",
        "topic/new",
        "LNE-0000002A",
        Some("SNP-HEAD"),
        "idem-delete",
    )
    .unwrap();
    let delete_replay = delete_remote_line_with_task_workflow_task_remote(
        &mut remote,
        "repo",
        "topic/new",
        "LNE-0000002A",
        Some("SNP-HEAD"),
        "idem-delete",
    )
    .unwrap();
    assert_eq!(delete_first, delete_replay);
    assert_eq!(delete_first["tombstone"], true);
}

#[test]
fn task_workflow_line_remote_helpers_accept_narrow_trait_object() {
    let mut remote = FakeLineRemote;
    let remote_port: &mut dyn TaskWorkflowLineRemote = &mut remote;

    assert_eq!(
        change_lineage_payload_with_task_workflow_task_remote(
            remote_port,
            "main",
            Some(&json!({"head_snapshot_id": "SNP-BASE"})),
        )
        .unwrap()["fork_snapshot_id"],
        "SNP-BASE"
    );
    assert_eq!(
        get_line_with_task_workflow_task_remote(remote_port, "repo", "main").unwrap()
            ["head_snapshot_id"],
        "SNP-LINE"
    );
    assert_eq!(
        list_lines_with_task_workflow_task_remote(remote_port, "repo").unwrap()[0]["line_name"],
        "main"
    );
    assert_eq!(
        update_remote_line_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "main",
            Some("SNP-NEW"),
            Some("SNP-OLD"),
        )
        .unwrap()["expected_head_snapshot_id"],
        "SNP-OLD"
    );
    assert_eq!(
        close_line_with_task_workflow_task_remote(remote_port, "repo", "main", "archived").unwrap()
            ["status"],
        "archived"
    );
}

#[test]
fn task_workflow_line_helpers_accept_single_capability_ports() {
    let lineage_builder = FakeLineagePayloadBuilder;
    assert_eq!(
        change_lineage_payload_with_task_workflow_task_remote(
            &lineage_builder,
            "main",
            Some(&json!({"head_snapshot_id": "SNP-BASE"})),
        )
        .unwrap()["fork_snapshot_id"],
        "SNP-BASE"
    );

    let mut line_reader = FakeLineReader;
    assert_eq!(
        get_line_with_task_workflow_task_remote(&mut line_reader, "repo", "main").unwrap()
            ["head_snapshot_id"],
        "SNP-LINE"
    );

    let mut line_lister = FakeLineLister;
    assert_eq!(
        list_lines_with_task_workflow_task_remote(&mut line_lister, "repo").unwrap()[0]
            ["line_name"],
        "main"
    );

    let mut line_updater = FakeLineHeadUpdater;
    assert_eq!(
        update_remote_line_with_task_workflow_task_remote(
            &mut line_updater,
            "repo",
            "main",
            Some("SNP-NEW"),
            Some("SNP-OLD"),
        )
        .unwrap()["expected_head_snapshot_id"],
        "SNP-OLD"
    );

    let mut line_closer = FakeLineCloser;
    assert_eq!(
        close_line_with_task_workflow_task_remote(&mut line_closer, "repo", "main", "archived")
            .unwrap()["status"],
        "archived"
    );
}
