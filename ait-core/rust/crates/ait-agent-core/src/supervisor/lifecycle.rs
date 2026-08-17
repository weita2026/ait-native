use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ait_core::json_support::{json, JsonValue};
use serde::{Deserialize, Serialize};

use crate::json_support::{decode_from_value, encode_to_value, parse_value, write_value};
use crate::transport::TransportKind;

use super::process::process_is_alive;

const DEFAULT_STOP_TIMEOUT_SECONDS: f64 = 10.0;
const DEFAULT_KILL_GRACE_SECONDS: f64 = 2.0;
const MIGRATION_STAGE: &str = "rust_agent_supervisor_lifecycle_contract";
const LIFECYCLE_LOCK_MIGRATION_STAGE: &str = "rust_agent_supervisor_lifecycle_lock_contract";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkerLifecycleOperation {
    Start,
    Stop,
    Restart,
    Status,
}

impl AgentWorkerLifecycleOperation {
    fn requires_lock(self) -> bool {
        matches!(self, Self::Start | Self::Stop | Self::Restart)
    }

    fn spawns_worker(self) -> bool {
        matches!(self, Self::Start | Self::Restart)
    }

    fn clears_termination_context_before_start(self) -> bool {
        matches!(self, Self::Start | Self::Restart)
    }

    fn writes_termination_context_before_stop(self) -> bool {
        matches!(self, Self::Stop | Self::Restart)
    }

    fn removes_pid_file_on_successful_stop(self) -> bool {
        matches!(self, Self::Stop | Self::Restart)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerLifecyclePlanInput {
    pub repo_root: String,
    pub operation: AgentWorkerLifecycleOperation,
    pub worker: AgentWorkerLifecycleSpec,
    #[serde(default)]
    pub runtime_root: Option<String>,
    #[serde(default)]
    pub stop_timeout_seconds: Option<f64>,
    #[serde(default)]
    pub kill_grace_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerLifecycleSpec {
    #[serde(rename = "kind", alias = "transport")]
    pub transport: TransportKind,
    pub name: String,
    #[serde(default)]
    pub sync_state_path: Option<String>,
    #[serde(default)]
    pub pid_file: Option<String>,
    #[serde(default)]
    pub log_file: Option<String>,
    #[serde(default)]
    pub env_path: Option<String>,
    #[serde(default)]
    pub termination_context_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerLifecycleLockAcquireInput {
    pub repo_root: String,
    #[serde(rename = "kind", alias = "transport")]
    pub transport: TransportKind,
    pub action: String,
    #[serde(default)]
    pub runtime_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWorkerLifecycleLockReleaseInput {
    pub lifecycle_lock_path: String,
    pub lock_token: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentWorkerRuntimePaths {
    pub runtime_root: String,
    pub sync_state_path: String,
    pub pid_file: String,
    pub log_file: String,
    pub env_path: String,
    pub termination_context_path: String,
    pub lifecycle_lock_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWorkerLifecycleLockAcquireResult {
    pub acquired: bool,
    pub action: String,
    pub transport: TransportKind,
    pub lifecycle_lock_path: String,
    pub lock_token: String,
    pub stale_lock_removed: bool,
    pub lock_pid: i64,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentWorkerLifecycleLockReleaseResult {
    pub released: bool,
    pub lifecycle_lock_path: String,
    pub lock_token: String,
    pub lock_present: bool,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentWorkerLifecyclePlan {
    pub worker_key: String,
    pub worker_name: String,
    pub transport: TransportKind,
    pub operation: AgentWorkerLifecycleOperation,
    pub paths: AgentWorkerRuntimePaths,
    pub directories_to_create: Vec<String>,
    pub lock_required: bool,
    pub status_probe_required: bool,
    pub clear_termination_context_before_start: bool,
    pub write_termination_context_before_stop: bool,
    pub remove_pid_file_on_successful_stop: bool,
    pub spawn_worker: bool,
    pub stop_timeout_seconds: f64,
    pub kill_grace_seconds: f64,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: String,
}

pub fn plan_worker_supervisor_lifecycle(
    input: AgentWorkerLifecyclePlanInput,
) -> Result<AgentWorkerLifecyclePlan, String> {
    let repo_root = normalize_repo_root(&input.repo_root)?;
    let runtime_root = resolve_runtime_root(&repo_root, input.runtime_root.as_deref());
    let transport = input.worker.transport;
    let worker_name = normalize_worker_name(&input.worker.name)?;
    let worker_key = format!("{transport}/{worker_name}");
    let label = safe_file_label(&worker_name);
    let transport_label = transport.as_str();

    let sync_state_path = resolve_runtime_path(
        &repo_root,
        &runtime_root,
        input.worker.sync_state_path.as_deref(),
        &format!("{transport_label}-{label}-sync.json"),
    );
    let pid_file = resolve_runtime_path(
        &repo_root,
        &runtime_root,
        input.worker.pid_file.as_deref(),
        &format!("{transport_label}-{label}.pid"),
    );
    let log_file = resolve_runtime_path(
        &repo_root,
        &runtime_root,
        input.worker.log_file.as_deref(),
        &format!("{transport_label}-{label}.log"),
    );
    let env_path = resolve_runtime_path(
        &repo_root,
        &runtime_root,
        input.worker.env_path.as_deref(),
        &format!("{transport_label}.env"),
    );
    let termination_context_path = resolve_runtime_path(
        &repo_root,
        &runtime_root,
        input.worker.termination_context_path.as_deref(),
        &format!("{transport_label}-{label}-termination.json"),
    );
    let lifecycle_lock_path = runtime_root.join(format!("{transport_label}-lifecycle.lock"));
    let paths = AgentWorkerRuntimePaths {
        runtime_root: path_to_string(&runtime_root),
        sync_state_path: path_to_string(&sync_state_path),
        pid_file: path_to_string(&pid_file),
        log_file: path_to_string(&log_file),
        env_path: path_to_string(&env_path),
        termination_context_path: path_to_string(&termination_context_path),
        lifecycle_lock_path: path_to_string(&lifecycle_lock_path),
    };
    let directories_to_create = directories_to_create(&[
        sync_state_path,
        pid_file,
        log_file,
        env_path,
        termination_context_path,
        lifecycle_lock_path,
    ]);
    Ok(AgentWorkerLifecyclePlan {
        worker_key,
        worker_name,
        transport,
        operation: input.operation,
        paths,
        directories_to_create,
        lock_required: input.operation.requires_lock(),
        status_probe_required: true,
        clear_termination_context_before_start: input
            .operation
            .clears_termination_context_before_start(),
        write_termination_context_before_stop: input
            .operation
            .writes_termination_context_before_stop(),
        remove_pid_file_on_successful_stop: input.operation.removes_pid_file_on_successful_stop(),
        spawn_worker: input.operation.spawns_worker(),
        stop_timeout_seconds: normalize_seconds(
            input.stop_timeout_seconds,
            DEFAULT_STOP_TIMEOUT_SECONDS,
            "stop_timeout_seconds",
        )?,
        kill_grace_seconds: normalize_seconds(
            input.kill_grace_seconds,
            DEFAULT_KILL_GRACE_SECONDS,
            "kill_grace_seconds",
        )?,
        python_worker_execution_allowed: false,
        migration_stage: MIGRATION_STAGE.to_string(),
    })
}

pub fn plan_worker_supervisor_lifecycle_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentWorkerLifecyclePlanInput =
        decode_from_value(request, "invalid ait-agent supervisor lifecycle request")?;
    encode_to_value(
        &plan_worker_supervisor_lifecycle(input)?,
        "failed to serialize ait-agent supervisor lifecycle plan",
    )
}

pub fn acquire_worker_lifecycle_lock(
    input: AgentWorkerLifecycleLockAcquireInput,
) -> Result<AgentWorkerLifecycleLockAcquireResult, String> {
    let action = normalize_lifecycle_lock_action(&input.action)?;
    let lock_path = supervisor_lifecycle_lock_path(
        &input.repo_root,
        input.transport,
        input.runtime_root.as_deref(),
    )?;
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create ait-agent lifecycle lock directory `{}`: {err}",
                path_to_string(parent)
            )
        })?;
    }

    let mut stale_lock_removed = false;
    loop {
        let file_result = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path);
        match file_result {
            Ok(mut file) => {
                let lock_pid = std::process::id() as i64;
                let lock_token = format!("{lock_pid}-{}", unique_nonce());
                let payload = json!({
                    "action": action,
                    "created_at_unix_ms": unix_epoch_millis(),
                    "lock_token": lock_token,
                    "migration_stage": LIFECYCLE_LOCK_MIGRATION_STAGE,
                    "pid": lock_pid,
                });
                let write_result = (|| -> Result<(), String> {
                    write_value(
                        &mut file,
                        &payload,
                        &format!(
                            "failed to write ait-agent lifecycle lock `{}`",
                            path_to_string(&lock_path)
                        ),
                    )?;
                    file.write_all(b"\n").map_err(|err| {
                        format!(
                            "failed to finish ait-agent lifecycle lock `{}`: {err}",
                            path_to_string(&lock_path)
                        )
                    })?;
                    Ok(())
                })();
                if let Err(err) = write_result {
                    let _ = fs::remove_file(&lock_path);
                    return Err(err);
                }
                return Ok(AgentWorkerLifecycleLockAcquireResult {
                    acquired: true,
                    action,
                    transport: input.transport,
                    lifecycle_lock_path: path_to_string(&lock_path),
                    lock_token,
                    stale_lock_removed,
                    lock_pid,
                    python_worker_execution_allowed: false,
                    migration_stage: LIFECYCLE_LOCK_MIGRATION_STAGE.to_string(),
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let pid = read_lifecycle_lock_pid(&lock_path);
                if pid.is_some_and(|pid| !process_is_alive(pid)) {
                    fs::remove_file(&lock_path).map_err(|remove_err| {
                        format!(
                            "failed to remove stale ait-agent lifecycle lock `{}`: {remove_err}",
                            path_to_string(&lock_path)
                        )
                    })?;
                    stale_lock_removed = true;
                    continue;
                }
                return Err(format!(
                    "{} lifecycle lock is busy: {}",
                    input.transport.as_str(),
                    path_to_string(&lock_path)
                ));
            }
            Err(err) => {
                return Err(format!(
                    "failed to create ait-agent lifecycle lock `{}`: {err}",
                    path_to_string(&lock_path)
                ));
            }
        }
    }
}

pub fn acquire_worker_lifecycle_lock_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentWorkerLifecycleLockAcquireInput = decode_from_value(
        request,
        "invalid ait-agent supervisor lifecycle lock request",
    )?;
    encode_to_value(
        &acquire_worker_lifecycle_lock(input)?,
        "failed to serialize ait-agent supervisor lifecycle lock acquire result",
    )
}

pub fn release_worker_lifecycle_lock(
    input: AgentWorkerLifecycleLockReleaseInput,
) -> Result<AgentWorkerLifecycleLockReleaseResult, String> {
    let lifecycle_lock_path = normalize_lifecycle_lock_path(&input.lifecycle_lock_path)?;
    let lock_token = normalize_lifecycle_lock_token(&input.lock_token)?;
    if !lifecycle_lock_path.exists() {
        return Ok(AgentWorkerLifecycleLockReleaseResult {
            released: false,
            lifecycle_lock_path: path_to_string(&lifecycle_lock_path),
            lock_token,
            lock_present: false,
            python_worker_execution_allowed: false,
            migration_stage: LIFECYCLE_LOCK_MIGRATION_STAGE.to_string(),
        });
    }
    let current_token = read_lifecycle_lock_token(&lifecycle_lock_path).ok_or_else(|| {
        format!(
            "ait-agent lifecycle lock `{}` is missing an owned lock token",
            path_to_string(&lifecycle_lock_path)
        )
    })?;
    if current_token != lock_token {
        return Err(format!(
            "ait-agent lifecycle lock token mismatch for `{}`",
            path_to_string(&lifecycle_lock_path)
        ));
    }
    fs::remove_file(&lifecycle_lock_path).map_err(|err| {
        format!(
            "failed to release ait-agent lifecycle lock `{}`: {err}",
            path_to_string(&lifecycle_lock_path)
        )
    })?;
    Ok(AgentWorkerLifecycleLockReleaseResult {
        released: true,
        lifecycle_lock_path: path_to_string(&lifecycle_lock_path),
        lock_token,
        lock_present: true,
        python_worker_execution_allowed: false,
        migration_stage: LIFECYCLE_LOCK_MIGRATION_STAGE.to_string(),
    })
}

pub fn release_worker_lifecycle_lock_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentWorkerLifecycleLockReleaseInput = decode_from_value(
        request,
        "invalid ait-agent supervisor lifecycle lock release request",
    )?;
    encode_to_value(
        &release_worker_lifecycle_lock(input)?,
        "failed to serialize ait-agent supervisor lifecycle lock release result",
    )
}

fn normalize_repo_root(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("repo_root must not be empty".to_string());
    }
    let path = expand_home(trimmed);
    if !path.is_absolute() {
        return Err(format!(
            "repo_root must be an absolute path, got `{trimmed}`"
        ));
    }
    Ok(path)
}

fn supervisor_lifecycle_lock_path(
    repo_root: &str,
    transport: TransportKind,
    runtime_root: Option<&str>,
) -> Result<PathBuf, String> {
    let plan = plan_worker_supervisor_lifecycle(AgentWorkerLifecyclePlanInput {
        repo_root: repo_root.to_string(),
        operation: AgentWorkerLifecycleOperation::Start,
        worker: AgentWorkerLifecycleSpec {
            transport,
            name: "supervisor".to_string(),
            sync_state_path: None,
            pid_file: None,
            log_file: None,
            env_path: None,
            termination_context_path: None,
        },
        runtime_root: runtime_root.map(str::to_string),
        stop_timeout_seconds: None,
        kill_grace_seconds: None,
    })?;
    Ok(PathBuf::from(plan.paths.lifecycle_lock_path))
}

fn normalize_lifecycle_lock_action(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("lifecycle lock action must not be empty".to_string());
    }
    Ok(trimmed.to_string())
}

fn normalize_lifecycle_lock_path(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("lifecycle_lock_path must not be empty".to_string());
    }
    Ok(expand_home(trimmed))
}

fn normalize_lifecycle_lock_token(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("lock_token must not be empty".to_string());
    }
    Ok(trimmed.to_string())
}

fn read_lifecycle_lock_pid(path: &Path) -> Option<i64> {
    let payload = parse_value(
        &fs::read_to_string(path).ok()?,
        "failed to parse ait-agent lifecycle lock",
    )
    .ok()?;
    let pid = payload.get("pid")?.as_i64()?;
    (pid > 0).then_some(pid)
}

fn read_lifecycle_lock_token(path: &Path) -> Option<String> {
    let payload = parse_value(
        &fs::read_to_string(path).ok()?,
        "failed to parse ait-agent lifecycle lock",
    )
    .ok()?;
    payload
        .get("lock_token")?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn unix_epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn unique_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn resolve_runtime_root(repo_root: &Path, override_value: Option<&str>) -> PathBuf {
    let Some(value) = override_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return repo_root.join(".ait").join("agent-runtime");
    };
    let path = expand_home(value);
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

fn resolve_runtime_path(
    repo_root: &Path,
    runtime_root: &Path,
    override_value: Option<&str>,
    default_name: &str,
) -> PathBuf {
    let Some(value) = override_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return runtime_root.join(default_name);
    };
    let path = expand_home(value);
    if path.is_absolute() {
        path
    } else {
        repo_root.join(path)
    }
}

fn normalize_worker_name(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("worker.name must not be empty".to_string());
    }
    if trimmed.contains('/') {
        return Err("worker.name must not contain `/`".to_string());
    }
    Ok(trimmed.to_string())
}

fn safe_file_label(value: &str) -> String {
    let label = value
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let label = label.trim_matches('-');
    if label.is_empty() {
        "worker".to_string()
    } else {
        label.to_string()
    }
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn normalize_seconds(
    value: Option<f64>,
    default_value: f64,
    field_name: &str,
) -> Result<f64, String> {
    let seconds = value.unwrap_or(default_value);
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(format!("{field_name} must be a non-negative finite number"));
    }
    Ok(seconds)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn directories_to_create(paths: &[PathBuf]) -> Vec<String> {
    let mut directories = BTreeSet::new();
    for path in paths {
        if let Some(parent) = path.parent() {
            directories.insert(path_to_string(parent));
        }
    }
    directories.into_iter().collect()
}

#[cfg(test)]
mod tests;
