use super::*;
use ait_core::agent_local_workflow_backend::LOCAL_WORKFLOW_AUTHORITY_ERROR;
use ait_core::workflow_event_store::{WorkflowEventRecord, WorkflowEventStore};
use ait_core::workflow_release_store::{
    WorkflowReleaseRecord, WorkflowReleaseStore, WorkflowReleaseStoreResult, WorkflowReleaseUpdate,
};

#[derive(Clone, Debug, Default)]
pub struct UnavailableLocalWorkflowStore;

fn unavailable_workflow<T>() -> Result<T, String> {
    Err(LOCAL_WORKFLOW_AUTHORITY_ERROR.to_string())
}

impl WorkflowEventStore for UnavailableLocalWorkflowStore {
    fn record_event(&self, _event: &WorkflowEventRecord) -> Result<bool, String> {
        unavailable_workflow()
    }
}

impl WorkflowReleaseStore for UnavailableLocalWorkflowStore {
    fn create_release(
        &self,
        _record: &WorkflowReleaseRecord,
    ) -> WorkflowReleaseStoreResult<WorkflowReleaseRecord> {
        unavailable_workflow()
    }

    fn list_releases(&self) -> WorkflowReleaseStoreResult<Vec<WorkflowReleaseRecord>> {
        unavailable_workflow()
    }

    fn release_by_id(
        &self,
        _release_id: &str,
    ) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>> {
        unavailable_workflow()
    }

    fn latest_published_release_excluding_version(
        &self,
        _version: &str,
    ) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>> {
        unavailable_workflow()
    }

    fn update_release(
        &self,
        _release_id: &str,
        _update: &WorkflowReleaseUpdate,
    ) -> WorkflowReleaseStoreResult<Option<WorkflowReleaseRecord>> {
        unavailable_workflow()
    }
}

impl RepoRuntime {
    pub fn task_store(&self) -> Result<RepoWorkflowStore<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>, String> {
        Ok(self
            .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .workflows())
    }

    pub fn change_store(
        &self,
    ) -> Result<RepoWorkflowStore<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>, String> {
        Ok(self
            .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .workflows())
    }

    pub fn line_store(&self) -> Result<impl LineStore, String> {
        let workspace_root = self.workspace_root();
        self.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
    }

    pub fn stash_store(&self) -> Result<RepoStashStore<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>, String> {
        Ok(self
            .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .stashes())
    }

    pub fn remote_store(&self) -> Result<impl RemoteStore, String> {
        ConfigRemoteStore::new(self.root.join(APP_DIR).join(CONFIG_NAME))
    }

    pub fn workflow_event_store(&self) -> Result<UnavailableLocalWorkflowStore, String> {
        Err(LOCAL_WORKFLOW_AUTHORITY_ERROR.to_string())
    }

    pub fn workflow_release_store(&self) -> Result<UnavailableLocalWorkflowStore, String> {
        Err(LOCAL_WORKFLOW_AUTHORITY_ERROR.to_string())
    }

    pub fn repo_status_store(&self) -> Result<impl RepoStatusStore, String> {
        Ok(self
            .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .status())
    }
}

#[cfg(test)]
mod local_workflow_store_tests {
    use super::*;

    #[test]
    fn task_and_change_authority_use_explicit_binary_store() {
        fn assert_task_store<S: TaskStore>() {}
        fn assert_change_store<S: ChangeStore>() {}
        assert_task_store::<RepoWorkflowStore<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>>();
        assert_change_store::<RepoWorkflowStore<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>>();
    }
}
