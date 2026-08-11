use super::helpers::{object_text, required_text};
use super::*;

pub fn repository_worker_status_read_model(
    input: &RepositoryWorkerStatusInput,
) -> Result<JsonValue, String> {
    let repo_name = required_text(&input.repository, "repo_name")?;
    let mut state_summary = BTreeMap::<String, i64>::new();
    let mut workers = BTreeMap::<String, ActiveRepositoryWorker>::new();
    let mut snapshot_at = JsonValue::Null;
    let mut snapshot_seen = false;

    for job in input.jobs.iter().filter(|job| {
        object_text(job, "repo_name")
            .as_deref()
            .map_or(true, |name| name == repo_name)
    }) {
        let state = object_text(job, "state").unwrap_or_default();
        *state_summary.entry(state.clone()).or_default() += 1;
        if !snapshot_seen {
            snapshot_at = job.get("updated_at").cloned().unwrap_or(JsonValue::Null);
            snapshot_seen = true;
        }
        if state == "running" {
            if let Some(locked_by) = object_text(job, "locked_by") {
                let worker = workers
                    .entry(locked_by.clone())
                    .or_insert_with(|| ActiveRepositoryWorker::new(locked_by));
                worker.running_jobs += 1;
                if let Some(locked_at) = object_text(job, "locked_at") {
                    worker.observe_locked_at(locked_at);
                }
            }
        }
    }

    let mut active_workers = workers.into_values().collect::<Vec<_>>();
    active_workers.sort_by(|left, right| {
        right
            .running_jobs
            .cmp(&left.running_jobs)
            .then_with(|| left.worker_id.cmp(&right.worker_id))
    });
    let active_worker_values = active_workers
        .iter()
        .map(ActiveRepositoryWorker::to_json)
        .collect::<Vec<_>>();
    let state_summary_json = state_summary
        .iter()
        .map(|(state, count)| (state.clone(), json!(count)))
        .collect::<JsonMap<_, _>>();

    Ok(json!({
        "repo_name": repo_name,
        "snapshot_at": snapshot_at,
        "state_summary": state_summary_json,
        "workers": active_worker_values,
        "worker_count": active_workers.len(),
        "queued_jobs": state_summary.get("queued").copied().unwrap_or(0),
        "running_jobs": state_summary.get("running").copied().unwrap_or(0),
        "succeeded_jobs": state_summary.get("succeeded").copied().unwrap_or(0),
        "failed_jobs": state_summary.get("failed").copied().unwrap_or(0),
        "diagnostics": JsonValue::Object(input.diagnostics.clone()),
        "recent_jobs": input
            .recent_jobs
            .iter()
            .filter(|job| {
                object_text(job, "repo_name")
                    .as_deref()
                    .map_or(true, |name| name == repo_name)
            })
            .cloned()
            .map(JsonValue::Object)
            .collect::<Vec<_>>(),
    }))
}

#[derive(Debug, Clone)]
struct ActiveRepositoryWorker {
    worker_id: String,
    running_jobs: i64,
    oldest_locked_job: Option<String>,
    latest_locked_job: Option<String>,
}

impl ActiveRepositoryWorker {
    fn new(worker_id: String) -> Self {
        Self {
            worker_id,
            running_jobs: 0,
            oldest_locked_job: None,
            latest_locked_job: None,
        }
    }

    fn observe_locked_at(&mut self, locked_at: String) {
        if self
            .oldest_locked_job
            .as_ref()
            .map(|oldest| locked_at < *oldest)
            .unwrap_or(true)
        {
            self.oldest_locked_job = Some(locked_at.clone());
        }
        if self
            .latest_locked_job
            .as_ref()
            .map(|latest| locked_at > *latest)
            .unwrap_or(true)
        {
            self.latest_locked_job = Some(locked_at);
        }
    }

    fn to_json(&self) -> JsonValue {
        json!({
            "worker_id": self.worker_id,
            "running_jobs": self.running_jobs,
            "oldest_locked_job": self.oldest_locked_job,
            "latest_locked_job": self.latest_locked_job,
        })
    }
}
