use crate::foundation::workflow_artifacts::{
    ci_rollout_patchset_suite_checks, ci_rollout_summary_message, requires_code_review_summary,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const POLICY_GATE_CONTRACT: &str = "ait.server.policy_gate.v1";

const RULE_LABELS: &[(&str, &str)] = &[
    ("require_attestation", "Patchset must include attestation"),
    ("ai_provenance", "AI provenance must be policy-readable"),
    ("tests", "Tests must pass"),
    ("lint", "Lint must pass"),
    ("security_scan", "Security scan must pass"),
    ("license_scan", "License scan must pass"),
    (
        "code_review_summary",
        "Code review summary must be recorded",
    ),
    (
        "required_human_review",
        "Required human approvals must be present",
    ),
];

pub fn policy_gate_json(operation: &str, request: &JsonValue) -> Result<JsonValue, String> {
    let payload = request
        .as_object()
        .ok_or_else(|| "policy-gate payload must be a JSON object.".to_string())?;
    match operation {
        "contract" => Ok(json!({
            "contract": POLICY_GATE_CONTRACT,
            "reference_modules": [],
            "migration_status": "rust_owned_no_python_reference",
            "mutates_state": false,
            "operations": [
                "evaluate",
                "input-fingerprint",
                "active-waiver-rules",
                "waiver-request",
            ],
        })),
        "evaluate" => Ok(json!({
            "contract": POLICY_GATE_CONTRACT,
            "reference_modules": [],
            "migration_status": "rust_owned_no_python_reference",
            "evaluation": policy_gate_evaluation(payload),
        })),
        "input-fingerprint" => Ok(json!({
            "contract": POLICY_GATE_CONTRACT,
            "reference_modules": [],
            "migration_status": "rust_owned_no_python_reference",
            "fingerprint": policy_input_fingerprint(payload),
        })),
        "active-waiver-rules" => Ok(json!({
            "contract": POLICY_GATE_CONTRACT,
            "reference_modules": [],
            "migration_status": "rust_owned_no_python_reference",
            "active_waiver_rules": active_waiver_rules(payload),
        })),
        "waiver-request" => Ok(json!({
            "contract": POLICY_GATE_CONTRACT,
            "reference_modules": [],
            "migration_status": "rust_owned_no_python_reference",
            "waiver": policy_waiver_request(payload)?,
        })),
        other => Err(format!("Unsupported policy-gate operation `{other}`.")),
    }
}

pub fn active_waiver_rules(input: &JsonMap<String, JsonValue>) -> Vec<String> {
    let now = optional_text(input.get("now")).unwrap_or_default();
    let mut rules = BTreeSet::new();
    for row in input
        .get("waivers")
        .or_else(|| input.get("waiver_rows"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_object)
    {
        let expires_at = optional_text(row.get("expires_at"));
        if expires_at
            .as_deref()
            .is_some_and(|value| !now.is_empty() && value < now.as_str())
        {
            continue;
        }
        if let Some(rule_name) = optional_text(row.get("rule_name")) {
            rules.insert(rule_name);
        }
    }
    rules.into_iter().collect()
}

pub fn policy_waiver_request(
    input: &JsonMap<String, JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let patchset_id = required_text(input.get("patchset_id"), "patchset_id")?;
    let rule_name = optional_text(input.get("rule_name")).unwrap_or_default();
    if !allows_waiver(&rule_name) {
        return Err(format!(
            "CI-backed rule `{rule_name}` cannot be waived. Fix the CI failure and rerun the required checks to `pass` before remote land."
        ));
    }
    let reason = optional_text(input.get("reason")).unwrap_or_default();
    let expires_at = optional_text(input.get("expires_at"));
    let created_at = required_text(input.get("created_at"), "created_at")?;
    let change_id = required_text(input.get("change_id"), "change_id")?;
    let count = optional_i64(input.get("existing_waiver_count"))
        .or_else(|| optional_i64(input.get("waiver_count")))
        .unwrap_or(0);
    let local_patchset_id = patchset_id
        .split_once('-')
        .map(|(_, local)| local)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            format!("patchset_id `{patchset_id}` must contain a local id after the first dash")
        })?;
    let mut waiver = JsonMap::new();
    waiver.insert(
        "waiver_id".to_string(),
        json!(format!("W-{local_patchset_id}-{}", count + 1)),
    );
    waiver.insert("patchset_id".to_string(), json!(patchset_id));
    waiver.insert("rule_name".to_string(), json!(rule_name));
    waiver.insert("reason".to_string(), json!(reason));
    waiver.insert(
        "expires_at".to_string(),
        expires_at.map(JsonValue::String).unwrap_or(JsonValue::Null),
    );
    waiver.insert("created_at".to_string(), json!(created_at));
    waiver.insert("change_id".to_string(), json!(change_id));
    Ok(waiver)
}

pub fn policy_input_fingerprint(input: &JsonMap<String, JsonValue>) -> String {
    let patchset = optional_object(input.get("patchset")).unwrap_or(input);
    let attestation = optional_object(input.get("attestation"));
    let active_waiver_rules = sorted_text_values(input.get("active_waiver_rules"));
    let payload = json!({
        "revision_snapshot_id": optional_text(patchset.get("revision_snapshot_id")).unwrap_or_default(),
        "patchset_author_mode": optional_text(patchset.get("author_mode")).unwrap_or_default(),
        "diff_stats_json": optional_text(patchset.get("diff_stats_json")).unwrap_or_default(),
        "repo_policy": input.get("repo_policy").filter(|value| value.is_object()).cloned().unwrap_or_else(|| json!({})),
        "attestation_author_mode": attestation.and_then(|row| optional_text(row.get("author_mode"))).unwrap_or_default(),
        "attestation_evaluation_summary_json": attestation.and_then(|row| optional_text(row.get("evaluation_summary_json"))).unwrap_or_default(),
        "attestation_provenance_summary_json": attestation.and_then(|row| optional_text(row.get("provenance_summary_json"))).unwrap_or_default(),
        "attestation_detail_json": attestation.and_then(|row| optional_text(row.get("detail_json"))).unwrap_or_default(),
        "max_review_id": optional_i64(input.get("max_review_id")).unwrap_or(0),
        "active_waiver_rules": active_waiver_rules,
    });
    policy_content_fingerprint(&payload)
}

pub(crate) fn policy_content_fingerprint(value: &JsonValue) -> String {
    sha256_hex(canonical_json(value).as_bytes())
}

pub fn policy_gate_evaluation(input: &JsonMap<String, JsonValue>) -> JsonMap<String, JsonValue> {
    let policy_context = optional_object(input.get("policy_context"));
    let effective_requirements = optional_object(input.get("effective_requirements"))
        .or_else(|| {
            policy_context
                .and_then(|context| optional_object(context.get("effective_requirements")))
        })
        .cloned()
        .unwrap_or_default();
    let attestation = optional_object(input.get("attestation"));
    let evaluation_summary = input
        .get("evaluation_summary")
        .filter(|value| value.is_object())
        .cloned()
        .or_else(|| attestation.and_then(|row| parse_json_field(row, "evaluation_summary")))
        .or_else(|| attestation.and_then(|row| parse_json_field(row, "evaluation_summary_json")))
        .unwrap_or_else(|| json!({}));
    let provenance_summary = input
        .get("provenance_summary")
        .filter(|value| value.is_object())
        .cloned()
        .or_else(|| attestation.and_then(|row| parse_json_field(row, "provenance_summary")))
        .or_else(|| attestation.and_then(|row| parse_json_field(row, "provenance_summary_json")))
        .unwrap_or_else(|| json!({}));
    let review_summary = optional_object(input.get("review_summary"));
    let active_waiver_rules = sorted_text_values(input.get("active_waiver_rules"))
        .into_iter()
        .collect::<BTreeSet<_>>();
    let required_approvals = optional_i64(input.get("required_approvals"))
        .filter(|value| *value > 0)
        .unwrap_or(1);
    let requires_summary = input
        .get("requires_code_review_summary")
        .map(|value| truthy(Some(value)))
        .unwrap_or_else(|| {
            let mut context = policy_context.cloned().unwrap_or_default();
            context
                .entry("effective_requirements".to_string())
                .or_insert_with(|| JsonValue::Object(effective_requirements.clone()));
            requires_code_review_summary(&context)
        });

    let mut checks = Vec::new();
    let mut decision = "pass".to_string();
    let mut waived_any = false;

    let require_attestation =
        requirement_bool(&effective_requirements, "require_attestation", true);
    if require_attestation {
        if attestation.is_some() {
            add_check(
                &mut checks,
                &mut decision,
                &mut waived_any,
                &active_waiver_rules,
                "require_attestation",
                "pass",
                None,
                None,
            );
        } else {
            add_check(
                &mut checks,
                &mut decision,
                &mut waived_any,
                &active_waiver_rules,
                "require_attestation",
                "pending",
                Some("Attestation is required before landing"),
                None,
            );
        }
    } else {
        add_check(
            &mut checks,
            &mut decision,
            &mut waived_any,
            &active_waiver_rules,
            "require_attestation",
            "not_required",
            Some("Attestation is optional by repository policy"),
            None,
        );
    }

    if requirement_bool(&effective_requirements, "require_ai_provenance", false) {
        if attestation.is_none() {
            add_check(
                &mut checks,
                &mut decision,
                &mut waived_any,
                &active_waiver_rules,
                "ai_provenance",
                "pending",
                Some("AI provenance is required before landing"),
                None,
            );
        } else if provenance_summary
            .get("policy_readable")
            .is_some_and(|value| truthy(Some(value)))
        {
            add_check(
                &mut checks,
                &mut decision,
                &mut waived_any,
                &active_waiver_rules,
                "ai_provenance",
                "pass",
                None,
                None,
            );
        } else {
            let missing_fields = string_list(provenance_summary.get("missing_fields"));
            let detail = if missing_fields.is_empty() {
                "minimum provenance fields are missing".to_string()
            } else {
                missing_fields.join(", ")
            };
            add_check(
                &mut checks,
                &mut decision,
                &mut waived_any,
                &active_waiver_rules,
                "ai_provenance",
                "pending",
                Some(format!("AI provenance is incomplete: {detail}").as_str()),
                None,
            );
        }
    } else {
        add_check(
            &mut checks,
            &mut decision,
            &mut waived_any,
            &active_waiver_rules,
            "ai_provenance",
            "not_required",
            Some("AI provenance is optional by repository policy"),
            None,
        );
    }

    if requires_summary {
        if review_count(review_summary, "code_review_summary_count") > 0 {
            add_check(
                &mut checks,
                &mut decision,
                &mut waived_any,
                &active_waiver_rules,
                "code_review_summary",
                "pass",
                None,
                None,
            );
        } else {
            add_check(
                &mut checks,
                &mut decision,
                &mut waived_any,
                &active_waiver_rules,
                "code_review_summary",
                "optional_fail",
                Some("Agent-prepared code review summary is recommended before landing"),
                None,
            );
        }
    } else {
        add_check(
            &mut checks,
            &mut decision,
            &mut waived_any,
            &active_waiver_rules,
            "code_review_summary",
            "not_required",
            Some("Code review summary is not required for this patchset by repository policy"),
            None,
        );
    }

    for (key, requirement_key) in [
        ("tests", "require_tests"),
        ("lint", "require_lint"),
        ("security_scan", "require_security_scan"),
        ("license_scan", "require_license_scan"),
    ] {
        add_evidence_check(
            &mut checks,
            &mut decision,
            &mut waived_any,
            &active_waiver_rules,
            key,
            requirement_bool(&effective_requirements, requirement_key, false),
            optional_text(evaluation_summary.get(key)).as_deref(),
        );
    }

    if let Some(ci_rollout) = optional_object(input.get("ci_rollout")) {
        let rollout_message = ci_rollout_summary_message(ci_rollout);
        add_check(
            &mut checks,
            &mut decision,
            &mut waived_any,
            &active_waiver_rules,
            "ci_rollout_phase",
            "pass",
            Some(rollout_message.as_str()),
            Some("CI rollout phase"),
        );
        for suite_check in ci_rollout_patchset_suite_checks(ci_rollout) {
            if let Some(check) = suite_check.as_object() {
                let name = optional_text(check.get("name")).unwrap_or_default();
                let status = optional_text(check.get("status")).unwrap_or_default();
                let message = optional_text(check.get("message"));
                let label = optional_text(check.get("label"));
                add_check(
                    &mut checks,
                    &mut decision,
                    &mut waived_any,
                    &active_waiver_rules,
                    &name,
                    &status,
                    message.as_deref(),
                    label.as_deref(),
                );
            }
        }
    }

    if review_count(review_summary, "approval_count") >= required_approvals {
        add_check(
            &mut checks,
            &mut decision,
            &mut waived_any,
            &active_waiver_rules,
            "required_human_review",
            "pass",
            None,
            None,
        );
    } else {
        add_check(
            &mut checks,
            &mut decision,
            &mut waived_any,
            &active_waiver_rules,
            "required_human_review",
            "pending",
            Some(
                format!(
                    "{required_approvals} approval(s) required by the current server compatibility review policy"
                )
                .as_str(),
            ),
            None,
        );
    }

    if decision == "pass" && waived_any {
        decision = "waived".to_string();
    }

    let patchset = optional_object(input.get("patchset")).unwrap_or(input);
    let mut out = JsonMap::new();
    out.insert(
        "patchset_id".to_string(),
        optional_text(input.get("patchset_id"))
            .or_else(|| optional_text(patchset.get("patchset_id")))
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    out.insert("decision".to_string(), json!(decision));
    out.insert("checks".to_string(), JsonValue::Array(checks));
    out.insert(
        "effective_requirements".to_string(),
        JsonValue::Object(effective_requirements),
    );
    out.insert("required_approvals".to_string(), json!(required_approvals));
    out
}

fn add_evidence_check(
    checks: &mut Vec<JsonValue>,
    decision: &mut String,
    waived_any: &mut bool,
    waivers: &BTreeSet<String>,
    key: &str,
    required: bool,
    value: Option<&str>,
) {
    if required {
        match value {
            Some("pass") => add_check(
                checks, decision, waived_any, waivers, key, "pass", None, None,
            ),
            Some("fail" | "failed") => add_check(
                checks,
                decision,
                waived_any,
                waivers,
                key,
                "hard_fail",
                Some(rule_label(key)),
                None,
            ),
            _ => add_check(
                checks,
                decision,
                waived_any,
                waivers,
                key,
                "pending",
                Some(rule_label(key)),
                None,
            ),
        }
    } else {
        match value {
            Some("pass") => add_check(
                checks,
                decision,
                waived_any,
                waivers,
                key,
                "pass",
                Some(format!("{} (optional)", rule_label(key)).as_str()),
                None,
            ),
            Some("fail" | "failed") => add_check(
                checks,
                decision,
                waived_any,
                waivers,
                key,
                "optional_fail",
                Some(format!("{} (optional)", rule_label(key)).as_str()),
                None,
            ),
            _ => add_check(
                checks,
                decision,
                waived_any,
                waivers,
                key,
                "not_required",
                Some(format!("{} not required by repository policy", rule_label(key)).as_str()),
                None,
            ),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_check(
    checks: &mut Vec<JsonValue>,
    decision: &mut String,
    waived_any: &mut bool,
    waivers: &BTreeSet<String>,
    rule_name: &str,
    status: &str,
    message: Option<&str>,
    label: Option<&str>,
) {
    let final_status =
        if status == "hard_fail" && waivers.contains(rule_name) && allows_waiver(rule_name) {
            *waived_any = true;
            "waived"
        } else {
            status
        };
    let label = label.unwrap_or_else(|| rule_label(rule_name));
    checks.push(json!({
        "name": rule_name,
        "label": label,
        "status": final_status,
        "message": message.unwrap_or(label),
    }));
    if final_status == "hard_fail" {
        *decision = "hard_fail".to_string();
    } else if final_status == "pending" && decision != "hard_fail" {
        *decision = "pending".to_string();
    } else if final_status == "soft_fail" && !matches!(decision.as_str(), "hard_fail" | "pending") {
        *decision = "soft_fail".to_string();
    }
}

fn allows_waiver(rule_name: &str) -> bool {
    let normalized = rule_name.trim();
    normalized != "tests"
        && normalized != "require_tests"
        && !normalized.starts_with("ci_patchset_suite_")
}

fn rule_label(rule_name: &str) -> &str {
    RULE_LABELS
        .iter()
        .find_map(|(name, label)| (*name == rule_name).then_some(*label))
        .unwrap_or(rule_name)
}

fn requirement_bool(
    requirements: &JsonMap<String, JsonValue>,
    key: &str,
    default_value: bool,
) -> bool {
    requirements
        .get(key)
        .map(|value| truthy(Some(value)))
        .unwrap_or(default_value)
}

fn review_count(review: Option<&JsonMap<String, JsonValue>>, key: &str) -> i64 {
    review
        .and_then(|review| optional_i64(review.get(key)))
        .unwrap_or(0)
}

fn parse_json_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<JsonValue> {
    match obj.get(field) {
        Some(JsonValue::String(text)) => serde_json::from_str(text).ok(),
        Some(JsonValue::Object(_)) | Some(JsonValue::Array(_)) => obj.get(field).cloned(),
        _ => None,
    }
}

fn sorted_text_values(value: Option<&JsonValue>) -> Vec<String> {
    let mut out = string_list(value);
    out.sort();
    out.dedup();
    out
}

fn string_list(value: Option<&JsonValue>) -> Vec<String> {
    match value {
        Some(JsonValue::Array(items)) => items
            .iter()
            .filter_map(|item| optional_text(Some(item)))
            .collect(),
        Some(value) => optional_text(Some(value)).into_iter().collect(),
        None => Vec::new(),
    }
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value)
        .ok_or_else(|| format!("policy-gate payload requires text field `{field}`."))
}

fn canonical_json(value: &JsonValue) -> String {
    match value {
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
        }
        JsonValue::Array(values) => {
            let items = values.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", items.join(","))
        }
        JsonValue::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let entries = keys
                .into_iter()
                .map(|key| {
                    let key_json =
                        serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    let value_json = canonical_json(map.get(key).unwrap_or(&JsonValue::Null));
                    format!("{key_json}:{value_json}")
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", entries.join(","))
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn optional_object(value: Option<&JsonValue>) -> Option<&JsonMap<String, JsonValue>> {
    value.and_then(JsonValue::as_object)
}

fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if !truthy(Some(value)) {
        return None;
    }
    let text = match value {
        JsonValue::String(text) => text.clone(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => String::new(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
        JsonValue::Null => String::new(),
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value {
        None | Some(JsonValue::Null) => None,
        Some(value) if !truthy(Some(value)) => None,
        Some(JsonValue::Number(number)) => number.as_i64(),
        Some(JsonValue::String(text)) => text.trim().parse::<i64>().ok(),
        Some(_) => None,
    }
}

fn truthy(value: Option<&JsonValue>) -> bool {
    match value {
        None | Some(JsonValue::Null) => false,
        Some(JsonValue::Bool(value)) => *value,
        Some(JsonValue::Number(number)) => {
            number.as_f64().map(|value| value != 0.0).unwrap_or(true)
        }
        Some(JsonValue::String(text)) => !text.trim().is_empty(),
        Some(JsonValue::Array(values)) => !values.is_empty(),
        Some(JsonValue::Object(values)) => !values.is_empty(),
    }
}
