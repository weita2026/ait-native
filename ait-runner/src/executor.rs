use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ait_core::server_operational::{
    NATIVE_JOB_REPOSITORY_CI_UNIX_PATH, NATIVE_JOB_REPOSITORY_CI_WINDOWS_PATH,
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use sha2::{Digest, Sha256};

use crate::RunnerError;
use crate::materialize::{
    MaterializationStats, RemoteSnapshotProvider, RemoteSnapshotReference,
    materialize_remote_snapshot,
};
use crate::protocol::{
    CleanupEvidence, ExecutionEvidence, LEGACY_NATIVE_JOB_CONTRACT, MaterializationEvidence,
    NATIVE_RESULT_CONTRACT, NativeJobRequest, NativeResult, STREAM_TAIL_BYTES, SourceSpec,
    StreamEvidence, SuiteResult, TerminalStatus, TestStatus,
};

const MAX_SOURCE_ENTRIES: u64 = 1_000_000;
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROCESS_GROUP_GRACE: Duration = Duration::from_millis(50);
static ATTEMPT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct ExecutorConfig {
    pub source_root: PathBuf,
    pub attempt_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct NativeExecutor {
    config: ExecutorConfig,
}

#[derive(Debug)]
pub(crate) struct AttemptRootLease {
    _lock: File,
}

#[derive(Debug)]
pub struct Preflight {
    source: Option<PathBuf>,
}

#[derive(Debug)]
struct AttemptGuard {
    path: PathBuf,
    cleaned: bool,
}

#[derive(Debug)]
struct ProcessOutcome {
    status: ExitStatus,
    timed_out: bool,
    duration: Duration,
    stdout: StreamEvidence,
    stderr: StreamEvidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunnerPlatform {
    Unix,
    Windows,
}

#[derive(Debug, PartialEq, Eq)]
struct ProcessInvocation {
    program: PathBuf,
    arguments: Vec<OsString>,
    fixed_environment: Vec<(OsString, OsString)>,
}

#[derive(Debug)]
struct StreamCapture {
    byte_count: u64,
    sha256: String,
    tail: Vec<u8>,
}

impl NativeExecutor {
    pub fn new(config: ExecutorConfig) -> Self {
        Self { config }
    }

    pub fn preflight(&self, request: &NativeJobRequest) -> Result<Preflight, RunnerError> {
        request.validate_execution()?;
        let platform = execution_platform(request)?;
        validate_platform_runtime(platform)?;
        let SourceSpec::LocalDirectory { path } = &request.source else {
            return Ok(Preflight { source: None });
        };
        let configured_root = canonical_directory(&self.config.source_root, "canonicalize")?;
        let requested_source = configured_root.join(path);
        let source_metadata = fs::symlink_metadata(&requested_source)
            .map_err(|error| RunnerError::fs("inspect source", &requested_source, error))?;
        if source_metadata.file_type().is_symlink() || !source_metadata.is_dir() {
            return Err(RunnerError::InvalidRequest(format!(
                "source.path `{path}` must identify a regular directory"
            )));
        }
        let source = canonical_directory(&requested_source, "canonicalize source")?;
        if !source.starts_with(&configured_root) {
            return Err(RunnerError::InvalidRequest(format!(
                "source.path `{path}` escapes the configured source root"
            )));
        }
        let working_directory = source.join(&request.command.working_directory);
        let working_directory =
            canonical_directory(&working_directory, "canonicalize working directory")?;
        if !working_directory.starts_with(&source) {
            return Err(RunnerError::InvalidRequest(
                "command.working_directory escapes the selected source".to_string(),
            ));
        }
        let source_entrypoint = working_directory.join(repository_ci_entrypoint(platform));
        validate_repository_ci_entrypoint(&source_entrypoint, platform)?;
        Ok(Preflight {
            source: Some(source),
        })
    }

    pub fn execute(&self, request: &NativeJobRequest) -> Result<NativeResult, RunnerError> {
        self.execute_with_provider(request, None)
    }

    pub(crate) fn acquire_attempt_root_lease(&self) -> Result<AttemptRootLease, RunnerError> {
        let attempt_parent = prepare_attempt_parent(&self.config.attempt_root)?;
        let lock_path = attempt_parent.join(".ait-runner-attempt-root.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| RunnerError::fs("open attempt root lock", &lock_path, error))?;
        lock.try_lock().map_err(|error| {
            RunnerError::Process(format!(
                "attempt root `{}` is already owned by another runner: {error}",
                attempt_parent.display()
            ))
        })?;
        reclaim_stale_attempts(&attempt_parent)?;
        Ok(AttemptRootLease { _lock: lock })
    }

    pub fn execute_with_provider(
        &self,
        request: &NativeJobRequest,
        provider: Option<&dyn RemoteSnapshotProvider>,
    ) -> Result<NativeResult, RunnerError> {
        let preflight = self.preflight(request)?;
        let attempt_parent = prepare_attempt_parent(&self.config.attempt_root)?;
        if preflight
            .source
            .as_ref()
            .is_some_and(|source| attempt_parent.starts_with(source))
        {
            return Err(RunnerError::InvalidRequest(
                "attempt root must not be nested inside the selected source".to_string(),
            ));
        }
        let mut attempt = AttemptGuard::create(&attempt_parent)?;
        let run_result = self.execute_in_attempt(request, &preflight, &attempt, provider);
        let cleanup_result = attempt.cleanup();
        match (run_result, cleanup_result) {
            (Ok(mut result), Ok(())) => {
                let cleanup = CleanupEvidence {
                    attempt_root_removed: true,
                    remaining_owned_paths: 0,
                };
                result.cleanup = cleanup.clone();
                if let Some(suite) = result.suite_results.first_mut() {
                    suite.execution.cleanup = cleanup;
                }
                result.validate_bound()?;
                Ok(result)
            }
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(RunnerError::Cleanup(format!(
                "{cleanup}; original execution error: {error}"
            ))),
        }
    }

    fn execute_in_attempt(
        &self,
        request: &NativeJobRequest,
        preflight: &Preflight,
        attempt: &AttemptGuard,
        provider: Option<&dyn RemoteSnapshotProvider>,
    ) -> Result<NativeResult, RunnerError> {
        let workspace = attempt.path.join("workspace");
        fs::create_dir(&workspace)
            .map_err(|error| RunnerError::fs("create workspace", &workspace, error))?;
        let (materialization, external_environment) = match (&request.source, &preflight.source) {
            (SourceSpec::LocalDirectory { .. }, Some(source)) => {
                let mut stats = MaterializationStats::default();
                copy_directory_contents(source, &workspace, &mut stats, true)?;
                (stats, BTreeMap::new())
            }
            (
                SourceSpec::RemoteSnapshot {
                    repository_index,
                    repository_name,
                    snapshot_id,
                    external_repository_indexes,
                },
                None,
            ) => {
                let provider = provider.ok_or_else(|| {
                    RunnerError::InvalidRequest(
                        "remote_snapshot execution requires an ait-server source provider"
                            .to_string(),
                    )
                })?;
                let materialized = materialize_remote_snapshot(
                    provider,
                    &RemoteSnapshotReference {
                        repository_index: Some(*repository_index),
                        repository_name: repository_name.clone(),
                        legacy_repo_id: None,
                        snapshot_id: snapshot_id.clone(),
                        external_repository_indexes: external_repository_indexes.clone(),
                    },
                    &workspace,
                    &attempt.path.join("packs"),
                )?;
                (materialized.stats, materialized.environment)
            }
            (
                SourceSpec::LegacyRemoteSnapshot {
                    repo_name,
                    repo_id,
                    snapshot_id,
                },
                None,
            ) => {
                let provider = provider.ok_or_else(|| {
                    RunnerError::InvalidRequest(
                        "remote_snapshot execution requires an ait-server source provider"
                            .to_string(),
                    )
                })?;
                let materialized = materialize_remote_snapshot(
                    provider,
                    &RemoteSnapshotReference {
                        repository_index: None,
                        repository_name: repo_name.clone(),
                        legacy_repo_id: repo_id.clone(),
                        snapshot_id: snapshot_id.clone(),
                        external_repository_indexes: BTreeMap::new(),
                    },
                    &workspace,
                    &attempt.path.join("packs"),
                )?;
                (materialized.stats, materialized.environment)
            }
            _ => {
                return Err(RunnerError::Process(
                    "native source preflight state is inconsistent".to_string(),
                ));
            }
        };

        let working_directory = workspace.join(&request.command.working_directory);
        let working_directory = canonical_directory(
            &working_directory,
            "canonicalize materialized working directory",
        )?;
        if !working_directory.starts_with(&workspace) {
            return Err(RunnerError::InvalidRequest(
                "materialized working directory escaped its attempt workspace".to_string(),
            ));
        }
        let platform = execution_platform(request)?;
        let entrypoint = working_directory.join(repository_ci_entrypoint(platform));
        validate_repository_ci_entrypoint(&entrypoint, platform)?;

        let logs = attempt.path.join("logs");
        fs::create_dir(&logs)
            .map_err(|error| RunnerError::fs("create attempt logs", &logs, error))?;
        let stdout_path = logs.join("stdout.log");
        let stderr_path = logs.join("stderr.log");
        let environment = attempt_environment(
            &attempt.path,
            &workspace,
            &request.command.environment,
            &external_environment,
        )?;
        let invocation = repository_ci_invocation(
            platform,
            &entrypoint,
            &request.command.argv[1..],
            windows_system_root(platform)?.as_deref(),
        )?;
        validate_process_program(&invocation)?;
        let process = execute_process(
            &invocation,
            &environment,
            &working_directory,
            &stdout_path,
            &stderr_path,
            Duration::from_millis(request.timeout_ms),
        )?;

        let status = if process.timed_out {
            TerminalStatus::TimedOut
        } else if process.status.success() {
            TerminalStatus::Succeeded
        } else {
            TerminalStatus::CommandFailed
        };
        let tests_status = if status == TerminalStatus::Succeeded {
            TestStatus::Pass
        } else {
            TestStatus::Fail
        };
        let cleanup = CleanupEvidence {
            attempt_root_removed: false,
            remaining_owned_paths: 1,
        };
        let summary = match status {
            TerminalStatus::Succeeded => "repository CI entrypoint completed successfully",
            TerminalStatus::CommandFailed => {
                "repository CI entrypoint returned a non-zero exit status"
            }
            TerminalStatus::TimedOut => "repository CI entrypoint exceeded its execution timeout",
        }
        .to_string();
        let duration_ms = process.duration.as_millis().min(u128::from(u64::MAX)) as u64;
        let execution = ExecutionEvidence {
            contract: NATIVE_RESULT_CONTRACT,
            exit_code: process.status.code(),
            signal: exit_signal(&process.status),
            timed_out: process.timed_out,
            duration_ms,
            materialization: MaterializationEvidence {
                source_kind: request.source.source_kind(),
                file_count: materialization.file_count,
                total_bytes: materialization.total_bytes,
            },
            stdout: process.stdout,
            stderr: process.stderr,
            cleanup: cleanup.clone(),
        };
        Ok(NativeResult {
            contract: NATIVE_RESULT_CONTRACT,
            status,
            tests_status,
            suite_result_count: 1,
            suite_results: vec![SuiteResult {
                suite_id: request
                    .suite_id
                    .clone()
                    .unwrap_or_else(|| "repository-ci".to_string()),
                status: tests_status,
                blocking: true,
                mode: "gate",
                plane: "runner",
                runner_kind: "ait-runner/native",
                duration_seconds: process.duration.as_secs_f64(),
                summary,
                execution,
            }],
            cleanup,
        })
    }
}

impl AttemptGuard {
    fn create(parent: &Path) -> Result<Self, RunnerError> {
        let pid = std::process::id();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for _ in 0..128 {
            let sequence = ATTEMPT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("attempt-{pid}-{now}-{sequence}"));
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        cleaned: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(RunnerError::fs("create attempt root", &path, error));
                }
            }
        }
        Err(RunnerError::Process(
            "could not allocate a unique attempt directory".to_string(),
        ))
    }

    fn cleanup(&mut self) -> Result<(), RunnerError> {
        if self.cleaned {
            return Ok(());
        }
        match fs::remove_dir_all(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(RunnerError::Cleanup(format!(
                    "could not remove `{}`: {error}",
                    self.path.display()
                )));
            }
        }
        match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.cleaned = true;
                Ok(())
            }
            Ok(_) => Err(RunnerError::Cleanup(format!(
                "`{}` still exists after recursive cleanup",
                self.path.display()
            ))),
            Err(error) => Err(RunnerError::Cleanup(format!(
                "could not verify removal of `{}`: {error}",
                self.path.display()
            ))),
        }
    }
}

impl Drop for AttemptGuard {
    fn drop(&mut self) {
        if !self.cleaned {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn prepare_attempt_parent(path: &Path) -> Result<PathBuf, RunnerError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(RunnerError::InvalidRequest(format!(
                    "attempt root `{}` must be a regular directory",
                    path.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)
                .map_err(|source| RunnerError::fs("create attempt parent", path, source))?;
        }
        Err(error) => return Err(RunnerError::fs("inspect attempt parent", path, error)),
    }
    canonical_directory(path, "canonicalize attempt parent")
}

fn reclaim_stale_attempts(attempt_parent: &Path) -> Result<(), RunnerError> {
    for entry in fs::read_dir(attempt_parent)
        .map_err(|error| RunnerError::fs("read attempt parent", attempt_parent, error))?
    {
        let entry = entry
            .map_err(|error| RunnerError::fs("read attempt parent entry", attempt_parent, error))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_runner_attempt_name(name) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| RunnerError::fs("inspect stale attempt", &path, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(RunnerError::Cleanup(format!(
                "runner-owned stale path `{}` is not a regular directory",
                path.display()
            )));
        }
        fs::remove_dir_all(&path)
            .map_err(|error| RunnerError::fs("remove stale runner attempt", &path, error))?;
    }
    Ok(())
}

fn is_runner_attempt_name(name: &str) -> bool {
    let mut parts = name.split('-');
    parts.next() == Some("attempt")
        && parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts.next().is_none()
}

fn canonical_directory(path: &Path, operation: &'static str) -> Result<PathBuf, RunnerError> {
    let canonical =
        fs::canonicalize(path).map_err(|error| RunnerError::fs(operation, path, error))?;
    let metadata = fs::metadata(&canonical)
        .map_err(|error| RunnerError::fs("inspect canonical directory", &canonical, error))?;
    if !metadata.is_dir() {
        return Err(RunnerError::InvalidRequest(format!(
            "`{}` is not a directory",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn runner_platform(os: &str) -> Result<RunnerPlatform, RunnerError> {
    match os {
        "linux" | "macos" => Ok(RunnerPlatform::Unix),
        "windows" => Ok(RunnerPlatform::Windows),
        other => Err(RunnerError::Process(format!(
            "repository CI entrypoint is unsupported on runner operating system `{other}`"
        ))),
    }
}

fn execution_platform(request: &NativeJobRequest) -> Result<RunnerPlatform, RunnerError> {
    let platform = runner_platform(std::env::consts::OS)?;
    if request.contract == LEGACY_NATIVE_JOB_CONTRACT && platform == RunnerPlatform::Windows {
        return Err(RunnerError::InvalidRequest(
            "legacy native-job.v1 execution is unsupported on Windows; use native-job.v3"
                .to_string(),
        ));
    }
    Ok(platform)
}

fn repository_ci_entrypoint(platform: RunnerPlatform) -> &'static str {
    match platform {
        RunnerPlatform::Unix => NATIVE_JOB_REPOSITORY_CI_UNIX_PATH,
        RunnerPlatform::Windows => NATIVE_JOB_REPOSITORY_CI_WINDOWS_PATH,
    }
}

fn validate_repository_ci_entrypoint(
    path: &Path,
    platform: RunnerPlatform,
) -> Result<(), RunnerError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| RunnerError::fs("inspect repository CI entrypoint", path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(RunnerError::InvalidRequest(format!(
            "`{}` must be a regular, non-symlink file",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if platform == RunnerPlatform::Unix && metadata.permissions().mode() & 0o111 == 0 {
            return Err(RunnerError::InvalidRequest(format!(
                "`{}` must be executable",
                path.display()
            )));
        }
    }
    Ok(())
}

fn windows_system_root(platform: RunnerPlatform) -> Result<Option<PathBuf>, RunnerError> {
    if platform != RunnerPlatform::Windows {
        return Ok(None);
    }
    let raw = std::env::var_os("SystemRoot")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RunnerError::Process(
                "Windows runner process must provide an absolute SystemRoot".to_string(),
            )
        })?;
    let root = PathBuf::from(raw);
    if !root.is_absolute() {
        return Err(RunnerError::Process(
            "Windows runner process SystemRoot must be absolute".to_string(),
        ));
    }
    Ok(Some(root))
}

fn windows_powershell_path(system_root: &Path) -> PathBuf {
    system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe")
}

fn validate_platform_runtime(platform: RunnerPlatform) -> Result<(), RunnerError> {
    let Some(system_root) = windows_system_root(platform)? else {
        return Ok(());
    };
    let invocation = repository_ci_invocation(
        platform,
        Path::new(NATIVE_JOB_REPOSITORY_CI_WINDOWS_PATH),
        &[],
        Some(&system_root),
    )?;
    validate_process_program(&invocation)
}

fn repository_ci_invocation(
    platform: RunnerPlatform,
    entrypoint: &Path,
    arguments: &[String],
    system_root: Option<&Path>,
) -> Result<ProcessInvocation, RunnerError> {
    match platform {
        RunnerPlatform::Unix => Ok(ProcessInvocation {
            program: entrypoint.to_path_buf(),
            arguments: arguments.iter().map(OsString::from).collect(),
            fixed_environment: Vec::new(),
        }),
        RunnerPlatform::Windows => {
            let system_root = system_root.ok_or_else(|| {
                RunnerError::Process(
                    "Windows PowerShell resolution requires runner SystemRoot".to_string(),
                )
            })?;
            let mut invocation_arguments = vec![
                OsString::from("-NoLogo"),
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-File"),
                entrypoint.as_os_str().to_os_string(),
            ];
            invocation_arguments.extend(arguments.iter().map(OsString::from));
            Ok(ProcessInvocation {
                program: windows_powershell_path(system_root),
                arguments: invocation_arguments,
                fixed_environment: vec![(
                    OsString::from("SystemRoot"),
                    system_root.as_os_str().to_os_string(),
                )],
            })
        }
    }
}

fn validate_process_program(invocation: &ProcessInvocation) -> Result<(), RunnerError> {
    let metadata = fs::symlink_metadata(&invocation.program).map_err(|error| {
        RunnerError::fs(
            "inspect repository CI process executable",
            &invocation.program,
            error,
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(RunnerError::Process(format!(
            "repository CI process executable `{}` must be a regular, non-symlink file",
            invocation.program.display()
        )));
    }
    Ok(())
}

fn copy_directory_contents(
    source: &Path,
    destination: &Path,
    stats: &mut MaterializationStats,
    source_root: bool,
) -> Result<(), RunnerError> {
    let mut entries = fs::read_dir(source)
        .map_err(|error| RunnerError::fs("read source directory", source, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| RunnerError::fs("read source directory entry", source, error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if source_root
            && matches!(
                name.to_str(),
                Some(".ait" | ".git" | ".ait-worktree.json" | ".ait-worktree-links")
            )
        {
            continue;
        }
        stats.entry_count = stats.entry_count.saturating_add(1);
        if stats.entry_count > MAX_SOURCE_ENTRIES {
            return Err(RunnerError::InvalidRequest(format!(
                "source contains more than {MAX_SOURCE_ENTRIES} filesystem entries"
            )));
        }
        let source_path = entry.path();
        let destination_path = destination.join(&name);
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|error| RunnerError::fs("inspect source entry", &source_path, error))?;
        if metadata.file_type().is_symlink() {
            return Err(RunnerError::InvalidRequest(format!(
                "source entry `{}` is a symbolic link; local_directory v1 accepts regular files and directories only",
                source_path.display()
            )));
        }
        if metadata.is_dir() {
            fs::create_dir(&destination_path).map_err(|error| {
                RunnerError::fs("create materialized directory", &destination_path, error)
            })?;
            copy_directory_contents(&source_path, &destination_path, stats, false)?;
            make_directory_owner_accessible(&destination_path, &metadata)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(RunnerError::InvalidRequest(format!(
                "source entry `{}` is not a regular file or directory",
                source_path.display()
            )));
        }
        let copied = fs::copy(&source_path, &destination_path)
            .map_err(|error| RunnerError::fs("copy materialized file", &destination_path, error))?;
        make_file_owner_writable(&destination_path, &metadata)?;
        stats.file_count = stats.file_count.saturating_add(1);
        stats.total_bytes = stats.total_bytes.saturating_add(copied);
    }
    Ok(())
}

fn attempt_environment(
    attempt_root: &Path,
    workspace: &Path,
    requested: &BTreeMap<String, String>,
    external: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, RunnerError> {
    let runtime = attempt_root.join("runtime");
    let tmp = runtime.join("tmp");
    let cache = runtime.join("cache");
    let state = runtime.join("state");
    let data = runtime.join("data");
    let build = runtime.join("build");
    for path in [&tmp, &cache, &state, &data, &build] {
        fs::create_dir_all(path)
            .map_err(|error| RunnerError::fs("create attempt runtime directory", path, error))?;
    }

    let mut environment = requested.clone();
    environment.extend(external.clone());
    for (key, path) in [
        ("AIT_RUNNER_ATTEMPT_ROOT", attempt_root.to_path_buf()),
        ("AIT_RUNNER_WORKSPACE", workspace.to_path_buf()),
        ("TMPDIR", tmp.clone()),
        ("TMP", tmp.clone()),
        ("TEMP", tmp),
        ("XDG_CACHE_HOME", cache.clone()),
        ("XDG_STATE_HOME", state),
        ("XDG_DATA_HOME", data),
        ("CARGO_HOME", cache.join("cargo-home")),
        ("CARGO_TARGET_DIR", build.join("cargo-target")),
        ("CARGO_BUILD_BUILD_DIR", build.join("cargo-build")),
        ("PIP_CACHE_DIR", cache.join("pip")),
        ("npm_config_cache", cache.join("npm")),
        ("NPM_CONFIG_CACHE", cache.join("npm")),
        ("YARN_CACHE_FOLDER", cache.join("yarn")),
        ("PNPM_STORE_DIR", cache.join("pnpm")),
        ("GRADLE_USER_HOME", cache.join("gradle")),
        ("GOCACHE", cache.join("go-build")),
        ("GOMODCACHE", cache.join("go-mod")),
        ("GOPATH", runtime.join("go-path")),
        ("NUGET_PACKAGES", cache.join("nuget")),
        ("CCACHE_DIR", cache.join("ccache")),
        ("SCCACHE_DIR", cache.join("sccache")),
    ] {
        environment.insert(key.to_string(), path_string(&path, key)?);
    }
    environment.insert("PIP_NO_CACHE_DIR".to_string(), "1".to_string());
    Ok(environment)
}

fn path_string(path: &Path, field: &str) -> Result<String, RunnerError> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        RunnerError::InvalidRequest(format!(
            "attempt-owned path for environment field `{field}` is not valid UTF-8"
        ))
    })
}

#[cfg(unix)]
fn make_directory_owner_accessible(path: &Path, source: &fs::Metadata) -> Result<(), RunnerError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = (source.permissions().mode() & 0o777) | 0o700;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| RunnerError::fs("set materialized directory permissions", path, error))
}

#[cfg(not(unix))]
fn make_directory_owner_accessible(
    _path: &Path,
    _source: &fs::Metadata,
) -> Result<(), RunnerError> {
    Ok(())
}

#[cfg(unix)]
fn make_file_owner_writable(path: &Path, source: &fs::Metadata) -> Result<(), RunnerError> {
    use std::os::unix::fs::PermissionsExt;
    let mode = (source.permissions().mode() & 0o777) | 0o200;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| RunnerError::fs("set materialized file permissions", path, error))
}

#[cfg(not(unix))]
fn make_file_owner_writable(path: &Path, _source: &fs::Metadata) -> Result<(), RunnerError> {
    let mut permissions = fs::metadata(path)
        .map_err(|error| RunnerError::fs("inspect materialized file permissions", path, error))?
        .permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)
        .map_err(|error| RunnerError::fs("set materialized file permissions", path, error))
}

#[allow(clippy::too_many_arguments)]
fn execute_process(
    invocation: &ProcessInvocation,
    environment: &std::collections::BTreeMap<String, String>,
    working_directory: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
    timeout: Duration,
) -> Result<ProcessOutcome, RunnerError> {
    let mut command = Command::new(&invocation.program);
    command
        .args(&invocation.arguments)
        .current_dir(working_directory)
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &invocation.fixed_environment {
        command.env(key, value);
    }
    configure_process_group(&mut command);

    let started = Instant::now();
    let mut child = command.spawn().map_err(|error| {
        RunnerError::Process(format!("could not start repository CI entrypoint: {error}"))
    })?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or_else(|| {
        RunnerError::Process("could not capture repository CI stdout".to_string())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        RunnerError::Process("could not capture repository CI stderr".to_string())
    })?;
    let stdout_path = stdout_path.to_path_buf();
    let stderr_path = stderr_path.to_path_buf();
    let stdout_thread = thread::spawn(move || capture_stream(stdout, &stdout_path));
    let stderr_thread = thread::spawn(move || capture_stream(stderr, &stderr_path));

    let wait_result = wait_for_process(&mut child, pid, timeout);
    if wait_result.is_err() {
        terminate_process_group(pid);
        let _ = child.kill();
        let _ = child.wait();
    }
    terminate_descendants(pid);
    let stdout_result = join_capture(stdout_thread, "stdout");
    let stderr_result = join_capture(stderr_thread, "stderr");
    let (status, timed_out) = wait_result?;
    let stdout = stdout_result?;
    let stderr = stderr_result?;
    Ok(ProcessOutcome {
        status,
        timed_out,
        duration: started.elapsed(),
        stdout: stream_evidence(stdout),
        stderr: stream_evidence(stderr),
    })
}

fn wait_for_process(
    child: &mut Child,
    pid: u32,
    timeout: Duration,
) -> Result<(ExitStatus, bool), RunnerError> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|error| {
            RunnerError::Process(format!("could not poll repository CI entrypoint: {error}"))
        })? {
            return Ok((status, false));
        }
        if started.elapsed() >= timeout {
            terminate_process_group(pid);
            let status = child.wait().map_err(|error| {
                RunnerError::Process(format!(
                    "could not reap timed-out repository CI entrypoint: {error}"
                ))
            })?;
            return Ok((status, true));
        }
        thread::sleep(PROCESS_POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
    }
}

fn capture_stream<R: Read>(mut reader: R, log_path: &Path) -> Result<StreamCapture, RunnerError> {
    let mut log = File::create(log_path)
        .map_err(|error| RunnerError::fs("create process log", log_path, error))?;
    let mut digest = Sha256::new();
    let mut tail = VecDeque::with_capacity(STREAM_TAIL_BYTES);
    let mut byte_count = 0u64;
    let mut buffer = [0u8; 16 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| RunnerError::fs("read process stream", log_path, error))?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        log.write_all(chunk)
            .map_err(|error| RunnerError::fs("write process log", log_path, error))?;
        digest.update(chunk);
        byte_count = byte_count.saturating_add(read as u64);
        for byte in chunk {
            if tail.len() == STREAM_TAIL_BYTES {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
    log.sync_all()
        .map_err(|error| RunnerError::fs("flush process log", log_path, error))?;
    Ok(StreamCapture {
        byte_count,
        sha256: format!("{:x}", digest.finalize()),
        tail: tail.into_iter().collect(),
    })
}

fn join_capture(
    thread: thread::JoinHandle<Result<StreamCapture, RunnerError>>,
    stream: &str,
) -> Result<StreamCapture, RunnerError> {
    thread
        .join()
        .map_err(|_| RunnerError::Process(format!("{stream} capture thread panicked")))?
}

fn stream_evidence(capture: StreamCapture) -> StreamEvidence {
    StreamEvidence {
        byte_count: capture.byte_count,
        sha256: capture.sha256,
        tail_base64: BASE64_STANDARD.encode(&capture.tail),
        tail_byte_count: capture.tail.len(),
        truncated: capture.byte_count > capture.tail.len() as u64,
    }
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn configure_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_process_group(pid: u32) {
    let process_group = -(pid as i32);
    // SAFETY: kill only targets the process group created for this exact child.
    unsafe {
        libc::kill(process_group, libc::SIGTERM);
    }
    thread::sleep(PROCESS_GROUP_GRACE);
    // SAFETY: same bounded process-group target as above.
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn terminate_process_group(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_group(_pid: u32) {}

fn terminate_descendants(pid: u32) {
    terminate_process_group(pid);
}

#[cfg(unix)]
fn exit_signal(status: &ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: &ExitStatus) -> Option<i32> {
    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::protocol::{CommandSpec, NATIVE_JOB_CONTRACT, SourceSpec};

    fn make_source(script: &str) -> TempDir {
        let source = tempfile::tempdir().expect("source tempdir");
        let ci = source.path().join("ci");
        fs::create_dir(&ci).expect("ci directory");
        let run = ci.join("run.sh");
        fs::write(&run, script).expect("run.sh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).expect("chmod run.sh");
        }
        source
    }

    fn request(environment: BTreeMap<String, String>, timeout_ms: u64) -> NativeJobRequest {
        NativeJobRequest {
            contract: NATIVE_JOB_CONTRACT.to_string(),
            label: None,
            source: SourceSpec::local_directory("."),
            command: CommandSpec {
                argv: vec![
                    ait_core::server_operational::NATIVE_JOB_REPOSITORY_CI_ARGV0.to_string(),
                    "patchset".to_string(),
                ],
                working_directory: ".".to_string(),
                environment,
            },
            timeout_ms,
            suite_id: Some("test-suite".to_string()),
        }
    }

    fn executor(source: &TempDir, attempts: &TempDir) -> NativeExecutor {
        NativeExecutor::new(ExecutorConfig {
            source_root: source.path().to_path_buf(),
            attempt_root: attempts.path().to_path_buf(),
        })
    }

    #[test]
    fn serve_attempt_root_lease_reclaims_only_exact_runner_orphans() {
        let source = tempfile::tempdir().expect("source");
        let attempts = tempfile::tempdir().expect("attempts");
        let stale = attempts.path().join("attempt-123-456-7");
        fs::create_dir(&stale).expect("stale attempt");
        fs::write(stale.join("owned"), b"owned").expect("owned file");
        let unrelated = attempts.path().join("attempt-company-cache");
        fs::create_dir(&unrelated).expect("unrelated directory");
        fs::write(unrelated.join("keep"), b"keep").expect("unrelated file");

        let executor = NativeExecutor::new(ExecutorConfig {
            source_root: source.path().to_path_buf(),
            attempt_root: attempts.path().to_path_buf(),
        });
        let lease = executor.acquire_attempt_root_lease().expect("first lease");
        assert!(!stale.exists());
        assert!(unrelated.join("keep").is_file());
        assert!(executor.acquire_attempt_root_lease().is_err());
        drop(lease);
        executor
            .acquire_attempt_root_lease()
            .expect("lease after prior owner exits");
    }

    #[test]
    fn runner_attempt_name_requires_the_exact_generated_shape() {
        assert!(is_runner_attempt_name("attempt-123-456-7"));
        assert!(!is_runner_attempt_name("attempt-123-456"));
        assert!(!is_runner_attempt_name("attempt-123-456-7-extra"));
        assert!(!is_runner_attempt_name("attempt-company-cache"));
        assert!(!is_runner_attempt_name("attempt-12x-456-7"));
    }

    #[test]
    fn operating_system_mapping_is_exact_and_fail_closed() {
        assert_eq!(runner_platform("macos").unwrap(), RunnerPlatform::Unix);
        assert_eq!(runner_platform("linux").unwrap(), RunnerPlatform::Unix);
        assert_eq!(runner_platform("windows").unwrap(), RunnerPlatform::Windows);
        assert!(runner_platform("freebsd").is_err());
        assert_eq!(repository_ci_entrypoint(RunnerPlatform::Unix), "ci/run.sh");
        assert_eq!(
            repository_ci_entrypoint(RunnerPlatform::Windows),
            "ci/run.ps1"
        );
    }

    #[test]
    fn windows_invocation_uses_fixed_powershell_file_argv() {
        let system_root = if cfg!(windows) {
            PathBuf::from(r"C:\Windows")
        } else {
            PathBuf::from("/Windows")
        };
        let entrypoint = system_root.join("workspace").join("ci").join("run.ps1");
        let arguments = vec![
            "patchset".to_string(),
            "argument with spaces".to_string(),
            "$(must-not-execute)".to_string(),
        ];
        let invocation = repository_ci_invocation(
            RunnerPlatform::Windows,
            &entrypoint,
            &arguments,
            Some(&system_root),
        )
        .expect("Windows invocation");
        assert_eq!(invocation.program, windows_powershell_path(&system_root));
        assert_eq!(
            invocation.fixed_environment,
            vec![(
                OsString::from("SystemRoot"),
                system_root.as_os_str().to_os_string(),
            )]
        );
        assert_eq!(
            invocation.arguments,
            vec![
                OsString::from("-NoLogo"),
                OsString::from("-NoProfile"),
                OsString::from("-NonInteractive"),
                OsString::from("-ExecutionPolicy"),
                OsString::from("Bypass"),
                OsString::from("-File"),
                entrypoint.as_os_str().to_os_string(),
                OsString::from("patchset"),
                OsString::from("argument with spaces"),
                OsString::from("$(must-not-execute)"),
            ]
        );
    }

    fn assert_attempts_empty(attempts: &TempDir) {
        assert_eq!(
            fs::read_dir(attempts.path())
                .expect("attempt parent")
                .count(),
            0
        );
    }

    #[test]
    fn success_and_command_failure_are_bounded_and_clean() {
        let success =
            make_source("#!/bin/sh\nset -eu\nprintf 'hello-out'\nprintf 'hello-err' >&2\nexit 0\n");
        let attempts = tempfile::tempdir().expect("attempts");
        let result = executor(&success, &attempts)
            .execute(&request(BTreeMap::new(), 5_000))
            .expect("successful execution");
        assert_eq!(result.status, TerminalStatus::Succeeded);
        assert_eq!(result.tests_status, TestStatus::Pass);
        assert!(result.cleanup.attempt_root_removed);
        assert!(result.encoded_len().unwrap() <= crate::protocol::MAX_TERMINAL_RESULT_BYTES);
        assert_attempts_empty(&attempts);

        let failure = make_source("#!/bin/sh\nset -eu\nprintf 'failed' >&2\nexit 7\n");
        let result = executor(&failure, &attempts)
            .execute(&request(BTreeMap::new(), 5_000))
            .expect("command failure is a terminal result");
        assert_eq!(result.status, TerminalStatus::CommandFailed);
        assert_eq!(result.suite_results[0].execution.exit_code, Some(7));
        assert_attempts_empty(&attempts);
    }

    #[test]
    fn large_output_is_reduced_to_bounded_digest_and_tail() {
        let source = make_source(
            "#!/bin/sh\nset -eu\ni=0\nwhile [ \"$i\" -lt 20000 ]; do\n  printf 'abcdef'\n  i=$((i + 1))\ndone\n",
        );
        let attempts = tempfile::tempdir().expect("attempts");
        let result = executor(&source, &attempts)
            .execute(&request(BTreeMap::new(), 5_000))
            .expect("large output execution");
        let stdout = &result.suite_results[0].execution.stdout;
        assert_eq!(stdout.byte_count, 120_000);
        assert_eq!(stdout.tail_byte_count, STREAM_TAIL_BYTES);
        assert!(stdout.truncated);
        assert!(result.encoded_len().unwrap() <= crate::protocol::MAX_TERMINAL_RESULT_BYTES);
        assert_attempts_empty(&attempts);
    }

    #[test]
    fn timeout_kills_process_group_and_cleans_attempt() {
        let source = make_source(
            "#!/bin/sh\nset -eu\nsleep 30 &\nchild=$!\nprintf '%s' \"$child\" > \"$PID_FILE\"\nwait \"$child\"\n",
        );
        let attempts = tempfile::tempdir().expect("attempts");
        let pid_file = attempts
            .path()
            .parent()
            .unwrap()
            .join(format!("ait-runner-child-{}", std::process::id()));
        let mut environment = BTreeMap::new();
        environment.insert(
            "PID_FILE".to_string(),
            pid_file.to_string_lossy().to_string(),
        );
        let result = executor(&source, &attempts)
            .execute(&request(environment, 2_000))
            .expect("timeout result");
        assert_eq!(result.status, TerminalStatus::TimedOut);
        assert!(result.suite_results[0].execution.timed_out);
        assert_attempts_empty(&attempts);

        #[cfg(unix)]
        {
            let child_pid = fs::read_to_string(&pid_file)
                .expect("child pid")
                .parse::<i32>()
                .expect("numeric pid");
            let mut gone = false;
            for _ in 0..50 {
                // SAFETY: signal zero only probes the exact test child PID.
                let result = unsafe { libc::kill(child_pid, 0) };
                if result == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    gone = true;
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            assert!(
                gone,
                "descendant process {child_pid} survived timeout cleanup"
            );
        }
        let _ = fs::remove_file(pid_file);
    }

    #[cfg(unix)]
    #[test]
    fn source_symlink_is_rejected_and_attempt_is_cleaned() {
        use std::os::unix::fs::symlink;

        let source = make_source("#!/bin/sh\nexit 0\n");
        let outside = tempfile::tempdir().expect("outside");
        symlink(outside.path(), source.path().join("escape")).expect("symlink");
        let attempts = tempfile::tempdir().expect("attempts");
        let error = executor(&source, &attempts)
            .execute(&request(BTreeMap::new(), 5_000))
            .expect_err("symlink must fail closed");
        assert!(error.to_string().contains("symbolic link"));
        assert_attempts_empty(&attempts);
    }

    #[cfg(unix)]
    #[test]
    fn repository_ci_entrypoint_symlink_is_rejected_during_preflight() {
        use std::os::unix::fs::symlink;

        let source = make_source("#!/bin/sh\nexit 0\n");
        let outside = tempfile::NamedTempFile::new().expect("outside script");
        let run = source.path().join("ci/run.sh");
        fs::remove_file(&run).expect("remove regular entrypoint");
        symlink(outside.path(), &run).expect("entrypoint symlink");
        let attempts = tempfile::tempdir().expect("attempts");
        let error = executor(&source, &attempts)
            .preflight(&request(BTreeMap::new(), 5_000))
            .expect_err("entrypoint symlink must fail closed");
        assert!(error.to_string().contains("regular, non-symlink"));
        assert_attempts_empty(&attempts);
    }
}
