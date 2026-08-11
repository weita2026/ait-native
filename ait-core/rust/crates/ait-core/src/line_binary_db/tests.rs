use super::*;
use crate::binary_db::{AuthorityId, BinaryDbCommandScope, LocalBinaryDbFs, LocalStateScope};
use crate::content_binary_db::{
    snapshot_hash48_from_id, BinarySnapshotPayload, BinarySnapshotRecord,
};
use std::fs;
use tempfile::tempdir;

type TestLineStore = BinaryDbLineStore<LocalBinaryDbFs, 1>;
type FutureLineStore = BinaryDbLineStore<LocalBinaryDbFs, 99>;

fn new_store() -> (TestLineStore, tempfile::TempDir) {
    let temp_dir = tempdir().unwrap();
    let db = LocalBinaryDbFs::new(
        temp_dir.path(),
        temp_dir.path(),
        AuthorityId::new("test-authority"),
        LocalStateScope::Repository,
    );
    (BinaryDbLineStore::new(db), temp_dir)
}

fn decode_hex_fixture(value: &str) -> Vec<u8> {
    value
        .trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).expect("fixture hex byte")
        })
        .collect()
}

fn append_snapshot(store: &TestLineStore, snapshot_id: &str) {
    let payload = BinarySnapshotPayload {
        line_name: "main".to_string(),
        message: None,
        additional_parent_snapshot_indices: Vec::new(),
    };
    let payload_bytes = BinarySnapshotCodec::<1>::encode_payload(&payload).unwrap();
    let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::ContentWrite).unwrap();
    let range = tx
        .append_payload(BinarySnapshotCodec::<1>::payload_file(), &payload_bytes)
        .unwrap();
    let record = BinarySnapshotRecord {
        snapshot_meta: BinarySnapshotRecord::META_HAS_LINE_NAME_PAYLOAD,
        history_flags: 0,
        payload_len: u16::try_from(range.payload_len).unwrap(),
        payload_offset: range.payload_offset,
        snapshot_hash48: snapshot_hash48_from_id(snapshot_id).unwrap(),
        parent_snapshot_index_plus1: 0,
        root_tree_pack_index_plus1: 0,
        root_entry_ordinal: 0,
        line_index_plus1: 0,
        manifest_hash: [0; 32],
        file_count: 0,
        total_bytes: 0,
        created_at_s: 1,
    };
    let bytes = BinarySnapshotCodec::<1>::encode_record(&record).unwrap();
    let index = tx
        .append_record(BinarySnapshotCodec::<1>::record_file(), &bytes)
        .unwrap();
    tx.append_index_candidate(
        BinarySnapshotCodec::<1>::id_index(),
        &snapshot_id_index_key(snapshot_id).unwrap(),
        index,
    )
    .unwrap();
    tx.commit().unwrap();
}

#[test]
fn line_identity_survives_rename_and_delete_tombstones_only_the_ref() {
    let (store, _temp_dir) = new_store();
    append_snapshot(&store, "SNP-123456789ABC");
    let created = store
        .create_line(
            "topic/old",
            Some("SNP-123456789ABC"),
            "2026-07-19T00:00:01Z",
        )
        .expect("create line");
    assert_eq!(created.line_id, "LNE-00000001");

    let renamed = store
        .rename_line("topic/old", "topic/new", "2026-07-19T00:00:02Z")
        .expect("rename line");
    assert_eq!(renamed.line_id, created.line_id);
    assert_eq!(renamed.head_snapshot_id, created.head_snapshot_id);
    assert!(store.line_by_name("topic/old").unwrap().is_none());
    assert_eq!(
        store.line_by_name("topic/new").unwrap().unwrap().line_id,
        created.line_id
    );

    store
        .create_line("occupied", None, "2026-07-19T00:00:03Z")
        .expect("create collision target");
    let collision = store
        .rename_line("topic/new", "occupied", "2026-07-19T00:00:04Z")
        .expect_err("collision must fail");
    assert!(collision.contains("Line already exists: occupied"));
    assert_eq!(
        store.line_by_name("topic/new").unwrap().unwrap().line_id,
        created.line_id
    );

    let deleted = store
        .delete_line("topic/new", "2026-07-19T00:00:05Z")
        .expect("delete line ref");
    assert_eq!(deleted.line_id, created.line_id);
    assert_eq!(deleted.status, "deleted");
    assert_eq!(
        deleted.head_snapshot_id.as_deref(),
        Some("SNP-123456789ABC")
    );
    assert!(store.line_by_name("topic/new").unwrap().is_none());
    assert!(store
        .list_lines()
        .unwrap()
        .iter()
        .all(|line| line.line_id != created.line_id));
    let read = BinaryDbReadTxn::new(store.db());
    assert_eq!(
        binary_line_name_at(&read, 0).expect("historical Snapshot line identity remains readable"),
        "topic/new"
    );
    drop(read);

    let recreated = store
        .create_line(
            "topic/new",
            Some("SNP-123456789ABC"),
            "2026-07-19T00:00:06Z",
        )
        .expect("recreate deleted name using preserved Snapshot");
    assert_ne!(recreated.line_id, created.line_id);
    assert_eq!(
        recreated.head_snapshot_id.as_deref(),
        Some("SNP-123456789ABC")
    );
}

#[test]
fn line_count_reads_logical_fixed_records_without_counting_tombstones() {
    let (store, _temp_dir) = new_store();
    assert_eq!(store.line_count().unwrap(), 0);

    store
        .create_line("active", None, "2026-07-19T00:00:01Z")
        .expect("create active line");
    store
        .create_line("archived", None, "2026-07-19T00:00:02Z")
        .expect("create archived line");
    store
        .archive_line("archived", "2026-07-19T00:00:03Z")
        .expect("archive line");
    store
        .create_line("deleted", None, "2026-07-19T00:00:04Z")
        .expect("create line to delete");

    assert_eq!(store.line_count().unwrap(), 3);
    store
        .delete_line("deleted", "2026-07-19T00:00:05Z")
        .expect("tombstone line");
    assert_eq!(store.line_count().unwrap(), 2);
    assert_eq!(store.list_lines().unwrap().len(), 2);
}

#[test]
fn canonical_line_codec_has_fixed_golden_bytes() {
    let record = BinaryLineRecord {
        line_meta: BinaryLineRecord::META_ARCHIVED,
        reserved0: 0,
        line_name_len: 4,
        line_name_offset: 4,
        head_snapshot_index_plus1: 2,
        created_at_s: 3,
        updated_at_s: 5,
        archived_at_s: 5,
    };
    let bytes = BinaryLineCodec::<1>::encode_record(&record).unwrap();
    assert_eq!(
        bytes,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/binary_db_layout1_line_record.hex"
        ))
    );
    assert_eq!(BinaryLineCodec::<1>::decode_record(&bytes).unwrap(), record);
    assert!(BinaryLineCodec::<2>::encode_record(&record)
        .unwrap_err()
        .contains("unsupported Binary DB line layout"));
    let mut index_bytes = line_name_hash64(b"main").to_le_bytes().to_vec();
    index_bytes.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(
        index_bytes,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/binary_db_layout1_line_name_index.hex"
        ))
    );
}

#[test]
fn line_codec_round_trips_full_u64_seconds_and_projection_fails_closed() {
    for seconds in [u64::from(u32::MAX) + 1, u64::MAX] {
        let record = BinaryLineRecord {
            line_meta: BinaryLineRecord::META_ARCHIVED,
            reserved0: 0,
            line_name_len: 4,
            line_name_offset: 4,
            head_snapshot_index_plus1: 0,
            created_at_s: seconds,
            updated_at_s: seconds,
            archived_at_s: seconds,
        };
        let bytes = BinaryLineCodec::<1>::encode_record(&record).unwrap();
        assert_eq!(bytes.len(), LINE_RECORD_SIZE as usize);
        assert_eq!(BinaryLineCodec::<1>::decode_record(&bytes).unwrap(), record);
    }

    assert!(format_epoch_seconds(u64::MAX).is_err());
}

#[test]
fn core_line_reads_dispatch_from_persisted_layout_not_write_layout() {
    let (store, temp_dir) = new_store();
    store
        .append_line_for_bootstrap(
            "main",
            "active",
            Some("2026-06-20T00:00:00Z"),
            Some("2026-06-20T00:00:00Z"),
            None,
            None,
        )
        .unwrap();
    let line_before = fs::read(temp_dir.path().join(LINE_BIN)).unwrap();
    let index_before = fs::read(temp_dir.path().join(LINE_NAME_IDX)).unwrap();
    let payload_before = fs::read(temp_dir.path().join(LINE_NAME_PAYLOAD_BIN)).unwrap();
    let future = FutureLineStore::new(store.db().clone());

    assert_eq!(future.list_lines().unwrap().len(), 1);
    assert_eq!(
        future.line_by_name("main").unwrap().unwrap().line_name,
        "main"
    );
    let read = BinaryDbReadTxn::new(future.db());
    assert_eq!(
        future.line_index_by_name_with_read(&read, "main").unwrap(),
        Some(0)
    );
    assert_eq!(binary_line_name_at(&read, 0).unwrap(), "main");
    drop(read);
    assert_eq!(
        fs::read(temp_dir.path().join(LINE_BIN)).unwrap(),
        line_before
    );
    assert_eq!(
        fs::read(temp_dir.path().join(LINE_NAME_IDX)).unwrap(),
        index_before
    );
    assert_eq!(
        fs::read(temp_dir.path().join(LINE_NAME_PAYLOAD_BIN)).unwrap(),
        payload_before
    );

    let error = future
        .create_line("future", None, "2026-06-20T00:00:01Z")
        .expect_err("unsupported writer layout must fail before mutation");
    assert!(error.contains("unsupported Binary DB line layout: 99"));
    assert_eq!(
        fs::read(temp_dir.path().join(LINE_BIN)).unwrap(),
        line_before
    );
}

#[test]
fn core_line_persisted_layout_failures_are_typed_and_non_mutating() {
    let unknown_dir = tempdir().unwrap();
    fs::write(unknown_dir.path().join(LINE_BIN), 99_u32.to_le_bytes()).unwrap();
    let unknown_db = LocalBinaryDbFs::new(
        unknown_dir.path(),
        unknown_dir.path(),
        AuthorityId::new("unknown-layout"),
        LocalStateScope::Repository,
    );
    let unknown_read = BinaryDbReadTxn::new(&unknown_db);
    let unknown = binary_line_index_by_name(&unknown_read, "main")
        .expect_err("unknown persisted Line layout must fail");
    assert_eq!(unknown.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(unknown.contains("unsupported persisted Binary DB line layout: 99"));

    let truncated_dir = tempdir().unwrap();
    fs::write(truncated_dir.path().join(LINE_BIN), [1_u8, 0_u8]).unwrap();
    let truncated_db = LocalBinaryDbFs::new(
        truncated_dir.path(),
        truncated_dir.path(),
        AuthorityId::new("truncated-layout"),
        LocalStateScope::Repository,
    );
    let truncated_read = BinaryDbReadTxn::new(&truncated_db);
    let truncated = binary_line_index_by_name(&truncated_read, "main")
        .expect_err("truncated persisted Line header must fail");
    assert_eq!(truncated.kind(), BinaryDbErrorKind::Corruption);

    let header_only_dir = tempdir().unwrap();
    fs::write(
        header_only_dir.path().join(LINE_BIN),
        BINARY_DB_LINE_LAYOUT_ID.to_le_bytes(),
    )
    .unwrap();
    let header_only_db = LocalBinaryDbFs::new(
        header_only_dir.path(),
        header_only_dir.path(),
        AuthorityId::new("header-only-layout"),
        LocalStateScope::Repository,
    );
    assert!(FutureLineStore::new(header_only_db)
        .list_lines()
        .unwrap()
        .is_empty());

    let missing_dir = tempdir().unwrap();
    let missing_db = LocalBinaryDbFs::new(
        missing_dir.path(),
        missing_dir.path(),
        AuthorityId::new("missing-layout"),
        LocalStateScope::Repository,
    );
    assert!(FutureLineStore::new(missing_db)
        .list_lines()
        .unwrap()
        .is_empty());
}

#[test]
fn core_line_rejects_mixed_primary_index_and_payload_layouts() {
    let (store, temp_dir) = new_store();
    store
        .append_line_for_bootstrap(
            "main",
            "active",
            Some("2026-06-20T00:00:00Z"),
            Some("2026-06-20T00:00:00Z"),
            None,
            None,
        )
        .unwrap();
    let index_path = temp_dir.path().join(LINE_NAME_IDX);
    let original_index = fs::read(&index_path).unwrap();
    let mut mixed_index = original_index.clone();
    mixed_index[0..4].copy_from_slice(&99_u32.to_le_bytes());
    fs::write(&index_path, &mixed_index).unwrap();
    let read = BinaryDbReadTxn::new(store.db());
    let index_error = binary_line_index_by_name(&read, "main")
        .expect_err("mixed Line primary/index layout must fail");
    assert_eq!(index_error.kind(), BinaryDbErrorKind::LayoutMismatch);
    drop(read);
    fs::write(&index_path, original_index).unwrap();

    let payload_path = temp_dir.path().join(LINE_NAME_PAYLOAD_BIN);
    let mut mixed_payload = fs::read(&payload_path).unwrap();
    mixed_payload[0..4].copy_from_slice(&99_u32.to_le_bytes());
    fs::write(&payload_path, mixed_payload).unwrap();
    let read = BinaryDbReadTxn::new(store.db());
    let payload_error =
        binary_line_name_at(&read, 0).expect_err("mixed Line primary/payload layout must fail");
    assert_eq!(payload_error.kind(), BinaryDbErrorKind::LayoutMismatch);
}

#[test]
fn canonical_line_store_keeps_stable_index_and_fixed_index_bytes() {
    let (store, temp_dir) = new_store();
    append_snapshot(&store, "SNP-000000000001");
    append_snapshot(&store, "SNP-000000000002");

    let created = store
        .create_line("main", Some("SNP-000000000001"), "2026-06-20T00:00:00Z")
        .unwrap();
    assert_eq!(
        created.head_snapshot_id.as_deref(),
        Some("SNP-000000000001")
    );

    store
        .set_line_head("main", Some("SNP-000000000002"), "2026-06-20T00:00:01Z")
        .unwrap();
    store
        .touch_line_updated_at("main", "2026-06-20T00:00:02Z")
        .unwrap();
    let archived = store.archive_line("main", "2026-06-20T00:00:03Z").unwrap();

    assert_eq!(archived.status, "archived");
    assert_eq!(
        archived.head_snapshot_id.as_deref(),
        Some("SNP-000000000002")
    );
    assert_eq!(store.list_lines().unwrap().len(), 1);
    assert_eq!(
        store.db().record_count(TestLineStore::line_file()).unwrap(),
        1
    );
    assert_eq!(
        fs::metadata(temp_dir.path().join(LINE_BIN)).unwrap().len(),
        4 + 40
    );
    assert_eq!(
        fs::metadata(temp_dir.path().join(LINE_NAME_IDX))
            .unwrap()
            .len(),
        4 + u64::from(LINE_NAME_INDEX_RECORD_SIZE)
    );
    assert_eq!(
        fs::metadata(temp_dir.path().join("snapshot_id.idx"))
            .unwrap()
            .len(),
        4 + 2 * 12
    );
}

#[test]
fn line_head_compare_and_swap_is_atomic_and_preserves_mismatched_head() {
    let (store, _temp_dir) = new_store();
    append_snapshot(&store, "SNP-000000000001");
    append_snapshot(&store, "SNP-000000000002");
    store
        .create_line("main", Some("SNP-000000000001"), "2026-06-20T00:00:00Z")
        .unwrap();

    let error = store
        .compare_and_swap_line_head(
            "main",
            Some("SNP-000000000002"),
            Some("SNP-000000000002"),
            "2026-06-20T00:00:01Z",
        )
        .expect_err("stale expected head must fail");
    assert!(error.contains("expected head SNP-000000000002"));
    assert_eq!(
        store
            .line_by_name("main")
            .unwrap()
            .unwrap()
            .head_snapshot_id
            .as_deref(),
        Some("SNP-000000000001")
    );

    let updated = store
        .compare_and_swap_line_head(
            "main",
            Some("SNP-000000000001"),
            Some("SNP-000000000002"),
            "2026-06-20T00:00:02Z",
        )
        .unwrap();
    assert_eq!(
        updated.head_snapshot_id.as_deref(),
        Some("SNP-000000000002")
    );
}

#[test]
fn line_name_hash_candidates_require_exact_payload_match() {
    let (store, _temp_dir) = new_store();
    let main_index = store
        .append_line_for_bootstrap(
            "main",
            "active",
            Some("2026-06-20T00:00:00Z"),
            Some("2026-06-20T00:00:00Z"),
            None,
            None,
        )
        .unwrap();
    let other_index = store
        .append_line_for_bootstrap(
            "other",
            "active",
            Some("2026-06-20T00:00:00Z"),
            Some("2026-06-20T00:00:00Z"),
            None,
            None,
        )
        .unwrap();
    let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::ContentWrite).unwrap();
    tx.append_index_candidate(
        TestLineStore::line_name_index(),
        &line_name_hash64(b"main").to_le_bytes(),
        other_index,
    )
    .unwrap();
    tx.commit().unwrap();

    let read = BinaryDbReadTxn::new(store.db());
    assert_eq!(
        store.line_index_by_name_with_read(&read, "main").unwrap(),
        Some(main_index)
    );
}

#[test]
fn fixed_record_overwrite_abort_restores_before_image() {
    let (store, _temp_dir) = new_store();
    store
        .append_line_for_bootstrap(
            "main",
            "active",
            Some("2026-06-20T00:00:00Z"),
            Some("2026-06-20T00:00:00Z"),
            None,
            None,
        )
        .unwrap();
    let original = store
        .db()
        .read_record(TestLineStore::line_file(), 0)
        .unwrap();
    let mut changed = BinaryLineCodec::<1>::decode_record(&original).unwrap();
    changed.updated_at_s += 1;
    let changed = BinaryLineCodec::<1>::encode_record(&changed).unwrap();

    let mut tx = BinaryDbWriteTxn::begin(store.db(), BinaryDbCommandScope::ContentWrite).unwrap();
    tx.overwrite_record(TestLineStore::line_file(), 0, &changed)
        .unwrap();
    tx.abort().unwrap();

    assert_eq!(
        store
            .db()
            .read_record(TestLineStore::line_file(), 0)
            .unwrap(),
        original
    );
}

#[test]
fn canonical_line_reads_use_read_committed_guard() {
    let (store, _temp_dir) = new_store();
    let mut writer = store
        .db()
        .begin_write_txn(BinaryDbCommandScope::ContentWrite)
        .unwrap();
    assert!(store
        .list_lines()
        .unwrap_err()
        .contains("Binary DB writer is active"));
    writer.commit().unwrap();
    assert!(store.list_lines().is_ok());
}

#[test]
fn drifted_74_byte_line_files_require_regeneration() {
    let (store, temp_dir) = new_store();
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&[0; 74]);
    fs::write(temp_dir.path().join(LINE_BIN), bytes).unwrap();

    let error = store
        .list_lines()
        .expect_err("drifted file must fail closed");
    assert!(error.contains("invalid payload length for record size 40"));
}
