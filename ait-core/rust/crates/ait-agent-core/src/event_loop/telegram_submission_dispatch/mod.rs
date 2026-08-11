use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{
    agent_telegram_update_dispatch_plan_json, TelegramServiceCycleDispatchPort,
    TelegramSubmissionRuntime, TelegramWebhookTransactionDispatchPort,
};

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramSubmissionDispatchPort.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_submission_dispatch_port";
const DISPATCH_FAILURE: &str = "Telegram submission dispatch failed.";
const STOP_FAILURE: &str = "Telegram submission dispatch stop failed.";
const IDLE_FAILURE: &str = "Telegram submission dispatch idle wait failed.";

#[derive(Clone)]
pub struct TelegramSubmissionDispatchPort {
    runtime: Arc<TelegramSubmissionRuntime>,
}

impl TelegramSubmissionDispatchPort {
    pub fn new(runtime: Arc<TelegramSubmissionRuntime>) -> Self {
        Self { runtime }
    }

    pub fn request_stop(&self) -> Result<(), String> {
        self.runtime
            .request_stop()
            .map_err(|_| STOP_FAILURE.to_string())
    }

    pub fn wait_for_idle(&self, timeout: Option<Duration>) -> Result<bool, String> {
        let idle = self
            .runtime
            .wait_for_idle(timeout)
            .map_err(|_| IDLE_FAILURE.to_string())?;
        if idle && snapshot_has_failures(&self.runtime.snapshot_json()) {
            return Err(IDLE_FAILURE.to_string());
        }
        Ok(idle)
    }

    pub fn snapshot_json(&self) -> JsonValue {
        let runtime = self.runtime.snapshot_json();
        json!({
            "contract": CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "transport": "telegram",
            "stopped": bool_field(&runtime, "stopped"),
            "submitted_planned_update_count": count_field(
                &runtime,
                "submitted_planned_update_count"
            ),
            "handled_update_count": count_field(&runtime, "handled_update_count"),
            "handled_logical_turn_count": count_field(
                &runtime,
                "handled_logical_turn_count"
            ),
            "skipped_duplicate_count": count_field(&runtime, "skipped_duplicate_count"),
            "execution_failure_count": count_field(&runtime, "execution_failure_count"),
            "inflight_count": count_field(&runtime, "dispatch_inflight_count"),
            "queued_count": count_field(&runtime, "dispatch_queued_count"),
            "running_count": count_field(&runtime, "dispatch_running_count"),
            "failed_count": count_field(&runtime, "dispatch_failed_count"),
            "panicked_count": count_field(&runtime, "dispatch_panicked_count"),
            "rust_submission_runtime_required": true,
            "python_dispatch_allowed": false,
            "python_callback_execution_allowed": false,
            "python_future_tracking_allowed": false,
        })
    }

    fn dispatch(&self, request: &JsonValue, origin: DispatchOrigin) -> Result<(), String> {
        let parsed = ParsedDispatchRequest::parse(request, origin)
            .map_err(|_| DISPATCH_FAILURE.to_string())?;
        self.runtime
            .submit_planned_update(parsed.update, parsed.dispatch_item)
            .map(|_future| ())
            .map_err(|_| DISPATCH_FAILURE.to_string())
    }
}

impl fmt::Debug for TelegramSubmissionDispatchPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramSubmissionDispatchPort")
            .field("runtime", &"<shared Rust submission runtime>")
            .finish_non_exhaustive()
    }
}

impl TelegramServiceCycleDispatchPort for TelegramSubmissionDispatchPort {
    fn dispatch_update(&self, request: &JsonValue) -> Result<(), String> {
        self.dispatch(request, DispatchOrigin::Polling)
    }
}

impl TelegramWebhookTransactionDispatchPort for TelegramSubmissionDispatchPort {
    fn dispatch_update(&self, request: &JsonValue) -> Result<(), String> {
        self.dispatch(request, DispatchOrigin::Webhook)
    }
}

#[derive(Clone, Copy)]
enum DispatchOrigin {
    Polling,
    Webhook,
}

struct ParsedDispatchRequest {
    update: JsonValue,
    dispatch_item: JsonValue,
}

impl ParsedDispatchRequest {
    fn parse(request: &JsonValue, origin: DispatchOrigin) -> Result<Self, ()> {
        let request = request.as_object().ok_or(())?;
        let fallback_update_key = validate_origin(request, origin)?;
        let update = request
            .get("update")
            .filter(|value| value.is_object())
            .ok_or(())?;
        let dispatch_item = request
            .get("dispatch_item")
            .and_then(JsonValue::as_object)
            .ok_or(())?;
        let planned = agent_telegram_update_dispatch_plan_json(&json!({
            "update": update,
            "fallback_update_key": fallback_update_key,
        }))
        .map_err(|_| ())?;
        let planned = planned.as_object().ok_or(())?;

        validate_index(request, dispatch_item)?;
        validate_identity_fields(dispatch_item, planned)?;
        let planned_queue_key = clean_text(planned.get("dispatch_key")).ok_or(())?;
        let planned_update_key = clean_text(planned.get("update_key")).ok_or(())?;
        if clean_text(request.get("queue_key")).as_deref() != Some(planned_queue_key.as_str())
            || clean_text(request.get("update_key")).as_deref() != Some(planned_update_key.as_str())
        {
            return Err(());
        }
        if matches!(origin, DispatchOrigin::Webhook)
            && clean_text(request.get("dispatch_key")).as_deref()
                != Some(planned_queue_key.as_str())
        {
            return Err(());
        }

        Ok(Self {
            update: update.clone(),
            dispatch_item: JsonValue::Object(dispatch_item.clone()),
        })
    }
}

fn validate_origin(request: &Map<String, JsonValue>, origin: DispatchOrigin) -> Result<String, ()> {
    match origin {
        DispatchOrigin::Polling => {
            if clean_text(request.get("callback_kind")).as_deref() != Some("dispatch_update")
                || clean_text(request.get("callback_group")).as_deref() != Some("dispatch")
            {
                return Err(());
            }
            clean_text(request.get("update_key")).ok_or(())
        }
        DispatchOrigin::Webhook => {
            if clean_text(request.get("source")).as_deref() != Some("telegram_webhook") {
                return Err(());
            }
            clean_text(request.get("fallback_update_key")).ok_or(())
        }
    }
}

fn validate_index(
    request: &Map<String, JsonValue>,
    dispatch_item: &Map<String, JsonValue>,
) -> Result<(), ()> {
    let request_index = nonnegative_i64(request.get("index")).ok_or(())?;
    let dispatch_index = nonnegative_i64(dispatch_item.get("index")).ok_or(())?;
    (request_index == dispatch_index).then_some(()).ok_or(())
}

fn validate_identity_fields(
    dispatch_item: &Map<String, JsonValue>,
    planned: &Map<String, JsonValue>,
) -> Result<(), ()> {
    for field in [
        "chat_id",
        "dispatch_key",
        "update_id",
        "message_id",
        "should_update_last_update_id",
        "update_key",
    ] {
        if dispatch_item.get(field) != planned.get(field) {
            return Err(());
        }
    }
    Ok(())
}

fn snapshot_has_failures(snapshot: &JsonValue) -> bool {
    [
        "execution_failure_count",
        "dispatch_failed_count",
        "dispatch_panicked_count",
    ]
    .iter()
    .any(|field| count_field(snapshot, field) > 0)
}

fn count_field(value: &JsonValue, field: &str) -> u64 {
    value.get(field).and_then(JsonValue::as_u64).unwrap_or(0)
}

fn bool_field(value: &JsonValue, field: &str) -> bool {
    value
        .get(field)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn nonnegative_i64(value: Option<&JsonValue>) -> Option<i64> {
    let value = match value? {
        JsonValue::Number(value) => value.as_i64(),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }?;
    (value >= 0).then_some(value)
}

#[cfg(test)]
mod tests;
