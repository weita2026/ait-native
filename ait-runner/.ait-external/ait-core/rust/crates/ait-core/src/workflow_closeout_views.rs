use crate::json_support::{JsonMap as Map, JsonValue as Value};

fn normalize_optional_text(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(format!(
            "Workflow closeout payload field `{}` must be non-empty.",
            field_name
        ));
    }
    Ok(normalized.to_string())
}

fn applied_action_delivery_suffix(result: &Value) -> String {
    let Some(result_map) = result.as_object() else {
        return String::new();
    };
    let mut notes: Vec<&str> = Vec::new();
    if matches!(result_map.get("response_recovery"), Some(Value::Object(_))) {
        notes.push("via authoritative recovery");
    }
    if matches!(result_map.get("selection_recovery"), Some(Value::Object(_))) {
        notes.push("with authoritative selection recovery");
    }
    if let Some(Value::Object(policy_refresh)) = result_map.get("policy_refresh") {
        if matches!(
            policy_refresh.get("response_recovery"),
            Some(Value::Object(_))
        ) {
            notes.push("with authoritative policy recovery");
        }
    }
    if notes.is_empty() {
        String::new()
    } else {
        format!(" {}", notes.join(" "))
    }
}

pub fn workflow_apply_phase_payload(
    phase: &str,
    code: &str,
    detail: Option<&str>,
    resumed_from_authoritative_state: bool,
) -> Result<Value, String> {
    let normalized_phase = normalize_required_text(phase, "phase")?;
    let normalized_code = normalize_required_text(code, "code")?;
    let mut payload = Map::from_iter([
        ("phase".to_string(), Value::String(normalized_phase)),
        ("code".to_string(), Value::String(normalized_code)),
    ]);
    if let Some(detail) = detail.map(str::trim).filter(|value| !value.is_empty()) {
        payload.insert("detail".to_string(), Value::String(detail.to_string()));
    }
    if resumed_from_authoritative_state {
        payload.insert(
            "resumed_from_authoritative_state".to_string(),
            Value::Bool(true),
        );
    }
    Ok(Value::Object(payload))
}

pub fn workflow_apply_phase_summary(apply_phase: &Value) -> Result<Option<String>, String> {
    let Some(payload) = apply_phase.as_object() else {
        return Ok(None);
    };
    let phase = normalize_optional_text(payload.get("phase")).unwrap_or_default();
    let code =
        normalize_optional_text(payload.get("code")).unwrap_or_else(|| "unknown".to_string());
    let detail = normalize_optional_text(payload.get("detail"))
        .or_else(|| normalize_optional_text(payload.get("summary")))
        .unwrap_or_default();
    let summary = match phase.as_str() {
        "authoritative_resume" => {
            if detail.is_empty() {
                format!("Resumed from authoritative `{code}` state without a new mutation.")
            } else {
                detail
            }
        }
        "pending_gate" => {
            if detail.is_empty() {
                format!("Waiting at pending helper gate `{code}`.")
            } else {
                detail
            }
        }
        "done" => {
            if detail.is_empty() {
                "Helper completed and exited normally.".to_string()
            } else {
                detail
            }
        }
        "stopped" => {
            if detail.is_empty() {
                format!("Helper stopped at `{code}`.")
            } else {
                detail
            }
        }
        "incomplete" => {
            if detail.is_empty() {
                format!("Helper ended without terminal state at `{code}`.")
            } else {
                detail
            }
        }
        _ => {
            if detail.is_empty() {
                format!("{phase} `{code}`")
            } else {
                detail
            }
        }
    };
    Ok(Some(summary))
}

pub fn workflow_mutation_receipt_summary(receipt: &Value) -> Result<String, String> {
    let receipt_map = receipt
        .as_object()
        .ok_or_else(|| "Workflow mutation receipt summary expects an object.".to_string())?;
    let action = normalize_optional_text(receipt_map.get("action"))
        .unwrap_or_else(|| "mutation".to_string());
    let delivery = normalize_optional_text(receipt_map.get("delivery"))
        .unwrap_or_else(|| "unknown".to_string());
    let mut identifiers: Vec<String> = Vec::new();
    for key in [
        "patchset_id",
        "attestation_id",
        "review_id",
        "submission_id",
        "task_id",
        "snapshot_id",
        "job_id",
    ] {
        if let Some(value) = normalize_optional_text(receipt_map.get(key)) {
            identifiers.push(format!("{key}={value}"));
        }
    }
    if let Some(value) = normalize_optional_text(receipt_map.get("status")) {
        identifiers.push(format!("status={value}"));
    }
    if let Some(value) = normalize_optional_text(receipt_map.get("decision")) {
        identifiers.push(format!("decision={value}"));
    }
    if let Some(flag) = receipt_map.get("queued").and_then(Value::as_bool) {
        identifiers.push(format!("queued={flag}"));
    }
    let suffix = if identifiers.is_empty() {
        String::new()
    } else {
        format!(" ({})", identifiers.join(", "))
    };
    if delivery == "response_recovery" {
        Ok(format!(
            "`{action}` accepted via authoritative recovery{suffix}"
        ))
    } else {
        Ok(format!("`{action}` accepted via direct response{suffix}"))
    }
}

pub fn workflow_applied_action_summary(action: &Value) -> Result<String, String> {
    let action_map = action
        .as_object()
        .ok_or_else(|| "Workflow applied action summary expects an object.".to_string())?;
    let code =
        normalize_optional_text(action_map.get("code")).unwrap_or_else(|| "action".to_string());
    let result = action_map
        .get("result")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let result_map = result.as_object().expect("result object");
    let suffix = applied_action_delivery_suffix(&result);
    let summary = match code.as_str() {
        "snapshot_create" => {
            let snapshot_id = normalize_optional_text(result_map.get("snapshot_id"))
                .unwrap_or_else(|| "unknown".to_string());
            format!("created snapshot `{snapshot_id}`")
        }
        "publish_patchset" | "refresh_patchset" => {
            let patchset_id = normalize_optional_text(result_map.get("patchset_id"))
                .unwrap_or_else(|| "unknown".to_string());
            let with_rebase = result_map
                .get("auto_rebase")
                .and_then(Value::as_object)
                .and_then(|payload| payload.get("rebase"))
                .and_then(Value::as_object);
            if let Some(rebase) = with_rebase {
                let status = normalize_optional_text(rebase.get("status"))
                    .unwrap_or_else(|| "applied".to_string());
                format!("published patchset `{patchset_id}` after auto-rebase `{status}`{suffix}")
            } else {
                format!("published patchset `{patchset_id}`{suffix}")
            }
        }
        "run_patchset_ci" => {
            let patchset_id = normalize_optional_text(result_map.get("patchset_id"))
                .unwrap_or_else(|| "unknown".to_string());
            if matches!(result_map.get("queued"), Some(Value::Bool(true))) {
                format!("queued patchset CI for `{patchset_id}`{suffix}")
            } else {
                format!("updated patchset CI evidence for `{patchset_id}`{suffix}")
            }
        }
        "record_attestation" => {
            let attestation_id = normalize_optional_text(result_map.get("attestation_id"))
                .unwrap_or_else(|| "unknown".to_string());
            format!("recorded attestation `{attestation_id}`{suffix}")
        }
        "record_review" | "record_code_review_summary" => {
            let review_id = normalize_optional_text(result_map.get("review_id"))
                .unwrap_or_else(|| "unknown".to_string());
            format!("recorded review `{review_id}`{suffix}")
        }
        "evaluate_policy" => {
            let decision = normalize_optional_text(result_map.get("decision"))
                .unwrap_or_else(|| "unknown".to_string());
            format!("policy is now `{decision}`{suffix}")
        }
        "submit_land" => {
            let submission_id = normalize_optional_text(result_map.get("submission_id"))
                .unwrap_or_else(|| "unknown".to_string());
            let status = normalize_optional_text(result_map.get("status"))
                .unwrap_or_else(|| "unknown".to_string());
            let cleanup = result_map
                .get("bound_worktree_cleanup")
                .and_then(Value::as_object);
            let cleanup_status = cleanup
                .and_then(|payload| normalize_optional_text(payload.get("status")))
                .unwrap_or_default();
            if cleanup_status == "removed" {
                let cleanup_worktree = cleanup
                    .and_then(|payload| payload.get("worktree"))
                    .and_then(Value::as_object)
                    .and_then(|payload| normalize_optional_text(payload.get("name")))
                    .or_else(|| {
                        cleanup.and_then(|payload| {
                            normalize_optional_text(payload.get("worktree_name"))
                        })
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                format!(
                    "finish operation `{submission_id}` is `{status}` and removed bound worktree `{cleanup_worktree}`{suffix}"
                )
            } else {
                format!("finish operation `{submission_id}` is `{status}`{suffix}")
            }
        }
        "complete_task" => {
            let task_id = normalize_optional_text(result_map.get("task_id"))
                .unwrap_or_else(|| "unknown".to_string());
            let status = normalize_optional_text(result_map.get("status"))
                .unwrap_or_else(|| "unknown".to_string());
            format!("task `{task_id}` is `{status}`{suffix}")
        }
        _ => code,
    };
    Ok(summary)
}

#[cfg(test)]
mod tests;
