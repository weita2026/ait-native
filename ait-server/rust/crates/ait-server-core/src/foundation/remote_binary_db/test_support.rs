use super::*;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BinaryDbTestStorageOperation {
    ReadBytes,
    ReadToString,
    ReadRange,
    MetadataLen,
    CreateParentDirs,
    AppendBytes,
    OverwriteRange,
    TruncateFile,
    RemoveFile,
    SyncFile,
    SyncDirectory,
    AcquireProcessLock,
    ReleaseProcessLock,
}

impl BinaryDbTestStorageOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ReadBytes => "read_bytes",
            Self::ReadToString => "read_to_string",
            Self::ReadRange => "read_range",
            Self::MetadataLen => "metadata_len",
            Self::CreateParentDirs => "create_parent_dirs",
            Self::AppendBytes => "append_bytes",
            Self::OverwriteRange => "overwrite_range",
            Self::TruncateFile => "truncate_file",
            Self::RemoveFile => "remove_file",
            Self::SyncFile => "sync_file",
            Self::SyncDirectory => "sync_directory",
            Self::AcquireProcessLock => "acquire_process_lock",
            Self::ReleaseProcessLock => "release_process_lock",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BinaryDbTestFaultTiming {
    Before,
    After,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BinaryDbTestFault {
    pub operation: BinaryDbTestStorageOperation,
    pub timing: BinaryDbTestFaultTiming,
    pub occurrence: usize,
    pub path_suffix: Option<String>,
}

impl BinaryDbTestFault {
    pub(crate) fn once(
        operation: BinaryDbTestStorageOperation,
        timing: BinaryDbTestFaultTiming,
        path_suffix: impl Into<String>,
    ) -> Self {
        Self::on_occurrence(operation, timing, 1, Some(path_suffix.into()))
    }

    pub(crate) fn on_occurrence(
        operation: BinaryDbTestStorageOperation,
        timing: BinaryDbTestFaultTiming,
        occurrence: usize,
        path_suffix: Option<String>,
    ) -> Self {
        assert!(occurrence > 0, "fault occurrence must be one-based");
        Self {
            operation,
            timing,
            occurrence,
            path_suffix,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BinaryDbTestStorageEvent {
    pub operation: BinaryDbTestStorageOperation,
    pub timing: BinaryDbTestFaultTiming,
    pub path: PathBuf,
}

#[derive(Debug)]
struct ArmedBinaryDbTestFault {
    fault: BinaryDbTestFault,
    matches_seen: usize,
    fired: bool,
}

#[derive(Debug, Default)]
struct BinaryDbTestFaultState {
    faults: Vec<ArmedBinaryDbTestFault>,
    events: Vec<BinaryDbTestStorageEvent>,
}

#[derive(Clone, Debug)]
pub(crate) struct FaultInjectingServerBinaryDbStore<S> {
    inner: S,
    state: Arc<Mutex<BinaryDbTestFaultState>>,
}

impl<S> FaultInjectingServerBinaryDbStore<S> {
    pub(crate) fn new(inner: S) -> Self {
        Self {
            inner,
            state: Arc::new(Mutex::new(BinaryDbTestFaultState::default())),
        }
    }

    pub(crate) fn arm(&self, fault: BinaryDbTestFault) {
        assert!(fault.occurrence > 0, "fault occurrence must be one-based");
        self.state
            .lock()
            .expect("Binary DB test fault state mutex poisoned")
            .faults
            .push(ArmedBinaryDbTestFault {
                fault,
                matches_seen: 0,
                fired: false,
            });
    }

    pub(crate) fn events(&self) -> Vec<BinaryDbTestStorageEvent> {
        self.state
            .lock()
            .expect("Binary DB test fault state mutex poisoned")
            .events
            .clone()
    }

    pub(crate) fn fired_fault_count(&self) -> usize {
        self.state
            .lock()
            .expect("Binary DB test fault state mutex poisoned")
            .faults
            .iter()
            .filter(|fault| fault.fired)
            .count()
    }

    fn checkpoint(
        &self,
        operation: BinaryDbTestStorageOperation,
        timing: BinaryDbTestFaultTiming,
        path: &Path,
    ) -> StoreResult<()> {
        let mut state = self
            .state
            .lock()
            .expect("Binary DB test fault state mutex poisoned");
        state.events.push(BinaryDbTestStorageEvent {
            operation,
            timing,
            path: path.to_path_buf(),
        });
        for armed in &mut state.faults {
            if armed.fired
                || armed.fault.operation != operation
                || armed.fault.timing != timing
                || armed
                    .fault
                    .path_suffix
                    .as_deref()
                    .is_some_and(|suffix| !path.to_string_lossy().ends_with(suffix))
            {
                continue;
            }
            armed.matches_seen += 1;
            if armed.matches_seen == armed.fault.occurrence {
                armed.fired = true;
                return Err(BinaryDbError::new(
                    BinaryDbErrorKind::Io,
                    format!(
                        "injected Binary DB {} fault for {} at {}",
                        match timing {
                            BinaryDbTestFaultTiming::Before => "before",
                            BinaryDbTestFaultTiming::After => "after",
                        },
                        operation.as_str(),
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn before(&self, operation: BinaryDbTestStorageOperation, path: &Path) -> StoreResult<()> {
        self.checkpoint(operation, BinaryDbTestFaultTiming::Before, path)
    }

    fn after(&self, operation: BinaryDbTestStorageOperation, path: &Path) -> StoreResult<()> {
        self.checkpoint(operation, BinaryDbTestFaultTiming::After, path)
    }
}

impl<S> ServerBinaryDbFileStore for FaultInjectingServerBinaryDbStore<S>
where
    S: ServerBinaryDbFileStore,
{
    fn path_exists(&self, path: &Path) -> bool {
        self.inner.path_exists(path)
    }

    fn read_bytes(&self, path: &Path) -> StoreResult<Vec<u8>> {
        let operation = BinaryDbTestStorageOperation::ReadBytes;
        self.before(operation, path)?;
        let value = self.inner.read_bytes(path)?;
        self.after(operation, path)?;
        Ok(value)
    }

    fn read_to_string(&self, path: &Path) -> StoreResult<String> {
        let operation = BinaryDbTestStorageOperation::ReadToString;
        self.before(operation, path)?;
        let value = self.inner.read_to_string(path)?;
        self.after(operation, path)?;
        Ok(value)
    }
}

impl<S> ServerBinaryDbByteStore for FaultInjectingServerBinaryDbStore<S>
where
    S: ServerBinaryDbByteStore,
{
    fn read_range(&self, path: &Path, offset: u64, len: u32) -> StoreResult<Vec<u8>> {
        let operation = BinaryDbTestStorageOperation::ReadRange;
        self.before(operation, path)?;
        let value = self.inner.read_range(path, offset, len)?;
        self.after(operation, path)?;
        Ok(value)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        let operation = BinaryDbTestStorageOperation::MetadataLen;
        self.before(operation, path)?;
        let value = self.inner.metadata_len(path)?;
        self.after(operation, path)?;
        Ok(value)
    }

    fn create_parent_dirs(&self, path: &Path) -> StoreResult<()> {
        let operation = BinaryDbTestStorageOperation::CreateParentDirs;
        self.before(operation, path)?;
        self.inner.create_parent_dirs(path)?;
        self.after(operation, path)
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        let operation = BinaryDbTestStorageOperation::AppendBytes;
        self.before(operation, path)?;
        let offset = self.inner.append_bytes(path, bytes)?;
        self.after(operation, path)?;
        Ok(offset)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> StoreResult<()> {
        let operation = BinaryDbTestStorageOperation::OverwriteRange;
        self.before(operation, path)?;
        self.inner.overwrite_range(path, offset, bytes)?;
        self.after(operation, path)
    }

    fn truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        let operation = BinaryDbTestStorageOperation::TruncateFile;
        self.before(operation, path)?;
        self.inner.truncate_file(path, len)?;
        self.after(operation, path)
    }

    fn remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        let operation = BinaryDbTestStorageOperation::RemoveFile;
        self.before(operation, path)?;
        self.inner.remove_file_if_exists(path)?;
        self.after(operation, path)
    }
}

impl<S> ServerBinaryDbDurabilityStore for FaultInjectingServerBinaryDbStore<S>
where
    S: ServerBinaryDbDurabilityStore,
{
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        let operation = BinaryDbTestStorageOperation::SyncFile;
        self.before(operation, path)?;
        self.inner.sync_file(path)?;
        self.after(operation, path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        let operation = BinaryDbTestStorageOperation::SyncDirectory;
        self.before(operation, path)?;
        self.inner.sync_directory(path)?;
        self.after(operation, path)
    }
}

#[derive(Debug)]
struct FaultInjectingServerBinaryDbProcessLockGuard {
    inner: BoxedServerBinaryDbProcessLockGuard,
    path: PathBuf,
    injector: FaultInjectingServerBinaryDbStore<()>,
}

impl ServerBinaryDbProcessLockGuard for FaultInjectingServerBinaryDbProcessLockGuard {
    fn replace_contents_and_flush(&mut self, bytes: &[u8]) -> StoreResult<()> {
        self.inner.replace_contents_and_flush(bytes)
    }

    fn clear_contents_and_flush(&mut self) -> StoreResult<()> {
        self.inner.clear_contents_and_flush()
    }

    fn release(&mut self) -> StoreResult<()> {
        let operation = BinaryDbTestStorageOperation::ReleaseProcessLock;
        self.injector.before(operation, &self.path)?;
        self.inner.release()?;
        self.injector.after(operation, &self.path)
    }
}

impl<S> ServerBinaryDbLockStore for FaultInjectingServerBinaryDbStore<S>
where
    S: ServerBinaryDbLockStore,
{
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: ServerBinaryDbLockMode,
        wait: ServerBinaryDbLockWait,
    ) -> StoreResult<Option<BoxedServerBinaryDbProcessLockGuard>> {
        let operation = BinaryDbTestStorageOperation::AcquireProcessLock;
        self.before(operation, path)?;
        let guard = self
            .inner
            .acquire_process_lock(path, mode, wait)?
            .map(|inner| {
                Box::new(FaultInjectingServerBinaryDbProcessLockGuard {
                    inner,
                    path: path.to_path_buf(),
                    injector: FaultInjectingServerBinaryDbStore {
                        inner: (),
                        state: Arc::clone(&self.state),
                    },
                }) as BoxedServerBinaryDbProcessLockGuard
            });
        self.after(operation, path)?;
        Ok(guard)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BinaryDbTestFileSnapshot {
    relative_path: PathBuf,
    bytes: Option<Vec<u8>>,
}

pub(crate) fn capture_binary_db_files(
    root: &Path,
    relative_paths: &[&str],
) -> StoreResult<Vec<BinaryDbTestFileSnapshot>> {
    relative_paths
        .iter()
        .map(|relative_path| {
            let path = root.join(relative_path);
            let bytes = match fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(err) if err.kind() == ErrorKind::NotFound => None,
                Err(err) => {
                    return Err(BinaryDbError::io(
                        format!("capture Binary DB test file {}", path.display()),
                        err,
                    ));
                }
            };
            Ok(BinaryDbTestFileSnapshot {
                relative_path: PathBuf::from(relative_path),
                bytes,
            })
        })
        .collect()
}

#[track_caller]
pub(crate) fn assert_binary_db_files_unchanged(
    root: &Path,
    snapshots: &[BinaryDbTestFileSnapshot],
) {
    for snapshot in snapshots {
        let path = root.join(&snapshot.relative_path);
        let current = match fs::read(&path) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => None,
            Err(err) => panic!("read Binary DB test file {}: {err}", path.display()),
        };
        assert_eq!(
            current,
            snapshot.bytes,
            "Binary DB file changed unexpectedly: {}",
            path.display()
        );
    }
}

#[track_caller]
pub(crate) fn assert_binary_db_path_missing(path: &Path) {
    assert!(
        !path.exists(),
        "Binary DB path should be absent after transaction completion: {}",
        path.display()
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerBinaryDbAuthorityClass {
    CanonicalRepository,
    WorkflowState,
    ImmutableRepositoryPack,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ServerBinaryDbAggregateWriteFamily {
    pub aggregate: &'static str,
    pub family: &'static str,
    pub authority: ServerBinaryDbAuthorityClass,
    pub mutable_during_commit: bool,
    pub files: &'static [&'static str],
}

pub(crate) const SERVER_LAND_AGGREGATE_WRITE_FAMILIES: &[ServerBinaryDbAggregateWriteFamily] = &[
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "land",
        family: "canonical_line",
        authority: ServerBinaryDbAuthorityClass::CanonicalRepository,
        mutable_during_commit: true,
        files: &["line.bin"],
    },
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "land",
        family: "workflow_land",
        authority: ServerBinaryDbAuthorityClass::WorkflowState,
        mutable_during_commit: true,
        files: &["land.bin", "change_land_index.bin", "task_land_index.bin"],
    },
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "land",
        family: "workflow_change",
        authority: ServerBinaryDbAuthorityClass::WorkflowState,
        mutable_during_commit: true,
        files: &["change.bin"],
    },
];

pub(crate) const SERVER_ZSTD_BULK_AGGREGATE_WRITE_FAMILIES:
    &[ServerBinaryDbAggregateWriteFamily] = &[
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "zstd_bulk",
        family: "object_pack_bytes",
        authority: ServerBinaryDbAuthorityClass::ImmutableRepositoryPack,
        mutable_during_commit: false,
        files: &[".ait/objects/packs/{pack_id}.zstpack"],
    },
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "zstd_bulk",
        family: "tree_pack_bytes",
        authority: ServerBinaryDbAuthorityClass::ImmutableRepositoryPack,
        mutable_during_commit: false,
        files: &[".ait/objects/tree-packs/{pack_id}.zstpack"],
    },
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "zstd_bulk",
        family: "object_pack_records",
        authority: ServerBinaryDbAuthorityClass::CanonicalRepository,
        mutable_during_commit: true,
        files: &[
            "object_pack.bin",
            "object_pack_id.idx",
            "object_pack_member.bin",
            "blob.bin",
            "blob_id.idx",
        ],
    },
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "zstd_bulk",
        family: "tree_pack_records",
        authority: ServerBinaryDbAuthorityClass::CanonicalRepository,
        mutable_during_commit: true,
        files: &[
            "tree_pack.bin",
            "tree_pack_id.idx",
            "tree.bin",
            "tree_id.idx",
        ],
    },
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "zstd_bulk",
        family: "canonical_snapshot",
        authority: ServerBinaryDbAuthorityClass::CanonicalRepository,
        mutable_during_commit: true,
        files: &["snapshot.bin", "snapshot_payload.bin", "snapshot_id.idx"],
    },
    ServerBinaryDbAggregateWriteFamily {
        aggregate: "zstd_bulk",
        family: "canonical_line",
        authority: ServerBinaryDbAuthorityClass::CanonicalRepository,
        mutable_during_commit: true,
        files: &["line.bin", "line_name_payload.bin", "line_name.idx"],
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerBinaryDbAggregateVisibility {
    Before,
    InProgress,
    AfterCommit,
    AfterAbortOrRecovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ServerBinaryDbAggregateVisibilityContract {
    pub phase: ServerBinaryDbAggregateVisibility,
    pub externally_visible_new_state: bool,
    pub expected_state: &'static str,
}

pub(crate) const SERVER_BINARY_DB_AGGREGATE_VISIBILITY_CONTRACT:
    &[ServerBinaryDbAggregateVisibilityContract] = &[
    ServerBinaryDbAggregateVisibilityContract {
        phase: ServerBinaryDbAggregateVisibility::Before,
        externally_visible_new_state: false,
        expected_state: "exact pre-transaction bytes",
    },
    ServerBinaryDbAggregateVisibilityContract {
        phase: ServerBinaryDbAggregateVisibility::InProgress,
        externally_visible_new_state: false,
        expected_state: "locks held and rollback journal covers every touched file",
    },
    ServerBinaryDbAggregateVisibilityContract {
        phase: ServerBinaryDbAggregateVisibility::AfterCommit,
        externally_visible_new_state: true,
        expected_state: "all aggregate families committed and journal removed",
    },
    ServerBinaryDbAggregateVisibilityContract {
        phase: ServerBinaryDbAggregateVisibility::AfterAbortOrRecovery,
        externally_visible_new_state: false,
        expected_state: "exact pre-transaction bytes and recovery is idempotent",
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerZstdPackLifecycleState {
    Missing,
    Uploaded,
    Ready,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ServerZstdPackLifecycleTransition {
    pub from: ServerZstdPackLifecycleState,
    pub to: ServerZstdPackLifecycleState,
    pub idempotent: bool,
    pub appends_payload: bool,
}

pub(crate) const SERVER_ZSTD_PACK_LIFECYCLE_TRANSITIONS: &[ServerZstdPackLifecycleTransition] = &[
    ServerZstdPackLifecycleTransition {
        from: ServerZstdPackLifecycleState::Missing,
        to: ServerZstdPackLifecycleState::Uploaded,
        idempotent: false,
        appends_payload: true,
    },
    ServerZstdPackLifecycleTransition {
        from: ServerZstdPackLifecycleState::Uploaded,
        to: ServerZstdPackLifecycleState::Uploaded,
        idempotent: true,
        appends_payload: false,
    },
    ServerZstdPackLifecycleTransition {
        from: ServerZstdPackLifecycleState::Uploaded,
        to: ServerZstdPackLifecycleState::Ready,
        idempotent: false,
        appends_payload: false,
    },
    ServerZstdPackLifecycleTransition {
        from: ServerZstdPackLifecycleState::Ready,
        to: ServerZstdPackLifecycleState::Ready,
        idempotent: true,
        appends_payload: false,
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerLandRetryState {
    Missing,
    Complete,
    IncompleteRecoverable,
    Conflicting,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ServerLandRetryContract {
    pub state: ServerLandRetryState,
    pub action: &'static str,
}

pub(crate) const SERVER_LAND_RETRY_CONTRACT: &[ServerLandRetryContract] = &[
    ServerLandRetryContract {
        state: ServerLandRetryState::Missing,
        action: "validate and commit the entire aggregate",
    },
    ServerLandRetryContract {
        state: ServerLandRetryState::Complete,
        action: "verify the entire aggregate and return the existing result",
    },
    ServerLandRetryContract {
        state: ServerLandRetryState::IncompleteRecoverable,
        action: "recover then complete all missing effects in one transaction",
    },
    ServerLandRetryContract {
        state: ServerLandRetryState::Conflicting,
        action: "return a typed conflict without mutation",
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ServerBinaryDbFollowupTestInventoryRow {
    pub sprint_ref: &'static str,
    pub required_test: &'static str,
}

pub(crate) const SERVER_BINARY_DB_FOLLOWUP_TEST_INVENTORY:
    &[ServerBinaryDbFollowupTestInventoryRow] = &[
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-01",
        required_test: "fixed_record_append_rejects_misaligned_existing_body_without_mutation",
    },
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-02",
        required_test: "recovery_overwrite_uses_injected_file_store",
    },
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-03",
        required_test: "write_capability_is_transaction_owned_and_family_scoped",
    },
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-04",
        required_test: "composite_scope_failure_restores_all_file_families",
    },
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-05",
        required_test: "land_failure_matrix_restores_canonical_aggregate",
    },
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-06",
        required_test: "zstd_ready_transition_reuses_uploaded_payload_range",
    },
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-07",
        required_test: "zstd_bulk_failure_matrix_restores_all_publication_files",
    },
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-08",
        required_test: "server_content_reads_dispatch_from_persisted_layout",
    },
    ServerBinaryDbFollowupTestInventoryRow {
        sprint_ref: "SBDH-09",
        required_test: "binary_db_conformance_vector_version_matches_core",
    },
];
