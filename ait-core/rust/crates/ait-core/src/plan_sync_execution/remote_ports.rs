use crate::json_support::JsonValue;
use crate::repository_pack_json::{
    ZstdBulkCommitRequest, ZstdBulkCommitResponse, ZstdPackUploadResponse,
};

pub(super) trait PlanSyncRemoteInventorySource {
    fn list_plan_summaries(
        &mut self,
        repo_name: &str,
        artifact_path: Option<&str>,
    ) -> Result<Vec<JsonValue>, String>;
}

pub(super) fn list_plan_summaries_with_plan_sync_remote_inventory_source<S>(
    source: &mut S,
    repo_name: &str,
    artifact_path: Option<&str>,
) -> Result<Vec<JsonValue>, String>
where
    S: PlanSyncRemoteInventorySource + ?Sized,
{
    source.list_plan_summaries(repo_name, artifact_path)
}

pub(super) trait PlanSyncRemotePlanReader {
    fn get_plan(&mut self, plan_id: &str) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncRemoteRevisionLister {
    fn list_plan_revisions(&mut self, plan_id: &str) -> Result<Vec<JsonValue>, String>;
}

pub(super) trait PlanSyncRemoteRevisionReader {
    fn get_plan_revision(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncRemoteContinuitySource:
    PlanSyncRemoteInventorySource + PlanSyncRemoteRevisionLister + PlanSyncRemoteRevisionReader
{
}

impl<C> PlanSyncRemoteContinuitySource for C where
    C: PlanSyncRemoteInventorySource
        + PlanSyncRemoteRevisionLister
        + PlanSyncRemoteRevisionReader
        + ?Sized
{
}

pub(super) trait PlanSyncRemotePlanCreator {
    #[allow(clippy::too_many_arguments)]
    fn create_plan(
        &mut self,
        repo_name: &str,
        title: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items: &[JsonValue],
        summary: Option<&str>,
        status: &str,
        plan_id: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
        packed_artifact: Option<&JsonValue>,
    ) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncRemotePlanReviser {
    #[allow(clippy::too_many_arguments)]
    fn revise_plan(
        &mut self,
        plan_id: &str,
        artifact_path: &str,
        artifact_selector: Option<&str>,
        artifact_heading: &str,
        items: &[JsonValue],
        title: Option<&str>,
        summary: Option<&str>,
        source_kind: &str,
        artifact_body: Option<&str>,
        expected_head_revision_id: Option<&str>,
        packed_artifact: Option<&JsonValue>,
    ) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncRemotePackedArtifactUploader {
    fn get_remote_zstd_object_pack_if_present(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Option<Vec<u8>>, String>;

    fn get_remote_zstd_tree_pack_if_present(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> Result<Option<Vec<u8>>, String>;

    fn put_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> Result<ZstdPackUploadResponse, String>;

    fn put_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
        pack_bytes: &[u8],
    ) -> Result<ZstdPackUploadResponse, String>;

    fn commit_remote_zstd_bulk(
        &mut self,
        repo_name: &str,
        request: &ZstdBulkCommitRequest,
    ) -> Result<ZstdBulkCommitResponse, String>;
}

pub(super) trait PlanSyncRemoteStatusUpdater {
    fn update_plan_status(&mut self, plan_id: &str, status: &str) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncRemoteTaskStarter {
    fn start_plan_bound_task(
        &mut self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncRemoteRevisionArtifactWriter {
    fn put_plan_revision_artifacts(
        &mut self,
        plan_id: &str,
        plan_revision_id: &str,
        artifacts: &[JsonValue],
    ) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncRemotePublishHistoryReader:
    PlanSyncRemotePlanReader + PlanSyncRemoteRevisionLister + PlanSyncRemoteRevisionReader
{
}

impl<C> PlanSyncRemotePublishHistoryReader for C where
    C: PlanSyncRemotePlanReader
        + PlanSyncRemoteRevisionLister
        + PlanSyncRemoteRevisionReader
        + ?Sized
{
}

pub(super) trait PlanSyncRemotePublishMutator:
    PlanSyncRemotePlanCreator
    + PlanSyncRemotePlanReviser
    + PlanSyncRemotePackedArtifactUploader
    + PlanSyncRemoteStatusUpdater
    + PlanSyncRemoteTaskStarter
{
}

impl<C> PlanSyncRemotePublishMutator for C where
    C: PlanSyncRemotePlanCreator
        + PlanSyncRemotePlanReviser
        + PlanSyncRemotePackedArtifactUploader
        + PlanSyncRemoteStatusUpdater
        + PlanSyncRemoteTaskStarter
        + ?Sized
{
}

pub(super) trait PlanSyncRemotePublisher:
    PlanSyncRemotePublishHistoryReader + PlanSyncRemotePublishMutator
{
}

impl<C> PlanSyncRemotePublisher for C where
    C: PlanSyncRemotePublishHistoryReader + PlanSyncRemotePublishMutator + ?Sized
{
}

pub(super) fn get_plan_with_plan_sync_remote_client<C>(
    client: &mut C,
    plan_id: &str,
) -> Result<JsonValue, String>
where
    C: PlanSyncRemotePlanReader + ?Sized,
{
    client.get_plan(plan_id)
}

pub(super) fn start_plan_bound_task_with_plan_sync_remote_client<C>(
    client: &mut C,
    repo_name: &str,
    payload: &JsonValue,
) -> Result<JsonValue, String>
where
    C: PlanSyncRemoteTaskStarter + ?Sized,
{
    client.start_plan_bound_task(repo_name, payload)
}

pub(super) fn list_plan_revisions_with_plan_sync_remote_client<C>(
    client: &mut C,
    plan_id: &str,
) -> Result<Vec<JsonValue>, String>
where
    C: PlanSyncRemoteRevisionLister + ?Sized,
{
    client.list_plan_revisions(plan_id)
}

pub(super) fn get_plan_revision_with_plan_sync_remote_client<C>(
    client: &mut C,
    plan_id: &str,
    plan_revision_id: &str,
) -> Result<JsonValue, String>
where
    C: PlanSyncRemoteRevisionReader + ?Sized,
{
    client.get_plan_revision(plan_id, plan_revision_id)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_plan_with_plan_sync_remote_client<C>(
    client: &mut C,
    repo_name: &str,
    title: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: &[JsonValue],
    summary: Option<&str>,
    status: &str,
    plan_id: Option<&str>,
    source_kind: &str,
    artifact_body: Option<&str>,
    packed_artifact: Option<&JsonValue>,
) -> Result<JsonValue, String>
where
    C: PlanSyncRemotePlanCreator + ?Sized,
{
    client.create_plan(
        repo_name,
        title,
        artifact_path,
        artifact_selector,
        artifact_heading,
        items,
        summary,
        status,
        plan_id,
        source_kind,
        artifact_body,
        packed_artifact,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn revise_plan_with_plan_sync_remote_client<C>(
    client: &mut C,
    plan_id: &str,
    artifact_path: &str,
    artifact_selector: Option<&str>,
    artifact_heading: &str,
    items: &[JsonValue],
    title: Option<&str>,
    summary: Option<&str>,
    source_kind: &str,
    artifact_body: Option<&str>,
    expected_head_revision_id: Option<&str>,
    packed_artifact: Option<&JsonValue>,
) -> Result<JsonValue, String>
where
    C: PlanSyncRemotePlanReviser + ?Sized,
{
    client.revise_plan(
        plan_id,
        artifact_path,
        artifact_selector,
        artifact_heading,
        items,
        title,
        summary,
        source_kind,
        artifact_body,
        expected_head_revision_id,
        packed_artifact,
    )
}

pub(super) fn put_remote_zstd_object_pack_with_plan_sync_remote_client<C>(
    client: &mut C,
    repo_name: &str,
    pack_id: &str,
    pack_bytes: &[u8],
) -> Result<ZstdPackUploadResponse, String>
where
    C: PlanSyncRemotePackedArtifactUploader + ?Sized,
{
    client.put_remote_zstd_object_pack(repo_name, pack_id, pack_bytes)
}

pub(super) fn get_remote_zstd_object_pack_if_present_with_plan_sync_remote_client<C>(
    client: &mut C,
    repo_name: &str,
    pack_id: &str,
) -> Result<Option<Vec<u8>>, String>
where
    C: PlanSyncRemotePackedArtifactUploader + ?Sized,
{
    client.get_remote_zstd_object_pack_if_present(repo_name, pack_id)
}

pub(super) fn put_remote_zstd_tree_pack_with_plan_sync_remote_client<C>(
    client: &mut C,
    repo_name: &str,
    pack_id: &str,
    pack_bytes: &[u8],
) -> Result<ZstdPackUploadResponse, String>
where
    C: PlanSyncRemotePackedArtifactUploader + ?Sized,
{
    client.put_remote_zstd_tree_pack(repo_name, pack_id, pack_bytes)
}

pub(super) fn get_remote_zstd_tree_pack_if_present_with_plan_sync_remote_client<C>(
    client: &mut C,
    repo_name: &str,
    pack_id: &str,
) -> Result<Option<Vec<u8>>, String>
where
    C: PlanSyncRemotePackedArtifactUploader + ?Sized,
{
    client.get_remote_zstd_tree_pack_if_present(repo_name, pack_id)
}

pub(super) fn commit_remote_zstd_bulk_with_plan_sync_remote_client<C>(
    client: &mut C,
    repo_name: &str,
    request: &ZstdBulkCommitRequest,
) -> Result<ZstdBulkCommitResponse, String>
where
    C: PlanSyncRemotePackedArtifactUploader + ?Sized,
{
    client.commit_remote_zstd_bulk(repo_name, request)
}

pub(super) fn update_plan_status_with_plan_sync_remote_client<C>(
    client: &mut C,
    plan_id: &str,
    status: &str,
) -> Result<JsonValue, String>
where
    C: PlanSyncRemoteStatusUpdater + ?Sized,
{
    client.update_plan_status(plan_id, status)
}

pub(super) fn put_plan_revision_artifacts_with_plan_sync_remote_client<C>(
    client: &mut C,
    plan_id: &str,
    plan_revision_id: &str,
    artifacts: &[JsonValue],
) -> Result<JsonValue, String>
where
    C: PlanSyncRemoteRevisionArtifactWriter + ?Sized,
{
    client.put_plan_revision_artifacts(plan_id, plan_revision_id, artifacts)
}
