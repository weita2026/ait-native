use super::*;

fn source(root: &Path, name: &str, repository_index: u32, bytes: &[u8]) -> ManagedCiPackage {
    let worktree = root.join(name);
    fs::create_dir_all(worktree.join("source")).unwrap();
    fs::write(worktree.join("package.zst"), bytes).unwrap();
    ManagedCiPackage::open(
        repository_index,
        ManagedCiPackagePaths {
            managed_root: root.to_path_buf(),
            sources: worktree.join("source"),
            package: worktree.join("package.zst"),
            worktree,
        },
        Sha256::digest(bytes).into(),
        PreparedCiTarget {
            target_line: "main".to_string(),
            base_snapshot_id: "SNP-000000000001".to_string(),
            revision_snapshot_id: "SNP-000000000002".to_string(),
        },
    )
    .unwrap()
}

fn key(repository_index: u32, worker_job_index: u32) -> WorkerJobKey {
    WorkerJobKey {
        repository_index,
        worker_job_index,
    }
}

fn passed(at: u64) -> WorkerJobCompactCiEvidence {
    WorkerJobCompactCiEvidence {
        ci_completed_at_s: at,
        ci_run_seq: 1,
        selected_suite_count: 1,
        suite_result_count: 1,
        blocking_failure_count: 0,
        overall_status: CI_STATUS_PASS,
        tests_status: CI_STATUS_PASS,
        lint_status: CI_STATUS_PASS,
    }
}

#[test]
fn managed_source_verifies_compressed_bytes_and_separates_test_tree() {
    let root = tempfile::tempdir().unwrap();
    let source = source(root.path(), "a", 0, b"compressed bytes");
    assert!(!source.package().starts_with(source.sources()));
    fs::write(source.sources().join("test-output"), b"generated").unwrap();
    source.verify().unwrap();
    fs::write(source.package(), b"changed compressed bytes").unwrap();
    assert!(source.verify().unwrap_err().contains("hash changed"));
}

#[test]
fn package_inside_test_sources_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let worktree = root.path().join("worktree");
    let sources = worktree.join("sources");
    fs::create_dir_all(&sources).unwrap();
    let package = sources.join("package.zst");
    fs::write(&package, b"bytes").unwrap();
    assert!(ManagedCiPackage::open(
        0,
        ManagedCiPackagePaths {
            managed_root: root.path().to_path_buf(),
            worktree,
            sources,
            package,
        },
        Sha256::digest(b"bytes").into(),
        PreparedCiTarget {
            target_line: "main".to_string(),
            base_snapshot_id: "base".to_string(),
            revision_snapshot_id: "final".to_string(),
        }
    )
    .is_err());
}

#[test]
fn winner_uses_atomic_recording_order_and_hands_over_without_rerun() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let first = source(root.path(), "first", 0, b"first");
    let second = source(root.path(), "second", 0, b"second");
    runtime.register(key(0, 0), first).unwrap();
    runtime.register(key(0, 1), second.clone()).unwrap();
    runtime.record_result(key(0, 1), passed(900)).unwrap();
    runtime.record_result(key(0, 0), passed(100)).unwrap();
    assert_eq!(
        runtime.winner(0, "main", "SNP-000000000001").unwrap(),
        Some(key(0, 1))
    );
    runtime.record_result(key(0, 1), passed(900)).unwrap();
    runtime
        .release_priority(key(0, 1), second.sha256())
        .unwrap();
    assert_eq!(
        runtime.winner(0, "main", "SNP-000000000001").unwrap(),
        Some(key(0, 0))
    );
    assert_eq!(runtime.evidence(key(0, 0)).unwrap(), Some(passed(100)));
    assert_eq!(
        runtime.winner(0, "other", "SNP-000000000001").unwrap(),
        None
    );
    assert_eq!(runtime.winner(0, "main", "changed-base").unwrap(), None);
}

#[test]
fn selecting_a_new_job_invalidates_old_completion_and_priority() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let package = source(root.path(), "first", 0, b"same");
    runtime.register(key(0, 0), package.clone()).unwrap();
    runtime.record_result(key(0, 0), passed(100)).unwrap();
    runtime.register(key(0, 1), package).unwrap();
    assert_eq!(runtime.winner(0, "main", "SNP-000000000001").unwrap(), None);
    assert!(runtime.record_result(key(0, 0), passed(100)).is_err());
    assert_eq!(runtime.evidence(key(0, 1)).unwrap(), None);
    runtime.record_result(key(0, 1), passed(200)).unwrap();
    assert_eq!(
        runtime.winner(0, "main", "SNP-000000000001").unwrap(),
        Some(key(0, 1))
    );
    runtime.record_result(key(0, 1), passed(201)).unwrap();
    assert_eq!(runtime.evidence(key(0, 1)).unwrap(), Some(passed(200)));
    assert!(runtime
        .record_result(
            key(0, 1),
            WorkerJobCompactCiEvidence {
                ci_run_seq: 2,
                ..passed(202)
            }
        )
        .is_err());
}

#[test]
fn same_hash_and_job_ordinal_are_repository_scoped() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let first = source(root.path(), "first", 0, b"same");
    let second = source(root.path(), "second", 1, b"same");
    runtime.register(key(0, 0), first.clone()).unwrap();
    runtime.register(key(1, 0), second).unwrap();
    runtime.record_result(key(1, 0), passed(100)).unwrap();
    assert_eq!(
        runtime.selected_job(0, first.sha256()).unwrap(),
        Some(key(0, 0))
    );
    assert_eq!(runtime.winner(0, "main", "SNP-000000000001").unwrap(), None);
    assert_eq!(
        runtime.winner(1, "main", "SNP-000000000001").unwrap(),
        Some(key(1, 0))
    );
    assert!(runtime.source(key(2, 0)).is_err());
}

fn review(
    source: &ManagedCiPackage,
    key: WorkerJobKey,
    id: &str,
    action: &str,
) -> PackageEvidenceRequest {
    PackageEvidenceRequest::parse(&serde_json::json!({
        "contract":"workflow-package-evidence/v1", "operation":"review",
        "idempotency_key":id, "actor_key_index":0,
        "expected_identity":source.expected_identity(key).unwrap(),
        "payload":{"reviewer":"reviewer","action":action,"reviewer_kind":"user"}
    }))
    .unwrap()
}

#[test]
fn completed_execution_with_failed_or_missing_tests_never_gets_priority() {
    let root = tempfile::tempdir().unwrap();
    for (index, evidence) in [
        WorkerJobCompactCiEvidence {
            tests_status: crate::foundation::workflow_binary_v0::CI_STATUS_FAIL,
            ..passed(100)
        },
        WorkerJobCompactCiEvidence {
            suite_result_count: 0,
            selected_suite_count: 0,
            ..passed(100)
        },
    ]
    .into_iter()
    .enumerate()
    {
        let runtime = PreparedCiRuntime::default();
        runtime
            .register(
                key(0, 0),
                source(root.path(), &index.to_string(), 0, b"same"),
            )
            .unwrap();
        runtime.record_result(key(0, 0), evidence).unwrap();
        assert_eq!(runtime.winner(0, "main", "SNP-000000000001").unwrap(), None);
    }
}

#[test]
fn evidence_retry_keeps_original_time_and_changed_body_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let package = source(root.path(), "a", 0, b"same");
    runtime.register(key(0, 0), package.clone()).unwrap();
    let request = review(&package, key(0, 0), "approval", "approve");
    assert!(runtime
        .accept_evidence(key(0, 0), request.clone(), 1, 110)
        .is_err());
    runtime.record_result(key(0, 0), passed(100)).unwrap();
    assert_eq!(
        runtime
            .accept_evidence(key(0, 0), request.clone(), 1, 110)
            .unwrap()
            .recorded_at_s,
        110
    );
    assert_eq!(
        runtime
            .accept_evidence(key(0, 0), request, 1, 200)
            .unwrap()
            .recorded_at_s,
        110
    );
    assert!(runtime
        .accept_evidence(
            key(0, 0),
            review(&package, key(0, 0), "approval", "request_changes"),
            1,
            201
        )
        .is_err());
    assert!(runtime
        .accept_evidence(
            key(0, 1),
            review(&package, key(0, 0), "other", "approve"),
            1,
            202
        )
        .is_err());
}

#[test]
fn later_rejection_releases_priority_and_fresh_approval_can_restore_it() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let first = source(root.path(), "a", 0, b"first");
    runtime.register(key(0, 0), first.clone()).unwrap();
    runtime
        .register(key(0, 1), source(root.path(), "b", 0, b"second"))
        .unwrap();
    runtime.record_result(key(0, 0), passed(100)).unwrap();
    runtime.record_result(key(0, 1), passed(101)).unwrap();
    runtime
        .accept_evidence(
            key(0, 0),
            review(&first, key(0, 0), "approve", "approve"),
            1,
            102,
        )
        .unwrap();
    runtime
        .accept_evidence(
            key(0, 0),
            review(&first, key(0, 0), "reject", "request_changes"),
            1,
            103,
        )
        .unwrap();
    assert_eq!(
        runtime.winner(0, "main", "SNP-000000000001").unwrap(),
        Some(key(0, 1))
    );
    // Retrying an older approval cannot hide a later rejection.
    runtime
        .accept_evidence(
            key(0, 0),
            review(&first, key(0, 0), "approve", "approve"),
            1,
            104,
        )
        .unwrap();
    assert_eq!(
        runtime.winner(0, "main", "SNP-000000000001").unwrap(),
        Some(key(0, 1))
    );
    runtime
        .accept_evidence(
            key(0, 0),
            review(&first, key(0, 0), "approve-again", "approve"),
            1,
            105,
        )
        .unwrap();
    assert_eq!(
        runtime.winner(0, "main", "SNP-000000000001").unwrap(),
        Some(key(0, 0))
    );
    assert_eq!(runtime.evidence(key(0, 1)).unwrap(), Some(passed(101)));
}

#[test]
fn terminal_projection_distinguishes_missing_test_evidence_from_failed_tests() {
    assert_eq!(prepared_ci_state(None), "unknown");
    assert_eq!(
        prepared_ci_state(Some(WorkerJobCompactCiEvidence {
            selected_suite_count: 0,
            suite_result_count: 0,
            ..passed(100)
        })),
        "unknown"
    );
    assert_eq!(
        prepared_ci_state(Some(WorkerJobCompactCiEvidence {
            tests_status: CI_STATUS_FAIL,
            ..passed(100)
        })),
        "failed"
    );
    assert_eq!(prepared_ci_state(Some(passed(100))), "passed");
}

#[test]
fn evidence_capacity_rejects_new_events_but_preserves_exact_retry_and_new_intent_order() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let source = source(root.path(), "capacity", 0, b"capacity");
    let job = key(0, 0);
    let identity = source.expected_identity(job).unwrap();
    runtime.register(job, source).unwrap();
    runtime.record_result(job, passed(10)).unwrap();
    let request = |operation: &str, key: String| {
        PackageEvidenceRequest::parse(&serde_json::json!({
        "contract":"workflow-package-evidence/v1","operation":operation,"idempotency_key":key,"actor_key_index":0,"expected_identity":identity,
        "payload":if operation == "review" { serde_json::json!({"reviewer":"bot","reviewer_kind":"bot","action":"code_review_summary"}) } else { serde_json::json!({"author_mode":"ai_only_experimental","verification_state":"pass"}) }
    })).unwrap()
    };
    for i in 0..64 {
        runtime
            .accept_evidence(job, request("review", format!("decision-{i}")), 1, 100 + i)
            .unwrap();
    }
    let retry = request("review", "decision-0".into());
    assert_eq!(
        runtime
            .accept_evidence(job, retry.clone(), 1, 999)
            .unwrap()
            .recorded_at_s,
        100
    );
    assert!(runtime
        .accept_evidence(job, request("review", "decision-64".into()), 1, 999)
        .unwrap_err()
        .to_string()
        .contains("capacity"));
    let mut changed = retry;
    changed
        .payload
        .insert("action".into(), serde_json::json!("request_changes"));
    assert!(runtime
        .accept_evidence(job, changed, 1, 999)
        .unwrap_err()
        .to_string()
        .contains("different body"));
    // Three prior formal Attestations leave 61 events, independently of Reviews.
    for i in 0..61 {
        runtime
            .accept_evidence_with_capacity(
                job,
                request("attestation", format!("attest-{i}")),
                1,
                200 + i,
                || Ok(61),
            )
            .unwrap();
    }
    runtime
        .accept_evidence_with_capacity(
            job,
            request("attestation", "attest-0".into()),
            1,
            999,
            || panic!("exact retry must not consume/check a new slot"),
        )
        .unwrap();
    assert!(runtime
        .accept_evidence_with_capacity(
            job,
            request("attestation", "attest-61".into()),
            1,
            999,
            || Ok(61)
        )
        .is_err());
}

#[test]
fn committed_cleanup_retains_exact_owner_after_failure_and_recovers_without_finish_or_ci() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let source = source(root.path(), "committed", 0, b"committed");
    let job = key(0, 0);
    runtime.register(job, source.clone()).unwrap();
    runtime.record_result(job, passed(10)).unwrap();
    runtime
        .with_finish_candidate(job, source.sha256(), |_, _, _| Ok(()))
        .unwrap();
    assert!(!runtime.recoverable_selection(job));
    let moved = root.path().join("retained-for-fault");
    fs::rename(source.worktree(), &moved).unwrap();
    fs::write(source.worktree(), b"unexpected path owner").unwrap();
    assert!(runtime.cleanup_committed(job).is_err());
    assert_eq!(
        fs::read(source.worktree()).unwrap(),
        b"unexpected path owner"
    );
    assert!(moved.join("package.zst").exists());
    assert!(runtime
        .with_finish_candidate(job, source.sha256(), |_, _, _| Ok(()))
        .is_err());
    fs::remove_file(source.worktree()).unwrap();
    fs::rename(&moved, source.worktree()).unwrap();
    assert!(runtime.cleanup_committed(job).unwrap());
    assert!(!source.worktree().exists());
    assert!(runtime.source(job).is_err());
    assert!(!runtime.cleanup_committed(job).unwrap());
    // Files alone after a restart never grant this exact committed RAM marker.
    assert!(!PreparedCiRuntime::default().cleanup_committed(job).unwrap());
}

#[test]
fn committed_native_response_survives_reader_cleanup_and_releases_only_on_strict_finish() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let source = source(root.path(), "committed-reader", 0, b"sealed");
    let job = key(0, 0);
    runtime.register(job, source.clone()).unwrap();
    runtime.record_result(job, passed(10)).unwrap();
    let reader = source.pin_download().unwrap();
    runtime
        .with_finish_candidate(job, source.sha256(), |_, _, _| Ok(()))
        .unwrap();
    let result = serde_json::json!({
        "outcome":"confirmed", "expected_identity":source.expected_identity(job).unwrap(),
        "entries":[{"local_task_id":"LT-0001","remote_task_id":"RT-0001"}]
    });
    runtime
        .retain_committed_result(job, result.clone())
        .unwrap();
    assert!(!runtime.cleanup_committed(job).unwrap());
    assert!(runtime.release_completed_result(job).is_err());
    assert!(source.worktree().is_dir());
    assert!(source.pin_download().is_err());
    drop(reader);
    assert!(!source.worktree().exists());
    assert!(runtime.cleanup_committed(job).unwrap());
    assert!(runtime.committed_cleanup_complete(job).unwrap());
    assert_eq!(runtime.committed_result(job).unwrap(), Some(result));
    assert!(runtime.committed_source(job).unwrap().is_some());
    runtime.release_completed_result(job).unwrap();
    assert!(runtime.committed_result(job).unwrap().is_none());
    assert!(runtime.committed_source(job).unwrap().is_none());
    assert!(PreparedCiRuntime::default()
        .committed_result(job)
        .unwrap()
        .is_none());
}

#[test]
fn parent_finalization_failure_after_source_removal_keeps_original_result_and_owner() {
    let root = tempfile::tempdir().unwrap();
    let runtime = PreparedCiRuntime::default();
    let source = source(root.path(), "committed-parent", 0, b"sealed");
    let job = key(0, 0);
    runtime.register(job, source.clone()).unwrap();
    runtime.record_result(job, passed(10)).unwrap();
    runtime
        .with_finish_candidate(job, source.sha256(), |_, _, _| Ok(()))
        .unwrap();
    let result = serde_json::json!({"expected_identity":source.expected_identity(job).unwrap(),"outcome":"confirmed","entries":[]});
    runtime
        .retain_committed_result(job, result.clone())
        .unwrap();
    let obstructed_parent = root.path().join("parent-obstruction");
    fs::write(&obstructed_parent, b"not a directory").unwrap();
    let error = runtime
        .cleanup_committed_with(job, |source| {
            assert!(source.request_cleanup()?);
            fs::remove_dir(&obstructed_parent)
                .map_err(|error| BinaryDbError::io("finalize parent", error))?;
            Ok(true)
        })
        .unwrap_err();
    assert_eq!(
        error.kind(),
        crate::foundation::remote_binary_db::BinaryDbErrorKind::Io
    );
    assert!(!source.worktree().exists());
    assert!(!runtime.committed_cleanup_complete(job).unwrap());
    assert!(runtime.release_completed_result(job).is_err());
    assert_eq!(runtime.committed_result(job).unwrap(), Some(result.clone()));
    let mut wrong = result.clone();
    wrong["expected_identity"]["worker_job_index"] = serde_json::json!(1);
    assert!(runtime.retain_committed_result(job, wrong).is_err());
    // Once the owned child is removed, a recreated path is another resource,
    // even while the original owner's hash-parent finalization is pending.
    fs::create_dir(source.worktree()).unwrap();
    let replacement = source.worktree().join("foreign-resource");
    fs::write(&replacement, b"preserve").unwrap();
    assert!(runtime.cleanup_committed(job).is_err());
    assert_eq!(fs::read(&replacement).unwrap(), b"preserve");
    fs::remove_dir_all(source.worktree()).unwrap();
    fs::remove_file(&obstructed_parent).unwrap();
    assert!(runtime.cleanup_committed(job).unwrap());
    assert_eq!(runtime.committed_result(job).unwrap(), Some(result));
    runtime.release_completed_result(job).unwrap();
}
