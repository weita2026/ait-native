use std::collections::HashSet;
use std::io;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::{
    AgentEventLoopBackend, AgentEventLoopBackendPort, AgentEventLoopUnregistrationPort,
};
use crate::platform::{
    close_native_socket, native_socket_from_u64, native_socket_is_valid, shutdown_native_socket,
    NativeSocket, INVALID_NATIVE_SOCKET,
};

const MIGRATION_STAGE: &str = "rust_agent_websocket_lifecycle_action_execution";
const LIFECYCLE_CONTRACT: &str = "ait_agent_core.event_loop.WebSocketLifecycleActions.v1";

pub trait WebSocketLifecycleExecutor {
    fn close_websocket_fd(&mut self, fd: NativeSocket) -> io::Result<()>;
}

#[derive(Debug, Default)]
pub struct DefaultWebSocketLifecycleExecutor;

impl WebSocketLifecycleExecutor for DefaultWebSocketLifecycleExecutor {
    fn close_websocket_fd(&mut self, fd: NativeSocket) -> io::Result<()> {
        if !native_socket_is_valid(fd) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "websocket native socket must be valid",
            ));
        }
        let _ = shutdown_native_socket(fd);
        close_native_socket(fd)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum LifecycleOperationKind {
    Unregister,
    CloseSocket,
    Reconnect,
    StopRuntime,
}

impl LifecycleOperationKind {
    fn label(self) -> &'static str {
        match self {
            Self::Unregister => "unregister",
            Self::CloseSocket => "close_socket",
            Self::Reconnect => "reconnect",
            Self::StopRuntime => "stop_runtime",
        }
    }
}

#[derive(Debug, Clone)]
struct LifecycleOperation {
    kind: LifecycleOperationKind,
    token: Option<u64>,
    fd: Option<NativeSocket>,
    transport: Option<String>,
    reason: Option<String>,
    delay_seconds: Option<f64>,
    source_action_kind: String,
    worker_key: Option<String>,
    shard_index: Option<u64>,
}

#[derive(Debug, Clone, Default)]
struct ActionContext {
    token: Option<u64>,
    fd: Option<NativeSocket>,
    transport: Option<String>,
    worker_key: Option<String>,
    shard_index: Option<u64>,
}

#[derive(Debug, Clone)]
struct ParsedLifecycleActions {
    operations: Vec<LifecycleOperation>,
    diagnostics: Vec<String>,
    skipped_action_count: usize,
}

impl ParsedLifecycleActions {
    fn new() -> Self {
        Self {
            operations: Vec::new(),
            diagnostics: Vec::new(),
            skipped_action_count: 0,
        }
    }

    fn push_operation(&mut self, operation: LifecycleOperation) {
        self.operations.push(operation);
    }

    fn into_result(mut self) -> Result<(Vec<LifecycleOperation>, usize), Vec<String>> {
        if !self.diagnostics.is_empty() {
            return Err(self.diagnostics);
        }
        let mut seen = HashSet::new();
        self.operations
            .retain(|operation| seen.insert(operation_key(operation)));
        Ok((self.operations, self.skipped_action_count))
    }
}

pub fn execute_agent_websocket_lifecycle_actions<E>(
    event_loop: &mut E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: AgentEventLoopBackendPort + AgentEventLoopUnregistrationPort + ?Sized,
{
    let mut executor = DefaultWebSocketLifecycleExecutor;
    execute_with_websocket_lifecycle_executor(event_loop, &mut executor, request)
}

pub fn execute_with_websocket_lifecycle_executor<E, X>(
    event_loop: &mut E,
    lifecycle_executor: &mut X,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: AgentEventLoopBackendPort + AgentEventLoopUnregistrationPort + ?Sized,
    X: WebSocketLifecycleExecutor + ?Sized,
{
    let object = request_object(request)?;
    let parsed = parse_lifecycle_action_request(object);
    let (operations, skipped_action_count) = match parsed.into_result() {
        Ok(parsed) => parsed,
        Err(diagnostics) => {
            return Ok(configuration_error_payload(
                object,
                event_loop.backend(),
                diagnostics,
            ));
        }
    };

    let mut operation_results = Vec::new();
    let mut reconnect_requests = Vec::new();
    let mut stop_requests = Vec::new();
    for (operation_index, operation) in operations.iter().enumerate() {
        match execute_operation(
            event_loop,
            lifecycle_executor,
            operation,
            &mut reconnect_requests,
            &mut stop_requests,
        ) {
            Ok(status) => operation_results.push(operation_result_json(
                operation_index,
                operation,
                status,
                JsonValue::Null,
            )),
            Err(err) => {
                let message = format!(
                    "WebSocket lifecycle operation `{}` failed: {err}",
                    operation.kind.label()
                );
                operation_results.push(operation_result_json(
                    operation_index,
                    operation,
                    "failed",
                    JsonValue::String(message.clone()),
                ));
                return Ok(base_payload(
                    object,
                    event_loop.backend(),
                    "execution_error",
                    json!({
                        "ok": false,
                        "executed": true,
                        "operation_count": operations.len(),
                        "applied_operation_count": operation_index,
                        "skipped_action_count": skipped_action_count,
                        "unregister_operation_count": count_operations(&operations, LifecycleOperationKind::Unregister),
                        "close_operation_count": count_operations(&operations, LifecycleOperationKind::CloseSocket),
                        "reconnect_operation_count": count_operations(&operations, LifecycleOperationKind::Reconnect),
                        "stop_operation_count": count_operations(&operations, LifecycleOperationKind::StopRuntime),
                        "operations": operations_json(&operations),
                        "operation_results": operation_results,
                        "reconnect_requests": reconnect_requests,
                        "stop_requests": stop_requests,
                        "diagnostics": [message.clone()],
                        "error": message,
                        "actions": [],
                    }),
                ));
            }
        }
    }

    Ok(base_payload(
        object,
        event_loop.backend(),
        if operations.is_empty() {
            "idle"
        } else {
            "operations_applied"
        },
        json!({
            "ok": true,
            "executed": true,
            "operation_count": operations.len(),
            "applied_operation_count": operation_results.len(),
            "skipped_action_count": skipped_action_count,
            "unregister_operation_count": count_operations(&operations, LifecycleOperationKind::Unregister),
            "close_operation_count": count_operations(&operations, LifecycleOperationKind::CloseSocket),
            "reconnect_operation_count": count_operations(&operations, LifecycleOperationKind::Reconnect),
            "stop_operation_count": count_operations(&operations, LifecycleOperationKind::StopRuntime),
            "operations": operations_json(&operations),
            "operation_results": operation_results,
            "reconnect_requests": reconnect_requests,
            "stop_requests": stop_requests,
            "diagnostics": [],
            "actions": [],
        }),
    ))
}

fn execute_operation<E, X>(
    event_loop: &mut E,
    lifecycle_executor: &mut X,
    operation: &LifecycleOperation,
    reconnect_requests: &mut Vec<JsonValue>,
    stop_requests: &mut Vec<JsonValue>,
) -> io::Result<&'static str>
where
    E: AgentEventLoopUnregistrationPort + ?Sized,
    X: WebSocketLifecycleExecutor + ?Sized,
{
    match operation.kind {
        LifecycleOperationKind::Unregister => {
            event_loop.unregister(required_token(operation)?)?;
            Ok("applied")
        }
        LifecycleOperationKind::CloseSocket => {
            lifecycle_executor.close_websocket_fd(required_fd(operation)?)?;
            Ok("applied")
        }
        LifecycleOperationKind::Reconnect => {
            reconnect_requests.push(reconnect_request_json(operation));
            Ok("projected")
        }
        LifecycleOperationKind::StopRuntime => {
            stop_requests.push(stop_request_json(operation));
            Ok("projected")
        }
    }
}

fn required_token(operation: &LifecycleOperation) -> io::Result<u64> {
    operation.token.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "lifecycle operation requires event-loop token",
        )
    })
}

fn required_fd(operation: &LifecycleOperation) -> io::Result<NativeSocket> {
    operation.fd.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "lifecycle operation requires websocket fd",
        )
    })
}

fn parse_lifecycle_action_request(object: &Map<String, JsonValue>) -> ParsedLifecycleActions {
    let mut parsed = ParsedLifecycleActions::new();
    parse_action_source(
        &JsonValue::Object(object.clone()),
        &ActionContext::default(),
        &mut parsed,
    );
    parsed
}

fn parse_action_source(
    value: &JsonValue,
    context: &ActionContext,
    parsed: &mut ParsedLifecycleActions,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    let source_context = context.with_action(object);
    if clean_text(object.get("kind")).is_some() {
        parse_lifecycle_action(value, &source_context, parsed, 0);
        return;
    }
    if let Some(action) = object.get("action") {
        parse_lifecycle_action(action, &source_context, parsed, 0);
    }
    if let Some(actions) = object.get("actions").and_then(JsonValue::as_array) {
        for (action_index, action) in actions.iter().enumerate() {
            parse_lifecycle_action(action, &source_context, parsed, action_index);
        }
    }
    if let Some(connections) = object
        .get("final_connections")
        .or_else(|| object.get("connection_state_updates"))
        .and_then(JsonValue::as_array)
    {
        for (connection_index, connection) in connections.iter().enumerate() {
            parse_final_connection_state(connection, &source_context, connection_index, parsed);
        }
    }
    for key in [
        "reactor_run_result",
        "run_result",
        "websocket_reactor_run",
        "last_tick",
        "reactor_tick_result",
        "tick_result",
        "websocket_reactor_tick",
        "shard_batch_plan",
        "batch_plan",
        "turn_plan",
        "runtime_plan",
        "registration_result",
    ] {
        if let Some(nested) = object.get(key) {
            parse_action_source(nested, &source_context, parsed);
        }
    }
}

fn parse_lifecycle_action(
    action: &JsonValue,
    context: &ActionContext,
    parsed: &mut ParsedLifecycleActions,
    action_index: usize,
) {
    let Some(action_object) = action.as_object() else {
        parsed.diagnostics.push(format!(
            "WebSocket lifecycle action at index {action_index} must be an object."
        ));
        return;
    };
    let Some(kind) = clean_text(action_object.get("kind")) else {
        parsed.diagnostics.push(format!(
            "WebSocket lifecycle action at index {action_index} is missing kind."
        ));
        return;
    };
    if kind == "websocket_shard_worker_action" {
        let wrapper_context = context.with_action(action_object);
        if let Some(inner) = action_object.get("action") {
            parse_lifecycle_action(inner, &wrapper_context, parsed, action_index);
        } else {
            parsed.diagnostics.push(format!(
                "WebSocket shard worker lifecycle action at index {action_index} is missing nested action."
            ));
        }
        return;
    }

    let operation = match kind.as_str() {
        "close_websocket" => {
            let fd = action_fd(action_object).or(context.fd);
            if fd.is_none() || fd.is_some_and(|fd| !native_socket_is_valid(fd)) {
                parsed.diagnostics.push(format!(
                    "WebSocket lifecycle close action at index {action_index} is missing a non-negative websocket_fd."
                ));
                return;
            }
            LifecycleOperation {
                kind: LifecycleOperationKind::CloseSocket,
                token: action_token(action_object).or(context.token),
                fd,
                transport: action_transport(action_object).or_else(|| context.transport.clone()),
                reason: action_reason(action_object),
                delay_seconds: None,
                source_action_kind: kind,
                worker_key: action_worker_key(action_object).or_else(|| context.worker_key.clone()),
                shard_index: action_shard_index(action_object).or(context.shard_index),
            }
        }
        "unregister_websocket_readable" | "unregister_websocket" => {
            let token = action_token(action_object).or(context.token);
            if token.is_none() {
                parsed.diagnostics.push(format!(
                    "WebSocket lifecycle unregister action at index {action_index} is missing event_loop_token."
                ));
                return;
            }
            LifecycleOperation {
                kind: LifecycleOperationKind::Unregister,
                token,
                fd: action_fd(action_object).or(context.fd),
                transport: action_transport(action_object).or_else(|| context.transport.clone()),
                reason: action_reason(action_object),
                delay_seconds: None,
                source_action_kind: kind,
                worker_key: action_worker_key(action_object).or_else(|| context.worker_key.clone()),
                shard_index: action_shard_index(action_object).or(context.shard_index),
            }
        }
        "reconnect_socket_mode" | "reconnect_gateway" | "reconnect_websocket" => {
            LifecycleOperation {
                kind: LifecycleOperationKind::Reconnect,
                token: action_token(action_object).or(context.token),
                fd: action_fd(action_object).or(context.fd),
                transport: reconnect_transport(&kind, action_object, context),
                reason: action_reason(action_object),
                delay_seconds: optional_f64(action_object.get("delay_seconds"))
                    .or_else(|| optional_f64(action_object.get("reconnect_delay_seconds"))),
                source_action_kind: kind,
                worker_key: action_worker_key(action_object).or_else(|| context.worker_key.clone()),
                shard_index: action_shard_index(action_object).or(context.shard_index),
            }
        }
        "stop_socket_mode_runtime" | "stop_websocket_runtime" | "stop_gateway_runtime" => {
            LifecycleOperation {
                kind: LifecycleOperationKind::StopRuntime,
                token: action_token(action_object).or(context.token),
                fd: action_fd(action_object).or(context.fd),
                transport: action_transport(action_object).or_else(|| context.transport.clone()),
                reason: action_reason(action_object),
                delay_seconds: None,
                source_action_kind: kind,
                worker_key: action_worker_key(action_object).or_else(|| context.worker_key.clone()),
                shard_index: action_shard_index(action_object).or(context.shard_index),
            }
        }
        _ => {
            parsed.skipped_action_count += 1;
            return;
        }
    };
    parsed.push_operation(operation);
}

fn parse_final_connection_state(
    connection: &JsonValue,
    parent_context: &ActionContext,
    connection_index: usize,
    parsed: &mut ParsedLifecycleActions,
) {
    let Some(object) = connection.as_object() else {
        parsed.diagnostics.push(format!(
            "WebSocket lifecycle final connection at index {connection_index} must be an object."
        ));
        return;
    };
    let context = parent_context.with_action(object);
    if bool_field(object.get("should_unregister")).unwrap_or(false) {
        if context.token.is_none() {
            parsed.diagnostics.push(format!(
                "WebSocket lifecycle final connection at index {connection_index} is missing event_loop_token for unregister."
            ));
        } else {
            parsed.push_operation(LifecycleOperation {
                kind: LifecycleOperationKind::Unregister,
                token: context.token,
                fd: context.fd,
                transport: context.transport.clone(),
                reason: clean_text(object.get("websocket_turn_state"))
                    .or_else(|| clean_text(object.get("last_websocket_turn_state"))),
                delay_seconds: None,
                source_action_kind: "final_connection_should_unregister".to_string(),
                worker_key: context.worker_key.clone(),
                shard_index: context.shard_index,
            });
        }
    }
    if bool_field(object.get("should_close_websocket")).unwrap_or(false) {
        if context.fd.is_none() || context.fd.is_some_and(|fd| !native_socket_is_valid(fd)) {
            parsed.diagnostics.push(format!(
                "WebSocket lifecycle final connection at index {connection_index} is missing a non-negative websocket_fd for close."
            ));
        } else {
            parsed.push_operation(LifecycleOperation {
                kind: LifecycleOperationKind::CloseSocket,
                token: context.token,
                fd: context.fd,
                transport: context.transport.clone(),
                reason: clean_text(object.get("websocket_turn_state"))
                    .or_else(|| clean_text(object.get("last_websocket_turn_state"))),
                delay_seconds: None,
                source_action_kind: "final_connection_should_close".to_string(),
                worker_key: context.worker_key.clone(),
                shard_index: context.shard_index,
            });
        }
    }
    if bool_field(object.get("should_reconnect")).unwrap_or(false) {
        parsed.push_operation(LifecycleOperation {
            kind: LifecycleOperationKind::Reconnect,
            token: context.token,
            fd: context.fd,
            transport: context.transport.clone(),
            reason: clean_text(object.get("websocket_turn_state"))
                .or_else(|| clean_text(object.get("last_websocket_turn_state"))),
            delay_seconds: optional_f64(object.get("delay_seconds"))
                .or_else(|| optional_f64(object.get("reconnect_delay_seconds"))),
            source_action_kind: "final_connection_should_reconnect".to_string(),
            worker_key: context.worker_key,
            shard_index: context.shard_index,
        });
    }
}

impl ActionContext {
    fn with_action(&self, object: &Map<String, JsonValue>) -> Self {
        Self {
            token: action_token(object).or(self.token),
            fd: action_fd(object).or(self.fd),
            transport: action_transport(object).or_else(|| self.transport.clone()),
            worker_key: action_worker_key(object).or_else(|| self.worker_key.clone()),
            shard_index: action_shard_index(object).or(self.shard_index),
        }
    }
}

fn action_token(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(object.get("event_loop_token"))
        .or_else(|| optional_u64(object.get("token")))
        .or_else(|| nested_u64(object, "worker_lease", "token"))
        .or_else(|| nested_u64(object, "registration", "token"))
        .or_else(|| nested_u64(object, "event_loop_registration", "token"))
        .or_else(|| nested_u64(object, "event", "token"))
}

fn action_fd(object: &Map<String, JsonValue>) -> Option<NativeSocket> {
    optional_u64(object.get("websocket_fd"))
        .or_else(|| optional_u64(object.get("fd")))
        .or_else(|| nested_u64(object, "worker_lease", "fd"))
        .or_else(|| nested_u64(object, "registration", "fd"))
        .or_else(|| nested_u64(object, "event_loop_registration", "fd"))
        .or_else(|| nested_u64(object, "event", "fd"))
        .and_then(|fd| native_socket_from_u64(fd).ok())
}

fn action_transport(object: &Map<String, JsonValue>) -> Option<String> {
    clean_text(object.get("transport"))
        .or_else(|| clean_text(object.get("websocket_transport")))
        .or_else(|| nested_text(object, "worker_lease", "transport"))
        .or_else(|| nested_text(object, "registration", "transport"))
        .or_else(|| nested_text(object, "event_loop_registration", "transport"))
}

fn action_worker_key(object: &Map<String, JsonValue>) -> Option<String> {
    clean_text(object.get("worker_key"))
        .or_else(|| clean_text(object.get("key")))
        .or_else(|| nested_text(object, "worker_lease", "worker_key"))
        .or_else(|| nested_text(object, "worker_lease", "key"))
        .or_else(|| nested_text(object, "registration", "worker_key"))
        .or_else(|| nested_text(object, "event_loop_registration", "worker_key"))
}

fn action_shard_index(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(object.get("shard_index"))
        .or_else(|| nested_u64(object, "worker_lease", "shard_index"))
        .or_else(|| nested_u64(object, "registration", "shard_index"))
        .or_else(|| nested_u64(object, "event_loop_registration", "shard_index"))
}

fn action_reason(object: &Map<String, JsonValue>) -> Option<String> {
    clean_text(object.get("reason"))
        .or_else(|| clean_text(object.get("reconnect_reason")))
        .or_else(|| clean_text(object.get("error")))
}

fn reconnect_transport(
    kind: &str,
    object: &Map<String, JsonValue>,
    context: &ActionContext,
) -> Option<String> {
    action_transport(object)
        .or_else(|| context.transport.clone())
        .or_else(|| match kind {
            "reconnect_socket_mode" => Some("slack".to_string()),
            "reconnect_gateway" => Some("discord".to_string()),
            _ => None,
        })
}

fn operation_key(operation: &LifecycleOperation) -> String {
    match operation.kind {
        LifecycleOperationKind::Unregister => {
            format!("unregister:{}", operation.token.unwrap_or_default())
        }
        LifecycleOperationKind::CloseSocket => {
            format!("close:{}", operation.fd.unwrap_or(INVALID_NATIVE_SOCKET))
        }
        LifecycleOperationKind::Reconnect => format!(
            "reconnect:{}:{}:{}:{}",
            operation.transport.as_deref().unwrap_or(""),
            operation
                .token
                .map(|value| value.to_string())
                .unwrap_or_default(),
            operation.worker_key.as_deref().unwrap_or(""),
            operation.reason.as_deref().unwrap_or("")
        ),
        LifecycleOperationKind::StopRuntime => format!(
            "stop:{}:{}:{}",
            operation.transport.as_deref().unwrap_or(""),
            operation.worker_key.as_deref().unwrap_or(""),
            operation.reason.as_deref().unwrap_or("")
        ),
    }
}

fn operations_json(operations: &[LifecycleOperation]) -> JsonValue {
    JsonValue::Array(
        operations
            .iter()
            .enumerate()
            .map(|(operation_index, operation)| {
                json!({
                    "operation_index": operation_index,
                    "kind": operation.kind.label(),
                    "source_action_kind": operation.source_action_kind,
                    "event_loop_token": operation.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "websocket_fd": operation.fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "transport": operation.transport.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "worker_key": operation.worker_key.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "shard_index": operation.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "reason": operation.reason.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "delay_seconds": operation.delay_seconds.map(JsonValue::from).unwrap_or(JsonValue::Null),
                })
            })
            .collect(),
    )
}

fn operation_result_json(
    operation_index: usize,
    operation: &LifecycleOperation,
    status: &str,
    error: JsonValue,
) -> JsonValue {
    json!({
        "operation_index": operation_index,
        "kind": operation.kind.label(),
        "source_action_kind": operation.source_action_kind,
        "status": status,
        "event_loop_token": operation.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "websocket_fd": operation.fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "transport": operation.transport.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "worker_key": operation.worker_key.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": operation.reason.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "error": error,
    })
}

fn reconnect_request_json(operation: &LifecycleOperation) -> JsonValue {
    json!({
        "kind": "reconnect_websocket",
        "source_action_kind": operation.source_action_kind,
        "transport": operation.transport.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "worker_key": operation.worker_key.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "event_loop_token": operation.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "websocket_fd": operation.fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "shard_index": operation.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": operation.reason.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "delay_seconds": operation.delay_seconds.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "execute_connect": false,
    })
}

fn stop_request_json(operation: &LifecycleOperation) -> JsonValue {
    json!({
        "kind": "stop_websocket_runtime",
        "source_action_kind": operation.source_action_kind,
        "transport": operation.transport.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "worker_key": operation.worker_key.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "event_loop_token": operation.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": operation.reason.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
    })
}

fn configuration_error_payload(
    object: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    diagnostics: Vec<String>,
) -> JsonValue {
    base_payload(
        object,
        backend,
        "configuration_error",
        json!({
            "ok": false,
            "executed": false,
            "operation_count": 0,
            "applied_operation_count": 0,
            "skipped_action_count": 0,
            "unregister_operation_count": 0,
            "close_operation_count": 0,
            "reconnect_operation_count": 0,
            "stop_operation_count": 0,
            "operations": [],
            "operation_results": [],
            "reconnect_requests": [],
            "stop_requests": [],
            "diagnostics": diagnostics,
            "error": diagnostics.first().cloned().unwrap_or_else(|| "WebSocket lifecycle configuration error.".to_string()),
            "actions": [{
                "kind": "diagnose_websocket_lifecycle_configuration_error",
            }],
        }),
    )
}

fn base_payload(
    object: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    state: &str,
    payload: JsonValue,
) -> JsonValue {
    let mut output = payload.as_object().cloned().unwrap_or_default();
    output.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    output.insert(
        "websocket_lifecycle_contract".to_string(),
        JsonValue::String(LIFECYCLE_CONTRACT.to_string()),
    );
    output.insert(
        "stage".to_string(),
        JsonValue::String("execute".to_string()),
    );
    output.insert(
        "websocket_lifecycle_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    output.insert(
        "backend".to_string(),
        JsonValue::String(backend.label().to_string()),
    );
    if let Some(shard_index) = optional_u64(object.get("shard_index")) {
        output.insert("shard_index".to_string(), JsonValue::from(shard_index));
    }
    output.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    output.insert(
        "python_websocket_lifecycle_allowed".to_string(),
        JsonValue::Bool(false),
    );
    output.insert(
        "python_websocket_event_loop_allowed".to_string(),
        JsonValue::Bool(false),
    );
    output.insert(
        "python_websocket_reactor_allowed".to_string(),
        JsonValue::Bool(false),
    );
    output.insert(
        "python_websocket_registration_allowed".to_string(),
        JsonValue::Bool(false),
    );
    output.insert(
        "python_fallback_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(output)
}

fn count_operations(operations: &[LifecycleOperation], kind: LifecycleOperationKind) -> usize {
    operations
        .iter()
        .filter(|operation| operation.kind == kind)
        .count()
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket lifecycle request must be an object.".to_string())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = match value? {
        JsonValue::String(text) => text.trim().to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => return None,
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn bool_field(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        },
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_u64(value: Option<&JsonValue>) -> Option<u64> {
    match value? {
        JsonValue::Number(number) => number.as_u64(),
        JsonValue::String(text) => text.trim().parse::<u64>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => text.trim().parse::<f64>().ok(),
        JsonValue::Bool(_) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn nested_text(object: &Map<String, JsonValue>, parent: &str, key: &str) -> Option<String> {
    object
        .get(parent)
        .and_then(JsonValue::as_object)
        .and_then(|nested| clean_text(nested.get(key)))
}

fn nested_u64(object: &Map<String, JsonValue>, parent: &str, key: &str) -> Option<u64> {
    object
        .get(parent)
        .and_then(JsonValue::as_object)
        .and_then(|nested| optional_u64(nested.get(key)))
}
