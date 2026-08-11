use super::helpers::{required_text, required_text_from_object, unique_strs};
use serde_json::{json, Value as JsonValue};

pub fn patchset_ci_completion_json(request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "patchset-ci completion payload must be a JSON object.".to_string())?;
    let patchset = payload
        .get("patchset")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "patchset-ci completion payload must include `patchset`.".to_string())?;
    let suites = payload
        .get("suites")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "patchset-ci completion payload must include `suites`.".to_string())?;
    let tests_status = match required_text(payload, "tests_status")?.as_str() {
        "pass" | "passed" | "success" | "succeeded" => "pass",
        "fail" | "failed" | "failure" => "fail",
        "none" | "pending" | "queued" | "running" => "none",
        _ => "error",
    };
    let job_state = required_text(payload, "job_state")?;
    if job_state != "succeeded" {
        return Err(format!(
            "patchset-ci completion requires succeeded job_state, got `{job_state}`"
        ));
    }
    let suite_results = payload
        .get("suite_results")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let blocking_failures = unique_strs(
        payload
            .get("blocking_failures")
            .and_then(JsonValue::as_array),
    );
    let run_seq = patchset
        .get("ci_run_seq")
        .and_then(JsonValue::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| "patchset-ci completion requires patchset ci_run_seq".to_string())?;
    let overall_status = match tests_status {
        "pass" => "pass",
        "fail" => "fail",
        _ => "error",
    };
    let mut lint_status = "none";
    for result in &suite_results {
        let suite_id = result
            .get("suite_id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        if !matches!(suite_id, "cargo_fmt" | "rustfmt") {
            continue;
        }
        lint_status = match result.get("status").and_then(JsonValue::as_str) {
            Some("pass" | "passed" | "success" | "succeeded") if lint_status == "none" => "pass",
            Some("pass" | "passed" | "success" | "succeeded") => lint_status,
            Some("fail" | "failed" | "failure") if lint_status != "error" => "fail",
            Some("fail" | "failed" | "failure") => lint_status,
            Some(_) | None => "error",
        };
    }
    Ok(json!({
        "patchset_id": required_text_from_object(patchset, "patchset_id")?,
        "ci_run_seq": run_seq,
        "selected_suite_count": suites.len(),
        "suite_result_count": suite_results.len(),
        "blocking_failure_count": blocking_failures.len(),
        "overall_status": overall_status,
        "tests_status": tests_status,
        "lint_status": lint_status,
    }))
}
