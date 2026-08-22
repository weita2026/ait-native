use crate::json_support::JsonValue as Value;
use crate::plan_store::PlanStoreResult;

pub trait TaskStore {
    fn list_tasks(&self) -> PlanStoreResult<Vec<Value>>;
    fn has_tasks(&self) -> PlanStoreResult<bool> {
        Ok(!self.list_tasks()?.is_empty())
    }
    fn list_completed_tasks_with_landed_changes(&self) -> PlanStoreResult<Vec<Value>>;
    fn get_task(&self, task_id: &str) -> PlanStoreResult<Value>;
    fn allocate_task_identity(
        &self,
        repo_name: &str,
        namespace_prefix: Option<&str>,
    ) -> PlanStoreResult<Value>;
    fn sequence_floor(&self, repo_name: &str, family: &str) -> PlanStoreResult<i64>;
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
    #[allow(clippy::too_many_arguments)]
    fn create_task_explicit(
        &self,
        task_id: &str,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_seq: Option<i64>,
        identity_source: Option<&str>,
        planning_state: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
        plan_linked_at: Option<&str>,
        status: Option<&str>,
        publication_state: Option<&str>,
    ) -> PlanStoreResult<Value>;
    fn close_task(&self, task_id: &str, status: &str) -> PlanStoreResult<Value>;
    fn mark_task_published(
        &self,
        task_id: &str,
        remote_name: Option<&str>,
        published_task_id: Option<&str>,
    ) -> PlanStoreResult<Value>;
}

pub fn has_tasks_with_task_store<S>(store: &S) -> PlanStoreResult<bool>
where
    S: TaskStore + ?Sized,
{
    store.has_tasks()
}

pub fn list_completed_tasks_with_landed_changes_with_task_store<S>(
    store: &S,
) -> PlanStoreResult<Vec<Value>>
where
    S: TaskStore + ?Sized,
{
    store.list_completed_tasks_with_landed_changes()
}
