use super::*;

#[test]
fn write_txn_begin_recovers_journal_after_stale_lock_is_removed() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let record = vec![5_u8; record_file.record_size() as usize];
    let lock_path = server_workflow_write_lock_path(&root);
    let journal_path = root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name());

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    interrupted.append_record(record_file.clone(), &record)?;
    assert!(lock_path.exists());
    assert!(journal_path.exists());
    std::mem::forget(interrupted);
    fs::remove_file(&lock_path).expect("operator removes stale lock after writer death");

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert_eq!(db.record_count(record_file.clone())?, 0);
    assert!(journal_path.exists());
    recovered.abort()?;
    assert!(!journal_path.exists());
    Ok(())
}

#[test]
fn write_txn_begin_recovers_existing_payload_record_and_index_lengths() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let payload_file = task_payload_file_id();
    let index_file = task_change_index_id();
    let first_record = vec![1_u8; record_file.record_size() as usize];
    let second_record = vec![2_u8; record_file.record_size() as usize];

    let mut committed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    committed.append_payload(payload_file.clone(), b"first")?;
    committed.append_record(record_file.clone(), &first_record)?;
    committed.append_index_candidate(index_file.clone(), b"task", 0)?;
    committed.commit()?;

    let record_path = db.resolve_record_path(&record_file)?;
    let payload_path = db.resolve_payload_path(&payload_file)?;
    let index_path = db.resolve_index_path(&index_file)?;
    let original_record_len = fs::metadata(&record_path).expect("record metadata").len();
    let original_payload_len = fs::metadata(&payload_path).expect("payload metadata").len();
    let original_index_len = fs::metadata(&index_path).expect("index metadata").len();

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    interrupted.append_payload(payload_file.clone(), b"second")?;
    interrupted.append_record(record_file.clone(), &second_record)?;
    interrupted.append_index_candidate(index_file.clone(), b"task", 1)?;
    std::mem::forget(interrupted);
    fs::remove_file(server_workflow_write_lock_path(&root))
        .expect("operator removes stale lock after writer death");

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
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
    recovered.abort()?;
    Ok(())
}

#[test]
fn write_txn_stale_recovery_restores_overwrite_before_image() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let original = vec![1_u8; record_file.record_size() as usize];
    let changed = vec![2_u8; record_file.record_size() as usize];

    let mut committed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    committed.append_record(record_file.clone(), &original)?;
    committed.commit()?;

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    interrupted.overwrite_record(record_file.clone(), 0, &changed)?;
    assert_eq!(db.read_record(record_file.clone(), 0)?, changed);
    std::mem::forget(interrupted);
    fs::remove_file(server_workflow_write_lock_path(&root))
        .expect("operator removes stale lock after writer death");

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert_eq!(db.read_record(record_file, 0)?, original);
    recovered.abort()?;
    Ok(())
}

#[test]
fn write_txn_stale_recovery_restores_batched_overwrite_before_images() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let record_file = task_file_id();
    let first = vec![1_u8; record_file.record_size() as usize];
    let second = vec![2_u8; record_file.record_size() as usize];
    let changed_first = vec![3_u8; record_file.record_size() as usize];
    let changed_second = vec![4_u8; record_file.record_size() as usize];

    let mut committed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    committed.append_records(record_file.clone(), &[first.clone(), second.clone()])?;
    committed.commit()?;

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    interrupted.overwrite_records(
        record_file.clone(),
        &[(0, changed_first.clone()), (1, changed_second.clone())],
    )?;
    assert_eq!(
        db.read_records(record_file.clone(), 0, 2)?,
        vec![changed_first, changed_second]
    );
    std::mem::forget(interrupted);
    fs::remove_file(server_workflow_write_lock_path(&root))
        .expect("operator removes stale lock after writer death");

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert_eq!(db.read_records(record_file, 0, 2)?, vec![first, second]);
    recovered.abort()?;
    Ok(())
}

#[test]
fn recovery_overwrite_uses_injected_file_store() -> StoreResult<()> {
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
    let original = vec![1_u8; record_file.record_size() as usize];
    let changed = vec![2_u8; record_file.record_size() as usize];

    let mut committed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    committed.append_record(record_file.clone(), &original)?;
    committed.commit()?;

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    interrupted.overwrite_record(record_file.clone(), 0, &changed)?;
    std::mem::forget(interrupted);
    fs::remove_file(server_workflow_write_lock_path(&root))
        .expect("operator removes stale lock after writer death");
    store.clear_events();

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert_eq!(db.read_record(record_file.clone(), 0)?, original);
    assert!(store
        .events()
        .iter()
        .any(|event| { event.starts_with("overwrite:") && event.contains(record_file.as_str()) }));
    recovered.abort()?;
    Ok(())
}

#[test]
fn recovery_overwrite_failure_preserves_journal_for_retry() -> StoreResult<()> {
    let authority_root = make_temporary_root();
    let root = authority_root.as_path().to_path_buf();
    let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
    let db = FilesystemServerRemoteBinaryDb::with_file_store(
        store.clone(),
        RepoId::new("repo-uuid-001"),
        RepoName::new("repo-name"),
        authority_root,
        StoreGeneration::new(7),
        ServerBinaryDbAuthorityMode::TestFixture,
    );
    let record_file = task_file_id();
    let original = vec![1_u8; record_file.record_size() as usize];
    let changed = vec![2_u8; record_file.record_size() as usize];
    let journal_path = root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name());

    let mut committed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    committed.append_record(record_file.clone(), &original)?;
    committed.commit()?;

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    interrupted.overwrite_record(record_file.clone(), 0, &changed)?;
    std::mem::forget(interrupted);
    fs::remove_file(server_workflow_write_lock_path(&root))
        .expect("operator removes stale lock after writer death");
    store.arm(BinaryDbTestFault::once(
        BinaryDbTestStorageOperation::OverwriteRange,
        BinaryDbTestFaultTiming::Before,
        record_file.as_str(),
    ));

    let error = match BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow) {
        Ok(_) => panic!("injected recovery overwrite failure must fail transaction begin"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), BinaryDbErrorKind::Io);
    assert!(
        journal_path.exists(),
        "failed recovery must retain its journal"
    );

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert_eq!(db.read_record(record_file, 0)?, original);
    recovered.abort()?;
    assert_binary_db_path_missing(&journal_path);
    Ok(())
}

#[test]
fn write_txn_begin_rejects_corrupt_recovery_journal_and_releases_lock() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let journal_path = root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name());
    fs::write(&journal_path, b"not-a-supported-journal\n").expect("failed to seed corrupt journal");

    let err = match BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow) {
        Ok(_) => panic!("corrupt journal must stop recovery"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), BinaryDbErrorKind::Corruption);
    assert!(err.contains("unsupported Binary DB journal header"));
    assert!(server_workflow_write_lock_path(&root).exists());
    assert!(journal_path.exists());
    Ok(())
}

#[test]
fn write_txn_begin_rejects_invalid_journal_path_and_releases_lock() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let journal_path = root.join(BinaryDbCommandScope::ServerWorkflow.journal_file_name());
    fs::write(
        &journal_path,
        b"ait-binary-db-rollback-journal-v1\nfile\t1\t0\t../escape.bin\n",
    )
    .expect("failed to seed invalid path journal");

    let err = match BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow) {
        Ok(_) => panic!("invalid journal path must stop recovery"),
        Err(err) => err,
    };
    assert_eq!(err.kind(), BinaryDbErrorKind::InvalidDomainData);
    assert!(err.contains("parent traversal"));
    assert!(server_workflow_write_lock_path(&root).exists());
    assert!(journal_path.exists());

    fs::remove_file(&journal_path).expect("test cleanup removes invalid journal");
    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    recovered.abort()?;
    Ok(())
}
