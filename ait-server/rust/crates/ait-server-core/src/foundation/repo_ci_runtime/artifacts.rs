use super::*;

pub(super) fn failing_suite_ids(suite_results: &[JsonValue]) -> Vec<String> {
    suite_results
        .iter()
        .filter(|suite| suite.get("status").and_then(JsonValue::as_str) != Some("pass"))
        .filter_map(|suite| {
            suite
                .get("suite_id")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>()
}

pub(super) fn build_repo_ci_detail(
    config: &RepoCiRuntimeConfig,
    suite_results: &[JsonValue],
    native_prewarm: Option<JsonValue>,
    tests_status: &str,
) -> JsonValue {
    let suite_failures = failing_suite_ids(suite_results);
    let suite_status = if suite_failures.is_empty() {
        "pass"
    } else {
        "fail"
    };
    let blocking_failures = suite_results
        .iter()
        .filter(|suite| {
            suite.get("blocking").and_then(JsonValue::as_bool) == Some(true)
                && suite.get("status").and_then(JsonValue::as_str) != Some("pass")
        })
        .filter_map(|suite| {
            suite
                .get("suite_id")
                .and_then(JsonValue::as_str)
                .map(str::to_string)
        })
        .collect::<Vec<_>>();
    json!({
        "trigger": config.trigger,
        "repo_name": config.repo_name,
        "repo_id": config.repo_id,
        "snapshot_id": config.snapshot_id,
        "target_line": config.target_line,
        "plane": config.plane,
        "selected_suite_ids": suite_results.iter().filter_map(|suite| suite.get("suite_id").and_then(JsonValue::as_str)).collect::<Vec<_>>(),
        "blocking_suite_ids": suite_results.iter().filter(|suite| suite.get("blocking").and_then(JsonValue::as_bool) == Some(true)).filter_map(|suite| suite.get("suite_id").and_then(JsonValue::as_str)).collect::<Vec<_>>(),
        "blocking_failures": blocking_failures,
        "tests_status": tests_status,
        "suite_status": suite_status,
        "suite_failures": suite_failures,
        "suite_results": suite_results,
        "native_prewarm": native_prewarm,
        "server_ci_gate": {
            "component": "ait-server-core",
            "python_server_ci_executor": false,
            "rust_repo_ci_runtime": true,
        }
    })
}

pub(super) fn build_result(
    config: &RepoCiRuntimeConfig,
    detail: JsonValue,
    suite_results: Vec<JsonValue>,
    native_prewarm: Option<JsonValue>,
) -> JsonValue {
    json!({
        "contract": "ait.server.repo_ci.run.v1",
        "repo_name": config.repo_name,
        "repo_id": config.repo_id,
        "snapshot_id": config.snapshot_id,
        "target_line": config.target_line,
        "trigger": config.trigger,
        "plane": config.plane,
        "tests_status": detail["tests_status"].clone(),
        "selected_suite_ids": detail["selected_suite_ids"].clone(),
        "blocking_suite_ids": detail["blocking_suite_ids"].clone(),
        "blocking_failures": detail["blocking_failures"].clone(),
        "suite_status": detail["suite_status"].clone(),
        "suite_failures": detail["suite_failures"].clone(),
        "suite_results": suite_results,
        "native_prewarm": native_prewarm,
        "repo_ci_detail": detail,
        "server_ci_gate": {
            "component": "ait-server-core",
            "python_server_ci_executor": false,
            "rust_repo_ci_runtime": true
        }
    })
}

pub(super) fn artifact_payload(path: &Path) -> JsonValue {
    let size = fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len());
    json!({
        "path": path_string(path),
        "exists": path.is_file(),
        "size_bytes": size,
    })
}

pub(super) fn optional_json_text(value: Option<&str>) -> JsonValue {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| json!(value))
        .unwrap_or(JsonValue::Null)
}
