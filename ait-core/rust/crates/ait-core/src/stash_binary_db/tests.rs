use super::*;
use crate::binary_db::{AuthorityId, BinaryDbCommandScope, LocalBinaryDbFs, LocalStateScope};
use crate::content_binary_db::{
    snapshot_hash48_from_id, snapshot_id_index_key, BinarySnapshotPayload,
};
use tempfile::tempdir;

type TestStashStore = BinaryDbStashStore<LocalBinaryDbFs, 1>;

fn new_store() -> (TestStashStore, tempfile::TempDir) {
    let temp = tempdir().unwrap();
    let db = LocalBinaryDbFs::new(
        temp.path(),
        temp.path(),
        AuthorityId::new("stash-test"),
        LocalStateScope::Repository,
    );
    (BinaryDbStashStore::new(db), temp)
}

fn append_snapshot(
    store: &TestStashStore,
    snapshot_id: &str,
    parent_snapshot_index_plus1: u32,
    snapshot_kind: BinarySnapshotKind,
    created_at_s: u64,
    message: Option<&str>,
) -> u32 {
    let mut write =
        BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::ContentWrite).unwrap();
    let snapshot_meta = match snapshot_kind {
        BinarySnapshotKind::Line => 0,
        BinarySnapshotKind::Stash => 1,
        BinarySnapshotKind::Reserved(value) => value,
    };
    let snapshots = BinaryDbSnapshotStore::<_, 1>::new(store.db().clone(), ".");
    let (index, actual_id, _) = snapshots
        .append_snapshot_with_id_index(
            &mut write,
            BinarySnapshotRecord {
                snapshot_meta,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: snapshot_hash48_from_id(snapshot_id).unwrap(),
                parent_snapshot_index_plus1,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: 0,
                manifest_hash: [0; 32],
                file_count: 3,
                total_bytes: 42,
                created_at_s,
            },
            &BinarySnapshotPayload {
                line_name: "feature/demo".to_string(),
                message: message.map(str::to_string),
                additional_parent_snapshot_indices: Vec::new(),
            },
        )
        .unwrap();
    assert_eq!(actual_id, snapshot_id);
    write.commit().unwrap();
    index
}

#[test]
fn stash_codec_is_exactly_eight_bytes_and_rejects_reserved_state() {
    let record = BinaryStashRecord {
        stash_meta: BinaryStashRecord::META_WORKSPACE_CLEARED,
        reserved0: 0,
        reserved1: 0,
        stash_snapshot_index: 0x1122_3344,
    };
    let bytes = BinaryStashCodec::<1>::encode_record(&record).unwrap();
    assert_eq!(bytes, [1, 0, 0, 0, 0x44, 0x33, 0x22, 0x11]);
    assert_eq!(
        BinaryStashCodec::<1>::decode_record(&bytes).unwrap(),
        record
    );
    assert!(BinaryStashCodec::<1>::decode_record(&[0; 7]).is_err());

    let mut invalid = record.clone();
    invalid.stash_meta |= 0b0000_0010;
    assert!(BinaryStashCodec::<1>::encode_record(&invalid)
        .unwrap_err()
        .contains("unsupported stash_meta bits"));
    invalid = record.clone();
    invalid.reserved1 = 1;
    assert!(BinaryStashCodec::<1>::encode_record(&invalid)
        .unwrap_err()
        .contains("reserved1 must be zero"));
}

#[test]
fn stash_ids_are_derived_exactly_from_immutable_record_indexes() {
    assert_eq!(stash_id_from_index(0).unwrap(), "STH-000001");
    assert_eq!(stash_id_from_index(999_999).unwrap(), "STH-1000000");
    assert_eq!(stash_index_from_id("STH-000001"), Some(0));
    assert_eq!(stash_index_from_id("STH-1000000"), Some(999_999));
    assert_eq!(stash_index_from_id("STH-1"), None);
    assert_eq!(stash_index_from_id("sth-000001"), None);
    assert_eq!(stash_index_from_id("STH-000000"), None);
    assert_eq!(stash_index_from_id("STH-00001A"), None);
}

#[test]
fn binary_stash_store_derives_snapshot_fields_and_tombstones_without_shifting() {
    let (store, temp) = new_store();
    let base_index = append_snapshot(
        &store,
        "SNP-000000000001",
        0,
        BinarySnapshotKind::Line,
        10,
        None,
    );
    let stash_snapshot_index = append_snapshot(
        &store,
        "SNP-000000000002",
        base_index + 1,
        BinarySnapshotKind::Stash,
        20,
        Some("park work"),
    );
    assert_eq!(stash_snapshot_index, 1);

    let new_record = NewStashRecord {
        stash_id: "STH-IGNORED",
        snapshot_id: "SNP-000000000002",
        source_line_name: "feature/demo",
        base_snapshot_id: Some("SNP-000000000001"),
        message: Some("park work"),
        workspace_cleared: true,
        created_at: "ignored",
    };
    let first = store.create_stash(new_record).unwrap();
    let second = store.create_stash(new_record).unwrap();
    assert_eq!(first.stash_id, "STH-000001");
    assert_eq!(second.stash_id, "STH-000002");
    assert_eq!(first.source_line_name, "feature/demo");
    assert_eq!(first.message.as_deref(), Some("park work"));
    assert_eq!(first.base_snapshot_id.as_deref(), Some("SNP-000000000001"));
    assert!(first.workspace_cleared);
    assert_eq!(first.file_count, 3);
    assert_eq!(first.total_bytes, 42);
    assert_eq!(store.list_stashes().unwrap().len(), 2);
    assert_eq!(
        std::fs::metadata(temp.path().join(STASH_BIN))
            .unwrap()
            .len(),
        4 + 16
    );

    let dropped_first = store.drop_stash("STH-000001").unwrap().unwrap();
    assert!(!dropped_first.snapshot_deleted);
    assert!(store.stash_by_id("STH-000001").unwrap().is_none());
    assert!(store.stash_by_id("STH-000002").unwrap().is_some());

    let dropped_second = store.drop_stash("STH-000002").unwrap().unwrap();
    assert!(dropped_second.snapshot_deleted);
    assert!(store.list_stashes().unwrap().is_empty());
    assert_eq!(
        std::fs::metadata(temp.path().join(STASH_BIN))
            .unwrap()
            .len(),
        4 + 16
    );

    let read = BinaryDbReadTxn::new(store.db());
    let candidates = read
        .lookup_index(
            BinarySnapshotCodec::<1>::id_index(),
            &snapshot_id_index_key("SNP-000000000002").unwrap(),
        )
        .unwrap();
    assert_eq!(candidates, [1]);
    let snapshot = BinarySnapshotCodec::<1>::decode_record(
        &read
            .read_record(BinarySnapshotCodec::<1>::record_file(), 1)
            .unwrap(),
    )
    .unwrap();
    assert!(snapshot.is_tombstone());
}

#[test]
fn binary_stash_store_rejects_non_stash_snapshot_references() {
    let (store, _temp) = new_store();
    append_snapshot(
        &store,
        "SNP-000000000001",
        0,
        BinarySnapshotKind::Line,
        10,
        None,
    );
    let error = store
        .create_stash(NewStashRecord {
            stash_id: "STH-000001",
            snapshot_id: "SNP-000000000001",
            source_line_name: "feature/demo",
            base_snapshot_id: None,
            message: None,
            workspace_cleared: false,
            created_at: "ignored",
        })
        .unwrap_err();
    assert!(error.contains("is not a stash snapshot"));
    assert!(!store
        .db()
        .authority_root()
        .as_path()
        .join(STASH_BIN)
        .exists());
}

#[test]
fn binary_stash_store_rejects_a_message_that_cannot_be_derived_exactly() {
    let (store, _temp) = new_store();
    append_snapshot(
        &store,
        "SNP-000000000001",
        0,
        BinarySnapshotKind::Stash,
        10,
        Some("snapshot message"),
    );
    let error = store
        .create_stash(NewStashRecord {
            stash_id: "STH-000001",
            snapshot_id: "SNP-000000000001",
            source_line_name: "feature/demo",
            base_snapshot_id: None,
            message: None,
            workspace_cleared: false,
            created_at: "ignored",
        })
        .unwrap_err();
    assert!(error.contains("stash message does not match snapshot"));
    assert!(!store
        .db()
        .authority_root()
        .as_path()
        .join(STASH_BIN)
        .exists());
}
