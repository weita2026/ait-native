use super::helpers::{normalize_execution_profile, optional_text, required_text};
use serde_json::{json, Value as JsonValue};

pub fn patchset_ci_active_state_json(request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "patchset-ci active-state payload must be a JSON object.".to_string())?;
    let patchset_ci = payload.get("patchset_ci").and_then(JsonValue::as_object);
    let Some(patchset_ci) = patchset_ci else {
        return Ok(json!({"active_state": JsonValue::Null}));
    };
    let requested_profile =
        normalize_execution_profile(payload.get("requested_execution_profile"))?;
    let queue_mode = payload
        .get("queue_mode")
        .and_then(optional_text)
        .unwrap_or_else(|| "inline".to_string());
    let inline_thread_alive = payload
        .get("inline_thread_alive")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let job_state = patchset_ci
        .get("job_state")
        .and_then(optional_text)
        .unwrap_or_default();
    let tests_status = patchset_ci
        .get("tests_status")
        .and_then(optional_text)
        .unwrap_or_default();
    if !matches!(job_state.as_str(), "queued" | "running" | "pending") && tests_status != "pending"
    {
        return Ok(json!({"active_state": JsonValue::Null}));
    }
    let active_profile = normalize_execution_profile(patchset_ci.get("execution_profile"))?;
    if active_profile != requested_profile {
        return Ok(json!({"active_state": JsonValue::Null}));
    }
    let patchset_id = required_text(payload, "patchset_id")?;
    let trigger = patchset_ci
        .get("trigger")
        .and_then(optional_text)
        .unwrap_or_else(|| "existing_active".to_string());
    if queue_mode == "inline" && !inline_thread_alive {
        return Ok(json!({"active_state": JsonValue::Null}));
    }
    Ok(json!({
        "active_state": {
            "patchset_id": patchset_id,
            "queued": true,
            "job": {
                "job_type": "patchset.ci",
                "state": if job_state.is_empty() { "pending" } else { &job_state },
                "delivery": "existing_active",
                "execution_profile": active_profile,
            },
            "trigger": trigger,
            "execution_profile": requested_profile,
            "idempotency": {
                "state": "reused_active_patchset_ci",
                "patchset_id": patchset_id,
            },
        }
    }))
}
