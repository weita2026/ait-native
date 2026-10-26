use super::*;
use crate::init_surface::{init_repo, InitRequest};
use ait_core::json_support::json;
use ait_core::task_workflow_http_adapter::{
    TaskWorkflowActionMutationReceiptsBuilder, TaskWorkflowAttestationReader,
    TaskWorkflowAttestationWriter, TaskWorkflowHttpClientCloser, TaskWorkflowHttpClientError,
    TaskWorkflowHttpClientInspector, TaskWorkflowHttpClientResult, TaskWorkflowHttpClientStats,
    TaskWorkflowLandReader, TaskWorkflowLandRetryer, TaskWorkflowLandSubmitter,
    TaskWorkflowLineCloser, TaskWorkflowLineReader, TaskWorkflowMutationReceiptBuilder,
    TaskWorkflowPatchsetCiRunner, TaskWorkflowPatchsetCiStatusReader, TaskWorkflowPatchsetLister,
    TaskWorkflowPatchsetPublisher, TaskWorkflowPatchsetReader, TaskWorkflowPatchsetSelector,
    TaskWorkflowPolicyEvaluator, TaskWorkflowPolicyReader, TaskWorkflowPolicyWaiverCreator,
    TaskWorkflowRemoteChangeDetailReader, TaskWorkflowRemoteChangeLister,
    TaskWorkflowRemoteChangeReader, TaskWorkflowRemoteTaskCloser, TaskWorkflowRemoteTaskReader,
    TaskWorkflowRepoJobLister, TaskWorkflowReviewLister, TaskWorkflowReviewRecorder,
    TaskWorkflowReviewRequester,
};
use tempfile::tempdir;

#[derive(Debug, Default)]
struct FakeChangeRemote {
    changes: BTreeMap<String, JsonValue>,
    change_requests: Vec<(String, Option<String>)>,
    change_details: BTreeMap<String, JsonValue>,
    change_detail_requests: Vec<(String, Option<String>)>,
    change_rows: Vec<JsonValue>,
}

#[derive(Debug, Default)]
struct FakeWorkflowReadRemote {
    tasks: BTreeMap<String, JsonValue>,
    changes: BTreeMap<String, JsonValue>,
    change_requests: Vec<(String, Option<String>)>,
    change_details: BTreeMap<String, JsonValue>,
    change_detail_requests: Vec<(String, Option<String>)>,
    lines: BTreeMap<String, JsonValue>,
    line_requests: Vec<(String, String)>,
    line_close_requests: Vec<(String, String, String)>,
    close_line_identity_override: Option<String>,
    change_rows: Vec<JsonValue>,
}

#[derive(Debug, Default)]
struct FakeLineRemote {
    lines: BTreeMap<String, JsonValue>,
    line_requests: Vec<(String, String)>,
}

fn fake_task_remote_stats(closed: bool) -> TaskWorkflowHttpClientStats {
    TaskWorkflowHttpClientStats {
        base_url: "https://ait.example".to_string(),
        default_timeout_ms: 30_000,
        retry_attempts: 1,
        retry_backoff_ms: 10,
        pool_max_idle_per_host: 2,
        request_count: 0,
        retry_count: 0,
        closed,
    }
}

impl TaskWorkflowLineReader for FakeLineRemote {
    fn get_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.line_requests
            .push((repo_name.to_string(), line_name.to_string()));
        self.lines.get(line_name).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET line {line_name} failed: 404 Unknown line"
            ))
        })
    }
}

#[derive(Debug, Default)]
struct FakeWorkflowCloseoutRemote {
    patchsets: BTreeMap<String, JsonValue>,
    ci_statuses: BTreeMap<String, JsonValue>,
    attestations: BTreeMap<String, JsonValue>,
    policies: BTreeMap<String, JsonValue>,
    reviews: BTreeMap<String, JsonValue>,
    land_submissions: Vec<JsonValue>,
    requests: Vec<(String, Option<String>, Option<String>)>,
    attestation_requests: Vec<(String, Option<String>, bool)>,
    policy_requests: Vec<(String, Option<String>, bool)>,
    review_requests: Vec<(String, Option<String>, bool)>,
    ci_status_requests: Vec<(String, i64, Option<String>, bool)>,
    ci_readiness_requests: Vec<(String, i64, Option<String>, bool)>,
    repo_job_requests: usize,
}

impl TaskWorkflowHttpClientInspector for FakeWorkflowCloseoutRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        fake_task_remote_stats(false)
    }
}

impl TaskWorkflowHttpClientCloser for FakeWorkflowCloseoutRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        fake_task_remote_stats(true)
    }
}

impl TaskWorkflowMutationReceiptBuilder for FakeWorkflowCloseoutRemote {
    fn mutation_receipt(
        &self,
        _action: &str,
        _source_action: &str,
        _delivery: &str,
        _response_recovery: Option<&JsonValue>,
        _result: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowActionMutationReceiptsBuilder for FakeWorkflowCloseoutRemote {
    fn action_mutation_receipts(
        &self,
        _code: &str,
        _result: &JsonValue,
    ) -> Result<JsonValue, String> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowPatchsetLister for FakeWorkflowCloseoutRemote {
    fn list_patchsets(
        &mut self,
        _change_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowPatchsetReader for FakeWorkflowCloseoutRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.requests.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            change_ref.map(str::to_string),
        ));
        self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })
    }
}

impl TaskWorkflowPatchsetPublisher for FakeWorkflowCloseoutRemote {
    fn publish_patchset(
        &mut self,
        _change_id: &str,
        _base_snapshot_id: &str,
        _revision_snapshot_id: &str,
        _summary: &str,
        _author_mode: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowPatchsetSelector for FakeWorkflowCloseoutRemote {
    fn select_patchset(
        &mut self,
        _change_id: &str,
        _patchset_id: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowPatchsetCiRunner for FakeWorkflowCloseoutRemote {
    fn run_patchset_ci(
        &mut self,
        _patchset_id: &str,
        _trigger: &str,
        _execution_profile: Option<&str>,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowReviewRequester for FakeWorkflowCloseoutRemote {
    fn request_review(
        &mut self,
        _change_id: &str,
        _patchset_id: &str,
        _reviewer_groups: &[String],
        _note: Option<&str>,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakeWorkflowCloseoutRemote {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.ci_status_requests.push((
            patchset_id.to_string(),
            recent_limit,
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.ci_statuses.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset CI status {patchset_id} failed: 404 Unknown patchset CI status"
            ))
        })
    }

    fn read_patchset_ci_readiness(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.ci_readiness_requests.push((
            patchset_id.to_string(),
            recent_limit,
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.ci_statuses.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset CI readiness {patchset_id} failed: 404 Unknown patchset CI readiness"
            ))
        })
    }
}

impl TaskWorkflowRepoJobLister for FakeWorkflowCloseoutRemote {
    fn list_repo_jobs(
        &mut self,
        _repo_name: &str,
        _state: Option<&str>,
        _limit: i64,
        _diagnostics: bool,
        _stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.repo_job_requests += 1;
        Ok(json!([]))
    }
}

impl TaskWorkflowReviewRecorder for FakeWorkflowCloseoutRemote {
    fn record_review(
        &mut self,
        _change_id: &str,
        _patchset_id: &str,
        _reviewer: &str,
        _action: &str,
        _comment: Option<&str>,
        _blocking: bool,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowReviewLister for FakeWorkflowCloseoutRemote {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.review_requests.push((
            change_id.to_string(),
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.reviews.get(change_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET reviews {change_id} failed: 404 Unknown review summary"
            ))
        })
    }
}

impl TaskWorkflowAttestationWriter for FakeWorkflowCloseoutRemote {
    fn put_attestation(
        &mut self,
        _patchset_id: &str,
        _author_mode: &str,
        _evaluation_summary: &JsonValue,
        _provenance_summary: &JsonValue,
        _detail: &JsonValue,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowAttestationReader for FakeWorkflowCloseoutRemote {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.attestation_requests.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.attestations.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET attestation {patchset_id} failed: 404 Unknown attestation"
            ))
        })
    }
}

impl TaskWorkflowPolicyEvaluator for FakeWorkflowCloseoutRemote {
    fn evaluate_policy(
        &mut self,
        _patchset_id: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowPolicyReader for FakeWorkflowCloseoutRemote {
    fn get_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.policy_requests.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.policies.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET policy {patchset_id} failed: 404 Unknown policy"
            ))
        })
    }
}

impl TaskWorkflowPolicyWaiverCreator for FakeWorkflowCloseoutRemote {
    fn create_waiver(
        &mut self,
        _patchset_id: &str,
        _rule_name: &str,
        _reason: &str,
        _expires_at: Option<&str>,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowLandSubmitter for FakeWorkflowCloseoutRemote {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let landed_snapshot_id = patchset_id
            .and_then(|value| self.patchsets.get(value))
            .and_then(|value| string_field(value, "revision_snapshot_id"));
        let submission = json!({
            "submission_id": format!("LAND-{}", self.land_submissions.len() + 1),
            "change_id": change_id,
            "patchset_id": patchset_id,
            "target_line": target_line,
            "mode": mode,
            "repo_name": repo_name,
            "status": "succeeded",
            "result": {
                "target_line": target_line,
                "landed_snapshot_id": landed_snapshot_id,
            }
        });
        self.land_submissions.push(submission.clone());
        Ok(submission)
    }
}

impl TaskWorkflowLandReader for FakeWorkflowCloseoutRemote {
    fn get_land(
        &mut self,
        _submission_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowLandRetryer for FakeWorkflowCloseoutRemote {
    fn retry_land(
        &mut self,
        _submission_id: &str,
        _reason: Option<&str>,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

impl TaskWorkflowRemoteTaskCloser for FakeWorkflowCloseoutRemote {
    fn close_task(
        &mut self,
        _task_id: &str,
        _status: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        unimplemented!("unused by workflow closeout helper tests")
    }
}

#[derive(Debug, Default)]
struct FakeWorkflowRemoteStateReadRemote {
    patchsets: BTreeMap<String, JsonValue>,
    attestations: BTreeMap<String, JsonValue>,
    policies: BTreeMap<String, JsonValue>,
    reviews: BTreeMap<String, JsonValue>,
    ci_statuses: BTreeMap<String, JsonValue>,
    requests: Vec<(String, Option<String>, Option<String>)>,
    attestation_requests: Vec<(String, Option<String>, bool)>,
    policy_requests: Vec<(String, Option<String>, bool)>,
    review_requests: Vec<(String, Option<String>, bool)>,
    ci_status_requests: Vec<(String, i64, Option<String>, bool)>,
}

impl TaskWorkflowPatchsetReader for FakeWorkflowRemoteStateReadRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.requests.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            change_ref.map(str::to_string),
        ));
        self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })
    }
}

impl TaskWorkflowReviewLister for FakeWorkflowRemoteStateReadRemote {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.review_requests.push((
            change_id.to_string(),
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.reviews.get(change_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET reviews {change_id} failed: 404 Unknown review summary"
            ))
        })
    }
}

impl TaskWorkflowAttestationReader for FakeWorkflowRemoteStateReadRemote {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.attestation_requests.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.attestations.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET attestation {patchset_id} failed: 404 Unknown attestation"
            ))
        })
    }
}

impl TaskWorkflowPolicyReader for FakeWorkflowRemoteStateReadRemote {
    fn get_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.policy_requests.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.policies.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET policy {patchset_id} failed: 404 Unknown policy"
            ))
        })
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakeWorkflowRemoteStateReadRemote {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.ci_status_requests.push((
            patchset_id.to_string(),
            recent_limit,
            repo_name.map(str::to_string),
            exact_id,
        ));
        self.ci_statuses.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset CI status {patchset_id} failed: 404 Unknown patchset CI status"
            ))
        })
    }
}

impl TaskWorkflowRepoJobLister for FakeWorkflowRemoteStateReadRemote {
    fn list_repo_jobs(
        &mut self,
        _repo_name: &str,
        _state: Option<&str>,
        _limit: i64,
        _diagnostics: bool,
        _stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!([]))
    }
}

#[derive(Debug, Default)]
struct FakeWorkflowReviewActionRemote {
    patchsets: BTreeMap<String, JsonValue>,
    patchset_requests: Vec<(String, Option<String>, Option<String>)>,
    recorded_reviews: Vec<JsonValue>,
    policy_evaluations: Vec<(String, Option<String>, bool)>,
}

impl TaskWorkflowPatchsetReader for FakeWorkflowReviewActionRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.patchset_requests.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            change_ref.map(str::to_string),
        ));
        self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })
    }
}

impl TaskWorkflowReviewRecorder for FakeWorkflowReviewActionRemote {
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
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let review = json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "action": action,
            "comment": comment,
            "blocking": blocking,
            "repo_name": repo_name,
            "exact_id": exact_id,
        });
        self.recorded_reviews.push(review.clone());
        Ok(review)
    }
}

impl TaskWorkflowPolicyEvaluator for FakeWorkflowReviewActionRemote {
    fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.policy_evaluations.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            exact_id,
        ));
        Ok(json!({
            "patchset_id": patchset_id,
            "decision": "pass",
        }))
    }
}

#[derive(Debug, Default)]
struct FakeWorkflowReviewOnlyActionRemote {
    patchsets: BTreeMap<String, JsonValue>,
    patchset_requests: Vec<(String, Option<String>, Option<String>)>,
    recorded_reviews: Vec<JsonValue>,
}

impl TaskWorkflowPatchsetReader for FakeWorkflowReviewOnlyActionRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.patchset_requests.push((
            patchset_id.to_string(),
            repo_name.map(str::to_string),
            change_ref.map(str::to_string),
        ));
        self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })
    }
}

impl TaskWorkflowReviewRecorder for FakeWorkflowReviewOnlyActionRemote {
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
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let review = json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "action": action,
            "comment": comment,
            "blocking": blocking,
            "repo_name": repo_name,
            "exact_id": exact_id,
        });
        self.recorded_reviews.push(review.clone());
        Ok(review)
    }
}

impl TaskWorkflowRemoteChangeLister for FakeChangeRemote {
    fn list_changes(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.change_rows.clone())
    }
}

impl TaskWorkflowRemoteChangeDetailReader for FakeChangeRemote {
    fn get_change_detail(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.change_detail_requests
            .push((change_ref.to_string(), repo_name.map(str::to_string)));
        self.change_details.get(change_ref).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET change detail {change_ref} failed: 404 Unknown change"
            ))
        })
    }
}

impl TaskWorkflowRemoteChangeReader for FakeChangeRemote {
    fn get_change(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.change_requests
            .push((change_ref.to_string(), repo_name.map(str::to_string)));
        self.changes.get(change_ref).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET change {change_ref} failed: 404 Unknown change"
            ))
        })
    }
}

impl TaskWorkflowRemoteTaskReader for FakeWorkflowReadRemote {
    fn get_task(
        &mut self,
        task_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.tasks.get(task_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET task {task_id} failed: 404 Unknown task"
            ))
        })
    }
}

impl TaskWorkflowRemoteChangeLister for FakeWorkflowReadRemote {
    fn list_changes(&mut self, _repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self.change_rows.clone())
    }
}

impl TaskWorkflowRemoteChangeDetailReader for FakeWorkflowReadRemote {
    fn get_change_detail(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.change_detail_requests
            .push((change_ref.to_string(), repo_name.map(str::to_string)));
        self.change_details.get(change_ref).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET change detail {change_ref} failed: 404 Unknown change"
            ))
        })
    }
}

impl TaskWorkflowRemoteChangeReader for FakeWorkflowReadRemote {
    fn get_change(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.change_requests
            .push((change_ref.to_string(), repo_name.map(str::to_string)));
        self.changes.get(change_ref).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET change {change_ref} failed: 404 Unknown change"
            ))
        })
    }
}

impl TaskWorkflowLineReader for FakeWorkflowReadRemote {
    fn get_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.line_requests
            .push((repo_name.to_string(), line_name.to_string()));
        self.lines.get(line_name).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET line {line_name} failed: 404 Unknown line"
            ))
        })
    }
}

impl TaskWorkflowLineCloser for FakeWorkflowReadRemote {
    fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.line_close_requests.push((
            repo_name.to_string(),
            line_name.to_string(),
            status.to_string(),
        ));
        let line = self.lines.get_mut(line_name).ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "POST close line {line_name} failed: 404 Unknown line"
            ))
        })?;
        line["status"] = JsonValue::String(status.to_string());
        if let Some(line_id) = self.close_line_identity_override.as_ref() {
            line["line_id"] = JsonValue::String(line_id.clone());
        }
        Ok(line.clone())
    }
}

mod land;
mod local;
mod ready_poll;
mod task_land;
mod wait_hint;
