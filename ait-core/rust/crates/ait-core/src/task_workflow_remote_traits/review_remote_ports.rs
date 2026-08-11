use crate::json_support::JsonValue as Value;

use super::http_client_types::TaskWorkflowHttpClientResult;

pub trait TaskWorkflowReviewRequester {
    #[allow(clippy::too_many_arguments)]
    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowReviewRecorder {
    #[allow(clippy::too_many_arguments)]
    fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowReviewLister {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value>;
}

pub trait TaskWorkflowReviewRemote:
    TaskWorkflowReviewRequester + TaskWorkflowReviewRecorder + TaskWorkflowReviewLister
{
}

impl<R> TaskWorkflowReviewRemote for R where
    R: TaskWorkflowReviewRequester + TaskWorkflowReviewRecorder + TaskWorkflowReviewLister + ?Sized
{
}
