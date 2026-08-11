use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const EXECUTION_KIND: &str = "telegram_background_sync_state";

pub trait TelegramBackgroundSyncStatePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramBackgroundSyncStatePlanner;

impl TelegramBackgroundSyncStatePlanner for DefaultTelegramBackgroundSyncStatePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_background_sync_state_json(request)
    }
}

pub fn agent_telegram_background_sync_state_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_telegram_background_sync_state_planner(
        &DefaultTelegramBackgroundSyncStatePlanner,
        request,
    )
}

pub fn plan_with_telegram_background_sync_state_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramBackgroundSyncStatePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_background_sync_state_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "Telegram background sync state request must be an object".to_string())?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| {
        if object.contains_key("error") || object.contains_key("exception") {
            "failure".to_string()
        } else {
            "gate".to_string()
        }
    });

    match stage.as_str() {
        "work" => Ok(plan_work(object)),
        "gate" => Ok(plan_gate(object)),
        "success" => Ok(plan_success()),
        "failure" => Ok(plan_failure(object)),
        "operation_error" => Ok(plan_operation_error(object)),
        other => Err(format!(
            "unsupported Telegram background sync state stage: {other}"
        )),
    }
}

fn plan_work(object: &Map<String, JsonValue>) -> JsonValue {
    let work = background_sync_work(object);
    json!({
        "stage": "work",
        "execution_kind": EXECUTION_KIND,
        "completed": true,
        "ok": true,
        "has_work": work.has_work,
        "should_execute": work.has_work,
        "should_run": work.has_work,
        "reason": if work.has_work { "has_work" } else { "no_work" },
        "workflow_notifications_enabled": work.workflow_notifications_enabled,
    })
}

fn plan_gate(object: &Map<String, JsonValue>) -> JsonValue {
    let work = background_sync_work(object);
    let binding = binding_source(object);
    let now_epoch = optional_f64(object.get("now_epoch")).unwrap_or(0.0);
    let retry_after_epoch = optional_f64(object.get("retry_after_epoch").or_else(|| {
        binding
            .as_object()
            .and_then(|binding| binding.get("background_sync_retry_after_epoch"))
    }))
    .unwrap_or(0.0);
    let backoff_active = retry_after_epoch > now_epoch;
    let should_run = work.has_work && !backoff_active;
    let reason = if !work.has_work {
        "no_work"
    } else if backoff_active {
        "backoff_active"
    } else {
        "ready"
    };
    json!({
        "stage": "gate",
        "execution_kind": EXECUTION_KIND,
        "completed": true,
        "ok": true,
        "has_work": work.has_work,
        "should_execute": should_run,
        "should_run": should_run,
        "reason": reason,
        "backoff_active": backoff_active,
        "retry_after_epoch": if retry_after_epoch > 0.0 { json!(retry_after_epoch) } else { JsonValue::Null },
        "now_epoch": now_epoch,
        "workflow_notifications_enabled": work.workflow_notifications_enabled,
    })
}

fn plan_success() -> JsonValue {
    let patch = json!({
        "background_sync_failure_streak": 0,
        "background_sync_retry_after_epoch": JsonValue::Null,
        "background_sync_last_failure_at": JsonValue::Null,
        "background_sync_last_error": JsonValue::Null,
    });
    json!({
        "stage": "success",
        "execution_kind": EXECUTION_KIND,
        "completed": true,
        "ok": true,
        "should_patch": true,
        "patch": patch,
    })
}

fn plan_failure(object: &Map<String, JsonValue>) -> JsonValue {
    let binding = binding_source(object);
    let current_failure_streak = optional_i64(object.get("failure_streak").or_else(|| {
        binding
            .as_object()
            .and_then(|binding| binding.get("background_sync_failure_streak"))
    }))
    .unwrap_or(0)
    .max(0);
    let failure_streak = (current_failure_streak + 1).max(1);
    let retryable_error = optional_bool(object.get("retryable_error"))
        .or_else(|| optional_bool(object.get("retryable")))
        .unwrap_or(false);
    let threshold = optional_i64(object.get("backoff_threshold"))
        .unwrap_or(1)
        .max(1);
    let delay_seconds = optional_f64(object.get("backoff_delay_seconds")).unwrap_or(0.0);
    let now_epoch = optional_f64(object.get("now_epoch")).unwrap_or(0.0);
    let retry_after_epoch = if retryable_error && failure_streak >= threshold {
        Some(now_epoch + delay_seconds.max(0.0))
    } else {
        None
    };
    let error = clean_text(object.get("error"))
        .or_else(|| clean_text(object.get("exception")))
        .unwrap_or_else(|| "Telegram background sync failed.".to_string());
    let last_failure_at = clean_text(object.get("now_iso"));
    let error_kind = if retryable_error {
        "retryable_error"
    } else {
        "runtime_error"
    };
    let patch = json!({
        "background_sync_failure_streak": failure_streak,
        "background_sync_retry_after_epoch": retry_after_epoch.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "background_sync_last_failure_at": last_failure_at.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "background_sync_last_error": error,
    });
    json!({
        "stage": "failure",
        "execution_kind": EXECUTION_KIND,
        "completed": true,
        "ok": true,
        "should_patch": true,
        "patch": patch,
        "failure_streak": failure_streak,
        "retry_after_epoch": retry_after_epoch.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "retryable_error": retryable_error,
        "error_kind": error_kind,
        "error": error,
    })
}

fn plan_operation_error(object: &Map<String, JsonValue>) -> JsonValue {
    let operation = object
        .get("operation")
        .or_else(|| object.get("request"))
        .and_then(JsonValue::as_object);
    let index = optional_i64(object.get("index")).unwrap_or(0);
    let kind =
        clean_text(operation.and_then(|operation| operation.get("kind"))).unwrap_or_default();
    let retryable_error = optional_bool(object.get("retryable_error"))
        .or_else(|| optional_bool(object.get("retryable")))
        .unwrap_or(false);
    let error = clean_text(object.get("error"))
        .or_else(|| clean_text(object.get("exception")))
        .unwrap_or_else(|| "Telegram background sync operation failed.".to_string());
    let error_kind = if retryable_error {
        "retryable_error"
    } else {
        "runtime_error"
    };
    let operation_result = json!({
        "index": index,
        "kind": kind,
        "ok": false,
        "error": error,
        "error_kind": error_kind,
        "retryable_error": retryable_error,
    });
    json!({
        "stage": "operation_error",
        "execution_kind": EXECUTION_KIND,
        "completed": true,
        "ok": true,
        "operation_result": operation_result,
    })
}

struct BackgroundSyncWork {
    has_work: bool,
    workflow_notifications_enabled: bool,
}

fn background_sync_work(object: &Map<String, JsonValue>) -> BackgroundSyncWork {
    let binding = binding_source(object);
    let binding_object = binding.as_object();
    let workflow_notifications_enabled =
        optional_bool(object.get("workflow_notifications_enabled")).unwrap_or_else(|| {
            optional_bool(
                binding_object.and_then(|binding| binding.get("workflow_notifications_enabled")),
            )
            .unwrap_or(false)
        });
    BackgroundSyncWork {
        has_work: workflow_notifications_enabled,
        workflow_notifications_enabled,
    }
}

fn binding_source(object: &Map<String, JsonValue>) -> JsonValue {
    object
        .get("binding")
        .filter(|value| value.as_object().is_some())
        .cloned()
        .unwrap_or_else(|| json!({}))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = match value? {
        JsonValue::String(value) => value.trim().to_string(),
        JsonValue::Null => return None,
        other => other.to_string().trim().to_string(),
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
        JsonValue::Number(value) => value.as_i64().map(|value| value != 0),
        JsonValue::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        },
        JsonValue::Null => None,
        _ => None,
    }
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(value) => value
            .as_i64()
            .or_else(|| value.as_f64().map(|value| value as i64)),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        JsonValue::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(value) => value.as_f64(),
        JsonValue::String(value) => value.trim().parse::<f64>().ok(),
        JsonValue::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        _ => None,
    }
}
