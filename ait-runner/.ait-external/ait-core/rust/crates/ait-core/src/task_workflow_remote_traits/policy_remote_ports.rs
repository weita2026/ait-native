use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowPolicyEvaluator {
    fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowPolicyReader {
    fn get_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowPolicyWaiverCreator {
    fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowPolicyRemote:
    TaskWorkflowPolicyEvaluator + TaskWorkflowPolicyReader + TaskWorkflowPolicyWaiverCreator
{
}

impl<R> TaskWorkflowPolicyRemote for R where
    R: TaskWorkflowPolicyEvaluator
        + TaskWorkflowPolicyReader
        + TaskWorkflowPolicyWaiverCreator
        + ?Sized
{
}
