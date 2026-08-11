use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowAtomicTaskLandSubmitter {
    fn submit_task_land(
        &mut self,
        task_or_change_ref: &str,
        target_line: Option<&str>,
        mode: &str,
        idempotency_key: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLandSubmitter {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLandReader {
    fn get_land(
        &mut self,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLandRetryer {
    fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowLandRemote:
    TaskWorkflowLandSubmitter + TaskWorkflowLandReader + TaskWorkflowLandRetryer
{
}

impl<R> TaskWorkflowLandRemote for R where
    R: TaskWorkflowLandSubmitter + TaskWorkflowLandReader + TaskWorkflowLandRetryer + ?Sized
{
}
