use super::*;
use crate::foundation::remote_binary_db::{
    binary_db_runtime_error_kind, BinaryDbCommandScope, BinaryDbErrorKind, BinaryDbReadTxn,
    BinaryDbWriteTxn, FilesystemServerRemoteBinaryDb, RepoId, RepoName, ServerRemoteBinaryDb,
    StoreGeneration, StorePath,
};
use crate::foundation::server_binary_db_schema_registry::{
    SERVER_BINARY_DB_BIN_SCHEMAS, SERVER_BINARY_DB_INDEX_SCHEMAS, SERVER_BINARY_DB_LAYOUT_ID,
};
use crate::foundation::server_content_binary_db::{
    server_snapshot_hash48_from_id, ServerBinaryDbLineStore, ServerBinaryDbSnapshotStore,
    ServerBinarySnapshotCodec, ServerBinarySnapshotPayload, ServerBinarySnapshotRecord,
    SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use crate::foundation::server_plan_binary_db::BinaryDbServerPlanService;
use crate::foundation::server_queue_binary_db::BinaryDbServerWorkflowReadModelService;
use crate::foundation::server_workflow_store::{
    ServerWorkflowAttestationStore, ServerWorkflowChangeStore, ServerWorkflowLandStore,
    ServerWorkflowPatchsetStore, ServerWorkflowPolicyStore, ServerWorkflowReviewStore,
    ServerWorkflowTaskStore,
};
use crate::foundation::workflow_binary_v0::{
    V0LandRecord, V0PolicyCheckRecord, WorkflowBinaryV0Codec, CHANGE_LIFECYCLE_ACTIVE,
    CHANGE_RECORD_SIZE, LAND_HAS_LANDED_SNAPSHOT, LAND_STATUS_FAILED, LAND_STATUS_SUCCEEDED,
    LAND_TOMBSTONE, TASK_META_COMPLETED,
};
use serde_json::{json, Value as JsonValue};
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

type TestDb = FilesystemServerRemoteBinaryDb;

fn temporary_root(label: &str) -> StorePath {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    StorePath::new(std::env::temp_dir().join(format!(
        "ait-server-workflow-v0-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )))
}

fn initialized_db(label: &str) -> TestDb {
    let authority_root = temporary_root(label);
    fs::create_dir_all(authority_root.as_path()).expect("create v0 fixture authority");
    for path in SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
    {
        fs::write(
            authority_root.as_path().join(path),
            SERVER_BINARY_DB_LAYOUT_ID.to_le_bytes(),
        )
        .unwrap_or_else(|error| panic!("initialize {path}: {error}"));
    }
    FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("REPO-V0"),
        RepoName::new("repo"),
        authority_root,
        StoreGeneration::new(1),
    )
}

#[test]
fn remote_task_identity_preserves_origin_for_empty_and_nonempty_namespaces() {
    let empty_namespace =
        BinaryDbServerWorkflowV0Store::new_remote(initialized_db("empty-remote-namespace"), "")
            .expect("empty namespace keeps Remote origin");
    assert_eq!(empty_namespace.origin_namespace_prefix(), "R");
    let task = empty_namespace
        .create_task(
            "repo",
            &json!({
                "title": "Empty namespace Remote Task",
                "intent": "Preserve RT identity"
            }),
        )
        .expect("create empty-namespace Remote Task");
    assert_eq!(task["task_id"], "RT-0001");
    assert_eq!(
        empty_namespace
            .get_task(Some("repo"), "RT-0001")
            .expect("read canonical Remote Task")["task_id"],
        "RT-0001"
    );
    assert!(empty_namespace.get_task(Some("repo"), "T-0001").is_err());

    let namespaced =
        BinaryDbServerWorkflowV0Store::new_remote(initialized_db("named-remote-namespace"), "SE")
            .expect("nonempty namespace keeps Remote origin");
    assert_eq!(namespaced.origin_namespace_prefix(), "RSE");
    let task = namespaced
        .create_task(
            "repo",
            &json!({
                "title": "Namespaced Remote Task",
                "intent": "Preserve RSET identity"
            }),
        )
        .expect("create namespaced Remote Task");
    assert_eq!(task["task_id"], "RSET-0001");
}

fn seed_content(db: &TestDb) {
    const BASE: &str = "SNP-0000000000A1";
    const REVISION_ONE: &str = "SNP-0000000000A2";
    const REVISION_TWO: &str = "SNP-0000000000A3";
    let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    let snapshots =
        ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    let line_index = lines.create_line("main", 0, 1).expect("create main Line");
    let payload = ServerBinarySnapshotPayload {
        line_name: "main".to_string(),
        message: Some("Binary DB v0 adapter fixture".to_string()),
    };
    let record = |snapshot_id: &str, parent_snapshot_index_plus1: u32| ServerBinarySnapshotRecord {
        snapshot_meta: 0,
        history_flags: 0,
        payload_len: 0,
        payload_offset: 0,
        snapshot_hash48: server_snapshot_hash48_from_id(snapshot_id)
            .expect("valid fixture Snapshot id"),
        parent_snapshot_index_plus1,
        root_tree_pack_index_plus1: 0,
        root_entry_ordinal: 0,
        line_index_plus1: line_index + 1,
        manifest_hash: [0; 32],
        file_count: 0,
        total_bytes: 0,
        created_at_s: 1,
    };
    let base_index = snapshots
        .append_snapshot(BASE, record(BASE, 0), &payload)
        .expect("append base Snapshot");
    let revision_one_index = snapshots
        .append_snapshot(REVISION_ONE, record(REVISION_ONE, base_index + 1), &payload)
        .expect("append first revision Snapshot");
    snapshots
        .append_snapshot(
            REVISION_TWO,
            record(REVISION_TWO, revision_one_index + 1),
            &payload,
        )
        .expect("append second revision Snapshot");
    lines
        .set_line_head("main", base_index + 1, 2)
        .expect("set main Line head");
}

#[test]
fn activation_repair_normalizes_active_patchset_ci_job_locator() {
    let db = initialized_db("activation-repair");
    seed_content(&db);
    let active = BinaryDbServerWorkflowV0Store::new(db.clone());
    active
        .create_task(
            "repo",
            &json!({
                "title": "Active Patchset",
                "intent": "Exercise active Patchset locator normalization"
            }),
        )
        .expect("create active Task");
    active
        .create_change(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Active Patchset Change",
                "base_line": "main"
            }),
        )
        .expect("create active Change");
    active
        .publish_patchset(
            "T-0001/C-01",
            &json!({
                "base_snapshot_id": "SNP-0000000000A1",
                "revision_snapshot_id": "SNP-0000000000A2",
                "summary": "active Patchset locator fixture",
                "author_mode": "ai_with_human_review"
            }),
        )
        .expect("publish active Patchset");

    let frozen = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());
    let locators = BTreeMap::from([(0, 5)]);
    assert_eq!(
        frozen
            .repair_frozen_patchsets_for_activation(&locators)
            .expect("normalize active Patchset locator"),
        1
    );
    validate_frozen_server_workflow_v0(&db).expect("validate repaired frozen authority");
    let read = BinaryDbReadTxn::new(&db);
    let repaired = WorkflowBinaryV0Codec::decode_frozen_patchset(
        &read
            .read_record(WorkflowBinaryV0Codec::patchset_file(), 0)
            .expect("read repaired Patchset"),
    )
    .expect("decode repaired frozen Patchset");
    assert_eq!(repaired.ci_worker_job_index_plus1, 5);
    drop(read);

    let ci = frozen
        .run_patchset_ci("T-0001/C-01/P-01", &json!({"trigger": "manual_rerun"}))
        .expect("mutate repaired Patchset through frozen workflow serving");
    assert_eq!(ci["ci_run_seq"], 1);
    let read = BinaryDbReadTxn::new(&db);
    let updated = WorkflowBinaryV0Codec::decode_frozen_patchset(
        &read
            .read_record(WorkflowBinaryV0Codec::patchset_file(), 0)
            .expect("read updated Patchset"),
    )
    .expect("decode updated frozen Patchset");
    assert_eq!(updated.ci_worker_job_index_plus1, 5);
    drop(read);
    assert_eq!(
        frozen
            .repair_frozen_patchsets_for_activation(&locators)
            .expect("replay bounded activation repair"),
        0
    );
}

fn seed_history_content(db: &TestDb, local_land_count: usize) -> Vec<String> {
    let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    let snapshots =
        ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    let line_index = lines
        .create_line("main", 0, 1)
        .expect("create history main Line");
    let payload = ServerBinarySnapshotPayload {
        line_name: "main".to_string(),
        message: Some("History promotion fixture".to_string()),
    };
    let mut snapshot_ids = Vec::with_capacity(local_land_count + 1);
    let mut parent_index_plus1 = 0;
    for ordinal in 0..=local_land_count {
        let snapshot_id = format!("SNP-{:012X}", ordinal + 1);
        let record = ServerBinarySnapshotRecord {
            snapshot_meta: 0,
            history_flags: 0,
            payload_len: 0,
            payload_offset: 0,
            snapshot_hash48: server_snapshot_hash48_from_id(&snapshot_id)
                .expect("valid history Snapshot id"),
            parent_snapshot_index_plus1: parent_index_plus1,
            root_tree_pack_index_plus1: 0,
            root_entry_ordinal: 0,
            line_index_plus1: line_index + 1,
            manifest_hash: [0; 32],
            file_count: 0,
            total_bytes: 0,
            created_at_s: u64::try_from(ordinal + 1).unwrap(),
        };
        let snapshot_index = snapshots
            .append_snapshot(&snapshot_id, record, &payload)
            .expect("append history Snapshot");
        parent_index_plus1 = snapshot_index + 1;
        snapshot_ids.push(snapshot_id);
    }
    lines
        .set_line_head("main", 1, 2)
        .expect("set history main base");
    snapshot_ids
}

fn history_promotion_request(snapshot_ids: &[String]) -> JsonValue {
    let entries = snapshot_ids
        .windows(2)
        .enumerate()
        .map(|(ordinal, boundary)| {
            let local_task_id = format!("LCT-{:04}", ordinal + 1);
            let local_change_id = "C-01";
            json!({
                "local_task_id": local_task_id,
                "local_change_id": local_change_id,
                "local_change_ref": format!("{local_task_id}/{local_change_id}"),
                "task": {
                    "title": format!("Local Task {}", ordinal + 1),
                    "intent": format!("Preserve local history {}", ordinal + 1),
                    "plan_id": JsonValue::Null,
                    "origin_plan_revision_id": JsonValue::Null,
                    "plan_item_ref": JsonValue::Null
                },
                "change": {
                    "title": format!("Local Change {}", ordinal + 1),
                    "base_line": "main",
                    "fork_snapshot_id": boundary[0]
                },
                "pre_land_target_snapshot_id": boundary[0],
                "landed_snapshot_id": boundary[1],
                "landed_at_s": 100 + ordinal,
                "snapshots": [{
                    "snapshot_id": boundary[1],
                    "created_at_s": ordinal + 2
                }]
            })
        })
        .collect::<Vec<_>>();
    json!({
        "contract": "history-promotion-prepare/v1",
        "idempotency_key": "history-promotion:ten-local-lands",
        "target_line": "main",
        "base_snapshot_id": snapshot_ids.first().unwrap(),
        "revision_snapshot_id": snapshot_ids.last().unwrap(),
        "author_mode": "ai_with_human_review",
        "summary": "Promote ten local lands",
        "entries": entries
    })
}

fn staged_history_promotion_request(
    snapshot_ids: &[String],
    stage_ordinal: usize,
    previous_stage_patchset_id: Option<&str>,
) -> JsonValue {
    let all_entries = history_promotion_request(snapshot_ids)["entries"]
        .as_array()
        .expect("history promotion entries")
        .clone();
    let stage_start = stage_ordinal * 64;
    let stage_end = (stage_start + 64).min(all_entries.len());
    let final_stage = stage_end == all_entries.len();
    json!({
        "contract": "history-promotion-prepare/v2",
        "promotion_id": format!(
            "history-promotion:{}:{}",
            snapshot_ids.first().unwrap(),
            snapshot_ids.last().unwrap()
        ),
        "idempotency_key": format!("history-promotion:65-local-lands:stage-{stage_ordinal}"),
        "target_line": "main",
        "base_snapshot_id": snapshot_ids.first().unwrap(),
        "revision_snapshot_id": snapshot_ids.last().unwrap(),
        "stage_ordinal": stage_ordinal,
        "stage_base_snapshot_id": snapshot_ids[stage_start],
        "stage_revision_snapshot_id": snapshot_ids[stage_end],
        "previous_stage_patchset_id": previous_stage_patchset_id,
        "total_entry_count": all_entries.len(),
        "final_stage": final_stage,
        "author_mode": "ai_with_human_review",
        "summary": "Promote 65 local lands without a public count limit",
        "entries": all_entries[stage_start..stage_end].to_vec()
    })
}

fn atomic_plan_payload(title: &str, item_ref: &str) -> JsonValue {
    json!({
        "title": title,
        "status": "draft",
        "summary": format!("{title} summary"),
        "artifact_path": "docs/sprints/atomic-task-start.md",
        "artifact_heading": "Atomic task start",
        "items": [{
            "plan_item_ref": item_ref,
            "text": format!("Implement {title}"),
            "checkbox_state": "open",
            "heading_path": ["Atomic task start"],
            "line_number": 1
        }]
    })
}

fn atomic_task_start_request(
    idempotency_key: &str,
    item_ref: &str,
    plan: JsonValue,
    task_title: &str,
    change_title: &str,
    base_line: &str,
) -> JsonValue {
    json!({
        "contract": "task-start-atomic/v1",
        "idempotency_key": idempotency_key,
        "plan_item_ref": item_ref,
        "plan": plan,
        "task": {
            "title": task_title,
            "intent": format!("Complete {task_title}")
        },
        "change": {
            "title": change_title,
            "base_line": base_line
        }
    })
}

#[test]
fn atomic_task_start_creates_plan_task_and_change_and_replays_exact_binding() {
    let db = initialized_db("atomic-task-start-create-replay");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let item_ref = "ATOMIC-START-01";
    let created = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-create-1",
                item_ref,
                json!({
                    "action": "create",
                    "payload": atomic_plan_payload("Atomic create", item_ref)
                }),
                "Atomic Task",
                "Atomic Change",
                "main",
            ),
        )
        .expect("atomically create Plan, Task, and Change");

    assert_eq!(created["contract"], "task-start-atomic/v1");
    assert_eq!(created["replayed"], false);
    assert_eq!(created["plan_action"], "created");
    assert_eq!(created["plan_id"], "PR-0");
    assert_eq!(created["plan_revision_id"], "plan-revision:0");
    assert_eq!(created["plan_item_ref"], item_ref);
    assert_eq!(created["task_id"], "T-0001");
    assert_eq!(created["task"]["plan_id"], "PR-0");
    assert_eq!(
        created["task"]["origin_plan_revision_id"],
        "plan-revision:0"
    );
    assert_eq!(created["task"]["plan_item_ref"], item_ref);
    assert_eq!(created["change"]["change_ref"], "T-0001/C-01");
    assert_eq!(created["change"]["base_line"], "main");

    let replayed_create = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-create-1",
                item_ref,
                json!({
                    "action": "create",
                    "payload": atomic_plan_payload("Atomic create", item_ref)
                }),
                "Atomic Task",
                "Atomic Change",
                "main",
            ),
        )
        .expect("exact create retry should replay");
    assert_eq!(replayed_create["replayed"], true);
    assert_eq!(replayed_create["task_id"], created["task_id"]);

    let replayed_existing = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-create-1",
                item_ref,
                json!({
                    "action": "existing",
                    "plan_id": "PR-0",
                    "plan_revision_id": "plan-revision:0"
                }),
                "Atomic Task",
                "Atomic Change",
                "main",
            ),
        )
        .expect("exact Plan binding should replay");
    assert_eq!(replayed_existing["replayed"], true);
    assert_eq!(replayed_existing["task_id"], created["task_id"]);
    assert_eq!(
        replayed_existing["change"]["change_ref"],
        created["change"]["change_ref"]
    );
    assert_eq!(
        store
            .list_tasks("repo")
            .expect("list replayed Tasks")
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        store
            .list_changes("repo")
            .expect("list replayed Changes")
            .as_array()
            .map(Vec::len),
        Some(1)
    );

    let conflict = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-create-1",
                item_ref,
                json!({
                    "action": "existing",
                    "plan_id": "PR-0",
                    "plan_revision_id": "plan-revision:0"
                }),
                "Conflicting Task title",
                "Atomic Change",
                "main",
            ),
        )
        .expect_err("conflicting replay must fail closed");
    assert!(conflict.contains("replay conflicts"), "{conflict}");
}

#[test]
fn atomic_task_start_rolls_back_plan_and_workflow_when_change_is_invalid() {
    let db = initialized_db("atomic-task-start-rollback");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let item_ref = "ATOMIC-ROLLBACK-01";
    let error = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-rollback-1",
                item_ref,
                json!({
                    "action": "create",
                    "payload": atomic_plan_payload("Atomic rollback", item_ref)
                }),
                "Rollback Task",
                "Invalid Line Change",
                "missing-line",
            ),
        )
        .expect_err("unknown base Line must abort the composite transaction");
    assert!(error.contains("Unknown canonical base Line"), "{error}");

    let plans = BinaryDbServerPlanService::new(db.clone())
        .list_plans("repo", None)
        .expect("list Plans after rollback");
    assert_eq!(plans.as_array().map(Vec::len), Some(0));
    assert_eq!(
        store
            .list_tasks("repo")
            .expect("list Tasks after rollback")
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        store
            .list_changes("repo")
            .expect("list Changes after rollback")
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert!(
        !ServerRemoteBinaryDb::authority_root(&db)
            .as_path()
            .join("server-task-start.write.journal")
            .exists(),
        "aborted composite transaction must remove its rollback journal"
    );
}

#[test]
fn atomic_task_start_revise_uses_head_cas_and_validates_open_item_before_write() {
    let db = initialized_db("atomic-task-start-revise-cas");
    seed_content(&db);
    let plans = BinaryDbServerPlanService::new(db.clone());
    plans
        .create_plan("repo", &atomic_plan_payload("Existing Plan", "EXISTING-01"))
        .expect("create existing Plan");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let item_ref = "REVISED-01";
    let revised = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-revise-1",
                item_ref,
                json!({
                    "action": "revise",
                    "plan_id": "PR-0",
                    "expected_head_revision_id": "plan-revision:0",
                    "payload": atomic_plan_payload("Revised Plan", item_ref)
                }),
                "Revised Task",
                "Revised Change",
                "main",
            ),
        )
        .expect("revise Plan and create bound Task");
    assert_eq!(revised["plan_action"], "revised");
    assert_eq!(revised["plan_revision_id"], "plan-revision:1");

    let replayed_revise = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-revise-1",
                item_ref,
                json!({
                    "action": "revise",
                    "plan_id": "PR-0",
                    "expected_head_revision_id": "plan-revision:0",
                    "payload": atomic_plan_payload("Revised Plan", item_ref)
                }),
                "Revised Task",
                "Revised Change",
                "main",
            ),
        )
        .expect("exact revise retry should replay current head");
    assert_eq!(replayed_revise["replayed"], true);
    assert_eq!(replayed_revise["plan_action"], "existing");
    assert_eq!(replayed_revise["task_id"], revised["task_id"]);

    let stale_error = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-revise-stale",
                "STALE-01",
                json!({
                    "action": "revise",
                    "plan_id": "PR-0",
                    "expected_head_revision_id": "plan-revision:0",
                    "payload": atomic_plan_payload("Stale Plan", "STALE-01")
                }),
                "Stale Task",
                "Stale Change",
                "main",
            ),
        )
        .expect_err("stale Plan CAS must fail");
    assert!(
        stale_error.contains("must equal current head plan-revision:1"),
        "{stale_error}"
    );

    let missing_item_error = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "atomic-invalid-item",
                "MISSING-01",
                json!({
                    "action": "create",
                    "payload": atomic_plan_payload("Invalid item", "OTHER-01")
                }),
                "Missing Item Task",
                "Missing Item Change",
                "main",
            ),
        )
        .expect_err("missing Plan item must fail before write admission");
    assert!(
        missing_item_error.contains("does not contain item"),
        "{missing_item_error}"
    );

    assert_eq!(
        plans
            .list_plan_revisions("PR-0")
            .expect("list revisions after stale CAS")
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        plans
            .list_plans("repo", None)
            .expect("list Plans after invalid create")
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        store
            .list_tasks("repo")
            .expect("list Tasks after rejected writes")
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        store
            .list_changes("repo")
            .expect("list Changes after rejected writes")
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}

fn seed_task_change_and_patchset(store: &BinaryDbServerWorkflowV0Store<TestDb>) {
    store
        .create_task(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Project selected Patchset",
                "intent": "Keep Change reads on fixed workflow authority"
            }),
        )
        .expect("create Task");
    store
        .create_change(
            "repo",
            &json!({
                "change_id": "C-01",
                "task_id": "T-0001",
                "title": "Narrow selected Patchset projection",
                "base_line": "main"
            }),
        )
        .expect("create Change");
    store
        .publish_patchset(
            "T-0001/C-01",
            &json!({
                "base_snapshot_id": "SNP-0000000000A1",
                "revision_snapshot_id": "SNP-0000000000A2",
                "summary": "selected Patchset fixture",
                "author_mode": "ai_with_human_review"
            }),
        )
        .expect("publish selected Patchset");
}

#[test]
fn patchset_ci_start_reuses_pending_and_terminal_ensure_runs() {
    let db = initialized_db("patchset-ci-idempotent-start");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db);
    seed_task_change_and_patchset(&store);
    let patchset_id = "T-0001/C-01/P-01";
    let ensure = json!({
        "trigger": "workflow_ready_apply",
        "execution_profile": "workflow_ready_foreground"
    });

    let first = store
        .run_patchset_ci(patchset_id, &ensure)
        .expect("start first readiness CI");
    assert_eq!(first["ci_run_seq"], 1);
    assert_eq!(first["ci_completed_at_s"], 0);

    let pending_reuse = store
        .run_patchset_ci(patchset_id, &ensure)
        .expect("reuse pending readiness CI");
    assert_eq!(pending_reuse["ci_run_seq"], 1);
    assert_eq!(pending_reuse["ci_completed_at_s"], 0);

    let completed = store
        .complete_patchset_ci(
            patchset_id,
            &json!({
                "patchset_id": patchset_id,
                "ci_run_seq": 1,
                "selected_suite_count": 1,
                "suite_result_count": 1,
                "blocking_failure_count": 0,
                "overall_status": "pass",
                "tests_status": "pass",
                "lint_status": "none"
            }),
        )
        .expect("complete first readiness CI");
    let completed_at_s = completed["ci_completed_at_s"]
        .as_u64()
        .filter(|value| *value > 0)
        .expect("terminal CI completion time");

    let terminal_reuse = store
        .run_patchset_ci(patchset_id, &ensure)
        .expect("reuse terminal readiness CI");
    assert_eq!(terminal_reuse["ci_run_seq"], 1);
    assert_eq!(terminal_reuse["ci_completed_at_s"], completed_at_s);
    assert_eq!(terminal_reuse["ci"]["tests_status"], "pass");

    let rerun = store
        .run_patchset_ci(patchset_id, &json!({"trigger": "manual_rerun"}))
        .expect("start explicit rerun");
    assert_eq!(rerun["ci_run_seq"], 2);
    assert_eq!(rerun["ci_completed_at_s"], 0);
    assert_eq!(rerun["ci"]["tests_status"], "none");

    let pending_rerun_reuse = store
        .run_patchset_ci(patchset_id, &json!({"trigger": "manual_rerun"}))
        .expect("reuse pending explicit rerun");
    assert_eq!(pending_rerun_reuse["ci_run_seq"], 2);
    assert_eq!(pending_rerun_reuse["ci_completed_at_s"], 0);
}

#[test]
fn change_creation_and_close_commit_lifecycle_in_one_change_record() {
    let db = initialized_db("change-inline-lifecycle");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .create_task(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Inline Change lifecycle",
                "intent": "Keep base Line and archive time in change.bin"
            }),
        )
        .expect("create Task");
    let created = store
        .create_change(
            "repo",
            &json!({
                "change_id": "C-01",
                "task_id": "T-0001",
                "title": "Inline lifecycle fields",
                "base_line": "main"
            }),
        )
        .expect("create Change");
    assert_eq!(created["status"], "draft");
    assert_eq!(created["base_line"], "main");
    assert!(created["archived_at"].is_null());

    let authority_root = ServerRemoteBinaryDb::authority_root(&db);
    assert_eq!(
        fs::metadata(authority_root.as_path().join("change.bin"))
            .expect("stat change.bin")
            .len(),
        std::mem::size_of::<u32>() as u64 + u64::from(CHANGE_RECORD_SIZE)
    );
    assert!(!authority_root
        .as_path()
        .join("change_lifecycle.bin")
        .exists());

    let closed = store
        .close_change("T-0001/C-01", &json!({ "status": "archived" }))
        .expect("close Change");
    assert_eq!(closed["status"], "archived");
    assert_eq!(closed["base_line"], "main");
    assert!(closed["archived_at"].is_string());

    let read = BinaryDbReadTxn::new(&db);
    let raw = read
        .read_record(WorkflowBinaryV0Codec::change_file(), 0)
        .expect("read inline Change");
    let record = WorkflowBinaryV0Codec::decode_change(&raw).expect("decode inline Change");
    assert_eq!(record.base_line_index_plus1, 1);
    assert_ne!(record.archived_at_s, 0);
}

#[test]
fn task_local_short_change_selectors_are_unique_only_with_task_context() {
    let db = initialized_db("task-local-short-change-selectors");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db);

    for (task_id, title) in [("T-0001", "First Task"), ("T-0002", "Second Task")] {
        store
            .create_task(
                "repo",
                &json!({
                    "task_id": task_id,
                    "title": title,
                    "intent": "Prove Change ordinals are Task-local"
                }),
            )
            .expect("create Task");
        let requested_change_id = if task_id == "T-0001" {
            "C-01"
        } else {
            "T-0002/C-01"
        };
        let change = store
            .create_change(
                "repo",
                &json!({
                    "change_id": requested_change_id,
                    "task_id": task_id,
                    "title": format!("{title} Change"),
                    "base_line": "main"
                }),
            )
            .expect("create Task-local Change");
        assert_eq!(change["change_id"], "C-01");
        assert_eq!(change["change_ref"], format!("{task_id}/C-01"));

        if task_id == "T-0001" {
            let unique = store
                .get_change(Some("repo"), "C-01")
                .expect("resolve a unique short Change selector");
            assert_eq!(unique["change_ref"], "T-0001/C-01");
        }
    }

    for task_id in ["T-0001", "T-0002"] {
        let change_ref = format!("{task_id}/C-01");
        let exact = store
            .get_change(Some("repo"), &change_ref)
            .expect("resolve contextual Change ref");
        assert_eq!(exact["task_id"], task_id);
        assert_eq!(exact["change_id"], "C-01");
        assert_eq!(exact["change_ref"], change_ref);
    }

    let ambiguous = store
        .get_change(Some("repo"), "C-01")
        .expect_err("a repeated Task-local short selector must be ambiguous");
    assert!(
        ambiguous.contains("Ambiguous short Change selector \"C-01\"")
            && ambiguous.contains("T-0001")
            && ambiguous.contains("T-0002"),
        "{ambiguous}"
    );

    let changes = store
        .list_changes("repo")
        .expect("list Task-local Changes")
        .as_array()
        .expect("Change list")
        .clone();
    assert_eq!(
        changes
            .iter()
            .map(|change| change["change_id"].as_str())
            .collect::<Vec<_>>(),
        vec![Some("C-01"), Some("C-01")]
    );
    assert_eq!(
        changes
            .iter()
            .map(|change| change["change_ref"].as_str())
            .collect::<Vec<_>>(),
        vec![Some("T-0002/C-01"), Some("T-0001/C-01")]
    );
}

#[test]
fn queue_latest_succeeded_lands_selects_once_per_change_and_fails_closed() {
    let db = initialized_db("queue-latest-succeeded-lands");
    let store = BinaryDbServerWorkflowV0Store::new(db);
    let land = |change_index: u32, land_ordinal: u8, status: u8, tombstone: bool| {
        let succeeded = status == LAND_STATUS_SUCCEEDED;
        V0LandRecord {
            land_meta: status
                | if succeeded {
                    LAND_HAS_LANDED_SNAPSHOT
                } else {
                    0
                }
                | if tombstone { LAND_TOMBSTONE } else { 0 },
            land_ordinal,
            change_ordinal: 0,
            failure_kind: 0,
            change_index,
            patchset_index: 0,
            previous_task_land_index_plus1: 0,
            previous_change_land_index_plus1: 0,
            pre_land_target_snapshot_index_plus1: 0,
            landed_snapshot_index_plus1: if succeeded { 1 } else { 0 },
            submitted_at_s: 1,
            updated_at_s: 1,
            target_line_index_plus1: 1,
        }
    };
    let records = [
        land(0, 0, LAND_STATUS_SUCCEEDED, false),
        land(0, 5, LAND_STATUS_FAILED, false),
        land(0, 2, LAND_STATUS_SUCCEEDED, false),
        land(0, 1, LAND_STATUS_SUCCEEDED, false),
        land(1, 4, LAND_STATUS_SUCCEEDED, true),
        land(1, 1, LAND_STATUS_SUCCEEDED, false),
        land(0, 2, LAND_STATUS_SUCCEEDED, false),
    ]
    .into_iter()
    .map(|record| WorkflowBinaryV0Codec::encode_land(record).expect("encode fixture Land"))
    .collect::<Vec<_>>();

    let latest = store
        .latest_succeeded_lands_from_records(&records)
        .expect("select latest succeeded Lands");
    assert_eq!(latest.len(), 2);
    assert_eq!(
        latest
            .get(&0)
            .map(|(index, land)| (*index, land.land_ordinal)),
        Some((2, 2))
    );
    assert_eq!(
        latest
            .get(&1)
            .map(|(index, land)| (*index, land.land_ordinal)),
        Some((5, 1))
    );

    let malformed = vec![vec![0_u8; 35]];
    let error = store
        .latest_succeeded_lands_from_records(&malformed)
        .expect_err("malformed Land row must fail closed");
    assert!(error.contains("queue Land decode"));
}

#[test]
fn change_list_projects_greatest_successful_land_from_one_inventory() {
    let db = initialized_db("change-list-linear-lands");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .create_task(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Bound Change list Land reads",
                "intent": "Project all Change lifecycle facts from one Land inventory"
            }),
        )
        .expect("create Task");
    for (change_id, title) in [("C-01", "Landed Change"), ("C-02", "Unlanded Change")] {
        store
            .create_change(
                "repo",
                &json!({
                    "change_id": change_id,
                    "task_id": "T-0001",
                    "title": title,
                    "base_line": "main"
                }),
            )
            .expect("create Change");
    }

    let land = |change_index: u32,
                change_ordinal: u8,
                land_ordinal: u8,
                status: u8,
                tombstone: bool,
                landed_snapshot_index_plus1: u32,
                updated_at_s: u64| V0LandRecord {
        land_meta: status
            | if status == LAND_STATUS_SUCCEEDED {
                LAND_HAS_LANDED_SNAPSHOT
            } else {
                0
            }
            | if tombstone { LAND_TOMBSTONE } else { 0 },
        land_ordinal,
        change_ordinal,
        failure_kind: 0,
        change_index,
        patchset_index: 0,
        previous_task_land_index_plus1: 0,
        previous_change_land_index_plus1: 0,
        pre_land_target_snapshot_index_plus1: 0,
        landed_snapshot_index_plus1,
        submitted_at_s: updated_at_s,
        updated_at_s,
        target_line_index_plus1: 1,
    };
    let fixture_lands = [
        land(0, 0, 0, LAND_STATUS_SUCCEEDED, false, 1, 10),
        land(0, 0, 3, LAND_STATUS_FAILED, false, 0, 20),
        land(0, 0, 2, LAND_STATUS_SUCCEEDED, false, 2, 30),
        land(0, 0, 4, LAND_STATUS_SUCCEEDED, true, 3, 40),
        land(1, 1, 0, LAND_STATUS_FAILED, false, 0, 50),
    ];
    let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("begin Land fixture transaction");
    for record in fixture_lands {
        let raw = WorkflowBinaryV0Codec::encode_land(record).expect("encode fixture Land");
        write
            .append_record(WorkflowBinaryV0Codec::land_file(), &raw)
            .expect("append fixture Land");
    }
    write.commit().expect("commit Land fixtures");

    let changes = store.list_changes("repo").expect("list Changes");
    let changes = changes.as_array().expect("Change list array");
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0]["change_ref"], json!("T-0001/C-02"));
    assert!(changes[0]["target_line"].is_null());
    assert!(changes[0]["landed_at"].is_null());
    assert!(changes[0]["landed_snapshot_id"].is_null());
    assert_eq!(changes[1]["change_ref"], json!("T-0001/C-01"));
    assert_eq!(changes[1]["target_line"], json!("main"));
    assert_eq!(changes[1]["landed_at"], json!("1970-01-01T00:00:30+00:00"));
    assert_eq!(changes[1]["landed_snapshot_id"], json!("SNP-0000000000A2"));

    let landed = store
        .get_change(Some("repo"), "T-0001/C-01")
        .expect("read landed Change");
    assert_eq!(landed["landed_snapshot_id"], json!("SNP-0000000000A2"));
    let unlanded = store
        .get_change(Some("repo"), "T-0001/C-02")
        .expect("read unlanded Change");
    assert!(unlanded["landed_snapshot_id"].is_null());
}

#[test]
fn change_reads_project_selected_patchset_without_snapshot_tree_hydration() {
    let db = initialized_db("selected-patchset-projection");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    seed_task_change_and_patchset(&store);

    let read = BinaryDbReadTxn::new(&db);
    let mut revision = ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(
        &read
            .read_record(
                ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
                1,
            )
            .expect("read revision Snapshot"),
    )
    .expect("decode revision Snapshot");
    drop(read);
    revision.snapshot_meta |= ServerBinarySnapshotRecord::META_HAS_ROOT_LOCATOR;
    revision.root_tree_pack_index_plus1 = 1;
    let revision_raw =
        ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::encode_record(&revision)
            .expect("encode revision Snapshot with unavailable Tree locator");
    let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent)
        .expect("begin Snapshot fixture rewrite");
    write
        .overwrite_record(
            ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            1,
            &revision_raw,
        )
        .expect("rewrite revision Snapshot");
    write.commit().expect("commit Snapshot fixture rewrite");

    let patchset_error = store
        .get_patchset(Some("repo"), "T-0001/C-01/P-01")
        .expect_err("Patchset detail still materializes exact diff statistics");
    assert!(patchset_error.contains("Patchset Snapshot Tree comparison"));

    let projection = store
        .queue_projection_values_nonblocking()
        .expect("queue projection avoids Patchset Snapshot Trees");
    let queue_patchset = projection["patchsets"]
        .as_array()
        .and_then(|rows| rows.first())
        .expect("one queue Patchset");
    assert_eq!(queue_patchset["patchset_id"], json!("T-0001/C-01/P-01"));
    assert_eq!(
        queue_patchset["revision_snapshot_id"],
        json!("SNP-0000000000A2")
    );
    assert!(queue_patchset.get("summary").is_none());
    assert!(queue_patchset.get("diff_stats").is_none());

    let service = BinaryDbServerWorkflowReadModelService::new(db.clone(), store.clone().into_arc());
    let warming = service
        .read_queue_summary(Some("repo"), Some("active"), true)
        .expect_err("cold v0 queue projection warms in the background");
    assert_eq!(
        binary_db_runtime_error_kind(&warming),
        Some(BinaryDbErrorKind::RetryableBusy)
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let summary = loop {
        match service.read_queue_summary(Some("repo"), Some("active"), true) {
            Ok(summary) => break summary,
            Err(error)
                if binary_db_runtime_error_kind(&error)
                    == Some(BinaryDbErrorKind::RetryableBusy)
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("v0 queue projection did not warm: {error}"),
        }
    };
    assert_eq!(summary["task_queue"]["count"], json!(1));
    assert_eq!(summary["query_plan"]["input_counts"]["patchsets"], json!(1));

    let changes = store.list_changes("repo").expect("list Changes");
    let row = changes
        .as_array()
        .and_then(|rows| rows.first())
        .expect("one Change row");
    assert_eq!(row["selected_patchset_id"], json!("T-0001/C-01/P-01"));
    assert_eq!(row["selected_patchset_number"], json!(1));

    let change = store
        .get_change(Some("repo"), "T-0001/C-01")
        .expect("get Change");
    assert_eq!(change["selected_patchset_id"], json!("T-0001/C-01/P-01"));
    assert_eq!(change["selected_patchset_number"], json!(1));
}

#[test]
fn selected_patchset_projection_rejects_another_change_owner() {
    let db = initialized_db("selected-patchset-owner");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    seed_task_change_and_patchset(&store);
    store
        .create_change(
            "repo",
            &json!({
                "change_id": "C-02",
                "task_id": "T-0001",
                "title": "Second Change",
                "base_line": "main"
            }),
        )
        .expect("create second Change");
    store
        .publish_patchset(
            "T-0001/C-02",
            &json!({
                "base_snapshot_id": "SNP-0000000000A1",
                "revision_snapshot_id": "SNP-0000000000A3",
                "summary": "second Change Patchset",
                "author_mode": "ai_with_human_review"
            }),
        )
        .expect("publish second Change Patchset");

    let read = BinaryDbReadTxn::new(&db);
    let mut first_change = WorkflowBinaryV0Codec::decode_change(
        &read
            .read_record(WorkflowBinaryV0Codec::change_file(), 0)
            .expect("read first Change"),
    )
    .expect("decode first Change");
    drop(read);
    first_change.selected_patchset_index_plus1 = 2;
    let raw = WorkflowBinaryV0Codec::encode_change(first_change)
        .expect("encode cross-owned selected Patchset reference");
    let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("begin Change fixture rewrite");
    write
        .overwrite_record(WorkflowBinaryV0Codec::change_file(), 0, &raw)
        .expect("rewrite first Change");
    write.commit().expect("commit Change fixture rewrite");

    let error = store
        .get_change(Some("repo"), "T-0001/C-01")
        .expect_err("cross-owned selected Patchset must fail closed");
    assert_eq!(
        error,
        "Binary DB v0 selected Patchset belongs to another Change"
    );
}

#[test]
fn selected_patchset_projection_rejects_disagreeing_change_ordinal() {
    let db = initialized_db("selected-patchset-change-ordinal");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    seed_task_change_and_patchset(&store);

    let read = BinaryDbReadTxn::new(&db);
    let mut patchset = WorkflowBinaryV0Codec::decode_patchset(
        &read
            .read_record(WorkflowBinaryV0Codec::patchset_file(), 0)
            .expect("read selected Patchset"),
    )
    .expect("decode selected Patchset");
    drop(read);
    patchset.change_ordinal = 1;
    let raw = WorkflowBinaryV0Codec::encode_patchset(patchset)
        .expect("encode disagreeing Change ordinal");
    let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("begin Patchset fixture rewrite");
    write
        .overwrite_record(WorkflowBinaryV0Codec::patchset_file(), 0, &raw)
        .expect("rewrite selected Patchset");
    write.commit().expect("commit Patchset fixture rewrite");

    let error = store
        .get_change(Some("repo"), "T-0001/C-01")
        .expect_err("disagreeing Change ordinal must fail closed");
    assert_eq!(
        error,
        "Binary DB v0 selected Patchset Change ordinal disagrees"
    );
}

#[test]
fn actor_lookup_decodes_plus_one_targets_and_preserves_collision_candidates() {
    let db = initialized_db("actor-lookup-plus-one");
    let index = WorkflowBinaryV0Codec::actor_lookup_index();
    assert!(index.stores_record_index_plus_one());
    let key = 0x8877_6655_4433_2211_u64.to_le_bytes();

    let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("begin Actor lookup fixture");
    write
        .append_index_candidate(index.clone(), &key, 0)
        .expect("append Actor 0 candidate");
    write
        .append_index_candidate(index.clone(), &key, 1)
        .expect("append colliding Actor 1 candidate");
    write.commit().expect("commit Actor lookup fixture");

    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.lookup_index(index.clone(), &key)
            .expect("lookup Actor collision candidates"),
        vec![0, 1]
    );
    let raw = fs::read(
        db.resolve_index_path(&index)
            .expect("resolve Actor lookup index"),
    )
    .expect("read Actor lookup index");
    assert_eq!(raw.len(), 4 + 2 * 12);
    assert_eq!(u32::from_le_bytes(raw[12..16].try_into().unwrap()), 1);
    assert_eq!(u32::from_le_bytes(raw[24..28].try_into().unwrap()), 2);
}

fn atomic_task_land_request(task_or_change_ref: &str) -> JsonValue {
    json!({
        "contract": "task-land-atomic/v1",
        "idempotency_key": format!("atomic-land:{task_or_change_ref}"),
        "task_or_change_ref": task_or_change_ref,
        "target_line": "main",
        "mode": "direct"
    })
}

fn prepare_ready_atomic_task_land(
    store: &BinaryDbServerWorkflowV0Store<TestDb>,
    revision_snapshot_id: &str,
) {
    store
        .create_task(
            "repo",
            &json!({
                "title": "Ready Atomic Task Land",
                "intent": "Prepare an already-ready selected Patchset"
            }),
        )
        .expect("create Task");
    store
        .create_change(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Ready Atomic Task Land Change",
                "base_line": "main"
            }),
        )
        .expect("create Change");
    store
        .publish_patchset(
            "T-0001/C-01",
            &json!({
                "base_snapshot_id": "SNP-0000000000A1",
                "revision_snapshot_id": revision_snapshot_id,
                "summary": "ready atomic Task Land",
                "author_mode": "ai_with_human_review"
            }),
        )
        .expect("publish Patchset");
    store
        .record_review(
            "T-0001/C-01",
            &json!({
                "patchset_id": "T-0001/C-01/P-01",
                "reviewer": "owner",
                "action": "approve"
            }),
        )
        .expect("approve Patchset");
    store
        .put_attestation(
            "T-0001/C-01/P-01",
            &json!({
                "verification_state": "pass",
                "require_tests_pass": false
            }),
        )
        .expect("attest Patchset");
    assert_eq!(
        store
            .evaluate_policy("T-0001/C-01/P-01")
            .expect("evaluate Policy")["decision"],
        "pass"
    );
}

#[test]
fn atomic_task_land_commits_land_change_task_and_line_and_replays_without_append() {
    let db = initialized_db("atomic-task-land");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .create_task(
            "repo",
            &json!({
                "title": "Atomic Task Land",
                "intent": "Close all Task Land state in one transaction"
            }),
        )
        .expect("create Task");
    store
        .create_change(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Atomic Task Land Change",
                "base_line": "main"
            }),
        )
        .expect("create Change");
    store
        .publish_patchset(
            "T-0001/C-01",
            &json!({
                "base_snapshot_id": "SNP-0000000000A1",
                "revision_snapshot_id": "SNP-0000000000A2",
                "summary": "ready atomic Task Land",
                "author_mode": "ai_with_human_review"
            }),
        )
        .expect("publish Patchset");
    store
        .record_review(
            "T-0001/C-01",
            &json!({
                "patchset_id": "T-0001/C-01/P-01",
                "reviewer": "owner",
                "action": "approve"
            }),
        )
        .expect("approve Patchset");
    store
        .put_attestation(
            "T-0001/C-01/P-01",
            &json!({
                "verification_state": "pass",
                "require_tests_pass": false
            }),
        )
        .expect("attest Patchset");
    assert_eq!(
        store
            .evaluate_policy("T-0001/C-01/P-01")
            .expect("evaluate Policy")["decision"],
        "pass"
    );

    let request = atomic_task_land_request("T-0001");
    let landed = store
        .submit_task_land("T-0001", &request)
        .expect("atomically land Task");
    assert_eq!(landed["contract"], "task-land-atomic/v1");
    assert_eq!(landed["replayed"], false);
    assert_eq!(landed["status"], "succeeded");
    assert_eq!(landed["task"]["status"], "completed");
    assert_eq!(landed["task"]["task_seq"], 1);
    assert_eq!(landed["change"]["status"], "landed");
    assert_eq!(landed["change"]["change_ref"], "T-0001/C-01");
    assert_eq!(landed["change"]["selected_patchset_id"], "T-0001/C-01/P-01");
    assert_eq!(landed["patchset"]["patchset_id"], "T-0001/C-01/P-01");
    assert_eq!(landed["patchset"]["patchset_number"], 1);
    assert_eq!(landed["land"]["land_seq"], 1);
    assert_eq!(landed["land"]["change_ref"], "T-0001/C-01");
    assert_eq!(landed["land"]["patchset_id"], "T-0001/C-01/P-01");
    assert_eq!(landed["land"]["landed_snapshot_id"], "SNP-0000000000A2");

    let replayed = store
        .submit_task_land("T-0001", &request)
        .expect("replay atomic Task Land");
    assert_eq!(replayed["replayed"], true);
    assert_eq!(
        replayed["land"]["submission_id"],
        landed["land"]["submission_id"]
    );
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::land_file())
            .expect("Land count"),
        1
    );
    let physical_land = WorkflowBinaryV0Codec::decode_land(
        &read
            .read_record(WorkflowBinaryV0Codec::land_file(), 0)
            .expect("read physical Land"),
    )
    .expect("decode physical Land");
    assert_eq!(physical_land.change_index, 0);
    assert_eq!(physical_land.patchset_index, 0);
    let physical_change = WorkflowBinaryV0Codec::decode_change(
        &read
            .read_record(WorkflowBinaryV0Codec::change_file(), 0)
            .expect("read physical Change"),
    )
    .expect("decode physical Change");
    assert_eq!(physical_change.task_index, 0);
    assert_eq!(physical_change.selected_patchset_index_plus1, 1);
}

#[test]
fn task_audit_terminal_status_precedes_landed_change_readiness() {
    let db = initialized_db("task-audit-terminal-precedence");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db);
    prepare_ready_atomic_task_land(&store, "SNP-0000000000A2");

    store
        .submit_land(
            "T-0001/C-01",
            &json!({
                "target_line": "main",
                "mode": "direct"
            }),
        )
        .expect("land Change without closing Task");
    let active = store
        .read_task_audit("repo", "T-0001", "main")
        .expect("audit active Task with landed Change");
    assert_eq!(active["task"]["status"], "active");
    assert_eq!(active["summary"]["verdict"], "ready_to_close");
    assert_eq!(active["verdict"]["status"], "ready_to_close");

    store
        .close_task("T-0001", &json!({"status": "completed"}))
        .expect("complete landed Task");
    let completed = store
        .read_task_audit("repo", "T-0001", "main")
        .expect("audit completed Task");
    assert_eq!(completed["task"]["status"], "completed");
    assert_eq!(completed["summary"]["verdict"], "task_completed");
    assert_eq!(completed["verdict"]["code"], "task_completed");
    assert_eq!(completed["verdict"]["status"], "task_completed");
}

#[test]
fn atomic_task_land_rolls_back_when_response_projection_fails() {
    let db = initialized_db("atomic-task-land-response-projection-rollback");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    prepare_ready_atomic_task_land(&store, "SNP-0000000000A2");

    let read = BinaryDbReadTxn::new(&db);
    let mut task = WorkflowBinaryV0Codec::decode_task(
        &read
            .read_record(WorkflowBinaryV0Codec::task_file(), 0)
            .expect("read Task before invalid projection fixture"),
    )
    .expect("decode Task before invalid projection fixture");
    drop(read);
    task.origin_plan_revision_index_plus1 = 1;
    task.plan_item_index_plus1 = 1;
    let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("begin invalid Task projection fixture");
    write
        .overwrite_record(
            WorkflowBinaryV0Codec::task_file(),
            0,
            &WorkflowBinaryV0Codec::encode_task(task)
                .expect("encode invalid Task projection fixture"),
        )
        .expect("write invalid Task projection fixture");
    write
        .commit()
        .expect("commit invalid Task projection fixture");

    let error = store
        .submit_task_land("T-0001", &atomic_task_land_request("T-0001"))
        .expect_err("response projection failure must abort atomic Task Land");
    assert!(error.contains("plan_revision.bin"), "{error}");

    let read = BinaryDbReadTxn::new(&db);
    let task = WorkflowBinaryV0Codec::decode_task(
        &read
            .read_record(WorkflowBinaryV0Codec::task_file(), 0)
            .expect("read Task after projection rollback"),
    )
    .expect("decode Task after projection rollback");
    assert_eq!(task.task_meta & TASK_META_COMPLETED, 0);
    assert_eq!(task.closed_at_s, 0);
    let change = WorkflowBinaryV0Codec::decode_change(
        &read
            .read_record(WorkflowBinaryV0Codec::change_file(), 0)
            .expect("read Change after projection rollback"),
    )
    .expect("decode Change after projection rollback");
    assert_eq!(change.lifecycle(), CHANGE_LIFECYCLE_ACTIVE);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::land_file())
            .expect("Land count after projection rollback"),
        0
    );
    let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    let (_, line) = lines
        .line_by_name(&read, "main")
        .expect("read main Line after projection rollback")
        .expect("main Line exists after projection rollback");
    assert_eq!(line.head_snapshot_index_plus1, 1);
}

#[test]
fn history_promotion_preserves_ten_local_receipts_and_lands_one_aggregate() {
    let db = initialized_db("history-promotion-ten-local-lands");
    let snapshot_ids = seed_history_content(&db, 10);
    let plan_service = BinaryDbServerPlanService::new(db.clone());
    let plans = (0..10)
        .map(|ordinal| {
            let item_ref = format!("HISTORY-PROMOTION-{:02}", ordinal + 1);
            let mut payload =
                atomic_plan_payload(&format!("History Plan {}", ordinal + 1), &item_ref);
            payload["artifact_path"] = json!(format!("docs/sprints/history-{:02}.md", ordinal + 1));
            let plan = plan_service
                .create_plan("repo", &payload)
                .expect("create promoted history Plan");
            (plan, item_ref)
        })
        .collect::<Vec<_>>();
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());
    let mut request = history_promotion_request(&snapshot_ids);
    for (entry, plan) in request["entries"]
        .as_array_mut()
        .expect("history entries")
        .iter_mut()
        .zip(plans.iter())
    {
        entry["task"]["plan_id"] = plan.0["plan_id"].clone();
        entry["task"]["origin_plan_revision_id"] = plan.0["head_revision_id"].clone();
        entry["task"]["plan_item_ref"] = json!(plan.1);
    }

    let prepare_started = Instant::now();
    let prepared = store
        .prepare_history_promotion("repo", &request)
        .expect("prepare ten local histories");
    let prepare_elapsed_ms = prepare_started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(prepared["contract"], "history-promotion-prepare/v1");
    assert_eq!(prepared["replayed"], false);
    assert_eq!(prepared["entries"].as_array().unwrap().len(), 10);
    assert_eq!(
        prepared["aggregate"]["patchset"]["source_kind"],
        "history_promotion_aggregate"
    );
    assert_eq!(
        prepared["aggregate"]["patchset"]["governance_authority"],
        true
    );
    for entry in prepared["entries"].as_array().unwrap() {
        let receipt = store
            .get_patchset(None, entry["receipt_patchset_id"].as_str().unwrap())
            .expect("read receipt Patchset");
        assert_eq!(receipt["source_kind"], "imported_local_land_receipt");
        assert_eq!(receipt["governance_authority"], false);
        assert_eq!(receipt["evaluation_state"], "pending");
    }
    let first_entry = &prepared["entries"][0];
    let first_receipt_id = first_entry["receipt_patchset_id"].as_str().unwrap();
    let ci_error = store
        .run_patchset_ci(first_receipt_id, &json!({}))
        .expect_err("receipt Patchset cannot run CI");
    assert!(ci_error.contains("provenance-only"), "{ci_error}");
    let policy_error = store
        .evaluate_policy(first_receipt_id)
        .expect_err("receipt Patchset cannot evaluate Policy");
    assert!(policy_error.contains("provenance-only"), "{policy_error}");
    let review_error = store
        .record_review(
            first_entry["change_ref"].as_str().unwrap(),
            &json!({
                "patchset_id": first_receipt_id,
                "reviewer": "owner",
                "action": "approve"
            }),
        )
        .expect_err("receipt Patchset cannot claim Review approval");
    assert!(review_error.contains("provenance-only"), "{review_error}");
    let receipt_land_error = store
        .submit_land(
            first_entry["change_ref"].as_str().unwrap(),
            &json!({
                "target_line": "main",
                "mode": "direct"
            }),
        )
        .expect_err("receipt Patchset cannot land independently");
    assert!(
        receipt_land_error.contains("provenance-only"),
        "{receipt_land_error}"
    );
    let replayed = store
        .prepare_history_promotion("repo", &request)
        .expect("replay history prepare");
    assert_eq!(replayed["replayed"], true);
    assert_eq!(
        replayed["aggregate"]["patchset_id"],
        prepared["aggregate"]["patchset_id"]
    );
    let mut conflicting_request = request.clone();
    conflicting_request["summary"] = json!("different request under the same key");
    let conflict = store
        .prepare_history_promotion("repo", &conflicting_request)
        .expect_err("same idempotency key with another fingerprint must fail");
    assert!(
        conflict.contains("HISTORY_PROMOTION_IDEMPOTENCY_CONFLICT"),
        "{conflict}"
    );

    let aggregate_change_ref = prepared["aggregate"]["change_ref"].as_str().unwrap();
    let aggregate_patchset_id = prepared["aggregate"]["patchset_id"].as_str().unwrap();
    store
        .record_review(
            aggregate_change_ref,
            &json!({
                "patchset_id": aggregate_patchset_id,
                "reviewer": "owner",
                "action": "approve"
            }),
        )
        .expect("approve aggregate Patchset");
    store
        .put_attestation(
            aggregate_patchset_id,
            &json!({
                "verification_state": "pass",
                "require_tests_pass": false
            }),
        )
        .expect("attest aggregate Patchset");
    assert_eq!(
        store
            .evaluate_policy(aggregate_patchset_id)
            .expect("evaluate aggregate Policy")["decision"],
        "pass"
    );

    let land_request = json!({
        "contract": "task-land-atomic/v1",
        "idempotency_key": "atomic-land:history-promotion-ten",
        "task_or_change_ref": aggregate_change_ref,
        "target_line": "main",
        "mode": "direct"
    });
    let land_started = Instant::now();
    let landed = store
        .submit_task_land(aggregate_change_ref, &land_request)
        .expect("atomically land receipts and aggregate");
    let land_elapsed_ms = land_started.elapsed().as_secs_f64() * 1_000.0;
    assert_eq!(landed["status"], "succeeded");
    assert_eq!(
        landed["landed_snapshot_id"],
        snapshot_ids.last().unwrap().as_str()
    );
    assert_eq!(
        landed["history_promotion"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        10
    );

    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .expect("Task count"),
        10
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .expect("Change count"),
        10
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .expect("Patchset count"),
        11
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::snapshot_link_file())
            .expect("Snapshot Link count"),
        10
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::land_file())
            .expect("Land count"),
        11
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::policy_file())
            .expect("Policy count"),
        1
    );
    drop(read);

    for (ordinal, entry) in prepared["entries"].as_array().unwrap().iter().enumerate() {
        let task = store
            .get_task(Some("repo"), entry["task_id"].as_str().unwrap())
            .expect("read completed promoted Task");
        assert_eq!(task["status"], "completed");
        assert_eq!(task["plan_id"], plans[ordinal].0["plan_id"]);
        assert_eq!(
            task["origin_plan_revision_id"],
            plans[ordinal].0["head_revision_id"]
        );
        assert_eq!(task["plan_item_ref"], plans[ordinal].1);
        let change = store
            .get_change(Some("repo"), entry["change_ref"].as_str().unwrap())
            .expect("read landed promoted Change");
        assert_eq!(change["status"], "landed");
        let receipt_land = store
            .get_land(
                Some("repo"),
                &format!("{}/L-01", entry["change_ref"].as_str().unwrap()),
            )
            .expect("read imported receipt Land");
        assert_eq!(receipt_land["source_kind"], "imported_local_land_receipt");
        assert_eq!(receipt_land["governance_authority"], false);
        assert_eq!(
            receipt_land["result"]["target_line_head"],
            request["entries"][ordinal]["pre_land_target_snapshot_id"]
        );
    }
    let aggregate_land = store
        .get_land(
            Some("repo"),
            landed["land"]["submission_id"].as_str().unwrap(),
        )
        .expect("read aggregate Land");
    assert_eq!(aggregate_land["source_kind"], "history_promotion_aggregate");
    assert_eq!(aggregate_land["governance_authority"], true);
    assert_eq!(
        aggregate_land["result"]["target_line_head"],
        request["base_snapshot_id"]
    );
    assert_eq!(
        aggregate_land["landed_snapshot_id"],
        request["revision_snapshot_id"]
    );

    let landed_replay = store
        .submit_task_land(aggregate_change_ref, &land_request)
        .expect("replay aggregate Task Land");
    assert_eq!(landed_replay["replayed"], true);
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .expect("Land count after replay"),
        11
    );
    eprintln!(
        "AIT_SERVER_HISTORY_TIMINGS {}",
        json!({
            "history_entry_count": 10,
            "prepare_ms": prepare_elapsed_ms,
            "atomic_receipts_plus_aggregate_land_ms": land_elapsed_ms,
        })
    );
    validate_frozen_server_workflow_v0(&db).expect("validate promoted frozen v0 workflow");
}

#[test]
fn staged_history_promotion_preserves_sixty_five_receipts_with_recoverable_closeout() {
    let db = initialized_db("history-promotion-sixty-five-local-lands");
    let snapshot_ids = seed_history_content(&db, 65);
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());

    let stage_zero_request = staged_history_promotion_request(&snapshot_ids, 0, None);
    let stage_zero = store
        .prepare_history_promotion("repo", &stage_zero_request)
        .expect("prepare first bounded history stage");
    assert_eq!(stage_zero["contract"], "history-promotion-prepare/v2");
    assert_eq!(stage_zero["stage_ordinal"], 0);
    assert_eq!(stage_zero["final_stage"], false);
    assert_eq!(stage_zero["entries"].as_array().unwrap().len(), 64);
    assert!(stage_zero["aggregate"].is_null());
    assert_eq!(
        stage_zero["stage"]["patchset"]["source_kind"],
        "history_promotion_stage"
    );
    assert_eq!(
        stage_zero["stage"]["patchset"]["governance_authority"],
        false
    );
    let stage_zero_patchset_id = stage_zero["stage"]["patchset_id"]
        .as_str()
        .expect("stage zero Patchset identity");
    let governance_error = store
        .evaluate_policy(stage_zero_patchset_id)
        .expect_err("intermediate stage must not have governance authority");
    assert!(
        governance_error.contains("provenance-only"),
        "{governance_error}"
    );

    let stage_zero_replay = store
        .prepare_history_promotion("repo", &stage_zero_request)
        .expect("replay first bounded history stage");
    assert_eq!(stage_zero_replay["replayed"], true);
    assert_eq!(
        stage_zero_replay["stage"]["patchset_id"],
        stage_zero["stage"]["patchset_id"]
    );

    let missing_predecessor = staged_history_promotion_request(&snapshot_ids, 1, None);
    let missing_error = store
        .prepare_history_promotion("repo", &missing_predecessor)
        .expect_err("continuation without predecessor must fail");
    assert!(
        missing_error.contains("requires a previous stage"),
        "{missing_error}"
    );

    let final_request =
        staged_history_promotion_request(&snapshot_ids, 1, Some(stage_zero_patchset_id));
    let prepared = store
        .prepare_history_promotion("repo", &final_request)
        .expect("prepare final bounded history stage");
    assert_eq!(prepared["stage_ordinal"], 1);
    assert_eq!(prepared["final_stage"], true);
    assert_eq!(prepared["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        prepared["aggregate"]["patchset"]["source_kind"],
        "history_promotion_aggregate"
    );
    assert_eq!(
        prepared["aggregate"]["patchset"]["governance_authority"],
        true
    );

    let mut conflicting_successor = final_request.clone();
    conflicting_successor["idempotency_key"] =
        json!("history-promotion:65-local-lands:conflicting-stage-1");
    let conflict = store
        .prepare_history_promotion("repo", &conflicting_successor)
        .expect_err("promotion ordinal must have one canonical successor");
    assert!(
        conflict.contains("HISTORY_PROMOTION_STAGE_CONFLICT"),
        "{conflict}"
    );

    let aggregate_change_ref = prepared["aggregate"]["change_ref"]
        .as_str()
        .expect("aggregate Change identity");
    let aggregate_patchset_id = prepared["aggregate"]["patchset_id"]
        .as_str()
        .expect("aggregate Patchset identity");
    store
        .record_review(
            aggregate_change_ref,
            &json!({
                "patchset_id": aggregate_patchset_id,
                "reviewer": "owner",
                "action": "approve"
            }),
        )
        .expect("approve staged history aggregate");
    store
        .put_attestation(
            aggregate_patchset_id,
            &json!({
                "verification_state": "pass",
                "require_tests_pass": false
            }),
        )
        .expect("attest staged history aggregate");
    assert_eq!(
        store
            .evaluate_policy(aggregate_patchset_id)
            .expect("evaluate staged history aggregate Policy")["decision"],
        "pass"
    );

    store
        .apply_staged_history_receipts_before_atomic_land_for_test(
            aggregate_change_ref,
            aggregate_change_ref,
            "main",
            None,
            None,
        )
        .expect("close preceding receipt stage");
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .expect("partial receipt Land count"),
        64
    );
    store
        .apply_staged_history_receipts_before_atomic_land_for_test(
            aggregate_change_ref,
            aggregate_change_ref,
            "main",
            None,
            None,
        )
        .expect("replay preceding receipt closeout");
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .expect("replayed partial receipt Land count"),
        64
    );
    let read = BinaryDbReadTxn::new(&db);
    let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    let (_, line) = lines
        .line_by_name(&read, "main")
        .expect("read main Line during partial closeout")
        .expect("main Line during partial closeout");
    assert_eq!(line.head_snapshot_index_plus1, 1);
    drop(read);

    let land_request = json!({
        "contract": "task-land-atomic/v1",
        "idempotency_key": "atomic-land:history-promotion-sixty-five",
        "task_or_change_ref": aggregate_change_ref,
        "target_line": "main",
        "mode": "direct"
    });
    let landed = store
        .submit_task_land(aggregate_change_ref, &land_request)
        .expect("land all staged receipts and the sole aggregate");
    assert_eq!(landed["status"], "succeeded");
    assert_eq!(
        landed["landed_snapshot_id"],
        snapshot_ids.last().unwrap().as_str()
    );
    assert_eq!(landed["history_promotion"]["total_entry_count"], 65);

    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .expect("staged Task count"),
        65
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .expect("staged Change count"),
        65
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .expect("staged Patchset count"),
        67
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::land_file())
            .expect("staged Land count"),
        66
    );
    let (_, line) = lines
        .line_by_name(&read, "main")
        .expect("read final staged main Line")
        .expect("final staged main Line");
    assert_eq!(line.head_snapshot_index_plus1, 66);
    drop(read);

    let replayed = store
        .submit_task_land(aggregate_change_ref, &land_request)
        .expect("replay final staged aggregate Land");
    assert_eq!(replayed["replayed"], true);
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .expect("staged Land count after replay"),
        66
    );
    validate_frozen_server_workflow_v0(&db).expect("validate staged frozen v0 workflow");
}

#[cfg(feature = "perfetto-tracing")]
#[test]
#[ignore = "release-profile Perfetto evidence harness"]
fn perfetto_history_promotion_and_atomic_land_ten_by_thirty() {
    let trace_path = temporary_root("perfetto-history-promotion")
        .as_path()
        .with_extension("json");

    for sample in 0..30 {
        let label = format!("perfetto-history-promotion-{sample}");
        let db = initialized_db(&label);
        let snapshot_ids = seed_history_content(&db, 10);
        let store = BinaryDbServerWorkflowV0Store::new(db.clone());
        let mut request = history_promotion_request(&snapshot_ids);
        request["idempotency_key"] = json!(format!("history-promotion:perfetto-sample-{sample}"));

        let prepared = {
            let _trace = crate::perfetto_trace::PerfettoRange::for_test(
                "ait.server.history_promotion.perf.prepare_10",
                trace_path.clone(),
            );
            store
                .prepare_history_promotion("repo", &request)
                .expect("prepare ten-entry history promotion")
        };
        assert_eq!(prepared["entries"].as_array().map(Vec::len), Some(10));

        let aggregate_change_ref = prepared["aggregate"]["change_ref"]
            .as_str()
            .expect("aggregate Change reference");
        let aggregate_patchset_id = prepared["aggregate"]["patchset_id"]
            .as_str()
            .expect("aggregate Patchset identity");
        store
            .record_review(
                aggregate_change_ref,
                &json!({
                    "patchset_id": aggregate_patchset_id,
                    "reviewer": "owner",
                    "action": "approve"
                }),
            )
            .expect("approve aggregate Patchset");
        store
            .put_attestation(
                aggregate_patchset_id,
                &json!({
                    "verification_state": "pass",
                    "require_tests_pass": false
                }),
            )
            .expect("attest aggregate Patchset");
        assert_eq!(
            store
                .evaluate_policy(aggregate_patchset_id)
                .expect("evaluate aggregate Policy")["decision"],
            "pass"
        );

        let land_request = json!({
            "contract": "task-land-atomic/v1",
            "idempotency_key": format!("atomic-land:perfetto-sample-{sample}"),
            "task_or_change_ref": aggregate_change_ref,
            "target_line": "main",
            "mode": "direct"
        });
        let landed = {
            let _trace = crate::perfetto_trace::PerfettoRange::for_test(
                "ait.server.task_land.perf.receipts_plus_aggregate_10",
                trace_path.clone(),
            );
            store
                .submit_task_land(aggregate_change_ref, &land_request)
                .expect("land ten receipts and aggregate atomically")
        };
        assert_eq!(landed["status"], "succeeded");
        assert_eq!(
            BinaryDbReadTxn::new(&db)
                .record_count(WorkflowBinaryV0Codec::land_file())
                .expect("Land count after aggregate closeout"),
            11
        );
    }

    let trace: JsonValue =
        serde_json::from_slice(&fs::read(&trace_path).expect("read Perfetto evidence trace"))
            .expect("decode Perfetto evidence trace");
    let events = trace["traceEvents"]
        .as_array()
        .expect("Perfetto traceEvents");
    for expected in [
        "ait.server.history_promotion.perf.prepare_10",
        "ait.server.history_promotion.request_decode_validate",
        "ait.server.history_promotion.plan_binding_resolution",
        "ait.server.history_promotion.writer_critical_section",
        "ait.server.history_promotion.writer_admission",
        "ait.server.history_promotion.idempotent_replay_lookup",
        "ait.server.history_promotion.write_set_prepare",
        "ait.server.history_promotion.transaction_mutation",
        "ait.server.history_promotion.transaction_commit",
        "ait.server.history_promotion.response_projection",
        "ait.server.task_land.perf.receipts_plus_aggregate_10",
        "ait.server.task_land.atomic.request_prepare",
        "ait.server.task_land.atomic.writer_critical_section",
        "ait.server.task_land.atomic.writer_admission",
        "ait.server.task_land.atomic.authoritative_revalidation",
        "ait.server.task_land.atomic.history_receipt_mutation",
        "ait.server.task_land.atomic.aggregate_land_mutation",
        "ait.server.task_land.atomic.transaction_commit",
        "ait.server.task_land.atomic.response_projection",
    ] {
        let count = events
            .iter()
            .filter(|event| event["name"].as_str() == Some(expected))
            .count();
        assert_eq!(count, 30, "expected thirty Perfetto samples for {expected}");
    }
    let _ = fs::remove_file(trace_path);
}

#[test]
fn history_promotion_resolves_plan_binding_before_opening_workflow_writer() {
    let db = initialized_db("history-promotion-plan-binding-before-writer");
    let snapshot_ids = seed_history_content(&db, 1);
    let plan = BinaryDbServerPlanService::new(db.clone())
        .create_plan(
            "repo",
            &atomic_plan_payload("History promotion Plan", "HISTORY-PROMOTION-01"),
        )
        .expect("create bound history Plan");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let mut request = history_promotion_request(&snapshot_ids);
    request["idempotency_key"] = json!("history-promotion:plan-bound");
    request["entries"][0]["task"]["plan_id"] = plan["plan_id"].clone();
    request["entries"][0]["task"]["origin_plan_revision_id"] = plan["head_revision_id"].clone();
    request["entries"][0]["task"]["plan_item_ref"] = json!("HISTORY-PROMOTION-01");

    let prepared = store
        .prepare_history_promotion("repo", &request)
        .expect("prepare plan-bound history without a self-conflicting read");
    assert_eq!(prepared["replayed"], false);
    let task_id = prepared["entries"][0]["task_id"].as_str().unwrap();
    let task = store
        .get_task(Some("repo"), task_id)
        .expect("read promoted plan-bound Task");
    assert_eq!(task["plan_id"], plan["plan_id"]);
    assert_eq!(task["origin_plan_revision_id"], plan["head_revision_id"]);
    assert_eq!(task["plan_item_ref"], "HISTORY-PROMOTION-01");

    let replayed = store
        .prepare_history_promotion("repo", &request)
        .expect("replay plan-bound history");
    assert_eq!(replayed["replayed"], true);
    assert_eq!(
        replayed["aggregate"]["patchset_id"],
        prepared["aggregate"]["patchset_id"]
    );
}

#[test]
fn history_promotion_supersedes_an_abandoned_plan_bound_task() {
    let db = initialized_db("history-promotion-abandoned-plan-binding");
    let snapshot_ids = seed_history_content(&db, 1);
    let item_ref = "HISTORY-ABANDONED-01";
    let plan = BinaryDbServerPlanService::new(db.clone())
        .create_plan(
            "repo",
            &atomic_plan_payload("Abandoned history binding", item_ref),
        )
        .expect("create history Plan");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let abandoned = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "history-abandoned-first",
                item_ref,
                json!({
                    "action": "existing",
                    "plan_id": plan["plan_id"],
                    "plan_revision_id": plan["head_revision_id"]
                }),
                "Abandoned remote Task",
                "Superseded remote Change",
                "main",
            ),
        )
        .expect("start first Plan-bound Task");
    let abandoned_task_id = abandoned["task_id"].as_str().unwrap();
    let abandoned_change_ref = abandoned["change"]["change_ref"].as_str().unwrap();
    store
        .close_task(abandoned_task_id, &json!({"status": "abandoned"}))
        .expect("abandon first Task");
    store
        .close_change(abandoned_change_ref, &json!({"status": "superseded"}))
        .expect("supersede first Change");

    let mut request = history_promotion_request(&snapshot_ids);
    request["idempotency_key"] = json!("history-promotion:abandoned-plan-binding");
    request["entries"][0]["task"]["plan_id"] = plan["plan_id"].clone();
    request["entries"][0]["task"]["origin_plan_revision_id"] = plan["head_revision_id"].clone();
    request["entries"][0]["task"]["plan_item_ref"] = json!(item_ref);

    let prepared = store
        .prepare_history_promotion("repo", &request)
        .expect("supersede abandoned Plan-bound Task with local history");
    assert_eq!(prepared["replayed"], false);
    assert_eq!(prepared["entries"][0]["task_id"], "T-0002");
    assert_ne!(prepared["entries"][0]["task_id"], abandoned_task_id);
    assert_eq!(
        store
            .get_task(None, abandoned_task_id)
            .expect("read abandoned Task")["status"],
        "abandoned"
    );
    assert_eq!(
        store
            .get_change(None, abandoned_change_ref)
            .expect("read superseded Change")["status"],
        "superseded"
    );

    let replayed = store
        .prepare_history_promotion("repo", &request)
        .expect("replay replacement history receipt");
    assert_eq!(replayed["replayed"], true);
    assert_eq!(
        replayed["aggregate"]["patchset_id"],
        prepared["aggregate"]["patchset_id"]
    );
    assert_eq!(
        store
            .list_tasks("repo")
            .expect("list Plan-binding owners")
            .as_array()
            .map(Vec::len),
        Some(2)
    );
}

#[test]
fn history_promotion_rejects_active_or_completed_plan_bindings_without_a_receipt() {
    let db = initialized_db("history-promotion-active-plan-binding");
    let snapshot_ids = seed_history_content(&db, 1);
    let item_ref = "HISTORY-ACTIVE-01";
    let plan = BinaryDbServerPlanService::new(db.clone())
        .create_plan(
            "repo",
            &atomic_plan_payload("Active history binding", item_ref),
        )
        .expect("create history Plan");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let active = store
        .start_plan_bound_task(
            "repo",
            &atomic_task_start_request(
                "history-active-first",
                item_ref,
                json!({
                    "action": "existing",
                    "plan_id": plan["plan_id"],
                    "plan_revision_id": plan["head_revision_id"]
                }),
                "Active remote Task",
                "Active remote Change",
                "main",
            ),
        )
        .expect("start active Plan-bound Task");
    let active_task_id = active["task_id"].as_str().unwrap();

    let mut request = history_promotion_request(&snapshot_ids);
    request["idempotency_key"] = json!("history-promotion:active-plan-binding");
    request["entries"][0]["task"]["plan_id"] = plan["plan_id"].clone();
    request["entries"][0]["task"]["origin_plan_revision_id"] = plan["head_revision_id"].clone();
    request["entries"][0]["task"]["plan_item_ref"] = json!(item_ref);

    let error = store
        .prepare_history_promotion("repo", &request)
        .expect_err("active Plan binding must remain exclusive");
    assert!(
        error.contains("already owns a server Task without an exact reusable receipt"),
        "{error}"
    );
    store
        .close_task(active_task_id, &json!({"status": "completed"}))
        .expect("complete first Task");
    request["idempotency_key"] = json!("history-promotion:completed-plan-binding");
    let completed_error = store
        .prepare_history_promotion("repo", &request)
        .expect_err("completed Plan binding must remain exclusive");
    assert!(
        completed_error.contains("already owns a server Task without an exact reusable receipt"),
        "{completed_error}"
    );
    assert_eq!(
        store
            .list_tasks("repo")
            .expect("list active Plan-binding owner")
            .as_array()
            .map(Vec::len),
        Some(1)
    );
}

#[test]
fn history_promotion_replay_validates_expected_publication_identity_without_fingerprint_drift() {
    let db = initialized_db("history-promotion-expected-publication-replay");
    let snapshot_ids = seed_history_content(&db, 1);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let mut request = history_promotion_request(&snapshot_ids);
    request["idempotency_key"] = json!("history-promotion:expected-publication-replay");

    let first = store
        .prepare_history_promotion("repo", &request)
        .expect("prepare initial publication receipt");
    let task_id = first["entries"][0]["task_id"]
        .as_str()
        .expect("remote Task id")
        .to_string();
    let change_ref = first["entries"][0]["change_ref"]
        .as_str()
        .expect("remote Change ref")
        .to_string();

    let mut exact_retry = request.clone();
    exact_retry["entries"][0]["expected_remote_task_id"] = json!(task_id);
    exact_retry["entries"][0]["expected_remote_change_ref"] = json!(change_ref);
    let replayed = store
        .prepare_history_promotion("repo", &exact_retry)
        .expect("expected identities must preserve idempotent replay");
    assert_eq!(replayed["replayed"], true);
    assert_eq!(replayed["entries"][0]["task_id"], task_id);
    assert_eq!(replayed["entries"][0]["change_ref"], change_ref);

    let mut task_first_retry = exact_retry.clone();
    task_first_retry["entries"][0]["expected_remote_change_ref"] = JsonValue::Null;
    let task_first = store
        .prepare_history_promotion("repo", &task_first_retry)
        .expect("task-first publication interruption must remain replayable");
    assert_eq!(task_first["replayed"], true);
    assert_eq!(task_first["entries"][0]["task_id"], task_id);

    let mut mismatched_retry = exact_retry.clone();
    mismatched_retry["entries"][0]["expected_remote_task_id"] = json!("RCT-9999");
    let mismatch = store
        .prepare_history_promotion("repo", &mismatched_retry)
        .expect_err("replayed mapping mismatch must fail closed");
    assert!(
        mismatch.contains("HISTORY_PROMOTION_RECEIPT_CONFLICT"),
        "{mismatch}"
    );

    let mut orphan_change = exact_retry;
    orphan_change["entries"][0]["expected_remote_task_id"] = JsonValue::Null;
    let orphan_error = store
        .prepare_history_promotion("repo", &orphan_change)
        .expect_err("expected Change without its Task must fail");
    assert!(
        orphan_error.contains("expected_remote_change_ref requires expected_remote_task_id"),
        "{orphan_error}"
    );

    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .expect("Task count after replay validation"),
        1
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .expect("Change count after replay validation"),
        1
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .expect("Patchset count after replay validation"),
        2
    );
}

#[test]
fn history_promotion_rejects_a_fully_retired_exact_receipt_without_replacement() {
    let db = initialized_db("history-promotion-retired-receipt-owner");
    let snapshot_ids = seed_history_content(&db, 1);
    let item_ref = "HISTORY-RETIRED-RECEIPT-01";
    let plan = BinaryDbServerPlanService::new(db.clone())
        .create_plan(
            "repo",
            &atomic_plan_payload("Retired receipt rejection", item_ref),
        )
        .expect("create retired receipt Plan");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let mut request = history_promotion_request(&snapshot_ids);
    request["idempotency_key"] = json!("history-promotion:retired-receipt-first");
    request["entries"][0]["task"]["plan_id"] = plan["plan_id"].clone();
    request["entries"][0]["task"]["origin_plan_revision_id"] = plan["head_revision_id"].clone();
    request["entries"][0]["task"]["plan_item_ref"] = json!(item_ref);

    let first = store
        .prepare_history_promotion("repo", &request)
        .expect("prepare first receipt owner");
    let old_task_id = first["entries"][0]["task_id"]
        .as_str()
        .expect("old Task id")
        .to_string();
    let old_change_ref = first["entries"][0]["change_ref"]
        .as_str()
        .expect("old Change ref")
        .to_string();
    let old_receipt_patchset_id = first["entries"][0]["receipt_patchset_id"]
        .as_str()
        .expect("old receipt Patchset id")
        .to_string();
    store
        .close_task(&old_task_id, &json!({"status": "abandoned"}))
        .expect("cancel old receipt Task");

    let mut incomplete_retirement = request.clone();
    incomplete_retirement["idempotency_key"] =
        json!("history-promotion:retired-receipt-incomplete");
    let incomplete_error = store
        .prepare_history_promotion("repo", &incomplete_retirement)
        .expect_err("active Change under canceled Task must not be replaced");
    assert!(
        incomplete_error.contains("no longer has reusable Task/Change ownership"),
        "{incomplete_error}"
    );

    store
        .close_change(&old_change_ref, &json!({"status": "archived"}))
        .expect("archive old receipt Change");
    let mut expected_retry = request.clone();
    expected_retry["idempotency_key"] = json!("history-promotion:retired-receipt-expected");
    expected_retry["entries"][0]["expected_remote_task_id"] = json!(old_task_id);
    expected_retry["entries"][0]["expected_remote_change_ref"] = json!(old_change_ref);
    let expected_error = store
        .prepare_history_promotion("repo", &expected_retry)
        .expect_err("retired expected owner must never be replaced");
    assert!(
        expected_error.contains("no longer has reusable Task/Change ownership"),
        "{expected_error}"
    );

    let mut unbound_retry = request;
    unbound_retry["idempotency_key"] = json!("history-promotion:retired-receipt-unbound");
    let unbound_error = store
        .prepare_history_promotion("repo", &unbound_retry)
        .expect_err("retired receipt must block even an unbound replacement request");
    assert!(
        unbound_error.contains("no longer has reusable Task/Change ownership"),
        "{unbound_error}"
    );

    assert_eq!(
        store
            .get_task(None, &old_task_id)
            .expect("read retired receipt Task")["status"],
        "abandoned"
    );
    assert_eq!(
        store
            .get_change(None, &old_change_ref)
            .expect("read retired receipt Change")["status"],
        "archived"
    );
    assert_eq!(
        store
            .get_patchset(None, &old_receipt_patchset_id)
            .expect("read immutable retired receipt Patchset")["source_kind"],
        "imported_local_land_receipt"
    );
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .expect("Task count after rejected replacement"),
        1
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .expect("Change count after rejected replacement"),
        1
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .expect("Patchset count after rejected replacement"),
        2
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::snapshot_link_file())
            .expect("Snapshot Link count after rejected replacement"),
        1
    );
}

#[test]
fn history_promotion_rejects_a_missing_expected_publication_owner_before_append() {
    let db = initialized_db("history-promotion-missing-expected-owner");
    let snapshot_ids = seed_history_content(&db, 1);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let mut request = history_promotion_request(&snapshot_ids);
    request["idempotency_key"] = json!("history-promotion:missing-expected-owner");
    request["entries"][0]["expected_remote_task_id"] = json!("RCT-1412");

    let error = store
        .prepare_history_promotion("repo", &request)
        .expect_err("missing expected receipt owner must fail before allocation");
    assert!(error.contains("has no exact reusable receipt"), "{error}");

    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .expect("Task count after missing expected owner"),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .expect("Change count after missing expected owner"),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .expect("Patchset count after missing expected owner"),
        0
    );
}

#[test]
fn history_promotion_reuses_receipts_when_an_unlanded_chain_is_extended() {
    let db = initialized_db("history-promotion-extended-retry");
    let snapshot_ids = seed_history_content(&db, 2);
    let plan_service = BinaryDbServerPlanService::new(db.clone());
    let plans = (0..2)
        .map(|ordinal| {
            let item_ref = format!("HISTORY-EXTENDED-{:02}", ordinal + 1);
            let mut payload =
                atomic_plan_payload(&format!("Extended History {}", ordinal + 1), &item_ref);
            payload["artifact_path"] = json!(format!(
                "docs/sprints/history-extended-{:02}.md",
                ordinal + 1
            ));
            let plan = plan_service
                .create_plan("repo", &payload)
                .expect("create extended history Plan");
            (plan, item_ref)
        })
        .collect::<Vec<_>>();
    let bind_entries = |request: &mut JsonValue, count: usize| {
        for (entry, plan) in request["entries"]
            .as_array_mut()
            .expect("history entries")
            .iter_mut()
            .zip(plans.iter())
            .take(count)
        {
            entry["task"]["plan_id"] = plan.0["plan_id"].clone();
            entry["task"]["origin_plan_revision_id"] = plan.0["head_revision_id"].clone();
            entry["task"]["plan_item_ref"] = json!(plan.1);
        }
    };
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());

    let mut first_request = history_promotion_request(&snapshot_ids[..2]);
    first_request["idempotency_key"] = json!("history-promotion:extended-first");
    bind_entries(&mut first_request, 1);
    let first = store
        .prepare_history_promotion("repo", &first_request)
        .expect("prepare first local history");
    let first_entry = first["entries"][0].clone();

    let mut extended_request = history_promotion_request(&snapshot_ids);
    extended_request["idempotency_key"] = json!("history-promotion:extended-second");
    extended_request["summary"] = json!("Extend the unlanded local history");
    bind_entries(&mut extended_request, 2);
    let extended = store
        .prepare_history_promotion("repo", &extended_request)
        .expect("reuse first receipt and append second local history");
    assert_eq!(extended["replayed"], false);
    assert_eq!(extended["entries"].as_array().unwrap().len(), 2);
    for field in [
        "task_id",
        "change_ref",
        "receipt_patchset_id",
        "task_index",
        "change_index",
        "receipt_patchset_index",
    ] {
        assert_eq!(
            extended["entries"][0][field], first_entry[field],
            "extended history must reuse {field}"
        );
    }
    assert_ne!(
        extended["entries"][1]["task_id"],
        extended["entries"][0]["task_id"]
    );
    assert_eq!(
        extended["aggregate"]["change_ref"],
        extended["entries"][1]["change_ref"]
    );

    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .expect("Task count"),
        2
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .expect("Change count"),
        2
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .expect("Patchset count"),
        4
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::snapshot_link_file())
            .expect("Snapshot Link count"),
        2
    );
    drop(read);

    let mut conflicting = extended_request.clone();
    conflicting["idempotency_key"] = json!("history-promotion:extended-conflict");
    conflicting["entries"][0]["task"]["title"] = json!("Divergent source Task");
    let error = store
        .prepare_history_promotion("repo", &conflicting)
        .expect_err("divergent already-published history must fail closed");
    assert!(
        error.contains("HISTORY_PROMOTION_RECEIPT_CONFLICT"),
        "{error}"
    );
}

#[test]
fn history_promotion_rejects_an_incomplete_snapshot_difference_and_rolls_back() {
    let db = initialized_db("history-promotion-incomplete-snapshot-difference");
    let snapshot_ids = seed_history_content(&db, 3);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let mut request = history_promotion_request(&snapshot_ids);
    request["idempotency_key"] =
        JsonValue::String("history-promotion:incomplete-difference".to_string());
    request["revision_snapshot_id"] = JsonValue::String(snapshot_ids[3].clone());
    let first = request["entries"][0].clone();
    let mut second = request["entries"][1].clone();
    second["landed_snapshot_id"] = JsonValue::String(snapshot_ids[3].clone());
    second["snapshots"] = json!([{
        "snapshot_id": snapshot_ids[3],
        "created_at_s": 4
    }]);
    request["entries"] = JsonValue::Array(vec![first, second]);

    let error = store
        .prepare_history_promotion("repo", &request)
        .expect_err("an omitted intermediate Snapshot must fail closed");
    assert!(
        error.contains("complete Snapshot DAG difference"),
        "{error}"
    );
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .expect("Task count"),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .expect("Change count"),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::snapshot_link_file())
            .expect("Snapshot Link count"),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .expect("Patchset count"),
        0
    );
}

#[test]
fn history_promotion_rejects_invalid_source_identity_and_time_before_writing() {
    let db = initialized_db("history-promotion-invalid-source-identity");
    let snapshot_ids = seed_history_content(&db, 1);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let mut request = history_promotion_request(&snapshot_ids);
    request["entries"][0]["local_change_ref"] = json!("LCT-OTHER/C-01");

    let identity_error = store
        .prepare_history_promotion("repo", &request)
        .expect_err("inconsistent exact source identity must fail");
    assert!(
        identity_error.contains("exact source identity"),
        "{identity_error}"
    );

    request["entries"][0]["local_change_ref"] = json!("LCT-0001/C-01");
    request["entries"][0]["landed_at_s"] = json!(0);
    let time_error = store
        .prepare_history_promotion("repo", &request)
        .expect_err("zero local Land time must fail");
    assert!(time_error.contains("event time"), "{time_error}");

    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .expect("Task count"),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .expect("Change count"),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .expect("Patchset count"),
        0
    );
}

#[test]
fn atomic_task_land_rejects_a_diverged_target_line_without_mutation() {
    let db = initialized_db("atomic-task-land-diverged-line");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    prepare_ready_atomic_task_land(&store, "SNP-0000000000A3");
    let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    lines
        .set_line_head("main", 2, 3)
        .expect("advance main away from the selected Patchset base");

    let error = store
        .submit_task_land("T-0001", &atomic_task_land_request("T-0001"))
        .expect_err("diverged target Line must fail closed");
    assert!(error.contains("TASK_LAND_TARGET_LINE_BLOCKED"), "{error}");
    assert_eq!(
        store.get_task(None, "T-0001").expect("read Task")["status"],
        "active"
    );
    assert_eq!(
        store.get_change(None, "T-0001/C-01").expect("read Change")["status"],
        "active"
    );
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .expect("Land count"),
        0
    );
    let read = BinaryDbReadTxn::new(&db);
    let (_, line) = lines
        .line_by_name(&read, "main")
        .expect("read main Line")
        .expect("main Line exists");
    assert_eq!(line.head_snapshot_index_plus1, 2);
}

#[test]
fn atomic_task_land_rolls_back_land_change_and_line_when_task_completion_fails() {
    let db = initialized_db("atomic-task-land-terminal-task");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    prepare_ready_atomic_task_land(&store, "SNP-0000000000A2");
    store
        .close_task("T-0001", &json!({"status": "abandoned"}))
        .expect("abandon Task before Land");

    let error = store
        .submit_task_land("T-0001/C-01", &atomic_task_land_request("T-0001/C-01"))
        .expect_err("terminal Task must roll back the atomic Land transaction");
    assert!(error.contains("already terminal"), "{error}");
    assert_eq!(
        store.get_task(None, "T-0001").expect("read Task")["status"],
        "abandoned"
    );
    assert_eq!(
        store.get_change(None, "T-0001/C-01").expect("read Change")["status"],
        "active"
    );
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .expect("Land count"),
        0
    );
    let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    let read = BinaryDbReadTxn::new(&db);
    let (_, line) = lines
        .line_by_name(&read, "main")
        .expect("read main Line")
        .expect("main Line exists");
    assert_eq!(line.head_snapshot_index_plus1, 1);
}

#[test]
fn atomic_task_land_rejects_non_ready_and_ambiguous_task_without_mutation() {
    let db = initialized_db("atomic-task-land-preconditions");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .create_task(
            "repo",
            &json!({
                "title": "Atomic Task Land preconditions",
                "intent": "Reject incomplete closeout"
            }),
        )
        .expect("create Task");
    store
        .create_change(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "First Change",
                "base_line": "main"
            }),
        )
        .expect("create first Change");
    store
        .publish_patchset(
            "T-0001/C-01",
            &json!({
                "base_snapshot_id": "SNP-0000000000A1",
                "revision_snapshot_id": "SNP-0000000000A2",
                "summary": "not ready",
                "author_mode": "ai_with_human_review"
            }),
        )
        .expect("publish non-ready Patchset");

    let not_ready = store
        .submit_task_land("T-0001/C-01", &atomic_task_land_request("T-0001/C-01"))
        .expect_err("non-ready Task Land must fail");
    assert!(not_ready.contains("TASK_LAND_NOT_READY"), "{not_ready}");
    assert_eq!(
        store
            .get_task(None, "T-0001")
            .expect("read Task after refusal")["status"],
        "active"
    );
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .expect("Land count after refusal"),
        0
    );

    store
        .create_change(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Second Change",
                "base_line": "main"
            }),
        )
        .expect("create second Change");
    let ambiguous = store
        .resolve_task_land_change_ref("T-0001")
        .expect_err("ambiguous Task must require exact Change");
    assert!(
        ambiguous.contains("multiple landable Changes"),
        "{ambiguous}"
    );
}

#[test]
fn runtime_writes_keep_patch_review_policy_and_land_ordinals_in_v0_scope() {
    let db = initialized_db("ordinal-scopes");
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());

    let task = store
        .create_task(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Wire Binary DB v0",
                "intent": "Exercise exact v0 runtime scopes"
            }),
        )
        .expect("create Task");
    assert_eq!(task["task_id"], "T-0001");
    let change = store
        .create_change(
            "repo",
            &json!({
                "change_id": "C-01",
                "task_id": "T-0001",
                "title": "Exact ordinal scopes",
                "base_line": "main"
            }),
        )
        .expect("create Change");
    assert_eq!(change["change_ref"], "T-0001/C-01");

    let patchset_one = store
        .publish_patchset(
            "T-0001/C-01",
            &json!({
                "base_snapshot_id": "SNP-0000000000A1",
                "revision_snapshot_id": "SNP-0000000000A2",
                "summary": "first Patchset",
                "author_mode": "ai_with_human_review"
            }),
        )
        .expect("publish first Patchset");
    assert_eq!(patchset_one["patchset_id"], "T-0001/C-01/P-01");
    let review_one = store
        .record_review(
            "T-0001/C-01",
            &json!({
                "patchset_id": "T-0001/C-01/P-01",
                "reviewer": "owner",
                "action": "approve"
            }),
        )
        .expect("approve first Patchset");
    assert_eq!(review_one["review_id"], "T-0001/C-01/P-01/R-01");
    store
        .put_attestation(
            "T-0001/C-01/P-01",
            &json!({
                "verification_state": "pass",
                "require_tests_pass": false
            }),
        )
        .expect("attest first Patchset");
    let policy_one = store
        .evaluate_policy("T-0001/C-01/P-01")
        .expect("evaluate first Patchset");
    assert_eq!(policy_one["policy_decision_id"], "T-0001/C-01/P-01/K-01");
    assert_eq!(policy_one["decision"], "pass");

    let patchset_two = store
        .publish_patchset(
            "T-0001/C-01",
            &json!({
                "base_snapshot_id": "SNP-0000000000A1",
                "revision_snapshot_id": "SNP-0000000000A3",
                "summary": "second Patchset",
                "author_mode": "ai_with_human_review"
            }),
        )
        .expect("publish second Patchset");
    assert_eq!(patchset_two["patchset_id"], "T-0001/C-01/P-02");
    store
        .select_patchset("T-0001/C-01", &json!({"patchset_id": "T-0001/C-01/P-02"}))
        .expect("select second Patchset");
    let review_two = store
        .record_review(
            "T-0001/C-01",
            &json!({
                "patchset_id": "T-0001/C-01/P-02",
                "reviewer": "owner",
                "action": "approve",
                "comment": "second approval"
            }),
        )
        .expect("approve second Patchset");
    assert_eq!(review_two["review_id"], "T-0001/C-01/P-02/R-01");
    let ci = store
        .run_patchset_ci("T-0001/C-01/P-02", &json!({}))
        .expect("start Patchset CI");
    assert_eq!(ci["ci_run_seq"], 1);
    store
        .complete_patchset_ci(
            "T-0001/C-01/P-02",
            &json!({
                "patchset_id": "T-0001/C-01/P-02",
                "ci_run_seq": 1,
                "selected_suite_count": 2,
                "suite_result_count": 2,
                "blocking_failure_count": 0,
                "overall_status": "pass",
                "tests_status": "pass",
                "lint_status": "pass"
            }),
        )
        .expect("complete Patchset CI");
    let completed_patchset = store
        .get_patchset(None, "T-0001/C-01/P-02")
        .expect("read completed Patchset CI projection");
    assert!(completed_patchset["ci_completed_at_s"]
        .as_u64()
        .is_some_and(|value| value > 0));
    assert!(completed_patchset["ci"]["completed_at"].is_string());
    store
        .put_attestation(
            "T-0001/C-01/P-02",
            &json!({
                "verification_state": "pass",
                "require_tests_pass": true,
                "require_human_review": true,
                "require_lint_pass": true
            }),
        )
        .expect("attest second Patchset");
    let policy_two = store
        .evaluate_policy("T-0001/C-01/P-02")
        .expect("evaluate second Patchset");
    assert_eq!(policy_two["policy_decision_id"], "T-0001/C-01/P-02/K-01");
    assert_eq!(policy_two["decision"], "pass");

    let land = store
        .submit_land(
            "T-0001/C-01",
            &json!({
                "submission_id": "T-0001/C-01/L-01",
                "patchset_id": "T-0001/C-01/P-02",
                "target_line": "main",
                "expected_head_snapshot_id": "SNP-0000000000A1",
                "mode": "direct"
            }),
        )
        .expect("land selected Patchset");
    assert_eq!(land["submission_id"], "T-0001/C-01/L-01");
    assert_eq!(land["status"], "succeeded");

    validate_frozen_server_workflow_v0(&db).expect("validate exact frozen v0 authority");
    let read = BinaryDbReadTxn::new(&db);
    let patchsets = (0..2)
        .map(|index| {
            WorkflowBinaryV0Codec::decode_frozen_patchset(
                &read
                    .read_record(WorkflowBinaryV0Codec::patchset_file(), index)
                    .expect("read Patchset"),
            )
            .expect("decode Patchset")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        patchsets
            .iter()
            .map(|record| (record.change_index, record.patch_ordinal))
            .collect::<Vec<_>>(),
        vec![(0, 0), (0, 1)]
    );
    let reviews = (0..2)
        .map(|index| {
            WorkflowBinaryV0Codec::decode_review(
                &read
                    .read_record(WorkflowBinaryV0Codec::review_file(), index)
                    .expect("read Review"),
            )
            .expect("decode Review")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reviews
            .iter()
            .map(|record| (record.patchset_index, record.review_ordinal))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 0)]
    );
    let policies = (0..2)
        .map(|index| {
            WorkflowBinaryV0Codec::decode_policy(
                &read
                    .read_record(WorkflowBinaryV0Codec::policy_file(), index)
                    .expect("read Policy"),
            )
            .expect("decode Policy")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        policies
            .iter()
            .map(|record| (record.patchset_index, record.policy_ordinal))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 0)]
    );
    let land_record = WorkflowBinaryV0Codec::decode_land(
        &read
            .read_record(WorkflowBinaryV0Codec::land_file(), 0)
            .expect("read Land"),
    )
    .expect("decode Land");
    assert_eq!(land_record.change_index, 0);
    assert_eq!(land_record.patchset_index, 1);
    assert_eq!(land_record.land_ordinal, 0);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::actor_file())
            .expect("Actor count"),
        1
    );
}

#[test]
fn historical_policy_suite_catalog_is_independent_from_patchset_compact_ci_count() {
    for (label, selected_suite_count) in [("zero-ci", 0_u16), ("three-ci", 3_u16)] {
        let db = initialized_db(label);
        seed_content(&db);
        let store = BinaryDbServerWorkflowV0Store::new(db.clone());

        store
            .create_task(
                "repo",
                &json!({
                    "task_id": "T-0001",
                    "title": "Preserve historical Policy catalog",
                    "intent": "Validate independent Policy and compact CI projections"
                }),
            )
            .expect("create Task");
        store
            .create_change(
                "repo",
                &json!({
                    "change_id": "C-01",
                    "task_id": "T-0001",
                    "title": "Independent historical Policy catalog",
                    "base_line": "main"
                }),
            )
            .expect("create Change");
        store
            .publish_patchset(
                "T-0001/C-01",
                &json!({
                    "base_snapshot_id": "SNP-0000000000A1",
                    "revision_snapshot_id": "SNP-0000000000A2",
                    "summary": "historical Policy catalog fixture",
                    "author_mode": "ai_with_human_review"
                }),
            )
            .expect("publish Patchset");
        if selected_suite_count != 0 {
            let ci = store
                .run_patchset_ci("T-0001/C-01/P-01", &json!({}))
                .expect("start Patchset CI");
            store
                .complete_patchset_ci(
                    "T-0001/C-01/P-01",
                    &json!({
                        "patchset_id": "T-0001/C-01/P-01",
                        "ci_run_seq": ci["ci_run_seq"],
                        "selected_suite_count": selected_suite_count,
                        "suite_result_count": selected_suite_count,
                        "blocking_failure_count": 0,
                        "overall_status": "pass",
                        "tests_status": "pass",
                        "lint_status": "pass"
                    }),
                )
                .expect("complete Patchset CI");
        }
        store
            .put_attestation(
                "T-0001/C-01/P-01",
                &json!({
                    "verification_state": "pass",
                    "require_tests_pass": selected_suite_count != 0
                }),
            )
            .expect("attest Patchset");
        store
            .evaluate_policy("T-0001/C-01/P-01")
            .expect("evaluate Policy");

        let read = BinaryDbReadTxn::new(&db);
        let patchset = WorkflowBinaryV0Codec::decode_patchset(
            &read
                .read_record(WorkflowBinaryV0Codec::patchset_file(), 0)
                .expect("read Patchset"),
        )
        .expect("decode Patchset");
        assert_eq!(patchset.ci_selected_suite_count, selected_suite_count);
        drop(read);

        let historical_suite_check = V0PolicyCheckRecord {
            check_kind: 9,
            check_status: 3,
            subject_ordinal: 4,
            detail_flags: 2,
        };
        let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)
            .expect("begin Policy fixture rewrite");
        write
            .overwrite_record(
                WorkflowBinaryV0Codec::policy_check_file(),
                0,
                &WorkflowBinaryV0Codec::encode_policy_check(historical_suite_check)
                    .expect("encode historical suite Check"),
            )
            .expect("rewrite one Policy Check");
        write.commit().expect("commit Policy fixture rewrite");

        validate_server_workflow_v0(&db)
            .expect("historical Policy suite ordinal is not bounded by compact CI count");
    }
}
