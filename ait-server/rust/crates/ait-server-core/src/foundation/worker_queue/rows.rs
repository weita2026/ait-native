use super::*;

const WORKER_QUEUE_SUMMARY_MAX_TEXT_CHARS: usize = 4096;
const WORKER_QUEUE_SUMMARY_MAX_ID_CHARS: usize = 256;
const WORKER_QUEUE_SUMMARY_MAX_LIST_ITEMS: usize = 64;
const WORKER_QUEUE_STORAGE_MAX_DEPTH: usize = 4;
const WORKER_QUEUE_STORAGE_MAX_BYTES: usize = 256 * 1024;
const WORKER_QUEUE_STORAGE_REFERENCE_MAX_TEXT_CHARS: usize = 512;

const CI_STORAGE_OPTIONAL_FIELDS: [&str; 12] = [
    "blocking_failures",
    "suite_failures",
    "artifacts",
    "server_ci_gate",
    "scheduler_admission",
    "native_prewarm",
    "suite_pool",
    "policy_job_payload",
    "patchset_ci_write",
    "cleanup",
    "shard_cleanup",
    "base_stale_rerun",
];

pub(super) fn shape_job_rows(
    rows: &[JsonMap<String, JsonValue>],
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    rows.iter().map(row_to_job).collect()
}

pub(super) fn object_rows(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    let value = obj
        .get(field)
        .ok_or_else(|| format!("worker queue kernel payload requires `{field}`."))?;
    let rows = value
        .as_array()
        .ok_or_else(|| format!("`{field}` must be an array."))?;
    rows.iter()
        .map(|row| {
            row.as_object()
                .cloned()
                .ok_or_else(|| format!("`{field}` rows must be JSON objects."))
        })
        .collect()
}

pub(super) fn text_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Result<String, String> {
    optional_text(obj, field)
        .ok_or_else(|| format!("worker queue kernel payload requires `{field}`."))
}

pub(super) fn optional_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    obj.get(field).and_then(value_text)
}

pub(super) fn i64_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Result<i64, String> {
    obj.get(field)
        .and_then(value_i64)
        .ok_or_else(|| format!("worker queue kernel payload requires integer `{field}`."))
}

pub(super) fn optional_i64(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<i64>, String> {
    match obj.get(field) {
        Some(JsonValue::Null) | None => Ok(None),
        Some(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
            .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
            .map(Some)
            .ok_or_else(|| format!("worker queue payload field `{field}` must be an integer.")),
    }
}

pub(super) fn optional_bool(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<bool> {
    obj.get(field).and_then(|value| match value {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

pub(super) fn value_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

pub(super) fn value_i64(value: &JsonValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

pub(super) fn row_text(row: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    row.get(field).and_then(value_text)
}

pub(super) fn row_i64(row: &JsonMap<String, JsonValue>, field: &str) -> i64 {
    row.get(field).and_then(value_i64).unwrap_or(0)
}

pub(super) fn row_bool(row: &JsonMap<String, JsonValue>, field: &str) -> bool {
    row.get(field)
        .and_then(|value| match value {
            JsonValue::Bool(value) => Some(*value),
            JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Some(true),
                "false" | "0" | "no" => Some(false),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(false)
}

pub(super) fn job_id_i64(row: &JsonMap<String, JsonValue>) -> i64 {
    row_i64(row, "job_id")
}

pub(super) fn count_jobs_by(
    jobs: &[JsonMap<String, JsonValue>],
    field: &str,
) -> JsonMap<String, JsonValue> {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for job in jobs {
        let key = row_text(job, field).unwrap_or_else(|| "unknown".to_string());
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(key, value)| (key, json!(value)))
        .collect()
}

pub(super) fn parse_job_id(value: &str) -> Result<i64, String> {
    value
        .trim()
        .parse::<i64>()
        .map_err(|_| format!("scheduler returned non-integer job id `{value}`"))
}

pub(super) fn repo_matches(row: &JsonMap<String, JsonValue>, repo_name: Option<&str>) -> bool {
    let Some(repo_name) = repo_name else {
        return true;
    };
    row_text(row, "repo_name").as_deref() == Some(repo_name)
}

pub(super) fn repo_scope_matches(
    row: &JsonMap<String, JsonValue>,
    repo_name: &str,
    repo_id: Option<&str>,
) -> bool {
    if let Some(repo_id) = repo_id {
        row_text(row, "repo_id").as_deref() == Some(repo_id)
            || (row_text(row, "repo_id").is_none()
                && row_text(row, "repo_name").as_deref() == Some(repo_name))
    } else {
        row_text(row, "repo_name").as_deref() == Some(repo_name)
    }
}

pub(super) fn sorted_rows(
    mut rows: Vec<JsonMap<String, JsonValue>>,
) -> Vec<JsonMap<String, JsonValue>> {
    rows.sort_by_key(job_id_i64);
    rows
}

pub(super) fn clear_lease(row: &mut JsonMap<String, JsonValue>) {
    row.insert("locked_at".to_string(), JsonValue::Null);
    row.insert("locked_by".to_string(), JsonValue::Null);
}

pub(super) fn retry_at_from_now(now: &str, retry_delay_seconds: i64) -> String {
    DateTime::parse_from_rfc3339(now)
        .map(|parsed| (parsed + Duration::seconds(retry_delay_seconds.max(0))).to_rfc3339())
        .unwrap_or_else(|_| now.to_string())
}

pub fn utc_now_string() -> String {
    Utc::now().to_rfc3339()
}

pub(super) fn postgres_job_row_to_json(row: &Row) -> JsonMap<String, JsonValue> {
    let repo_id: Option<String> = row.get("repo_id");
    let locked_at: Option<String> = row.get("locked_at");
    let locked_by: Option<String> = row.get("locked_by");
    let last_error: Option<String> = row.get("last_error");
    JsonMap::from_iter([
        ("job_id".to_string(), json!(row.get::<_, i64>("job_id"))),
        (
            "repo_name".to_string(),
            json!(row.get::<_, String>("repo_name")),
        ),
        (
            "repo_id".to_string(),
            repo_id.map(JsonValue::from).unwrap_or(JsonValue::Null),
        ),
        (
            "job_type".to_string(),
            json!(row.get::<_, String>("job_type")),
        ),
        ("state".to_string(), json!(row.get::<_, String>("state"))),
        (
            "payload_json".to_string(),
            json!(row.get::<_, String>("payload_json")),
        ),
        (
            "result_json".to_string(),
            json!(row.get::<_, String>("result_json")),
        ),
        (
            "attempt_count".to_string(),
            json!(row.get::<_, i32>("attempt_count")),
        ),
        (
            "max_attempts".to_string(),
            json!(row.get::<_, i32>("max_attempts")),
        ),
        (
            "available_at".to_string(),
            json!(row.get::<_, String>("available_at")),
        ),
        (
            "locked_at".to_string(),
            locked_at.map(JsonValue::from).unwrap_or(JsonValue::Null),
        ),
        (
            "locked_by".to_string(),
            locked_by.map(JsonValue::from).unwrap_or(JsonValue::Null),
        ),
        (
            "last_error".to_string(),
            last_error.map(JsonValue::from).unwrap_or(JsonValue::Null),
        ),
        (
            "created_at".to_string(),
            json!(row.get::<_, String>("created_at")),
        ),
        (
            "updated_at".to_string(),
            json!(row.get::<_, String>("updated_at")),
        ),
    ])
}

fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn compact_summary_json_object(
    source: &JsonMap<String, JsonValue>,
    text_fields: &[&str],
    list_fields: &[&str],
    scalar_fields: &[&str],
) -> JsonValue {
    let mut out = JsonMap::new();
    for field in text_fields {
        if let Some(value) = source.get(*field).and_then(JsonValue::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                out.insert(
                    (*field).to_string(),
                    JsonValue::String(truncate_chars(value, WORKER_QUEUE_SUMMARY_MAX_ID_CHARS)),
                );
            }
        }
    }
    for field in list_fields {
        let values = source
            .get(*field)
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
            .filter_map(JsonValue::as_str)
            .take(WORKER_QUEUE_SUMMARY_MAX_LIST_ITEMS)
            .map(|value| {
                JsonValue::String(truncate_chars(
                    value.trim(),
                    WORKER_QUEUE_SUMMARY_MAX_ID_CHARS,
                ))
            })
            .collect::<Vec<_>>();
        if !values.is_empty() {
            out.insert((*field).to_string(), JsonValue::Array(values));
        }
    }
    for field in scalar_fields {
        match source.get(*field) {
            Some(JsonValue::Bool(value)) => {
                out.insert((*field).to_string(), JsonValue::Bool(*value));
            }
            Some(JsonValue::Number(value)) => {
                out.insert((*field).to_string(), JsonValue::Number(value.clone()));
            }
            _ => {}
        }
    }
    JsonValue::Object(out)
}

fn compact_worker_queue_payload(payload: &JsonValue) -> JsonValue {
    payload
        .as_object()
        .map(|payload| {
            compact_summary_json_object(
                payload,
                &[
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
                    "suite_id",
                    "previous_snapshot_id",
                    "selector",
                    "curated_corpus",
                    "submission_id",
                    "idempotency_key",
                    "transport",
                ],
                &[
                    "suite_ids",
                    "task_ids",
                    "dependency_evidence",
                    "compliance_evidence",
                ],
                &[
                    "change_seq",
                    "patchset_number",
                    "land_seq",
                    "count",
                    "window_days",
                    "max_members",
                    "prune_unreferenced",
                    "prune_orphan_packs",
                    "repair",
                    "repack",
                ],
            )
        })
        .unwrap_or_else(|| json!({}))
}

fn compact_worker_queue_result(result: &JsonValue) -> JsonValue {
    let has_suite_result_count = result.get("suite_result_count").is_some()
        || result.get("suite_results").is_some()
        || result.get("selected_suite_ids").is_some()
        || result.get("all_patchset_suite_ids").is_some();
    let has_blocking_failure_count = result.get("blocking_failure_count").is_some()
        || result.get("blocking_failures").is_some()
        || result.get("blocking_suite_ids").is_some();
    let suite_result_count = result
        .get("suite_result_count")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_else(|| {
            result
                .get("suite_results")
                .and_then(JsonValue::as_array)
                .map(Vec::len)
                .unwrap_or_default()
        });
    let blocking_failure_count = result
        .get("blocking_failure_count")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_else(|| {
            result
                .get("blocking_failures")
                .and_then(JsonValue::as_array)
                .map(Vec::len)
                .unwrap_or_default()
        });
    let mut result = result
        .as_object()
        .map(|result| {
            compact_summary_json_object(
                result,
                &[
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
                    "plane",
                    "submission_id",
                    "task_id",
                    "snapshot_id",
                    "revision_snapshot_id",
                ],
                &[
                    "selected_suite_ids",
                    "all_patchset_suite_ids",
                    "blocking_suite_ids",
                    "blocking_failures",
                    "suite_failures",
                    "completed_suite_ids",
                ],
                &[
                    "admitted_cpu_tokens",
                    "runner_parallelism",
                    "count",
                    "processed_count",
                    "failed_count",
                ],
            )
        })
        .unwrap_or_else(|| json!({}));
    if let Some(result_object) = result.as_object_mut() {
        if has_suite_result_count {
            result_object.insert("suite_result_count".to_string(), json!(suite_result_count));
        }
        if has_blocking_failure_count {
            result_object.insert(
                "blocking_failure_count".to_string(),
                json!(blocking_failure_count),
            );
        }
    }
    result
}

fn compact_worker_queue_storage_evidence(value: &JsonValue, remaining_depth: usize) -> JsonValue {
    match value {
        JsonValue::String(text) => {
            JsonValue::String(truncate_chars(text, WORKER_QUEUE_SUMMARY_MAX_TEXT_CHARS))
        }
        JsonValue::Array(values) if values.len() > WORKER_QUEUE_SUMMARY_MAX_LIST_ITEMS => json!({
            "item_count": values.len(),
            "detail_omitted": true,
        }),
        JsonValue::Object(values) if values.len() > WORKER_QUEUE_SUMMARY_MAX_LIST_ITEMS => json!({
            "field_count": values.len(),
            "detail_omitted": true,
        }),
        JsonValue::Array(values) if remaining_depth > 0 => JsonValue::Array(
            values
                .iter()
                .map(|value| compact_worker_queue_storage_evidence(value, remaining_depth - 1))
                .collect(),
        ),
        JsonValue::Object(values) if remaining_depth > 0 => JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        compact_worker_queue_storage_evidence(value, remaining_depth - 1),
                    )
                })
                .collect(),
        ),
        JsonValue::Array(values) => json!({
            "item_count": values.len(),
            "detail_omitted": true,
        }),
        JsonValue::Object(values) => json!({
            "field_count": values.len(),
            "detail_omitted": true,
        }),
        _ => value.clone(),
    }
}

fn compact_ci_suite_result_for_storage(value: &JsonValue) -> JsonValue {
    let Some(source) = value.as_object() else {
        return compact_worker_queue_storage_evidence(value, 1);
    };
    let mut out = JsonMap::new();
    for field in [
        "suite_id",
        "display_name",
        "status",
        "blocking",
        "mode",
        "plane",
        "runner_kind",
        "artifact_path",
        "purpose",
        "duration_seconds",
        "artifacts",
        "failure",
        "failure_reason",
        "checks",
        "doc_tests",
        "summary",
        "test_counts",
        "execution",
        "server_ci_gate",
    ] {
        if let Some(value) = source.get(field).filter(|value| !value.is_null()) {
            out.insert(
                field.to_string(),
                compact_worker_queue_storage_evidence(value, WORKER_QUEUE_STORAGE_MAX_DEPTH),
            );
        }
    }
    JsonValue::Object(out)
}

fn compact_ci_suite_result_reference_for_storage(value: &JsonValue) -> JsonValue {
    let Some(source) = value.as_object() else {
        return json!({"detail_omitted": true});
    };
    let mut out = JsonMap::new();
    for field in [
        "suite_id",
        "status",
        "blocking",
        "mode",
        "plane",
        "artifact_path",
    ] {
        let Some(value) = source.get(field).filter(|value| !value.is_null()) else {
            continue;
        };
        let value = match value {
            JsonValue::String(text) => JsonValue::String(truncate_chars(
                text,
                WORKER_QUEUE_STORAGE_REFERENCE_MAX_TEXT_CHARS,
            )),
            JsonValue::Bool(_) | JsonValue::Number(_) => value.clone(),
            _ => continue,
        };
        out.insert(field.to_string(), value);
    }
    JsonValue::Object(out)
}

fn worker_queue_storage_exceeds_byte_limit(value: &JsonMap<String, JsonValue>) -> bool {
    serde_json::to_vec(value)
        .map(|encoded| encoded.len() > WORKER_QUEUE_STORAGE_MAX_BYTES)
        .unwrap_or(true)
}

pub fn compact_ci_job_result_for_storage(job_type: &str, result: &JsonValue) -> JsonValue {
    if !matches!(
        job_type,
        "patchset.ci" | "patchset.ci.aggregate" | "repo.ci"
    ) {
        return result.clone();
    }

    let mut compact = compact_worker_queue_result(result);
    let Some(out) = compact.as_object_mut() else {
        return compact;
    };
    out.insert(
        "storage_contract".to_string(),
        json!("ait.server.worker_queue.ci_result_summary.v1"),
    );
    out.insert(
        "detail_authority".to_string(),
        json!("postgresql_ci_jobs_and_artifacts"),
    );

    if let Some(suite_results) = result.get("suite_results").and_then(JsonValue::as_array) {
        out.insert(
            "suite_results".to_string(),
            JsonValue::Array(
                suite_results
                    .iter()
                    .take(WORKER_QUEUE_SUMMARY_MAX_LIST_ITEMS)
                    .map(compact_ci_suite_result_for_storage)
                    .collect(),
            ),
        );
        out.insert("suite_result_count".to_string(), json!(suite_results.len()));
    }

    for field in CI_STORAGE_OPTIONAL_FIELDS
        .into_iter()
        .chain(["base_stale_after_land"])
    {
        if let Some(value) = result.get(field).filter(|value| !value.is_null()) {
            out.insert(
                field.to_string(),
                compact_worker_queue_storage_evidence(value, WORKER_QUEUE_STORAGE_MAX_DEPTH),
            );
        }
    }

    let omitted_fields = [
        "attestation",
        "attestation_update",
        "patchset_ci_completion",
        "patchset_ci_detail",
        "repo_ci_detail",
    ]
    .into_iter()
    .filter(|field| result.get(*field).is_some())
    .map(JsonValue::from)
    .collect::<Vec<_>>();
    if !omitted_fields.is_empty() {
        out.insert(
            "omitted_duplicate_fields".to_string(),
            JsonValue::Array(omitted_fields),
        );
    }

    if worker_queue_storage_exceeds_byte_limit(out) {
        for field in CI_STORAGE_OPTIONAL_FIELDS {
            out.remove(field);
        }
        out.remove("base_stale_after_land");
        if let Some(suite_results) = result.get("suite_results").and_then(JsonValue::as_array) {
            out.insert(
                "suite_results".to_string(),
                JsonValue::Array(
                    suite_results
                        .iter()
                        .take(WORKER_QUEUE_SUMMARY_MAX_LIST_ITEMS)
                        .map(compact_ci_suite_result_reference_for_storage)
                        .collect(),
                ),
            );
        }
        out.insert("storage_detail_truncated".to_string(), json!(true));
    }
    if worker_queue_storage_exceeds_byte_limit(out) {
        out.remove("suite_results");
        out.insert("suite_results_omitted".to_string(), json!(true));
    }
    compact
}

pub fn compact_job_result_for_storage(job_type: &str, result: &JsonValue) -> JsonValue {
    let compact = compact_ci_job_result_for_storage(job_type, result);
    let compact_exceeds_byte_limit = compact
        .as_object()
        .map(worker_queue_storage_exceeds_byte_limit)
        .unwrap_or_else(|| {
            serde_json::to_vec(&compact)
                .map(|encoded| encoded.len() > WORKER_QUEUE_STORAGE_MAX_BYTES)
                .unwrap_or(true)
        });
    if !compact_exceeds_byte_limit {
        return compact;
    }

    let original_result_bytes = serde_json::to_vec(result)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX);
    let mut summary = compact_worker_queue_result(result);
    let Some(out) = summary.as_object_mut() else {
        return json!({
            "storage_contract": "ait.server.worker_queue.result_summary.v1",
            "job_type": truncate_chars(job_type, WORKER_QUEUE_SUMMARY_MAX_ID_CHARS),
            "original_result_bytes": original_result_bytes,
            "storage_detail_truncated": true,
        });
    };
    out.insert(
        "storage_contract".to_string(),
        json!("ait.server.worker_queue.result_summary.v1"),
    );
    out.insert(
        "job_type".to_string(),
        json!(truncate_chars(job_type, WORKER_QUEUE_SUMMARY_MAX_ID_CHARS)),
    );
    out.insert(
        "original_result_bytes".to_string(),
        json!(original_result_bytes),
    );
    out.insert("storage_detail_truncated".to_string(), json!(true));

    if worker_queue_storage_exceeds_byte_limit(out) {
        let mut minimal = JsonMap::new();
        for field in ["contract", "status", "repo_name", "snapshot_id"] {
            if let Some(value) = out.get(field) {
                minimal.insert(field.to_string(), value.clone());
            }
        }
        minimal.insert(
            "storage_contract".to_string(),
            json!("ait.server.worker_queue.result_summary.v1"),
        );
        minimal.insert(
            "job_type".to_string(),
            json!(truncate_chars(job_type, WORKER_QUEUE_SUMMARY_MAX_ID_CHARS)),
        );
        minimal.insert(
            "original_result_bytes".to_string(),
            json!(original_result_bytes),
        );
        minimal.insert("storage_detail_truncated".to_string(), json!(true));
        return JsonValue::Object(minimal);
    }
    summary
}

fn compact_worker_queue_metadata_row(
    row: &JsonMap<String, JsonValue>,
) -> JsonMap<String, JsonValue> {
    let mut out = JsonMap::new();
    for field in [
        "job_id",
        "repo_name",
        "repo_id",
        "job_type",
        "state",
        "attempt_count",
        "max_attempts",
        "available_at",
        "locked_at",
        "locked_by",
        "created_at",
        "updated_at",
    ] {
        if let Some(value) = row.get(field) {
            out.insert(field.to_string(), value.clone());
        }
    }
    let last_error = row
        .get("last_error")
        .and_then(JsonValue::as_str)
        .map(|value| JsonValue::String(truncate_chars(value, WORKER_QUEUE_SUMMARY_MAX_TEXT_CHARS)))
        .unwrap_or(JsonValue::Null);
    out.insert("last_error".to_string(), last_error);
    out
}

fn parse_row_json(row: &JsonMap<String, JsonValue>, field: &str) -> Result<JsonValue, String> {
    row.get(field)
        .and_then(JsonValue::as_str)
        .map(serde_json::from_str::<JsonValue>)
        .transpose()
        .map_err(|error| format!("{field} must be valid JSON: {error}"))
        .map(|value| value.unwrap_or_else(|| json!({})))
}

pub(super) fn compact_worker_queue_index_row(
    row: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = compact_worker_queue_metadata_row(row);
    out.insert("payload_json".to_string(), json!("{}"));
    out.insert("result_json".to_string(), json!("{}"));
    Ok(out)
}

pub(super) fn compact_worker_queue_readiness_row(
    row: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let payload = compact_worker_queue_payload(&parse_row_json(row, "payload_json")?);
    let result = compact_worker_queue_result(&parse_row_json(row, "result_json")?);
    let mut out = compact_worker_queue_metadata_row(row);
    out.insert("payload_json".to_string(), json!(payload.to_string()));
    out.insert("result_json".to_string(), json!(result.to_string()));
    Ok(out)
}

pub(super) fn compact_worker_queue_completion_row(
    row: &JsonMap<String, JsonValue>,
    result: &JsonValue,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = compact_worker_queue_metadata_row(row);
    out.insert("payload_json".to_string(), json!("{}"));
    out.insert(
        "result_json".to_string(),
        json!(compact_worker_queue_result(result).to_string()),
    );
    Ok(out)
}

pub(super) fn compact_worker_queue_summary_row(
    row: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let payload = compact_worker_queue_payload(&parse_row_json(row, "payload_json")?);
    let result = compact_worker_queue_result(&parse_row_json(row, "result_json")?);
    let mut out = compact_worker_queue_metadata_row(row);
    out.insert("payload_json".to_string(), json!(payload.to_string()));
    out.insert("result_json".to_string(), json!(result.to_string()));
    Ok(out)
}

pub(super) fn postgres_int4(field: &str, value: i64) -> Result<i32, String> {
    i32::try_from(value).map_err(|_| format!("{field} must fit PostgreSQL int4, got {value}"))
}

pub(super) fn postgres_timestamptz(field: &str, value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(value)
        .or_else(|_| DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f%:z"))
        .or_else(|_| DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f%#z"))
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|exc| format!("{field} must be a timestamptz-compatible timestamp: {exc}"))
}
