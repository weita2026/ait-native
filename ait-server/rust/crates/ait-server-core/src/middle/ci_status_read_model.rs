use crate::middle::read_model_contract::{
    json_value_to_text, object_text_field, read_model_payload_object, ReadModelContract,
    ReadModelRowSetSpec, ReadModelRows,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};

pub static REPOSITORY_CI_RUNS_ROW_SETS: &[ReadModelRowSetSpec] = &[ReadModelRowSetSpec {
    field: "jobs",
    required: false,
    description: "Worker queue job rows used to project repository CI run status.",
}];

pub static REPOSITORY_CI_RUNS_READ_MODEL_CONTRACT: ReadModelContract = ReadModelContract {
    domain_id: "repository_ci_runs",
    reference_module: "rust_owned_no_python_reference",
    payload_label: "repository CI runs read-model",
    public_surface: "native.read.repository_ci_runs",
    output_shape: "repo_name, filters, count, summary, items",
    mutates_state: false,
    row_sets: REPOSITORY_CI_RUNS_ROW_SETS,
};

pub fn repository_ci_runs_read_model_contract() -> &'static ReadModelContract {
    &REPOSITORY_CI_RUNS_READ_MODEL_CONTRACT
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryCiRunsInput {
    pub repo_name: String,
    pub limit: i64,
    pub plane: Option<String>,
    pub suite_id: Option<String>,
    pub jobs: Vec<JsonMap<String, JsonValue>>,
}

impl RepositoryCiRunsInput {
    pub fn from_value(value: &JsonValue) -> Result<Self, String> {
        let contract = repository_ci_runs_read_model_contract();
        let obj = read_model_payload_object(value, contract.payload_label)?;
        let mut rows = ReadModelRows::from_object(obj, contract)?;
        Ok(Self {
            repo_name: required_text(obj, "repo_name")?,
            limit: obj.get("limit").and_then(int_value).unwrap_or(20),
            plane: optional_text(obj, "plane"),
            suite_id: optional_text(obj, "suite_id"),
            jobs: rows.take("jobs"),
        })
    }
}

pub fn repository_ci_runs_read_model(input: &RepositoryCiRunsInput) -> Result<JsonValue, String> {
    let resolved_limit = input.limit.max(1) as usize;
    let normalized_plane = input.plane.as_deref().and_then(normalize_optional_text);
    let normalized_suite = input.suite_id.as_deref().and_then(normalize_optional_text);
    let mut items = Vec::new();
    for job in input
        .jobs
        .iter()
        .filter(|job| object_text(job, "job_type").as_deref() == Some("repo.ci"))
    {
        let summary = summarize_repo_ci_job(job);
        if let Some(plane) = normalized_plane.as_deref() {
            let mut planes = string_set(summary.get("selected_planes"));
            if let Some(value) = value_text(&summary, "plane") {
                planes.insert(value);
            }
            if !planes.contains(plane) {
                continue;
            }
        }
        if let Some(suite_id) = normalized_suite.as_deref() {
            if !string_set(summary.get("selected_suite_ids")).contains(suite_id) {
                continue;
            }
        }
        items.push(summary);
        if items.len() >= resolved_limit {
            break;
        }
    }

    let mut latest_by_suite = JsonMap::new();
    let mut latest_by_plane = JsonMap::new();
    for item in &items {
        for suite in string_list(item.get("selected_suite_ids")) {
            latest_by_suite
                .entry(suite)
                .or_insert_with(|| latest_summary(item));
        }
        for plane in string_list(item.get("selected_planes")) {
            latest_by_plane
                .entry(plane)
                .or_insert_with(|| latest_summary(item));
        }
        if let Some(plane) = value_text(item, "plane") {
            latest_by_plane
                .entry(plane)
                .or_insert_with(|| latest_summary(item));
        }
    }

    let active_runs = items
        .iter()
        .filter(|item| {
            matches!(
                value_text(item, "state").as_deref(),
                Some("queued" | "running")
            )
        })
        .count();
    let failed_runs = items
        .iter()
        .filter(|item| value_text(item, "status").as_deref() == Some("fail"))
        .count();
    Ok(json!({
        "repo_name": input.repo_name,
        "filters": {
            "limit": resolved_limit,
            "plane": normalized_plane,
            "suite_id": normalized_suite,
        },
        "count": items.len(),
        "summary": {
            "active_runs": active_runs,
            "failed_runs": failed_runs,
            "latest_by_suite": latest_by_suite,
            "latest_by_plane": latest_by_plane,
        },
        "items": items,
    }))
}

fn summarize_repo_ci_job(job: &JsonMap<String, JsonValue>) -> JsonValue {
    let result = object_field(job, "result");
    let payload = object_field(job, "payload");
    let suite_results = array_field(&result, "suite_results");
    let selected_suite_ids = suite_ids_for_job(&payload, &result);
    let selected_planes = planes_for_job(&payload, &result);
    json!({
        "job_id": object_text(job, "job_id")
            .and_then(|value| value.parse::<i64>().ok())
            .or_else(|| job.get("job_id").and_then(JsonValue::as_i64))
            .unwrap_or(0),
        "job_type": object_text(job, "job_type").unwrap_or_default(),
        "state": object_text(job, "state").unwrap_or_default(),
        "diagnostic_status": object_text(job, "diagnostic_status").unwrap_or_default(),
        "trigger": object_text(&result, "trigger").or_else(|| object_text(&payload, "trigger")),
        "created_at": job.get("created_at").cloned().unwrap_or(JsonValue::Null),
        "updated_at": job.get("updated_at").cloned().unwrap_or(JsonValue::Null),
        "target_line": object_text(&result, "target_line")
            .or_else(|| object_text(&payload, "target_line"))
            .unwrap_or_else(|| "main".to_string()),
        "status": repo_job_status(job),
        "plane": object_text(&payload, "plane"),
        "selected_planes": selected_planes,
        "requested_suite_ids": string_list(payload.get("suite_ids")),
        "selected_suite_ids": selected_suite_ids,
        "blocking_failures": string_list(result.get("blocking_failures")),
        "task_batch": task_batch_summary(&suite_results),
        "summary_artifacts": summary_artifacts_for_suite_results(&suite_results),
        "suite_results": suite_results,
        "rerun": {"cli": repo_rerun_command(&payload, &result)},
        "payload": JsonValue::Object(payload),
        "result": JsonValue::Object(result),
    })
}

fn repo_job_status(job: &JsonMap<String, JsonValue>) -> String {
    let result = object_field(job, "result");
    if let Some(status) = object_text(&result, "status") {
        return status;
    }
    match object_text(job, "state")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "succeeded" => "pass".to_string(),
        "failed" | "blocked" => "fail".to_string(),
        "queued" | "running" => "pending".to_string(),
        "" => "unknown".to_string(),
        other => other.to_string(),
    }
}

fn suite_ids_for_job(
    payload: &JsonMap<String, JsonValue>,
    result: &JsonMap<String, JsonValue>,
) -> Vec<String> {
    let selected = string_list(result.get("selected_suite_ids"));
    if selected.is_empty() {
        string_list(payload.get("suite_ids"))
    } else {
        selected
    }
}

fn planes_for_job(
    payload: &JsonMap<String, JsonValue>,
    result: &JsonMap<String, JsonValue>,
) -> Vec<String> {
    let selected = string_list(result.get("selected_planes"));
    if !selected.is_empty() {
        return selected;
    }
    object_text(payload, "plane").into_iter().collect()
}

fn task_batch_summary(suite_results: &[JsonValue]) -> JsonValue {
    for suite in suite_results {
        let Some(suite) = suite.as_object() else {
            continue;
        };
        let runner_kind = object_text(suite, "runner_kind")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(runner_kind.as_str(), "task_batch" | "rust_task_batch") {
            continue;
        }
        let default_reason = object_text(suite, "selector");
        let selected_tasks = suite
            .get("selected_tasks")
            .and_then(JsonValue::as_array)
            .map(|tasks| {
                tasks
                    .iter()
                    .map(|task| {
                        let mut task = task.as_object().cloned().unwrap_or_default();
                        if let Some(reason) = default_reason.as_ref() {
                            let needs_reason = task
                                .get("selection_reason")
                                .and_then(json_value_to_text)
                                .is_none();
                            if needs_reason {
                                task.insert("selection_reason".to_string(), json!(reason));
                            }
                        }
                        JsonValue::Object(task)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let lineage = object_field(suite, "lineage_findings");
        let behavior = object_field(suite, "behavior_regressions");
        return json!({
            "suite_id": object_text(suite, "suite_id"),
            "selector": object_text(suite, "selector"),
            "selected_task_count": selected_tasks.len(),
            "selected_tasks": selected_tasks,
            "lineage_problem_count": lineage.get("problem_count").and_then(int_value).unwrap_or(0),
            "lineage_findings": lineage,
            "behavior_status": object_text(&behavior, "status").unwrap_or_else(|| "pending".to_string()),
            "behavior_regressions": behavior,
        });
    }
    JsonValue::Null
}

fn summary_artifacts_for_suite_results(suite_results: &[JsonValue]) -> Vec<JsonValue> {
    let mut artifacts = Vec::new();
    for suite in suite_results {
        let Some(suite) = suite.as_object() else {
            continue;
        };
        let suite_id = object_text(suite, "suite_id").unwrap_or_default();
        let artifacts_obj = object_field(suite, "artifacts");
        for key in ["summary_json", "summary_markdown"] {
            let Some(payload) = artifacts_obj.get(key).and_then(JsonValue::as_object) else {
                continue;
            };
            let Some(path) = object_text(payload, "path") else {
                continue;
            };
            artifacts.push(json!({
                "suite_id": suite_id,
                "artifact_key": key,
                "path": path,
                "exists": payload.get("exists").and_then(JsonValue::as_bool).unwrap_or(false),
                "size_bytes": payload.get("size_bytes").cloned().unwrap_or(JsonValue::Null),
            }));
        }
    }
    artifacts
}

fn repo_rerun_command(
    payload: &JsonMap<String, JsonValue>,
    result: &JsonMap<String, JsonValue>,
) -> String {
    let suite_ids = string_list(payload.get("suite_ids"));
    let suite_ids = if suite_ids.is_empty() {
        suite_ids_for_job(payload, result)
    } else {
        suite_ids
    };
    let mut parts = vec!["ait".to_string(), "repo".to_string(), "run-ci".to_string()];
    if let Some(plane) = object_text(payload, "plane") {
        parts.extend(["--plane".to_string(), plane]);
    } else {
        for suite_id in suite_ids {
            parts.extend(["--suite".to_string(), suite_id]);
        }
    }
    if let Some(target_line) = object_text(payload, "target_line") {
        if target_line != "main" {
            parts.extend(["--target-line".to_string(), target_line]);
        }
    }
    for (flag, field) in [
        ("--selector", "selector"),
        ("--curated-corpus", "curated_corpus"),
    ] {
        if let Some(value) = object_text(payload, field) {
            parts.extend([flag.to_string(), value]);
        }
    }
    for task_id in string_list(payload.get("task_ids")) {
        parts.extend(["--task-id".to_string(), task_id]);
    }
    for (flag, field) in [("--count", "count"), ("--window-days", "window_days")] {
        if let Some(value) = payload.get(field).and_then(json_value_to_text) {
            parts.extend([flag.to_string(), value]);
        }
    }
    cli_command(&parts)
}

fn cli_command(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|part| !part.trim().is_empty())
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':'))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn latest_summary(item: &JsonValue) -> JsonValue {
    json!({
        "job_id": item.get("job_id").cloned().unwrap_or(JsonValue::Null),
        "status": item.get("status").cloned().unwrap_or(JsonValue::Null),
        "state": item.get("state").cloned().unwrap_or(JsonValue::Null),
        "updated_at": item.get("updated_at").cloned().unwrap_or(JsonValue::Null),
    })
}

fn object_field(obj: &JsonMap<String, JsonValue>, field: &str) -> JsonMap<String, JsonValue> {
    obj.get(field)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

fn array_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Vec<JsonValue> {
    obj.get(field)
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
}

fn required_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Result<String, String> {
    object_text(obj, field).ok_or_else(|| format!("`{field}` must be a non-empty string."))
}

fn optional_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    obj.get(field).and_then(json_value_to_text)
}

fn object_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    object_text_field(obj, field)
}

fn value_text(value: &JsonValue, field: &str) -> Option<String> {
    value.as_object().and_then(|obj| object_text(obj, field))
}

fn normalize_optional_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn int_value(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn string_list(value: Option<&JsonValue>) -> Vec<String> {
    match value {
        Some(JsonValue::Array(items)) => {
            unique_strings(items.iter().filter_map(json_value_to_text))
        }
        Some(value) => json_value_to_text(value).into_iter().collect(),
        None => Vec::new(),
    }
}

fn string_set(value: Option<&JsonValue>) -> BTreeSet<String> {
    string_list(value).into_iter().collect()
}

fn unique_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeMap::new();
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() || seen.insert(trimmed.to_string(), ()).is_some() {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}
