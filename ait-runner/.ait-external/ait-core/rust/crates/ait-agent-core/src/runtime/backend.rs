use std::thread;
use std::time::Duration;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::transport::{
    agent_transport_http_execute_json_request_json, agent_transport_retry_delay_seconds,
    agent_transport_retry_is_loopback_url,
};

pub const AGENT_REMOTE_RUNTIME_BACKEND_CONTRACT: &str = "ait.agent.remote_runtime_backend.v1";

const MAX_RETRY_ATTEMPTS: u64 = 4;
const RETRY_BASE_DELAY_SECONDS: f64 = 0.75;

pub trait AgentRuntimeBackend {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait AgentRuntimeHttpExecutor {
    fn execute_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait AgentRuntimeRetrySleeper {
    fn sleep(&self, delay: Duration);
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NativeAgentRuntimeHttpExecutor;

impl AgentRuntimeHttpExecutor for NativeAgentRuntimeHttpExecutor {
    fn execute_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_transport_http_execute_json_request_json(request)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadAgentRuntimeRetrySleeper;

impl AgentRuntimeRetrySleeper for ThreadAgentRuntimeRetrySleeper {
    fn sleep(&self, delay: Duration) {
        thread::sleep(delay);
    }
}

#[derive(Debug, Clone)]
pub struct RemoteAitRuntimeBackend<
    E = NativeAgentRuntimeHttpExecutor,
    S = ThreadAgentRuntimeRetrySleeper,
> {
    executor: E,
    sleeper: S,
}

impl<E, S> RemoteAitRuntimeBackend<E, S> {
    pub fn new(executor: E, sleeper: S) -> Self {
        Self { executor, sleeper }
    }
}

impl Default for RemoteAitRuntimeBackend {
    fn default() -> Self {
        Self::new(
            NativeAgentRuntimeHttpExecutor,
            ThreadAgentRuntimeRetrySleeper,
        )
    }
}

impl<E, S> AgentRuntimeBackend for RemoteAitRuntimeBackend<E, S>
where
    E: AgentRuntimeHttpExecutor,
    S: AgentRuntimeRetrySleeper,
{
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let plan = RemoteOperationPlan::from_request(request)?;
        let max_attempts = if plan.retry_safe
            && !plan.optional_authority
            && agent_transport_retry_is_loopback_url(&plan.server_url)
        {
            MAX_RETRY_ATTEMPTS
        } else {
            1
        };
        let mut attempts = 0_u64;
        let mut retry_delays = Vec::new();

        loop {
            attempts += 1;
            let result = self.executor.execute_json(&plan.http_request)?;
            let ok = result
                .get("ok")
                .and_then(JsonValue::as_bool)
                .ok_or_else(|| {
                    "Rust ait-agent transport HTTP executor returned an invalid status.".to_string()
                })?;
            if ok {
                return plan.success_envelope(result, attempts, retry_delays);
            }

            let retryable = retryable_execution_error(&result);
            if retryable && attempts < max_attempts {
                let delay = agent_transport_retry_delay_seconds(
                    RETRY_BASE_DELAY_SECONDS,
                    (attempts - 1) as i64,
                );
                retry_delays.push(delay);
                self.sleeper.sleep(Duration::from_secs_f64(delay));
                continue;
            }
            return Ok(plan.error_envelope(
                result,
                attempts,
                retry_delays,
                retryable,
                retryable && attempts >= max_attempts && max_attempts > 1,
            ));
        }
    }
}

pub fn agent_remote_runtime_backend_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    RemoteAitRuntimeBackend::default().execute(request)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseShape {
    Object,
}

impl ResponseShape {
    fn description(self) -> &'static str {
        match self {
            Self::Object => "an object",
        }
    }

    fn accepts(self, value: &JsonValue) -> bool {
        match self {
            Self::Object => value.is_object(),
        }
    }
}

#[derive(Debug, Clone)]
struct RemoteOperationPlan {
    operation: String,
    server_url: String,
    method: String,
    url: String,
    http_request: JsonValue,
    retry_safe: bool,
    optional_authority: bool,
    response_shape: ResponseShape,
}

impl RemoteOperationPlan {
    fn from_request(request: &JsonValue) -> Result<Self, String> {
        let request = require_object(request, "remote runtime backend request")?;
        let operation = require_text(request.get("operation"), "operation")?;
        let target = require_object(
            request.get("target").ok_or_else(|| {
                "remote runtime backend request field `target` is required".to_string()
            })?,
            "target",
        )?;
        let workflow_mode = validate_remote_target(target)?;
        if workflow_mode == "solo_local" && operation != "read_task_queue" {
            return Err(
                "solo_local may use a remote runtime only for the optional task-queue notification read"
                    .to_string(),
            );
        }
        let repo_name = require_text(target.get("repo_name"), "target.repo_name")?;
        let server_url = validate_server_url(&require_text(
            target.get("server_url"),
            "target.server_url",
        )?)?;
        let actor = require_object(
            request.get("actor").ok_or_else(|| {
                "remote runtime backend request field `actor` is required".to_string()
            })?,
            "actor",
        )?;
        let actor_identity = require_header_text(actor.get("identity"), "actor.identity")?;
        let actor_type = require_header_text(actor.get("type"), "actor.type")?;
        let empty_arguments = JsonValue::Object(Map::new());
        let arguments = require_object(
            request.get("arguments").unwrap_or(&empty_arguments),
            "arguments",
        )?;
        let timeout_seconds = parse_timeout(request.get("timeout_seconds"))?;

        let mut headers = parse_auth_headers(request.get("auth_headers"))?;
        headers.insert("X-AIT-Actor".to_string(), JsonValue::String(actor_identity));
        headers.insert(
            "X-AIT-Actor-Type".to_string(),
            JsonValue::String(actor_type),
        );

        let built = build_operation(&operation, &repo_name, arguments)?;
        let url = format!("{server_url}{}", built.path_and_query);
        let http_request = json!({
            "url": url,
            "method": built.method,
            "payload": built.payload,
            "headers": headers,
            "timeout_seconds": timeout_seconds,
            "timeout_repr": timeout_seconds
                .map(|value| value.to_string())
                .unwrap_or_else(|| "None".to_string()),
        });
        Ok(Self {
            operation,
            server_url,
            method: built.method.to_string(),
            url,
            http_request,
            retry_safe: built.retry_safe,
            optional_authority: workflow_mode == "solo_local",
            response_shape: built.response_shape,
        })
    }

    fn success_envelope(
        &self,
        result: JsonValue,
        attempts: u64,
        retry_delays: Vec<f64>,
    ) -> Result<JsonValue, String> {
        let response_kind = result
            .get("response_kind")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        let payload = result.get("payload").cloned().unwrap_or(JsonValue::Null);
        if response_kind != "json" || !self.response_shape.accepts(&payload) {
            let message = if response_kind != "json" {
                format!(
                    "{} {} returned a non-JSON response for `{}`.",
                    self.method, self.url, self.operation
                )
            } else {
                format!(
                    "{} {} returned an invalid payload for `{}`; expected {}.",
                    self.method,
                    self.url,
                    self.operation,
                    self.response_shape.description()
                )
            };
            return Ok(self.error_envelope(
                json!({
                    "ok": false,
                    "error_kind": "response",
                    "method": self.method,
                    "url": self.url,
                    "message": message,
                    "response_kind": response_kind,
                    "detail": payload,
                }),
                attempts,
                retry_delays,
                false,
                false,
            ));
        }
        Ok(json!({
            "contract": AGENT_REMOTE_RUNTIME_BACKEND_CONTRACT,
            "ok": true,
            "operation": self.operation,
            "attempts": attempts,
            "retry_delays_seconds": retry_delays,
            "payload": payload,
        }))
    }

    fn error_envelope(
        &self,
        error: JsonValue,
        attempts: u64,
        retry_delays: Vec<f64>,
        retryable: bool,
        retry_exhausted: bool,
    ) -> JsonValue {
        let message = error
            .get("message")
            .and_then(JsonValue::as_str)
            .unwrap_or("Rust ait-agent remote runtime backend request failed.");
        json!({
            "contract": AGENT_REMOTE_RUNTIME_BACKEND_CONTRACT,
            "ok": false,
            "operation": self.operation,
            "attempts": attempts,
            "retry_delays_seconds": retry_delays,
            "retryable": retryable,
            "retry_exhausted": retry_exhausted,
            "message": message,
            "error": error,
        })
    }
}

struct BuiltOperation {
    method: &'static str,
    path_and_query: String,
    payload: JsonValue,
    retry_safe: bool,
    response_shape: ResponseShape,
}

fn build_operation(
    operation: &str,
    repo_name: &str,
    arguments: &Map<String, JsonValue>,
) -> Result<BuiltOperation, String> {
    let repo = encode_component(repo_name);
    let object_response = ResponseShape::Object;
    let null = JsonValue::Null;
    match operation {
        "read_task_queue" => Ok(BuiltOperation {
            method: "GET",
            path_and_query: format!("/v1/native/read/task-queue?repo_name={repo}&status=active"),
            payload: null,
            retry_safe: true,
            response_shape: object_response,
        }),
        "read_task" => {
            let task = encoded_argument(arguments, "task_id")?;
            Ok(BuiltOperation {
                method: "GET",
                path_and_query: format!("/v1/native/repositories/{repo}/read/tasks/{task}"),
                payload: null,
                retry_safe: true,
                response_shape: object_response,
            })
        }
        "read_change" => {
            let change = encoded_argument(arguments, "change_id")?;
            Ok(BuiltOperation {
                method: "GET",
                path_and_query: format!("/v1/native/repositories/{repo}/read/changes/{change}"),
                payload: null,
                retry_safe: true,
                response_shape: object_response,
            })
        }
        "read_task_audit" => {
            let task = encoded_argument(arguments, "task_id")?;
            let target_line = arguments
                .get("target_line")
                .map(|value| require_text(Some(value), "arguments.target_line"))
                .transpose()?
                .unwrap_or_else(|| "main".to_string());
            Ok(BuiltOperation {
                method: "GET",
                path_and_query: format!(
                    "/v1/native/repositories/{repo}/read/tasks/{task}/audit?target_line={}",
                    encode_component(&target_line)
                ),
                payload: null,
                retry_safe: true,
                response_shape: object_response,
            })
        }
        _ => Err(format!(
            "unsupported remote runtime backend operation `{operation}`"
        )),
    }
}

fn validate_remote_target(target: &Map<String, JsonValue>) -> Result<String, String> {
    let mode = require_text(target.get("mode"), "target.mode")?;
    if mode != "remote" {
        return Err("remote runtime backend target mode must be `remote`".to_string());
    }
    let workflow_mode = require_text(target.get("workflow_mode"), "target.workflow_mode")?;
    if !matches!(
        workflow_mode.as_str(),
        "solo_local" | "solo_remote" | "team_remote"
    ) {
        return Err(
            "remote runtime backend workflow_mode must be `solo_local`, `solo_remote`, or `team_remote`"
                .to_string(),
        );
    }
    Ok(workflow_mode)
}

fn validate_server_url(value: &str) -> Result<String, String> {
    let parsed = reqwest::Url::parse(value)
        .map_err(|error| format!("invalid remote runtime backend server URL: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(
            "remote runtime backend server URL must use HTTP(S) and include a host".to_string(),
        );
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("remote runtime backend server URL must not contain credentials".to_string());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(
            "remote runtime backend server URL must not contain a query or fragment".to_string(),
        );
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn parse_timeout(value: Option<&JsonValue>) -> Result<Option<f64>, String> {
    match value {
        None => Ok(Some(20.0)),
        Some(JsonValue::Null) => Ok(None),
        Some(value) => {
            let timeout = value.as_f64().ok_or_else(|| {
                "remote runtime backend field `timeout_seconds` must be a number or null"
                    .to_string()
            })?;
            if !timeout.is_finite() || timeout <= 0.0 {
                return Err(
                    "remote runtime backend field `timeout_seconds` must be greater than zero"
                        .to_string(),
                );
            }
            Ok(Some(timeout))
        }
    }
}

fn parse_auth_headers(value: Option<&JsonValue>) -> Result<Map<String, JsonValue>, String> {
    let mut headers = Map::new();
    let Some(value) = value else {
        return Ok(headers);
    };
    if value.is_null() {
        return Ok(headers);
    }
    for (key, value) in require_object(value, "auth_headers")? {
        let normalized = key.to_ascii_lowercase();
        let canonical = match normalized.as_str() {
            "authorization" => "Authorization",
            "x-ait-roles" => "X-AIT-Roles",
            "x-ait-repos" => "X-AIT-Repos",
            _ => {
                return Err(format!(
                    "remote runtime backend auth header `{key}` is not allowed"
                ))
            }
        };
        headers.insert(
            canonical.to_string(),
            JsonValue::String(require_header_text(Some(value), canonical)?),
        );
    }
    Ok(headers)
}

fn require_header_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    let text = require_text(value, field)?;
    if text.contains(['\r', '\n']) {
        return Err(format!(
            "remote runtime backend field `{field}` must not contain line breaks"
        ));
    }
    Ok(text)
}

fn encoded_argument(arguments: &Map<String, JsonValue>, field: &str) -> Result<String, String> {
    Ok(encode_component(&require_text(
        arguments.get(field),
        &format!("arguments.{field}"),
    )?))
}

fn encode_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~') {
            output.push(ch);
        } else {
            output.push('%');
            output.push_str(&format!("{byte:02X}"));
        }
    }
    output
}

fn retryable_execution_error(error: &JsonValue) -> bool {
    match error.get("error_kind").and_then(JsonValue::as_str) {
        Some("timeout" | "transport") => true,
        Some("http") => matches!(
            error.get("status_code").and_then(JsonValue::as_i64),
            Some(500 | 502 | 503 | 504)
        ),
        _ => false,
    }
}

fn require_object<'a>(
    value: &'a JsonValue,
    field: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("remote runtime backend field `{field}` must be an object"))
}

fn require_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    let text = value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!("remote runtime backend field `{field}` must be a non-empty string")
        })?;
    Ok(text.to_string())
}

#[cfg(test)]
mod solo_local_optional_authority_tests {
    use super::*;

    struct TransportFailureExecutor;

    impl AgentRuntimeHttpExecutor for TransportFailureExecutor {
        fn execute_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "ok": false,
                "error_kind": "transport",
                "message": "server unavailable",
            }))
        }
    }

    struct NoRetrySleeper;

    impl AgentRuntimeRetrySleeper for NoRetrySleeper {
        fn sleep(&self, _delay: Duration) {
            panic!("optional solo_local authority must not wait for a retry");
        }
    }

    fn request(operation: &str) -> JsonValue {
        json!({
            "operation": operation,
            "target": {
                "mode": "remote",
                "workflow_mode": "solo_local",
                "repo_name": "fixture",
                "remote_name": "origin",
                "server_url": "http://127.0.0.1:8088",
            },
            "actor": {"identity": "ait-agent", "type": "telegram_bot"},
            "arguments": if operation == "read_task" {
                json!({"task_id": "RT-1"})
            } else {
                json!({})
            },
        })
    }

    #[test]
    fn solo_local_remote_authority_is_narrow_and_non_retrying() {
        let plan = RemoteOperationPlan::from_request(&request("read_task_queue"))
            .expect("optional queue plan");
        assert!(plan.optional_authority);
        assert!(plan.retry_safe);

        let error = RemoteOperationPlan::from_request(&request("read_task"))
            .expect_err("manual local task read must not use the optional server");
        assert!(error.contains("only for the optional task-queue notification"));

        let result = RemoteAitRuntimeBackend::new(TransportFailureExecutor, NoRetrySleeper)
            .execute(&request("read_task_queue"))
            .expect("classified optional remote failure");
        assert_eq!(result["ok"], false);
        assert_eq!(result["attempts"], 1);
        assert_eq!(result["retryable"], true);
        assert_eq!(result["retry_exhausted"], false);
    }
}
