use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ait_agent_core::{
    AGENT_GATEWAY_CODEX_THREAD_BINDING_CONTRACT, AGENT_GATEWAY_REPLY_PROVIDER_REQUEST_CONTRACT,
    AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT,
};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use fs2::FileExt;
use ring::digest::{digest, SHA256};

mod turn_telemetry;

use turn_telemetry::{telemetry_from_jsonl, TurnTelemetryCollector};

const DEFAULT_CODEX_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_PROVIDER_REQUEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_CODEX_STDOUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_CODEX_STDERR_BYTES: usize = 64 * 1024;
const MAX_PROMPT_BYTES: usize = 2 * 1024 * 1024;
const MAX_THREAD_BINDING_BYTES: u64 = 64 * 1024;
const MAX_THREAD_ID_BYTES: usize = 256;
const MAX_CODEX_LOCK_WAIT: Duration = Duration::from_secs(30);
const CODEX_MISSING_ROLLOUT_MARKER: &str = "no rollout found for thread id";

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodexReplySettings {
    program: String,
    model: Option<String>,
    reasoning_effort: Option<String>,
    sandbox: String,
    timeout: Option<Duration>,
}

#[derive(Debug)]
struct CodexReplyOutput {
    text: String,
    thread_id: String,
    usage: JsonValue,
    thread_mode: &'static str,
    prompt_bytes: usize,
    context_event_count: usize,
    provider_thread: JsonValue,
    turn_telemetry: JsonValue,
}

#[derive(Debug)]
struct ParsedCodexReply {
    text: String,
    thread_id: Option<String>,
    usage: JsonValue,
    turn_telemetry: JsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodexThreadBinding {
    thread_id: String,
    conversation_key: String,
    surface: String,
    model: String,
    reasoning_effort: Option<String>,
    sandbox: String,
    repository_fingerprint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CodexInvocationMode<'a> {
    Start,
    Resume { thread_id: &'a str },
}

#[derive(Debug)]
struct CodexPrompt {
    text: String,
    context_event_count: usize,
}

#[derive(Debug)]
struct CodexProcessOutput {
    success: bool,
    stdout: String,
    stderr: String,
    stdout_exceeded: bool,
}

struct CodexThreadLock {
    file: File,
    binding_path: PathBuf,
}

/// Installs the current Rust worker executable as the reply-provider process.
/// A complete explicit provider configuration always wins over this default.
pub fn configure_native_reply_provider(_surface: &str) -> Result<(), String> {
    let executable = env::current_exe()
        .map_err(|_| "The native reply provider executable could not be resolved.".to_string())?;
    ait_agent_core::configure_agent_local_reply_process_defaults(
        executable.to_string_lossy().into_owned(),
        vec!["reply-provider".to_string()],
    )
}

/// Executes one versioned, conversation-keyed gateway provider request.
pub fn execute_native_reply_provider(
    raw_request: &str,
    process_env: &BTreeMap<String, String>,
) -> JsonValue {
    if raw_request.len() > MAX_PROVIDER_REQUEST_BYTES {
        return provider_failure(
            AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT,
            "provider_request_too_large",
            "The native reply provider request exceeded its bounded input size.",
            false,
            None,
        );
    }
    let request = match raw_request.trim().parse::<JsonValue>() {
        Ok(value) => value,
        Err(_) => {
            return provider_failure(
                AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT,
                "provider_request_invalid",
                "The native reply provider request was not valid JSON.",
                false,
                None,
            )
        }
    };
    if request.get("contract").and_then(JsonValue::as_str)
        != Some(AGENT_GATEWAY_REPLY_PROVIDER_REQUEST_CONTRACT)
    {
        return provider_failure(
            AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT,
            "provider_request_contract",
            "The native reply provider request used an unsupported contract.",
            false,
            None,
        );
    }
    let settings = match CodexReplySettings::from_request(&request) {
        Ok(value) => value,
        Err(message) => {
            return provider_failure(
                AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT,
                "provider_config",
                &message,
                false,
                None,
            )
        }
    };
    let surface = request_surface(&request);
    match execute_codex(&request, &settings, process_env) {
        Ok(output) => {
            let command_count = output.turn_telemetry["command_count"].clone();
            let distinct_command_count = output.turn_telemetry["distinct_command_count"].clone();
            let ait_command_count = output.turn_telemetry["ait_command_count"].clone();
            let distinct_ait_command_count =
                output.turn_telemetry["distinct_ait_command_count"].clone();
            let turn_telemetry = output.turn_telemetry;
            let turn_analysis = json!({
                "provider": "codex_exec",
                "surface": surface,
                "sandbox": settings.sandbox,
                "thread_mode": output.thread_mode,
                "prompt_bytes": output.prompt_bytes,
                "context_event_count": output.context_event_count,
                "command_count": command_count,
                "distinct_command_count": distinct_command_count,
                "ait_command_count": ait_command_count,
                "distinct_ait_command_count": distinct_ait_command_count,
                "provider_thread": output.provider_thread,
                "turn_telemetry": turn_telemetry.clone(),
            });
            json!({
                "contract": AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT,
                "ok": true,
                "reply": {
                    "text": output.text,
                    "model": settings.model.as_deref().unwrap_or("codex-default"),
                    "source": "codex_exec",
                    "response_id": output.thread_id,
                    "usage": output.usage,
                    "turn_analysis": turn_analysis,
                    "turn_telemetry": turn_telemetry,
                    "attachments": [],
                },
            })
        }
        Err(failure) => provider_failure(
            AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT,
            &failure.kind,
            &failure.message,
            failure.retryable,
            failure.turn_telemetry,
        ),
    }
}

impl CodexReplySettings {
    fn from_request(request: &JsonValue) -> Result<Self, String> {
        let settings = match request.get("settings") {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(value.as_object().ok_or_else(|| {
                "The native reply provider settings must be an object.".to_string()
            })?),
        };
        let program =
            optional_setting_text(settings, "codex_program")?.unwrap_or_else(default_codex_program);
        validate_codex_program(&program)?;

        let model = optional_setting_text(settings, "model")?;
        if model.as_deref().is_some_and(invalid_option_value) {
            return Err("The configured Codex model is invalid.".to_string());
        }

        let reasoning_effort = optional_setting_text(settings, "reasoning_effort")?
            .map(|value| value.to_ascii_lowercase());
        if reasoning_effort.as_deref().is_some_and(|value| {
            !matches!(
                value,
                "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
            )
        }) {
            return Err("The configured Codex reasoning effort is invalid.".to_string());
        }

        let sandbox = optional_setting_text(settings, "sandbox")?
            .unwrap_or_else(|| "workspace-write".to_string())
            .to_ascii_lowercase();
        if !matches!(
            sandbox.as_str(),
            "read-only" | "workspace-write" | "danger-full-access"
        ) {
            return Err("The configured Codex sandbox is invalid.".to_string());
        }

        let timeout = match settings.and_then(|settings| settings.get("turn_timeout_seconds")) {
            None | Some(JsonValue::Null) => Some(DEFAULT_CODEX_TIMEOUT),
            Some(JsonValue::String(value)) => parse_timeout(value)?,
            Some(value) => value
                .as_f64()
                .filter(|value| value.is_finite() && *value > 0.0)
                .map(Duration::from_secs_f64)
                .map(Some)
                .ok_or_else(|| "The configured Codex turn timeout is invalid.".to_string())?,
        };

        Ok(Self {
            program,
            model,
            reasoning_effort,
            sandbox,
            timeout,
        })
    }
}

fn optional_setting_text(
    settings: Option<&Map<String, JsonValue>>,
    field: &str,
) -> Result<Option<String>, String> {
    match settings.and_then(|settings| settings.get(field)) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Err(format!("The configured reply provider {field} is empty."))
            } else {
                Ok(Some(value.to_string()))
            }
        }
        Some(_) => Err(format!(
            "The configured reply provider {field} must be a string."
        )),
    }
}

fn execute_codex(
    request: &JsonValue,
    settings: &CodexReplySettings,
    process_env: &BTreeMap<String, String>,
) -> Result<CodexReplyOutput, ProviderFailure> {
    let repo_root = request
        .get("repository")
        .and_then(|value| value.get("repo_root"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            ProviderFailure::configuration("The provider repository root is missing.")
        })?;
    let repo_root = fs::canonicalize(repo_root).map_err(|_| {
        ProviderFailure::configuration("The provider repository root is unavailable.")
    })?;
    if !repo_root.is_dir() {
        return Err(ProviderFailure::configuration(
            "The provider repository root is not a directory.",
        ));
    }
    let conversation_key = request_conversation_key(request)?;
    let surface = request_surface(request);
    let repository_fingerprint = repository_fingerprint(&repo_root);
    let thread_lock = CodexThreadLock::acquire(
        &repo_root,
        &repository_fingerprint,
        &conversation_key,
        &surface,
        settings.timeout,
    )?;
    let binding = compatible_thread_binding(
        request,
        settings,
        &conversation_key,
        &surface,
        &repository_fingerprint,
    )
    .or_else(|| {
        thread_lock.load_binding(
            settings,
            &conversation_key,
            &surface,
            &repository_fingerprint,
        )
    });

    let output = if let Some(binding) = binding {
        let prompt = build_resume_prompt(request)?;
        let process = run_codex_process(
            settings,
            &repo_root,
            process_env,
            CodexInvocationMode::Resume {
                thread_id: &binding.thread_id,
            },
            &prompt.text,
        )?;
        if process.success {
            completed_codex_reply(
                parse_successful_codex_process(&process)?,
                request,
                settings,
                &repository_fingerprint,
                "resumed",
                &prompt,
                Some(&binding.thread_id),
            )?
        } else if !process.stdout_exceeded
            && resume_rejected_before_turn(&process.stdout, &process.stderr)
        {
            start_codex_thread(
                request,
                settings,
                process_env,
                &repo_root,
                &repository_fingerprint,
                "recovered",
            )?
        } else {
            return Err(ProviderFailure {
                kind: "provider_exit".to_string(),
                message: "The resumed Codex reply process failed after an ambiguous turn boundary; it was not replayed automatically.".to_string(),
                retryable: false,
                turn_telemetry: telemetry_from_jsonl(&process.stdout),
            });
        }
    } else {
        start_codex_thread(
            request,
            settings,
            process_env,
            &repo_root,
            &repository_fingerprint,
            "started",
        )?
    };
    thread_lock.store_binding(&output.provider_thread)?;
    Ok(output)
}

fn start_codex_thread(
    request: &JsonValue,
    settings: &CodexReplySettings,
    process_env: &BTreeMap<String, String>,
    repo_root: &Path,
    repository_fingerprint: &str,
    thread_mode: &'static str,
) -> Result<CodexReplyOutput, ProviderFailure> {
    let prompt = build_bootstrap_prompt(request)?;
    let process = run_codex_process(
        settings,
        repo_root,
        process_env,
        CodexInvocationMode::Start,
        &prompt.text,
    )?;
    if !process.success {
        return Err(ProviderFailure {
            kind: "provider_exit".to_string(),
            message: "The native Codex reply process exited unsuccessfully.".to_string(),
            retryable: false,
            turn_telemetry: telemetry_from_jsonl(&process.stdout),
        });
    }
    completed_codex_reply(
        parse_successful_codex_process(&process)?,
        request,
        settings,
        repository_fingerprint,
        thread_mode,
        &prompt,
        None,
    )
}

fn completed_codex_reply(
    parsed: ParsedCodexReply,
    request: &JsonValue,
    settings: &CodexReplySettings,
    repository_fingerprint: &str,
    thread_mode: &'static str,
    prompt: &CodexPrompt,
    expected_thread_id: Option<&str>,
) -> Result<CodexReplyOutput, ProviderFailure> {
    let thread_id = match (parsed.thread_id, expected_thread_id) {
        (Some(actual), Some(expected)) if actual != expected => {
            return Err(ProviderFailure::contract(
                "The resumed Codex reply returned a different thread identifier.",
            ))
        }
        (Some(actual), _) => actual,
        (None, Some(expected)) => expected.to_string(),
        (None, None) => {
            return Err(ProviderFailure::contract(
                "The native Codex reply stream omitted its persistent thread identifier.",
            ))
        }
    };
    if !valid_thread_id(&thread_id) {
        return Err(ProviderFailure::contract(
            "The native Codex reply returned an invalid thread identifier.",
        ));
    }
    let binding = CodexThreadBinding {
        thread_id: thread_id.clone(),
        conversation_key: request_conversation_key(request)?,
        surface: request_surface(request),
        model: configured_model(settings).to_string(),
        reasoning_effort: settings.reasoning_effort.clone(),
        sandbox: settings.sandbox.clone(),
        repository_fingerprint: repository_fingerprint.to_string(),
    };
    Ok(CodexReplyOutput {
        text: parsed.text,
        thread_id,
        usage: parsed.usage,
        thread_mode,
        prompt_bytes: prompt.text.len(),
        context_event_count: prompt.context_event_count,
        provider_thread: binding.to_json(),
        turn_telemetry: parsed.turn_telemetry,
    })
}

fn parse_successful_codex_process(
    process: &CodexProcessOutput,
) -> Result<ParsedCodexReply, ProviderFailure> {
    if process.stdout_exceeded {
        return Err(ProviderFailure::contract(
            "The native Codex reply output exceeded 4 MiB.",
        ));
    }
    parse_codex_jsonl(&process.stdout)
}

fn configure_codex_command(
    settings: &CodexReplySettings,
    repo_root: &Path,
    process_env: &BTreeMap<String, String>,
    mode: CodexInvocationMode<'_>,
) -> Command {
    let mut command = Command::new(&settings.program);
    command.arg("exec");
    match mode {
        CodexInvocationMode::Start => {
            command
                .arg("--json")
                .arg("--color")
                .arg("never")
                .arg("--skip-git-repo-check")
                .arg("-C")
                .arg(repo_root)
                .arg("--sandbox")
                .arg(&settings.sandbox);
        }
        CodexInvocationMode::Resume { .. } => {
            command
                .arg("resume")
                .arg("--json")
                .arg("--skip-git-repo-check");
        }
    }
    if let Some(model) = &settings.model {
        command.arg("--model").arg(model);
    }
    if let Some(reasoning_effort) = &settings.reasoning_effort {
        command
            .arg("--config")
            .arg(format!("model_reasoning_effort=\"{reasoning_effort}\""));
    }
    if let CodexInvocationMode::Resume { thread_id } = mode {
        command.arg(thread_id);
    }
    command
        .arg("-")
        .current_dir(repo_root)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    remove_transport_credentials(&mut command, process_env);
    command
}

fn run_codex_process(
    settings: &CodexReplySettings,
    repo_root: &Path,
    process_env: &BTreeMap<String, String>,
    mode: CodexInvocationMode<'_>,
    prompt: &str,
) -> Result<CodexProcessOutput, ProviderFailure> {
    let mut command = configure_codex_command(settings, repo_root, process_env, mode);
    let mut child = command.spawn().map_err(|_| ProviderFailure {
        kind: "provider_spawn".to_string(),
        message: "The native Codex reply process could not be started.".to_string(),
        retryable: false,
        turn_telemetry: None,
    })?;
    let mut stdin = child.stdin.take().ok_or_else(|| {
        terminate(&mut child);
        ProviderFailure::io("The native Codex reply stdin was unavailable.")
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        terminate(&mut child);
        ProviderFailure::io("The native Codex reply stdout was unavailable.")
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        terminate(&mut child);
        ProviderFailure::io("The native Codex reply stderr was unavailable.")
    })?;
    let prompt = prompt.to_string();
    let writer = thread::spawn(move || {
        stdin.write_all(prompt.as_bytes())?;
        drop(stdin);
        Ok::<(), io::Error>(())
    });
    let stdout_reader = bounded_reader(stdout, MAX_CODEX_STDOUT_BYTES);
    let stderr_reader = bounded_reader(stderr, MAX_CODEX_STDERR_BYTES);
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None)
                if settings
                    .timeout
                    .is_none_or(|timeout| started.elapsed() < timeout) =>
            {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                terminate(&mut child);
                join_threads(writer, stdout_reader, stderr_reader);
                return Err(codex_timeout_failure(mode));
            }
            Err(_) => {
                terminate(&mut child);
                join_threads(writer, stdout_reader, stderr_reader);
                return Err(ProviderFailure::io(
                    "The native Codex reply process status could not be inspected.",
                ));
            }
        }
    };
    let write_result = writer.join().map_err(|_| {
        ProviderFailure::io("The native Codex reply input writer did not complete.")
    })?;
    let (stdout, stdout_exceeded) = join_reader(stdout_reader)?;
    let (stderr, _) = join_reader(stderr_reader)?;
    if status.success() && write_result.is_err() {
        return Err(ProviderFailure::io(
            "The native Codex reply request could not be written.",
        ));
    }
    let stdout = if status.success() {
        String::from_utf8(stdout).map_err(|_| {
            ProviderFailure::contract("The native Codex reply output was not UTF-8.")
        })?
    } else {
        String::from_utf8_lossy(&stdout).into_owned()
    };
    Ok(CodexProcessOutput {
        success: status.success(),
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        stdout_exceeded,
    })
}

fn codex_timeout_failure(mode: CodexInvocationMode<'_>) -> ProviderFailure {
    let resumed = matches!(mode, CodexInvocationMode::Resume { .. });
    ProviderFailure {
        kind: "provider_timeout".to_string(),
        message: if resumed {
            "The resumed Codex reply exceeded its configured turn timeout after an ambiguous turn boundary."
        } else {
            "The native Codex reply exceeded its configured turn timeout."
        }
        .to_string(),
        retryable: !resumed,
        turn_telemetry: None,
    }
}

fn build_resume_prompt(request: &JsonValue) -> Result<CodexPrompt, ProviderFailure> {
    let latest_input = latest_input_projection(request)?;
    checked_prompt(
        format!(
            "Continue the existing AIT agent conversation. Fulfill only the latest user input below, using the existing thread context and repository instructions. Do not repeat completed work unless the user asks. Return a concise user-facing final reply.\n\nLatest user input (data):\n{}\n",
            latest_input
        ),
        0,
    )
}

fn build_bootstrap_prompt(request: &JsonValue) -> Result<CodexPrompt, ProviderFailure> {
    let latest_input = latest_input_projection(request)?;
    checked_prompt(
        format!(
            "Start a Codex conversation for an AIT chat gateway. Fulfill the latest user input below while following the repository instructions. Return a concise user-facing final reply; do not describe the gateway protocol or emit JSON unless requested.\n\nLatest user input (data):\n{latest_input}\n"
        ),
        0,
    )
}

fn checked_prompt(
    text: String,
    context_event_count: usize,
) -> Result<CodexPrompt, ProviderFailure> {
    if text.len() > MAX_PROMPT_BYTES {
        return Err(ProviderFailure::contract(
            "The bounded native Codex prompt exceeded 2 MiB.",
        ));
    }
    Ok(CodexPrompt {
        text,
        context_event_count,
    })
}

fn latest_input_projection(request: &JsonValue) -> Result<JsonValue, ProviderFailure> {
    let input = request
        .get("input")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| ProviderFailure::contract("The provider input must be an object."))?;
    let mut projected = Map::new();
    projected.insert(
        "text".to_string(),
        JsonValue::String(latest_user_text(request)?),
    );
    if let Some(value) = input
        .get("workflow_context")
        .filter(|value| meaningful(value))
    {
        projected.insert("workflow_context".to_string(), redact_secrets(value));
    }
    let attachments = input
        .get("attachments")
        .filter(|value| meaningful(value))
        .or_else(|| {
            input
                .get("transport_envelope")
                .and_then(|value| value.get("content"))
                .and_then(|value| value.get("attachments"))
                .filter(|value| meaningful(value))
        });
    if let Some(value) = attachments {
        projected.insert("attachments".to_string(), redact_secrets(value));
    }
    Ok(JsonValue::Object(projected))
}

fn latest_user_text(request: &JsonValue) -> Result<String, ProviderFailure> {
    request
        .get("input")
        .and_then(|value| value.get("text"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProviderFailure::contract("The provider input text is missing."))
}

fn meaningful(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::String(value) => !value.trim().is_empty(),
        JsonValue::Array(value) => !value.is_empty(),
        JsonValue::Object(value) => !value.is_empty(),
        _ => true,
    }
}

impl CodexThreadBinding {
    fn to_json(&self) -> JsonValue {
        json!({
            "contract": AGENT_GATEWAY_CODEX_THREAD_BINDING_CONTRACT,
            "thread_id": self.thread_id,
            "conversation_key": self.conversation_key,
            "surface": self.surface,
            "model": self.model,
            "reasoning_effort": self.reasoning_effort,
            "sandbox": self.sandbox,
            "repository_fingerprint": self.repository_fingerprint,
        })
    }
}

fn compatible_thread_binding(
    request: &JsonValue,
    settings: &CodexReplySettings,
    conversation_key: &str,
    surface: &str,
    repository_fingerprint: &str,
) -> Option<CodexThreadBinding> {
    compatible_thread_binding_value(
        request.get("provider_thread")?,
        settings,
        conversation_key,
        surface,
        repository_fingerprint,
    )
}

fn compatible_thread_binding_value(
    value: &JsonValue,
    settings: &CodexReplySettings,
    conversation_key: &str,
    surface: &str,
    repository_fingerprint: &str,
) -> Option<CodexThreadBinding> {
    let value = value.as_object()?;
    if value.get("contract").and_then(JsonValue::as_str)
        != Some(AGENT_GATEWAY_CODEX_THREAD_BINDING_CONTRACT)
    {
        return None;
    }
    let thread_id = binding_text(value.get("thread_id"))?;
    if !valid_thread_id(&thread_id) {
        return None;
    }
    let reasoning_effort = match value.get("reasoning_effort") {
        None | Some(JsonValue::Null) => None,
        Some(value) => Some(binding_text(Some(value))?.to_ascii_lowercase()),
    };
    let binding = CodexThreadBinding {
        thread_id,
        conversation_key: binding_text(value.get("conversation_key"))?,
        surface: binding_text(value.get("surface"))?.to_ascii_lowercase(),
        model: binding_text(value.get("model"))?,
        reasoning_effort,
        sandbox: binding_text(value.get("sandbox"))?.to_ascii_lowercase(),
        repository_fingerprint: binding_text(value.get("repository_fingerprint"))?,
    };
    (binding.conversation_key == conversation_key
        && binding.surface == surface
        && binding.model == configured_model(settings)
        && binding.reasoning_effort == settings.reasoning_effort
        && binding.sandbox == settings.sandbox
        && binding.repository_fingerprint == repository_fingerprint)
        .then_some(binding)
}

fn binding_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .filter(|value| !value.chars().any(char::is_control))
        .map(str::to_string)
}

fn configured_model(settings: &CodexReplySettings) -> &str {
    settings.model.as_deref().unwrap_or("codex-default")
}

fn request_conversation_key(request: &JsonValue) -> Result<String, ProviderFailure> {
    request
        .get("conversation")
        .and_then(|value| value.get("key"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 4096)
        .filter(|value| !value.chars().any(char::is_control))
        .map(str::to_string)
        .ok_or_else(|| ProviderFailure::configuration("The provider conversation key is missing."))
}

fn valid_thread_id(value: &str) -> bool {
    (8..=MAX_THREAD_ID_BYTES).contains(&value.len())
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn repository_fingerprint(repo_root: &Path) -> String {
    format!(
        "sha256:{}",
        sha256_hex(repo_root.to_string_lossy().as_bytes())
    )
}

fn sha256_hex(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = digest(&SHA256, value);
    let mut encoded = String::with_capacity(digest.as_ref().len() * 2);
    for byte in digest.as_ref() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

impl CodexThreadLock {
    fn acquire(
        repo_root: &Path,
        repository_fingerprint: &str,
        conversation_key: &str,
        surface: &str,
        configured_timeout: Option<Duration>,
    ) -> Result<Self, ProviderFailure> {
        let ait_root = repo_root.join(".ait");
        let lock_root = if ait_root.is_dir() {
            ait_root.join("agent-runtime").join("codex-thread-locks")
        } else {
            env::temp_dir().join("ait-agent").join("codex-thread-locks")
        };
        fs::create_dir_all(&lock_root).map_err(|_| {
            ProviderFailure::io("The Codex thread lock directory could not be created.")
        })?;
        let lock_key = sha256_hex(
            format!("{repository_fingerprint}\0{surface}\0{conversation_key}").as_bytes(),
        );
        let binding_path = lock_root
            .parent()
            .unwrap_or(&lock_root)
            .join("codex-thread-bindings")
            .join(format!("{lock_key}.json"));
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_root.join(format!("{lock_key}.lock")))
            .map_err(|_| ProviderFailure::io("The Codex thread lock could not be opened."))?;
        let wait_limit = configured_timeout
            .unwrap_or(MAX_CODEX_LOCK_WAIT)
            .min(MAX_CODEX_LOCK_WAIT);
        let started = Instant::now();
        loop {
            match FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(Self { file, binding_path }),
                Err(error) if lock_would_block(&error) && started.elapsed() < wait_limit => {
                    thread::sleep(
                        Duration::from_millis(20).min(wait_limit.saturating_sub(started.elapsed())),
                    );
                }
                Err(error) if lock_would_block(&error) => {
                    return Err(ProviderFailure {
                        kind: "provider_busy".to_string(),
                        message:
                            "Another Codex turn for this gateway conversation is still running."
                                .to_string(),
                        retryable: true,
                        turn_telemetry: None,
                    })
                }
                Err(_) => {
                    return Err(ProviderFailure::io(
                        "The Codex thread lock could not be acquired.",
                    ))
                }
            }
        }
    }

    fn load_binding(
        &self,
        settings: &CodexReplySettings,
        conversation_key: &str,
        surface: &str,
        repository_fingerprint: &str,
    ) -> Option<CodexThreadBinding> {
        let metadata = fs::metadata(&self.binding_path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_THREAD_BINDING_BYTES {
            return None;
        }
        let encoded = fs::read_to_string(&self.binding_path).ok()?;
        let value = encoded.parse::<JsonValue>().ok()?;
        compatible_thread_binding_value(
            &value,
            settings,
            conversation_key,
            surface,
            repository_fingerprint,
        )
    }

    fn store_binding(&self, binding: &JsonValue) -> Result<(), ProviderFailure> {
        let parent = self
            .binding_path
            .parent()
            .ok_or_else(|| ProviderFailure::io("The Codex thread binding path is unavailable."))?;
        fs::create_dir_all(parent).map_err(|_| {
            ProviderFailure::io("The Codex thread binding directory could not be created.")
        })?;
        let encoded = binding.to_string();
        if encoded.len() as u64 > MAX_THREAD_BINDING_BYTES {
            return Err(ProviderFailure::contract(
                "The Codex thread binding exceeded its bounded size.",
            ));
        }
        let temporary = self
            .binding_path
            .with_extension(format!("json.next-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|_| ProviderFailure::io("The Codex thread binding could not be staged."))?;
        file.write_all(encoded.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|_| ProviderFailure::io("The Codex thread binding could not be persisted."))?;
        drop(file);
        if self.binding_path.exists() {
            fs::remove_file(&self.binding_path).map_err(|_| {
                ProviderFailure::io("The prior Codex thread binding could not be replaced.")
            })?;
        }
        fs::rename(&temporary, &self.binding_path).map_err(|_| {
            let _ = fs::remove_file(&temporary);
            ProviderFailure::io("The Codex thread binding could not be published.")
        })
    }
}

impl Drop for CodexThreadLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn lock_would_block(error: &io::Error) -> bool {
    if error.kind() == ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(unix)]
    {
        matches!(
            error.raw_os_error(),
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
        )
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn resume_rejected_before_turn(stdout: &str, stderr: &str) -> bool {
    if !stderr
        .to_ascii_lowercase()
        .contains(CODEX_MISSING_ROLLOUT_MARKER)
    {
        return false;
    }
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Ok(event) = line.parse::<JsonValue>() else {
            return false;
        };
        let event_type = event
            .get("type")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        if event_type == "turn.started"
            || event_type == "turn.completed"
            || event_type == "turn.failed"
            || event_type.starts_with("item.")
        {
            return false;
        }
    }
    true
}

fn redact_secrets(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(object) => JsonValue::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let lowered = key.to_ascii_lowercase();
                    let value = if [
                        "token",
                        "secret",
                        "authorization",
                        "api_key",
                        "response_url",
                        "webhook_url",
                    ]
                    .iter()
                    .any(|needle| lowered.contains(needle))
                    {
                        JsonValue::String("[redacted]".to_string())
                    } else {
                        redact_secrets(value)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        JsonValue::Array(values) => JsonValue::Array(values.iter().map(redact_secrets).collect()),
        _ => value.clone(),
    }
}

fn parse_codex_jsonl(output: &str) -> Result<ParsedCodexReply, ProviderFailure> {
    let mut text = None;
    let mut thread_id = None;
    let mut usage = json!({});
    let mut saw_event = false;
    let mut telemetry = TurnTelemetryCollector::default();
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let event = line.parse::<JsonValue>().map_err(|_| {
            ProviderFailure::contract("The native Codex reply stream contained invalid JSONL.")
        })?;
        saw_event = true;
        telemetry.observe(&event);
        match event.get("type").and_then(JsonValue::as_str) {
            Some("thread.started") => {
                thread_id = event
                    .get("thread_id")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string);
            }
            Some("item.completed")
                if event
                    .get("item")
                    .and_then(|item| item.get("type"))
                    .and_then(JsonValue::as_str)
                    == Some("agent_message") =>
            {
                text = event
                    .get("item")
                    .and_then(|item| item.get("text"))
                    .and_then(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
            }
            Some("turn.completed") => {
                if let Some(value) = event.get("usage").filter(|value| value.is_object()) {
                    usage = value.clone();
                }
            }
            _ => {}
        }
    }
    if !saw_event {
        return Err(ProviderFailure::contract(
            "The native Codex reply stream was empty.",
        ));
    }
    let text = text.ok_or_else(|| {
        ProviderFailure::contract("The native Codex reply stream omitted an assistant message.")
    })?;
    Ok(ParsedCodexReply {
        text,
        thread_id,
        usage,
        turn_telemetry: telemetry.into_json(),
    })
}

fn request_surface(request: &JsonValue) -> String {
    request
        .get("surface")
        .and_then(|value| value.get("name"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("gateway")
        .to_ascii_lowercase()
}

fn parse_timeout(value: &str) -> Result<Option<Duration>, String> {
    if matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "inf" | "infinite" | "unlimited" | "none"
    ) {
        return Ok(None);
    }
    let seconds = value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| "The configured Codex turn timeout is invalid.".to_string())?;
    Ok(Some(Duration::from_secs_f64(seconds.max(0.01))))
}

fn validate_codex_program(program: &str) -> Result<(), String> {
    if program.len() > 4096 || program.chars().any(char::is_control) {
        return Err("The configured Codex executable path is invalid.".to_string());
    }
    let name = Path::new(program)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !name.starts_with("codex") || name.ends_with(".py") {
        return Err(
            "The native reply provider only accepts a direct Codex executable.".to_string(),
        );
    }
    Ok(())
}

fn invalid_option_value(value: &str) -> bool {
    value.len() > 256
        || value.starts_with('-')
        || value.chars().any(char::is_control)
        || value.trim().is_empty()
}

fn default_codex_program() -> String {
    let app_binary = PathBuf::from("/Applications/ChatGPT.app/Contents/Resources/codex");
    if app_binary.is_file() {
        app_binary.to_string_lossy().into_owned()
    } else {
        "codex".to_string()
    }
}

fn remove_transport_credentials(command: &mut Command, process_env: &BTreeMap<String, String>) {
    for name in process_env
        .keys()
        .filter(|name| is_transport_credential_name(name))
    {
        command.env_remove(name);
    }
}

fn is_transport_credential_name(name: &str) -> bool {
    let name = name.trim().to_ascii_uppercase();
    if matches!(
        name.as_str(),
        "BOT_TOKEN"
            | "DISCORD_BOT_TOKEN"
            | "DISCORD_PUBLIC_KEY"
            | "SLACK_BOT_TOKEN"
            | "SLACK_APP_TOKEN"
            | "SLACK_SIGNING_SECRET"
            | "LINE_CHANNEL_ACCESS_TOKEN"
            | "LINE_CHANNEL_SECRET"
    ) {
        return true;
    }
    if name.starts_with("AIT_AUTH_") || name.starts_with("AIT_REMOTE_AUTH_") {
        return true;
    }
    let transport_scoped = ["AIT_TELEGRAM_", "AIT_DISCORD_", "AIT_SLACK_", "AIT_LINE_"]
        .iter()
        .any(|prefix| name.starts_with(prefix));
    let credential_field = [
        "TOKEN",
        "SECRET",
        "SIGNATURE",
        "PUBLIC_KEY",
        "AUTHORIZATION",
        "RESPONSE_URL",
        "WEBHOOK_URL",
    ]
    .iter()
    .any(|field| name.contains(field));
    transport_scoped && credential_field
}

#[derive(Debug)]
struct ProviderFailure {
    kind: String,
    message: String,
    retryable: bool,
    turn_telemetry: Option<JsonValue>,
}

impl ProviderFailure {
    fn configuration(message: &str) -> Self {
        Self {
            kind: "provider_config".to_string(),
            message: message.to_string(),
            retryable: false,
            turn_telemetry: None,
        }
    }

    fn io(message: &str) -> Self {
        Self {
            kind: "provider_io".to_string(),
            message: message.to_string(),
            retryable: false,
            turn_telemetry: None,
        }
    }

    fn contract(message: &str) -> Self {
        Self {
            kind: "provider_contract".to_string(),
            message: message.to_string(),
            retryable: false,
            turn_telemetry: None,
        }
    }
}

fn provider_failure(
    response_contract: &str,
    kind: &str,
    message: &str,
    retryable: bool,
    turn_telemetry: Option<JsonValue>,
) -> JsonValue {
    let mut response = json!({
        "contract": response_contract,
        "ok": false,
        "error": {
            "kind": kind,
            "message": message,
        },
        "retryable": retryable,
    });
    if let Some(turn_telemetry) = turn_telemetry {
        response["turn_telemetry"] = turn_telemetry;
    }
    response
}

type ReaderResult = io::Result<(Vec<u8>, bool)>;

fn bounded_reader<R>(mut reader: R, limit: usize) -> JoinHandle<ReaderResult>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut captured = Vec::with_capacity(limit.min(64 * 1024));
        let mut exceeded = false;
        let mut chunk = [0_u8; 8192];
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

fn join_reader(handle: JoinHandle<ReaderResult>) -> Result<(Vec<u8>, bool), ProviderFailure> {
    handle
        .join()
        .map_err(|_| ProviderFailure::io("A native Codex reply output reader did not complete."))?
        .map_err(|_| ProviderFailure::io("A native Codex reply output stream could not be read."))
}

fn join_threads(
    writer: JoinHandle<io::Result<()>>,
    stdout: JoinHandle<ReaderResult>,
    stderr: JoinHandle<ReaderResult>,
) {
    let _ = writer.join();
    let _ = stdout.join();
    let _ = stderr.join();
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use ait_core::environment_contract::names;

    use super::*;

    fn gateway_request(surface: &str) -> JsonValue {
        json!({
            "contract": AGENT_GATEWAY_REPLY_PROVIDER_REQUEST_CONTRACT,
            "repository": {"repo_root": "/tmp", "repo_name": "fixture"},
            "surface": {"name": surface},
            "conversation": {"key": format!("{surface}:fixture-conversation")},
            "provider_thread": null,
            "input": {"text": "hello"}
        })
    }

    fn settings() -> CodexReplySettings {
        CodexReplySettings {
            program: "codex".to_string(),
            model: Some("gpt-5.6".to_string()),
            reasoning_effort: Some("xhigh".to_string()),
            sandbox: "workspace-write".to_string(),
            timeout: Some(Duration::from_secs(30)),
        }
    }

    fn binding(
        conversation_key: &str,
        surface: &str,
        settings: &CodexReplySettings,
        repository_fingerprint: &str,
    ) -> JsonValue {
        CodexThreadBinding {
            thread_id: "019c-thread-fixture".to_string(),
            conversation_key: conversation_key.to_string(),
            surface: surface.to_string(),
            model: configured_model(settings).to_string(),
            reasoning_effort: settings.reasoning_effort.clone(),
            sandbox: settings.sandbox.clone(),
            repository_fingerprint: repository_fingerprint.to_string(),
        }
        .to_json()
    }

    #[test]
    fn settings_honor_typed_request_configuration() {
        let mut request = gateway_request("telegram");
        request["settings"] = json!({
            "codex_program": "/opt/codex-next",
            "model": "gpt-5.6",
            "reasoning_effort": "xhigh",
            "sandbox": "danger-full-access",
            "turn_timeout_seconds": "inf",
        });
        let settings = CodexReplySettings::from_request(&request).expect("settings");

        assert_eq!(settings.program, "/opt/codex-next");
        assert_eq!(settings.model.as_deref(), Some("gpt-5.6"));
        assert_eq!(settings.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(settings.sandbox, "danger-full-access");
        assert_eq!(settings.timeout, None);
    }

    #[test]
    fn prompts_project_only_the_current_turn() {
        let mut request = gateway_request("telegram");
        request["input"] = json!({
            "text": "current request",
            "authorization": "Bearer secret",
            "workflow_context": {"task": "ASG-03", "bot_token": "secret"},
            "attachments": [{"kind": "document", "name": "notes.txt"}]
        });
        request["session"] = json!({"session_id": "retired-secret"});
        request["events"] = json!([{"payload": {"text": "stale transcript"}}]);
        request["checkpoint"] = json!({"summary": "stale checkpoint"});

        for prompt in [
            build_bootstrap_prompt(&request).expect("bootstrap"),
            build_resume_prompt(&request).expect("resume"),
        ] {
            assert!(prompt.text.contains("current request"));
            assert!(prompt.text.contains("ASG-03"));
            assert!(prompt.text.contains("notes.txt"));
            assert!(!prompt.text.contains("Bearer secret"));
            assert!(!prompt.text.contains("\"bot_token\":\"secret\""));
            assert!(prompt.text.contains("[redacted]"));
            assert!(!prompt.text.contains("retired-secret"));
            assert!(!prompt.text.contains("stale transcript"));
            assert!(!prompt.text.contains("stale checkpoint"));
            assert_eq!(prompt.context_event_count, 0);
        }
    }

    #[test]
    fn thread_binding_requires_exact_conversation_and_runtime_fingerprint() {
        let settings = settings();
        let conversation_key = "discord:channel:42";
        let fingerprint = "sha256:repo-fixture";
        let mut request = gateway_request("discord");
        request["conversation"]["key"] = json!(conversation_key);
        request["provider_thread"] = binding(conversation_key, "discord", &settings, fingerprint);

        assert!(compatible_thread_binding(
            &request,
            &settings,
            conversation_key,
            "discord",
            fingerprint,
        )
        .is_some());
        assert!(request["provider_thread"].get("session_id").is_none());

        for field in [
            "conversation_key",
            "surface",
            "model",
            "reasoning_effort",
            "sandbox",
            "repository_fingerprint",
        ] {
            let mut changed = request.clone();
            changed["provider_thread"][field] = json!("different");
            assert!(compatible_thread_binding(
                &changed,
                &settings,
                conversation_key,
                "discord",
                fingerprint,
            )
            .is_none());
        }
    }

    #[test]
    fn thread_binding_store_contains_no_transcript_or_session_state() {
        let repo = tempfile::tempdir().expect("temporary repository");
        fs::create_dir_all(repo.path().join(".ait")).expect("AIT directory");
        let settings = settings();
        let fingerprint = repository_fingerprint(repo.path());
        let conversation_key = "discord:channel:998877665544332211";
        let binding = binding(conversation_key, "discord", &settings, &fingerprint);

        let lock = CodexThreadLock::acquire(
            repo.path(),
            &fingerprint,
            conversation_key,
            "discord",
            settings.timeout,
        )
        .expect("lock");
        lock.store_binding(&binding).expect("store binding");
        let encoded = fs::read_to_string(&lock.binding_path).expect("stored binding");
        assert!(encoded.contains(conversation_key));
        assert!(!encoded.contains("session_id"));
        assert!(!encoded.contains("events"));
        assert!(!encoded.contains("checkpoint"));

        let loaded = lock
            .load_binding(&settings, conversation_key, "discord", &fingerprint)
            .expect("compatible binding");
        assert_eq!(loaded.conversation_key, conversation_key);
    }

    #[test]
    fn codex_resume_uses_native_thread_resume_arguments() {
        let settings = settings();
        let command = configure_codex_command(
            &settings,
            Path::new("/tmp/fixture-repo"),
            &BTreeMap::new(),
            CodexInvocationMode::Resume {
                thread_id: "019c-thread-fixture",
            },
        );
        let args = command
            .get_args()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(args[0], "exec");
        assert_eq!(args[1], "resume");
        assert!(args.contains(&"019c-thread-fixture".to_string()));
        assert!(!args.contains(&"--color".to_string()));
    }

    #[test]
    fn codex_jsonl_parser_returns_final_reply_usage_and_deduplicated_telemetry() {
        let output = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"thread-1\"}\n",
            "{\"type\":\"item.started\",\"item\":{\"id\":\"command-1\",\"type\":\"command_execution\",\"command\":\"ait status\",\"status\":\"in_progress\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"command-1\",\"type\":\"command_execution\",\"command\":\"ait status\",\"exit_code\":0,\"status\":\"completed\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"final reply\"}}\n",
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":7,\"output_tokens\":3}}\n"
        );
        let parsed = parse_codex_jsonl(output).expect("Codex output");

        assert_eq!(parsed.text, "final reply");
        assert_eq!(parsed.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(parsed.usage["input_tokens"], 7);
        assert_eq!(parsed.turn_telemetry["command_count"], 1);
        assert_eq!(parsed.turn_telemetry["ait_command_count"], 1);
    }

    #[test]
    fn invalid_or_legacy_requests_fail_with_gateway_response_contract() {
        let malformed = execute_native_reply_provider("not-json", &BTreeMap::new());
        assert_eq!(
            malformed["contract"],
            AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT
        );
        assert_eq!(malformed["error"]["kind"], "provider_request_invalid");

        let legacy = execute_native_reply_provider(
            &json!({
                "contract": "ait.agent.local_reply_provider.request.v1",
                "session": {"session_id": "retired"}
            })
            .to_string(),
            &BTreeMap::new(),
        );
        assert_eq!(legacy["error"]["kind"], "provider_request_contract");
    }

    #[test]
    fn transport_credentials_are_scrubbed_from_codex_child_environment() {
        for name in [
            "BOT_TOKEN",
            names::AIT_TELEGRAM_BOT_TOKEN,
            names::AIT_DISCORD_BOT_TOKEN,
            names::AIT_SLACK_SIGNING_SECRET,
            names::AIT_LINE_CHANNEL_ACCESS_TOKEN,
        ] {
            assert!(is_transport_credential_name(name), "{name}");
        }
        assert!(!is_transport_credential_name("OPENAI_API_KEY"));
    }
}
