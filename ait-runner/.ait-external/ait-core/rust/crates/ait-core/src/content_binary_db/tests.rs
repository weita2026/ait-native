use super::*;
use crate::binary_db::{
    AuthorityId, BinaryDb, BinaryDbCommandLockSet, BinaryDbCommandScope, BinaryDbErrorKind,
    BinaryDbIndexAppender, BinaryDbNoopFsyncPolicy, BinaryDbWriteTxn, BinaryFileId, BinaryIndexId,
    BinaryIndexKeyRef, BinaryPayloadFileId, BinaryRecordBytes, BinaryRecordBytesRef,
    BinaryWriteContext, LocalBinaryDbFs, LocalStateScope, PayloadRange, RemoteBinaryDbFs, RepoId,
    RepoName, StorePath, StoreResult,
};
use crate::content_store::{BlobStore, ObjectPackStore, TreePackStore, TreeStore};
use crate::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};
use crate::line_binary_db::BinaryDbLineStore;
use crate::line_store::LineStore;
use crate::local_snapshot::{LocalSnapshotReadStore, LocalSnapshotTreeReadStore};
use crate::object_diff_ports::{BlobReader, SnapshotReader};
use crate::snapshot_store::SnapshotStore;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use tempfile::tempdir;

const TEST_LAYOUT: u32 = 1;

#[test]
fn content_codecs_round_trip_full_u64_seconds() {
    for seconds in [u64::from(u32::MAX) + 1, u64::MAX] {
        let blob = BinaryBlobRecord {
            blob_meta: 0,
            hash_kind: 0,
            reserved0: 0,
            size_bytes: 0,
            pack_member_index_plus1: 0,
            created_at_s: seconds,
            pruned_at_s: seconds,
            sha256: [0; 32],
        };
        let bytes = BinaryBlobCodec::<TEST_LAYOUT>::encode_record(&blob).unwrap();
        assert_eq!(
            BinaryBlobCodec::<TEST_LAYOUT>::decode_record(&bytes).unwrap(),
            blob
        );

        let snapshot = BinarySnapshotRecord {
            snapshot_meta: 0,
            history_flags: 0,
            payload_len: 0,
            payload_offset: 0,
            snapshot_hash48: 1,
            parent_snapshot_index_plus1: 0,
            root_tree_pack_index_plus1: 0,
            root_entry_ordinal: 0,
            line_index_plus1: 0,
            manifest_hash: [0; 32],
            file_count: 0,
            total_bytes: 0,
            created_at_s: seconds,
        };
        let bytes = BinarySnapshotCodec::<TEST_LAYOUT>::encode_record(&snapshot).unwrap();
        assert_eq!(
            BinarySnapshotCodec::<TEST_LAYOUT>::decode_record(&bytes).unwrap(),
            snapshot
        );

        let object_pack = BinaryObjectPackRecord {
            pack_meta: 0,
            pack_format_kind: 0,
            pack_hash_hi16: 0,
            pack_hash_lo32: 1,
            first_member_index: 0,
            member_count: 0,
            total_bytes: 0,
            created_at_s: seconds,
        };
        let bytes = BinaryObjectPackCodec::<TEST_LAYOUT>::encode_record(&object_pack).unwrap();
        assert_eq!(
            BinaryObjectPackCodec::<TEST_LAYOUT>::decode_record(&bytes).unwrap(),
            object_pack
        );

        let tree_pack = BinaryTreePackRecord {
            pack_meta: 0,
            pack_format_kind: 0,
            pack_hash_hi16: 0,
            pack_hash_lo32: 1,
            first_tree_index: 0,
            tree_count: 0,
            total_bytes: 0,
            created_at_s: seconds,
        };
        let bytes = BinaryTreePackCodec::<TEST_LAYOUT>::encode_record(&tree_pack).unwrap();
        assert_eq!(
            BinaryTreePackCodec::<TEST_LAYOUT>::decode_record(&bytes).unwrap(),
            tree_pack
        );
    }
}

fn split_hash48(value: u64) -> (u16, u32) {
    ((value >> 32) as u16, value as u32)
}

fn local_db(root: &std::path::Path) -> LocalBinaryDbFs {
    LocalBinaryDbFs::new(
        root.join("binary-db"),
        root,
        AuthorityId::new("test-authority"),
        LocalStateScope::Repository,
    )
}

#[derive(Clone)]
struct AdvanceObjectMemberBeforeWriteLockDb {
    inner: LocalBinaryDbFs,
    advance_once: Arc<AtomicBool>,
}

impl AdvanceObjectMemberBeforeWriteLockDb {
    fn new(inner: LocalBinaryDbFs) -> Self {
        Self {
            inner,
            advance_once: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl crate::binary_db::BinaryDbRecoveryIo for AdvanceObjectMemberBeforeWriteLockDb {
    fn recovery_truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        self.inner.recovery_truncate_file(path, len)
    }

    fn recovery_remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        self.inner.recovery_remove_file_if_exists(path)
    }
}

impl BinaryDb for AdvanceObjectMemberBeforeWriteLockDb {
    fn authority_root(&self) -> &StorePath {
        self.inner.authority_root()
    }

    fn acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        if command_scope
            .lock_file_names()
            .contains(&"content.write.lock")
            && self
                .advance_once
                .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            let mut seed =
                BinaryDbWriteTxn::begin(&self.inner, BinaryDbCommandScope::ContentWrite)?;
            let bytes = BinaryObjectPackMemberCodec::<TEST_LAYOUT>::encode_record(
                &BinaryObjectPackMemberRecord {
                    member_meta: 0,
                    delta_chain_depth: 0,
                    reserved0: 0,
                    pack_index: 0,
                    blob_index: 0,
                    base_blob_index_plus1: 0,
                },
            )?;
            seed.append_record(
                BinaryDbObjectPackStore::<LocalBinaryDbFs, TEST_LAYOUT>::object_pack_member_file(),
                &bytes,
            )?;
            seed.commit()?;
        }
        self.inner.acquire_command_lock(command_scope)
    }

    fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.inner.layout_id(file)
    }

    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.inner.record_count(file)
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<BinaryRecordBytes> {
        self.inner.read_record(file, record_index)
    }

    fn append_record(
        &self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32> {
        self.inner.append_record(file, record, write)
    }

    fn overwrite_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        self.inner
            .overwrite_record(file, record_index, record, write)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.inner.read_payload(file, offset, len)
    }

    fn append_payload(
        &self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<PayloadRange> {
        self.inner.append_payload(file, bytes, write)
    }

    fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        self.inner.lookup_index(index, key)
    }
}

impl BinaryDbIndexAppender for AdvanceObjectMemberBeforeWriteLockDb {
    fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        self.inner
            .append_index_candidate(index, key, record_index, write)
    }
}

#[derive(Clone)]
struct CountingWriteDb {
    inner: LocalBinaryDbFs,
    command_lock_calls: Arc<AtomicUsize>,
    sync_file_calls: Arc<AtomicUsize>,
}

impl CountingWriteDb {
    fn new(inner: LocalBinaryDbFs) -> Self {
        Self {
            inner,
            command_lock_calls: Arc::new(AtomicUsize::new(0)),
            sync_file_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn reset_counts(&self) {
        self.command_lock_calls.store(0, Ordering::SeqCst);
        self.sync_file_calls.store(0, Ordering::SeqCst);
    }

    fn command_lock_calls(&self) -> usize {
        self.command_lock_calls.load(Ordering::SeqCst)
    }

    fn sync_file_calls(&self) -> usize {
        self.sync_file_calls.load(Ordering::SeqCst)
    }
}

impl crate::binary_db::BinaryDbRecoveryIo for CountingWriteDb {
    fn recovery_truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        self.inner.recovery_truncate_file(path, len)
    }

    fn recovery_remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        self.inner.recovery_remove_file_if_exists(path)
    }
}

impl BinaryDb for CountingWriteDb {
    fn authority_root(&self) -> &StorePath {
        self.inner.authority_root()
    }

    fn acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        self.command_lock_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.acquire_command_lock(command_scope)
    }

    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.sync_file_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.sync_file(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.inner.sync_directory(path)
    }

    fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.inner.layout_id(file)
    }

    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.inner.record_count(file)
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<BinaryRecordBytes> {
        self.inner.read_record(file, record_index)
    }

    fn append_record(
        &self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32> {
        self.inner.append_record(file, record, write)
    }

    fn overwrite_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        self.inner
            .overwrite_record(file, record_index, record, write)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.inner.read_payload(file, offset, len)
    }

    fn append_payload(
        &self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<PayloadRange> {
        self.inner.append_payload(file, bytes, write)
    }

    fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        self.inner.lookup_index(index, key)
    }
}

impl BinaryDbIndexAppender for CountingWriteDb {
    fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        self.inner
            .append_index_candidate(index, key, record_index, write)
    }
}

#[derive(Clone)]
struct CountingSnapshotReadDb {
    inner: LocalBinaryDbFs,
    snapshot_record_reads: Arc<AtomicUsize>,
}

impl CountingSnapshotReadDb {
    fn new(inner: LocalBinaryDbFs) -> Self {
        Self {
            inner,
            snapshot_record_reads: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn reset_snapshot_record_reads(&self) {
        self.snapshot_record_reads.store(0, Ordering::SeqCst);
    }

    fn snapshot_record_reads(&self) -> usize {
        self.snapshot_record_reads.load(Ordering::SeqCst)
    }
}

impl crate::binary_db::BinaryDbRecoveryIo for CountingSnapshotReadDb {
    fn recovery_truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        self.inner.recovery_truncate_file(path, len)
    }

    fn recovery_remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        self.inner.recovery_remove_file_if_exists(path)
    }
}

impl BinaryDb for CountingSnapshotReadDb {
    fn authority_root(&self) -> &StorePath {
        self.inner.authority_root()
    }

    fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.inner.layout_id(file)
    }

    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.inner.record_count(file)
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<BinaryRecordBytes> {
        if file.relative_path().as_path() == Path::new(SNAPSHOT_BIN) {
            self.snapshot_record_reads.fetch_add(1, Ordering::SeqCst);
        }
        self.inner.read_record(file, record_index)
    }

    fn append_record(
        &self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32> {
        self.inner.append_record(file, record, write)
    }

    fn overwrite_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        self.inner
            .overwrite_record(file, record_index, record, write)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.inner.read_payload(file, offset, len)
    }

    fn append_payload(
        &self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<PayloadRange> {
        self.inner.append_payload(file, bytes, write)
    }

    fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        self.inner.lookup_index(index, key)
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn write_tree_pack_fixture(
    repo_root: &std::path::Path,
    tree_pack_id: &str,
    tree_id: &str,
    entries: &JsonValue,
) {
    let raw_payload = JsonCodec::encode_value(
        &json!({"tree_id": tree_id, "entries": entries}),
        JsonEncodeOptions::compact(),
    )
    .expect("tree pack payload json");
    let pack_path = repo_root.join(
        tree_pack_relative_path(
            tree_pack_id,
            crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .expect("tree pack path"),
    );
    crate::pack_substrate::write_tree_pack_archive_with_format(
        pack_path.to_str().expect("pack path utf8"),
        tree_pack_id,
        "2026-07-15T00:00:00Z",
        &json!([{
            "tree_id": tree_id,
            "entry_name": format!("trees/{tree_id}.json"),
            "entry_count": entries.as_array().expect("tree entries array").len(),
            "data": raw_payload,
        }]),
        crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write tree pack");
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

fn assert_header_u32(path: &std::path::Path, expected: u32) {
    let bytes = fs::read(path).expect("read binary db file");
    assert!(bytes.len() >= 4);
    assert_eq!(&bytes[0..4], expected.to_le_bytes().as_slice());
}

fn overwrite_header_u32(path: &std::path::Path, value: u32) {
    let mut bytes = fs::read(path).expect("read binary db file");
    assert!(bytes.len() >= 4);
    bytes[0..4].copy_from_slice(&value.to_le_bytes());
    fs::write(path, bytes).expect("rewrite binary db header");
}

#[test]
fn snapshot_numeric_line_identity_overrides_preserved_historical_name_payload() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let lines = BinaryDbLineStore::<_, TEST_LAYOUT>::new(db.clone());
    lines
        .create_line(
            "archive/rct-1202-pre-local-authority-recovery",
            None,
            "2026-07-26T00:00:00Z",
        )
        .expect("create numeric authoring Line");

    let snapshots = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let mut tx = snapshots
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::SnapshotWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin snapshot transaction");
    let (_, snapshot_id, persisted_record) = snapshots
        .append_snapshot_with_id_index(
            &mut tx,
            BinarySnapshotRecord {
                snapshot_meta: 0,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: 0x9AAB_44D4_9894,
                parent_snapshot_index_plus1: 0,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: 1,
                manifest_hash: sha256(b"historical line projection"),
                file_count: 0,
                total_bytes: 0,
                created_at_s: 1,
            },
            &BinarySnapshotPayload {
                line_name: "feature/rct-1202".to_string(),
                message: Some("preserved historical projection".to_string()),
                additional_parent_snapshot_indices: Vec::new(),
            },
        )
        .expect("append historical Snapshot projection");
    tx.commit().expect("commit snapshot transaction");

    let payload_before = db
        .read_payload(
            BinarySnapshotCodec::<TEST_LAYOUT>::payload_file(),
            persisted_record.payload_offset,
            u32::from(persisted_record.payload_len),
        )
        .expect("read preserved payload");
    let decoded_before = BinarySnapshotCodec::<TEST_LAYOUT>::decode_payload(
        &payload_before,
        persisted_record.has_line_name_payload(),
        persisted_record.has_additional_parents(),
    )
    .expect("decode preserved payload");
    assert_eq!(decoded_before.line_name, "feature/rct-1202");

    let public = snapshots
        .snapshot_by_id(&snapshot_id)
        .expect("read Snapshot")
        .expect("Snapshot exists");
    assert_eq!(
        public.line_name,
        "archive/rct-1202-pre-local-authority-recovery"
    );
    let listed = snapshots
        .list_line_snapshots()
        .expect("list Snapshots through numeric Line authority");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].line_name, public.line_name);

    let payload_after = db
        .read_payload(
            BinarySnapshotCodec::<TEST_LAYOUT>::payload_file(),
            persisted_record.payload_offset,
            u32::from(persisted_record.payload_len),
        )
        .expect("reread preserved payload");
    assert_eq!(payload_after, payload_before);
}

#[test]
fn snapshot_parent_reads_only_the_child_and_direct_parent_records() {
    let temp = tempdir().expect("tempdir");
    let inner = local_db(temp.path());
    let snapshots = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(inner.clone(), temp.path());
    let mut tx = BinaryDbWriteTxn::begin_with_fsync_policy(
        &inner,
        BinaryDbCommandScope::ContentWrite,
        BinaryDbNoopFsyncPolicy,
    )
    .expect("begin snapshot seed transaction");
    let mut snapshot_ids = Vec::new();
    for index in 0_u32..4 {
        let (_, snapshot_id, _) = snapshots
            .append_snapshot_with_id_index(
                &mut tx,
                BinarySnapshotRecord {
                    snapshot_meta: 0,
                    history_flags: 0,
                    payload_len: 0,
                    payload_offset: 0,
                    snapshot_hash48: 0x1000_0000 + u64::from(index),
                    parent_snapshot_index_plus1: index,
                    root_tree_pack_index_plus1: 0,
                    root_entry_ordinal: 0,
                    line_index_plus1: 0,
                    manifest_hash: sha256(format!("manifest-{index}").as_bytes()),
                    file_count: 0,
                    total_bytes: 0,
                    created_at_s: u64::from(index),
                },
                &BinarySnapshotPayload {
                    line_name: "main".to_string(),
                    message: Some(format!("snapshot {index}")),
                    additional_parent_snapshot_indices: Vec::new(),
                },
            )
            .expect("append snapshot");
        snapshot_ids.push(snapshot_id);
    }
    tx.commit().expect("commit snapshot seed transaction");
    drop(tx);

    let counting = CountingSnapshotReadDb::new(inner);
    let snapshots = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(counting.clone(), temp.path());
    let tail = snapshot_ids.last().expect("tail snapshot");
    counting.reset_snapshot_record_reads();
    let link = snapshots
        .snapshot_parent_link(tail)
        .expect("read direct parent")
        .expect("snapshot parent link");
    assert_eq!(link.snapshot_id, *tail);
    assert_eq!(
        link.parent_snapshot_id.as_deref(),
        Some(snapshot_ids[2].as_str())
    );
    assert_eq!(counting.snapshot_record_reads(), 2);

    counting.reset_snapshot_record_reads();
    assert_eq!(
        snapshots.snapshot_chain(tail).expect("snapshot chain"),
        snapshot_ids
    );
    assert_eq!(counting.snapshot_record_reads(), 4);
}

#[test]
fn content_binary_db_codec_record_sizes_round_trip() {
    let blob = BinaryBlobRecord {
        blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
        hash_kind: 0,
        reserved0: 0,
        size_bytes: 6,
        pack_member_index_plus1: 1,
        created_at_s: 10,
        pruned_at_s: 0,
        sha256: [7; 32],
    };
    let encoded = BinaryBlobCodec::<TEST_LAYOUT>::encode_record(&blob).expect("encode blob");
    assert_eq!(encoded.len(), BLOB_RECORD_SIZE_USIZE);
    assert_eq!(
        BinaryBlobCodec::<TEST_LAYOUT>::decode_record(&encoded).expect("decode blob"),
        blob
    );

    let snapshot = BinarySnapshotRecord {
        snapshot_meta: BinarySnapshotRecord::META_HAS_ROOT_LOCATOR,
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
    let encoded =
        BinarySnapshotCodec::<TEST_LAYOUT>::encode_record(&snapshot).expect("encode snapshot");
    assert_eq!(encoded.len(), SNAPSHOT_RECORD_SIZE_USIZE);
    let expected_snapshot_bytes = decode_hex_fixture(include_str!(
        "../../tests/fixtures/binary_db_layout1_snapshot_record.hex"
    ));
    assert_eq!(encoded, expected_snapshot_bytes);
    assert_eq!(
        BinarySnapshotCodec::<TEST_LAYOUT>::decode_record(&encoded).expect("decode snapshot"),
        snapshot
    );
    let mut reserved_snapshot_meta = snapshot.clone();
    reserved_snapshot_meta.snapshot_meta |= 0b0100_0000;
    assert!(
        BinarySnapshotCodec::<TEST_LAYOUT>::encode_record(&reserved_snapshot_meta)
            .expect_err("reserved Snapshot metadata bit must fail closed")
            .to_string()
            .contains("reserved snapshot_meta bits")
    );
    let mut boundary_snapshot = snapshot.clone();
    boundary_snapshot.history_flags = BinarySnapshotRecord::FLAG_REMOTE_HEAD_HISTORY_BOUNDARY;
    boundary_snapshot.parent_snapshot_index_plus1 = 0;
    let encoded_boundary = BinarySnapshotCodec::<TEST_LAYOUT>::encode_record(&boundary_snapshot)
        .expect("encode remote-head history boundary");
    assert_eq!(
        BinarySnapshotCodec::<TEST_LAYOUT>::decode_record(&encoded_boundary)
            .expect("decode remote-head history boundary"),
        boundary_snapshot
    );
    let mut unknown_history_flag = snapshot.clone();
    unknown_history_flag.history_flags = 0b1000_0000;
    assert!(
        BinarySnapshotCodec::<TEST_LAYOUT>::encode_record(&unknown_history_flag)
            .expect_err("unknown snapshot history flag must fail closed")
            .contains("unknown history flags")
    );
    let payload = BinarySnapshotPayload {
        line_name: "main".to_string(),
        message: Some("seed".to_string()),
        additional_parent_snapshot_indices: Vec::new(),
    };
    let encoded =
        BinarySnapshotCodec::<TEST_LAYOUT>::encode_payload(&payload).expect("encode payload");
    assert_eq!(
        encoded,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/binary_db_layout1_snapshot_payload.hex"
        ))
    );
    assert_eq!(
        BinarySnapshotCodec::<TEST_LAYOUT>::decode_payload(&encoded, true, false)
            .expect("decode payload"),
        payload
    );
    let extended_payload = BinarySnapshotPayload {
        line_name: "main".to_string(),
        message: Some("seed".to_string()),
        additional_parent_snapshot_indices: vec![0, 6],
    };
    let encoded_extended = BinarySnapshotCodec::<TEST_LAYOUT>::encode_payload(&extended_payload)
        .expect("encode structured parent extension");
    assert_eq!(
        encoded_extended,
        vec![1, 2, 0, 1, 0, 0, 0, 7, 0, 0, 0, 4, 0, b's', b'e', b'e', b'd', b'm', b'a', b'i', b'n']
    );
    assert_eq!(
        BinarySnapshotCodec::<TEST_LAYOUT>::decode_payload(&encoded_extended, true, true)
            .expect("decode structured parent extension"),
        extended_payload
    );
    assert!(
        BinarySnapshotCodec::<TEST_LAYOUT>::decode_payload(&[1, 0, 0, 0, 0], false, true)
            .expect_err("additional-parent flag with zero entries must fail closed")
            .contains("at least one additional parent")
    );
    assert!(BinarySnapshotCodec::<TEST_LAYOUT>::decode_payload(
        &[1, 1, 0, 0, 0, 0, 0, 0, 0],
        false,
        true,
    )
    .expect_err("zero plus-one parent index must fail closed")
    .contains("zero plus-one index"));
    assert!(BinarySnapshotCodec::<2>::encode_record(&snapshot)
        .expect_err("layout 2 must fail closed")
        .contains("unsupported Binary DB snapshot layout"));
    let mut snapshot_index_bytes = 0x010203040506_u64.to_le_bytes().to_vec();
    snapshot_index_bytes.extend_from_slice(&1_u32.to_le_bytes());
    assert_eq!(
        snapshot_index_bytes,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/binary_db_layout1_snapshot_id_index.hex"
        ))
    );

    let pack = BinaryObjectPackRecord {
        pack_meta: BinaryObjectPackRecord::META_READY,
        pack_format_kind: 1,
        pack_hash_hi16: 0x0102,
        pack_hash_lo32: 0x03040506,
        first_member_index: 2,
        member_count: 3,
        total_bytes: 123,
        created_at_s: 11,
    };
    let encoded = BinaryObjectPackCodec::<TEST_LAYOUT>::encode_record(&pack).expect("encode pack");
    assert_eq!(encoded.len(), OBJECT_PACK_RECORD_SIZE_USIZE);
    assert_eq!(
        BinaryObjectPackCodec::<TEST_LAYOUT>::decode_record(&encoded).expect("decode pack"),
        pack
    );
    let unsupported_pack = BinaryObjectPackRecord {
        pack_format_kind: 0,
        ..pack.clone()
    };
    assert_eq!(
        unsupported_pack.format_kind(),
        BinaryObjectPackFormatKind::Reserved(0)
    );
    assert!(object_pack_format_name(unsupported_pack.format_kind())
        .expect_err("non-current object pack kind must fail closed")
        .to_string()
        .contains("unsupported object pack format kind: 0"));

    let member = BinaryObjectPackMemberRecord {
        member_meta: 0,
        delta_chain_depth: 0,
        reserved0: 0,
        pack_index: 1,
        blob_index: 2,
        base_blob_index_plus1: 0,
    };
    let encoded =
        BinaryObjectPackMemberCodec::<TEST_LAYOUT>::encode_record(&member).expect("encode member");
    assert_eq!(encoded.len(), OBJECT_PACK_MEMBER_RECORD_SIZE_USIZE);
    assert_eq!(
        BinaryObjectPackMemberCodec::<TEST_LAYOUT>::decode_record(&encoded).expect("decode member"),
        member
    );
    let unsupported_member = BinaryObjectPackMemberRecord {
        member_meta: 1 << 2,
        ..member.clone()
    };
    assert_eq!(
        unsupported_member.compression_kind(),
        BinaryObjectPackCompressionKind::Reserved(1)
    );

    let tree_pack = BinaryTreePackRecord {
        pack_meta: BinaryTreePackRecord::META_READY,
        pack_format_kind: 1,
        pack_hash_hi16: 0x0A0B,
        pack_hash_lo32: 0x0C0D0E0F,
        first_tree_index: 4,
        tree_count: 5,
        total_bytes: 456,
        created_at_s: 12,
    };
    let encoded =
        BinaryTreePackCodec::<TEST_LAYOUT>::encode_record(&tree_pack).expect("encode tree pack");
    assert_eq!(encoded.len(), TREE_PACK_RECORD_SIZE_USIZE);
    assert_eq!(
        BinaryTreePackCodec::<TEST_LAYOUT>::decode_record(&encoded).expect("decode tree pack"),
        tree_pack
    );
    let unsupported_tree_pack = BinaryTreePackRecord {
        pack_format_kind: 0,
        ..tree_pack.clone()
    };
    assert_eq!(
        unsupported_tree_pack.format_kind(),
        BinaryTreePackFormatKind::Reserved(0)
    );
    assert!(tree_pack_format_name(unsupported_tree_pack.format_kind())
        .expect_err("non-current tree pack kind must fail closed")
        .to_string()
        .contains("unsupported tree pack format kind: 0"));

    let tree = BinaryTreeRecord {
        tree_meta: 0,
        reserved0: 0,
        pack_entry_ordinal: 6,
        entry_count: 7,
        tree_hash80: [9; 10],
    };
    let encoded = BinaryTreeCodec::<TEST_LAYOUT>::encode_record(&tree).expect("encode tree");
    assert_eq!(encoded.len(), TREE_RECORD_SIZE_USIZE);
    assert_eq!(
        BinaryTreeCodec::<TEST_LAYOUT>::decode_record(&encoded).expect("decode tree"),
        tree
    );
}

#[test]
fn content_binary_db_reads_blob_bytes_from_object_pack() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());

    let blob_bytes = b"hello\n";
    let blob_sha = sha256(blob_bytes);
    let blob_id = blob_id_from_sha256(&blob_sha);
    let pack_hash = 0x010203040506_u64;
    let (pack_hash_hi16, pack_hash_lo32) = split_hash48(pack_hash);
    let pack_id = object_pack_id_from_hash48(pack_hash);
    let pack_path = temp.path().join(
        object_pack_relative_path(&pack_id, crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1)
            .unwrap(),
    );
    crate::pack_substrate::write_pack_archive_with_format(
        pack_path.to_str().expect("pack path utf8"),
        &pack_id,
        "2026-07-05T00:00:00Z",
        &json!([{
            "entry_name": format!("blobs/{blob_id}"),
            "blob_id": blob_id,
            "data": blob_bytes.as_slice(),
        }]),
        crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write object pack");

    let mut tx = blob_store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin txn");
    let (pack_index, _) = object_pack_store
        .append_object_pack_with_id_index(
            &mut tx,
            &BinaryObjectPackRecord {
                pack_meta: BinaryObjectPackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16,
                pack_hash_lo32,
                first_member_index: 0,
                member_count: 1,
                total_bytes: blob_bytes.len() as u64,
                created_at_s: 1,
            },
        )
        .expect("append pack");
    let (blob_index, _) = blob_store
        .append_blob_with_id_index(
            &mut tx,
            &BinaryBlobRecord {
                blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
                hash_kind: 0,
                reserved0: 0,
                size_bytes: blob_bytes.len() as u64,
                pack_member_index_plus1: 1,
                created_at_s: 1,
                pruned_at_s: 0,
                sha256: blob_sha,
            },
        )
        .expect("append blob");
    object_pack_store
        .append_object_pack_member_record(
            &mut tx,
            &BinaryObjectPackMemberRecord {
                member_meta: 0,
                delta_chain_depth: 0,
                reserved0: 0,
                pack_index,
                blob_index,
                base_blob_index_plus1: 0,
            },
        )
        .expect("append member");
    tx.commit().expect("commit txn");

    let read = blob_store.begin_read_txn();
    assert_eq!(
        blob_store
            .read_blob_bytes_for_id(&read, &blob_id)
            .expect("read blob bytes"),
        Some(blob_bytes.to_vec())
    );
    assert_eq!(
        BlobReader::read_blob_bytes(&blob_store, &blob_id).expect("object diff blob reader"),
        Some(blob_bytes.to_vec())
    );
    assert_header_u32(&temp.path().join("binary-db").join(BLOB_BIN), TEST_LAYOUT);
    assert_header_u32(
        &temp.path().join("binary-db").join(BLOB_ID_IDX),
        TEST_LAYOUT,
    );
    assert_header_u32(
        &temp.path().join("binary-db").join(OBJECT_PACK_BIN),
        TEST_LAYOUT,
    );
    assert_header_u32(
        &temp.path().join("binary-db").join(OBJECT_PACK_ID_IDX),
        TEST_LAYOUT,
    );
    assert_header_u32(
        &temp.path().join("binary-db").join(OBJECT_PACK_MEMBER_BIN),
        TEST_LAYOUT,
    );
    assert_eq!(
        blob_store
            .read_blob_bytes_for_id(&read, &blob_id.to_ascii_lowercase())
            .expect("read lowercase blob id"),
        Some(blob_bytes.to_vec())
    );
}

#[test]
fn content_binary_db_reads_historical_cross_pack_chain_beyond_writer_depth() {
    use crate::pack_substrate::{
        build_git_binary_delta_member, write_pack_archive_with_format,
        DEFAULT_MAX_DELTA_CHAIN_DEPTH, MAX_DELTA_CHAIN_READ_DEPTH, PACK_FORMAT_ZSTD_CHUNKED_V1,
    };

    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db, temp.path());
    let historical_depth = DEFAULT_MAX_DELTA_CHAIN_DEPTH + 1;
    assert!(historical_depth < MAX_DELTA_CHAIN_READ_DEPTH);

    let versions = (0..=historical_depth)
        .map(|version| {
            let mut bytes = vec![b'a'; 4096];
            bytes[version] = b'A' + u8::try_from(version).expect("version byte");
            bytes
        })
        .collect::<Vec<_>>();
    let blob_hashes = versions
        .iter()
        .map(|bytes| sha256(bytes))
        .collect::<Vec<_>>();
    let blob_ids = blob_hashes
        .iter()
        .map(blob_id_from_sha256)
        .collect::<Vec<_>>();

    let mut tx = blob_store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin txn");
    for depth in 0..=historical_depth {
        let pack_hash = 0x1000_0000_0000_u64 + u64::try_from(depth).expect("pack depth");
        let (pack_hash_hi16, pack_hash_lo32) = split_hash48(pack_hash);
        let pack_id = object_pack_id_from_hash48(pack_hash);
        let pack_path = temp.path().join(
            object_pack_relative_path(&pack_id, PACK_FORMAT_ZSTD_CHUNKED_V1).expect("pack path"),
        );
        let members = if depth == 0 {
            json!([{
                "entry_name": format!("blobs/{}", blob_ids[depth]),
                "blob_id": blob_ids[depth],
                "data": versions[depth].as_slice(),
            }])
        } else {
            json!([build_git_binary_delta_member(
                &format!("blobs/{}", blob_ids[depth]),
                &blob_ids[depth],
                &blob_ids[depth - 1],
                &versions[depth - 1],
                &versions[depth],
                depth,
            )])
        };
        write_pack_archive_with_format(
            pack_path.to_str().expect("pack path utf8"),
            &pack_id,
            "2026-07-15T00:00:00Z",
            &members,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .expect("write historical object pack");

        let (pack_index, _) = object_pack_store
            .append_object_pack_with_id_index(
                &mut tx,
                &BinaryObjectPackRecord {
                    pack_meta: BinaryObjectPackRecord::META_READY,
                    pack_format_kind: 1,
                    pack_hash_hi16,
                    pack_hash_lo32,
                    first_member_index: u32::try_from(depth).expect("member index"),
                    member_count: 1,
                    total_bytes: versions[depth].len() as u64,
                    created_at_s: 1,
                },
            )
            .expect("append pack");
        let (blob_index, _) = blob_store
            .append_blob_with_id_index(
                &mut tx,
                &BinaryBlobRecord {
                    blob_meta: BinaryBlobRecord::META_HAS_PACK_MEMBER,
                    hash_kind: 0,
                    reserved0: 0,
                    size_bytes: versions[depth].len() as u64,
                    pack_member_index_plus1: u32::try_from(depth + 1)
                        .expect("member index plus one"),
                    created_at_s: 1,
                    pruned_at_s: 0,
                    sha256: blob_hashes[depth],
                },
            )
            .expect("append blob");
        object_pack_store
            .append_object_pack_member_record(
                &mut tx,
                &BinaryObjectPackMemberRecord {
                    member_meta: u8::from(depth != 0),
                    delta_chain_depth: u8::try_from(depth).expect("delta depth"),
                    reserved0: 0,
                    pack_index,
                    blob_index,
                    base_blob_index_plus1: u32::try_from(depth).expect("base blob pointer"),
                },
            )
            .expect("append member");
    }
    tx.commit().expect("commit historical chain");

    let read = blob_store.begin_read_txn();
    assert_eq!(
        blob_store
            .read_blob_bytes_for_id(&read, &blob_ids[historical_depth])
            .expect("read historical chain"),
        Some(versions[historical_depth].clone())
    );
    let deepest = blob_store
        .get_blob_view(&read, &blob_ids[historical_depth])
        .expect("read deepest view")
        .expect("deepest view");
    let error = blob_store
        .read_blob_bytes_for_view(&read, &deepest, MAX_DELTA_CHAIN_READ_DEPTH + 1)
        .expect_err("reader must retain an absolute safety ceiling");
    assert!(error.to_string().contains("safety read limit 64"));
}

#[test]
fn content_binary_db_resolves_tree_payload_and_snapshot_manifest() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let blob_bytes = b"tree file\n";
    let blob_sha = sha256(blob_bytes);
    let tree_hash = [0xAB; 10];
    let tree_id = tree_id_from_hash80(&tree_hash);
    let tree_pack_hash = 0x111213141516_u64;
    let tree_pack_id = tree_pack_id_from_hash48(tree_pack_hash);
    let (tree_pack_hash_hi16, tree_pack_hash_lo32) = split_hash48(tree_pack_hash);

    let mut tx = tree_store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin txn");
    let (_blob_index, blob_id) = blob_store
        .append_blob_with_id_index(
            &mut tx,
            &BinaryBlobRecord {
                blob_meta: 0,
                hash_kind: 0,
                reserved0: 0,
                size_bytes: blob_bytes.len() as u64,
                pack_member_index_plus1: 0,
                created_at_s: 2,
                pruned_at_s: 0,
                sha256: blob_sha,
            },
        )
        .expect("append blob");
    let (_tree_index, appended_tree_id) = tree_store
        .append_tree_with_id_index(
            &mut tx,
            &BinaryTreeRecord {
                tree_meta: 0,
                reserved0: 0,
                pack_entry_ordinal: 0,
                entry_count: 1,
                tree_hash80: tree_hash,
            },
        )
        .expect("append tree");
    tree_pack_store
        .append_tree_pack_with_id_index(
            &mut tx,
            &BinaryTreePackRecord {
                pack_meta: BinaryTreePackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: tree_pack_hash_hi16,
                pack_hash_lo32: tree_pack_hash_lo32,
                first_tree_index: 0,
                tree_count: 1,
                total_bytes: blob_bytes.len() as u64,
                created_at_s: 3,
            },
        )
        .expect("append tree pack");
    tx.commit().expect("commit txn");
    assert_eq!(appended_tree_id, tree_id);
    write_tree_pack_fixture(
        temp.path(),
        &tree_pack_id,
        &tree_id,
        &json!([{
            "entry_name": "file.txt",
            "entry_type": "blob",
            "target_id": blob_id,
            "size_bytes": blob_bytes.len(),
            "mode": "0o100644",
        }]),
    );

    let read = tree_store.begin_read_txn();
    let entries = tree_store
        .list_tree_entry_views(&read, &tree_id)
        .expect("list tree entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].entry_name, "file.txt");
    assert_eq!(entries[0].target_id, blob_id);

    let payload = tree_store
        .read_tree_payload_json(&read, &tree_id)
        .expect("read tree payload")
        .expect("tree payload");
    assert_eq!(payload["tree_id"], tree_id);
    assert_eq!(payload["entries"][0]["entry_name"], "file.txt");

    let snapshot_reader = BinaryDbSnapshotReader::new(tree_store.clone())
        .with_snapshot_root("snap-1", tree_id.as_str());
    let manifest = snapshot_reader
        .read_snapshot_manifest("snap-1")
        .expect("snapshot manifest");
    assert_eq!(manifest["file.txt"]["blob_id"], blob_id);
    assert_eq!(
        snapshot_reader
            .read_snapshot_root_tree_payload("snap-1")
            .expect("snapshot root tree")
            .expect("root tree")["tree_id"],
        tree_id
    );
    assert_header_u32(&temp.path().join("binary-db").join(TREE_BIN), TEST_LAYOUT);
    assert_header_u32(
        &temp.path().join("binary-db").join(TREE_ID_IDX),
        TEST_LAYOUT,
    );
}

#[test]
fn content_binary_db_reads_tree_payload_from_tree_pack() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_hash = [0xCD; 10];
    let tree_id = tree_id_from_hash80(&tree_hash);
    let tree_pack_hash = 0x0A0B0C0D0E0F_u64;
    let (pack_hash_hi16, pack_hash_lo32) = split_hash48(tree_pack_hash);
    let tree_pack_id = tree_pack_id_from_hash48(tree_pack_hash);
    let raw_payload = JsonCodec::encode_value_to_vec(
        &json!({
            "tree_id": tree_id,
            "entries": [{
                "entry_name": "packed.txt",
                "entry_type": "blob",
                "target_id": "BLB-00112233445566778899",
                "size_bytes": 12,
                "mode": "0o100644",
            }],
        }),
        JsonEncodeOptions::compact(),
    )
    .expect("tree pack payload json");
    let pack_path = temp.path().join(
        tree_pack_relative_path(
            &tree_pack_id,
            crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .expect("tree pack path"),
    );
    crate::pack_substrate::write_tree_pack_archive_with_format(
        pack_path.to_str().expect("pack path utf8"),
        &tree_pack_id,
        "2026-07-05T00:00:00Z",
        &json!([{
            "tree_id": tree_id,
            "entry_name": format!("trees/{tree_id}.json"),
            "entry_count": 1,
            "data": raw_payload,
        }]),
        crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("write tree pack");

    let mut tx = tree_store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin txn");
    let (tree_index, appended_tree_id) = tree_store
        .append_tree_with_id_index(
            &mut tx,
            &BinaryTreeRecord {
                tree_meta: 0,
                reserved0: 0,
                pack_entry_ordinal: 0,
                entry_count: 1,
                tree_hash80: tree_hash,
            },
        )
        .expect("append tree");
    assert_eq!(appended_tree_id, tree_id);
    let (_pack_index, appended_tree_pack_id) = tree_pack_store
        .append_tree_pack_with_id_index(
            &mut tx,
            &BinaryTreePackRecord {
                pack_meta: BinaryTreePackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16,
                pack_hash_lo32,
                first_tree_index: tree_index,
                tree_count: 1,
                total_bytes: 12,
                created_at_s: 3,
            },
        )
        .expect("append tree pack");
    assert_eq!(appended_tree_pack_id, tree_pack_id);
    tx.commit().expect("commit txn");

    let payload = tree_pack_store
        .read_tree_payload(&tree_id)
        .expect("read tree payload")
        .expect("tree payload");
    assert_eq!(payload["tree_id"], tree_id);
    assert_eq!(payload["entries"][0]["entry_name"], "packed.txt");
    let read = tree_store.begin_read_txn();
    let mut cache = BinaryDbTreeReadCache::default();
    tree_store
        .list_tree_entry_views_with_cache(&read, &tree_id, &mut cache)
        .expect("first cached tree read");
    assert_eq!(
        cache.cached_tree_pack_count(),
        1,
        "first tree read must build one transaction-local pack locator"
    );
    assert_eq!(
        cache.cached_tree_entry_count(),
        1,
        "first tree read must cache the decoded tree entries"
    );
    let cached_chunks = cache.cached_zstd_chunk_count();
    assert!(
        cached_chunks > 0,
        "first tree read must populate the zstd chunk cache"
    );
    tree_store
        .list_tree_entry_views_with_cache(&read, &tree_id, &mut cache)
        .expect("second cached tree read");
    assert_eq!(
        cache.archive_count(),
        1,
        "tree pack must open once per read session"
    );
    assert_eq!(
        cache.cached_zstd_chunk_count(),
        cached_chunks,
        "repeated tree reads must reuse decompressed zstd chunks"
    );
    assert_eq!(
        cache.cached_tree_pack_count(),
        1,
        "repeated tree reads must reuse the pack locator"
    );
    assert_eq!(
        cache.cached_tree_entry_count(),
        1,
        "repeated tree reads must reuse decoded entries"
    );
    cache.clear_tree_entries();
    assert_eq!(
        cache.cached_tree_entry_count(),
        0,
        "callers must be able to bound decoded-entry memory without dropping pack caches"
    );
    tree_store
        .list_tree_entry_views_with_cache(&read, &tree_id, &mut cache)
        .expect("cached tree read after entry release");
    assert_eq!(cache.archive_count(), 1);
    assert_eq!(cache.cached_zstd_chunk_count(), cached_chunks);
    assert_eq!(cache.cached_tree_pack_count(), 1);
    assert_eq!(cache.cached_tree_entry_count(), 1);
    assert_header_u32(
        &temp.path().join("binary-db").join(TREE_PACK_BIN),
        TEST_LAYOUT,
    );
    assert_header_u32(
        &temp.path().join("binary-db").join(TREE_PACK_ID_IDX),
        TEST_LAYOUT,
    );
}

#[test]
fn content_binary_db_snapshot_store_reads_snapshot_and_tree_rows() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());

    let blob_bytes = b"snapshot file\n";
    let blob_sha = sha256(blob_bytes);
    let tree_hash = [0xEF; 10];
    let tree_id = tree_id_from_hash80(&tree_hash);
    let tree_pack_hash = 0x101112131415_u64;
    let (tree_pack_hash_hi16, tree_pack_hash_lo32) = split_hash48(tree_pack_hash);
    let tree_pack_id = tree_pack_id_from_hash48(tree_pack_hash);
    let snapshot_hash = 0x010203040506_u64;
    let expected_snapshot_id = snapshot_id_from_hash48(snapshot_hash);

    let mut tx = snapshot_store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::SnapshotWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin txn");
    let (_blob_index, blob_id) = blob_store
        .append_blob_with_id_index(
            &mut tx,
            &BinaryBlobRecord {
                blob_meta: 0,
                hash_kind: 0,
                reserved0: 0,
                size_bytes: blob_bytes.len() as u64,
                pack_member_index_plus1: 0,
                created_at_s: 10,
                pruned_at_s: 0,
                sha256: blob_sha,
            },
        )
        .expect("append blob");
    let (tree_index, appended_tree_id) = tree_store
        .append_tree_with_id_index(
            &mut tx,
            &BinaryTreeRecord {
                tree_meta: 0,
                reserved0: 0,
                pack_entry_ordinal: 0,
                entry_count: 1,
                tree_hash80: tree_hash,
            },
        )
        .expect("append tree");
    assert_eq!(appended_tree_id, tree_id);
    let (tree_pack_index, appended_tree_pack_id) = tree_pack_store
        .append_tree_pack_with_id_index(
            &mut tx,
            &BinaryTreePackRecord {
                pack_meta: BinaryTreePackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: tree_pack_hash_hi16,
                pack_hash_lo32: tree_pack_hash_lo32,
                first_tree_index: tree_index,
                tree_count: 1,
                total_bytes: blob_bytes.len() as u64,
                created_at_s: 11,
            },
        )
        .expect("append tree pack");
    assert_eq!(appended_tree_pack_id, tree_pack_id);
    let (_snapshot_index, snapshot_id, _record) = snapshot_store
        .append_snapshot_with_id_index(
            &mut tx,
            BinarySnapshotRecord {
                snapshot_meta: BinarySnapshotRecord::META_HAS_ROOT_LOCATOR,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: snapshot_hash,
                parent_snapshot_index_plus1: 0,
                root_tree_pack_index_plus1: tree_pack_index + 1,
                root_entry_ordinal: 0,
                line_index_plus1: 0,
                manifest_hash: sha256(b"manifest"),
                file_count: 1,
                total_bytes: blob_bytes.len() as u64,
                created_at_s: 12,
            },
            &BinarySnapshotPayload {
                line_name: "main".to_string(),
                message: Some("seed".to_string()),
                additional_parent_snapshot_indices: Vec::new(),
            },
        )
        .expect("append snapshot");
    tx.commit().expect("commit txn");
    assert_eq!(snapshot_id, expected_snapshot_id);
    write_tree_pack_fixture(
        temp.path(),
        &tree_pack_id,
        &tree_id,
        &json!([{
            "entry_name": "file.txt",
            "entry_type": "blob",
            "target_id": blob_id,
            "size_bytes": blob_bytes.len(),
            "mode": "0o100644",
        }]),
    );

    assert!(snapshot_store
        .snapshot_exists(&snapshot_id)
        .expect("snapshot exists"));
    let record = snapshot_store
        .snapshot_by_id(&snapshot_id)
        .expect("snapshot by id")
        .expect("snapshot record");
    assert_eq!(record.snapshot_id, snapshot_id);
    assert_eq!(
        record.root_tree_pack_id.as_deref(),
        Some(tree_pack_id.as_str())
    );
    assert_eq!(record.root_entry_ordinal, Some(0));
    assert_eq!(record.line_name, "main");
    assert_eq!(record.message.as_deref(), Some("seed"));
    assert_eq!(record.snapshot_kind, "line");
    assert_eq!(
        snapshot_store
            .snapshot_chain(&snapshot_id)
            .expect("snapshot chain"),
        vec![snapshot_id.clone()]
    );

    let root_locator = snapshot_store
        .snapshot_tree_root_locator(&snapshot_id)
        .expect("root locator");
    assert_eq!(root_locator.root_tree_id, tree_id);
    assert_eq!(root_locator.root_tree_pack_id, tree_pack_id);
    assert_eq!(root_locator.root_entry_ordinal, 0);
    assert!(snapshot_store
        .snapshot_tree_manifest_path(&snapshot_id)
        .expect("manifest path")
        .ends_with(&format!("#trees/{tree_id}.json")));

    let rows = snapshot_store
        .snapshot_tree_file_rows(Some(&snapshot_id))
        .expect("file rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].path, "file.txt");
    assert_eq!(rows[0].blob_id, blob_id);
    assert_eq!(rows[0].sha256, hex_lower(&blob_sha));
    let row = snapshot_store
        .snapshot_tree_path_row(&snapshot_id, "file.txt")
        .expect("path row")
        .expect("path row exists");
    assert_eq!(row["blob_id"], blob_id);
    assert_eq!(row["sha256"], hex_lower(&blob_sha));

    let counting = CountingSnapshotReadDb::new(db.clone());
    let counted_snapshot_store =
        BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(counting.clone(), temp.path());
    let requested_paths = (0..32)
        .flat_map(|_| ["file.txt".to_string(), "missing.txt".to_string()])
        .collect::<Vec<_>>();
    let path_rows = counted_snapshot_store
        .snapshot_tree_path_file_rows(&snapshot_id, &requested_paths)
        .expect("batch path rows");
    assert_eq!(path_rows.len(), 1);
    assert_eq!(path_rows["file.txt"].blob_id, blob_id);

    let local = LocalContentBinaryDb::<TEST_LAYOUT>::from_db(db.clone(), temp.path());
    let listed = local.list_snapshots().expect("local snapshot list");
    assert_eq!(listed[0]["snapshot_id"], snapshot_id);
    assert!(listed[0]["manifest_path"]
        .as_str()
        .expect("manifest path")
        .ends_with(&format!("#trees/{tree_id}.json")));
    let shown = local
        .get_snapshot(&snapshot_id)
        .expect("local snapshot show");
    assert_eq!(shown["snapshot_id"], snapshot_id);
    assert_eq!(shown["files"][0]["path"], "file.txt");

    assert_header_u32(
        &temp.path().join("binary-db").join(SNAPSHOT_BIN),
        TEST_LAYOUT,
    );
    assert_header_u32(
        &temp.path().join("binary-db").join(SNAPSHOT_ID_IDX),
        TEST_LAYOUT,
    );
    assert_header_u32(
        &temp.path().join("binary-db").join(SNAPSHOT_PAYLOAD_BIN),
        TEST_LAYOUT,
    );
}

#[test]
fn core_content_reads_dispatch_from_persisted_layout_not_write_layout() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());

    let blob_hash = sha256(b"persisted layout fixture");
    let blob_id = blob_id_from_sha256(&blob_hash);
    let object_pack_hash = 0x0102_0304_0506;
    let (object_pack_hash_hi16, object_pack_hash_lo32) = split_hash48(object_pack_hash);
    let object_pack_id = object_pack_id_from_hash48(object_pack_hash);
    let tree_pack_hash = 0x1112_1314_1516;
    let (tree_pack_hash_hi16, tree_pack_hash_lo32) = split_hash48(tree_pack_hash);
    let tree_pack_id = tree_pack_id_from_hash48(tree_pack_hash);
    let tree_hash = [0x42; 10];
    let tree_id = tree_id_from_hash80(&tree_hash);
    let snapshot_hash = 0x2122_2324_2526;
    let snapshot_id = snapshot_id_from_hash48(snapshot_hash);

    let mut write = snapshot_store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::SnapshotWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin fixture transaction");
    let (blob_index, _) = blob_store
        .append_blob_with_id_index(
            &mut write,
            &BinaryBlobRecord {
                blob_meta: 0,
                hash_kind: 0,
                reserved0: 0,
                size_bytes: 24,
                pack_member_index_plus1: 1,
                created_at_s: 1,
                pruned_at_s: 0,
                sha256: blob_hash,
            },
        )
        .expect("append blob");
    let (object_pack_index, _) = object_pack_store
        .append_object_pack_with_id_index(
            &mut write,
            &BinaryObjectPackRecord {
                pack_meta: BinaryObjectPackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: object_pack_hash_hi16,
                pack_hash_lo32: object_pack_hash_lo32,
                first_member_index: 0,
                member_count: 1,
                total_bytes: 24,
                created_at_s: 2,
            },
        )
        .expect("append object pack");
    object_pack_store
        .append_object_pack_member_record(
            &mut write,
            &BinaryObjectPackMemberRecord {
                member_meta: 0,
                delta_chain_depth: 0,
                reserved0: 0,
                pack_index: object_pack_index,
                blob_index,
                base_blob_index_plus1: 0,
            },
        )
        .expect("append object-pack member");
    let (tree_index, _) = tree_store
        .append_tree_with_id_index(
            &mut write,
            &BinaryTreeRecord {
                tree_meta: 0,
                reserved0: 0,
                pack_entry_ordinal: 0,
                entry_count: 1,
                tree_hash80: tree_hash,
            },
        )
        .expect("append tree");
    let (tree_pack_index, _) = tree_pack_store
        .append_tree_pack_with_id_index(
            &mut write,
            &BinaryTreePackRecord {
                pack_meta: BinaryTreePackRecord::META_READY,
                pack_format_kind: 1,
                pack_hash_hi16: tree_pack_hash_hi16,
                pack_hash_lo32: tree_pack_hash_lo32,
                first_tree_index: tree_index,
                tree_count: 1,
                total_bytes: 24,
                created_at_s: 3,
            },
        )
        .expect("append tree pack");
    snapshot_store
        .append_snapshot_with_id_index(
            &mut write,
            BinarySnapshotRecord {
                snapshot_meta: BinarySnapshotRecord::META_HAS_ROOT_LOCATOR,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: snapshot_hash,
                parent_snapshot_index_plus1: 0,
                root_tree_pack_index_plus1: tree_pack_index + 1,
                root_entry_ordinal: 0,
                line_index_plus1: 0,
                manifest_hash: sha256(b"manifest"),
                file_count: 1,
                total_bytes: 24,
                created_at_s: 4,
            },
            &BinarySnapshotPayload {
                line_name: "main".to_string(),
                message: Some("fixture".to_string()),
                additional_parent_snapshot_indices: Vec::new(),
            },
        )
        .expect("append snapshot");
    write.commit().expect("commit fixture transaction");
    write_tree_pack_fixture(
        temp.path(),
        &tree_pack_id,
        &tree_id,
        &json!([{
            "entry_name": "fixture.txt",
            "entry_type": "blob",
            "target_id": blob_id,
            "size_bytes": 24,
            "mode": "0o100644",
        }]),
    );

    let binary_root = temp.path().join("binary-db");
    let fixture_files = [
        BLOB_BIN,
        BLOB_ID_IDX,
        OBJECT_PACK_BIN,
        OBJECT_PACK_ID_IDX,
        OBJECT_PACK_MEMBER_BIN,
        TREE_PACK_BIN,
        TREE_PACK_ID_IDX,
        TREE_BIN,
        TREE_ID_IDX,
        SNAPSHOT_BIN,
        SNAPSHOT_ID_IDX,
        SNAPSHOT_PAYLOAD_BIN,
    ];
    let before = fixture_files
        .iter()
        .map(|name| {
            (
                (*name).to_string(),
                fs::read(binary_root.join(name)).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    // The generic parameter is a writer choice. Reads must select layout 1
    // from each persisted header and must never rewrite compatible old bytes.
    let future_blobs = BinaryDbBlobStore::<_, 99>::new(db.clone(), temp.path());
    let future_object_packs = BinaryDbObjectPackStore::<_, 99>::new(db.clone(), temp.path());
    let future_tree_packs = BinaryDbTreePackStore::<_, 99>::new(db.clone(), temp.path());
    let future_trees = BinaryDbTreeStore::<_, 99>::new(db.clone(), temp.path());
    let future_snapshots = BinaryDbSnapshotStore::<_, 99>::new(db.clone(), temp.path());
    let read = future_blobs.begin_read_txn();
    assert_eq!(
        future_blobs
            .get_blob_view(&read, &blob_id)
            .expect("read blob")
            .expect("blob")
            .blob_index,
        blob_index
    );
    assert_eq!(
        future_object_packs
            .get_object_pack_view(&read, &object_pack_id)
            .expect("read object pack")
            .expect("object pack")
            .pack_index,
        object_pack_index
    );
    assert_eq!(
        future_object_packs
            .object_pack_member_view_at(&read, 0)
            .expect("read object-pack member")
            .blob_id,
        blob_id
    );
    assert_eq!(
        future_tree_packs
            .get_tree_pack_view(&read, &tree_pack_id)
            .expect("read tree pack")
            .expect("tree pack")
            .tree_pack_index,
        tree_pack_index
    );
    assert_eq!(
        future_trees
            .get_tree_view(&read, &tree_id)
            .expect("read tree")
            .expect("tree")
            .tree_index,
        tree_index
    );
    assert_eq!(
        future_trees
            .list_tree_entry_views(&read, &tree_id)
            .expect("read tree entries")[0]
            .entry_name,
        "fixture.txt"
    );
    assert_eq!(
        future_snapshots
            .get_snapshot_view(&read, &snapshot_id)
            .expect("read snapshot")
            .expect("snapshot")
            .payload
            .message
            .as_deref(),
        Some("fixture")
    );
    let after = fixture_files
        .iter()
        .map(|name| {
            (
                (*name).to_string(),
                fs::read(binary_root.join(name)).unwrap(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        after, before,
        "read-only compatibility mutated fixture bytes"
    );

    // A read transaction owns an immutable view while its lock set is held.
    // Corruption injected outside the write protocol is authoritative to the
    // next transaction, which must still reject every incompatible layout.
    drop(read);
    overwrite_header_u32(&binary_root.join(BLOB_ID_IDX), 99);
    let read = future_blobs.begin_read_txn();
    let error = future_blobs
        .get_blob_view(&read, &blob_id)
        .expect_err("mixed blob/index layouts must fail");
    assert_eq!(error.kind(), BinaryDbErrorKind::LayoutMismatch);

    drop(read);
    overwrite_header_u32(&binary_root.join(TREE_PACK_BIN), 99);
    let read = future_blobs.begin_read_txn();
    let error = future_tree_packs
        .get_tree_pack_view(&read, &tree_pack_id)
        .expect_err("future tree-pack layout must fail");
    assert_eq!(error.kind(), BinaryDbErrorKind::LayoutMismatch);

    drop(read);
    let payload_path = binary_root.join(SNAPSHOT_PAYLOAD_BIN);
    let mut payload = fs::read(&payload_path).expect("read snapshot payload");
    payload.truncate(4);
    fs::write(payload_path, payload).expect("truncate snapshot payload body");
    let read = future_blobs.begin_read_txn();
    let error = future_snapshots
        .get_snapshot_view(&read, &snapshot_id)
        .expect_err("snapshot payload bounds must fail");
    assert_eq!(error.kind(), BinaryDbErrorKind::MissingData);
}

#[test]
fn content_write_coordinator_reads_dense_indices_after_acquiring_write_lock() {
    let temp = tempdir().expect("tempdir");
    let db = AdvanceObjectMemberBeforeWriteLockDb::new(local_db(temp.path()));
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db, temp.path());
    let coordinator = BinaryDbContentWriteCoordinator::new(
        &blob_store,
        &object_pack_store,
        &tree_pack_store,
        &tree_store,
        &snapshot_store,
    );

    let blob_sha = sha256(b"locked coordinator blob\n");
    let blob_id = blob_id_from_sha256(&blob_sha);
    let pack_id = object_pack_id_from_hash48(0x1112_1314_1516);
    coordinator
        .record_object_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbObjectPackWriteInput {
                pack_id: pack_id.clone(),
                pack_rel_path: object_pack_relative_path(
                    &pack_id,
                    crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
                )
                .expect("object pack path"),
                pack_format: crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                member_count: 1,
                total_bytes: 64,
                created_at: "2026-07-10T00:00:00Z".to_string(),
                members: vec![BinaryDbObjectPackMemberWriteInput {
                    blob_id: blob_id.clone(),
                    sha256: hex_lower(&blob_sha),
                    size_bytes: 24,
                    pack_entry_type: "full".to_string(),
                    pack_base_blob_id: None,
                    pack_chain_depth: 0,
                    created_at: "2026-07-10T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("record object pack after lock-time advance");

    let read = object_pack_store.begin_read_txn();
    let pack = object_pack_store
        .get_object_pack_view(&read, &pack_id)
        .expect("read object pack")
        .expect("object pack exists");
    assert_eq!(pack.record.first_member_index, 1);
    let blob = blob_store
        .get_blob_view(&read, &blob_id)
        .expect("read blob")
        .expect("blob exists");
    assert_eq!(blob.record.pack_member_index(), Some(1));
    let member = object_pack_store
        .object_pack_member_view_at(&read, 1)
        .expect("read locked member index");
    assert_eq!(member.pack_id, pack_id);
    assert_eq!(member.blob_id, blob_id);
}

#[test]
fn content_write_coordinator_reuses_blob_record_across_object_packs() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let coordinator = BinaryDbContentWriteCoordinator::new(
        &blob_store,
        &object_pack_store,
        &tree_pack_store,
        &tree_store,
        &snapshot_store,
    );

    let blob_bytes = b"one blob stored by two object packs\n";
    let blob_sha = sha256(blob_bytes);
    let blob_id = blob_id_from_sha256(&blob_sha);
    for (pack_hash, created_at) in [
        (0x1112_1314_1516, "2026-07-10T00:00:00Z"),
        (0x2122_2324_2526, "2026-07-11T00:00:00Z"),
    ] {
        let pack_id = object_pack_id_from_hash48(pack_hash);
        coordinator
            .record_object_pack_metadata(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbObjectPackWriteInput {
                    pack_id: pack_id.clone(),
                    pack_rel_path: object_pack_relative_path(
                        &pack_id,
                        crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
                    )
                    .expect("object pack path"),
                    pack_format: crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                    member_count: 1,
                    total_bytes: 64,
                    created_at: created_at.to_string(),
                    members: vec![BinaryDbObjectPackMemberWriteInput {
                        blob_id: blob_id.clone(),
                        sha256: hex_lower(&blob_sha),
                        size_bytes: blob_bytes.len() as i64,
                        pack_entry_type: "full".to_string(),
                        pack_base_blob_id: None,
                        pack_chain_depth: 0,
                        created_at: created_at.to_string(),
                    }],
                },
            )
            .expect("record object pack metadata");
    }

    assert_eq!(
        db.record_count(BinaryBlobCodec::<TEST_LAYOUT>::record_file())
            .expect("blob record count"),
        1,
        "one content identity must own one canonical Blob row"
    );
    assert_eq!(
        db.record_count(BinaryObjectPackMemberCodec::<TEST_LAYOUT>::record_file())
            .expect("object-pack member count"),
        2
    );
    let read = blob_store.begin_read_txn();
    let blob = blob_store
        .get_blob_view(&read, &blob_id)
        .expect("read Blob")
        .expect("Blob exists");
    assert_eq!(blob.record.pack_member_index(), Some(0));
    for member_index in 0..2 {
        let member = object_pack_store
            .object_pack_member_view_at(&read, member_index)
            .expect("read object-pack member");
        assert_eq!(member.record.blob_index, 0);
        assert_eq!(member.blob_id, blob_id);
    }
}

#[test]
fn content_write_coordinator_retains_repeated_tree_without_replacing_selected_identity() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let coordinator = BinaryDbContentWriteCoordinator::new(
        &blob_store,
        &object_pack_store,
        &tree_pack_store,
        &tree_store,
        &snapshot_store,
    );

    let tree_id = "TRE-0102030405060708090A";
    let selected_pack_id = tree_pack_id_from_hash48(0x1112_1314_1516);
    let overlapping_pack_id = tree_pack_id_from_hash48(0x2122_2324_2526);
    for (pack_id, created_at) in [
        (&selected_pack_id, "2026-08-04T00:00:00Z"),
        (&overlapping_pack_id, "2026-08-04T00:00:01Z"),
    ] {
        coordinator
            .record_tree_pack_metadata(
                BinaryDbCommandScope::RemoteSyncLocalImport,
                &BinaryDbTreePackWriteInput {
                    pack_id: pack_id.clone(),
                    pack_rel_path: tree_pack_relative_path(
                        pack_id,
                        crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                    )
                    .expect("tree pack path"),
                    pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1
                        .to_string(),
                    tree_count: 1,
                    total_bytes: 64,
                    created_at: created_at.to_string(),
                    trees: vec![BinaryDbTreePackTreeWriteInput {
                        tree_id: tree_id.to_string(),
                        entry_count: 0,
                    }],
                },
            )
            .expect("record overlapping tree pack metadata");
    }

    assert_eq!(
        db.record_count(BinaryTreeCodec::<TEST_LAYOUT>::record_file())
            .expect("tree record count"),
        2,
        "both verified physical tree-pack members must remain represented"
    );
    let read = tree_store.begin_read_txn();
    let selected_tree = tree_store
        .get_tree_view(&read, tree_id)
        .expect("read selected Tree")
        .expect("selected Tree exists");
    assert_eq!(selected_tree.tree_index, 0);
    assert_eq!(
        selected_tree.tree_pack_id.as_deref(),
        Some(selected_pack_id.as_str()),
        "the repeated member must not replace the manifest-selected Tree identity"
    );
    let repeated_tree = tree_store
        .tree_view_at(&read, 1)
        .expect("read repeated Tree");
    assert_eq!(repeated_tree.tree_id, tree_id);
    assert_eq!(
        repeated_tree.tree_pack_id.as_deref(),
        Some(overlapping_pack_id.as_str())
    );
}

#[test]
fn content_write_coordinator_batches_dependency_ordered_remote_metadata() {
    let temp = tempdir().expect("tempdir");
    let db = CountingWriteDb::new(local_db(temp.path()));
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    BinaryDbLineStore::<_, TEST_LAYOUT>::new(db.clone())
        .create_line("main", None, "2026-07-23T00:00:00Z")
        .expect("create main line");
    let coordinator = BinaryDbContentWriteCoordinator::new(
        &blob_store,
        &object_pack_store,
        &tree_pack_store,
        &tree_store,
        &snapshot_store,
    );

    let base_sha = sha256(b"batch base blob\n");
    let base_blob_id = blob_id_from_sha256(&base_sha);
    let delta_sha = sha256(b"batch delta blob\n");
    let delta_blob_id = blob_id_from_sha256(&delta_sha);
    let base_pack_id = object_pack_id_from_hash48(0x1011_1213_1415);
    let delta_pack_id = object_pack_id_from_hash48(0x2021_2223_2425);
    let object_packs = vec![
        BinaryDbObjectPackWriteInput {
            pack_id: base_pack_id.clone(),
            pack_rel_path: object_pack_relative_path(
                &base_pack_id,
                crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
            )
            .expect("base object pack path"),
            pack_format: crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
            member_count: 1,
            total_bytes: 64,
            created_at: "2026-07-23T00:00:00Z".to_string(),
            members: vec![BinaryDbObjectPackMemberWriteInput {
                blob_id: base_blob_id.clone(),
                sha256: hex_lower(&base_sha),
                size_bytes: 16,
                pack_entry_type: "full".to_string(),
                pack_base_blob_id: None,
                pack_chain_depth: 0,
                created_at: "2026-07-23T00:00:00Z".to_string(),
            }],
        },
        BinaryDbObjectPackWriteInput {
            pack_id: delta_pack_id.clone(),
            pack_rel_path: object_pack_relative_path(
                &delta_pack_id,
                crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
            )
            .expect("delta object pack path"),
            pack_format: crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
            member_count: 1,
            total_bytes: 64,
            created_at: "2026-07-23T00:00:01Z".to_string(),
            members: vec![BinaryDbObjectPackMemberWriteInput {
                blob_id: delta_blob_id.clone(),
                sha256: hex_lower(&delta_sha),
                size_bytes: 17,
                pack_entry_type: "delta".to_string(),
                pack_base_blob_id: Some(base_blob_id.clone()),
                pack_chain_depth: 1,
                created_at: "2026-07-23T00:00:01Z".to_string(),
            }],
        },
    ];
    db.reset_counts();
    coordinator
        .record_object_pack_metadata_batch(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &object_packs,
        )
        .expect("record object-pack batch");
    assert_eq!(db.command_lock_calls(), 1);
    assert_eq!(
        db.sync_file_calls(),
        5,
        "two object packs must sync each touched Binary DB file once"
    );
    let delta_member = object_pack_store
        .list_object_pack_members(&delta_pack_id)
        .expect("delta pack members")
        .into_iter()
        .next()
        .expect("delta pack member");
    assert_eq!(
        delta_member.base_blob_id.as_deref(),
        Some(base_blob_id.as_str())
    );

    let leaf_tree_id = "TRE-0102030405060708090A";
    let root_tree_id = "TRE-1112131415161718191A";
    let leaf_pack_id = tree_pack_id_from_hash48(0x3031_3233_3435);
    let root_pack_id = tree_pack_id_from_hash48(0x4041_4243_4445);
    let tree_packs = vec![
        (
            BinaryDbTreePackWriteInput {
                pack_id: leaf_pack_id.clone(),
                pack_rel_path: tree_pack_relative_path(
                    &leaf_pack_id,
                    crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                )
                .expect("leaf tree pack path"),
                pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 1,
                total_bytes: 64,
                created_at: "2026-07-23T00:00:02Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: leaf_tree_id.to_string(),
                    entry_count: 1,
                }],
            },
            vec![BinaryDbTreeEntryWriteInput {
                tree_id: leaf_tree_id.to_string(),
                entry_name: "file.txt".to_string(),
                entry_type: "blob".to_string(),
                target_id: base_blob_id.clone(),
                mode: "100644".to_string(),
            }],
        ),
        (
            BinaryDbTreePackWriteInput {
                pack_id: root_pack_id.clone(),
                pack_rel_path: tree_pack_relative_path(
                    &root_pack_id,
                    crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                )
                .expect("root tree pack path"),
                pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 1,
                total_bytes: 64,
                created_at: "2026-07-23T00:00:03Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: root_tree_id.to_string(),
                    entry_count: 1,
                }],
            },
            vec![BinaryDbTreeEntryWriteInput {
                tree_id: root_tree_id.to_string(),
                entry_name: "dir".to_string(),
                entry_type: "tree".to_string(),
                target_id: leaf_tree_id.to_string(),
                mode: "tree".to_string(),
            }],
        ),
    ];
    db.reset_counts();
    coordinator
        .record_tree_pack_metadata_batch_with_entries(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &tree_packs,
        )
        .expect("record tree-pack batch");
    assert_eq!(db.command_lock_calls(), 1);
    assert_eq!(
        db.sync_file_calls(),
        4,
        "two tree packs must sync each touched Binary DB file once"
    );

    let parent_snapshot_id = snapshot_id_from_hash48(0x5051_5253_5455);
    let child_snapshot_id = snapshot_id_from_hash48(0x6061_6263_6465);
    let snapshots = vec![
        BinaryDbSnapshotWriteInput {
            snapshot_id: parent_snapshot_id.clone(),
            parent_snapshot_ids: Vec::new(),
            root_tree_pack_id: root_pack_id.clone(),
            root_entry_ordinal: 0,
            manifest_hash: hex_lower(&sha256(b"batch parent manifest")),
            message: Some("batch parent".to_string()),
            line_name: "main".to_string(),
            snapshot_kind: "line".to_string(),
            file_count: 1,
            total_bytes: 16,
            created_at: "2026-07-23T00:00:04Z".to_string(),
        },
        BinaryDbSnapshotWriteInput {
            snapshot_id: child_snapshot_id.clone(),
            parent_snapshot_ids: vec![parent_snapshot_id.clone()],
            root_tree_pack_id: root_pack_id,
            root_entry_ordinal: 0,
            manifest_hash: hex_lower(&sha256(b"batch child manifest")),
            message: Some("batch child".to_string()),
            line_name: "main".to_string(),
            snapshot_kind: "line".to_string(),
            file_count: 1,
            total_bytes: 16,
            created_at: "2026-07-23T00:00:05Z".to_string(),
        },
    ];
    db.reset_counts();
    assert_eq!(
        coordinator
            .record_snapshots(BinaryDbCommandScope::RemoteSyncLocalImport, &snapshots)
            .expect("record parent-first snapshot batch"),
        vec![true, true]
    );
    assert_eq!(db.command_lock_calls(), 1);
    assert_eq!(
        db.sync_file_calls(),
        3,
        "two snapshots must sync each touched Binary DB file once"
    );
    assert_eq!(
        snapshot_store
            .snapshot_chain(&child_snapshot_id)
            .expect("read parent-first snapshot chain"),
        vec![parent_snapshot_id, child_snapshot_id]
    );
}

#[test]
fn content_write_coordinator_rolls_back_failed_metadata_batches() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let coordinator = BinaryDbContentWriteCoordinator::new(
        &blob_store,
        &object_pack_store,
        &tree_pack_store,
        &tree_store,
        &snapshot_store,
    );

    let first_sha = sha256(b"rollback first blob\n");
    let first_blob_id = blob_id_from_sha256(&first_sha);
    let first_pack_id = object_pack_id_from_hash48(0x7071_7273_7475);
    let invalid_pack_id = object_pack_id_from_hash48(0x8081_8283_8485);
    let error = coordinator
        .record_object_pack_metadata_batch(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &[
                BinaryDbObjectPackWriteInput {
                    pack_id: first_pack_id.clone(),
                    pack_rel_path: object_pack_relative_path(
                        &first_pack_id,
                        crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
                    )
                    .expect("first pack path"),
                    pack_format: crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                    member_count: 1,
                    total_bytes: 64,
                    created_at: "2026-07-23T00:00:00Z".to_string(),
                    members: vec![BinaryDbObjectPackMemberWriteInput {
                        blob_id: first_blob_id,
                        sha256: hex_lower(&first_sha),
                        size_bytes: 20,
                        pack_entry_type: "full".to_string(),
                        pack_base_blob_id: None,
                        pack_chain_depth: 0,
                        created_at: "2026-07-23T00:00:00Z".to_string(),
                    }],
                },
                BinaryDbObjectPackWriteInput {
                    pack_id: invalid_pack_id,
                    pack_rel_path: "objects/packs/not-canonical.zstpack".to_string(),
                    pack_format: crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                    member_count: 0,
                    total_bytes: 0,
                    created_at: "2026-07-23T00:00:01Z".to_string(),
                    members: Vec::new(),
                },
            ],
        )
        .expect_err("second invalid pack must abort the complete batch");
    assert!(error.to_string().contains("path mismatch"));
    assert_eq!(
        db.record_count(BinaryObjectPackCodec::<TEST_LAYOUT>::record_file())
            .expect("object-pack count after rollback"),
        0
    );
    assert_eq!(
        db.record_count(BinaryObjectPackMemberCodec::<TEST_LAYOUT>::record_file())
            .expect("object-pack member count after rollback"),
        0
    );
    assert_eq!(
        db.record_count(BinaryBlobCodec::<TEST_LAYOUT>::record_file())
            .expect("blob count after rollback"),
        0
    );

    let seed_sha = sha256(b"rollback tree seed blob\n");
    let seed_blob_id = blob_id_from_sha256(&seed_sha);
    let seed_pack_id = object_pack_id_from_hash48(0x9091_9293_9495);
    coordinator
        .record_object_pack_metadata(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &BinaryDbObjectPackWriteInput {
                pack_id: seed_pack_id.clone(),
                pack_rel_path: object_pack_relative_path(
                    &seed_pack_id,
                    crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
                )
                .expect("seed object pack path"),
                pack_format: crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                member_count: 1,
                total_bytes: 64,
                created_at: "2026-07-23T00:00:02Z".to_string(),
                members: vec![BinaryDbObjectPackMemberWriteInput {
                    blob_id: seed_blob_id.clone(),
                    sha256: hex_lower(&seed_sha),
                    size_bytes: 24,
                    pack_entry_type: "full".to_string(),
                    pack_base_blob_id: None,
                    pack_chain_depth: 0,
                    created_at: "2026-07-23T00:00:02Z".to_string(),
                }],
            },
        )
        .expect("seed blob before tree rollback");

    let leaf_tree_id = "TRE-2122232425262728292A";
    let invalid_root_tree_id = "TRE-3132333435363738393A";
    let missing_tree_id = "TRE-4142434445464748494A";
    let leaf_tree_pack_id = tree_pack_id_from_hash48(0xA0A1_A2A3_A4A5);
    let invalid_tree_pack_id = tree_pack_id_from_hash48(0xB0B1_B2B3_B4B5);
    let tree_error = coordinator
        .record_tree_pack_metadata_batch_with_entries(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &[
                (
                    BinaryDbTreePackWriteInput {
                        pack_id: leaf_tree_pack_id.clone(),
                        pack_rel_path: tree_pack_relative_path(
                            &leaf_tree_pack_id,
                            crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                        )
                        .expect("leaf tree pack path"),
                        pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1
                            .to_string(),
                        tree_count: 1,
                        total_bytes: 64,
                        created_at: "2026-07-23T00:00:03Z".to_string(),
                        trees: vec![BinaryDbTreePackTreeWriteInput {
                            tree_id: leaf_tree_id.to_string(),
                            entry_count: 1,
                        }],
                    },
                    vec![BinaryDbTreeEntryWriteInput {
                        tree_id: leaf_tree_id.to_string(),
                        entry_name: "seed.txt".to_string(),
                        entry_type: "blob".to_string(),
                        target_id: seed_blob_id,
                        mode: "100644".to_string(),
                    }],
                ),
                (
                    BinaryDbTreePackWriteInput {
                        pack_id: invalid_tree_pack_id.clone(),
                        pack_rel_path: tree_pack_relative_path(
                            &invalid_tree_pack_id,
                            crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                        )
                        .expect("invalid tree pack path"),
                        pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1
                            .to_string(),
                        tree_count: 1,
                        total_bytes: 64,
                        created_at: "2026-07-23T00:00:04Z".to_string(),
                        trees: vec![BinaryDbTreePackTreeWriteInput {
                            tree_id: invalid_root_tree_id.to_string(),
                            entry_count: 1,
                        }],
                    },
                    vec![BinaryDbTreeEntryWriteInput {
                        tree_id: invalid_root_tree_id.to_string(),
                        entry_name: "missing".to_string(),
                        entry_type: "tree".to_string(),
                        target_id: missing_tree_id.to_string(),
                        mode: "tree".to_string(),
                    }],
                ),
            ],
        )
        .expect_err("missing cross-pack tree must abort the complete tree batch");
    assert!(tree_error.to_string().contains("is missing for tree entry"));
    assert_eq!(
        db.record_count(BinaryTreePackCodec::<TEST_LAYOUT>::record_file())
            .expect("tree-pack count after rollback"),
        0
    );
    assert_eq!(
        db.record_count(BinaryTreeCodec::<TEST_LAYOUT>::record_file())
            .expect("tree count after rollback"),
        0
    );

    let valid_tree_id = "TRE-5152535455565758595A";
    let valid_tree_pack_id = tree_pack_id_from_hash48(0xC0C1_C2C3_C4C5);
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &BinaryDbTreePackWriteInput {
                pack_id: valid_tree_pack_id.clone(),
                pack_rel_path: tree_pack_relative_path(
                    &valid_tree_pack_id,
                    crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                )
                .expect("valid tree pack path"),
                pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 1,
                total_bytes: 64,
                created_at: "2026-07-23T00:00:05Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: valid_tree_id.to_string(),
                    entry_count: 0,
                }],
            },
        )
        .expect("seed valid tree pack before snapshot rollback");
    BinaryDbLineStore::<_, TEST_LAYOUT>::new(db.clone())
        .create_line("main", None, "2026-07-23T00:00:05Z")
        .expect("create main line");

    let root_snapshot_id = snapshot_id_from_hash48(0xD0D1_D2D3_D4D5);
    let invalid_child_snapshot_id = snapshot_id_from_hash48(0xE0E1_E2E3_E4E5);
    let missing_parent_snapshot_id = snapshot_id_from_hash48(0xF0F1_F2F3_F4F5);
    let snapshot_error = coordinator
        .record_snapshots(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &[
                BinaryDbSnapshotWriteInput {
                    snapshot_id: root_snapshot_id,
                    parent_snapshot_ids: Vec::new(),
                    root_tree_pack_id: valid_tree_pack_id.clone(),
                    root_entry_ordinal: 0,
                    manifest_hash: hex_lower(&sha256(b"rollback root snapshot")),
                    message: Some("rollback root".to_string()),
                    line_name: "main".to_string(),
                    snapshot_kind: "line".to_string(),
                    file_count: 0,
                    total_bytes: 0,
                    created_at: "2026-07-23T00:00:06Z".to_string(),
                },
                BinaryDbSnapshotWriteInput {
                    snapshot_id: invalid_child_snapshot_id,
                    parent_snapshot_ids: vec![missing_parent_snapshot_id],
                    root_tree_pack_id: valid_tree_pack_id,
                    root_entry_ordinal: 0,
                    manifest_hash: hex_lower(&sha256(b"rollback invalid child snapshot")),
                    message: Some("rollback invalid child".to_string()),
                    line_name: "main".to_string(),
                    snapshot_kind: "line".to_string(),
                    file_count: 0,
                    total_bytes: 0,
                    created_at: "2026-07-23T00:00:07Z".to_string(),
                },
            ],
        )
        .expect_err("missing parent must abort the complete snapshot batch");
    assert!(snapshot_error.to_string().contains("parent snapshot"));
    assert_eq!(
        db.record_count(BinarySnapshotCodec::<TEST_LAYOUT>::record_file())
            .expect("snapshot count after rollback"),
        0
    );
}

#[test]
fn content_write_coordinator_rejects_scope_without_local_content_lock() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db, temp.path());
    let coordinator = BinaryDbContentWriteCoordinator::new(
        &blob_store,
        &object_pack_store,
        &tree_pack_store,
        &tree_store,
        &snapshot_store,
    );
    let pack_id = tree_pack_id_from_hash48(0x2122_2324_2526);
    let error = coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::PlanSyncRemote,
            &BinaryDbTreePackWriteInput {
                pack_id: pack_id.clone(),
                pack_rel_path: tree_pack_relative_path(
                    &pack_id,
                    crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                )
                .expect("tree pack path"),
                pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 0,
                total_bytes: 0,
                created_at: "2026-07-10T00:00:00Z".to_string(),
                trees: Vec::new(),
            },
        )
        .expect_err("remote authority scope must not guard local content files");
    assert_eq!(error.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(error.to_string().contains("content.write.lock"));
}

#[test]
fn content_binary_db_write_coordinator_records_pack_tree_and_snapshot_metadata() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let line_store = BinaryDbLineStore::<_, TEST_LAYOUT>::new(db.clone());
    line_store
        .create_line("main", None, "2026-07-07T00:00:00Z")
        .expect("create canonical line");
    let coordinator = BinaryDbContentWriteCoordinator::new(
        &blob_store,
        &object_pack_store,
        &tree_pack_store,
        &tree_store,
        &snapshot_store,
    );

    let blob_bytes = b"coordinator blob\n";
    let blob_sha = sha256(blob_bytes);
    let blob_sha_hex = hex_lower(&blob_sha);
    let blob_id = blob_id_from_sha256(&blob_sha);
    let object_pack_id = object_pack_id_from_hash48(0x0102_0304_0506);
    let object_pack_path = object_pack_relative_path(
        &object_pack_id,
        crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("object pack path");
    coordinator
        .record_object_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbObjectPackWriteInput {
                pack_id: object_pack_id.clone(),
                pack_rel_path: object_pack_path,
                pack_format: crate::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                member_count: 1,
                total_bytes: 256,
                created_at: "2026-07-07T00:00:00Z".to_string(),
                members: vec![BinaryDbObjectPackMemberWriteInput {
                    blob_id: blob_id.clone(),
                    sha256: blob_sha_hex.clone(),
                    size_bytes: blob_bytes.len() as i64,
                    pack_entry_type: "full".to_string(),
                    pack_base_blob_id: None,
                    pack_chain_depth: 0,
                    created_at: "2026-07-07T00:00:00Z".to_string(),
                }],
            },
        )
        .expect("record object pack metadata");
    let blob = blob_store
        .get_blob(&blob_id)
        .expect("get blob")
        .expect("blob record");
    assert_eq!(blob.object_pack_locator.unwrap().pack_id, object_pack_id);
    assert_eq!(blob.pack_entry_type.as_deref(), Some("full"));
    assert_eq!(blob.sha256, blob_sha_hex);
    assert_eq!(
        object_pack_store
            .list_object_pack_members(&object_pack_id)
            .expect("pack members")
            .len(),
        1
    );

    let tree_id = "TRE-0102030405060708090A";
    let tree_pack_id = tree_pack_id_from_hash48(0x0A0B_0C0D_0E0F);
    let tree_pack_path = tree_pack_relative_path(
        &tree_pack_id,
        crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("tree pack path");
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: tree_pack_id.clone(),
                pack_rel_path: tree_pack_path,
                pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 1,
                total_bytes: 128,
                created_at: "2026-07-07T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: tree_id.to_string(),
                    entry_count: 1,
                }],
            },
        )
        .expect("record tree pack metadata");
    let tree = tree_store
        .get_tree(tree_id)
        .expect("get tree")
        .expect("tree record");
    assert_eq!(tree.tree_pack_id.as_deref(), Some(tree_pack_id.as_str()));
    let read = tree_store.begin_read_txn();
    assert_eq!(
        tree_store
            .existing_tree_ids(&read)
            .expect("batch existing tree ids"),
        BTreeSet::from([tree_id.to_string()])
    );
    drop(read);

    let snapshot_id = snapshot_id_from_hash48(0x0B0C_0D0E_0F10);
    let manifest_hash = hex_lower(&sha256(b"manifest"));
    let imported = coordinator
        .record_snapshot(
            BinaryDbCommandScope::SnapshotWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: snapshot_id.clone(),
                parent_snapshot_ids: Vec::new(),
                root_tree_pack_id: tree_pack_id.clone(),
                root_entry_ordinal: 0,
                manifest_hash: manifest_hash.clone(),
                message: Some("coordinator".to_string()),
                line_name: "main".to_string(),
                snapshot_kind: "line".to_string(),
                file_count: 1,
                total_bytes: blob_bytes.len() as i64,
                created_at: "2026-07-07T00:00:00Z".to_string(),
            },
        )
        .expect("record snapshot");
    assert!(imported);
    let snapshot = snapshot_store
        .snapshot_by_id(&snapshot_id)
        .expect("snapshot by id")
        .expect("snapshot record");
    assert_eq!(
        snapshot.root_tree_pack_id.as_deref(),
        Some(tree_pack_id.as_str())
    );
    assert_eq!(snapshot.manifest_hash, manifest_hash);
    assert_eq!(snapshot.message.as_deref(), Some("coordinator"));
    let read = snapshot_store.begin_read_txn();
    let snapshot_view = snapshot_store
        .get_snapshot_view(&read, &snapshot_id)
        .expect("snapshot view")
        .expect("snapshot exists");
    assert_eq!(snapshot_view.record.line_index_plus1, 1);
    drop(read);

    let missing_remote_parent_id = snapshot_id_from_hash48(0x1111_1111_1111);
    let boundary_snapshot_id = snapshot_id_from_hash48(0x2222_2222_2222);
    assert!(coordinator
        .record_snapshot_at_remote_head_history_boundary(
            BinaryDbCommandScope::RemoteSyncLocalImport,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: boundary_snapshot_id.clone(),
                parent_snapshot_ids: vec![missing_remote_parent_id],
                root_tree_pack_id: tree_pack_id.clone(),
                root_entry_ordinal: 0,
                manifest_hash: hex_lower(&sha256(b"remote boundary manifest")),
                message: Some("remote head boundary".to_string()),
                line_name: "main".to_string(),
                snapshot_kind: "line".to_string(),
                file_count: 1,
                total_bytes: blob_bytes.len() as i64,
                created_at: "2026-07-07T00:00:01Z".to_string(),
            },
        )
        .expect("record remote head boundary"));
    let read = snapshot_store.begin_read_txn();
    let boundary = snapshot_store
        .get_snapshot_view(&read, &boundary_snapshot_id)
        .expect("read remote head boundary")
        .expect("remote head boundary exists");
    assert_eq!(boundary.parent_snapshot_id, None);
    assert!(boundary.record.is_remote_head_history_boundary());
    drop(read);

    let child_snapshot_id = snapshot_id_from_hash48(0x3333_3333_3333);
    assert!(coordinator
        .record_snapshot(
            BinaryDbCommandScope::SnapshotWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: child_snapshot_id.clone(),
                parent_snapshot_ids: vec![boundary_snapshot_id.clone()],
                root_tree_pack_id: tree_pack_id.clone(),
                root_entry_ordinal: 0,
                manifest_hash: hex_lower(&sha256(b"child manifest")),
                message: Some("child after remote boundary".to_string()),
                line_name: "main".to_string(),
                snapshot_kind: "line".to_string(),
                file_count: 1,
                total_bytes: blob_bytes.len() as i64,
                created_at: "2026-07-07T00:00:02Z".to_string(),
            },
        )
        .expect("record child after remote boundary"));
    assert_eq!(
        snapshot_store
            .snapshot_chain(&child_snapshot_id)
            .expect("snapshot chain stops at explicit remote boundary"),
        vec![boundary_snapshot_id.clone(), child_snapshot_id.clone()]
    );

    let alternate_parent_ids = [
        snapshot_id_from_hash48(0x4444_4444_4444),
        snapshot_id_from_hash48(0x5555_5555_5555),
    ];
    for (offset, alternate_parent_id) in alternate_parent_ids.iter().enumerate() {
        assert!(coordinator
            .record_snapshot(
                BinaryDbCommandScope::SnapshotWrite,
                &BinaryDbSnapshotWriteInput {
                    snapshot_id: alternate_parent_id.clone(),
                    parent_snapshot_ids: Vec::new(),
                    root_tree_pack_id: tree_pack_id.clone(),
                    root_entry_ordinal: 0,
                    manifest_hash: hex_lower(&sha256(
                        format!("alternate parent {offset}").as_bytes()
                    )),
                    message: Some(format!("alternate parent {offset}")),
                    line_name: "main".to_string(),
                    snapshot_kind: "line".to_string(),
                    file_count: 1,
                    total_bytes: blob_bytes.len() as i64,
                    created_at: format!("2026-07-07T00:00:0{}Z", offset + 3),
                },
            )
            .expect("record alternate root parent"));
    }
    let merge_snapshot_id = snapshot_id_from_hash48(0x6666_6666_6666);
    let ordered_parents = vec![
        child_snapshot_id.clone(),
        alternate_parent_ids[0].clone(),
        alternate_parent_ids[1].clone(),
    ];
    assert!(coordinator
        .record_snapshot(
            BinaryDbCommandScope::SnapshotWrite,
            &BinaryDbSnapshotWriteInput {
                snapshot_id: merge_snapshot_id.clone(),
                parent_snapshot_ids: ordered_parents.clone(),
                root_tree_pack_id: tree_pack_id.clone(),
                root_entry_ordinal: 0,
                manifest_hash: hex_lower(&sha256(b"three-parent manifest")),
                message: Some("three-parent snapshot".to_string()),
                line_name: "main".to_string(),
                snapshot_kind: "line".to_string(),
                file_count: 1,
                total_bytes: blob_bytes.len() as i64,
                created_at: "2026-07-07T00:00:05Z".to_string(),
            },
        )
        .expect("record three-parent snapshot"));
    let read = snapshot_store.begin_read_txn();
    let merge = snapshot_store
        .get_snapshot_view(&read, &merge_snapshot_id)
        .expect("read merge snapshot")
        .expect("merge snapshot exists");
    let primary = snapshot_store
        .get_snapshot_view(&read, &child_snapshot_id)
        .expect("read primary parent")
        .expect("primary parent exists");
    assert_eq!(
        merge.parent_snapshot_ids, ordered_parents,
        "structured payload must preserve immutable parent order"
    );
    assert_eq!(
        merge.primary_parent_snapshot_id.as_deref(),
        Some(child_snapshot_id.as_str())
    );
    assert_eq!(
        merge.parent_snapshot_id, merge.primary_parent_snapshot_id,
        "compatibility projection must remain ordinal zero"
    );
    assert_eq!(
        merge.record.parent_snapshot_index(),
        Some(primary.snapshot_index),
        "snapshot.bin must store parent ordinal zero"
    );
    assert!(merge.record.has_additional_parents());
    assert_eq!(
        merge.payload.additional_parent_snapshot_indices.len(),
        2,
        "snapshot_payload.bin must store only parent ordinals one and two"
    );
    drop(read);
    assert!(!temp
        .path()
        .join("binary-db/snapshot_parent_edge.bin")
        .exists());
    assert!(!temp
        .path()
        .join("binary-db/snapshot_parent_child.idx")
        .exists());

    assert_header_u32(
        &temp.path().join("binary-db").join(OBJECT_PACK_BIN),
        TEST_LAYOUT,
    );
    assert_header_u32(
        &temp.path().join("binary-db").join(TREE_PACK_BIN),
        TEST_LAYOUT,
    );
    assert_header_u32(
        &temp.path().join("binary-db").join(SNAPSHOT_BIN),
        TEST_LAYOUT,
    );
}

#[test]
fn snapshot_fixed_parent_pointer_writes_are_atomic_idempotent_and_preserve_linear_records() {
    let temp = tempdir().expect("tempdir");
    let db = local_db(temp.path());
    let blob_store = BinaryDbBlobStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let object_pack_store = BinaryDbObjectPackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_pack_store = BinaryDbTreePackStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let tree_store = BinaryDbTreeStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let snapshot_store = BinaryDbSnapshotStore::<_, TEST_LAYOUT>::new(db.clone(), temp.path());
    let coordinator = BinaryDbContentWriteCoordinator::new(
        &blob_store,
        &object_pack_store,
        &tree_pack_store,
        &tree_store,
        &snapshot_store,
    );
    let tree_pack_id = tree_pack_id_from_hash48(0x7777_7777_7777);
    coordinator
        .record_tree_pack_metadata(
            BinaryDbCommandScope::ContentWrite,
            &BinaryDbTreePackWriteInput {
                pack_id: tree_pack_id.clone(),
                pack_rel_path: tree_pack_relative_path(
                    &tree_pack_id,
                    crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                )
                .expect("tree pack path"),
                pack_format: crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
                tree_count: 1,
                total_bytes: 0,
                created_at: "2026-07-18T00:00:00Z".to_string(),
                trees: vec![BinaryDbTreePackTreeWriteInput {
                    tree_id: "TRE-0102030405060708090A".to_string(),
                    entry_count: 0,
                }],
            },
        )
        .expect("record migration fixture tree pack");

    let root_id = snapshot_id_from_hash48(0x7777_0000_0001);
    let child_id = snapshot_id_from_hash48(0x7777_0000_0002);
    let legacy_records = [
        BinarySnapshotRecord {
            snapshot_meta: BinarySnapshotRecord::META_HAS_ROOT_LOCATOR,
            history_flags: 0,
            payload_len: 0,
            payload_offset: 0,
            snapshot_hash48: 0x7777_0000_0001,
            parent_snapshot_index_plus1: 0,
            root_tree_pack_index_plus1: 1,
            root_entry_ordinal: 0,
            line_index_plus1: 0,
            manifest_hash: sha256(b"legacy root manifest"),
            file_count: 0,
            total_bytes: 0,
            created_at_s: 1,
        },
        BinarySnapshotRecord {
            snapshot_meta: BinarySnapshotRecord::META_HAS_ROOT_LOCATOR,
            history_flags: 0,
            payload_len: 0,
            payload_offset: 0,
            snapshot_hash48: 0x7777_0000_0002,
            parent_snapshot_index_plus1: 1,
            root_tree_pack_index_plus1: 1,
            root_entry_ordinal: 0,
            line_index_plus1: 0,
            manifest_hash: sha256(b"legacy child manifest"),
            file_count: 0,
            total_bytes: 0,
            created_at_s: 2,
        },
    ];
    let mut seed = snapshot_store
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::SnapshotWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin legacy seed transaction");
    let mut persisted_legacy_records = Vec::new();
    for record in &legacy_records {
        let (_, _, persisted) = snapshot_store
            .append_snapshot_with_id_index(
                &mut seed,
                record.clone(),
                &BinarySnapshotPayload {
                    line_name: "main".to_string(),
                    message: Some("legacy".to_string()),
                    additional_parent_snapshot_indices: Vec::new(),
                },
            )
            .expect("append legacy snapshot");
        persisted_legacy_records.push(persisted);
    }
    seed.commit().expect("commit legacy seed");
    assert_eq!(
        snapshot_store
            .snapshot_parent_link(&child_id)
            .expect("legacy parent link")
            .expect("legacy child")
            .parent_snapshot_ids,
        vec![root_id.clone()]
    );

    let new_id = snapshot_id_from_hash48(0x7777_0000_0003);
    let mut new_input = BinaryDbSnapshotWriteInput {
        snapshot_id: new_id.clone(),
        parent_snapshot_ids: vec![child_id.clone()],
        root_tree_pack_id: "TPK-FFFFFFFFFFFF".to_string(),
        root_entry_ordinal: 0,
        manifest_hash: hex_lower(&sha256(b"new child manifest")),
        message: Some("new child".to_string()),
        line_name: "main".to_string(),
        snapshot_kind: "line".to_string(),
        file_count: 0,
        total_bytes: 0,
        created_at: "2026-07-18T00:00:03Z".to_string(),
    };
    let error = coordinator
        .record_snapshot(BinaryDbCommandScope::SnapshotWrite, &new_input)
        .expect_err("post-activation validation failure must abort the whole transaction");
    assert!(error.to_string().contains("root tree pack"));
    let read = snapshot_store.begin_read_txn();
    for (index, expected) in persisted_legacy_records.iter().enumerate() {
        assert_eq!(
            snapshot_store
                .read_snapshot_record(&read, index as u32)
                .expect("read legacy record after aborted activation"),
            *expected
        );
    }
    assert_eq!(
        read.record_count(BinaryDbSnapshotStore::<LocalBinaryDbFs, TEST_LAYOUT>::snapshot_file())
            .expect("snapshot count after abort"),
        2
    );
    drop(read);
    assert!(!temp
        .path()
        .join("binary-db")
        .join("snapshot_parent_child.idx")
        .exists());

    new_input.root_tree_pack_id = tree_pack_id;
    assert!(coordinator
        .record_snapshot(BinaryDbCommandScope::SnapshotWrite, &new_input)
        .expect("retry atomic canonical parent write"));
    let read = snapshot_store.begin_read_txn();
    for (index, legacy) in persisted_legacy_records.iter().enumerate() {
        assert_eq!(
            snapshot_store
                .read_snapshot_record(&read, index as u32)
                .expect("read original linear record"),
            *legacy,
            "appending a child must not rewrite existing Snapshot records"
        );
    }
    assert_eq!(
        read.record_count(BinaryDbSnapshotStore::<LocalBinaryDbFs, TEST_LAYOUT>::snapshot_file())
            .expect("snapshot count after append"),
        3
    );
    drop(read);
    assert_eq!(
        snapshot_store
            .snapshot_parent_link(&new_id)
            .expect("new parent link")
            .expect("new snapshot")
            .parent_snapshot_ids,
        vec![child_id]
    );
    assert!(!coordinator
        .record_snapshot(BinaryDbCommandScope::SnapshotWrite, &new_input)
        .expect("activation replay"));
    let read = snapshot_store.begin_read_txn();
    assert_eq!(
        read.record_count(BinaryDbSnapshotStore::<LocalBinaryDbFs, TEST_LAYOUT>::snapshot_file())
            .expect("snapshot count after replay"),
        3,
        "idempotent replay must not duplicate canonical Snapshot records"
    );
    assert!(!temp
        .path()
        .join("binary-db/snapshot_parent_edge.bin")
        .exists());
}

#[test]
fn content_binary_db_local_and_remote_adapters_construct_stores() {
    let temp = tempdir().expect("tempdir");
    let local = LocalContentBinaryDb::<TEST_LAYOUT>::new(
        temp.path().join("local-db"),
        temp.path(),
        AuthorityId::new("local"),
        LocalStateScope::Repository,
    );
    assert_eq!(local.blobs().repo_root().as_path(), temp.path());
    assert_eq!(local.snapshots().repo_root().as_path(), temp.path());
    assert_eq!(local.object_packs().repo_root().as_path(), temp.path());
    assert_eq!(local.tree_packs().repo_root().as_path(), temp.path());
    assert_eq!(local.trees().repo_root().as_path(), temp.path());

    let remote_db = RemoteBinaryDbFs::test_fixture(
        temp.path().join("remote-db"),
        RepoId::new("REPO-TEST"),
        RepoName::new("ait-core"),
    );
    let remote = RemoteContentBinaryDb::<_, TEST_LAYOUT>::from_db(remote_db, temp.path());
    assert_eq!(remote.blobs().repo_root().as_path(), temp.path());
    assert_eq!(remote.snapshots().repo_root().as_path(), temp.path());
    assert_eq!(remote.object_packs().repo_root().as_path(), temp.path());
    assert_eq!(remote.tree_packs().repo_root().as_path(), temp.path());
    assert_eq!(remote.trees().repo_root().as_path(), temp.path());
}

#[test]
fn local_content_binary_db_snapshot_delta_depth_follows_persisted_parent_member() {
    let temp = tempdir().expect("tempdir");
    let local = LocalContentBinaryDb::<TEST_LAYOUT>::new(
        temp.path().join(".ait/binary-db"),
        temp.path(),
        AuthorityId::new("local"),
        LocalStateScope::Repository,
    );
    let workspace_file = temp.path().join("file.txt");
    let mut parent_snapshot_id = None;

    for (version, expected_depth) in [0_u8, 1, 2, 3, 4, 0].into_iter().enumerate() {
        let mut data = vec![b'a'; 4096];
        data[version] = b'A' + u8::try_from(version).expect("test version");
        fs::write(&workspace_file, data).expect("write workspace file");
        let snapshot = local
            .create_snapshot_content(
                "repo",
                "main",
                parent_snapshot_id.as_deref(),
                Some(&format!("version {version}")),
                false,
            )
            .expect("create Binary DB snapshot");
        let snapshot_id = snapshot["snapshot_id"]
            .as_str()
            .expect("snapshot id")
            .to_string();
        let blob_id = snapshot["files"][0]["blob_id"]
            .as_str()
            .expect("snapshot blob id");
        let read = local.blobs().begin_read_txn();
        let blob = local
            .blobs()
            .get_blob_view(&read, blob_id)
            .expect("read blob")
            .expect("blob exists");
        let member = local
            .object_packs()
            .object_pack_member_view_at(
                &read,
                blob.record.pack_member_index().expect("pack member index"),
            )
            .expect("read pack member");
        assert_eq!(
            member.record.delta_chain_depth, expected_depth,
            "version {version} must follow its persisted parent depth"
        );
        if expected_depth == 0 {
            assert!(member.base_blob_id.is_none());
            assert_eq!(
                member.record.member_kind(),
                BinaryObjectPackMemberKind::Full
            );
        } else {
            assert!(member.base_blob_id.is_some());
            assert_eq!(
                member.record.member_kind(),
                BinaryObjectPackMemberKind::Delta
            );
        }
        parent_snapshot_id = Some(snapshot_id);
    }
}

#[test]
fn content_binary_db_rejects_absolute_pack_paths() {
    let temp = tempdir().expect("tempdir");
    let err = absolute_repo_path(&StorePath::from(temp.path()), "/tmp/outside-pack.zip")
        .expect_err("absolute pack path should be rejected");
    assert!(err.contains("repo-relative"));
}
