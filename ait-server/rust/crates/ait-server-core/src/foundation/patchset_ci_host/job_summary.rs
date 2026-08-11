use super::helpers::{
    bounded_unique_strs, int_value, optional_int, optional_text, truncate_chars,
    STATUS_MAX_DIAGNOSTIC_CHARS, STATUS_MAX_ID_CHARS, STATUS_MAX_LIST_ITEMS,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

pub(super) fn is_patchset_ci_status_job(job: &JsonValue, patchset_id: &str) -> bool {
    let Some(job) = job.as_object() else {
        return false;
    };
    if !matches!(
        job.get("job_type").and_then(optional_text).as_deref(),
        Some("patchset.ci" | "patchset.ci.aggregate")
    ) {
        return false;
    }
    let payload_patchset_id = job
        .get("payload")
        .and_then(JsonValue::as_object)
        .and_then(|payload| payload.get("patchset_id"))
        .and_then(optional_text);
    payload_patchset_id
        .as_deref()
        .map(|value| value == patchset_id)
        .unwrap_or(true)
}

fn patchset_job_status(job: &JsonMap<String, JsonValue>) -> String {
    let result = job.get("result").and_then(JsonValue::as_object);
    if let Some(status) = result
        .and_then(|value| value.get("tests_status"))
        .and_then(optional_text)
    {
        return truncate_chars(&status, STATUS_MAX_ID_CHARS);
    }
    if result
        .and_then(|value| value.get("status"))
        .and_then(optional_text)
        .as_deref()
        == Some("attached")
    {
        return "pending".to_string();
    }
    let state = job
        .get("state")
        .and_then(optional_text)
        .unwrap_or_default()
        .to_lowercase();
    match state.as_str() {
        "succeeded" => "pending".to_string(),
        "failed" | "blocked" => "fail".to_string(),
        "queued" | "running" => "pending".to_string(),
        _ => {
            if state.is_empty() {
                "unknown".to_string()
            } else {
                truncate_chars(&state, STATUS_MAX_ID_CHARS)
            }
        }
    }
}

fn suite_ids_for_job(job: &JsonMap<String, JsonValue>) -> Vec<String> {
    let result = job.get("result").and_then(JsonValue::as_object);
    let selected = bounded_unique_strs(
        result
            .and_then(|value| value.get("selected_suite_ids"))
            .and_then(JsonValue::as_array),
        STATUS_MAX_LIST_ITEMS,
        STATUS_MAX_ID_CHARS,
    );
    if !selected.is_empty() {
        return selected;
    }
    let payload = job.get("payload").and_then(JsonValue::as_object);
    bounded_unique_strs(
        payload
            .and_then(|value| value.get("suite_ids"))
            .and_then(JsonValue::as_array),
        STATUS_MAX_LIST_ITEMS,
        STATUS_MAX_ID_CHARS,
    )
}

fn insert_bounded_text(
    out: &mut JsonMap<String, JsonValue>,
    source: &JsonMap<String, JsonValue>,
    field: &str,
    char_limit: usize,
) {
    if let Some(value) = source.get(field).and_then(optional_text) {
        out.insert(
            field.to_string(),
            JsonValue::String(truncate_chars(&value, char_limit)),
        );
    }
}

fn insert_scalar(
    out: &mut JsonMap<String, JsonValue>,
    source: &JsonMap<String, JsonValue>,
    field: &str,
) {
    match source.get(field) {
        Some(JsonValue::Bool(value)) => {
            out.insert(field.to_string(), JsonValue::Bool(*value));
        }
        Some(JsonValue::Number(value)) => {
            out.insert(field.to_string(), JsonValue::Number(value.clone()));
        }
        _ => {}
    }
}

fn bounded_text_or_null(value: Option<&JsonValue>, char_limit: usize) -> JsonValue {
    value
        .and_then(optional_text)
        .map(|value| JsonValue::String(truncate_chars(&value, char_limit)))
        .unwrap_or(JsonValue::Null)
}

fn compact_patchset_job_payload(payload: Option<&JsonMap<String, JsonValue>>) -> JsonValue {
    let Some(payload) = payload else {
        return json!({});
    };
    let mut out = JsonMap::new();
    for field in [
        "patchset_id",
        "change_id",
        "repo_name",
        "repo_id",
        "revision_snapshot_id",
        "snapshot_id",
        "stage",
        "plane",
        "target_line",
        "trigger",
        "execution_profile",
    ] {
        insert_bounded_text(&mut out, payload, field, STATUS_MAX_ID_CHARS);
    }
    for field in ["suite_ids", "task_ids"] {
        let values = bounded_unique_strs(
            payload.get(field).and_then(JsonValue::as_array),
            STATUS_MAX_LIST_ITEMS,
            STATUS_MAX_ID_CHARS,
        );
        if !values.is_empty() {
            out.insert(field.to_string(), json!(values));
        }
    }
    for field in ["change_seq", "patchset_number", "count", "window_days"] {
        insert_scalar(&mut out, payload, field);
    }
    JsonValue::Object(out)
}

fn compact_patchset_job_result(result: Option<&JsonMap<String, JsonValue>>) -> JsonValue {
    let Some(result) = result else {
        return json!({});
    };
    let mut out = JsonMap::new();
    for field in [
        "contract",
        "status",
        "patchset_id",
        "change_id",
        "repo_name",
        "target_line",
        "trigger",
        "execution_profile",
        "tests_status",
        "stage",
    ] {
        insert_bounded_text(&mut out, result, field, STATUS_MAX_ID_CHARS);
    }
    for field in ["selected_suite_ids", "blocking_failures", "suite_failures"] {
        let values = bounded_unique_strs(
            result.get(field).and_then(JsonValue::as_array),
            STATUS_MAX_LIST_ITEMS,
            STATUS_MAX_DIAGNOSTIC_CHARS,
        );
        if !values.is_empty() {
            out.insert(field.to_string(), json!(values));
        }
    }
    for field in ["admitted_cpu_tokens", "runner_parallelism"] {
        insert_scalar(&mut out, result, field);
    }
    let suite_result_count = result
        .get("suite_result_count")
        .and_then(|value| optional_int(Some(value)))
        .unwrap_or_else(|| {
            result
                .get("suite_results")
                .and_then(JsonValue::as_array)
                .map(Vec::len)
                .and_then(|value| i64::try_from(value).ok())
                .unwrap_or_default()
        })
        .max(0);
    let blocking_failure_count = result
        .get("blocking_failure_count")
        .and_then(|value| optional_int(Some(value)))
        .unwrap_or_else(|| {
            result
                .get("blocking_failures")
                .and_then(JsonValue::as_array)
                .map(Vec::len)
                .and_then(|value| i64::try_from(value).ok())
                .unwrap_or_default()
        })
        .max(0);
    out.insert("suite_result_count".to_string(), json!(suite_result_count));
    out.insert(
        "blocking_failure_count".to_string(),
        json!(blocking_failure_count),
    );
    JsonValue::Object(out)
}

fn compact_tg1_required_summary(value: Option<&JsonValue>) -> JsonValue {
    let Some(value) = value.and_then(JsonValue::as_object) else {
        return JsonValue::Null;
    };
    let mut out = JsonMap::new();
    for field in ["status", "validation_status", "pytest_status"] {
        insert_bounded_text(&mut out, value, field, STATUS_MAX_ID_CHARS);
    }
    for field in ["live_count", "minimum_count"] {
        insert_scalar(&mut out, value, field);
    }
    if out.is_empty() {
        JsonValue::Null
    } else {
        JsonValue::Object(out)
    }
}

fn compact_suite_result(value: &JsonValue) -> Option<JsonValue> {
    let source = value.as_object()?;
    let mut out = JsonMap::new();
    for field in ["suite_id", "status", "plane", "mode"] {
        insert_bounded_text(&mut out, source, field, STATUS_MAX_ID_CHARS);
    }
    for field in ["error", "message"] {
        insert_bounded_text(&mut out, source, field, STATUS_MAX_DIAGNOSTIC_CHARS);
    }
    for field in ["artifact_id", "artifact_path", "log_artifact_id"] {
        insert_bounded_text(&mut out, source, field, STATUS_MAX_DIAGNOSTIC_CHARS);
    }
    for field in ["blocking", "duration_ms", "duration_seconds", "exit_code"] {
        insert_scalar(&mut out, source, field);
    }
    let tg1_required_summary = compact_tg1_required_summary(source.get("tg1_required_summary"));
    if !tg1_required_summary.is_null() {
        out.insert("tg1_required_summary".to_string(), tg1_required_summary);
    }
    let detail_omitted = [
        "log",
        "logs",
        "stdout",
        "stderr",
        "output",
        "materialization",
        "attestation_update",
        "patchset_ci_completion",
    ]
    .iter()
    .any(|field| source.contains_key(*field));
    if detail_omitted {
        out.insert("detail_omitted".to_string(), JsonValue::Bool(true));
    }
    Some(JsonValue::Object(out))
}

pub(super) fn compact_suite_results(values: Option<&Vec<JsonValue>>) -> Vec<JsonValue> {
    values
        .into_iter()
        .flatten()
        .take(STATUS_MAX_LIST_ITEMS)
        .filter_map(compact_suite_result)
        .collect()
}

fn count_from_result(
    result: Option<&JsonMap<String, JsonValue>>,
    count_field: &str,
    list_field: &str,
) -> usize {
    result
        .and_then(|value| optional_int(value.get(count_field)))
        .and_then(|value| usize::try_from(value.max(0)).ok())
        .unwrap_or_else(|| {
            result
                .and_then(|value| value.get(list_field))
                .and_then(JsonValue::as_array)
                .map(Vec::len)
                .unwrap_or_default()
        })
}

pub(super) fn summarize_patchset_job(job: &JsonValue, patchset_id: &str) -> JsonValue {
    let Some(job) = job.as_object() else {
        return JsonValue::Null;
    };
    let result = job.get("result").and_then(JsonValue::as_object);
    let payload = job.get("payload").and_then(JsonValue::as_object);
    let suite_results = compact_suite_results(
        result
            .and_then(|value| value.get("suite_results"))
            .and_then(JsonValue::as_array),
    );
    let trigger = result
        .and_then(|value| value.get("trigger"))
        .and_then(optional_text)
        .or_else(|| {
            payload
                .and_then(|value| value.get("trigger"))
                .and_then(optional_text)
        })
        .map(|value| truncate_chars(&value, STATUS_MAX_ID_CHARS));
    let payload_patchset_id = payload
        .and_then(|value| value.get("patchset_id"))
        .and_then(optional_text)
        .map(|value| truncate_chars(&value, STATUS_MAX_ID_CHARS))
        .unwrap_or_else(|| truncate_chars(patchset_id, STATUS_MAX_ID_CHARS));
    let last_error = bounded_text_or_null(job.get("last_error"), STATUS_MAX_DIAGNOSTIC_CHARS);
    let blocking_failures = bounded_unique_strs(
        result
            .and_then(|value| value.get("blocking_failures"))
            .and_then(JsonValue::as_array),
        STATUS_MAX_LIST_ITEMS,
        STATUS_MAX_DIAGNOSTIC_CHARS,
    );
    let suite_result_count = count_from_result(result, "suite_result_count", "suite_results");

    json!({
        "job_id": int_value(job.get("job_id")),
        "job_type": bounded_text_or_null(job.get("job_type"), STATUS_MAX_ID_CHARS),
        "state": bounded_text_or_null(job.get("state"), STATUS_MAX_ID_CHARS),
        "diagnostic_status": bounded_text_or_null(job.get("diagnostic_status"), STATUS_MAX_ID_CHARS),
        "last_error": last_error,
        "retry_pending": job.get("retry_pending").and_then(JsonValue::as_bool).unwrap_or(false),
        "attempt_count": int_value(job.get("attempt_count")),
        "max_attempts": int_value(job.get("max_attempts")),
        "attempts_remaining": int_value(job.get("attempts_remaining")),
        "available_at": bounded_text_or_null(job.get("available_at"), STATUS_MAX_ID_CHARS),
        "trigger": trigger,
        "created_at": bounded_text_or_null(job.get("created_at"), STATUS_MAX_ID_CHARS),
        "updated_at": bounded_text_or_null(job.get("updated_at"), STATUS_MAX_ID_CHARS),
        "tests_status": patchset_job_status(job),
        "selected_suite_ids": suite_ids_for_job(job),
        "blocking_failures": blocking_failures,
        "suite_result_count": suite_result_count,
        "suite_results": suite_results,
        "rerun": {"cli": patchset_rerun_command(&payload_patchset_id)},
        "payload": compact_patchset_job_payload(payload),
        "result": compact_patchset_job_result(result),
    })
}

pub(super) fn summarize_patchset_job_readiness(job: &JsonValue, patchset_id: &str) -> JsonValue {
    let Some(job) = job.as_object() else {
        return JsonValue::Null;
    };
    let result = job.get("result").and_then(JsonValue::as_object);
    let payload = job.get("payload").and_then(JsonValue::as_object);
    let trigger = result
        .and_then(|value| value.get("trigger"))
        .and_then(optional_text)
        .or_else(|| {
            payload
                .and_then(|value| value.get("trigger"))
                .and_then(optional_text)
        })
        .map(|value| truncate_chars(&value, STATUS_MAX_ID_CHARS));
    let payload_patchset_id = payload
        .and_then(|value| value.get("patchset_id"))
        .and_then(optional_text)
        .map(|value| truncate_chars(&value, STATUS_MAX_ID_CHARS))
        .unwrap_or_else(|| truncate_chars(patchset_id, STATUS_MAX_ID_CHARS));
    let suite_result_count = count_from_result(result, "suite_result_count", "suite_results");
    let blocking_failure_count =
        count_from_result(result, "blocking_failure_count", "blocking_failures");
    let last_error = bounded_text_or_null(job.get("last_error"), STATUS_MAX_DIAGNOSTIC_CHARS);

    json!({
        "job_id": int_value(job.get("job_id")),
        "job_type": bounded_text_or_null(job.get("job_type"), STATUS_MAX_ID_CHARS),
        "state": bounded_text_or_null(job.get("state"), STATUS_MAX_ID_CHARS),
        "diagnostic_status": bounded_text_or_null(job.get("diagnostic_status"), STATUS_MAX_ID_CHARS),
        "last_error": last_error,
        "retry_pending": job.get("retry_pending").and_then(JsonValue::as_bool).unwrap_or(false),
        "attempt_count": int_value(job.get("attempt_count")),
        "max_attempts": int_value(job.get("max_attempts")),
        "attempts_remaining": int_value(job.get("attempts_remaining")),
        "available_at": bounded_text_or_null(job.get("available_at"), STATUS_MAX_ID_CHARS),
        "trigger": trigger,
        "created_at": bounded_text_or_null(job.get("created_at"), STATUS_MAX_ID_CHARS),
        "updated_at": bounded_text_or_null(job.get("updated_at"), STATUS_MAX_ID_CHARS),
        "tests_status": patchset_job_status(job),
        "selected_suite_ids": suite_ids_for_job(job),
        "suite_result_count": suite_result_count,
        "blocking_failure_count": blocking_failure_count,
        "rerun": {"cli": patchset_rerun_command(&payload_patchset_id)},
    })
}

pub(super) fn patchset_rerun_command(patchset_id: &str) -> String {
    let trimmed = patchset_id.trim();
    if trimmed.is_empty() {
        "ait patchset rerun-ci".to_string()
    } else {
        format!("ait patchset rerun-ci {trimmed}")
    }
}
