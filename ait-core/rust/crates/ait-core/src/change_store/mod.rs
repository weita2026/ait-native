use crate::json_support::JsonValue as Value;
use crate::plan_store::{PlanStoreError, PlanStoreResult};

pub trait ChangeStore {
    fn list_changes(&self) -> PlanStoreResult<Vec<Value>>;
    fn list_changes_for_task(&self, task_id: &str) -> PlanStoreResult<Vec<Value>> {
        let task_id = task_id.trim();
        if task_id.is_empty() {
            return Err(PlanStoreError::Invalid(
                "task_id must not be empty.".to_string(),
            ));
        }
        self.list_changes().map(|rows| {
            rows.into_iter()
                .filter(|row| row.get("task_id").and_then(Value::as_str) == Some(task_id))
                .collect()
        })
    }
    fn get_change(&self, change_id: &str) -> PlanStoreResult<Value>;
    fn allocate_change_identity(
        &self,
        repo_name: &str,
        namespace_prefix: Option<&str>,
    ) -> PlanStoreResult<Value>;
    fn create_change(
        &self,
        task_id: &str,
        repo_name: &str,
        title: &str,
        base_line: &str,
        namespace_prefix: Option<&str>,
        fork_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<Value>;
    #[allow(clippy::too_many_arguments)]
    fn create_change_explicit(
        &self,
        change_id: &str,
        task_id: &str,
        repo_name: &str,
        title: &str,
        base_line: &str,
        change_seq: Option<i64>,
        identity_source: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
        status: Option<&str>,
        publication_state: Option<&str>,
    ) -> PlanStoreResult<Value>;
    fn close_change(&self, change_id: &str, status: &str) -> PlanStoreResult<Value>;
    fn reopen_change_as_draft(&self, change_id: &str) -> PlanStoreResult<Value>;
    fn land_change(
        &self,
        change_id: &str,
        target_line: &str,
        landed_snapshot_id: &str,
        pre_land_target_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<Value>;
    fn mark_change_published(
        &self,
        change_id: &str,
        remote_name: Option<&str>,
        published_change_id: Option<&str>,
        allow_landed: bool,
    ) -> PlanStoreResult<Value>;
}

pub fn list_changes_with_change_store<S>(store: &S) -> PlanStoreResult<Vec<Value>>
where
    S: ChangeStore + ?Sized,
{
    store.list_changes()
}

pub fn list_changes_for_task_with_change_store<S>(
    store: &S,
    task_id: &str,
) -> PlanStoreResult<Vec<Value>>
where
    S: ChangeStore + ?Sized,
{
    store.list_changes_for_task(task_id)
}

pub fn get_change_with_change_store<S>(store: &S, change_id: &str) -> PlanStoreResult<Value>
where
    S: ChangeStore + ?Sized,
{
    store.get_change(change_id)
}

pub fn allocate_change_identity_with_change_store<S>(
    store: &S,
    repo_name: &str,
    namespace_prefix: Option<&str>,
) -> PlanStoreResult<Value>
where
    S: ChangeStore + ?Sized,
{
    store.allocate_change_identity(repo_name, namespace_prefix)
}

pub fn create_change_with_change_store<S>(
    store: &S,
    task_id: &str,
    repo_name: &str,
    title: &str,
    base_line: &str,
    namespace_prefix: Option<&str>,
    fork_snapshot_id: Option<&str>,
) -> PlanStoreResult<Value>
where
    S: ChangeStore + ?Sized,
{
    store.create_change(
        task_id,
        repo_name,
        title,
        base_line,
        namespace_prefix,
        fork_snapshot_id,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn create_change_explicit_with_change_store<S>(
    store: &S,
    change_id: &str,
    task_id: &str,
    repo_name: &str,
    title: &str,
    base_line: &str,
    change_seq: Option<i64>,
    identity_source: Option<&str>,
    fork_snapshot_id: Option<&str>,
    forked_from_line: Option<&str>,
    status: Option<&str>,
    publication_state: Option<&str>,
) -> PlanStoreResult<Value>
where
    S: ChangeStore + ?Sized,
{
    store.create_change_explicit(
        change_id,
        task_id,
        repo_name,
        title,
        base_line,
        change_seq,
        identity_source,
        fork_snapshot_id,
        forked_from_line,
        status,
        publication_state,
    )
}

pub fn close_change_with_change_store<S>(
    store: &S,
    change_id: &str,
    status: &str,
) -> PlanStoreResult<Value>
where
    S: ChangeStore + ?Sized,
{
    store.close_change(change_id, status)
}

pub fn reopen_change_as_draft_with_change_store<S>(
    store: &S,
    change_id: &str,
) -> PlanStoreResult<Value>
where
    S: ChangeStore + ?Sized,
{
    store.reopen_change_as_draft(change_id)
}

pub fn land_change_with_change_store<S>(
    store: &S,
    change_id: &str,
    target_line: &str,
    landed_snapshot_id: &str,
    pre_land_target_snapshot_id: Option<&str>,
) -> PlanStoreResult<Value>
where
    S: ChangeStore + ?Sized,
{
    store.land_change(
        change_id,
        target_line,
        landed_snapshot_id,
        pre_land_target_snapshot_id,
    )
}

pub fn mark_change_published_with_change_store<S>(
    store: &S,
    change_id: &str,
    remote_name: Option<&str>,
    published_change_id: Option<&str>,
    allow_landed: bool,
) -> PlanStoreResult<Value>
where
    S: ChangeStore + ?Sized,
{
    store.mark_change_published(change_id, remote_name, published_change_id, allow_landed)
}
