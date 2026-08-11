use std::io;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::{AgentEventLoopBackendPort, AgentEventLoopRegistrationPort};
use crate::platform::{native_socket_from_u64, NativeSocket};

const MIGRATION_STAGE: &str = "rust_agent_websocket_registration_action_execution";
const REGISTRATION_CONTRACT: &str = "ait_agent_core.event_loop.WebSocketRegistrationActions.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegistrationOperationKind {
    RegisterReadable,
    RegisterReadWrite,
    Unregister,
}

impl RegistrationOperationKind {
    fn label(self) -> &'static str {
        match self {
            Self::RegisterReadable => "register_readable",
            Self::RegisterReadWrite => "register_read_write",
            Self::Unregister => "unregister",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistrationOperation {
    kind: RegistrationOperationKind,
    token: u64,
    fd: Option<NativeSocket>,
    source_action_kind: String,
    worker_key: Option<String>,
    shard_index: Option<u64>,
}

pub fn agent_websocket_registration_action_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let parsed = parse_registration_action_request(object);
    let skipped_action_count = parsed.skipped_action_count;
    match parsed.into_result() {
        Ok(operations) => Ok(base_payload(
            object,
            "plan",
            if operations.is_empty() {
                "idle"
            } else {
                "operations_planned"
            },
            json!({
                "ok": true,
                "executed": false,
                "operation_count": operations.len(),
                "skipped_action_count": skipped_action_count,
                "operations": operations_json(&operations),
                "operation_results": [],
                "diagnostics": [],
                "actions": [],
            }),
        )),
        Err(diagnostics) => Ok(configuration_error_payload(
            object,
            skipped_action_count,
            diagnostics,
        )),
    }
}

pub fn execute_agent_websocket_registration_actions<E>(
    event_loop: &mut E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: AgentEventLoopBackendPort + AgentEventLoopRegistrationPort + ?Sized,
{
    let object = request_object(request)?;
    let parsed = parse_registration_action_request(object);
    let skipped_action_count = parsed.skipped_action_count;
    let operations = match parsed.into_result() {
        Ok(operations) => operations,
        Err(diagnostics) => {
            return Ok(configuration_error_payload(
                object,
                skipped_action_count,
                diagnostics,
            ));
        }
    };

    let mut operation_results = Vec::new();
    for (operation_index, operation) in operations.iter().enumerate() {
        match execute_operation(event_loop, operation) {
            Ok(()) => operation_results.push(operation_result_json(
                operation_index,
                operation,
                "applied",
                JsonValue::Null,
            )),
            Err(err) => {
                let message = format!(
                    "WebSocket registration operation `{}` for token {} failed: {err}",
                    operation.kind.label(),
                    operation.token
                );
                operation_results.push(operation_result_json(
                    operation_index,
                    operation,
                    "failed",
                    JsonValue::String(message.clone()),
                ));
                return Ok(base_payload(
                    object,
                    "execute",
                    "execution_error",
                    json!({
                        "ok": false,
                        "executed": true,
                        "backend": event_loop.backend().label(),
                        "operation_count": operations.len(),
                        "applied_operation_count": operation_index,
                        "skipped_action_count": skipped_action_count,
                        "operations": operations_json(&operations),
                        "operation_results": operation_results,
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
        "execute",
        if operations.is_empty() {
            "idle"
        } else {
            "operations_applied"
        },
        json!({
            "ok": true,
            "executed": true,
            "backend": event_loop.backend().label(),
            "operation_count": operations.len(),
            "applied_operation_count": operation_results.len(),
            "skipped_action_count": skipped_action_count,
            "operations": operations_json(&operations),
            "operation_results": operation_results,
            "diagnostics": [],
            "actions": [],
        }),
    ))
}

fn execute_operation<E>(event_loop: &mut E, operation: &RegistrationOperation) -> io::Result<()>
where
    E: AgentEventLoopRegistrationPort + ?Sized,
{
    match operation.kind {
        RegistrationOperationKind::RegisterReadable => {
            event_loop.register_readable(operation.token, required_operation_fd(operation)?)
        }
        RegistrationOperationKind::RegisterReadWrite => {
            event_loop.register_read_write(operation.token, required_operation_fd(operation)?)
        }
        RegistrationOperationKind::Unregister => event_loop.unregister(operation.token),
    }
}

fn required_operation_fd(operation: &RegistrationOperation) -> io::Result<NativeSocket> {
    operation.fd.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "registration operation requires fd",
        )
    })
}

struct ParsedRegistrationActions {
    operations: Vec<RegistrationOperation>,
    diagnostics: Vec<String>,
    skipped_action_count: usize,
}

impl ParsedRegistrationActions {
    fn into_result(self) -> Result<Vec<RegistrationOperation>, Vec<String>> {
        if self.diagnostics.is_empty() {
            Ok(self.operations)
        } else {
            Err(self.diagnostics)
        }
    }
}

fn parse_registration_action_request(object: &Map<String, JsonValue>) -> ParsedRegistrationActions {
    let mut parsed = ParsedRegistrationActions {
        operations: Vec::new(),
        diagnostics: Vec::new(),
        skipped_action_count: 0,
    };
    let action_values = action_values_from_request(object);
    for (action_index, action) in action_values.iter().enumerate() {
        parse_registration_action(action, action_index, None, &mut parsed);
    }
    parsed
}

fn action_values_from_request(object: &Map<String, JsonValue>) -> Vec<JsonValue> {
    if let Some(actions) = object.get("actions").and_then(JsonValue::as_array) {
        return actions.clone();
    }
    for key in ["shard_batch_plan", "batch_plan", "turn_plan"] {
        if let Some(actions) = object
            .get(key)
            .and_then(JsonValue::as_object)
            .and_then(|plan| plan.get("actions"))
            .and_then(JsonValue::as_array)
        {
            return actions.clone();
        }
    }
    if let Some(action) = object.get("action") {
        return vec![action.clone()];
    }
    Vec::new()
}

fn parse_registration_action(
    action: &JsonValue,
    action_index: usize,
    wrapper_context: Option<&Map<String, JsonValue>>,
    parsed: &mut ParsedRegistrationActions,
) {
    let Some(action_object) = action.as_object() else {
        parsed.diagnostics.push(format!(
            "WebSocket registration action at index {action_index} must be an object."
        ));
        return;
    };
    if clean_text(action_object.get("kind")).as_deref() == Some("websocket_shard_worker_action") {
        if let Some(inner) = action_object.get("action") {
            parse_registration_action(inner, action_index, Some(action_object), parsed);
        } else {
            parsed.diagnostics.push(format!(
                "WebSocket shard worker action at index {action_index} is missing nested action."
            ));
        }
        return;
    }

    let Some(kind) = clean_text(action_object.get("kind")) else {
        parsed.diagnostics.push(format!(
            "WebSocket registration action at index {action_index} is missing kind."
        ));
        return;
    };
    let Some(operation_kind) = operation_kind_for_action(&kind, action_object) else {
        parsed.skipped_action_count += 1;
        return;
    };
    match parse_operation(
        action_object,
        wrapper_context,
        &kind,
        operation_kind,
        action_index,
    ) {
        Ok(operation) => parsed.operations.push(operation),
        Err(reason) => parsed.diagnostics.push(reason),
    }
}

fn operation_kind_for_action(
    kind: &str,
    action_object: &Map<String, JsonValue>,
) -> Option<RegistrationOperationKind> {
    match kind {
        "register_websocket_readable" | "keep_websocket_readable_registered" => {
            Some(RegistrationOperationKind::RegisterReadable)
        }
        "register_websocket_read_write" | "keep_websocket_read_write_registered" => {
            Some(RegistrationOperationKind::RegisterReadWrite)
        }
        "unregister_websocket_readable" | "unregister_websocket" => {
            Some(RegistrationOperationKind::Unregister)
        }
        "register_websocket" | "keep_websocket_registered" => {
            match registration_interest(action_object).as_deref() {
                Some("read_write" | "readable_writable" | "readable+writable") => {
                    Some(RegistrationOperationKind::RegisterReadWrite)
                }
                Some("readable" | "read") | None => {
                    Some(RegistrationOperationKind::RegisterReadable)
                }
                Some(_) => None,
            }
        }
        _ => None,
    }
}

fn parse_operation(
    action_object: &Map<String, JsonValue>,
    wrapper_context: Option<&Map<String, JsonValue>>,
    source_action_kind: &str,
    kind: RegistrationOperationKind,
    action_index: usize,
) -> Result<RegistrationOperation, String> {
    let registration = action_object
        .get("registration")
        .and_then(JsonValue::as_object);
    let token = token_from(action_object, registration).ok_or_else(|| {
        format!(
            "WebSocket registration action `{source_action_kind}` at index {action_index} is missing event_loop_token."
        )
    })?;
    let fd = match kind {
        RegistrationOperationKind::Unregister => fd_from(action_object, registration)
            .transpose()
            .map_err(|reason| format!("{reason} in action `{source_action_kind}` at index {action_index}."))?,
        RegistrationOperationKind::RegisterReadable | RegistrationOperationKind::RegisterReadWrite => {
            Some(
                fd_from(action_object, registration)
                    .transpose()
                    .map_err(|reason| {
                        format!("{reason} in action `{source_action_kind}` at index {action_index}.")
                    })?
                    .ok_or_else(|| {
                        format!(
                            "WebSocket registration action `{source_action_kind}` at index {action_index} is missing websocket_fd."
                        )
                    })?,
            )
        }
    };
    Ok(RegistrationOperation {
        kind,
        token,
        fd,
        source_action_kind: source_action_kind.to_string(),
        worker_key: clean_text(action_object.get("worker_key"))
            .or_else(|| {
                registration.and_then(|registration| clean_text(registration.get("worker_key")))
            })
            .or_else(|| wrapper_context.and_then(|context| clean_text(context.get("worker_key")))),
        shard_index: optional_u64(action_object.get("shard_index"))
            .or_else(|| {
                registration.and_then(|registration| optional_u64(registration.get("shard_index")))
            })
            .or_else(|| {
                wrapper_context.and_then(|context| optional_u64(context.get("shard_index")))
            }),
    })
}

fn token_from(
    action_object: &Map<String, JsonValue>,
    registration: Option<&Map<String, JsonValue>>,
) -> Option<u64> {
    optional_u64(action_object.get("event_loop_token"))
        .or_else(|| optional_u64(action_object.get("token")))
        .or_else(|| {
            registration.and_then(|registration| optional_u64(registration.get("event_loop_token")))
        })
        .or_else(|| registration.and_then(|registration| optional_u64(registration.get("token"))))
}

fn fd_from(
    action_object: &Map<String, JsonValue>,
    registration: Option<&Map<String, JsonValue>>,
) -> Option<Result<NativeSocket, String>> {
    optional_u64(action_object.get("websocket_fd"))
        .or_else(|| optional_u64(action_object.get("fd")))
        .or_else(|| {
            registration.and_then(|registration| optional_u64(registration.get("websocket_fd")))
        })
        .or_else(|| registration.and_then(|registration| optional_u64(registration.get("fd"))))
        .map(raw_fd_from_u64)
}

fn raw_fd_from_u64(raw: u64) -> Result<NativeSocket, String> {
    native_socket_from_u64(raw)
        .map_err(|_| "WebSocket registration fd is outside native socket range".to_string())
}

fn registration_interest(action_object: &Map<String, JsonValue>) -> Option<String> {
    action_object
        .get("registration")
        .and_then(JsonValue::as_object)
        .and_then(|registration| clean_text(registration.get("interest")))
        .or_else(|| clean_text(action_object.get("interest")))
        .map(|interest| interest.trim().to_ascii_lowercase().replace('-', "_"))
}

fn operations_json(operations: &[RegistrationOperation]) -> JsonValue {
    JsonValue::Array(
        operations
            .iter()
            .enumerate()
            .map(|(operation_index, operation)| operation_json(operation_index, operation))
            .collect(),
    )
}

fn operation_json(operation_index: usize, operation: &RegistrationOperation) -> JsonValue {
    json!({
        "operation_index": operation_index,
        "operation": operation.kind.label(),
        "event_loop_token": operation.token,
        "websocket_fd": operation.fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "source_action_kind": operation.source_action_kind,
        "worker_key": operation.worker_key.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "shard_index": operation.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
    })
}

fn operation_result_json(
    operation_index: usize,
    operation: &RegistrationOperation,
    status: &str,
    error: JsonValue,
) -> JsonValue {
    let mut object = operation_json(operation_index, operation)
        .as_object()
        .cloned()
        .unwrap_or_default();
    object.insert("status".to_string(), JsonValue::String(status.to_string()));
    object.insert("error".to_string(), error);
    JsonValue::Object(object)
}

fn configuration_error_payload(
    object: &Map<String, JsonValue>,
    skipped_action_count: usize,
    diagnostics: Vec<String>,
) -> JsonValue {
    base_payload(
        object,
        "execute",
        "configuration_error",
        json!({
            "ok": false,
            "executed": false,
            "operation_count": 0,
            "applied_operation_count": 0,
            "skipped_action_count": skipped_action_count,
            "operations": [],
            "operation_results": [],
            "diagnostics": diagnostics,
            "error": "WebSocket registration actions were invalid.",
            "actions": [],
        }),
    )
}

fn base_payload(
    object: &Map<String, JsonValue>,
    stage: &str,
    state: &str,
    payload: JsonValue,
) -> JsonValue {
    let mut object_out = payload.as_object().cloned().unwrap_or_default();
    object_out.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object_out.insert(
        "websocket_registration_contract".to_string(),
        JsonValue::String(REGISTRATION_CONTRACT.to_string()),
    );
    object_out.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object_out.insert(
        "websocket_registration_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object_out.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object_out.insert(
        "python_websocket_registration_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object_out.insert(
        "python_fallback_allowed".to_string(),
        JsonValue::Bool(false),
    );
    if let Some(backend) = clean_text(object.get("backend")) {
        object_out
            .entry("backend".to_string())
            .or_insert(JsonValue::String(backend));
    }
    if let Some(shard_index) = optional_u64(object.get("shard_index")) {
        object_out
            .entry("shard_index".to_string())
            .or_insert(JsonValue::from(shard_index));
    }
    JsonValue::Object(object_out)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket registration action request must be an object.".to_string())
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
