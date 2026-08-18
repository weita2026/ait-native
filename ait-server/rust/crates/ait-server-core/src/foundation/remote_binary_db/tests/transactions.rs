use super::*;

#[test]
fn domain_scoped_writers_can_coexist_while_same_scope_conflicts() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");

    let mut plan = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerPlan)?;
    let mut workflow = BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("different scope writer should acquire independently");
    let queue = match BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerPlan) {
        Ok(_) => panic!("same scope writer should be rejected"),
        Err(err) => err,
    };
    assert_eq!(queue.kind(), BinaryDbErrorKind::RetryableBusy);
    assert!(scoped_write_lock_path(&root, BinaryDbCommandScope::ServerPlan).exists());
    assert!(scoped_write_lock_path(&root, BinaryDbCommandScope::ServerWorkflow).exists());

    workflow.abort()?;
    plan.abort()?;
    Ok(())
}

#[test]
fn same_family_writers_in_different_repository_roots_never_contend() -> StoreResult<()> {
    let (first_db, first_root, _first_ctx) = make_db();
    let (second_db, second_root, _second_ctx) = make_db();
    assert_ne!(first_root, second_root);
    fs::create_dir_all(&first_root).expect("failed to create first authority root");
    fs::create_dir_all(&second_root).expect("failed to create second authority root");

    let mut first = BinaryDbWriteTxn::begin(&first_db, BinaryDbCommandScope::ServerWorkflow)?;
    let mut second = BinaryDbWriteTxn::try_begin(&second_db, BinaryDbCommandScope::ServerWorkflow)
        .expect("same family in a different repository authority must be independent");
    assert_ne!(first.write_lock_paths(), second.write_lock_paths());

    second.abort()?;
    first.abort()?;
    Ok(())
}

#[test]
fn content_only_plan_artifact_commit_does_not_block_workflow_or_pack_writers() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");

    let mut artifact_commit =
        BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerRemoteSyncCommit)?;
    let mut workflow = BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("content-only artifact commit must not take Workflow");
    let mut repository_pack =
        BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerRepositoryPack)
            .expect("content metadata commit must not take RepositoryPack");
    let content_busy = match BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerContent) {
        Ok(_) => panic!("content metadata writers must serialize"),
        Err(error) => error,
    };
    assert_eq!(content_busy.kind(), BinaryDbErrorKind::RetryableBusy);

    repository_pack.abort()?;
    workflow.abort()?;
    artifact_commit.abort()?;
    Ok(())
}

#[test]
fn read_txn_blocks_on_any_domain_scoped_writer() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let mut queue = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerQueue)?;

    let read_txn = BinaryDbReadTxn::new(&db);
    let read_busy = read_txn
        .record_count(task_file_id())
        .expect_err("read transaction should fail while queue writer is active");
    assert_eq!(read_busy.kind(), BinaryDbErrorKind::RetryableBusy);
    assert!(scoped_write_lock_path(&root, BinaryDbCommandScope::ServerQueue).exists());

    queue.abort()?;
    Ok(())
}

#[test]
fn scoped_read_txn_allows_disjoint_writer_and_waits_only_for_matching_writer() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let mut plan = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerPlan)?;

    let workflow_read = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::WORKFLOW);
    let paths = workflow_read.read_lock_paths()?;
    assert_eq!(paths.len(), 1);
    assert!(paths[0].ends_with("server-workflow.write.lock"));

    let plan_read = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::PLAN);
    let busy = plan_read
        .read_lock_paths()
        .expect_err("matching Plan reader must be busy");
    assert_eq!(busy.kind(), BinaryDbErrorKind::RetryableBusy);
    drop(workflow_read);

    let reader_db = db.clone();
    let reader = thread::spawn(move || {
        let read = BinaryDbReadTxn::new_queued_for_scope(
            &reader_db,
            BinaryDbReadScope::PLAN,
            Duration::from_secs(1),
            Duration::from_millis(5),
        );
        read.read_lock_paths().map(|paths| paths.len())
    });
    thread::sleep(Duration::from_millis(25));
    plan.abort()?;
    assert_eq!(
        reader.join().expect("reader thread should join")?,
        1,
        "Plan reader should acquire only the Plan lock after writer release"
    );
    Ok(())
}

#[test]
fn scoped_readers_share_locks_and_bounded_wait_stays_short() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");

    let first = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::PLAN);
    let second = BinaryDbReadTxn::new_for_scope(&db, BinaryDbReadScope::PLAN);
    assert_eq!(first.read_lock_paths()?.len(), 1);
    assert_eq!(second.read_lock_paths()?.len(), 1);
    drop(second);
    drop(first);

    assert_eq!(BINARY_DB_READ_MAX_WAIT, Duration::from_millis(100));
    let mut writer = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerPlan)?;
    let started = Instant::now();
    let read = BinaryDbReadTxn::new_bounded_for_scope(&db, BinaryDbReadScope::PLAN);
    let busy = read
        .read_lock_paths()
        .expect_err("bounded matching reader must stop waiting and report busy");
    let elapsed = started.elapsed();
    assert_eq!(busy.kind(), BinaryDbErrorKind::RetryableBusy);
    assert!(elapsed >= Duration::from_millis(75), "elapsed={elapsed:?}");
    assert!(elapsed < Duration::from_secs(1), "elapsed={elapsed:?}");
    writer.abort()?;
    Ok(())
}

#[test]
fn std_fsync_policy_syncs_files_and_directories() -> StoreResult<()> {
    let root = make_temporary_root();
    fs::create_dir_all(root.as_path()).expect("failed to create fsync test root");
    let file_path = root.as_path().join("fsync.bin");
    fs::write(&file_path, b"sync me").expect("failed to write fsync test file");

    let policy = BinaryDbStdFsyncPolicy;
    policy.sync_file(&file_path)?;
    policy.sync_file_data(&file_path)?;
    policy.sync_directory(root.as_path())?;
    assert_eq!(fs::read(&file_path).unwrap(), b"sync me");
    Ok(())
}

#[cfg(windows)]
#[test]
fn std_fsync_policy_rejects_a_file_as_a_directory_on_windows() {
    let root = make_temporary_root();
    fs::create_dir_all(root.as_path()).expect("failed to create fsync test root");
    let file_path = root.as_path().join("not-a-directory.bin");
    fs::write(&file_path, b"file").expect("failed to write fsync test file");

    let error = BinaryDbStdFsyncPolicy
        .sync_directory(&file_path)
        .expect_err("Windows directory durability must reject a file target");

    assert!(
        error
            .to_string()
            .contains("directory sync target is not a directory"),
        "{error}"
    );
}

#[test]
fn journal_append_syncs_data_without_repeating_directory_sync() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let policy = RecordingFsyncPolicy::default();
    let scope = BinaryDbCommandScope::ServerWorkflow;
    let journal_path = root.join(scope.journal_file_name());
    let mut tx = BinaryDbWriteTxn::begin_with_fsync_policy(&db, scope, policy.clone())?;

    assert_eq!(
        policy.events(),
        vec![
            format!("file:{}", journal_path.display()),
            format!("dir:{}", root.display()),
        ],
        "journal creation owns the file and directory durability boundary"
    );

    let record_file = task_file_id();
    tx.append_record(
        record_file.clone(),
        &vec![5_u8; record_file.record_size() as usize],
    )?;
    let before_commit = policy.events();
    assert_eq!(
        before_commit
            .iter()
            .filter(|event| event.as_str() == format!("data:{}", journal_path.display()))
            .count(),
        1,
        "one recovery entry requires one journal data sync"
    );
    assert_eq!(
        before_commit
            .iter()
            .filter(|event| event.as_str() == format!("dir:{}", root.display()))
            .count(),
        1,
        "an append must not resync the already-durable journal directory entry"
    );

    tx.commit()?;
    assert!(!journal_path.exists());
    Ok(())
}

#[test]
fn overwrite_records_batches_journal_sync_and_preserves_abort_recovery() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let first = vec![1_u8; record_file.record_size() as usize];
    let second = vec![2_u8; record_file.record_size() as usize];
    let changed_first = vec![3_u8; record_file.record_size() as usize];
    let changed_second = vec![4_u8; record_file.record_size() as usize];

    let mut seed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    seed.append_records(record_file.clone(), &[first.clone(), second.clone()])?;
    seed.commit()?;

    let policy = RecordingFsyncPolicy::default();
    let journal_path = root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name());
    let mut committed = BinaryDbWriteTxn::begin_with_fsync_policy(
        &db,
        BinaryDbCommandScope::ServerWorkflow,
        policy.clone(),
    )?;
    committed.overwrite_records(
        record_file.clone(),
        &[(0, changed_first.clone()), (1, changed_second.clone())],
    )?;
    assert_eq!(
        policy
            .events()
            .iter()
            .filter(|event| event.as_str() == format!("data:{}", journal_path.display()))
            .count(),
        2,
        "one File entry and one batched before-image entry should each sync the journal once"
    );
    committed.commit()?;
    assert_eq!(db.read_record(record_file.clone(), 0)?, changed_first);
    assert_eq!(db.read_record(record_file.clone(), 1)?, changed_second);

    let mut aborted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    aborted.overwrite_records(record_file.clone(), &[(0, first), (1, second)])?;
    aborted.abort()?;
    assert_eq!(db.read_record(record_file.clone(), 0)?, changed_first);
    assert_eq!(db.read_record(record_file, 1)?, changed_second);
    Ok(())
}

#[test]
fn default_write_txn_uses_injected_store_fsync_policy() -> StoreResult<()> {
    let authority_root = make_temporary_root();
    let root = authority_root.as_path().to_path_buf();
    let store = RecordingServerBinaryDbStore::default();
    let db = FilesystemServerRemoteBinaryDb::with_file_store(
        store.clone(),
        RepoId::new("repo-uuid-001"),
        RepoName::new("repo-name"),
        authority_root,
        StoreGeneration::new(7),
        ServerBinaryDbAuthorityMode::TestFixture,
    );
    let record_file = task_file_id();
    let record = vec![8_u8; record_file.record_size() as usize];

    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    tx.append_record(record_file.clone(), &record)?;
    tx.commit()?;

    let events = store.events();
    assert!(has_event(
        &events,
        "file",
        BinaryDbCommandScope::ServerWorkflow.journal_file_name()
    ));
    assert!(has_event(&events, "file", record_file.as_str()));
    assert!(has_event(&events, "dir", root.to_string_lossy().as_ref()));
    Ok(())
}

#[test]
fn durable_commit_reports_lock_cleanup_warning_without_rollback() -> StoreResult<()> {
    let authority_root = make_temporary_root();
    let root = authority_root.as_path().to_path_buf();
    let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
    let db = FilesystemServerRemoteBinaryDb::with_file_store(
        store.clone(),
        RepoId::new("cleanup-warning-repo-id"),
        RepoName::new("cleanup-warning-repo"),
        authority_root,
        StoreGeneration::new(1),
        ServerBinaryDbAuthorityMode::TestFixture,
    );
    let record_file = task_file_id();
    let record = vec![4_u8; record_file.record_size() as usize];

    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    tx.append_record(record_file.clone(), &record)?;
    store.arm(BinaryDbTestFault::once(
        BinaryDbTestStorageOperation::ReleaseProcessLock,
        BinaryDbTestFaultTiming::Before,
        "server-workflow.write.lock",
    ));
    let outcome = tx.commit()?;
    assert!(tx.is_finished());
    assert!(!outcome.committed_cleanly());
    let warning = outcome
        .lock_cleanup_warning()
        .expect("lock cleanup warning");
    assert_eq!(warning.kind(), BinaryDbErrorKind::Io);
    assert!(warning.contains("release_process_lock"));
    assert_eq!(tx.commit()?, outcome);
    assert_eq!(store.fired_fault_count(), 1);
    assert_binary_db_path_missing(
        &root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name()),
    );
    drop(tx);

    assert_eq!(db.record_count(record_file.clone())?, 1);
    assert_eq!(db.read_record(record_file, 0)?, record);
    Ok(())
}

#[test]
fn record_append_read_and_count() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let file = task_file_id();
    assert_eq!(db.record_count(file.clone())?, 0);
    let record_a = vec![1_u8; file.record_size() as usize];
    let record_b = vec![2_u8; file.record_size() as usize];
    let index_a = db.append_record(file.clone(), &record_a, &mut ctx)?;
    let index_b = db.append_record(file.clone(), &record_b, &mut ctx)?;
    assert_eq!(index_a, 0);
    assert_eq!(index_b, 1);
    assert_eq!(db.record_count(file.clone())?, 2);
    assert_eq!(db.read_record(file.clone(), 0)?, record_a);
    assert_eq!(db.read_record(file.clone(), 1)?, record_b);
    assert_eq!(db.layout_id(file.clone())?, 1);
    Ok(())
}

#[test]
fn read_and_write_transactions_define_delegate_boundary() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let file = task_file_id();
    let record = vec![7_u8; file.record_size() as usize];

    let mut write_txn = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert_eq!(
        write_txn.command_scope(),
        BinaryDbCommandScope::ServerWorkflow
    );
    assert_eq!(write_txn.append_record(file.clone(), &record)?, 0);
    assert!(!write_txn.is_finished());
    write_txn.commit()?;
    assert!(write_txn.is_finished());

    let read_txn = BinaryDbReadTxn::new(&db);
    assert_eq!(read_txn.record_count(file.clone())?, 1);
    assert_eq!(read_txn.read_record(file.clone(), 0)?, record);
    Ok(())
}

#[test]
fn write_capability_is_transaction_owned_and_family_scoped() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let workflow_file = task_file_id();
    let workflow_record = vec![7_u8; workflow_file.record_size() as usize];
    let plan_file = BinaryFileId::new("plan.bin", 1, 36, BinaryDbFileFamily::Plan);
    let plan_record = vec![3_u8; plan_file.record_size() as usize];
    let content_payload =
        BinaryPayloadFileId::new("snapshot_payload.bin", 1, BinaryDbFileFamily::Content);
    let queue_index = BinaryIndexId::new("queue.idx", 1, BinaryDbFileFamily::Queue);

    let mut plan = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerPlan)?;
    plan.append_record(plan_file.clone(), &plan_record)?;
    plan.commit()?;
    let plan_path = db.resolve_record_path(&plan_file)?;
    let plan_before = fs::read(&plan_path).expect("seeded plan file should read");

    let mut workflow = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert_eq!(
        workflow.write_context().command_scope(),
        BinaryDbCommandScope::ServerWorkflow
    );
    assert!(workflow.write_context().is_active());
    workflow
        .write_context()
        .ensure_authorized_family(BinaryDbFileFamily::Workflow)?;
    let family_error = workflow
        .write_context()
        .ensure_authorized_family(BinaryDbFileFamily::Content)
        .expect_err("workflow capability must not authorize content writes");
    assert_eq!(family_error.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(family_error.contains("cannot mutate Content files"));

    let append_error = workflow
        .append_record(plan_file.clone(), &plan_record)
        .expect_err("workflow transaction must reject Plan record append");
    assert_eq!(append_error.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(append_error.contains("cannot mutate Plan files"));
    let overwrite_error = workflow
        .overwrite_record(plan_file.clone(), 0, &vec![4_u8; plan_record.len()])
        .expect_err("workflow transaction must reject Plan record overwrite");
    assert_eq!(overwrite_error.kind(), BinaryDbErrorKind::InvalidDomainData);
    let payload_error = workflow
        .append_payload(content_payload.clone(), b"content")
        .expect_err("workflow transaction must reject Content payload append");
    assert_eq!(payload_error.kind(), BinaryDbErrorKind::InvalidDomainData);
    let index_error = workflow
        .append_index_candidate(queue_index.clone(), b"queue", 0)
        .expect_err("workflow transaction must reject Queue index append");
    assert_eq!(index_error.kind(), BinaryDbErrorKind::InvalidDomainData);

    assert!(workflow.touched_files().is_empty());
    assert!(workflow.touched_directories().is_empty());
    let journal =
        fs::read_to_string(root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name()))
            .expect("active workflow journal should read");
    for unauthorized_path in [
        plan_file.as_str(),
        content_payload.as_str(),
        queue_index.as_str(),
    ] {
        assert!(
            !journal.contains(unauthorized_path),
            "unauthorized path {unauthorized_path} must not enter the journal"
        );
    }
    assert_eq!(
        fs::read(&plan_path).expect("plan file should remain readable"),
        plan_before
    );
    assert!(!db.resolve_payload_path(&content_payload)?.exists());
    assert!(!db.resolve_index_path(&queue_index)?.exists());

    workflow.append_record(workflow_file.clone(), &workflow_record)?;
    workflow.commit()?;
    assert!(!workflow.write_context().is_active());
    let finished_error = workflow
        .append_record(workflow_file, &workflow_record)
        .expect_err("finished transaction must reject further writes");
    assert_eq!(finished_error.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(finished_error.contains("already finished"));

    let mut general = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General)?;
    for (path, family) in [
        ("change.bin", BinaryDbFileFamily::Workflow),
        ("plan_revision.bin", BinaryDbFileFamily::Plan),
        ("general-queue.idx", BinaryDbFileFamily::Queue),
        (
            "general-repository-pack.idx",
            BinaryDbFileFamily::RepositoryPack,
        ),
        ("blob.bin", BinaryDbFileFamily::Content),
    ] {
        general.append_record(BinaryFileId::new(path, 1, 1, family), &[1])?;
    }
    general.commit()?;
    assert!(!general.write_context().is_active());
    Ok(())
}

#[test]
fn write_txn_enforces_single_writer_and_blocks_readers() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let file = task_file_id();

    let mut first = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    let busy = match BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerWorkflow) {
        Ok(_) => panic!("second writer should be rejected while first writer is active"),
        Err(err) => err,
    };
    assert_eq!(busy.kind(), BinaryDbErrorKind::RetryableBusy);
    assert!(busy.is_retryable_busy());
    assert!(server_workflow_write_lock_path(&root).exists());

    let read_txn = BinaryDbReadTxn::new(&db);
    let read_busy = read_txn
        .record_count(file.clone())
        .expect_err("read transaction should fail while writer is active");
    assert_eq!(read_busy.kind(), BinaryDbErrorKind::RetryableBusy);

    first.abort()?;
    assert!(server_workflow_write_lock_path(&root).exists());
    let second = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    drop(second);
    assert!(server_workflow_write_lock_path(&root).exists());
    let read_txn = BinaryDbReadTxn::new(&db);
    assert_eq!(read_txn.record_count(file.clone())?, 0);
    Ok(())
}

#[test]
fn read_txn_blocks_writer_until_read_lock_is_released() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let workflow_lock_path = server_workflow_write_lock_path(&root);
    fs::create_dir_all(workflow_lock_path.parent().expect("lock parent"))
        .expect("failed to create lock root");
    fs::write(
        &workflow_lock_path,
        b"scope=ServerWorkflow\npid=diagnostic\n",
    )
    .expect("failed to seed lock diagnostic metadata");
    let read_txn = BinaryDbReadTxn::new(&db);
    let read_lock_paths = read_txn.read_lock_paths()?;
    assert_eq!(
        read_lock_paths.len(),
        BinaryDbCommandScope::all_write_lock_file_names().len()
    );
    assert!(read_lock_paths.iter().all(|path| path.exists()));

    let busy = match BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerWorkflow) {
        Ok(_) => panic!("writer should be rejected while reader is active"),
        Err(err) => err,
    };
    assert_eq!(busy.kind(), BinaryDbErrorKind::RetryableBusy);
    drop(read_txn);
    assert_eq!(
        fs::read(&workflow_lock_path).expect("lock metadata should remain readable"),
        b"scope=ServerWorkflow\npid=diagnostic\n"
    );

    let mut writer = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert!(workflow_lock_path.exists());
    writer.abort()?;
    assert!(workflow_lock_path.exists());
    Ok(())
}

#[test]
fn write_txn_begin_queued_waits_for_same_scope_writer() -> StoreResult<()> {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let first_record = vec![1_u8; record_file.record_size() as usize];
    let second_record = vec![2_u8; record_file.record_size() as usize];
    let (first_locked_tx, first_locked_rx) = mpsc::channel();
    let (release_first_tx, release_first_rx) = mpsc::channel();
    let (queued_started_tx, queued_started_rx) = mpsc::channel();

    let first_db = db.clone();
    let first_file = record_file.clone();
    let first_handle = thread::spawn(move || -> StoreResult<()> {
        let mut tx = BinaryDbWriteTxn::begin(&first_db, BinaryDbCommandScope::ServerWorkflow)?;
        tx.append_record(first_file, &first_record)?;
        first_locked_tx
            .send(())
            .expect("test coordinator should receive first writer lock");
        release_first_rx
            .recv()
            .expect("test coordinator should release first writer");
        tx.commit().map(|_| ())
    });

    first_locked_rx
        .recv()
        .expect("first writer should acquire lock");
    assert!(server_workflow_write_lock_path(&root).exists());

    let queued_db = db.clone();
    let queued_file = record_file.clone();
    let queued_handle = thread::spawn(
        move || -> StoreResult<(Duration, Duration, Duration, u32)> {
            queued_started_tx
                .send(())
                .expect("test coordinator should observe queued writer start");
            let started = Instant::now();
            let mut tx = BinaryDbWriteTxn::begin_queued(
                &queued_db,
                BinaryDbCommandScope::ServerWorkflow,
                Duration::from_secs(2),
                Duration::from_millis(5),
            )?;
            let waited = started.elapsed();
            let reported_wait = tx.admission_wait_duration();
            let record_index = tx.append_record(queued_file, &second_record)?;
            thread::sleep(Duration::from_millis(10));
            let outcome = tx.commit()?;
            Ok((
                waited,
                reported_wait,
                outcome.lock_hold_duration(),
                record_index,
            ))
        },
    );

    queued_started_rx
        .recv()
        .expect("queued writer should start before first writer is released");
    thread::sleep(Duration::from_millis(100));
    release_first_tx
        .send(())
        .expect("first writer should still be waiting");

    first_handle.join().expect("first writer panicked")?;
    let (waited, reported_wait, reported_hold, queued_record_index) =
        queued_handle.join().expect("queued writer panicked")?;
    assert!(waited >= Duration::from_millis(50));
    assert!(reported_wait >= Duration::from_millis(50));
    assert!(reported_hold >= Duration::from_millis(10));
    assert_eq!(queued_record_index, 1);
    assert_eq!(db.record_count(record_file.clone())?, 2);
    assert_eq!(db.read_record(record_file.clone(), 0)?[0], 1);
    assert_eq!(db.read_record(record_file.clone(), 1)?[0], 2);
    assert!(server_workflow_write_lock_path(&root).exists());
    Ok(())
}

#[test]
fn write_txn_begin_queued_times_out_while_same_scope_writer_remains_active() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let mut active = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;

    let err = match BinaryDbWriteTxn::begin_queued(
        &db,
        BinaryDbCommandScope::ServerWorkflow,
        Duration::from_millis(10),
        Duration::from_millis(1),
    ) {
        Ok(_) => panic!("queued writer should time out while active writer remains"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), BinaryDbErrorKind::RetryableBusy);
    assert!(err.contains("timed out waiting"));
    assert!(err.contains("waited_ms="));
    assert!(err.contains("max_wait_ms=10"));
    assert!(err.contains("holder_scope=ServerWorkflow"));
    assert!(err.contains("holder_pid="));
    assert!(err.contains("holder_acquired_unix_ms="));
    assert!(err.contains("holder_held_ms="));

    active.abort()?;
    Ok(())
}

#[test]
fn serving_writer_admission_deadlines_are_scope_bounded() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    assert_eq!(BINARY_DB_SERVING_WRITE_MAX_WAIT, Duration::from_secs(5));
    assert_eq!(
        BINARY_DB_SERVING_WORKFLOW_WRITE_MAX_WAIT,
        Duration::from_secs(20)
    );
    assert_eq!(
        BINARY_DB_SERVING_WRITE_RETRY_INTERVAL,
        Duration::from_millis(5)
    );
    for scope in [
        BinaryDbCommandScope::ServerPlan,
        BinaryDbCommandScope::ServerContent,
        BinaryDbCommandScope::ServerRemoteSyncCommit,
        BinaryDbCommandScope::ServerRepositoryPack,
        BinaryDbCommandScope::ServerQueue,
    ] {
        assert_eq!(
            binary_db_serving_write_max_wait(scope),
            Duration::from_secs(5),
            "{scope:?}"
        );
    }
    for scope in [
        BinaryDbCommandScope::ServerWorkflow,
        BinaryDbCommandScope::ServerLand,
    ] {
        assert_eq!(
            binary_db_serving_write_max_wait(scope),
            Duration::from_secs(20),
            "{scope:?}"
        );
    }
    let mut active = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerPlan)?;

    let started = Instant::now();
    let err = match BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerPlan) {
        Ok(_) => panic!("serving writer must stop waiting at its deadline"),
        Err(err) => err,
    };
    let elapsed = started.elapsed();
    assert_eq!(err.kind(), BinaryDbErrorKind::RetryableBusy);
    assert!(
        elapsed >= Duration::from_millis(4_500),
        "elapsed={elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(8), "elapsed={elapsed:?}");
    assert!(err.contains("max_wait_ms=5000"));
    assert!(err.contains("holder_scope=ServerPlan"));

    active.abort()?;
    Ok(())
}

#[test]
fn serving_writer_rejects_offline_general_scope() {
    let (db, _root, _ctx) = make_db();
    let error = match BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::General) {
        Ok(_) => panic!("serving writer must not acquire the offline all-family barrier"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(error.contains("offline whole-authority"));
}

#[test]
fn write_txn_commit_fsyncs_record_payload_and_index_files() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let policy = RecordingFsyncPolicy::default();
    let mut write_txn = BinaryDbWriteTxn::begin_with_fsync_policy(
        &db,
        BinaryDbCommandScope::ServerWorkflow,
        policy.clone(),
    )?;

    let record_file = task_file_id();
    let payload_file = task_payload_file_id();
    let index_file = task_change_index_id();
    let record = vec![9_u8; record_file.record_size() as usize];

    write_txn.append_payload(payload_file.clone(), b"payload")?;
    write_txn.append_record(record_file.clone(), &record)?;
    write_txn.append_index_candidate(index_file.clone(), b"task", 0)?;
    assert_eq!(write_txn.touched_files().len(), 3);
    assert_eq!(write_txn.touched_directories().len(), 1);

    write_txn.commit()?;
    let events = policy.events();
    assert!(has_event(&events, "file", record_file.as_str()));
    assert!(has_event(&events, "file", payload_file.as_str()));
    assert!(has_event(&events, "file", index_file.as_str()));
    assert!(has_event(&events, "dir", root.to_string_lossy().as_ref()));
    assert!(!root
        .join(BinaryDbCommandScope::ServerWorkflow.journal_file_name())
        .exists());
    Ok(())
}

#[test]
fn write_txn_rollback_truncates_overwritten_appended_record_without_before_image() -> StoreResult<()>
{
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let first = vec![4_u8; record_file.record_size() as usize];
    let replacement = vec![8_u8; record_file.record_size() as usize];
    let journal_path = root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name());

    let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    tx.append_record(record_file.clone(), &first)?;
    tx.overwrite_record(record_file.clone(), 0, &replacement)?;

    let journal = fs::read_to_string(&journal_path).expect("journal should be readable");
    assert!(
        !journal.lines().any(|line| line.starts_with("before\t")),
        "an appended record should be protected by the original file length"
    );
    tx.abort()?;
    assert_eq!(db.record_count(record_file)?, 0);
    Ok(())
}

#[test]
fn write_txn_commit_failure_keeps_journal_until_drop_rolls_back() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let record = vec![4_u8; record_file.record_size() as usize];
    // Directory event 1 publishes the new journal. Event 2 is the touched
    // data directory at commit, before journal removal. Journal appends no
    // longer manufacture an unrelated directory event.
    let policy = FailOnceDirectoryFsyncPolicy::new(2);

    {
        let mut tx = BinaryDbWriteTxn::begin_with_fsync_policy(
            &db,
            BinaryDbCommandScope::ServerWorkflow,
            policy,
        )?;
        tx.append_record(record_file.clone(), &record)?;
        let err = tx
            .commit()
            .expect_err("injected directory fsync failure should fail commit");
        assert_eq!(err.kind(), BinaryDbErrorKind::Io);
        assert!(root
            .join(BinaryDbCommandScope::ServerWorkflow.journal_file_name())
            .exists());
    }

    assert_eq!(db.record_count(record_file.clone())?, 0);
    assert!(server_workflow_write_lock_path(&root).exists());
    assert!(!root
        .join(BinaryDbCommandScope::ServerWorkflow.journal_file_name())
        .exists());
    Ok(())
}

#[test]
fn write_txn_abort_removes_new_record_payload_and_index_files() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let payload_file = task_payload_file_id();
    let index_file = task_change_index_id();
    let record = vec![9_u8; record_file.record_size() as usize];
    let record_path = db.resolve_record_path(&record_file)?;
    let payload_path = db.resolve_payload_path(&payload_file)?;
    let index_path = db.resolve_index_path(&index_file)?;

    let mut write_txn = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    write_txn.append_payload(payload_file.clone(), b"payload")?;
    write_txn.append_record(record_file.clone(), &record)?;
    write_txn.append_index_candidate(index_file.clone(), b"task", 0)?;
    assert!(record_path.exists());
    assert!(payload_path.exists());
    assert!(index_path.exists());

    write_txn.abort()?;

    assert!(!record_path.exists());
    assert!(!payload_path.exists());
    assert!(!index_path.exists());
    assert_eq!(db.record_count(record_file.clone())?, 0);
    assert_eq!(
        db.lookup_index(index_file.clone(), b"task")?,
        Vec::<u32>::new()
    );
    assert!(!root
        .join(BinaryDbCommandScope::ServerWorkflow.journal_file_name())
        .exists());
    Ok(())
}

#[test]
fn write_txn_abort_truncates_existing_files_to_original_lengths() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let payload_file = task_payload_file_id();
    let index_file = task_change_index_id();
    let first_record = vec![1_u8; record_file.record_size() as usize];
    let second_record = vec![2_u8; record_file.record_size() as usize];

    let mut committed_txn = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    committed_txn.append_payload(payload_file.clone(), b"first")?;
    committed_txn.append_record(record_file.clone(), &first_record)?;
    committed_txn.append_index_candidate(index_file.clone(), b"task", 0)?;
    committed_txn.commit()?;

    let record_path = db.resolve_record_path(&record_file)?;
    let payload_path = db.resolve_payload_path(&payload_file)?;
    let index_path = db.resolve_index_path(&index_file)?;
    let original_record_len = fs::metadata(&record_path).expect("record metadata").len();
    let original_payload_len = fs::metadata(&payload_path).expect("payload metadata").len();
    let original_index_len = fs::metadata(&index_path).expect("index metadata").len();

    let mut aborted_txn = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    aborted_txn.append_payload(payload_file.clone(), b"second")?;
    aborted_txn.append_record(record_file.clone(), &second_record)?;
    aborted_txn.append_index_candidate(index_file.clone(), b"task", 1)?;
    aborted_txn.abort()?;

    assert_eq!(
        fs::metadata(&record_path).expect("record metadata").len(),
        original_record_len
    );
    assert_eq!(
        fs::metadata(&payload_path).expect("payload metadata").len(),
        original_payload_len
    );
    assert_eq!(
        fs::metadata(&index_path).expect("index metadata").len(),
        original_index_len
    );
    assert_eq!(db.record_count(record_file.clone())?, 1);
    assert_eq!(db.read_record(record_file.clone(), 0)?, first_record);
    assert_eq!(db.lookup_index(index_file.clone(), b"task")?, vec![0]);
    Ok(())
}

#[test]
fn write_txn_drop_aborts_uncommitted_writes() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let record = vec![7_u8; record_file.record_size() as usize];

    {
        let mut write_txn = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
        write_txn.append_record(record_file.clone(), &record)?;
        assert_eq!(db.record_count(record_file.clone())?, 1);
    }

    assert_eq!(db.record_count(record_file.clone())?, 0);
    assert!(server_workflow_write_lock_path(&root).exists());
    assert!(!root
        .join(BinaryDbCommandScope::ServerWorkflow.journal_file_name())
        .exists());
    Ok(())
}
