use crate::remote_repository::read_remote_repository_authority;
#[cfg(test)]
use crate::runtime::create_binary_test_snapshot as create_local_snapshot;
use crate::runtime::{
    RemoteRow, RepoRuntime, REMOTE_SYNC_BINARY_DB_WRITE_LAYOUT, SNAPSHOT_BINARY_DB_WRITE_LAYOUT,
};
use crate::task_worktree_layout;
use crate::workspace_lock::run_locked_workspace_command;
use ait_core::change_json::ChangeJson;
use ait_core::change_store::{
    list_changes_for_task_with_change_store, list_changes_with_change_store,
    reopen_change_as_draft_with_change_store, ChangeStore,
};
use ait_core::content_store::TreePackStore;
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::line_store::LineStore;
use ait_core::local_snapshot::{
    LocalSnapshotTreeReadStore, LocalSnapshotWriteStore, SnapshotFileRow,
    SnapshotTreeManifestIndex, SnapshotTreeManifestRow,
};
use ait_core::object_diff::{
    snapshot_diff_from_readers, workspace_diff_from_entries, BlobReader, SnapshotReader,
    WorkspaceDiffEntry,
};
use ait_core::plan_filesystem::{
    is_markdown_artifact_path, list_visible_workspace_entries, list_visible_workspace_paths,
    parse_workspace_ignore_matcher, path_is_projected_out_for_workspace,
    workspace_relative_path_is_ignored_with_matcher, WorkspaceIgnoreMatcher,
};
use ait_core::plan_http_client::PlanHttpClientConfig;
use ait_core::plan_store::{
    get_plan_revision_by_id_with_plan_store,
    resolve_reconciled_plan_publish_linkage_with_plan_store,
};
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
#[cfg(test)]
use ait_core::remote_sync_backend::RemoteSyncSnapshotInventory;
#[cfg(test)]
use ait_core::remote_sync_backend::REMOTE_SYNC_CAPABILITY_SNAPSHOT_DAG_V2;
use ait_core::remote_sync_backend::{
    remote_sync_backend_payload, require_snapshot_dag_remote_capability, RemoteSyncBackendKind,
    RemoteSyncBackendNegotiation, RemoteSyncCapabilities, RemoteSyncInventoryDiff,
    REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK, REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD,
};
use ait_core::repository_pack_json::{
    ZstdBulkCommitRequest, ZstdBulkCommitResponse, ZstdBulkCommitResponseJson, ZstdBulkLineUpdate,
    ZstdBulkObjectPackRow, ZstdBulkPlanRequest, ZstdBulkPlanResponse, ZstdBulkPlanResponseJson,
    ZstdBulkTreePackRow, ZstdImportManifestPayload, ZstdPullManifestPayload,
    ZstdPullManifestRequest, ZSTD_IMPORT_MANIFEST_CONTRACT_NAME,
    ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME,
};
#[cfg(test)]
use ait_core::repository_pack_json::{
    ZstdBulkCommitRequestJson, ZstdBulkPlanRequestJson, ZstdBulkRemoteLine, ZstdPackUploadResponse,
};
use ait_core::snapshot_dag::{
    snapshot_ancestor_closure, snapshot_ancestor_closure_from_parent_map,
    snapshot_ancestor_distance_with_cache, snapshot_descendant_closure,
    snapshot_first_parent_chain, snapshot_is_ancestor, snapshot_merge_bases,
    topological_snapshot_order, SnapshotAncestorDistanceCache, SnapshotDagLimitMode,
    SnapshotDagLimits, SnapshotDagTraversal, SnapshotParentMode,
};
use ait_core::snapshot_store::{
    normalize_snapshot_parent_set, snapshot_by_id_with_snapshot_store, SnapshotRecord,
    SnapshotStore,
};
use ait_core::tag_store::{FilesystemTagStore, TagStore};
use ait_core::task_lifecycle::build_task_audit_verdict_payload;
#[cfg(test)]
use ait_core::task_store::has_tasks_with_task_store;
use ait_core::task_store::{restart_task_with_task_store, TaskStore};
use ait_core::task_workflow_http_adapter::TaskWorkflowSnapshotMetadataReader;
use ait_core::task_workflow_http_adapter::{
    HttpTaskRemote, HttpWorkflowCloseoutRemote, TaskWorkflowAttestationReader,
    TaskWorkflowAttestationWriter, TaskWorkflowHistoryPromotionPreparer, TaskWorkflowLineCloser,
    TaskWorkflowLineDeleter, TaskWorkflowLineHeadUpdater, TaskWorkflowLineLister,
    TaskWorkflowLineReader, TaskWorkflowLineRenamer, TaskWorkflowLineagePayloadBuilder,
    TaskWorkflowPatchsetCiRunner, TaskWorkflowPatchsetLister, TaskWorkflowPatchsetPublisher,
    TaskWorkflowPatchsetReader, TaskWorkflowPatchsetSelector, TaskWorkflowPolicyEvaluator,
    TaskWorkflowPolicyReader, TaskWorkflowPolicyWaiverCreator, TaskWorkflowRemoteChangeCloser,
    TaskWorkflowRemoteChangeCreator, TaskWorkflowRemoteChangeDetailReader,
    TaskWorkflowRemoteChangeLister, TaskWorkflowRemoteChangeReader,
    TaskWorkflowRemoteTaskAuditReader, TaskWorkflowRemoteTaskCreator, TaskWorkflowRemoteTaskLister,
    TaskWorkflowRemoteTaskReader, TaskWorkflowRepositoryReader, TaskWorkflowReviewLister,
    TaskWorkflowReviewRecorder, TaskWorkflowReviewRequester, TaskWorkflowSnapshotExistenceReader,
    TaskWorkflowZstdPackReader, TaskWorkflowZstdPackUploader,
};
use ait_core::task_workflow_remote_traits::{
    TaskWorkflowLandReader, TaskWorkflowLandRetryer, TaskWorkflowLandSubmitter,
    TaskWorkflowPatchsetCiStatusReader, TaskWorkflowRemoteTaskCloser,
    TaskWorkflowRemoteTaskRestarter,
};
use ait_core::task_workflow_store::{
    TaskWorkflowChangeCloser, TaskWorkflowChangeCreator, TaskWorkflowChangeLander,
    TaskWorkflowChangeLister, TaskWorkflowChangePublisher, TaskWorkflowChangeReader,
    TaskWorkflowTaskCloser, TaskWorkflowTaskCreator, TaskWorkflowTaskLister,
    TaskWorkflowTaskPublisher, TaskWorkflowTaskReader,
};
use ait_core::time_identity::build_plan_workflow_id_payload_json;
use ait_core::workflow_closeout_facts::{
    workflow_land_full_facts, workflow_land_phase_facts, workflow_landed_facts,
    workflow_ready_facts,
};
use ait_core::workflow_closeout_read_model::{
    project_workflow_land_full_read_model, project_workflow_land_phase_read_model,
    project_workflow_landed_read_model, project_workflow_ready_read_model,
};
use ait_core::workflow_closeout_remote::workflow_remote_action_mutation_receipts;
use ait_core::workflow_closeout_views::{
    workflow_applied_action_summary, workflow_apply_phase_payload,
};
use ait_core::workspace_hash_cache::{
    load_workspace_hash_cache, workspace_file_fingerprint,
    workspace_file_fingerprint_from_visible_metadata, workspace_hash_cache_entry,
    write_workspace_hash_cache, WorkspaceFileFingerprint, WorkspaceHashCacheLoad,
};
use chrono::{DateTime, Duration as ChronoDuration, FixedOffset, SecondsFormat, Utc};
use fs2::FileExt;
#[cfg(target_os = "linux")]
use libc;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
#[cfg(target_os = "macos")]
use std::os::raw::{c_char, c_int};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

mod change_flow;
mod change_identity;
mod foundation;
mod git_interop;
mod line;
mod line_merge;
mod plan_checklist_closeout;
mod queue;
mod reconciliation;
mod remote_sync;
mod review_support;
mod runtime_support;
mod snapshot;
mod sprint_card_retention;
mod stash;
mod status_cache;
mod task;
mod task_start_from;
mod workflow;
mod workflow_tier;
mod workspace;
mod worktree;

pub use change_flow::{
    attest_put, attest_show, change_close, change_create, change_list, change_publish,
    change_replay, change_revert, change_show, land_retry, land_show, land_submit,
    patchset_ci_status, patchset_list, patchset_publish, patchset_publish_explicit,
    patchset_rerun_ci, patchset_select, patchset_show, policy_eval, policy_show, policy_waive,
    review_code_submit, review_code_template, review_record, review_request, review_show,
    review_task_approve, review_team_approve, task_close, task_complete, task_publish,
    task_restart,
};
pub use foundation::{ensure_status_manifest, TaskStartBootstrapRequest};
pub use git_interop::{git_export, git_import, git_mirror};
pub use line::{
    line_archive, line_cleanup, line_cleanup_candidates, line_create, line_delete, line_list,
    line_rename, line_set_head, line_show, line_switch, repo_status,
};
pub(in crate::primitives) use line_merge::guard_no_active_line_merge;
pub use line_merge::line_merge;
pub use queue::queue_summary;
pub use reconciliation::{
    workflow_reconcile_apply, workflow_reconcile_automatic,
    workflow_reconcile_automatic_best_effort, workflow_reconcile_inventory,
    workflow_reconciliation_cached_summary, AutomaticReconciliationScope,
    AutomaticReconciliationTrigger,
};
pub(crate) use remote_sync::{
    hydrate_remote_snapshot_boundary_for_repo, remote_sync_snapshot_content_complete_for_repo,
};
pub use remote_sync::{pull, push, upload_snapshot_chain};
pub use snapshot::{
    blob_ensure_bytes, blob_read_bytes, snapshot_ancestry, snapshot_chain, snapshot_diff,
    snapshot_exists, snapshot_is_ancestor_query, snapshot_list, snapshot_merge_base_query,
    snapshot_replay, snapshot_revert, snapshot_show, SnapshotAncestryDirection,
    DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_LIMIT, DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_MAX_DEPTH,
};
pub use stash::{stash_apply, stash_drop, stash_list, stash_pop, stash_save, stash_show};
pub use task::{task_audit, task_create, task_list, task_show, task_tokens};
pub use task_start_from::task_start_from_with_progress;
pub use workflow::{
    task_land_apply, task_land_apply_scoped, task_land_payload, task_land_payload_scoped,
    workflow_completed_local_batch_retired_error, workflow_land_apply,
    workflow_land_completed_local_apply, workflow_land_completed_local_payload,
    workflow_land_local, workflow_land_payload, workflow_ready_apply, workflow_ready_payload,
};
pub use workflow_tier::{snapshot_create_quick, workflow_tier_payload};
pub use workspace::{
    snapshot_create, snapshot_create_explicit, workflow_workspace_status, workspace_delta,
    workspace_dirty_diff, workspace_restore, workspace_restore_paths,
};
pub use worktree::{
    task_ensure_main_seed_mirror, task_resolve_main_seed_mirror_location,
    task_resolve_worktree_location, task_start, task_start_bootstrap, task_start_with_progress,
    worktree_abort_rebase, worktree_bind_existing, worktree_cleanup, worktree_cleanup_candidates,
    worktree_continue_rebase, worktree_doctor, worktree_get, worktree_list,
    worktree_preview_rebase, worktree_prune_stale, worktree_rebase, worktree_recover_task,
    worktree_recreate, worktree_remove, worktree_restore, worktree_restore_owned_head,
    worktree_status, worktree_sync, worktree_sync_all, worktree_touch_usage,
};

use change_flow::published_local_task_plan_linkage;
use change_identity::*;
use foundation::*;
use line::list_remote_names;
use remote_sync::{set_or_create_local_line_head, sync_patchset_revision_snapshot};
use review_support::*;
use runtime_support::*;
use status_cache::*;
use task::*;
use workspace::*;

pub(crate) fn resolved_snapshot_ownership_rows(
    repo: &RepoRuntime,
    snapshot_ids: &[String],
) -> Result<Vec<JsonValue>, String> {
    workspace::snapshot_ownership_rows(repo, snapshot_ids)
}
use worktree::*;

#[cfg(test)]
mod tests;
