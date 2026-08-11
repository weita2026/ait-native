use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

pub const TELEGRAM_STT_EXECUTION_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramSttExecution.v1";
pub const TELEGRAM_STT_REQUEST_CONTRACT: &str = "ait.agent.telegram_stt_request.v1";
pub const TELEGRAM_STT_RESPONSE_CONTRACT: &str = "ait.agent.telegram_stt_response.v1";

const MIGRATION_STAGE: &str = "rust_agent_telegram_external_stt_execution";
const MAX_AUDIO_BYTES: u64 = 50 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_CONFIG_BYTES: usize = 16 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 512 * 1024;
const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_TIMEOUT: Duration = Duration::from_secs(3_600);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramSttExecutionErrorKind {
    InvalidRequest,
    Configuration,
    Unavailable,
    Io,
    Timeout,
    Exit,
    OutputLimit,
    Contract,
    Transcription,
    Empty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramSttExecutionError {
    kind: TelegramSttExecutionErrorKind,
}

impl TelegramSttExecutionError {
    pub fn new(kind: TelegramSttExecutionErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(self) -> TelegramSttExecutionErrorKind {
        self.kind
    }
}

impl fmt::Display for TelegramSttExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Telegram external STT execution failed.")
    }
}

impl std::error::Error for TelegramSttExecutionError {}

pub trait TelegramSttExecutor: Send + Sync + 'static {
    fn execute_stt(&self, request: &JsonValue) -> Result<JsonValue, TelegramSttExecutionError>;
}

pub struct ExternalProgramTelegramSttExecutor {
    program: PathBuf,
    timeout: Duration,
    execution_lock: Mutex<()>,
}

impl ExternalProgramTelegramSttExecutor {
    pub fn new(
        program: impl Into<PathBuf>,
        timeout: Duration,
    ) -> Result<Self, TelegramSttExecutionError> {
        let program = program.into();
        validate_program_configuration(&program, timeout)?;
        Ok(Self {
            program,
            timeout,
            execution_lock: Mutex::new(()),
        })
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl fmt::Debug for ExternalProgramTelegramSttExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExternalProgramTelegramSttExecutor")
            .field("program_configured", &true)
            .field("timeout_seconds", &self.timeout.as_secs_f64())
            .field("program_path_exposed", &false)
            .field("request_exposed", &false)
            .field("response_exposed", &false)
            .finish()
    }
}

impl TelegramSttExecutor for ExternalProgramTelegramSttExecutor {
    fn execute_stt(&self, request: &JsonValue) -> Result<JsonValue, TelegramSttExecutionError> {
        let validated = ValidatedRequest::parse(request)?;
        validate_program_file(&self.program)?;
        let _execution = lock_unpoisoned(&self.execution_lock);
        let wire_request = validated.wire_request();
        let input = wire_request.to_string().into_bytes();
        if input.len() > MAX_REQUEST_BYTES {
            return Err(error(TelegramSttExecutionErrorKind::InvalidRequest));
        }

        let mut command = Command::new(&self.program);
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|_| error(TelegramSttExecutionErrorKind::Unavailable))?;
        let Some(mut stdin) = child.stdin.take() else {
            terminate_child(&mut child);
            return Err(error(TelegramSttExecutionErrorKind::Io));
        };
        let Some(stdout) = child.stdout.take() else {
            terminate_child(&mut child);
            return Err(error(TelegramSttExecutionErrorKind::Io));
        };
        let Some(stderr) = child.stderr.take() else {
            terminate_child(&mut child);
            return Err(error(TelegramSttExecutionErrorKind::Io));
        };
        let stdin_writer = thread::spawn(move || {
            stdin.write_all(&input)?;
            drop(stdin);
            Ok::<(), io::Error>(())
        });
        let stdout_reader = spawn_bounded_reader(stdout, MAX_OUTPUT_BYTES);
        let stderr_reader = spawn_bounded_reader(stderr, MAX_STDERR_BYTES);

        let deadline = Instant::now() + self.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    terminate_child(&mut child);
                    join_process_threads(stdin_writer, stdout_reader, stderr_reader);
                    return Err(error(TelegramSttExecutionErrorKind::Timeout));
                }
                Err(_) => {
                    terminate_child(&mut child);
                    join_process_threads(stdin_writer, stdout_reader, stderr_reader);
                    return Err(error(TelegramSttExecutionErrorKind::Io));
                }
            }
        };
        let stdin_result = join_writer(stdin_writer)?;
        let (stdout, stdout_exceeded) = join_reader(stdout_reader)?;
        let _ = join_reader(stderr_reader)?;
        if stdin_result.is_err() {
            return Err(error(TelegramSttExecutionErrorKind::Io));
        }
        if !status.success() {
            return Err(error(TelegramSttExecutionErrorKind::Exit));
        }
        if stdout_exceeded {
            return Err(error(TelegramSttExecutionErrorKind::OutputLimit));
        }
        let stdout = String::from_utf8(stdout)
            .map_err(|_| error(TelegramSttExecutionErrorKind::Contract))?;
        let response = stdout
            .trim()
            .parse::<JsonValue>()
            .map_err(|_| error(TelegramSttExecutionErrorKind::Contract))?;
        normalize_response(&response)
    }
}

struct ValidatedRequest {
    local_path: PathBuf,
    model: String,
    device: String,
    compute_type: Option<String>,
    language: Option<String>,
}

impl ValidatedRequest {
    fn parse(request: &JsonValue) -> Result<Self, TelegramSttExecutionError> {
        let object = request
            .as_object()
            .ok_or_else(|| error(TelegramSttExecutionErrorKind::InvalidRequest))?;
        let allowed = [
            "operation",
            "local_path",
            "model",
            "device",
            "compute_type",
            "language",
        ];
        if object.keys().any(|key| !allowed.contains(&key.as_str()))
            || text(object, "operation") != Some("transcribe")
        {
            return Err(error(TelegramSttExecutionErrorKind::InvalidRequest));
        }
        let local_path_text = required_text(object, "local_path", MAX_PATH_BYTES)?;
        let local_path = PathBuf::from(&local_path_text);
        if !local_path.is_absolute()
            || local_path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(error(TelegramSttExecutionErrorKind::InvalidRequest));
        }
        let metadata = fs::symlink_metadata(&local_path)
            .map_err(|_| error(TelegramSttExecutionErrorKind::InvalidRequest))?;
        if metadata.file_type().is_symlink()
            || !metadata.file_type().is_file()
            || !(1..=MAX_AUDIO_BYTES).contains(&metadata.len())
        {
            return Err(error(TelegramSttExecutionErrorKind::InvalidRequest));
        }
        Ok(Self {
            local_path,
            model: required_text(object, "model", MAX_CONFIG_BYTES)?,
            device: required_text(object, "device", MAX_CONFIG_BYTES)?,
            compute_type: optional_text(object.get("compute_type"), MAX_CONFIG_BYTES)?,
            language: optional_text(object.get("language"), MAX_CONFIG_BYTES)?,
        })
    }

    fn wire_request(&self) -> JsonValue {
        json!({
            "contract": TELEGRAM_STT_REQUEST_CONTRACT,
            "operation": "transcribe",
            "audio_path": self.local_path.to_string_lossy(),
            "model": self.model,
            "device": self.device,
            "compute_type": self.compute_type,
            "language": self.language,
            "python_runtime_allowed": false,
        })
    }
}

fn normalize_response(response: &JsonValue) -> Result<JsonValue, TelegramSttExecutionError> {
    let object = response
        .as_object()
        .ok_or_else(|| error(TelegramSttExecutionErrorKind::Contract))?;
    let allowed = ["contract", "ok", "transcript", "language", "error_kind"];
    if object.keys().any(|key| !allowed.contains(&key.as_str()))
        || text(object, "contract") != Some(TELEGRAM_STT_RESPONSE_CONTRACT)
    {
        return Err(error(TelegramSttExecutionErrorKind::Contract));
    }
    let ok = object
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(TelegramSttExecutionErrorKind::Contract))?;
    if !ok {
        return Err(error(TelegramSttExecutionErrorKind::Transcription));
    }
    let transcript = object
        .get("transcript")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .ok_or_else(|| error(TelegramSttExecutionErrorKind::Contract))?;
    if transcript.is_empty() {
        return Err(error(TelegramSttExecutionErrorKind::Empty));
    }
    if transcript.len() > MAX_TRANSCRIPT_BYTES || transcript.contains('\0') {
        return Err(error(TelegramSttExecutionErrorKind::Contract));
    }
    Ok(json!({
        "contract": TELEGRAM_STT_EXECUTION_CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "stt_state": "completed",
        "ok": true,
        "completed": true,
        "transcript": transcript,
        "language": optional_text(object.get("language"), MAX_CONFIG_BYTES)?,
        "python_stt_allowed": false,
        "python_runtime_allowed": false,
        "shell_execution_allowed": false,
        "inherited_environment_allowed": false,
        "request_payload_exposed": false,
        "response_payload_exposed": false,
        "audio_path_exposed": false,
        "program_path_exposed": false,
        "stderr_exposed": false,
        "downstream_error_exposed": false,
    }))
}

fn validate_program_configuration(
    program: &Path,
    timeout: Duration,
) -> Result<(), TelegramSttExecutionError> {
    let text = program.to_string_lossy();
    let file_name = program
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if !program.is_absolute()
        || text.is_empty()
        || text.len() > MAX_PATH_BYTES
        || text.chars().any(char::is_control)
        || timeout < Duration::from_millis(10)
        || timeout > MAX_TIMEOUT
        || program_is_forbidden(&file_name)
    {
        return Err(error(TelegramSttExecutionErrorKind::Configuration));
    }
    Ok(())
}

fn validate_program_file(program: &Path) -> Result<(), TelegramSttExecutionError> {
    let metadata = fs::symlink_metadata(program)
        .map_err(|_| error(TelegramSttExecutionErrorKind::Unavailable))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(error(TelegramSttExecutionErrorKind::Unavailable));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(error(TelegramSttExecutionErrorKind::Unavailable));
        }
    }
    Ok(())
}

fn program_is_forbidden(file_name: &str) -> bool {
    file_name.starts_with("python")
        || file_name.ends_with(".py")
        || matches!(
            file_name,
            "sh" | "bash"
                | "zsh"
                | "dash"
                | "fish"
                | "pwsh"
                | "powershell"
                | "cmd"
                | "cmd.exe"
                | "env"
                | "nohup"
        )
}

type ReaderResult = io::Result<(Vec<u8>, bool)>;

fn spawn_bounded_reader<R>(mut reader: R, limit: usize) -> JoinHandle<ReaderResult>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut captured = Vec::with_capacity(limit.min(64 * 1024));
        let mut exceeded = false;
        let mut chunk = [0_u8; 8_192];
        loop {
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            let remaining = limit.saturating_sub(captured.len());
            let retained = read.min(remaining);
            captured.extend_from_slice(&chunk[..retained]);
            exceeded |= retained < read;
        }
        Ok((captured, exceeded))
    })
}

fn join_writer(
    handle: JoinHandle<io::Result<()>>,
) -> Result<io::Result<()>, TelegramSttExecutionError> {
    handle
        .join()
        .map_err(|_| error(TelegramSttExecutionErrorKind::Io))
}

fn join_reader(
    handle: JoinHandle<ReaderResult>,
) -> Result<(Vec<u8>, bool), TelegramSttExecutionError> {
    handle
        .join()
        .map_err(|_| error(TelegramSttExecutionErrorKind::Io))?
        .map_err(|_| error(TelegramSttExecutionErrorKind::Io))
}

fn join_process_threads(
    stdin_writer: JoinHandle<io::Result<()>>,
    stdout_reader: JoinHandle<ReaderResult>,
    stderr_reader: JoinHandle<ReaderResult>,
) {
    let _ = stdin_writer.join();
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();
}

fn terminate_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn required_text(
    object: &Map<String, JsonValue>,
    key: &str,
    max_bytes: usize,
) -> Result<String, TelegramSttExecutionError> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= max_bytes
                && !value.contains('\0')
                && !value.contains('\r')
        })
        .map(str::to_string)
        .ok_or_else(|| error(TelegramSttExecutionErrorKind::InvalidRequest))
}

fn optional_text(
    value: Option<&JsonValue>,
    max_bytes: usize,
) -> Result<Option<String>, TelegramSttExecutionError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => {
            let value = value.trim();
            if value.is_empty()
                || value.len() > max_bytes
                || value.contains('\0')
                || value.contains('\r')
            {
                return Err(error(TelegramSttExecutionErrorKind::Contract));
            }
            Ok(Some(value.to_string()))
        }
        _ => Err(error(TelegramSttExecutionErrorKind::Contract)),
    }
}

fn text<'a>(object: &'a Map<String, JsonValue>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(JsonValue::as_str)
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn error(kind: TelegramSttExecutionErrorKind) -> TelegramSttExecutionError {
    TelegramSttExecutionError { kind }
}

#[cfg(test)]
#[path = "telegram_stt_execution/tests.rs"]
mod tests;
