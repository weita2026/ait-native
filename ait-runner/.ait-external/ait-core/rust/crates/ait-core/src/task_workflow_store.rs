use crate::json_support::JsonValue as Value;
use crate::plan_store::PlanStoreResult;

pub use crate::task_workflow_store_traits::{
    TaskWorkflowChangeCloser, TaskWorkflowChangeCreator, TaskWorkflowChangeLander,
    TaskWorkflowChangeLister, TaskWorkflowChangePublisher, TaskWorkflowChangeReader,
    TaskWorkflowChangeStore, TaskWorkflowTaskCloser, TaskWorkflowTaskCreator,
    TaskWorkflowTaskLister, TaskWorkflowTaskPublisher, TaskWorkflowTaskReader,
    TaskWorkflowTaskStore,
};

pub fn list_tasks_with_task_workflow_task_store<S>(store: &S) -> PlanStoreResult<Vec<Value>>
where
    S: TaskWorkflowTaskLister + ?Sized,
{
    store.list_tasks()
}

pub fn get_task_with_task_workflow_task_store<S>(store: &S, task_id: &str) -> PlanStoreResult<Value>
where
    S: TaskWorkflowTaskReader + ?Sized,
{
    store.get_task(task_id)
}

#[allow(clippy::too_many_arguments)]
pub fn create_task_with_task_workflow_task_store<S>(
    store: &S,
    repo_name: &str,
    title: &str,
    intent: &str,
    namespace_prefix: Option<&str>,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> PlanStoreResult<Value>
where
    S: TaskWorkflowTaskCreator + ?Sized,
{
    store.create_task(
        repo_name,
        title,
        intent,
        namespace_prefix,
        plan_id,
        origin_plan_revision_id,
        plan_item_ref,
    )
}

pub fn close_task_with_task_workflow_task_store<S>(
    store: &S,
    task_id: &str,
    status: &str,
) -> PlanStoreResult<Value>
where
    S: TaskWorkflowTaskCloser + ?Sized,
{
    store.close_task(task_id, status)
}

pub fn mark_task_published_with_task_workflow_task_store<S>(
    store: &S,
    task_id: &str,
    remote_name: Option<&str>,
    published_task_id: Option<&str>,
) -> PlanStoreResult<Value>
where
    S: TaskWorkflowTaskPublisher + ?Sized,
{
    store.mark_task_published(task_id, remote_name, published_task_id)
}

pub fn list_changes_with_task_workflow_change_store<S>(store: &S) -> PlanStoreResult<Vec<Value>>
where
    S: TaskWorkflowChangeLister + ?Sized,
{
    store.list_changes()
}

pub fn get_change_with_task_workflow_change_store<S>(
    store: &S,
    change_id: &str,
) -> PlanStoreResult<Value>
where
    S: TaskWorkflowChangeReader + ?Sized,
{
    store.get_change(change_id)
}

pub fn land_change_with_task_workflow_change_store<S>(
    store: &S,
    change_id: &str,
    target_line: &str,
    landed_snapshot_id: &str,
    pre_land_target_snapshot_id: Option<&str>,
) -> PlanStoreResult<Value>
where
    S: TaskWorkflowChangeLander + ?Sized,
{
    store.land_change(
        change_id,
        target_line,
        landed_snapshot_id,
        pre_land_target_snapshot_id,
    )
}
