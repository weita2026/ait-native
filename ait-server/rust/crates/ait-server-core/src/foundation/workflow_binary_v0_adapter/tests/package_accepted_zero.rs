use super::*;
use crate::foundation::operational_binary_v0::*;
use crate::foundation::prepared_ci_runtime::{
    ManagedCiPackage, ManagedCiPackagePaths, PreparedCiTarget,
};
use crate::foundation::remote_binary_db::{BinaryDbError, StoreResult};
use crate::foundation::server_operational_job_domain::WorkerJobCompactCiEvidence;
use crate::foundation::server_operational_worker_jobs::{
    ServerOperationalWorkerJobStore, WorkerJobDomainAuthority, WorkerJobEntry,
};
use crate::foundation::workflow_binary_v0::*;
use crate::foundation::workflow_package::WorkflowPackageManifest;
use crate::foundation::workflow_package_contract::{
    PackageEvidenceRequest, PackageExpectedIdentity, PackageFinishRequest,
};
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

struct Fixture {
    _root: tempfile::TempDir,
    db: TestDb,
    workflow: BinaryDbServerWorkflowV0Store<TestDb>,
    job: WorkerJobEntry,
    identity: PackageExpectedIdentity,
    request: PackageFinishRequest,
    original: JsonValue,
}

fn fixture(tasks: usize) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let original = initialized_db("accepted-zero");
    let authority = root.path().join("0");
    fs::rename(original.authority_root().as_path(), &authority).unwrap();
    let db = TestDb::test_fixture(
        RepoId::new("REPO-V0"),
        RepoName::new("repo"),
        StorePath::new(authority.clone()),
        StoreGeneration::new(1),
    );
    seed_content(&db);
    let ids = ["SNP-0000000000A1", "SNP-0000000000A2", "SNP-0000000000A3"];
    let entries = (0..tasks)
        .map(|i| {
            json!({
                "local_task_id":format!("LT-{:04}", i+1), "expected_remote_task_id":null,
                "task":{"title":format!("Source {i}"),"intent":"Publish original completed source"},
                "pre_land_target_snapshot_id":ids[i], "landed_snapshot_id":ids[i+1],
                "landed_at_s":100+i, "changes":[],
                "snapshots":[{"snapshot_id":ids[i+1],"created_at_s":1,"local_change_ref":null}]
            })
        })
        .collect::<Vec<_>>();
    let manifest: WorkflowPackageManifest = serde_json::from_value(json!({
        "contract":"workflow-package/v1","repository_index":0,"target_line":"main",
        "base_snapshot_id":ids[0],"revision_snapshot_id":ids[tasks],
        "author_mode":"ai_with_human_review","history_entries":entries,
        "content":{"object_packs":[],"tree_packs":[],"blob_locators":[],
            "tree_locators":[],"snapshots":[],"line_update":null,"capture_binding":null}
    }))
    .unwrap();
    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "")
        .unwrap()
        .with_repository_index(0);
    let actor = workflow
        .resolve_user_actor(&UserActorIdentity::new("Reviewer", "reviewer@example.test").unwrap())
        .unwrap();
    let jobs = ServerOperationalWorkerJobStore::new(0, &authority, Arc::new(Domain)).unwrap();
    jobs.initialize().unwrap();
    let worktree = root.path().join("managed/job");
    fs::create_dir_all(worktree.join("source")).unwrap();
    fs::write(worktree.join("package.zst"), b"sealed zero fixture").unwrap();
    let source = ManagedCiPackage::open(
        0,
        ManagedCiPackagePaths {
            managed_root: root.path().join("managed"),
            sources: worktree.join("source"),
            package: worktree.join("package.zst"),
            worktree,
        },
        Sha256::digest(b"sealed zero fixture").into(),
        PreparedCiTarget {
            target_line: "main".into(),
            base_snapshot_id: manifest.base_snapshot_id.clone(),
            revision_snapshot_id: manifest.revision_snapshot_id.clone(),
        },
    )
    .unwrap();
    let job = jobs.enqueue_prepared_ci(source.clone(), 1, 100).unwrap();
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
        jobs.commit_prepared_ci_result(
            running,
            WorkerJobCompactCiEvidence {
                ci_completed_at_s: 900,
                ci_run_seq: 1,
                selected_suite_count: 1,
                suite_result_count: 1,
                blocking_failure_count: 0,
                overall_status: CI_STATUS_PASS,
                tests_status: CI_STATUS_PASS,
                lint_status: CI_STATUS_PASS,
            },
        )?;
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
            json!({"reviewer":"Reviewer","reviewer_kind":"user","action":"approve"}),
            1001,
        ),
        (
            "review",
            json!({"reviewer":"Reviewer","reviewer_kind":"user","action":"code_review_summary"}),
            1002,
        ),
        (
            "review",
            json!({"reviewer":"Reviewer","reviewer_kind":"user","action":"task_approve"}),
            1003,
        ),
        (
            "attestation",
            json!({"author_mode":"ai_with_human_review","verification_state":"pass",
            "require_tests_pass":true,"require_lint_pass":true,"require_human_review":true}),
            1004,
        ),
    ] {
        let request = PackageEvidenceRequest::parse(&json!({
            "contract":"workflow-package-evidence/v1","operation":operation,
            "idempotency_key":format!("{operation}-{at}"),"actor_key_index":actor,
            "expected_identity":identity,"payload":payload,
        }))
        .unwrap();
        let accepted = workflow
            .validate_package_evidence(&request, &manifest.author_mode)
            .unwrap();
        jobs.prepared_ci_runtime()
            .accept_evidence(job.key, request, accepted, at)
            .unwrap();
    }
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
    let request = PackageFinishRequest::parse(&json!({
        "contract":"task-land-package-guarded/v1","idempotency_key":identity.idempotency_key(),
        "actor_key_index":actor,"target_line":"main","mode":"direct","expected_identity":identity
    }))
    .unwrap();
    Fixture {
        _root: root,
        db,
        workflow,
        job: jobs.get(job.key.worker_job_index).unwrap(),
        identity,
        request,
        original: committed.result,
    }
}

fn bytes(f: &Fixture) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(f.db.authority_root().as_path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("bin" | "idx")
            )
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_str().unwrap().into(),
                fs::read(path).unwrap(),
            )
        })
        .collect()
}

fn replay(f: &Fixture) -> JsonValue {
    f.workflow
        .lookup_workflow_package_finish(f.job, &f.request, None)
        .unwrap()
        .unwrap()
}

fn mutate_summary(f: &Fixture, change: impl FnOnce(&mut JsonValue)) {
    let index = f.job.record.patchset_index_plus1 - 1;
    let read = BinaryDbReadTxn::new(&f.db);
    let mut record = WorkflowBinaryV0Codec::decode_frozen_patchset(
        &read
            .read_record(WorkflowBinaryV0Codec::patchset_file(), index)
            .unwrap(),
    )
    .unwrap();
    let raw = read
        .read_payload(
            WorkflowBinaryV0Codec::patchset_summary_file(),
            record.summary_offset,
            u32::from(record.summary_len),
        )
        .unwrap();
    let raw = std::str::from_utf8(&raw).unwrap();
    let (prefix, json) = raw.split_once(' ').unwrap();
    let mut value = serde_json::from_str(json).unwrap();
    change(&mut value);
    let updated = format!("{prefix} {}", serde_json::to_string(&value).unwrap());
    drop(read);
    let mut tx = BinaryDbWriteTxn::begin(&f.db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    record.summary_offset = tx
        .append_payload(
            WorkflowBinaryV0Codec::patchset_summary_file(),
            updated.as_bytes(),
        )
        .unwrap()
        .payload_offset;
    record.summary_len = updated.len().try_into().unwrap();
    tx.overwrite_record(
        WorkflowBinaryV0Codec::patchset_file(),
        index,
        &WorkflowBinaryV0Codec::encode_frozen_patchset(record).unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
}

#[test]
fn accepted_zero_change_replay_is_complete_ordered_and_read_only_after_reopen_or_head_move() {
    for tasks in [1, 2] {
        let mut f = fixture(tasks);
        let lines = ServerBinaryDbLineStore::<_, 1>::new(f.db.clone());
        if tasks == 1 {
            lines.set_line_head("main", 3, 5000).unwrap();
        }
        f.workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(f.db.clone(), "")
            .unwrap()
            .with_repository_index(0);
        let before = bytes(&f);
        for _ in 0..2 {
            assert_eq!(replay(&f), f.original);
            assert_eq!(
                f.workflow
                    .lookup_accepted_workflow_package(
                        f.job,
                        &f.identity.package_hash,
                        f.request.actor_key_index
                    )
                    .unwrap()
                    .unwrap(),
                f.original
            );
            assert_eq!(bytes(&f), before);
        }
    }
}

#[test]
fn accepted_zero_change_requires_complete_durable_task_receipts_and_labels() {
    for variant in [
        "empty",
        "missing",
        "duplicate",
        "omitted-prefix",
        "label",
        "duplicate-label",
        "swapped-labels",
        "receipt",
        "owner",
    ] {
        let f = fixture(2);
        mutate_summary(&f, |value| match variant {
            "empty" => value["entries"] = json!([]),
            "missing" => {
                value.as_object_mut().unwrap().remove("entries");
            }
            "duplicate" => {
                let row = value["entries"][0].clone();
                value["entries"].as_array_mut().unwrap().push(row);
            }
            "omitted-prefix" => {
                value["entries"].as_array_mut().unwrap().remove(0);
            }
            "label" => {
                value["entries"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("local_task_id");
            }
            "duplicate-label" => {
                value["entries"][1]["local_task_id"] = value["entries"][0]["local_task_id"].clone()
            }
            "swapped-labels" => {
                let first = value["entries"][0]["local_task_id"].clone();
                value["entries"][0]["local_task_id"] = value["entries"][1]["local_task_id"].clone();
                value["entries"][1]["local_task_id"] = first;
            }
            "receipt" => value["entries"][0]["receipt_patchset_index"] = json!(u32::MAX),
            "owner" => value["entries"][0]["task_id"] = json!("RT-0002"),
            _ => unreachable!(),
        });
        let before = bytes(&f);
        let result = replay(&f);
        assert_eq!(result["outcome"], "incomplete", "{variant}: {result}");
        assert_eq!(result["entries"], json!([]), "{variant}");
        assert_eq!(bytes(&f), before, "{variant}");
    }
}

#[test]
fn accepted_zero_change_never_treats_missing_required_family_or_configured_slots_as_empty() {
    for variant in ["missing", "truncated", "layout", "configured"] {
        let f = fixture(1);
        let path = f.db.authority_root().as_path().join("change.bin");
        match variant {
            "missing" => fs::remove_file(path).unwrap(),
            "truncated" => fs::write(path, [2, 0]).unwrap(),
            "layout" => fs::write(path, 1_u32.to_le_bytes()).unwrap(),
            "configured" => {
                let path =
                    f.db.authority_root()
                        .as_path()
                        .join("task_change_index.bin");
                let mut raw = fs::read(&path).unwrap();
                raw[10] = 1;
                fs::write(path, raw).unwrap();
            }
            _ => unreachable!(),
        }
        let before = bytes(&f);
        assert_eq!(replay(&f)["outcome"], "incomplete", "{variant}");
        assert_eq!(bytes(&f), before, "{variant}");
    }
}

#[test]
fn accepted_zero_change_excludes_foreign_rows_but_includes_retired_source_rows() {
    for retired_source in [false, true] {
        let f = fixture(1);
        let foreign = f.workflow.create_task("repo",&json!({
            "actor_key_index":0,"title":"Later independent Task","intent":"Keep separate ownership"
        })).unwrap();
        f.workflow.create_change("repo",&json!({
            "actor_key_index":0,"task_id":foreign["task_id"],"title":"Other revision","base_line":"main"
        })).unwrap();
        if retired_source {
            let read = BinaryDbReadTxn::new(&f.db);
            let mut row = WorkflowBinaryV0Codec::decode_change(
                &read
                    .read_record(WorkflowBinaryV0Codec::change_file(), 0)
                    .unwrap(),
            )
            .unwrap();
            drop(read);
            row.task_index = 0;
            row.remote_meta |= 1;
            let mut tx =
                BinaryDbWriteTxn::begin(&f.db, BinaryDbCommandScope::ServerWorkflow).unwrap();
            tx.overwrite_record(
                WorkflowBinaryV0Codec::change_file(),
                0,
                &WorkflowBinaryV0Codec::encode_change(row).unwrap(),
            )
            .unwrap();
            tx.commit().unwrap();
        }
        let before = bytes(&f);
        let result = replay(&f);
        assert_eq!(
            result["outcome"],
            if retired_source {
                "incomplete"
            } else {
                "confirmed"
            }
        );
        assert_eq!(bytes(&f), before);
    }
}

#[test]
fn accepted_zero_change_requires_fixed_snapshot_owner_and_acyclic_content_closure() {
    for variant in ["time", "change-owner", "cycle", "content"] {
        let f = fixture(1);
        let mut tx =
            BinaryDbWriteTxn::begin(&f.db, BinaryDbCommandScope::ServerSnapshotCapture).unwrap();
        if matches!(variant, "time" | "change-owner") {
            let mut link = WorkflowBinaryV0Codec::decode_snapshot_link(
                &tx.read_record(WorkflowBinaryV0Codec::snapshot_link_file(), 0)
                    .unwrap(),
            )
            .unwrap();
            if variant == "time" {
                link.created_at_s += 1;
            } else {
                link.change_index_plus1 = 1;
                link.link_meta |= SNAPSHOT_LINK_HAS_CHANGE;
            }
            tx.overwrite_record(
                WorkflowBinaryV0Codec::snapshot_link_file(),
                0,
                &WorkflowBinaryV0Codec::encode_snapshot_link(link).unwrap(),
            )
            .unwrap();
        } else {
            let file = ServerBinarySnapshotCodec::<2>::record_file();
            let mut snapshot = ServerBinarySnapshotCodec::<2>::decode_record(
                &tx.read_record(file.clone(), 1).unwrap(),
            )
            .unwrap();
            if variant == "cycle" {
                snapshot.parent_snapshot_index_plus1 = 2;
            } else {
                snapshot.root_tree_pack_index_plus1 = u32::MAX;
                snapshot.snapshot_meta |= ServerBinarySnapshotRecord::META_HAS_ROOT_LOCATOR;
            }
            tx.overwrite_record(
                file,
                1,
                &ServerBinarySnapshotCodec::<2>::encode_record(&snapshot).unwrap(),
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let before = bytes(&f);
        assert_eq!(replay(&f)["outcome"], "incomplete", "{variant}");
        assert_eq!(bytes(&f), before, "{variant}");
    }
}
