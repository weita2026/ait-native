use super::content_ports::PlanSyncArtifactTreeRootLocator;
use crate::json_support::JsonValue;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PlanSyncLocalRevisionArtifact {
    pub(super) artifact_path: String,
    pub(super) artifact_blob_id: Option<String>,
    pub(super) remote_published: bool,
}

pub(super) trait PlanSyncLocalInventoryStore {
    fn list_plan_summaries(&self) -> Result<Vec<JsonValue>, String>;

    fn list_plan_inventory_details(&self) -> Result<Option<Vec<JsonValue>>, String> {
        Ok(None)
    }
}

pub(super) trait PlanSyncLocalPlanStore {
    fn get_plan(&self, plan_id: &str) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncLocalRevisionStore {
    fn list_plan_revisions(&self, plan_id: &str) -> Result<Vec<JsonValue>, String>;

    fn get_plan_revision_artifact(
        &self,
        plan_revision_id: &str,
    ) -> Result<Option<PlanSyncLocalRevisionArtifact>, String>;
}

pub(super) trait PlanSyncLocalPublicationStore {
    fn remote_adoption_allocates_fresh_local_plan_identity(&self) -> bool {
        false
    }

    fn remote_adoption_preserves_local_plan_identity(&self) -> bool {
        false
    }

    fn mark_plan_published(
        &self,
        plan_id: &str,
        remote_name: Option<&str>,
        published_plan_id: &str,
        published_head_revision_id: Option<&str>,
        revision_mappings: &[(String, String)],
        published_at: &str,
    ) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncLocalLifecycleStore {
    fn close_plan(&self, plan_id: &str, status: &str, closed_at: &str)
        -> Result<JsonValue, String>;

    fn rekey_plan(
        &self,
        plan_id: &str,
        new_plan_id: &str,
        rekeyed_at: &str,
    ) -> Result<JsonValue, String>;
}

pub(super) struct PlanSyncLocalPlanCreate<'a> {
    pub(super) plan_id: &'a str,
    pub(super) plan_revision_id: &'a str,
    pub(super) repo_name: &'a str,
    pub(super) title: &'a str,
    pub(super) artifact_path: &'a str,
    pub(super) artifact_selector: Option<&'a str>,
    pub(super) artifact_heading: &'a str,
    pub(super) items_json: &'a str,
    pub(super) artifact_blob_id: Option<&'a str>,
    pub(super) artifact_root: Option<PlanSyncArtifactTreeRootLocator>,
    pub(super) summary: Option<&'a str>,
    pub(super) status: &'a str,
    pub(super) source_kind: &'a str,
    pub(super) created_by: Option<&'a str>,
    pub(super) actor_type: &'a str,
    pub(super) publication_state: &'a str,
    pub(super) now: &'a str,
}

pub(super) struct PlanSyncLocalPlanRevision<'a> {
    pub(super) plan_id: &'a str,
    pub(super) plan_revision_id: &'a str,
    pub(super) artifact_path: &'a str,
    pub(super) artifact_selector: Option<&'a str>,
    pub(super) artifact_heading: &'a str,
    pub(super) items_json: &'a str,
    pub(super) artifact_blob_id: Option<&'a str>,
    pub(super) artifact_root: Option<PlanSyncArtifactTreeRootLocator>,
    pub(super) title: Option<&'a str>,
    pub(super) summary: Option<&'a str>,
    pub(super) source_kind: &'a str,
    pub(super) created_by: Option<&'a str>,
    pub(super) actor_type: &'a str,
    pub(super) now: &'a str,
}

pub(super) trait PlanSyncLocalArtifactWriter {
    fn create_plan(&self, request: &PlanSyncLocalPlanCreate<'_>) -> Result<JsonValue, String>;

    fn revise_plan(&self, request: &PlanSyncLocalPlanRevision<'_>) -> Result<JsonValue, String>;
}

pub(super) trait PlanSyncLocalAdoptionStore:
    PlanSyncLocalPlanStore + PlanSyncLocalArtifactWriter + PlanSyncLocalPublicationStore
{
}

impl<S> PlanSyncLocalAdoptionStore for S where
    S: PlanSyncLocalPlanStore
        + PlanSyncLocalArtifactWriter
        + PlanSyncLocalPublicationStore
        + ?Sized
{
}

pub(super) trait PlanSyncLocalIdentityRebindStore:
    PlanSyncLocalRevisionStore + PlanSyncLocalLifecycleStore + PlanSyncLocalPublicationStore
{
}

impl<S> PlanSyncLocalIdentityRebindStore for S where
    S: PlanSyncLocalRevisionStore
        + PlanSyncLocalLifecycleStore
        + PlanSyncLocalPublicationStore
        + ?Sized
{
}

pub(super) trait PlanSyncLocalStore:
    PlanSyncLocalInventoryStore + PlanSyncLocalPlanStore
{
}

impl<S> PlanSyncLocalStore for S where
    S: PlanSyncLocalInventoryStore + PlanSyncLocalPlanStore + ?Sized
{
}

pub(super) trait PlanSyncLocalPublishSource:
    PlanSyncLocalPlanStore + PlanSyncLocalRevisionStore + PlanSyncLocalPublicationStore
{
}

impl<S> PlanSyncLocalPublishSource for S where
    S: PlanSyncLocalPlanStore + PlanSyncLocalRevisionStore + PlanSyncLocalPublicationStore + ?Sized
{
}

pub(super) trait PlanSyncLocalFullStore:
    PlanSyncLocalStore
    + PlanSyncLocalAdoptionStore
    + PlanSyncLocalIdentityRebindStore
    + PlanSyncLocalPublishSource
    + PlanSyncLocalLifecycleStore
    + PlanSyncLocalArtifactWriter
{
}

impl<S> PlanSyncLocalFullStore for S where
    S: PlanSyncLocalStore
        + PlanSyncLocalAdoptionStore
        + PlanSyncLocalIdentityRebindStore
        + PlanSyncLocalPublishSource
        + PlanSyncLocalLifecycleStore
        + PlanSyncLocalArtifactWriter
        + ?Sized
{
}

pub(super) fn list_plan_summaries_with_plan_sync_local_store<S>(
    store: &S,
) -> Result<Vec<JsonValue>, String>
where
    S: PlanSyncLocalInventoryStore + ?Sized,
{
    store.list_plan_summaries()
}

pub(super) fn get_plan_with_plan_sync_local_store<S>(
    store: &S,
    plan_id: &str,
) -> Result<JsonValue, String>
where
    S: PlanSyncLocalPlanStore + ?Sized,
{
    store.get_plan(plan_id)
}

pub(super) fn list_plan_revisions_with_plan_sync_local_store<S>(
    store: &S,
    plan_id: &str,
) -> Result<Vec<JsonValue>, String>
where
    S: PlanSyncLocalRevisionStore + ?Sized,
{
    store.list_plan_revisions(plan_id)
}

pub(super) fn get_plan_revision_artifact_with_plan_sync_local_store<S>(
    store: &S,
    plan_revision_id: &str,
) -> Result<Option<PlanSyncLocalRevisionArtifact>, String>
where
    S: PlanSyncLocalRevisionStore + ?Sized,
{
    store.get_plan_revision_artifact(plan_revision_id)
}

pub(super) fn mark_plan_published_with_plan_sync_local_store<S>(
    store: &S,
    plan_id: &str,
    remote_name: Option<&str>,
    published_plan_id: &str,
    published_head_revision_id: Option<&str>,
    revision_mappings: &[(String, String)],
    published_at: &str,
) -> Result<JsonValue, String>
where
    S: PlanSyncLocalPublicationStore + ?Sized,
{
    store.mark_plan_published(
        plan_id,
        remote_name,
        published_plan_id,
        published_head_revision_id,
        revision_mappings,
        published_at,
    )
}

pub(super) fn close_plan_with_plan_sync_local_lifecycle_store<S>(
    store: &S,
    plan_id: &str,
    status: &str,
    closed_at: &str,
) -> Result<JsonValue, String>
where
    S: PlanSyncLocalLifecycleStore + ?Sized,
{
    store.close_plan(plan_id, status, closed_at)
}

pub(super) fn rekey_plan_with_plan_sync_local_lifecycle_store<S>(
    store: &S,
    plan_id: &str,
    new_plan_id: &str,
    rekeyed_at: &str,
) -> Result<JsonValue, String>
where
    S: PlanSyncLocalLifecycleStore + ?Sized,
{
    store.rekey_plan(plan_id, new_plan_id, rekeyed_at)
}

pub(super) fn create_plan_with_plan_sync_local_artifact_writer<W>(
    writer: &W,
    request: &PlanSyncLocalPlanCreate<'_>,
) -> Result<JsonValue, String>
where
    W: PlanSyncLocalArtifactWriter + ?Sized,
{
    writer.create_plan(request)
}

pub(super) fn revise_plan_with_plan_sync_local_artifact_writer<W>(
    writer: &W,
    request: &PlanSyncLocalPlanRevision<'_>,
) -> Result<JsonValue, String>
where
    W: PlanSyncLocalArtifactWriter + ?Sized,
{
    writer.revise_plan(request)
}
