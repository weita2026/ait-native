use super::*;

#[test]
fn composite_scopes_lock_all_member_families_in_stable_order() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");

    let mut land = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerLand)?;
    let land_lock_names = land
        .write_lock_paths()
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        land_lock_names,
        BinaryDbCommandScope::ServerLand.lock_file_names()
    );
    assert!(land
        .command_scope()
        .authorizes_file_family(BinaryDbFileFamily::Content));
    assert!(land
        .command_scope()
        .authorizes_file_family(BinaryDbFileFamily::Workflow));
    for blocked_scope in [
        BinaryDbCommandScope::ServerContent,
        BinaryDbCommandScope::ServerWorkflow,
        BinaryDbCommandScope::ServerRemoteSyncCommit,
    ] {
        let error = match BinaryDbWriteTxn::try_begin(&db, blocked_scope) {
            Ok(_) => panic!("overlapping scope {blocked_scope:?} must serialize"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), BinaryDbErrorKind::RetryableBusy);
    }
    let mut plan = BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerPlan)?;
    plan.abort()?;
    land.abort()?;

    let mut general = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General)?;
    for blocked_scope in [
        BinaryDbCommandScope::ServerWorkflow,
        BinaryDbCommandScope::ServerPlan,
        BinaryDbCommandScope::ServerQueue,
        BinaryDbCommandScope::ServerRepositoryPack,
        BinaryDbCommandScope::ServerContent,
    ] {
        let error = match BinaryDbWriteTxn::try_begin(&db, blocked_scope) {
            Ok(_) => panic!("General must conflict with {blocked_scope:?}"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), BinaryDbErrorKind::RetryableBusy);
    }
    general.abort()?;
    Ok(())
}

#[test]
fn composite_scope_failure_restores_all_file_families() -> StoreResult<()> {
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
    let workflow_file = task_file_id();
    let content_file = BinaryFileId::new("line.bin", 1, 28, BinaryDbFileFamily::Content);
    let before = capture_binary_db_files(&root, &[workflow_file.as_str(), content_file.as_str()])?;
    store.arm(BinaryDbTestFault::once(
        BinaryDbTestStorageOperation::AppendBytes,
        BinaryDbTestFaultTiming::After,
        content_file.as_str(),
    ));

    {
        let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerLand)?;
        tx.append_record(
            workflow_file,
            &vec![1_u8; task_file_id().record_size() as usize],
        )?;
        let error = tx
            .append_record(content_file.clone(), &[2_u8; 28])
            .expect_err("second-family injected append failure must abort the aggregate");
        assert_eq!(error.kind(), BinaryDbErrorKind::Io);
    }

    assert_binary_db_files_unchanged(&root, &before);
    assert_binary_db_path_missing(&root.join(BinaryDbCommandScope::ServerLand.journal_file_name()));
    Ok(())
}

#[test]
fn remote_sync_stale_recovery_is_content_only() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let pack_file = BinaryFileId::new("object_pack.bin", 1, 28, BinaryDbFileFamily::Content);
    let content_file = BinaryFileId::new("snapshot.bin", 1, 84, BinaryDbFileFamily::Content);
    let workflow_file = task_file_id();
    let journal_path = root.join(BinaryDbCommandScope::ServerRemoteSyncCommit.journal_file_name());

    let mut interrupted =
        BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerRemoteSyncCommit)?;
    interrupted.append_record(pack_file.clone(), &[2_u8; 28])?;
    interrupted.append_record(content_file.clone(), &[3_u8; 84])?;
    let unauthorized = interrupted
        .append_record(
            workflow_file.clone(),
            &vec![4_u8; workflow_file.record_size() as usize],
        )
        .expect_err("remote sync must not authorize workflow writes");
    assert_eq!(unauthorized.kind(), BinaryDbErrorKind::InvalidDomainData);
    std::mem::forget(interrupted);
    for lock_name in BinaryDbCommandScope::ServerRemoteSyncCommit.lock_file_names() {
        fs::remove_file(root.join(".locks").join("binary-db").join(lock_name))
            .expect("operator removes stale content lock after writer death");
    }

    let mut workflow = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)?;
    assert!(journal_path.exists());
    assert_eq!(db.record_count(pack_file.clone())?, 1);
    workflow.abort()?;

    let mut repository_pack =
        BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerRepositoryPack)?;
    assert!(journal_path.exists());
    assert_eq!(db.record_count(content_file.clone())?, 1);
    repository_pack.abort()?;

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerRemoteSyncCommit)?;
    assert_binary_db_path_missing(&db.resolve_record_path(&pack_file)?);
    assert_binary_db_path_missing(&db.resolve_record_path(&content_file)?);
    assert_binary_db_path_missing(&db.resolve_record_path(&workflow_file)?);
    recovered.abort()?;
    assert_binary_db_path_missing(&journal_path);
    Ok(())
}

#[test]
fn persistent_journal_contract_documents_server_strengthening() {
    let invariants = SERVER_BINARY_DB_PERSISTENT_JOURNAL_CONTRACT
        .iter()
        .map(|row| row.invariant)
        .collect::<Vec<_>>();
    assert!(invariants.contains(&"journal header"));
    assert!(invariants.contains(&"relative paths"));
    assert!(invariants.contains(&"original lengths"));
    assert!(invariants.contains(&"overwrite before-images"));
    assert!(invariants.contains(&"commit cleanup"));
    assert!(invariants.contains(&"post-commit lock cleanup"));
    assert!(invariants.contains(&"abort cleanup"));
    assert!(invariants.contains(&"stale recovery"));
    assert!(invariants.contains(&"corrupt journal"));
    assert!(SERVER_BINARY_DB_PERSISTENT_JOURNAL_CONTRACT
        .iter()
        .any(|row| { row.guarantee.contains("ait-binary-db-rollback-journal-v1") }));
}
