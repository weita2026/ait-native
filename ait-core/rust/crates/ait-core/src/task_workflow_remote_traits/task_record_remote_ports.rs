use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowRemoteTaskReader {
    fn get_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowRemoteTaskLister {
    fn list_tasks(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>>;
}

pub trait TaskWorkflowRemoteTaskAuditReader {
    fn read_task_audit(
        &mut self,
        repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowRemoteTaskCreator {
    #[allow(clippy::too_many_arguments)]
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowTaskRecordRemote:
    TaskWorkflowRemoteTaskReader
    + TaskWorkflowRemoteTaskLister
    + TaskWorkflowRemoteTaskAuditReader
    + TaskWorkflowRemoteTaskCreator
{
}

impl<R> TaskWorkflowTaskRecordRemote for R where
    R: TaskWorkflowRemoteTaskReader
        + TaskWorkflowRemoteTaskLister
        + TaskWorkflowRemoteTaskAuditReader
        + TaskWorkflowRemoteTaskCreator
        + ?Sized
{
}
