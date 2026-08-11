use super::*;

fn forget_writer_and_release_scope_locks<B, F>(
    writer: BinaryDbWriteTxn<'_, B, F>,
    root: &Path,
    scope: BinaryDbCommandScope,
) where
    B: BinaryDb + ?Sized,
    F: BinaryDbFsyncPolicy,
{
    std::mem::forget(writer);
    for lock_name in scope.lock_file_names() {
        fs::remove_file(root.join(".locks").join("binary-db").join(lock_name))
            .expect("test simulates process death by releasing its scope lock inode");
    }
}

#[test]
fn stale_land_journal_is_recovered_before_newer_content_commit() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let content_file = BinaryFileId::new("line.bin", 1, 28, BinaryDbFileFamily::Content);
    let original = [1_u8; 28];
    let interrupted_value = [2_u8; 28];
    let newer_value = [3_u8; 28];

    let mut seed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent)?;
    seed.append_record(content_file.clone(), &original)?;
    seed.commit()?;

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerLand)?;
    interrupted.append_record(content_file.clone(), &interrupted_value)?;
    forget_writer_and_release_scope_locks(interrupted, &root, BinaryDbCommandScope::ServerLand);

    let land_journal = root.join(BinaryDbCommandScope::ServerLand.journal_file_name());
    assert!(land_journal.exists());
    let mut newer = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent)?;
    assert_eq!(db.record_count(content_file.clone())?, 1);
    assert_eq!(db.read_record(content_file.clone(), 0)?, original);
    newer.append_record(content_file.clone(), &newer_value)?;
    newer.commit()?;
    assert_binary_db_path_missing(&land_journal);

    let mut later_land = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerLand)?;
    later_land.abort()?;
    assert_eq!(db.record_count(content_file.clone())?, 2);
    assert_eq!(db.read_record(content_file.clone(), 0)?, original);
    assert_eq!(db.read_record(content_file, 1)?, newer_value);
    Ok(())
}

#[test]
fn active_land_writer_finishes_before_overlapping_content_writer_begins() -> StoreResult<()> {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let content_file = BinaryFileId::new("line.bin", 1, 28, BinaryDbFileFamily::Content);
    let (land_started_tx, land_started_rx) = mpsc::channel();
    let (release_land_tx, release_land_rx) = mpsc::channel();

    let land_db = db.clone();
    let land_file = content_file.clone();
    let land_handle = thread::spawn(move || -> StoreResult<()> {
        let mut land = BinaryDbWriteTxn::begin(&land_db, BinaryDbCommandScope::ServerLand)?;
        land.append_record(land_file, &[7_u8; 28])?;
        land_started_tx
            .send(())
            .expect("test coordinator should observe active land writer");
        release_land_rx
            .recv()
            .expect("test coordinator should release land writer");
        land.commit().map(|_| ())
    });

    land_started_rx
        .recv()
        .expect("land writer should acquire its complete lock set");
    let content_db = db.clone();
    let queued_file = content_file.clone();
    let content_handle = thread::spawn(move || -> StoreResult<Duration> {
        let started = Instant::now();
        let mut content = BinaryDbWriteTxn::begin_queued(
            &content_db,
            BinaryDbCommandScope::ServerContent,
            Duration::from_secs(2),
            Duration::from_millis(5),
        )?;
        let waited = started.elapsed();
        content.append_record(queued_file, &[8_u8; 28])?;
        content.commit()?;
        Ok(waited)
    });

    thread::sleep(Duration::from_millis(100));
    assert!(root
        .join(BinaryDbCommandScope::ServerLand.journal_file_name())
        .exists());
    release_land_tx
        .send(())
        .expect("land writer should still be waiting");
    land_handle.join().expect("land writer panicked")?;
    let waited = content_handle.join().expect("content writer panicked")?;

    assert!(waited >= Duration::from_millis(50));
    assert_eq!(db.record_count(content_file.clone())?, 2);
    assert_eq!(db.read_record(content_file.clone(), 0)?, [7_u8; 28]);
    assert_eq!(db.read_record(content_file, 1)?, [8_u8; 28]);
    assert_binary_db_path_missing(&root.join(BinaryDbCommandScope::ServerLand.journal_file_name()));
    Ok(())
}

#[test]
fn stale_remote_sync_journal_overlaps_only_content_scope() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let pack_file = BinaryFileId::new("object_pack.bin", 1, 28, BinaryDbFileFamily::Content);
    let content_file = BinaryFileId::new("snapshot.bin", 1, 84, BinaryDbFileFamily::Content);
    let journal = root.join(BinaryDbCommandScope::ServerRemoteSyncCommit.journal_file_name());

    let mut interrupted =
        BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerRemoteSyncCommit)?;
    interrupted.append_record(pack_file.clone(), &[3_u8; 28])?;
    interrupted.append_record(content_file.clone(), &[4_u8; 84])?;
    forget_writer_and_release_scope_locks(
        interrupted,
        &root,
        BinaryDbCommandScope::ServerRemoteSyncCommit,
    );

    for disjoint_scope in [
        BinaryDbCommandScope::ServerWorkflow,
        BinaryDbCommandScope::ServerRepositoryPack,
        BinaryDbCommandScope::ServerPlan,
    ] {
        let mut disjoint = BinaryDbWriteTxn::begin(&db, disjoint_scope)?;
        assert!(journal.exists());
        assert_eq!(db.record_count(pack_file.clone())?, 1);
        assert_eq!(db.record_count(content_file.clone())?, 1);
        disjoint.abort()?;
    }

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent)?;
    assert_binary_db_path_missing(&db.resolve_record_path(&pack_file)?);
    assert_binary_db_path_missing(&db.resolve_record_path(&content_file)?);
    recovered.abort()?;
    assert_binary_db_path_missing(&journal);
    Ok(())
}

#[test]
fn stale_general_journal_is_recovered_before_every_domain_scope() -> StoreResult<()> {
    for requested_scope in [
        BinaryDbCommandScope::ServerContent,
        BinaryDbCommandScope::ServerWorkflow,
        BinaryDbCommandScope::ServerPlan,
        BinaryDbCommandScope::ServerQueue,
        BinaryDbCommandScope::ServerRepositoryPack,
    ]
    .into_iter()
    {
        let (db, root, _ctx) = make_db();
        fs::create_dir_all(&root).expect("failed to create authority root");
        let probe_file = BinaryFileId::new("blob.bin", 1, 4, BinaryDbFileFamily::Content);
        let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General)?;
        interrupted.append_record(probe_file.clone(), &[1_u8; 4])?;
        forget_writer_and_release_scope_locks(interrupted, &root, BinaryDbCommandScope::General);

        let mut recovered = BinaryDbWriteTxn::begin(&db, requested_scope)?;
        assert_binary_db_path_missing(&db.resolve_record_path(&probe_file)?);
        recovered.abort()?;
        assert_binary_db_path_missing(
            &root.join(BinaryDbCommandScope::General.journal_file_name()),
        );
    }
    Ok(())
}

#[test]
fn disjoint_plan_writer_does_not_recover_stale_content_journal() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let content_file = BinaryFileId::new("line.bin", 1, 28, BinaryDbFileFamily::Content);
    let content_journal = root.join(BinaryDbCommandScope::ServerContent.journal_file_name());

    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent)?;
    interrupted.append_record(content_file.clone(), &[6_u8; 28])?;
    forget_writer_and_release_scope_locks(interrupted, &root, BinaryDbCommandScope::ServerContent);

    let mut plan = BinaryDbWriteTxn::try_begin(&db, BinaryDbCommandScope::ServerPlan)?;
    assert!(content_journal.exists());
    assert_eq!(db.record_count(content_file.clone())?, 1);
    plan.abort()?;

    let mut content = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent)?;
    assert_eq!(db.record_count(content_file)?, 0);
    content.abort()?;
    assert_binary_db_path_missing(&content_journal);
    Ok(())
}

#[test]
fn overlapping_recovery_failure_preserves_source_journal_and_blocks_new_writer() -> StoreResult<()>
{
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
    let content_file = BinaryFileId::new("line.bin", 1, 28, BinaryDbFileFamily::Content);
    let land_journal = root.join(BinaryDbCommandScope::ServerLand.journal_file_name());
    let content_journal = root.join(BinaryDbCommandScope::ServerContent.journal_file_name());

    let mut seed = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent)?;
    seed.append_record(content_file.clone(), &[1_u8; 28])?;
    seed.commit()?;
    let mut interrupted = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerLand)?;
    interrupted.append_record(content_file.clone(), &[2_u8; 28])?;
    forget_writer_and_release_scope_locks(interrupted, &root, BinaryDbCommandScope::ServerLand);
    store.arm(BinaryDbTestFault::once(
        BinaryDbTestStorageOperation::TruncateFile,
        BinaryDbTestFaultTiming::Before,
        content_file.as_str(),
    ));

    let error = match BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent) {
        Ok(_) => panic!("failed overlapping recovery must not open a new writer"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), BinaryDbErrorKind::Io);
    assert!(land_journal.exists());
    assert_binary_db_path_missing(&content_journal);
    assert_eq!(db.record_count(content_file.clone())?, 2);

    let mut recovered = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerContent)?;
    assert_eq!(db.record_count(content_file)?, 1);
    recovered.abort()?;
    assert_binary_db_path_missing(&land_journal);
    Ok(())
}
