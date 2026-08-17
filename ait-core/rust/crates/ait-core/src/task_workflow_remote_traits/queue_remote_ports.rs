use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowTaskQueueReader {
    fn read_task_queue(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowReviewerInboxReader {
    fn read_reviewer_inbox(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowQueueSummaryBundleReader {
    fn read_queue_summary_bundle(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowQueueRemote:
    TaskWorkflowTaskQueueReader + TaskWorkflowReviewerInboxReader + TaskWorkflowQueueSummaryBundleReader
{
}

impl<R> TaskWorkflowQueueRemote for R where
    R: TaskWorkflowTaskQueueReader
        + TaskWorkflowReviewerInboxReader
        + TaskWorkflowQueueSummaryBundleReader
        + ?Sized
{
}
