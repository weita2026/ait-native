use crate::json_support::JsonValue as Value;

use crate::plan_store::PlanStoreResult;

pub trait TaskWorkflowTaskLister {
    fn list_tasks(&self) -> PlanStoreResult<Vec<Value>>;
}

pub trait TaskWorkflowTaskReader {
    fn get_task(&self, task_id: &str) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowTaskCreator {
    #[allow(clippy::too_many_arguments)]
    fn create_task(
        &self,
        repo_name: &str,
        title: &str,
        intent: &str,
        namespace_prefix: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowTaskCloser {
    fn close_task(&self, task_id: &str, status: &str) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowTaskPublisher {
    fn mark_task_published(
        &self,
        task_id: &str,
        remote_name: Option<&str>,
        published_task_id: Option<&str>,
    ) -> PlanStoreResult<Value>;
}

pub trait TaskWorkflowTaskStore:
    TaskWorkflowTaskLister
    + TaskWorkflowTaskReader
    + TaskWorkflowTaskCreator
    + TaskWorkflowTaskCloser
    + TaskWorkflowTaskPublisher
{
}

impl<S> TaskWorkflowTaskStore for S where
    S: TaskWorkflowTaskLister
        + TaskWorkflowTaskReader
        + TaskWorkflowTaskCreator
        + TaskWorkflowTaskCloser
        + TaskWorkflowTaskPublisher
        + ?Sized
{
}
