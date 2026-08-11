use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowRemoteChangeCreator {
    #[allow(clippy::too_many_arguments)]
    fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowRemoteChangeLister {
    fn list_changes(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>>;
}

pub trait TaskWorkflowRemoteChangeDetailReader {
    fn get_change_detail(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowRemoteChangeReader {
    fn get_change(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowRemoteChangeCloser {
    fn close_change(
        &mut self,
        change_ref: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowChangeRemote:
    TaskWorkflowRemoteChangeCreator
    + TaskWorkflowRemoteChangeLister
    + TaskWorkflowRemoteChangeDetailReader
    + TaskWorkflowRemoteChangeReader
    + TaskWorkflowRemoteChangeCloser
{
}

impl<R> TaskWorkflowChangeRemote for R where
    R: TaskWorkflowRemoteChangeCreator
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeCloser
        + ?Sized
{
}
