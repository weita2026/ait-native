use super::*;

pub(super) fn make_temporary_root() -> StorePath {
    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);
    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos();
    let sequence = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ait-server-core-remote-binary-db-test-{}-{now_nanos}-{sequence}",
        std::process::id()
    ));
    StorePath::new(dir)
}

pub(super) fn make_db() -> (FilesystemServerRemoteBinaryDb, PathBuf, BinaryWriteContext) {
    let authority_root = make_temporary_root();
    let root_path = authority_root.as_path().to_path_buf();
    let db = FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("repo-uuid-001"),
        RepoName::new("repo-name"),
        authority_root.clone(),
        StoreGeneration::new(7),
    );
    (
        db,
        root_path,
        BinaryWriteContext::test_fixture(BinaryDbCommandScope::ServerWorkflow),
    )
}

pub(super) fn task_file_id() -> BinaryFileId {
    BinaryFileId::new(
        "task.bin",
        1,
        crate::foundation::workflow_binary_v0::TASK_RECORD_SIZE,
        BinaryDbFileFamily::Workflow,
    )
}

pub(super) fn task_payload_file_id() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new("task_payload.bin", 1, BinaryDbFileFamily::Workflow)
}

pub(super) fn task_change_index_id() -> BinaryIndexId {
    BinaryIndexId::new("task_change_index.idx", 1, BinaryDbFileFamily::Workflow)
}

pub(super) fn server_workflow_write_lock_path(root: &Path) -> PathBuf {
    scoped_write_lock_path(root, BinaryDbCommandScope::ServerWorkflow)
}

pub(super) fn scoped_write_lock_path(root: &Path, scope: BinaryDbCommandScope) -> PathBuf {
    root.join(".locks")
        .join("binary-db")
        .join(scope.lock_file_names()[0])
}

#[derive(Clone, Default)]
pub(super) struct RecordingFsyncPolicy {
    events: Rc<RefCell<Vec<String>>>,
}

impl RecordingFsyncPolicy {
    pub(super) fn events(&self) -> Vec<String> {
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

#[derive(Clone)]
pub(super) struct FailOnceDirectoryFsyncPolicy {
    fail_on_dir_event: usize,
    dir_events: Rc<RefCell<usize>>,
}

impl FailOnceDirectoryFsyncPolicy {
    pub(super) fn new(fail_on_dir_event: usize) -> Self {
        Self {
            fail_on_dir_event,
            dir_events: Rc::new(RefCell::new(0)),
        }
    }
}

impl BinaryDbFsyncPolicy for FailOnceDirectoryFsyncPolicy {
    fn sync_file(&self, _path: &Path) -> StoreResult<()> {
        Ok(())
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        let mut dir_events = self.dir_events.borrow_mut();
        *dir_events += 1;
        if *dir_events == self.fail_on_dir_event {
            return Err(BinaryDbError::new(
                BinaryDbErrorKind::Io,
                format!("injected directory fsync failure at {}", path.display()),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingServerBinaryDbStore {
    events: Rc<RefCell<Vec<String>>>,
}

impl RecordingServerBinaryDbStore {
    pub(super) fn events(&self) -> Vec<String> {
        self.events.borrow().clone()
    }

    pub(super) fn clear_events(&self) {
        self.events.borrow_mut().clear();
    }
}

impl ServerBinaryDbFileStore for RecordingServerBinaryDbStore {
    fn path_exists(&self, path: &Path) -> bool {
        ServerBinaryDbFilesystemStore.path_exists(path)
    }

    fn read_bytes(&self, path: &Path) -> StoreResult<Vec<u8>> {
        self.events
            .borrow_mut()
            .push(format!("read:{}", path.display()));
        ServerBinaryDbFilesystemStore.read_bytes(path)
    }

    fn read_to_string(&self, path: &Path) -> StoreResult<String> {
        ServerBinaryDbFilesystemStore.read_to_string(path)
    }
}

impl ServerBinaryDbByteStore for RecordingServerBinaryDbStore {
    fn read_range(&self, path: &Path, offset: u64, len: u32) -> StoreResult<Vec<u8>> {
        self.events
            .borrow_mut()
            .push(format!("read-range:{}:{offset}:{len}", path.display()));
        ServerBinaryDbFilesystemStore.read_range(path, offset, len)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        ServerBinaryDbFilesystemStore.metadata_len(path)
    }

    fn create_parent_dirs(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.create_parent_dirs(path)
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        self.events
            .borrow_mut()
            .push(format!("append:{}:{}", path.display(), bytes.len()));
        ServerBinaryDbFilesystemStore.append_bytes(path, bytes)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> StoreResult<()> {
        self.events.borrow_mut().push(format!(
            "overwrite:{}:{offset}:{}",
            path.display(),
            bytes.len()
        ));
        ServerBinaryDbFilesystemStore.overwrite_range(path, offset, bytes)
    }

    fn truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.truncate_file(path, len)
    }

    fn remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.remove_file_if_exists(path)
    }
}

impl ServerBinaryDbDurabilityStore for RecordingServerBinaryDbStore {
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.events
            .borrow_mut()
            .push(format!("file:{}", path.display()));
        ServerBinaryDbFilesystemStore.sync_file(path)
    }

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        self.events
            .borrow_mut()
            .push(format!("data:{}", path.display()));
        ServerBinaryDbFilesystemStore.sync_file_data(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.events
            .borrow_mut()
            .push(format!("dir:{}", path.display()));
        ServerBinaryDbFilesystemStore.sync_directory(path)
    }
}

impl ServerBinaryDbLockStore for RecordingServerBinaryDbStore {
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: ServerBinaryDbLockMode,
        wait: ServerBinaryDbLockWait,
    ) -> StoreResult<Option<BoxedServerBinaryDbProcessLockGuard>> {
        ServerBinaryDbFilesystemStore.acquire_process_lock(path, mode, wait)
    }
}

pub(super) fn has_event(events: &[String], kind: &str, path_fragment: &str) -> bool {
    events
        .iter()
        .any(|event| event.starts_with(kind) && event.contains(path_fragment))
}
