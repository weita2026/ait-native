use ait_server_core::foundation::policy_gate::{
    active_waiver_rules, policy_gate_evaluation, policy_gate_json, policy_input_fingerprint,
    policy_waiver_request, POLICY_GATE_CONTRACT,
};
use serde_json::{json, Value as JsonValue};

fn run(operation: &str, payload: JsonValue) -> JsonValue {
    policy_gate_json(operation, &payload).expect("policy gate shaping should succeed")
}

fn check_status(value: &JsonValue, name: &str) -> String {
    value["checks"]
        .as_array()
        .expect("checks should be an array")
        .iter()
        .find(|check| check["name"] == json!(name))
        .unwrap_or_else(|| panic!("missing check {name}"))["status"]
        .as_str()
        .expect("status should be text")
        .to_string()
}

#[test]
fn policy_gate_contract_declares_no_python_reference_and_operations() {
    let value = run("contract", json!({}));
    assert_eq!(value["contract"], json!(POLICY_GATE_CONTRACT));
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert_eq!(value["reference_modules"], json!([]));
    assert!(value.get("reference_module").is_none());
    assert_eq!(value["mutates_state"], json!(false));
    assert!(value["operations"]
        .as_array()
        .unwrap()
        .contains(&json!("evaluate")));
    assert!(value["operations"]
        .as_array()
        .unwrap()
        .contains(&json!("input-fingerprint")));
    assert!(value["operations"]
        .as_array()
        .unwrap()
        .contains(&json!("active-waiver-rules")));
    assert!(value["operations"]
        .as_array()
        .unwrap()
        .contains(&json!("waiver-request")));
}

#[test]
fn policy_input_fingerprint_is_stable_for_sorted_policy_and_waiver_inputs() {
    let left = json!({
        "patchset": {
            "revision_snapshot_id": "SNP-1",
            "author_mode": "ai_with_human_review",
            "diff_stats_json": "{\"paths\":{\"modified\":[\"src/lib.rs\"]}}"
        },
        "repo_policy": {
            "defaults": {"require_tests": true, "require_lint": false},
            "policy_id": "prototype"
        },
        "attestation": {
            "author_mode": "ai_with_human_review",
            "evaluation_summary_json": "{\"tests\":\"pass\"}",
            "provenance_summary_json": "{\"policy_readable\":true}",
            "detail_json": "{}"
        },
        "max_review_id": 10,
        "active_waiver_rules": ["lint", "ai_provenance", "lint"]
    });
    let right = json!({
        "active_waiver_rules": ["ai_provenance", "lint"],
        "max_review_id": 10,
        "attestation": {
            "detail_json": "{}",
            "provenance_summary_json": "{\"policy_readable\":true}",
            "evaluation_summary_json": "{\"tests\":\"pass\"}",
            "author_mode": "ai_with_human_review"
        },
        "repo_policy": {
            "policy_id": "prototype",
            "defaults": {"require_lint": false, "require_tests": true}
        },
        "patchset": {
            "diff_stats_json": "{\"paths\":{\"modified\":[\"src/lib.rs\"]}}",
            "author_mode": "ai_with_human_review",
            "revision_snapshot_id": "SNP-1"
        }
    });
    let left_fp = policy_input_fingerprint(left.as_object().unwrap());
    let right_fp = policy_input_fingerprint(right.as_object().unwrap());
    assert_eq!(left_fp, right_fp);
    assert_eq!(left_fp.len(), 64);

    let changed = run(
        "input-fingerprint",
        json!({
            "patchset": left["patchset"].clone(),
            "repo_policy": left["repo_policy"].clone(),
            "attestation": left["attestation"].clone(),
            "max_review_id": 11,
            "active_waiver_rules": ["ai_provenance", "lint"]
        }),
    );
    assert_ne!(changed["fingerprint"], json!(left_fp));
}

#[test]
fn policy_gate_evaluation_matches_reference_check_decision_rules() {
    let input = json!({
        "patchset": {"patchset_id": "RSEP-1"},
        "policy_context": {
            "content_class": "code_change",
            "author_class": "ai_related",
            "effective_requirements": {
                "require_attestation": true,
                "require_ai_provenance": true,
                "require_code_review_summary": true,
                "require_tests": true,
                "require_lint": true,
                "require_security_scan": false,
                "require_license_scan": false
            },
            "matched_overrides": []
        },
        "attestation": {
            "evaluation_summary": {
                "tests": "fail",
                "lint": "fail",
                "security_scan": "failed"
            },
            "provenance_summary": {
                "policy_readable": false,
                "missing_fields": ["model_name"]
            }
        },
        "review_summary": {
            "approval_count": 0,
            "code_review_summary_count": 0
        },
        "active_waiver_rules": ["lint"],
        "required_approvals": 1
    });
    let direct = policy_gate_evaluation(input.as_object().unwrap());
    let value = JsonValue::Object(direct.clone());

    assert_eq!(value["decision"], json!("hard_fail"));
    assert_eq!(check_status(&value, "require_attestation"), "pass");
    assert_eq!(check_status(&value, "ai_provenance"), "pending");
    assert_eq!(check_status(&value, "code_review_summary"), "optional_fail");
    assert_eq!(check_status(&value, "tests"), "hard_fail");
    assert_eq!(check_status(&value, "lint"), "waived");
    assert_eq!(check_status(&value, "security_scan"), "optional_fail");
    assert_eq!(check_status(&value, "license_scan"), "not_required");
    assert_eq!(check_status(&value, "required_human_review"), "pending");

    let via_seam = run("evaluate", input);
    assert_eq!(via_seam["evaluation"], JsonValue::Object(direct));
}

#[test]
fn policy_gate_evaluation_includes_ci_rollout_suite_failures() {
    let value = run(
        "evaluate",
        json!({
            "patchset": {"patchset_id": "RSEP-2"},
            "effective_requirements": {
                "require_attestation": false,
                "require_tests": false
            },
            "review_summary": {"approval_count": 1, "code_review_summary_count": 0},
            "ci_rollout": {
                "phase": 1,
                "required_patchset_suites": ["unit"],
                "informational_patchset_suites": ["lint"],
                "promotion_candidates": {},
                "suite_results_by_id": {
                    "unit": {"status": "fail"},
                    "lint": {"status": "fail"}
                }
            }
        }),
    );
    let evaluation = &value["evaluation"];
    assert_eq!(evaluation["decision"], json!("hard_fail"));
    assert_eq!(
        check_status(evaluation, "ci_patchset_suite_unit"),
        "hard_fail"
    );
    assert_eq!(
        check_status(evaluation, "ci_patchset_suite_lint"),
        "optional_fail"
    );
}

#[test]
fn active_waiver_rules_filters_expired_rows_and_dedupes() {
    let payload = json!({
        "now": "2026-07-08T00:00:00+00:00",
        "waivers": [
            {"rule_name": "lint", "expires_at": null},
            {"rule_name": "security_scan", "expires_at": "2026-07-09T00:00:00+00:00"},
            {"rule_name": "tests", "expires_at": "2026-07-07T00:00:00+00:00"},
            {"rule_name": "lint", "expires_at": null}
        ]
    });
    assert_eq!(
        active_waiver_rules(payload.as_object().unwrap()),
        vec!["lint".to_string(), "security_scan".to_string()]
    );
    let via_seam = run("active-waiver-rules", payload);
    assert_eq!(
        via_seam["active_waiver_rules"],
        json!(["lint", "security_scan"])
    );
}

#[test]
fn policy_waiver_request_shapes_id_and_rejects_ci_backed_rules() {
    let payload = json!({
        "patchset_id": "RP-2477-1",
        "rule_name": "security_scan",
        "reason": "accepted risk",
        "expires_at": "2026-08-01T00:00:00+00:00",
        "existing_waiver_count": 2,
        "created_at": "2026-07-08T00:00:00+00:00",
        "change_id": "RC-2477"
    });
    let direct = policy_waiver_request(payload.as_object().unwrap()).expect("waiver should shape");
    assert_eq!(direct["waiver_id"], json!("W-2477-1-3"));
    assert_eq!(direct["rule_name"], json!("security_scan"));
    assert_eq!(direct["change_id"], json!("RC-2477"));
    let via_seam = run("waiver-request", payload);
    assert_eq!(via_seam["waiver"], JsonValue::Object(direct));

    let err = policy_gate_json(
        "waiver-request",
        &json!({
            "patchset_id": "RP-1",
            "rule_name": "ci_patchset_suite_unit",
            "existing_waiver_count": 0,
            "created_at": "2026-07-08T00:00:00+00:00",
            "change_id": "RC-1"
        }),
    )
    .expect_err("CI-backed rule should be rejected");
    assert!(err.contains("CI-backed rule `ci_patchset_suite_unit` cannot be waived"));
}
