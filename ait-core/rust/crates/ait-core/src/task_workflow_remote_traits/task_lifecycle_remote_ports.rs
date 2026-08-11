use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowRemoteTaskCloser {
    fn close_task(
        &mut self,
        task_id: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowRemoteTaskRestarter {
    fn restart_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowTaskLifecycleRemote:
    TaskWorkflowRemoteTaskCloser + TaskWorkflowRemoteTaskRestarter
{
}

impl<R> TaskWorkflowTaskLifecycleRemote for R where
    R: TaskWorkflowRemoteTaskCloser + TaskWorkflowRemoteTaskRestarter + ?Sized
{
}
