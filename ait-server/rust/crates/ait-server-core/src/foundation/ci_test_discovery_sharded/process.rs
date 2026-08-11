use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use super::config::DiscoveryShardedConfig;
use super::paths::{command_line, duration_seconds, path_string};
use crate::foundation::ci_process_env::{apply_clean_ci_process_env, clean_ci_process_env};
use crate::foundation::ci_process_stream::{
    run_streamed_command, CiProcessExecutionOptions, CiProcessStdoutCapture,
};
#[cfg(test)]
use crate::foundation::ci_process_stream::{
    PROCESS_COMBINED_TAIL_CHARS as STREAM_COMBINED_TAIL_CHARS,
    PROCESS_STDERR_TAIL_BYTES as STREAM_STDERR_TAIL_BYTES,
    PROCESS_STDOUT_TAIL_BYTES as STREAM_STDOUT_TAIL_BYTES,
};

const PROCESS_STDOUT_CAPTURE_LIMIT_BYTES: u64 = 8 * 1024 * 1024;
#[cfg(test)]
const PROCESS_STDOUT_TAIL_BYTES: u64 = STREAM_STDOUT_TAIL_BYTES;
#[cfg(test)]
const PROCESS_STDERR_TAIL_BYTES: u64 = STREAM_STDERR_TAIL_BYTES;
#[cfg(test)]
const PROCESS_COMBINED_TAIL_BYTES: usize = STREAM_COMBINED_TAIL_CHARS;

#[derive(Debug)]
pub(super) struct ProcessReport {
    pub(super) index: usize,
    pub(super) phase: &'static str,
    pub(super) command: String,
    pub(super) status: &'static str,
    pub(super) exit_code: i32,
    pub(super) timed_out: bool,
    pub(super) timeout_seconds: u64,
    pub(super) duration_seconds: f64,
    pub(super) stdout_tail: String,
    pub(super) stderr_tail: String,
    pub(super) combined_tail: String,
    pub(super) log_path: PathBuf,
    pub(super) stdout_bytes: usize,
    pub(super) stderr_bytes: usize,
}

impl ProcessReport {
    pub(super) fn to_json(&self) -> JsonValue {
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

    pub(super) fn failure_json(&self, stage: &str) -> JsonValue {
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

#[derive(Debug)]
pub(super) struct ProcessRun {
    pub(super) report: ProcessReport,
    pub(super) stdout: String,
}

#[derive(Clone, Copy)]
pub(super) struct ProcessContext<'a> {
    pub(super) output_dir: &'a Path,
    pub(super) env: &'a BTreeMap<String, String>,
    pub(super) timeout_seconds: u64,
}

#[derive(Clone, Copy)]
pub(super) enum EnvMode<'a> {
    Build,
    TestList,
    TestShard {
        shard_id: &'a str,
        shard_index: usize,
    },
}

pub(super) fn command_env(
    config: &DiscoveryShardedConfig,
    mode: EnvMode<'_>,
) -> BTreeMap<String, String> {
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
        "AIT_CI_TEST_DISCOVERY_ADAPTER".to_string(),
        config.adapter.clone(),
    );
    env.insert(
        "AIT_CI_TEST_SHARDING".to_string(),
        match mode {
            EnvMode::Build => "test_case",
            EnvMode::TestList => "test_case_discovery",
            EnvMode::TestShard { .. } => "test_case",
        }
        .to_string(),
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
        env.insert("CARGO_BUILD_JOBS".to_string(), text);
    }
    env.insert("RUST_TEST_THREADS".to_string(), "1".to_string());
    match mode {
        EnvMode::Build => {}
        EnvMode::TestList => {
            env.insert("AIT_CI_TEST_CASE_DISCOVERY".to_string(), "1".to_string());
        }
        EnvMode::TestShard {
            shard_id,
            shard_index,
        } => {
            env.insert("AIT_CI_SHARD_ID".to_string(), shard_id.to_string());
            env.insert("AIT_CI_SHARD_INDEX".to_string(), shard_index.to_string());
        }
    }
    env
}

pub(super) fn run_process(
    phase: &'static str,
    index: usize,
    program: &str,
    args: &[String],
    cwd: &Path,
    context: ProcessContext<'_>,
) -> Result<ProcessReport, String> {
    Ok(run_process_inner(phase, index, program, args, cwd, context, false)?.report)
}

pub(super) fn run_process_with_output(
    phase: &'static str,
    index: usize,
    program: &str,
    args: &[String],
    cwd: &Path,
    context: ProcessContext<'_>,
) -> Result<ProcessRun, String> {
    run_process_inner(phase, index, program, args, cwd, context, true)
}

fn run_process_inner(
    phase: &'static str,
    index: usize,
    program: &str,
    args: &[String],
    cwd: &Path,
    context: ProcessContext<'_>,
    capture_stdout: bool,
) -> Result<ProcessRun, String> {
    let started = Instant::now();
    let resolved_program = resolve_ci_process_program(program, context.env).map_err(|message| {
        format!(
            "Failed to execute CI {phase} process `{}`: {message}",
            command_line(program, args)
        )
    })?;
    fs::create_dir_all(context.output_dir).map_err(|exc| {
        format!(
            "Failed to create CI {phase} output directory `{}`: {exc}",
            path_string(context.output_dir)
        )
    })?;
    let log_path = context.output_dir.join(format!("{phase}-{index:03}.log"));
    let command_text = command_line(program, args);
    let mut command = Command::new(&resolved_program);
    command.args(args).current_dir(cwd);
    apply_clean_ci_process_env(&mut command, context.env);
    let output = run_streamed_command(
        &mut command,
        &log_path,
        &command_text,
        cwd,
        if capture_stdout {
            CiProcessStdoutCapture::Required(PROCESS_STDOUT_CAPTURE_LIMIT_BYTES)
        } else {
            CiProcessStdoutCapture::None
        },
        CiProcessExecutionOptions::from_timeout_seconds(context.timeout_seconds),
    )
    .map_err(|exc| {
        format!(
            "Failed to execute CI {phase} process `{}`: {exc}",
            command_text
        )
    })?;
    let exit_code = output.status.code().unwrap_or(-1);
    let report_status = if output.status.success() {
        "pass"
    } else {
        "fail"
    };
    let report = ProcessReport {
        index,
        phase,
        command: command_text,
        status: report_status,
        exit_code,
        timed_out: output.timed_out,
        timeout_seconds: context.timeout_seconds,
        duration_seconds: duration_seconds(started),
        stdout_tail: output.stdout_tail,
        stderr_tail: output.stderr_tail,
        combined_tail: output.combined_tail,
        log_path,
        stdout_bytes: output.stdout_bytes,
        stderr_bytes: output.stderr_bytes,
    };
    Ok(ProcessRun {
        report,
        stdout: output.captured_stdout.unwrap_or_default(),
    })
}

pub(super) fn resolve_ci_process_program(
    program: &str,
    env_map: &BTreeMap<String, String>,
) -> Result<String, String> {
    if program_has_path_separator(program) {
        let path = PathBuf::from(program);
        if is_executable_file(&path) {
            return Ok(path_string(&path));
        }
        return Err(format!(
            "program `{program}` is not an executable file for the server runner."
        ));
    }

    let path_value = env_map.get("PATH").cloned().unwrap_or_default();
    for dir in env::split_paths(&path_value) {
        let candidate = dir.join(program);
        if is_executable_file(&candidate) {
            return Ok(path_string(&candidate));
        }
    }

    let rendered_path = if path_value.trim().is_empty() {
        "<empty>"
    } else {
        path_value.as_str()
    };
    Err(format!(
        "executable `{program}` was not found in PATH for the server runner. PATH={rendered_path}"
    ))
}

fn program_has_path_separator(program: &str) -> bool {
    program.contains('/') || program.contains('\\')
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn large_child_output_streams_to_log_with_bounded_report_tails() {
        let output_dir = env::temp_dir().join(format!(
            "ait-ci-process-stream-{}-{}",
            std::process::id(),
            TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let args = vec![
            "-c".to_string(),
            "head -c 4194304 /dev/zero | tr '\\000' x; printf stderr-marker >&2".to_string(),
        ];
        let report = run_process(
            "large_output",
            1,
            "/bin/sh",
            &args,
            Path::new("/"),
            ProcessContext {
                output_dir: &output_dir,
                env: &BTreeMap::new(),
                timeout_seconds: 5,
            },
        )
        .expect("large child output should stream successfully");

        assert_eq!(report.status, "pass");
        assert_eq!(report.stdout_bytes, 4_194_304);
        assert_eq!(report.stderr_bytes, "stderr-marker".len());
        assert!(report.stdout_tail.len() <= PROCESS_STDOUT_TAIL_BYTES as usize);
        assert!(report.stderr_tail.len() <= PROCESS_STDERR_TAIL_BYTES as usize);
        assert!(report.combined_tail.len() <= PROCESS_COMBINED_TAIL_BYTES);
        assert!(report.combined_tail.ends_with("stderr-marker"));
        assert!(fs::metadata(&report.log_path).unwrap().len() > 4_194_304);
        assert!(!report.log_path.with_extension("stdout.tmp").exists());
        assert!(!report.log_path.with_extension("stderr.tmp").exists());

        let _ = fs::remove_dir_all(output_dir);
    }
}
