use super::*;
use crate::foundation::remote_binary_db::{
    BoxedServerBinaryDbProcessLockGuard, ServerBinaryDbAuthorityMode, ServerBinaryDbByteStore,
    ServerBinaryDbDurabilityStore, ServerBinaryDbFileStore, ServerBinaryDbFilesystemStore,
    ServerBinaryDbLockMode, ServerBinaryDbLockStore, ServerBinaryDbLockWait, StoreResult,
};
use crate::foundation::server_content_binary_db::SERVER_LINE_RECORD_SIZE;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc, Barrier, Mutex};

fn authority_bytes(db: &TestDb) -> BTreeMap<String, Vec<u8>> {
    let root = ServerRemoteBinaryDb::authority_root(db);
    SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
        .map(|path| {
            (
                path.to_owned(),
                fs::read(root.as_path().join(path)).unwrap(),
            )
        })
        .collect()
}

fn correction(task: &str, fork: &str) -> JsonValue {
    json!({"actor_key_index":0,"task_id":task,"title":"Exact reviewed correction",
        "base_line":"main","fork_snapshot_id":fork})
}

#[test]
fn ordinary_change_rejects_target_advance_after_candidate_read_without_writes() {
    let (db, store, line) = fixture("change-target-advance");
    capture(&db, &store, line, FIRST, 0, None);
    let candidate = publish(&store, TASK, FIRST);
    let observed = Barrier::new(2);
    let advanced = Barrier::new(2);
    let (result, before, after) = std::thread::scope(|scope| {
        let caller = scope.spawn(|| {
            let preflight = store
                .get_patchset(Some("repo"), candidate["patchset_id"].as_str().unwrap())
                .unwrap();
            assert_eq!(preflight["base_snapshot_id"], BASE);
            assert_eq!(preflight["revision_snapshot_id"], FIRST);
            observed.wait();
            advanced.wait();
            let before = authority_bytes(&db);
            let result = store.create_change("repo", &correction(TASK, FIRST));
            (result, before, authority_bytes(&db))
        });
        observed.wait();
        ServerBinaryDbLineStore::<_, 1>::new(db.clone())
            .set_line_head("main", 2, 9)
            .unwrap();
        advanced.wait();
        caller.join().unwrap()
    });
    assert!(result.unwrap_err().contains("STALE_TARGET_LINE"));
    assert_eq!(before, after);
    // Rejection consumed neither authority bytes nor the next Change ordinal.
    ServerBinaryDbLineStore::<_, 1>::new(db.clone())
        .set_line_head("main", 1, 10)
        .unwrap();
    let created = store
        .create_change("repo", &correction(TASK, FIRST))
        .unwrap();
    assert_eq!(created["change_ref"], "RCPT-0001/C-01");
    assert_eq!(created["fork_snapshot_id"], FIRST);
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn ordinary_change_rejects_candidate_advance_and_foreign_forks_without_writes() {
    let (db, store, line) = fixture("change-candidate-advance");
    let first = capture(&db, &store, line, FIRST, 0, None);
    let candidate = publish(&store, TASK, FIRST);
    let observed = Barrier::new(2);
    let advanced = Barrier::new(2);
    let (result, before, after) = std::thread::scope(|scope| {
        let caller = scope.spawn(|| {
            store
                .get_patchset(Some("repo"), candidate["patchset_id"].as_str().unwrap())
                .unwrap();
            observed.wait();
            advanced.wait();
            let before = authority_bytes(&db);
            let result = store.create_change("repo", &correction(TASK, FIRST));
            (result, before, authority_bytes(&db))
        });
        observed.wait();
        capture(&db, &store, line, SECOND, first, None);
        publish(&store, TASK, SECOND);
        advanced.wait();
        caller.join().unwrap()
    });
    assert!(result.unwrap_err().contains("exact current Task candidate"));
    assert_eq!(before, after);
    let foreign = store
        .create_task(
            "repo",
            &json!({"actor_key_index":0,"title":"Foreign Task","intent":"Separate owner"}),
        )
        .unwrap();
    for payload in [
        correction(TASK, BASE),
        correction(foreign["task_id"].as_str().unwrap(), SECOND),
    ] {
        let before = authority_bytes(&db);
        assert!(store.create_change("repo", &payload).is_err());
        assert_eq!(before, authority_bytes(&db));
    }
    let created = store
        .create_change("repo", &correction(TASK, SECOND))
        .unwrap();
    assert_eq!(created["change_ref"], "RCPT-0001/C-01");
    assert_eq!(created["fork_snapshot_id"], SECOND);
}

#[derive(Debug)]
struct ReadGate {
    armed: AtomicBool,
    observed: mpsc::Sender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}

// Per-instance filesystem hooks preserve the actual native locks and writes.
// Receives are only bounded deadlock diagnostics, not interleaving selectors.
#[derive(Clone, Debug, Default)]
struct GatedFilesystem {
    read: Option<Arc<ReadGate>>,
}

impl ServerBinaryDbFileStore for GatedFilesystem {
    fn path_exists(&self, path: &Path) -> bool {
        ServerBinaryDbFilesystemStore.path_exists(path)
    }

    fn read_bytes(&self, path: &Path) -> StoreResult<Vec<u8>> {
        ServerBinaryDbFilesystemStore.read_bytes(path)
    }

    fn read_to_string(&self, path: &Path) -> StoreResult<String> {
        ServerBinaryDbFilesystemStore.read_to_string(path)
    }
}

impl ServerBinaryDbByteStore for GatedFilesystem {
    fn read_range(&self, path: &Path, offset: u64, len: u32) -> StoreResult<Vec<u8>> {
        let bytes = ServerBinaryDbFilesystemStore.read_range(path, offset, len)?;
        if path.ends_with("line.bin") && offset == 4 && len == SERVER_LINE_RECORD_SIZE {
            if let Some(gate) = &self.read {
                if gate.armed.swap(false, Ordering::SeqCst) {
                    gate.observed.send(()).unwrap();
                    gate.resume
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(10))
                        .unwrap();
                }
            }
        }
        Ok(bytes)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        ServerBinaryDbFilesystemStore.metadata_len(path)
    }

    fn create_parent_dirs(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.create_parent_dirs(path)
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        ServerBinaryDbFilesystemStore.append_bytes(path, bytes)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.overwrite_range(path, offset, bytes)
    }

    fn truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.truncate_file(path, len)
    }

    fn remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.remove_file_if_exists(path)
    }
}

impl ServerBinaryDbDurabilityStore for GatedFilesystem {
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_file(path)
    }

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_file_data(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_directory(path)
    }
}

impl ServerBinaryDbLockStore for GatedFilesystem {
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: ServerBinaryDbLockMode,
        wait: ServerBinaryDbLockWait,
    ) -> StoreResult<Option<BoxedServerBinaryDbProcessLockGuard>> {
        ServerBinaryDbFilesystemStore.acquire_process_lock(path, mode, wait)
    }
}

fn gated_db(
    db: &TestDb,
    files: GatedFilesystem,
) -> FilesystemServerRemoteBinaryDb<GatedFilesystem> {
    FilesystemServerRemoteBinaryDb::with_file_store(
        files,
        db.repo_id().clone(),
        db.repo_name().clone(),
        ServerRemoteBinaryDb::authority_root(db).clone(),
        db.storage_generation(),
        ServerBinaryDbAuthorityMode::TestFixture,
    )
}

#[test]
fn ordinary_change_excludes_content_writer_after_target_read_until_commit() {
    let (db, store, line) = fixture("change-target-locked-read");
    capture(&db, &store, line, FIRST, 0, None);
    publish(&store, TASK, FIRST);
    let (read_send, read_receive) = mpsc::channel();
    let (change_resume, change_wait) = mpsc::channel();
    let read_gate = Arc::new(ReadGate {
        armed: AtomicBool::new(false),
        observed: read_send,
        resume: Mutex::new(change_wait),
    });
    let guarded = BinaryDbServerWorkflowV0Store::new_remote_frozen(
        gated_db(
            &db,
            GatedFilesystem {
                read: Some(read_gate.clone()),
            },
        ),
        "CP",
    )
    .unwrap()
    .with_repository_index(0);
    let (lock_send, lock_receive) = mpsc::channel();
    let (writer_resume, writer_wait) = mpsc::channel();
    let writer = ServerBinaryDbLineStore::<_, 1>::new(db.clone());
    read_gate.armed.store(true, Ordering::SeqCst);
    let (created, acquired, before, after_attempt) = std::thread::scope(|scope| {
        let change = scope.spawn(|| guarded.create_change("repo", &correction(TASK, FIRST)));
        read_receive.recv_timeout(Duration::from_secs(10)).unwrap();
        let before = authority_bytes(&db);
        let writer_db = &db;
        let advance = scope.spawn(move || {
            // Use a real nonblocking Content transaction so native exclusion,
            // rather than a scheduler timeout, decides the acquisition order.
            match BinaryDbWriteTxn::try_begin(writer_db, BinaryDbCommandScope::ServerContent) {
                Ok(mut tx) => {
                    writer
                        .set_line_head_in_tx(&mut tx, "main", Some(0), 1, 9)
                        .unwrap();
                    tx.commit().unwrap();
                    drop(tx);
                    lock_send.send(true).unwrap();
                }
                Err(error) => {
                    assert!(error.is_retryable_busy(), "{error}");
                    lock_send.send(false).unwrap();
                    writer_wait.recv_timeout(Duration::from_secs(10)).unwrap();
                    writer.set_line_head("main", 2, 9).unwrap();
                }
            }
        });
        let acquired = lock_receive.recv_timeout(Duration::from_secs(10)).unwrap();
        let after_attempt = authority_bytes(&db);
        let created = if acquired {
            // Counterexample order: an insufficient scope lets this writer
            // commit before Change resumes with its already-read head.
            advance.join().unwrap();
            change_resume.send(()).unwrap();
            change.join().unwrap().unwrap()
        } else {
            change_resume.send(()).unwrap();
            let created = change.join().unwrap().unwrap();
            writer_resume.send(()).unwrap();
            advance.join().unwrap();
            created
        };
        (created, acquired, before, after_attempt)
    });
    assert!(
        !acquired,
        "Content writer acquired the lock after target read and before Change append"
    );
    assert_eq!(before, after_attempt);
    assert_eq!(created["change_ref"], "RCPT-0001/C-01");
    assert_eq!(created["fork_snapshot_id"], FIRST);
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        ServerBinaryDbLineStore::<_, 1>::new(db.clone())
            .line_by_name(&read, "main")
            .unwrap()
            .unwrap()
            .1
            .head_snapshot_index_plus1,
        2
    );
}
