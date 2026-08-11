use crate::json_support::JsonValue as Value;

use crate::json_support::JsonCodec;
use crate::plan_http_client::PlanHttpClientError;
use crate::repository_pack_json::{
    ZstdBulkCommitRequest, ZstdBulkCommitResponse, ZstdBulkPlanRequest, ZstdBulkPlanResponse,
    ZstdImportManifestPayload, ZstdPackUploadResponse, ZstdPullManifestPayload,
    ZstdPullManifestRequest,
};
pub use crate::task_workflow_remote_traits::{
    TaskWorkflowActionMutationReceiptsBuilder, TaskWorkflowAtomicTaskLandSubmitter,
    TaskWorkflowAttestationReader, TaskWorkflowAttestationRemote, TaskWorkflowAttestationWriter,
    TaskWorkflowChangeRemote, TaskWorkflowCloseoutRemote, TaskWorkflowHistoryPromotionPreparer,
    TaskWorkflowHttpClientCloser, TaskWorkflowHttpClientConfig, TaskWorkflowHttpClientError,
    TaskWorkflowHttpClientInspector, TaskWorkflowHttpClientManager, TaskWorkflowHttpClientRemote,
    TaskWorkflowHttpClientResult, TaskWorkflowHttpClientStats, TaskWorkflowLandReader,
    TaskWorkflowLandRemote, TaskWorkflowLandRetryer, TaskWorkflowLandSubmitter,
    TaskWorkflowLineCloser, TaskWorkflowLineDeleter, TaskWorkflowLineHeadUpdater,
    TaskWorkflowLineLister, TaskWorkflowLineReader, TaskWorkflowLineRemote,
    TaskWorkflowLineRenamer, TaskWorkflowLineagePayloadBuilder, TaskWorkflowMutationReceiptBuilder,
    TaskWorkflowMutationReceiptRemote, TaskWorkflowPatchsetCiRemote, TaskWorkflowPatchsetCiRunner,
    TaskWorkflowPatchsetCiStatusReader, TaskWorkflowPatchsetLister, TaskWorkflowPatchsetPublisher,
    TaskWorkflowPatchsetReader, TaskWorkflowPatchsetRemote, TaskWorkflowPatchsetSelector,
    TaskWorkflowPolicyEvaluator, TaskWorkflowPolicyReader, TaskWorkflowPolicyRemote,
    TaskWorkflowPolicyWaiverCreator, TaskWorkflowQueueChangeLister, TaskWorkflowQueueRemote,
    TaskWorkflowQueueSummaryBundleReader, TaskWorkflowRemoteChangeCloser,
    TaskWorkflowRemoteChangeCreator, TaskWorkflowRemoteChangeDetailReader,
    TaskWorkflowRemoteChangeLister, TaskWorkflowRemoteChangeReader,
    TaskWorkflowRemoteTaskAuditReader, TaskWorkflowRemoteTaskCloser, TaskWorkflowRemoteTaskCreator,
    TaskWorkflowRemoteTaskLister, TaskWorkflowRemoteTaskReader, TaskWorkflowRemoteTaskRestarter,
    TaskWorkflowRepoJobLister, TaskWorkflowRepositoryEnsurer, TaskWorkflowRepositoryReader,
    TaskWorkflowRepositoryRemote, TaskWorkflowReviewLister, TaskWorkflowReviewRecorder,
    TaskWorkflowReviewRemote, TaskWorkflowReviewRequester, TaskWorkflowReviewerInboxReader,
    TaskWorkflowSnapshotExistenceReader, TaskWorkflowSnapshotMetadataReader,
    TaskWorkflowSnapshotRemote, TaskWorkflowTaskLifecycleRemote, TaskWorkflowTaskQueueReader,
    TaskWorkflowTaskRecordRemote, TaskWorkflowTaskRemote, TaskWorkflowZstdPackReader,
    TaskWorkflowZstdPackUploader,
};

mod closeout_remote;
mod helpers;
mod task_remote;

pub use self::closeout_remote::HttpWorkflowCloseoutRemote;
pub use self::task_remote::HttpTaskRemote;

pub struct TaskWorkflowHttpJson<S> {
    store: S,
}

impl<S> TaskWorkflowHttpJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn normalize_compatibility_payload_json(
        &self,
        payload_json: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.parse_object_payload(payload_json, "task workflow HTTP compatibility")
    }

    pub fn normalize_readiness_payload_json(
        &self,
        payload_json: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.parse_object_payload(payload_json, "task workflow HTTP readiness")
    }

    fn parse_object_payload(
        &self,
        payload_json: &str,
        label: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let _ = &self.store;
        let payload = JsonCodec::parse_value_with_error_prefix(
            payload_json,
            &format!("{label} payload invalid JSON"),
        )
        .map_err(|err| PlanHttpClientError::Remote(err.to_string()))?;
        if payload.is_object() {
            Ok(payload)
        } else {
            Err(PlanHttpClientError::Invalid(format!(
                "{label} payload must be an object"
            )))
        }
    }
}

impl TaskWorkflowHttpJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

pub fn normalize_task_workflow_http_compatibility_payload_json(
    payload_json: &str,
) -> TaskWorkflowHttpClientResult<Value> {
    TaskWorkflowHttpJson::stateless().normalize_compatibility_payload_json(payload_json)
}

pub fn normalize_task_workflow_http_readiness_payload_json(
    payload_json: &str,
) -> TaskWorkflowHttpClientResult<Value> {
    TaskWorkflowHttpJson::stateless().normalize_readiness_payload_json(payload_json)
}

pub fn inspect_client_with_task_workflow_task_remote<R>(remote: &R) -> TaskWorkflowHttpClientStats
where
    R: TaskWorkflowHttpClientInspector + ?Sized,
{
    remote.inspect_client()
}

pub fn close_client_with_task_workflow_task_remote<R>(remote: &mut R) -> TaskWorkflowHttpClientStats
where
    R: TaskWorkflowHttpClientCloser + ?Sized,
{
    remote.close_client()
}

pub fn ensure_repository_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    default_line: &str,
    policy: Option<&Value>,
    id_namespace_prefix: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRepositoryEnsurer + ?Sized,
{
    remote.ensure_repository(repo_name, default_line, policy, id_namespace_prefix)
}

pub fn get_repository_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRepositoryReader + ?Sized,
{
    remote.get_repository(repo_name)
}

pub fn change_lineage_payload_with_task_workflow_task_remote<R>(
    remote: &R,
    base_line: &str,
    line_row: Option<&Value>,
) -> Result<Value, String>
where
    R: TaskWorkflowLineagePayloadBuilder + ?Sized,
{
    remote.change_lineage_payload(base_line, line_row)
}

pub fn get_line_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    line_name: &str,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    remote.get_line(repo_name, line_name)
}

pub fn list_lines_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
) -> TaskWorkflowHttpClientResult<Vec<Value>>
where
    R: TaskWorkflowLineLister + ?Sized,
{
    remote.list_lines(repo_name)
}

#[allow(clippy::too_many_arguments)]
pub fn rename_remote_line_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    old_line_name: &str,
    new_line_name: &str,
    expected_line_id: &str,
    expected_head_snapshot_id: Option<&str>,
    idempotency_key: &str,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowLineRenamer + ?Sized,
{
    remote.rename_remote_line(
        repo_name,
        old_line_name,
        new_line_name,
        expected_line_id,
        expected_head_snapshot_id,
        idempotency_key,
    )
}

pub fn delete_remote_line_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    line_name: &str,
    expected_line_id: &str,
    expected_head_snapshot_id: Option<&str>,
    idempotency_key: &str,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowLineDeleter + ?Sized,
{
    remote.delete_remote_line(
        repo_name,
        line_name,
        expected_line_id,
        expected_head_snapshot_id,
        idempotency_key,
    )
}

pub fn get_task_with_task_workflow_task_remote<R>(
    remote: &mut R,
    task_id: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteTaskReader + ?Sized,
{
    remote.get_task(task_id, repo_name)
}

pub fn list_tasks_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
) -> TaskWorkflowHttpClientResult<Vec<Value>>
where
    R: TaskWorkflowRemoteTaskLister + ?Sized,
{
    remote.list_tasks(repo_name)
}

pub fn read_task_audit_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    task_id: &str,
    target_line: &str,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteTaskAuditReader + ?Sized,
{
    remote.read_task_audit(repo_name, task_id, target_line)
}

pub fn read_task_queue_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    status: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowTaskQueueReader + ?Sized,
{
    remote.read_task_queue(repo_name, status)
}

pub fn read_reviewer_inbox_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowReviewerInboxReader + ?Sized,
{
    remote.read_reviewer_inbox(repo_name)
}

pub fn read_queue_summary_bundle_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    status: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowQueueSummaryBundleReader + ?Sized,
{
    remote.read_queue_summary_bundle(repo_name, status)
}

#[allow(clippy::too_many_arguments)]
pub fn create_task_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    title: &str,
    intent: &str,
    task_id: Option<&str>,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteTaskCreator + ?Sized,
{
    remote.create_task(
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
pub fn create_change_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    task_id: &str,
    title: &str,
    base_line: &str,
    change_id: Option<&str>,
    fork_snapshot_id: Option<&str>,
    forked_from_line: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteChangeCreator + ?Sized,
{
    remote.create_change(
        repo_name,
        task_id,
        title,
        base_line,
        change_id,
        fork_snapshot_id,
        forked_from_line,
    )
}

pub fn list_changes_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
) -> TaskWorkflowHttpClientResult<Vec<Value>>
where
    R: TaskWorkflowRemoteChangeLister + ?Sized,
{
    remote.list_changes(repo_name)
}

pub fn get_change_detail_with_task_workflow_task_remote<R>(
    remote: &mut R,
    change_ref: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteChangeDetailReader + ?Sized,
{
    remote.get_change_detail(change_ref, repo_name)
}

pub fn get_change_with_task_workflow_task_remote<R>(
    remote: &mut R,
    change_ref: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteChangeReader + ?Sized,
{
    remote.get_change(change_ref, repo_name)
}

pub fn close_change_with_task_workflow_task_remote<R>(
    remote: &mut R,
    change_ref: &str,
    status: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteChangeCloser + ?Sized,
{
    remote.close_change(change_ref, status, repo_name)
}

pub fn update_remote_line_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    line_name: &str,
    head_snapshot_id: Option<&str>,
    expected_head_snapshot_id: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowLineHeadUpdater + ?Sized,
{
    remote.update_remote_line(
        repo_name,
        line_name,
        head_snapshot_id,
        expected_head_snapshot_id,
    )
}

pub fn close_line_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    line_name: &str,
    status: &str,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowLineCloser + ?Sized,
{
    remote.close_line(repo_name, line_name, status)
}

pub fn plan_remote_zstd_bulk_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    request: &ZstdBulkPlanRequest,
) -> TaskWorkflowHttpClientResult<ZstdBulkPlanResponse>
where
    R: TaskWorkflowZstdPackUploader + ?Sized,
{
    remote.plan_remote_zstd_bulk(repo_name, request)
}

pub fn put_remote_zstd_object_pack_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    pack_id: &str,
    pack_bytes: &[u8],
) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse>
where
    R: TaskWorkflowZstdPackUploader + ?Sized,
{
    remote.put_remote_zstd_object_pack(repo_name, pack_id, pack_bytes)
}

pub fn put_remote_zstd_tree_pack_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    pack_id: &str,
    pack_bytes: &[u8],
) -> TaskWorkflowHttpClientResult<ZstdPackUploadResponse>
where
    R: TaskWorkflowZstdPackUploader + ?Sized,
{
    remote.put_remote_zstd_tree_pack(repo_name, pack_id, pack_bytes)
}

pub fn commit_remote_zstd_bulk_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    request: &ZstdBulkCommitRequest,
) -> TaskWorkflowHttpClientResult<ZstdBulkCommitResponse>
where
    R: TaskWorkflowZstdPackUploader + ?Sized,
{
    remote.commit_remote_zstd_bulk(repo_name, request)
}

pub fn get_remote_snapshot_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    snapshot_id: &str,
    include_content: bool,
    path: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowSnapshotMetadataReader + ?Sized,
{
    remote.get_remote_snapshot(repo_name, snapshot_id, include_content, path)
}

pub fn get_remote_zstd_import_manifest_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    snapshot_id: &str,
) -> TaskWorkflowHttpClientResult<ZstdImportManifestPayload>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    remote.get_remote_zstd_import_manifest(repo_name, snapshot_id)
}

pub fn get_remote_zstd_pull_manifest_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    request: &ZstdPullManifestRequest,
) -> TaskWorkflowHttpClientResult<ZstdPullManifestPayload>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    remote.get_remote_zstd_pull_manifest(repo_name, request)
}

pub fn get_remote_zstd_object_pack_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    pack_id: &str,
) -> TaskWorkflowHttpClientResult<Vec<u8>>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    remote.get_remote_zstd_object_pack(repo_name, pack_id)
}

pub fn get_remote_zstd_tree_pack_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    pack_id: &str,
) -> TaskWorkflowHttpClientResult<Vec<u8>>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    remote.get_remote_zstd_tree_pack(repo_name, pack_id)
}

pub fn get_remote_snapshots_existence_with_task_workflow_task_remote<R>(
    remote: &mut R,
    repo_name: &str,
    snapshot_ids: &[String],
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowSnapshotExistenceReader + ?Sized,
{
    remote.get_remote_snapshots_existence(repo_name, snapshot_ids)
}

pub fn inspect_client_with_task_workflow_closeout_remote<R>(
    remote: &R,
) -> TaskWorkflowHttpClientStats
where
    R: TaskWorkflowHttpClientInspector + ?Sized,
{
    remote.inspect_client()
}

pub fn close_client_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
) -> TaskWorkflowHttpClientStats
where
    R: TaskWorkflowHttpClientCloser + ?Sized,
{
    remote.close_client()
}

pub fn mutation_receipt_with_task_workflow_closeout_remote<R>(
    remote: &R,
    action: &str,
    source_action: &str,
    delivery: &str,
    response_recovery: Option<&Value>,
    result: Option<&Value>,
) -> Result<Value, String>
where
    R: TaskWorkflowMutationReceiptBuilder + ?Sized,
{
    remote.mutation_receipt(action, source_action, delivery, response_recovery, result)
}

pub fn action_mutation_receipts_with_task_workflow_closeout_remote<R>(
    remote: &R,
    code: &str,
    result: &Value,
) -> Result<Value, String>
where
    R: TaskWorkflowActionMutationReceiptsBuilder + ?Sized,
{
    remote.action_mutation_receipts(code, result)
}

pub fn prepare_history_promotion_with_task_workflow_remote<R>(
    remote: &mut R,
    repo_name: &str,
    payload: &Value,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowHistoryPromotionPreparer + ?Sized,
{
    remote.prepare_history_promotion(repo_name, payload)
}

pub fn list_patchsets_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    change_id: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Vec<Value>>
where
    R: TaskWorkflowPatchsetLister + ?Sized,
{
    remote.list_patchsets(change_id, repo_name)
}

pub fn get_patchset_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    repo_name: Option<&str>,
    change_ref: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPatchsetReader + ?Sized,
{
    remote.get_patchset(patchset_id, repo_name, change_ref)
}

#[allow(clippy::too_many_arguments)]
pub fn publish_patchset_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    change_id: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    summary: &str,
    author_mode: &str,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPatchsetPublisher + ?Sized,
{
    remote.publish_patchset(
        change_id,
        base_snapshot_id,
        revision_snapshot_id,
        summary,
        author_mode,
        repo_name,
        exact_id,
    )
}

pub fn select_patchset_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    change_id: &str,
    patchset_id: &str,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPatchsetSelector + ?Sized,
{
    remote.select_patchset(change_id, patchset_id, repo_name, exact_id)
}

pub fn run_patchset_ci_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    trigger: &str,
    execution_profile: Option<&str>,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPatchsetCiRunner + ?Sized,
{
    remote.run_patchset_ci(patchset_id, trigger, execution_profile, repo_name, exact_id)
}

#[allow(clippy::too_many_arguments)]
pub fn request_review_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    change_id: &str,
    patchset_id: &str,
    reviewer_groups: &[String],
    note: Option<&str>,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowReviewRequester + ?Sized,
{
    remote.request_review(
        change_id,
        patchset_id,
        reviewer_groups,
        note,
        repo_name,
        exact_id,
    )
}

pub fn read_patchset_ci_status_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    recent_limit: i64,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPatchsetCiStatusReader + ?Sized,
{
    remote.read_patchset_ci_status(patchset_id, recent_limit, repo_name, exact_id)
}

pub fn read_patchset_ci_readiness_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    recent_limit: i64,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPatchsetCiStatusReader + ?Sized,
{
    remote.read_patchset_ci_readiness(patchset_id, recent_limit, repo_name, exact_id)
}

pub fn list_repo_jobs_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    repo_name: &str,
    state: Option<&str>,
    limit: i64,
    diagnostics: bool,
    stale_after_seconds: i64,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRepoJobLister + ?Sized,
{
    remote.list_repo_jobs(repo_name, state, limit, diagnostics, stale_after_seconds)
}

#[allow(clippy::too_many_arguments)]
pub fn record_review_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    change_id: &str,
    patchset_id: &str,
    reviewer: &str,
    action: &str,
    comment: Option<&str>,
    blocking: bool,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowReviewRecorder + ?Sized,
{
    remote.record_review(
        change_id,
        patchset_id,
        reviewer,
        action,
        comment,
        blocking,
        repo_name,
        exact_id,
    )
}

pub fn list_reviews_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    change_id: &str,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowReviewLister + ?Sized,
{
    remote.list_reviews(change_id, repo_name, exact_id)
}

#[allow(clippy::too_many_arguments)]
pub fn put_attestation_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    author_mode: &str,
    evaluation_summary: &Value,
    provenance_summary: &Value,
    detail: &Value,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowAttestationWriter + ?Sized,
{
    remote.put_attestation(
        patchset_id,
        author_mode,
        evaluation_summary,
        provenance_summary,
        detail,
        repo_name,
        exact_id,
    )
}

pub fn get_attestation_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowAttestationReader + ?Sized,
{
    remote.get_attestation(patchset_id, repo_name, exact_id)
}

pub fn evaluate_policy_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPolicyEvaluator + ?Sized,
{
    remote.evaluate_policy(patchset_id, repo_name, exact_id)
}

pub fn get_policy_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPolicyReader + ?Sized,
{
    remote.get_policy(patchset_id, repo_name, exact_id)
}

pub fn create_waiver_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    patchset_id: &str,
    rule_name: &str,
    reason: &str,
    expires_at: Option<&str>,
    repo_name: Option<&str>,
    exact_id: bool,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowPolicyWaiverCreator + ?Sized,
{
    remote.create_waiver(
        patchset_id,
        rule_name,
        reason,
        expires_at,
        repo_name,
        exact_id,
    )
}

pub fn submit_land_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    change_id: &str,
    patchset_id: Option<&str>,
    target_line: &str,
    mode: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowLandSubmitter + ?Sized,
{
    remote.submit_land(change_id, patchset_id, target_line, mode, repo_name)
}

pub fn submit_task_land_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    task_or_change_ref: &str,
    target_line: Option<&str>,
    mode: &str,
    idempotency_key: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowAtomicTaskLandSubmitter + ?Sized,
{
    remote.submit_task_land(
        task_or_change_ref,
        target_line,
        mode,
        idempotency_key,
        repo_name,
    )
}

pub fn get_land_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    submission_id: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowLandReader + ?Sized,
{
    remote.get_land(submission_id, repo_name)
}

pub fn retry_land_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    submission_id: &str,
    reason: Option<&str>,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowLandRetryer + ?Sized,
{
    remote.retry_land(submission_id, reason, repo_name)
}

pub fn close_task_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    task_id: &str,
    status: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteTaskCloser + ?Sized,
{
    remote.close_task(task_id, status, repo_name)
}

pub fn restart_task_with_task_workflow_closeout_remote<R>(
    remote: &mut R,
    task_id: &str,
    repo_name: Option<&str>,
) -> TaskWorkflowHttpClientResult<Value>
where
    R: TaskWorkflowRemoteTaskRestarter + ?Sized,
{
    remote.restart_task(task_id, repo_name)
}

#[cfg(test)]
mod tests;
