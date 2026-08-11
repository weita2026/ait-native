use crate::foundation::ci_process_env::{
    apply_clean_ci_process_env, clean_ci_process_env, CI_PROCESS_ENVIRONMENT_POLICY,
};
use crate::foundation::ci_process_stream::{
    run_streamed_command, validated_ci_process_timeout_seconds, CiProcessExecutionOptions,
    CiProcessStdoutCapture,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use super::config::PrewarmConfig;
use super::helpers::{
    duration_seconds, optional_string_array, optional_string_map, optional_text, path_string,
    positive_u64, required_text, safe_path_segment,
};
use super::paths::{relative_path_array, validate_relative_path, verify_step_required_paths};
use super::PREWARM_LOG_DIR;

#[derive(Clone)]
pub(super) struct PrewarmStep {
    pub(super) step_id: String,
    program: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
    timeout_seconds: Option<u64>,
    pub(super) required_paths: Vec<PathBuf>,
}

impl PrewarmStep {
    fn from_value(value: &JsonValue, index: usize) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| format!("prewarm_steps[{index}] must be a JSON object."))?;
        let step_id =
            optional_text(object, "step_id").unwrap_or_else(|| format!("prewarm-step-{index}"));
        let program = required_text(object, "program")
            .map_err(|exc| format!("prewarm_steps[{index}]: {exc}"))?;
        let args = optional_string_array(object, "args")?.unwrap_or_default();
        let env = optional_string_map(object, "env")?.unwrap_or_default();
        let cwd = optional_text(object, "cwd")
            .map(|value| validate_relative_path(&value, "cwd"))
            .transpose()?;
        let timeout_seconds = positive_u64(object, "timeout_seconds")?
            .map(|value| {
                i64::try_from(value)
                    .map_err(|_| "Field `timeout_seconds` is too large.".to_string())
            })
            .transpose()?
            .map(|value| validated_ci_process_timeout_seconds(Some(value), "timeout_seconds"))
            .transpose()?;
        let required_paths = relative_path_array(object, "required_paths")?.unwrap_or_default();
        Ok(Self {
            step_id,
            program,
            args,
            env,
            cwd,
            timeout_seconds,
            required_paths,
        })
    }

    fn cargo_package(
        package: String,
        cargo_args: &[String],
        target_dir: Option<String>,
        build_dir: Option<String>,
    ) -> Self {
        let mut args = vec!["build".to_string(), "-p".to_string(), package.clone()];
        args.extend(cargo_args.iter().cloned());
        let mut env = BTreeMap::new();
        if let Some(target_dir) = target_dir {
            env.insert("CARGO_TARGET_DIR".to_string(), target_dir);
        }
        if let Some(build_dir) = build_dir {
            env.insert("CARGO_BUILD_BUILD_DIR".to_string(), build_dir.clone());
            env.insert("AIT_SHARED_CARGO_BUILD_DIR".to_string(), build_dir);
        }
        Self {
            step_id: format!("cargo-build-{package}"),
            program: "cargo".to_string(),
            args,
            env,
            cwd: None,
            timeout_seconds: None,
            required_paths: Vec::new(),
        }
    }

    pub(super) fn fingerprint_json(&self) -> JsonValue {
        json!({
            "step_id": self.step_id,
            "program": self.program,
            "args": self.args,
            "env": self.env,
            "cwd": self.cwd.as_ref().map(|path| path_string(path)),
            "timeout_seconds": self.timeout_seconds,
            "required_paths": self.required_paths.iter().map(|path| path_string(path)).collect::<Vec<_>>(),
        })
    }
}

pub(super) fn run_steps_parallel(
    config: &PrewarmConfig,
    seed_path: &Path,
) -> Result<Vec<JsonValue>, String> {
    if config.steps.is_empty() {
        return Ok(Vec::new());
    }
    let queue = Arc::new(Mutex::new(
        config
            .steps
            .iter()
            .cloned()
            .enumerate()
            .collect::<VecDeque<_>>(),
    ));
    let results = Arc::new(Mutex::new(Vec::new()));
    let worker_count = config.parallelism.min(config.steps.len()).max(1);
    let mut handles = Vec::new();
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let results = Arc::clone(&results);
        let seed_path = seed_path.to_path_buf();
        let parallelism = config.parallelism;
        let dry_run = config.dry_run;
        let timeout_seconds = config.timeout_seconds;
        handles.push(thread::spawn(move || loop {
            let next = {
                let mut queue = queue.lock().expect("prewarm queue lock should not poison");
                queue.pop_front()
            };
            let Some((index, step)) = next else {
                break;
            };
            let result = run_one_step(
                index,
                &step,
                &seed_path,
                parallelism,
                dry_run,
                timeout_seconds,
            );
            let mut results = results
                .lock()
                .expect("prewarm result lock should not poison");
            results.push((index, result));
        }));
    }
    for handle in handles {
        handle
            .join()
            .map_err(|_| "main-seed prewarm worker thread panicked".to_string())?;
    }
    let mut results = Arc::try_unwrap(results)
        .map_err(|_| "main-seed prewarm results still shared.".to_string())?
        .into_inner()
        .map_err(|_| "main-seed prewarm result lock poisoned.".to_string())?;
    results.sort_by_key(|(index, _)| *index);

    let mut payloads = Vec::new();
    let mut failures = Vec::new();
    for (_, result) in results {
        match result {
            Ok(payload) => {
                if payload.get("status").and_then(JsonValue::as_str) != Some("pass") {
                    let step_id = payload
                        .get("step_id")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("unknown");
                    if payload.get("timed_out").and_then(JsonValue::as_bool) == Some(true) {
                        failures.push(format!(
                            "{step_id} timed out after {} seconds",
                            payload
                                .get("timeout_seconds")
                                .and_then(JsonValue::as_u64)
                                .unwrap_or(config.timeout_seconds)
                        ));
                    } else {
                        failures.push(format!(
                            "{step_id} failed with exit_code {:?}",
                            payload.get("exit_code")
                        ));
                    }
                }
                payloads.push(payload);
            }
            Err(message) => failures.push(message),
        }
    }
    if !failures.is_empty() {
        return Err(format!("Main-seed prewarm failed: {}", failures.join("; ")));
    }
    Ok(payloads)
}

fn run_one_step(
    index: usize,
    step: &PrewarmStep,
    seed_path: &Path,
    parallelism: usize,
    dry_run: bool,
    default_timeout_seconds: u64,
) -> Result<JsonValue, String> {
    let started = Instant::now();
    let cwd = match &step.cwd {
        Some(relative) => seed_path.join(relative),
        None => seed_path.to_path_buf(),
    };
    let log_dir = seed_path.join(PREWARM_LOG_DIR);
    if !dry_run {
        fs::create_dir_all(&log_dir).map_err(|exc| {
            format!(
                "Failed to create prewarm log dir `{}`: {exc}",
                path_string(&log_dir)
            )
        })?;
    }
    let log_path = log_dir.join(format!(
        "{}-{}.log",
        index,
        safe_path_segment(&step.step_id)
    ));
    let timeout_seconds = step.timeout_seconds.unwrap_or(default_timeout_seconds);
    if dry_run {
        return Ok(json!({
            "step_id": step.step_id,
            "status": "pass",
            "dry_run": true,
            "program": step.program,
            "args": step.args,
            "cwd": path_string(&cwd),
            "log_path": path_string(&log_path),
            "timeout_seconds": timeout_seconds,
            "timed_out": false,
            "environment_policy": CI_PROCESS_ENVIRONMENT_POLICY,
            "duration_seconds": duration_seconds(started)
        }));
    }

    let mut command = Command::new(&step.program);
    command.current_dir(&cwd);
    command.args(&step.args);
    let mut command_environment = clean_ci_process_env(&step.env);
    command_environment.insert(
        "AIT_PREWARM_PARALLELISM".to_string(),
        parallelism.to_string(),
    );
    command_environment.insert("CARGO_BUILD_JOBS".to_string(), parallelism.to_string());
    apply_clean_ci_process_env(&mut command, &command_environment);
    let command_text = rendered_command(step);
    let output = run_streamed_command(
        &mut command,
        &log_path,
        &command_text,
        &cwd,
        CiProcessStdoutCapture::None,
        CiProcessExecutionOptions::from_timeout_seconds(timeout_seconds),
    )
    .map_err(|exc| {
        format!(
            "Failed to execute prewarm step `{}` with program `{}`: {exc}",
            step.step_id, step.program
        )
    })?;
    let required_paths = verify_step_required_paths(step, seed_path)?;
    Ok(json!({
        "step_id": step.step_id,
        "status": if output.status.success() { "pass" } else { "fail" },
        "exit_code": output.status.code(),
        "timed_out": output.timed_out,
        "timeout_seconds": timeout_seconds,
        "environment_policy": CI_PROCESS_ENVIRONMENT_POLICY,
        "duration_seconds": duration_seconds(started),
        "program": step.program,
        "args": step.args,
        "cwd": path_string(&cwd),
        "log_path": path_string(&log_path),
        "stdout": output.stdout_tail,
        "stderr": output.stderr_tail,
        "stdout_bytes": output.stdout_bytes,
        "stderr_bytes": output.stderr_bytes,
        "required_paths": required_paths
    }))
}

pub(super) fn prewarm_steps_from_request(
    request: &JsonMap<String, JsonValue>,
    prewarm: Option<&JsonMap<String, JsonValue>>,
    parallelism: usize,
) -> Result<Vec<PrewarmStep>, String> {
    if let Some(values) = request
        .get("prewarm_steps")
        .or_else(|| prewarm.and_then(|value| value.get("steps")))
        .and_then(JsonValue::as_array)
    {
        return values
            .iter()
            .enumerate()
            .map(|(index, value)| PrewarmStep::from_value(value, index))
            .collect();
    }
    let packages = optional_string_array(request, "cargo_packages")?
        .or(match prewarm {
            Some(value) => optional_string_array(value, "cargo_packages")?,
            None => None,
        })
        .unwrap_or_default();
    let cargo_args = optional_string_array(request, "cargo_args")?
        .or(match prewarm {
            Some(value) => optional_string_array(value, "cargo_args")?,
            None => None,
        })
        .unwrap_or_else(|| vec!["-j".to_string(), parallelism.to_string()]);
    let target_dir = optional_text(request, "cargo_target_dir")
        .or_else(|| prewarm.and_then(|value| optional_text(value, "cargo_target_dir")));
    let build_dir = optional_text(request, "cargo_build_dir")
        .or_else(|| optional_text(request, "shared_cargo_build_dir"))
        .or_else(|| prewarm.and_then(|value| optional_text(value, "cargo_build_dir")))
        .or_else(|| prewarm.and_then(|value| optional_text(value, "shared_cargo_build_dir")));
    Ok(packages
        .into_iter()
        .map(|package| {
            PrewarmStep::cargo_package(package, &cargo_args, target_dir.clone(), build_dir.clone())
        })
        .collect())
}

fn rendered_command(step: &PrewarmStep) -> String {
    let mut parts = vec![step.program.clone()];
    parts.extend(step.args.clone());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn generated_cargo_prewarm_step_keeps_target_and_build_dirs_separate() {
        let request = json!({
            "cargo_packages": ["ait-cli"],
            "cargo_target_dir": "/ram/cargo-target/ait-core",
            "cargo_build_dir": "/ram/cargo-build/ait-core",
        });
        let steps = prewarm_steps_from_request(
            request.as_object().expect("request should be an object"),
            None,
            4,
        )
        .expect("Cargo prewarm steps should parse");

        assert_eq!(steps.len(), 1);
        let fingerprint = steps[0].fingerprint_json();
        assert_eq!(
            fingerprint["env"]["CARGO_TARGET_DIR"],
            json!("/ram/cargo-target/ait-core")
        );
        assert_eq!(
            fingerprint["env"]["CARGO_BUILD_BUILD_DIR"],
            json!("/ram/cargo-build/ait-core")
        );
        assert_eq!(
            fingerprint["env"]["AIT_SHARED_CARGO_BUILD_DIR"],
            json!("/ram/cargo-build/ait-core")
        );
    }

    #[cfg(unix)]
    #[test]
    fn prewarm_step_streams_large_output_and_honors_step_timeout() {
        let seed_path = env::temp_dir().join(format!(
            "ait-main-seed-prewarm-timeout-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&seed_path).expect("seed path should be created");
        let step = PrewarmStep::from_value(
            &json!({
                "step_id": "large-timeout",
                "program": "/bin/sh",
                "args": [
                    "-c",
                    "head -c 1048576 /dev/zero | tr '\\000' x; printf prewarm-tail; sleep 30"
                ],
                "timeout_seconds": 1
            }),
            0,
        )
        .expect("prewarm step should parse");

        let result = run_one_step(0, &step, &seed_path, 1, false, 9)
            .expect("prewarm timeout should return bounded evidence");

        assert_eq!(result["status"], json!("fail"));
        assert_eq!(result["timed_out"], json!(true));
        assert_eq!(result["timeout_seconds"], json!(1));
        assert_eq!(result["stdout_bytes"], json!(1_048_588));
        let stdout_tail = result["stdout"]
            .as_str()
            .expect("prewarm stdout tail should be text");
        assert!(stdout_tail.len() <= 8_000);
        assert!(stdout_tail.ends_with("prewarm-tail"));
        let log_path = PathBuf::from(
            result["log_path"]
                .as_str()
                .expect("prewarm log path should be text"),
        );
        assert!(
            fs::metadata(&log_path)
                .expect("prewarm log should exist")
                .len()
                > 1_048_576
        );
        assert!(fs::read_to_string(&log_path)
            .expect("prewarm log should read")
            .contains("timed_out=true"));
        assert!(!log_path.with_extension("stdout.tmp").exists());
        assert!(!log_path.with_extension("stderr.tmp").exists());

        let _ = fs::remove_dir_all(seed_path);
    }

    #[test]
    fn prewarm_step_rejects_invalid_timeout() {
        for timeout in [json!(0), json!(-1), json!(86_401), json!("1")] {
            let error = PrewarmStep::from_value(
                &json!({
                    "step_id": "invalid-timeout",
                    "program": "/bin/true",
                    "timeout_seconds": timeout
                }),
                0,
            )
            .err()
            .expect("invalid prewarm timeout should fail closed");
            assert!(error.contains("timeout_seconds"), "{error}");
        }
    }
}
