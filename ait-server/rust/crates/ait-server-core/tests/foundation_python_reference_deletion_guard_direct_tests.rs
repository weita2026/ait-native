use ait_server_core::foundation::python_reference_deletion_guard::{
    package_marker_modules, python_reference_deletion_guard_contract,
    python_reference_deletion_guard_json, python_reference_modules,
    PYTHON_REFERENCE_DELETION_GUARD_CONTRACT_VERSION, PYTHON_REFERENCE_INVENTORY,
};
use serde_json::json;

#[test]
fn python_reference_deletion_guard_declares_zero_python_server_sources() {
    let contract = python_reference_deletion_guard_contract();

    assert_eq!(
        contract["contract"],
        json!(PYTHON_REFERENCE_DELETION_GUARD_CONTRACT_VERSION)
    );
    assert_eq!(contract["reference_root"], json!("../ait/src/ait_server"));
    assert_eq!(contract["expected_reference_count"], json!(0));
    assert_eq!(contract["cross_repo_audit"]["expected_count"], json!(0));
    assert!(PYTHON_REFERENCE_INVENTORY.is_empty());
    assert!(python_reference_modules().is_empty());
    assert!(package_marker_modules().is_empty());
    assert_eq!(contract["inventory"], json!([]));
    assert_eq!(contract["package_markers"], json!([]));
}

#[test]
fn python_reference_deletion_guard_accepts_only_an_empty_audit() {
    let clean = python_reference_deletion_guard_json("audit", &json!({"references": []}))
        .expect("empty audit");
    assert_eq!(clean["matches_expected"], json!(true));
    assert_eq!(clean["expected_count"], json!(0));
    assert_eq!(clean["observed_count"], json!(0));

    let stale = python_reference_deletion_guard_json(
        "audit",
        &json!({
            "references": [
                "../ait/src/ait_server/session_routes.py",
                "../ait/src/ait_server/session_routes.py"
            ]
        }),
    )
    .expect("stale audit");
    assert_eq!(stale["matches_expected"], json!(false));
    assert_eq!(
        stale["unknown_observed"],
        json!(["../ait/src/ait_server/session_routes.py"])
    );
    assert_eq!(
        stale["duplicate_observed"],
        json!([{"module": "../ait/src/ait_server/session_routes.py", "count": 2}])
    );
}

#[test]
fn python_reference_deletion_guard_rejects_all_retired_modules() {
    for operation in ["classify", "fallback-decision", "migration-decision"] {
        let error = python_reference_deletion_guard_json(
            operation,
            &json!({
                "module": "../ait/src/ait_server/session_routes.py",
                "requires_python_fallback": true,
                "migration_kind": "delete_after_rust_owner"
            }),
        )
        .expect_err("retired module must be unknown");
        assert_eq!(
            error,
            "Unknown ait-server Python reference module: `../ait/src/ait_server/session_routes.py`"
        );
    }
}
