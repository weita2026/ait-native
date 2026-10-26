use super::*;
use crate::foundation::operational_binary_v0::WORKER_JOB_KIND_CONTENT_GC;
use crate::foundation::remote_binary_db::{
    ServerBinaryDbByteStore, ServerBinaryDbDurabilityStore, ServerBinaryDbFileStore,
    ServerBinaryDbLockStore,
};
use std::sync::Mutex;

const CHILD_ROOT: &str = "AIT_SERVER_JOB_ROLLBACK_TEST_ROOT";
const CHILD_TEST: &str = "foundation::server_operational_worker_jobs::replacement_fault_tests::job_replacement_crash_child";
const CRASHED_WITH_JOURNAL: i32 = 86;
const CRASHED_WITHOUT_JOURNAL: i32 = 87;

struct Domain;

impl WorkerJobDomainAuthority for Domain {
    fn validate_patchset_index(&self, _: u32) -> StoreResult<()> {
        Ok(())
    }

    fn validate_snapshot_index(&self, _: u32) -> StoreResult<()> {
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum FaultMode {
    ParentBarrier,
    ReprotectBarrier,
    CrashDuringRollback,
}

#[derive(Default)]
struct FaultState {
    unlinked: bool,
    parent_failed: bool,
    reprotect_failed: bool,
    rollback_writes: usize,
}

struct FaultStore {
    root: PathBuf,
    mode: FaultMode,
    state: Mutex<FaultState>,
    inner: ServerBinaryDbFilesystemStore,
}

impl FaultStore {
    fn new(root: &Path, mode: FaultMode) -> Self {
        Self {
            root: root.to_path_buf(),
            mode,
            state: Mutex::new(FaultState::default()),
            inner: ServerBinaryDbFilesystemStore,
        }
    }

    fn journal(&self) -> PathBuf {
        self.root.join(WORKER_JOB_UPDATE_JOURNAL_FILE_NAME)
    }
}

impl ServerBinaryDbFileStore for FaultStore {
    fn path_exists(&self, path: &Path) -> bool {
        self.inner.path_exists(path)
    }

    fn read_bytes(&self, path: &Path) -> StoreResult<Vec<u8>> {
        self.inner.read_bytes(path)
    }

    fn read_to_string(&self, path: &Path) -> StoreResult<String> {
        self.inner.read_to_string(path)
    }
}

impl ServerBinaryDbByteStore for FaultStore {
    fn read_range(&self, path: &Path, offset: u64, len: u32) -> StoreResult<Vec<u8>> {
        self.inner.read_range(path, offset, len)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        self.inner.metadata_len(path)
    }

    fn create_parent_dirs(&self, path: &Path) -> StoreResult<()> {
        self.inner.create_parent_dirs(path)
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        self.inner.append_bytes(path, bytes)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> StoreResult<()> {
        let rollback = {
            let mut state = self.state.lock().unwrap();
            if path == self.root.join(WORKER_JOB_FILE_NAME) && state.parent_failed {
                state.rollback_writes += 1;
                true
            } else {
                false
            }
        };
        if rollback {
            // The production writer must have durably restored the ordinary
            // journal before reaching even the first rollback byte.
            let protected = fs::read(self.journal())
                .ok()
                .filter(|raw| raw.len() == 148)
                .is_some_and(|raw| decode_worker_job_update_journal(&raw).is_ok());
            if matches!(self.mode, FaultMode::CrashDuringRollback) {
                if !protected {
                    std::process::exit(CRASHED_WITHOUT_JOURNAL);
                }
                self.inner
                    .overwrite_range(path, offset, &bytes[..bytes.len() / 2])?;
                self.inner.sync_file(path)?;
                std::process::exit(CRASHED_WITH_JOURNAL);
            }
            assert!(protected, "rollback reached an unprotected Job row");
        }
        self.inner.overwrite_range(path, offset, bytes)
    }

    fn truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        self.inner.truncate_file(path, len)
    }

    fn remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        self.inner.remove_file_if_exists(path)?;
        if path == self.journal() {
            self.state.lock().unwrap().unlinked = true;
        }
        Ok(())
    }
}

impl ServerBinaryDbDurabilityStore for FaultStore {
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.inner.sync_file(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        if path == self.root {
            let mut state = self.state.lock().unwrap();
            if state.unlinked && !state.parent_failed {
                assert!(
                    !self.journal().exists(),
                    "fault must follow successful unlink"
                );
                state.parent_failed = true;
                return Err(BinaryDbError::io(
                    "injected final parent barrier after journal unlink",
                    std::io::Error::other("controlled parent barrier failure"),
                ));
            }
            if state.parent_failed
                && matches!(self.mode, FaultMode::ReprotectBarrier)
                && !state.reprotect_failed
            {
                assert!(self.journal().exists());
                state.reprotect_failed = true;
                return Err(BinaryDbError::io(
                    "injected rollback journal publication barrier",
                    std::io::Error::other("controlled reprotection failure"),
                ));
            }
        }
        self.inner.sync_directory(path)
    }
}

impl ServerBinaryDbLockStore for FaultStore {
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: ServerBinaryDbLockMode,
        wait: ServerBinaryDbLockWait,
    ) -> StoreResult<Option<BoxedServerBinaryDbProcessLockGuard>> {
        self.inner.acquire_process_lock(path, mode, wait)
    }
}

fn open(root: &Path) -> ServerOperationalWorkerJobStore {
    ServerOperationalWorkerJobStore::new(1, root, Arc::new(Domain)).unwrap()
}

fn fixture() -> (
    tempfile::TempDir,
    ServerOperationalWorkerJobStore,
    WorkerJobEntry,
) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("1");
    fs::create_dir(&root).unwrap();
    let store = open(&root);
    store.initialize().unwrap();
    let queued = store
        .enqueue(
            WorkerJobCreateSpec {
                job_kind: WORKER_JOB_KIND_CONTENT_GC,
                max_attempts: 1,
                patchset_index_plus1: 0,
                snapshot_index_plus1: 0,
                available_at_s: 100,
                created_at_s: 100,
            },
            WorkerJobEnqueueDisposition::Queued,
        )
        .unwrap();
    (directory, store, queued)
}

fn running(queued: ServerWorkerJobRecord) -> ServerWorkerJobRecord {
    ServerWorkerJobRecord {
        state_kind: WORKER_JOB_STATE_RUNNING,
        attempt_count: 1,
        locked_at_s: 101,
        updated_at_s: 101,
        ..queued
    }
}

#[test]
fn journal_unlink_parent_failure_reprotects_before_rollback() {
    let (_directory, mut store, queued) = fixture();
    let before = fs::read(store.worker_job_path()).unwrap();
    let faults = Arc::new(FaultStore::new(
        &store.authority_root,
        FaultMode::ParentBarrier,
    ));
    store.files = faults.clone();
    let error = store.replace_job(0, |row| Ok(running(row))).unwrap_err();
    assert!(error.to_string().contains("final parent barrier"));
    assert_eq!(faults.state.lock().unwrap().rollback_writes, 1);
    assert_eq!(fs::read(store.worker_job_path()).unwrap(), before);
    assert_eq!(before.len(), 4 + 52);
    assert_eq!(store.get(0).unwrap(), queued);
    assert_eq!(store.validate().unwrap(), vec![queued]);
    assert!(!store.worker_job_update_journal_path().exists());
}

#[test]
fn journal_unlink_reprotection_failure_never_writes_rollback_bytes() {
    let (_directory, mut store, queued) = fixture();
    let faults = Arc::new(FaultStore::new(
        &store.authority_root,
        FaultMode::ReprotectBarrier,
    ));
    store.files = faults.clone();
    let error = store.replace_job(0, |row| Ok(running(row))).unwrap_err();
    assert!(error.to_string().contains("final parent barrier"));
    assert!(error
        .to_string()
        .contains("rollback journal publication barrier"));
    assert_eq!(faults.state.lock().unwrap().rollback_writes, 0);
    let raw = fs::read(store.worker_job_path()).unwrap();
    assert_eq!(
        &raw[4..],
        ServerOperationalBinaryV0Codec::encode_worker_job(running(queued.record)).unwrap()
    );
    assert!(
        store.get(0).is_err(),
        "normal reads must hold while recovery is pending"
    );
    let reopened = open(&store.authority_root);
    assert_eq!(reopened.recover().unwrap(), vec![queued]);
    assert_eq!(reopened.validate().unwrap(), vec![queued]);
}

#[test]
fn journal_unlink_rollback_process_interruption_recovers_the_exact_old_row() {
    let (_directory, store, queued) = fixture();
    let before_jobs = fs::read(store.worker_job_path()).unwrap();
    let before_ready = fs::read(store.worker_ready_index_path()).unwrap();
    let before_state = fs::read(store.worker_state_index_path()).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", CHILD_TEST])
        .env(CHILD_ROOT, &store.authority_root)
        .output()
        .unwrap();
    assert_eq!(
        result.status.code(),
        Some(CRASHED_WITH_JOURNAL),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let interrupted = fs::read(store.worker_job_path()).unwrap();
    assert_ne!(
        interrupted, before_jobs,
        "the rollback must actually stop mid-row"
    );
    assert_ne!(
        &interrupted[4..],
        ServerOperationalBinaryV0Codec::encode_worker_job(running(queued.record)).unwrap()
    );
    assert_eq!(
        fs::metadata(store.worker_job_update_journal_path())
            .unwrap()
            .len(),
        148
    );
    assert!(store.get(0).is_err());
    let reopened = open(&store.authority_root);
    assert_eq!(reopened.recover().unwrap(), vec![queued]);
    assert_eq!(fs::read(reopened.worker_job_path()).unwrap(), before_jobs);
    assert_eq!(
        fs::read(reopened.worker_ready_index_path()).unwrap(),
        before_ready
    );
    assert_eq!(
        fs::read(reopened.worker_state_index_path()).unwrap(),
        before_state
    );
    assert_eq!(reopened.validate().unwrap(), vec![queued]);
}

#[test]
#[ignore = "subprocess fault helper; invoked by the parent interruption test"]
fn job_replacement_crash_child() {
    let root = std::env::var_os(CHILD_ROOT).expect("isolated subprocess root");
    let mut store = open(Path::new(&root));
    store.files = Arc::new(FaultStore::new(
        &store.authority_root,
        FaultMode::CrashDuringRollback,
    ));
    let _ = store.replace_job(0, |row| Ok(running(row)));
    panic!("controlled rollback interruption was not reached");
}
