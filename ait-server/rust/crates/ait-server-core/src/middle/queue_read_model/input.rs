use super::helpers::*;
use super::*;
use std::sync::Arc;

pub type SharedQueueRows = Arc<Vec<JsonMap<String, JsonValue>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueReadModelInput {
    pub repo_name: Option<String>,
    pub status: String,
    pub include_all_changes: bool,
    pub tasks: SharedQueueRows,
    pub changes: SharedQueueRows,
    pub patchsets: SharedQueueRows,
    pub reviews: SharedQueueRows,
    pub review_requests: SharedQueueRows,
    pub attestations: SharedQueueRows,
    pub policy_decisions: SharedQueueRows,
    pub refs: SharedQueueRows,
    pub ci_statuses: SharedQueueRows,
}

impl QueueReadModelInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = queue_summary_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        Ok(Self {
            repo_name: optional_text(obj, "repo_name"),
            status: optional_text(obj, "status").unwrap_or_else(|| TASK_STATUS_ACTIVE.to_string()),
            include_all_changes: obj
                .get("include_all_changes")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            tasks: Arc::new(rows.take("tasks")),
            changes: Arc::new(rows.take("changes")),
            patchsets: Arc::new(rows.take("patchsets")),
            reviews: Arc::new(rows.take("reviews")),
            review_requests: Arc::new(rows.take("review_requests")),
            attestations: Arc::new(rows.take("attestations")),
            policy_decisions: Arc::new(rows.take("policy_decisions")),
            refs: Arc::new(rows.take("refs")),
            ci_statuses: Arc::new(rows.take("ci_statuses")),
        })
    }
}
