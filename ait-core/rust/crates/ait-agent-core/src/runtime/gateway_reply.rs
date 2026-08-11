use std::env;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

pub const AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT: &str = "ait.agent.gateway_reply_runtime.v1";
pub const AGENT_GATEWAY_REPLY_PROVIDER_REQUEST_CONTRACT: &str =
    "ait.agent.gateway_reply_provider_request.v1";
pub const AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT: &str =
    "ait.agent.gateway_reply_provider_response.v1";
pub const AGENT_GATEWAY_CODEX_THREAD_BINDING_CONTRACT: &str =
    "ait.agent.gateway_codex_thread_binding.v1";
pub const AGENT_GATEWAY_TURN_TELEMETRY_CONTRACT: &str = "ait.agent.gateway_turn_telemetry.v1";

const MAX_CONVERSATION_KEY_BYTES: usize = 4_096;
const DEFAULT_PROVIDER_TIMEOUT_SECONDS: f64 = 120.0;
const PRACTICALLY_UNLIMITED_PROVIDER_TIMEOUT_SECONDS: f64 = 365.0 * 24.0 * 60.0 * 60.0;
const MAX_PROVIDER_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROVIDER_STDERR_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentLocalReplyProviderError {
    kind: String,
    message: String,
    retryable: bool,
}

impl AgentLocalReplyProviderError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            retryable,
        }
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }

    fn config(message: impl Into<String>) -> Self {
        Self::new("provider_config", message, false)
    }
}

/// Provider boundary used by every direct gateway turn. The request contains
/// only the transport conversation key and latest inbound payload; Codex owns
/// all transcript continuity.
pub trait AgentLocalReplyProvider {
    fn generate(&self, request: &JsonValue) -> Result<JsonValue, AgentLocalReplyProviderError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentLocalReplyRuntimeSettings {
    pub append_turn_analysis: bool,
}

impl AgentLocalReplyRuntimeSettings {
    pub fn from_request(request: &JsonValue) -> Result<Self, String> {
        let config = optional_provider_config(request)?;
        let append_turn_analysis = match config.and_then(|value| value.get("append_turn_analysis"))
        {
            None | Some(JsonValue::Null) => env_bool(&[
                "AIT_CHAT_APPEND_TURN_ANALYSIS",
                "AIT_TELEGRAM_APPEND_TURN_ANALYSIS",
            ])
            .unwrap_or(false),
            Some(value) => value
                .as_bool()
                .ok_or_else(|| "local_reply.append_turn_analysis must be a boolean".to_string())?,
        };
        Ok(Self {
            append_turn_analysis,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentLocalReplyProcessConfig {
    pub program: String,
    pub args: Vec<String>,
    pub timeout: Duration,
}

impl AgentLocalReplyProcessConfig {
    pub fn from_request(request: &JsonValue) -> Result<Self, AgentLocalReplyProviderError> {
        let config =
            optional_provider_config(request).map_err(AgentLocalReplyProviderError::config)?;
        let program = match config.and_then(|value| value.get("program")) {
            None | Some(JsonValue::Null) => env_nonempty(&["AIT_AGENT_LOCAL_REPLY_PROGRAM"]),
            Some(value) => Some(
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        AgentLocalReplyProviderError::config(
                            "local_reply.program must be a non-empty string",
                        )
                    })?,
            ),
        }
        .ok_or_else(|| {
            AgentLocalReplyProviderError::config(
                "Local reply provider program is required; set local_reply.program or AIT_AGENT_LOCAL_REPLY_PROGRAM.",
            )
        })?;
        if provider_program_is_forbidden(&program) {
            return Err(AgentLocalReplyProviderError::config(format!(
                "Local reply provider program `{program}` is forbidden; shell and Python execution are not supported."
            )));
        }
        let args = match config.and_then(|value| value.get("args")) {
            Some(value) => string_array(value, "local_reply.args")
                .map_err(AgentLocalReplyProviderError::config)?,
            None => env_args()?,
        };
        if args
            .iter()
            .any(|value| value.trim().to_ascii_lowercase().ends_with(".py"))
        {
            return Err(AgentLocalReplyProviderError::config(
                "Local reply provider arguments must not reference Python scripts.",
            ));
        }
        let timeout_seconds = match config.and_then(|value| value.get("timeout_seconds")) {
            None | Some(JsonValue::Null) => {
                env_provider_timeout_seconds(&["AIT_AGENT_LOCAL_REPLY_TIMEOUT_SECONDS"])
                    .unwrap_or(DEFAULT_PROVIDER_TIMEOUT_SECONDS)
            }
            Some(value) => value
                .as_f64()
                .filter(|value| value.is_finite() && *value > 0.0)
                .ok_or_else(|| {
                    AgentLocalReplyProviderError::config(
                        "Local reply provider timeout_seconds must be a finite positive number.",
                    )
                })?,
        };
        Ok(Self {
            program,
            args,
            timeout: Duration::from_secs_f64(timeout_seconds.max(0.01)),
        })
    }
}

#[derive(Clone, Debug)]
pub struct ExternalProcessAgentLocalReplyProvider {
    config: AgentLocalReplyProcessConfig,
}

impl ExternalProcessAgentLocalReplyProvider {
    pub fn new(config: AgentLocalReplyProcessConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &AgentLocalReplyProcessConfig {
        &self.config
    }
}

impl AgentLocalReplyProvider for ExternalProcessAgentLocalReplyProvider {
    fn generate(&self, request: &JsonValue) -> Result<JsonValue, AgentLocalReplyProviderError> {
        let mut command = Command::new(&self.config.program);
        command
            .args(&self.config.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(repo_root) = request
            .get("repository")
            .and_then(|value| value.get("repo_root"))
            .and_then(JsonValue::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            command.current_dir(repo_root);
        }
        let mut child = command.spawn().map_err(|error| {
            AgentLocalReplyProviderError::new(
                "provider_spawn",
                format!(
                    "Failed to start local reply provider '{}': {error}",
                    self.config.program
                ),
                false,
            )
        })?;
        let Some(mut stdin) = child.stdin.take() else {
            terminate_child(&mut child);
            return Err(provider_io(
                "Local reply provider stdin pipe was unavailable.",
            ));
        };
        let Some(stdout) = child.stdout.take() else {
            terminate_child(&mut child);
            return Err(provider_io(
                "Local reply provider stdout pipe was unavailable.",
            ));
        };
        let Some(stderr) = child.stderr.take() else {
            terminate_child(&mut child);
            return Err(provider_io(
                "Local reply provider stderr pipe was unavailable.",
            ));
        };
        let input = request.to_string().into_bytes();
        let stdin_writer = thread::spawn(move || {
            stdin.write_all(&input)?;
            drop(stdin);
            Ok::<(), io::Error>(())
        });
        let stdout_reader = spawn_bounded_reader(stdout, MAX_PROVIDER_OUTPUT_BYTES);
        let stderr_reader = spawn_bounded_reader(stderr, MAX_PROVIDER_STDERR_BYTES);

        let deadline = Instant::now() + self.config.timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    terminate_child(&mut child);
                    join_provider_threads(stdin_writer, stdout_reader, stderr_reader);
                    return Err(AgentLocalReplyProviderError::new(
                        "provider_timeout",
                        format!(
                            "Local reply provider timed out after {:.3} seconds.",
                            self.config.timeout.as_secs_f64()
                        ),
                        true,
                    ));
                }
                Err(error) => {
                    terminate_child(&mut child);
                    join_provider_threads(stdin_writer, stdout_reader, stderr_reader);
                    return Err(AgentLocalReplyProviderError::new(
                        "provider_io",
                        format!("Failed to inspect local reply provider status: {error}"),
                        false,
                    ));
                }
            }
        };
        let stdin_result = join_writer(stdin_writer)?;
        let (stdout, stdout_exceeded) = join_reader(stdout_reader, "stdout")?;
        let _ = join_reader(stderr_reader, "stderr")?;
        if !status.success() {
            return Err(AgentLocalReplyProviderError::new(
                "provider_exit",
                format!("Local reply provider exited with status {status}"),
                false,
            ));
        }
        if let Err(error) = stdin_result {
            if error.kind() != io::ErrorKind::BrokenPipe {
                return Err(provider_io(format!(
                    "Failed to write local reply provider request: {error}"
                )));
            }
        }
        if stdout_exceeded {
            return Err(AgentLocalReplyProviderError::new(
                "provider_contract",
                "Local reply provider output exceeded 4 MiB.",
                false,
            ));
        }
        let stdout = String::from_utf8(stdout).map_err(|error| {
            AgentLocalReplyProviderError::new(
                "provider_contract",
                format!("Local reply provider output was not UTF-8: {error}"),
                false,
            )
        })?;
        let response = stdout.trim().parse::<JsonValue>().map_err(|error| {
            AgentLocalReplyProviderError::new(
                "provider_contract",
                format!("Local reply provider output was not valid JSON: {error}"),
                false,
            )
        })?;
        validate_process_response(response)
    }
}

/// Executes a chat turn without reading or mutating an AIT session. Codex owns
/// conversation history; this layer only supplies transport identity and the
/// latest input to the versioned gateway provider.
pub fn agent_gateway_reply_runtime_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    let operation = request
        .get("operation")
        .and_then(JsonValue::as_str)
        .unwrap_or("create_turn")
        .trim()
        .to_string();
    let config = match AgentLocalReplyProcessConfig::from_request(request) {
        Ok(config) => config,
        Err(error) => return Ok(runtime_provider_failure(&operation, &error)),
    };
    execute_with_agent_gateway_reply_provider(
        &ExternalProcessAgentLocalReplyProvider::new(config),
        request,
    )
}

pub fn execute_with_agent_gateway_reply_provider<P>(
    provider: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: AgentLocalReplyProvider + ?Sized,
{
    let request = required_object(Some(request), "gateway reply runtime request")?;
    let operation = required_text(request.get("operation"), "operation")?;
    if !matches!(operation.as_str(), "create_turn" | "create_telegram_turn") {
        return Err(format!(
            "unsupported gateway reply runtime operation `{operation}`"
        ));
    }
    let target = required_object(request.get("target"), "target")?;
    let repo_root = required_text(target.get("repo_root"), "target.repo_root")?;
    let repo_name = required_text(target.get("repo_name"), "target.repo_name")?;
    let arguments = required_object(request.get("arguments"), "arguments")?;
    let conversation_key = bounded_conversation_key(arguments.get("conversation_key"))?;
    let payload = required_object(arguments.get("payload"), "arguments.payload")?;
    required_text(payload.get("text"), "arguments.payload.text")?;
    let settings =
        AgentLocalReplyRuntimeSettings::from_request(&JsonValue::Object(request.clone()))?;
    let surface = if operation == "create_telegram_turn" {
        "telegram".to_string()
    } else {
        clean_text(payload.get("surface")).unwrap_or_else(|| "gateway".to_string())
    };
    let actor = optional_object(request.get("actor"), "actor")?;
    let actor_identity = actor
        .and_then(|value| clean_text(value.get("identity")))
        .unwrap_or_else(|| "gateway-user".to_string());
    let actor_type = actor
        .and_then(|value| clean_text(value.get("type")))
        .unwrap_or_else(|| "human".to_string());
    let provider_thread =
        match arguments.get("provider_thread") {
            None | Some(JsonValue::Null) => JsonValue::Null,
            Some(value) if value.is_object() => value.clone(),
            Some(_) => return Err(
                "gateway reply runtime field `arguments.provider_thread` must be an object or null"
                    .to_string(),
            ),
        };
    let provider_request = json!({
        "contract": AGENT_GATEWAY_REPLY_PROVIDER_REQUEST_CONTRACT,
        "operation": operation,
        "repository": {
            "repo_root": repo_root,
            "canonical_repo_root": repo_root,
            "repo_name": repo_name,
            "worktree_name": target.get("worktree_name").cloned().unwrap_or(JsonValue::Null),
        },
        "conversation": {"key": conversation_key},
        "provider_thread": provider_thread,
        "surface": {
            "name": surface,
            "title": clean_text(payload.get("title"))
                .or_else(|| clean_text(payload.get("chat_title")))
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
            "transport_envelope": payload
                .get("transport_envelope")
                .cloned()
                .unwrap_or(JsonValue::Null),
        },
        "actor": {"identity": actor_identity, "type": actor_type},
        "input": JsonValue::Object(payload.clone()),
    });

    let response = match provider.generate(&provider_request) {
        Ok(response) => response,
        Err(error) => {
            return Ok(runtime_success(
                &operation,
                failed_turn_payload(&conversation_key, &surface, &error, None),
            ))
        }
    };
    let reply = match normalized_gateway_reply(&response) {
        Ok(reply) => reply,
        Err(error) => {
            return Ok(runtime_success(
                &operation,
                failed_turn_payload(
                    &conversation_key,
                    &surface,
                    &error,
                    response
                        .get("turn_telemetry")
                        .filter(|value| value.is_object()),
                ),
            ))
        }
    };
    let assistant_text = required_text(reply.get("text"), "provider.reply.text")?;
    let reply_text = reply_text_with_turn_analysis(
        &assistant_text,
        reply.get("turn_analysis"),
        settings.append_turn_analysis,
    );
    let turn_analysis = reply
        .get("turn_analysis")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let turn_telemetry = reply
        .get("turn_telemetry")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let provider_thread = turn_analysis
        .get("provider_thread")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(JsonValue::Null);
    Ok(runtime_success(
        &operation,
        json!({
            "ok": true,
            "conversation_key": conversation_key,
            "reply": reply,
            "reply_text": reply_text,
            "provider_thread": provider_thread,
            "turn_analysis": turn_analysis,
            "turn_telemetry": turn_telemetry,
            "surface": surface,
        }),
    ))
}

fn normalized_gateway_reply(
    response: &JsonValue,
) -> Result<JsonValue, AgentLocalReplyProviderError> {
    let object = response.as_object().ok_or_else(|| {
        provider_contract_error("Gateway reply provider response must be a JSON object.")
    })?;
    if object.get("contract").and_then(JsonValue::as_str)
        != Some(AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT)
    {
        return Err(provider_contract_error(
            "Gateway reply provider returned an unsupported contract.",
        ));
    }
    match object.get("ok").and_then(JsonValue::as_bool) {
        Some(true) => {}
        Some(false) => {
            let error = object.get("error").and_then(JsonValue::as_object);
            return Err(AgentLocalReplyProviderError::new(
                error
                    .and_then(|value| clean_text(value.get("kind")))
                    .unwrap_or_else(|| "provider_error".to_string()),
                clean_text(object.get("message"))
                    .or_else(|| error.and_then(|value| clean_text(value.get("message"))))
                    .unwrap_or_else(|| "Gateway reply provider reported an error.".to_string()),
                object.get("retryable").and_then(JsonValue::as_bool) == Some(true),
            ));
        }
        None => {
            return Err(provider_contract_error(
                "Gateway reply provider response omitted boolean `ok`.",
            ))
        }
    }
    let reply = object
        .get("reply")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            provider_contract_error(
                "Gateway reply provider success response omitted object `reply`.",
            )
        })?;
    let text = clean_text(reply.get("text"))
        .ok_or_else(|| provider_contract_error("Gateway reply provider returned empty text."))?;
    let model = clean_text(reply.get("model"))
        .ok_or_else(|| provider_contract_error("Gateway reply provider returned empty model."))?;
    for field in ["usage", "turn_analysis", "turn_telemetry"] {
        if reply
            .get(field)
            .is_some_and(|value| !value.is_null() && !value.is_object())
        {
            return Err(provider_contract_error(format!(
                "Gateway reply provider field `reply.{field}` must be an object or null."
            )));
        }
    }
    if reply.get("attachments").is_some_and(|value| {
        value
            .as_array()
            .is_none_or(|items| items.iter().any(|item| !item.is_object()))
    }) {
        return Err(provider_contract_error(
            "Gateway reply provider attachments must be JSON objects.",
        ));
    }
    let mut normalized = reply.clone();
    normalized.insert("text".to_string(), JsonValue::String(text));
    normalized.insert("model".to_string(), JsonValue::String(model));
    normalized
        .entry("source".to_string())
        .or_insert_with(|| JsonValue::String("external".to_string()));
    normalized
        .entry("response_id".to_string())
        .or_insert(JsonValue::Null);
    normalized
        .entry("usage".to_string())
        .or_insert_with(|| json!({}));
    normalized
        .entry("turn_analysis".to_string())
        .or_insert_with(|| json!({}));
    normalized
        .entry("turn_telemetry".to_string())
        .or_insert_with(|| json!({}));
    normalized
        .entry("attachments".to_string())
        .or_insert_with(|| json!([]));
    Ok(JsonValue::Object(normalized))
}

fn failed_turn_payload(
    conversation_key: &str,
    surface: &str,
    error: &AgentLocalReplyProviderError,
    telemetry: Option<&JsonValue>,
) -> JsonValue {
    json!({
        "ok": false,
        "conversation_key": conversation_key,
        "reply": JsonValue::Null,
        "reply_text": JsonValue::Null,
        "provider_thread": JsonValue::Null,
        "turn_analysis": {},
        "turn_telemetry": telemetry.cloned().unwrap_or_else(|| json!({})),
        "surface": surface,
        "error": error.message(),
        "provider_error": {
            "kind": error.kind(),
            "message": error.message(),
            "retryable": error.retryable(),
        },
    })
}

fn runtime_success(operation: &str, payload: JsonValue) -> JsonValue {
    json!({
        "contract": AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT,
        "ok": true,
        "operation": operation,
        "payload": payload,
    })
}

fn runtime_provider_failure(operation: &str, error: &AgentLocalReplyProviderError) -> JsonValue {
    json!({
        "contract": AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT,
        "ok": false,
        "operation": operation,
        "retryable": error.retryable(),
        "retry_exhausted": false,
        "message": error.message(),
        "error": {"kind": error.kind(), "message": error.message()},
    })
}

fn reply_text_with_turn_analysis(
    text: &str,
    turn_analysis: Option<&JsonValue>,
    append: bool,
) -> String {
    if !append {
        return text.trim().to_string();
    }
    let analysis = turn_analysis.and_then(JsonValue::as_object);
    let command_count = analysis
        .and_then(|value| value.get("command_count"))
        .and_then(JsonValue::as_i64)
        .unwrap_or(0);
    let optimization_summary = analysis
        .and_then(|value| clean_text(value.get("optimization_summary")))
        .unwrap_or_default();
    if command_count <= 0 && optimization_summary.is_empty() {
        return text.trim().to_string();
    }
    let mut parts = vec![format!("ran {command_count} commands")];
    if !optimization_summary.is_empty() {
        parts.push(optimization_summary);
    }
    format!("{}\n\n[turn analysis] {}", text.trim(), parts.join(" · "))
}

fn provider_contract_error(message: impl Into<String>) -> AgentLocalReplyProviderError {
    AgentLocalReplyProviderError::new("provider_contract", message, false)
}

fn bounded_conversation_key(value: Option<&JsonValue>) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= MAX_CONVERSATION_KEY_BYTES)
        .filter(|value| !value.chars().any(char::is_control))
        .map(str::to_string)
        .ok_or_else(|| {
            "gateway reply runtime field `arguments.conversation_key` must be a bounded non-empty string"
                .to_string()
        })
}

fn required_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    value
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("gateway reply runtime field `{field}` must be an object"))
}

fn optional_object<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<Option<&'a Map<String, JsonValue>>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(value)) => Ok(Some(value)),
        Some(_) => Err(format!(
            "gateway reply runtime field `{field}` must be an object or null"
        )),
    }
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    clean_text(value)
        .ok_or_else(|| format!("gateway reply runtime field `{field}` must be a non-empty string"))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_provider_config(
    request: &JsonValue,
) -> Result<Option<&Map<String, JsonValue>>, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "gateway reply runtime request must be an object".to_string())?;
    match request.get("local_reply") {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(value)) => Ok(Some(value)),
        Some(_) => {
            Err("gateway reply runtime field `local_reply` must be an object or null".to_string())
        }
    }
}

fn env_nonempty(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn env_provider_timeout_seconds(names: &[&str]) -> Option<f64> {
    parse_provider_timeout_seconds(&env_nonempty(names)?)
}

fn parse_provider_timeout_seconds(value: &str) -> Option<f64> {
    if matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "inf" | "infinite" | "unlimited" | "none"
    ) {
        return Some(PRACTICALLY_UNLIMITED_PROVIDER_TIMEOUT_SECONDS);
    }
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn env_bool(names: &[&str]) -> Option<bool> {
    match env_nonempty(names)?.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn env_args() -> Result<Vec<String>, AgentLocalReplyProviderError> {
    let Some(value) = env_nonempty(&["AIT_AGENT_LOCAL_REPLY_ARGS_JSON"]) else {
        return Ok(Vec::new());
    };
    let parsed = value.parse::<JsonValue>().map_err(|error| {
        AgentLocalReplyProviderError::config(format!(
            "AIT_AGENT_LOCAL_REPLY_ARGS_JSON must be a JSON string array: {error}"
        ))
    })?;
    string_array(&parsed, "AIT_AGENT_LOCAL_REPLY_ARGS_JSON")
        .map_err(AgentLocalReplyProviderError::config)
}

fn string_array(value: &JsonValue, field: &str) -> Result<Vec<String>, String> {
    let values = value
        .as_array()
        .ok_or_else(|| format!("{field} must be a JSON string array"))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{field}[{index}] must be a string"))
        })
        .collect()
}

fn provider_program_is_forbidden(program: &str) -> bool {
    let file_name = PathBuf::from(program)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(program)
        .trim()
        .to_ascii_lowercase();
    file_name.starts_with("python")
        || file_name.ends_with(".py")
        || matches!(
            file_name.as_str(),
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

fn join_writer(
    handle: JoinHandle<io::Result<()>>,
) -> Result<io::Result<()>, AgentLocalReplyProviderError> {
    handle
        .join()
        .map_err(|_| provider_io("Local reply provider stdin writer panicked."))
}

fn join_reader(
    handle: JoinHandle<ReaderResult>,
    stream: &str,
) -> Result<(Vec<u8>, bool), AgentLocalReplyProviderError> {
    handle
        .join()
        .map_err(|_| provider_io(format!("Local reply provider {stream} reader panicked.")))?
        .map_err(|error| {
            provider_io(format!(
                "Failed to read local reply provider {stream}: {error}"
            ))
        })
}

fn join_provider_threads(
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

fn provider_io(message: impl Into<String>) -> AgentLocalReplyProviderError {
    AgentLocalReplyProviderError::new("provider_io", message, false)
}

fn validate_process_response(
    response: JsonValue,
) -> Result<JsonValue, AgentLocalReplyProviderError> {
    let object = response.as_object().ok_or_else(|| {
        provider_contract_error("Gateway reply provider response must be a JSON object.")
    })?;
    if object.get("contract").and_then(JsonValue::as_str)
        != Some(AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT)
    {
        return Err(provider_contract_error(
            "Gateway reply provider returned an unsupported contract.",
        ));
    }
    if object.get("ok").and_then(JsonValue::as_bool).is_none() {
        return Err(provider_contract_error(
            "Gateway reply provider response omitted boolean `ok`.",
        ));
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[derive(Default)]
    struct RecordingProvider {
        requests: RefCell<Vec<JsonValue>>,
    }

    impl AgentLocalReplyProvider for RecordingProvider {
        fn generate(&self, request: &JsonValue) -> Result<JsonValue, AgentLocalReplyProviderError> {
            self.requests.borrow_mut().push(request.clone());
            Ok(json!({
                "contract": AGENT_GATEWAY_REPLY_PROVIDER_RESPONSE_CONTRACT,
                "ok": true,
                "reply": {
                    "text": "gateway reply",
                    "model": "gpt-5.6",
                    "source": "codex_exec",
                    "turn_analysis": {
                        "provider_thread": {
                            "contract": AGENT_GATEWAY_CODEX_THREAD_BINDING_CONTRACT,
                            "thread_id": "019c-gateway-thread",
                            "conversation_key": "discord:channel-1",
                        },
                        "command_count": 2,
                    },
                    "turn_telemetry": {
                        "contract": AGENT_GATEWAY_TURN_TELEMETRY_CONTRACT,
                        "command_count": 2,
                    },
                    "attachments": [],
                },
            }))
        }
    }

    fn request() -> JsonValue {
        json!({
            "operation": "create_turn",
            "target": {
                "mode": "remote",
                "workflow_mode": "solo_remote",
                "repo_name": "fixture",
                "repo_root": "/tmp/fixture",
                "server_url": "http://127.0.0.1:1",
            },
            "actor": {"identity": "discord-user", "type": "discord_user"},
            "arguments": {
                "conversation_key": "discord:channel-1",
                "payload": {
                    "text": "hello",
                    "surface": "discord",
                    "title": "general",
                },
            },
            "local_reply": {"append_turn_analysis": false},
        })
    }

    #[test]
    fn direct_gateway_turn_contains_no_session_history_or_checkpoint() {
        let provider = RecordingProvider::default();
        let response = execute_with_agent_gateway_reply_provider(&provider, &request())
            .expect("gateway reply");
        assert_eq!(response["contract"], AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT);
        assert_eq!(response["payload"]["ok"], true);
        assert_eq!(response["payload"]["reply_text"], "gateway reply");
        let requests = provider.requests.borrow();
        let provider_request = &requests[0];
        assert_eq!(
            provider_request["contract"],
            AGENT_GATEWAY_REPLY_PROVIDER_REQUEST_CONTRACT
        );
        assert_eq!(provider_request["conversation"]["key"], "discord:channel-1");
        for forbidden in ["session", "events", "checkpoint"] {
            assert!(provider_request.get(forbidden).is_none());
        }
        assert!(!provider_request.to_string().contains("server_url"));
    }

    #[test]
    fn conversation_key_is_required_instead_of_session_id() {
        let provider = RecordingProvider::default();
        let mut invalid = request();
        invalid["arguments"]
            .as_object_mut()
            .expect("arguments")
            .remove("conversation_key");
        invalid["arguments"]["session_id"] = json!("legacy-session");
        let error = execute_with_agent_gateway_reply_provider(&provider, &invalid)
            .expect_err("conversation key required");
        assert!(error.contains("conversation_key"));
        assert!(provider.requests.borrow().is_empty());
    }
}
