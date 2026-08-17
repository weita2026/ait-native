use serde_json::{json, Map as JsonMap, Value as JsonValue};

const PATCHSET_CI_PROFILE_FULL: &str = "full";
const PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND: &str = "workflow_ready_foreground";

pub fn workflow_async_runtime_json(
    operation: &str,
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "workflow-async-runtime payload must be a JSON object.".to_string())?;
    match operation {
        "queue-mode" => Ok(json!({
            "contract": "ait.server.workflow_async.queue_mode.v1",
            "queue_mode": queue_mode(payload),
        })),
        "normalize-patchset-ci-execution-profile" => {
            let profile = normalize_patchset_ci_execution_profile(optional_text(payload, "execution_profile"))?;
            Ok(json!({
                "contract": "ait.server.workflow_async.patchset_ci_execution_profile.v1",
                "execution_profile": profile,
            }))
        }
        "policy-job-payload" => Ok(json!({
            "contract": "ait.server.workflow_async.policy_job_payload.v1",
            "payload": policy_job_payload(payload)?,
        })),
        "patchset-ci-job-payload" => Ok(json!({
            "contract": "ait.server.workflow_async.patchset_ci_job_payload.v1",
            "payload": patchset_ci_job_payload(payload)?,
        })),
        "land-job-payload" => Ok(json!({
            "contract": "ait.server.workflow_async.land_job_payload.v1",
            "payload": land_job_payload(payload)?,
        })),
        "patchset-publish-policy-followup" => patchset_publish_policy_followup(payload),
        "patchset-ci-start-plan" => patchset_ci_start_plan(payload),
        other => Err(format!(
            "Unsupported workflow async runtime operation `{other}`. Expected one of: queue-mode, normalize-patchset-ci-execution-profile, policy-job-payload, patchset-ci-job-payload, land-job-payload, patchset-publish-policy-followup, patchset-ci-start-plan."
        )),
    }
}

fn queue_mode(payload: &JsonMap<String, JsonValue>) -> String {
    let candidate = optional_text(payload, "queue_mode").or_else(|| optional_text(payload, "mode"));
    match candidate
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("async") => "async".to_string(),
        _ => "inline".to_string(),
    }
}

pub fn normalize_patchset_ci_execution_profile(value: Option<String>) -> Result<String, String> {
    let profile = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(PATCHSET_CI_PROFILE_FULL);
    match profile {
        PATCHSET_CI_PROFILE_FULL | PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND => {
            Ok(profile.to_string())
        }
        other => Err(format!(
            "Unsupported patchset CI execution_profile `{other}`."
        )),
    }
}

fn policy_job_payload(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let patchset = required_object(payload, "patchset")?;
    let change = required_object(payload, "change")?;
    let mut out = JsonMap::new();
    out.insert(
        "patchset_id".to_string(),
        json!(required_text(patchset, "patchset_id")?),
    );
    out.insert(
        "repo_name".to_string(),
        json!(required_text(change, "repo_name")?),
    );
    insert_optional_repo_id(&mut out, patchset, change);
    insert_optional_clone(&mut out, change, "change_id");
    insert_optional_clone(&mut out, change, "change_seq");
    insert_optional_clone(&mut out, patchset, "patchset_number");
    Ok(JsonValue::Object(out))
}

fn patchset_ci_job_payload(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let mut out = policy_job_payload(payload)?
        .as_object()
        .cloned()
        .ok_or_else(|| "policy job payload should be an object.".to_string())?;
    let trigger = optional_text(payload, "trigger").unwrap_or_else(|| "worker_job".to_string());
    let execution_profile =
        normalize_patchset_ci_execution_profile(optional_text(payload, "execution_profile"))?;
    out.insert("trigger".to_string(), json!(trigger));
    out.insert("execution_profile".to_string(), json!(execution_profile));
    Ok(JsonValue::Object(out))
}

fn land_job_payload(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let land = required_object(payload, "land")?;
    let change = required_object(payload, "change")?;
    let mut out = JsonMap::new();
    out.insert(
        "submission_id".to_string(),
        json!(required_text(land, "submission_id")?),
    );
    out.insert(
        "repo_name".to_string(),
        json!(required_text(change, "repo_name")?),
    );
    insert_optional_repo_id(&mut out, land, change);
    insert_optional_clone(&mut out, land, "change_id");
    if !out.contains_key("change_id") {
        insert_optional_clone(&mut out, change, "change_id");
    }
    insert_optional_clone(&mut out, change, "change_seq");
    insert_optional_clone(&mut out, land, "patchset_id");
    insert_optional_clone(&mut out, land, "land_seq");
    Ok(JsonValue::Object(out))
}

fn patchset_publish_policy_followup(
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let patchset_id = required_text(payload, "patchset_id")?;
    let mode = queue_mode(payload);
    let mut policy_followup = JsonMap::new();
    policy_followup.insert("state".to_string(), json!("deferred"));
    policy_followup.insert("queue_mode".to_string(), json!(mode));
    policy_followup.insert(
        "reason".to_string(),
        json!("Patchset publish keeps policy evaluation off the request path until patchset evidence changes through attestation, review, patchset selection, or waiver actions."),
    );
    policy_followup.insert(
        "activation_events".to_string(),
        json!([
            "patchset.selected",
            "attestation.upserted",
            "review.recorded",
            "policy.waived"
        ]),
    );
    if policy_followup
        .get("queue_mode")
        .and_then(JsonValue::as_str)
        == Some("inline")
    {
        policy_followup.insert(
            "command".to_string(),
            json!(format!("ait policy eval {patchset_id}")),
        );
    }
    Ok(json!({
        "contract": "ait.server.workflow_async.patchset_publish_policy_followup.v1",
        "policy_followup": policy_followup,
    }))
}

fn patchset_ci_start_plan(payload: &JsonMap<String, JsonValue>) -> Result<JsonValue, String> {
    let patchset_id = required_text(payload, "patchset_id")?;
    let trigger = optional_text(payload, "trigger").unwrap_or_else(|| "manual_rerun".to_string());
    let execution_profile =
        normalize_patchset_ci_execution_profile(optional_text(payload, "execution_profile"))?;
    if optional_bool(payload, "contract_available").unwrap_or(false) != true {
        return Ok(json!({
            "contract": "ait.server.workflow_async.patchset_ci_start_plan.v1",
            "state": "unavailable",
            "patchset_id": patchset_id,
            "trigger": trigger,
            "execution_profile": execution_profile,
            "actions": [],
            "result": null,
        }));
    }
    if let Some(active_state) = payload
        .get("active_state")
        .filter(|value| value.is_object())
    {
        return Ok(json!({
            "contract": "ait.server.workflow_async.patchset_ci_start_plan.v1",
            "state": "reuse_active",
            "patchset_id": patchset_id,
            "trigger": trigger,
            "execution_profile": execution_profile,
            "actions": [],
            "result": active_state,
        }));
    }
    let mode = queue_mode(payload);
    let delivery = if mode == "async" {
        "async_queue"
    } else {
        "background_thread"
    };
    Ok(json!({
        "contract": "ait.server.workflow_async.patchset_ci_start_plan.v1",
        "state": "enqueue",
        "patchset_id": patchset_id,
        "trigger": trigger,
        "execution_profile": execution_profile,
        "queue_mode": mode,
        "delivery": delivery,
        "mark_pending": {
            "tests_status": "pending",
            "job_state": "queued"
        },
        "enqueue": {
            "job_type": "patchset.ci",
            "max_attempts": 3,
            "dedupe_active": true
        },
    }))
}

fn required_object<'a>(
    payload: &'a JsonMap<String, JsonValue>,
    key: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    payload
        .get(key)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("workflow async payload requires object field `{key}`."))
}

fn required_text(payload: &JsonMap<String, JsonValue>, key: &str) -> Result<String, String> {
    optional_text(payload, key)
        .ok_or_else(|| format!("workflow async payload requires text field `{key}`."))
}

fn optional_text(payload: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_bool(payload: &JsonMap<String, JsonValue>, key: &str) -> Option<bool> {
    payload.get(key).and_then(JsonValue::as_bool)
}

fn insert_optional_clone(
    out: &mut JsonMap<String, JsonValue>,
    source: &JsonMap<String, JsonValue>,
    key: &str,
) {
    if let Some(value) = source.get(key).filter(|value| !value.is_null()) {
        out.insert(key.to_string(), value.clone());
    }
}

fn insert_optional_repo_id(
    out: &mut JsonMap<String, JsonValue>,
    primary: &JsonMap<String, JsonValue>,
    secondary: &JsonMap<String, JsonValue>,
) {
    if let Some(value) = primary
        .get("repo_id")
        .or_else(|| secondary.get("repo_id"))
        .filter(|value| !value.is_null())
    {
        out.insert("repo_id".to_string(), value.clone());
    }
}
