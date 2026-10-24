use super::*;
use crate::foundation::remote_binary_db::FilesystemServerRemoteBinaryDb;
use crate::foundation::server_content_binary_db::ServerBinarySnapshotPayload;
use crate::foundation::workflow_binary_v0_adapter::tests::initialized_db;

type Db = FilesystemServerRemoteBinaryDb;
const BASE: &str = "SNP-000000000001";
const ORIGINAL: &str = "SNP-000000000002";
const TESTED: &str = "SNP-00000000FFFF";
struct Fixture {
    db: Db,
    store: BinaryDbServerWorkflowV0Store<Db>,
    manifest: WorkflowPackageManifest,
    identity: PackageExpectedIdentity,
}
impl Fixture {
    fn new(label: &str) -> Self {
        let db = initialized_db(label);
        let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "CP")
            .unwrap()
            .with_repository_index(0);
        ServerBinaryDbLineStore::<_, 1>::new(db.clone())
            .create_line("main", 0, 1)
            .unwrap();
        append_snapshot(&db, BASE, None, 1);
        append_snapshot(&db, ORIGINAL, Some(0), 2);
        ServerBinaryDbLineStore::<_, 1>::new(db.clone())
            .set_line_head("main", 1, 2)
            .unwrap();
        store
            .create_task(
                "repo",
                &json!({"actor_key_index":0,"title":"Source","intent":"Preserve history"}),
            )
            .unwrap();
        let owner = WorkflowOwner {
            task_index: 0,
            change: None,
        };
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
        store
            .append_numeric_snapshot_link(&mut tx, owner, 1, 0)
            .unwrap();
        let receipt = json!({"contract":"ait-local-land-receipt/v2","local_task_id":"LCT-0001","local_change_id":null,"local_change_ref":null,"task_id":"RCPT-0001","change_ref":null,"target_line":"main","pre_land_target_snapshot_id":BASE,"landed_snapshot_id":ORIGINAL,"landed_at_s":100,"source_kind":"imported_local_land_receipt"});
        store
            .append_history_patchset_in_write(
                &mut tx,
                owner,
                0,
                1,
                LOCAL_LAND_RECEIPT_SUMMARY_PREFIX,
                &receipt,
                "ai_only_experimental",
                100,
                0,
            )
            .unwrap();
        tx.commit().unwrap();
        drop(tx);
        let manifest = WorkflowPackageManifest::parse(&serde_json::to_vec(&json!({"contract":"workflow-package/v1","repository_index":0,"target_line":"main","base_snapshot_id":BASE,"revision_snapshot_id":ORIGINAL,"author_mode":"ai_only_experimental","history_entries":[{"local_task_id":"LCT-0001","expected_remote_task_id":"RCPT-0001","task":{"title":"Source","intent":"Preserve history"},"pre_land_target_snapshot_id":BASE,"landed_snapshot_id":ORIGINAL,"landed_at_s":100,"changes":[],"snapshots":[{"snapshot_id":ORIGINAL,"created_at_s":2,"local_change_ref":null}]}],"content":{"object_packs":[],"tree_packs":[],"blob_locators":[],"tree_locators":[],"snapshots":[],"line_update":null,"capture_binding":null}})).unwrap()).unwrap();
        let identity = PackageExpectedIdentity {
            repository_index: 0,
            package_hash: "a".repeat(64),
            worker_job_index: 0,
            base_snapshot_id: BASE.into(),
            revision_snapshot_id: ORIGINAL.into(),
        };
        Self {
            db,
            store,
            manifest,
            identity,
        }
    }
    fn preflight(&self) -> Result<usize, String> {
        self.store
            .preflight_workflow_package_history(&self.manifest, &self.identity)
    }
    fn append_candidate(&self) -> String {
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .unwrap();
        let index = self
            .store
            .append_candidate_in_write(
                &mut tx,
                WorkflowOwner {
                    task_index: 0,
                    change: None,
                },
                0,
                1,
                "Ordinary candidate",
                "ai_only_experimental",
                110,
                1,
            )
            .unwrap();
        let id = self
            .store
            .patchset_public_id(self.store.read_patchset(&tx, index).unwrap());
        tx.commit().unwrap();
        id
    }
    fn add_tested(&mut self, change: bool) {
        append_snapshot(&self.db, TESTED, Some(1), 3);
        self.manifest.revision_snapshot_id = TESTED.into();
        self.identity.revision_snapshot_id = TESTED.into();
        self.manifest.history_entries[0].snapshots.push(
            crate::foundation::workflow_package::PackageHistorySnapshot {
                snapshot_id: TESTED.into(),
                created_at_s: 3,
                local_change_ref: change.then(|| "LCT-0001/C-01".into()),
            },
        );
        if change {
            self.manifest.history_entries[0].changes.push(crate::foundation::workflow_package::PackageHistoryChange { local_change_id:"C-01".into(), local_change_ref:"LCT-0001/C-01".into(), expected_remote_change_ref:None, change:json!({"title":"New historical revision","base_line":"feature/source","fork_snapshot_id":ORIGINAL}).as_object().unwrap().clone() });
        }
    }
}
fn append_snapshot(db: &Db, id: &str, parent: Option<u32>, time: u64) -> u32 {
    let record = ServerBinarySnapshotRecord {
        actor_index_plus1: 0,
        snapshot_meta: 0,
        history_flags: 0,
        payload_len: 0,
        payload_offset: 0,
        snapshot_hash48: server_snapshot_hash48_from_id(id).unwrap(),
        parent_snapshot_index_plus1: parent.map_or(0, |i| i + 1),
        root_tree_pack_index_plus1: 0,
        root_entry_ordinal: 0,
        line_index_plus1: 0,
        manifest_hash: [0; 32],
        file_count: 0,
        total_bytes: 0,
        created_at_s: time,
    };
    ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone())
        .append_snapshot(
            id,
            record,
            &ServerBinarySnapshotPayload {
                line_name: "feature/source".into(),
                message: None,
            },
        )
        .unwrap()
}

#[test]
fn preflight_rejects_missing_and_tombstoned_creation_forks_without_writes() {
    let mut f = Fixture::new("package-creation-fork-admission");
    f.add_tested(true);
    let tombstone = "SNP-000000000003";
    let read = BinaryDbReadTxn::new(&f.db);
    let mut record = ServerBinarySnapshotCodec::<2>::decode_record(
        &read
            .read_record(ServerBinarySnapshotCodec::<2>::record_file(), 1)
            .unwrap(),
    )
    .unwrap();
    drop(read);
    record.snapshot_hash48 = server_snapshot_hash48_from_id(tombstone).unwrap();
    record.snapshot_meta |= ServerBinarySnapshotRecord::META_TOMBSTONE;
    ServerBinaryDbSnapshotStore::<_, 1>::new(f.db.clone())
        .append_snapshot(
            tombstone,
            record,
            &ServerBinarySnapshotPayload {
                line_name: "feature/source".into(),
                message: None,
            },
        )
        .unwrap();
    let counts = || {
        let read = BinaryDbReadTxn::new(&f.db);
        [
            WorkflowBinaryV0Codec::task_file(),
            WorkflowBinaryV0Codec::change_file(),
            WorkflowBinaryV0Codec::patchset_file(),
            WorkflowBinaryV0Codec::snapshot_link_file(),
            ServerBinaryLineCodec::<1>::record_file(),
            ServerBinarySnapshotCodec::<2>::record_file(),
        ]
        .into_iter()
        .map(|file| read.record_count(file).unwrap())
        .collect::<Vec<_>>()
    };
    let before = counts();
    for (fork, expected) in [("SNP-00000000EEEE", "missing"), (tombstone, "retired")] {
        f.manifest.history_entries[0].changes[0]
            .change
            .insert("fork_snapshot_id".into(), json!(fork));
        assert!(f.preflight().unwrap_err().contains(expected));
        assert_eq!(counts(), before);
    }
}

#[test]
fn preflight_counts_existing_and_final_patchsets_in_the_task_scope() {
    let f = Fixture::new("package-budget-patchset");
    for _ in 0..62 {
        f.append_candidate();
    }
    assert_eq!(f.preflight().unwrap(), 64);
    f.append_candidate();
    assert!(f.preflight().unwrap_err().contains("Patchset64/Task"));
}
#[test]
fn preflight_counts_existing_attestations_independently_and_before_ci() {
    let f = Fixture::new("package-budget-attestation");
    let patchset = f.append_candidate();
    let payload = json!({"author_mode":"ai_only_experimental","verification_state":"pass","require_tests_pass":false});
    for _ in 0..63 {
        f.store.put_attestation(&patchset, &payload).unwrap();
    }
    assert_eq!(f.preflight().unwrap(), 1);
    assert_eq!(
        f.store.package_available_attestations(&f.manifest).unwrap(),
        1
    );
    f.store.put_attestation(&patchset, &payload).unwrap();
    assert_eq!(
        f.store.package_available_attestations(&f.manifest).unwrap(),
        0
    );
    assert!(f.preflight().unwrap_err().contains("Attestation64/Task"));
}
#[test]
fn preflight_counts_existing_changes_instead_of_reusing_local_ordinals() {
    let mut f = Fixture::new("package-budget-change");
    f.add_tested(true);
    let payload = json!({"actor_key_index":0,"task_id":"RCPT-0001","title":"Existing revision","base_line":"main","fork_snapshot_id":ORIGINAL});
    for _ in 0..63 {
        f.store.create_change("repo", &payload).unwrap();
    }
    assert_eq!(f.preflight().unwrap(), 64);
    f.store.create_change("repo", &payload).unwrap();
    assert!(f.preflight().unwrap_err().contains("Change64/Task"));
}
#[test]
fn preflight_counts_all_256_existing_owner_links_including_prior_replays() {
    let mut f = Fixture::new("package-budget-snapshot");
    f.add_tested(false);
    for i in 0..255u32 {
        let id = format!("SNP-{:012X}", 0x100 + i);
        let index = append_snapshot(&f.db, &id, Some(1), u64::from(i) + 4);
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&f.db, BinaryDbCommandScope::ServerSnapshotCapture)
                .unwrap();
        f.store
            .append_numeric_snapshot_link(
                &mut tx,
                WorkflowOwner {
                    task_index: 0,
                    change: None,
                },
                index,
                0,
            )
            .unwrap();
        tx.commit().unwrap();
        if i == 253 {
            assert_eq!(f.preflight().unwrap(), 64);
        }
    }
    assert!(f
        .preflight()
        .unwrap_err()
        .contains("Snapshot-owner256/Task"));
}
#[test]
fn preflight_counts_tombstoned_land_slots_before_ci() {
    let f = Fixture::new("package-budget-land");
    f.append_candidate();
    for ordinal in 0..64u8 {
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&f.db, BinaryDbCommandScope::ServerWorkflow).unwrap();
        let land = V0LandRecord {
            land_meta: LAND_STATUS_FAILED | LAND_TOMBSTONE,
            land_ordinal: ordinal,
            failure_kind: 1,
            task_index: 0,
            patchset_index: 1,
            previous_task_land_index_plus1: u32::from(ordinal),
            submitted_at_s: 120,
            updated_at_s: 120,
            target_line_index_plus1: 1,
            actor_index_plus1: 1,
            ..Default::default()
        };
        tx.append_record(
            WorkflowBinaryV0Codec::land_file(),
            &WorkflowBinaryV0Codec::encode_land(land).unwrap(),
        )
        .unwrap();
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_land_index.bin"),
            0,
            &WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord {
                latest_index_plus1: u32::from(ordinal) + 1,
                count: u16::from(ordinal) + 1,
                reserved0: 0,
            })
            .unwrap(),
        )
        .unwrap();
        tx.commit().unwrap();
        if ordinal == 62 {
            assert_eq!(f.preflight().unwrap(), 64);
        }
    }
    assert!(f.preflight().unwrap_err().contains("Land64/Task"));
}

#[test]
fn preflight_accepts_64_tasks_with_shared_source_names_and_rejects_65_without_reservations() {
    let mut f = Fixture::new("package-budget-task");
    let mut parent = 1;
    let mut previous = ORIGINAL.to_string();
    for number in 2..=65 {
        let id = format!("SNP-{:012X}", 0x100 + number);
        let index = append_snapshot(&f.db, &id, Some(parent), number);
        let task = format!("LCT-{number:04}");
        let entry = serde_json::from_value(json!({"local_task_id":task,"expected_remote_task_id":null,"task":{"title":format!("Source {number}"),"intent":"Preserve history"},"pre_land_target_snapshot_id":previous,"landed_snapshot_id":id,"landed_at_s":100+number,"changes":[{"local_change_id":"C-01","local_change_ref":format!("{task}/C-01"),"expected_remote_change_ref":null,"change":{"title":"Historical revision","base_line":"shared/local/base","fork_snapshot_id":previous}}],"snapshots":[{"snapshot_id":id,"created_at_s":number,"local_change_ref":format!("{task}/C-01")}]})).unwrap();
        f.manifest.history_entries.push(entry);
        f.manifest.revision_snapshot_id = id.clone();
        f.identity.revision_snapshot_id = id.clone();
        previous = id;
        parent = index;
        if number == 64 {
            assert_eq!(f.preflight().unwrap(), 64);
        }
    }
    assert!(f.preflight().unwrap_err().contains("64 source Tasks"));
    let read = BinaryDbReadTxn::new(&f.db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .unwrap(),
        1
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .unwrap(),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .unwrap(),
        1
    );
    assert_eq!(
        read.record_count(ServerBinaryLineCodec::<1>::record_file())
            .unwrap(),
        1
    );
}
