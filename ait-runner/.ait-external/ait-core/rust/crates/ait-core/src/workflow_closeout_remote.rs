use crate::json_support::{JsonMap as Map, JsonValue as Value};

pub fn workflow_remote_mutation_receipt(
    action: &str,
    source_action: &str,
    delivery: &str,
    response_recovery: Option<&Value>,
    result: Option<&Value>,
) -> Result<Value, String> {
    let normalized_action = normalize_required_text(action, "action")?;
    let normalized_source_action = normalize_required_text(source_action, "source_action")?;
    let normalized_delivery = normalize_required_text(delivery, "delivery")?;
    let mut payload = Map::new();
    payload.insert("action".to_string(), Value::String(normalized_action));
    payload.insert(
        "source_action".to_string(),
        Value::String(normalized_source_action),
    );
    payload.insert("delivery".to_string(), Value::String(normalized_delivery));

    if let Some(Value::Object(result_map)) = result {
        for key in [
            "change_id",
            "patchset_id",
            "attestation_id",
            "review_id",
            "submission_id",
            "task_id",
            "snapshot_id",
            "decision",
            "status",
        ] {
            if let Some(value) = result_map.get(key) {
                if !value.is_null() {
                    payload.insert(key.to_string(), value.clone());
                }
            }
        }
        if let Some(value) = result_map.get("queued") {
            if let Some(flag) = value.as_bool() {
                payload.insert("queued".to_string(), Value::Bool(flag));
            }
        }
        if let Some(Value::Object(job)) = result_map.get("job") {
            if let Some(value) = job.get("job_id") {
                if !value.is_null() {
                    payload.insert("job_id".to_string(), value.clone());
                }
            }
            if let Some(value) = job.get("state") {
                if !value.is_null() {
                    payload.insert("job_state".to_string(), value.clone());
                }
            }
        }
    }

    if let Some(Value::Object(recovery)) = response_recovery {
        let filtered = recovery
            .iter()
            .filter(|&(_key, value)| !value.is_null())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        payload.insert("response_recovery".to_string(), Value::Object(filtered));
    }
    Ok(Value::Object(payload))
}

pub fn workflow_remote_action_mutation_receipts(
    code: &str,
    result: &Value,
) -> Result<Value, String> {
    let normalized_code = code.trim();
    let result_map = match result {
        Value::Object(map) => map,
        _ => return Err("Workflow closeout remote result must be an object.".to_string()),
    };
    let primary_recovery = match result_map.get("response_recovery") {
        Some(Value::Object(_)) => result_map.get("response_recovery"),
        _ => None,
    };
    let primary_delivery = if primary_recovery.is_some() {
        "response_recovery"
    } else {
        "direct_response"
    };
    let mut receipts = Vec::new();

    if matches!(normalized_code, "publish_patchset" | "refresh_patchset") {
        receipts.push(workflow_remote_mutation_receipt(
            "publish_patchset",
            normalized_code,
            primary_delivery,
            primary_recovery,
            Some(result),
        )?);
        if let Some(Value::Object(_)) = result_map.get("selection_recovery") {
            receipts.push(workflow_remote_mutation_receipt(
                "select_patchset",
                normalized_code,
                "response_recovery",
                result_map.get("selection_recovery"),
                Some(result),
            )?);
        }
        return Ok(Value::Array(receipts));
    }

    if matches!(
        normalized_code,
        "record_review" | "record_code_review_summary"
    ) {
        receipts.push(workflow_remote_mutation_receipt(
            "record_review",
            normalized_code,
            primary_delivery,
            primary_recovery,
            Some(result),
        )?);
        if let Some(Value::Object(policy_refresh)) = result_map.get("policy_refresh") {
            let policy_recovery = match policy_refresh.get("response_recovery") {
                Some(Value::Object(_)) => policy_refresh.get("response_recovery"),
                _ => None,
            };
            let policy_delivery = if policy_recovery.is_some() {
                "response_recovery"
            } else {
                "direct_response"
            };
            receipts.push(workflow_remote_mutation_receipt(
                "evaluate_policy",
                normalized_code,
                policy_delivery,
                policy_recovery,
                result_map.get("policy_refresh"),
            )?);
        }
        return Ok(Value::Array(receipts));
    }

    if matches!(
        normalized_code,
        "evaluate_policy"
            | "run_patchset_ci"
            | "record_attestation"
            | "submit_land"
            | "complete_task"
    ) {
        let action = match normalized_code {
            "evaluate_policy" => "evaluate_policy",
            "run_patchset_ci" => "run_patchset_ci",
            "record_attestation" => "record_attestation",
            "submit_land" => "submit_land",
            "complete_task" => "complete_task",
            _ => unreachable!(),
        };
        receipts.push(workflow_remote_mutation_receipt(
            action,
            normalized_code,
            primary_delivery,
            primary_recovery,
            Some(result),
        )?);
    }

    Ok(Value::Array(receipts))
}

fn normalize_required_text(value: &str, field_name: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(format!(
            "Workflow closeout remote payload field `{}` must be non-empty.",
            field_name
        ));
    }
    Ok(normalized.to_string())
}

#[cfg(test)]
mod tests;
