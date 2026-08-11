use super::*;
use crate::binary_db::{
    binary_db_plan_golden_checksum, AuthorityId, BinaryDb, BinaryDbCommandScope, BinaryDbErrorKind,
    BinaryDbNoopFsyncPolicy, BinaryFileId, BinaryIndexId, BinaryIndexKeyRef, BinaryPayloadFileId,
    BinaryRecordBytes, BinaryRecordBytesRef, BinaryWriteContext, LocalBinaryDbFs, LocalStateScope,
    PayloadRange, RemoteBinaryDb, RemoteBinaryDbFs, RepoId, RepoName, StorePath, StoreResult,
    BINARY_DB_PLAN_GOLDEN_CHECKSUM, BINARY_DB_PLAN_GOLDEN_SOURCE, BINARY_DB_PLAN_GOLDEN_VERSION,
};
use crate::json_support::JsonValue;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use tempfile::tempdir;

const TEST_WRITE_LAYOUT: u32 = PLAN_LAYOUT_ID;
const UNSUPPORTED_TEST_LAYOUT: u32 = PLAN_LAYOUT_ID + 1;
type TestPlanStore = BinaryDbPlanStore<TestBinaryDb, TEST_WRITE_LAYOUT>;
type LocalTestPlanStore = BinaryDbPlanStore<LocalBinaryDbFs, TEST_WRITE_LAYOUT>;
type UnsupportedLocalTestPlanStore = BinaryDbPlanStore<LocalBinaryDbFs, UNSUPPORTED_TEST_LAYOUT>;

#[test]
fn repository_plan_identity_is_the_direct_plan_bin_ordinal() {
    assert_eq!(repository_plan_id(4), "PR-4");
    assert_eq!(parse_repository_plan_id("PR-4"), Ok(4));

    for noncanonical in ["plan:4", "4", "PR-04", "PR-4 ", "PR-"] {
        assert!(
            parse_repository_plan_id(noncanonical).is_err(),
            "{noncanonical} must not introduce an alternate Plan identity"
        );
    }
}

fn new_local_store() -> (LocalTestPlanStore, tempfile::TempDir) {
    let temp_dir = tempdir().expect("tempdir");
    let db = LocalBinaryDbFs::new(
        temp_dir.path(),
        temp_dir.path(),
        AuthorityId::new("test-authority"),
        LocalStateScope::Repository,
    );
    (LocalTestPlanStore::new(db), temp_dir)
}

fn new_unsupported_layout_local_store() -> (UnsupportedLocalTestPlanStore, tempfile::TempDir) {
    let temp_dir = tempdir().expect("tempdir");
    let db = LocalBinaryDbFs::new(
        temp_dir.path(),
        temp_dir.path(),
        AuthorityId::new("test-authority"),
        LocalStateScope::Repository,
    );
    (UnsupportedLocalTestPlanStore::new(db), temp_dir)
}

fn seed_plan_read_surface_fixture(store: &LocalTestPlanStore) {
    let mut tx = store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin read surface fixture txn");

    store
        .append_plan(
            &mut tx,
            PlanRecord {
                plan_meta: 0b0000_0100,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                latest_revision_index_plus1: 2,
                published_plan_index_plus1: 11,
                published_latest_revision_index_plus1: 12,
                created_at_s: 100,
                updated_at_s: 200,
                published_at_s: 210,
            },
            &PlanPayload {
                title_bytes: b"Runtime Plan".to_vec(),
            },
        )
        .expect("append runtime plan");
    store
        .append_plan(
            &mut tx,
            PlanRecord {
                plan_meta: 0b0000_0001,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                latest_revision_index_plus1: 3,
                published_plan_index_plus1: 0,
                published_latest_revision_index_plus1: 0,
                created_at_s: 90,
                updated_at_s: 250,
                published_at_s: 0,
            },
            &PlanPayload {
                title_bytes: b"Archived Plan".to_vec(),
            },
        )
        .expect("append archived plan");

    store
        .append_plan_item(
            &mut tx,
            PlanItemRecord {
                item_meta: 0b0000_1101,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                line_number: 7,
            },
            &PlanItemPayload {
                plan_item_ref_bytes: b"runtime/taskable".to_vec(),
                text_bytes: b"Implement fused read".to_vec(),
                heading_path: vec!["Runtime".to_string()],
            },
        )
        .expect("append runtime item");
    store
        .append_plan_item(
            &mut tx,
            PlanItemRecord {
                item_meta: 0b0000_0110,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                line_number: 8,
            },
            &PlanItemPayload {
                plan_item_ref_bytes: b"runtime/done".to_vec(),
                text_bytes: b"Already complete".to_vec(),
                heading_path: vec!["Runtime".to_string()],
            },
        )
        .expect("append done item");
    store
        .append_plan_item(
            &mut tx,
            PlanItemRecord {
                item_meta: 0b0000_1101,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                line_number: 3,
            },
            &PlanItemPayload {
                plan_item_ref_bytes: b"archived/item".to_vec(),
                text_bytes: b"Historical item".to_vec(),
                heading_path: vec!["Archive".to_string()],
            },
        )
        .expect("append archived item");

    store
        .append_plan_revision(
            &mut tx,
            PlanRevisionRecord {
                revision_meta: 0b0000_1000,
                reserved0: 0,
                payload_len: 0,
                revision_number: 1,
                item_count: 1,
                payload_offset: 0,
                plan_index: 0,
                previous_revision_index_plus1: 0,
                item_start_index: 0,
                published_revision_index_plus1: 0,
                root_tree_pack_index_plus1: 2,
                root_entry_ordinal: 5,
                created_at_s: 150,
                published_at_s: 0,
            },
            &PlanRevisionPayload {
                title_snapshot_bytes: b"Runtime Plan".to_vec(),
                summary_bytes: b"initial".to_vec(),
                artifact_path_bytes: b"docs/runtime.md".to_vec(),
                artifact_selector_bytes: b"runtime".to_vec(),
                artifact_heading_bytes: b"Runtime".to_vec(),
                artifact_blob_id_bytes: b"BLB-OLD".to_vec(),
            },
        )
        .expect("append first revision");
    store
        .append_plan_revision(
            &mut tx,
            PlanRevisionRecord {
                revision_meta: 0b0000_1001,
                reserved0: 0,
                payload_len: 0,
                revision_number: 2,
                item_count: 2,
                payload_offset: 0,
                plan_index: 0,
                previous_revision_index_plus1: 1,
                item_start_index: 0,
                published_revision_index_plus1: 22,
                root_tree_pack_index_plus1: 3,
                root_entry_ordinal: 6,
                created_at_s: 220,
                published_at_s: 230,
            },
            &PlanRevisionPayload {
                title_snapshot_bytes: b"Runtime Plan".to_vec(),
                summary_bytes: b"head".to_vec(),
                artifact_path_bytes: b"docs/runtime.md".to_vec(),
                artifact_selector_bytes: b"runtime".to_vec(),
                artifact_heading_bytes: b"Runtime".to_vec(),
                artifact_blob_id_bytes: b"BLB-HEAD".to_vec(),
            },
        )
        .expect("append head revision");
    store
        .append_plan_revision(
            &mut tx,
            PlanRevisionRecord {
                revision_meta: 0b0000_1000,
                reserved0: 0,
                payload_len: 0,
                revision_number: 1,
                item_count: 1,
                payload_offset: 0,
                plan_index: 1,
                previous_revision_index_plus1: 0,
                item_start_index: 2,
                published_revision_index_plus1: 0,
                root_tree_pack_index_plus1: 4,
                root_entry_ordinal: 9,
                created_at_s: 140,
                published_at_s: 0,
            },
            &PlanRevisionPayload {
                title_snapshot_bytes: b"Archived Plan".to_vec(),
                summary_bytes: b"archived".to_vec(),
                artifact_path_bytes: b"docs/archive.md".to_vec(),
                artifact_selector_bytes: Vec::new(),
                artifact_heading_bytes: b"Archive".to_vec(),
                artifact_blob_id_bytes: b"BLB-ARCHIVE".to_vec(),
            },
        )
        .expect("append archived revision");

    tx.commit().expect("commit read surface fixture");
}

fn assert_header_u32_at(path: &std::path::Path, expected: u32) {
    let bytes = fs::read(path).expect("read db file");
    assert!(bytes.len() >= 4);
    assert_eq!(&bytes[0..4], expected.to_le_bytes().as_slice());
}

#[derive(Clone, Debug)]
struct TestBinaryDb {
    authority_root: StorePath,
}

impl crate::binary_db::BinaryDbRecoveryIo for TestBinaryDb {
    fn recovery_truncate_file(&self, _path: &std::path::Path, _len: u64) -> StoreResult<()> {
        Err("test plan file specs do not use recovery writes".into())
    }

    fn recovery_remove_file_if_exists(&self, _path: &std::path::Path) -> StoreResult<()> {
        Err("test plan file specs do not use recovery writes".into())
    }
}

impl BinaryDb for TestBinaryDb {
    fn authority_root(&self) -> &StorePath {
        &self.authority_root
    }

    fn layout_id(&self, _file: BinaryFileId) -> StoreResult<u32> {
        Err("test plan file specs do not use runtime reads".into())
    }

    fn record_count(&self, _file: BinaryFileId) -> StoreResult<u32> {
        Err("test plan file specs do not use runtime reads".into())
    }

    fn read_record(
        &self,
        _file: BinaryFileId,
        _record_index: u32,
    ) -> StoreResult<BinaryRecordBytes> {
        Err("test plan file specs do not use runtime reads".into())
    }

    fn append_record(
        &self,
        _file: BinaryFileId,
        _record: BinaryRecordBytesRef<'_>,
        _write: &mut BinaryWriteContext,
    ) -> StoreResult<u32> {
        Err("test plan file specs do not use writes".into())
    }

    fn read_payload(
        &self,
        _file: BinaryPayloadFileId,
        _offset: u64,
        _len: u32,
    ) -> StoreResult<Vec<u8>> {
        Err("test plan file specs do not use runtime reads".into())
    }

    fn append_payload(
        &self,
        _file: BinaryPayloadFileId,
        _bytes: &[u8],
        _write: &mut BinaryWriteContext,
    ) -> StoreResult<PayloadRange> {
        Err("test plan file specs do not use writes".into())
    }

    fn lookup_index(
        &self,
        _index: BinaryIndexId,
        _key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        Err("test plan file specs do not use lookup indexes".into())
    }
}

#[test]
fn plan_binary_db_file_specs_use_layout_header_ids() {
    let plan_file = TestPlanStore::plan_file();
    assert_eq!(
        plan_file.relative_path().as_path(),
        std::path::Path::new(PLAN_BIN)
    );
    assert_eq!(plan_file.layout_id(), TEST_WRITE_LAYOUT);
    assert_eq!(plan_file.record_size(), PLAN_RECORD_SIZE);

    let payload_file = TestPlanStore::plan_revision_payload_file();
    assert_eq!(
        payload_file.relative_path().as_path(),
        std::path::Path::new(PLAN_REVISION_PAYLOAD_BIN)
    );
    assert_eq!(payload_file.layout_id(), TEST_WRITE_LAYOUT);

    let declared_paths = [
        TestPlanStore::plan_file()
            .relative_path()
            .as_path()
            .to_path_buf(),
        TestPlanStore::plan_payload_file()
            .relative_path()
            .as_path()
            .to_path_buf(),
        TestPlanStore::plan_revision_file()
            .relative_path()
            .as_path()
            .to_path_buf(),
        TestPlanStore::plan_revision_payload_file()
            .relative_path()
            .as_path()
            .to_path_buf(),
        TestPlanStore::plan_item_file()
            .relative_path()
            .as_path()
            .to_path_buf(),
        TestPlanStore::plan_item_payload_file()
            .relative_path()
            .as_path()
            .to_path_buf(),
    ];
    assert_eq!(
        declared_paths,
        [
            std::path::PathBuf::from(PLAN_BIN),
            std::path::PathBuf::from(PLAN_PAYLOAD_BIN),
            std::path::PathBuf::from(PLAN_REVISION_BIN),
            std::path::PathBuf::from(PLAN_REVISION_PAYLOAD_BIN),
            std::path::PathBuf::from(PLAN_ITEM_BIN),
            std::path::PathBuf::from(PLAN_ITEM_PAYLOAD_BIN),
        ]
    );
}

#[test]
fn plan_binary_db_has_explicit_local_and_remote_adapters() {
    let temp_dir = tempdir().expect("tempdir");
    let local = LocalPlanBinaryDb::<TEST_WRITE_LAYOUT>::new(
        temp_dir.path().join("local"),
        temp_dir.path(),
        AuthorityId::new("local-authority"),
        LocalStateScope::Repository,
    );
    let remote =
        RemoteFsPlanBinaryDb::<TEST_WRITE_LAYOUT>::from_fs(RemoteBinaryDbFs::test_fixture(
            temp_dir.path().join("remote"),
            RepoId::new("repo-1"),
            RepoName::new("origin"),
        ));

    assert_eq!(remote.db().remote_repo_id(), &RepoId::new("repo-1"));
    assert_eq!(remote.db().remote_repo_name(), &RepoName::new("origin"));

    let mut local_tx = local
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin local plan txn");
    let local_payload = PlanPayload {
        title_bytes: b"Local plan".to_vec(),
    };
    let local_record = PlanRecord {
        plan_meta: 1,
        reserved0: 0,
        payload_len: 0,
        payload_offset: 0,
        latest_revision_index_plus1: 0,
        published_plan_index_plus1: 0,
        published_latest_revision_index_plus1: 0,
        created_at_s: 10,
        updated_at_s: 11,
        published_at_s: 0,
    };
    let (local_index, local_record) = local
        .append_plan(&mut local_tx, local_record, &local_payload)
        .expect("append local plan");
    local_tx.commit().expect("commit local plan txn");

    let local_read = local.begin_read_txn();
    assert_eq!(
        local
            .read_plan(&local_read, local_index)
            .expect("read local plan"),
        (local_record, local_payload)
    );

    let mut remote_tx = remote
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncRemote,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin remote plan txn");
    let remote_payload = PlanPayload {
        title_bytes: b"Remote plan".to_vec(),
    };
    let remote_record = PlanRecord {
        plan_meta: 2,
        reserved0: 0,
        payload_len: 0,
        payload_offset: 0,
        latest_revision_index_plus1: 0,
        published_plan_index_plus1: 0,
        published_latest_revision_index_plus1: 0,
        created_at_s: 20,
        updated_at_s: 21,
        published_at_s: 0,
    };
    let (remote_index, remote_record) = remote
        .append_plan(&mut remote_tx, remote_record, &remote_payload)
        .expect("append remote plan");
    remote_tx.commit().expect("commit remote plan txn");

    let remote_read = remote.begin_read_txn();
    assert_eq!(
        remote
            .read_plan(&remote_read, remote_index)
            .expect("read remote plan"),
        (remote_record, remote_payload)
    );

    assert_header_u32_at(
        &temp_dir.path().join("local").join(PLAN_BIN),
        TEST_WRITE_LAYOUT,
    );
    assert_header_u32_at(
        &temp_dir.path().join("remote").join(PLAN_BIN),
        TEST_WRITE_LAYOUT,
    );
}

#[test]
fn remote_plan_sync_publish_txn_uses_remote_locks_and_plan_revision_commit_point() {
    let temp_dir = tempdir().expect("tempdir");
    let remote =
        RemoteFsPlanBinaryDb::<TEST_WRITE_LAYOUT>::from_fs(RemoteBinaryDbFs::test_fixture(
            temp_dir.path().join("remote"),
            RepoId::new("repo-1"),
            RepoName::new("origin"),
        ));
    let mut tx = remote
        .begin_publish_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
        .expect("begin remote publish txn");

    assert_eq!(tx.command_scope(), BinaryDbCommandScope::PlanSyncRemote);
    let lock_paths = tx.lock_paths();
    assert_eq!(lock_paths.len(), 2);
    assert!(lock_paths
        .iter()
        .any(|path| path.ends_with("remote-content.write.lock")));
    assert!(lock_paths
        .iter()
        .any(|path| path.ends_with("remote-plan.write.lock")));

    tx.track_content_dependency_path(&StorePath::from("tree.bin"))
        .expect("track content dependency");
    let (_plan_index, _plan_record) = tx
        .append_plan(
            PlanRecord {
                plan_meta: 1,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                latest_revision_index_plus1: 1,
                published_plan_index_plus1: 0,
                published_latest_revision_index_plus1: 0,
                created_at_s: 10,
                updated_at_s: 11,
                published_at_s: 12,
            },
            &PlanPayload {
                title_bytes: b"Remote plan".to_vec(),
            },
        )
        .expect("append plan");
    let (_item_index, _item_record) = tx
        .append_plan_item(
            PlanItemRecord {
                item_meta: 1,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                line_number: 7,
            },
            &PlanItemPayload {
                plan_item_ref_bytes: b"plan/root".to_vec(),
                text_bytes: b"ship binary db".to_vec(),
                heading_path: vec!["Binary DB".to_string()],
            },
        )
        .expect("append item");
    let (revision_index, revision_record) = tx
        .append_plan_revision_commit(
            PlanRevisionRecord {
                revision_meta: 1,
                reserved0: 0,
                payload_len: 0,
                revision_number: 1,
                item_count: 1,
                payload_offset: 0,
                plan_index: 0,
                previous_revision_index_plus1: 0,
                item_start_index: 0,
                published_revision_index_plus1: 0,
                root_tree_pack_index_plus1: 3,
                root_entry_ordinal: 4,
                created_at_s: 20,
                published_at_s: 21,
            },
            &PlanRevisionPayload {
                title_snapshot_bytes: b"Remote plan".to_vec(),
                summary_bytes: b"publish".to_vec(),
                artifact_path_bytes: b"docs/plan.md".to_vec(),
                artifact_selector_bytes: b"plan/root".to_vec(),
                artifact_heading_bytes: b"Binary DB".to_vec(),
                artifact_blob_id_bytes: Vec::new(),
            },
        )
        .expect("append revision commit");
    assert_eq!(revision_index, 0);
    assert!(tx
        .append_plan_item(
            PlanItemRecord {
                item_meta: 1,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                line_number: 8,
            },
            &PlanItemPayload {
                plan_item_ref_bytes: b"after/commit".to_vec(),
                text_bytes: b"should fail".to_vec(),
                heading_path: Vec::new(),
            },
        )
        .is_err());
    let commit_point = tx.commit().expect("commit remote publish txn");
    assert_eq!(
        commit_point,
        RemotePlanSyncCommitPoint::PlanRevision {
            plan_revision_index: revision_index
        }
    );

    let read = remote.begin_read_txn();
    assert_eq!(
        remote
            .read_plan_revision_record(&read, revision_index)
            .expect("read revision"),
        revision_record
    );
    assert_header_u32_at(
        &temp_dir.path().join("remote").join(PLAN_REVISION_BIN),
        TEST_WRITE_LAYOUT,
    );
}

#[test]
fn local_plan_sync_txn_purposes_use_command_scoped_locks() {
    let temp_dir = tempdir().expect("tempdir");
    let local = LocalPlanBinaryDb::<TEST_WRITE_LAYOUT>::new(
        temp_dir.path().join("local"),
        temp_dir.path(),
        AuthorityId::new("local-authority"),
        LocalStateScope::Repository,
    );

    let upsert = local
        .begin_local_upsert_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
        .expect("begin local upsert txn");
    assert_eq!(
        upsert.purpose(),
        PlanBinaryDbWritePurpose::LocalPlanSyncUpsert
    );
    assert_eq!(upsert.command_scope(), BinaryDbCommandScope::PlanSyncLocal);
    assert!(upsert
        .lock_paths()
        .iter()
        .any(|path| path.ends_with("content.write.lock")));
    assert!(upsert
        .lock_paths()
        .iter()
        .any(|path| path.ends_with("plan.write.lock")));
    upsert.abort().expect("abort upsert");

    {
        let purpose = PlanBinaryDbWritePurpose::LocalPlanSyncPrune;
        let tx = local
            .begin_local_prune_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
            .expect("begin prune txn");
        assert_eq!(tx.purpose(), purpose);
        assert_eq!(tx.command_scope(), BinaryDbCommandScope::PlanSyncLocalPlan);
        let lock_paths = tx.lock_paths();
        assert_eq!(lock_paths.len(), 1);
        assert!(lock_paths[0].ends_with("plan.write.lock"));
        let commit_point = tx.commit().expect("commit plan-only txn");
        assert_eq!(
            commit_point,
            PlanBinaryDbCommitPoint::NoRecordCommit { purpose }
        );
    }
    {
        let purpose = PlanBinaryDbWritePurpose::LocalPlanSyncAdoption;
        let tx = local
            .begin_local_adoption_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
            .expect("begin adoption txn");
        assert_eq!(tx.purpose(), purpose);
        assert_eq!(tx.command_scope(), BinaryDbCommandScope::PlanSyncLocalPlan);
        let lock_paths = tx.lock_paths();
        assert_eq!(lock_paths.len(), 1);
        assert!(lock_paths[0].ends_with("plan.write.lock"));
        let commit_point = tx.commit().expect("commit plan-only txn");
        assert_eq!(
            commit_point,
            PlanBinaryDbCommitPoint::NoRecordCommit { purpose }
        );
    }
    {
        let purpose = PlanBinaryDbWritePurpose::LocalPlanSyncPublishReceipt;
        let tx = local
            .begin_local_publish_receipt_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
            .expect("begin publish receipt txn");
        assert_eq!(tx.purpose(), purpose);
        assert_eq!(tx.command_scope(), BinaryDbCommandScope::PlanSyncLocalPlan);
        let lock_paths = tx.lock_paths();
        assert_eq!(lock_paths.len(), 1);
        assert!(lock_paths[0].ends_with("plan.write.lock"));
        let commit_point = tx.commit().expect("commit plan-only txn");
        assert_eq!(
            commit_point,
            PlanBinaryDbCommitPoint::NoRecordCommit { purpose }
        );
    }
}

#[test]
fn local_plan_sync_txn_rejects_stale_plan_state_under_the_write_lock() {
    let temp_dir = tempdir().expect("tempdir");
    let local = LocalPlanBinaryDb::<TEST_WRITE_LAYOUT>::new(
        temp_dir.path().join("local"),
        temp_dir.path(),
        AuthorityId::new("local-authority"),
        LocalStateScope::Repository,
    );
    let base_record = PlanRecord {
        plan_meta: 0,
        reserved0: 0,
        payload_len: 0,
        payload_offset: 0,
        latest_revision_index_plus1: 1,
        published_plan_index_plus1: 0,
        published_latest_revision_index_plus1: 0,
        created_at_s: 10,
        updated_at_s: 10,
        published_at_s: 0,
    };
    let payload = PlanPayload {
        title_bytes: b"CAS plan".to_vec(),
    };
    let base_record = {
        let mut tx = local
            .begin_local_upsert_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
            .expect("seed transaction should begin");
        let (_, persisted) = tx
            .append_plan(base_record.clone(), &payload)
            .expect("plan should append");
        tx.append_plan_revision_commit(
            PlanRevisionRecord {
                revision_meta: 0,
                reserved0: 0,
                payload_len: 0,
                revision_number: 1,
                item_count: 0,
                payload_offset: 0,
                plan_index: 0,
                previous_revision_index_plus1: 0,
                item_start_index: 0,
                published_revision_index_plus1: 0,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                created_at_s: 10,
                published_at_s: 0,
            },
            &PlanRevisionPayload {
                title_snapshot_bytes: b"CAS plan".to_vec(),
                summary_bytes: Vec::new(),
                artifact_path_bytes: b"docs/plan.md".to_vec(),
                artifact_selector_bytes: Vec::new(),
                artifact_heading_bytes: b"Plan".to_vec(),
                artifact_blob_id_bytes: Vec::new(),
            },
        )
        .expect("revision should append");
        tx.commit().expect("seed transaction should commit");
        persisted
    };

    {
        let mut winner = local
            .begin_local_prune_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
            .expect("winner transaction should begin");
        winner
            .require_unchanged_plan(0, &base_record)
            .expect("winner should observe the prepared state");
        let mut archived = base_record.clone();
        archived.plan_meta = 1;
        archived.updated_at_s = 11;
        winner
            .overwrite_plan_commit(0, archived, &payload)
            .expect("winner plan should overwrite");
        winner.commit().expect("winner should commit");
    }

    let stale = local
        .begin_local_upsert_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
        .expect("stale transaction should begin");
    let error = stale
        .require_unchanged_plan(0, &base_record)
        .expect_err("stale prepared state must fail under the lock");
    assert_eq!(error.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(error.contains("state advanced under the Binary DB write lock"));
    stale.abort().expect("stale transaction should abort");
}

#[test]
fn local_plan_sync_txn_rejects_missing_revision_content_root_before_append() {
    let temp_dir = tempdir().expect("tempdir");
    let local = LocalPlanBinaryDb::<TEST_WRITE_LAYOUT>::new(
        temp_dir.path().join("local"),
        temp_dir.path(),
        AuthorityId::new("local-authority"),
        LocalStateScope::Repository,
    );
    let record = PlanRevisionRecord {
        revision_meta: 0,
        reserved0: 0,
        payload_len: 0,
        revision_number: 1,
        item_count: 0,
        payload_offset: 0,
        plan_index: 0,
        previous_revision_index_plus1: 0,
        item_start_index: 0,
        published_revision_index_plus1: 0,
        root_tree_pack_index_plus1: 1,
        root_entry_ordinal: 0,
        created_at_s: 10,
        published_at_s: 0,
    };
    let payload = PlanRevisionPayload {
        title_snapshot_bytes: b"Root plan".to_vec(),
        summary_bytes: Vec::new(),
        artifact_path_bytes: b"docs/plan.md".to_vec(),
        artifact_selector_bytes: Vec::new(),
        artifact_heading_bytes: b"Plan".to_vec(),
        artifact_blob_id_bytes: b"BLB-00000000000000000000".to_vec(),
    };
    let mut tx = local
        .begin_local_upsert_txn_with_fsync_policy(BinaryDbNoopFsyncPolicy)
        .expect("transaction should begin");
    let error = tx
        .bind_revision_content_root(&record, &payload)
        .expect_err("missing tree pack must fail before revision append");
    assert!(matches!(
        error.kind(),
        BinaryDbErrorKind::MissingData | BinaryDbErrorKind::Io
    ));
    assert_eq!(
        tx.record_count(
            BinaryDbPlanStore::<LocalBinaryDbFs, TEST_WRITE_LAYOUT>::plan_revision_file()
        )
        .expect("revision count should read"),
        0
    );
    tx.abort().expect("transaction should abort");
}

#[test]
fn remote_plan_sync_artifact_attach_overwrites_root_locator_in_plan_revision_bin() {
    let temp_dir = tempdir().expect("tempdir");
    let remote =
        RemoteFsPlanBinaryDb::<TEST_WRITE_LAYOUT>::from_fs(RemoteBinaryDbFs::test_fixture(
            temp_dir.path().join("remote"),
            RepoId::new("repo-1"),
            RepoName::new("origin"),
        ));
    let no_roots = remote
        .begin_artifact_attach_txn_for_roots(&[], BinaryDbNoopFsyncPolicy)
        .expect("begin skipped attach txn");
    assert!(no_roots.is_none());
    assert!(!temp_dir.path().join("remote/.locks").exists());

    let mut seed = remote
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncRemote,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin seed transaction");
    let (revision_index, _) = remote
        .append_plan_revision(
            &mut seed,
            PlanRevisionRecord {
                revision_meta: 0,
                reserved0: 0,
                payload_len: 0,
                revision_number: 1,
                item_count: 0,
                payload_offset: 0,
                plan_index: 0,
                previous_revision_index_plus1: 0,
                item_start_index: 0,
                published_revision_index_plus1: 0,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                created_at_s: 1,
                published_at_s: 0,
            },
            &PlanRevisionPayload {
                title_snapshot_bytes: b"root locator".to_vec(),
                summary_bytes: Vec::new(),
                artifact_path_bytes: b"docs/root.md".to_vec(),
                artifact_selector_bytes: Vec::new(),
                artifact_heading_bytes: b"Root".to_vec(),
                artifact_blob_id_bytes: b"BLB-ROOT".to_vec(),
            },
        )
        .expect("seed plan revision");
    assert_eq!(revision_index, 0);
    seed.commit().expect("commit seed revision");

    let root_update = PlanRevisionRootUpdate {
        plan_revision_index: revision_index,
        root_tree_pack_index_plus1: 4,
        root_entry_ordinal: 5,
    };
    let mut tx = remote
        .begin_artifact_attach_txn_for_roots(
            std::slice::from_ref(&root_update),
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin attach txn")
        .expect("attach txn exists");
    assert_eq!(tx.command_scope(), BinaryDbCommandScope::PlanSyncRemote);
    tx.track_content_dependency_path(&StorePath::from("tree.bin"))
        .expect("track content dependency");
    let root_index = tx
        .attach_revision_root_commit(&root_update)
        .expect("overwrite root locator");
    assert_eq!(root_index, 0);
    assert!(tx
        .attach_revision_root_commit(&PlanRevisionRootUpdate {
            plan_revision_index: revision_index,
            root_tree_pack_index_plus1: 6,
            root_entry_ordinal: 7,
        })
        .is_err());
    let commit_point = tx.commit().expect("commit attach txn");
    assert_eq!(
        commit_point,
        RemotePlanSyncCommitPoint::RevisionRoot {
            plan_revision_index: revision_index,
        }
    );

    let read = remote.begin_read_txn();
    let stored = remote
        .read_plan_revision_record(&read, revision_index)
        .expect("read updated revision");
    assert_eq!(stored.root_tree_pack_index_plus1, 4);
    assert_eq!(stored.root_entry_ordinal, 5);
    assert_header_u32_at(
        &temp_dir.path().join("remote").join(PLAN_REVISION_BIN),
        TEST_WRITE_LAYOUT,
    );
    for forbidden in [
        "plan_head.bin",
        "plan_revision_head.bin",
        "plan_revision_root.bin",
    ] {
        assert!(!temp_dir.path().join("remote").join(forbidden).exists());
    }
}

#[test]
fn plan_binary_db_read_surfaces_list_get_and_walk_revision_chain() {
    let (store, _temp_dir) = new_local_store();
    seed_plan_read_surface_fixture(&store);
    let read = store.begin_read_txn();

    let plans = store
        .list_plans(&read, Some("ait-core"), None)
        .expect("list plans");
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0].plan_index, 1);
    assert_eq!(plans[0].record.status_name(), "archived");
    assert_eq!(plans[1].plan_index, 0);
    assert_eq!(plans[1].repo_name.as_deref(), Some("ait-core"));

    let runtime_plans = store
        .list_plans(&read, None, Some("docs/runtime.md"))
        .expect("list plans by artifact path");
    assert_eq!(runtime_plans.len(), 1);
    assert_eq!(runtime_plans[0].plan_index, 0);

    let plan = store
        .get_plan(&read, 0, Some("ait-core"))
        .expect("get plan");
    assert_eq!(plan.title_text().expect("title"), "Runtime Plan");
    assert_eq!(plan.status_name(), "draft");
    assert_eq!(plan.publication_state_name(), "published");
    assert_eq!(plan.record.published_plan_index(), Some(10));
    assert_eq!(plan.record.published_latest_revision_index(), Some(11));
    let head = plan.head_revision.as_ref().expect("head revision");
    assert_eq!(head.revision_index, 1);
    assert_eq!(head.record.revision_number, 2);
    assert_eq!(head.record.previous_revision_index(), Some(0));
    assert_eq!(head.record.published_revision_index(), Some(21));
    assert_eq!(head.payload.artifact_blob_id_text().unwrap(), "BLB-HEAD");
    assert_eq!(head.items.len(), 2);
    assert_eq!(head.items[0].record.checkbox_state_name(), "open");
    assert!(head.items[0].record.has_item_ref());
    assert!(head.items[0].record.is_taskable_hint());
    assert_eq!(
        head.items[0].payload.plan_item_ref_text().unwrap(),
        "runtime/taskable"
    );

    let revisions = store.list_plan_revisions(&read, 0).expect("list revisions");
    assert_eq!(
        revisions
            .iter()
            .map(|revision| revision.revision_index)
            .collect::<Vec<_>>(),
        vec![1, 0]
    );
    assert_eq!(revisions[1].record.revision_number, 1);

    assert!(store.get_plan_revision(&read, 1, 1).is_err());
}

#[test]
fn plan_binary_db_artifact_filter_fails_closed_on_invalid_utf8() {
    let (store, _temp_dir) = new_local_store();
    let mut tx = store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin corrupt fixture transaction");

    store
        .append_plan(
            &mut tx,
            PlanRecord {
                plan_meta: 0b0000_0001,
                reserved0: 0,
                payload_len: 0,
                payload_offset: 0,
                latest_revision_index_plus1: 1,
                published_plan_index_plus1: 0,
                published_latest_revision_index_plus1: 0,
                created_at_s: 1,
                updated_at_s: 1,
                published_at_s: 0,
            },
            &PlanPayload {
                title_bytes: b"Corrupt artifact path".to_vec(),
            },
        )
        .expect("append plan");
    store
        .append_plan_revision(
            &mut tx,
            PlanRevisionRecord {
                revision_meta: 0,
                reserved0: 0,
                payload_len: 0,
                revision_number: 1,
                item_count: 0,
                payload_offset: 0,
                plan_index: 0,
                previous_revision_index_plus1: 0,
                item_start_index: 0,
                published_revision_index_plus1: 0,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                created_at_s: 1,
                published_at_s: 0,
            },
            &PlanRevisionPayload {
                title_snapshot_bytes: b"Corrupt artifact path".to_vec(),
                summary_bytes: Vec::new(),
                artifact_path_bytes: vec![0xff],
                artifact_selector_bytes: Vec::new(),
                artifact_heading_bytes: Vec::new(),
                artifact_blob_id_bytes: Vec::new(),
            },
        )
        .expect("append corrupt revision payload");
    tx.commit().expect("commit corrupt fixture");

    let read = store.begin_read_txn();
    let error = store
        .list_plans(&read, None, Some("docs/plan.md"))
        .expect_err("invalid artifact path UTF-8 must fail closed");
    assert_eq!(error.kind(), BinaryDbErrorKind::Corruption);
    assert!(error.contains("artifact_path"));
}

#[test]
fn plan_binary_db_scan_plan_heads_filters_active_artifact_and_contains_terms() {
    let (store, _temp_dir) = new_local_store();
    seed_plan_read_surface_fixture(&store);
    let read = store.begin_read_txn();

    let active = store
        .scan_plan_heads(
            &read,
            PlanHeadScanFilter {
                repo_name: Some("ait-core"),
                artifact_path: None,
                contains_terms: &[],
                active_only: true,
            },
        )
        .expect("scan active");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].plan_index, 0);
    assert_eq!(active[0].repo_name.as_deref(), Some("ait-core"));
    assert_eq!(
        active[0].head_artifact_path().unwrap().as_deref(),
        Some("docs/runtime.md")
    );
    assert_eq!(active[0].head_publication_state_name(), Some("published"));

    let contains_terms = vec!["fused".to_string()];
    let filtered = store
        .scan_plan_heads(
            &read,
            PlanHeadScanFilter {
                repo_name: None,
                artifact_path: Some("docs/runtime.md"),
                contains_terms: &contains_terms,
                active_only: true,
            },
        )
        .expect("scan filtered");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].plan_index, 0);

    let missing_terms = vec!["does-not-match".to_string()];
    let missing = store
        .scan_plan_heads(
            &read,
            PlanHeadScanFilter {
                repo_name: None,
                artifact_path: Some("docs/runtime.md"),
                contains_terms: &missing_terms,
                active_only: true,
            },
        )
        .expect("scan missing");
    assert!(missing.is_empty());
}

#[test]
fn plan_record_codec_round_trips_48_bytes() {
    let record = PlanRecord {
        plan_meta: 0b0000_0101,
        reserved0: 0,
        payload_len: 12,
        payload_offset: 4,
        latest_revision_index_plus1: 9,
        published_plan_index_plus1: 2,
        published_latest_revision_index_plus1: 8,
        created_at_s: 10,
        updated_at_s: 11,
        published_at_s: 12,
    };

    let bytes = PlanCodec::<TEST_WRITE_LAYOUT>::encode_record(&record).expect("encode");
    assert_eq!(bytes.len(), PLAN_RECORD_SIZE_USIZE);
    assert_eq!(
        PlanCodec::<TEST_WRITE_LAYOUT>::decode_record(&bytes).expect("decode"),
        record
    );
}

#[test]
fn plan_revision_record_codec_round_trips_56_bytes_without_textual_id() {
    let record = PlanRevisionRecord {
        revision_meta: 0b0000_1001,
        reserved0: 0,
        payload_len: 31,
        revision_number: 3,
        item_count: 5,
        payload_offset: 44,
        plan_index: 7,
        previous_revision_index_plus1: 6,
        item_start_index: 20,
        published_revision_index_plus1: 0,
        root_tree_pack_index_plus1: 13,
        root_entry_ordinal: 2,
        created_at_s: 100,
        published_at_s: 0,
    };

    let bytes = PlanRevisionCodec::<TEST_WRITE_LAYOUT>::encode_record(&record).expect("encode");
    assert_eq!(bytes.len(), PLAN_REVISION_RECORD_SIZE_USIZE);
    assert_eq!(
        PlanRevisionCodec::<TEST_WRITE_LAYOUT>::decode_record(&bytes).expect("decode"),
        record
    );
}

#[test]
fn plan_codecs_round_trip_full_u64_seconds() {
    for seconds in [u64::from(u32::MAX) + 1, u64::MAX] {
        let plan = PlanRecord {
            plan_meta: 0,
            reserved0: 0,
            payload_len: 1,
            payload_offset: 4,
            latest_revision_index_plus1: 0,
            published_plan_index_plus1: 0,
            published_latest_revision_index_plus1: 0,
            created_at_s: seconds,
            updated_at_s: seconds,
            published_at_s: seconds,
        };
        let bytes = PlanCodec::<TEST_WRITE_LAYOUT>::encode_record(&plan).unwrap();
        assert_eq!(
            PlanCodec::<TEST_WRITE_LAYOUT>::decode_record(&bytes).unwrap(),
            plan
        );

        let revision = PlanRevisionRecord {
            revision_meta: 0,
            reserved0: 0,
            payload_len: 1,
            revision_number: 1,
            item_count: 0,
            payload_offset: 4,
            plan_index: 0,
            previous_revision_index_plus1: 0,
            item_start_index: 0,
            published_revision_index_plus1: 0,
            root_tree_pack_index_plus1: 0,
            root_entry_ordinal: 0,
            created_at_s: seconds,
            published_at_s: seconds,
        };
        let bytes = PlanRevisionCodec::<TEST_WRITE_LAYOUT>::encode_record(&revision).unwrap();
        assert_eq!(
            PlanRevisionCodec::<TEST_WRITE_LAYOUT>::decode_record(&bytes).unwrap(),
            revision
        );
    }
}

#[test]
fn plan_item_record_codec_round_trips_16_bytes() {
    let record = PlanItemRecord {
        item_meta: 0b0000_1101,
        reserved0: 0,
        payload_len: 17,
        payload_offset: 64,
        line_number: 42,
    };

    let bytes = PlanItemCodec::<TEST_WRITE_LAYOUT>::encode_record(&record).expect("encode");
    assert_eq!(bytes.len(), PLAN_ITEM_RECORD_SIZE_USIZE);
    assert_eq!(
        PlanItemCodec::<TEST_WRITE_LAYOUT>::decode_record(&bytes).expect("decode"),
        record
    );
}

#[test]
fn plan_revision_payload_derives_artifact_blob_id_length_from_payload_len() {
    let payload = PlanRevisionPayload {
        title_snapshot_bytes: b"title".to_vec(),
        summary_bytes: b"summary".to_vec(),
        artifact_path_bytes: b"docs/plan.md".to_vec(),
        artifact_selector_bytes: b"".to_vec(),
        artifact_heading_bytes: b"Plan".to_vec(),
        artifact_blob_id_bytes: b"BLB-transition-only".to_vec(),
    };

    let bytes = PlanRevisionCodec::<TEST_WRITE_LAYOUT>::encode_payload(&payload).expect("encode");
    let decoded = PlanRevisionCodec::<TEST_WRITE_LAYOUT>::decode_payload(&bytes).expect("decode");
    assert_eq!(decoded, payload);
}

#[test]
fn plan_item_payload_uses_server_compatible_binary_heading_path() {
    let payload = PlanItemPayload {
        plan_item_ref_bytes: b"SBDB/PARITY-11".to_vec(),
        text_bytes: b"Exact ref".to_vec(),
        heading_path: vec!["Binary DB".to_string(), "Fixtures".to_string()],
    };

    let bytes = PlanItemCodec::<TEST_WRITE_LAYOUT>::encode_payload(&payload).expect("encode");
    assert_eq!(
        bytes,
        vec![
            14, 0, 9, 0, 2, 0, 83, 66, 68, 66, 47, 80, 65, 82, 73, 84, 89, 45, 49, 49, 69, 120, 97,
            99, 116, 32, 114, 101, 102, 9, 0, 66, 105, 110, 97, 114, 121, 32, 68, 66, 8, 0, 70,
            105, 120, 116, 117, 114, 101, 115,
        ],
        "core and server compact layout-1 Plan item bytes must match"
    );
    let decoded = PlanItemCodec::<TEST_WRITE_LAYOUT>::decode_payload(&bytes).expect("decode");
    assert_eq!(decoded, payload);

    let mut trailing = bytes;
    trailing.push(0);
    let error = PlanItemCodec::<TEST_WRITE_LAYOUT>::decode_payload(&trailing)
        .expect_err("trailing bytes must fail closed");
    assert!(error.to_string().contains("trailing bytes"));

    let invalid_utf8 = [0, 0, 0, 0, 1, 0, 1, 0, 255];
    let error = PlanItemCodec::<TEST_WRITE_LAYOUT>::decode_payload(&invalid_utf8)
        .expect_err("invalid heading UTF-8 must fail closed");
    assert!(error.to_string().contains("not valid UTF-8"));
}

#[test]
fn plan_binary_db_heading_path_storage_has_no_json_codec_contract() {
    let sources = [
        include_str!("schema/payloads.rs"),
        include_str!("schema/codec.rs"),
        include_str!("read/filters.rs"),
    ];
    for source in sources {
        assert!(!source.contains("heading_path_bytes"));
        assert!(!source.contains("heading_path_text"));
        assert!(!source.contains("JsonCodec"));
    }
}

#[test]
fn plan_revision_root_locator_is_part_of_the_canonical_revision_record() {
    let record = PlanRevisionRecord {
        revision_meta: 0,
        reserved0: 0,
        payload_len: 0,
        revision_number: 1,
        item_count: 0,
        payload_offset: 0,
        plan_index: 3,
        previous_revision_index_plus1: 0,
        item_start_index: 0,
        published_revision_index_plus1: 0,
        root_tree_pack_index_plus1: 8,
        root_entry_ordinal: 13,
        created_at_s: 1,
        published_at_s: 0,
    };
    let bytes = PlanRevisionCodec::<TEST_WRITE_LAYOUT>::encode_record(&record).expect("encode");
    assert_eq!(bytes.len(), PLAN_REVISION_RECORD_SIZE_USIZE);
    let decoded =
        PlanRevisionCodec::<TEST_WRITE_LAYOUT>::decode_record(&bytes).expect("decode revision");
    assert_eq!(decoded.root_tree_pack_index_plus1, 8);
    assert_eq!(decoded.root_entry_ordinal, 13);
}

#[test]
fn plan_binary_db_txn_append_and_read_plan_records_and_payloads() {
    let (store, temp_dir) = new_local_store();
    let mut tx = store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin plan write txn");

    let plan_payload = PlanPayload {
        title_bytes: b"Plan title".to_vec(),
    };
    let plan_record = PlanRecord {
        plan_meta: 0b0000_0001,
        reserved0: 0,
        payload_len: 0,
        payload_offset: 0,
        latest_revision_index_plus1: 1,
        published_plan_index_plus1: 0,
        published_latest_revision_index_plus1: 0,
        created_at_s: 1_700_000_100,
        updated_at_s: 1_700_000_200,
        published_at_s: 0,
    };
    let (plan_record_index, plan_record) = store
        .append_plan(&mut tx, plan_record, &plan_payload)
        .expect("append plan");
    assert_eq!(plan_record_index, 0);

    let revision_payload = PlanRevisionPayload {
        title_snapshot_bytes: b"snapshot".to_vec(),
        summary_bytes: b"first revision".to_vec(),
        artifact_path_bytes: b".md".to_vec(),
        artifact_selector_bytes: b"".to_vec(),
        artifact_heading_bytes: b"Plan".to_vec(),
        artifact_blob_id_bytes: b"BLB-1".to_vec(),
    };
    let revision_record = PlanRevisionRecord {
        revision_meta: 0b0000_0001,
        reserved0: 0,
        payload_len: 0,
        revision_number: 1,
        item_count: 1,
        payload_offset: 0,
        plan_index: 1,
        previous_revision_index_plus1: 0,
        item_start_index: 0,
        published_revision_index_plus1: 0,
        root_tree_pack_index_plus1: 0,
        root_entry_ordinal: 0,
        created_at_s: 1_700_000_300,
        published_at_s: 0,
    };
    let (revision_record_index, revision_record) = store
        .append_plan_revision(&mut tx, revision_record, &revision_payload)
        .expect("append revision");
    assert_eq!(revision_record_index, 0);

    let item_payload = PlanItemPayload {
        plan_item_ref_bytes: b"plan/init".to_vec(),
        text_bytes: b"- add plan scaffolding".to_vec(),
        heading_path: vec!["Plan".to_string()],
    };
    let item_record = PlanItemRecord {
        item_meta: 0b0000_0001,
        reserved0: 0,
        payload_len: 0,
        payload_offset: 0,
        line_number: 42,
    };
    let (item_record_index, item_record) = store
        .append_plan_item(&mut tx, item_record, &item_payload)
        .expect("append item");
    assert_eq!(item_record_index, 0);

    tx.commit().expect("commit plan write txn");

    let legacy_hash_path = temp_dir.path().join("plan_item_ref.hash");
    assert!(!legacy_hash_path.exists());
    fs::write(&legacy_hash_path, b"stale legacy hash side file").expect("write stale side file");

    let read_txn = store.begin_read_txn();
    assert_eq!(
        read_txn
            .layout_id(TestPlanStore::plan_file())
            .expect("read layout id"),
        TEST_WRITE_LAYOUT
    );

    let (read_plan_record, read_plan_payload) = store
        .read_plan(&read_txn, plan_record_index)
        .expect("read plan");
    assert_eq!(read_plan_record, plan_record);
    assert_eq!(read_plan_payload, plan_payload);

    let (read_revision_record, read_revision_payload) = store
        .read_plan_revision(&read_txn, revision_record_index)
        .expect("read revision");
    assert_eq!(read_revision_record, revision_record);
    assert_eq!(read_revision_payload, revision_payload);

    let (read_item_record, read_item_payload) = store
        .read_plan_item(&read_txn, item_record_index)
        .expect("read item");
    assert_eq!(read_item_record, item_record);
    assert_eq!(read_item_payload, item_payload);

    assert_eq!(
        read_txn
            .read_record(TestPlanStore::plan_file(), plan_record_index)
            .expect("read plan record"),
        PlanCodec::<TEST_WRITE_LAYOUT>::encode_record(&plan_record).expect("encode plan record"),
    );
    assert_eq!(
        read_txn
            .read_record(TestPlanStore::plan_revision_file(), revision_record_index)
            .expect("read revision record"),
        PlanRevisionCodec::<TEST_WRITE_LAYOUT>::encode_record(&revision_record)
            .expect("encode revision record"),
    );
    assert_eq!(
        read_txn
            .read_record(TestPlanStore::plan_item_file(), item_record_index)
            .expect("read item record"),
        PlanItemCodec::<TEST_WRITE_LAYOUT>::encode_record(&item_record)
            .expect("encode item record"),
    );

    assert_header_u32_at(&temp_dir.path().join(PLAN_BIN), TEST_WRITE_LAYOUT);
    assert_header_u32_at(&temp_dir.path().join(PLAN_REVISION_BIN), TEST_WRITE_LAYOUT);
    assert_header_u32_at(&temp_dir.path().join(PLAN_ITEM_BIN), TEST_WRITE_LAYOUT);
    assert_header_u32_at(&temp_dir.path().join(PLAN_PAYLOAD_BIN), TEST_WRITE_LAYOUT);
    assert_header_u32_at(
        &temp_dir.path().join(PLAN_REVISION_PAYLOAD_BIN),
        TEST_WRITE_LAYOUT,
    );
    assert_header_u32_at(
        &temp_dir.path().join(PLAN_ITEM_PAYLOAD_BIN),
        TEST_WRITE_LAYOUT,
    );
}

#[test]
fn plan_binary_db_txn_enforces_write_layout_headers() {
    let (store, temp_dir) = new_local_store();
    for path in [
        PLAN_BIN,
        PLAN_REVISION_BIN,
        PLAN_PAYLOAD_BIN,
        PLAN_REVISION_PAYLOAD_BIN,
        PLAN_ITEM_PAYLOAD_BIN,
    ] {
        fs::write(temp_dir.path().join(path), 2_u32.to_le_bytes().as_slice())
            .expect("write legacy layout header");
    }
    let mut tx = store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin plan write txn");

    let bad_revision_record = PlanRevisionRecord {
        revision_meta: 0,
        reserved0: 0,
        payload_len: 0,
        revision_number: 1,
        item_count: 0,
        payload_offset: 0,
        plan_index: 0,
        previous_revision_index_plus1: 0,
        item_start_index: 0,
        published_revision_index_plus1: 0,
        root_tree_pack_index_plus1: 0,
        root_entry_ordinal: 0,
        created_at_s: 0,
        published_at_s: 0,
    };
    assert!(store
        .append_plan_revision_record(&mut tx, &bad_revision_record)
        .is_err());

    assert!(store
        .append_plan_payload(
            &mut tx,
            &PlanPayload {
                title_bytes: b"payload".to_vec()
            }
        )
        .is_err());
    assert!(store
        .append_plan_item_payload(
            &mut tx,
            &PlanItemPayload {
                plan_item_ref_bytes: b"ref".to_vec(),
                text_bytes: b"payload".to_vec(),
                heading_path: Vec::new(),
            }
        )
        .is_err());
    tx.abort().expect("abort plan write txn");

    assert_eq!(
        &fs::read(temp_dir.path().join(PLAN_REVISION_BIN)).expect("read old revision file")[0..4],
        2_u32.to_le_bytes().as_slice()
    );
}

#[test]
fn plan_binary_db_rejects_unsupported_write_layout() {
    let (store, _temp_dir) = new_unsupported_layout_local_store();
    let error = match store.begin_write_txn_with_fsync_policy(
        BinaryDbCommandScope::PlanSyncLocal,
        BinaryDbNoopFsyncPolicy,
    ) {
        Ok(_) => panic!("unsupported write layout should fail closed"),
        Err(error) => error,
    };

    assert!(error.contains("unsupported Plan Binary DB write layout"));
    assert!(error.contains("supported layout is 1"));
}

#[derive(Debug, Deserialize)]
struct PlanGoldenFixture {
    version: String,
    layout_id: u32,
    cases: Vec<PlanGoldenCase>,
}

#[derive(Debug, Deserialize)]
struct PlanGoldenCase {
    id: String,
    kind: String,
    input: JsonValue,
    expected_bytes: Vec<u8>,
}

fn golden_u64(input: &JsonValue, field: &str) -> u64 {
    input[field]
        .as_u64()
        .unwrap_or_else(|| panic!("golden field {field} must be u64"))
}

fn golden_u32(input: &JsonValue, field: &str) -> u32 {
    u32::try_from(golden_u64(input, field))
        .unwrap_or_else(|_| panic!("golden field {field} must fit u32"))
}

fn golden_u16(input: &JsonValue, field: &str) -> u16 {
    u16::try_from(golden_u64(input, field))
        .unwrap_or_else(|_| panic!("golden field {field} must fit u16"))
}

fn golden_u8(input: &JsonValue, field: &str) -> u8 {
    u8::try_from(golden_u64(input, field))
        .unwrap_or_else(|_| panic!("golden field {field} must fit u8"))
}

fn golden_hex_u64(input: &JsonValue, field: &str) -> u64 {
    u64::from_str_radix(
        input[field]
            .as_str()
            .unwrap_or_else(|| panic!("golden field {field} must be hex text")),
        16,
    )
    .unwrap_or_else(|_| panic!("golden field {field} must be valid u64 hex"))
}

fn golden_text(input: &JsonValue, field: &str) -> String {
    input[field]
        .as_str()
        .unwrap_or_else(|| panic!("golden field {field} must be text"))
        .to_string()
}

#[test]
fn plan_binary_db_complete_golden_fixture_matches_server_wire_contract() {
    let fixture: PlanGoldenFixture = serde_json::from_slice(BINARY_DB_PLAN_GOLDEN_SOURCE)
        .expect("Plan golden fixture must parse");
    assert_eq!(fixture.version, BINARY_DB_PLAN_GOLDEN_VERSION);
    assert_eq!(fixture.layout_id, PLAN_LAYOUT_ID);
    assert_eq!(
        binary_db_plan_golden_checksum(),
        BINARY_DB_PLAN_GOLDEN_CHECKSUM
    );

    let mut executed = BTreeSet::new();
    for case in fixture.cases {
        assert!(executed.insert(case.id.clone()), "duplicate {}", case.id);
        let input = &case.input;
        match case.kind.as_str() {
            "plan_record" => {
                let value = PlanRecord {
                    plan_meta: golden_u8(input, "plan_meta"),
                    reserved0: golden_u8(input, "reserved0"),
                    payload_len: golden_u16(input, "payload_len"),
                    payload_offset: golden_hex_u64(input, "payload_offset_hex"),
                    latest_revision_index_plus1: golden_u32(input, "latest_revision_index_plus1"),
                    published_plan_index_plus1: golden_u32(input, "published_plan_index_plus1"),
                    published_latest_revision_index_plus1: golden_u32(
                        input,
                        "published_latest_revision_index_plus1",
                    ),
                    created_at_s: golden_u64(input, "created_at_s"),
                    updated_at_s: golden_u64(input, "updated_at_s"),
                    published_at_s: golden_u64(input, "published_at_s"),
                };
                assert_eq!(
                    PlanCodec::<PLAN_LAYOUT_ID>::encode_record(&value).expect("encode plan"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    PlanCodec::<PLAN_LAYOUT_ID>::decode_record(&case.expected_bytes)
                        .expect("decode plan golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            "plan_payload" => {
                let value = PlanPayload {
                    title_bytes: golden_text(input, "title").into_bytes(),
                };
                assert_eq!(
                    PlanCodec::<PLAN_LAYOUT_ID>::encode_payload(&value).expect("encode title"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    PlanCodec::<PLAN_LAYOUT_ID>::decode_payload(&case.expected_bytes)
                        .expect("decode title golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            "plan_revision_record" => {
                let revision = PlanRevisionRecord {
                    revision_meta: golden_u8(input, "revision_meta"),
                    reserved0: golden_u8(input, "reserved0"),
                    payload_len: golden_u16(input, "payload_len"),
                    revision_number: golden_u16(input, "revision_number"),
                    item_count: golden_u16(input, "item_count"),
                    payload_offset: golden_hex_u64(input, "payload_offset_hex"),
                    plan_index: golden_u32(input, "plan_index"),
                    previous_revision_index_plus1: golden_u32(
                        input,
                        "previous_revision_index_plus1",
                    ),
                    item_start_index: golden_u32(input, "item_start_index"),
                    published_revision_index_plus1: golden_u32(
                        input,
                        "published_revision_index_plus1",
                    ),
                    root_tree_pack_index_plus1: golden_u32(input, "root_tree_pack_index_plus1"),
                    root_entry_ordinal: golden_u32(input, "root_entry_ordinal"),
                    created_at_s: golden_u64(input, "created_at_s"),
                    published_at_s: golden_u64(input, "published_at_s"),
                };
                assert_eq!(
                    PlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_record(&revision)
                        .expect("encode revision"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    PlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_record(&case.expected_bytes)
                        .expect("decode revision golden"),
                    revision,
                    "{} decode",
                    case.id
                );
            }
            "plan_revision_payload" => {
                let value = PlanRevisionPayload {
                    title_snapshot_bytes: golden_text(input, "title_snapshot").into_bytes(),
                    summary_bytes: golden_text(input, "summary").into_bytes(),
                    artifact_path_bytes: golden_text(input, "artifact_path").into_bytes(),
                    artifact_selector_bytes: golden_text(input, "artifact_selector").into_bytes(),
                    artifact_heading_bytes: golden_text(input, "artifact_heading").into_bytes(),
                    artifact_blob_id_bytes: golden_text(input, "artifact_blob_id").into_bytes(),
                };
                assert_eq!(
                    PlanRevisionCodec::<PLAN_LAYOUT_ID>::encode_payload(&value)
                        .expect("encode revision payload"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    PlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_payload(&case.expected_bytes)
                        .expect("decode revision payload golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            "plan_item_record" => {
                let value = PlanItemRecord {
                    item_meta: golden_u8(input, "item_meta"),
                    reserved0: golden_u8(input, "reserved0"),
                    payload_len: golden_u16(input, "payload_len"),
                    payload_offset: golden_hex_u64(input, "payload_offset_hex"),
                    line_number: golden_u32(input, "line_number"),
                };
                assert_eq!(
                    PlanItemCodec::<PLAN_LAYOUT_ID>::encode_record(&value).expect("encode item"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    PlanItemCodec::<PLAN_LAYOUT_ID>::decode_record(&case.expected_bytes)
                        .expect("decode item golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            "plan_item_payload" => {
                let value = PlanItemPayload {
                    plan_item_ref_bytes: golden_text(input, "plan_item_ref").into_bytes(),
                    text_bytes: golden_text(input, "text").into_bytes(),
                    heading_path: input["heading_path"]
                        .as_array()
                        .expect("heading_path must be an array")
                        .iter()
                        .map(|part| {
                            part.as_str()
                                .expect("heading_path entry must be text")
                                .to_string()
                        })
                        .collect(),
                };
                assert_eq!(
                    PlanItemCodec::<PLAN_LAYOUT_ID>::encode_payload(&value)
                        .expect("encode item payload"),
                    case.expected_bytes,
                    "{} encode",
                    case.id
                );
                assert_eq!(
                    PlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(&case.expected_bytes)
                        .expect("decode item payload golden"),
                    value,
                    "{} decode",
                    case.id
                );
            }
            kind => panic!("unsupported Plan golden case kind {kind}"),
        }
    }

    assert_eq!(
        executed,
        BTreeSet::from([
            "plan_record".to_string(),
            "plan_payload".to_string(),
            "plan_revision_record".to_string(),
            "plan_revision_payload".to_string(),
            "plan_item_record".to_string(),
            "plan_item_payload".to_string(),
        ])
    );
}
