use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowLineagePayloadBuilder {
    fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&Value>,
    ) -> Result<Value, String>;
}

pub trait TaskWorkflowLineReader {
    fn get_line(&mut self, repo_name: &str, line_name: &str)
        -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLineLister {
    fn list_lines(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<Value>>;
}

pub trait TaskWorkflowLineHeadUpdater {
    fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLineCloser {
    fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLineRenamer {
    #[allow(clippy::too_many_arguments)]
    fn rename_remote_line(
        &mut self,
        repo_name: &str,
        old_line_name: &str,
        new_line_name: &str,
        expected_line_id: &str,
        expected_head_snapshot_id: Option<&str>,
        idempotency_key: &str,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLineDeleter {
    fn delete_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        expected_line_id: &str,
        expected_head_snapshot_id: Option<&str>,
        idempotency_key: &str,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLineRemote:
    TaskWorkflowLineagePayloadBuilder
    + TaskWorkflowLineReader
    + TaskWorkflowLineLister
    + TaskWorkflowLineHeadUpdater
    + TaskWorkflowLineCloser
{
}

impl<R> TaskWorkflowLineRemote for R where
    R: TaskWorkflowLineagePayloadBuilder
        + TaskWorkflowLineReader
        + TaskWorkflowLineLister
        + TaskWorkflowLineHeadUpdater
        + TaskWorkflowLineCloser
        + ?Sized
{
}
