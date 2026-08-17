mod backend;
mod local_store;
mod sync;

#[cfg(test)]
pub(super) use backend::RemoteSyncBackend;
#[cfg(test)]
pub(super) use local_store::{
    RemoteSyncLocalInventoryMetadata, RemoteSyncLocalInventorySource,
    RemoteSyncLocalSnapshotParent, RemoteSyncLocalSnapshotSource, RemoteSyncLocalStoreContext,
    RemoteSyncZstdLocalPlanSource, ZstdBulkLocalPack, ZstdBulkLocalPlan,
};
pub(crate) use sync::{
    hydrate_remote_snapshot_boundary_for_repo, remote_sync_snapshot_content_complete_for_repo,
};
pub(super) use sync::{
    hydrate_remote_snapshot_chain_with_task_remote_and_capabilities, set_or_create_local_line_head,
    sync_patchset_revision_snapshot, sync_patchset_revision_snapshot_with_task_remote,
};
pub use sync::{pull, push, upload_snapshot_chain};

#[cfg(test)]
pub(super) use sync::{
    build_zstd_bulk_local_plan, build_zstd_bulk_local_plan_with_source,
    hydrate_remote_snapshot_boundary_with_task_remote_and_capabilities,
    local_remote_sync_inventory_for_snapshots,
    local_remote_sync_inventory_for_snapshots_with_source,
    local_repo_snapshot_ids_topological_with_source, ordered_object_pack_metadata,
    ordered_tree_pack_metadata, pull_line_with_task_remote_and_capabilities,
    pull_with_remote_sync_backend, push_line_to_remote_with_task_remote_and_capabilities,
    push_with_remote_sync_backend, remote_repository_authority_http_config,
    remote_sync_line_head_with_task_remote, remote_sync_line_read_with_task_remote,
    remote_sync_line_update_with_task_remote, remote_sync_present_snapshot_ids_with_task_remote,
    remote_sync_snapshot_metadata_read_with_task_remote,
    require_snapshot_dag_upload_capability_with_source,
    sync_patchset_revision_snapshot_with_remote_sync_backend,
    upload_snapshot_chain_to_remote_with_task_remote_and_capabilities,
    upload_snapshot_chain_with_remote_sync_backend, validate_zstd_object_pack_locator_coverage,
    zstd_bulk_commit_local_plan,
};
