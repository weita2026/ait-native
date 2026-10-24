use super::*;
use crate::file_io::{
    BoxedFileIoProcessLockGuard, FileIoByteStore, FileIoDurabilityStore, FileIoError,
    FileIoErrorKind, FileIoLockMode, FileIoLockStore, FileIoLockWait, FileIoProcessLockGuard,
    FileIoResult, FileIoStore, FilesystemFileIoStore,
};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fs::{self, read};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::tempdir;

fn new_db() -> (LocalBinaryDbFs, tempfile::TempDir, BinaryWriteContext) {
    let temp = tempdir().expect("tempdir");
    let db = LocalBinaryDbFs::new(
        temp.path(),
        temp.path(),
        AuthorityId::new("test-authority"),
        LocalStateScope::Repository,
    );
    (
        db,
        temp,
        BinaryWriteContext::test_fixture(BinaryDbCommandScope::General),
    )
}

fn collect_files(root: &Path) -> BTreeSet<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = BTreeSet::new();
    while let Some(current) = stack.pop() {
        let dir = match fs::read_dir(&current) {
            Ok(read_dir) => read_dir,
            Err(_) => continue,
        };
        for entry in dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .expect("path is within root")
                .to_path_buf();
            files.insert(relative);
        }
    }
    files
}

fn is_commit_marker_like_path(path: &Path) -> bool {
    let path_text = path.to_string_lossy();
    path_text.contains(".tmp") || path_text.contains("commit")
}

fn without_lock_files(files: &BTreeSet<PathBuf>) -> BTreeSet<PathBuf> {
    files
        .iter()
        .filter(|path| {
            path.components()
                .next()
                .is_none_or(|component| component.as_os_str() != ".locks")
        })
        .cloned()
        .collect()
}

#[derive(Clone, Default)]
struct RecordingFsyncPolicy {
    events: Rc<RefCell<Vec<(String, PathBuf)>>>,
}

impl RecordingFsyncPolicy {
    fn events(&self) -> Vec<(String, PathBuf)> {
        self.events.borrow().clone()
    }
}

#[derive(Clone, Debug, Default)]
struct TrackingBinaryDbFileIoStore {
    events: Rc<RefCell<Vec<&'static str>>>,
    fail_next_lock_release: Arc<AtomicBool>,
}

impl TrackingBinaryDbFileIoStore {
    fn with_lock_release_failure() -> Self {
        Self {
            fail_next_lock_release: Arc::new(AtomicBool::new(true)),
            ..Self::default()
        }
    }

    fn events(&self) -> Vec<&'static str> {
        self.events.borrow().clone()
    }

    fn record(&self, event: &'static str) {
        self.events.borrow_mut().push(event);
    }
}

#[derive(Debug)]
struct TrackingBinaryDbProcessLockGuard {
    inner: BoxedFileIoProcessLockGuard,
    fail_next_lock_release: Arc<AtomicBool>,
}

impl FileIoProcessLockGuard for TrackingBinaryDbProcessLockGuard {
    fn replace_contents_and_flush(&mut self, bytes: &[u8]) -> FileIoResult<()> {
        self.inner.replace_contents_and_flush(bytes)
    }

    fn clear_contents_and_flush(&mut self) -> FileIoResult<()> {
        self.inner.clear_contents_and_flush()
    }

    fn release(&mut self) -> FileIoResult<()> {
        if self.fail_next_lock_release.swap(false, Ordering::SeqCst) {
            return Err(FileIoError::new(
                FileIoErrorKind::Lock,
                "injected Binary DB lock release failure",
            ));
        }
        self.inner.release()
    }
}

impl FileIoStore for TrackingBinaryDbFileIoStore {
    fn home_dir(&self) -> Option<PathBuf> {
        FilesystemFileIoStore.home_dir()
    }

    fn path_exists(&self, path: &Path) -> bool {
        FilesystemFileIoStore.path_exists(path)
    }

    fn read_bytes(&self, path: &Path) -> FileIoResult<Vec<u8>> {
        self.record("read_bytes");
        FilesystemFileIoStore.read_bytes(path)
    }

    fn read_to_string(&self, path: &Path) -> FileIoResult<String> {
        FilesystemFileIoStore.read_to_string(path)
    }

    fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()> {
        FilesystemFileIoStore.write_string(path, text)
    }

    fn write_string_atomically(
        &self,
        path: &Path,
        text: &str,
        publish_label: &str,
    ) -> FileIoResult<()> {
        FilesystemFileIoStore.write_string_atomically(path, text, publish_label)
    }
}

impl FileIoByteStore for TrackingBinaryDbFileIoStore {
    fn write_bytes(&self, path: &Path, bytes: &[u8]) -> FileIoResult<()> {
        self.record("write_bytes");
        FilesystemFileIoStore.write_bytes(path, bytes)
    }

    fn read_range(&self, path: &Path, offset: u64, len: u32) -> FileIoResult<Vec<u8>> {
        self.record("read_range");
        FilesystemFileIoStore.read_range(path, offset, len)
    }

    fn metadata_len(&self, path: &Path) -> FileIoResult<Option<u64>> {
        self.record("metadata_len");
        FilesystemFileIoStore.metadata_len(path)
    }

    fn create_parent_dirs(&self, path: &Path) -> FileIoResult<()> {
        self.record("create_parent_dirs");
        FilesystemFileIoStore.create_parent_dirs(path)
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> FileIoResult<u64> {
        self.record("append_bytes");
        FilesystemFileIoStore.append_bytes(path, bytes)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> FileIoResult<()> {
        self.record("overwrite_range");
        FilesystemFileIoStore.overwrite_range(path, offset, bytes)
    }

    fn truncate_file(&self, path: &Path, len: u64) -> FileIoResult<()> {
        self.record("truncate_file");
        FilesystemFileIoStore.truncate_file(path, len)
    }

    fn remove_file_if_exists(&self, path: &Path) -> FileIoResult<()> {
        self.record("remove_file_if_exists");
        FilesystemFileIoStore.remove_file_if_exists(path)
    }
}

impl FileIoDurabilityStore for TrackingBinaryDbFileIoStore {
    fn sync_file(&self, path: &Path) -> FileIoResult<()> {
        self.record("sync_file");
        FilesystemFileIoStore.sync_file(path)
    }

    fn sync_dir(&self, path: &Path) -> FileIoResult<()> {
        self.record("sync_dir");
        FilesystemFileIoStore.sync_dir(path)
    }
}

impl FileIoLockStore for TrackingBinaryDbFileIoStore {
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: FileIoLockMode,
        wait: FileIoLockWait,
    ) -> FileIoResult<Option<BoxedFileIoProcessLockGuard>> {
        self.record("acquire_process_lock");
        Ok(FilesystemFileIoStore
            .acquire_process_lock(path, mode, wait)?
            .map(|inner| {
                Box::new(TrackingBinaryDbProcessLockGuard {
                    inner,
                    fail_next_lock_release: Arc::clone(&self.fail_next_lock_release),
                }) as BoxedFileIoProcessLockGuard
            }))
    }
}

impl BinaryDbFsyncPolicy for RecordingFsyncPolicy {
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.events
            .borrow_mut()
            .push(("file".to_string(), path.to_path_buf()));
        Ok(())
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.events
            .borrow_mut()
            .push(("dir".to_string(), path.to_path_buf()));
        Ok(())
    }
}

fn has_event(events: &[(String, PathBuf)], kind: &str, suffix: &str) -> bool {
    events
        .iter()
        .any(|(event_kind, path)| event_kind == kind && path.ends_with(suffix))
}

#[test]
fn fixed_record_append_read_count_and_layout_header() {
    let (db, temp, mut write) = new_db();
    let file = BinaryFileId::new("records/example.bin", 99, 16);

    let first = [0x11_u8; 16];
    let second = [0x22_u8; 16];
    let third = [0x33_u8; 16];

    let first_index = db
        .append_record(file.clone(), &first, &mut write)
        .expect("append first record");
    let second_index = db
        .append_record(file.clone(), &second, &mut write)
        .expect("append second record");
    let third_index = db
        .append_record(file.clone(), &third, &mut write)
        .expect("append third record");

    assert_eq!(first_index, 0);
    assert_eq!(second_index, 1);
    assert_eq!(third_index, 2);
    assert_eq!(db.record_count(file.clone()).expect("count"), 3);
    assert_eq!(db.layout_id(file.clone()).expect("layout"), 99);

    let read_txn = db.begin_read_txn();
    assert_eq!(
        read_txn.read_record(file.clone(), 0).expect("read first"),
        first
    );
    assert_eq!(
        read_txn.read_record(file.clone(), 1).expect("read second"),
        second
    );
    assert_eq!(
        read_txn.read_record(file.clone(), 2).expect("read third"),
        third
    );
    assert!(read_txn.read_record(file.clone(), 3).is_err());

    let bytes = read(temp.path().join("records/example.bin")).expect("read raw bytes");
    assert_eq!(&bytes[0..4], 99u32.to_le_bytes().as_slice());
    assert_eq!(bytes.len(), 4 + 16 * 3);
}

#[test]
fn read_transaction_reuses_validated_layout_record_and_index_file_bytes() {
    let temp = tempdir().expect("tempdir");
    let files = TrackingBinaryDbFileIoStore::default();
    let db = LocalBinaryDbFs::with_file_io_store(
        files.clone(),
        temp.path(),
        temp.path(),
        AuthorityId::new("read-cache-authority"),
        LocalStateScope::Repository,
    );
    let record_file = BinaryFileId::new("records/cached.bin", 111, 4);
    let index_file = BinaryIndexId::new_fixed("records/cached.idx", 112, 4, true);
    let first = [1_u8, 2, 3, 4];
    let second = [5_u8, 6, 7, 8];
    let third = [9_u8, 10, 11, 12];
    let mut write = BinaryWriteContext::test_fixture(BinaryDbCommandScope::General);
    let first_index = db
        .append_record(record_file.clone(), &first, &mut write)
        .expect("append first cached record");
    let second_index = db
        .append_record(record_file.clone(), &second, &mut write)
        .expect("append second cached record");
    db.append_index_candidate(index_file.clone(), b"one!", first_index, &mut write)
        .expect("index first cached record");
    db.append_index_candidate(index_file.clone(), b"two!", second_index, &mut write)
        .expect("index second cached record");

    let event_start;
    {
        let read = db.begin_read_txn();
        event_start = files.events().len();
        assert_eq!(
            read.layout_id(record_file.clone())
                .expect("read cached layout id"),
            111
        );
        assert_eq!(
            read.layout_id(record_file.clone())
                .expect("reread cached layout id"),
            111
        );
        let layout_events = files.events();
        let layout_events = &layout_events[event_start..];
        assert_eq!(
            layout_events
                .iter()
                .filter(|event| **event == "metadata_len")
                .count(),
            1,
            "one layout metadata check per transaction"
        );
        assert_eq!(
            layout_events
                .iter()
                .filter(|event| **event == "read_range")
                .count(),
            1,
            "one layout header read per transaction"
        );
        assert_eq!(
            read.read_record(record_file.clone(), first_index)
                .expect("read first cached record"),
            first
        );
        assert_eq!(
            read.read_record(record_file.clone(), second_index)
                .expect("read second cached record"),
            second
        );
        assert_eq!(
            read.read_record(record_file.clone(), first_index)
                .expect("reread first cached record"),
            first
        );
        assert_eq!(
            read.lookup_index(index_file.clone(), b"one!")
                .expect("lookup first cached key"),
            vec![first_index]
        );
        assert_eq!(
            read.lookup_index(index_file.clone(), b"two!")
                .expect("lookup second cached key"),
            vec![second_index]
        );
        assert!(read
            .lookup_index(index_file.clone(), b"none")
            .expect("lookup missing cached key")
            .is_empty());
    }
    let events = files.events();
    let first_read_events = &events[event_start..];
    assert_eq!(
        first_read_events
            .iter()
            .filter(|event| **event == "read_bytes")
            .count(),
        2,
        "one complete fixed-record read plus one complete index read"
    );

    let third_index = db
        .append_record(record_file.clone(), &third, &mut write)
        .expect("append third cached record after first read transaction");
    db.append_index_candidate(index_file.clone(), b"tri!", third_index, &mut write)
        .expect("index third cached record after first read transaction");

    let read = db.begin_read_txn();
    let event_start = files.events().len();
    assert_eq!(
        read.layout_id(record_file.clone())
            .expect("new transaction reloads layout id"),
        111
    );
    assert_eq!(
        files.events()[event_start..]
            .iter()
            .filter(|event| **event == "read_range")
            .count(),
        1,
        "a new transaction must reload the layout header"
    );
    assert_eq!(
        read.read_record(record_file.clone(), third_index)
            .expect("new transaction observes appended record"),
        third
    );
    assert_eq!(
        read.lookup_index(index_file, b"tri!")
            .expect("new transaction observes appended index entry"),
        vec![third_index]
    );
    assert_eq!(
        files.events()[event_start..]
            .iter()
            .filter(|event| **event == "read_bytes")
            .count(),
        2,
        "a new transaction must reload current record and index bytes"
    );
}

#[test]
fn fixed_record_batch_is_one_contiguous_append_and_preserves_dense_indexes() {
    let temp = tempdir().expect("tempdir");
    let files = TrackingBinaryDbFileIoStore::default();
    let db = LocalBinaryDbFs::with_file_io_store(
        files.clone(),
        temp.path(),
        temp.path(),
        AuthorityId::new("batch-authority"),
        LocalStateScope::Repository,
    );
    let file = BinaryFileId::new("records/batch.bin", 101, 4);
    let first_batch = [1_u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];

    let mut transaction = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::General, BinaryDbNoopFsyncPolicy)
        .expect("begin batch transaction");
    let event_start = files.events().len();
    let (start, count) = transaction
        .append_records(file.clone(), &first_batch)
        .expect("append first batch");
    let append_count = files.events()[event_start..]
        .iter()
        .filter(|event| **event == "append_bytes")
        .count();
    assert_eq!((start, count), (0, 3));
    assert_eq!(append_count, 2, "one header append plus one batch append");
    transaction.commit().expect("commit first batch");

    let second_batch = [13_u8, 14, 15, 16, 17, 18, 19, 20];
    let mut transaction = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::General, BinaryDbNoopFsyncPolicy)
        .expect("begin second batch transaction");
    let event_start = files.events().len();
    let (start, count) = transaction
        .append_records(file.clone(), &second_batch)
        .expect("append second batch");
    let append_count = files.events()[event_start..]
        .iter()
        .filter(|event| **event == "append_bytes")
        .count();
    assert_eq!((start, count), (3, 2));
    assert_eq!(append_count, 1, "existing file needs one batch append");
    let event_start = files.events().len();
    assert_eq!(
        transaction
            .append_records(file.clone(), &[])
            .expect("empty batch is a no-op"),
        (5, 0)
    );
    assert_eq!(
        files.events()[event_start..]
            .iter()
            .filter(|event| **event == "append_bytes")
            .count(),
        0
    );
    transaction.commit().expect("commit second batch");

    let read_txn = db.begin_read_txn();
    assert_eq!(read_txn.record_count(file.clone()).expect("count"), 5);
    assert_eq!(
        read_txn.read_record(file.clone(), 0).expect("first record"),
        [1_u8, 2, 3, 4]
    );
    assert_eq!(
        read_txn.read_record(file.clone(), 4).expect("last record"),
        [17_u8, 18, 19, 20]
    );
    let bytes = read(temp.path().join("records/batch.bin")).expect("read raw batch file");
    let mut expected_records = first_batch.to_vec();
    expected_records.extend_from_slice(&second_batch);
    assert_eq!(&bytes[..4], 101_u32.to_le_bytes().as_slice());
    assert_eq!(&bytes[4..], expected_records.as_slice());
}

#[test]
fn fixed_record_batch_rejects_partial_records_before_mutation() {
    let (db, temp, _write) = new_db();
    let file = BinaryFileId::new("records/partial-batch.bin", 102, 4);
    let mut transaction = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::General, BinaryDbNoopFsyncPolicy)
        .expect("begin batch transaction");
    let error = transaction
        .append_records(file, &[1_u8, 2, 3])
        .expect_err("partial batch must fail");
    assert_eq!(error.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(error.contains("not aligned to record size 4"));
    assert!(!temp.path().join("records/partial-batch.bin").exists());
    transaction.abort().expect("abort failed batch");
}

#[test]
fn payload_append_and_read_are_addressed() {
    let (db, temp, _write) = new_db();
    let payload = BinaryPayloadFileId::new("payload/example_payload.bin", 42);

    let first = vec![1_u8, 2, 3];
    let second = vec![9_u8, 8, 7, 6];

    let mut write_txn = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::General, BinaryDbNoopFsyncPolicy)
        .expect("begin write txn");
    let first_range = write_txn
        .append_payload(payload.clone(), &first)
        .expect("append first payload");
    let second_range = write_txn
        .append_payload(payload.clone(), &second)
        .expect("append second payload");
    write_txn.commit().expect("commit write transaction");

    assert_eq!(
        first_range,
        PayloadRange {
            payload_offset: u64::from(BIN_FILE_HEADER_BYTES),
            payload_len: 3
        }
    );
    assert_eq!(
        second_range,
        PayloadRange {
            payload_offset: u64::from(BIN_FILE_HEADER_BYTES) + 3,
            payload_len: 4
        }
    );

    let read_txn = db.begin_read_txn();
    assert_eq!(
        read_txn
            .read_payload(
                payload.clone(),
                first_range.payload_offset,
                first_range.payload_len
            )
            .expect("read first payload"),
        first
    );
    assert_eq!(
        read_txn
            .read_payload(
                payload.clone(),
                second_range.payload_offset,
                second_range.payload_len
            )
            .expect("read second payload"),
        second
    );

    let bytes = read(temp.path().join("payload/example_payload.bin")).expect("read payload raw");
    assert_eq!(&bytes[0..4], 42u32.to_le_bytes().as_slice());
}

#[test]
fn index_candidates_can_be_appended_and_looked_up() {
    let (db, _temp, _write) = new_db();
    let index = BinaryIndexId::new("index/line_name.idx", 11);
    let key_a = b"line/main";
    let key_b = b"line/dev";

    let mut write_txn = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::General, BinaryDbNoopFsyncPolicy)
        .expect("begin write txn");
    write_txn
        .append_index_candidate(index.clone(), key_a, 3)
        .expect("append key a");
    write_txn
        .append_index_candidate(index.clone(), key_b, 7)
        .expect("append key b");
    write_txn
        .append_index_candidate(index.clone(), key_a, 5)
        .expect("append key a duplicate");
    write_txn.commit().expect("commit write transaction");

    let read_txn = db.begin_read_txn();
    let found_a = read_txn
        .lookup_index(index.clone(), key_a)
        .expect("lookup key a");
    let found_b = read_txn
        .lookup_index(index.clone(), key_b)
        .expect("lookup key b");
    let found_missing = read_txn
        .lookup_index(index.clone(), b"line/missing")
        .expect("lookup missing");

    assert_eq!(found_a, vec![3, 5]);
    assert_eq!(found_b, vec![7]);
    assert!(found_missing.is_empty());
}

#[test]
fn index_candidates_are_parsed_once_per_read_transaction() {
    let (db, _temp, _write) = new_db();
    let index = BinaryIndexId::new_fixed("index/snapshot_id.idx", 12, 4, true);

    let mut write_txn = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::General, BinaryDbNoopFsyncPolicy)
        .expect("begin write txn");
    write_txn
        .append_index_candidate(index.clone(), b"aaaa", 0)
        .expect("append first key");
    write_txn
        .append_index_candidate(index.clone(), b"bbbb", 7)
        .expect("append second key");
    write_txn
        .append_index_candidate(index.clone(), b"aaaa", 9)
        .expect("append repeated key");
    write_txn.commit().expect("commit write transaction");

    let read_txn = db.begin_read_txn();
    assert_eq!(read_txn.cached_parsed_index_count(), 0);
    assert_eq!(
        read_txn
            .lookup_index(index.clone(), b"aaaa")
            .expect("lookup first key"),
        vec![0, 9]
    );
    assert_eq!(read_txn.cached_parsed_index_count(), 1);
    assert_eq!(
        read_txn
            .lookup_index(index.clone(), b"bbbb")
            .expect("lookup second key"),
        vec![7]
    );
    assert!(read_txn
        .lookup_index(index, b"cccc")
        .expect("lookup missing key")
        .is_empty());
    assert_eq!(read_txn.cached_parsed_index_count(), 1);
}

#[test]
fn variable_index_candidates_are_parsed_once_per_read_transaction() {
    let (db, _temp, _write) = new_db();
    let index = BinaryIndexId::new("index/tree_id.idx", 13);

    let mut write_txn = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::General, BinaryDbNoopFsyncPolicy)
        .expect("begin write txn");
    write_txn
        .append_index_candidate(index.clone(), b"tree-aaaaa", 3)
        .expect("append first key");
    write_txn
        .append_index_candidate(index.clone(), b"tree-bbbbb", 7)
        .expect("append second key");
    write_txn
        .append_index_candidate(index.clone(), b"tree-aaaaa", 5)
        .expect("append repeated key");
    write_txn.commit().expect("commit write transaction");

    let read_txn = db.begin_read_txn();
    assert_eq!(read_txn.cached_parsed_index_count(), 0);
    assert_eq!(
        read_txn
            .lookup_index(index.clone(), b"tree-aaaaa")
            .expect("lookup first key"),
        vec![3, 5]
    );
    assert_eq!(read_txn.cached_parsed_index_count(), 1);
    assert_eq!(
        read_txn
            .lookup_index(index.clone(), b"tree-bbbbb")
            .expect("lookup second key"),
        vec![7]
    );
    assert!(read_txn
        .lookup_index(index, b"tree-missing")
        .expect("lookup missing key")
        .is_empty());
    assert_eq!(read_txn.cached_parsed_index_count(), 1);
}

#[test]
fn local_binary_db_fs_uses_injected_file_io_primitives() {
    let temp = tempdir().expect("tempdir");
    let files = TrackingBinaryDbFileIoStore::default();
    let db = LocalBinaryDbFs::with_file_io_store(
        files.clone(),
        temp.path(),
        temp.path(),
        AuthorityId::new("tracked-authority"),
        LocalStateScope::Repository,
    );
    let record = BinaryFileId::new("records/tracked.bin", 21, 4);
    let payload = BinaryPayloadFileId::new("payload/tracked_payload.bin", 22);
    let index = BinaryIndexId::new("index/tracked.idx", 23);

    let mut write = db
        .begin_write_txn(BinaryDbCommandScope::PlanSyncLocal)
        .expect("begin write txn");
    write
        .append_record(record.clone(), &[1_u8, 2, 3, 4])
        .expect("append record");
    let payload_range = write
        .append_payload(payload.clone(), b"body")
        .expect("append payload");
    write
        .append_index_candidate(index.clone(), b"k", 0)
        .expect("append index");
    write.commit().expect("commit");

    let read = db.begin_read_txn();
    assert_eq!(
        read.read_record(record.clone(), 0).expect("read record"),
        [1_u8, 2, 3, 4]
    );
    assert_eq!(
        read.read_payload(
            payload.clone(),
            payload_range.payload_offset,
            payload_range.payload_len
        )
        .expect("read payload"),
        b"body"
    );
    assert_eq!(
        read.lookup_index(index.clone(), b"k").expect("index"),
        vec![0]
    );

    let events = files.events();
    for expected in [
        "acquire_process_lock",
        "create_parent_dirs",
        "metadata_len",
        "append_bytes",
        "read_range",
        "read_bytes",
        "sync_file",
        "sync_dir",
    ] {
        assert!(
            events.contains(&expected),
            "expected {expected} in events: {events:?}"
        );
    }
}

#[test]
fn write_txn_recovery_uses_injected_file_io_primitives() {
    let temp = tempdir().expect("tempdir");
    let files = TrackingBinaryDbFileIoStore::default();
    let db = LocalBinaryDbFs::with_file_io_store(
        files.clone(),
        temp.path(),
        temp.path(),
        AuthorityId::new("tracked-recovery-authority"),
        LocalStateScope::Repository,
    );
    let existing = BinaryFileId::new("records/existing.bin", 24, 2);
    let created = BinaryFileId::new("records/created.bin", 24, 2);

    let mut seed = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin seed transaction");
    seed.append_record(existing.clone(), &[1_u8, 2])
        .expect("append seed record");
    seed.commit().expect("commit seed transaction");
    let event_start = files.events().len();

    let mut aborted = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin aborted transaction");
    aborted
        .append_records(existing.clone(), &[3_u8, 4, 5, 6])
        .expect("append existing file batch");
    aborted
        .append_records(created, &[7_u8, 8, 9, 10])
        .expect("append created file batch");
    aborted.abort().expect("abort transaction");

    let recovery_events = files.events()[event_start..].to_vec();
    assert!(recovery_events.contains(&"truncate_file"));
    assert!(recovery_events.contains(&"remove_file_if_exists"));
    let read = db.begin_read_txn();
    assert_eq!(read.record_count(existing).expect("record count"), 1);
    assert!(!temp.path().join("records/created.bin").exists());
}

#[test]
fn public_binary_db_contract_exposes_no_raw_file_mutators() {
    let source = include_str!("contracts.rs");
    let public_contract = source
        .split_once("pub trait BinaryDb:")
        .expect("public BinaryDb trait")
        .1
        .split_once("#[derive(Clone, Copy, Debug)]")
        .expect("end of public BinaryDb trait")
        .0;

    for raw_mutator in ["fn truncate_file", "fn remove_file_if_exists"] {
        assert!(
            !public_contract.contains(raw_mutator),
            "public BinaryDb must not expose {raw_mutator}"
        );
    }
}

#[test]
fn durable_commit_reports_lock_cleanup_warning_without_rollback() {
    let temp = tempdir().expect("tempdir");
    let files = TrackingBinaryDbFileIoStore::with_lock_release_failure();
    let db = LocalBinaryDbFs::with_file_io_store(
        files,
        temp.path(),
        temp.path(),
        AuthorityId::new("commit-cleanup-authority"),
        LocalStateScope::Repository,
    );
    let record = BinaryFileId::new("records/commit_cleanup.bin", 71, 2);

    let mut tx = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin write transaction");
    tx.append_record(record.clone(), &[4_u8, 2])
        .expect("append committed record");
    let outcome = tx
        .commit()
        .expect("post-durable cleanup failure must not fail commit");
    assert!(tx.is_finished());
    assert!(!outcome.committed_cleanly());
    let warning = outcome
        .lock_cleanup_warning()
        .expect("lock cleanup warning");
    assert_eq!(warning.kind(), BinaryDbErrorKind::Io);
    assert!(warning.contains("release Binary DB command lock"));
    assert_eq!(tx.commit().expect("commit outcome is idempotent"), outcome);
    drop(tx);

    let read = db.begin_read_txn();
    assert_eq!(
        read.read_record(record, 0).expect("read committed record"),
        [4_u8, 2]
    );
}

#[test]
fn path_with_parent_traversal_is_rejected() {
    let (db, _temp, mut write) = new_db();
    let bad_record = BinaryFileId::new("../bad.bin", 1, 4);
    let record = [0_u8; 4];
    assert!(db
        .append_record(bad_record.clone(), &record, &mut write)
        .is_err());
    assert!(db.layout_id(bad_record.clone()).is_err());

    let bad_payload = BinaryPayloadFileId::new("/tmp/abs_payload.bin", 1);
    assert!(db
        .append_payload(bad_payload.clone(), &record, &mut write)
        .is_err());

    let bad_index = BinaryIndexId::new("../bad_index.idx", 1);
    assert!(db
        .append_index_candidate(bad_index, b"x", 1, &mut write)
        .is_err());
}

#[test]
fn binary_db_trait_object_is_usable() {
    let (db, _temp, mut write) = new_db();
    let file = BinaryFileId::new("records/object.bin", 77, 8);
    db.append_record(file.clone(), &[1_u8; 8], &mut write)
        .expect("append");

    let db_trait: &dyn BinaryDb = &db;
    let count = db_trait.record_count(file.clone()).expect("count");
    assert_eq!(count, 1);
    assert_eq!(db_trait.layout_id(file.clone()).expect("layout"), 77);
    assert_eq!(
        db_trait.read_record(file.clone(), 0).expect("read"),
        [1_u8; 8]
    );
}

#[test]
fn binary_db_errors_are_typed_and_keep_display_text() {
    let busy = BinaryDbError::retryable_busy("Binary DB writer is busy");
    assert_eq!(busy.kind(), BinaryDbErrorKind::RetryableBusy);
    assert!(busy.is_retryable_busy());
    assert_eq!(busy.to_string(), "Binary DB writer is busy");

    let layout = BinaryDbError::layout_mismatch("layout id mismatch");
    assert_eq!(layout.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert_eq!(layout.to_string(), "layout id mismatch");
}

#[test]
fn layout_mismatch_and_corruption_errors_are_distinct() {
    let (db, temp, mut write) = new_db();
    let file = BinaryFileId::new("records/layout.bin", 7, 4);
    db.append_record(file.clone(), &[1_u8; 4], &mut write)
        .expect("append");

    let wrong_layout = BinaryFileId::new("records/layout.bin", 8, 4);
    let layout_err = db.record_count(wrong_layout).expect_err("layout mismatch");
    assert_eq!(layout_err.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(layout_err.to_string().contains("layout id mismatch for"));

    let corrupt_path = temp.path().join("records/corrupt.bin");
    fs::create_dir_all(corrupt_path.parent().expect("parent")).expect("mkdir");
    fs::write(&corrupt_path, [1_u8, 2_u8]).expect("write corrupt file");

    let corrupt_file = BinaryFileId::new("records/corrupt.bin", 7, 4);
    let corrupt_err = db.record_count(corrupt_file).expect_err("corruption");
    assert_eq!(corrupt_err.kind(), BinaryDbErrorKind::Corruption);
    assert!(corrupt_err
        .to_string()
        .contains("too short to contain layout header"));
}

#[test]
fn remote_binary_db_fs_delegates_bytes_but_exposes_remote_identity() {
    let temp = tempdir().expect("tempdir");
    let db = RemoteBinaryDbFs::test_fixture(
        temp.path().join("remote-authority"),
        RepoId::new("repo-1"),
        RepoName::new("origin"),
    );
    let file = BinaryFileId::new("records/remote.bin", 31, 4);

    let mut tx = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncRemote,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin remote write txn");
    let index = tx
        .append_record(file.clone(), &[9_u8, 8, 7, 6])
        .expect("append remote record");
    tx.commit().expect("commit remote write txn");

    assert_eq!(index, 0);
    assert_eq!(db.remote_repo_id(), &RepoId::new("repo-1"));
    assert_eq!(db.remote_repo_name(), &RepoName::new("origin"));
    assert_eq!(db.remote_authority_root(), db.authority_root());
    assert_eq!(db.role(), RemoteBinaryDbFsRole::TestFixture);

    let read_txn = db.begin_read_txn();
    assert_eq!(read_txn.layout_id(file.clone()).expect("layout"), 31);
    assert_eq!(
        read_txn.read_record(file.clone(), index).expect("read"),
        [9_u8, 8, 7, 6]
    );

    let bytes = read(temp.path().join("remote-authority/records/remote.bin"))
        .expect("read remote raw bytes");
    assert_eq!(&bytes[0..4], 31_u32.to_le_bytes().as_slice());
}

#[test]
fn remote_binary_db_fs_requires_an_explicit_current_non_authoritative_role() {
    let temp = tempdir().expect("tempdir");
    let mirror = RemoteBinaryDbFs::local_mirror(
        temp.path().join("mirror"),
        RepoId::new("repo-1"),
        RepoName::new("origin"),
    );

    assert_eq!(mirror.role(), RemoteBinaryDbFsRole::LocalMirror);
    assert_eq!(mirror.role().as_str(), "local_mirror");
}

#[test]
fn command_scope_conflicts_follow_mutable_file_families() {
    assert!(BinaryDbCommandScope::PlanSyncLocal.conflicts_with(BinaryDbCommandScope::PlanImport));
    assert!(BinaryDbCommandScope::PlanSyncLocal.conflicts_with(BinaryDbCommandScope::SnapshotWrite));
    assert!(BinaryDbCommandScope::PlanImport.conflicts_with(BinaryDbCommandScope::ContentWrite));
    assert!(BinaryDbCommandScope::SnapshotWrite.conflicts_with(BinaryDbCommandScope::ContentWrite));
    assert!(
        BinaryDbCommandScope::PlanSyncLocalPlan.conflicts_with(BinaryDbCommandScope::PlanImport)
    );
    assert!(
        !BinaryDbCommandScope::PlanSyncRemote.conflicts_with(BinaryDbCommandScope::ContentWrite)
    );
    assert!(BinaryDbCommandScope::RemoteSyncLocalImport
        .conflicts_with(BinaryDbCommandScope::ContentWrite));
    assert!(BinaryDbCommandScope::RemoteSyncLocalImport
        .conflicts_with(BinaryDbCommandScope::SnapshotWrite));
    assert!(!BinaryDbCommandScope::RemoteSyncLocalImport
        .conflicts_with(BinaryDbCommandScope::PlanSyncRemote));
    assert!(BinaryDbCommandScope::Gc.conflicts_with(BinaryDbCommandScope::ContentWrite));
    assert!(BinaryDbCommandScope::Gc.conflicts_with(BinaryDbCommandScope::SnapshotWrite));
    assert!(BinaryDbCommandScope::Gc.conflicts_with(BinaryDbCommandScope::PlanSyncLocalPlan));
    assert!(BinaryDbCommandScope::Gc.conflicts_with(BinaryDbCommandScope::PlanSyncLocal));
    assert!(BinaryDbCommandScope::General.conflicts_with(BinaryDbCommandScope::PlanSyncRemote));
    assert!(BinaryDbCommandScope::PlanSyncRemote.conflicts_with(BinaryDbCommandScope::General));
}

#[test]
fn binary_write_capability_is_transaction_owned_scoped_and_invalidated() -> StoreResult<()> {
    let (db, temp, _) = new_db();
    let content_file = BinaryFileId::new("content.bin", 1, 2);
    let plan_file = BinaryFileId::new("plan.bin", 1, 2);

    let mut content_tx = db.begin_write_txn_with_fsync_policy(
        BinaryDbCommandScope::ContentWrite,
        BinaryDbNoopFsyncPolicy,
    )?;
    let capability_id = content_tx.write_context().transaction_id();
    assert!(content_tx.write_context().is_active());
    assert_eq!(
        content_tx.write_context().command_scope(),
        BinaryDbCommandScope::ContentWrite
    );
    assert_eq!(content_tx.append_record(content_file.clone(), b"ok")?, 0);
    let wrong_family = content_tx
        .append_record(plan_file.clone(), b"no")
        .expect_err("content capability must not mutate Plan files");
    assert_eq!(wrong_family.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(wrong_family.contains("cannot mutate Plan files"));
    assert!(!temp.path().join("plan.bin").exists());
    content_tx.commit()?;
    assert!(content_tx.is_finished());
    assert!(!content_tx.write_context().is_active());
    assert_eq!(content_tx.write_context().transaction_id(), capability_id);
    let after_commit = content_tx
        .append_record(content_file.clone(), b"no")
        .expect_err("finished transaction must reject mutation");
    assert_eq!(after_commit.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(after_commit.contains("already finished"));
    let raw_after_commit = db
        .append_record(content_file.clone(), b"no", content_tx.write_context())
        .expect_err("inactive raw capability must fail at substrate boundary");
    assert_eq!(
        raw_after_commit.kind(),
        BinaryDbErrorKind::InvalidDomainData
    );
    assert!(raw_after_commit.contains("no longer active"));

    let mut plan_tx = db.begin_write_txn_with_fsync_policy(
        BinaryDbCommandScope::PlanSyncLocalPlan,
        BinaryDbNoopFsyncPolicy,
    )?;
    let wrong_family = plan_tx
        .append_record(BinaryFileId::new("content-other.bin", 1, 2), b"no")
        .expect_err("Plan-only capability must not mutate content files");
    assert_eq!(wrong_family.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert_eq!(plan_tx.append_record(plan_file.clone(), b"ok")?, 0);
    plan_tx.abort()?;
    assert!(!temp.path().join("plan.bin").exists());

    let mut general = db.begin_write_txn_with_fsync_policy(
        BinaryDbCommandScope::General,
        BinaryDbNoopFsyncPolicy,
    )?;
    assert_eq!(general.append_record(plan_file, b"ok")?, 0);
    assert_eq!(
        general.append_record(BinaryFileId::new("general-content.bin", 1, 2), b"ok")?,
        0
    );
    general.commit()?;
    Ok(())
}

#[test]
fn read_txn_holds_shared_read_locks_and_preserves_data_files() {
    let (db, temp, mut write) = new_db();
    let file = BinaryFileId::new("records/readonly.bin", 8, 4);
    db.append_record(file.clone(), &[5_u8, 6, 7, 8], &mut write)
        .expect("append record");
    let payload = BinaryPayloadFileId::new("payload/readonly_payload.bin", 2);
    db.append_payload(payload.clone(), b"meta", &mut write)
        .expect("append payload");
    let index = BinaryIndexId::new("index/readonly.idx", 8);
    db.append_index_candidate(index.clone(), b"key", 0, &mut write)
        .expect("append index candidate");

    let before_files = collect_files(temp.path());
    let read_txn = db.begin_read_txn();
    let read_lock_paths = read_txn.read_lock_paths().expect("read lock paths");
    assert_eq!(
        read_lock_paths.len(),
        BinaryDbCommandScope::all_write_lock_file_names().len()
    );
    assert!(read_lock_paths
        .iter()
        .all(|path| path.starts_with(temp.path().join(".locks").join("binary-db"))));
    assert_eq!(read_txn.layout_id(file.clone()).expect("layout id"), 8);
    assert_eq!(
        read_txn.record_count(file.clone()).expect("record count"),
        1
    );
    assert_eq!(
        read_txn.read_record(file.clone(), 0).expect("read record"),
        [5_u8, 6, 7, 8]
    );
    assert_eq!(
        read_txn
            .lookup_index(index.clone(), b"key")
            .expect("index")
            .as_slice(),
        &[0]
    );
    assert_eq!(
        read_txn
            .read_payload(payload.clone(), 4, 4)
            .expect("read payload"),
        b"meta"
    );
    let after_files = collect_files(temp.path());

    assert_eq!(
        without_lock_files(&before_files),
        without_lock_files(&after_files)
    );
    assert!(temp.path().join(".locks").join("binary-db").exists());
}

#[test]
fn detached_generation_transactions_never_create_runtime_locks() {
    let temp = tempdir().expect("tempdir");
    let db = LocalBinaryDbFs::new(
        temp.path(),
        temp.path(),
        AuthorityId::new("detached-generation"),
        LocalStateScope::Repository,
    )
    .for_detached_generation_without_locks();
    let file = BinaryFileId::new("detached.bin", 1, 4);

    let read = db.begin_read_txn();
    assert!(read.read_lock_paths().expect("read lock paths").is_empty());
    drop(read);

    let mut write = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::General, BinaryDbNoopFsyncPolicy)
        .expect("begin detached generation write");
    assert!(write.lock_paths().is_empty());
    assert_eq!(
        write
            .append_record(file.clone(), &[1_u8, 2, 3, 4])
            .expect("append detached record"),
        0
    );
    write.commit().expect("commit detached generation write");

    let read = db.begin_read_txn();
    assert_eq!(read.record_count(file).expect("detached record count"), 1);
    assert!(!temp.path().join(".locks").exists());
}

#[test]
fn read_txn_reports_retryable_busy_while_writer_holds_lock() {
    let (db, _temp, _write) = new_db();
    let file = BinaryFileId::new("records/read_committed.bin", 12, 4);
    let mut tx = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin write txn");
    tx.append_record(file.clone(), &[1_u8, 2, 3, 4])
        .expect("append uncommitted record");

    let read_txn = db.begin_read_txn();
    let err = read_txn
        .record_count(file.clone())
        .expect_err("writer lock should make ordinary reader busy");
    assert_eq!(err.kind(), BinaryDbErrorKind::RetryableBusy);
    assert!(err.is_retryable_busy());

    tx.commit().expect("commit writer");
    let read_txn = db.begin_read_txn();
    assert_eq!(read_txn.record_count(file).expect("committed count"), 1);
}

#[test]
fn scoped_read_txn_ignores_disjoint_writers_and_blocks_matching_writers() {
    let (db, _temp, _write) = new_db();

    assert_eq!(
        BinaryDbReadScope::Content.lock_file_names(),
        ["content.write.lock", "remote-content.write.lock"]
    );
    assert_eq!(
        BinaryDbReadScope::Plan.lock_file_names(),
        ["plan.write.lock", "remote-plan.write.lock"]
    );

    let mut content_writer = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin content writer");
    let plan_read = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::Plan);
    assert_eq!(
        plan_read
            .read_lock_paths()
            .expect("disjoint Plan read")
            .len(),
        2
    );
    let content_read = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::Content);
    let busy = content_read
        .read_lock_paths()
        .expect_err("matching content read must be busy");
    assert_eq!(busy.kind(), BinaryDbErrorKind::RetryableBusy);
    drop(plan_read);
    content_writer.abort().expect("abort content writer");

    let mut plan_writer = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocalPlan,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin Plan-only writer");
    let content_read = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::Content);
    assert_eq!(
        content_read
            .read_lock_paths()
            .expect("disjoint content read")
            .len(),
        2
    );
    let plan_read = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::Plan);
    let busy = plan_read
        .read_lock_paths()
        .expect_err("Plan-only writer must block Plan read");
    assert_eq!(busy.kind(), BinaryDbErrorKind::RetryableBusy);
    drop(content_read);
    plan_writer.abort().expect("abort Plan-only writer");
}

#[test]
fn command_scopes_define_local_and_remote_plan_sync_write_locks() {
    assert_eq!(
        BinaryDbCommandScope::PlanSyncLocal.lock_file_names(),
        ["content.write.lock", "plan.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::PlanSyncRemote.lock_file_names(),
        ["remote-content.write.lock", "remote-plan.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::RemoteSyncLocalImport.lock_file_names(),
        ["content.write.lock"]
    );
    assert_eq!(
        BinaryDbCommandScope::Gc.lock_file_names(),
        [
            "content.write.lock",
            "gc.write.lock",
            "plan.write.lock",
            "snapshot.write.lock",
        ]
    );
    assert_eq!(
        BinaryDbCommandScope::General.lock_file_names(),
        BinaryDbCommandScope::all_write_lock_file_names()
    );
}

#[test]
fn gc_command_lock_serializes_with_each_local_mutable_domain() {
    let (_db, temp, _write) = new_db();
    let authority_root = StorePath::from(temp.path());

    for writer_scope in [
        BinaryDbCommandScope::ContentWrite,
        BinaryDbCommandScope::RemoteSyncLocalImport,
        BinaryDbCommandScope::SnapshotWrite,
        BinaryDbCommandScope::PlanSyncLocalPlan,
        BinaryDbCommandScope::PlanSyncLocal,
    ] {
        let writer = BinaryDbCommandLockSet::acquire(&authority_root, writer_scope)
            .expect("acquire local writer lock");
        assert!(
            BinaryDbCommandLockSet::try_acquire(&authority_root, BinaryDbCommandScope::Gc)
                .expect("try GC lock while local writer is active")
                .is_none(),
            "GC must conflict with {writer_scope:?}"
        );
        drop(writer);

        let gc = BinaryDbCommandLockSet::acquire(&authority_root, BinaryDbCommandScope::Gc)
            .expect("acquire GC lock");
        assert!(
            BinaryDbCommandLockSet::try_acquire(&authority_root, writer_scope)
                .expect("try local writer while GC is active")
                .is_none(),
            "{writer_scope:?} must conflict with GC"
        );
        drop(gc);
    }
}

#[test]
fn write_txn_uses_command_scoped_external_command_locks() {
    let (db, temp, _write) = new_db();

    let mut tx = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::PlanSyncLocal,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin write txn");

    assert_eq!(tx.command_scope(), BinaryDbCommandScope::PlanSyncLocal);
    let lock_paths = tx.lock_paths();
    assert_eq!(lock_paths.len(), 2);
    assert!(lock_paths
        .iter()
        .all(|path| path.starts_with(temp.path().join(".locks").join("binary-db"))));
    assert!(lock_paths
        .iter()
        .any(|path| path.ends_with("content.write.lock")));
    assert!(lock_paths
        .iter()
        .any(|path| path.ends_with("plan.write.lock")));
    assert!(lock_paths
        .iter()
        .all(|path| !path.starts_with(temp.path().join("records"))));

    let first_lock = fs::read_to_string(&lock_paths[0]).expect("lock metadata");
    assert!(first_lock.contains("PlanSyncLocal"));

    tx.abort().expect("abort write txn");
    for path in lock_paths {
        assert_eq!(read(path).expect("lock metadata cleared"), b"");
    }
}

#[test]
fn command_lock_try_acquire_reports_contention_until_release() {
    let (_db, temp, _write) = new_db();
    let authority_root = StorePath::from(temp.path());
    let mut first = BinaryDbCommandLockSet::acquire(&authority_root, BinaryDbCommandScope::General)
        .expect("acquire first lock");
    let second =
        BinaryDbCommandLockSet::try_acquire(&authority_root, BinaryDbCommandScope::General)
            .expect("try second lock");
    assert!(second.is_none());

    first.release().expect("release first lock");
    let second =
        BinaryDbCommandLockSet::try_acquire(&authority_root, BinaryDbCommandScope::General)
            .expect("try second lock after release");
    assert!(second.is_some());
}

#[test]
fn command_lock_release_is_idempotent_and_reports_scope_and_paths() {
    let (_db, temp, _write) = new_db();
    let authority_root = StorePath::from(temp.path());
    let mut lock =
        BinaryDbCommandLockSet::acquire(&authority_root, BinaryDbCommandScope::PlanSyncLocal)
            .expect("acquire command lock set");

    assert_eq!(lock.scope(), BinaryDbCommandScope::PlanSyncLocal);
    assert_eq!(lock.command_scope(), BinaryDbCommandScope::PlanSyncLocal);
    assert_eq!(lock.paths().len(), 2);
    assert!(lock
        .paths()
        .iter()
        .all(|path| path.starts_with(temp.path().join(".locks").join("binary-db"))));

    lock.release().expect("release once");
    lock.release().expect("release twice");
    for path in lock.paths() {
        assert_eq!(read(path).expect("lock metadata cleared"), b"");
    }
}

#[test]
fn command_lock_drop_releases_lock_set() {
    let (_db, temp, _write) = new_db();
    let authority_root = StorePath::from(temp.path());
    let lock = BinaryDbCommandLockSet::acquire(&authority_root, BinaryDbCommandScope::General)
        .expect("acquire command lock set");
    let paths = lock.paths().to_vec();
    assert!(
        BinaryDbCommandLockSet::try_acquire(&authority_root, BinaryDbCommandScope::General)
            .expect("try acquire while held")
            .is_none()
    );

    drop(lock);

    assert!(
        BinaryDbCommandLockSet::try_acquire(&authority_root, BinaryDbCommandScope::General)
            .expect("try acquire after drop")
            .is_some()
    );
    for path in paths {
        assert_eq!(read(path).expect("lock metadata cleared"), b"");
    }
}

#[test]
fn write_txn_commit_fsyncs_touched_files_and_directories() {
    let (db, _temp, _write) = new_db();
    let policy = RecordingFsyncPolicy::default();
    let mut tx = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::PlanSyncLocal, policy.clone())
        .expect("begin write txn");

    let record = BinaryFileId::new("records/commit.bin", 1, 2);
    let payload = BinaryPayloadFileId::new("payload/commit_payload.bin", 1);
    let index = BinaryIndexId::new("index/commit.idx", 1);

    tx.append_record(record.clone(), &[1_u8, 2])
        .expect("append record");
    tx.append_payload(payload.clone(), b"ok")
        .expect("append payload");
    tx.append_index_candidate(index.clone(), b"commit", 0)
        .expect("append index");

    assert_eq!(tx.touched_files().len(), 3);
    assert_eq!(tx.touched_directories().len(), 3);
    tx.commit().expect("commit write txn");

    let events = policy.events();
    assert!(has_event(&events, "file", "records/commit.bin"));
    assert!(has_event(&events, "file", "payload/commit_payload.bin"));
    assert!(has_event(&events, "file", "index/commit.idx"));
    assert!(has_event(&events, "dir", "records"));
    assert!(has_event(&events, "dir", "payload"));
    assert!(has_event(&events, "dir", "index"));
}

#[test]
fn write_txn_abort_does_not_fsync_or_create_commit_marker_like_files() {
    let (db, temp, _write) = new_db();
    let before_files = collect_files(temp.path());
    let policy = RecordingFsyncPolicy::default();
    let mut tx = db
        .begin_write_txn_with_fsync_policy(BinaryDbCommandScope::PlanSyncLocal, policy.clone())
        .expect("begin write txn");

    let record = BinaryFileId::new("records/rollback.bin", 1, 2);
    tx.append_record(record.clone(), &[1_u8, 2])
        .expect("append record");

    let payload = BinaryPayloadFileId::new("payload/rollback_payload.bin", 1);
    tx.append_payload(payload.clone(), b"ok")
        .expect("append payload");
    tx.abort().expect("abort txn");

    assert!(policy.events().is_empty());

    let after_files = collect_files(temp.path());
    let added: BTreeSet<PathBuf> = after_files.difference(&before_files).cloned().collect();
    let added_data = without_lock_files(&added);
    assert!(
        added_data.is_empty(),
        "unexpected data files: {added_data:?}"
    );
    assert!(!temp.path().join("records/rollback.bin").exists());
    assert!(!temp.path().join("payload/rollback_payload.bin").exists());
    assert!(added.iter().all(|path| !is_commit_marker_like_path(path)));
}

#[test]
fn write_txn_abort_rolls_back_existing_file_lengths() {
    let (db, temp, _write) = new_db();
    let record = BinaryFileId::new("records/truncate.bin", 31, 2);
    let payload = BinaryPayloadFileId::new("payload/truncate_payload.bin", 31);
    let index = BinaryIndexId::new("index/truncate.idx", 31);

    let mut committed = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin committed txn");
    committed
        .append_record(record.clone(), &[1_u8, 2])
        .expect("append committed record");
    let committed_payload = committed
        .append_payload(payload.clone(), b"ok")
        .expect("append committed payload");
    committed
        .append_index_candidate(index.clone(), b"key", 0)
        .expect("append committed index");
    committed.commit().expect("commit");

    let record_path = temp.path().join("records/truncate.bin");
    let payload_path = temp.path().join("payload/truncate_payload.bin");
    let index_path = temp.path().join("index/truncate.idx");
    let record_len = fs::metadata(&record_path).expect("record metadata").len();
    let payload_len = fs::metadata(&payload_path).expect("payload metadata").len();
    let index_len = fs::metadata(&index_path).expect("index metadata").len();

    let mut aborted = db
        .begin_write_txn_with_fsync_policy(
            BinaryDbCommandScope::ContentWrite,
            BinaryDbNoopFsyncPolicy,
        )
        .expect("begin aborted txn");
    aborted
        .append_record(record.clone(), &[3_u8, 4])
        .expect("append uncommitted record");
    aborted
        .append_payload(payload.clone(), b"no")
        .expect("append uncommitted payload");
    aborted
        .append_index_candidate(index.clone(), b"key", 1)
        .expect("append uncommitted index");
    aborted.abort().expect("abort");

    assert_eq!(
        fs::metadata(&record_path).expect("record metadata").len(),
        record_len
    );
    assert_eq!(
        fs::metadata(&payload_path).expect("payload metadata").len(),
        payload_len
    );
    assert_eq!(
        fs::metadata(&index_path).expect("index metadata").len(),
        index_len
    );
    let read = db.begin_read_txn();
    assert_eq!(read.record_count(record).expect("record count"), 1);
    assert_eq!(
        read.read_payload(
            payload,
            committed_payload.payload_offset,
            committed_payload.payload_len
        )
        .expect("payload"),
        b"ok"
    );
    assert_eq!(read.lookup_index(index, b"key").expect("index"), vec![0]);
}

#[test]
fn write_txn_drop_rolls_back_uncommitted_files() {
    let (db, temp, _write) = new_db();
    let record = BinaryFileId::new("records/drop_rollback.bin", 41, 2);
    {
        let mut tx = db
            .begin_write_txn_with_fsync_policy(
                BinaryDbCommandScope::ContentWrite,
                BinaryDbNoopFsyncPolicy,
            )
            .expect("begin write txn");
        tx.append_record(record, &[8_u8, 9])
            .expect("append uncommitted record");
    }

    assert!(!temp.path().join("records/drop_rollback.bin").exists());
}
