use super::*;
use std::cell::RefCell;
use tempfile::tempdir;

struct StringOnlyFileIoStore;

impl FileIoStore for StringOnlyFileIoStore {
    fn home_dir(&self) -> Option<PathBuf> {
        None
    }

    fn path_exists(&self, _path: &Path) -> bool {
        false
    }

    fn read_to_string(&self, _path: &Path) -> FileIoResult<String> {
        Ok(String::new())
    }

    fn write_string(&self, _path: &Path, _text: &str) -> FileIoResult<()> {
        Ok(())
    }

    fn write_string_atomically(
        &self,
        _path: &Path,
        _text: &str,
        _publish_label: &str,
    ) -> FileIoResult<()> {
        Ok(())
    }
}

impl FileIoByteStore for StringOnlyFileIoStore {}

struct ByteCountingFileIoStore {
    reads: RefCell<usize>,
}

impl FileIoStore for ByteCountingFileIoStore {
    fn home_dir(&self) -> Option<PathBuf> {
        None
    }

    fn path_exists(&self, _path: &Path) -> bool {
        true
    }

    fn read_bytes(&self, _path: &Path) -> FileIoResult<Vec<u8>> {
        *self.reads.borrow_mut() += 1;
        Ok(b"ait".to_vec())
    }

    fn read_to_string(&self, _path: &Path) -> FileIoResult<String> {
        Ok(String::new())
    }

    fn write_string(&self, _path: &Path, _text: &str) -> FileIoResult<()> {
        Ok(())
    }

    fn write_string_atomically(
        &self,
        _path: &Path,
        _text: &str,
        _publish_label: &str,
    ) -> FileIoResult<()> {
        Ok(())
    }
}

#[test]
fn file_io_error_classifies_supported_io_kinds() {
    let cases = [
        (io::ErrorKind::NotFound, FileIoErrorKind::NotFound),
        (
            io::ErrorKind::PermissionDenied,
            FileIoErrorKind::PermissionDenied,
        ),
        (io::ErrorKind::AlreadyExists, FileIoErrorKind::AlreadyExists),
        (io::ErrorKind::InvalidInput, FileIoErrorKind::InvalidInput),
        (io::ErrorKind::InvalidData, FileIoErrorKind::InvalidData),
        (io::ErrorKind::Interrupted, FileIoErrorKind::Interrupted),
        (io::ErrorKind::WouldBlock, FileIoErrorKind::WouldBlock),
        (io::ErrorKind::UnexpectedEof, FileIoErrorKind::UnexpectedEof),
        (io::ErrorKind::WriteZero, FileIoErrorKind::WriteZero),
        (io::ErrorKind::Unsupported, FileIoErrorKind::Unsupported),
    ];

    for (io_kind, expected_kind) in cases {
        let err = FileIoError::from(io::Error::new(io_kind, "typed file I/O error"));
        assert_eq!(err.kind(), expected_kind);
        assert_eq!(err.to_string(), "typed file I/O error");
    }
}

#[test]
fn file_io_error_maps_unlisted_io_kinds_to_other() {
    let err = FileIoError::from(io::Error::new(io::ErrorKind::TimedOut, "timed out"));

    assert_eq!(err.kind(), FileIoErrorKind::Other);
    assert_eq!(err.to_string(), "timed out");
}

#[test]
fn file_io_error_from_io_message_preserves_custom_display_and_io_kind() {
    let source = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
    let err = FileIoError::from_io_message("custom permission message", source);

    assert_eq!(err.kind(), FileIoErrorKind::PermissionDenied);
    assert_eq!(err.message(), "custom permission message");
    assert_eq!(err.to_string(), "custom permission message");
}

#[test]
fn file_io_error_accepts_legacy_string_messages_as_other() {
    let err: FileIoError = "disk full".into();

    assert_eq!(err.kind(), FileIoErrorKind::Other);
    assert_eq!(err.message(), "disk full");
    assert_eq!(err.to_string(), "disk full");
}

#[test]
fn file_io_store_default_byte_reads_are_explicitly_unsupported() {
    let store = StringOnlyFileIoStore;
    let err = store.read_bytes(Path::new("payload.bin")).unwrap_err();

    assert_eq!(err.kind(), FileIoErrorKind::Unsupported);
    assert_eq!(
        err.to_string(),
        "File byte reads are not supported for payload.bin"
    );
}

#[test]
fn file_io_store_reference_delegates_byte_reads() {
    let store = ByteCountingFileIoStore {
        reads: RefCell::new(0),
    };
    let bytes = store.read_bytes(Path::new("payload.bin")).unwrap();

    assert_eq!(bytes, b"ait".to_vec());
    assert_eq!(*store.reads.borrow(), 1);
}

#[test]
fn file_io_byte_store_default_atomic_writes_are_explicitly_unsupported() {
    let store = StringOnlyFileIoStore;
    let err = store
        .write_bytes_atomically(Path::new("payload.bin"), b"ait", "test payload")
        .unwrap_err();

    assert_eq!(err.kind(), FileIoErrorKind::Unsupported);
    assert_eq!(
        err.to_string(),
        "Atomic file byte writes are not supported for payload.bin"
    );
}

#[test]
fn filesystem_atomic_byte_writes_create_replace_and_preserve_binary_payloads() {
    let temp = tempdir().expect("tempdir");
    let store = FilesystemFileIoStore;
    let path = temp.path().join("nested/payload.bin");

    store
        .write_bytes_atomically(&path, &[0, 255, 1], "binary payload")
        .expect("initial publish");
    assert_eq!(store.read_bytes(&path).expect("initial bytes"), [0, 255, 1]);

    store
        .write_bytes_atomically(&path, b"", "empty payload")
        .expect("replacement publish");
    assert_eq!(store.read_bytes(&path).expect("empty bytes"), b"");
    assert_eq!(
        fs::read_dir(path.parent().expect("payload parent"))
            .expect("payload parent entries")
            .count(),
        1
    );
}

#[test]
fn filesystem_atomic_byte_write_failure_removes_temporary_file() {
    let temp = tempdir().expect("tempdir");
    let store = FilesystemFileIoStore;
    let target_directory = temp.path().join("target.bin");
    fs::create_dir(&target_directory).expect("target directory");

    let err = store
        .write_bytes_atomically(&target_directory, b"payload", "binary payload")
        .expect_err("directory target must reject rename");

    assert!(err.to_string().contains("Failed to publish binary payload"));
    let entries = fs::read_dir(temp.path())
        .expect("temp root entries")
        .map(|entry| entry.expect("temp root entry").path())
        .collect::<Vec<_>>();
    assert_eq!(entries, vec![target_directory]);
}

#[test]
fn filesystem_append_bytes_returns_pre_append_offsets_and_range_reads() {
    let temp = tempdir().expect("tempdir");
    let store = FilesystemFileIoStore;
    let path = temp.path().join("nested/payload.bin");

    assert_eq!(store.metadata_len(&path).expect("missing metadata"), None);
    let first = store.append_bytes(&path, b"abc").expect("append first");
    let second = store.append_bytes(&path, b"de").expect("append second");

    assert_eq!(first, 0);
    assert_eq!(second, 3);
    assert_eq!(store.metadata_len(&path).expect("metadata"), Some(5));
    assert_eq!(store.read_range(&path, 1, 3).expect("range read"), b"bcd");
    assert_eq!(store.read_bytes(&path).expect("read all"), b"abcde");

    store.truncate_file(&path, 2).expect("truncate");
    assert_eq!(store.read_bytes(&path).expect("truncated"), b"ab");
    store.remove_file_if_exists(&path).expect("remove");
    store.remove_file_if_exists(&path).expect("remove missing");
    assert_eq!(store.metadata_len(&path).expect("removed metadata"), None);
}

#[test]
fn filesystem_process_lock_guard_clears_metadata_and_releases_on_drop() {
    let temp = tempdir().expect("tempdir");
    let store = FilesystemFileIoStore;
    let path = temp.path().join("locks/resource.write.lock");

    let mut first = store
        .acquire_process_lock(
            &path,
            FileIoLockMode::Exclusive,
            FileIoLockWait::Nonblocking,
        )
        .expect("first lock")
        .expect("first acquired");
    first
        .replace_contents_and_flush(b"scope=test\n")
        .expect("write metadata");

    let second = store
        .acquire_process_lock(
            &path,
            FileIoLockMode::Exclusive,
            FileIoLockWait::Nonblocking,
        )
        .expect("second lock attempt");
    assert!(second.is_none());
    assert_eq!(store.read_bytes(&path).expect("metadata"), b"scope=test\n");

    first.clear_contents_and_flush().expect("clear metadata");
    first.release().expect("release");
    assert_eq!(store.read_bytes(&path).expect("cleared metadata"), b"");

    let second = store
        .acquire_process_lock(
            &path,
            FileIoLockMode::Exclusive,
            FileIoLockWait::Nonblocking,
        )
        .expect("second lock after release");
    assert!(second.is_some());
}

#[test]
fn filesystem_shared_and_exclusive_process_locks_preserve_cross_handle_semantics() {
    let temp = tempdir().expect("tempdir");
    let store = FilesystemFileIoStore;
    let path = temp.path().join("locks/shared.write.lock");

    let mut first_shared = store
        .acquire_process_lock(&path, FileIoLockMode::Shared, FileIoLockWait::Nonblocking)
        .expect("first shared lock")
        .expect("first shared acquired");
    let mut second_shared = store
        .acquire_process_lock(&path, FileIoLockMode::Shared, FileIoLockWait::Nonblocking)
        .expect("second shared lock")
        .expect("second shared acquired");
    let exclusive = store
        .acquire_process_lock(
            &path,
            FileIoLockMode::Exclusive,
            FileIoLockWait::Nonblocking,
        )
        .expect("exclusive attempt");
    assert!(exclusive.is_none());

    first_shared.release().expect("release first shared");
    second_shared.release().expect("release second shared");
    let mut exclusive = store
        .acquire_process_lock(
            &path,
            FileIoLockMode::Exclusive,
            FileIoLockWait::Nonblocking,
        )
        .expect("exclusive after release");
    let shared = store
        .acquire_process_lock(&path, FileIoLockMode::Shared, FileIoLockWait::Nonblocking)
        .expect("shared attempt while exclusively locked");
    assert!(shared.is_none());
    exclusive
        .as_mut()
        .expect("exclusive acquired after shared releases")
        .release()
        .expect("release exclusive");
}
