use sha2::{Digest, Sha256};

pub const BINARY_DB_CONFORMANCE_VECTOR_VERSION: &str = "ait.binary-db.conformance-vectors.v2";
pub const BINARY_DB_CONFORMANCE_VECTOR_CHECKSUM: &str =
    "98cb6f0eb09037bd88b42c9f984426d353c44ab81e19564510e0a8fa43c66866";
pub const BINARY_DB_CONFORMANCE_VECTOR_SOURCE: &[u8] =
    include_bytes!("../../tests/fixtures/binary_db_conformance_vectors_v2.json");
pub const BINARY_DB_PLAN_GOLDEN_VERSION: &str = "ait.plan-binary-db.golden-bytes.v1";
pub const BINARY_DB_PLAN_GOLDEN_CHECKSUM: &str =
    "feeb856eba66b4040b85b6a462b7342a94f978798052b8097b526e2cbcae0d96";
pub const BINARY_DB_PLAN_GOLDEN_SOURCE: &[u8] =
    include_bytes!("../../tests/fixtures/plan_binary_db_layout1_golden_v1.json");
pub const BINARY_DB_CROSS_REPO_PARITY_MANIFEST_VERSION: &str =
    "ait.binary-db.cross-repo-parity-manifest.v1";
pub const BINARY_DB_CROSS_REPO_PARITY_MANIFEST_CHECKSUM: &str =
    "eaf68ddc057e06fd6d01cecab0d6d89d7862a65432c651179f13fba9f7ef9535";
pub const BINARY_DB_CROSS_REPO_PARITY_MANIFEST_SOURCE: &[u8] =
    include_bytes!("../../tests/fixtures/binary_db_cross_repo_parity_manifest_v1.json");

pub fn binary_db_conformance_vector_checksum() -> String {
    format!("{:x}", Sha256::digest(BINARY_DB_CONFORMANCE_VECTOR_SOURCE))
}

pub fn binary_db_plan_golden_checksum() -> String {
    format!("{:x}", Sha256::digest(BINARY_DB_PLAN_GOLDEN_SOURCE))
}

pub fn binary_db_cross_repo_parity_manifest_checksum() -> String {
    format!(
        "{:x}",
        Sha256::digest(BINARY_DB_CROSS_REPO_PARITY_MANIFEST_SOURCE)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_db::{
        AuthorityId, BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbErrorKind,
        BinaryDbFsyncPolicy, BinaryDbNoopFsyncPolicy, BinaryFileId, BinaryIndexId,
        BinaryPayloadFileId, LocalBinaryDbFs, LocalStateScope,
    };
    use crate::file_io::{
        BoxedFileIoProcessLockGuard, FileIoByteStore, FileIoDurabilityStore, FileIoError,
        FileIoErrorKind, FileIoLockMode, FileIoLockStore, FileIoLockWait, FileIoProcessLockGuard,
        FileIoResult, FileIoStore, FilesystemFileIoStore,
    };
    use serde::Deserialize;
    use std::cell::Cell;
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;

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
        serde_json::from_slice(BINARY_DB_CONFORMANCE_VECTOR_SOURCE)
            .expect("conformance vector source must parse")
    }

    fn parity_manifest() -> ParityManifest {
        serde_json::from_slice(BINARY_DB_CROSS_REPO_PARITY_MANIFEST_SOURCE)
            .expect("cross-repo parity manifest must parse")
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

    fn initialize(path: &Path, initial_hex: Option<&str>) {
        if let Some(initial_hex) = initial_hex {
            fs::write(path, decode_hex(initial_hex)).expect("write initial vector bytes");
        }
    }

    fn initialize_files(root: &Path, files: &BTreeMap<String, Option<String>>) {
        for (relative, initial_hex) in files {
            let path = root.join(relative);
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

    fn assert_expected_files(
        root: &Path,
        vector: &ExtendedVector,
        expected: &BTreeMap<String, Option<String>>,
    ) {
        for (relative, expected_hex) in expected {
            assert_eq!(
                after_hex(&root.join(relative)),
                *expected_hex,
                "{} exact bytes for {relative}",
                vector.id
            );
        }
    }

    fn after_hex(path: &Path) -> Option<String> {
        fs::read(path).ok().map(hex::encode)
    }

    mod hex {
        pub fn encode(bytes: Vec<u8>) -> String {
            bytes.iter().map(|byte| format!("{byte:02x}")).collect()
        }
    }

    fn db(root: &Path) -> LocalBinaryDbFs {
        LocalBinaryDbFs::new(
            root,
            root,
            AuthorityId::new("conformance-authority"),
            LocalStateScope::Repository,
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
    fn binary_db_conformance_vectors_v2() {
        let fixture = fixture();
        assert_eq!(fixture.version, BINARY_DB_CONFORMANCE_VECTOR_VERSION);
        assert_eq!(
            binary_db_conformance_vector_checksum(),
            BINARY_DB_CONFORMANCE_VECTOR_CHECKSUM
        );

        for vector in &fixture.record_vectors {
            let temp = tempdir().expect("vector tempdir");
            let path = temp.path().join("vector.bin");
            initialize(&path, vector.initial_hex.as_deref());
            let db = db(temp.path());
            let record = BinaryFileId::new("vector.bin", fixture.layout_id, fixture.record_size);
            let payload = BinaryPayloadFileId::new("vector.bin", fixture.layout_id);

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
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
                            BinaryDbCommandScope::General,
                            BinaryDbNoopFsyncPolicy,
                        )
                        .expect("begin vector transaction");
                    let argument = decode_hex(vector.argument_hex.as_deref().unwrap());
                    let result = write.append_record(record, &argument);
                    match result {
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
        }
    }

    #[derive(Clone, Debug)]
    struct ShortWriteFileIoStore {
        fail_next_append: Rc<Cell<bool>>,
        fail_next_lock_release: Arc<AtomicBool>,
    }

    impl ShortWriteFileIoStore {
        fn armed() -> Self {
            Self {
                fail_next_append: Rc::new(Cell::new(true)),
                fail_next_lock_release: Arc::new(AtomicBool::new(false)),
            }
        }

        fn with_lock_release_failure() -> Self {
            Self {
                fail_next_append: Rc::new(Cell::new(false)),
                fail_next_lock_release: Arc::new(AtomicBool::new(true)),
            }
        }
    }

    #[derive(Debug)]
    struct FailOnceReleaseGuard {
        inner: BoxedFileIoProcessLockGuard,
        fail_next_lock_release: Arc<AtomicBool>,
    }

    impl FileIoProcessLockGuard for FailOnceReleaseGuard {
        fn replace_contents_and_flush(&mut self, bytes: &[u8]) -> FileIoResult<()> {
            self.inner.replace_contents_and_flush(bytes)
        }

        fn clear_contents_and_flush(&mut self) -> FileIoResult<()> {
            self.inner.clear_contents_and_flush()
        }

        fn release(&mut self) -> FileIoResult<()> {
            if self.fail_next_lock_release.swap(false, Ordering::SeqCst) {
                return Err(FileIoError::new(
                    FileIoErrorKind::Lock,
                    "injected conformance lock release failure",
                ));
            }
            self.inner.release()
        }
    }

    impl FileIoStore for ShortWriteFileIoStore {
        fn home_dir(&self) -> Option<PathBuf> {
            FilesystemFileIoStore.home_dir()
        }

        fn path_exists(&self, path: &Path) -> bool {
            FilesystemFileIoStore.path_exists(path)
        }

        fn read_bytes(&self, path: &Path) -> FileIoResult<Vec<u8>> {
            FilesystemFileIoStore.read_bytes(path)
        }

        fn read_to_string(&self, path: &Path) -> FileIoResult<String> {
            FilesystemFileIoStore.read_to_string(path)
        }

        fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()> {
            FilesystemFileIoStore.write_string(path, text)
        }

        fn write_string_atomically(
            &self,
            path: &Path,
            text: &str,
            publish_label: &str,
        ) -> FileIoResult<()> {
            FilesystemFileIoStore.write_string_atomically(path, text, publish_label)
        }
    }

    impl FileIoByteStore for ShortWriteFileIoStore {
        fn write_bytes(&self, path: &Path, bytes: &[u8]) -> FileIoResult<()> {
            FilesystemFileIoStore.write_bytes(path, bytes)
        }

        fn read_range(&self, path: &Path, offset: u64, len: u32) -> FileIoResult<Vec<u8>> {
            FilesystemFileIoStore.read_range(path, offset, len)
        }

        fn metadata_len(&self, path: &Path) -> FileIoResult<Option<u64>> {
            FilesystemFileIoStore.metadata_len(path)
        }

        fn create_parent_dirs(&self, path: &Path) -> FileIoResult<()> {
            FilesystemFileIoStore.create_parent_dirs(path)
        }

        fn append_bytes(&self, path: &Path, bytes: &[u8]) -> FileIoResult<u64> {
            if self.fail_next_append.replace(false) {
                let prefix_len = usize::from(!bytes.is_empty());
                if prefix_len > 0 {
                    FilesystemFileIoStore.append_bytes(path, &bytes[..prefix_len])?;
                }
                return Err(FileIoError::new(
                    FileIoErrorKind::WriteZero,
                    "injected conformance short write",
                ));
            }
            FilesystemFileIoStore.append_bytes(path, bytes)
        }

        fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> FileIoResult<()> {
            FilesystemFileIoStore.overwrite_range(path, offset, bytes)
        }

        fn truncate_file(&self, path: &Path, len: u64) -> FileIoResult<()> {
            FilesystemFileIoStore.truncate_file(path, len)
        }

        fn remove_file_if_exists(&self, path: &Path) -> FileIoResult<()> {
            FilesystemFileIoStore.remove_file_if_exists(path)
        }
    }

    impl FileIoDurabilityStore for ShortWriteFileIoStore {
        fn sync_file(&self, path: &Path) -> FileIoResult<()> {
            FilesystemFileIoStore.sync_file(path)
        }

        fn sync_dir(&self, path: &Path) -> FileIoResult<()> {
            FilesystemFileIoStore.sync_dir(path)
        }
    }

    impl FileIoLockStore for ShortWriteFileIoStore {
        fn acquire_process_lock(
            &self,
            path: &Path,
            mode: FileIoLockMode,
            wait: FileIoLockWait,
        ) -> FileIoResult<Option<BoxedFileIoProcessLockGuard>> {
            Ok(FilesystemFileIoStore
                .acquire_process_lock(path, mode, wait)?
                .map(|inner| {
                    Box::new(FailOnceReleaseGuard {
                        inner,
                        fail_next_lock_release: Arc::clone(&self.fail_next_lock_release),
                    }) as BoxedFileIoProcessLockGuard
                }))
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct FailSyncPolicy;

    impl BinaryDbFsyncPolicy for FailSyncPolicy {
        fn sync_file(&self, _path: &Path) -> Result<(), BinaryDbError> {
            Err(BinaryDbError::new(
                BinaryDbErrorKind::Io,
                "injected conformance sync failure",
            ))
        }

        fn sync_directory(&self, _path: &Path) -> Result<(), BinaryDbError> {
            Ok(())
        }
    }

    #[test]
    fn binary_db_transaction_conformance_v2() {
        let fixture = fixture();
        for vector in &fixture.transaction_vectors {
            let temp = tempdir().expect("transaction vector tempdir");
            let path = temp.path().join("vector.bin");
            initialize(&path, vector.initial_hex.as_deref());
            let record = BinaryFileId::new("vector.bin", fixture.layout_id, fixture.record_size);
            let argument = decode_hex(vector.argument_hex.as_deref().unwrap());

            let (actual_outcome, actual_value) = if vector.operation == "append_short_write" {
                let db = LocalBinaryDbFs::with_file_io_store(
                    ShortWriteFileIoStore::armed(),
                    temp.path(),
                    temp.path(),
                    AuthorityId::new("conformance-authority"),
                    LocalStateScope::Repository,
                );
                let mut write = db
                    .begin_write_txn_with_fsync_policy(
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
            } else if vector.operation == "commit_lock_cleanup_failure" {
                let db = LocalBinaryDbFs::with_file_io_store(
                    ShortWriteFileIoStore::with_lock_release_failure(),
                    temp.path(),
                    temp.path(),
                    AuthorityId::new("conformance-authority"),
                    LocalStateScope::Repository,
                );
                let mut write = db
                    .begin_write_txn_with_fsync_policy(
                        BinaryDbCommandScope::General,
                        BinaryDbNoopFsyncPolicy,
                    )
                    .expect("begin cleanup-failure vector transaction");
                let value = write
                    .append_record(record, &argument)
                    .expect("append before cleanup failure");
                let commit = write
                    .commit()
                    .expect("lock cleanup failure is a committed outcome");
                assert!(commit.lock_cleanup_warning().is_some());
                drop(write);
                ("success", Some(value as u64))
            } else {
                let db = db(temp.path());
                match vector.operation.as_str() {
                    "append_invalid_record" => {
                        let mut write = db
                            .begin_write_txn_with_fsync_policy(
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
                    "commit_sync_failure" => {
                        let mut write = db
                            .begin_write_txn_with_fsync_policy(
                                BinaryDbCommandScope::General,
                                FailSyncPolicy,
                            )
                            .expect("begin sync-failure vector transaction");
                        let value = write
                            .append_record(record, &argument)
                            .expect("append before sync failure");
                        let result = write.commit();
                        drop(write);
                        match result {
                            Ok(_) => ("success", Some(value as u64)),
                            Err(error) => (outcome(error), None),
                        }
                    }
                    "abort" | "abort_twice" => {
                        let mut write = db
                            .begin_write_txn_with_fsync_policy(
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
                        let mut write = db
                            .begin_write_txn_with_fsync_policy(
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
        }
    }

    fn required<'a>(value: &'a Option<String>, field: &str, id: &str) -> &'a str {
        value
            .as_deref()
            .unwrap_or_else(|| panic!("{id} is missing {field}"))
    }

    fn extended_record(vector: &ExtendedVector, layout_id: u32) -> BinaryFileId {
        BinaryFileId::new(
            required(&vector.record_path, "record_path", &vector.id),
            layout_id,
            2,
        )
    }

    fn append_multi_file<B, F>(
        write: &mut crate::binary_db::BinaryDbWriteTxn<'_, B, F>,
        vector: &ExtendedVector,
        layout_id: u32,
    ) where
        B: BinaryDb + crate::binary_db::BinaryDbIndexAppender + ?Sized,
        F: BinaryDbFsyncPolicy,
    {
        let record = extended_record(vector, layout_id);
        let payload = BinaryPayloadFileId::new(
            required(&vector.payload_path, "payload_path", &vector.id),
            layout_id,
        );
        let index = BinaryIndexId::new(
            required(&vector.index_path, "index_path", &vector.id),
            layout_id,
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
    fn binary_db_extended_conformance_v2() {
        let fixture = fixture();
        for vector in &fixture.extended_vectors {
            let temp = tempdir().expect("extended vector tempdir");
            let root = temp.path().join("authority");
            fs::create_dir_all(&root).expect("create extended vector authority root");
            initialize_files(&root, &vector.initial_files);
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
                    let db = db(&root);
                    let path = required(&vector.index_path, "index_path", &vector.id);
                    let index = match vector.fixed_key_size {
                        Some(key_size) => BinaryIndexId::new_fixed(
                            path,
                            fixture.layout_id,
                            key_size,
                            vector.stores_record_index_plus_one,
                        ),
                        None => BinaryIndexId::new(path, fixture.layout_id),
                    };
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
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
                    let db = db(&root);
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
                            BinaryDbCommandScope::General,
                            BinaryDbNoopFsyncPolicy,
                        )
                        .expect("begin overwrite vector transaction");
                    write
                        .overwrite_record(
                            extended_record(vector, fixture.layout_id),
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
                    let db = db(&root);
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
                            BinaryDbCommandScope::General,
                            FailSyncPolicy,
                        )
                        .expect("begin overwrite failure vector");
                    write
                        .overwrite_record(
                            extended_record(vector, fixture.layout_id),
                            vector.record_index.expect("overwrite record_index"),
                            &decode_hex(required(&vector.argument_hex, "argument_hex", &vector.id)),
                        )
                        .expect("overwrite before sync failure");
                    let result = write.commit();
                    drop(write);
                    match result {
                        Ok(_) => "success",
                        Err(error) => outcome(error),
                    }
                }
                "parent_path_rejected" => {
                    let escape_path = temp.path().join("binary-db-parity-escape.bin");
                    assert!(!escape_path.exists(), "escape fixture must start absent");
                    let db = db(&root);
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
                            BinaryDbCommandScope::PlanSyncLocalPlan,
                            BinaryDbNoopFsyncPolicy,
                        )
                        .expect("begin path vector");
                    let result = write.append_record(
                        extended_record(vector, fixture.layout_id),
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
                    let db = db(&root);
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
                            BinaryDbCommandScope::ContentWrite,
                            BinaryDbNoopFsyncPolicy,
                        )
                        .expect("begin unauthorized family vector");
                    let result = write.append_record(
                        extended_record(vector, fixture.layout_id),
                        &decode_hex(required(&vector.argument_hex, "argument_hex", &vector.id)),
                    );
                    drop(write);
                    match result {
                        Ok(_) => "success",
                        Err(error) => outcome(error),
                    }
                }
                "authorized_plan_family_commit" => {
                    let db = db(&root);
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
                            BinaryDbCommandScope::PlanSyncLocalPlan,
                            BinaryDbNoopFsyncPolicy,
                        )
                        .expect("begin authorized family vector");
                    write
                        .append_record(
                            extended_record(vector, fixture.layout_id),
                            &decode_hex(required(&vector.argument_hex, "argument_hex", &vector.id)),
                        )
                        .expect("append authorized Plan record");
                    write.commit().expect("commit authorized Plan record");
                    "success"
                }
                "multi_file_commit" | "multi_file_abort" => {
                    let db = db(&root);
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
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
                    let db = db(&root);
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
                            BinaryDbCommandScope::General,
                            FailSyncPolicy,
                        )
                        .expect("begin multi-file sync failure vector");
                    append_multi_file(&mut write, vector, fixture.layout_id);
                    let result = write.commit();
                    drop(write);
                    match result {
                        Ok(_) => "success",
                        Err(error) => outcome(error),
                    }
                }
                "multi_file_lock_cleanup_failure" => {
                    let db = LocalBinaryDbFs::with_file_io_store(
                        ShortWriteFileIoStore::with_lock_release_failure(),
                        root.as_path(),
                        root.as_path(),
                        AuthorityId::new("conformance-authority"),
                        LocalStateScope::Repository,
                    );
                    let mut write = db
                        .begin_write_txn_with_fsync_policy(
                            BinaryDbCommandScope::General,
                            BinaryDbNoopFsyncPolicy,
                        )
                        .expect("begin multi-file cleanup failure vector");
                    append_multi_file(&mut write, vector, fixture.layout_id);
                    let commit = write
                        .commit()
                        .expect("multi-file cleanup warning is committed");
                    cleanup_warning = commit.lock_cleanup_warning().is_some();
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
            assert_expected_files(&root, vector, &vector.expected_files);
        }
    }

    #[test]
    fn binary_db_cross_repo_parity_manifest_is_complete() {
        let fixture = fixture();
        let manifest = parity_manifest();
        let plan: PlanCaseIndex = serde_json::from_slice(BINARY_DB_PLAN_GOLDEN_SOURCE)
            .expect("Plan golden fixture must parse");

        assert_eq!(
            manifest.version,
            BINARY_DB_CROSS_REPO_PARITY_MANIFEST_VERSION
        );
        assert_eq!(
            binary_db_cross_repo_parity_manifest_checksum(),
            BINARY_DB_CROSS_REPO_PARITY_MANIFEST_CHECKSUM
        );
        assert_eq!(manifest.substrate_fixture.version, fixture.version);
        assert_eq!(
            manifest.substrate_fixture.sha256,
            binary_db_conformance_vector_checksum()
        );
        assert_eq!(manifest.plan_fixture.version, plan.version);
        assert_eq!(
            manifest.plan_fixture.sha256,
            binary_db_plan_golden_checksum()
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
}
