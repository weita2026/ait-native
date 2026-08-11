use super::helpers::{
    bounded_unique_strs, bounded_unique_strs_from_values, int_value, optional_int, optional_text,
    required_text, truncate_chars, STATUS_MAX_ID_CHARS, STATUS_MAX_LIST_ITEMS,
    STATUS_MAX_RECENT_JOBS, TG1_REQUIRED_SUITE_ID,
};
use super::job_summary::{
    compact_suite_results, is_patchset_ci_status_job, patchset_rerun_command,
    summarize_patchset_job, summarize_patchset_job_readiness,
};
use serde_json::{json, Value as JsonValue};

pub fn patchset_ci_embedded_status_summary_json(patchset_ci: Option<&JsonValue>) -> JsonValue {
    let patchset_ci = patchset_ci.and_then(JsonValue::as_object);
    let selected_suite_ids = bounded_unique_strs(
        patchset_ci
            .and_then(|value| value.get("selected_suite_ids"))
            .and_then(JsonValue::as_array),
        STATUS_MAX_LIST_ITEMS,
        STATUS_MAX_ID_CHARS,
    );
    let blocking_failures = bounded_unique_strs(
        patchset_ci
            .and_then(|value| value.get("blocking_failures"))
            .and_then(JsonValue::as_array),
        STATUS_MAX_LIST_ITEMS,
        STATUS_MAX_ID_CHARS,
    );
    let source_suite_results = patchset_ci
        .and_then(|value| value.get("suite_results"))
        .and_then(JsonValue::as_array);
    let suite_result_count = patchset_ci
        .and_then(|value| optional_int(value.get("suite_result_count")))
        .and_then(|count| usize::try_from(count.max(0)).ok())
        .unwrap_or_else(|| source_suite_results.map(Vec::len).unwrap_or_default());
    let selected_suite_count = patchset_ci
        .and_then(|value| optional_int(value.get("selected_suite_count")))
        .and_then(|count| usize::try_from(count.max(0)).ok())
        .unwrap_or_else(|| selected_suite_ids.len());
    let blocking_failure_count = patchset_ci
        .and_then(|value| optional_int(value.get("blocking_failure_count")))
        .and_then(|count| usize::try_from(count.max(0)).ok())
        .unwrap_or_else(|| blocking_failures.len());
    let suite_results = compact_suite_results(source_suite_results);
    let tests_status = patchset_ci
        .and_then(|value| value.get("tests_status"))
        .and_then(optional_text)
        .map(|value| truncate_chars(&value, STATUS_MAX_ID_CHARS))
        .unwrap_or_default();
    json!({
        "run_seq": patchset_ci
            .and_then(|value| optional_int(value.get("run_seq")))
            .unwrap_or_default(),
        "completed_at_s": patchset_ci
            .and_then(|value| optional_int(value.get("completed_at_s")))
            .unwrap_or_default(),
        "tests_status": tests_status,
        "overall_status": patchset_ci
            .and_then(|value| value.get("overall_status"))
            .and_then(optional_text)
            .unwrap_or_default(),
        "lint_status": patchset_ci
            .and_then(|value| value.get("lint_status"))
            .and_then(optional_text)
            .unwrap_or_default(),
        "selected_suite_ids": selected_suite_ids,
        "selected_suite_count": selected_suite_count,
        "blocking_failures": blocking_failures,
        "suite_results": suite_results,
        "suite_result_count": suite_result_count,
        "blocking_failure_count": blocking_failure_count,
        "suite_results_truncated": suite_result_count > STATUS_MAX_LIST_ITEMS,
    })
}

pub fn patchset_ci_status_summary_json(request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "patchset-ci status-summary payload must be a JSON object.".to_string())?;
    let patchset_id = truncate_chars(&required_text(payload, "patchset_id")?, STATUS_MAX_ID_CHARS);
    let change_id = truncate_chars(&required_text(payload, "change_id")?, STATUS_MAX_ID_CHARS);
    let change_ref = payload
        .get("change_ref")
        .and_then(optional_text)
        .map(|value| truncate_chars(&value, STATUS_MAX_ID_CHARS))
        .unwrap_or_else(|| change_id.clone());
    let repo_name = truncate_chars(&required_text(payload, "repo_name")?, STATUS_MAX_ID_CHARS);
    let projection = payload.get("projection").and_then(optional_text);
    if projection
        .as_deref()
        .is_some_and(|value| value != "readiness")
    {
        return Err(format!(
            "Unsupported patchset CI status projection `{}`. Expected `readiness`.",
            projection.as_deref().unwrap_or_default()
        ));
    }
    let readiness_projection = projection.as_deref() == Some("readiness");
    let requested_recent_limit = int_value(payload.get("recent_limit")).max(0) as usize;
    let recent_limit = requested_recent_limit.min(STATUS_MAX_RECENT_JOBS);
    let jobs = payload
        .get("jobs")
        .and_then(JsonValue::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let embedded_available = payload
        .get("embedded_patchset_ci")
        .and_then(JsonValue::as_object)
        .map(|value| {
            optional_int(value.get("run_seq")).unwrap_or_default() > 0
                || optional_int(value.get("completed_at_s")).unwrap_or_default() > 0
                || value
                    .get("overall_status")
                    .and_then(optional_text)
                    .is_some_and(|status| status != "none")
        })
        .unwrap_or(false);
    let compact_embedded =
        patchset_ci_embedded_status_summary_json(payload.get("embedded_patchset_ci"));
    let embedded_patchset_ci = embedded_available
        .then(|| compact_embedded.as_object())
        .flatten();
    let embedded_completed_at_s = embedded_patchset_ci
        .and_then(|value| optional_int(value.get("completed_at_s")))
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default();
    let completed_at_s = payload
        .get("ci_completed_at_s")
        .and_then(JsonValue::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(embedded_completed_at_s);
    let run_seq = embedded_patchset_ci
        .and_then(|value| optional_int(value.get("run_seq")))
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or_default();
    let embedded_completed = run_seq > 0 && completed_at_s > 0;
    let ci_completed_at_s = (completed_at_s > 0)
        .then(|| JsonValue::from(completed_at_s))
        .unwrap_or(JsonValue::Null);
    let ci_run_seq = (run_seq > 0)
        .then(|| JsonValue::from(run_seq))
        .unwrap_or(JsonValue::Null);

    let recent_jobs: Vec<JsonValue> = jobs
        .iter()
        .filter(|job| is_patchset_ci_status_job(job, &patchset_id))
        .take(recent_limit)
        .map(|job| {
            if readiness_projection {
                summarize_patchset_job_readiness(job, &patchset_id)
            } else {
                summarize_patchset_job(job, &patchset_id)
            }
        })
        .collect();
    let latest_job = recent_jobs.first().cloned().unwrap_or(JsonValue::Null);

    let suite_results = if readiness_projection {
        Vec::new()
    } else {
        compact_suite_results(
            embedded_patchset_ci
                .and_then(|value| value.get("suite_results"))
                .and_then(JsonValue::as_array),
        )
    };
    let selected_suite_ids = bounded_unique_strs(
        embedded_patchset_ci
            .and_then(|value| value.get("selected_suite_ids"))
            .and_then(JsonValue::as_array),
        STATUS_MAX_LIST_ITEMS,
        STATUS_MAX_ID_CHARS,
    );
    let blocking_failures = bounded_unique_strs(
        embedded_patchset_ci
            .and_then(|value| value.get("blocking_failures"))
            .and_then(JsonValue::as_array),
        STATUS_MAX_LIST_ITEMS,
        STATUS_MAX_ID_CHARS,
    );
    let mut tests_status = embedded_patchset_ci
        .and_then(|value| value.get("tests_status"))
        .and_then(optional_text)
        .unwrap_or_default();
    if tests_status.is_empty() {
        tests_status = latest_job
            .as_object()
            .and_then(|value| value.get("tests_status"))
            .and_then(optional_text)
            .unwrap_or_default();
    }
    if tests_status.is_empty() {
        tests_status = "pending".to_string();
    }
    if readiness_projection
        && !embedded_completed
        && matches!(tests_status.as_str(), "pass" | "none")
    {
        tests_status = "pending".to_string();
    }

    let final_selected_suite_ids = if selected_suite_ids.is_empty() {
        latest_job
            .as_object()
            .and_then(|value| value.get("selected_suite_ids"))
            .and_then(JsonValue::as_array)
            .map(|items| {
                bounded_unique_strs_from_values(items, STATUS_MAX_LIST_ITEMS, STATUS_MAX_ID_CHARS)
            })
            .unwrap_or_default()
    } else {
        selected_suite_ids
    };
    let final_blocking_failures = if blocking_failures.is_empty() {
        latest_job
            .as_object()
            .and_then(|value| value.get("blocking_failures"))
            .and_then(JsonValue::as_array)
            .map(|items| {
                bounded_unique_strs_from_values(items, STATUS_MAX_LIST_ITEMS, STATUS_MAX_ID_CHARS)
            })
            .unwrap_or_default()
    } else {
        blocking_failures
    };
    let final_suite_results = if suite_results.is_empty() {
        latest_job
            .as_object()
            .and_then(|value| value.get("suite_results"))
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default()
    } else {
        suite_results
    };

    let embedded_suite_result_count = embedded_patchset_ci
        .and_then(|value| optional_int(value.get("suite_result_count")))
        .and_then(|count| usize::try_from(count.max(0)).ok())
        .unwrap_or_default();
    let suite_result_count = (embedded_suite_result_count > 0)
        .then_some(embedded_suite_result_count)
        .or_else(|| {
            latest_job
                .get("suite_result_count")
                .and_then(JsonValue::as_u64)
                .and_then(|count| usize::try_from(count).ok())
        })
        .unwrap_or_else(|| final_suite_results.len());
    let embedded_blocking_failure_count = embedded_patchset_ci
        .and_then(|value| optional_int(value.get("blocking_failure_count")))
        .and_then(|count| usize::try_from(count.max(0)).ok())
        .unwrap_or_default();
    let blocking_failure_count = (embedded_blocking_failure_count > 0)
        .then_some(embedded_blocking_failure_count)
        .or_else(|| {
            latest_job
                .get("blocking_failure_count")
                .and_then(JsonValue::as_u64)
                .and_then(|count| usize::try_from(count).ok())
        })
        .unwrap_or_else(|| final_blocking_failures.len());

    if readiness_projection && tests_status == "pass" {
        if blocking_failure_count > 0 {
            tests_status = "fail".to_string();
        } else if suite_result_count == 0 {
            tests_status = "pending".to_string();
        }
    }
    let has_runnable_evidence = embedded_completed
        && (embedded_suite_result_count > 0 || embedded_blocking_failure_count > 0);

    let tg1_required = normalize_tg1_required_summary(
        &final_suite_results,
        &final_selected_suite_ids,
        &final_blocking_failures,
        &tests_status,
    );
    let status_notice = patchset_ci_status_notice(&latest_job);
    let recommended_action = status_notice
        .as_object()
        .and_then(|notice| notice.get("recommended_action"))
        .cloned()
        .unwrap_or(JsonValue::Null);

    let available = embedded_available || !latest_job.is_null();
    if readiness_projection {
        return Ok(json!({
            "contract": "ait.server.patchset_ci.readiness.v1",
            "projection": "readiness",
            "patchset_id": patchset_id,
            "change_id": change_id,
            "change_ref": change_ref,
            "repo_name": repo_name,
            "available": available,
            "tests_status": tests_status,
            "selected_suite_ids": final_selected_suite_ids,
            "suite_result_count": suite_result_count,
            "blocking_failure_count": blocking_failure_count,
            "has_runnable_evidence": has_runnable_evidence,
            "ci_run_seq": ci_run_seq,
            "ci_completed_at_s": ci_completed_at_s,
            "recent_limit_applied": recent_limit,
            "latest_job": latest_job,
            "recent_jobs": recent_jobs,
            "status_notice": status_notice,
            "recommended_action": recommended_action,
            "rerun": {"cli": patchset_rerun_command(&patchset_id)},
        }));
    }

    Ok(json!({
        "patchset_id": patchset_id,
        "change_id": change_id,
        "change_ref": change_ref,
        "repo_name": repo_name,
        "available": available,
        "tests_status": tests_status,
        "selected_suite_ids": final_selected_suite_ids,
        "blocking_failures": final_blocking_failures,
        "suite_result_count": suite_result_count,
        "blocking_failure_count": blocking_failure_count,
        "suite_results": final_suite_results,
        "detail_bounded": true,
        "ci_run_seq": ci_run_seq,
        "recent_limit_applied": recent_limit,
        "tg1_required": tg1_required,
        "ci_completed_at_s": ci_completed_at_s,
        "latest_job": latest_job,
        "recent_jobs": recent_jobs,
        "status_notice": status_notice,
        "recommended_action": recommended_action,
        "rerun": {"cli": patchset_rerun_command(&patchset_id)},
    }))
}

fn normalize_tg1_required_summary(
    suite_results: &[JsonValue],
    selected_suite_ids: &[String],
    blocking_failures: &[String],
    tests_status: &str,
) -> JsonValue {
    let suite_result = suite_results.iter().find_map(|item| {
        let object = item.as_object()?;
        let suite_id = object.get("suite_id").and_then(optional_text)?;
        (suite_id == TG1_REQUIRED_SUITE_ID).then(|| object.clone())
    });
    let summary = suite_result
        .as_ref()
        .and_then(|item| item.get("tg1_required_summary"))
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    if suite_result.is_none()
        && !selected_suite_ids
            .iter()
            .any(|item| item == TG1_REQUIRED_SUITE_ID)
    {
        return JsonValue::Null;
    }
    let mut status = summary
        .get("status")
        .and_then(optional_text)
        .or_else(|| {
            suite_result
                .as_ref()
                .and_then(|item| item.get("status"))
                .and_then(optional_text)
        })
        .unwrap_or_default()
        .to_lowercase();
    if status.is_empty() {
        if blocking_failures
            .iter()
            .any(|item| item == TG1_REQUIRED_SUITE_ID)
        {
            status = "fail".to_string();
        } else if matches!(tests_status, "pending" | "queued" | "running") {
            status = "pending".to_string();
        } else if selected_suite_ids
            .iter()
            .any(|item| item == TG1_REQUIRED_SUITE_ID)
        {
            status = "pass".to_string();
        }
    }
    json!({
        "status": if status.is_empty() { JsonValue::Null } else { JsonValue::String(status) },
        "validation_status": summary.get("validation_status").and_then(optional_text),
        "pytest_status": summary.get("pytest_status").and_then(optional_text),
        "live_count": optional_int(summary.get("live_count")),
        "minimum_count": optional_int(summary.get("minimum_count")),
    })
}

fn patchset_ci_status_notice(latest_job: &JsonValue) -> JsonValue {
    let Some(job) = latest_job.as_object() else {
        return JsonValue::Null;
    };
    let last_error = job
        .get("last_error")
        .and_then(optional_text)
        .unwrap_or_default();
    if last_error.is_empty() {
        return JsonValue::Null;
    }
    if last_error.starts_with("Patchset CI reset after land moved ") {
        return json!({
            "contract": "ait.server.patchset_ci.status_notice.v1",
            "kind": "patchset_ci_reset_after_land",
            "severity": "action_required",
            "message": last_error,
            "recommended_action": "rebase_patchset_to_latest_main",
            "tests_status_semantics": "pending_not_test_failure",
            "agent_visible": true,
        });
    }
    let retry_pending = job
        .get("retry_pending")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if retry_pending {
        return json!({
            "contract": "ait.server.patchset_ci.status_notice.v1",
            "kind": "patchset_ci_retry_pending",
            "severity": "wait",
            "message": last_error,
            "recommended_action": "wait_for_retry_or_rerun_ci",
            "tests_status_semantics": "pending_retry",
            "agent_visible": true,
        });
    }
    JsonValue::Null
}
