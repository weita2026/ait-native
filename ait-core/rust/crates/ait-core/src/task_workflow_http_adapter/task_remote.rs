use super::helpers::change_matches_reference;
use super::*;
use crate::change_json::ChangeJson;
use crate::json_support::JsonValue as Value;
use crate::repository_pack_json::{
    ZstdBulkCommitRequest, ZstdBulkCommitResponse, ZstdBulkPlanRequest, ZstdBulkPlanResponse,
    ZstdImportManifestPayload, ZstdPackUploadResponse, ZstdPullManifestPayload,
    ZstdPullManifestRequest,
};
use crate::server_operational::RepositoryIndex;
use crate::task_remote;

#[derive(Debug)]
pub struct HttpTaskRemote {
    manager: TaskWorkflowHttpClientManager,
    bound_task_id: Option<String>,
    bound_change_id: Option<String>,
    bound_change_ref: Option<String>,
}

impl HttpTaskRemote {
    pub fn new(config: TaskWorkflowHttpClientConfig) -> TaskWorkflowHttpClientResult<Self> {
        Ok(Self {
            manager: TaskWorkflowHttpClientManager::new(config)?,
            bound_task_id: None,
            bound_change_id: None,
            bound_change_ref: None,
        })
    }

    pub fn set_bound_change_context(
        &mut self,
        task_id: Option<&str>,
        change_id: Option<&str>,
    ) -> Result<(), String> {
        self.set_bound_change_identity_context(task_id, change_id, None)
    }

    pub fn set_bound_change_identity_context(
        &mut self,
        task_id: Option<&str>,
        change_id: Option<&str>,
        change_ref: Option<&str>,
    ) -> Result<(), String> {
        let bound_task_id = task_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let bound_change_id = change_id
            .map(|value| ChangeJson::stateless().canonical_change_id(value))
            .transpose()?;
        let derived_change_ref = bound_change_id
            .as_deref()
            .map(|value| {
                ChangeJson::stateless().rolling_server_change_id(bound_task_id.as_deref(), value)
            })
            .transpose()?;
        let provided_change_ref = change_ref
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let (Some(provided), Some(derived)) = (
            provided_change_ref.as_deref(),
            derived_change_ref.as_deref(),
        ) {
            if provided != derived {
                return Err(format!(
                    "Bound change_ref `{provided}` does not match derived `{derived}`."
                ));
            }
        }
        self.bound_task_id = bound_task_id;
        self.bound_change_id = bound_change_id;
        self.bound_change_ref = provided_change_ref.or(derived_change_ref);
        Ok(())
    }

    fn wire_change_id(&self, change_id: &str) -> TaskWorkflowHttpClientResult<String> {
        let requested = change_id.trim();
        let canonical = ChangeJson::stateless()
            .canonical_change_id(requested)
            .map_err(PlanHttpClientError::Invalid)?;
        if requested
            .rsplit_once('/')
            .is_some_and(|(_, child)| child == canonical)
        {
            return Ok(requested.to_string());
        }
        let task_id = if self.bound_change_id.as_deref() == Some(canonical.as_str()) {
            if let Some(change_ref) = self.bound_change_ref.as_deref() {
                return Ok(change_ref.to_string());
            }
            self.bound_task_id.as_deref()
        } else {
            None
        };
        ChangeJson::stateless()
            .rolling_server_change_id(task_id, &canonical)
            .map_err(PlanHttpClientError::Invalid)
    }

    fn expected_task_id_for_change(&self, change_id: &str) -> Option<String> {
        let requested = change_id.trim();
        if let Some((task_id, child)) = requested.rsplit_once('/') {
            if ChangeJson::stateless()
                .canonical_change_id(requested)
                .ok()
                .as_deref()
                == Some(child)
            {
                return Some(task_id.to_string());
            }
        }
        let canonical = ChangeJson::stateless()
            .canonical_change_id(requested)
            .ok()?;
        (self.bound_change_id.as_deref() == Some(canonical.as_str()))
            .then(|| self.bound_task_id.clone())
            .flatten()
    }

    fn normalize_change(
        &self,
        change: &Value,
        expected_task_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        ChangeJson::stateless()
            .normalize_remote_change_payload(change, expected_task_id)
            .map_err(PlanHttpClientError::Invalid)
    }

    fn normalize_change_detail(
        &self,
        detail: &Value,
        expected_task_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        ChangeJson::stateless()
            .normalize_remote_change_detail_payload(detail, expected_task_id)
            .map_err(PlanHttpClientError::Invalid)
    }

    pub fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        inspect_client_with_task_workflow_task_remote(self)
    }

    pub fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        close_client_with_task_workflow_task_remote(self)
    }

    pub fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&Value>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        ensure_repository_with_task_workflow_task_remote(
            self,
            repo_name,
            default_line,
            policy,
            id_namespace_prefix,
        )
    }

    pub fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Value> {
        get_repository_with_task_workflow_task_remote(self, repo_name)
    }

    pub fn get_repository_by_index(
        &mut self,
        repository_index: RepositoryIndex,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager
            .get_repository_authority_by_index(repository_index)
    }

    pub fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&Value>,
    ) -> Result<Value, String> {
        change_lineage_payload_with_task_workflow_task_remote(self, base_line, line_row)
    }

    pub fn get_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_line_with_task_workflow_task_remote(self, repo_name, line_name)
    }

    pub fn list_lines(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>> {
        list_lines_with_task_workflow_task_remote(self, repo_name)
    }

    pub fn get_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_task_with_task_workflow_task_remote(self, task_id, repo_name)
    }

    pub fn list_tasks(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>> {
        list_tasks_with_task_workflow_task_remote(self, repo_name)
    }

    pub fn read_task_audit(
        &mut self,
        repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        read_task_audit_with_task_workflow_task_remote(self, repo_name, task_id, target_line)
    }

    pub fn read_task_queue(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        read_task_queue_with_task_workflow_task_remote(self, repo_name, status)
    }

    pub fn read_reviewer_inbox(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Value> {
        read_reviewer_inbox_with_task_workflow_task_remote(self, repo_name)
    }

    pub fn read_queue_summary_bundle(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        read_queue_summary_bundle_with_task_workflow_task_remote(self, repo_name, status)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        create_task_with_task_workflow_task_remote(
            self,
            repo_name,
            title,
            intent,
            task_id,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        create_change_with_task_workflow_task_remote(
            self,
            repo_name,
            task_id,
            title,
            base_line,
            change_id,
            fork_snapshot_id,
            forked_from_line,
        )
    }

    pub fn list_changes(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>> {
        list_changes_with_task_workflow_task_remote(self, repo_name)
    }

    pub fn get_change_detail(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_change_detail_with_task_workflow_task_remote(self, change_ref, repo_name)
    }

    pub fn get_change(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_change_with_task_workflow_task_remote(self, change_ref, repo_name)
    }

    pub fn close_change(
        &mut self,
        change_ref: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        close_change_with_task_workflow_task_remote(self, change_ref, status, repo_name)
    }

    pub fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        update_remote_line_with_task_workflow_task_remote(
            self,
            repo_name,
            line_name,
            head_snapshot_id,
            expected_head_snapshot_id,
        )
    }

    pub fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        close_line_with_task_workflow_task_remote(self, repo_name, line_name, status)
    }

    pub fn plan_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkPlanRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkPlanResponse> {
        plan_remote_zstd_bulk_with_task_workflow_task_remote(self, repo_name, request)
    }

    pub fn put_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        put_remote_zstd_object_pack_with_task_workflow_task_remote(
            self, repo_name, pack_id, pack_bytes,
        )
    }

    pub fn put_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        put_remote_zstd_tree_pack_with_task_workflow_task_remote(
            self, repo_name, pack_id, pack_bytes,
        )
    }

    pub fn commit_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkCommitRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkCommitResponse> {
        commit_remote_zstd_bulk_with_task_workflow_task_remote(self, repo_name, request)
    }

    pub fn get_remote_zstd_import_manifest(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> TaskWorkflowHttpClientResult<ZstdImportManifestPayload> {
        get_remote_zstd_import_manifest_with_task_workflow_task_remote(self, repo_name, snapshot_id)
    }

    pub fn get_remote_zstd_pull_manifest(
        &mut self,
        repo_name: &str,
        request: &ZstdPullManifestRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdPullManifestPayload> {
        get_remote_zstd_pull_manifest_with_task_workflow_task_remote(self, repo_name, request)
    }

    pub fn get_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        get_remote_zstd_object_pack_with_task_workflow_task_remote(self, repo_name, pack_id)
    }

    pub fn get_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        get_remote_zstd_tree_pack_with_task_workflow_task_remote(self, repo_name, pack_id)
    }

    pub fn get_remote_snapshot(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_remote_snapshot_with_task_workflow_task_remote(
            self,
            repo_name,
            snapshot_id,
            include_content,
            path,
        )
    }

    pub fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_remote_snapshots_existence_with_task_workflow_task_remote(self, repo_name, snapshot_ids)
    }

    fn resolve_change_row(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let wire_change_id = self.wire_change_id(change_id)?;
        match self.manager.get_change(&wire_change_id, repo_name) {
            Ok(change) => {
                let expected_task_id = self.expected_task_id_for_change(change_id);
                self.normalize_change(&change, expected_task_id.as_deref())
            }
            Err(err) => self.recover_change_via_repo_listing(change_id, repo_name, err),
        }
    }

    fn recover_change_via_repo_listing(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        original_error: TaskWorkflowHttpClientError,
    ) -> TaskWorkflowHttpClientResult<Value> {
        if !super::helpers::change_read_error_allows_listing_recovery(&original_error) {
            return Err(original_error);
        }
        let Some(repo_name) = repo_name else {
            return Err(original_error);
        };
        let canonical = ChangeJson::stateless()
            .canonical_change_id(change_id)
            .map_err(PlanHttpClientError::Invalid)?;
        let expected_task_id = self.expected_task_id_for_change(change_id);
        let rows = self.manager.list_changes(repo_name)?;
        rows.into_iter()
            .filter_map(|row| self.normalize_change(&row, None).ok())
            .find(|row| {
                change_matches_reference(row, &canonical)
                    && expected_task_id.as_deref().is_none_or(|task_id| {
                        row.get("task_id").and_then(Value::as_str) == Some(task_id)
                    })
            })
            .ok_or(original_error)
    }
}

impl TaskWorkflowHttpClientInspector for HttpTaskRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        self.manager.inspect()
    }
}

impl TaskWorkflowHttpClientCloser for HttpTaskRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        self.manager.close()
    }
}

impl TaskWorkflowRepositoryEnsurer for HttpTaskRemote {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&Value>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager
            .ensure_repository(repo_name, default_line, policy, id_namespace_prefix)
    }
}

impl TaskWorkflowRepositoryReader for HttpTaskRemote {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.get_repository(repo_name)
    }
}

impl TaskWorkflowLineagePayloadBuilder for HttpTaskRemote {
    fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&Value>,
    ) -> Result<Value, String> {
        task_remote::task_remote_change_lineage_payload(base_line, line_row)
    }
}

impl TaskWorkflowLineReader for HttpTaskRemote {
    fn get_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.get_line(repo_name, line_name)
    }
}

impl TaskWorkflowLineLister for HttpTaskRemote {
    fn list_lines(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>> {
        self.manager.list_lines(repo_name)
    }
}

impl TaskWorkflowRemoteTaskReader for HttpTaskRemote {
    fn get_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.get_task(task_id, repo_name)
    }
}

impl TaskWorkflowRemoteTaskLister for HttpTaskRemote {
    fn list_tasks(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>> {
        self.manager.list_tasks(repo_name)
    }
}

impl TaskWorkflowRemoteTaskAuditReader for HttpTaskRemote {
    fn read_task_audit(
        &mut self,
        repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let payload = self
            .manager
            .read_task_audit(repo_name, task_id, target_line)?;
        ChangeJson::stateless()
            .normalize_remote_task_audit_payload(&payload, task_id)
            .map_err(PlanHttpClientError::Invalid)
    }
}

impl TaskWorkflowTaskQueueReader for HttpTaskRemote {
    fn read_task_queue(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.read_task_queue(repo_name, status)
    }
}

impl TaskWorkflowReviewerInboxReader for HttpTaskRemote {
    fn read_reviewer_inbox(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.read_reviewer_inbox(repo_name)
    }
}

impl TaskWorkflowQueueSummaryBundleReader for HttpTaskRemote {
    fn read_queue_summary_bundle(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.read_queue_summary_bundle(repo_name, status)
    }
}

impl TaskWorkflowRemoteTaskCreator for HttpTaskRemote {
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.create_task(
            repo_name,
            title,
            intent,
            task_id,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        )
    }
}

impl TaskWorkflowRemoteChangeCreator for HttpTaskRemote {
    fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let change = self.manager.create_change(
            repo_name,
            task_id,
            title,
            base_line,
            change_id,
            fork_snapshot_id,
            forked_from_line,
        )?;
        self.normalize_change(&change, Some(task_id))
    }
}

impl TaskWorkflowRemoteChangeLister for HttpTaskRemote {
    fn list_changes(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>> {
        self.manager
            .list_changes(repo_name)?
            .into_iter()
            .map(|row| self.normalize_change(&row, None))
            .collect()
    }
}

impl TaskWorkflowRemoteChangeDetailReader for HttpTaskRemote {
    fn get_change_detail(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let wire_change_id = self.wire_change_id(change_id)?;
        let change = self.manager.get_change_detail(&wire_change_id, repo_name)?;
        let expected_task_id = self.expected_task_id_for_change(change_id);
        self.normalize_change_detail(&change, expected_task_id.as_deref())
    }
}

impl TaskWorkflowRemoteChangeReader for HttpTaskRemote {
    fn get_change(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.resolve_change_row(change_id, repo_name)
    }
}

impl TaskWorkflowRemoteChangeCloser for HttpTaskRemote {
    fn close_change(
        &mut self,
        change_id: &str,
        status: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let wire_change_id = self.wire_change_id(change_id)?;
        let change = self.manager.close_change(&wire_change_id, status)?;
        let expected_task_id = self.expected_task_id_for_change(change_id);
        self.normalize_change(&change, expected_task_id.as_deref())
    }
}

impl TaskWorkflowLineHeadUpdater for HttpTaskRemote {
    fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.update_remote_line(
            repo_name,
            line_name,
            head_snapshot_id,
            expected_head_snapshot_id,
        )
    }
}

impl TaskWorkflowLineCloser for HttpTaskRemote {
    fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.close_line(repo_name, line_name, status)
    }
}

impl TaskWorkflowLineRenamer for HttpTaskRemote {
    fn rename_remote_line(
        &mut self,
        repo_name: &str,
        old_line_name: &str,
        new_line_name: &str,
        expected_line_id: &str,
        expected_head_snapshot_id: Option<&str>,
        idempotency_key: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.rename_remote_line(
            repo_name,
            old_line_name,
            new_line_name,
            expected_line_id,
            expected_head_snapshot_id,
            idempotency_key,
        )
    }
}

impl TaskWorkflowLineDeleter for HttpTaskRemote {
    fn delete_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        expected_line_id: &str,
        expected_head_snapshot_id: Option<&str>,
        idempotency_key: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.delete_remote_line(
            repo_name,
            line_name,
            expected_line_id,
            expected_head_snapshot_id,
            idempotency_key,
        )
    }
}

impl TaskWorkflowZstdPackUploader for HttpTaskRemote {
    fn plan_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkPlanRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkPlanResponse> {
        self.manager.plan_remote_zstd_bulk(repo_name, request)
    }

    fn put_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        self.manager
            .put_remote_zstd_object_pack(repo_name, pack_id, pack_bytes)
    }

    fn put_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        self.manager
            .put_remote_zstd_tree_pack(repo_name, pack_id, pack_bytes)
    }

    fn put_remote_zstd_packs_bounded(
        &mut self,
        repo_name: &str,
        object_packs: &[(String, Vec<u8>)],
        tree_packs: &[(String, Vec<u8>)],
        max_parallelism: usize,
    ) -> TaskWorkflowHttpClientResult<(Vec<ZstdPackUploadResponse>, Vec<ZstdPackUploadResponse>)>
    {
        self.manager.put_remote_zstd_packs_bounded(
            repo_name,
            object_packs,
            tree_packs,
            max_parallelism,
        )
    }

    fn commit_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkCommitRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkCommitResponse> {
        self.manager.commit_remote_zstd_bulk(repo_name, request)
    }
}

impl TaskWorkflowSnapshotMetadataReader for HttpTaskRemote {
    fn get_remote_snapshot(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager
            .get_remote_snapshot(repo_name, snapshot_id, include_content, path)
    }
}

impl TaskWorkflowZstdPackReader for HttpTaskRemote {
    fn get_remote_zstd_import_manifest(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> TaskWorkflowHttpClientResult<ZstdImportManifestPayload> {
        self.manager
            .get_remote_zstd_import_manifest(repo_name, snapshot_id)
    }

    fn get_remote_zstd_pull_manifest(
        &mut self,
        repo_name: &str,
        request: &ZstdPullManifestRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdPullManifestPayload> {
        self.manager
            .get_remote_zstd_pull_manifest(repo_name, request)
    }

    fn get_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        self.manager.get_remote_zstd_object_pack(repo_name, pack_id)
    }

    fn get_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        self.manager.get_remote_zstd_tree_pack(repo_name, pack_id)
    }

    fn get_remote_zstd_packs_bounded(
        &mut self,
        repo_name: &str,
        object_pack_ids: &[String],
        tree_pack_ids: &[String],
        max_parallelism: usize,
    ) -> TaskWorkflowHttpClientResult<(
        std::collections::BTreeMap<String, Vec<u8>>,
        std::collections::BTreeMap<String, Vec<u8>>,
    )> {
        self.manager.get_remote_zstd_packs_bounded(
            repo_name,
            object_pack_ids,
            tree_pack_ids,
            max_parallelism,
        )
    }
}

impl TaskWorkflowSnapshotExistenceReader for HttpTaskRemote {
    fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager
            .get_remote_snapshots_existence(repo_name, snapshot_ids)
    }
}
