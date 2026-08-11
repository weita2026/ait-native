use super::helpers::{
    bool_value, int_value, object_text, repository_ci_summary, required_text, string_list,
    value_text,
};
use super::*;

pub fn repository_detail_read_model(input: &RepositoryDetailInput) -> Result<JsonValue, String> {
    let repo_name = required_text(&input.repository, "repo_name")?;
    let default_line =
        object_text(&input.repository, "default_line").unwrap_or_else(|| "main".to_string());
    let context_by_line = input
        .line_work_contexts
        .iter()
        .filter_map(|ctx| {
            object_text(ctx, "line_name").map(|line| (line, JsonValue::Object(ctx.clone())))
        })
        .collect::<HashMap<_, _>>();

    let mut annotated_lines = Vec::new();
    let mut active_lines = Vec::new();
    let mut archived_lines = Vec::new();
    for line in input
        .lines
        .iter()
        .filter(|line| object_text(line, "repo_name").as_deref() == Some(repo_name.as_str()))
    {
        let mut item = line.clone();
        let status = object_text(&item, "status").unwrap_or_else(|| "active".to_string());
        item.insert("status".to_string(), json!(status));
        let line_name = object_text(&item, "line_name").unwrap_or_default();
        let work_context = if line_name == default_line {
            JsonValue::Null
        } else {
            context_by_line
                .get(&line_name)
                .cloned()
                .unwrap_or(JsonValue::Null)
        };
        item.insert("work_context".to_string(), work_context);
        let value = JsonValue::Object(item);
        if value_text(&value, "status").as_deref() == Some("archived") {
            archived_lines.push(value.clone());
        } else {
            active_lines.push(value.clone());
        }
        annotated_lines.push(value);
    }

    let jobs = input
        .jobs
        .iter()
        .filter(|job| {
            object_text(job, "repo_name")
                .as_deref()
                .map_or(true, |name| name == repo_name)
        })
        .cloned()
        .map(JsonValue::Object)
        .collect::<Vec<_>>();
    let active_jobs = jobs
        .iter()
        .filter(|job| {
            matches!(
                value_text(job, "state").as_deref(),
                Some("queued" | "running" | "active")
            )
        })
        .count();
    let failed_jobs = jobs
        .iter()
        .filter(|job| {
            matches!(
                value_text(job, "state").as_deref(),
                Some("failed" | "blocked")
            )
        })
        .count();
    let diagnostics = JsonValue::Object(input.diagnostics.clone());
    let storage = JsonValue::Object(input.storage.clone());
    let validation = storage
        .get("validation_summary")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let signals = storage
        .get("signals_summary")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();

    Ok(json!({
        "repository": JsonValue::Object(input.repository.clone()),
        "lines": annotated_lines,
        "active_lines": active_lines,
        "archived_lines": archived_lines,
        "line_summary": {
            "total_lines": annotated_lines.len(),
            "active_lines": active_lines.len(),
            "archived_lines": archived_lines.len(),
        },
        "jobs": jobs,
        "ci_runs": input.ci_runs.iter().cloned().map(JsonValue::Object).collect::<Vec<_>>(),
        "ci_summary": repository_ci_summary(&input.ci_runs),
        "job_diagnostics": diagnostics,
        "storage": storage,
        "storage_summary": {
            "state": object_text(&validation, "state").unwrap_or_else(|| "unknown".to_string()),
            "recommended_action": object_text(&validation, "recommended_action").unwrap_or_else(|| "none".to_string()),
            "next_actions": string_list(validation.get("next_actions")),
            "reasons": string_list(validation.get("reasons")),
            "needs_attention": validation.get("needs_attention").and_then(bool_value).unwrap_or(false),
            "drift_count": signals.get("drift_count").and_then(int_value).unwrap_or(0),
            "repairable_drift_count": signals.get("repairable_drift_count").and_then(int_value).unwrap_or(0),
        },
        "job_summary": {
            "job_limit": input.job_limit,
            "recent_jobs": jobs.len(),
            "active_jobs": active_jobs,
            "failed_jobs": failed_jobs,
            "stale_running_jobs": diagnostics.get("stale_running_jobs").and_then(int_value).unwrap_or(0),
            "delayed_retry_jobs": diagnostics.get("delayed_retry_jobs").and_then(int_value).unwrap_or(0),
            "exhausted_jobs": diagnostics.get("exhausted_jobs").and_then(int_value).unwrap_or(0),
            "recommended_action": value_text(&diagnostics, "recommended_action").unwrap_or_else(|| "none".to_string()),
        },
    }))
}
