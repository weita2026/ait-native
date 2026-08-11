use ait_server_core::foundation::identity::{
    assert_repo_scope, derive_patchset_id, identity_json, local_id_after_first_dash,
    normalize_change_row, normalize_task_row, repo_scope_predicate, repo_scoped_sequence_ref,
    sequence_after_first_dash, sequence_after_last_dash, IDENTITY_CONTRACT,
    REPO_SCOPED_KEYS_REFERENCE_MODULE,
};
use serde_json::{json, Value as JsonValue};

fn run(operation: &str, payload: JsonValue) -> JsonValue {
    identity_json(operation, &payload).expect("identity operation should succeed")
}

fn array_contains(value: &JsonValue, expected: &str) -> bool {
    value
        .as_array()
        .expect("value should be an array")
        .iter()
        .any(|item| item == expected)
}

#[test]
fn identity_contract_reports_removed_python_reference_and_excluded_db_backfills() {
    let value = run("contract", json!({}));
    assert_eq!(value["contract"], json!(IDENTITY_CONTRACT));
    assert_eq!(
        value["migration_status"],
        json!("rust_owned_no_python_reference")
    );
    assert!(array_contains(
        &value["reference_modules"],
        REPO_SCOPED_KEYS_REFERENCE_MODULE
    ));
    assert_eq!(value["mutates_state"], json!(false));
    assert!(array_contains(
        &value["excluded_reference_behaviors"],
        "control-plane repo_id backfill"
    ));
    assert!(array_contains(&value["operations"], "normalize-task-row"));
}

#[test]
fn repo_scope_predicates_and_scope_assertions_match_reference_diagnostics() {
    assert_eq!(
        repo_scope_predicate(None),
        "(repo_id = ? or (repo_id is null and repo_name = ?))"
    );
    assert_eq!(
        repo_scope_predicate(Some("c")),
        "(c.repo_id = ? or (c.repo_id is null and c.repo_name = ?))"
    );
    assert_eq!(
        assert_repo_scope("ait", "REPO-1", Some("REPO-1")).unwrap(),
        "REPO-1"
    );
    assert_eq!(assert_repo_scope("ait", "REPO-1", None).unwrap(), "REPO-1");
    assert_eq!(
        assert_repo_scope("ait", "REPO-1", Some("REPO-2")).unwrap_err(),
        "Repository scope mismatch for ait: repo_id REPO-2 does not match REPO-1"
    );

    let value = run("repo-scope-predicate", json!({"alias": "lr"}));
    assert_eq!(
        value["predicate"],
        json!("(lr.repo_id = ? or (lr.repo_id is null and lr.repo_name = ?))")
    );
}

#[test]
fn local_id_and_sequence_helpers_preserve_python_dash_rules() {
    assert_eq!(
        local_id_after_first_dash(Some(" RSEC-0133 ")),
        Some("0133".to_string())
    );
    assert_eq!(
        local_id_after_first_dash(Some("RSEC-")),
        Some("RSEC-".to_string())
    );
    assert_eq!(
        local_id_after_first_dash(Some("0133")),
        Some("0133".to_string())
    );
    assert_eq!(local_id_after_first_dash(Some("  ")), None);
    assert_eq!(sequence_after_first_dash(Some("RSEC-0133")), Some(133));
    assert_eq!(sequence_after_first_dash(Some("RSEC-0133-extra")), None);
    assert_eq!(sequence_after_last_dash(Some("LAND-RSEC-1-0003")), Some(3));
    assert_eq!(sequence_after_last_dash(Some("RSEC-nope")), None);
    assert_eq!(repo_scoped_sequence_ref(Some("0012")), Some(12));
    assert_eq!(repo_scoped_sequence_ref(Some("12x")), None);

    let value = run("sequence-after-first-dash", json!({"value": "RSET-0237"}));
    assert_eq!(value["sequence"], json!(237));
    let numeric = run("local-id-after-first-dash", json!({"value": 23}));
    assert_eq!(numeric["local_id"], json!("23"));
    let zero = run("local-id-after-first-dash", json!({"value": 0}));
    assert!(zero["local_id"].is_null());
}

#[test]
fn row_normalization_removes_retired_risk_lane_fields_only() {
    let row = json!({
        "task_id": "RSET-1",
        "repo_id": "REPO-1",
        "risk_tier": "high",
        "lane": "legacy",
        "status": "active"
    });
    let object = row.as_object().expect("row should be object");
    let task = normalize_task_row(object);
    let change = normalize_change_row(object);
    assert!(task.get("risk_tier").is_none());
    assert!(task.get("lane").is_none());
    assert_eq!(task.get("task_id"), Some(&json!("RSET-1")));
    assert_eq!(change.get("status"), Some(&json!("active")));

    let value = run("normalize-task-row", json!({"row": row}));
    assert!(value["row"].get("risk_tier").is_none());
    assert!(value["row"].get("lane").is_none());
    assert_eq!(value["row"]["repo_id"], json!("REPO-1"));
}

#[test]
fn patchset_id_derivation_is_shared_with_server_bridge() {
    assert_eq!(derive_patchset_id("RC-0008", 1), "RP-0008-1");
    assert_eq!(derive_patchset_id("LC-0008", 1), "LP-0008-1");
    assert_eq!(derive_patchset_id("RCC-0230", 2), "RCP-0230-2");
    assert_eq!(derive_patchset_id("LCC-0230", 2), "LCP-0230-2");
    assert_eq!(derive_patchset_id("RSEC-0133", 1), "RSEP-0133-1");
    assert_eq!(derive_patchset_id("LSEC-0133", 1), "LSEP-0133-1");
    assert_eq!(derive_patchset_id("C-0008", 1), "P-0008-1");
    assert_eq!(derive_patchset_id("CC-0230", 2), "P-CC-0230-2");

    let value = run(
        "derive-patchset-id",
        json!({"change_id": "RSEC-0133", "patchset_number": "3"}),
    );
    assert_eq!(value["patchset_id"], json!("RSEP-0133-3"));
}
