use ait_core::json_support::{json, JsonValue};

use crate::runtime::AgentRuntimeBindingStore;

use super::{
    agent_telegram_service_runtime_shell_plan_json, agent_telegram_service_shell_callback_plan_json,
};

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramServiceCycleExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_service_cycle_execution";

pub trait TelegramServiceCycleStatePort {
    fn execute_state(
        &self,
        path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String>;
}

pub trait TelegramServiceCyclePollPort {
    fn poll_updates(&self, request: &JsonValue) -> Result<Vec<JsonValue>, String>;
}

pub trait TelegramServiceCycleDispatchPort {
    fn dispatch_update(&self, request: &JsonValue) -> Result<(), String>;
}

pub trait TelegramServiceCycleBackgroundSyncPort {
    fn run_background_sync_once(&self, request: &JsonValue) -> Result<usize, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramServiceCycleStatePort;

impl TelegramServiceCycleStatePort for DefaultTelegramServiceCycleStatePort {
    fn execute_state(
        &self,
        path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String> {
        AgentRuntimeBindingStore::new(path).execute(operation, request)
    }
}

pub fn execute_with_telegram_service_cycle_ports<S, P, D, B>(
    state: &S,
    poll: &P,
    dispatch: &D,
    background_sync: &B,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    S: TelegramServiceCycleStatePort + ?Sized,
    P: TelegramServiceCyclePollPort + ?Sized,
    D: TelegramServiceCycleDispatchPort + ?Sized,
    B: TelegramServiceCycleBackgroundSyncPort + ?Sized,
{
    let input = CycleInput::parse(request)?;
    let mut progress = CycleProgress {
        next_background_sync_at: input.next_background_sync_at,
        ..CycleProgress::default()
    };
    let loaded = match state.execute_state(&input.state_path, "load", &json!({})) {
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => {
            return Ok(cycle_payload(
                &progress,
                "state_load_failed",
                false,
                Some("state"),
            ))
        }
    };
    progress.last_update_id_before = optional_i64(loaded.get("last_update_id"))
        .unwrap_or(0)
        .max(0);
    progress.last_update_id_after = progress.last_update_id_before;

    let poll_shell = match agent_telegram_service_runtime_shell_plan_json(&json!({
        "stage": "poll",
        "last_update_id": progress.last_update_id_before,
        "poll_timeout_seconds": input.poll_timeout_seconds,
        "background_sync_enabled": input.background_sync_enabled,
        "background_sync_interval_seconds": input.background_sync_interval_seconds,
        "now_monotonic_seconds": input.now_monotonic_seconds,
        "next_background_sync_at": optional_f64_json(input.next_background_sync_at),
    })) {
        Ok(plan) if plan.is_object() => plan,
        Ok(_) | Err(_) => {
            return Ok(cycle_payload(
                &progress,
                "poll_plan_invalid",
                false,
                Some("contract"),
            ))
        }
    };
    progress.background_sync_due = poll_shell
        .get("background_sync")
        .and_then(|value| value.get("due"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    progress.next_background_sync_at = optional_f64(poll_shell.get("next_background_sync_at"));
    let Some(actions) = poll_shell.get("actions").and_then(JsonValue::as_array) else {
        return Ok(cycle_payload(
            &progress,
            "poll_plan_invalid",
            false,
            Some("contract"),
        ));
    };

    let mut updates = None;
    for action in actions {
        let callback = match callback_request(action, None) {
            Ok(callback) => callback,
            Err(_) => {
                return Ok(cycle_payload(
                    &progress,
                    "poll_callback_invalid",
                    false,
                    Some("contract"),
                ))
            }
        };
        let callback_kind = clean_text(callback.get("callback_kind")).unwrap_or_default();
        let callback_request = callback
            .get("request")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        match callback_kind.as_str() {
            "run_background_sync_once" => {
                let future_count = match background_sync.run_background_sync_once(&callback_request)
                {
                    Ok(count) => count,
                    Err(_) => {
                        return Ok(cycle_payload(
                            &progress,
                            "background_sync_failed",
                            false,
                            Some("background_sync"),
                        ))
                    }
                };
                if callback_result(action, json!({"future_count": future_count})).is_err() {
                    return Ok(cycle_payload(
                        &progress,
                        "background_sync_result_invalid",
                        false,
                        Some("contract"),
                    ));
                }
                progress.background_sync_ran = true;
                progress.background_future_count = future_count;
            }
            "poll_updates" => {
                let polled = match poll.poll_updates(&callback_request) {
                    Ok(updates) => updates,
                    Err(_) => {
                        return Ok(cycle_payload(&progress, "poll_failed", false, Some("poll")))
                    }
                };
                let result = match callback_result(action, json!({"updates": polled})) {
                    Ok(result) => result,
                    Err(_) => {
                        return Ok(cycle_payload(
                            &progress,
                            "poll_result_invalid",
                            false,
                            Some("contract"),
                        ))
                    }
                };
                let normalized = result
                    .get("result")
                    .and_then(|value| value.get("updates"))
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default();
                progress.polled = true;
                progress.update_count = normalized.len();
                updates = Some(normalized);
            }
            _ => {
                return Ok(cycle_payload(
                    &progress,
                    "poll_callback_invalid",
                    false,
                    Some("contract"),
                ))
            }
        }
    }
    let Some(updates) = updates else {
        return Ok(cycle_payload(
            &progress,
            "poll_callback_missing",
            false,
            Some("contract"),
        ));
    };
    if updates.is_empty() {
        return Ok(cycle_payload(&progress, "empty_poll", true, None));
    }

    let updates_shell = match agent_telegram_service_runtime_shell_plan_json(&json!({
        "stage": "updates",
        "updates": updates,
    })) {
        Ok(plan) if plan.is_object() => plan,
        Ok(_) | Err(_) => {
            return Ok(cycle_payload(
                &progress,
                "updates_plan_invalid",
                false,
                Some("contract"),
            ))
        }
    };
    let Some(actions) = updates_shell.get("actions").and_then(JsonValue::as_array) else {
        return Ok(cycle_payload(
            &progress,
            "updates_plan_invalid",
            false,
            Some("contract"),
        ));
    };
    for action in actions {
        let callback = match callback_request(action, Some(&updates)) {
            Ok(callback) => callback,
            Err(_) => {
                return Ok(cycle_payload(
                    &progress,
                    "updates_callback_invalid",
                    false,
                    Some("contract"),
                ))
            }
        };
        let callback_kind = clean_text(callback.get("callback_kind")).unwrap_or_default();
        let callback_request = callback
            .get("request")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        match callback_kind.as_str() {
            "dispatch_update" => {
                if dispatch.dispatch_update(&callback_request).is_err() {
                    return Ok(cycle_payload(
                        &progress,
                        "dispatch_failed",
                        false,
                        Some("dispatch"),
                    ));
                }
                let result = callback_result(action, json!({"submitted": true}));
                if result
                    .as_ref()
                    .ok()
                    .and_then(|value| value.get("completed").and_then(JsonValue::as_bool))
                    != Some(true)
                {
                    return Ok(cycle_payload(
                        &progress,
                        "dispatch_result_invalid",
                        false,
                        Some("contract"),
                    ));
                }
                progress.dispatched_count += 1;
            }
            "update_last_update_id" => {
                let last_update_id = optional_i64(callback_request.get("last_update_id"))
                    .unwrap_or(0)
                    .max(0);
                let stored = match state.execute_state(
                    &input.state_path,
                    "update_last_update_id",
                    &json!({"update_id": last_update_id}),
                ) {
                    Ok(value) if value.is_object() => value,
                    Ok(_) | Err(_) => {
                        return Ok(cycle_payload(
                            &progress,
                            "cursor_update_failed",
                            false,
                            Some("state"),
                        ))
                    }
                };
                let stored_last_update_id =
                    optional_i64(stored.get("last_update_id")).unwrap_or(-1);
                let result =
                    callback_result(action, json!({"last_update_id": stored_last_update_id}));
                if stored_last_update_id != last_update_id
                    || result
                        .as_ref()
                        .ok()
                        .and_then(|value| value.get("completed").and_then(JsonValue::as_bool))
                        != Some(true)
                {
                    return Ok(cycle_payload(
                        &progress,
                        "cursor_result_invalid",
                        false,
                        Some("contract"),
                    ));
                }
                progress.cursor_updated = true;
                progress.last_update_id_after = stored_last_update_id;
            }
            _ => {
                return Ok(cycle_payload(
                    &progress,
                    "updates_callback_invalid",
                    false,
                    Some("contract"),
                ))
            }
        }
    }

    Ok(cycle_payload(&progress, "completed", true, None))
}

fn callback_request(
    action: &JsonValue,
    updates: Option<&[JsonValue]>,
) -> Result<JsonValue, String> {
    let mut request = json!({
        "stage": "request",
        "action": action,
    });
    if let Some(updates) = updates {
        request["updates"] = JsonValue::Array(updates.to_vec());
    }
    let planned = agent_telegram_service_shell_callback_plan_json(&request)?;
    if clean_text(planned.get("execution_kind")).as_deref()
        != Some("telegram_service_shell_callback")
        || planned.get("should_execute").and_then(JsonValue::as_bool) != Some(true)
        || !planned.get("request").is_some_and(JsonValue::is_object)
    {
        return Err("Telegram service shell callback request contract is invalid.".to_string());
    }
    Ok(planned)
}

fn callback_result(action: &JsonValue, callback_result: JsonValue) -> Result<JsonValue, String> {
    let planned = agent_telegram_service_shell_callback_plan_json(&json!({
        "stage": "result",
        "action": action,
        "callback_result": callback_result,
    }))?;
    if clean_text(planned.get("execution_kind")).as_deref()
        != Some("telegram_service_shell_callback")
        || planned.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || !planned.get("result").is_some_and(JsonValue::is_object)
    {
        return Err("Telegram service shell callback result contract is invalid.".to_string());
    }
    Ok(planned)
}

#[derive(Default)]
struct CycleProgress {
    last_update_id_before: i64,
    last_update_id_after: i64,
    polled: bool,
    update_count: usize,
    dispatched_count: usize,
    cursor_updated: bool,
    background_sync_due: bool,
    background_sync_ran: bool,
    background_future_count: usize,
    next_background_sync_at: Option<f64>,
}

fn cycle_payload(
    progress: &CycleProgress,
    state: &str,
    ok: bool,
    error_kind: Option<&str>,
) -> JsonValue {
    let error = error_kind.map(|kind| match kind {
        "state" => "Telegram service-cycle state execution failed.",
        "poll" => "Telegram service-cycle polling failed.",
        "dispatch" => "Telegram service-cycle update dispatch failed.",
        "background_sync" => "Telegram service-cycle background sync failed.",
        _ => "Telegram service-cycle contract validation failed.",
    });
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "service_cycle_state": state,
        "ok": ok,
        "completed": ok,
        "polled": progress.polled,
        "update_count": progress.update_count,
        "dispatched_count": progress.dispatched_count,
        "last_update_id_before": progress.last_update_id_before,
        "last_update_id_after": progress.last_update_id_after,
        "cursor_updated": progress.cursor_updated,
        "background_sync_due": progress.background_sync_due,
        "background_sync_ran": progress.background_sync_ran,
        "background_future_count": progress.background_future_count,
        "next_background_sync_at": optional_f64_json(progress.next_background_sync_at),
        "error_kind": error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "error": error.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "python_service_loop_allowed": false,
        "python_callback_execution_allowed": false,
        "python_state_mutation_allowed": false,
        "python_telegram_api_allowed": false,
        "python_update_dispatch_allowed": false,
        "python_background_sync_allowed": false,
    })
}

struct CycleInput {
    state_path: String,
    poll_timeout_seconds: i64,
    background_sync_enabled: bool,
    background_sync_interval_seconds: f64,
    now_monotonic_seconds: f64,
    next_background_sync_at: Option<f64>,
}

impl CycleInput {
    fn parse(request: &JsonValue) -> Result<Self, String> {
        let object = request
            .as_object()
            .ok_or_else(|| "Telegram service-cycle request must be an object.".to_string())?;
        let state_path = clean_text(object.get("state_path"))
            .ok_or_else(|| "Telegram service-cycle state_path is required.".to_string())?;
        let poll_timeout_seconds = optional_i64(object.get("poll_timeout_seconds")).unwrap_or(0);
        if poll_timeout_seconds < 0 {
            return Err(
                "Telegram service-cycle poll_timeout_seconds must not be negative.".to_string(),
            );
        }
        let background_sync_interval_seconds =
            optional_f64(object.get("background_sync_interval_seconds")).unwrap_or(0.0);
        let now_monotonic_seconds =
            optional_f64(object.get("now_monotonic_seconds")).unwrap_or(0.0);
        let next_background_sync_at = optional_f64(object.get("next_background_sync_at"));
        for (name, value) in [
            (
                "background_sync_interval_seconds",
                background_sync_interval_seconds,
            ),
            ("now_monotonic_seconds", now_monotonic_seconds),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(format!(
                    "Telegram service-cycle {name} must be finite and non-negative."
                ));
            }
        }
        if next_background_sync_at.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(
                "Telegram service-cycle next_background_sync_at must be finite and non-negative."
                    .to_string(),
            );
        }
        Ok(Self {
            state_path,
            poll_timeout_seconds,
            background_sync_enabled: object
                .get("background_sync_enabled")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            background_sync_interval_seconds,
            now_monotonic_seconds,
            next_background_sync_at,
        })
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?.as_str()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(value) => value.as_i64(),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(value) => value.as_f64(),
        JsonValue::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn optional_f64_json(value: Option<f64>) -> JsonValue {
    value
        .and_then(ait_core::json_support::JsonNumber::from_f64)
        .map(JsonValue::Number)
        .unwrap_or(JsonValue::Null)
}

#[cfg(test)]
mod tests;
