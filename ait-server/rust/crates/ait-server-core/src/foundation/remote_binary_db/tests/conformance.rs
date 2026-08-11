use super::*;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize)]
struct Fixture {
    version: String,
    layout_id: u32,
    record_size: u32,
    record_vectors: Vec<Vector>,
    transaction_vectors: Vec<Vector>,
    extended_vectors: Vec<ExtendedVector>,
}

#[derive(Debug, Deserialize)]
struct Vector {
    id: String,
    operation: String,
    initial_hex: Option<String>,
    #[serde(default)]
    argument_hex: Option<String>,
    #[serde(default)]
    record_index: Option<u32>,
    #[serde(default)]
    payload_offset: Option<u64>,
    #[serde(default)]
    payload_len: Option<u32>,
    expected_outcome: String,
    expected_value: Option<u64>,
    expected_after_hex: Option<String>,
    mutation: bool,
}

#[derive(Debug, Deserialize)]
struct ExtendedVector {
    id: String,
    operation: String,
    expected_outcome: String,
    #[serde(default)]
    initial_files: BTreeMap<String, Option<String>>,
    #[serde(default)]
    expected_files: BTreeMap<String, Option<String>>,
    #[serde(default)]
    record_path: Option<String>,
    #[serde(default)]
    payload_path: Option<String>,
    #[serde(default)]
    index_path: Option<String>,
    #[serde(default)]
    argument_hex: Option<String>,
    #[serde(default)]
    record_hex: Option<String>,
    #[serde(default)]
    payload_hex: Option<String>,
    #[serde(default)]
    index_key_hex: Option<String>,
    #[serde(default)]
    record_index: Option<u32>,
    #[serde(default)]
    candidate: Option<u32>,
    #[serde(default)]
    keys_hex: Vec<String>,
    #[serde(default)]
    candidates: Vec<u32>,
    #[serde(default)]
    lookup_key_hex: Option<String>,
    #[serde(default)]
    expected_values: Vec<u32>,
    #[serde(default)]
    fixed_key_size: Option<u32>,
    #[serde(default)]
    stores_record_index_plus_one: bool,
    #[serde(default)]
    expected_cleanup_warning: bool,
}

#[derive(Debug, Deserialize)]
struct FixturePin {
    version: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct ParityManifest {
    version: String,
    substrate_fixture: FixturePin,
    plan_fixture: FixturePin,
    required_vector_categories: Vec<String>,
    required_extended_operations: Vec<String>,
    required_plan_cases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PlanCaseIndex {
    version: String,
    layout_id: u32,
    cases: Vec<PlanCaseId>,
}

#[derive(Debug, Deserialize)]
struct PlanCaseId {
    id: String,
}

fn fixture() -> Fixture {
    serde_json::from_slice(SERVER_BINARY_DB_CONFORMANCE_VECTOR_SOURCE)
        .expect("server conformance vector source must parse")
}

fn parity_manifest() -> ParityManifest {
    serde_json::from_slice(SERVER_BINARY_DB_CROSS_REPO_PARITY_MANIFEST_SOURCE)
        .expect("server cross-repo parity manifest must parse")
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex fixture has complete bytes");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16)
                .expect("conformance fixture hex byte")
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn outcome(error: BinaryDbError) -> &'static str {
    match error.kind() {
        BinaryDbErrorKind::RetryableBusy => "retryable_busy",
        BinaryDbErrorKind::Corruption => "corruption",
        BinaryDbErrorKind::LayoutMismatch => "layout_mismatch",
        BinaryDbErrorKind::MissingData => "missing_data",
        BinaryDbErrorKind::InvalidDomainData => "invalid_domain_data",
        BinaryDbErrorKind::Io => "io",
        BinaryDbErrorKind::Unsupported => "unsupported",
        BinaryDbErrorKind::Other => "other",
    }
}

fn schema_fixture_path(path: &str) -> &str {
    match path {
        "vector.bin" => "task.bin",
        "overwrite.bin" | "record.bin" => "blob.bin",
        "payload.bin" => "snapshot_payload.bin",
        "index.bin" => "schema-conformance.idx",
        _ => path,
    }
}

fn initialize(root: &Path, initial_hex: Option<&str>) -> PathBuf {
    fs::create_dir_all(root).expect("create conformance vector root");
    let path = root.join(schema_fixture_path("vector.bin"));
    if let Some(initial_hex) = initial_hex {
        fs::write(&path, decode_hex(initial_hex)).expect("write initial vector bytes");
    }
    path
}

fn after_hex(path: &Path) -> Option<String> {
    fs::read(path).ok().map(|bytes| encode_hex(&bytes))
}

fn initialize_files(root: &Path, files: &BTreeMap<String, Option<String>>) {
    for (relative, initial_hex) in files {
        let path = root.join(schema_fixture_path(relative));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create extended vector parent");
        }
        match initial_hex {
            Some(initial_hex) => fs::write(&path, decode_hex(initial_hex))
                .expect("write extended vector initial bytes"),
            None => {
                let _ = fs::remove_file(path);
            }
        }
    }
}

fn assert_expected_files(root: &Path, vector: &ExtendedVector) {
    for (relative, expected_hex) in &vector.expected_files {
        assert_eq!(
            after_hex(&root.join(schema_fixture_path(relative))),
            *expected_hex,
            "{} exact bytes for {relative}",
            vector.id
        );
    }
}

fn vector_db(authority_root: StorePath) -> FilesystemServerRemoteBinaryDb {
    FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("conformance-repo-id"),
        RepoName::new("conformance-repo"),
        authority_root,
        StoreGeneration::new(1),
    )
}

fn assert_exact_after(vector: &Vector, path: &Path) {
    let actual_after = after_hex(path);
    assert_eq!(
        actual_after, vector.expected_after_hex,
        "{} exact resulting bytes",
        vector.id
    );
    assert_eq!(
        actual_after != vector.initial_hex,
        vector.mutation,
        "{} mutation flag",
        vector.id
    );
}

#[test]
fn server_binary_db_conformance_vectors_v2() {
    let fixture = fixture();
    assert_eq!(fixture.version, SERVER_BINARY_DB_CONFORMANCE_VECTOR_VERSION);
    assert_eq!(
        server_binary_db_conformance_vector_checksum(),
        SERVER_BINARY_DB_CONFORMANCE_VECTOR_CHECKSUM
    );

    for vector in &fixture.record_vectors {
        let authority_root = make_temporary_root();
        let path = initialize(authority_root.as_path(), vector.initial_hex.as_deref());
        let db = vector_db(authority_root.clone());
        let record = BinaryFileId::new(
            schema_fixture_path("vector.bin"),
            fixture.layout_id,
            fixture.record_size,
            BinaryDbFileFamily::Workflow,
        );
        let payload = BinaryPayloadFileId::new(
            schema_fixture_path("vector.bin"),
            fixture.layout_id,
            BinaryDbFileFamily::Workflow,
        );

        let (actual_outcome, actual_value) = match vector.operation.as_str() {
            "record_count" => match db.record_count(record) {
                Ok(value) => ("success", Some(value as u64)),
                Err(error) => (outcome(error), None),
            },
            "read_record" => {
                match db.read_record(record, vector.record_index.expect("read_record index")) {
                    Ok(value) => ("success", Some(value.len() as u64)),
                    Err(error) => (outcome(error), None),
                }
            }
            "read_payload" => match db.read_payload(
                payload,
                vector.payload_offset.expect("payload offset"),
                vector.payload_len.expect("payload length"),
            ) {
                Ok(value) => ("success", Some(value.len() as u64)),
                Err(error) => (outcome(error), None),
            },
            "append_record" => {
                let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                    &db,
                    BinaryDbCommandScope::General,
                    BinaryDbNoopFsyncPolicy,
                )
                .expect("begin vector transaction");
                let argument = decode_hex(vector.argument_hex.as_deref().unwrap());
                match write.append_record(record, &argument) {
                    Ok(value) => {
                        write.commit().expect("commit append vector");
                        ("success", Some(value as u64))
                    }
                    Err(error) => (outcome(error), None),
                }
            }
            operation => panic!("unsupported record vector operation {operation}"),
        };
        assert_eq!(actual_outcome, vector.expected_outcome, "{}", vector.id);
        assert_eq!(actual_value, vector.expected_value, "{}", vector.id);
        assert_exact_after(vector, &path);
        fs::remove_dir_all(authority_root.as_path()).expect("remove vector root");
    }
}

#[test]
fn server_binary_db_transaction_conformance_v2() {
    let fixture = fixture();
    for vector in &fixture.transaction_vectors {
        let authority_root = make_temporary_root();
        let path = initialize(authority_root.as_path(), vector.initial_hex.as_deref());
        let record = BinaryFileId::new(
            schema_fixture_path("vector.bin"),
            fixture.layout_id,
            fixture.record_size,
            BinaryDbFileFamily::Workflow,
        );
        let argument = decode_hex(vector.argument_hex.as_deref().unwrap());

        let (actual_outcome, actual_value) = if vector.operation == "append_short_write" {
            let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
            let db = FilesystemServerRemoteBinaryDb::with_file_store(
                store.clone(),
                RepoId::new("conformance-repo-id"),
                RepoName::new("conformance-repo"),
                authority_root.clone(),
                StoreGeneration::new(1),
                ServerBinaryDbAuthorityMode::TestFixture,
            );
            store.arm(BinaryDbTestFault::once(
                BinaryDbTestStorageOperation::AppendBytes,
                BinaryDbTestFaultTiming::After,
                schema_fixture_path("vector.bin"),
            ));
            let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                &db,
                BinaryDbCommandScope::General,
                BinaryDbNoopFsyncPolicy,
            )
            .expect("begin short-write vector transaction");
            let result = write.append_record(record, &argument);
            drop(write);
            match result {
                Ok(value) => ("success", Some(value as u64)),
                Err(error) => (outcome(error), None),
            }
        } else if vector.operation == "commit_sync_failure" {
            let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
            let db = FilesystemServerRemoteBinaryDb::with_file_store(
                store.clone(),
                RepoId::new("conformance-repo-id"),
                RepoName::new("conformance-repo"),
                authority_root.clone(),
                StoreGeneration::new(1),
                ServerBinaryDbAuthorityMode::TestFixture,
            );
            let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General)
                .expect("begin sync-failure vector transaction");
            let value = write
                .append_record(record, &argument)
                .expect("append before sync failure");
            store.arm(BinaryDbTestFault::once(
                BinaryDbTestStorageOperation::SyncFile,
                BinaryDbTestFaultTiming::After,
                schema_fixture_path("vector.bin"),
            ));
            let result = write.commit();
            drop(write);
            match result {
                Ok(_) => ("success", Some(value as u64)),
                Err(error) => (outcome(error), None),
            }
        } else if vector.operation == "commit_lock_cleanup_failure" {
            let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
            let db = FilesystemServerRemoteBinaryDb::with_file_store(
                store.clone(),
                RepoId::new("conformance-repo-id"),
                RepoName::new("conformance-repo"),
                authority_root.clone(),
                StoreGeneration::new(1),
                ServerBinaryDbAuthorityMode::TestFixture,
            );
            let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General)
                .expect("begin cleanup-failure vector transaction");
            let value = write
                .append_record(record, &argument)
                .expect("append before cleanup failure");
            store.arm(BinaryDbTestFault::once(
                BinaryDbTestStorageOperation::ReleaseProcessLock,
                BinaryDbTestFaultTiming::Before,
                "global.write.lock",
            ));
            let commit = write
                .commit()
                .expect("lock cleanup failure is a committed outcome");
            assert!(commit.lock_cleanup_warning().is_some());
            assert!(!authority_root
                .as_path()
                .join(BinaryDbCommandScope::General.journal_file_name())
                .exists());
            drop(write);
            ("success", Some(value as u64))
        } else {
            let db = vector_db(authority_root.clone());
            match vector.operation.as_str() {
                "append_invalid_record" => {
                    let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                        &db,
                        BinaryDbCommandScope::General,
                        BinaryDbNoopFsyncPolicy,
                    )
                    .expect("begin validation vector transaction");
                    let result = write.append_record(record, &argument);
                    drop(write);
                    match result {
                        Ok(value) => ("success", Some(value as u64)),
                        Err(error) => (outcome(error), None),
                    }
                }
                "abort" | "abort_twice" => {
                    let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                        &db,
                        BinaryDbCommandScope::General,
                        BinaryDbNoopFsyncPolicy,
                    )
                    .expect("begin abort vector transaction");
                    let value = write
                        .append_record(record, &argument)
                        .expect("append before abort");
                    write.abort().expect("abort transaction");
                    if vector.operation == "abort_twice" {
                        write.abort().expect("idempotent second recovery/abort");
                    }
                    ("success", Some(value as u64))
                }
                "commit" => {
                    let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                        &db,
                        BinaryDbCommandScope::General,
                        BinaryDbNoopFsyncPolicy,
                    )
                    .expect("begin commit vector transaction");
                    let value = write
                        .append_record(record, &argument)
                        .expect("append before commit");
                    write.commit().expect("commit transaction");
                    ("success", Some(value as u64))
                }
                operation => panic!("unsupported transaction vector operation {operation}"),
            }
        };
        assert_eq!(actual_outcome, vector.expected_outcome, "{}", vector.id);
        assert_eq!(actual_value, vector.expected_value, "{}", vector.id);
        assert_exact_after(vector, &path);
        fs::remove_dir_all(authority_root.as_path()).expect("remove transaction vector root");
    }
}

fn required<'a>(value: &'a Option<String>, field: &str, id: &str) -> &'a str {
    value
        .as_deref()
        .unwrap_or_else(|| panic!("{id} is missing {field}"))
}

fn extended_record(
    vector: &ExtendedVector,
    layout_id: u32,
    family: BinaryDbFileFamily,
) -> BinaryFileId {
    BinaryFileId::new(
        schema_fixture_path(required(&vector.record_path, "record_path", &vector.id)),
        layout_id,
        2,
        family,
    )
}

fn append_multi_file<B, F>(
    write: &mut BinaryDbWriteTxn<'_, B, F>,
    vector: &ExtendedVector,
    layout_id: u32,
) where
    B: BinaryDb + BinaryDbIndexAppender + ?Sized,
    F: BinaryDbFsyncPolicy,
{
    let record = extended_record(vector, layout_id, BinaryDbFileFamily::Content);
    let payload = BinaryPayloadFileId::new(
        schema_fixture_path(required(&vector.payload_path, "payload_path", &vector.id)),
        layout_id,
        BinaryDbFileFamily::Content,
    );
    let index = BinaryIndexId::new(
        schema_fixture_path(required(&vector.index_path, "index_path", &vector.id)),
        layout_id,
        BinaryDbFileFamily::Content,
    );
    write
        .append_record(
            record,
            &decode_hex(required(&vector.record_hex, "record_hex", &vector.id)),
        )
        .expect("append extended record");
    write
        .append_payload(
            payload,
            &decode_hex(required(&vector.payload_hex, "payload_hex", &vector.id)),
        )
        .expect("append extended payload");
    write
        .append_index_candidate(
            index,
            &decode_hex(required(&vector.index_key_hex, "index_key_hex", &vector.id)),
            vector.candidate.expect("extended candidate"),
        )
        .expect("append extended index candidate");
}

#[test]
fn server_binary_db_extended_conformance_v2() {
    let fixture = fixture();
    for vector in &fixture.extended_vectors {
        let sandbox = make_temporary_root();
        fs::create_dir_all(sandbox.as_path()).expect("create extended vector sandbox");
        let root = sandbox.as_path().join("authority");
        fs::create_dir_all(&root).expect("create extended vector authority root");
        initialize_files(&root, &vector.initial_files);
        let authority_root = StorePath::new(root.clone());
        let mut actual_values = Vec::new();
        let mut cleanup_warning = false;

        let actual_outcome = match vector.operation.as_str() {
            "variable_index_round_trip" | "fixed_index_plus_one_round_trip" => {
                assert_eq!(
                    vector.keys_hex.len(),
                    vector.candidates.len(),
                    "{}",
                    vector.id
                );
                let db = vector_db(authority_root.clone());
                let path =
                    schema_fixture_path(required(&vector.index_path, "index_path", &vector.id));
                let index = match vector.fixed_key_size {
                    Some(key_size) => BinaryIndexId::new_fixed(
                        path,
                        fixture.layout_id,
                        key_size,
                        vector.stores_record_index_plus_one,
                        BinaryDbFileFamily::Content,
                    ),
                    None => {
                        BinaryIndexId::new(path, fixture.layout_id, BinaryDbFileFamily::Content)
                    }
                };
                let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                    &db,
                    BinaryDbCommandScope::General,
                    BinaryDbNoopFsyncPolicy,
                )
                .expect("begin index vector transaction");
                for (key, candidate) in vector.keys_hex.iter().zip(&vector.candidates) {
                    write
                        .append_index_candidate(index.clone(), &decode_hex(key), *candidate)
                        .expect("append index vector candidate");
                }
                write.commit().expect("commit index vector");
                actual_values = db
                    .lookup_index(
                        index,
                        &decode_hex(required(
                            &vector.lookup_key_hex,
                            "lookup_key_hex",
                            &vector.id,
                        )),
                    )
                    .expect("lookup index vector");
                "success"
            }
            "overwrite_commit" | "overwrite_abort" => {
                let db = vector_db(authority_root.clone());
                let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                    &db,
                    BinaryDbCommandScope::General,
                    BinaryDbNoopFsyncPolicy,
                )
                .expect("begin overwrite vector transaction");
                write
                    .overwrite_record(
                        extended_record(vector, fixture.layout_id, BinaryDbFileFamily::Content),
                        vector.record_index.expect("overwrite record_index"),
                        &decode_hex(required(&vector.argument_hex, "argument_hex", &vector.id)),
                    )
                    .expect("overwrite vector record");
                if vector.operation == "overwrite_commit" {
                    write.commit().expect("commit overwrite vector");
                } else {
                    write.abort().expect("abort overwrite vector");
                }
                "success"
            }
            "overwrite_sync_failure" => {
                let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
                let db = FilesystemServerRemoteBinaryDb::with_file_store(
                    store.clone(),
                    RepoId::new("conformance-repo-id"),
                    RepoName::new("conformance-repo"),
                    authority_root.clone(),
                    StoreGeneration::new(1),
                    ServerBinaryDbAuthorityMode::TestFixture,
                );
                let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General)
                    .expect("begin overwrite failure vector");
                write
                    .overwrite_record(
                        extended_record(vector, fixture.layout_id, BinaryDbFileFamily::Content),
                        vector.record_index.expect("overwrite record_index"),
                        &decode_hex(required(&vector.argument_hex, "argument_hex", &vector.id)),
                    )
                    .expect("overwrite before sync failure");
                store.arm(BinaryDbTestFault::once(
                    BinaryDbTestStorageOperation::SyncFile,
                    BinaryDbTestFaultTiming::After,
                    schema_fixture_path(required(&vector.record_path, "record_path", &vector.id)),
                ));
                let result = write.commit();
                drop(write);
                match result {
                    Ok(_) => "success",
                    Err(error) => outcome(error),
                }
            }
            "parent_path_rejected" => {
                let escape_path = sandbox.as_path().join("binary-db-parity-escape.bin");
                assert!(!escape_path.exists(), "escape fixture must start absent");
                let db = vector_db(authority_root.clone());
                let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                    &db,
                    BinaryDbCommandScope::ServerPlan,
                    BinaryDbNoopFsyncPolicy,
                )
                .expect("begin path vector");
                let result = write.append_record(
                    extended_record(vector, fixture.layout_id, BinaryDbFileFamily::Plan),
                    &decode_hex(required(&vector.argument_hex, "argument_hex", &vector.id)),
                );
                drop(write);
                assert!(!escape_path.exists(), "parent path must not be created");
                match result {
                    Ok(_) => "success",
                    Err(error) => outcome(error),
                }
            }
            "unauthorized_plan_family_rejected" => {
                let db = vector_db(authority_root.clone());
                let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                    &db,
                    BinaryDbCommandScope::ServerContent,
                    BinaryDbNoopFsyncPolicy,
                )
                .expect("begin unauthorized family vector");
                let result = write.append_record(
                    extended_record(vector, fixture.layout_id, BinaryDbFileFamily::Plan),
                    &decode_hex(required(&vector.argument_hex, "argument_hex", &vector.id)),
                );
                drop(write);
                match result {
                    Ok(_) => "success",
                    Err(error) => outcome(error),
                }
            }
            "authorized_plan_family_commit" => {
                let db = vector_db(authority_root.clone());
                let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                    &db,
                    BinaryDbCommandScope::ServerPlan,
                    BinaryDbNoopFsyncPolicy,
                )
                .expect("begin authorized family vector");
                write
                    .append_record(
                        extended_record(vector, fixture.layout_id, BinaryDbFileFamily::Plan),
                        &decode_hex(required(&vector.argument_hex, "argument_hex", &vector.id)),
                    )
                    .expect("append authorized Plan record");
                write.commit().expect("commit authorized Plan record");
                "success"
            }
            "multi_file_commit" | "multi_file_abort" => {
                let db = vector_db(authority_root.clone());
                let mut write = BinaryDbWriteTxn::begin_with_fsync_policy(
                    &db,
                    BinaryDbCommandScope::General,
                    BinaryDbNoopFsyncPolicy,
                )
                .expect("begin multi-file vector");
                append_multi_file(&mut write, vector, fixture.layout_id);
                if vector.operation == "multi_file_commit" {
                    write.commit().expect("commit multi-file vector");
                } else {
                    write.abort().expect("abort multi-file vector");
                }
                "success"
            }
            "multi_file_sync_failure" => {
                let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
                let db = FilesystemServerRemoteBinaryDb::with_file_store(
                    store.clone(),
                    RepoId::new("conformance-repo-id"),
                    RepoName::new("conformance-repo"),
                    authority_root.clone(),
                    StoreGeneration::new(1),
                    ServerBinaryDbAuthorityMode::TestFixture,
                );
                let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General)
                    .expect("begin multi-file sync failure vector");
                append_multi_file(&mut write, vector, fixture.layout_id);
                store.arm(BinaryDbTestFault::once(
                    BinaryDbTestStorageOperation::SyncFile,
                    BinaryDbTestFaultTiming::After,
                    schema_fixture_path(required(&vector.record_path, "record_path", &vector.id)),
                ));
                let result = write.commit();
                drop(write);
                match result {
                    Ok(_) => "success",
                    Err(error) => outcome(error),
                }
            }
            "multi_file_lock_cleanup_failure" => {
                let store = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
                let db = FilesystemServerRemoteBinaryDb::with_file_store(
                    store.clone(),
                    RepoId::new("conformance-repo-id"),
                    RepoName::new("conformance-repo"),
                    authority_root.clone(),
                    StoreGeneration::new(1),
                    ServerBinaryDbAuthorityMode::TestFixture,
                );
                let mut write = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::General)
                    .expect("begin multi-file cleanup failure vector");
                append_multi_file(&mut write, vector, fixture.layout_id);
                store.arm(BinaryDbTestFault::once(
                    BinaryDbTestStorageOperation::ReleaseProcessLock,
                    BinaryDbTestFaultTiming::Before,
                    "global.write.lock",
                ));
                let commit = write
                    .commit()
                    .expect("multi-file cleanup warning is committed");
                cleanup_warning = commit.lock_cleanup_warning().is_some();
                assert!(!root
                    .join(BinaryDbCommandScope::General.journal_file_name())
                    .exists());
                drop(write);
                "success"
            }
            operation => panic!("unsupported extended vector operation {operation}"),
        };

        assert_eq!(actual_outcome, vector.expected_outcome, "{}", vector.id);
        assert_eq!(
            actual_values, vector.expected_values,
            "{} values",
            vector.id
        );
        assert_eq!(
            cleanup_warning, vector.expected_cleanup_warning,
            "{} cleanup warning",
            vector.id
        );
        assert_expected_files(&root, vector);
        fs::remove_dir_all(sandbox.as_path()).expect("remove extended vector sandbox");
    }
}

#[test]
fn server_binary_db_cross_repo_parity_manifest_is_complete() {
    let fixture = fixture();
    let manifest = parity_manifest();
    let plan: PlanCaseIndex = serde_json::from_slice(SERVER_BINARY_DB_PLAN_GOLDEN_SOURCE)
        .expect("server Plan golden fixture must parse");

    assert_eq!(
        manifest.version,
        SERVER_BINARY_DB_CROSS_REPO_PARITY_MANIFEST_VERSION
    );
    assert_eq!(
        server_binary_db_cross_repo_parity_manifest_checksum(),
        SERVER_BINARY_DB_CROSS_REPO_PARITY_MANIFEST_CHECKSUM
    );
    assert_eq!(manifest.substrate_fixture.version, fixture.version);
    assert_eq!(
        manifest.substrate_fixture.sha256,
        server_binary_db_conformance_vector_checksum()
    );
    assert_eq!(manifest.plan_fixture.version, plan.version);
    assert_eq!(
        manifest.plan_fixture.sha256,
        server_binary_db_plan_golden_checksum()
    );
    assert_eq!(plan.layout_id, fixture.layout_id);

    let required_categories = manifest
        .required_vector_categories
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        required_categories,
        BTreeSet::from([
            "record",
            "transaction",
            "secondary_index",
            "overwrite",
            "path_family_binding",
            "multi_file_transaction",
            "plan_golden_bytes",
        ])
    );
    let actual_operations = fixture
        .extended_vectors
        .iter()
        .map(|vector| vector.operation.as_str())
        .collect::<BTreeSet<_>>();
    let required_operations = manifest
        .required_extended_operations
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_operations, required_operations);
    let actual_plan_cases = plan
        .cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<BTreeSet<_>>();
    let required_plan_cases = manifest
        .required_plan_cases
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_plan_cases, required_plan_cases);
}

#[test]
fn binary_db_conformance_vector_version_matches_core() {
    assert_eq!(
        SERVER_BINARY_DB_CONFORMANCE_VECTOR_VERSION,
        "ait.binary-db.conformance-vectors.v2"
    );
    assert_eq!(
        server_binary_db_conformance_vector_checksum(),
        "98cb6f0eb09037bd88b42c9f984426d353c44ab81e19564510e0a8fa43c66866"
    );
    for required in SERVER_BINARY_DB_CANONICAL_ROLLOUT_TESTS {
        assert!(!required.trim().is_empty());
    }
}
