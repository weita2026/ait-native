use super::*;

pub(in crate::primitives) fn workflow_root_text(value: &JsonValue, key: &str) -> Option<String> {
    string_field(value, key)
}

pub(in crate::primitives) fn workflow_nested_text(
    value: &JsonValue,
    outer: &str,
    key: &str,
) -> Option<String> {
    value
        .get(outer)
        .and_then(|payload| string_field(payload, key))
}

pub(in crate::primitives) fn workflow_apply_phase_payload_json(
    phase: &str,
    code: &str,
    detail: Option<&str>,
    resumed_from_authoritative_state: bool,
) -> JsonValue {
    workflow_apply_phase_payload(phase, code, detail, resumed_from_authoritative_state)
        .unwrap_or_else(|_| {
            json!({
                "phase": phase,
                "code": code,
                "detail": detail,
                "resumed_from_authoritative_state": resumed_from_authoritative_state,
            })
        })
}

#[expect(
    clippy::too_many_arguments,
    reason = "progress arguments map directly to the stable workflow event payload"
)]
pub(in crate::primitives) fn workflow_progress_emit<F>(
    progress: &mut Option<F>,
    status: &str,
    code: &str,
    change_id: Option<&str>,
    patchset_id: Option<&str>,
    step_number: Option<usize>,
    detail: Option<&str>,
    phase: Option<&str>,
    reason: Option<&str>,
    estimated_wait: Option<&JsonValue>,
    summary: Option<&str>,
) -> Result<(), String>
where
    F: FnMut(&JsonValue) -> Result<(), String>,
{
    let Some(progress) = progress.as_mut() else {
        return Ok(());
    };
    let mut payload = JsonMap::from_iter([
        ("status".to_string(), JsonValue::String(status.to_string())),
        ("code".to_string(), JsonValue::String(code.to_string())),
    ]);
    if let Some(change_id) = change_id.and_then(|value| normalized_text(Some(value))) {
        payload.insert("change_id".to_string(), JsonValue::String(change_id));
    }
    if let Some(patchset_id) = patchset_id.and_then(|value| normalized_text(Some(value))) {
        payload.insert("patchset_id".to_string(), JsonValue::String(patchset_id));
    }
    if let Some(step_number) = step_number {
        payload.insert(
            "step_number".to_string(),
            JsonValue::from(step_number as i64),
        );
    }
    if let Some(detail) = detail.and_then(|value| normalized_text(Some(value))) {
        payload.insert("detail".to_string(), JsonValue::String(detail));
    }
    if let Some(phase) = phase.and_then(|value| normalized_text(Some(value))) {
        payload.insert("phase".to_string(), JsonValue::String(phase));
    }
    if let Some(reason) = reason.and_then(|value| normalized_text(Some(value))) {
        payload.insert("reason".to_string(), JsonValue::String(reason));
    }
    if let Some(estimated_wait) = estimated_wait.cloned() {
        payload.insert("estimated_wait".to_string(), estimated_wait);
    }
    if let Some(summary) = summary.and_then(|value| normalized_text(Some(value))) {
        payload.insert("summary".to_string(), JsonValue::String(summary));
    }
    progress(&JsonValue::Object(payload))
}

pub(in crate::primitives) fn workflow_current_ids(
    state: &JsonValue,
) -> (Option<String>, Option<String>) {
    (
        workflow_nested_text(state, "change", "change_id"),
        workflow_nested_text(state, "patchset", "patchset_id"),
    )
}

pub(in crate::primitives) fn workflow_json_text(value: Option<&JsonValue>) -> Option<String> {
    normalized_text(value.and_then(JsonValue::as_str))
}

pub(in crate::primitives) fn workflow_wait_for_pending_state<F>(
    repo: &RepoRuntime,
    initial_state: &JsonValue,
    pending_code: &str,
    mut read_state: F,
) -> Result<JsonValue, String>
where
    F: FnMut() -> Result<JsonValue, String>,
{
    let _wait_range = perfetto_range!("ait.workflow_ready.wait");
    let max_wait_seconds = {
        let _range = perfetto_range!("ait.workflow_ready.wait.hint");
        workflow_wait_seconds_hint(repo, pending_code, initial_state)?
    };
    let deadline = Instant::now() + Duration::from_secs_f64(max_wait_seconds);
    let mut latest_state = initial_state.clone();
    let mut first_probe = true;
    loop {
        let latest_code =
            workflow_nested_text(&latest_state, "next_action", "code").unwrap_or_default();
        if latest_code != pending_code {
            return Ok(latest_state);
        }
        let now = Instant::now();
        if !first_probe && now >= deadline {
            return Ok(latest_state);
        }
        if !first_probe {
            let remaining = deadline.saturating_duration_since(now);
            let sleep_duration = remaining.min(Duration::from_secs_f64(
                WORKFLOW_APPLY_FOREGROUND_WAIT_POLL_SECONDS,
            ));
            if sleep_duration > Duration::ZERO {
                let _range = perfetto_range!("ait.workflow_ready.wait.sleep");
                sleep(sleep_duration);
            }
        }
        first_probe = false;
        latest_state = {
            let _range = perfetto_range!("ait.workflow_ready.wait.poll");
            read_state()?
        };
    }
}
