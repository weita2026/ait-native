use crate::foundation::ci_process_env::{
    apply_clean_ci_process_env, ci_process_environment_report, clean_ci_process_env,
};
use crate::foundation::ci_process_stream::{
    run_streamed_command, validated_ci_process_timeout_seconds, CiProcessExecutionOptions,
    CiProcessStdoutCapture,
};
use crate::foundation::test_shard_runtime::{
    ci_test_shard_cleanup_json, ci_test_shard_prepare_json,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Instant;

pub fn ci_test_shard_run_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request_object = request
        .as_object()
        .ok_or_else(|| "ci-test-shard-run payload must be a JSON object.".to_string())?;
    let runner = RunnerSpec::from_request(request_object)?;
    let cleanup_enabled = optional_bool(request_object, "cleanup")?.unwrap_or(true);

    let prepared = ci_test_shard_prepare_json(request)?;
    let shards = prepared["thread_pool_shards"]["shards"]
        .as_array()
        .ok_or_else(|| "ci-test-shard-prepare did not return shard array.".to_string())?;
    let merged_output_dir = merged_output_dir(request_object, &prepared)?;
    fs::create_dir_all(&merged_output_dir).map_err(|exc| {
        format!(
            "Failed to create merged output dir `{}`: {exc}",
            path_string(&merged_output_dir)
        )
    })?;

    let started = Instant::now();
    let mut handles = Vec::new();
    for shard in shards {
        let runner = runner.clone();
        let shard = shard.clone();
        handles.push(thread::spawn(move || run_one_shard(&runner, &shard)));
    }

    let mut shard_results = Vec::new();
    for handle in handles {
        match handle.join() {
            Ok(result) => shard_results.push(result),
            Err(_) => shard_results.push(Err("shard runner thread panicked".to_string())),
        }
    }

    let mut normalized_shard_results = Vec::new();
    let mut completed = true;
    let mut passed = true;
    for result in shard_results {
        match result {
            Ok(payload) => {
                if payload.get("status").and_then(JsonValue::as_str) != Some("pass") {
                    passed = false;
                }
                normalized_shard_results.push(payload);
            }
            Err(message) => {
                completed = false;
                passed = false;
                normalized_shard_results.push(json!({
                    "status": "fail",
                    "error": message,
                }));
            }
        }
    }

    let mut summary = json!({
        "contract": "ait.server.ci_test_shard_run.v1",
        "operation": "run",
        "job_type": request_object.get("job_type").cloned().unwrap_or(JsonValue::Null),
        "job_id": request_object.get("job_id").cloned().unwrap_or(JsonValue::Null),
        "payload": request_object.get("payload").cloned().unwrap_or(JsonValue::Null),
        "status": if passed { "pass" } else { "fail" },
        "duration_seconds": duration_seconds(started),
        "runner": runner.to_json(),
        "execution": prepared["execution"].clone(),
        "main_seed": prepared["main_seed"].clone(),
        "thread_pool_shards": {
            "shard_count": normalized_shard_results.len(),
            "shards": normalized_shard_results,
        },
        "merged_output_dir": path_string(&merged_output_dir),
    });

    let artifacts = write_merged_artifacts(request_object, &merged_output_dir, &summary)?;
    summary["artifacts"] = artifacts;

    if cleanup_enabled {
        let cleanup = cleanup_after_run(request, completed, true)?;
        summary["cleanup"] = cleanup;
    } else {
        summary["cleanup"] = json!({
            "status": "skipped",
            "reason": "cleanup=false"
        });
    }

    Ok(summary)
}

#[derive(Debug, Clone)]
struct RunnerSpec {
    kind: String,
    program: String,
    args: Vec<String>,
    append_test_items: bool,
    env: BTreeMap<String, String>,
    timeout_seconds: u64,
}

impl RunnerSpec {
    fn from_request(request: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let runner = required_object(request, "runner")?;
        let kind = optional_text(runner, "kind").unwrap_or_else(|| "command".to_string());
        let kind = kind.trim().to_ascii_lowercase();
        let args = optional_string_array(runner, "args")?.unwrap_or_default();
        let env = optional_string_map(runner, "env")?.unwrap_or_default();
        let append_test_items = optional_bool(runner, "append_test_items")?.unwrap_or(false);
        let program = optional_text(runner, "program").unwrap_or_default();
        if program.trim().is_empty() {
            return Err("runner.program is required for shard runners.".to_string());
        }
        let timeout_seconds = validated_ci_process_timeout_seconds(
            optional_positive_i64(runner, "timeout_seconds")?,
            "runner.timeout_seconds",
        )?;
        Ok(Self {
            kind,
            program,
            args,
            append_test_items,
            env,
            timeout_seconds,
        })
    }

    fn to_json(&self) -> JsonValue {
        json!({
            "kind": self.kind,
            "program": self.program,
            "args": self.args,
            "append_test_items": self.append_test_items,
            "env": self.env,
            "timeout_seconds": self.timeout_seconds,
            "process_environment": ci_process_environment_report(),
        })
    }
}

fn run_one_shard(runner: &RunnerSpec, shard: &JsonValue) -> Result<JsonValue, String> {
    let shard_id = shard["shard_id"]
        .as_str()
        .ok_or_else(|| "prepared shard is missing shard_id.".to_string())?
        .to_string();
    let repo_dir = path_from_json(&shard["repo_dir"], "shard.repo_dir")?;
    let output_dir = path_from_json(&shard["output_dir"], "shard.output_dir")?;
    fs::create_dir_all(&output_dir).map_err(|exc| {
        format!(
            "Failed to create shard output dir `{}`: {exc}",
            path_string(&output_dir)
        )
    })?;
    let assignment = shard
        .get("assignment")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "prepared shard is missing assignment object.".to_string())?;
    let test_items = string_array_from_value(assignment.get("test_items")).unwrap_or_default();
    if runner.append_test_items && test_items.is_empty() {
        let log_path = output_dir.join("runner.log");
        fs::write(&log_path, "no assigned test items\n").map_err(|exc| {
            format!(
                "Failed to write empty-shard log `{}`: {exc}",
                path_string(&log_path)
            )
        })?;
        return Ok(json!({
            "shard_id": shard_id,
            "status": "pass",
            "skipped": true,
            "reason": "no_assigned_test_items",
            "test_count": 0,
            "repo_dir": path_string(&repo_dir),
            "output_dir": path_string(&output_dir),
            "log_path": path_string(&log_path),
        }));
    }

    let mut command_environment = clean_ci_process_env(&runner.env);
    let resolved_program = resolve_runner_program(&runner.program, &command_environment)?;
    let mut command = Command::new(&resolved_program);
    command.current_dir(&repo_dir);
    command.args(&runner.args);
    if runner.append_test_items {
        command.args(&test_items);
    }
    command_environment.insert("AIT_SHARD_ID".to_string(), shard_id.clone());
    command_environment.insert("AIT_SHARD_REPO_DIR".to_string(), path_string(&repo_dir));
    command_environment.insert("AIT_SHARD_OUTPUT_DIR".to_string(), path_string(&output_dir));
    for key in [
        "AIT_REPO_ROOT",
        "AIT_NATIVE_WORKSPACE_ROOT",
        "AIT_WORKSPACE_ROOT",
    ] {
        command_environment.insert(key.to_string(), path_string(&repo_dir));
    }
    command_environment.insert(
        "AIT_TEST_ITEMS_JSON".to_string(),
        serde_json::to_string(&test_items).map_err(|exc| exc.to_string())?,
    );
    command_environment.insert("AIT_TEST_ITEMS".to_string(), test_items.join("\n"));
    apply_clean_ci_process_env(&mut command, &command_environment);

    let started = Instant::now();
    let log_path = output_dir.join("runner.log");
    let command_text = rendered_command(&resolved_program, runner, &test_items);
    let output = run_streamed_command(
        &mut command,
        &log_path,
        &command_text,
        &repo_dir,
        CiProcessStdoutCapture::None,
        CiProcessExecutionOptions::from_timeout_seconds(runner.timeout_seconds),
    )
    .map_err(|exc| {
        format!(
            "Failed to execute shard runner `{}` for shard `{shard_id}`: {exc}",
            runner.program
        )
    })?;

    Ok(json!({
        "shard_id": shard_id,
        "status": if output.status.success() { "pass" } else { "fail" },
        "exit_code": output.status.code(),
        "timed_out": output.timed_out,
        "timeout_seconds": runner.timeout_seconds,
        "duration_seconds": duration_seconds(started),
        "test_count": test_items.len(),
        "test_items": test_items,
        "repo_dir": path_string(&repo_dir),
        "output_dir": path_string(&output_dir),
        "log_path": path_string(&log_path),
        "stdout": output.stdout_tail,
        "stderr": output.stderr_tail,
        "stdout_bytes": output.stdout_bytes,
        "stderr_bytes": output.stderr_bytes,
    }))
}

fn resolve_runner_program(
    program: &str,
    runner_env: &BTreeMap<String, String>,
) -> Result<String, String> {
    if !program.contains('$') {
        return Ok(program.to_string());
    }

    let mut output = String::with_capacity(program.len());
    let mut chars = program.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '$' {
            output.push(ch);
            continue;
        }

        match chars.peek() {
            Some('{') => {
                chars.next();
                let mut variable = String::new();
                while let Some(&next_char) = chars.peek() {
                    if next_char == '}' {
                        break;
                    }
                    variable.push(next_char);
                    chars.next();
                }
                if chars.next() != Some('}') {
                    return Err(
                        "runner.program has unterminated ${...} variable reference.".to_string()
                    );
                }
                if variable.is_empty() {
                    return Err("runner.program has empty ${} variable reference.".to_string());
                }
                let value = runner_program_env_value(&variable, runner_env).ok_or_else(|| {
                    format!("runner.program references missing environment variable `{variable}`.")
                })?;
                output.push_str(value);
            }
            Some(ch) if is_env_var_name_start(*ch) => {
                let mut variable = String::new();
                while let Some(&next_char) = chars.peek() {
                    if is_env_var_name_char(next_char) {
                        variable.push(next_char);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let value = runner_program_env_value(&variable, runner_env).ok_or_else(|| {
                    format!("runner.program references missing environment variable `{variable}`.")
                })?;
                output.push_str(value);
            }
            Some(_) | None => {
                output.push('$');
            }
        }
    }

    Ok(output)
}

fn runner_program_env_value<'a>(
    variable: &str,
    runner_env: &'a BTreeMap<String, String>,
) -> Option<&'a str> {
    runner_env.get(variable).map(String::as_str)
}

fn is_env_var_name_start(value: char) -> bool {
    value == '_' || value.is_ascii_alphabetic()
}

fn is_env_var_name_char(value: char) -> bool {
    is_env_var_name_start(value) || value.is_ascii_digit()
}

fn cleanup_after_run(
    original_request: &JsonValue,
    all_shards_completed: bool,
    outputs_merged: bool,
) -> Result<JsonValue, String> {
    let mut cleanup_request = original_request.clone();
    let cleanup_object = cleanup_request
        .as_object_mut()
        .ok_or_else(|| "cleanup request must be a JSON object.".to_string())?;
    if all_shards_completed && outputs_merged {
        cleanup_object.insert(
            "cleanup_reason".to_string(),
            JsonValue::String("all_assigned_tests_complete".to_string()),
        );
        cleanup_object.insert("all_shards_completed".to_string(), JsonValue::Bool(true));
        cleanup_object.insert("outputs_merged".to_string(), JsonValue::Bool(true));
    } else {
        cleanup_object.insert(
            "cleanup_reason".to_string(),
            JsonValue::String("core_token_reclaimed".to_string()),
        );
    }
    ci_test_shard_cleanup_json(&cleanup_request)
}

fn merged_output_dir(
    request: &JsonMap<String, JsonValue>,
    prepared: &JsonValue,
) -> Result<PathBuf, String> {
    if let Some(path) = optional_text(request, "merged_output_dir") {
        return Ok(PathBuf::from(path));
    }
    let root = path_from_json(
        &prepared["thread_pool_shards"]["shards"][0]["path"],
        "thread_pool_shards.shards[0].path",
    )?
    .parent()
    .ok_or_else(|| "prepared shard path has no parent.".to_string())?
    .join("merged-output");
    Ok(root)
}

fn write_merged_artifacts(
    request: &JsonMap<String, JsonValue>,
    merged_output_dir: &Path,
    summary: &JsonValue,
) -> Result<JsonValue, String> {
    let artifacts = request
        .get("artifacts")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let summary_rel =
        optional_text(&artifacts, "summary_json").unwrap_or_else(|| "summary.json".to_string());
    let log_rel = optional_text(&artifacts, "log_path").unwrap_or_else(|| "run.log".to_string());
    let summary_path = merged_output_dir.join(&summary_rel);
    let log_path = merged_output_dir.join(&log_rel);
    if let Some(parent) = summary_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create summary artifact parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create log artifact parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    fs::write(
        &summary_path,
        serde_json::to_string_pretty(summary).map_err(|exc| exc.to_string())? + "\n",
    )
    .map_err(|exc| {
        format!(
            "Failed to write summary artifact `{}`: {exc}",
            path_string(&summary_path)
        )
    })?;
    fs::write(&log_path, merged_log_text(summary)).map_err(|exc| {
        format!(
            "Failed to write merged log artifact `{}`: {exc}",
            path_string(&log_path)
        )
    })?;
    Ok(json!({
        "summary_json": artifact_payload(&summary_path),
        "log_path": artifact_payload(&log_path),
    }))
}

fn merged_log_text(summary: &JsonValue) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "status={}",
        summary
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown")
    ));
    if let Some(shards) = summary["thread_pool_shards"]["shards"].as_array() {
        for shard in shards {
            lines.push(format!(
                "shard={} status={} exit_code={}",
                shard
                    .get("shard_id")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("unknown"),
                shard
                    .get("status")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("unknown"),
                shard
                    .get("exit_code")
                    .and_then(JsonValue::as_i64)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_string())
            ));
        }
    }
    lines.join("\n") + "\n"
}

fn artifact_payload(path: &Path) -> JsonValue {
    let size = fs::metadata(path)
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len());
    json!({
        "path": path_string(path),
        "exists": path.exists(),
        "size_bytes": size,
    })
}

fn rendered_command(program: &str, runner: &RunnerSpec, test_items: &[String]) -> String {
    let mut parts = Vec::from([program.to_string()]);
    parts.extend(runner.args.clone());
    if runner.append_test_items {
        parts.extend(test_items.to_vec());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn expands_env_vars_in_runner_program() {
        let runner_env = clean_ci_process_env(&BTreeMap::new());
        let home = runner_env
            .get("HOME")
            .expect("HOME is expected in clean test environments");
        assert_eq!(
            resolve_runner_program("$HOME/bin/ait-cli", &runner_env)
                .expect("runner should expand $HOME"),
            format!("{home}/bin/ait-cli"),
        );
        assert_eq!(
            resolve_runner_program("${HOME}/bin/ait-cli", &runner_env)
                .expect("runner should expand ${HOME}"),
            format!("{home}/bin/ait-cli"),
        );
    }

    #[test]
    fn expands_runner_env_vars_before_process_env() {
        let explicit = BTreeMap::from([(
            "AIT_TEST_RUNNER_BIN_DIR".to_string(),
            "/tmp/ait-runner-bin".to_string(),
        )]);
        let runner_env = clean_ci_process_env(&explicit);

        assert_eq!(
            resolve_runner_program("$AIT_TEST_RUNNER_BIN_DIR/ait-cli", &runner_env)
                .expect("runner env should expand"),
            "/tmp/ait-runner-bin/ait-cli",
        );
    }

    #[test]
    fn errors_when_runner_program_env_var_is_missing() {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should progress");
        let missing = format!("AIT_TEST_SHARD_MISSING_ENV_{}", duration.as_nanos());
        let runner_env = clean_ci_process_env(&BTreeMap::new());
        assert!(!runner_env.contains_key(&missing));
        let error = resolve_runner_program(&format!("${{{missing}}}/bin/ait-cli"), &runner_env)
            .expect_err("missing variable should fail");
        assert!(error.contains("missing"));
        assert!(error.contains(&missing));
    }
}

fn duration_seconds(started: Instant) -> f64 {
    let millis = started.elapsed().as_millis() as f64;
    (millis / 1000.0 * 1000.0).round() / 1000.0
}

fn required_object<'a>(
    value: &'a JsonMap<String, JsonValue>,
    key: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .get(key)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Field `{key}` must be a JSON object."))
}

fn optional_positive_i64(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<i64>, String> {
    let Some(raw) = value.get(key) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(parsed) = raw.as_i64() else {
        return Err(format!("Field `{key}` must be a positive integer."));
    };
    if parsed < 1 {
        return Err(format!("Field `{key}` must be a positive integer."));
    }
    Ok(Some(parsed))
}

fn optional_text(value: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_bool(value: &JsonMap<String, JsonValue>, key: &str) -> Result<Option<bool>, String> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("Field `{key}` must be a boolean.")),
    }
}

fn optional_string_array(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(values) = value.get(key) else {
        return Ok(None);
    };
    let values = values
        .as_array()
        .ok_or_else(|| format!("Field `{key}` must be an array of non-empty strings."))?;
    Ok(Some(string_array_from_array(values, key)?))
}

fn string_array_from_value(value: Option<&JsonValue>) -> Result<Vec<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(values)) => string_array_from_array(values, "test_items"),
        Some(_) => Err("test_items must be an array of non-empty strings.".to_string()),
    }
}

fn string_array_from_array(values: &[JsonValue], key: &str) -> Result<Vec<String>, String> {
    let mut parsed = Vec::new();
    for value in values {
        let item = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Field `{key}` must contain non-empty strings."))?;
        parsed.push(item.to_string());
    }
    Ok(parsed)
}

fn optional_string_map(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(raw) = value.get(key) else {
        return Ok(None);
    };
    let raw = raw
        .as_object()
        .ok_or_else(|| format!("Field `{key}` must be an object of string values."))?;
    let mut parsed = BTreeMap::new();
    for (entry_key, entry_value) in raw {
        let text = entry_value
            .as_str()
            .ok_or_else(|| format!("Field `{key}.{entry_key}` must be a string."))?;
        parsed.insert(entry_key.clone(), text.to_string());
    }
    Ok(Some(parsed))
}

fn path_from_json(value: &JsonValue, field: &str) -> Result<PathBuf, String> {
    let text = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Field `{field}` must be a non-empty string path."))?;
    Ok(PathBuf::from(text))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
