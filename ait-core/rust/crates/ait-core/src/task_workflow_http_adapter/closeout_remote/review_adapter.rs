use super::*;

impl HttpWorkflowCloseoutRemote {
    pub fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        request_review_with_task_workflow_closeout_remote(
            self,
            change_id,
            patchset_id,
            reviewer_groups,
            note,
            repo_name,
            exact_id,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        record_review_with_task_workflow_closeout_remote(
            self,
            change_id,
            patchset_id,
            reviewer,
            action,
            comment,
            blocking,
            repo_name,
            exact_id,
        )
    }

    pub fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        list_reviews_with_task_workflow_closeout_remote(self, change_id, repo_name, exact_id)
    }
}

impl TaskWorkflowReviewRequester for HttpWorkflowCloseoutRemote {
    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_change_ref = self.resolved_change_ref(change_id, repo_name, exact_id)?;
        let review = self.manager.request_review(
            &resolved_change_ref,
            patchset_id,
            reviewer_groups,
            note,
        )?;
        self.normalize_change_identity_payload(review, change_id)
    }
}

impl TaskWorkflowReviewRecorder for HttpWorkflowCloseoutRemote {
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
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_change_ref = self.resolved_change_ref(change_id, repo_name, exact_id)?;
        let review = self.manager.record_review(
            &resolved_change_ref,
            patchset_id,
            reviewer,
            action,
            comment,
            blocking,
        )?;
        self.normalize_change_identity_payload(review, change_id)
    }
}

impl TaskWorkflowReviewLister for HttpWorkflowCloseoutRemote {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_change_ref = self.resolved_change_ref(change_id, repo_name, exact_id)?;
        let reviews = self.manager.list_reviews(&resolved_change_ref)?;
        self.normalize_change_identity_payload(reviews, change_id)
    }
}
