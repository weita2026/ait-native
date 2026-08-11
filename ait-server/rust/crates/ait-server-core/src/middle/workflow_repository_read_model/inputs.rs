use super::helpers::{int_value, optional_object, required_object};
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorkflowDetailInput {
    pub task: JsonMap<String, JsonValue>,
    pub repository: JsonMap<String, JsonValue>,
    pub changes: Vec<JsonMap<String, JsonValue>>,
    pub patchsets: Vec<JsonMap<String, JsonValue>>,
    pub reviews: Vec<JsonMap<String, JsonValue>>,
    pub attestations: Vec<JsonMap<String, JsonValue>>,
    pub policy_decisions: Vec<JsonMap<String, JsonValue>>,
    pub land_requests: Vec<JsonMap<String, JsonValue>>,
    pub refs: Vec<JsonMap<String, JsonValue>>,
    pub patchset_deltas: Vec<JsonMap<String, JsonValue>>,
    pub events: Vec<JsonMap<String, JsonValue>>,
}

impl TaskWorkflowDetailInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = task_workflow_detail_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        Ok(Self {
            task: required_object(obj, "task")?,
            repository: required_object(obj, "repository")?,
            changes: rows.take("changes"),
            patchsets: rows.take("patchsets"),
            reviews: rows.take("reviews"),
            attestations: rows.take("attestations"),
            policy_decisions: rows.take("policy_decisions"),
            land_requests: rows.take("land_requests"),
            refs: rows.take("refs"),
            patchset_deltas: rows.take("patchset_deltas"),
            events: rows.take("events"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryIndexInput {
    pub repositories: Vec<JsonMap<String, JsonValue>>,
    pub lines: Vec<JsonMap<String, JsonValue>>,
    pub groups: Vec<JsonMap<String, JsonValue>>,
}

impl RepositoryIndexInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = repository_index_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        Ok(Self {
            repositories: rows.take("repositories"),
            lines: rows.take("lines"),
            groups: rows.take("groups"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryDetailInput {
    pub repository: JsonMap<String, JsonValue>,
    pub job_limit: i64,
    pub diagnostics: JsonMap<String, JsonValue>,
    pub storage: JsonMap<String, JsonValue>,
    pub lines: Vec<JsonMap<String, JsonValue>>,
    pub line_work_contexts: Vec<JsonMap<String, JsonValue>>,
    pub jobs: Vec<JsonMap<String, JsonValue>>,
    pub ci_runs: Vec<JsonMap<String, JsonValue>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryWorkerStatusInput {
    pub repository: JsonMap<String, JsonValue>,
    pub diagnostics: JsonMap<String, JsonValue>,
    pub jobs: Vec<JsonMap<String, JsonValue>>,
    pub recent_jobs: Vec<JsonMap<String, JsonValue>>,
}

impl RepositoryWorkerStatusInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = repository_worker_status_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        Ok(Self {
            repository: required_object(obj, "repository")?,
            diagnostics: optional_object(obj, "diagnostics"),
            jobs: rows.take("jobs"),
            recent_jobs: rows.take("recent_jobs"),
        })
    }
}

impl RepositoryDetailInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = repository_detail_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        Ok(Self {
            repository: required_object(obj, "repository")?,
            job_limit: obj.get("job_limit").and_then(int_value).unwrap_or(20),
            diagnostics: optional_object(obj, "diagnostics"),
            storage: optional_object(obj, "storage"),
            lines: rows.take("lines"),
            line_work_contexts: rows.take("line_work_contexts"),
            jobs: rows.take("jobs"),
            ci_runs: rows.take("ci_runs"),
        })
    }
}
