use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ait_core::environment_contract::names;
use ait_core::json_support::{json, JsonValue};
use serde::{Deserialize, Serialize};

use crate::json_support::{decode_from_value, encode_to_value, write_pretty_value};
use crate::transport::TransportKind;

const DEFAULT_LOG_TAIL_LINES: usize = 100;
const DEFAULT_STOP_TIMEOUT_SECONDS: f64 = 10.0;
const DEFAULT_KILL_GRACE_SECONDS: f64 = 2.0;
const MIGRATION_STAGE: &str = "rust_agent_supervisor_process_contract";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerProcessPaths {
    pub pid_file: String,
    pub log_file: String,
    pub sync_state_path: String,
    pub env_path: String,
    pub termination_context_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerProcessStatusInput {
    pub paths: AgentWorkerProcessPaths,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWorkerPidFileInspection {
    pub pid_file_exists: bool,
    pub pid_file_readable: bool,
    pub pid_file_valid: bool,
    pub running: bool,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWorkerRuntimeHealth {
    pub pid_file_exists: bool,
    pub pid_file_readable: bool,
    pub pid_file_valid: bool,
    pub pid_file_state: String,
    pub log_exists: bool,
    pub log_size_bytes: u64,
    pub sync_state_exists: bool,
    pub env_exists: bool,
    pub termination_context_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWorkerProcessStatus {
    pub running: bool,
    pub pid: Option<i64>,
    pub health: AgentWorkerRuntimeHealth,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerLogTailInput {
    pub log_file: String,
    #[serde(default)]
    pub lines: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWorkerLogTail {
    pub lines: Vec<String>,
    pub log_exists: bool,
    pub lines_requested: usize,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerStopInput {
    pub paths: AgentWorkerProcessPaths,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub worker_name: Option<String>,
    #[serde(default)]
    pub stop_timeout_seconds: Option<f64>,
    #[serde(default)]
    pub kill_grace_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerStartInput {
    pub repo_root: String,
    pub paths: AgentWorkerProcessPaths,
    pub worker: AgentWorkerStartSpec,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub parent_env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerStartSpec {
    #[serde(rename = "kind", alias = "transport")]
    pub transport: TransportKind,
    pub name: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub app_token: Option<String>,
    #[serde(default)]
    pub bot_token: Option<String>,
    #[serde(default)]
    pub application_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWorkerStopResult {
    pub stopped: bool,
    pub stop_state: String,
    pub running: bool,
    pub pid: Option<i64>,
    pub target_pid: Option<i64>,
    pub health: AgentWorkerRuntimeHealth,
    pub termination_context_written: bool,
    pub termination_context_removed: bool,
    pub pid_file_removed: bool,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWorkerStartResult {
    pub started: bool,
    pub start_state: String,
    pub running: bool,
    pub pid: Option<i64>,
    pub command: Vec<String>,
    pub diagnostic: Option<String>,
    pub health: AgentWorkerRuntimeHealth,
    pub env_file_seeded: bool,
    pub termination_context_removed: bool,
    pub pid_file_written: bool,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: String,
}

pub trait AgentWorkerProcessStatusPort {
    fn inspect_worker_process_status(
        &self,
        input: AgentWorkerProcessStatusInput,
    ) -> AgentWorkerProcessStatus;
}

pub trait AgentWorkerProcessLogTailPort {
    fn read_worker_log_tail(&self, input: AgentWorkerLogTailInput) -> AgentWorkerLogTail;
}

pub trait AgentWorkerProcessStopPort {
    fn stop_worker_process(
        &self,
        input: AgentWorkerStopInput,
    ) -> Result<AgentWorkerStopResult, String>;
}

pub trait AgentWorkerProcessStartPort {
    fn start_worker_process(
        &self,
        input: AgentWorkerStartInput,
    ) -> Result<AgentWorkerStartResult, String>;
}

pub trait AgentWorkerProcessPort:
    AgentWorkerProcessStatusPort
    + AgentWorkerProcessLogTailPort
    + AgentWorkerProcessStopPort
    + AgentWorkerProcessStartPort
{
}

impl<P> AgentWorkerProcessPort for P where
    P: AgentWorkerProcessStatusPort
        + AgentWorkerProcessLogTailPort
        + AgentWorkerProcessStopPort
        + AgentWorkerProcessStartPort
        + ?Sized
{
}

pub fn inspect_worker_process_status_with_port<P>(
    port: &P,
    input: AgentWorkerProcessStatusInput,
) -> AgentWorkerProcessStatus
where
    P: AgentWorkerProcessStatusPort + ?Sized,
{
    port.inspect_worker_process_status(input)
}

pub fn read_worker_log_tail_with_port<P>(
    port: &P,
    input: AgentWorkerLogTailInput,
) -> AgentWorkerLogTail
where
    P: AgentWorkerProcessLogTailPort + ?Sized,
{
    port.read_worker_log_tail(input)
}

pub fn stop_worker_process_with_port<P>(
    port: &P,
    input: AgentWorkerStopInput,
) -> Result<AgentWorkerStopResult, String>
where
    P: AgentWorkerProcessStopPort + ?Sized,
{
    port.stop_worker_process(input)
}

pub fn start_worker_process_with_port<P>(
    port: &P,
    input: AgentWorkerStartInput,
) -> Result<AgentWorkerStartResult, String>
where
    P: AgentWorkerProcessStartPort + ?Sized,
{
    port.start_worker_process(input)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeAgentWorkerProcessPort;

impl AgentWorkerProcessStatusPort for NativeAgentWorkerProcessPort {
    fn inspect_worker_process_status(
        &self,
        input: AgentWorkerProcessStatusInput,
    ) -> AgentWorkerProcessStatus {
        inspect_worker_process_status_native(input)
    }
}

impl AgentWorkerProcessLogTailPort for NativeAgentWorkerProcessPort {
    fn read_worker_log_tail(&self, input: AgentWorkerLogTailInput) -> AgentWorkerLogTail {
        read_worker_log_tail_native(input)
    }
}

impl AgentWorkerProcessStopPort for NativeAgentWorkerProcessPort {
    fn stop_worker_process(
        &self,
        input: AgentWorkerStopInput,
    ) -> Result<AgentWorkerStopResult, String> {
        stop_worker_process_native(input)
    }
}

impl AgentWorkerProcessStartPort for NativeAgentWorkerProcessPort {
    fn start_worker_process(
        &self,
        input: AgentWorkerStartInput,
    ) -> Result<AgentWorkerStartResult, String> {
        start_worker_process_native(input)
    }
}

pub fn inspect_worker_process_status(
    input: AgentWorkerProcessStatusInput,
) -> AgentWorkerProcessStatus {
    inspect_worker_process_status_with_port(&NativeAgentWorkerProcessPort, input)
}

fn inspect_worker_process_status_native(
    input: AgentWorkerProcessStatusInput,
) -> AgentWorkerProcessStatus {
    inspect_worker_process_status_for_paths(&input.paths)
}

fn inspect_worker_process_status_for_paths(
    paths: &AgentWorkerProcessPaths,
) -> AgentWorkerProcessStatus {
    let pid_file = PathBuf::from(&paths.pid_file);
    let pid_info = inspect_pid_file(&pid_file);
    let pid = pid_info.pid;
    let running = pid_info.running;
    let health = runtime_health(paths, pid_info);
    AgentWorkerProcessStatus {
        running,
        pid,
        health,
        python_worker_execution_allowed: false,
        migration_stage: MIGRATION_STAGE.to_string(),
    }
}

pub fn inspect_worker_process_status_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentWorkerProcessStatusInput = decode_from_value(
        request,
        "invalid ait-agent supervisor process status request",
    )?;
    encode_to_value(
        &inspect_worker_process_status(input),
        "failed to serialize ait-agent supervisor process status",
    )
}

pub fn read_worker_log_tail(input: AgentWorkerLogTailInput) -> AgentWorkerLogTail {
    read_worker_log_tail_with_port(&NativeAgentWorkerProcessPort, input)
}

fn read_worker_log_tail_native(input: AgentWorkerLogTailInput) -> AgentWorkerLogTail {
    let lines_requested = input.lines.unwrap_or(DEFAULT_LOG_TAIL_LINES);
    let log_file = PathBuf::from(input.log_file);
    AgentWorkerLogTail {
        lines: read_tail_lines(&log_file, lines_requested),
        log_exists: path_exists(&log_file),
        lines_requested,
        python_worker_execution_allowed: false,
        migration_stage: MIGRATION_STAGE.to_string(),
    }
}

pub fn read_worker_log_tail_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentWorkerLogTailInput =
        decode_from_value(request, "invalid ait-agent supervisor log-tail request")?;
    encode_to_value(
        &read_worker_log_tail(input),
        "failed to serialize ait-agent supervisor log-tail",
    )
}

pub fn stop_worker_process(input: AgentWorkerStopInput) -> Result<AgentWorkerStopResult, String> {
    stop_worker_process_with_port(&NativeAgentWorkerProcessPort, input)
}

fn stop_worker_process_native(
    input: AgentWorkerStopInput,
) -> Result<AgentWorkerStopResult, String> {
    let paths = input.paths;
    let pid_file = PathBuf::from(&paths.pid_file);
    let termination_context_path = PathBuf::from(&paths.termination_context_path);
    let pid_info = inspect_pid_file(&pid_file);
    let target_pid = pid_info.pid;

    let mut termination_context_written = false;
    let mut termination_context_removed = false;
    let mut pid_file_removed = false;

    if !pid_info.pid_file_valid || !pid_info.running {
        pid_file_removed = remove_file_if_exists(&pid_file)?;
        termination_context_removed = remove_file_if_exists(&termination_context_path)?;
        return Ok(stop_result(
            &paths,
            "not_running",
            false,
            target_pid,
            termination_context_written,
            termination_context_removed,
            pid_file_removed,
        ));
    }

    let pid = pid_info
        .pid
        .ok_or_else(|| "valid running pid inspection did not include a pid".to_string())?;
    write_termination_context(
        &termination_context_path,
        pid,
        clean_optional_text(input.reason.as_deref()).unwrap_or_else(|| "worker_stop".to_string()),
        clean_optional_text(input.worker_name.as_deref()).unwrap_or_else(|| "worker".to_string()),
    )?;
    termination_context_written = true;

    let stop_state = stop_pid(
        pid,
        duration_from_seconds(input.stop_timeout_seconds, DEFAULT_STOP_TIMEOUT_SECONDS),
        duration_from_seconds(input.kill_grace_seconds, DEFAULT_KILL_GRACE_SECONDS),
    )?;
    let stopped = stop_success_state(&stop_state);
    if stopped {
        pid_file_removed = remove_file_if_exists(&pid_file)?;
    }

    Ok(stop_result(
        &paths,
        &stop_state,
        stopped,
        target_pid,
        termination_context_written,
        termination_context_removed,
        pid_file_removed,
    ))
}

pub fn stop_worker_process_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentWorkerStopInput =
        decode_from_value(request, "invalid ait-agent supervisor stop request")?;
    encode_to_value(
        &stop_worker_process(input)?,
        "failed to serialize ait-agent supervisor stop result",
    )
}

pub fn start_worker_process(
    input: AgentWorkerStartInput,
) -> Result<AgentWorkerStartResult, String> {
    start_worker_process_with_port(&NativeAgentWorkerProcessPort, input)
}

fn start_worker_process_native(
    input: AgentWorkerStartInput,
) -> Result<AgentWorkerStartResult, String> {
    let repo_root = normalize_start_repo_root(&input.repo_root)?;
    let paths = input.paths;
    let command = normalize_argv(input.argv);

    if command.is_empty() {
        return Ok(start_result(
            &paths,
            "rust_launch_command_missing",
            false,
            command,
            Some("Rust launch contract returned an empty command; refusing Python fallback."),
            false,
            false,
            false,
        ));
    }

    let existing_status = inspect_worker_process_status_for_paths(&paths);
    if existing_status.running {
        return Ok(start_result(
            &paths,
            "already_running",
            false,
            command,
            None,
            false,
            false,
            false,
        ));
    }

    ensure_parent_dir(&PathBuf::from(&paths.pid_file), "pid file")?;
    ensure_parent_dir(&PathBuf::from(&paths.log_file), "worker log")?;
    ensure_parent_dir(&PathBuf::from(&paths.env_path), "worker env")?;
    ensure_parent_dir(
        &PathBuf::from(&paths.termination_context_path),
        "termination context",
    )?;

    let termination_context_removed =
        remove_file_if_exists(&PathBuf::from(&paths.termination_context_path))?;
    let env = build_worker_start_env(&repo_root, input.parent_env);
    let env_file_seeded = false;
    let pid = spawn_worker_command(&command, &repo_root, &PathBuf::from(&paths.log_file), &env)?;
    write_pid_file(&PathBuf::from(&paths.pid_file), pid)?;

    Ok(start_result(
        &paths,
        "started",
        true,
        command,
        None,
        env_file_seeded,
        termination_context_removed,
        true,
    ))
}

pub fn start_worker_process_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentWorkerStartInput =
        decode_from_value(request, "invalid ait-agent supervisor start request")?;
    encode_to_value(
        &start_worker_process(input)?,
        "failed to serialize ait-agent supervisor start result",
    )
}

fn runtime_health(
    paths: &AgentWorkerProcessPaths,
    pid_info: AgentWorkerPidFileInspection,
) -> AgentWorkerRuntimeHealth {
    let log_file = PathBuf::from(&paths.log_file);
    AgentWorkerRuntimeHealth {
        pid_file_exists: pid_info.pid_file_exists,
        pid_file_readable: pid_info.pid_file_readable,
        pid_file_valid: pid_info.pid_file_valid,
        pid_file_state: pid_info.state,
        log_exists: path_exists(&log_file),
        log_size_bytes: path_size_bytes(&log_file),
        sync_state_exists: path_exists(&PathBuf::from(&paths.sync_state_path)),
        env_exists: path_exists(&PathBuf::from(&paths.env_path)),
        termination_context_exists: path_exists(&PathBuf::from(&paths.termination_context_path)),
    }
}

fn stop_result(
    paths: &AgentWorkerProcessPaths,
    stop_state: &str,
    stopped: bool,
    target_pid: Option<i64>,
    termination_context_written: bool,
    termination_context_removed: bool,
    pid_file_removed: bool,
) -> AgentWorkerStopResult {
    let status = inspect_worker_process_status_for_paths(paths);
    AgentWorkerStopResult {
        stopped,
        stop_state: stop_state.to_string(),
        running: status.running,
        pid: status.pid,
        target_pid,
        health: status.health,
        termination_context_written,
        termination_context_removed,
        pid_file_removed,
        python_worker_execution_allowed: false,
        migration_stage: MIGRATION_STAGE.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn start_result(
    paths: &AgentWorkerProcessPaths,
    start_state: &str,
    started: bool,
    command: Vec<String>,
    diagnostic: Option<&str>,
    env_file_seeded: bool,
    termination_context_removed: bool,
    pid_file_written: bool,
) -> AgentWorkerStartResult {
    let status = inspect_worker_process_status_for_paths(paths);
    AgentWorkerStartResult {
        started,
        start_state: start_state.to_string(),
        running: status.running,
        pid: status.pid,
        command,
        diagnostic: diagnostic.map(ToString::to_string),
        health: status.health,
        env_file_seeded,
        termination_context_removed,
        pid_file_written,
        python_worker_execution_allowed: false,
        migration_stage: MIGRATION_STAGE.to_string(),
    }
}

fn inspect_pid_file(path: &Path) -> AgentWorkerPidFileInspection {
    let mut state = AgentWorkerPidFileInspection {
        pid_file_exists: false,
        pid_file_readable: false,
        pid_file_valid: false,
        running: false,
        state: "missing".to_string(),
        pid: None,
    };
    if !path_exists(path) {
        return state;
    }
    state.pid_file_exists = true;
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw.trim().to_string(),
        Err(_) => {
            state.state = "unreadable".to_string();
            return state;
        }
    };
    state.pid_file_readable = true;
    if raw.is_empty() {
        state.state = "empty".to_string();
        return state;
    }
    let Ok(pid) = raw.parse::<i64>() else {
        state.state = "invalid".to_string();
        return state;
    };
    if pid <= 0 {
        state.state = "invalid".to_string();
        return state;
    }
    state.pid = Some(pid);
    state.pid_file_valid = true;
    state.running = process_is_alive(pid);
    state.state = if state.running { "running" } else { "stale" }.to_string();
    state
}

fn normalize_start_repo_root(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("repo_root must not be empty".to_string());
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        return Err(format!(
            "repo_root must be an absolute path, got `{trimmed}`"
        ));
    }
    Ok(path)
}

fn normalize_argv(argv: Vec<String>) -> Vec<String> {
    argv.into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

fn ensure_parent_dir(path: &Path, label: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{label} path `{}` has no parent", path.display()))?;
    fs::create_dir_all(parent).map_err(|err| {
        format!(
            "failed to create {label} directory `{}`: {err}",
            parent.display()
        )
    })
}

fn build_worker_start_env(
    repo_root: &Path,
    mut env: BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    env.insert(
        names::AIT_REPO_ROOT.to_string(),
        repo_root.to_string_lossy().into_owned(),
    );
    env
}

pub(crate) fn runtime_env_value(path: &Path, names: &[&str]) -> Option<String> {
    let values = load_simple_env_file(path);
    names.iter().find_map(|name| {
        values
            .get(*name)
            .and_then(|value| clean_optional_text(Some(value)))
    })
}

fn load_simple_env_file(path: &Path) -> BTreeMap<String, String> {
    let Ok(raw) = fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    raw.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            let (key, value) = trimmed.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let mut value = value.trim().to_string();
            if value.len() >= 2 {
                let first = value.as_bytes()[0] as char;
                let last = value.as_bytes()[value.len() - 1] as char;
                if first == last && (first == '"' || first == '\'') {
                    value = value[1..value.len() - 1].to_string();
                }
            }
            Some((key.to_string(), value))
        })
        .collect()
}

fn spawn_worker_command(
    argv: &[String],
    repo_root: &Path,
    log_file: &Path,
    env: &BTreeMap<String, String>,
) -> Result<i64, String> {
    let log_handle = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_file)
        .map_err(|err| format!("failed to open worker log `{}`: {err}", log_file.display()))?;
    let stderr_handle = log_handle.try_clone().map_err(|err| {
        format!(
            "failed to clone worker log handle `{}` for stderr: {err}",
            log_file.display()
        )
    })?;
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(repo_root)
        .env_clear()
        .envs(env.iter())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_handle))
        .stderr(Stdio::from(stderr_handle));
    detach_worker_session(&mut command);
    let child = command.spawn().map_err(|err| {
        format!(
            "failed to spawn Rust ait-agent worker command `{}`: {err}",
            argv.join(" ")
        )
    })?;
    Ok(i64::from(child.id()))
}

#[cfg(unix)]
fn detach_worker_session(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

#[cfg(windows)]
fn detach_worker_session(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW};

    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

fn write_pid_file(path: &Path, pid: i64) -> Result<(), String> {
    ensure_parent_dir(path, "pid file")?;
    fs::write(path, format!("{pid}\n")).map_err(|err| {
        format!(
            "failed to write worker pid file `{}`: {err}",
            path.display()
        )
    })
}

fn read_tail_lines(path: &Path, line_count: usize) -> Vec<String> {
    if line_count == 0 || !path_exists(path) {
        return Vec::new();
    }
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut tail = VecDeque::with_capacity(line_count);
    for line in text.lines() {
        if tail.len() == line_count {
            tail.pop_front();
        }
        tail.push_back(line.to_string());
    }
    tail.into_iter().collect()
}

fn path_exists(path: &Path) -> bool {
    fs::metadata(path).is_ok()
}

fn path_size_bytes(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn remove_file_if_exists(path: &Path) -> Result<bool, String> {
    if !path_exists(path) {
        return Ok(false);
    }
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(format!("failed to remove `{}`: {err}", path.display())),
    }
}

fn write_termination_context(
    path: &Path,
    pid: i64,
    reason: String,
    worker_name: String,
) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "termination context path `{}` has no parent",
            path.display()
        )
    })?;
    fs::create_dir_all(parent).map_err(|err| {
        format!(
            "failed to create termination context directory `{}`: {err}",
            parent.display()
        )
    })?;
    let payload = json!({
        "pid": pid,
        "reason": reason,
        "worker_name": worker_name,
        "issued_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        "issued_by_pid": std::process::id(),
    });
    let tmp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "termination.json".into()),
        unique_nonce()
    ));
    {
        let mut handle = fs::File::create(&tmp_path).map_err(|err| {
            format!(
                "failed to create temporary termination context `{}`: {err}",
                tmp_path.display()
            )
        })?;
        write_pretty_value(
            &mut handle,
            &payload,
            &format!(
                "failed to serialize termination context `{}`",
                path.display()
            ),
        )?;
        handle.write_all(b"\n").map_err(|err| {
            format!(
                "failed to finish termination context `{}`: {err}",
                tmp_path.display()
            )
        })?;
    }
    #[cfg(windows)]
    if path_exists(path) {
        fs::remove_file(path).map_err(|err| {
            format!(
                "failed to replace termination context `{}`: {err}",
                path.display()
            )
        })?;
    }
    fs::rename(&tmp_path, path).map_err(|err| {
        format!(
            "failed to publish termination context `{}` from `{}`: {err}",
            path.display(),
            tmp_path.display()
        )
    })
}

fn unique_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn clean_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn duration_from_seconds(value: Option<f64>, default_seconds: f64) -> Duration {
    let seconds = value
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(default_seconds);
    Duration::from_secs_f64(seconds)
}

fn stop_success_state(value: &str) -> bool {
    matches!(value, "already_stopped" | "stopped" | "killed")
}

fn stop_pid(pid: i64, stop_timeout: Duration, kill_grace: Duration) -> Result<String, String> {
    match send_process_signal(pid, SignalKind::Term)? {
        SignalDelivery::Delivered => {}
        SignalDelivery::NotFound => return Ok("already_stopped".to_string()),
    }
    if wait_until_not_running(pid, stop_timeout) {
        return Ok("stopped".to_string());
    }
    match send_process_signal(pid, SignalKind::Kill)? {
        SignalDelivery::Delivered => {}
        SignalDelivery::NotFound => return Ok("stopped".to_string()),
    }
    if wait_until_not_running(pid, kill_grace) {
        return Ok("killed".to_string());
    }
    Ok("still_running".to_string())
}

fn wait_until_not_running(pid: i64, timeout: Duration) -> bool {
    if !process_is_alive(pid) {
        return true;
    }
    let deadline = Instant::now() + timeout;
    loop {
        if !process_is_alive(pid) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let remaining = deadline.saturating_duration_since(now);
        thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}

#[derive(Debug, Clone, Copy)]
enum SignalKind {
    Term,
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignalDelivery {
    Delivered,
    NotFound,
}

#[cfg(unix)]
fn send_process_signal(pid: i64, signal: SignalKind) -> Result<SignalDelivery, String> {
    if pid <= 0 || pid > libc::pid_t::MAX as i64 {
        return Ok(SignalDelivery::NotFound);
    }
    let raw_signal = match signal {
        SignalKind::Term => libc::SIGTERM,
        SignalKind::Kill => libc::SIGKILL,
    };
    let result = unsafe { libc::kill(pid as libc::pid_t, raw_signal) };
    if result == 0 {
        return Ok(SignalDelivery::Delivered);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::ESRCH) => Ok(SignalDelivery::NotFound),
        Some(libc::EPERM) => Err(format!("permission denied while signaling pid {pid}")),
        _ => Err(format!("failed to signal pid {pid}: {err}")),
    }
}

#[cfg(windows)]
fn send_process_signal(pid: i64, signal: SignalKind) -> Result<SignalDelivery, String> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    let Ok(pid) = u32::try_from(pid) else {
        return Ok(SignalDelivery::NotFound);
    };
    if pid == 0 || !process_is_alive(i64::from(pid)) {
        return Ok(SignalDelivery::NotFound);
    }
    if matches!(signal, SignalKind::Term) {
        // The supervisor writes the worker termination context before reaching
        // this function. Windows workers poll that context as their portable
        // graceful-stop control channel, so no console attachment is required.
        return Ok(SignalDelivery::Delivered);
    }

    let handle = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if handle.is_null() {
        let error = std::io::Error::last_os_error();
        return match error.raw_os_error().map(|code| code as u32) {
            Some(ERROR_INVALID_PARAMETER) => Ok(SignalDelivery::NotFound),
            Some(ERROR_ACCESS_DENIED) => {
                Err(format!("permission denied while terminating pid {pid}"))
            }
            _ => Err(format!("failed to open pid {pid} for termination: {error}")),
        };
    }
    let result = unsafe { TerminateProcess(handle, 1) };
    let error = (result == 0).then(std::io::Error::last_os_error);
    unsafe {
        CloseHandle(handle);
    }
    match error {
        Some(error) => Err(format!("failed to terminate pid {pid}: {error}")),
        None => Ok(SignalDelivery::Delivered),
    }
}

#[cfg(unix)]
pub(super) fn process_is_alive(pid: i64) -> bool {
    if pid <= 0 || pid > libc::pid_t::MAX as i64 {
        return false;
    }
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result != 0 {
        return std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    }
    match pid_status_text(pid) {
        None => true,
        Some(status) if status.is_empty() => false,
        Some(status) => !status.to_ascii_uppercase().contains('Z'),
    }
}

#[cfg(windows)]
pub(super) fn process_is_alive(pid: i64) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, STILL_ACTIVE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let Ok(pid) = u32::try_from(pid) else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return match std::io::Error::last_os_error()
            .raw_os_error()
            .map(|code| code as u32)
        {
            Some(ERROR_ACCESS_DENIED) => true,
            Some(ERROR_INVALID_PARAMETER) | None => false,
            Some(_) => false,
        };
    }
    let mut exit_code = 0_u32;
    let inspected = unsafe { GetExitCodeProcess(handle, &mut exit_code) } != 0;
    unsafe {
        CloseHandle(handle);
    }
    inspected && exit_code == STILL_ACTIVE as u32
}

#[cfg(target_os = "linux")]
fn pid_status_text(pid: i64) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    linux_proc_process_state(&stat).map(str::to_string)
}

#[cfg(target_os = "linux")]
fn linux_proc_process_state(stat: &str) -> Option<&str> {
    let command_end = stat.rfind(')')?;
    stat.get(command_end + 1..)?.split_whitespace().next()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn pid_status_text(pid: i64) -> Option<String> {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return Some(String::new());
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests;
