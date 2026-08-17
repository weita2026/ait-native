use crate::json_support::JsonValue as Value;
use crate::land_json::LandJson;
use crate::task_json::TaskJson;
use crate::task_workflow_remote_traits::TaskWorkflowHttpClientError;
use std::env;
use std::time::Duration;

pub(super) fn change_matches_reference(row: &Value, change_ref: &str) -> bool {
    row.get("change_id").and_then(Value::as_str) == Some(change_ref)
        || row.get("change_ref").and_then(Value::as_str) == Some(change_ref)
        || row.get("published_change_id").and_then(Value::as_str) == Some(change_ref)
}

pub(super) fn change_read_error_allows_listing_recovery(
    error: &TaskWorkflowHttpClientError,
) -> bool {
    !error.is_retryable_busy()
}

pub(super) fn remote_mutation_response_deadline_timeout_ms() -> Option<u64> {
    duration_from_env(
        crate::environment_contract::names::AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS,
        Duration::from_secs(10),
    )
    .and_then(duration_to_timeout_ms)
}

pub(super) fn remote_task_land_response_deadline_timeout_ms() -> Option<u64> {
    duration_from_env(
        crate::environment_contract::names::AIT_REMOTE_MUTATION_RESPONSE_DEADLINE_SECONDS,
        Duration::from_secs(30),
    )
    .and_then(duration_to_timeout_ms)
}

pub(super) fn remote_mutation_settle_window() -> Duration {
    duration_from_env(
        crate::environment_contract::names::AIT_REMOTE_MUTATION_SETTLE_WINDOW_SECONDS,
        Duration::from_secs(5),
    )
    .unwrap_or_else(|| Duration::from_secs(5))
}

pub(super) fn remote_mutation_settle_poll() -> Duration {
    duration_from_env(
        crate::environment_contract::names::AIT_REMOTE_MUTATION_SETTLE_POLL_SECONDS,
        Duration::from_millis(250),
    )
    .unwrap_or_else(|| Duration::from_millis(250))
}

pub(super) fn duration_from_env(key: &str, default: Duration) -> Option<Duration> {
    let raw = match env::var(key) {
        Ok(value) => value,
        Err(_) => return Some(default),
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Some(default);
    }
    let parsed = match trimmed.parse::<f64>() {
        Ok(value) => value,
        Err(_) => return Some(default),
    };
    if parsed <= 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(parsed))
}

pub(super) fn duration_to_timeout_ms(duration: Duration) -> Option<u64> {
    let millis = duration.as_millis();
    if millis == 0 {
        return Some(1);
    }
    u64::try_from(millis).ok().map(|value| value.max(1))
}

pub(super) fn is_remote_mutation_timeout(err: &TaskWorkflowHttpClientError) -> bool {
    matches!(err, TaskWorkflowHttpClientError::Transport(message) if {
        let lowered = message.to_ascii_lowercase();
        lowered.contains("timed out") || lowered.contains("timeout")
    })
}

pub(super) fn task_close_error_needs_landed_settle(err: &TaskWorkflowHttpClientError) -> bool {
    let message = err.to_string().to_ascii_lowercase();
    message.contains("cannot be completed while changes are still open")
        || message.contains("cannot be completed before remote land")
}

pub(super) fn normalize_optional_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub(super) fn change_has_landed_status(change: &Value) -> bool {
    LandJson::stateless().change_has_landed_status(change)
}

pub(super) fn change_has_landing_evidence(change: &Value) -> bool {
    LandJson::stateless().change_has_landing_evidence(change)
}

pub(super) fn recover_land_submission_from_change_state(
    change: &Value,
    fallback_change_id: &str,
) -> Option<Value> {
    LandJson::stateless().recover_land_submission_from_change_state(change, fallback_change_id)
}

pub(super) fn recover_closed_task_from_state(
    task: &Value,
    fallback_task_id: &str,
) -> Option<Value> {
    TaskJson::stateless().recover_closed_task_from_state(task, fallback_task_id)
}
