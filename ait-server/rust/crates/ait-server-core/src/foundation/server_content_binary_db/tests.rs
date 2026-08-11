use super::*;
use crate::foundation::remote_binary_db::{
    BinaryDb, BinaryDbFsyncPolicy, BinaryDbNoopFsyncPolicy, FilesystemServerRemoteBinaryDb, RepoId,
    RepoName, StoreGeneration, StorePath,
};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

type TestDb = FilesystemServerRemoteBinaryDb;
type TestLineStore = ServerBinaryDbLineStore<TestDb, 1>;
type TestSnapshotStore = ServerBinaryDbSnapshotStore<TestDb, 1>;
type FutureLineStore = ServerBinaryDbLineStore<TestDb, 99>;
type FutureSnapshotStore = ServerBinaryDbSnapshotStore<TestDb, 99>;

#[derive(Clone, Default)]
struct RecordingFsyncPolicy {
    events: Rc<RefCell<Vec<String>>>,
}

impl RecordingFsyncPolicy {
    fn events(&self) -> Vec<String> {
        self.events.borrow().clone()
    }
}

impl BinaryDbFsyncPolicy for RecordingFsyncPolicy {
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.events
            .borrow_mut()
            .push(format!("file:{}", path.display()));
        Ok(())
    }

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        self.events
            .borrow_mut()
            .push(format!("data:{}", path.display()));
        Ok(())
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.events
            .borrow_mut()
            .push(format!("dir:{}", path.display()));
        Ok(())
    }
}

fn test_db(root: &std::path::Path) -> TestDb {
    FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("REPO-TEST"),
        RepoName::new("test"),
        StorePath::new(root),
        StoreGeneration::new(1),
    )
}

fn temporary_root() -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ait-server-content-binary-db-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
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

#[test]
fn fixed_v0_indexes_insert_in_key_then_target_order() -> StoreResult<()> {
    let root = temporary_root();
    fs::create_dir_all(&root).expect("create index fixture root");
    let db = test_db(&root);
    let index = ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::id_index();
    let mut write = crate::foundation::remote_binary_db::BinaryWriteContext::test_fixture(
        crate::foundation::remote_binary_db::BinaryDbCommandScope::ServerContent,
    );
    let high = 9_u64.to_le_bytes();
    let low = 1_u64.to_le_bytes();
    db.append_index_candidate(index.clone(), &high, 5, &mut write)?;
    db.append_index_candidate(index.clone(), &low, 2, &mut write)?;
    db.append_index_candidate(index.clone(), &low, 1, &mut write)?;

    let mut expected = SERVER_CONTENT_BINARY_LAYOUT_ID.to_le_bytes().to_vec();
    expected.extend_from_slice(&low);
    expected.extend_from_slice(&2_u32.to_le_bytes());
    expected.extend_from_slice(&low);
    expected.extend_from_slice(&3_u32.to_le_bytes());
    expected.extend_from_slice(&high);
    expected.extend_from_slice(&6_u32.to_le_bytes());
    assert_eq!(fs::read(db.resolve_index_path(&index)?).unwrap(), expected);
    assert_eq!(db.lookup_index(index.clone(), &low)?, vec![1, 2]);
    assert_eq!(db.lookup_index(index, &high)?, vec![5]);
    Ok(())
}

#[test]
fn remote_sync_write_set_is_journaled_in_one_durable_batch() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    let policy = RecordingFsyncPolicy::default();
    let mut tx = BinaryDbWriteTxn::begin_with_fsync_policy(
        &db,
        BinaryDbCommandScope::ServerRemoteSyncCommit,
        policy.clone(),
    )?;

    content.prepare_remote_sync_write_set(
        &mut tx,
        true,
        true,
        true,
        Some(ServerBinaryRemoteSyncLineWrite::Update),
    )?;
    content.prepare_remote_sync_write_set(
        &mut tx,
        true,
        true,
        true,
        Some(ServerBinaryRemoteSyncLineWrite::Update),
    )?;

    assert_eq!(tx.touched_files().len(), 17);
    for required in [
        "snapshot_parent_edge.bin",
        "tree_entry.bin",
        "tree_entry_range.bin",
        "tree_name_payload.bin",
    ] {
        assert!(tx
            .touched_files()
            .iter()
            .any(|path| { path.file_name().and_then(|name| name.to_str()) == Some(required) }));
    }
    assert_eq!(
        policy
            .events()
            .iter()
            .filter(|event| event.starts_with("data:"))
            .count(),
        1,
        "the complete remote-sync write set must require one journal data sync"
    );
    tx.abort()?;
    Ok(())
}

fn snapshot_record(hash48: u64) -> ServerBinarySnapshotRecord {
    ServerBinarySnapshotRecord {
        snapshot_meta: 0,
        history_flags: 0,
        payload_len: 0,
        payload_offset: 0,
        snapshot_hash48: hash48,
        parent_snapshot_index_plus1: 0,
        root_tree_pack_index_plus1: 0,
        root_entry_ordinal: 0,
        line_index_plus1: 0,
        manifest_hash: [0; 32],
        file_count: 0,
        total_bytes: 0,
        created_at_s: 1,
    }
}

#[test]
fn canonical_line_and_snapshot_codecs_match_core_golden_bytes() -> StoreResult<()> {
    let line = ServerBinaryLineRecord {
        line_meta: ServerBinaryLineRecord::META_ARCHIVED,
        reserved0: 0,
        line_name_len: 4,
        line_name_offset: 4,
        head_snapshot_index_plus1: 2,
        created_at_s: 3,
        updated_at_s: 5,
        archived_at_s: 5,
    };
    let line_bytes = ServerBinaryLineCodec::<1>::encode_record(&line)?;
    assert_eq!(
        line_bytes,
        decode_hex_fixture(include_str!(
            "../../../tests/fixtures/binary_db_layout1_line_record.hex"
        ))
    );
    assert_eq!(
        ServerBinaryLineCodec::<1>::decode_record(&line_bytes)?,
        line
    );

    let snapshot = ServerBinarySnapshotRecord {
        snapshot_meta: ServerBinarySnapshotRecord::META_HAS_ROOT_LOCATOR,
        history_flags: 0,
        payload_len: 8,
        payload_offset: 7,
        snapshot_hash48: 0x010203040506,
        parent_snapshot_index_plus1: 1,
        root_tree_pack_index_plus1: 2,
        root_entry_ordinal: 3,
        line_index_plus1: 4,
        manifest_hash: [9; 32],
        file_count: 4,
        total_bytes: 5,
        created_at_s: 6,
    };
    let snapshot_bytes = ServerBinarySnapshotCodec::<1>::encode_record(&snapshot)?;
    assert_eq!(
        snapshot_bytes,
        decode_hex_fixture(include_str!(
            "../../../tests/fixtures/binary_db_layout1_snapshot_record.hex"
        ))
    );
    assert_eq!(
        ServerBinarySnapshotCodec::<1>::decode_record(&snapshot_bytes)?,
        snapshot
    );

    let payload = ServerBinarySnapshotPayload {
        line_name: "main".to_string(),
        message: Some("seed".to_string()),
    };
    let payload_bytes = ServerBinarySnapshotCodec::<1>::encode_payload(&payload)?;
    assert_eq!(
        payload_bytes,
        decode_hex_fixture(include_str!(
            "../../../tests/fixtures/binary_db_layout1_snapshot_payload.hex"
        ))
    );
    assert_eq!(
        ServerBinarySnapshotCodec::<1>::decode_payload(&payload_bytes, true)?,
        payload
    );

    let mut line_index = server_line_name_hash64(b"main").to_le_bytes().to_vec();
    line_index.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(
        line_index,
        decode_hex_fixture(include_str!(
            "../../../tests/fixtures/binary_db_layout1_line_name_index.hex"
        ))
    );
    let mut snapshot_index = 0x010203040506_u64.to_le_bytes().to_vec();
    snapshot_index.extend_from_slice(&1_u32.to_le_bytes());
    assert_eq!(
        snapshot_index,
        decode_hex_fixture(include_str!(
            "../../../tests/fixtures/binary_db_layout1_snapshot_id_index.hex"
        ))
    );
    Ok(())
}

#[test]
fn canonical_codecs_fail_closed_for_noncanonical_layout_and_invalid_fields() {
    let line = ServerBinaryLineRecord {
        line_meta: 0,
        reserved0: 0,
        line_name_len: 4,
        line_name_offset: 4,
        head_snapshot_index_plus1: 0,
        created_at_s: 1,
        updated_at_s: 1,
        archived_at_s: 0,
    };
    assert!(ServerBinaryLineCodec::<99>::encode_record(&line)
        .unwrap_err()
        .contains("unsupported Binary DB line layout"));
    let mut invalid_line = line.clone();
    invalid_line.reserved0 = 1;
    assert!(ServerBinaryLineCodec::<1>::encode_record(&invalid_line)
        .unwrap_err()
        .contains("reserved0"));

    let mut snapshot = snapshot_record(1);
    snapshot.snapshot_meta = 0b0100_0000;
    assert!(ServerBinarySnapshotCodec::<1>::encode_record(&snapshot)
        .unwrap_err()
        .contains("reserved snapshot_meta"));
    snapshot.snapshot_meta = 0;
    snapshot.history_flags = 0b0000_0010;
    assert!(ServerBinarySnapshotCodec::<1>::encode_record(&snapshot)
        .unwrap_err()
        .contains("reserved history_flags"));
    assert!(server_snapshot_hash48_from_id("SNP-not-hex").is_err());
    assert!(ServerBinarySnapshotCodec::<1>::decode_record(&[0; 83]).is_err());
}

#[test]
fn server_content_reads_dispatch_from_persisted_layout() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let lines = TestLineStore::new(db.clone());
    let snapshots = TestSnapshotStore::new(db.clone());
    lines.create_line("main", 0, 10)?;
    let mut record = snapshot_record(1);
    record.line_index_plus1 = 1;
    let payload = ServerBinarySnapshotPayload {
        line_name: "main".to_string(),
        message: Some("persisted-v1".to_string()),
    };
    snapshots.append_snapshot("SNP-000000000001", record, &payload)?;

    let future_lines = FutureLineStore::new(db.clone());
    let future_snapshots = FutureSnapshotStore::new(db.clone());
    let read = BinaryDbReadTxn::new(&db);
    let (_, line) = future_lines
        .line_by_name(&read, "main")?
        .expect("future writer facade must find persisted v1 line");
    assert_eq!(future_lines.line_name(&read, &line)?, "main");
    assert_eq!(future_lines.all_lines(&read)?.len(), 1);
    let (_, snapshot) = future_snapshots
        .snapshot_by_id(&read, "SNP-000000000001")?
        .expect("future writer facade must find persisted v1 snapshot");
    assert_eq!(
        future_snapshots.snapshot_payload(&read, &snapshot)?,
        payload
    );
    assert_eq!(future_snapshots.all_snapshots(&read)?.len(), 1);
    drop(read);

    let line_bytes_before = fs::read(root.join(SERVER_LINE_BIN)).expect("persisted line bytes");
    let error = future_lines
        .create_line("future", 0, 11)
        .expect_err("unsupported future write layout must fail before mutation");
    assert_eq!(error.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(error.contains("unsupported Binary DB line write layout"));
    assert_eq!(
        fs::read(root.join(SERVER_LINE_BIN)).expect("line bytes after rejected write"),
        line_bytes_before
    );
    assert!(!root.join("line.v99.bin").exists());
    Ok(())
}

#[test]
fn server_content_persisted_layout_errors_are_typed_and_fail_closed() -> StoreResult<()> {
    let unknown_root = temporary_root();
    fs::create_dir_all(&unknown_root).expect("unknown layout root");
    fs::write(unknown_root.join(SERVER_LINE_BIN), 99_u32.to_le_bytes())
        .expect("unknown line header");
    let unknown_db = test_db(&unknown_root);
    let unknown_read = BinaryDbReadTxn::new(&unknown_db);
    let unknown = FutureLineStore::new(unknown_db.clone())
        .all_lines(&unknown_read)
        .expect_err("unknown persisted line layout must fail");
    assert_eq!(unknown.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(unknown.contains("unsupported Binary DB line persisted layout: 99"));
    fs::write(unknown_root.join(SERVER_SNAPSHOT_BIN), 99_u32.to_le_bytes())
        .expect("unknown snapshot header");
    let unknown_snapshot = FutureSnapshotStore::new(unknown_db.clone())
        .all_snapshots(&unknown_read)
        .expect_err("unknown persisted snapshot layout must fail");
    assert_eq!(unknown_snapshot.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(unknown_snapshot.contains("unsupported Binary DB snapshot persisted layout: 99"));
    let truncated_root = temporary_root();
    fs::create_dir_all(&truncated_root).expect("truncated layout root");
    fs::write(truncated_root.join(SERVER_LINE_BIN), [1_u8, 0_u8]).expect("truncated line header");
    let truncated_db = test_db(&truncated_root);
    let truncated_read = BinaryDbReadTxn::new(&truncated_db);
    let truncated = FutureLineStore::new(truncated_db.clone())
        .all_lines(&truncated_read)
        .expect_err("truncated persisted line header must fail");
    assert_eq!(truncated.kind(), BinaryDbErrorKind::Corruption);

    let empty_root = temporary_root();
    fs::create_dir_all(&empty_root).expect("header-only root");
    fs::write(
        empty_root.join(SERVER_LINE_BIN),
        SERVER_CONTENT_BINARY_LAYOUT_ID.to_le_bytes(),
    )
    .expect("header-only line file");
    fs::write(
        empty_root.join(SERVER_SNAPSHOT_BIN),
        SERVER_CONTENT_BINARY_LAYOUT_ID.to_le_bytes(),
    )
    .expect("header-only snapshot file");
    let empty_db = test_db(&empty_root);
    let empty_read = BinaryDbReadTxn::new(&empty_db);
    assert!(FutureLineStore::new(empty_db.clone())
        .all_lines(&empty_read)?
        .is_empty());
    assert!(FutureSnapshotStore::new(empty_db.clone())
        .all_snapshots(&empty_read)?
        .is_empty());

    let missing_db = test_db(&temporary_root());
    let missing_read = BinaryDbReadTxn::new(&missing_db);
    assert!(FutureLineStore::new(missing_db.clone())
        .line_by_name(&missing_read, "main")?
        .is_none());
    assert!(FutureSnapshotStore::new(missing_db.clone())
        .snapshot_by_id(&missing_read, "SNP-000000000001")?
        .is_none());
    Ok(())
}

#[test]
fn server_content_mixed_layout_payload_fails_without_reinterpretation() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let snapshots = TestSnapshotStore::new(db.clone());
    snapshots.append_snapshot(
        "SNP-000000000001",
        snapshot_record(1),
        &ServerBinarySnapshotPayload {
            line_name: "main".to_string(),
            message: Some("mixed-layout".to_string()),
        },
    )?;
    let payload_path = root.join(SERVER_SNAPSHOT_PAYLOAD_BIN);
    let mut payload_bytes = fs::read(&payload_path).expect("snapshot payload bytes");
    payload_bytes[0..4].copy_from_slice(&99_u32.to_le_bytes());
    fs::write(&payload_path, payload_bytes).expect("replace snapshot payload header");

    let reader = FutureSnapshotStore::new(db.clone());
    let read = BinaryDbReadTxn::new(&db);
    let (_, snapshot) = reader
        .snapshot_by_id(&read, "SNP-000000000001")?
        .expect("v1 snapshot record remains readable");
    let error = reader
        .snapshot_payload(&read, &snapshot)
        .expect_err("mixed snapshot record/payload layouts must fail");
    assert_eq!(error.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(error.contains("existing=99, expected=1"));
    Ok(())
}

#[test]
fn canonical_stores_use_stable_line_index_and_fixed_index_bytes() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let snapshots = TestSnapshotStore::new(db.clone());
    let lines = TestLineStore::new(db.clone());
    let payload = ServerBinarySnapshotPayload {
        line_name: "main".to_string(),
        message: None,
    };
    assert_eq!(
        snapshots.append_snapshot("SNP-000000000001", snapshot_record(1), &payload,)?,
        0
    );
    assert_eq!(lines.create_line("main", 1, 10)?, 0);
    let updated = lines.set_line_head("main", 1, 11)?;
    assert_eq!(updated.updated_at_s, 11);
    let archived = lines.archive_line("main", 12)?;
    assert!(archived.is_archived());

    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(lines.line_by_name(&read, "main")?.unwrap().0, 0);
    let (snapshot_index, snapshot) = snapshots
        .snapshot_by_id(&read, "SNP-000000000001")?
        .unwrap();
    assert_eq!(snapshot_index, 0);
    assert_eq!(snapshots.snapshot_payload(&read, &snapshot)?, payload);
    assert_eq!(
        db.record_count(ServerBinaryLineCodec::<1>::record_file())?,
        1
    );
    assert_eq!(
        fs::metadata(root.join(SERVER_LINE_NAME_IDX)).unwrap().len(),
        4 + u64::from(SERVER_FIXED_INDEX_RECORD_SIZE)
    );
    assert_eq!(
        fs::metadata(root.join(SERVER_SNAPSHOT_ID_IDX))
            .unwrap()
            .len(),
        4 + u64::from(SERVER_FIXED_INDEX_RECORD_SIZE)
    );
    Ok(())
}

#[test]
fn canonical_line_overwrite_abort_restores_before_image() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let lines = TestLineStore::new(db.clone());
    lines.create_line("main", 0, 10)?;
    let original = db.read_record(ServerBinaryLineCodec::<1>::record_file(), 0)?;
    let mut changed = ServerBinaryLineCodec::<1>::decode_record(&original)?;
    changed.updated_at_s = 11;
    let changed = ServerBinaryLineCodec::<1>::encode_record(&changed)?;

    let mut tx = BinaryDbWriteTxn::begin_with_fsync_policy(
        &db,
        BinaryDbCommandScope::ServerContent,
        BinaryDbNoopFsyncPolicy,
    )?;
    tx.overwrite_record(ServerBinaryLineCodec::<1>::record_file(), 0, &changed)?;
    tx.abort()?;

    assert_eq!(
        db.read_record(ServerBinaryLineCodec::<1>::record_file(), 0)?,
        original
    );
    Ok(())
}

#[test]
fn canonical_snapshot_rejects_line_index_payload_mismatch() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let lines = TestLineStore::new(db.clone());
    let snapshots = TestSnapshotStore::new(db);
    lines.create_line("main", 0, 10)?;
    let mut record = snapshot_record(1);
    record.line_index_plus1 = 1;
    let err = snapshots
        .append_snapshot(
            "SNP-000000000001",
            record,
            &ServerBinarySnapshotPayload {
                line_name: "other".to_string(),
                message: None,
            },
        )
        .expect_err("snapshot payload must match its line index");
    assert!(err.contains("does not match line index 0"));
    Ok(())
}

#[test]
fn canonical_line_cas_rejects_stale_state_under_server_content_lock() -> StoreResult<()> {
    let root = temporary_root();
    let db = test_db(&root);
    let snapshots = TestSnapshotStore::new(db.clone());
    let lines = TestLineStore::new(db.clone());
    snapshots.append_snapshot(
        "SNP-000000000001",
        snapshot_record(1),
        &ServerBinarySnapshotPayload {
            line_name: "main".to_string(),
            message: None,
        },
    )?;
    lines.create_line("main", 0, 10)?;
    let read = BinaryDbReadTxn::new(&db);
    let (_, stale) = lines.line_by_name(&read, "main")?.unwrap();
    drop(read);
    lines.set_line_head("main", 1, 11)?;

    let error = lines
        .set_line_head_if_current("main", &stale, 0, 12)
        .expect_err("stale line intent must fail under the write lock");

    assert_eq!(error.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(error.contains("advanced under the ServerContent write lock"));
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        lines
            .line_by_name(&read, "main")?
            .unwrap()
            .1
            .head_snapshot_index_plus1,
        1
    );
    Ok(())
}
