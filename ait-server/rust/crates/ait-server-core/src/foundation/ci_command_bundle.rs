use crate::foundation::ci_process_env::{
    apply_clean_ci_process_env, ci_process_environment_report, clean_ci_process_env,
};
use crate::foundation::ci_process_stream::{
    run_streamed_command, validated_ci_process_timeout_seconds, CiProcessExecutionOptions,
    CiProcessStdoutCapture,
};
use crate::foundation::ci_runtime_json::CommandBundleRunJson;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const CONTRACT: &str = "ait.server.ci_command_bundle_run.v1";

pub fn ci_command_bundle_run_json(request: &JsonValue) -> Result<JsonValue, String> {
    CommandBundleRunJson::stateless().run(request)
}

pub(crate) fn ci_command_bundle_run_json_impl(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "ci-command-bundle-run payload must be a JSON object.".to_string())?;
    let config = CommandBundleConfig::from_request(request)?;
    fs::create_dir_all(&config.output_dir).map_err(|exc| {
        format!(
            "Failed to create CI command-bundle output dir `{}`: {exc}",
            path_string(&config.output_dir)
        )
    })?;

    let started = Instant::now();
    let mut env = command_env(&config);
    let mut prewarm_reports = Vec::new();
    let mut command_reports = Vec::new();
    let mut status = "pass";
    let mut failure = JsonValue::Null;

    for (index, command) in config.prewarm_commands.iter().enumerate() {
        let report = run_shell_command(
            "prewarm",
            index + 1,
            command,
            &config.workspace_path,
            &config.output_dir,
            &env,
            config.timeout_seconds,
        )?;
        if report.status != "pass" {
            status = "fail";
            failure = report.failure_json("prewarm");
            prewarm_reports.push(report.to_json());
            break;
        }
        prewarm_reports.push(report.to_json());
    }

    if status == "pass" && !config.prewarm_only {
        env.insert("AIT_CI_PREWARM_COMPLETE".to_string(), "1".to_string());
        for (index, command) in config.commands.iter().enumerate() {
            let report = run_shell_command(
                "command",
                index + 1,
                command,
                &config.workspace_path,
                &config.output_dir,
                &env,
                config.timeout_seconds,
            )?;
            if report.status != "pass" {
                status = "fail";
                failure = report.failure_json("command");
                command_reports.push(report.to_json());
                break;
            }
            command_reports.push(report.to_json());
        }
    }

    let mut summary = json!({
        "contract": CONTRACT,
        "status": status,
        "duration_seconds": duration_seconds(started),
        "suite_id": config.suite_id,
        "job_type": config.job_type,
        "job_id": config.job_id,
        "workspace_path": path_string(&config.workspace_path),
        "output_dir": path_string(&config.output_dir),
        "runner": {
            "kind": "command_bundle",
            "command_count": config.commands.len(),
            "prewarm_command_count": config.prewarm_commands.len(),
            "timeout_seconds": config.timeout_seconds
        },
        "prewarm": {
            "status": if prewarm_reports.iter().all(report_passed) { "pass" } else { "fail" },
            "required": !config.prewarm_commands.is_empty(),
            "once_per_bundle": true,
            "reports": prewarm_reports
        },
        "command_reports": command_reports,
        "failure": failure,
        "environment": {
            "process_policy": ci_process_environment_report(),
            "shared_cargo_target_dir": config.shared_cargo_target_dir.as_ref().map(|path| path_string(path)),
            "shared_cargo_build_dir": config.shared_cargo_build_dir.as_ref().map(|path| path_string(path)),
            "temp_dir": config.temp_dir.as_ref().map(|path| path_string(path)),
            "output_dir": path_string(&config.output_dir),
            "workspace_path": path_string(&config.workspace_path),
            "runner_parallelism": config.runner_parallelism,
            "admitted_cpu_tokens": config.runner_parallelism,
            "parallelism_source": if config.runner_parallelism.is_some() { "scheduler" } else { "unspecified" },
            "cargo_build_jobs_env": config.runner_parallelism.map(|value| value.to_string()),
            "rust_test_threads_env": config.runner_parallelism.map(|value| value.to_string())
        },
        "diagnostics": {
            "full_logs_retained": true,
            "json_output_uses_tail": true,
            "stop_after_first_failed_command": true,
            "prewarm_and_commands_share_environment": true,
            "scheduler_parallelism_controls_command_environment": config.runner_parallelism.is_some()
        }
    });
    let artifacts = write_artifacts(&config, &summary)?;
    summary["artifacts"] = artifacts;
    Ok(summary)
}

#[derive(Debug)]
struct CommandBundleConfig {
    workspace_path: PathBuf,
    output_dir: PathBuf,
    suite_id: Option<String>,
    job_type: Option<String>,
    job_id: Option<String>,
    commands: Vec<String>,
    prewarm_commands: Vec<String>,
    prewarm_only: bool,
    env: BTreeMap<String, String>,
    shared_cargo_target_dir: Option<PathBuf>,
    shared_cargo_build_dir: Option<PathBuf>,
    temp_dir: Option<PathBuf>,
    runner_parallelism: Option<i64>,
    timeout_seconds: u64,
    log_path: PathBuf,
    summary_path: PathBuf,
}

impl CommandBundleConfig {
    fn from_request(request: &JsonMap<String, JsonValue>) -> Result<Self, String> {
        let workspace_path = path_field(request, "workspace_path")?;
        if !workspace_path.is_dir() {
            return Err(format!(
                "workspace_path `{}` must be an existing directory.",
                path_string(&workspace_path)
            ));
        }
        let output_dir = optional_path(request, "output_dir")
            .unwrap_or_else(|| workspace_path.join(".ait/generated/ci"));
        let runner = request
            .get("runner")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| "Field `runner` must be a JSON object.".to_string())?;
        let kind = optional_text(runner, "kind").unwrap_or_else(|| "command_bundle".to_string());
        if kind != "command_bundle" {
            return Err(format!(
                "ci-command-bundle-run requires runner.kind `command_bundle`; got `{kind}`."
            ));
        }
        let commands = string_array(runner, "commands")?;
        let prewarm_only = optional_bool(request, "prewarm_only")?.unwrap_or(false);
        if commands.is_empty() && !prewarm_only {
            return Err("runner.commands must contain at least one command.".to_string());
        }
        let prewarm_commands = string_array(runner, "prewarm_commands").or_else(|_| {
            request
                .get("prewarm")
                .and_then(JsonValue::as_object)
                .map(|prewarm| string_array(prewarm, "commands"))
                .unwrap_or_else(|| Ok(Vec::new()))
        })?;
        let mut env = string_map(request, "env")?;
        env.extend(string_map(runner, "env")?);
        let shared_cargo_target_dir = optional_path(request, "shared_cargo_target_dir")
            .or_else(|| optional_path(request, "cargo_target_dir"));
        let shared_cargo_build_dir = optional_path(request, "shared_cargo_build_dir")
            .or_else(|| optional_path(request, "cargo_build_dir"));
        let temp_dir = optional_path(request, "temp_dir");
        let runner_parallelism = optional_positive_i64(request, "runner_parallelism")?
            .or(optional_positive_i64(request, "admitted_cpu_tokens")?)
            .or(optional_positive_i64(runner, "runner_parallelism")?)
            .or(optional_positive_i64(runner, "cpu_tokens")?)
            .or(optional_positive_i64(runner, "workers")?);
        let timeout_seconds = validated_ci_process_timeout_seconds(
            optional_positive_i64(runner, "timeout_seconds")?
                .or(optional_positive_i64(request, "timeout_seconds")?),
            "timeout_seconds",
        )?;
        let artifacts = request
            .get("artifacts")
            .and_then(JsonValue::as_object)
            .cloned()
            .unwrap_or_default();
        let log_path = artifact_path(&output_dir, &artifacts, "log_path", "run.log")?;
        let summary_path = artifact_path(&output_dir, &artifacts, "summary_json", "summary.json")?;
        Ok(Self {
            workspace_path,
            output_dir,
            suite_id: optional_text(request, "suite_id"),
            job_type: optional_text(request, "job_type"),
            job_id: optional_text(request, "job_id"),
            commands,
            prewarm_commands,
            prewarm_only,
            env,
            shared_cargo_target_dir,
            shared_cargo_build_dir,
            temp_dir,
            runner_parallelism,
            timeout_seconds,
            log_path,
            summary_path,
        })
    }
}

#[derive(Debug)]
struct CommandReport {
    index: usize,
    phase: &'static str,
    command: String,
    status: &'static str,
    exit_code: i32,
    timed_out: bool,
    timeout_seconds: u64,
    duration_seconds: f64,
    stdout_tail: String,
    stderr_tail: String,
    combined_tail: String,
    log_path: PathBuf,
    stdout_bytes: usize,
    stderr_bytes: usize,
}

impl CommandReport {
    fn to_json(&self) -> JsonValue {
        json!({
            "index": self.index,
            "phase": self.phase,
            "command": self.command,
            "status": self.status,
            "exit_code": self.exit_code,
            "timed_out": self.timed_out,
            "timeout_seconds": self.timeout_seconds,
            "duration_seconds": self.duration_seconds,
            "stdout_tail": self.stdout_tail,
            "stderr_tail": self.stderr_tail,
            "combined_tail": self.combined_tail,
            "log_path": path_string(&self.log_path),
            "stdout_bytes": self.stdout_bytes,
            "stderr_bytes": self.stderr_bytes,
        })
    }

    fn failure_json(&self, stage: &str) -> JsonValue {
        json!({
            "stage": stage,
            "index": self.index,
            "command": self.command,
            "exit_code": self.exit_code,
            "timed_out": self.timed_out,
            "timeout_seconds": self.timeout_seconds,
            "log_path": path_string(&self.log_path),
            "stdout_tail": self.stdout_tail,
            "stderr_tail": self.stderr_tail,
            "combined_tail": self.combined_tail,
        })
    }
}

fn command_env(config: &CommandBundleConfig) -> BTreeMap<String, String> {
    let mut env = clean_ci_process_env(&config.env);
    env.insert(
        "AIT_REPO_ROOT".to_string(),
        path_string(&config.workspace_path),
    );
    env.insert(
        "AIT_CI_WORKSPACE_PATH".to_string(),
        path_string(&config.workspace_path),
    );
    env.insert(
        "AIT_CI_COMMAND_BUNDLE_OUTPUT_DIR".to_string(),
        path_string(&config.output_dir),
    );
    if let Some(path) = &config.shared_cargo_target_dir {
        let text = path_string(path);
        env.insert("CARGO_TARGET_DIR".to_string(), text.clone());
        env.insert("AIT_SHARED_CARGO_TARGET_DIR".to_string(), text);
    }
    if let Some(path) = &config.shared_cargo_build_dir {
        let text = path_string(path);
        env.insert("CARGO_BUILD_BUILD_DIR".to_string(), text.clone());
        env.insert("AIT_SHARED_CARGO_BUILD_DIR".to_string(), text);
    }
    if let Some(path) = &config.temp_dir {
        let text = path_string(path);
        env.insert("TMPDIR".to_string(), text.clone());
        env.insert("TMP".to_string(), text.clone());
        env.insert("TEMP".to_string(), text);
    }
    if let Some(parallelism) = config.runner_parallelism {
        let text = parallelism.max(1).to_string();
        env.insert("AIT_RUNNER_PARALLELISM".to_string(), text.clone());
        env.insert("AIT_CI_RUNNER_PARALLELISM".to_string(), text.clone());
        env.insert("AIT_CI_ADMITTED_CPU_TOKENS".to_string(), text.clone());
        env.insert("CARGO_BUILD_JOBS".to_string(), text.clone());
        env.insert("RUST_TEST_THREADS".to_string(), text);
    }
    env
}

fn run_shell_command(
    phase: &'static str,
    index: usize,
    command: &str,
    workspace_path: &Path,
    output_dir: &Path,
    env: &BTreeMap<String, String>,
    timeout_seconds: u64,
) -> Result<CommandReport, String> {
    let started = Instant::now();
    let log_path = output_dir.join(format!("{phase}-{index:03}.log"));
    let mut child = Command::new("sh");
    child.arg("-c").arg(command).current_dir(workspace_path);
    apply_clean_ci_process_env(&mut child, env);
    let output = run_streamed_command(
        &mut child,
        &log_path,
        &format!("sh -c {command}"),
        workspace_path,
        CiProcessStdoutCapture::None,
        CiProcessExecutionOptions::from_timeout_seconds(timeout_seconds),
    )
    .map_err(|exc| format!("Failed to execute CI {phase} command {index}: {exc}"))?;
    let exit_code = output.status.code().unwrap_or(-1);
    let status = if output.status.success() {
        "pass"
    } else {
        "fail"
    };
    Ok(CommandReport {
        index,
        phase,
        command: command.to_string(),
        status,
        exit_code,
        timed_out: output.timed_out,
        timeout_seconds,
        duration_seconds: duration_seconds(started),
        stdout_tail: output.stdout_tail,
        stderr_tail: output.stderr_tail,
        combined_tail: output.combined_tail,
        log_path,
        stdout_bytes: output.stdout_bytes,
        stderr_bytes: output.stderr_bytes,
    })
}

fn write_artifacts(config: &CommandBundleConfig, summary: &JsonValue) -> Result<JsonValue, String> {
    if let Some(parent) = config.summary_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create summary artifact parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    if let Some(parent) = config.log_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create log artifact parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    fs::write(
        &config.summary_path,
        serde_json::to_string_pretty(summary).map_err(|exc| exc.to_string())? + "\n",
    )
    .map_err(|exc| {
        format!(
            "Failed to write summary artifact `{}`: {exc}",
            path_string(&config.summary_path)
        )
    })?;
    fs::write(&config.log_path, merged_log_text(summary)).map_err(|exc| {
        format!(
            "Failed to write merged log artifact `{}`: {exc}",
            path_string(&config.log_path)
        )
    })?;
    Ok(json!({
        "summary_json": artifact_payload(&config.summary_path),
        "log_path": artifact_payload(&config.log_path),
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
    for report in summary["prewarm"]["reports"]
        .as_array()
        .into_iter()
        .flatten()
    {
        lines.push(report_log_line("prewarm", report));
    }
    for report in summary["command_reports"].as_array().into_iter().flatten() {
        lines.push(report_log_line("command", report));
    }
    lines.join("\n") + "\n"
}

fn report_log_line(phase: &str, report: &JsonValue) -> String {
    format!(
        "{phase}[{}] status={} exit_code={} log_path={}",
        report
            .get("index")
            .and_then(JsonValue::as_u64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "?".to_string()),
        report
            .get("status")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown"),
        report
            .get("exit_code")
            .and_then(JsonValue::as_i64)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_string()),
        report
            .get("log_path")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
    )
}

fn report_passed(report: &JsonValue) -> bool {
    report.get("status").and_then(JsonValue::as_str) == Some("pass")
}

fn artifact_payload(path: &Path) -> JsonValue {
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

fn artifact_path(
    output_dir: &Path,
    artifacts: &JsonMap<String, JsonValue>,
    key: &str,
    default_name: &str,
) -> Result<PathBuf, String> {
    let rel = optional_text(artifacts, key).unwrap_or_else(|| default_name.to_string());
    if path_has_parent_escape(&rel) || Path::new(&rel).is_absolute() {
        return Err(format!(
            "Artifact path `{key}` must be relative and stay inside output_dir."
        ));
    }
    Ok(output_dir.join(rel))
}

fn path_has_parent_escape(value: &str) -> bool {
    Path::new(value)
        .components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn path_field(value: &JsonMap<String, JsonValue>, key: &str) -> Result<PathBuf, String> {
    optional_path(value, key).ok_or_else(|| format!("Field `{key}` must be a non-empty path."))
}

fn optional_path(value: &JsonMap<String, JsonValue>, key: &str) -> Option<PathBuf> {
    optional_text(value, key).map(PathBuf::from)
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

fn string_array(value: &JsonMap<String, JsonValue>, key: &str) -> Result<Vec<String>, String> {
    let Some(raw) = value.get(key) else {
        return Ok(Vec::new());
    };
    let values = raw
        .as_array()
        .ok_or_else(|| format!("Field `{key}` must be an array of non-empty strings."))?;
    let mut parsed = Vec::new();
    for item in values {
        let text = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Field `{key}` must contain non-empty strings."))?;
        parsed.push(text.to_string());
    }
    Ok(parsed)
}

fn string_map(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<BTreeMap<String, String>, String> {
    let Some(raw) = value.get(key) else {
        return Ok(BTreeMap::new());
    };
    let object = raw
        .as_object()
        .ok_or_else(|| format!("Field `{key}` must be an object of string values."))?;
    let mut parsed = BTreeMap::new();
    for (entry_key, entry_value) in object {
        let text = entry_value
            .as_str()
            .ok_or_else(|| format!("Field `{key}.{entry_key}` must be a string."))?;
        parsed.insert(entry_key.clone(), text.to_string());
    }
    Ok(parsed)
}

fn duration_seconds(started: Instant) -> f64 {
    let millis = started.elapsed().as_millis() as f64;
    (millis / 1000.0 * 1000.0).round() / 1000.0
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
