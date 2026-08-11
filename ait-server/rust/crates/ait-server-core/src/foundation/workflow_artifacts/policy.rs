use super::*;

pub fn policy_status_view(
    patchset_id: &str,
    decision: &str,
    checks: Vec<JsonValue>,
    evaluated_at: Option<String>,
) -> JsonValue {
    json!({
        "patchset_id": patchset_id,
        "decision": decision,
        "checks": checks,
        "evaluated_at": evaluated_at,
    })
}

pub fn effective_policy_status(
    patchset: &JsonMap<String, JsonValue>,
    latest_status: Option<&JsonMap<String, JsonValue>>,
) -> Result<JsonValue, String> {
    let patchset_id = required_text(patchset.get("patchset_id"), "patchset.patchset_id")?;
    let evaluation_state =
        optional_text(patchset.get("evaluation_state")).unwrap_or_else(|| "pending".to_string());
    if evaluation_state == "pending" {
        if let Some(latest) = latest_status {
            if optional_text(latest.get("decision")).as_deref() == Some("pending") {
                return Ok(JsonValue::Object(latest.clone()));
            }
        }
        return Ok(policy_status_view(
            &patchset_id,
            "pending",
            Vec::new(),
            None,
        ));
    }
    if let Some(latest) = latest_status {
        let mut out = latest.clone();
        out.insert("decision".to_string(), json!(evaluation_state));
        return Ok(JsonValue::Object(out));
    }
    Ok(policy_status_view(
        &patchset_id,
        &evaluation_state,
        Vec::new(),
        None,
    ))
}
