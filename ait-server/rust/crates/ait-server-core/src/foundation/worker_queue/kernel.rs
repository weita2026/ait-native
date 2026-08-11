use super::in_memory::InMemoryWorkerQueuePool;
use super::rows::{
    compact_worker_queue_index_row, compact_worker_queue_readiness_row,
    compact_worker_queue_summary_row, count_jobs_by, i64_field, job_id_i64, object_rows,
    optional_bool, optional_i64, optional_text, parse_job_id, postgres_timestamptz, repo_matches,
    retry_at_from_now, row_bool, row_i64, row_text, shape_job_rows, text_field,
};
use super::scheduler_projection::{
    is_scheduler_ci_job, scheduler_job_json, scheduler_policy_json, scheduler_queued_jobs,
    scheduler_running_jobs,
};
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerQueueReclaimSummary {
    pub stale_count: usize,
    pub requeued_job_ids: Vec<i64>,
    pub failed_job_ids: Vec<i64>,
    pub superseded_job_ids: Vec<i64>,
    pub reconciled_queued_job_ids: Vec<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerQueueClaimCapabilities {
    pub accepted_job_types: Vec<String>,
    pub accepted_runtime_contracts: Vec<String>,
    pub excluded_runtime_contracts: Vec<String>,
}

pub trait WorkerQueueConnection {
    fn active_duplicate_job_row(
        &mut self,
        repo_name: &str,
        repo_id: Option<&str>,
        job_type: &str,
        payload_json: &str,
        patchset_dedupe_payload: Option<&JsonMap<String, JsonValue>>,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String>;
    fn insert_job(
        &mut self,
        repo_name: &str,
        repo_id: Option<&str>,
        job_type: &str,
        payload_json: &str,
        patchset_dedupe_payload: Option<&JsonMap<String, JsonValue>>,
        max_attempts: i64,
        available_at: &str,
        now: &str,
    ) -> Result<JsonMap<String, JsonValue>, String>;
    fn queued_job_rows(
        &mut self,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String>;
    fn running_job_rows(
        &mut self,
        repo_name: Option<&str>,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String>;
    fn job_row(&mut self, job_id: i64) -> Result<Option<JsonMap<String, JsonValue>>, String>;
    fn list_job_rows(
        &mut self,
        repo_name: Option<&str>,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String>;
    fn list_job_summary_rows(
        &mut self,
        repo_name: Option<&str>,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        self.list_job_rows(repo_name, state, limit)?
            .iter()
            .map(compact_worker_queue_index_row)
            .collect()
    }
    fn list_patchset_ci_job_rows(
        &mut self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String>;
    fn list_patchset_ci_status_job_rows(
        &mut self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        self.list_patchset_ci_job_rows(repo_name, patchset_id, state, limit)?
            .iter()
            .map(compact_worker_queue_summary_row)
            .collect()
    }
    fn list_patchset_ci_readiness_job_rows(
        &mut self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        self.list_patchset_ci_job_rows(repo_name, patchset_id, state, limit)?
            .iter()
            .map(compact_worker_queue_readiness_row)
            .collect()
    }
    fn mark_running(
        &mut self,
        job_id: i64,
        worker_id: &str,
        now: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String>;
    fn renew_lease(
        &mut self,
        job_id: i64,
        worker_id: &str,
        now: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String>;
    fn mark_attached(
        &mut self,
        job_id: i64,
        active_job_id: &str,
        singleflight_key: &str,
        now: &str,
    ) -> Result<bool, String>;
    fn mark_succeeded(
        &mut self,
        job_id: i64,
        result: &JsonValue,
        now: &str,
        required_worker_id: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String>;
    fn mark_failed_or_retry(
        &mut self,
        job_id: i64,
        error: &str,
        retryable: bool,
        retry_available_at: Option<&str>,
        now: &str,
        required_worker_id: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String>;
    fn reconcile_superseded_patchset_ci(
        &mut self,
        repo_name: Option<&str>,
        patchset_id: Option<&str>,
        now: &str,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String>;
    fn reclaim_stale(
        &mut self,
        stale_cutoff: &str,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<WorkerQueueReclaimSummary, String>;
    fn commit(&mut self) -> Result<(), String> {
        Ok(())
    }
}

pub trait WorkerQueueConnectionPool: Clone {
    type Connection: WorkerQueueConnection;

    fn checkout(&self) -> Result<Self::Connection, String>;
}

#[derive(Clone)]
pub struct WorkerQueueKernel<P: WorkerQueueConnectionPool> {
    pool: P,
    policy: SchedulerPolicy,
}

impl<P: WorkerQueueConnectionPool> WorkerQueueKernel<P> {
    pub fn new(pool: P, policy: SchedulerPolicy) -> Self {
        Self { pool, policy }
    }

    pub fn policy(&self) -> SchedulerPolicy {
        self.policy.clone()
    }

    pub fn enqueue_job(
        &self,
        repo_name: &str,
        repo_id: Option<&str>,
        job_type: &str,
        payload: &JsonValue,
        available_at: Option<&str>,
        max_attempts: Option<i64>,
        dedupe_active: bool,
        now: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let payload = payload
            .as_object()
            .ok_or_else(|| "job payload must be a JSON object.".to_string())?;
        let normalized = normalize_async_job_payload(job_type, payload)?;
        let patchset_dedupe_payload = (job_type == "patchset.ci").then_some(&normalized);
        let payload_json = serde_json::to_string(&normalized)
            .map_err(|error| format!("Could not encode {job_type} payload: {error}"))?;
        let mut conn = self.pool.checkout()?;
        if dedupe_active {
            let duplicate = {
                conn.active_duplicate_job_row(
                    repo_name,
                    repo_id,
                    job_type,
                    &payload_json,
                    patchset_dedupe_payload,
                )?
            };
            if let Some(row) = duplicate {
                let mut job = row_to_job(&row)?;
                job.insert("deduplicated".to_string(), json!(true));
                return Ok(job);
            }
        }
        let row = {
            conn.insert_job(
                repo_name,
                repo_id,
                job_type,
                &payload_json,
                patchset_dedupe_payload,
                max_attempts.unwrap_or_else(|| max_attempts_for_job(job_type)),
                available_at.unwrap_or(now),
                now,
            )?
        };
        conn.commit()?;
        let mut job = row_to_job(&row)?;
        job.insert("deduplicated".to_string(), json!(false));
        Ok(job)
    }

    pub fn get_job(&self, job_id: i64) -> Result<JsonMap<String, JsonValue>, String> {
        let mut conn = self.pool.checkout()?;
        let row = conn
            .job_row(job_id)?
            .ok_or_else(|| format!("Unknown job: {job_id}"))?;
        row_to_job(&row)
    }

    pub fn list_jobs(
        &self,
        repo_name: Option<&str>,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let mut conn = self.pool.checkout()?;
        conn.list_job_summary_rows(repo_name, state, limit.clamp(1, 20))
            .and_then(|rows| rows.iter().map(row_to_job).collect())
    }

    pub fn list_patchset_ci_jobs(
        &self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let mut conn = self.pool.checkout()?;
        conn.list_patchset_ci_job_rows(repo_name, patchset_id, state, limit.max(1))
            .and_then(|rows| rows.iter().map(row_to_job).collect())
    }

    pub fn list_patchset_ci_status_jobs(
        &self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let mut conn = self.pool.checkout()?;
        conn.list_patchset_ci_status_job_rows(repo_name, patchset_id, state, limit.max(1))
            .and_then(|rows| rows.iter().map(row_to_job).collect())
    }

    pub fn list_patchset_ci_readiness_jobs(
        &self,
        repo_name: &str,
        patchset_id: &str,
        state: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let mut conn = self.pool.checkout()?;
        conn.list_patchset_ci_readiness_job_rows(repo_name, patchset_id, state, limit.clamp(1, 20))
            .and_then(|rows| rows.iter().map(row_to_job).collect())
    }

    pub fn job_diagnostics(
        &self,
        repo_name: Option<&str>,
        stale_after_seconds: i64,
        limit: i64,
        now: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let mut conn = self.pool.checkout()?;
        let recent_limit = limit.clamp(1, 20);
        let diagnostic_state_limit = limit.clamp(1, 100);
        let recent_jobs = conn
            .list_job_summary_rows(repo_name, None, recent_limit)?
            .iter()
            .map(row_to_job)
            .collect::<Result<Vec<_>, _>>()?;
        let mut jobs = recent_jobs.clone();
        let mut seen = jobs
            .iter()
            .map(|job| (job_id_i64(job), ()))
            .collect::<BTreeMap<_, _>>();
        for state in ["queued", "running", "failed"] {
            for job in conn
                .list_job_summary_rows(repo_name, Some(state), diagnostic_state_limit)?
                .iter()
                .map(row_to_job)
            {
                let job = job?;
                if seen.insert(job_id_i64(&job), ()).is_none() {
                    jobs.push(job);
                }
            }
        }
        let diagnostic_job_count = jobs.len();
        let mut diagnostics = worker_queue_job_diagnostics_from_jobs(
            repo_name,
            stale_after_seconds,
            limit,
            now,
            jobs,
        )?;
        diagnostics.insert(
            "diagnostic_projection".to_string(),
            json!("state_aware_compact"),
        );
        diagnostics.insert(
            "diagnostic_job_count".to_string(),
            json!(diagnostic_job_count),
        );
        diagnostics.insert("recent_job_count".to_string(), json!(recent_jobs.len()));
        diagnostics.insert(
            "recent_jobs".to_string(),
            JsonValue::Array(recent_jobs.into_iter().map(JsonValue::Object).collect()),
        );
        Ok(diagnostics)
    }

    pub fn claim_next_job(
        &self,
        worker_id: &str,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        self.claim_next_job_with_capabilities(
            worker_id,
            now,
            repo_name,
            &WorkerQueueClaimCapabilities::default(),
        )
    }

    pub fn claim_next_job_with_capabilities(
        &self,
        worker_id: &str,
        now: &str,
        repo_name: Option<&str>,
        capabilities: &WorkerQueueClaimCapabilities,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let mut conn = self.pool.checkout()?;
        for _ in 0..64 {
            let queued_rows = conn.queued_job_rows(now, repo_name)?;
            if queued_rows.is_empty() {
                return Ok(None);
            }
            let queued_jobs = shape_job_rows(&queued_rows)?
                .into_iter()
                .filter(|job| claim_capabilities_match(job, capabilities))
                .collect::<Vec<_>>();
            if queued_jobs.is_empty() {
                return Ok(None);
            }
            let scheduler_queued = scheduler_queued_jobs(&queued_jobs, &self.policy)?;
            if scheduler_queued
                .iter()
                .any(|job| is_scheduler_ci_job(&job.spec.job_kind))
            {
                let running_rows = conn.running_job_rows(repo_name)?;
                let running_jobs = shape_job_rows(&running_rows)?;
                let scheduler_running = scheduler_running_jobs(&running_jobs, &self.policy)?;
                match admit_next(&scheduler_queued, &scheduler_running, &self.policy) {
                    SchedulerAdmissionDecision::Admit { job_id } => {
                        let job_id_i64 = parse_job_id(&job_id)?;
                        let Some(row) = conn.mark_running(job_id_i64, worker_id, now)? else {
                            return Ok(None);
                        };
                        let mut job = row_to_job(&row)?;
                        if let Some(admitted) =
                            scheduler_queued.iter().find(|job| job.job_id == job_id)
                        {
                            job.insert(
                                "admitted_cpu_tokens".to_string(),
                                json!(admitted.spec.cpu_tokens),
                            );
                            job.insert(
                                "scheduler_admission".to_string(),
                                json!({
                                    "decision": {
                                        "kind": "admit",
                                        "job_id": job_id,
                                        "job": scheduler_job_json(&admitted.spec),
                                    },
                                    "policy": scheduler_policy_json(&self.policy),
                                }),
                            );
                        }
                        conn.commit()?;
                        return Ok(Some(job));
                    }
                    SchedulerAdmissionDecision::Attach {
                        job_id,
                        active_job_id,
                        singleflight_key,
                    } => {
                        let job_id_i64 = parse_job_id(&job_id)?;
                        if !conn.mark_attached(
                            job_id_i64,
                            &active_job_id,
                            &singleflight_key,
                            now,
                        )? {
                            return Ok(None);
                        }
                        conn.commit()?;
                        continue;
                    }
                    SchedulerAdmissionDecision::Wait { .. } => return Ok(None),
                }
            }

            let Some(first) = queued_jobs.first() else {
                return Ok(None);
            };
            let job_id = job_id_i64(first);
            let Some(row) = conn.mark_running(job_id, worker_id, now)? else {
                return Ok(None);
            };
            let job = row_to_job(&row)?;
            conn.commit()?;
            return Ok(Some(job));
        }
        Err("worker queue scheduler attach loop exceeded the safety limit".to_string())
    }

    pub fn heartbeat_job(
        &self,
        job_id: i64,
        worker_id: &str,
        now: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let mut conn = self.pool.checkout()?;
        let row = conn.renew_lease(job_id, worker_id, now)?.ok_or_else(|| {
            format!("Cannot renew job {job_id}: expected running state owned by `{worker_id}`.")
        })?;
        conn.commit()?;
        row_to_job(&row)
    }

    pub fn claim_job(
        &self,
        job_id: i64,
        worker_id: &str,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let mut conn = self.pool.checkout()?;
        let row = conn
            .job_row(job_id)?
            .ok_or_else(|| format!("Unknown job: {job_id}"))?;
        if !repo_matches(&row, repo_name) {
            let actual = row_text(&row, "repo_name").unwrap_or_else(|| "<unknown>".to_string());
            let expected = repo_name.unwrap_or("<none>");
            return Err(format!(
                "Cannot claim job {job_id}: repo_name mismatch, expected `{expected}`, got `{actual}`."
            ));
        }
        let state = row_text(&row, "state").unwrap_or_default();
        if state == "running" && row_text(&row, "locked_by").as_deref() == Some(worker_id) {
            return row_to_job(&row);
        }
        if state != "queued" {
            return Err(format!(
                "Cannot claim job {job_id}: expected queued state, got `{state}`."
            ));
        }
        let Some(row) = conn.mark_running(job_id, worker_id, now)? else {
            return Err(format!(
                "Cannot claim job {job_id}: job was no longer queued."
            ));
        };
        conn.commit()?;
        row_to_job(&row)
    }

    pub fn complete_job(
        &self,
        job_id: i64,
        result: &JsonValue,
        now: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.complete_job_for_worker(job_id, result, now, None)
    }

    pub fn complete_job_for_worker(
        &self,
        job_id: i64,
        result: &JsonValue,
        now: &str,
        required_worker_id: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let mut conn = self.pool.checkout()?;
        let current = conn
            .job_row(job_id)?
            .ok_or_else(|| format!("Unknown job: {job_id}"))?;
        validate_terminal_lease_owner(&current, job_id, required_worker_id)?;
        let job_type = row_text(&current, "job_type").unwrap_or_default();
        let durable_result = compact_job_result_for_storage(&job_type, result);
        let row = conn.mark_succeeded(job_id, &durable_result, now, required_worker_id)?;
        let superseded_rows = if job_type == "patchset.ci"
            && patchset_ci_result_supersedes_older_duplicates(result)
        {
            let repo_name = row_text(&current, "repo_name");
            let patchset_id = row_payload_text(&current, "patchset_id");
            conn.reconcile_superseded_patchset_ci(
                repo_name.as_deref(),
                patchset_id.as_deref(),
                now,
            )?
        } else {
            Vec::new()
        };
        conn.commit()?;
        let mut job = row_to_job(&row)?;
        job.insert(
            "superseded_job_ids".to_string(),
            json!(superseded_rows.iter().map(job_id_i64).collect::<Vec<_>>()),
        );
        Ok(job)
    }

    pub fn fail_job(
        &self,
        job_id: i64,
        error: &str,
        retryable: bool,
        retry_available_at: Option<&str>,
        now: &str,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        self.fail_job_for_worker(job_id, error, retryable, retry_available_at, now, None)
    }

    pub fn fail_job_for_worker(
        &self,
        job_id: i64,
        error: &str,
        retryable: bool,
        retry_available_at: Option<&str>,
        now: &str,
        required_worker_id: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let mut conn = self.pool.checkout()?;
        let row = conn
            .job_row(job_id)?
            .ok_or_else(|| format!("Unknown job: {job_id}"))?;
        validate_terminal_lease_owner(&row, job_id, required_worker_id)?;
        let job_type = row_text(&row, "job_type").unwrap_or_default();
        let retry_delay_seconds = retry_delay_seconds_for_job(&job_type);
        let computed_retry_at;
        let retry_at = match retry_available_at {
            Some(value) => value,
            None => {
                computed_retry_at = retry_at_from_now(now, retry_delay_seconds);
                computed_retry_at.as_str()
            }
        };
        let result = conn.mark_failed_or_retry(
            job_id,
            error,
            retryable,
            Some(retry_at),
            now,
            required_worker_id,
        )?;
        conn.commit()?;
        let mut job = row_to_job(&result)?;
        job.insert(
            "retry_delay_seconds".to_string(),
            json!(retry_delay_seconds),
        );
        Ok(job)
    }

    pub fn reclaim_stale_jobs(
        &self,
        stale_cutoff: &str,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<WorkerQueueReclaimSummary, String> {
        let mut conn = self.pool.checkout()?;
        let mut summary = conn.reclaim_stale(stale_cutoff, now, repo_name)?;
        summary.reconciled_queued_job_ids = conn
            .reconcile_superseded_patchset_ci(repo_name, None, now)?
            .iter()
            .map(job_id_i64)
            .collect();
        conn.commit()?;
        Ok(summary)
    }

    pub fn reconcile_superseded_patchset_ci_jobs(
        &self,
        repo_name: Option<&str>,
        patchset_id: Option<&str>,
        now: &str,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let mut conn = self.pool.checkout()?;
        let rows = conn.reconcile_superseded_patchset_ci(repo_name, patchset_id, now)?;
        conn.commit()?;
        rows.iter().map(row_to_job).collect()
    }
}

fn claim_capabilities_match(
    job: &JsonMap<String, JsonValue>,
    capabilities: &WorkerQueueClaimCapabilities,
) -> bool {
    let job_type = job.get("job_type").and_then(JsonValue::as_str);
    if !capabilities.accepted_job_types.is_empty()
        && !job_type.is_some_and(|value| {
            capabilities
                .accepted_job_types
                .iter()
                .any(|accepted| accepted == value)
        })
    {
        return false;
    }
    let runtime_contract = job
        .get("payload")
        .and_then(|payload| payload.get("runtime_payload"))
        .and_then(|runtime| runtime.get("contract"))
        .and_then(JsonValue::as_str);
    if !capabilities.accepted_runtime_contracts.is_empty()
        && !runtime_contract.is_some_and(|value| {
            capabilities
                .accepted_runtime_contracts
                .iter()
                .any(|accepted| accepted == value)
        })
    {
        return false;
    }
    !runtime_contract.is_some_and(|value| {
        capabilities
            .excluded_runtime_contracts
            .iter()
            .any(|excluded| excluded == value)
    })
}

fn validate_terminal_lease_owner(
    row: &JsonMap<String, JsonValue>,
    job_id: i64,
    required_worker_id: Option<&str>,
) -> Result<(), String> {
    let Some(worker_id) = required_worker_id else {
        return Ok(());
    };
    let state = row_text(row, "state").unwrap_or_default();
    let locked_by = row_text(row, "locked_by");
    if state == "running" && locked_by.as_deref() == Some(worker_id) {
        return Ok(());
    }
    Err(format!(
        "Cannot finish job {job_id}: expected running state owned by `{worker_id}`, got state `{state}` owned by `{}`.",
        locked_by.as_deref().unwrap_or("<none>")
    ))
}

fn bounded_string_list(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = obj.get(field) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("Field `{field}` must be an array of strings."))?;
    if values.len() > 32 {
        return Err(format!("Field `{field}` accepts at most 32 values."));
    }
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        let text = value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| format!("Field `{field}` must contain non-empty strings."))?;
        if text.len() > 256 {
            return Err(format!("Field `{field}` values must be at most 256 bytes."));
        }
        if !result.iter().any(|candidate| candidate == text) {
            result.push(text.to_string());
        }
    }
    Ok(result)
}

fn claim_capabilities_from_object(
    obj: &JsonMap<String, JsonValue>,
) -> Result<WorkerQueueClaimCapabilities, String> {
    Ok(WorkerQueueClaimCapabilities {
        accepted_job_types: bounded_string_list(obj, "accepted_job_types")?,
        accepted_runtime_contracts: bounded_string_list(obj, "accepted_runtime_contracts")?,
        excluded_runtime_contracts: bounded_string_list(obj, "excluded_runtime_contracts")?,
    })
}

fn row_payload_text(row: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    row_text(row, "payload_json")
        .and_then(|payload| serde_json::from_str::<JsonValue>(&payload).ok())
        .and_then(|payload| {
            payload
                .get(field)
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn patchset_ci_result_supersedes_older_duplicates(result: &JsonValue) -> bool {
    result.get("tests_status").and_then(JsonValue::as_str) == Some("pass")
        || (result.get("status").and_then(JsonValue::as_str) == Some("skipped")
            && result.get("reason").and_then(JsonValue::as_str) == Some("change_already_landed"))
}

pub fn worker_queue_job_diagnostics_from_jobs(
    repo_name: Option<&str>,
    stale_after_seconds: i64,
    limit: i64,
    now: &str,
    jobs: Vec<JsonMap<String, JsonValue>>,
) -> Result<JsonMap<String, JsonValue>, String> {
    WorkerQueueJobJson::stateless().job_diagnostics_from_jobs(
        repo_name,
        stale_after_seconds,
        limit,
        now,
        jobs,
    )
}

pub(crate) fn worker_queue_job_diagnostics_from_jobs_impl(
    repo_name: Option<&str>,
    stale_after_seconds: i64,
    limit: i64,
    now: &str,
    jobs: Vec<JsonMap<String, JsonValue>>,
) -> Result<JsonMap<String, JsonValue>, String> {
    if stale_after_seconds <= 0 {
        return Err("stale_after_seconds must be greater than zero".to_string());
    }
    if limit < 0 {
        return Err("limit must be greater than or equal to zero".to_string());
    }
    let now_timestamp = postgres_timestamptz("now", now)?;
    let stale_cutoff = now_timestamp - Duration::seconds(stale_after_seconds);
    let mut stale_job_ids: Vec<i64> = Vec::new();
    let mut retryable_job_ids: Vec<i64> = Vec::new();
    let mut delayed_retry_job_ids: Vec<i64> = Vec::new();
    let mut exhausted_job_ids: Vec<i64> = Vec::new();
    let mut failed_job_ids: Vec<i64> = Vec::new();
    let mut main_seed_refresh_job_ids: Vec<i64> = Vec::new();
    let mut main_seed_refresh_queued_job_ids: Vec<i64> = Vec::new();
    let mut main_seed_refresh_running_job_ids: Vec<i64> = Vec::new();
    let mut main_seed_refresh_delayed_retry_job_ids: Vec<i64> = Vec::new();
    let mut main_seed_refresh_failed_job_ids: Vec<i64> = Vec::new();
    let mut main_seed_refresh_exhausted_job_ids: Vec<i64> = Vec::new();

    for job in &jobs {
        let state = row_text(job, "state").unwrap_or_default();
        let job_type = row_text(job, "job_type").unwrap_or_default();
        let job_id = job_id_i64(job);
        let locked_at = row_text(job, "locked_at")
            .as_deref()
            .and_then(|value| postgres_timestamptz("locked_at", value).ok());
        let available_at = row_text(job, "available_at")
            .as_deref()
            .and_then(|value| postgres_timestamptz("available_at", value).ok());
        let attempts_remaining = row_i64(job, "attempts_remaining");
        let attempts_exhausted = row_bool(job, "attempts_exhausted");
        let has_last_error = row_text(job, "last_error").is_some();
        let stale_running =
            state == "running" && locked_at.is_some_and(|value| value <= stale_cutoff);
        let delayed_retry = state == "queued"
            && has_last_error
            && attempts_remaining > 0
            && available_at.is_some_and(|value| value > now_timestamp);
        let exhausted_failed = state == "failed" && attempts_exhausted;

        if stale_running {
            stale_job_ids.push(job_id);
        }
        if matches!(state.as_str(), "queued" | "running") && attempts_remaining > 0 {
            retryable_job_ids.push(job_id);
        }
        if delayed_retry {
            delayed_retry_job_ids.push(job_id);
        }
        if exhausted_failed {
            exhausted_job_ids.push(job_id);
        }
        if state == "failed" {
            failed_job_ids.push(job_id);
        }
        if job_type == "main-seed.refresh" {
            main_seed_refresh_job_ids.push(job_id);
            match state.as_str() {
                "queued" => main_seed_refresh_queued_job_ids.push(job_id),
                "running" => main_seed_refresh_running_job_ids.push(job_id),
                "failed" => main_seed_refresh_failed_job_ids.push(job_id),
                _ => {}
            }
            if delayed_retry {
                main_seed_refresh_delayed_retry_job_ids.push(job_id);
            }
            if exhausted_failed {
                main_seed_refresh_exhausted_job_ids.push(job_id);
            }
        }
    }

    let active_jobs = jobs.iter().any(|job| {
        matches!(
            row_text(job, "state").unwrap_or_default().as_str(),
            "queued" | "running"
        )
    });
    let (recommended_action, recommended_action_reason) = if !stale_job_ids.is_empty() {
        (
            "reclaim_stale",
            format!(
                "{} running job(s) have stale worker locks.",
                stale_job_ids.len()
            ),
        )
    } else if !failed_job_ids.is_empty() || !exhausted_job_ids.is_empty() {
        (
            "inspect_failed",
            format!(
                "{} job(s) need failure inspection.",
                if failed_job_ids.is_empty() {
                    exhausted_job_ids.len()
                } else {
                    failed_job_ids.len()
                }
            ),
        )
    } else if !delayed_retry_job_ids.is_empty() {
        (
            "wait_for_retry",
            format!(
                "{} job(s) are waiting for their retry window.",
                delayed_retry_job_ids.len()
            ),
        )
    } else if active_jobs {
        (
            "monitor_workers",
            "Queue has active jobs but no recovery action is required yet.".to_string(),
        )
    } else {
        (
            "none",
            "No active or failed async jobs require operator action.".to_string(),
        )
    };

    let recovery_summary = json!({
        "stale_running_jobs": stale_job_ids.len(),
        "stale_job_ids": stale_job_ids,
        "retryable_jobs": retryable_job_ids.len(),
        "retryable_job_ids": retryable_job_ids,
        "delayed_retry_jobs": delayed_retry_job_ids.len(),
        "delayed_retry_job_ids": delayed_retry_job_ids,
        "exhausted_jobs": exhausted_job_ids.len(),
        "exhausted_job_ids": exhausted_job_ids,
        "failed_jobs": failed_job_ids.len(),
        "failed_job_ids": failed_job_ids,
    });
    let main_seed_refresh_requires_attention = !main_seed_refresh_failed_job_ids.is_empty()
        || !main_seed_refresh_exhausted_job_ids.is_empty();
    let main_seed_refresh_summary = json!({
        "job_count": main_seed_refresh_job_ids.len(),
        "job_ids": main_seed_refresh_job_ids,
        "queued_jobs": main_seed_refresh_queued_job_ids.len(),
        "queued_job_ids": main_seed_refresh_queued_job_ids,
        "running_jobs": main_seed_refresh_running_job_ids.len(),
        "running_job_ids": main_seed_refresh_running_job_ids,
        "delayed_retry_jobs": main_seed_refresh_delayed_retry_job_ids.len(),
        "delayed_retry_job_ids": main_seed_refresh_delayed_retry_job_ids,
        "failed_jobs": main_seed_refresh_failed_job_ids.len(),
        "failed_job_ids": main_seed_refresh_failed_job_ids,
        "exhausted_jobs": main_seed_refresh_exhausted_job_ids.len(),
        "exhausted_job_ids": main_seed_refresh_exhausted_job_ids,
        "requires_attention": main_seed_refresh_requires_attention,
    });
    let mut payload = JsonMap::from_iter([
        (
            "repo_name".to_string(),
            repo_name.map_or(JsonValue::Null, |value| json!(value)),
        ),
        ("snapshot_at".to_string(), json!(now)),
        ("limit".to_string(), json!(limit)),
        ("job_count".to_string(), json!(jobs.len())),
        (
            "stale_after_seconds".to_string(),
            json!(stale_after_seconds),
        ),
        ("stale_cutoff".to_string(), json!(stale_cutoff.to_rfc3339())),
        (
            "state_summary".to_string(),
            JsonValue::Object(count_jobs_by(&jobs, "state")),
        ),
        (
            "job_type_summary".to_string(),
            JsonValue::Object(count_jobs_by(&jobs, "job_type")),
        ),
        ("recommended_action".to_string(), json!(recommended_action)),
        (
            "recommended_action_reason".to_string(),
            json!(recommended_action_reason),
        ),
        ("recovery_summary".to_string(), recovery_summary.clone()),
        (
            "main_seed_refresh".to_string(),
            main_seed_refresh_summary.clone(),
        ),
        (
            "recent_jobs".to_string(),
            JsonValue::Array(jobs.into_iter().map(JsonValue::Object).collect()),
        ),
    ]);
    if let JsonValue::Object(summary) = recovery_summary {
        payload.extend(summary);
    }
    if let JsonValue::Object(summary) = main_seed_refresh_summary {
        for (key, value) in summary {
            payload.insert(format!("main_seed_refresh_{key}"), value);
        }
    }
    Ok(payload)
}

pub fn worker_queue_kernel_json(request: &JsonValue) -> Result<JsonValue, String> {
    WorkerQueueJobJson::stateless().kernel_json(request)
}

pub(crate) fn worker_queue_kernel_json_impl(request: &JsonValue) -> Result<JsonValue, String> {
    let obj = request
        .as_object()
        .ok_or_else(|| "worker-queue-kernel payload must be a JSON object.".to_string())?;
    let operation = text_field(obj, "operation")?;
    let now = text_field(obj, "now")?;
    let repo_name = optional_text(obj, "repo_name");
    let rows = object_rows(obj, "jobs")?;
    let pool = InMemoryWorkerQueuePool::new(rows);
    let kernel = WorkerQueueKernel::new(pool.clone(), SchedulerPolicy::default());
    let payload = match operation.as_str() {
        "claim-next-job" => {
            let worker_id = text_field(obj, "worker_id")?;
            let capabilities = claim_capabilities_from_object(obj)?;
            json!({
                "contract": "ait.server.worker_queue.kernel.v1",
                "operation": operation,
                "claimed_job": kernel.claim_next_job_with_capabilities(
                    &worker_id,
                    &now,
                    repo_name.as_deref(),
                    &capabilities,
                )?,
                "jobs": pool.rows(),
                "connection_pool": pool.stats(),
            })
        }
        "claim-job" => {
            let job_id = i64_field(obj, "job_id")?;
            let worker_id = text_field(obj, "worker_id")?;
            json!({
                "contract": "ait.server.worker_queue.kernel.v1",
                "operation": operation,
                "claimed_job": kernel.claim_job(job_id, &worker_id, &now, repo_name.as_deref())?,
                "jobs": pool.rows(),
                "connection_pool": pool.stats(),
            })
        }
        "heartbeat-job" => {
            let job_id = i64_field(obj, "job_id")?;
            let worker_id = text_field(obj, "worker_id")?;
            json!({
                "contract": "ait.server.worker_queue.kernel.v1",
                "operation": operation,
                "job": kernel.heartbeat_job(job_id, &worker_id, &now)?,
                "jobs": pool.rows(),
                "connection_pool": pool.stats(),
            })
        }
        "complete-job" => {
            let job_id = i64_field(obj, "job_id")?;
            let result = obj.get("result").cloned().unwrap_or_else(|| json!({}));
            let worker_id = optional_text(obj, "worker_id");
            json!({
                "contract": "ait.server.worker_queue.kernel.v1",
                "operation": operation,
                "job": kernel.complete_job_for_worker(
                    job_id,
                    &result,
                    &now,
                    worker_id.as_deref(),
                )?,
                "jobs": pool.rows(),
                "connection_pool": pool.stats(),
            })
        }
        "fail-job" => {
            let job_id = i64_field(obj, "job_id")?;
            let error = text_field(obj, "error")?;
            let retryable = obj
                .get("retryable")
                .and_then(JsonValue::as_bool)
                .unwrap_or(true);
            let retry_available_at = optional_text(obj, "retry_available_at");
            let worker_id = optional_text(obj, "worker_id");
            json!({
                "contract": "ait.server.worker_queue.kernel.v1",
                "operation": operation,
                "job": kernel.fail_job_for_worker(
                    job_id,
                    &error,
                    retryable,
                    retry_available_at.as_deref(),
                    &now,
                    worker_id.as_deref(),
                )?,
                "jobs": pool.rows(),
                "connection_pool": pool.stats(),
            })
        }
        "reclaim-stale-jobs" => {
            let stale_cutoff = text_field(obj, "stale_cutoff")?;
            let summary = kernel.reclaim_stale_jobs(&stale_cutoff, &now, repo_name.as_deref())?;
            json!({
                "contract": "ait.server.worker_queue.kernel.v1",
                "operation": operation,
                "stale_count": summary.stale_count,
                "requeued_job_ids": summary.requeued_job_ids,
                "failed_job_ids": summary.failed_job_ids,
                "superseded_job_ids": summary.superseded_job_ids,
                "reconciled_queued_job_ids": summary.reconciled_queued_job_ids,
                "jobs": pool.rows(),
                "connection_pool": pool.stats(),
            })
        }
        _ => {
            return Err(format!(
                "Unsupported worker queue kernel operation `{operation}`. Expected one of: claim-next-job, claim-job, heartbeat-job, complete-job, fail-job, reclaim-stale-jobs."
            ))
        }
    };
    Ok(payload)
}

pub fn worker_queue_service_json<P: WorkerQueueConnectionPool>(
    kernel: &WorkerQueueKernel<P>,
    request: &JsonValue,
) -> Result<JsonValue, String> {
    WorkerQueueJobJson::stateless().service_json(kernel, request)
}

pub(crate) fn worker_queue_service_json_impl<P: WorkerQueueConnectionPool>(
    kernel: &WorkerQueueKernel<P>,
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let obj = request
        .as_object()
        .ok_or_else(|| "worker-queue payload must be a JSON object.".to_string())?;
    let operation = text_field(obj, "operation")?;
    let repo_name = optional_text(obj, "repo_name");
    let now = optional_text(obj, "now").unwrap_or_else(utc_now_string);
    let payload = match operation.as_str() {
        "enqueue-job" => {
            let repo_name = repo_name
                .as_deref()
                .ok_or_else(|| "worker-queue payload requires `repo_name`.".to_string())?;
            let repo_id = optional_text(obj, "repo_id");
            let job_type = text_field(obj, "job_type")?;
            let payload = obj
                .get("payload")
                .ok_or_else(|| "worker-queue payload requires `payload`.".to_string())?;
            let available_at = optional_text(obj, "available_at");
            let max_attempts = optional_i64(obj, "max_attempts")?;
            let dedupe_active = optional_bool(obj, "dedupe_active").unwrap_or(false);
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "job": kernel.enqueue_job(
                    repo_name,
                    repo_id.as_deref(),
                    &job_type,
                    payload,
                    available_at.as_deref(),
                    max_attempts,
                    dedupe_active,
                    &now,
                )?,
            })
        }
        "get-job" => {
            let job_id = i64_field(obj, "job_id")?;
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "job": kernel.get_job(job_id)?,
            })
        }
        "list-jobs" => {
            let state = optional_text(obj, "state");
            let requested_limit = optional_i64(obj, "limit")?.unwrap_or(100);
            let limit = requested_limit.clamp(1, 20);
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "projection": "summary",
                "limit_requested": requested_limit,
                "limit_applied": limit,
                "jobs": kernel.list_jobs(repo_name.as_deref(), state.as_deref(), limit)?,
            })
        }
        "job-diagnostics" => {
            let stale_after_seconds = optional_i64(obj, "stale_after_seconds")?.unwrap_or(300);
            let limit = optional_i64(obj, "limit")?.unwrap_or(100);
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "diagnostics": kernel.job_diagnostics(
                    repo_name.as_deref(),
                    stale_after_seconds,
                    limit,
                    &now,
                )?,
            })
        }
        "claim-next-job" => {
            let worker_id = text_field(obj, "worker_id")?;
            let capabilities = claim_capabilities_from_object(obj)?;
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "claimed_job": kernel.claim_next_job_with_capabilities(
                    &worker_id,
                    &now,
                    repo_name.as_deref(),
                    &capabilities,
                )?,
            })
        }
        "claim-job" => {
            let job_id = i64_field(obj, "job_id")?;
            let worker_id = text_field(obj, "worker_id")?;
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "claimed_job": kernel.claim_job(job_id, &worker_id, &now, repo_name.as_deref())?,
            })
        }
        "heartbeat-job" => {
            let job_id = i64_field(obj, "job_id")?;
            let worker_id = text_field(obj, "worker_id")?;
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "job": kernel.heartbeat_job(job_id, &worker_id, &now)?,
            })
        }
        "complete-job" => {
            let job_id = i64_field(obj, "job_id")?;
            let result = obj.get("result").cloned().unwrap_or_else(|| json!({}));
            let worker_id = optional_text(obj, "worker_id");
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "job": kernel.complete_job_for_worker(
                    job_id,
                    &result,
                    &now,
                    worker_id.as_deref(),
                )?,
            })
        }
        "fail-job" => {
            let job_id = i64_field(obj, "job_id")?;
            let error = text_field(obj, "error")?;
            let retryable = optional_bool(obj, "retryable").unwrap_or(true);
            let retry_available_at = optional_text(obj, "retry_available_at");
            let worker_id = optional_text(obj, "worker_id");
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "job": kernel.fail_job_for_worker(
                    job_id,
                    &error,
                    retryable,
                    retry_available_at.as_deref(),
                    &now,
                    worker_id.as_deref(),
                )?,
            })
        }
        "reclaim-stale-jobs" => {
            let stale_cutoff = text_field(obj, "stale_cutoff")?;
            let summary = kernel.reclaim_stale_jobs(&stale_cutoff, &now, repo_name.as_deref())?;
            json!({
                "contract": "ait.server.worker_queue.service.v1",
                "operation": operation,
                "stale_count": summary.stale_count,
                "requeued_job_ids": summary.requeued_job_ids,
                "failed_job_ids": summary.failed_job_ids,
                "superseded_job_ids": summary.superseded_job_ids,
                "reconciled_queued_job_ids": summary.reconciled_queued_job_ids,
            })
        }
        _ => {
            return Err(format!(
                "Unsupported worker queue operation `{operation}`. Expected one of: enqueue-job, get-job, list-jobs, job-diagnostics, claim-next-job, claim-job, heartbeat-job, complete-job, fail-job, reclaim-stale-jobs."
            ))
        }
    };
    Ok(payload)
}
