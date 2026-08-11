use std::fmt::{Display, Formatter};

use crate::json_support::{JsonMap as Map, JsonValue as Value};

mod json_projection;
mod publication_linkage;

pub(crate) use self::json_projection::*;
pub use self::publication_linkage::resolve_reconciled_plan_publish_linkage_with_plan_store;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanStoreError {
    Invalid(String),
    NotFound(String),
    Concurrency(String),
    Storage(String),
}

impl Display for PlanStoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message)
            | Self::NotFound(message)
            | Self::Concurrency(message)
            | Self::Storage(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for PlanStoreError {}

pub type PlanStoreResult<T> = Result<T, PlanStoreError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanPublishLinkage {
    pub plan_id: String,
    pub published_plan_id: Option<String>,
    pub plan_revision_id: Option<String>,
    pub published_plan_revision_id: Option<String>,
}

pub type WorkflowPlanPublishLinkage = PlanPublishLinkage;

#[derive(Clone, Debug, PartialEq)]
pub struct PlanItemRecord {
    pub plan_item_ref: Option<String>,
    pub text: Option<String>,
    pub checkbox_state: Option<String>,
    pub heading_path: Vec<String>,
    pub line_number: Option<i64>,
    pub payload: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanRevisionRecord {
    pub plan_revision_id: String,
    pub plan_id: String,
    pub revision_number: i64,
    pub parent_plan_revision_id: Option<String>,
    pub title_snapshot: String,
    pub summary: Option<String>,
    pub artifact_path: String,
    pub artifact_selector: Option<String>,
    pub artifact_heading: String,
    pub artifact_blob_id: Option<String>,
    pub items: Vec<PlanItemRecord>,
    pub source_kind: String,
    pub created_by: Option<String>,
    pub actor_type: String,
    pub publication_state: String,
    pub published_plan_revision_id: Option<String>,
    pub published_at: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanHeadSummary {
    pub head_revision_number: Option<i64>,
    pub head_revision_summary: Option<String>,
    pub head_artifact_path: Option<String>,
    pub head_artifact_selector: Option<String>,
    pub head_artifact_heading: Option<String>,
    pub head_artifact_blob_id: Option<String>,
    pub head_revision_created_at: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanRecord {
    pub plan_id: String,
    pub repo_name: String,
    pub title: String,
    pub status: String,
    pub head_revision_id: Option<String>,
    pub publication_state: String,
    pub published_remote_name: Option<String>,
    pub published_plan_id: Option<String>,
    pub published_head_revision_id: Option<String>,
    pub published_at: Option<String>,
    pub created_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub head_revision: Option<PlanRevisionRecord>,
    pub head_summary: Option<PlanHeadSummary>,
}

pub struct CreatePlanInput<'a> {
    pub plan_id: &'a str,
    pub plan_revision_id: &'a str,
    pub repo_name: &'a str,
    pub title: &'a str,
    pub artifact_path: &'a str,
    pub artifact_selector: Option<&'a str>,
    pub artifact_heading: &'a str,
    pub items_json: &'a str,
    pub artifact_blob_id: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub status: &'a str,
    pub source_kind: &'a str,
    pub created_by: Option<&'a str>,
    pub actor_type: &'a str,
    pub publication_state: &'a str,
    pub now: &'a str,
}

pub struct RevisePlanInput<'a> {
    pub plan_id: &'a str,
    pub plan_revision_id: &'a str,
    pub artifact_path: &'a str,
    pub artifact_selector: Option<&'a str>,
    pub artifact_heading: &'a str,
    pub items_json: &'a str,
    pub artifact_blob_id: Option<&'a str>,
    pub title: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub source_kind: &'a str,
    pub created_by: Option<&'a str>,
    pub actor_type: &'a str,
    pub now: &'a str,
}

pub struct ClosePlanInput<'a> {
    pub plan_id: &'a str,
    pub status: &'a str,
    pub now: &'a str,
}

pub struct RekeyPlanInput<'a> {
    pub plan_id: &'a str,
    pub new_plan_id: &'a str,
    pub now: &'a str,
}

pub struct PublishPlanInput<'a> {
    pub plan_id: &'a str,
    pub remote_name: Option<&'a str>,
    pub published_plan_id: &'a str,
    pub published_head_revision_id: Option<&'a str>,
    pub revision_mappings: &'a [(String, String)],
    pub now: &'a str,
}

pub trait PlanStore {
    fn list_plans(&self) -> PlanStoreResult<Vec<PlanRecord>>;
    fn get_plan(&self, plan_id: &str) -> PlanStoreResult<PlanRecord>;
    fn list_plan_revisions(&self, plan_id: &str) -> PlanStoreResult<Vec<PlanRevisionRecord>>;
    fn get_plan_revision(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> PlanStoreResult<PlanRevisionRecord>;
    fn get_plan_revision_by_id(
        &self,
        plan_revision_id: &str,
    ) -> PlanStoreResult<PlanRevisionRecord>;
    fn resolve_plan_publish_linkage(
        &self,
        plan_id: Option<&str>,
        plan_revision_id: Option<&str>,
    ) -> PlanStoreResult<PlanPublishLinkage>;
    fn create_plan(&self, input: CreatePlanInput<'_>) -> PlanStoreResult<PlanRecord>;
    fn revise_plan(&self, input: RevisePlanInput<'_>) -> PlanStoreResult<PlanRecord>;
    fn close_plan(&self, input: ClosePlanInput<'_>) -> PlanStoreResult<PlanRecord>;
    fn rekey_plan(&self, input: RekeyPlanInput<'_>) -> PlanStoreResult<PlanRecord>;
    fn mark_plan_published(&self, input: PublishPlanInput<'_>) -> PlanStoreResult<PlanRecord>;
}

/// Read-only Plan port used by command surfaces that must not acquire a write boundary.
pub trait PlanReadStore {
    fn list_plans(&self) -> PlanStoreResult<Vec<PlanRecord>>;
    fn get_plan(&self, plan_id: &str) -> PlanStoreResult<PlanRecord>;
    fn list_plan_revisions(&self, plan_id: &str) -> PlanStoreResult<Vec<PlanRevisionRecord>>;
    fn get_plan_revision(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> PlanStoreResult<PlanRevisionRecord>;
    fn get_plan_revision_by_id(
        &self,
        plan_revision_id: &str,
    ) -> PlanStoreResult<PlanRevisionRecord>;
    fn resolve_plan_publish_linkage(
        &self,
        plan_id: Option<&str>,
        plan_revision_id: Option<&str>,
    ) -> PlanStoreResult<PlanPublishLinkage>;
}

impl<S> PlanReadStore for S
where
    S: PlanStore + ?Sized,
{
    fn list_plans(&self) -> PlanStoreResult<Vec<PlanRecord>> {
        PlanStore::list_plans(self)
    }

    fn get_plan(&self, plan_id: &str) -> PlanStoreResult<PlanRecord> {
        PlanStore::get_plan(self, plan_id)
    }

    fn list_plan_revisions(&self, plan_id: &str) -> PlanStoreResult<Vec<PlanRevisionRecord>> {
        PlanStore::list_plan_revisions(self, plan_id)
    }

    fn get_plan_revision(
        &self,
        plan_id: &str,
        plan_revision_id: &str,
    ) -> PlanStoreResult<PlanRevisionRecord> {
        PlanStore::get_plan_revision(self, plan_id, plan_revision_id)
    }

    fn get_plan_revision_by_id(
        &self,
        plan_revision_id: &str,
    ) -> PlanStoreResult<PlanRevisionRecord> {
        PlanStore::get_plan_revision_by_id(self, plan_revision_id)
    }

    fn resolve_plan_publish_linkage(
        &self,
        plan_id: Option<&str>,
        plan_revision_id: Option<&str>,
    ) -> PlanStoreResult<PlanPublishLinkage> {
        PlanStore::resolve_plan_publish_linkage(self, plan_id, plan_revision_id)
    }
}

pub fn list_plans_with_plan_store<S>(store: &S) -> PlanStoreResult<Vec<PlanRecord>>
where
    S: PlanReadStore + ?Sized,
{
    PlanReadStore::list_plans(store)
}

pub fn get_plan_with_plan_store<S>(store: &S, plan_id: &str) -> PlanStoreResult<PlanRecord>
where
    S: PlanReadStore + ?Sized,
{
    PlanReadStore::get_plan(store, plan_id)
}

pub fn list_plan_revisions_with_plan_store<S>(
    store: &S,
    plan_id: &str,
) -> PlanStoreResult<Vec<PlanRevisionRecord>>
where
    S: PlanReadStore + ?Sized,
{
    PlanReadStore::list_plan_revisions(store, plan_id)
}

pub fn get_plan_revision_with_plan_store<S>(
    store: &S,
    plan_id: &str,
    plan_revision_id: &str,
) -> PlanStoreResult<PlanRevisionRecord>
where
    S: PlanReadStore + ?Sized,
{
    PlanReadStore::get_plan_revision(store, plan_id, plan_revision_id)
}

pub fn get_plan_revision_by_id_with_plan_store<S>(
    store: &S,
    plan_revision_id: &str,
) -> PlanStoreResult<PlanRevisionRecord>
where
    S: PlanReadStore + ?Sized,
{
    PlanReadStore::get_plan_revision_by_id(store, plan_revision_id)
}

pub fn resolve_plan_publish_linkage_with_plan_store<S>(
    store: &S,
    plan_id: Option<&str>,
    plan_revision_id: Option<&str>,
) -> PlanStoreResult<PlanPublishLinkage>
where
    S: PlanReadStore + ?Sized,
{
    PlanReadStore::resolve_plan_publish_linkage(store, plan_id, plan_revision_id)
}

pub fn create_plan_with_plan_store<S>(
    store: &S,
    input: CreatePlanInput<'_>,
) -> PlanStoreResult<PlanRecord>
where
    S: PlanStore + ?Sized,
{
    store.create_plan(input)
}

pub fn revise_plan_with_plan_store<S>(
    store: &S,
    input: RevisePlanInput<'_>,
) -> PlanStoreResult<PlanRecord>
where
    S: PlanStore + ?Sized,
{
    store.revise_plan(input)
}

pub fn close_plan_with_plan_store<S>(
    store: &S,
    input: ClosePlanInput<'_>,
) -> PlanStoreResult<PlanRecord>
where
    S: PlanStore + ?Sized,
{
    store.close_plan(input)
}

pub fn rekey_plan_with_plan_store<S>(
    store: &S,
    input: RekeyPlanInput<'_>,
) -> PlanStoreResult<PlanRecord>
where
    S: PlanStore + ?Sized,
{
    store.rekey_plan(input)
}

pub fn mark_plan_published_with_plan_store<S>(
    store: &S,
    input: PublishPlanInput<'_>,
) -> PlanStoreResult<PlanRecord>
where
    S: PlanStore + ?Sized,
{
    store.mark_plan_published(input)
}

pub fn plan_record_list_json(record: &PlanRecord) -> Value {
    plan_record_list_payload(record)
}

pub fn plan_record_detail_json(record: &PlanRecord) -> Value {
    plan_record_detail_payload(record)
}

pub fn plan_revision_record_json(record: &PlanRevisionRecord) -> Value {
    plan_revision_record_payload(record)
}
