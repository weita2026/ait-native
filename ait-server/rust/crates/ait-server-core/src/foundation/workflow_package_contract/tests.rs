use super::*;
use serde_json::json;

fn identity() -> Value {
    json!({"repository_index":0,"package_hash":"ab".repeat(32),"worker_job_index":0,
        "base_snapshot_id":"SNP-000000000001","revision_snapshot_id":"SNP-000000000002"})
}

#[test]
fn package_identity_preserves_zero_keys_and_rejects_mixed_missing_or_invalid_fields() {
    let parsed = PackageExpectedIdentity::parse(&identity()).unwrap();
    assert_eq!(parsed.worker_job_index, 0);
    assert_eq!(parsed.repository_index, 0);
    assert_eq!(serde_json::to_value(&parsed).unwrap(), identity());
    for field in [
        "repository_index",
        "package_hash",
        "worker_job_index",
        "base_snapshot_id",
        "revision_snapshot_id",
    ] {
        let mut missing = identity();
        missing.as_object_mut().unwrap().remove(field);
        assert!(PackageExpectedIdentity::parse(&missing).is_err());
        let mut null = identity();
        null[field] = Value::Null;
        assert!(PackageExpectedIdentity::parse(&null).is_err());
    }
    for (field, value) in [
        ("patchset_id", json!("RT-0001/P-01")),
        ("extra", json!(0)),
        ("package_hash", json!("AB".repeat(32))),
        ("package_hash", json!("00".repeat(31))),
        ("worker_job_index", json!(-1)),
        ("worker_job_index", json!(4294967296_u64)),
        ("base_snapshot_id", json!("invalid")),
    ] {
        let mut invalid = identity();
        invalid[field] = value;
        assert!(PackageExpectedIdentity::parse(&invalid).is_err());
    }
}

#[test]
fn package_finish_cannot_supply_remote_or_local_task_selectors() {
    let expected = PackageExpectedIdentity::parse(&identity()).unwrap();
    let request = json!({"contract":PACKAGE_FINISH_CONTRACT,"idempotency_key":expected.idempotency_key(),
        "actor_key_index":0,"target_line":"main","mode":"direct","expected_identity":identity()});
    PackageFinishRequest::parse(&request).unwrap();
    for field in [
        "task_or_change_ref",
        "patchset_id",
        "history_entries",
        "package",
    ] {
        let mut invalid = request.clone();
        invalid[field] = json!("not allowed");
        assert!(PackageFinishRequest::parse(&invalid).is_err());
    }
    let mut wrong_key = request;
    wrong_key["idempotency_key"] = json!("another-key");
    assert!(PackageFinishRequest::parse(&wrong_key).is_err());
}

#[test]
fn package_evidence_rejects_forged_time_results_and_group_requests() {
    let request = json!({"contract":PACKAGE_EVIDENCE_CONTRACT,"operation":"review","idempotency_key":"review-1",
        "actor_key_index":0,"expected_identity":identity(),"payload":{"reviewer":"Reviewer","action":"approve"}});
    PackageEvidenceRequest::parse(&request).unwrap();
    for field in [
        "review_id",
        "patchset_id",
        "created_at_s",
        "recorded_at_s",
        "reviewer_groups",
        "patchset_ci",
    ] {
        let mut invalid = request.clone();
        invalid["payload"][field] = json!("not allowed");
        assert!(PackageEvidenceRequest::parse(&invalid).is_err());
    }
    let mut invalid = request;
    invalid["operation"] = json!("attestation");
    invalid["payload"] = json!({"detail":{"patchset_ci":{}}});
    assert!(PackageEvidenceRequest::parse(&invalid).is_err());
}
