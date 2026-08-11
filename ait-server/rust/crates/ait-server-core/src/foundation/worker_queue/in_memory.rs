use super::rows::{
    clear_lease, compact_worker_queue_completion_row, job_id_i64, repo_matches, repo_scope_matches,
    row_i64, row_text, sorted_rows,
};
use super::*;
use std::collections::HashMap;

#[derive(Clone)]
pub struct InMemoryWorkerQueuePool {
    state: Arc<Mutex<InMemoryWorkerQueueState>>,
}

#[derive(Debug, Default)]
struct InMemoryWorkerQueueState {
    rows: Vec<JsonMap<String, JsonValue>>,
    patchset_semantic_payloads: HashMap<i64, JsonMap<String, JsonValue>>,
    checkout_count: usize,
}

pub struct InMemoryWorkerQueueConnection {
    state: Arc<Mutex<InMemoryWorkerQueueState>>,
}

impl InMemoryWorkerQueuePool {
    pub fn new(rows: Vec<JsonMap<String, JsonValue>>) -> Self {
        let patchset_semantic_payloads = rows
            .iter()
            .filter(|row| row_text(row, "job_type").as_deref() == Some("patchset.ci"))
            .filter_map(|row| {
                row_semantic_patchset_payload(row).map(|payload| (job_id_i64(row), payload))
            })
            .collect();
        Self {
            state: Arc::new(Mutex::new(InMemoryWorkerQueueState {
                rows,
                patchset_semantic_payloads,
                checkout_count: 0,
            })),
        }
    }

    pub fn rows(&self) -> Vec<JsonMap<String, JsonValue>> {
        let state = self.state.lock().expect("worker queue mutex poisoned");
        let mut rows = state.rows.clone();
        rows.sort_by_key(job_id_i64);
        rows
    }

    pub fn stats(&self) -> JsonValue {
        let state = self.state.lock().expect("worker queue mutex poisoned");
        json!({
            "driver": "in_memory",
            "checkout_count": state.checkout_count,
            "reusable_connection_pool": true,
        })
    }
}

impl WorkerQueueConnectionPool for InMemoryWorkerQueuePool {
    type Connection = InMemoryWorkerQueueConnection;

    fn checkout(&self) -> Result<Self::Connection, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        state.checkout_count += 1;
        drop(state);
        Ok(InMemoryWorkerQueueConnection {
            state: Arc::clone(&self.state),
        })
    }
}

impl WorkerQueueConnection for InMemoryWorkerQueueConnection {
    fn active_duplicate_job_row(
        &mut self,
        repo_name: &str,
        repo_id: Option<&str>,
        job_type: &str,
        payload_json: &str,
        patchset_dedupe_payload: Option<&JsonMap<String, JsonValue>>,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let state = self.state.lock().expect("worker queue mutex poisoned");
        let semantic_patchset_payload = (job_type == "patchset.ci")
            .then_some(patchset_dedupe_payload)
            .flatten();
        Ok(state
            .rows
            .iter()
            .filter(|row| {
                matches!(
                    row_text(row, "state").as_deref(),
                    Some("queued") | Some("running")
                )
            })
            .find(|row| {
                repo_scope_matches(row, repo_name, repo_id)
                    && row_text(row, "job_type").as_deref() == Some(job_type)
                    && semantic_patchset_payload.as_ref().map_or_else(
                        || row_text(row, "payload_json").as_deref() == Some(payload_json),
                        |identity| {
                            state
                                .patchset_semantic_payloads
                                .get(&job_id_i64(row))
                                .is_some_and(|candidate| {
                                    patchset_semantic_payload_matches(candidate, identity)
                                })
                        },
                    )
            })
            .cloned())
    }

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
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        let job_id = state.rows.iter().map(job_id_i64).max().unwrap_or(0) + 1;
        let row = JsonMap::from_iter([
            ("job_id".to_string(), json!(job_id)),
            ("repo_name".to_string(), json!(repo_name)),
            (
                "repo_id".to_string(),
                repo_id.map(JsonValue::from).unwrap_or(JsonValue::Null),
            ),
            ("job_type".to_string(), json!(job_type)),
            ("state".to_string(), json!("queued")),
            ("payload_json".to_string(), json!(payload_json)),
            ("result_json".to_string(), json!("{}")),
            ("attempt_count".to_string(), json!(0)),
            ("max_attempts".to_string(), json!(max_attempts.max(1))),
            ("available_at".to_string(), json!(available_at)),
            ("locked_at".to_string(), JsonValue::Null),
            ("locked_by".to_string(), JsonValue::Null),
            ("last_error".to_string(), JsonValue::Null),
            ("created_at".to_string(), json!(now)),
            ("updated_at".to_string(), json!(now)),
        ]);
        state.rows.push(row.clone());
        if job_type == "patchset.ci" {
            let semantic_payload = patchset_dedupe_payload
                .and_then(patchset_semantic_payload_from_map)
                .or_else(|| patchset_semantic_payload(payload_json));
            if let Some(semantic_payload) = semantic_payload {
                state
                    .patchset_semantic_payloads
                    .insert(job_id, semantic_payload);
            }
        }
        Ok(row)
    }

    fn queued_job_rows(
        &mut self,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let state = self.state.lock().expect("worker queue mutex poisoned");
        Ok(sorted_rows(
            state
                .rows
                .iter()
                .filter(|row| row_text(row, "state").as_deref() == Some("queued"))
                .filter(|row| row_text(row, "available_at").as_deref().unwrap_or("") <= now)
                .filter(|row| repo_matches(row, repo_name))
                .cloned()
                .collect(),
        ))
    }

    fn running_job_rows(
        &mut self,
        repo_name: Option<&str>,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let state = self.state.lock().expect("worker queue mutex poisoned");
        Ok(sorted_rows(
            state
                .rows
                .iter()
                .filter(|row| row_text(row, "state").as_deref() == Some("running"))
                .filter(|row| repo_matches(row, repo_name))
                .cloned()
                .collect(),
        ))
    }

    fn job_row(&mut self, job_id: i64) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let state = self.state.lock().expect("worker queue mutex poisoned");
        Ok(state
            .rows
            .iter()
            .find(|row| job_id_i64(row) == job_id)
            .cloned())
    }

    fn list_job_rows(
        &mut self,
        repo_name: Option<&str>,
        state_filter: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let state = self.state.lock().expect("worker queue mutex poisoned");
        let mut rows = state
            .rows
            .iter()
            .filter(|row| repo_matches(row, repo_name))
            .filter(|row| {
                state_filter
                    .map(|state_filter| row_text(row, "state").as_deref() == Some(state_filter))
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| -job_id_i64(row));
        rows.truncate(limit.max(1) as usize);
        Ok(rows)
    }

    fn list_patchset_ci_job_rows(
        &mut self,
        repo_name: &str,
        patchset_id: &str,
        state_filter: Option<&str>,
        limit: i64,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let state = self.state.lock().expect("worker queue mutex poisoned");
        let mut rows = state
            .rows
            .iter()
            .filter(|row| repo_matches(row, Some(repo_name)))
            .filter(|row| {
                matches!(
                    row_text(row, "job_type").as_deref(),
                    Some("patchset.ci" | "patchset.ci.aggregate")
                )
            })
            .filter(|row| {
                state_filter
                    .map(|state_filter| row_text(row, "state").as_deref() == Some(state_filter))
                    .unwrap_or(true)
            })
            .filter(|row| {
                row_text(row, "payload_json")
                    .and_then(|payload| serde_json::from_str::<JsonValue>(&payload).ok())
                    .and_then(|payload| {
                        payload
                            .get("patchset_id")
                            .and_then(JsonValue::as_str)
                            .map(str::to_string)
                    })
                    .as_deref()
                    == Some(patchset_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        rows.sort_by_key(|row| -job_id_i64(row));
        rows.truncate(limit.max(1) as usize);
        Ok(rows)
    }

    fn mark_running(
        &mut self,
        job_id: i64,
        worker_id: &str,
        now: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        let Some(row) = state.rows.iter_mut().find(|row| {
            job_id_i64(row) == job_id && row_text(row, "state").as_deref() == Some("queued")
        }) else {
            return Ok(None);
        };
        let attempt_count = row_i64(row, "attempt_count") + 1;
        row.insert("state".to_string(), json!("running"));
        row.insert("attempt_count".to_string(), json!(attempt_count));
        row.insert("locked_at".to_string(), json!(now));
        row.insert("locked_by".to_string(), json!(worker_id));
        row.insert("updated_at".to_string(), json!(now));
        Ok(Some(row.clone()))
    }

    fn renew_lease(
        &mut self,
        job_id: i64,
        worker_id: &str,
        now: &str,
    ) -> Result<Option<JsonMap<String, JsonValue>>, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        let Some(row) = state.rows.iter_mut().find(|row| {
            job_id_i64(row) == job_id
                && row_text(row, "state").as_deref() == Some("running")
                && row_text(row, "locked_by").as_deref() == Some(worker_id)
        }) else {
            return Ok(None);
        };
        row.insert("locked_at".to_string(), json!(now));
        row.insert("updated_at".to_string(), json!(now));
        Ok(Some(row.clone()))
    }

    fn mark_attached(
        &mut self,
        job_id: i64,
        active_job_id: &str,
        singleflight_key: &str,
        now: &str,
    ) -> Result<bool, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        let Some(row) = state.rows.iter_mut().find(|row| {
            job_id_i64(row) == job_id && row_text(row, "state").as_deref() == Some("queued")
        }) else {
            return Ok(false);
        };
        row.insert("state".to_string(), json!("succeeded"));
        row.insert(
            "result_json".to_string(),
            json!(json!({
                "status": "attached",
                "scheduler": {
                    "decision": "attach",
                    "active_job_id": active_job_id,
                    "singleflight_key": singleflight_key,
                }
            })
            .to_string()),
        );
        clear_lease(row);
        row.insert("updated_at".to_string(), json!(now));
        Ok(true)
    }

    fn mark_succeeded(
        &mut self,
        job_id: i64,
        result: &JsonValue,
        now: &str,
        required_worker_id: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        let row = state
            .rows
            .iter_mut()
            .find(|row| {
                job_id_i64(row) == job_id
                    && required_worker_id.is_none_or(|worker_id| {
                        row_text(row, "state").as_deref() == Some("running")
                            && row_text(row, "locked_by").as_deref() == Some(worker_id)
                    })
            })
            .ok_or_else(|| {
                if let Some(worker_id) = required_worker_id {
                    format!(
                        "Cannot finish job {job_id}: running lease is not owned by `{worker_id}`."
                    )
                } else {
                    format!("Unknown job: {job_id}")
                }
            })?;
        row.insert("state".to_string(), json!("succeeded"));
        row.insert("result_json".to_string(), json!(result.to_string()));
        clear_lease(row);
        row.insert("last_error".to_string(), JsonValue::Null);
        row.insert("updated_at".to_string(), json!(now));
        compact_worker_queue_completion_row(row, result)
    }

    fn mark_failed_or_retry(
        &mut self,
        job_id: i64,
        error: &str,
        retryable: bool,
        retry_available_at: Option<&str>,
        now: &str,
        required_worker_id: Option<&str>,
    ) -> Result<JsonMap<String, JsonValue>, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        let row = state
            .rows
            .iter_mut()
            .find(|row| {
                job_id_i64(row) == job_id
                    && required_worker_id.is_none_or(|worker_id| {
                        row_text(row, "state").as_deref() == Some("running")
                            && row_text(row, "locked_by").as_deref() == Some(worker_id)
                    })
            })
            .ok_or_else(|| {
                if let Some(worker_id) = required_worker_id {
                    format!(
                        "Cannot finish job {job_id}: running lease is not owned by `{worker_id}`."
                    )
                } else {
                    format!("Unknown job: {job_id}")
                }
            })?;
        let attempt_count = row_i64(row, "attempt_count");
        let max_attempts = row_i64(row, "max_attempts");
        if retryable && attempt_count < max_attempts {
            row.insert("state".to_string(), json!("queued"));
            row.insert(
                "available_at".to_string(),
                json!(retry_available_at.unwrap_or(now)),
            );
        } else {
            row.insert("state".to_string(), json!("failed"));
            row.insert("available_at".to_string(), json!(now));
        }
        clear_lease(row);
        row.insert("last_error".to_string(), json!(error));
        row.insert("updated_at".to_string(), json!(now));
        Ok(row.clone())
    }

    fn reconcile_superseded_patchset_ci(
        &mut self,
        repo_name: Option<&str>,
        patchset_id: Option<&str>,
        now: &str,
    ) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        let candidates = state
            .rows
            .iter()
            .filter(|row| row_text(row, "state").as_deref() == Some("queued"))
            .filter(|row| row_text(row, "job_type").as_deref() == Some("patchset.ci"))
            .filter(|row| repo_matches(row, repo_name))
            .filter(|row| {
                patchset_id
                    .map(|expected| {
                        row_payload_text(row, "patchset_id").as_deref() == Some(expected)
                    })
                    .unwrap_or(true)
            })
            .filter_map(|row| {
                successful_later_patchset_ci_job_id(&state.rows, row)
                    .map(|successor_job_id| (job_id_i64(row), successor_job_id))
            })
            .collect::<Vec<_>>();
        let mut reconciled = Vec::new();
        for (job_id, successor_job_id) in candidates {
            let Some(row) = state.rows.iter_mut().find(|row| {
                job_id_i64(row) == job_id && row_text(row, "state").as_deref() == Some("queued")
            }) else {
                continue;
            };
            mark_patchset_ci_superseded(row, successor_job_id, now);
            reconciled.push(row.clone());
        }
        Ok(reconciled)
    }

    fn reclaim_stale(
        &mut self,
        stale_cutoff: &str,
        now: &str,
        repo_name: Option<&str>,
    ) -> Result<WorkerQueueReclaimSummary, String> {
        let mut state = self.state.lock().expect("worker queue mutex poisoned");
        let stale_jobs = state
            .rows
            .iter()
            .filter(|row| row_text(row, "state").as_deref() == Some("running"))
            .filter(|row| row_text(row, "locked_at").as_deref().unwrap_or("") <= stale_cutoff)
            .filter(|row| repo_matches(row, repo_name))
            .map(|row| {
                (
                    job_id_i64(row),
                    successful_later_patchset_ci_job_id(&state.rows, row),
                )
            })
            .collect::<Vec<_>>();
        let mut requeued_job_ids = Vec::new();
        let mut failed_job_ids = Vec::new();
        let mut superseded_job_ids = Vec::new();
        for (job_id, successor_job_id) in stale_jobs {
            let Some(row) = state.rows.iter_mut().find(|row| {
                job_id_i64(row) == job_id && row_text(row, "state").as_deref() == Some("running")
            }) else {
                continue;
            };
            if let Some(successor_job_id) = successor_job_id {
                mark_patchset_ci_superseded(row, successor_job_id, now);
                superseded_job_ids.push(job_id);
                continue;
            } else if row_i64(row, "attempt_count") >= row_i64(row, "max_attempts") {
                row.insert("state".to_string(), json!("failed"));
                row.insert(
                    "last_error".to_string(),
                    json!("Worker lease expired after max attempts"),
                );
                failed_job_ids.push(job_id);
            } else {
                row.insert("state".to_string(), json!("queued"));
                row.insert(
                    "last_error".to_string(),
                    json!("Worker lease expired; job returned to queue"),
                );
                requeued_job_ids.push(job_id);
            }
            row.insert("available_at".to_string(), json!(now));
            clear_lease(row);
            row.insert("updated_at".to_string(), json!(now));
        }
        Ok(WorkerQueueReclaimSummary {
            stale_count: requeued_job_ids.len() + failed_job_ids.len() + superseded_job_ids.len(),
            requeued_job_ids,
            failed_job_ids,
            superseded_job_ids,
            reconciled_queued_job_ids: Vec::new(),
        })
    }
}

fn json_text_field(encoded: &str, field: &str) -> Option<String> {
    serde_json::from_str::<JsonValue>(encoded)
        .ok()?
        .get(field)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn row_payload_text(row: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    json_text_field(&row_text(row, "payload_json")?, field)
}

fn patchset_semantic_payload(encoded: &str) -> Option<JsonMap<String, JsonValue>> {
    let parsed = serde_json::from_str::<JsonValue>(encoded).ok()?;
    patchset_semantic_payload_from_map(parsed.as_object()?)
}

fn patchset_semantic_payload_from_map(
    payload: &JsonMap<String, JsonValue>,
) -> Option<JsonMap<String, JsonValue>> {
    payload
        .get("patchset_id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(
        payload
            .iter()
            .filter(|(key, _)| key.as_str() != "runtime_payload")
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

fn patchset_semantic_payload_matches(
    semantic_payload: &JsonMap<String, JsonValue>,
    candidate: &JsonMap<String, JsonValue>,
) -> bool {
    semantic_payload.len() + usize::from(candidate.contains_key("runtime_payload"))
        == candidate.len()
        && semantic_payload
            .iter()
            .all(|(key, value)| candidate.get(key) == Some(value))
}

fn row_semantic_patchset_payload(
    row: &JsonMap<String, JsonValue>,
) -> Option<JsonMap<String, JsonValue>> {
    patchset_semantic_payload(&row_text(row, "payload_json")?)
}

fn row_result_supersedes_patchset_ci(row: &JsonMap<String, JsonValue>) -> bool {
    let Some(encoded) = row_text(row, "result_json") else {
        return false;
    };
    let Ok(result) = serde_json::from_str::<JsonValue>(&encoded) else {
        return false;
    };
    result.get("tests_status").and_then(JsonValue::as_str) == Some("pass")
        || (result.get("status").and_then(JsonValue::as_str) == Some("skipped")
            && result.get("reason").and_then(JsonValue::as_str) == Some("change_already_landed"))
}

fn successful_later_patchset_ci_job_id(
    rows: &[JsonMap<String, JsonValue>],
    candidate: &JsonMap<String, JsonValue>,
) -> Option<i64> {
    if row_text(candidate, "job_type").as_deref() != Some("patchset.ci") {
        return None;
    }
    let candidate_id = job_id_i64(candidate);
    let repo_name = row_text(candidate, "repo_name")?;
    let repo_id = row_text(candidate, "repo_id");
    let semantic_payload = row_semantic_patchset_payload(candidate)?;
    rows.iter()
        .filter(|row| job_id_i64(row) > candidate_id)
        .filter(|row| row_text(row, "state").as_deref() == Some("succeeded"))
        .filter(|row| row_text(row, "job_type").as_deref() == Some("patchset.ci"))
        .filter(|row| repo_scope_matches(row, &repo_name, repo_id.as_deref()))
        .filter(|row| row_semantic_patchset_payload(row).as_ref() == Some(&semantic_payload))
        .filter(|row| row_result_supersedes_patchset_ci(row))
        .map(job_id_i64)
        .min()
}

fn mark_patchset_ci_superseded(
    row: &mut JsonMap<String, JsonValue>,
    successor_job_id: i64,
    now: &str,
) {
    let job_id = job_id_i64(row);
    row.insert("state".to_string(), json!("succeeded"));
    row.insert(
        "result_json".to_string(),
        json!(json!({
            "contract": "ait.server.worker.superseded_patchset_ci.v1",
            "status": "skipped",
            "tests_status": "pass",
            "reason": "equivalent_later_patchset_ci_succeeded",
            "superseded_job_id": job_id,
            "successor_job_id": successor_job_id,
        })
        .to_string()),
    );
    row.insert("available_at".to_string(), json!(now));
    clear_lease(row);
    row.insert("last_error".to_string(), JsonValue::Null);
    row.insert("updated_at".to_string(), json!(now));
}
