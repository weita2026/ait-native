use super::*;

pub(super) fn run_one_suite(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> Result<JsonValue, String> {
    let kind = suite
        .runner
        .get("kind")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .unwrap_or("");
    let result = match kind {
        "command_bundle" => run_command_bundle_suite(config, suite)?,
        "test_shard" if is_full_test_suite(suite) => run_full_test_shard_suite(config, suite)?,
        "test_shard" => {
            return Err(format!(
                "repo CI runner kind `test_shard` is only supported for full-test suites; suite `{}` must use `command_bundle` or `task_batch`.",
                suite.suite_id
            ));
        }
        "pytest" => {
            return Err(format!(
                "repo CI runner kind `pytest` is not supported in ait-server; suite `{}` must use `command_bundle`, `task_batch`, or full-test `test_shard` with an explicit runner.program.",
                suite.suite_id
            ));
        }
        "task_batch" => run_task_batch_suite(config, suite)?,
        value => {
            return Err(format!(
                "Unsupported repo CI runner kind `{value}` for suite `{}`.",
                suite.suite_id
            ));
        }
    };
    let status = result
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("fail")
        .to_string();
    let mut payload = JsonMap::new();
    payload.insert("suite_id".to_string(), json!(suite.suite_id.trim()));
    payload.insert(
        "display_name".to_string(),
        optional_json_text(suite.display_name.as_deref()),
    );
    payload.insert(
        "artifact_path".to_string(),
        optional_json_text(suite.artifact_path.as_deref()),
    );
    payload.insert("plane".to_string(), json!(suite.plane.trim()));
    payload.insert("mode".to_string(), json!(suite.mode.trim()));
    payload.insert("blocking".to_string(), json!(suite.default_blocking));
    payload.insert(
        "purpose".to_string(),
        optional_json_text(suite.purpose.as_deref()),
    );
    payload.insert("runner_kind".to_string(), json!(rust_runner_kind(kind)));
    if let Some(value) = result.get("runner_kind").and_then(JsonValue::as_str) {
        payload.insert("runner_kind".to_string(), json!(value));
    }
    payload.insert("status".to_string(), json!(status));
    payload.insert(
        "artifacts".to_string(),
        result
            .get("artifacts")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    payload.insert(
        "duration_seconds".to_string(),
        result
            .get("duration_seconds")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    if let Some(reports) = result.get("command_reports") {
        payload.insert("command_reports".to_string(), reports.clone());
    }
    if let Some(failure) = result.get("failure") {
        payload.insert("failure".to_string(), failure.clone());
    }
    if let Some(summary) = result.get("task_batch_summary") {
        payload.insert("task_batch_summary".to_string(), summary.clone());
    }
    for key in [
        "main_seed",
        "main_seed_prewarm",
        "test_collection",
        "execution",
        "thread_pool_shards",
        "cleanup",
        "shard_run",
    ] {
        if let Some(value) = result.get(key) {
            payload.insert(key.to_string(), value.clone());
        }
    }
    if suite.plane.trim() == "release" {
        payload.insert(
            "release_gate_evidence".to_string(),
            release_gate_evidence(config, suite),
        );
    }
    payload.insert(
        "server_ci_gate".to_string(),
        json!({
            "component": "ait-server-core",
            "python_server_ci_executor": false,
            "rust_repo_ci_runtime": true,
        }),
    );
    Ok(JsonValue::Object(payload))
}

pub(super) fn run_command_bundle_suite(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> Result<JsonValue, String> {
    let mut payload =
        command_bundle_base_payload(config, config.output_dir.join(suite.suite_id.trim()));
    payload.insert("suite_id".to_string(), json!(suite.suite_id.trim()));
    payload.insert("runner".to_string(), suite.runner.clone());
    if let Some(artifacts) = suite_value(config, suite)
        .and_then(|value| value.get("artifacts"))
        .cloned()
    {
        payload.insert("artifacts".to_string(), artifacts);
    }
    ci_command_bundle_run_json(&JsonValue::Object(payload))
}

pub(super) fn run_task_batch_suite(
    config: &RepoCiRuntimeConfig,
    suite: &PatchsetSuiteManifest,
) -> Result<JsonValue, String> {
    let suite_id = suite.suite_id.trim();
    let input = config
        .task_batch_inputs
        .get(suite_id)
        .or_else(|| config.task_batch_inputs.get("*"))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            format!(
                "repo CI task_batch suite `{suite_id}` requires explicit task_batch_inputs for Rust execution."
            )
        })?;
    let status = optional_text(input, "status").unwrap_or_else(|| "pass".to_string());
    let output_dir = config.output_dir.join(suite_id);
    fs::create_dir_all(&output_dir).map_err(|exc| {
        format!(
            "Failed to create task_batch output dir `{}`: {exc}",
            path_string(&output_dir)
        )
    })?;
    let summary = json!({
        "status": status,
        "selector": input.get("selector").cloned().unwrap_or(JsonValue::Null),
        "selected_tasks": input.get("selected_tasks").cloned().unwrap_or_else(|| json!([])),
        "lineage_findings": input.get("lineage_findings").cloned().unwrap_or_else(|| json!({})),
        "behavior_regressions": input.get("behavior_regressions").cloned().unwrap_or_else(|| json!({})),
        "server_ci_gate": {
            "component": "ait-server-core",
            "python_server_ci_executor": false,
            "rust_repo_ci_runtime": true,
        }
    });
    let summary_path = output_dir.join("task_batch_summary.json");
    fs::write(
        &summary_path,
        serde_json::to_string_pretty(&summary).map_err(|exc| exc.to_string())? + "\n",
    )
    .map_err(|exc| format!("Failed to write `{}`: {exc}", path_string(&summary_path)))?;
    let log_path = output_dir.join("task_batch.log");
    fs::write(
        &log_path,
        format!(
            "suite={suite_id}\nstatus={status}\nselector={}\n",
            summary
                .get("selector")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
        ),
    )
    .map_err(|exc| format!("Failed to write `{}`: {exc}", path_string(&log_path)))?;
    Ok(json!({
        "status": status,
        "duration_seconds": 0.0,
        "task_batch_summary": summary,
        "artifacts": {
            "summary_json": artifact_payload(&summary_path),
            "log_path": artifact_payload(&log_path),
        }
    }))
}

pub(super) fn command_bundle_base_payload(
    config: &RepoCiRuntimeConfig,
    output_dir: PathBuf,
) -> JsonMap<String, JsonValue> {
    let mut payload = JsonMap::new();
    payload.insert(
        "workspace_path".to_string(),
        json!(path_string(&config.workspace_path)),
    );
    payload.insert("output_dir".to_string(), json!(path_string(&output_dir)));
    payload.insert("job_type".to_string(), json!("repo.ci"));
    payload.insert("job_id".to_string(), json!(config.snapshot_id));
    if let Some(path) = &config.temp_dir {
        payload.insert("temp_dir".to_string(), json!(path_string(path)));
    }
    if let Some(path) = &config.shared_cargo_target_dir {
        payload.insert(
            "shared_cargo_target_dir".to_string(),
            json!(path_string(path)),
        );
    }
    if let Some(path) = &config.shared_cargo_build_dir {
        payload.insert(
            "shared_cargo_build_dir".to_string(),
            json!(path_string(path)),
        );
    }
    payload.insert("env".to_string(), JsonValue::Object(config.env.clone()));
    payload
}

pub(super) fn rust_runner_kind(kind: &str) -> &'static str {
    match kind {
        "test_shard" => "rust_test_shard",
        "task_batch" => "rust_task_batch",
        _ => "rust_repo_ci",
    }
}
