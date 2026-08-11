pub type WorkflowReleaseStoreResult<T> = Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowReleaseRecord {
    pub release_id: String,
    pub repo_name: String,
    pub version: String,
    pub line_name: String,
    pub snapshot_id: String,
    pub manifest_hash: String,
    pub profile: String,
    pub package_name: Option<String>,
    pub package_version: Option<String>,
    pub package_requires_python: Option<String>,
    pub status: String,
    pub checks_json: String,
    pub artifacts_json: String,
    pub formula_json: String,
    pub metadata_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowReleaseUpdate {
    pub status: Option<String>,
    pub checks_json: Option<String>,
    pub artifacts_json: Option<String>,
    pub formula_json: Option<String>,
    pub metadata_json: Option<String>,
    pub updated_at: String,
}

pub trait WorkflowReleaseStore {
    fn create_release(
        &self,
        record: &WorkflowReleaseRecord,
    ) -> WorkflowReleaseStoreResult<WorkflowReleaseRecord>;
    fn list_releases(&self) -> WorkflowReleaseStoreResult<Vec<WorkflowReleaseRecord>>;
    fn release_by_id(
        &self,
        release_id: &str,
    ) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>>;
    fn latest_published_release_excluding_version(
        &self,
        version: &str,
    ) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>>;
    fn update_release(
        &self,
        release_id: &str,
        update: &WorkflowReleaseUpdate,
    ) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>>;
}

pub fn create_workflow_release_with_store<S>(
    store: &S,
    record: &WorkflowReleaseRecord,
) -> WorkflowReleaseStoreResult<WorkflowReleaseRecord>
where
    S: WorkflowReleaseStore + ?Sized,
{
    store.create_release(record)
}

pub fn list_workflow_releases_with_store<S>(
    store: &S,
) -> WorkflowReleaseStoreResult<Vec<WorkflowReleaseRecord>>
where
    S: WorkflowReleaseStore + ?Sized,
{
    store.list_releases()
}

pub fn get_workflow_release_with_store<S>(
    store: &S,
    release_id: &str,
) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>>
where
    S: WorkflowReleaseStore + ?Sized,
{
    store.release_by_id(release_id)
}

pub fn latest_published_workflow_release_excluding_version_with_store<S>(
    store: &S,
    version: &str,
) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>>
where
    S: WorkflowReleaseStore + ?Sized,
{
    store.latest_published_release_excluding_version(version)
}

pub fn update_workflow_release_with_store<S>(
    store: &S,
    release_id: &str,
    update: &WorkflowReleaseUpdate,
) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>>
where
    S: WorkflowReleaseStore + ?Sized,
{
    store.update_release(release_id, update)
}
