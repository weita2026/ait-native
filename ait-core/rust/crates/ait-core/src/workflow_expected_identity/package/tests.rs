use super::*;
use serde_json::json;

fn value() -> Value {
    json!({"repository_index":0,"package_hash":"0123456789abcdef".repeat(4),"worker_job_index":0,
        "base_snapshot_id":"SNP-000000000001","revision_snapshot_id":"SNP-000000000002"})
}

#[test]
fn package_identity_preserves_zero_indexes_and_round_trips_as_exact_five_field_object() {
    let value = value();
    let WorkflowExpectedIdentityInput::Package(identity) =
        WorkflowExpectedIdentityInput::from_value(&value).unwrap()
    else {
        panic!("package form")
    };
    assert_eq!(identity.repository_index, 0);
    assert_eq!(identity.worker_job_index, 0);
    assert_eq!(identity.to_value(), value);
    let bytes = serde_json::to_vec(&identity).unwrap();
    assert_eq!(
        WorkflowExpectedIdentityInput::from_slice(&bytes).unwrap(),
        WorkflowExpectedIdentityInput::Package(identity)
    );
}

#[test]
fn identity_forms_reject_missing_null_mixed_unknown_and_duplicate_fields() {
    let value = value();
    for field in value.as_object().unwrap().keys() {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            WorkflowExpectedIdentityInput::from_value(&missing).is_err(),
            "{field}"
        );
        let mut null = value.clone();
        null[field] = Value::Null;
        assert!(
            WorkflowExpectedIdentityInput::from_value(&null).is_err(),
            "{field}"
        );
    }
    for field in ["patchset_id", "job_id", "extra"] {
        let mut mixed = value.clone();
        mixed[field] = json!("unexpected");
        assert!(WorkflowExpectedIdentityInput::from_value(&mixed).is_err());
    }
    let duplicate =
        serde_json::to_string(&value)
            .unwrap()
            .replacen("{", "{\"worker_job_index\":5,", 1);
    assert!(WorkflowExpectedIdentityInput::from_slice(duplicate.as_bytes()).is_err());
    let legacy = json!({"repository_index":1,"patchset_id":"RCT-1/P-01","base_snapshot_id":"B","revision_snapshot_id":"R"});
    assert!(matches!(
        WorkflowExpectedIdentityInput::from_value(&legacy),
        Ok(WorkflowExpectedIdentityInput::Patchset(_))
    ));
    let mut mixed = legacy;
    mixed["worker_job_index"] = json!(0);
    assert!(WorkflowExpectedIdentityInput::from_value(&mixed).is_err());
}

#[test]
fn malformed_hash_snapshot_and_numeric_fields_fail_closed_without_panics() {
    for (field, malformed) in [
        ("package_hash", json!("AB".repeat(32))),
        ("package_hash", json!("ab".repeat(31))),
        ("package_hash", json!("gg".repeat(32))),
        ("worker_job_index", json!(-1)),
        ("worker_job_index", json!(u64::from(u32::MAX) + 1)),
        ("repository_index", json!("0")),
        ("base_snapshot_id", json!("SNP-other")),
        ("base_snapshot_id", json!("SNP-000000000001 ")),
        ("revision_snapshot_id", json!("例例例例")),
    ] {
        let mut invalid = value();
        invalid[field] = malformed;
        assert!(
            WorkflowExpectedIdentityInput::from_value(&invalid).is_err(),
            "{field}"
        );
    }
}

#[test]
fn selected_ready_identity_cannot_switch_jobs_packages_repositories_or_snapshots_on_retry() {
    let identity = PackageWorkflowExpectedIdentity::from_value(&value()).unwrap();
    let mut selection = PackageReadySelection::fresh();
    assert!(selection.may_select_job());
    selection.select(identity.clone()).unwrap();
    assert!(!selection.may_select_job());
    selection.select(identity.clone()).unwrap();
    for (field, changed) in [
        ("repository_index", json!(1)),
        ("worker_job_index", json!(1)),
        ("package_hash", json!("ff".repeat(32))),
        ("base_snapshot_id", json!("SNP-000000000003")),
        ("revision_snapshot_id", json!("SNP-000000000004")),
    ] {
        let mut changed_value = value();
        changed_value[field] = changed;
        assert!(selection
            .select(PackageWorkflowExpectedIdentity::from_value(&changed_value).unwrap())
            .is_err());
        assert_eq!(selection.expected_identity(), Some(&identity));
    }
    let recovered = PackageReadySelection::recover(identity.clone()).unwrap();
    assert!(!recovered.may_select_job());
    assert_eq!(recovered.expected_identity(), Some(&identity));
}
