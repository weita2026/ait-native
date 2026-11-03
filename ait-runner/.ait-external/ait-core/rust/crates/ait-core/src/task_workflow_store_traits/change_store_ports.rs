use crate::json_support::JsonValue as Value;

use crate::plan_store::PlanStoreResult;

pub trait TaskWorkflowChangeLister {
    fn list_changes(&self) -> PlanStoreResult<Vec<Value>>;
    fn list_task_changes(&self, task_id: &str) -> PlanStoreResult<Vec<Value>> {
        Ok(self
            .list_changes()?
            .into_iter()
            .filter(|row| row["task_id"].as_str() == Some(task_id))
            .collect())
    }
}

pub trait TaskWorkflowChangeReader {
    fn get_change(&self, change_id: &str) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowChangeCreator {
    fn create_change(
        &self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        namespace_prefix: Option<&str>,
        fork_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowChangeCloser {
    fn close_change(&self, change_id: &str, status: &str) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowChangeLander {
    fn land_change(
        &self,
        change_id: &str,
        target_line: &str,
        landed_snapshot_id: &str,
        pre_land_target_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowChangePublisher {
    fn mark_change_published(
        &self,
        change_id: &str,
        remote_name: Option<&str>,
        published_change_id: Option<&str>,
        allow_landed: bool,
    ) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowChangeStore:
    TaskWorkflowChangeLister
    + TaskWorkflowChangeReader
    + TaskWorkflowChangeCreator
    + TaskWorkflowChangeCloser
    + TaskWorkflowChangeLander
    + TaskWorkflowChangePublisher
{
}

impl<S> TaskWorkflowChangeStore for S where
    S: TaskWorkflowChangeLister
        + TaskWorkflowChangeReader
        + TaskWorkflowChangeCreator
        + TaskWorkflowChangeCloser
        + TaskWorkflowChangeLander
        + TaskWorkflowChangePublisher
        + ?Sized
{
}
