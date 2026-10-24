use super::*;
use crate::foundation::operational_binary_v0::*;
use crate::foundation::prepared_ci_runtime::{
    ManagedCiPackage, ManagedCiPackagePaths, PreparedCiTarget,
};
use crate::foundation::remote_binary_db::{BinaryDbError, StoreResult};
use crate::foundation::server_operational_job_domain::WorkerJobCompactCiEvidence;
use crate::foundation::server_operational_repository_registry::canonical_repository_directory_name;
use crate::foundation::server_operational_worker_jobs::{
    PreparedCiPublication, ServerOperationalWorkerJobStore, WorkerJobDomainAuthority,
};
use crate::foundation::workflow_binary_v0::CI_STATUS_PASS;
use crate::foundation::workflow_package::WorkflowPackageManifest;
use crate::foundation::workflow_package_contract::{PackageEvidenceRequest, PackageFinishRequest};
use sha2::{Digest, Sha256};
use std::sync::Arc;

struct Domain;
impl WorkerJobDomainAuthority for Domain {
    fn validate_patchset_index(&self, _: u32) -> StoreResult<()> {
        Ok(())
    }
    fn validate_snapshot_index(&self, _: u32) -> StoreResult<()> {
        Ok(())
    }
}

#[test]
fn package_five_task_rebased_mixed_history_rolls_back_then_binds_original_job_atomically() {
    let root = tempfile::tempdir().unwrap();
    let original = initialized_db("package-atomic");
    let authority = root.path().join(canonical_repository_directory_name(0));
    fs::rename(
        ServerRemoteBinaryDb::authority_root(&original).as_path(),
        &authority,
    )
    .unwrap();
    let db = TestDb::test_fixture(
        RepoId::new("REPO-V0"),
        RepoName::new("repo"),
        StorePath::new(authority.clone()),
        StoreGeneration::new(1),
    );
    seed_content(&db);
    let lines = ServerBinaryDbLineStore::<_, 1>::new(db.clone());
    lines.set_line_head("main", 2, 3).unwrap();
    let source_line = lines.create_line("feature/package", 0, 1).unwrap();
    let snapshots = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
    let mut original_parent = 0;
    let mut tested_parent = 1;
    let mut entries = Vec::new();
    for task in 0..5 {
        let local = format!("LSET-{:04}", 1001 + task);
        let original_base = if task == 0 {
            "SNP-0000000000A1".into()
        } else {
            format!("SNP-{:012X}", 0xB00 + (task - 1) * 16 + 2)
        };
        let mut owned = Vec::new();
        for (branch, parent, time) in [
            (0, &mut original_parent, 10 + task * 40),
            (1, &mut tested_parent, 5 + task * 5),
        ] {
            for step in 0..3 {
                let id = format!(
                    "SNP-{:012X}",
                    if branch == 0 { 0xB00 } else { 0xC00 } + task * 16 + step
                );
                let record = ServerBinarySnapshotRecord {
                    actor_index_plus1: 0,
                    snapshot_meta: 0,
                    history_flags: 0,
                    payload_len: 0,
                    payload_offset: 0,
                    snapshot_hash48: server_snapshot_hash48_from_id(&id).unwrap(),
                    parent_snapshot_index_plus1: *parent + 1,
                    root_tree_pack_index_plus1: 0,
                    root_entry_ordinal: 0,
                    line_index_plus1: source_line + 1,
                    manifest_hash: [0; 32],
                    file_count: 0,
                    total_bytes: 0,
                    created_at_s: (time + step) as u64,
                };
                *parent = snapshots
                    .append_snapshot(
                        &id,
                        record,
                        &ServerBinarySnapshotPayload {
                            line_name: "feature/package".into(),
                            message: None,
                        },
                    )
                    .unwrap();
                owned.push(json!({"snapshot_id":id,"created_at_s":time+step,"local_change_ref":if task==0&&step<2{Some(format!("{local}/C-{:02}",if step==0{3}else{8}))}else{None}}));
            }
        }
        let changes = if task == 0 {
            json!([
                {"local_change_id":"C-03","local_change_ref":format!("{local}/C-03"),"expected_remote_change_ref":null,"change":{"title":"First real owner","base_line":"feature/package","fork_snapshot_id":"SNP-0000000000A1"}},
                {"local_change_id":"C-08","local_change_ref":format!("{local}/C-08"),"expected_remote_change_ref":null,"change":{"title":"Second real owner","base_line":"feature/lct-1122","fork_snapshot_id":"SNP-0000000000A1"}}
            ])
        } else {
            json!([])
        };
        entries.push(json!({"local_task_id":local,"expected_remote_task_id":null,"task":{"title":format!("Source {task}"),"intent":"Publish original completed Local history"},
            "pre_land_target_snapshot_id":original_base,"landed_snapshot_id":format!("SNP-{:012X}",0xB00+task*16+2),"landed_at_s":20+task*40,"changes":changes,"snapshots":owned}));
    }
    let manifest=WorkflowPackageManifest::parse(&serde_json::to_vec(&json!({"contract":"workflow-package/v1","repository_index":0,"target_line":"main","base_snapshot_id":"SNP-0000000000A2","revision_snapshot_id":"SNP-000000000C42",
        "author_mode":"ai_with_human_review","summary":"Human supplied description","history_entries":entries,"content":{"object_packs":[],"tree_packs":[],"blob_locators":[],"tree_locators":[],"snapshots":[],"line_update":null,"capture_binding":null}})).unwrap()).unwrap();
    lines.archive_line("feature/package", 900).unwrap();
    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "CP")
        .unwrap()
        .with_repository_index(0);
    let actor = workflow
        .resolve_user_actor(&UserActorIdentity::new("Reviewer", "reviewer@example.test").unwrap())
        .unwrap();
    let jobs = ServerOperationalWorkerJobStore::new(0, &authority, Arc::new(Domain)).unwrap();
    jobs.initialize().unwrap();
    let worktree = root.path().join("managed/job");
    fs::create_dir_all(worktree.join("source")).unwrap();
    fs::write(worktree.join("package.zst"), b"sealed fixture package").unwrap();
    let source = ManagedCiPackage::open(
        0,
        ManagedCiPackagePaths {
            managed_root: root.path().join("managed"),
            sources: worktree.join("source"),
            package: worktree.join("package.zst"),
            worktree,
        },
        Sha256::digest(b"sealed fixture package").into(),
        PreparedCiTarget {
            target_line: "main".into(),
            base_snapshot_id: manifest.base_snapshot_id.clone(),
            revision_snapshot_id: manifest.revision_snapshot_id.clone(),
        },
    )
    .unwrap();
    let job = jobs.enqueue_prepared_ci(source.clone(), 1, 100).unwrap();
    let ci = WorkerJobCompactCiEvidence {
        ci_completed_at_s: 900,
        ci_run_seq: 1,
        selected_suite_count: 1,
        suite_result_count: 1,
        blocking_failure_count: 0,
        overall_status: CI_STATUS_PASS,
        tests_status: CI_STATUS_PASS,
        lint_status: CI_STATUS_PASS,
    };
    // A domain result whose Queue completion was interrupted cannot retain
    // priority after the actual Job has failed. Allocation order is irrelevant.
    let failed_worktree = root.path().join("managed/failed");
    fs::create_dir_all(failed_worktree.join("source")).unwrap();
    fs::write(failed_worktree.join("package.zst"), b"other sealed package").unwrap();
    let failed_source = ManagedCiPackage::open(
        0,
        ManagedCiPackagePaths {
            managed_root: root.path().join("managed"),
            sources: failed_worktree.join("source"),
            package: failed_worktree.join("package.zst"),
            worktree: failed_worktree,
        },
        Sha256::digest(b"other sealed package").into(),
        source.target().clone(),
    )
    .unwrap();
    let failed_job = jobs
        .enqueue_prepared_ci(failed_source.clone(), 1, 100)
        .unwrap();
    jobs.with_exclusive_queue(|queue| {
        let running = queue.replace(
            failed_job.key.worker_job_index,
            ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_RUNNING,
                attempt_count: 1,
                locked_at_s: 200,
                updated_at_s: 200,
                ..failed_job.record
            },
        )?;
        jobs.commit_prepared_ci_result(
            running,
            WorkerJobCompactCiEvidence {
                ci_completed_at_s: 800,
                ..ci
            },
        )?;
        queue.replace(
            failed_job.key.worker_job_index,
            ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_FAILED,
                outcome_kind: WORKER_JOB_OUTCOME_FAILED,
                error_kind: WORKER_JOB_ERROR_LEASE_EXPIRED,
                locked_at_s: 0,
                updated_at_s: 801,
                ..running.record
            },
        )?;
        Ok(())
    })
    .unwrap();
    jobs.with_exclusive_queue(|queue| {
        let running = queue.replace(
            job.key.worker_job_index,
            ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_RUNNING,
                attempt_count: 1,
                locked_at_s: 200,
                updated_at_s: 200,
                ..job.record
            },
        )?;
        jobs.commit_prepared_ci_result(running, ci)?;
        queue.replace(
            job.key.worker_job_index,
            ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_SUCCEEDED,
                outcome_kind: WORKER_JOB_OUTCOME_COMPLETED,
                locked_at_s: 0,
                updated_at_s: 901,
                ..running.record
            },
        )?;
        Ok(())
    })
    .unwrap();
    let identity = source.expected_identity(job.key).unwrap();
    for (operation, payload, at) in [
        (
            "review",
            json!({"reviewer":"Reviewer","action":"approve","reviewer_kind":"user"}),
            1001,
        ),
        (
            "review",
            json!({"reviewer":"Reviewer","action":"code_review_summary","reviewer_kind":"user"}),
            1003,
        ),
        (
            "review",
            json!({"reviewer":"Reviewer","action":"task_approve","reviewer_kind":"user"}),
            1004,
        ),
        (
            "attestation",
            json!({"author_mode":"ai_with_human_review","verification_state":"pass","require_tests_pass":true,"require_human_review":true,"require_lint_pass":true}),
            1002,
        ),
    ] {
        let request=PackageEvidenceRequest::parse(&json!({"contract":"workflow-package-evidence/v1","operation":operation,"idempotency_key":format!("{operation}-{at}"),"actor_key_index":actor,"expected_identity":identity,"payload":payload})).unwrap();
        let accepted = workflow
            .validate_package_evidence(&request, &manifest.author_mode)
            .unwrap();
        jobs.prepared_ci_runtime()
            .accept_evidence(job.key, request, accepted, at)
            .unwrap();
    }
    let assert_empty = || {
        let read = BinaryDbReadTxn::new(&db);
        assert_eq!(read.record_count(crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::record_file()).unwrap(), 2);
        for name in ["feature/package", "feature/lct-1122"] {
            let projected =
                crate::foundation::workflow_package::projected_package_line("RCPT-0001", name)
                    .unwrap();
            assert!(lines.line_by_name(&read, &projected).unwrap().is_none());
        }
        for file in [
            WorkflowBinaryV0Codec::task_file(),
            WorkflowBinaryV0Codec::change_file(),
            WorkflowBinaryV0Codec::patchset_file(),
            WorkflowBinaryV0Codec::review_file(),
            WorkflowBinaryV0Codec::attest_file(),
            WorkflowBinaryV0Codec::land_file(),
        ] {
            assert_eq!(read.record_count(file).unwrap(), 0);
        }
        assert_eq!(
            lines
                .line_by_name(&read, "main")
                .unwrap()
                .unwrap()
                .1
                .head_snapshot_index(),
            Some(1)
        );
    };
    assert_empty();
    let prepared = workflow
        .prepare_workflow_package_history(&manifest, &identity)
        .unwrap();
    let failed: StoreResult<_> = jobs.finish_prepared_ci(
        &db,
        job.key.worker_job_index,
        source.sha256(),
        |tx, context| {
            let publication = workflow
                .publish_workflow_package_in_write(
                    tx,
                    prepared,
                    &identity,
                    actor,
                    context.evidence,
                    &context.actions,
                    "direct",
                )
                .map_err(BinaryDbError::invalid_domain_data)?;
            let _ = publication;
            Err::<
                crate::foundation::server_operational_worker_jobs::PreparedCiPublication<JsonValue>,
                _,
            >(BinaryDbError::invalid_domain_data(
                "injected precommit publication failure",
            ))
        },
    );
    assert!(failed.err().unwrap().contains("injected precommit"));
    assert_empty();
    assert_eq!(
        jobs.get(job.key.worker_job_index)
            .unwrap()
            .record
            .patchset_index_plus1,
        0
    );
    use crate::foundation::remote_binary_db::test_support::{
        BinaryDbTestFault, BinaryDbTestFaultTiming, BinaryDbTestStorageOperation,
        FaultInjectingServerBinaryDbStore,
    };
    use crate::foundation::remote_binary_db::{
        ServerBinaryDbAuthorityMode, ServerBinaryDbFilesystemStore,
    };
    let faults = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
    let fault_db = FilesystemServerRemoteBinaryDb::with_file_store(
        faults.clone(),
        RepoId::new("REPO-V0"),
        RepoName::new("repo"),
        StorePath::new(authority.clone()),
        StoreGeneration::new(1),
        ServerBinaryDbAuthorityMode::TestFixture,
    );
    let fault_workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(fault_db.clone(), "CP")
        .unwrap()
        .with_repository_index(0);
    let prepared = fault_workflow
        .prepare_workflow_package_history(&manifest, &identity)
        .unwrap();
    faults.arm(BinaryDbTestFault::once(
        BinaryDbTestStorageOperation::OverwriteRange,
        BinaryDbTestFaultTiming::After,
        "worker_job.bin",
    ));
    faults.arm(BinaryDbTestFault::once(
        BinaryDbTestStorageOperation::TruncateFile,
        BinaryDbTestFaultTiming::Before,
        "patchset.bin",
    ));
    let failed = jobs.finish_prepared_ci(
        &fault_db,
        job.key.worker_job_index,
        source.sha256(),
        |tx, context| {
            fault_workflow
                .publish_workflow_package_in_write(
                    tx,
                    prepared,
                    &identity,
                    actor,
                    context.evidence,
                    &context.actions,
                    "direct",
                )
                .map_err(BinaryDbError::invalid_domain_data)
        },
    );
    assert!(failed.is_err());
    assert_eq!(faults.fired_fault_count(), 2);
    assert!(authority.join("server-land.write.journal").exists());
    assert_eq!(
        jobs.get(job.key.worker_job_index).unwrap_err().kind(),
        BinaryDbErrorKind::RetryableBusy
    );
    jobs.recover_prepared_ci_publication(&db).unwrap();
    assert_empty();
    assert_eq!(
        jobs.get(job.key.worker_job_index)
            .unwrap()
            .record
            .patchset_index_plus1,
        0
    );
    let prepared = workflow
        .prepare_workflow_package_history(&manifest, &identity)
        .unwrap();
    let committed = jobs
        .finish_prepared_ci(
            &db,
            job.key.worker_job_index,
            source.sha256(),
            |tx, context| {
                workflow
                    .publish_workflow_package_in_write(
                        tx,
                        prepared,
                        &identity,
                        actor,
                        context.evidence,
                        &context.actions,
                        "direct",
                    )
                    .map_err(BinaryDbError::invalid_domain_data)
            },
        )
        .unwrap();
    let result = &committed.result;
    assert_eq!(result["outcome"], "confirmed");
    let projected_before = ["feature/package", "feature/lct-1122"]
        .into_iter()
        .map(|name| {
            let name =
                crate::foundation::workflow_package::projected_package_line("RCPT-0001", name)
                    .unwrap();
            let record = lines
                .line_by_name(&BinaryDbReadTxn::new(&db), &name)
                .unwrap()
                .unwrap()
                .1;
            assert!(record.is_archived());
            (name, record)
        })
        .collect::<Vec<_>>();
    assert_eq!(result["entries"].as_array().unwrap().len(), 5);
    assert_eq!(
        result["entries"][0]["changes"][0]["local_change_ref"],
        "LSET-1001/C-03"
    );
    assert_eq!(
        result["entries"][0]["changes"][0]["remote_change_ref"],
        "RCPT-0001/C-01"
    );
    assert_eq!(
        result["entries"][0]["changes"][1]["remote_change_ref"],
        "RCPT-0001/C-02"
    );
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .unwrap(),
        5
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        1
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .unwrap(),
        6
    );
    assert_eq!(
        WorkflowBinaryV0Codec::decode_review(
            &read
                .read_record(WorkflowBinaryV0Codec::review_file(), 0)
                .unwrap()
        )
        .unwrap()
        .created_at_s,
        1001
    );
    assert_eq!(
        WorkflowBinaryV0Codec::decode_attest(
            &read
                .read_record(WorkflowBinaryV0Codec::attest_file(), 0)
                .unwrap()
        )
        .unwrap()
        .created_at_s,
        1002
    );
    drop(read);
    let bound = jobs.get(job.key.worker_job_index).unwrap();
    validate_frozen_server_workflow_v0(&db).unwrap();
    assert_eq!(bound.record.patchset_index_plus1, 6);
    assert_eq!(bound.record.attempt_count, 1);
    let accepted = workflow
        .lookup_accepted_workflow_package(bound, &identity.package_hash, actor)
        .unwrap()
        .unwrap();
    assert_eq!(accepted["outcome"], "incomplete");
    assert_eq!(accepted["expected_identity"], result["expected_identity"]);
    assert_eq!(accepted["patchset_id"], result["patchset_id"]);
    assert_eq!(accepted["submission_id"], result["submission_id"]);
    assert_eq!(accepted["entries"], json!([]));
    assert!(workflow
        .lookup_accepted_workflow_package(bound, &"ff".repeat(32), actor)
        .unwrap()
        .is_none());
    assert!(workflow
        .lookup_accepted_workflow_package(
            jobs.get(failed_job.key.worker_job_index).unwrap(),
            &identity.package_hash,
            actor,
        )
        .unwrap()
        .is_none());
    for conflict in ["repository", "job", "state", "kind"] {
        let mut invalid = bound;
        match conflict {
            "repository" => invalid.key.repository_index += 1,
            "job" => invalid.key.worker_job_index += 1,
            "state" => invalid.record.state_kind = WORKER_JOB_STATE_FAILED,
            "kind" => invalid.record.job_kind = 1,
            _ => unreachable!(),
        }
        assert!(
            workflow
                .lookup_accepted_workflow_package(invalid, &identity.package_hash, actor)
                .is_err(),
            "{conflict}"
        );
    }
    assert!(source.worktree().exists());
    let failed_identity = failed_source.expected_identity(failed_job.key).unwrap();
    let cancel=crate::foundation::workflow_package_contract::PackageAbandonRequest::parse(&json!({"contract":"workflow-package-abandon/v1","actor_key_index":actor,"expected_identity":failed_identity,
        "idempotency_key":format!("workflow-package-abandon:{}:{}",failed_identity.package_hash,failed_identity.worker_job_index)})).unwrap();
    assert_eq!(
        jobs.abandon_prepared_ci(&cancel, actor + 1, 2000, |_| Ok(true))
            .unwrap(),
        crate::foundation::prepared_ci_runtime::PackageAbandonState::Unknown
    );
    assert!(failed_source.worktree().exists());
    let request=PackageFinishRequest::parse(&json!({"contract":"task-land-package-guarded/v1","idempotency_key":identity.idempotency_key(),"actor_key_index":actor,"target_line":"main","mode":"direct","expected_identity":identity})).unwrap();
    assert_eq!(
        workflow
            .lookup_workflow_package_finish(bound, &request, Some(&manifest))
            .unwrap()
            .unwrap()["outcome"],
        "incomplete"
    );
    assert_eq!(
        workflow
            .lookup_workflow_package_finish(bound, &request, None)
            .unwrap()
            .unwrap()["outcome"],
        "incomplete"
    );
    let read = BinaryDbReadTxn::new(&db);
    let completed_source = read
        .read_record(WorkflowBinaryV0Codec::task_file(), 4)
        .unwrap();
    let original_receipt = read
        .read_record(WorkflowBinaryV0Codec::patchset_file(), 4)
        .unwrap();
    let (_, template) = snapshots
        .snapshot_by_id(&read, &manifest.revision_snapshot_id)
        .unwrap()
        .unwrap();
    drop(read);
    let original_id = "SNP-000000000B52";
    let tested_id = "SNP-000000000D00";
    for (id, parent, time) in [
        (original_id, original_parent, 210),
        (tested_id, tested_parent, 400),
    ] {
        snapshots
            .append_snapshot(
                id,
                ServerBinarySnapshotRecord {
                    snapshot_hash48: server_snapshot_hash48_from_id(id).unwrap(),
                    parent_snapshot_index_plus1: parent + 1,
                    created_at_s: time,
                    ..template.clone()
                },
                &ServerBinarySnapshotPayload {
                    line_name: "feature/package".into(),
                    message: None,
                },
            )
            .unwrap();
    }
    for (reference, status) in [
        ("RCPT-0001/C-01", "archived"),
        ("RCPT-0001/C-02", "canceled"),
    ] {
        workflow
            .close_change(reference, &json!({"actor_key_index":actor,"status":status}))
            .unwrap();
    }
    let closed_changes = {
        let read = BinaryDbReadTxn::new(&db);
        (0..2)
            .map(|index| {
                read.read_record(WorkflowBinaryV0Codec::change_file(), index)
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let mut extended = manifest.clone();
    extended.base_snapshot_id = manifest.revision_snapshot_id.clone();
    extended.revision_snapshot_id = tested_id.into();
    extended.summary = Some("Overlapping source batch".into());
    extended.history_entries.push(serde_json::from_value(json!({"local_task_id":"LSET-1006","expected_remote_task_id":null,"task":{"title":"Source 6","intent":"Extend already published history"},
        "pre_land_target_snapshot_id":manifest.history_entries.last().unwrap().landed_snapshot_id,"landed_snapshot_id":original_id,"landed_at_s":220,"changes":[],
        "snapshots":[{"snapshot_id":original_id,"created_at_s":210,"local_change_ref":null},{"snapshot_id":tested_id,"created_at_s":400,"local_change_ref":null}]})).unwrap());
    let extended_worktree = root.path().join("managed/extended");
    fs::create_dir_all(extended_worktree.join("source")).unwrap();
    fs::write(
        extended_worktree.join("package.zst"),
        b"sealed extended package",
    )
    .unwrap();
    let extended_source = ManagedCiPackage::open(
        0,
        ManagedCiPackagePaths {
            managed_root: root.path().join("managed"),
            sources: extended_worktree.join("source"),
            package: extended_worktree.join("package.zst"),
            worktree: extended_worktree,
        },
        Sha256::digest(b"sealed extended package").into(),
        PreparedCiTarget {
            target_line: "main".into(),
            base_snapshot_id: extended.base_snapshot_id.clone(),
            revision_snapshot_id: extended.revision_snapshot_id.clone(),
        },
    )
    .unwrap();
    let extended_job = jobs
        .enqueue_prepared_ci(extended_source.clone(), 1, 1400)
        .unwrap();
    jobs.with_exclusive_queue(|queue| {
        let running = queue.replace(
            extended_job.key.worker_job_index,
            ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_RUNNING,
                attempt_count: 1,
                locked_at_s: 1500,
                updated_at_s: 1500,
                ..extended_job.record
            },
        )?;
        jobs.commit_prepared_ci_result(
            running,
            WorkerJobCompactCiEvidence {
                ci_completed_at_s: 1900,
                ..ci
            },
        )?;
        queue.replace(
            extended_job.key.worker_job_index,
            ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_SUCCEEDED,
                outcome_kind: WORKER_JOB_OUTCOME_COMPLETED,
                locked_at_s: 0,
                updated_at_s: 1901,
                ..running.record
            },
        )?;
        Ok(())
    })
    .unwrap();
    let extended_identity = extended_source.expected_identity(extended_job.key).unwrap();
    for (operation, payload, at) in [
        (
            "review",
            json!({"reviewer":"Reviewer","action":"approve","reviewer_kind":"user"}),
            2001,
        ),
        (
            "review",
            json!({"reviewer":"Reviewer","action":"code_review_summary","reviewer_kind":"user"}),
            1003,
        ),
        (
            "review",
            json!({"reviewer":"Reviewer","action":"task_approve","reviewer_kind":"user"}),
            1004,
        ),
        (
            "attestation",
            json!({"author_mode":"ai_with_human_review","verification_state":"pass","require_tests_pass":true,"require_human_review":true,"require_lint_pass":true}),
            2002,
        ),
    ] {
        let event=PackageEvidenceRequest::parse(&json!({"contract":"workflow-package-evidence/v1","operation":operation,"idempotency_key":format!("{operation}-{at}"),"actor_key_index":actor,"expected_identity":extended_identity,"payload":payload})).unwrap();
        let accepted = workflow
            .validate_package_evidence(&event, &extended.author_mode)
            .unwrap();
        jobs.prepared_ci_runtime()
            .accept_evidence(extended_job.key, event, accepted, at)
            .unwrap();
    }
    let prepared = workflow
        .prepare_workflow_package_history(&extended, &extended_identity)
        .unwrap();
    let first_remote = result["entries"][0]["remote_task_id"].as_str().unwrap();
    let mut independent = extended.clone();
    independent.history_entries = vec![extended.history_entries.last().unwrap().clone()];
    independent.history_entries[0].local_task_id =
        manifest.history_entries[0].local_task_id.clone();
    let mut mirror = extended.clone();
    let entry = &mut mirror.history_entries[0];
    let original_label = entry.local_task_id.clone();
    entry.local_task_id = "MIRROR-1001".into();
    for change in &mut entry.changes {
        change.local_change_ref = change
            .local_change_ref
            .replace(&original_label, &entry.local_task_id);
    }
    for snapshot in &mut entry.snapshots {
        snapshot.local_change_ref = snapshot
            .local_change_ref
            .as_ref()
            .map(|reference| reference.replace(&original_label, &entry.local_task_id));
    }
    let mut explicit_mirror = mirror.clone();
    explicit_mirror.history_entries[0].expected_remote_task_id = Some(first_remote.into());
    for (candidate, same_remote) in [
        (&independent, false),
        (&mirror, true),
        (&explicit_mirror, true),
    ] {
        let candidate = workflow
            .prepare_workflow_package_history(candidate, &extended_identity)
            .unwrap();
        let error = jobs
            .finish_prepared_ci(
                &db,
                extended_job.key.worker_job_index,
                extended_source.sha256(),
                |tx, context| -> StoreResult<PreparedCiPublication<JsonValue>> {
                    let publication = workflow
                        .publish_workflow_package_in_write(
                            tx,
                            candidate,
                            &extended_identity,
                            actor,
                            context.evidence,
                            &context.actions,
                            "direct",
                        )
                        .map_err(BinaryDbError::invalid_domain_data)?;
                    assert_eq!(
                        publication.result["entries"][0]["remote_task_id"] == first_remote,
                        same_remote
                    );
                    assert_eq!(tx.record_count(WorkflowBinaryV0Codec::task_file())?, 6);
                    assert_eq!(tx.record_count(WorkflowBinaryV0Codec::patchset_file())?, 8);
                    assert_eq!(tx.record_count(WorkflowBinaryV0Codec::land_file())?, 2);
                    // Roll back each successful scoped translation so the real overlap
                    // below uses the same unchanged Job, original receipts and head.
                    Err(BinaryDbError::invalid_domain_data(
                        "rollback scoped receipt probe",
                    ))
                },
            )
            .err()
            .expect("Probe deliberately rolls back after successful publication");
        assert!(
            error.to_string().contains("rollback scoped receipt probe"),
            "{error}"
        );
        let read = BinaryDbReadTxn::new(&db);
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::task_file())
                .unwrap(),
            5
        );
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::land_file())
                .unwrap(),
            1
        );
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::patchset_file())
                .unwrap(),
            6
        );
    }
    let mut wrong_explicit = independent.clone();
    wrong_explicit.history_entries[0].expected_remote_task_id = Some(first_remote.into());
    let mut wrong_receipt = extended.clone();
    wrong_receipt.history_entries[0].landed_at_s += 1;
    let mut mixed_owners = independent.clone();
    for entry in manifest.history_entries[..2].iter().rev() {
        let mut snapshot = entry.snapshots[0].clone();
        snapshot.local_change_ref = None;
        mixed_owners.history_entries[0]
            .snapshots
            .insert(0, snapshot);
    }
    for (candidate, expected_error) in [
        (&wrong_explicit, "immutable published Local receipt"),
        (&wrong_receipt, "immutable published Local receipt"),
        (&mixed_owners, "numeric owners conflict"),
    ] {
        let candidate = workflow
            .prepare_workflow_package_history(candidate, &extended_identity)
            .unwrap();
        let error = jobs
            .finish_prepared_ci(
                &db,
                extended_job.key.worker_job_index,
                extended_source.sha256(),
                |tx, context| {
                    workflow
                        .publish_workflow_package_in_write(
                            tx,
                            candidate,
                            &extended_identity,
                            actor,
                            context.evidence,
                            &context.actions,
                            "direct",
                        )
                        .map_err(BinaryDbError::invalid_domain_data)
                },
            )
            .err()
            .expect("Conflicting mapping or original history must reject atomically");
        assert!(error.to_string().contains(expected_error), "{error}");
        let read = BinaryDbReadTxn::new(&db);
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::task_file())
                .unwrap(),
            5
        );
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::land_file())
                .unwrap(),
            1
        );
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::patchset_file())
                .unwrap(),
            6
        );
    }
    for explicit in [true, false] {
        let mut aliased = extended.clone();
        let entry = &mut aliased.history_entries[0];
        let mut duplicate = entry.changes[0].clone();
        duplicate.local_change_id = "C-09".into();
        duplicate.local_change_ref = format!("{}/C-09", entry.local_task_id);
        if explicit {
            duplicate.expected_remote_change_ref = Some(
                result["entries"][0]["changes"][0]["remote_change_ref"]
                    .as_str()
                    .unwrap()
                    .into(),
            );
        } else {
            entry.snapshots[3].local_change_ref = Some(duplicate.local_change_ref.clone());
        }
        entry.changes.push(duplicate);
        let invalid = workflow
            .prepare_workflow_package_history(&aliased, &extended_identity)
            .unwrap();
        let error = jobs
            .finish_prepared_ci(
                &db,
                extended_job.key.worker_job_index,
                extended_source.sha256(),
                |tx, context| {
                    workflow
                        .publish_workflow_package_in_write(
                            tx,
                            invalid,
                            &extended_identity,
                            actor,
                            context.evidence,
                            &context.actions,
                            "direct",
                        )
                        .map_err(BinaryDbError::invalid_domain_data)
                },
            )
            .err()
            .expect("Aliased Remote Change owners must reject the whole batch");
        assert!(
            error.to_string().contains("alias one Remote Change"),
            "{error}"
        );
        let read = BinaryDbReadTxn::new(&db);
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::task_file())
                .unwrap(),
            5
        );
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::change_file())
                .unwrap(),
            2
        );
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::land_file())
                .unwrap(),
            1
        );
        assert_eq!(
            read.record_count(WorkflowBinaryV0Codec::patchset_file())
                .unwrap(),
            6
        );
        drop(read);
        assert_eq!(
            jobs.get(extended_job.key.worker_job_index)
                .unwrap()
                .record
                .patchset_index_plus1,
            0
        );
    }
    for variant in [
        "omitted-snapshot",
        "substituted-time",
        "extra-change",
        "omitted-changes",
        "invalid-source",
    ] {
        let mut source = manifest.clone();
        match variant {
            "omitted-snapshot" => {
                source.history_entries[0].snapshots.pop();
            }
            "substituted-time" => source.history_entries[0].snapshots[0].created_at_s += 1,
            "extra-change" => {
                let mut change = source.history_entries[0].changes[0].clone();
                change.local_change_id = "C-09".into();
                change.local_change_ref =
                    format!("{}/C-09", source.history_entries[0].local_task_id);
                change.expected_remote_change_ref = Some(
                    result["entries"][0]["changes"][0]["remote_change_ref"]
                        .as_str()
                        .unwrap()
                        .into(),
                );
                source.history_entries[0].changes.push(change);
            }
            "omitted-changes" => {
                source.history_entries[0].changes.clear();
                for snapshot in &mut source.history_entries[0].snapshots {
                    snapshot.local_change_ref = None;
                }
            }
            "invalid-source" => source.author_mode = "unavailable-local-source".into(),
            _ => unreachable!(),
        }
        let lookup = workflow
            .lookup_workflow_package_finish(bound, &request, Some(&source))
            .unwrap()
            .unwrap();
        assert_eq!(lookup["outcome"], "incomplete", "{variant}: {lookup}");
        assert_eq!(lookup["expected_identity"], result["expected_identity"]);
        assert_eq!(lookup["patchset_id"], result["patchset_id"]);
        assert_eq!(lookup["submission_id"], result["submission_id"]);
        assert_eq!(lookup["entries"], json!([]));
    }
    let extended_committed = jobs
        .finish_prepared_ci(
            &db,
            extended_job.key.worker_job_index,
            extended_source.sha256(),
            |tx, context| {
                workflow
                    .publish_workflow_package_in_write(
                        tx,
                        prepared,
                        &extended_identity,
                        actor,
                        context.evidence,
                        &context.actions,
                        "direct",
                    )
                    .map_err(BinaryDbError::invalid_domain_data)
            },
        )
        .unwrap();
    for (name, record) in &projected_before {
        assert_eq!(
            &lines
                .line_by_name(&BinaryDbReadTxn::new(&db), name)
                .unwrap()
                .unwrap()
                .1,
            record
        );
    }
    for (index, record) in closed_changes.iter().enumerate() {
        assert_eq!(
            &BinaryDbReadTxn::new(&db)
                .read_record(WorkflowBinaryV0Codec::change_file(), index as u32)
                .unwrap(),
            record
        );
    }
    let mapped = extended_committed.result["entries"].as_array().unwrap();
    assert_eq!(mapped.len(), 6);
    assert_eq!(
        &mapped[..5],
        result["entries"].as_array().unwrap().as_slice()
    );
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .unwrap(),
        6
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .unwrap(),
        8
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        2
    );
    assert_eq!(
        read.read_record(WorkflowBinaryV0Codec::task_file(), 4)
            .unwrap(),
        completed_source
    );
    assert_eq!(
        read.read_record(WorkflowBinaryV0Codec::patchset_file(), 4)
            .unwrap(),
        original_receipt
    );
    drop(read);
    validate_frozen_server_workflow_v0(&db).unwrap();
    assert_eq!(
        workflow
            .lookup_workflow_package_finish(bound, &request, Some(&manifest))
            .unwrap()
            .unwrap()["outcome"],
        "incomplete"
    );
    // A conflicting second actual receipt must hold acceptance lookup, even
    // with the same Job, hash and Patchset. This only corrupts this fixture.
    let read = BinaryDbReadTxn::new(&db);
    let patchset = WorkflowBinaryV0Codec::decode_frozen_patchset(
        &read
            .read_record(
                WorkflowBinaryV0Codec::patchset_file(),
                bound.record.patchset_index_plus1 - 1,
            )
            .unwrap(),
    )
    .unwrap();
    let mut inventory = WorkflowBinaryV0Codec::decode_inventory_index(
        &read
            .read_record(
                WorkflowBinaryV0Codec::chain_index_file("task_land_index.bin"),
                patchset.task_index,
            )
            .unwrap(),
    )
    .unwrap();
    let mut duplicate = WorkflowBinaryV0Codec::decode_land(
        &read
            .read_record(
                WorkflowBinaryV0Codec::land_file(),
                inventory.latest_index_plus1 - 1,
            )
            .unwrap(),
    )
    .unwrap();
    drop(read);
    duplicate.land_ordinal += 1;
    duplicate.previous_task_land_index_plus1 = inventory.latest_index_plus1;
    let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    let index = write
        .append_record(
            WorkflowBinaryV0Codec::land_file(),
            &WorkflowBinaryV0Codec::encode_land(duplicate).unwrap(),
        )
        .unwrap();
    inventory.latest_index_plus1 = index + 1;
    inventory.count += 1;
    write
        .overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_land_index.bin"),
            patchset.task_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(inventory).unwrap(),
        )
        .unwrap();
    write.commit().unwrap();
    assert!(workflow
        .lookup_workflow_package_finish(bound, &request, None)
        .unwrap_err()
        .contains("multiple successful receipts"));
    assert!(workflow
        .lookup_accepted_workflow_package(bound, &identity.package_hash, actor)
        .unwrap_err()
        .contains("multiple successful receipts"));
}
