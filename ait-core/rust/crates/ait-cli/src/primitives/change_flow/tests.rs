use super::*;
use ait_core::json_support::json;
use ait_core::task_workflow_http_adapter::{
    TaskWorkflowActionMutationReceiptsBuilder, TaskWorkflowAttestationReader,
    TaskWorkflowAttestationWriter, TaskWorkflowHttpClientCloser, TaskWorkflowHttpClientError,
    TaskWorkflowHttpClientInspector, TaskWorkflowHttpClientResult, TaskWorkflowHttpClientStats,
    TaskWorkflowLandReader, TaskWorkflowLandRetryer, TaskWorkflowLandSubmitter,
    TaskWorkflowMutationReceiptBuilder, TaskWorkflowPatchsetCiRunner,
    TaskWorkflowPatchsetCiStatusReader, TaskWorkflowPatchsetLister, TaskWorkflowPatchsetPublisher,
    TaskWorkflowPatchsetSelector, TaskWorkflowPolicyEvaluator, TaskWorkflowPolicyReader,
    TaskWorkflowPolicyWaiverCreator, TaskWorkflowRemoteTaskCloser, TaskWorkflowRepoJobLister,
    TaskWorkflowReviewLister, TaskWorkflowReviewRecorder, TaskWorkflowReviewRequester,
};
use ait_core::task_workflow_remote_traits::TaskWorkflowPatchsetCiRemote;

#[derive(Debug, Default)]
struct FakeCloseoutRemote {
    patchsets: BTreeMap<String, JsonValue>,
    patchsets_by_change: BTreeMap<String, Vec<JsonValue>>,
    ci_statuses: BTreeMap<String, JsonValue>,
    repo_jobs: JsonValue,
    repo_job_requests: usize,
    reviews: BTreeMap<String, JsonValue>,
    attestations: BTreeMap<String, JsonValue>,
    policies: BTreeMap<String, JsonValue>,
    land_submissions: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Default)]
struct FakePatchsetCiRemote {
    patchsets: BTreeMap<String, JsonValue>,
    ci_statuses: BTreeMap<String, JsonValue>,
    repo_jobs: JsonValue,
    repo_job_requests: usize,
}

#[derive(Debug, Default)]
struct FakePatchsetReaderOnly {
    patchsets: BTreeMap<String, JsonValue>,
}

#[derive(Debug, Default)]
struct FakeReviewRequester;

#[derive(Debug, Default)]
struct FakeReviewRecorder;

#[derive(Debug, Default)]
struct FakeReviewLister;

#[derive(Debug, Default)]
struct FakeAttestationWriter;

#[derive(Debug, Default)]
struct FakeAttestationReader;

#[derive(Debug, Default)]
struct FakePolicyEvaluator;

#[derive(Debug, Default)]
struct FakePolicyReader;

#[derive(Debug, Default)]
struct FakePolicyWaiverCreator;

#[derive(Debug, Default)]
struct FakeLandSubmitter;

#[derive(Debug, Default)]
struct FakeLandReader;

#[derive(Debug, Default)]
struct FakeLandRetryer;

#[derive(Debug, Default)]
struct FakeRemoteTaskCloser;

fn fake_closeout_remote_stats(closed: bool) -> TaskWorkflowHttpClientStats {
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

impl TaskWorkflowRemoteTaskCloser for FakeRemoteTaskCloser {
    fn close_task(
        &mut self,
        task_id: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "task_id": task_id,
            "status": status,
            "repo_name": repo_name,
        }))
    }
}

impl TaskWorkflowReviewRequester for FakeReviewRequester {
    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer_groups": reviewer_groups,
            "note": note,
            "repo_name": repo_name,
            "requested": true,
        }))
    }
}

impl TaskWorkflowReviewRecorder for FakeReviewRecorder {
    fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "action": action,
            "comment": comment,
            "blocking": blocking,
            "repo_name": repo_name,
            "recorded": true,
        }))
    }
}

impl TaskWorkflowReviewLister for FakeReviewLister {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "repo_name": repo_name,
            "reviews": [{"reviewer": "alice", "action": "approve"}],
        }))
    }
}

impl TaskWorkflowAttestationWriter for FakeAttestationWriter {
    fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &JsonValue,
        provenance_summary: &JsonValue,
        detail: &JsonValue,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "author_mode": author_mode,
            "evaluation_summary": evaluation_summary,
            "provenance_summary": provenance_summary,
            "detail": detail,
            "repo_name": repo_name,
        }))
    }
}

impl TaskWorkflowAttestationReader for FakeAttestationReader {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "author_mode": "ai_with_human_review",
        }))
    }
}

impl TaskWorkflowPolicyEvaluator for FakePolicyEvaluator {
    fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "decision": "pass",
        }))
    }
}

impl TaskWorkflowPolicyReader for FakePolicyReader {
    fn get_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "evaluated": true,
        }))
    }
}

impl TaskWorkflowPolicyWaiverCreator for FakePolicyWaiverCreator {
    fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "rule_name": rule_name,
            "reason": reason,
            "expires_at": expires_at,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "waived": true,
        }))
    }
}

impl TaskWorkflowLandSubmitter for FakeLandSubmitter {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": format!("RLS-{change_id}"),
            "change_id": change_id,
            "patchset_id": patchset_id,
            "target_line": target_line,
            "mode": mode,
            "repo_name": repo_name,
            "submitted": true
        }))
    }
}

impl TaskWorkflowLandReader for FakeLandReader {
    fn get_land(
        &mut self,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": submission_id,
            "repo_name": repo_name,
            "status": "queued",
        }))
    }
}

impl TaskWorkflowLandRetryer for FakeLandRetryer {
    fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": submission_id,
            "reason": reason,
            "repo_name": repo_name,
            "retried": true,
        }))
    }
}

impl TaskWorkflowHttpClientInspector for FakeCloseoutRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        fake_closeout_remote_stats(false)
    }
}

impl TaskWorkflowHttpClientCloser for FakeCloseoutRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        fake_closeout_remote_stats(true)
    }
}

impl TaskWorkflowMutationReceiptBuilder for FakeCloseoutRemote {
    fn mutation_receipt(
        &self,
        _action: &str,
        _source_action: &str,
        _delivery: &str,
        _response_recovery: Option<&JsonValue>,
        _result: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        unimplemented!("unused by patchset closeout helper tests")
    }
}

impl TaskWorkflowActionMutationReceiptsBuilder for FakeCloseoutRemote {
    fn action_mutation_receipts(
        &self,
        _code: &str,
        _result: &JsonValue,
    ) -> Result<JsonValue, String> {
        unimplemented!("unused by patchset closeout helper tests")
    }
}

impl TaskWorkflowPatchsetLister for FakeCloseoutRemote {
    fn list_patchsets(
        &mut self,
        change_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(self
            .patchsets_by_change
            .get(change_id)
            .cloned()
            .unwrap_or_default())
    }
}

impl TaskWorkflowPatchsetReader for FakeCloseoutRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        _repo_name: Option<&str>,
        _change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })
    }
}

impl TaskWorkflowPatchsetPublisher for FakeCloseoutRemote {
    fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let patchset_number = self
            .patchsets_by_change
            .get(change_id)
            .map(|rows| rows.len() + 1)
            .unwrap_or(1);
        let patchset_id = format!("RCP-{change_id}-{patchset_number}");
        let patchset = json!({
            "patchset_id": patchset_id.clone(),
            "patchset_number": patchset_number,
            "change_id": change_id,
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "author_mode": author_mode,
            "repo_name": repo_name
        });
        self.patchsets.insert(patchset_id, patchset.clone());
        self.patchsets_by_change
            .entry(change_id.to_string())
            .or_default()
            .push(patchset.clone());
        Ok(patchset)
    }
}

impl TaskWorkflowPatchsetSelector for FakeCloseoutRemote {
    fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let patchset = self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "POST select patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })?;
        if string_field(&patchset, "change_id").as_deref() != Some(change_id) {
            return Err(TaskWorkflowHttpClientError::Remote(format!(
                "Patchset {patchset_id} does not belong to change {change_id}"
            )));
        }
        Ok(json!({
            "change_id": change_id,
            "selected_patchset_id": patchset_id,
            "patchset": patchset
        }))
    }
}

impl TaskWorkflowPatchsetCiRunner for FakeCloseoutRemote {
    fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let patchset = self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "POST patchset CI {patchset_id} failed: 404 Unknown patchset"
            ))
        })?;
        Ok(json!({
            "patchset_id": patchset_id,
            "change_id": string_field(&patchset, "change_id"),
            "trigger": trigger,
            "execution_profile": execution_profile,
            "repo_name": repo_name,
            "queued": true
        }))
    }
}

impl TaskWorkflowReviewRequester for FakeCloseoutRemote {
    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer_groups": reviewer_groups,
            "note": note,
            "repo_name": repo_name,
            "requested": true
        }))
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakeCloseoutRemote {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        _recent_limit: i64,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.ci_statuses.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset CI status {patchset_id} failed: 404 Unknown status"
            ))
        })
    }
}

impl TaskWorkflowRepoJobLister for FakeCloseoutRemote {
    fn list_repo_jobs(
        &mut self,
        _repo_name: &str,
        _state: Option<&str>,
        _limit: i64,
        _diagnostics: bool,
        _stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.repo_job_requests += 1;
        Ok(self.repo_jobs.clone())
    }
}

impl TaskWorkflowReviewRecorder for FakeCloseoutRemote {
    fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "action": action,
            "comment": comment,
            "blocking": blocking,
            "repo_name": repo_name,
            "recorded": true
        }))
    }
}

impl TaskWorkflowReviewLister for FakeCloseoutRemote {
    fn list_reviews(
        &mut self,
        change_id: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(self
            .reviews
            .get(change_id)
            .cloned()
            .unwrap_or_else(|| json!({"reviews": []})))
    }
}

impl TaskWorkflowAttestationWriter for FakeCloseoutRemote {
    fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &JsonValue,
        provenance_summary: &JsonValue,
        detail: &JsonValue,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let payload = json!({
            "patchset_id": patchset_id,
            "author_mode": author_mode,
            "evaluation_summary": evaluation_summary,
            "provenance_summary": provenance_summary,
            "detail": detail,
            "repo_name": repo_name
        });
        self.attestations
            .insert(patchset_id.to_string(), payload.clone());
        Ok(payload)
    }
}

impl TaskWorkflowAttestationReader for FakeCloseoutRemote {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.attestations.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET attestation {patchset_id} failed: 404 Unknown attestation"
            ))
        })
    }
}

impl TaskWorkflowPolicyEvaluator for FakeCloseoutRemote {
    fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let policy = json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "decision": "pass",
            "evaluated": true
        });
        self.policies
            .insert(patchset_id.to_string(), policy.clone());
        Ok(policy)
    }
}

impl TaskWorkflowPolicyReader for FakeCloseoutRemote {
    fn get_policy(
        &mut self,
        patchset_id: &str,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.policies.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET policy {patchset_id} failed: 404 Unknown policy"
            ))
        })
    }
}

impl TaskWorkflowPolicyWaiverCreator for FakeCloseoutRemote {
    fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "rule_name": rule_name,
            "reason": reason,
            "expires_at": expires_at,
            "repo_name": repo_name,
            "waived": true
        }))
    }
}

impl TaskWorkflowLandSubmitter for FakeCloseoutRemote {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let submission_id = format!("RLS-{change_id}");
        let payload = json!({
            "submission_id": submission_id,
            "change_id": change_id,
            "patchset_id": patchset_id,
            "target_line": target_line,
            "mode": mode,
            "repo_name": repo_name,
            "submitted": true
        });
        self.land_submissions.insert(submission_id, payload.clone());
        Ok(payload)
    }
}

impl TaskWorkflowLandReader for FakeCloseoutRemote {
    fn get_land(
        &mut self,
        submission_id: &str,
        _repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.land_submissions
            .get(submission_id)
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "GET land {submission_id} failed: 404 Unknown land submission"
                ))
            })
    }
}

impl TaskWorkflowLandRetryer for FakeCloseoutRemote {
    fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        let mut payload = self
            .land_submissions
            .get(submission_id)
            .cloned()
            .ok_or_else(|| {
                TaskWorkflowHttpClientError::Remote(format!(
                    "POST retry land {submission_id} failed: 404 Unknown land submission"
                ))
            })?;
        if let Some(map) = payload.as_object_mut() {
            map.insert("reason".to_string(), json!(reason));
            map.insert("repo_name".to_string(), json!(repo_name));
            map.insert("retried".to_string(), json!(true));
        }
        self.land_submissions
            .insert(submission_id.to_string(), payload.clone());
        Ok(payload)
    }
}

impl TaskWorkflowRemoteTaskCloser for FakeCloseoutRemote {
    fn close_task(
        &mut self,
        task_id: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "task_id": task_id,
            "status": status,
            "repo_name": repo_name,
            "closed": true
        }))
    }
}

impl TaskWorkflowPatchsetReader for FakePatchsetCiRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        _repo_name: Option<&str>,
        _change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })
    }
}

impl TaskWorkflowPatchsetReader for FakePatchsetReaderOnly {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        _repo_name: Option<&str>,
        _change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.patchsets.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset {patchset_id} failed: 404 Unknown patchset"
            ))
        })
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakePatchsetCiRemote {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        _recent_limit: i64,
        _repo_name: Option<&str>,
        _exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.ci_statuses.get(patchset_id).cloned().ok_or_else(|| {
            TaskWorkflowHttpClientError::Remote(format!(
                "GET patchset CI status {patchset_id} failed: 404 Unknown status"
            ))
        })
    }
}

impl TaskWorkflowRepoJobLister for FakePatchsetCiRemote {
    fn list_repo_jobs(
        &mut self,
        _repo_name: &str,
        _state: Option<&str>,
        _limit: i64,
        _diagnostics: bool,
        _stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        self.repo_job_requests += 1;
        Ok(self.repo_jobs.clone())
    }
}

fn unused_repo_runtime() -> RepoRuntime {
    RepoRuntime {
        root: PathBuf::from("/tmp/ait-closeout-helper-test"),
        ait_dir: PathBuf::from("/tmp/ait-closeout-helper-test/.ait"),
        config: JsonMap::new(),
        worktree_config_path: None,
    }
}

#[test]
fn legacy_repo_patchset_alias_resolves_remote_namespace_drift() {
    assert_eq!(
        legacy_repo_patchset_alias("P-RCC-0083-1").as_deref(),
        Some("RCP-0083-1")
    );
    assert_eq!(
        legacy_repo_patchset_alias("P-LCC-0012-1").as_deref(),
        Some("LCP-0012-1")
    );
    assert_eq!(legacy_repo_patchset_alias("P-CC-0217-1"), None);
}

#[test]
fn patchset_closeout_helpers_accept_closeout_remote_trait() {
    let mut remote = FakeCloseoutRemote {
        patchsets: BTreeMap::from([
            (
                "RCP-0083-1".to_string(),
                json!({
                    "patchset_id": "RCP-0083-1",
                    "change_id": "RCC-0083"
                }),
            ),
            (
                "RCP-0099-1".to_string(),
                json!({
                    "patchset_id": "RCP-0099-1",
                    "change_id": "RCC-0099"
                }),
            ),
        ]),
        patchsets_by_change: BTreeMap::from([(
            "RCC-0083".to_string(),
            vec![
                json!({"patchset_id": "RCP-0083-0", "patchset_number": 0}),
                json!({"patchset_id": "RCP-0083-1", "patchset_number": 1}),
            ],
        )]),
        ..Default::default()
    };

    let aliased = get_patchset_for_ci_status(&mut remote, "P-RCC-0083-1", "fixture-ait")
        .expect("legacy patchset alias");
    assert_eq!(aliased["patchset_id"], json!("RCP-0083-1"));

    assert_eq!(
        resolve_patchset_id(&mut remote, "RCP-0099-1", Some("fixture-ait"))
            .expect("direct patchset id"),
        "RCP-0099-1"
    );

    assert_eq!(
        resolve_patchset_argument(
            &unused_repo_runtime(),
            &mut remote,
            Some("RCP-0083-1"),
            Some("RCC-0083"),
            Some("fixture-ait"),
            None,
        )
        .expect("explicit patchset argument"),
        "RCP-0083-1"
    );
}

#[test]
fn patchset_ci_status_patchset_read_accepts_closeout_remote_trait() {
    let mut remote = FakeCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "change_id": "RCC-0083"
            }),
        )]),
        ..Default::default()
    };

    let direct = patchset_ci_status_patchset_read_with_closeout_remote(
        &mut remote,
        "RCP-0083-1",
        "fixture-ait",
    )
    .expect("direct patchset read");
    assert_eq!(direct["patchset_id"], json!("RCP-0083-1"));

    let aliased = patchset_ci_status_patchset_read_with_closeout_remote(
        &mut remote,
        "P-RCC-0083-1",
        "fixture-ait",
    )
    .expect("legacy patchset alias");
    assert_eq!(aliased["patchset_id"], json!("RCP-0083-1"));

    let err = patchset_ci_status_patchset_read_with_closeout_remote(
        &mut remote,
        "P-RCC-MISSING-1",
        "fixture-ait",
    )
    .expect_err("missing aliased patchset should fail");
    assert!(err.contains("GET patchset P-RCC-MISSING-1 failed"));
}

#[test]
fn patchset_ci_status_patchset_read_accepts_patchset_reader_trait() {
    let mut remote = FakePatchsetReaderOnly {
        patchsets: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "change_id": "RCC-0083"
            }),
        )]),
    };

    let aliased = patchset_ci_status_patchset_read_with_closeout_remote(
        &mut remote,
        "P-RCC-0083-1",
        "fixture-ait",
    )
    .expect("legacy patchset alias");
    assert_eq!(aliased["patchset_id"], json!("RCP-0083-1"));
}

#[test]
fn patchset_ci_readiness_accepts_bounded_contract_without_repo_job_scan() {
    let mut remote = FakePatchsetCiRemote {
        patchsets: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "change_id": "RCC-0083"
            }),
        )]),
        ci_statuses: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "contract": "ait.server.patchset_ci.readiness.v1",
                "projection": "readiness",
                "patchset_id": "RCP-0083-1",
                "change_id": "RCC-0083",
                "repo_name": "fixture-ait",
                "available": true,
                "tests_status": "pass",
                "selected_suite_ids": ["rust_core"],
                "suite_result_count": 1,
                "blocking_failure_count": 0,
                "has_runnable_evidence": true,
                "recent_limit_applied": 10,
                "latest_job": {"job_id": 97, "job_type": "patchset.ci", "state": "succeeded"},
                "recent_jobs": []
            }),
        )]),
        repo_jobs: json!([{"result": {"large": "must not be read"}}]),
        repo_job_requests: 0,
    };

    let readiness =
        patchset_ci_readiness_with_closeout_remote(&mut remote, "P-RCC-0083-1", "fixture-ait", 10)
            .expect("bounded patchset CI readiness");

    assert_eq!(readiness["projection"], json!("readiness"));
    assert_eq!(readiness["tests_status"], json!("pass"));
    assert_eq!(remote.repo_job_requests, 0);
}

#[test]
fn patchset_ci_status_accepts_patchset_ci_remote_trait() {
    let mut remote = FakePatchsetCiRemote {
        patchsets: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "change_id": "RCC-0083"
            }),
        )]),
        ci_statuses: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "tests_status": "pending",
                "latest_job": null,
                "recent_jobs": []
            }),
        )]),
        repo_jobs: json!([
            {
                "job_id": 101,
                "job_type": "patchset.ci",
                "state": "succeeded",
                "payload": {
                    "patchset_id": "RCP-0083-1",
                    "change_id": "RCC-0083",
                    "repo_name": "fixture-ait"
                },
                "result": {
                    "tests_status": "pass",
                    "blocking_failures": []
                }
            }
        ]),
        repo_job_requests: 0,
    };
    let remote_port: &mut dyn TaskWorkflowPatchsetCiRemote = &mut remote;

    let status =
        patchset_ci_status_with_closeout_remote(remote_port, "P-RCC-0083-1", "fixture-ait", 10)
            .expect("patchset ci status");

    assert_eq!(status["patchset_id"], json!("RCP-0083-1"));
    assert_eq!(status["tests_status"], json!("pending"));
    assert!(status.get("ci_status_source").is_none());
    assert_eq!(remote.repo_job_requests, 0);
}

#[test]
fn patchset_ci_readiness_rejects_incomplete_contract_without_repo_job_scan() {
    let mut remote = FakePatchsetCiRemote {
        patchsets: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "change_id": "RCC-0083"
            }),
        )]),
        ci_statuses: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "tests_status": "pending",
                "latest_job": null,
                "recent_jobs": []
            }),
        )]),
        repo_jobs: json!([{"result": {"large": "must not be read"}}]),
        repo_job_requests: 0,
    };

    let error =
        patchset_ci_readiness_with_closeout_remote(&mut remote, "P-RCC-0083-1", "fixture-ait", 10)
            .expect_err("incomplete readiness must fail closed");

    assert!(error.contains("missing non-empty contract"));
    assert_eq!(remote.repo_job_requests, 0);
}

#[test]
fn patchset_ci_status_respects_explicit_unavailable_without_repo_job_scan() {
    let mut remote = FakeCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "change_id": "RCC-0083"
            }),
        )]),
        ci_statuses: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "available": false,
                "tests_status": "pending",
                "latest_job": null,
                "recent_jobs": []
            }),
        )]),
        repo_jobs: json!([
            {
                "job_id": 97,
                "job_type": "patchset.ci",
                "state": "succeeded",
                "payload": {
                    "patchset_id": "RCP-0083-1",
                    "change_id": "RCC-0083",
                    "repo_name": "fixture-ait"
                },
                "result": {
                    "tests_status": "pass",
                    "blocking_failures": []
                }
            }
        ]),
        ..Default::default()
    };

    let status =
        patchset_ci_status_with_closeout_remote(&mut remote, "P-RCC-0083-1", "fixture-ait", 10)
            .expect("patchset ci status");

    assert_eq!(status["available"], json!(false));
    assert_eq!(status["patchset_id"], json!("RCP-0083-1"));
    assert_eq!(status["tests_status"], json!("pending"));
    assert!(status.get("ci_status_source").is_none());
}

#[test]
fn patchset_command_helpers_accept_closeout_remote_trait() {
    let mut remote = FakeCloseoutRemote {
        patchsets: BTreeMap::from([
            (
                "RCP-0083-1".to_string(),
                json!({
                    "patchset_id": "RCP-0083-1",
                    "change_id": "RCC-0083",
                    "patchset_number": 1
                }),
            ),
            (
                "RCP-0083-2".to_string(),
                json!({
                    "patchset_id": "RCP-0083-2",
                    "change_id": "RCC-0083",
                    "patchset_number": 2
                }),
            ),
        ]),
        patchsets_by_change: BTreeMap::from([(
            "RCC-0083".to_string(),
            vec![
                json!({"patchset_id": "RCP-0083-1", "patchset_number": 1}),
                json!({"patchset_id": "RCP-0083-2", "patchset_number": 2}),
            ],
        )]),
        ..Default::default()
    };

    let listed = patchset_list_with_closeout_remote(&mut remote, "RCC-0083", Some("fixture-ait"))
        .expect("patchset list");
    assert_eq!(listed.as_array().expect("list array").len(), 2);

    let shown = patchset_show_with_closeout_remote(
        &mut remote,
        "RCP-0083-2",
        Some("fixture-ait"),
        Some("RCC-0083"),
    )
    .expect("patchset show");
    assert_eq!(shown["patchset_number"], json!(2));

    let selected =
        patchset_select_with_closeout_remote(&mut remote, "RCC-0083", "RCP-0083-2", "fixture-ait")
            .expect("patchset select");
    assert_eq!(selected["selected_patchset_id"], json!("RCP-0083-2"));
}

#[test]
fn patchset_publish_helper_accepts_closeout_remote_trait() {
    let mut remote = FakeCloseoutRemote::default();

    let patchset = patchset_publish_with_closeout_remote(
        &mut remote,
        "RCC-0088",
        "SNP-BASE",
        "SNP-REV",
        "trait publish",
        "ai_with_human_review",
        "fixture-ait",
    )
    .expect("publish patchset");
    assert_eq!(patchset["change_id"], json!("RCC-0088"));
    assert_eq!(patchset["base_snapshot_id"], json!("SNP-BASE"));
    assert_eq!(patchset["revision_snapshot_id"], json!("SNP-REV"));
    assert_eq!(patchset["summary"], json!("trait publish"));

    let patchset_id = string_field(&patchset, "patchset_id").expect("patchset id");
    let shown =
        patchset_show_with_closeout_remote(&mut remote, &patchset_id, Some("fixture-ait"), None)
            .expect("show published patchset");
    assert_eq!(shown["author_mode"], json!("ai_with_human_review"));
}

#[test]
fn patchset_publish_helper_rejects_empty_patchsets_before_remote_publish() {
    let mut remote = FakeCloseoutRemote::default();

    let err = patchset_publish_with_closeout_remote(
        &mut remote,
        "RCC-0089",
        "SNP-SAME",
        "SNP-SAME",
        "empty publish",
        "ai_with_human_review",
        "fixture-ait",
    )
    .expect_err("empty patchset should be rejected");

    assert!(err.contains("Empty patchsets are prohibited"));
    assert!(remote.patchsets.is_empty());
}

#[test]
fn patchset_ci_run_and_review_helpers_accept_closeout_remote_trait() {
    let mut remote = FakeCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-0083-1".to_string(),
            json!({
                "patchset_id": "RCP-0083-1",
                "change_id": "RCC-0083"
            }),
        )]),
        reviews: BTreeMap::from([(
            "RCC-0083".to_string(),
            json!({
                "change_id": "RCC-0083",
                "reviews": [
                    {
                        "reviewer": "alice",
                        "action": "approve"
                    }
                ]
            }),
        )]),
        ..Default::default()
    };

    let ci = patchset_run_ci_with_closeout_remote(
        &mut remote,
        "RCP-0083-1",
        "manual_rerun",
        Some("full"),
        "fixture-ait",
    )
    .expect("run patchset ci");
    assert_eq!(ci["queued"], json!(true));
    assert_eq!(ci["execution_profile"], json!("full"));

    let groups = vec!["core".to_string(), "release".to_string()];
    let requested = review_request_with_closeout_remote(
        &mut remote,
        "RCC-0083",
        "RCP-0083-1",
        &groups,
        Some("please review"),
        "fixture-ait",
    )
    .expect("request review");
    assert_eq!(requested["reviewer_groups"], json!(["core", "release"]));

    let recorded = review_record_with_closeout_remote(
        &mut remote,
        "RCC-0083",
        "RCP-0083-1",
        "alice",
        "approve",
        Some("looks good"),
        false,
        "fixture-ait",
    )
    .expect("record review");
    assert_eq!(recorded["recorded"], json!(true));
    assert_eq!(recorded["reviewer"], json!("alice"));

    let shown = review_show_with_closeout_remote(&mut remote, "RCC-0083", "fixture-ait")
        .expect("review show");
    assert_eq!(shown["reviews"][0]["reviewer"], json!("alice"));
}

#[test]
fn review_helpers_accept_single_capability_ports() {
    let mut requester = FakeReviewRequester;
    let mut recorder = FakeReviewRecorder;
    let mut lister = FakeReviewLister;
    let groups = vec!["core".to_string(), "release".to_string()];

    let requested = review_request_with_closeout_remote(
        &mut requester,
        "RCC-0083",
        "RCP-0083-1",
        &groups,
        Some("please review"),
        "fixture-ait",
    )
    .expect("request review");
    assert_eq!(requested["reviewer_groups"], json!(["core", "release"]));

    let recorded = review_record_with_closeout_remote(
        &mut recorder,
        "RCC-0083",
        "RCP-0083-1",
        "alice",
        "approve",
        Some("looks good"),
        false,
        "fixture-ait",
    )
    .expect("record review");
    assert_eq!(recorded["recorded"], json!(true));
    assert_eq!(recorded["action"], json!("approve"));

    let shown = review_show_with_closeout_remote(&mut lister, "RCC-0083", "fixture-ait")
        .expect("review show");
    assert_eq!(shown["reviews"][0]["reviewer"], json!("alice"));
}

#[test]
fn attestation_policy_helpers_accept_closeout_remote_trait() {
    let mut remote = FakeCloseoutRemote::default();
    let evaluation = json!({
        "tests": "pass",
        "lint": "pass"
    });
    let provenance = json!({
        "source": "test"
    });
    let detail = json!({
        "evidence": ["unit"]
    });

    let attestation = attestation_put_with_closeout_remote(
        &mut remote,
        "RCP-0083-1",
        "ai_with_human_review",
        &evaluation,
        &provenance,
        &detail,
        "fixture-ait",
    )
    .expect("put attestation");
    assert_eq!(attestation["evaluation_summary"]["tests"], json!("pass"));

    let shown = attestation_show_with_closeout_remote(&mut remote, "RCP-0083-1", "fixture-ait")
        .expect("show attestation");
    assert_eq!(shown["author_mode"], json!("ai_with_human_review"));

    let evaluated = policy_eval_with_closeout_remote(&mut remote, "RCP-0083-1", "fixture-ait")
        .expect("evaluate policy");
    assert_eq!(evaluated["decision"], json!("pass"));

    let policy = policy_show_with_closeout_remote(&mut remote, "RCP-0083-1", "fixture-ait")
        .expect("show policy");
    assert_eq!(policy["evaluated"], json!(true));

    let waiver = policy_waive_with_closeout_remote(
        &mut remote,
        "RCP-0083-1",
        "ci.required",
        "temporary maintenance",
        Some("2026-12-31T00:00:00Z"),
        "fixture-ait",
    )
    .expect("create waiver");
    assert_eq!(waiver["waived"], json!(true));
    assert_eq!(waiver["rule_name"], json!("ci.required"));
}

#[test]
fn policy_helpers_accept_single_capability_ports() {
    let mut evaluator = FakePolicyEvaluator;
    let mut reader = FakePolicyReader;
    let mut waiver_creator = FakePolicyWaiverCreator;

    let evaluated = policy_eval_with_closeout_remote(&mut evaluator, "RCP-0083-1", "fixture-ait")
        .expect("evaluate policy through evaluator port");
    assert_eq!(evaluated["decision"], json!("pass"));

    let policy = policy_show_with_closeout_remote(&mut reader, "RCP-0083-1", "fixture-ait")
        .expect("show policy through reader port");
    assert_eq!(policy["evaluated"], json!(true));

    let waiver = policy_waive_with_closeout_remote(
        &mut waiver_creator,
        "RCP-0083-1",
        "ci.required",
        "temporary maintenance",
        Some("2026-12-31T00:00:00Z"),
        "fixture-ait",
    )
    .expect("create waiver through waiver-creator port");
    assert_eq!(waiver["waived"], json!(true));
    assert_eq!(waiver["rule_name"], json!("ci.required"));
}

#[test]
fn attestation_helpers_accept_single_capability_ports() {
    let mut writer = FakeAttestationWriter;
    let mut reader = FakeAttestationReader;
    let evaluation = json!({
        "tests": "pass",
        "lint": "pass"
    });
    let provenance = json!({
        "source": "test"
    });
    let detail = json!({
        "evidence": ["unit"]
    });

    let attestation = attestation_put_with_closeout_remote(
        &mut writer,
        "RCP-0083-1",
        "ai_with_human_review",
        &evaluation,
        &provenance,
        &detail,
        "fixture-ait",
    )
    .expect("put attestation");
    assert_eq!(attestation["evaluation_summary"]["tests"], json!("pass"));

    let shown = attestation_show_with_closeout_remote(&mut reader, "RCP-0083-1", "fixture-ait")
        .expect("show attestation");
    assert_eq!(shown["author_mode"], json!("ai_with_human_review"));
}

#[test]
fn task_closeout_helpers_accept_closeout_remote_trait() {
    let mut remote = FakeCloseoutRemote::default();

    let closed =
        task_close_with_closeout_remote(&mut remote, "RT-0083", "abandoned", "fixture-ait")
            .expect("close task");
    assert_eq!(closed["task_id"], json!("RT-0083"));
    assert_eq!(closed["status"], json!("abandoned"));

    let completed = task_complete_with_closeout_remote(&mut remote, "RT-0083", "fixture-ait")
        .expect("complete task");
    assert_eq!(completed["status"], json!("completed"));
}

#[test]
fn task_close_helpers_accept_single_capability_port() {
    let mut closer = FakeRemoteTaskCloser;

    let closed =
        task_close_with_closeout_remote(&mut closer, "RT-0083", "abandoned", "fixture-ait")
            .expect("close task");
    assert_eq!(closed["task_id"], json!("RT-0083"));
    assert_eq!(closed["status"], json!("abandoned"));

    let completed = task_complete_with_closeout_remote(&mut closer, "RT-0083", "fixture-ait")
        .expect("complete task");
    assert_eq!(completed["status"], json!("completed"));
}

#[test]
fn land_closeout_helpers_accept_closeout_remote_trait() {
    let mut remote = FakeCloseoutRemote::default();

    let submitted = land_submit_with_closeout_remote(
        &mut remote,
        "RCC-0085",
        Some("RCP-0085-1"),
        "main",
        "merge",
        "fixture-ait",
    )
    .expect("submit land");
    assert_eq!(submitted["change_id"], json!("RCC-0085"));
    assert_eq!(submitted["patchset_id"], json!("RCP-0085-1"));

    let submission_id = string_field(&submitted, "submission_id").expect("submission id");
    let shown = land_show_with_closeout_remote(&mut remote, &submission_id, "fixture-ait")
        .expect("show land");
    assert_eq!(shown["submission_id"], json!(submission_id));
    assert_eq!(shown["target_line"], json!("main"));

    let retried = land_retry_with_closeout_remote(
        &mut remote,
        &submission_id,
        Some("operator requested"),
        "fixture-ait",
    )
    .expect("retry land");
    assert_eq!(retried["submission_id"], json!(submission_id));
    assert_eq!(retried["retried"], json!(true));
    assert_eq!(retried["reason"], json!("operator requested"));
}

#[test]
fn land_helpers_accept_single_capability_ports() {
    let mut submitter = FakeLandSubmitter;
    let mut reader = FakeLandReader;
    let mut retryer = FakeLandRetryer;

    let submitted = land_submit_with_closeout_remote(
        &mut submitter,
        "RCC-0085",
        Some("RCP-0085-1"),
        "main",
        "merge",
        "fixture-ait",
    )
    .expect("submit land through submitter port");
    assert_eq!(submitted["change_id"], json!("RCC-0085"));
    assert_eq!(submitted["submitted"], json!(true));

    let shown = land_show_with_closeout_remote(&mut reader, "RLS-RCC-0085", "fixture-ait")
        .expect("show land through reader port");
    assert_eq!(shown["submission_id"], json!("RLS-RCC-0085"));
    assert_eq!(shown["status"], json!("queued"));

    let retried = land_retry_with_closeout_remote(
        &mut retryer,
        "RLS-RCC-0085",
        Some("operator requested"),
        "fixture-ait",
    )
    .expect("retry land through retryer port");
    assert_eq!(retried["submission_id"], json!("RLS-RCC-0085"));
    assert_eq!(retried["retried"], json!(true));
}
