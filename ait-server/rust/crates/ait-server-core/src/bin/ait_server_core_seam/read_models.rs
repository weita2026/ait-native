use super::json_helpers::print_json;
use super::*;

pub(super) fn repository_ci_runs_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = RepositoryCiRunsInput::from_value(&payload)?;
    let runs = repository_ci_runs_read_model(&input)?;
    print_json(&runs)
}

pub(super) fn queue_read_model_summary_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = QueueReadModelInput::from_value(&payload)?;
    let summary = queue_summary_read_model(&input)?;
    print_json(&summary)
}

pub(super) fn runtime_metrics_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = RuntimeMetricsInput::from_value(&payload)?;
    let metrics = runtime_metrics_read_model(&input)?;
    print_json(&metrics)
}

pub(super) fn operator_metrics_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = OperatorMetricsInput::from_value(&payload)?;
    let metrics = operator_metrics_read_model(&input)?;
    print_json(&metrics)
}

pub(super) fn operator_readiness_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = OperatorMetricsInput::from_value(&payload)?;
    let readiness = operator_readiness_read_model(&input)?;
    print_json(&readiness)
}

pub(super) fn authority_map_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = AuthorityMapInput::from_value(&payload)?;
    let map = authority_map_read_model(&input)?;
    print_json(&map)
}

pub(super) fn reviewer_inbox_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = ReviewerInboxInput::from_value(&payload)?;
    let inbox = reviewer_inbox_read_model(&input)?;
    print_json(&inbox)
}

pub(super) fn workflow_task_detail_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = TaskWorkflowDetailInput::from_value(&payload)?;
    let detail = task_workflow_detail_read_model(&input)?;
    print_json(&detail)
}

pub(super) fn repository_index_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = RepositoryIndexInput::from_value(&payload)?;
    let index = repository_index_read_model(&input)?;
    print_json(&index)
}

pub(super) fn repository_detail_read_model_command(payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = RepositoryDetailInput::from_value(&payload)?;
    let detail = repository_detail_read_model(&input)?;
    print_json(&detail)
}

pub(super) fn repository_worker_status_read_model_command(
    payload_json: &str,
) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let input = RepositoryWorkerStatusInput::from_value(&payload)?;
    let status = repository_worker_status_read_model(&input)?;
    print_json(&status)
}
