use super::change_flow::{
    attestation_put_flow_with_task_and_closeout_remotes, change_local_close_with_change_store,
    change_local_create_with_change_store, change_local_list_with_change_store,
    change_local_mark_published_with_change_store, change_local_read_with_change_store,
    change_publish_with_local_stores_and_task_remote,
    land_submit_flow_with_task_and_closeout_remotes,
    patchset_publish_flow_with_task_and_closeout_remotes,
    patchset_publish_payload_with_closeout_remote,
    patchset_publish_remote_context_with_task_remote,
    resolve_patchset_argument_with_task_and_closeout_remotes,
    review_record_flow_with_task_and_closeout_remotes,
    review_request_flow_with_task_and_closeout_remotes, validate_short_remote_change_id,
};
use super::line::{
    line_show_with_line_store, remote_line_archive_with_task_remote,
    remote_line_list_with_task_remote, repo_status_storage_counts_with_store,
};
use super::queue::{
    queue_remote_change_rows_with_task_remote, queue_remote_reads_with_task_remote,
    queue_remote_reviewer_inbox_with_task_remote, queue_remote_summary_bundle_with_task_remote,
    queue_remote_task_queue_with_task_remote,
};
use super::stash::{
    drop_stash_record_with_stash_store, stash_list_with_stash_store, stash_show_with_stash_store,
};
use super::task::{
    local_task_audit_target_info_with_stores, task_audit_local_change_rows_with_change_store,
    task_audit_with_local_stores_and_task_remote, task_remote_audit_read_with_task_remote,
    task_remote_create_with_task_remote, task_remote_list_with_task_remote,
    task_remote_read_with_task_remote, validate_remote_task_create_response,
};
use super::workflow::{
    task_land_local_change_id_with_change_store, task_land_remote_task_read_with_task_remote,
    workflow_apply_phase_payload_json, workflow_coerce_wait_hint_seconds, workflow_current_ids,
    workflow_find_bound_task_worktree, workflow_json_text, workflow_land_apply_action,
    workflow_land_command_hints, workflow_land_evaluate_policy_with_closeout_remote,
    workflow_land_record_attestation_with_closeout_remote,
    workflow_land_record_code_review_summary_with_closeout_remote,
    workflow_land_record_task_review_with_closeout_remote,
    workflow_local_change_land_with_change_store, workflow_local_change_read_with_change_store,
    workflow_local_change_rows_with_change_store, workflow_local_task_close_with_task_store,
    workflow_local_task_read_with_task_store, workflow_maybe_record_ready_wait_hint_sample,
    workflow_nested_text, workflow_patchset_ci_contract_exists, workflow_progress_emit,
    workflow_publish_patchset_action_with_task_and_closeout_remotes, workflow_ready_apply,
    workflow_ready_apply_action, workflow_ready_command_hints,
    workflow_ready_record_attestation_with_closeout_remote,
    workflow_ready_run_patchset_ci_with_closeout_remote,
    workflow_record_attestation_action_with_closeout_remote,
    workflow_record_review_action_with_closeout_remote, workflow_resolve_wait_hint_seconds,
    workflow_root_text, workflow_run_patchset_ci_action_with_closeout_remote,
};
use super::workspace::{
    guard_no_planning_only_artifact_drift, repository_has_task_workflow_context_with_task_store,
    workspace_change_identity_aliases_with_change_store,
    workspace_local_change_read_with_change_store, workspace_local_task_read_with_task_store,
    workspace_task_identity_aliases_with_task_store,
};
use super::worktree::{
    archive_local_line_with_line_store, create_local_line_with_line_store,
    ensure_task_feature_line, line_change_usage_index_with_change_store,
    set_local_line_head_with_line_store, snapshot_distance_if_ancestor_with_snapshot_store,
    snapshot_distance_if_ancestor_with_snapshot_store_and_cache,
    task_start_remote_base_line_preflight_with_task_remote, worktree_doctor_from_rows,
    worktree_local_change_for_worktree_with_change_store,
    worktree_local_task_for_worktree_with_task_store, worktree_remote_change_read_with_task_remote,
    worktree_remote_patchset_read_with_closeout_remote,
    worktree_remote_patchset_revision_candidate_with_remotes,
    worktree_summary_from_metadata_for_repo_status,
};
use super::*;
use crate::init_surface::{init_repo, InitRequest};
use ait_core::line_store::{line_count_with_line_store, LineRecord, LineStore, LineStoreResult};
use ait_core::plan_store::{PlanStoreError, PlanStoreResult};
use ait_core::repo_status_store::{
    RepoStatusStorageCounts, RepoStatusStore, RepoStatusStoreResult,
};
use ait_core::snapshot_store::{SnapshotStore, SnapshotStoreResult};
use ait_core::stash_store::{
    DroppedStashRecord, NewStashRecord, StashRecord, StashStore, StashStoreResult,
};
use ait_core::task_workflow_http_adapter::{
    TaskWorkflowActionMutationReceiptsBuilder, TaskWorkflowAttestationReader,
    TaskWorkflowAttestationWriter, TaskWorkflowHttpClientCloser, TaskWorkflowHttpClientError,
    TaskWorkflowHttpClientInspector, TaskWorkflowHttpClientResult, TaskWorkflowHttpClientStats,
    TaskWorkflowLandReader, TaskWorkflowLandRetryer, TaskWorkflowLandSubmitter,
    TaskWorkflowLineCloser, TaskWorkflowLineHeadUpdater, TaskWorkflowLineLister,
    TaskWorkflowLineReader, TaskWorkflowLineagePayloadBuilder, TaskWorkflowMutationReceiptBuilder,
    TaskWorkflowPatchsetCiRunner, TaskWorkflowPatchsetCiStatusReader, TaskWorkflowPatchsetLister,
    TaskWorkflowPatchsetPublisher, TaskWorkflowPatchsetReader, TaskWorkflowPatchsetSelector,
    TaskWorkflowPolicyEvaluator, TaskWorkflowPolicyReader, TaskWorkflowPolicyWaiverCreator,
    TaskWorkflowQueueChangeLister, TaskWorkflowQueueSummaryBundleReader,
    TaskWorkflowRemoteChangeCloser, TaskWorkflowRemoteChangeCreator,
    TaskWorkflowRemoteChangeDetailReader, TaskWorkflowRemoteChangeLister,
    TaskWorkflowRemoteChangeReader, TaskWorkflowRemoteTaskAuditReader,
    TaskWorkflowRemoteTaskCloser, TaskWorkflowRemoteTaskCreator, TaskWorkflowRemoteTaskLister,
    TaskWorkflowRemoteTaskReader, TaskWorkflowRepoJobLister, TaskWorkflowRepositoryEnsurer,
    TaskWorkflowRepositoryReader, TaskWorkflowReviewLister, TaskWorkflowReviewRecorder,
    TaskWorkflowReviewRequester, TaskWorkflowReviewerInboxReader,
    TaskWorkflowSnapshotExistenceReader, TaskWorkflowSnapshotMetadataReader,
    TaskWorkflowTaskQueueReader, TaskWorkflowTaskRecordRemote, TaskWorkflowZstdPackReader,
    TaskWorkflowZstdPackUploader,
};
use ait_core::task_workflow_store::{
    TaskWorkflowChangeCloser, TaskWorkflowChangeCreator, TaskWorkflowChangeLander,
    TaskWorkflowChangeLister, TaskWorkflowChangePublisher, TaskWorkflowChangeReader,
    TaskWorkflowTaskCloser, TaskWorkflowTaskCreator, TaskWorkflowTaskLister,
    TaskWorkflowTaskPublisher, TaskWorkflowTaskReader,
};
use std::cell::{Cell, RefCell};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;
use tiny_http::{Response, Server};

fn fake_clonefile_success(
    source_path: &Path,
    target_path: &Path,
    _mode: u32,
) -> Result<SeedCopyFileStrategy, String> {
    fs::copy(source_path, target_path).map_err(|err| err.to_string())?;
    Ok(SeedCopyFileStrategy::Clonefile)
}

fn fake_reflink_success(
    source_path: &Path,
    target_path: &Path,
    _mode: u32,
) -> Result<SeedCopyFileStrategy, String> {
    fs::copy(source_path, target_path).map_err(|err| err.to_string())?;
    Ok(SeedCopyFileStrategy::Reflink)
}

fn fake_copy2_fallback(
    source_path: &Path,
    target_path: &Path,
    _mode: u32,
) -> Result<SeedCopyFileStrategy, String> {
    fs::copy(source_path, target_path).map_err(|err| err.to_string())?;
    Ok(SeedCopyFileStrategy::Copy2)
}

#[derive(Debug, Default)]
struct FakeWorkspaceTaskRemote {
    detail: Option<JsonValue>,
    changes: Vec<JsonValue>,
    lines: Vec<JsonValue>,
    queue_summary_bundle: Option<JsonValue>,
    queue_summary_error: Option<String>,
    task_queue: Option<JsonValue>,
    task_audit: Option<JsonValue>,
    reviewer_inbox: Option<JsonValue>,
    tasks: BTreeMap<String, JsonValue>,
    created_task_id_override: Option<String>,
    repository: Option<JsonValue>,
    ensured_repositories: Vec<JsonValue>,
    task_create_repository_present: Vec<bool>,
    remote_snapshots: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Default)]
struct FakeQueueRemote {
    queue_summary_bundle: Option<JsonValue>,
    queue_summary_error: Option<String>,
    task_queue: Option<JsonValue>,
    reviewer_inbox: Option<JsonValue>,
    changes: Vec<JsonValue>,
}

#[derive(Debug, Default)]
struct FakeChangeRemote {
    detail: Option<JsonValue>,
    changes: Vec<JsonValue>,
    lines: Vec<JsonValue>,
    repository: Option<JsonValue>,
    ensured_repositories: Vec<JsonValue>,
    remote_snapshots: BTreeMap<String, JsonValue>,
    zstd_plan_requests: Vec<JsonValue>,
    uploaded_zstd_object_packs: Vec<(String, Vec<u8>)>,
    uploaded_zstd_tree_packs: Vec<(String, Vec<u8>)>,
    zstd_commit_requests: Vec<JsonValue>,
    present_zstd_object_pack_ids: BTreeSet<String>,
    present_zstd_tree_pack_ids: BTreeSet<String>,
}

#[derive(Debug, Default)]
struct FakeLineSnapshotRemote {
    lines: Vec<JsonValue>,
    repository: Option<JsonValue>,
    ensured_repositories: Vec<JsonValue>,
    remote_snapshots: BTreeMap<String, JsonValue>,
    zstd_import_manifests: BTreeMap<String, ZstdImportManifestPayload>,
    zstd_pull_manifest: Option<ZstdPullManifestPayload>,
    zstd_pull_manifests: VecDeque<ZstdPullManifestPayload>,
    zstd_pull_manifest_requests: Vec<ZstdPullManifestRequest>,
    zstd_object_packs: BTreeMap<String, Vec<u8>>,
    zstd_tree_packs: BTreeMap<String, Vec<u8>>,
    zstd_import_manifest_reads: Vec<String>,
    zstd_object_pack_downloads: Vec<String>,
    zstd_tree_pack_downloads: Vec<String>,
    zstd_plan_requests: Vec<JsonValue>,
    uploaded_zstd_object_packs: Vec<(String, Vec<u8>)>,
    uploaded_zstd_tree_packs: Vec<(String, Vec<u8>)>,
    zstd_commit_requests: Vec<JsonValue>,
    present_zstd_object_pack_ids: BTreeSet<String>,
    present_zstd_tree_pack_ids: BTreeSet<String>,
    fail_zstd_object_pack_upload_for: Option<String>,
    fail_zstd_tree_pack_upload_for: Option<String>,
    line_update_calls: usize,
}

#[derive(Debug, Default)]
struct FakeTaskRecordRemote {
    tasks: BTreeMap<String, JsonValue>,
    task_audit: Option<JsonValue>,
    created_task_id_override: Option<String>,
    task_create_requested_ids: Vec<Option<String>>,
    repository: Option<JsonValue>,
    ensured_repositories: Vec<JsonValue>,
    task_create_repository_present: Vec<bool>,
}

#[derive(Debug, Default)]
struct FakeTaskAuditRemote {
    tasks: BTreeMap<String, JsonValue>,
    task_audit: Option<JsonValue>,
    task_audit_requests: Vec<String>,
    lines: Vec<JsonValue>,
    remote_snapshots: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Default)]
struct FakeRemoteTaskReaderOnly;

#[derive(Debug, Default)]
struct FakeRemoteTaskListerOnly;

#[derive(Debug, Default)]
struct FakeRemoteTaskAuditReaderOnly;

#[derive(Debug, Default)]
struct FakeRemoteTaskCreatorOnly;

#[derive(Debug, Default)]
struct FakeWorkspaceCloseoutRemote {
    patchsets: BTreeMap<String, JsonValue>,
    patchset_reads: Vec<String>,
    attestations: BTreeMap<String, JsonValue>,
    ci_runs: Vec<JsonValue>,
    reviews: BTreeMap<String, Vec<JsonValue>>,
    review_requests: BTreeMap<String, Vec<JsonValue>>,
    policy_evaluations: Vec<JsonValue>,
    land_submissions: Vec<JsonValue>,
    selected_patchsets: Vec<JsonValue>,
}

#[derive(Debug, Default)]
struct FakeTaskStore {
    tasks: RefCell<BTreeMap<String, JsonValue>>,
}

#[derive(Debug, Default)]
struct FakeChangeStore {
    changes: RefCell<BTreeMap<String, JsonValue>>,
}

#[derive(Debug, Default)]
struct FakeLocalLineStore {
    lines: RefCell<BTreeMap<String, LineRecord>>,
}

#[derive(Debug, Default)]
struct FakeSnapshotChainStore {
    chains: BTreeMap<String, Vec<String>>,
    parents: BTreeMap<String, Vec<String>>,
    snapshots: BTreeMap<String, SnapshotRecord>,
    parent_link_reads: Cell<usize>,
}

#[derive(Debug)]
struct FakeRepoStatusStore {
    storage_counts: RepoStatusStorageCounts,
}

impl LineStore for FakeLocalLineStore {
    fn list_lines(&self) -> LineStoreResult<Vec<LineRecord>> {
        Ok(self.lines.borrow().values().cloned().collect())
    }

    fn line_by_name(&self, line_name: &str) -> LineStoreResult<Option<LineRecord>> {
        Ok(self.lines.borrow().get(line_name).cloned())
    }

    fn create_line(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        timestamp: &str,
    ) -> LineStoreResult<LineRecord> {
        let mut lines = self.lines.borrow_mut();
        if lines.contains_key(line_name) {
            return Err(format!("Line already exists: {line_name}"));
        }
        let line = LineRecord {
            line_id: format!("LNE-FAKE-{}", lines.len() + 1),
            line_name: line_name.to_string(),
            status: "active".to_string(),
            archived_at: None,
            created_at: Some(timestamp.to_string()),
            updated_at: Some(timestamp.to_string()),
            head_snapshot_id: head_snapshot_id.map(ToString::to_string),
        };
        lines.insert(line_name.to_string(), line.clone());
        Ok(line)
    }

    fn archive_line(&self, line_name: &str, timestamp: &str) -> LineStoreResult<LineRecord> {
        let mut lines = self.lines.borrow_mut();
        let line = lines
            .get_mut(line_name)
            .ok_or_else(|| format!("Unknown line: {line_name}"))?;
        if line.status != "archived" {
            line.status = "archived".to_string();
            line.archived_at = Some(timestamp.to_string());
            line.updated_at = Some(timestamp.to_string());
        }
        Ok(line.clone())
    }

    fn set_line_head(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        timestamp: &str,
    ) -> LineStoreResult<LineRecord> {
        let mut lines = self.lines.borrow_mut();
        let line = lines
            .get_mut(line_name)
            .ok_or_else(|| format!("Unknown line: {line_name}"))?;
        line.head_snapshot_id = head_snapshot_id.map(ToString::to_string);
        line.updated_at = Some(timestamp.to_string());
        Ok(line.clone())
    }

    fn line_updated_at(&self, line_name: &str) -> LineStoreResult<Option<String>> {
        Ok(self
            .lines
            .borrow()
            .get(line_name)
            .and_then(|line| line.updated_at.clone()))
    }

    fn set_line_updated_at(
        &self,
        line_name: &str,
        updated_at: Option<&str>,
    ) -> LineStoreResult<()> {
        if let Some(line) = self.lines.borrow_mut().get_mut(line_name) {
            line.updated_at = updated_at.map(ToString::to_string);
        }
        Ok(())
    }

    fn touch_line_updated_at(&self, line_name: &str, timestamp: &str) -> LineStoreResult<()> {
        self.set_line_updated_at(line_name, Some(timestamp))
    }
}

impl SnapshotStore for FakeSnapshotChainStore {
    fn snapshot_exists(&self, snapshot_id: &str) -> SnapshotStoreResult<bool> {
        Ok(self.parents.contains_key(snapshot_id)
            || self.chains.contains_key(snapshot_id)
            || self
                .chains
                .values()
                .any(|chain| chain.iter().any(|value| value == snapshot_id)))
    }

    fn snapshot_parent_link(
        &self,
        snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<ait_core::snapshot_store::SnapshotParentLink>> {
        self.parent_link_reads
            .set(self.parent_link_reads.get().saturating_add(1));
        let parent_snapshot_ids = if let Some(parents) = self.parents.get(snapshot_id) {
            parents.clone()
        } else if let Some((chain, index)) = self.chains.values().find_map(|chain| {
            chain
                .iter()
                .position(|value| value == snapshot_id)
                .map(|index| (chain, index))
        }) {
            index
                .checked_sub(1)
                .and_then(|parent_index| chain.get(parent_index))
                .cloned()
                .into_iter()
                .collect()
        } else {
            return Ok(None);
        };
        let parent_snapshot_id = parent_snapshot_ids.first().cloned();
        Ok(Some(ait_core::snapshot_store::SnapshotParentLink {
            snapshot_id: snapshot_id.to_string(),
            parent_snapshot_ids,
            primary_parent_snapshot_id: parent_snapshot_id.clone(),
            parent_snapshot_id,
        }))
    }

    fn snapshot_by_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<SnapshotRecord>> {
        Ok(self.snapshots.get(snapshot_id).cloned())
    }

    fn list_line_snapshots(
        &self,
    ) -> SnapshotStoreResult<Vec<ait_core::snapshot_store::SnapshotRecord>> {
        Ok(Vec::new())
    }

    fn snapshot_total_bytes(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<i64>> {
        Ok(None)
    }

    fn snapshot_root_tree_pack_id(
        &self,
        _snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<String>> {
        Ok(None)
    }

    fn snapshot_kind(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        Ok(None)
    }

    fn snapshot_chain(&self, snapshot_id: &str) -> SnapshotStoreResult<Vec<String>> {
        self.chains
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| format!("Unknown snapshot: {snapshot_id}"))
    }

    fn set_snapshot_kind(
        &self,
        _snapshot_id: &str,
        _snapshot_kind: &str,
    ) -> SnapshotStoreResult<usize> {
        Ok(0)
    }
}

impl RepoStatusStore for FakeRepoStatusStore {
    fn storage_counts(&self) -> RepoStatusStoreResult<RepoStatusStorageCounts> {
        Ok(self.storage_counts.clone())
    }
}

fn fake_task_remote_stats(closed: bool) -> TaskWorkflowHttpClientStats {
    TaskWorkflowHttpClientStats {
        base_url: "https://ait.example".to_string(),
        default_timeout_ms: 30_000,
        retry_attempts: 1,
        retry_backoff_ms: 10,
        pool_max_idle_per_host: 2,
        request_count: 0,
        retry_count: 0,
        closed,
    }
}

impl TaskWorkflowTaskQueueReader for FakeQueueRemote {
    fn read_task_queue(
        &mut self,
        _repo_name: &str,
        _status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.task_queue
            .clone()
            .ok_or_else(|| TaskWorkflowHttpClientError::Remote("missing task queue".to_string()))
    }
}

impl TaskWorkflowReviewerInboxReader for FakeQueueRemote {
    fn read_reviewer_inbox(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.reviewer_inbox.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("missing reviewer inbox".to_string())
        })
    }
}

impl TaskWorkflowQueueSummaryBundleReader for FakeQueueRemote {
    fn read_queue_summary_bundle(
        &mut self,
        _repo_name: &str,
        _status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        if let Some(err) = self.queue_summary_error.as_ref() {
            return Err(TaskWorkflowHttpClientError::Remote(err.clone()));
        }
        self.queue_summary_bundle.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("missing queue summary bundle".to_string())
        })
    }
}

impl TaskWorkflowQueueChangeLister for FakeQueueRemote {
    fn list_changes(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.changes.clone())
    }
}

#[derive(Debug, Default)]
struct FakeTaskQueueReader;

impl TaskWorkflowTaskQueueReader for FakeTaskQueueReader {
    fn read_task_queue(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "status": status,
            "items": [],
        }))
    }
}

#[derive(Debug, Default)]
struct FakeReviewerInboxReader;

impl TaskWorkflowReviewerInboxReader for FakeReviewerInboxReader {
    fn read_reviewer_inbox(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "reviewer_inbox": true,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeQueueSummaryBundleReader;

impl TaskWorkflowQueueSummaryBundleReader for FakeQueueSummaryBundleReader {
    fn read_queue_summary_bundle(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "status": status,
            "summary": true,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeQueueChangeLister;

impl TaskWorkflowQueueChangeLister for FakeQueueChangeLister {
    fn list_changes(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "repo_name": repo_name,
            "change_id": "RCC-1",
        })])
    }
}

#[derive(Debug, Default)]
struct FakeRemoteChangeCreator;

impl TaskWorkflowRemoteChangeCreator for FakeRemoteChangeCreator {
    fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let contextual_change_id = format!("{task_id}/C-01");
        Ok(json!({
            "repo_name": repo_name,
            "change_id": change_id.unwrap_or(&contextual_change_id),
            "task_id": task_id,
            "title": title,
            "base_line": base_line,
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": forked_from_line,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeRemoteChangeLister;

impl TaskWorkflowRemoteChangeLister for FakeRemoteChangeLister {
    fn list_changes(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "repo_name": repo_name,
            "change_id": "RCC-1",
        })])
    }
}

#[derive(Debug, Default)]
struct FakeRemoteChangeDetailReader;

impl TaskWorkflowRemoteChangeDetailReader for FakeRemoteChangeDetailReader {
    fn get_change_detail(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "change_id": change_ref,
            "task_id": "RCT-1",
        }))
    }
}

#[derive(Debug, Default)]
struct FakeRemoteChangeReader;

impl TaskWorkflowRemoteChangeReader for FakeRemoteChangeReader {
    fn get_change(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "change_id": change_ref,
            "selected_patchset_id": "PS-1",
        }))
    }
}

#[derive(Debug, Default)]
struct FakeRemoteChangeCloser;

impl TaskWorkflowRemoteChangeCloser for FakeRemoteChangeCloser {
    fn close_change(
        &mut self,
        change_ref: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "change_id": change_ref,
            "status": status,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeLineLister;

impl TaskWorkflowLineLister for FakeLineLister {
    fn list_lines(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "repo_name": repo_name,
            "line_name": "main",
        })])
    }
}

#[derive(Debug, Default)]
struct FakeLineCloser;

impl TaskWorkflowLineCloser for FakeLineCloser {
    fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "status": status,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeLineReader;

impl TaskWorkflowLineReader for FakeLineReader {
    fn get_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "head_snapshot_id": "SNP-REMOTE",
        }))
    }
}

#[derive(Debug, Default)]
struct FakeLineHeadUpdater;

impl TaskWorkflowLineHeadUpdater for FakeLineHeadUpdater {
    fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "head_snapshot_id": head_snapshot_id,
            "expected_head_snapshot_id": expected_head_snapshot_id,
        }))
    }
}

impl TaskWorkflowRemoteChangeCreator for FakeChangeRemote {
    fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let resolved_change_id = change_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("{task_id}/C-{:02}", self.changes.len() + 1));
        let change = json!({
            "repo_name": repo_name,
            "change_id": resolved_change_id,
            "task_id": task_id,
            "title": title,
            "base_line": base_line,
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": forked_from_line,
            "status": "draft",
        });
        self.changes.push(change.clone());
        Ok(change)
    }
}

impl TaskWorkflowRemoteChangeLister for FakeChangeRemote {
    fn list_changes(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.changes.clone())
    }
}

impl TaskWorkflowRemoteChangeDetailReader for FakeChangeRemote {
    fn get_change_detail(
        &mut self,
        change_ref: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.detail.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!("missing detail for {change_ref}"))
        })
    }
}

impl TaskWorkflowRemoteChangeReader for FakeChangeRemote {
    fn get_change(
        &mut self,
        change_ref: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.changes
            .iter()
            .find(|row| string_field(row, "change_id").as_deref() == Some(change_ref))
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET change {change_ref} failed: 404 Unknown change"
                ))
            })
    }
}

impl TaskWorkflowRemoteChangeCloser for FakeChangeRemote {
    fn close_change(
        &mut self,
        change_ref: &str,
        status: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let change = self
            .changes
            .iter_mut()
            .find(|row| string_field(row, "change_id").as_deref() == Some(change_ref))
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "PATCH change {change_ref} failed: 404 Unknown change"
                ))
            })?;
        if let Some(change_obj) = change.as_object_mut() {
            change_obj.insert("status".to_string(), JsonValue::String(status.to_string()));
        }
        Ok(change.clone())
    }
}

#[derive(Debug, Default)]
struct FakePatchsetLister;

impl TaskWorkflowPatchsetLister for FakePatchsetLister {
    fn list_patchsets(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "repo_name": repo_name,
            "change_id": change_id,
            "patchset_id": "PS-1",
        })])
    }
}

#[derive(Debug, Default)]
struct FakePatchsetReader;

impl TaskWorkflowPatchsetReader for FakePatchsetReader {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "change_ref": change_ref,
            "patchset_id": patchset_id,
        }))
    }
}

#[derive(Debug, Default)]
struct FakePatchsetPublisher;

impl TaskWorkflowPatchsetPublisher for FakePatchsetPublisher {
    fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "patchset_id": "PS-1",
            "change_id": change_id,
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "author_mode": author_mode,
            "exact_id": exact_id,
        }))
    }
}

#[derive(Debug, Default)]
struct FakePatchsetSelector;

impl TaskWorkflowPatchsetSelector for FakePatchsetSelector {
    fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "change_id": change_id,
            "patchset_id": patchset_id,
            "exact_id": exact_id,
            "selected": true,
        }))
    }
}

#[derive(Debug, Default)]
struct FakePatchsetCiRunner;

impl TaskWorkflowPatchsetCiRunner for FakePatchsetCiRunner {
    fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "patchset_id": patchset_id,
            "trigger": trigger,
            "execution_profile": execution_profile,
            "exact_id": exact_id,
            "queued": true,
        }))
    }
}

impl TaskWorkflowLineagePayloadBuilder for FakeChangeRemote {
    fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        let fork_snapshot_id = line_row.and_then(|row| string_field(row, "head_snapshot_id"));
        Ok(json!({
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": base_line
        }))
    }
}

impl TaskWorkflowRepositoryEnsurer for FakeChangeRemote {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let capabilities = self
            .repository
            .as_ref()
            .and_then(|row| row.get("capabilities"))
            .cloned();
        let mut repository = json!({
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy.cloned().unwrap_or(JsonValue::Null),
            "id_namespace_prefix": id_namespace_prefix,
        });
        if let (Some(obj), Some(capabilities)) = (repository.as_object_mut(), capabilities) {
            obj.insert("capabilities".to_string(), capabilities);
        }
        self.repository = Some(repository.clone());
        self.ensured_repositories.push(repository.clone());
        Ok(repository)
    }
}

impl TaskWorkflowRepositoryReader for FakeChangeRemote {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.repository.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET repository {repo_name} failed: 404 Unknown repository"
            ))
        })
    }
}

impl TaskWorkflowLineReader for FakeChangeRemote {
    fn get_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.lines
            .iter()
            .find(|row| string_field(row, "line_name").as_deref() == Some(line_name))
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET line {line_name} failed: 404 Unknown line"
                ))
            })
    }
}

impl TaskWorkflowLineLister for FakeChangeRemote {
    fn list_lines(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.lines.clone())
    }
}

impl TaskWorkflowLineHeadUpdater for FakeChangeRemote {
    fn update_remote_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
        _head_snapshot_id: Option<&str>,
        _expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Err(TaskWorkflowHttpClientError::Remote(format!(
            "update line {line_name} is unused by change helper tests"
        )))
    }
}

impl TaskWorkflowLineCloser for FakeChangeRemote {
    fn close_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
        _status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Err(TaskWorkflowHttpClientError::Remote(format!(
            "close line {line_name} is unused by change helper tests"
        )))
    }
}

impl TaskWorkflowZstdPackUploader for FakeChangeRemote {
    fn plan_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkPlanRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkPlanResponse> {
        self.zstd_plan_requests.push(
            ZstdBulkPlanRequestJson::stateless()
                .encode_value(request)
                .map_err(TaskWorkflowHttpClientError::Remote)?,
        );
        let snapshot_ids = request.snapshot_ids.clone();
        let missing_snapshot_ids = snapshot_ids
            .iter()
            .filter(|snapshot_id| !self.remote_snapshots.contains_key(*snapshot_id))
            .cloned()
            .collect::<Vec<_>>();
        let present_snapshot_ids = snapshot_ids
            .iter()
            .filter(|snapshot_id| self.remote_snapshots.contains_key(*snapshot_id))
            .cloned()
            .collect::<Vec<_>>();
        let split_pack_ids =
            |pack_ids: Vec<String>, present_ids: &BTreeSet<String>| -> (Vec<String>, Vec<String>) {
                let mut present = Vec::new();
                let mut missing = Vec::new();
                for pack_id in pack_ids {
                    if present_ids.contains(&pack_id) {
                        present.push(pack_id);
                    } else {
                        missing.push(pack_id);
                    }
                }
                (present, missing)
            };
        let object_pack_ids = request
            .object_packs
            .iter()
            .map(|pack| pack.pack_id.clone())
            .collect::<Vec<_>>();
        let tree_pack_ids = request
            .tree_packs
            .iter()
            .map(|pack| pack.pack_id.clone())
            .collect::<Vec<_>>();
        let (present_object_pack_ids, missing_object_pack_ids) =
            split_pack_ids(object_pack_ids, &self.present_zstd_object_pack_ids);
        let (present_tree_pack_ids, missing_tree_pack_ids) =
            split_pack_ids(tree_pack_ids, &self.present_zstd_tree_pack_ids);
        Ok(ZstdBulkPlanResponse {
            repo_name: Some(repo_name.to_string()),
            present_snapshot_ids,
            missing_snapshot_ids,
            present_object_pack_ids,
            missing_object_pack_ids,
            present_tree_pack_ids,
            missing_tree_pack_ids,
        })
    }

    fn put_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        self.uploaded_zstd_object_packs
            .push((pack_id.to_string(), pack_bytes.to_vec()));
        self.present_zstd_object_pack_ids
            .insert(pack_id.to_string());
        Ok(ZstdPackUploadResponse {
            repo_name: Some(repo_name.to_string()),
            pack_id: pack_id.to_string(),
            stored: None,
            pack_format: None,
            checksum: None,
            pack_bytes: Some(pack_bytes.len() as i64),
            raw_binary_upload: Some(true),
        })
    }

    fn put_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        self.uploaded_zstd_tree_packs
            .push((pack_id.to_string(), pack_bytes.to_vec()));
        self.present_zstd_tree_pack_ids.insert(pack_id.to_string());
        Ok(ZstdPackUploadResponse {
            repo_name: Some(repo_name.to_string()),
            pack_id: pack_id.to_string(),
            stored: None,
            pack_format: None,
            checksum: None,
            pack_bytes: Some(pack_bytes.len() as i64),
            raw_binary_upload: Some(true),
        })
    }

    fn commit_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkCommitRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkCommitResponse> {
        self.zstd_commit_requests.push(
            ZstdBulkCommitRequestJson::stateless()
                .encode_value(request)
                .expect("fake zstd commit request should encode"),
        );
        let mut upserted_snapshots = 0_i64;
        for snapshot in &request.snapshots {
            let snapshot_id = snapshot.snapshot_id.clone();
            if !self.remote_snapshots.contains_key(&snapshot_id) {
                self.remote_snapshots.insert(
                    snapshot_id.clone(),
                    json!({
                        "repo_name": repo_name,
                        "snapshot_id": snapshot_id,
                    }),
                );
                upserted_snapshots += 1;
            }
        }
        let remote_line = match request.line_update.as_ref() {
            Some(line_update) => {
                let line_name = line_update.line_name.clone();
                let head_snapshot_id = line_update.head_snapshot_id.clone();
                let line = json!({
                    "repo_name": repo_name,
                    "line_name": line_name,
                    "status": "active",
                    "head_snapshot_id": head_snapshot_id,
                });
                if let Some(existing) = self
                    .lines
                    .iter_mut()
                    .find(|row| string_field(row, "line_name").as_deref() == Some(&line_name))
                {
                    *existing = line.clone();
                } else {
                    self.lines.push(line.clone());
                }
                Some(ZstdBulkRemoteLine {
                    repo_name: Some(repo_name.to_string()),
                    line_name: Some(line_name),
                    status: Some("active".to_string()),
                    head_snapshot_id,
                })
            }
            None => None,
        };
        Ok(ZstdBulkCommitResponse {
            repo_name: Some(repo_name.to_string()),
            committed_snapshot_ids: request
                .snapshots
                .iter()
                .map(|snapshot| snapshot.snapshot_id.clone())
                .collect(),
            committed_object_pack_ids: request
                .object_packs
                .iter()
                .map(|pack| pack.pack_id.clone())
                .collect(),
            committed_tree_pack_ids: request
                .tree_packs
                .iter()
                .map(|pack| pack.pack_id.clone())
                .collect(),
            upserted_snapshots: Some(upserted_snapshots),
            remote_line,
            line_update: None,
        })
    }
}

impl TaskWorkflowSnapshotMetadataReader for FakeChangeRemote {
    fn get_remote_snapshot(
        &mut self,
        _repo_name: &str,
        snapshot_id: &str,
        _include_content: bool,
        _path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.remote_snapshots
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET snapshot {snapshot_id} failed: 404 Unknown snapshot"
                ))
            })
    }
}

impl TaskWorkflowZstdPackReader for FakeChangeRemote {}

impl TaskWorkflowSnapshotExistenceReader for FakeChangeRemote {
    fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for snapshot_id in snapshot_ids {
            if self.remote_snapshots.contains_key(snapshot_id) {
                present.push(JsonValue::String(snapshot_id.clone()));
            } else {
                missing.push(JsonValue::String(snapshot_id.clone()));
            }
        }
        Ok(json!({
            "repo_name": repo_name,
            "requested": snapshot_ids,
            "present": present,
            "missing": missing,
        }))
    }
}

impl TaskWorkflowLineagePayloadBuilder for FakeLineSnapshotRemote {
    fn change_lineage_payload(
        &self,
        _base_line: &str,
        _line_row: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        unimplemented!("unused by remote sync helper tests")
    }
}

impl TaskWorkflowRepositoryEnsurer for FakeLineSnapshotRemote {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let capabilities = self
            .repository
            .as_ref()
            .and_then(|row| row.get("capabilities"))
            .cloned();
        let mut repository = json!({
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy.cloned().unwrap_or(JsonValue::Null),
            "id_namespace_prefix": id_namespace_prefix,
        });
        if let (Some(obj), Some(capabilities)) = (repository.as_object_mut(), capabilities) {
            obj.insert("capabilities".to_string(), capabilities);
        }
        self.repository = Some(repository.clone());
        self.ensured_repositories.push(repository.clone());
        Ok(repository)
    }
}

impl TaskWorkflowRepositoryReader for FakeLineSnapshotRemote {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.repository.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET repository {repo_name} failed: 404 Unknown repository"
            ))
        })
    }
}

impl TaskWorkflowLineReader for FakeLineSnapshotRemote {
    fn get_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.lines
            .iter()
            .find(|row| string_field(row, "line_name").as_deref() == Some(line_name))
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET line {line_name} failed: 404 Unknown line"
                ))
            })
    }
}

impl TaskWorkflowLineLister for FakeLineSnapshotRemote {
    fn list_lines(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.lines.clone())
    }
}

impl TaskWorkflowLineHeadUpdater for FakeLineSnapshotRemote {
    fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.line_update_calls += 1;
        let line_index = self
            .lines
            .iter()
            .position(|line| string_field(line, "line_name").as_deref() == Some(line_name));
        let Some(line_index) = line_index else {
            if expected_head_snapshot_id.is_some() {
                return Err(TaskWorkflowHttpClientError::Remote(format!(
                    "Remote line {line_name} missing while expected head was {expected_head_snapshot_id:?}"
                )));
            }
            let line = json!({
                "repo_name": repo_name,
                "line_name": line_name,
                "status": "active",
                "head_snapshot_id": head_snapshot_id,
            });
            self.lines.push(line.clone());
            return Ok(line);
        };
        let current_head_snapshot_id = string_field(&self.lines[line_index], "head_snapshot_id");
        if current_head_snapshot_id.as_deref() != expected_head_snapshot_id {
            return Err(TaskWorkflowHttpClientError::Remote(format!(
                "Remote line {line_name} expected head {expected_head_snapshot_id:?} but found {current_head_snapshot_id:?}"
            )));
        }
        let line = self.lines[line_index].as_object_mut().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("line row must be an object".to_string())
        })?;
        line.insert(
            "repo_name".to_string(),
            JsonValue::String(repo_name.to_string()),
        );
        line.insert(
            "line_name".to_string(),
            JsonValue::String(line_name.to_string()),
        );
        line.insert(
            "head_snapshot_id".to_string(),
            head_snapshot_id
                .map(|value| JsonValue::String(value.to_string()))
                .unwrap_or(JsonValue::Null),
        );
        Ok(JsonValue::Object(line.clone()))
    }
}

impl TaskWorkflowLineCloser for FakeLineSnapshotRemote {
    fn close_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let line = self
            .lines
            .iter_mut()
            .find(|line| string_field(line, "line_name").as_deref() == Some(line_name))
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "POST close line {line_name} failed: 404 Unknown line"
                ))
            })?;
        let obj = line.as_object_mut().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("line row must be an object".to_string())
        })?;
        obj.insert("status".to_string(), JsonValue::String(status.to_string()));
        Ok(JsonValue::Object(obj.clone()))
    }
}

impl TaskWorkflowZstdPackUploader for FakeLineSnapshotRemote {
    fn plan_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkPlanRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkPlanResponse> {
        self.zstd_plan_requests.push(
            ZstdBulkPlanRequestJson::stateless()
                .encode_value(request)
                .map_err(TaskWorkflowHttpClientError::Remote)?,
        );
        let snapshot_ids = request.snapshot_ids.clone();
        let missing_snapshot_ids = snapshot_ids
            .iter()
            .filter(|snapshot_id| !self.remote_snapshots.contains_key(*snapshot_id))
            .cloned()
            .collect::<Vec<_>>();
        let present_snapshot_ids = snapshot_ids
            .iter()
            .filter(|snapshot_id| self.remote_snapshots.contains_key(*snapshot_id))
            .cloned()
            .collect::<Vec<_>>();
        let split_pack_ids =
            |pack_ids: Vec<String>, present_ids: &BTreeSet<String>| -> (Vec<String>, Vec<String>) {
                let mut present = Vec::new();
                let mut missing = Vec::new();
                for pack_id in pack_ids {
                    if present_ids.contains(&pack_id) {
                        present.push(pack_id);
                    } else {
                        missing.push(pack_id);
                    }
                }
                (present, missing)
            };
        let object_pack_ids = request
            .object_packs
            .iter()
            .map(|pack| pack.pack_id.clone())
            .collect::<Vec<_>>();
        let tree_pack_ids = request
            .tree_packs
            .iter()
            .map(|pack| pack.pack_id.clone())
            .collect::<Vec<_>>();
        let (present_object_pack_ids, missing_object_pack_ids) =
            split_pack_ids(object_pack_ids, &self.present_zstd_object_pack_ids);
        let (present_tree_pack_ids, missing_tree_pack_ids) =
            split_pack_ids(tree_pack_ids, &self.present_zstd_tree_pack_ids);
        Ok(ZstdBulkPlanResponse {
            repo_name: Some(repo_name.to_string()),
            present_snapshot_ids,
            missing_snapshot_ids,
            present_object_pack_ids,
            missing_object_pack_ids,
            present_tree_pack_ids,
            missing_tree_pack_ids,
        })
    }

    fn put_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        if self.fail_zstd_object_pack_upload_for.as_deref() == Some(pack_id) {
            return Err(TaskWorkflowHttpClientError::Remote(format!(
                "PUT zstd object pack {pack_id} failed: injected upload failure"
            )));
        }
        self.uploaded_zstd_object_packs
            .push((pack_id.to_string(), pack_bytes.to_vec()));
        self.present_zstd_object_pack_ids
            .insert(pack_id.to_string());
        Ok(ZstdPackUploadResponse {
            repo_name: Some(repo_name.to_string()),
            pack_id: pack_id.to_string(),
            stored: None,
            pack_format: None,
            checksum: None,
            pack_bytes: Some(pack_bytes.len() as i64),
            raw_binary_upload: Some(true),
        })
    }

    fn put_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse> {
        if self.fail_zstd_tree_pack_upload_for.as_deref() == Some(pack_id) {
            return Err(TaskWorkflowHttpClientError::Remote(format!(
                "PUT zstd tree pack {pack_id} failed: injected upload failure"
            )));
        }
        self.uploaded_zstd_tree_packs
            .push((pack_id.to_string(), pack_bytes.to_vec()));
        self.present_zstd_tree_pack_ids.insert(pack_id.to_string());
        Ok(ZstdPackUploadResponse {
            repo_name: Some(repo_name.to_string()),
            pack_id: pack_id.to_string(),
            stored: None,
            pack_format: None,
            checksum: None,
            pack_bytes: Some(pack_bytes.len() as i64),
            raw_binary_upload: Some(true),
        })
    }

    fn commit_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkCommitRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdBulkCommitResponse> {
        self.zstd_commit_requests.push(
            ZstdBulkCommitRequestJson::stateless()
                .encode_value(request)
                .expect("fake zstd commit request should encode"),
        );
        let mut upserted_snapshots = 0_i64;
        for snapshot in &request.snapshots {
            let snapshot_id = snapshot.snapshot_id.clone();
            if !self.remote_snapshots.contains_key(&snapshot_id) {
                self.remote_snapshots.insert(
                    snapshot_id.clone(),
                    json!({
                        "repo_name": repo_name,
                        "snapshot_id": snapshot_id,
                    }),
                );
                upserted_snapshots += 1;
            }
        }
        let remote_line = match request.line_update.as_ref() {
            Some(line_update) => {
                let line_name = line_update.line_name.clone();
                let head_snapshot_id = line_update.head_snapshot_id.clone();
                let line = json!({
                    "repo_name": repo_name,
                    "line_name": line_name,
                    "status": "active",
                    "head_snapshot_id": head_snapshot_id,
                });
                if let Some(existing) = self
                    .lines
                    .iter_mut()
                    .find(|row| string_field(row, "line_name").as_deref() == Some(&line_name))
                {
                    *existing = line.clone();
                } else {
                    self.lines.push(line.clone());
                }
                Some(ZstdBulkRemoteLine {
                    repo_name: Some(repo_name.to_string()),
                    line_name: Some(line_name),
                    status: Some("active".to_string()),
                    head_snapshot_id,
                })
            }
            None => None,
        };
        Ok(ZstdBulkCommitResponse {
            repo_name: Some(repo_name.to_string()),
            committed_snapshot_ids: request
                .snapshots
                .iter()
                .map(|snapshot| snapshot.snapshot_id.clone())
                .collect(),
            committed_object_pack_ids: request
                .object_packs
                .iter()
                .map(|pack| pack.pack_id.clone())
                .collect(),
            committed_tree_pack_ids: request
                .tree_packs
                .iter()
                .map(|pack| pack.pack_id.clone())
                .collect(),
            upserted_snapshots: Some(upserted_snapshots),
            remote_line,
            line_update: None,
        })
    }
}

impl TaskWorkflowSnapshotMetadataReader for FakeLineSnapshotRemote {
    fn get_remote_snapshot(
        &mut self,
        _repo_name: &str,
        snapshot_id: &str,
        _include_content: bool,
        _path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.remote_snapshots
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET snapshot {snapshot_id} failed: 404 Unknown snapshot"
                ))
            })
    }
}

impl TaskWorkflowZstdPackReader for FakeLineSnapshotRemote {
    fn get_remote_zstd_import_manifest(
        &mut self,
        _repo_name: &str,
        snapshot_id: &str,
    ) -> TaskWorkflowHttpClientResult<ZstdImportManifestPayload> {
        self.zstd_import_manifest_reads
            .push(snapshot_id.to_string());
        self.zstd_import_manifests
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "missing zstd import manifest {snapshot_id}"
                ))
            })
    }

    fn get_remote_zstd_pull_manifest(
        &mut self,
        _repo_name: &str,
        request: &ZstdPullManifestRequest,
    ) -> TaskWorkflowHttpClientResult<ZstdPullManifestPayload> {
        self.zstd_pull_manifest_requests.push(request.clone());
        if let Some(manifest) = self.zstd_pull_manifests.pop_front() {
            return Ok(manifest);
        }
        self.zstd_pull_manifest.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("missing zstd pull manifest".to_string())
        })
    }

    fn get_remote_zstd_object_pack(
        &mut self,
        _repo_name: &str,
        pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        self.zstd_object_pack_downloads.push(pack_id.to_string());
        self.zstd_object_packs.get(pack_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!("missing zstd object pack {pack_id}"))
        })
    }

    fn get_remote_zstd_tree_pack(
        &mut self,
        _repo_name: &str,
        pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        self.zstd_tree_pack_downloads.push(pack_id.to_string());
        self.zstd_tree_packs.get(pack_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!("missing zstd tree pack {pack_id}"))
        })
    }
}

impl TaskWorkflowSnapshotExistenceReader for FakeLineSnapshotRemote {
    fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for snapshot_id in snapshot_ids {
            if self.remote_snapshots.contains_key(snapshot_id) {
                present.push(JsonValue::String(snapshot_id.clone()));
            } else {
                missing.push(JsonValue::String(snapshot_id.clone()));
            }
        }
        Ok(json!({
            "repo_name": repo_name,
            "requested": snapshot_ids,
            "present": present,
            "missing": missing,
        }))
    }
}

impl TaskWorkflowRemoteTaskReader for FakeTaskRecordRemote {
    fn get_task(
        &mut self,
        task_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.tasks.get(task_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET task {task_id} failed: 404 Unknown task"
            ))
        })
    }
}

impl TaskWorkflowRemoteTaskLister for FakeTaskRecordRemote {
    fn list_tasks(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.tasks.values().cloned().collect())
    }
}

impl TaskWorkflowRemoteTaskAuditReader for FakeTaskRecordRemote {
    fn read_task_audit(
        &mut self,
        _repo_name: &str,
        _task_id: &str,
        _target_line: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.task_audit
            .clone()
            .ok_or_else(|| TaskWorkflowHttpClientError::Remote("missing task audit".to_string()))
    }
}

impl TaskWorkflowRemoteTaskCreator for FakeTaskRecordRemote {
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.task_create_repository_present
            .push(self.repository.is_some());
        self.task_create_requested_ids
            .push(task_id.map(str::to_string));
        let resolved_task_id = task_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("RCT-{}", self.tasks.len() + 1));
        let returned_task_id = self
            .created_task_id_override
            .clone()
            .unwrap_or_else(|| resolved_task_id.clone());
        let task = json!({
            "repo_name": repo_name,
            "task_id": returned_task_id,
            "title": title,
            "intent": intent,
            "status": "open",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        });
        self.tasks.insert(returned_task_id, task.clone());
        Ok(task)
    }
}

impl TaskWorkflowRepositoryEnsurer for FakeTaskRecordRemote {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let repository = json!({
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy.cloned().unwrap_or(JsonValue::Null),
            "id_namespace_prefix": id_namespace_prefix,
        });
        self.repository = Some(repository.clone());
        self.ensured_repositories.push(repository.clone());
        Ok(repository)
    }
}

impl TaskWorkflowRepositoryReader for FakeTaskRecordRemote {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.repository.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET repository {repo_name} failed: 404 Unknown repository"
            ))
        })
    }
}

impl TaskWorkflowRemoteTaskReader for FakeTaskAuditRemote {
    fn get_task(
        &mut self,
        task_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.tasks.get(task_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET task {task_id} failed: 404 Unknown task"
            ))
        })
    }
}

impl TaskWorkflowRemoteTaskLister for FakeTaskAuditRemote {
    fn list_tasks(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.tasks.values().cloned().collect())
    }
}

impl TaskWorkflowRemoteTaskAuditReader for FakeTaskAuditRemote {
    fn read_task_audit(
        &mut self,
        _repo_name: &str,
        task_id: &str,
        _target_line: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.task_audit_requests.push(task_id.to_string());
        self.task_audit
            .clone()
            .ok_or_else(|| TaskWorkflowHttpClientError::Remote("missing task audit".to_string()))
    }
}

impl TaskWorkflowRemoteTaskCreator for FakeTaskAuditRemote {
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let resolved_task_id = task_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("RCT-{}", self.tasks.len() + 1));
        let task = json!({
            "repo_name": repo_name,
            "task_id": resolved_task_id,
            "title": title,
            "intent": intent,
            "status": "open",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        });
        self.tasks.insert(resolved_task_id, task.clone());
        Ok(task)
    }
}

impl TaskWorkflowRemoteTaskReader for FakeRemoteTaskReaderOnly {
    fn get_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "task_id": task_id,
            "repo_name": repo_name,
        }))
    }
}

impl TaskWorkflowRemoteTaskLister for FakeRemoteTaskListerOnly {
    fn list_tasks(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "task_id": "T-LIST",
            "repo_name": repo_name,
        })])
    }
}

impl TaskWorkflowRemoteTaskAuditReader for FakeRemoteTaskAuditReaderOnly {
    fn read_task_audit(
        &mut self,
        repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id,
            "target_line": target_line,
        }))
    }
}

impl TaskWorkflowRemoteTaskCreator for FakeRemoteTaskCreatorOnly {
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id.unwrap_or("T-CREATE"),
            "title": title,
            "intent": intent,
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        }))
    }
}

impl TaskWorkflowLineagePayloadBuilder for FakeTaskAuditRemote {
    fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        let fork_snapshot_id = line_row.and_then(|row| string_field(row, "head_snapshot_id"));
        Ok(json!({
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": base_line
        }))
    }
}

impl TaskWorkflowLineReader for FakeTaskAuditRemote {
    fn get_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.lines
            .iter()
            .find(|row| string_field(row, "line_name").as_deref() == Some(line_name))
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET line {line_name} failed: 404 Unknown line"
                ))
            })
    }
}

impl TaskWorkflowLineLister for FakeTaskAuditRemote {
    fn list_lines(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.lines.clone())
    }
}

impl TaskWorkflowLineHeadUpdater for FakeTaskAuditRemote {
    fn update_remote_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
        _head_snapshot_id: Option<&str>,
        _expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Err(TaskWorkflowHttpClientError::Remote(format!(
            "update line {line_name} is unused by task audit helper tests"
        )))
    }
}

impl TaskWorkflowLineCloser for FakeTaskAuditRemote {
    fn close_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
        _status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Err(TaskWorkflowHttpClientError::Remote(format!(
            "close line {line_name} is unused by task audit helper tests"
        )))
    }
}

impl TaskWorkflowZstdPackUploader for FakeTaskAuditRemote {}

impl TaskWorkflowSnapshotMetadataReader for FakeTaskAuditRemote {
    fn get_remote_snapshot(
        &mut self,
        _repo_name: &str,
        snapshot_id: &str,
        _include_content: bool,
        _path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.remote_snapshots
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET snapshot {snapshot_id} failed: 404 Unknown snapshot"
                ))
            })
    }
}

impl TaskWorkflowZstdPackReader for FakeTaskAuditRemote {}

impl TaskWorkflowSnapshotExistenceReader for FakeTaskAuditRemote {
    fn get_remote_snapshots_existence(
        &mut self,
        _repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let present = snapshot_ids
            .iter()
            .filter(|snapshot_id| self.remote_snapshots.contains_key(*snapshot_id))
            .cloned()
            .collect::<Vec<_>>();
        Ok(json!({"present": present}))
    }
}

impl TaskWorkflowTaskLister for FakeTaskStore {
    fn list_tasks(&self) -> PlanStoreResult<Vec<JsonValue>> {
        Ok(self.tasks.borrow().values().cloned().collect())
    }
}

impl TaskWorkflowTaskReader for FakeTaskStore {
    fn get_task(&self, task_id: &str) -> PlanStoreResult<JsonValue> {
        self.tasks
            .borrow()
            .get(task_id)
            .cloned()
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown task: {task_id}")))
    }
}

impl TaskWorkflowTaskCreator for FakeTaskStore {
    fn create_task(
        &self,
        repo_name: &str,
        title: &str,
        intent: &str,
        namespace_prefix: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        let task_id = format!(
            "{}-{}",
            namespace_prefix.unwrap_or("LCT"),
            self.tasks.borrow().len() + 1
        );
        let task = json!({
            "repo_name": repo_name,
            "task_id": task_id,
            "title": title,
            "intent": intent,
            "status": "active",
            "publication_state": "local_draft",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        });
        self.tasks
            .borrow_mut()
            .insert(task_id.to_string(), task.clone());
        Ok(task)
    }
}

impl TaskWorkflowTaskCloser for FakeTaskStore {
    fn close_task(&self, task_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
        let mut tasks = self.tasks.borrow_mut();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown task: {task_id}")))?;
        task["status"] = JsonValue::String(status.to_string());
        Ok(task.clone())
    }
}

impl TaskWorkflowTaskPublisher for FakeTaskStore {
    fn mark_task_published(
        &self,
        task_id: &str,
        remote_name: Option<&str>,
        published_task_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        let mut tasks = self.tasks.borrow_mut();
        let task = tasks
            .get_mut(task_id)
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown task: {task_id}")))?;
        task["publication_state"] = JsonValue::String("published".to_string());
        task["published_remote_name"] = remote_name
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null);
        task["published_task_id"] = published_task_id
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null);
        Ok(task.clone())
    }
}

impl ait_core::task_store::TaskStore for FakeTaskStore {
    fn list_tasks(&self) -> PlanStoreResult<Vec<JsonValue>> {
        TaskWorkflowTaskLister::list_tasks(self)
    }

    fn list_completed_tasks_with_landed_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
        TaskWorkflowTaskLister::list_tasks(self).map(|tasks| {
            tasks
                .into_iter()
                .filter(|task| task["status"].as_str() == Some("completed"))
                .collect()
        })
    }

    fn get_task(&self, task_id: &str) -> PlanStoreResult<JsonValue> {
        TaskWorkflowTaskReader::get_task(self, task_id)
    }

    fn allocate_task_identity(
        &self,
        _repo_name: &str,
        _namespace_prefix: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "fake task store does not allocate identities".to_string(),
        ))
    }

    fn sequence_floor(&self, _repo_name: &str, _family: &str) -> PlanStoreResult<i64> {
        Ok(0)
    }

    fn create_task(
        &self,
        repo_name: &str,
        title: &str,
        intent: &str,
        namespace_prefix: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        TaskWorkflowTaskCreator::create_task(
            self,
            repo_name,
            title,
            intent,
            namespace_prefix,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
        )
    }

    fn create_task_explicit(
        &self,
        _task_id: &str,
        _repo_name: &str,
        _title: &str,
        _intent: &str,
        _task_seq: Option<i64>,
        _identity_source: Option<&str>,
        _planning_state: Option<&str>,
        _plan_id: Option<&str>,
        _origin_plan_revision_id: Option<&str>,
        _plan_item_ref: Option<&str>,
        _plan_linked_at: Option<&str>,
        _status: Option<&str>,
        _publication_state: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "fake task store does not create explicit tasks".to_string(),
        ))
    }

    fn close_task(&self, task_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
        TaskWorkflowTaskCloser::close_task(self, task_id, status)
    }

    fn mark_task_published(
        &self,
        task_id: &str,
        remote_name: Option<&str>,
        published_task_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        TaskWorkflowTaskPublisher::mark_task_published(
            self,
            task_id,
            remote_name,
            published_task_id,
        )
    }
}

impl TaskWorkflowChangeLister for FakeChangeStore {
    fn list_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
        Ok(self.changes.borrow().values().cloned().collect())
    }
}

impl ait_core::change_store::ChangeStore for FakeChangeStore {
    fn list_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
        TaskWorkflowChangeLister::list_changes(self)
    }

    fn get_change(&self, change_id: &str) -> PlanStoreResult<JsonValue> {
        TaskWorkflowChangeReader::get_change(self, change_id)
    }

    fn allocate_change_identity(
        &self,
        _repo_name: &str,
        _namespace_prefix: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "fake change store does not allocate identities".to_string(),
        ))
    }

    fn create_change(
        &self,
        task_id: &str,
        repo_name: &str,
        title: &str,
        base_line: &str,
        namespace_prefix: Option<&str>,
        fork_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        TaskWorkflowChangeCreator::create_change(
            self,
            repo_name,
            task_id,
            title,
            base_line,
            namespace_prefix,
            fork_snapshot_id,
        )
    }

    fn create_change_explicit(
        &self,
        _change_id: &str,
        _task_id: &str,
        _repo_name: &str,
        _title: &str,
        _base_line: &str,
        _change_seq: Option<i64>,
        _identity_source: Option<&str>,
        _fork_snapshot_id: Option<&str>,
        _forked_from_line: Option<&str>,
        _status: Option<&str>,
        _publication_state: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "fake change store does not create explicit changes".to_string(),
        ))
    }

    fn close_change(&self, change_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
        TaskWorkflowChangeCloser::close_change(self, change_id, status)
    }

    fn land_change(
        &self,
        change_id: &str,
        target_line: &str,
        landed_snapshot_id: &str,
        pre_land_target_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        TaskWorkflowChangeLander::land_change(
            self,
            change_id,
            target_line,
            landed_snapshot_id,
            pre_land_target_snapshot_id,
        )
    }

    fn mark_change_published(
        &self,
        change_id: &str,
        remote_name: Option<&str>,
        published_change_id: Option<&str>,
        allow_landed: bool,
    ) -> PlanStoreResult<JsonValue> {
        TaskWorkflowChangePublisher::mark_change_published(
            self,
            change_id,
            remote_name,
            published_change_id,
            allow_landed,
        )
    }
}

impl TaskWorkflowChangeReader for FakeChangeStore {
    fn get_change(&self, change_id: &str) -> PlanStoreResult<JsonValue> {
        self.changes
            .borrow()
            .get(change_id)
            .cloned()
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown change: {change_id}")))
    }
}

impl TaskWorkflowChangeCreator for FakeChangeStore {
    fn create_change(
        &self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        namespace_prefix: Option<&str>,
        fork_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        let change_id = format!(
            "{}-{}",
            namespace_prefix.unwrap_or("LCC"),
            self.changes.borrow().len() + 1
        );
        let change = json!({
            "repo_name": repo_name,
            "task_id": task_id,
            "change_id": change_id,
            "title": title,
            "base_line": base_line,
            "fork_snapshot_id": fork_snapshot_id,
            "status": "draft",
            "publication_state": "local_draft",
        });
        self.changes
            .borrow_mut()
            .insert(change_id.to_string(), change.clone());
        Ok(change)
    }
}

impl TaskWorkflowChangeCloser for FakeChangeStore {
    fn close_change(&self, change_id: &str, status: &str) -> PlanStoreResult<JsonValue> {
        let mut changes = self.changes.borrow_mut();
        let change = changes
            .get_mut(change_id)
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown change: {change_id}")))?;
        change["status"] = JsonValue::String(status.to_string());
        Ok(change.clone())
    }
}

impl TaskWorkflowChangeLander for FakeChangeStore {
    fn land_change(
        &self,
        change_id: &str,
        target_line: &str,
        landed_snapshot_id: &str,
        pre_land_target_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        let mut changes = self.changes.borrow_mut();
        let change = changes
            .get_mut(change_id)
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown change: {change_id}")))?;
        change["status"] = JsonValue::String("landed".to_string());
        change["target_line"] = JsonValue::String(target_line.to_string());
        change["landed_snapshot_id"] = JsonValue::String(landed_snapshot_id.to_string());
        change["pre_land_target_snapshot_id"] = pre_land_target_snapshot_id
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null);
        Ok(change.clone())
    }
}

impl TaskWorkflowChangePublisher for FakeChangeStore {
    fn mark_change_published(
        &self,
        change_id: &str,
        remote_name: Option<&str>,
        published_change_id: Option<&str>,
        _allow_landed: bool,
    ) -> PlanStoreResult<JsonValue> {
        let mut changes = self.changes.borrow_mut();
        let change = changes
            .get_mut(change_id)
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown change: {change_id}")))?;
        change["publication_state"] = JsonValue::String("published".to_string());
        change["published_remote_name"] = remote_name
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null);
        change["published_change_id"] = published_change_id
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null);
        Ok(change.clone())
    }
}

impl TaskWorkflowHttpClientInspector for FakeWorkspaceCloseoutRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        fake_task_remote_stats(false)
    }
}

impl TaskWorkflowHttpClientCloser for FakeWorkspaceCloseoutRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        fake_task_remote_stats(true)
    }
}

impl TaskWorkflowMutationReceiptBuilder for FakeWorkspaceCloseoutRemote {
    fn mutation_receipt(
        &self,
        _action: &str,
        _source_action: &str,
        _delivery: &str,
        _response_recovery: Option<&JsonValue>,
        _result: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowActionMutationReceiptsBuilder for FakeWorkspaceCloseoutRemote {
    fn action_mutation_receipts(
        &self,
        _code: &str,
        _result: &JsonValue,
    ) -> Result<JsonValue, String> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowPatchsetLister for FakeWorkspaceCloseoutRemote {
    fn list_patchsets(
        &mut self,
        change_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self
            .patchsets
            .values()
            .filter(|row| string_field(row, "change_id").as_deref() == Some(change_id))
            .cloned()
            .collect())
    }
}

impl TaskWorkflowPatchsetReader for FakeWorkspaceCloseoutRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        _repo_name: Option<&str>,
        _change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.patchset_reads.push(patchset_id.to_string());
        self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })
    }
}

impl TaskWorkflowPatchsetPublisher for FakeWorkspaceCloseoutRemote {
    fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let patchset_number = self.patchsets.len() + 1;
        let patchset_id = format!("RCP-{change_id}-{patchset_number}");
        let patchset = json!({
            "patchset_id": patchset_id,
            "patchset_number": patchset_number,
            "change_id": change_id,
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "author_mode": author_mode,
            "repo_name": repo_name,
        });
        self.patchsets.insert(patchset_id, patchset.clone());
        Ok(patchset)
    }
}

impl TaskWorkflowPatchsetSelector for FakeWorkspaceCloseoutRemote {
    fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let patchset = self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "POST select patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })?;
        let selection = json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "patchset_number": patchset.get("patchset_number").cloned().unwrap_or(JsonValue::Null),
            "repo_name": repo_name,
            "selected": true,
        });
        self.selected_patchsets.push(selection.clone());
        Ok(selection)
    }
}

impl TaskWorkflowPatchsetCiRunner for FakeWorkspaceCloseoutRemote {
    fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let run = json!({
            "patchset_id": patchset_id,
            "trigger": trigger,
            "execution_profile": execution_profile,
            "repo_name": repo_name,
            "queued": false,
            "tests_status": "pass",
        });
        self.ci_runs.push(run.clone());
        Ok(run)
    }
}

impl TaskWorkflowReviewRequester for FakeWorkspaceCloseoutRemote {
    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let request = json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer_groups": reviewer_groups,
            "note": note,
            "repo_name": repo_name,
        });
        self.review_requests
            .entry(change_id.to_string())
            .or_default()
            .push(request.clone());
        Ok(request)
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakeWorkspaceCloseoutRemote {
    fn read_patchset_ci_status(
        &mut self,
        _patchset_id: &str,
        _recent_limit: i64,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowRepoJobLister for FakeWorkspaceCloseoutRemote {
    fn list_repo_jobs(
        &mut self,
        _repo_name: &str,
        _state: Option<&str>,
        _limit: i64,
        _diagnostics: bool,
        _stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowReviewRecorder for FakeWorkspaceCloseoutRemote {
    fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let review = json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "action": action,
            "comment": comment,
            "blocking": blocking,
            "repo_name": repo_name,
            "recorded": true,
        });
        self.reviews
            .entry(change_id.to_string())
            .or_default()
            .push(review.clone());
        Ok(review)
    }
}

impl TaskWorkflowReviewLister for FakeWorkspaceCloseoutRemote {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "repo_name": repo_name,
            "reviews": self.reviews.get(change_id).cloned().unwrap_or_default(),
        }))
    }
}

impl TaskWorkflowAttestationWriter for FakeWorkspaceCloseoutRemote {
    fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &JsonValue,
        provenance_summary: &JsonValue,
        detail: &JsonValue,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let attestation = json!({
            "patchset_id": patchset_id,
            "author_mode": author_mode,
            "evaluation_summary": evaluation_summary,
            "provenance_summary": provenance_summary,
            "detail": detail,
            "repo_name": repo_name,
        });
        self.attestations
            .insert(patchset_id.to_string(), attestation.clone());
        Ok(attestation)
    }
}

impl TaskWorkflowAttestationReader for FakeWorkspaceCloseoutRemote {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.attestations.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET attestation {patchset_id} failed: 404 Unknown attestation"
            ))
        })
    }
}

impl TaskWorkflowPolicyEvaluator for FakeWorkspaceCloseoutRemote {
    fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let policy = json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "decision": "pass",
        });
        self.policy_evaluations.push(policy.clone());
        Ok(policy)
    }
}

impl TaskWorkflowPolicyReader for FakeWorkspaceCloseoutRemote {
    fn get_policy(
        &mut self,
        _patchset_id: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowPolicyWaiverCreator for FakeWorkspaceCloseoutRemote {
    fn create_waiver(
        &mut self,
        _patchset_id: &str,
        _rule_name: &str,
        _reason: &str,
        _expires_at: Option<&str>,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowLandSubmitter for FakeWorkspaceCloseoutRemote {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let submission = json!({
            "submission_id": format!("LAND-{}", self.land_submissions.len() + 1),
            "change_id": change_id,
            "patchset_id": patchset_id,
            "target_line": target_line,
            "mode": mode,
            "repo_name": repo_name,
        });
        self.land_submissions.push(submission.clone());
        Ok(submission)
    }
}

impl TaskWorkflowLandReader for FakeWorkspaceCloseoutRemote {
    fn get_land(
        &mut self,
        _submission_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowLandRetryer for FakeWorkspaceCloseoutRemote {
    fn retry_land(
        &mut self,
        _submission_id: &str,
        _reason: Option<&str>,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowRemoteTaskCloser for FakeWorkspaceCloseoutRemote {
    fn close_task(
        &mut self,
        _task_id: &str,
        _status: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workspace closeout helper tests")
    }
}

impl TaskWorkflowHttpClientInspector for FakeWorkspaceTaskRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        fake_task_remote_stats(false)
    }
}

impl TaskWorkflowHttpClientCloser for FakeWorkspaceTaskRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        fake_task_remote_stats(true)
    }
}

impl TaskWorkflowRepositoryEnsurer for FakeWorkspaceTaskRemote {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let repository = json!({
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy.cloned().unwrap_or(JsonValue::Null),
            "id_namespace_prefix": id_namespace_prefix,
        });
        self.repository = Some(repository.clone());
        self.ensured_repositories.push(repository.clone());
        Ok(repository)
    }
}

impl TaskWorkflowRepositoryReader for FakeWorkspaceTaskRemote {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.repository.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET repository {repo_name} failed: 404 Unknown repository"
            ))
        })
    }
}

impl TaskWorkflowLineagePayloadBuilder for FakeWorkspaceTaskRemote {
    fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        let fork_snapshot_id = line_row.and_then(|row| string_field(row, "head_snapshot_id"));
        Ok(json!({
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": base_line
        }))
    }
}

impl TaskWorkflowLineReader for FakeWorkspaceTaskRemote {
    fn get_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.lines
            .iter()
            .find(|row| string_field(row, "line_name").as_deref() == Some(line_name))
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET line {line_name} failed: 404 Unknown line"
                ))
            })
    }
}

impl TaskWorkflowLineLister for FakeWorkspaceTaskRemote {
    fn list_lines(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.lines.clone())
    }
}

impl TaskWorkflowRemoteTaskReader for FakeWorkspaceTaskRemote {
    fn get_task(
        &mut self,
        task_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.tasks.get(task_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET task {task_id} failed: 404 Unknown task"
            ))
        })
    }
}

impl TaskWorkflowRemoteTaskLister for FakeWorkspaceTaskRemote {
    fn list_tasks(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.tasks.values().cloned().collect())
    }
}

impl TaskWorkflowRemoteTaskAuditReader for FakeWorkspaceTaskRemote {
    fn read_task_audit(
        &mut self,
        _repo_name: &str,
        _task_id: &str,
        _target_line: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.task_audit
            .clone()
            .ok_or_else(|| TaskWorkflowHttpClientError::Remote("missing task audit".to_string()))
    }
}

impl TaskWorkflowTaskQueueReader for FakeWorkspaceTaskRemote {
    fn read_task_queue(
        &mut self,
        _repo_name: &str,
        _status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.task_queue
            .clone()
            .ok_or_else(|| TaskWorkflowHttpClientError::Remote("missing task queue".to_string()))
    }
}

impl TaskWorkflowReviewerInboxReader for FakeWorkspaceTaskRemote {
    fn read_reviewer_inbox(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.reviewer_inbox.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("missing reviewer inbox".to_string())
        })
    }
}

impl TaskWorkflowQueueSummaryBundleReader for FakeWorkspaceTaskRemote {
    fn read_queue_summary_bundle(
        &mut self,
        _repo_name: &str,
        _status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        if let Some(err) = self.queue_summary_error.as_ref() {
            return Err(TaskWorkflowHttpClientError::Remote(err.clone()));
        }
        self.queue_summary_bundle.clone().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("missing queue summary bundle".to_string())
        })
    }
}

impl TaskWorkflowRemoteTaskCreator for FakeWorkspaceTaskRemote {
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.task_create_repository_present
            .push(self.repository.is_some());
        let resolved_task_id = task_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("RCT-{}", self.tasks.len() + 1));
        let returned_task_id = self
            .created_task_id_override
            .clone()
            .unwrap_or_else(|| resolved_task_id.clone());
        let task = json!({
            "repo_name": repo_name,
            "task_id": returned_task_id,
            "title": title,
            "intent": intent,
            "status": "open",
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        });
        self.tasks.insert(returned_task_id, task.clone());
        Ok(task)
    }
}

impl TaskWorkflowRemoteChangeCreator for FakeWorkspaceTaskRemote {
    fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let resolved_change_id = change_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("RCC-{task_id}"));
        let change = json!({
            "repo_name": repo_name,
            "change_id": resolved_change_id,
            "task_id": task_id,
            "title": title,
            "base_line": base_line,
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": forked_from_line,
            "status": "draft"
        });
        self.changes.push(change.clone());
        Ok(change)
    }
}

impl TaskWorkflowRemoteChangeLister for FakeWorkspaceTaskRemote {
    fn list_changes(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.changes.clone())
    }
}

impl TaskWorkflowQueueChangeLister for FakeWorkspaceTaskRemote {
    fn list_changes(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.changes.clone())
    }
}

impl TaskWorkflowRemoteChangeDetailReader for FakeWorkspaceTaskRemote {
    fn get_change_detail(
        &mut self,
        _change_ref: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.detail
            .clone()
            .ok_or_else(|| TaskWorkflowHttpClientError::Remote("missing detail".to_string()))
    }
}

impl TaskWorkflowRemoteChangeReader for FakeWorkspaceTaskRemote {
    fn get_change(
        &mut self,
        change_ref: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.changes
            .iter()
            .find(|row| string_field(row, "change_id").as_deref() == Some(change_ref))
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET change {change_ref} failed: 404 Unknown change"
                ))
            })
    }
}

impl TaskWorkflowRemoteChangeCloser for FakeWorkspaceTaskRemote {
    fn close_change(
        &mut self,
        change_ref: &str,
        status: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let change = self
            .changes
            .iter_mut()
            .find(|row| string_field(row, "change_id").as_deref() == Some(change_ref))
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "POST close change {change_ref} failed: 404 Unknown change"
                ))
            })?;
        let obj = change.as_object_mut().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("change row must be an object".to_string())
        })?;
        obj.insert("status".to_string(), JsonValue::String(status.to_string()));
        Ok(JsonValue::Object(obj.clone()))
    }
}

impl TaskWorkflowLineHeadUpdater for FakeWorkspaceTaskRemote {
    fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let line_index = self
            .lines
            .iter()
            .position(|line| string_field(line, "line_name").as_deref() == Some(line_name));
        let Some(line_index) = line_index else {
            if expected_head_snapshot_id.is_some() {
                return Err(TaskWorkflowHttpClientError::Remote(format!(
                    "Remote line {line_name} missing while expected head was {expected_head_snapshot_id:?}"
                )));
            }
            let line = json!({
                "repo_name": repo_name,
                "line_name": line_name,
                "status": "active",
                "head_snapshot_id": head_snapshot_id,
            });
            self.lines.push(line.clone());
            return Ok(line);
        };
        let current_head_snapshot_id = string_field(&self.lines[line_index], "head_snapshot_id");
        if current_head_snapshot_id.as_deref() != expected_head_snapshot_id {
            return Err(TaskWorkflowHttpClientError::Remote(format!(
                "Remote line {line_name} expected head {expected_head_snapshot_id:?} but found {current_head_snapshot_id:?}"
            )));
        }
        let line = self.lines[line_index].as_object_mut().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("line row must be an object".to_string())
        })?;
        line.insert(
            "repo_name".to_string(),
            JsonValue::String(repo_name.to_string()),
        );
        line.insert(
            "line_name".to_string(),
            JsonValue::String(line_name.to_string()),
        );
        line.insert(
            "head_snapshot_id".to_string(),
            head_snapshot_id
                .map(|value| JsonValue::String(value.to_string()))
                .unwrap_or(JsonValue::Null),
        );
        Ok(JsonValue::Object(line.clone()))
    }
}

impl TaskWorkflowLineCloser for FakeWorkspaceTaskRemote {
    fn close_line(
        &mut self,
        _repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let line = self
            .lines
            .iter_mut()
            .find(|line| string_field(line, "line_name").as_deref() == Some(line_name))
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "POST close line {line_name} failed: 404 Unknown line"
                ))
            })?;
        let obj = line.as_object_mut().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote("line row must be an object".to_string())
        })?;
        obj.insert("status".to_string(), JsonValue::String(status.to_string()));
        Ok(JsonValue::Object(obj.clone()))
    }
}

impl TaskWorkflowZstdPackUploader for FakeWorkspaceTaskRemote {}

impl TaskWorkflowSnapshotMetadataReader for FakeWorkspaceTaskRemote {
    fn get_remote_snapshot(
        &mut self,
        _repo_name: &str,
        snapshot_id: &str,
        _include_content: bool,
        _path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.remote_snapshots
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET snapshot {snapshot_id} failed: 404 Unknown snapshot"
                ))
            })
    }
}

impl TaskWorkflowZstdPackReader for FakeWorkspaceTaskRemote {}

impl TaskWorkflowSnapshotExistenceReader for FakeWorkspaceTaskRemote {
    fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let mut present = Vec::new();
        let mut missing = Vec::new();
        for snapshot_id in snapshot_ids {
            if self.remote_snapshots.contains_key(snapshot_id) {
                present.push(JsonValue::String(snapshot_id.clone()));
            } else {
                missing.push(JsonValue::String(snapshot_id.clone()));
            }
        }
        Ok(json!({
            "repo_name": repo_name,
            "requested": snapshot_ids,
            "present": present,
            "missing": missing,
        }))
    }
}

fn write_runtime_config(root: &Path, config_json: &str) {
    fs::create_dir_all(root.join(".ait")).expect("config dir");
    fs::write(root.join(".ait/config.json"), config_json).expect("config");
}

fn set_runtime_repository_index(root: &Path, repository_index: u32) {
    let config_path = root.join(".ait/config.json");
    let mut config = fs::read_to_string(&config_path)
        .expect("read config")
        .parse::<JsonValue>()
        .expect("parse config");
    config
        .as_object_mut()
        .expect("config object")
        .insert("repository_index".to_string(), json!(repository_index));
    fs::write(config_path, config.to_string()).expect("write repository index");
}

fn write_remote_config(root: &Path, base_url: &str) {
    fs::create_dir_all(root.join(".ait")).expect("ait dir");
    let config_path = root.join(".ait/config.json");
    let mut config = fs::read_to_string(&config_path)
        .expect("read config")
        .parse::<JsonValue>()
        .expect("parse config");
    let object = config.as_object_mut().expect("config object");
    object.insert("default_remote".to_string(), json!("origin"));
    object.insert(
        "remotes".to_string(),
        json!({
            "origin": {
                "remote_id": 1,
                "url": base_url,
                "repo_name": "fixture-ait",
                "created_at": "2026-06-20T00:00:00Z"
            }
        }),
    );
    fs::write(config_path, config.to_string()).expect("write remote config");
}

#[test]
fn task_bootstrap_reuses_corrected_task_feature_line_without_lifetime_inference() {
    let repo_tmp = tempdir().expect("repo tempdir");
    init_repo(&InitRequest {
        root: repo_tmp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_tmp.path()).expect("repo runtime");
    let lines = repo.line_store().expect("Line store");
    let existing = create_local_line_with_line_store(
        &lines,
        "feature/rct-line-reuse",
        None,
        "2026-07-24T00:00:00Z",
    )
    .expect("seed corrected task feature Line");
    let worktree_tmp = tempdir().expect("worktree tempdir");
    let task = json!({
        "task_id": "RCT-LINE-REUSE",
        "created_at": "2026-07-25T00:00:00Z",
    });
    let change = json!({
        "change_id": "C-01",
        "task_id": "RCT-LINE-REUSE",
        "base_line": "main",
    });

    let result = task_start_bootstrap(
        &repo,
        TaskStartBootstrapRequest {
            task: &task,
            change: Some(&change),
            title_hint: "Reuse corrected Line",
            intent_hint: "Trust the corrected Task-derived Line mapping",
            base_line_name: "main",
            local: true,
            remote_name: None,
            worktree_name: "rct-line-reuse",
            worktree_path: worktree_tmp.path().to_str().expect("worktree path"),
            worktree_alias_path: None,
            worktree_root_source: None,
            worktree_fallback_reason: None,
            worktree_default_line: None,
            worktree_seed_snapshot_id: None,
            worktree_seed_snapshot_total_bytes: None,
            worktree_main_seed_ram_max_bytes: None,
        },
    )
    .expect("bootstrap with corrected task feature Line");

    assert_eq!(
        result["worktree"]["registered_line_name"],
        json!("feature/rct-line-reuse")
    );
    let reused = local_line_row(&repo, "feature/rct-line-reuse").expect("reused Line");
    assert_eq!(reused["line_id"], existing["line_id"]);
}

#[test]
fn task_bootstrap_checks_nonempty_target_before_creating_feature_line() {
    let repo_tmp = tempdir().expect("repo tempdir");
    init_repo(&InitRequest {
        root: repo_tmp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_tmp.path()).expect("repo runtime");
    let worktree_tmp = tempdir().expect("worktree tempdir");
    fs::write(worktree_tmp.path().join("stale.lock"), b"").expect("stale path marker");
    let task = json!({
        "task_id": "RCT-PATH-COLLISION",
        "created_at": "2026-07-25T00:00:00Z",
    });
    let change = json!({
        "change_id": "C-01",
        "task_id": "RCT-PATH-COLLISION",
        "base_line": "main",
    });

    let error = task_start_bootstrap(
        &repo,
        TaskStartBootstrapRequest {
            task: &task,
            change: Some(&change),
            title_hint: "Path collision",
            intent_hint: "Do not leave an orphan Line",
            base_line_name: "main",
            local: true,
            remote_name: None,
            worktree_name: "rct-path-collision",
            worktree_path: worktree_tmp.path().to_str().expect("worktree path"),
            worktree_alias_path: None,
            worktree_root_source: None,
            worktree_fallback_reason: None,
            worktree_default_line: None,
            worktree_seed_snapshot_id: None,
            worktree_seed_snapshot_total_bytes: None,
            worktree_main_seed_ram_max_bytes: None,
        },
    )
    .expect_err("nonempty worktree target must fail");
    assert!(error.contains("Worktree path must be empty"));
    assert!(local_line_row(&repo, "feature/rct-path-collision").is_err());
}

#[test]
fn worktree_recover_task_dry_run_validates_remote_identity_without_local_mutation() {
    let repo_tmp = tempdir().expect("repo tempdir");
    init_repo(&InitRequest {
        root: repo_tmp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_tmp.path()).expect("repo runtime");
    let runtime_tmp = tempdir().expect("runtime tempdir");
    let mut remote = FakeWorkspaceTaskRemote::default();
    remote.tasks.insert(
        "RCT-RECOVER".to_string(),
        json!({
            "task_id": "RCT-RECOVER",
            "repo_name": "fixture-ait",
            "status": "active",
            "title": "Recover task worktree",
            "intent": "Recreate only local authoring state for an existing remote task"
        }),
    );
    remote.changes.push(json!({
        "change_id": "RCT-RECOVER/C-01",
        "task_id": "RCT-RECOVER",
        "repo_name": "fixture-ait",
        "status": "draft",
        "base_line": "main"
    }));
    let debug_probe = json!({
        "platform": "linux",
        "linux_detected_memory_roots": [runtime_tmp.path().to_string_lossy().to_string()],
    });

    let result = worktree_recover_task_with_task_remote(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "RCT-RECOVER",
        "RCT-RECOVER/C-01",
        true,
        Some(&debug_probe),
    )
    .expect("plan recovery");

    assert_eq!(result["status"], json!("recovery_planned"));
    assert_eq!(result["task_id"], json!("RCT-RECOVER"));
    assert_eq!(result["change_id"], json!("RCT-RECOVER/C-01"));
    assert_eq!(result["name"], json!("rct-recover"));
    assert_eq!(result["dry_run"], json!(true));
    let planned_path = PathBuf::from(result["path"].as_str().expect("planned path"));
    assert!(!planned_path.exists());
    assert!(!repo
        .authoritative_repo_root()
        .join(".ait/worktrees/rct-recover.json")
        .exists());

    fs::create_dir_all(&planned_path).expect("seed stale recovery path");
    fs::write(planned_path.join("stale.lock"), b"").expect("seed stale recovery marker");
    let path_error = worktree_recover_task_with_task_remote(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "RCT-RECOVER",
        "RCT-RECOVER/C-01",
        true,
        Some(&debug_probe),
    )
    .expect_err("dry-run must reject stale worktree materialization");
    assert!(path_error.contains("Worktree path must be empty"));
    assert!(local_line_row(&repo, "feature/rct-recover").is_err());
}

mod change_flow_ports;
mod line_queue_worktree_ports;
mod remote_sync_ports;
mod snapshot_query_ports;
mod stash_ports;
mod task_workspace_ports;
mod workflow_closeout_ports;
mod workflow_ready_worktree;
mod workflow_tier;
mod workspace_ci_contracts;
