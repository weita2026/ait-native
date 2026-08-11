use super::*;

#[test]
fn payload_append_and_read() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let payload = task_payload_file_id();
    let first = vec![3_u8; 7];
    let second = vec![4_u8; 3];
    let first_range = db.append_payload(payload.clone(), &first, &mut ctx)?;
    let second_range = db.append_payload(payload.clone(), &second, &mut ctx)?;
    let read_first = db.read_payload(
        payload.clone(),
        first_range.payload_offset,
        first_range.payload_len,
    )?;
    let read_second = db.read_payload(
        payload.clone(),
        second_range.payload_offset,
        second_range.payload_len,
    )?;
    assert_eq!(first_range.payload_offset, 4);
    assert_eq!(first_range.payload_len, 7);
    assert_eq!(second_range.payload_offset, 11);
    assert_eq!(second_range.payload_len, 3);
    assert_eq!(read_first, first);
    assert_eq!(read_second, second);
    Ok(())
}

#[test]
fn payload_append_rejects_noncanonical_layout_before_file_creation() -> StoreResult<()> {
    let authority_root = make_temporary_root();
    let root = authority_root.as_path().to_path_buf();
    let db = FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("repo-uuid-001"),
        RepoName::new("repo-name"),
        authority_root,
        StoreGeneration::new(7),
    );
    let mut ctx = BinaryWriteContext::test_fixture(BinaryDbCommandScope::ServerWorkflow);
    fs::create_dir_all(&root).expect("failed to create authority root");
    let payload = BinaryPayloadFileId::new("task_payload.bin", 2, BinaryDbFileFamily::Workflow);
    let error = db
        .append_payload(payload, b"unsupported", &mut ctx)
        .expect_err("server test fixtures must not create non-layout-1 payload files");
    assert_eq!(error.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(!root.join("task_payload.bin").exists());
    Ok(())
}

#[test]
fn index_append_lookup() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let index = task_change_index_id();
    db.append_index_candidate(index.clone(), b"alpha", 10, &mut ctx)?;
    db.append_index_candidate(index.clone(), b"beta", 11, &mut ctx)?;
    db.append_index_candidate(index.clone(), b"alpha", 12, &mut ctx)?;
    assert_eq!(db.lookup_index(index.clone(), b"alpha")?, vec![10, 12]);
    assert_eq!(
        db.lookup_index(index.clone(), b"missing")?,
        Vec::<u32>::new()
    );
    Ok(())
}

#[test]
fn batch_index_lookup_reads_once_and_preserves_key_alignment() -> StoreResult<()> {
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
    fs::create_dir_all(&root).expect("failed to create authority root");
    let index = task_change_index_id();
    let mut ctx = BinaryWriteContext::test_fixture(BinaryDbCommandScope::ServerWorkflow);
    db.append_index_candidate(index.clone(), b"alpha", 10, &mut ctx)?;
    db.append_index_candidate(index.clone(), b"beta", 11, &mut ctx)?;
    db.append_index_candidate(index.clone(), b"alpha", 12, &mut ctx)?;

    store.clear_events();
    let keys: [BinaryIndexKeyRef<'_>; 4] = [b"alpha", b"beta", b"missing", b"alpha"];
    assert_eq!(
        db.lookup_index_many(index.clone(), &keys)?,
        vec![vec![10, 12], vec![11], vec![], vec![10, 12]]
    );
    let index_reads = store
        .events()
        .into_iter()
        .filter(|event| event.starts_with("read:") && event.ends_with(index.as_str()))
        .count();
    assert_eq!(index_reads, 1, "a batch must read the index only once");
    Ok(())
}

#[test]
fn batch_record_read_opens_one_header_and_one_body_range() -> StoreResult<()> {
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
    fs::create_dir_all(&root).expect("failed to create authority root");
    let file = task_file_id();
    let records = [
        vec![1_u8; file.record_size() as usize],
        vec![2_u8; file.record_size() as usize],
        vec![3_u8; file.record_size() as usize],
    ];
    let mut ctx = BinaryWriteContext::test_fixture(BinaryDbCommandScope::ServerWorkflow);
    for record in &records {
        db.append_record(file.clone(), record, &mut ctx)?;
    }

    store.clear_events();
    assert_eq!(db.read_records(file.clone(), 0, 3)?, records);
    let range_reads = store
        .events()
        .into_iter()
        .filter(|event| event.starts_with("read-range:") && event.contains(file.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        range_reads.len(),
        2,
        "a contiguous batch must read one header and one body range"
    );
    assert!(range_reads[0].ends_with(":0:4"));
    assert!(range_reads[1].ends_with(&format!(":4:{}", file.record_size() * 3)));
    Ok(())
}

#[test]
fn batch_record_append_writes_one_contiguous_body() -> StoreResult<()> {
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
    fs::create_dir_all(&root).expect("failed to create authority root");
    let file = task_file_id();
    let records = vec![
        vec![1_u8; file.record_size() as usize],
        vec![2_u8; file.record_size() as usize],
        vec![3_u8; file.record_size() as usize],
    ];
    let mut ctx = BinaryWriteContext::test_fixture(BinaryDbCommandScope::ServerWorkflow);

    assert_eq!(
        db.append_records(file.clone(), &records, &mut ctx)?,
        vec![0, 1, 2]
    );
    assert_eq!(db.read_records(file.clone(), 0, 3)?, records);
    let appends = store
        .events()
        .into_iter()
        .filter(|event| event.starts_with("append:") && event.contains(file.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        appends.len(),
        2,
        "one header and one body append are expected"
    );
    assert!(appends[0].ends_with(":4"));
    assert!(appends[1].ends_with(&format!(":{}", file.record_size() * 3)));
    Ok(())
}

#[test]
fn batch_index_append_writes_one_contiguous_body() -> StoreResult<()> {
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
    fs::create_dir_all(&root).expect("failed to create authority root");
    let index = task_change_index_id();
    let candidates = vec![
        (b"alpha".to_vec(), 10),
        (b"beta".to_vec(), 11),
        (b"alpha".to_vec(), 12),
    ];
    let mut ctx = BinaryWriteContext::test_fixture(BinaryDbCommandScope::ServerWorkflow);

    db.append_index_candidates(index.clone(), &candidates, &mut ctx)?;
    assert_eq!(db.lookup_index(index.clone(), b"alpha")?, vec![10, 12]);
    let appends = store
        .events()
        .into_iter()
        .filter(|event| event.starts_with("append:") && event.contains(index.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        appends.len(),
        2,
        "one header and one body append are expected"
    );
    assert!(appends[0].ends_with(":4"));
    Ok(())
}

#[test]
fn index_append_and_lookup_honors_canonical_layout_id() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let index = BinaryIndexId::new("layout-one-probe.idx", 1, BinaryDbFileFamily::Workflow);
    db.append_index_candidate(index.clone(), b"alpha", 21, &mut ctx)?;
    assert_eq!(db.lookup_index(index.clone(), b"alpha")?, vec![21]);
    let path = db.resolve_index_path(&index)?;
    assert_eq!(
        fs::read(path).expect("index file should read")[0..4],
        1_u32.to_le_bytes()
    );
    Ok(())
}

#[test]
fn index_entry_uses_ait_core_u32_key_length_layout() -> StoreResult<()> {
    let entry = FilesystemServerRemoteBinaryDb::<ServerBinaryDbFilesystemStore>::build_index_entry(
        b"alpha", 12,
    )?;
    let mut expected = Vec::new();
    expected.extend_from_slice(&5_u32.to_le_bytes());
    expected.extend_from_slice(b"alpha");
    expected.extend_from_slice(&12_u32.to_le_bytes());
    assert_eq!(entry, expected);

    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let index = task_change_index_id();
    db.append_index_candidate(index.clone(), b"alpha", 12, &mut ctx)?;
    let bytes = fs::read(db.resolve_index_path(&index)?).expect("index bytes should read");
    let mut expected_file = Vec::new();
    expected_file.extend_from_slice(&index.layout_id().to_le_bytes());
    expected_file.extend_from_slice(&expected);
    assert_eq!(bytes, expected_file);
    Ok(())
}

#[test]
fn fixed_index_entries_match_canonical_twelve_byte_layout() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let key = 0x010203040506_u64.to_le_bytes();
    let index =
        BinaryIndexId::new_fixed("snapshot_id.idx", 1, 8, true, BinaryDbFileFamily::Workflow);
    db.append_index_candidate(index.clone(), &key, 0, &mut ctx)?;

    let mut expected = 1_u32.to_le_bytes().to_vec();
    let mut expected_file = 1_u32.to_le_bytes().to_vec();
    expected_file.extend_from_slice(&key);
    expected_file.append(&mut expected);
    assert_eq!(
        fs::read(db.resolve_index_path(&index)?).unwrap(),
        expected_file
    );
    assert_eq!(db.lookup_index(index.clone(), &key)?, vec![0]);
    let err = db
        .lookup_index(index, b"short")
        .expect_err("fixed index lookup must reject wrong key width");
    assert_eq!(err.kind(), BinaryDbErrorKind::InvalidDomainData);
    Ok(())
}

#[test]
fn payload_append_and_read_reject_mismatched_layout_header() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let payload = task_payload_file_id();
    let path = db.resolve_payload_path(&payload)?;
    fs::write(&path, 2_u32.to_le_bytes()).expect("failed to write corrupt payload header");

    let append_err = db
        .append_payload(payload.clone(), b"body", &mut ctx)
        .expect_err("mismatched payload layout should reject append");
    assert_eq!(append_err.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(append_err.contains("layout mismatch for payload"));

    let read_err = db
        .read_payload(payload.clone(), 4, 0)
        .expect_err("mismatched payload layout should reject read");
    assert_eq!(read_err.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(read_err.contains("layout mismatch for payload"));
    Ok(())
}

#[test]
fn index_append_and_lookup_reject_mismatched_layout_header() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let index = task_change_index_id();
    let path = db.resolve_index_path(&index)?;
    fs::write(&path, 2_u32.to_le_bytes()).expect("failed to write corrupt index header");

    let append_err = db
        .append_index_candidate(index.clone(), b"alpha", 1, &mut ctx)
        .expect_err("mismatched index layout should reject append");
    assert_eq!(append_err.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(append_err.contains("layout mismatch for index"));

    let lookup_err = db
        .lookup_index(index.clone(), b"alpha")
        .expect_err("mismatched index layout should reject lookup");
    assert_eq!(lookup_err.kind(), BinaryDbErrorKind::LayoutMismatch);
    assert!(lookup_err.contains("layout mismatch for index"));
    Ok(())
}

#[test]
fn reject_absolute_or_parent_paths() {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let absolute_payload =
        BinaryPayloadFileId::new("/tmp/absolute.bin", 1, BinaryDbFileFamily::Workflow);
    let record_size = crate::foundation::workflow_binary_v0::TASK_RECORD_SIZE;
    let traversal_file = BinaryFileId::new(
        "../escape.bin",
        1,
        record_size,
        BinaryDbFileFamily::Workflow,
    );
    let result = db.append_payload(absolute_payload, b"x", &mut ctx);
    assert_eq!(
        result
            .expect_err("absolute payload paths should fail")
            .kind(),
        BinaryDbErrorKind::InvalidDomainData
    );
    let traversal_err = db
        .append_record(traversal_file, &vec![0_u8; record_size as usize], &mut ctx)
        .expect_err("parent traversal record paths should fail");
    assert_eq!(traversal_err.kind(), BinaryDbErrorKind::InvalidDomainData);
    let layout_err = db
        .layout_id(BinaryFileId::new(
            "../escape.bin",
            1,
            record_size,
            BinaryDbFileFamily::Workflow,
        ))
        .expect_err("parent traversal layout paths should fail");
    assert_eq!(layout_err.kind(), BinaryDbErrorKind::InvalidDomainData);
}

#[test]
fn filesystem_mutations_repeat_file_family_authorization() -> StoreResult<()> {
    let (db, root, _ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let plan_file = BinaryFileId::new("plan.bin", 1, 4, BinaryDbFileFamily::Plan);
    let content_payload =
        BinaryPayloadFileId::new("snapshot_payload.bin", 1, BinaryDbFileFamily::Content);
    let queue_index = BinaryIndexId::new("queue-defense.idx", 1, BinaryDbFileFamily::Queue);

    let mut plan_context = BinaryWriteContext::test_fixture(BinaryDbCommandScope::ServerPlan);
    db.append_record(plan_file.clone(), &[1_u8; 4], &mut plan_context)?;
    let plan_path = db.resolve_record_path(&plan_file)?;
    let plan_before = fs::read(&plan_path).expect("seeded plan file should read");

    let mut workflow_context =
        BinaryWriteContext::test_fixture(BinaryDbCommandScope::ServerWorkflow);
    for error in [
        db.append_record(plan_file.clone(), &[2_u8; 4], &mut workflow_context)
            .expect_err("filesystem append must reject cross-family context"),
        db.overwrite_record(plan_file.clone(), 0, &[2_u8; 4], &mut workflow_context)
            .expect_err("filesystem overwrite must reject cross-family context"),
        db.append_payload(content_payload.clone(), b"content", &mut workflow_context)
            .expect_err("filesystem payload append must reject cross-family context"),
        db.append_index_candidate(queue_index.clone(), b"queue", 0, &mut workflow_context)
            .expect_err("filesystem index append must reject cross-family context"),
    ] {
        assert_eq!(error.kind(), BinaryDbErrorKind::InvalidDomainData);
        assert!(error.contains("cannot mutate"));
    }

    assert_eq!(
        fs::read(&plan_path).expect("plan file should remain readable"),
        plan_before
    );
    assert!(!db.resolve_payload_path(&content_payload)?.exists());
    assert!(!db.resolve_index_path(&queue_index)?.exists());
    Ok(())
}

#[test]
fn record_file_corruption_detection() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let file = task_file_id();
    let short_record = vec![9_u8; 1];
    let err = db
        .append_record(file.clone(), &short_record, &mut ctx)
        .expect_err("record size mismatch should fail");
    assert!(err.contains("does not match"));
    Ok(())
}

#[test]
fn fixed_record_append_rejects_misaligned_existing_body_without_mutation() -> StoreResult<()> {
    let (db, root, mut ctx) = make_db();
    fs::create_dir_all(&root).expect("failed to create authority root");
    let file = task_file_id();
    let path = db.resolve_record_path(&file)?;
    let mut corrupt_bytes = file.layout_id().to_le_bytes().to_vec();
    corrupt_bytes.extend(vec![3_u8; file.record_size() as usize]);
    corrupt_bytes.push(0xff);
    fs::write(&path, &corrupt_bytes).expect("seed misaligned fixed-record file");

    let valid_record = vec![4_u8; file.record_size() as usize];
    let error = db
        .append_record(file.clone(), &valid_record, &mut ctx)
        .expect_err("append must reject an existing misaligned record body");

    assert_eq!(error.kind(), BinaryDbErrorKind::Corruption);
    assert!(error.contains(&format!(
        "misaligned body length {}",
        file.record_size() + 1
    )));
    assert!(error.contains(&format!("{}-byte records", file.record_size())));
    assert_eq!(
        fs::read(&path).expect("read misaligned fixed-record file"),
        corrupt_bytes,
        "validation failure must not mutate the existing file"
    );
    Ok(())
}
