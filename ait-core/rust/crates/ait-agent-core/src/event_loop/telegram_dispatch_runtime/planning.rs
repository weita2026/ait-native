use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_telegram_dispatch_runtime";
const DISPATCH_RUNTIME_CONTRACT: &str = "ait_agent_core.event_loop.TelegramDispatchRuntime.v1";

pub trait TelegramDispatchRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramDispatchRuntimePlanner;

impl TelegramDispatchRuntimePlanner for DefaultTelegramDispatchRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_dispatch_runtime_json(request)
    }
}

pub fn agent_telegram_dispatch_runtime_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_telegram_dispatch_runtime_planner(&DefaultTelegramDispatchRuntimePlanner, request)
}

pub fn plan_with_telegram_dispatch_runtime_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramDispatchRuntimePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_dispatch_runtime_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "submit".to_string());

    match stage.as_str() {
        "configure" | "init" => plan_configure(object),
        "thread_name_prefix" => plan_thread_name_prefix(object),
        "submit" | "submit_serialized" | "submit_reply_serialized" => plan_submit(object, &stage),
        "stop" => plan_stop(object),
        other => Err(format!(
            "unsupported Telegram dispatch runtime stage: {other}"
        )),
    }
}

fn plan_configure(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let admission_plan = object.get("admission_plan").and_then(JsonValue::as_object);
    let backend = admission_plan
        .and_then(|plan| clean_text(plan.get("backend")))
        .or_else(|| clean_text(object.get("backend")))
        .unwrap_or_else(|| "portable_poll".to_string());
    let shard_index = admission_plan
        .and_then(first_admitted_shard_index)
        .or_else(|| optional_usize(object.get("shard_index")))
        .unwrap_or(0);
    let inflight_limit = admission_plan
        .and_then(first_shard_inflight_limit)
        .or_else(|| optional_usize(object.get("inflight_limit")))
        .or_else(|| {
            admission_plan
                .and_then(|plan| optional_usize(plan.get("workers_per_shard")))
                .filter(|value| *value > 0)
        })
        .unwrap_or(64)
        .max(1);

    Ok(base_payload(
        "configure",
        "configured",
        json!({
            "backend": backend,
            "shard_index": shard_index,
            "inflight_limit": inflight_limit,
            "actions": [
                {
                    "kind": "configure_dispatch_runtime",
                    "backend": backend,
                    "shard_index": shard_index,
                    "inflight_limit": inflight_limit,
                }
            ],
        }),
    ))
}

fn plan_thread_name_prefix(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let dispatcher_kind = dispatcher_kind(object, "dispatch");
    let queue_key = required_text(object, "queue_key", "queue_key is required")?;
    let backend = clean_text(object.get("backend")).unwrap_or_else(|| "portable_poll".to_string());
    let shard_index = optional_usize(object.get("shard_index")).unwrap_or(0);
    let thread_name_prefix =
        thread_name_prefix(&dispatcher_kind, &backend, shard_index, &queue_key);

    Ok(base_payload(
        "thread_name_prefix",
        "planned",
        json!({
            "dispatcher_kind": dispatcher_kind,
            "queue_key": queue_key,
            "backend": backend,
            "shard_index": shard_index,
            "thread_name_prefix": thread_name_prefix,
            "actions": [
                {
                    "kind": "thread_name_prefix",
                    "dispatcher_kind": dispatcher_kind,
                    "queue_key": queue_key,
                    "thread_name_prefix": thread_name_prefix,
                }
            ],
        }),
    ))
}

fn plan_submit(object: &Map<String, JsonValue>, stage: &str) -> Result<JsonValue, String> {
    let dispatcher_kind = dispatcher_kind(
        object,
        if stage == "submit_reply_serialized" {
            "reply"
        } else {
            "dispatch"
        },
    );
    let queue_key = required_text(object, "queue_key", "queue_key is required")?;
    let backend = clean_text(object.get("backend")).unwrap_or_else(|| "portable_poll".to_string());
    let shard_index = optional_usize(object.get("shard_index")).unwrap_or(0);
    let inflight_limit = optional_usize(object.get("inflight_limit"))
        .unwrap_or(64)
        .max(1);
    let inflight_count = optional_usize(object.get("inflight_count")).unwrap_or(0);
    let stop_requested = optional_bool(object.get("stop_requested")).unwrap_or(false);
    let has_executor = optional_bool(object.get("has_executor")).unwrap_or(false);
    let thread_name_prefix =
        thread_name_prefix(&dispatcher_kind, &backend, shard_index, &queue_key);

    if stop_requested {
        return Ok(rejected_submit(
            stage,
            "stopped",
            &dispatcher_kind,
            &queue_key,
            &backend,
            shard_index,
            inflight_count,
            inflight_limit,
            &thread_name_prefix,
            format!(
                "Telegram dispatch runtime is stopped; refusing Python fallback submission for queue {}.",
                python_repr(&queue_key)
            ),
        ));
    }

    if inflight_count >= inflight_limit {
        return Ok(rejected_submit(
            stage,
            "inflight_limit_reached",
            &dispatcher_kind,
            &queue_key,
            &backend,
            shard_index,
            inflight_count,
            inflight_limit,
            &thread_name_prefix,
            format!(
                "Telegram dispatch runtime inflight limit {inflight_limit} reached; refusing Python fallback submission for queue {}.",
                python_repr(&queue_key)
            ),
        ));
    }

    let should_create_executor = !has_executor;
    let mut actions = vec![json!({
        "kind": "reserve_inflight_slot",
        "dispatcher_kind": dispatcher_kind,
        "queue_key": queue_key,
        "inflight_count": inflight_count,
        "inflight_limit": inflight_limit,
    })];
    if should_create_executor {
        actions.push(json!({
            "kind": "ensure_executor",
            "dispatcher_kind": dispatcher_kind,
            "queue_key": queue_key,
            "thread_name_prefix": thread_name_prefix,
        }));
    }
    actions.push(json!({
        "kind": "submit_callable",
        "dispatcher_kind": dispatcher_kind,
        "queue_key": queue_key,
    }));
    actions.push(json!({
        "kind": "track_future",
        "dispatcher_kind": dispatcher_kind,
        "queue_key": queue_key,
    }));

    Ok(base_payload(
        stage,
        "accepted",
        json!({
            "dispatcher_kind": dispatcher_kind,
            "queue_key": queue_key,
            "backend": backend,
            "shard_index": shard_index,
            "inflight_count": inflight_count,
            "inflight_limit": inflight_limit,
            "thread_name_prefix": thread_name_prefix,
            "should_submit": true,
            "should_create_executor": should_create_executor,
            "should_reserve_inflight_slot": true,
            "rejection_message": JsonValue::Null,
            "actions": actions,
        }),
    ))
}

fn plan_stop(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let dispatch_queue_count = optional_usize(object.get("dispatch_queue_count")).unwrap_or(0);
    let reply_queue_count = optional_usize(object.get("reply_queue_count")).unwrap_or(0);

    Ok(base_payload(
        "stop",
        "stopped",
        json!({
            "dispatch_queue_count": dispatch_queue_count,
            "reply_queue_count": reply_queue_count,
            "should_stop": true,
            "actions": [
                {
                    "kind": "shutdown_dispatchers",
                    "dispatcher_kind": "dispatch",
                    "queue_count": dispatch_queue_count,
                },
                {
                    "kind": "shutdown_dispatchers",
                    "dispatcher_kind": "reply",
                    "queue_count": reply_queue_count,
                },
                {
                    "kind": "clear_dispatchers",
                }
            ],
        }),
    ))
}

#[allow(clippy::too_many_arguments)]
fn rejected_submit(
    stage: &str,
    state: &str,
    dispatcher_kind: &str,
    queue_key: &str,
    backend: &str,
    shard_index: usize,
    inflight_count: usize,
    inflight_limit: usize,
    thread_name_prefix: &str,
    message: String,
) -> JsonValue {
    base_payload(
        stage,
        state,
        json!({
            "dispatcher_kind": dispatcher_kind,
            "queue_key": queue_key,
            "backend": backend,
            "shard_index": shard_index,
            "inflight_count": inflight_count,
            "inflight_limit": inflight_limit,
            "thread_name_prefix": thread_name_prefix,
            "should_submit": false,
            "should_create_executor": false,
            "should_reserve_inflight_slot": false,
            "rejection_message": message,
            "actions": [],
        }),
    )
}

fn base_payload(stage: &str, state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "dispatch_runtime_contract".to_string(),
        JsonValue::String(DISPATCH_RUNTIME_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "transport".to_string(),
        JsonValue::String("telegram".to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_dispatch_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "dispatch_runtime_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    JsonValue::Object(object)
}

fn dispatcher_kind(object: &Map<String, JsonValue>, default: &str) -> String {
    clean_text(object.get("dispatcher_kind"))
        .unwrap_or_else(|| default.to_string())
        .to_ascii_lowercase()
}

fn first_admitted_shard_index(plan: &Map<String, JsonValue>) -> Option<usize> {
    plan.get("worker_leases")
        .and_then(JsonValue::as_array)
        .and_then(|leases| leases.first())
        .and_then(JsonValue::as_object)
        .and_then(|lease| optional_usize(lease.get("shard_index")))
        .or_else(|| {
            plan.get("shard_admissions")
                .and_then(JsonValue::as_array)
                .and_then(|shards| shards.first())
                .and_then(JsonValue::as_object)
                .and_then(|shard| optional_usize(shard.get("shard_index")))
        })
}

fn first_shard_inflight_limit(plan: &Map<String, JsonValue>) -> Option<usize> {
    let admitted_shard_index = first_admitted_shard_index(plan)?;
    plan.get("shard_admissions")
        .and_then(JsonValue::as_array)
        .and_then(|shards| {
            shards.iter().find(|shard| {
                optional_usize(shard.get("shard_index")) == Some(admitted_shard_index)
            })
        })
        .and_then(JsonValue::as_object)
        .and_then(|shard| optional_usize(shard.get("inflight_limit")))
        .filter(|value| *value > 0)
}

fn thread_name_prefix(
    dispatcher_kind: &str,
    backend: &str,
    shard_index: usize,
    queue_key: &str,
) -> String {
    format!("ait-telegram-{dispatcher_kind}-{backend}-s{shard_index}-{queue_key}")
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())
}

fn required_text(
    object: &Map<String, JsonValue>,
    key: &str,
    error: &str,
) -> Result<String, String> {
    clean_text(object.get(key)).ok_or_else(|| error.to_string())
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

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
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

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    match value? {
        JsonValue::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .or_else(|| {
                number
                    .as_i64()
                    .and_then(|value| usize::try_from(value).ok())
            }),
        JsonValue::String(text) => text.trim().parse::<usize>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn python_repr(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

#[cfg(test)]
mod tests;
