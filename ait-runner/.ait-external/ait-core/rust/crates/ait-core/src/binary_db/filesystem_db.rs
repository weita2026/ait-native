//! Injected filesystem adapter and fixed-record, payload, and index primitives.

use super::*;
use std::collections::HashMap;

/// Filesystem adapter for local authority Binary DB storage.
#[derive(Clone, Debug)]
pub struct LocalBinaryDbFs<S = FilesystemFileIoStore> {
    files: S,
    authority_root: StorePath,
    local_repo_root: StorePath,
    local_authority_id: AuthorityId,
    current_line_state_scope: LocalStateScope,
    declared_bin_paths: Option<&'static [&'static str]>,
    declared_index_paths: Option<&'static [&'static str]>,
    admission_error: Option<String>,
    generation_guard: Option<std::sync::Arc<std::sync::Mutex<BinaryDbReadLockSet>>>,
    detached_generation_without_locks: bool,
}

impl LocalBinaryDbFs<FilesystemFileIoStore> {
    pub fn new(
        authority_root: impl Into<StorePath>,
        local_repo_root: impl Into<StorePath>,
        local_authority_id: AuthorityId,
        current_line_state_scope: LocalStateScope,
    ) -> Self {
        Self::with_file_io_store(
            FilesystemFileIoStore,
            authority_root,
            local_repo_root,
            local_authority_id,
            current_line_state_scope,
        )
    }
}

impl<S> LocalBinaryDbFs<S>
where
    S: BinaryDbFileStore,
{
    pub fn with_file_io_store(
        files: S,
        authority_root: impl Into<StorePath>,
        local_repo_root: impl Into<StorePath>,
        local_authority_id: AuthorityId,
        current_line_state_scope: LocalStateScope,
    ) -> Self {
        Self {
            files,
            authority_root: authority_root.into(),
            local_repo_root: local_repo_root.into(),
            local_authority_id,
            current_line_state_scope,
            declared_bin_paths: None,
            declared_index_paths: None,
            admission_error: None,
            generation_guard: None,
            detached_generation_without_locks: false,
        }
    }

    pub fn with_declared_bin_paths(mut self, paths: &'static [&'static str]) -> Self {
        self.declared_bin_paths = Some(paths);
        self
    }

    pub fn with_declared_index_paths(mut self, paths: &'static [&'static str]) -> Self {
        self.declared_index_paths = Some(paths);
        self
    }

    pub fn with_admission_error(mut self, error: Option<String>) -> Self {
        self.admission_error = error;
        self
    }

    pub fn with_generation_guard(
        mut self,
        guard: Option<std::sync::Arc<std::sync::Mutex<BinaryDbReadLockSet>>>,
    ) -> Self {
        self.generation_guard = guard;
        self
    }

    /// Disables runtime read/write locks for one private, detached generation.
    ///
    /// Callers must provide their own immutable-input and private-output
    /// guarantees. Normal repository authorities must never use this mode.
    pub(crate) fn for_detached_generation_without_locks(mut self) -> Self {
        self.detached_generation_without_locks = true;
        self
    }

    fn ensure_admitted(&self) -> StoreResult<()> {
        match &self.admission_error {
            Some(error) => Err(BinaryDbError::invalid_domain_data(error.clone())),
            None => Ok(()),
        }
    }

    pub fn file_io_store(&self) -> &S {
        &self.files
    }

    pub fn begin_read_txn(&self) -> BinaryDbReadTxn<'_, Self> {
        BinaryDbReadTxn::new(self)
    }

    pub fn begin_write_txn(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbWriteTxn<'_, Self, BinaryDbStoreFsyncPolicy<'_, Self>>> {
        BinaryDbWriteTxn::begin(self, command_scope)
    }

    pub fn begin_write_txn_with_fsync_policy<F>(
        &self,
        command_scope: BinaryDbCommandScope,
        fsync_policy: F,
    ) -> StoreResult<BinaryDbWriteTxn<'_, Self, F>>
    where
        F: BinaryDbFsyncPolicy,
    {
        BinaryDbWriteTxn::begin_with_fsync_policy(self, command_scope, fsync_policy)
    }

    fn resolve_relative_path<'a>(&self, path: &'a Path) -> StoreResult<&'a Path> {
        self.ensure_admitted()?;
        let path = validate_store_relative_path(path)?;
        let normalized = path.to_string_lossy();
        let (kind, declared) = match path.extension().and_then(|value| value.to_str()) {
            Some("bin") => (".bin", self.declared_bin_paths),
            Some("idx") => (".idx", self.declared_index_paths),
            _ => return Ok(path),
        };
        if let Some(declared) = declared {
            if !declared.iter().any(|candidate| *candidate == normalized) {
                return Err(BinaryDbError::invalid_domain_data(format!(
                    "Binary DB {kind} path {normalized:?} has no local schema declaration"
                )));
            }
        }
        Ok(path)
    }

    fn file_path_for_binary_record(&self, file: &BinaryFileId) -> StoreResult<PathBuf> {
        let rel = self.resolve_relative_path(file.relative_path().as_path())?;
        Ok(self.authority_root.as_ref().join(rel))
    }

    fn file_path_for_payload(&self, file: &BinaryPayloadFileId) -> StoreResult<PathBuf> {
        let rel = self.resolve_relative_path(file.relative_path().as_path())?;
        Ok(self.authority_root.as_ref().join(rel))
    }

    fn file_path_for_index(&self, file: &BinaryIndexId) -> StoreResult<PathBuf> {
        let rel = self.resolve_relative_path(file.relative_path().as_path())?;
        Ok(self.authority_root.as_ref().join(rel))
    }

    fn ensure_parent_directory(&self, path: &Path) -> StoreResult<()> {
        self.files
            .create_parent_dirs(path)
            .map_err(|e| file_io_error_to_binary("create parent directories", e))
    }

    fn read_u32_le(data: &[u8], field: &str) -> StoreResult<u32> {
        let mut buf = [0_u8; 4];
        if data.len() != 4 {
            return Err(BinaryDbError::corruption(format!("invalid {field} bytes")));
        }
        buf.copy_from_slice(data);
        Ok(u32::from_le_bytes(buf))
    }

    fn ensure_binary_file_valid(
        &self,
        path: &Path,
        expected_layout_id: u32,
        record_size: u32,
    ) -> StoreResult<u64> {
        let metadata_len = self.files.metadata_len(path).map_err(|e| {
            file_io_error_to_binary(format!("read binary file metadata {}", path.display()), e)
        })?;
        let metadata_len = metadata_len.ok_or_else(|| {
            BinaryDbError::missing_data(format!("binary file {} is missing", path.display()))
        })?;
        if metadata_len < u64::from(BIN_FILE_HEADER_BYTES) {
            return Err(BinaryDbError::corruption(format!(
                "binary file {} is too short to contain layout header",
                path.display()
            )));
        }
        if record_size == 0 {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "invalid fixed record size: {}",
                record_size
            )));
        }
        let layout = self.read_layout_id(path)?;
        if layout != expected_layout_id {
            return Err(BinaryDbError::layout_mismatch(format!(
                "layout id mismatch for {}: expected {expected_layout_id}, got {layout}",
                path.display()
            )));
        }
        let payload_size = metadata_len - u64::from(BIN_FILE_HEADER_BYTES);
        if payload_size % u64::from(record_size) != 0 {
            return Err(BinaryDbError::corruption(format!(
                "binary file {} has invalid payload length for record size {}",
                path.display(),
                record_size
            )));
        }
        Ok(payload_size / u64::from(record_size))
    }

    fn validate_index_file_layout(&self, path: &Path, expected_layout_id: u32) -> StoreResult<()> {
        let data = self
            .files
            .read_bytes(path)
            .map_err(|e| file_io_error_to_binary("read index file", e))?;
        Self::validate_index_file_bytes(path, expected_layout_id, &data)
    }

    fn validate_index_file_bytes(
        path: &Path,
        expected_layout_id: u32,
        data: &[u8],
    ) -> StoreResult<()> {
        if data.is_empty() {
            return Err(BinaryDbError::corruption(format!(
                "index file {} has no layout header",
                path.display()
            )));
        }
        if data.len() < 4 {
            return Err(BinaryDbError::corruption(format!(
                "index file {} is too small to contain header",
                path.display()
            )));
        }
        let header = Self::read_u32_le(&data[0..4], "index layout header")?;
        if header != expected_layout_id {
            return Err(BinaryDbError::layout_mismatch(format!(
                "index layout mismatch for {}: expected {expected_layout_id}, got {header}",
                path.display()
            )));
        }
        Ok(())
    }

    fn read_record_file_bytes(&self, file: &BinaryFileId) -> StoreResult<Option<Vec<u8>>> {
        let path = self.file_path_for_binary_record(file)?;
        if self.metadata_len(&path)?.is_none() {
            return Ok(None);
        }
        self.ensure_binary_file_valid(&path, file.layout_id(), file.record_size())?;
        self.files
            .read_bytes(&path)
            .map(Some)
            .map_err(|e| file_io_error_to_binary("read fixed record file", e))
    }

    fn record_from_file_bytes(
        file: &BinaryFileId,
        record_index: u32,
        bytes: Option<&[u8]>,
        path: &Path,
    ) -> StoreResult<BinaryRecordBytes> {
        let Some(bytes) = bytes else {
            return Err(BinaryDbError::missing_data(format!(
                "record index {} out of range for {}",
                record_index,
                path.display()
            )));
        };
        let record_size = usize::try_from(file.record_size())
            .map_err(|_| format!("record size overflow: {}", file.record_size()))?;
        let payload = bytes
            .get(usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4)..)
            .ok_or_else(|| {
                BinaryDbError::corruption(format!(
                    "binary file {} is too short to contain layout header",
                    path.display()
                ))
            })?;
        let offset = usize::try_from(record_index)
            .map_err(|_| format!("record index overflows usize: {record_index}"))?
            .checked_mul(record_size)
            .ok_or_else(|| format!("record byte offset overflows usize: {record_index}"))?;
        let end = offset
            .checked_add(record_size)
            .ok_or_else(|| format!("record byte range overflows usize: {record_index}"))?;
        let Some(record) = payload.get(offset..end) else {
            return Err(BinaryDbError::missing_data(format!(
                "record index {} out of range for {}",
                record_index,
                path.display()
            )));
        };
        Ok(record.to_vec())
    }

    fn read_index_file_bytes(&self, index: &BinaryIndexId) -> StoreResult<Option<Vec<u8>>> {
        let path = self.file_path_for_index(index)?;
        if self.metadata_len(&path)?.is_none() {
            return Ok(None);
        }
        let bytes = self
            .files
            .read_bytes(&path)
            .map_err(|e| file_io_error_to_binary("read index file", e))?;
        Self::validate_index_file_bytes(&path, index.layout_id(), &bytes)?;
        Ok(Some(bytes))
    }

    fn index_candidates_from_file_bytes(
        index: &BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        bytes: Option<&[u8]>,
        path: &Path,
    ) -> StoreResult<Vec<u32>> {
        let Some(bytes) = bytes else {
            return Ok(Vec::new());
        };
        if bytes.len() == usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4) {
            return Ok(Vec::new());
        }
        let mut cursor = usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4);
        let mut candidates = Vec::new();
        if let Some(key_size) = index.fixed_key_size() {
            let key_size = usize::try_from(key_size)
                .map_err(|_| format!("fixed index key size overflows usize: {key_size}"))?;
            if key.len() != key_size {
                return Err(BinaryDbError::invalid_domain_data(format!(
                    "fixed index key length {} does not match configured size {key_size}",
                    key.len()
                )));
            }
            let record_size = key_size
                .checked_add(4)
                .ok_or_else(|| BinaryDbError::corruption("fixed index record size overflow"))?;
            if (bytes.len() - cursor) % record_size != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "fixed index file {} has invalid body length",
                    path.display()
                )));
            }
            while cursor < bytes.len() {
                let key_end = cursor + key_size;
                let stored_index =
                    Self::read_u32_le(&bytes[key_end..key_end + 4], "candidate record index")?;
                if &bytes[cursor..key_end] == key {
                    let record_index = if index.stores_record_index_plus_one() {
                        stored_index.checked_sub(1).ok_or_else(|| {
                            BinaryDbError::corruption(format!(
                                "fixed index file {} contains zero plus-one index",
                                path.display()
                            ))
                        })?
                    } else {
                        stored_index
                    };
                    candidates.push(record_index);
                }
                cursor += record_size;
            }
            return Ok(candidates);
        }
        while cursor < bytes.len() {
            if cursor + 4 > bytes.len() {
                return Err(BinaryDbError::corruption(format!(
                    "index file {} is malformed",
                    path.display()
                )));
            }
            let key_len = Self::read_u32_le(&bytes[cursor..cursor + 4], "candidate key length")?;
            cursor += 4;
            let key_len_usize = usize::try_from(key_len)
                .map_err(|_| format!("candidate key length overflow: {key_len}"))?;
            let key_end = cursor
                .checked_add(key_len_usize)
                .ok_or_else(|| format!("candidate key range overflow in {}", path.display()))?;
            if key_end > bytes.len() {
                return Err(BinaryDbError::corruption(format!(
                    "index file {} is malformed",
                    path.display()
                )));
            }
            let candidate_key = &bytes[cursor..key_end];
            cursor = key_end;
            if cursor + 4 > bytes.len() {
                return Err(BinaryDbError::corruption(format!(
                    "index file {} is malformed",
                    path.display()
                )));
            }
            let record_index =
                Self::read_u32_le(&bytes[cursor..cursor + 4], "candidate record index")?;
            cursor += 4;
            if candidate_key == key {
                candidates.push(record_index);
            }
        }
        Ok(candidates)
    }

    fn index_candidate_map_from_file_bytes(
        index: &BinaryIndexId,
        bytes: Option<&[u8]>,
        path: &Path,
    ) -> StoreResult<HashMap<Vec<u8>, Vec<u32>>> {
        let Some(bytes) = bytes else {
            return Ok(HashMap::new());
        };
        let mut cursor = usize::try_from(BIN_FILE_HEADER_BYTES).unwrap_or(4);
        if bytes.len() == cursor {
            return Ok(HashMap::new());
        }
        let mut candidates = HashMap::<Vec<u8>, Vec<u32>>::new();
        if let Some(key_size) = index.fixed_key_size() {
            let key_size = usize::try_from(key_size)
                .map_err(|_| format!("fixed index key size overflows usize: {key_size}"))?;
            let record_size = key_size
                .checked_add(4)
                .ok_or_else(|| BinaryDbError::corruption("fixed index record size overflow"))?;
            if bytes.len() < cursor || (bytes.len() - cursor) % record_size != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "fixed index file {} has invalid body length",
                    path.display()
                )));
            }
            while cursor < bytes.len() {
                let key_end = cursor + key_size;
                let stored_index =
                    Self::read_u32_le(&bytes[key_end..key_end + 4], "candidate record index")?;
                let record_index = if index.stores_record_index_plus_one() {
                    stored_index.checked_sub(1).ok_or_else(|| {
                        BinaryDbError::corruption(format!(
                            "fixed index file {} contains zero plus-one index",
                            path.display()
                        ))
                    })?
                } else {
                    stored_index
                };
                candidates
                    .entry(bytes[cursor..key_end].to_vec())
                    .or_default()
                    .push(record_index);
                cursor += record_size;
            }
            return Ok(candidates);
        }

        while cursor < bytes.len() {
            if cursor + 4 > bytes.len() {
                return Err(BinaryDbError::corruption(format!(
                    "index file {} is malformed",
                    path.display()
                )));
            }
            let key_len = Self::read_u32_le(&bytes[cursor..cursor + 4], "candidate key length")?;
            cursor += 4;
            let key_len = usize::try_from(key_len)
                .map_err(|_| format!("candidate key length overflow: {key_len}"))?;
            let key_end = cursor
                .checked_add(key_len)
                .ok_or_else(|| format!("candidate key range overflow in {}", path.display()))?;
            let index_end = key_end
                .checked_add(4)
                .ok_or_else(|| format!("candidate index range overflow in {}", path.display()))?;
            if index_end > bytes.len() {
                return Err(BinaryDbError::corruption(format!(
                    "index file {} is malformed",
                    path.display()
                )));
            }
            let record_index =
                Self::read_u32_le(&bytes[key_end..index_end], "candidate record index")?;
            candidates
                .entry(bytes[cursor..key_end].to_vec())
                .or_default()
                .push(record_index);
            cursor = index_end;
        }
        Ok(candidates)
    }

    fn validate_payload_file_layout(
        &self,
        path: &Path,
        expected_layout_id: u32,
    ) -> StoreResult<u64> {
        let metadata_len = self
            .files
            .metadata_len(path)
            .map_err(|e| file_io_error_to_binary("read payload metadata", e))?
            .ok_or_else(|| {
                BinaryDbError::missing_data(format!("payload file {} is missing", path.display()))
            })?;
        if metadata_len < u64::from(BIN_FILE_HEADER_BYTES) {
            return Err(BinaryDbError::corruption(format!(
                "payload file {} is too small to contain layout header",
                path.display()
            )));
        }
        let bytes = self
            .files
            .read_range(path, 0, BIN_FILE_HEADER_BYTES)
            .map_err(|e| file_io_error_to_binary("read payload layout header", e))?;
        let header = Self::read_u32_le(&bytes, "payload layout header")?;
        if header != expected_layout_id {
            return Err(BinaryDbError::layout_mismatch(format!(
                "payload layout mismatch for {}: expected {expected_layout_id}, got {header}",
                path.display()
            )));
        }
        Ok(metadata_len)
    }

    fn read_layout_id(&self, path: &Path) -> StoreResult<u32> {
        let bytes = self
            .files
            .read_range(path, 0, BIN_FILE_HEADER_BYTES)
            .map_err(|e| file_io_error_to_binary("read binary layout header", e))?;
        Self::read_u32_le(&bytes, "binary layout header")
    }

    pub fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        write.ensure_authorized_path(index.relative_path())?;
        let path = self.file_path_for_index(&index)?;
        self.ensure_parent_directory(&path)?;

        let metadata_len = self
            .files
            .metadata_len(&path)
            .map_err(|e| file_io_error_to_binary("read index metadata", e))?;
        let initialized = metadata_len.is_some_and(|len| len > 0);
        let append_offset = if !initialized {
            let header_offset = self
                .files
                .append_bytes(&path, &index.layout_id().to_le_bytes())
                .map_err(|e| file_io_error_to_binary("write index layout header", e))?;
            if header_offset != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "index layout header append offset changed for {}: expected 0, got {header_offset}",
                    path.display()
                )));
            }
            u64::from(BIN_FILE_HEADER_BYTES)
        } else {
            let len = metadata_len
                .unwrap_or_else(|| unreachable!("initialized index files have metadata"));
            self.validate_index_file_layout(&path, index.layout_id())?;
            len
        };

        let bytes = match index.fixed_key_size() {
            Some(key_size) => {
                let expected = usize::try_from(key_size)
                    .map_err(|_| format!("fixed index key size overflows usize: {key_size}"))?;
                if key.len() != expected {
                    return Err(BinaryDbError::invalid_domain_data(format!(
                        "fixed index key length {} does not match configured size {key_size}",
                        key.len()
                    )));
                }
                let stored_index = if index.stores_record_index_plus_one() {
                    record_index.checked_add(1).ok_or_else(|| {
                        BinaryDbError::invalid_domain_data("fixed index record index overflow")
                    })?
                } else {
                    record_index
                };
                let mut bytes = Vec::with_capacity(expected + 4);
                bytes.extend_from_slice(key);
                bytes.extend_from_slice(&stored_index.to_le_bytes());
                bytes
            }
            None => {
                let key_len = u32::try_from(key.len())
                    .map_err(|_| format!("index key length exceeds u32::MAX: {}", key.len()))?;
                let mut bytes = Vec::with_capacity(8 + key.len());
                bytes.extend_from_slice(&key_len.to_le_bytes());
                bytes.extend_from_slice(key);
                bytes.extend_from_slice(&record_index.to_le_bytes());
                bytes
            }
        };
        let actual_offset = self
            .files
            .append_bytes(&path, &bytes)
            .map_err(|e| file_io_error_to_binary("write index candidate", e))?;
        if actual_offset != append_offset {
            return Err(BinaryDbError::corruption(format!(
                "index candidate append offset changed for {}: expected {append_offset}, got {actual_offset}",
                path.display()
            )));
        }
        Ok(())
    }
}

impl<S> BinaryDbIndexAppender for LocalBinaryDbFs<S>
where
    S: BinaryDbFileStore,
{
    fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        LocalBinaryDbFs::append_index_candidate(self, index, key, record_index, write)
    }
}

impl<S> BinaryDbRecoveryIo for LocalBinaryDbFs<S>
where
    S: BinaryDbFileStore,
{
    fn recovery_truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        self.files.truncate_file(path, len).map_err(|e| {
            file_io_error_to_binary(format!("truncate Binary DB file {}", path.display()), e)
        })
    }

    fn recovery_remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        self.files.remove_file_if_exists(path).map_err(|e| {
            file_io_error_to_binary(format!("remove Binary DB file {}", path.display()), e)
        })
    }
}

impl<S> BinaryDb for LocalBinaryDbFs<S>
where
    S: BinaryDbFileStore,
{
    fn authority_root(&self) -> &StorePath {
        &self.authority_root
    }

    fn acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        self.ensure_admitted()?;
        if self.detached_generation_without_locks {
            return Ok(BinaryDbCommandLockSet::detached_generation_noop(
                command_scope,
            ));
        }
        BinaryDbCommandLockSet::acquire_with_file_io_store(
            &self.files,
            self.authority_root(),
            command_scope,
        )
    }

    fn acquire_read_lock(&self) -> StoreResult<BinaryDbReadLockSet> {
        self.ensure_admitted()?;
        if self.detached_generation_without_locks {
            return Ok(BinaryDbReadLockSet::detached_generation_noop());
        }
        BinaryDbReadLockSet::try_acquire_with_file_io_store(&self.files, self.authority_root())
    }

    fn acquire_read_lock_for_scope(
        &self,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<BinaryDbReadLockSet> {
        self.ensure_admitted()?;
        if self.detached_generation_without_locks {
            return Ok(BinaryDbReadLockSet::detached_generation_noop());
        }
        BinaryDbReadLockSet::try_acquire_for_scope_with_file_io_store(
            &self.files,
            self.authority_root(),
            read_scope,
        )
    }

    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.files.sync_file(path).map_err(|e| {
            file_io_error_to_binary(format!("sync Binary DB file {}", path.display()), e)
        })
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.files.sync_dir(path).map_err(|e| {
            file_io_error_to_binary(format!("sync Binary DB directory {}", path.display()), e)
        })
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        self.files.metadata_len(path).map_err(|e| {
            file_io_error_to_binary(format!("read Binary DB metadata {}", path.display()), e)
        })
    }

    fn replace_file_atomically(
        &self,
        path: &Path,
        bytes: &[u8],
        publish_label: &str,
    ) -> StoreResult<PathBuf> {
        let authority_parent = self.authority_root.as_path().parent().ok_or_else(|| {
            BinaryDbError::invalid_domain_data(format!(
                "Binary DB authority root has no staging parent: {}",
                self.authority_root.as_path().display()
            ))
        })?;
        let staging_directory = authority_parent.join("binary-db-staging");
        self.files
            .write_bytes_atomically_from_directory(path, &staging_directory, bytes, publish_label)
            .map_err(|e| {
                file_io_error_to_binary(
                    format!("atomically replace Binary DB file {}", path.display()),
                    e,
                )
            })?;
        Ok(staging_directory)
    }

    fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32> {
        let path = self.file_path_for_binary_record(&file)?;
        let size = self.metadata_len(&path)?.ok_or_else(|| {
            BinaryDbError::missing_data(format!("binary file {} is missing", path.display()))
        })?;
        if size < u64::from(BIN_FILE_HEADER_BYTES) {
            return Err(BinaryDbError::corruption(format!(
                "binary file {} is too short to contain layout header",
                path.display()
            )));
        }
        self.read_layout_id(&path)
    }

    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        let path = self.file_path_for_binary_record(&file)?;
        if self.metadata_len(&path)?.is_none() {
            return Ok(0);
        }
        let count = self.ensure_binary_file_valid(&path, file.layout_id(), file.record_size())?;
        u32::try_from(count).map_err(|_| {
            BinaryDbError::corruption(format!(
                "record count overflow while reading {}",
                path.display()
            ))
        })
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<BinaryRecordBytes> {
        let path = self.file_path_for_binary_record(&file)?;
        let count = self.record_count(file.clone())?;
        if u64::from(record_index) >= u64::from(count) {
            return Err(BinaryDbError::missing_data(format!(
                "record index {} out of range for {}",
                record_index,
                path.display()
            )));
        }
        let offset = u64::from(BIN_FILE_HEADER_BYTES)
            + u64::from(record_index) * u64::from(file.record_size());
        self.files
            .read_range(&path, offset, file.record_size())
            .map_err(|e| file_io_error_to_binary("read fixed record", e))
    }

    fn read_record_in_read_txn(
        &self,
        file: BinaryFileId,
        record_index: u32,
        cache: &mut BinaryDbReadCache,
    ) -> StoreResult<BinaryRecordBytes> {
        let path = self.file_path_for_binary_record(&file)?;
        if !cache.record_files.contains_key(&file) {
            let bytes = self.read_record_file_bytes(&file)?;
            cache.record_files.insert(file.clone(), bytes);
        }
        let bytes = cache
            .record_files
            .get(&file)
            .and_then(|bytes| bytes.as_deref());
        Self::record_from_file_bytes(&file, record_index, bytes, &path)
    }

    fn append_record(
        &self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32> {
        write.ensure_authorized_path(file.relative_path())?;
        if record.len()
            != usize::try_from(file.record_size())
                .map_err(|_| format!("record size overflow: {}", file.record_size()))?
        {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "record length {} does not match configured size {}",
                record.len(),
                file.record_size()
            )));
        }

        let path = self.file_path_for_binary_record(&file)?;
        self.ensure_parent_directory(&path)?;

        let metadata_len = self.metadata_len(&path)?;
        let index = if matches!(metadata_len, Some(size) if size >= u64::from(BIN_FILE_HEADER_BYTES))
        {
            self.record_count(file.clone())?
        } else {
            0
        };

        let append_offset = if metadata_len.is_none() || metadata_len == Some(0) {
            let header_offset = self
                .files
                .append_bytes(&path, &file.layout_id().to_le_bytes())
                .map_err(|e| file_io_error_to_binary("write binary layout header", e))?;
            if header_offset != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "binary layout header append offset changed for {}: expected 0, got {header_offset}",
                    path.display()
                )));
            }
            u64::from(BIN_FILE_HEADER_BYTES)
        } else {
            let metadata_len = metadata_len
                .unwrap_or_else(|| unreachable!("metadata_len should be Some in this branch"));
            if metadata_len < u64::from(BIN_FILE_HEADER_BYTES) {
                return Err(BinaryDbError::corruption(format!(
                    "binary file {} is too small to contain layout header",
                    path.display()
                )));
            }
            let _ = self.ensure_binary_file_valid(&path, file.layout_id(), file.record_size())?;
            metadata_len
        };

        let actual_offset = self
            .files
            .append_bytes(&path, record)
            .map_err(|e| file_io_error_to_binary("append fixed record", e))?;
        if actual_offset != append_offset {
            return Err(BinaryDbError::corruption(format!(
                "fixed record append offset changed for {}: expected {append_offset}, got {actual_offset}",
                path.display()
            )));
        }

        Ok(index)
    }

    fn append_records(
        &self,
        file: BinaryFileId,
        records: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<(u32, u32)> {
        write.ensure_authorized_path(file.relative_path())?;
        let record_size = usize::try_from(file.record_size())
            .map_err(|_| format!("record size overflow: {}", file.record_size()))?;
        if record_size == 0 {
            return Err(BinaryDbError::invalid_domain_data(
                "fixed record batch has a zero record size",
            ));
        }
        if !records.len().is_multiple_of(record_size) {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "fixed record batch length {} is not aligned to record size {}",
                records.len(),
                file.record_size()
            )));
        }

        let batch_count = u32::try_from(records.len() / record_size).map_err(|_| {
            BinaryDbError::invalid_domain_data("fixed record batch count exceeds u32::MAX")
        })?;
        if batch_count == 0 {
            return Ok((self.record_count(file)?, 0));
        }

        let path = self.file_path_for_binary_record(&file)?;
        self.ensure_parent_directory(&path)?;
        let metadata_len = self.metadata_len(&path)?;
        let (start_index, append_offset, initialize) = match metadata_len {
            None | Some(0) => (0, u64::from(BIN_FILE_HEADER_BYTES), true),
            Some(metadata_len) => {
                if metadata_len < u64::from(BIN_FILE_HEADER_BYTES) {
                    return Err(BinaryDbError::corruption(format!(
                        "binary file {} is too small to contain layout header",
                        path.display()
                    )));
                }
                let count =
                    self.ensure_binary_file_valid(&path, file.layout_id(), file.record_size())?;
                let start_index = u32::try_from(count).map_err(|_| {
                    BinaryDbError::corruption(format!(
                        "record count overflow while appending {}",
                        path.display()
                    ))
                })?;
                (start_index, metadata_len, false)
            }
        };
        start_index.checked_add(batch_count).ok_or_else(|| {
            BinaryDbError::invalid_domain_data("fixed record batch count overflows u32")
        })?;

        if initialize {
            let header_offset = self
                .files
                .append_bytes(&path, &file.layout_id().to_le_bytes())
                .map_err(|e| file_io_error_to_binary("write binary layout header", e))?;
            if header_offset != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "binary layout header append offset changed for {}: expected 0, got {header_offset}",
                    path.display()
                )));
            }
        }

        let actual_offset = self
            .files
            .append_bytes(&path, records)
            .map_err(|e| file_io_error_to_binary("append fixed record batch", e))?;
        if actual_offset != append_offset {
            return Err(BinaryDbError::corruption(format!(
                "fixed record batch append offset changed for {}: expected {append_offset}, got {actual_offset}",
                path.display()
            )));
        }

        Ok((start_index, batch_count))
    }

    fn overwrite_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        write.ensure_authorized_path(file.relative_path())?;
        if record.len()
            != usize::try_from(file.record_size())
                .map_err(|_| format!("record size overflow: {}", file.record_size()))?
        {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "record length {} does not match configured size {}",
                record.len(),
                file.record_size()
            )));
        }
        let path = self.file_path_for_binary_record(&file)?;
        let count = self.record_count(file.clone())?;
        if record_index >= count {
            return Err(BinaryDbError::missing_data(format!(
                "record index {record_index} out of range for {}",
                path.display()
            )));
        }
        let offset = u64::from(BIN_FILE_HEADER_BYTES)
            .checked_add(u64::from(record_index) * u64::from(file.record_size()))
            .ok_or_else(|| BinaryDbError::invalid_domain_data("record offset overflow"))?;
        self.files
            .overwrite_range(&path, offset, record)
            .map_err(|e| file_io_error_to_binary("overwrite fixed record", e))
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        let path = self.file_path_for_payload(&file)?;
        let metadata_len = self.validate_payload_file_layout(&path, file.layout_id())?;
        let min_offset = u64::from(BIN_FILE_HEADER_BYTES);
        let len_u64 = u64::from(len);
        if offset < min_offset
            || offset
                .checked_add(len_u64)
                .filter(|end| *end <= metadata_len)
                .is_none()
        {
            return Err(BinaryDbError::missing_data(format!(
                "payload read out of range: offset {} length {}",
                offset, len
            )));
        }

        self.files
            .read_range(&path, offset, len)
            .map_err(|e| file_io_error_to_binary("read payload bytes", e))
    }

    fn append_payload(
        &self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<PayloadRange> {
        write.ensure_authorized_path(file.relative_path())?;
        if self.declared_bin_paths.is_some() && bytes.is_empty() {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "declared Binary DB payload {:?} must not create or preserve a header-only .bin file",
                file.relative_path()
            )));
        }
        let path = self.file_path_for_payload(&file)?;
        self.ensure_parent_directory(&path)?;
        let metadata_len = self.metadata_len(&path)?;
        let offset = match metadata_len {
            None | Some(0) => {
                let header_offset = self
                    .files
                    .append_bytes(&path, &file.layout_id().to_le_bytes())
                    .map_err(|e| file_io_error_to_binary("write payload layout header", e))?;
                if header_offset != 0 {
                    return Err(BinaryDbError::corruption(format!(
                        "payload layout header append offset changed for {}: expected 0, got {header_offset}",
                        path.display()
                    )));
                }
                u64::from(BIN_FILE_HEADER_BYTES)
            }
            Some(len) if len < u64::from(BIN_FILE_HEADER_BYTES) => {
                return Err(BinaryDbError::corruption(format!(
                    "payload file {} is too small to contain layout header",
                    path.display()
                )));
            }
            Some(_) => self.validate_payload_file_layout(&path, file.layout_id())?,
        };
        let append_offset = self
            .files
            .append_bytes(&path, bytes)
            .map_err(|e| file_io_error_to_binary("append payload bytes", e))?;
        if append_offset != offset {
            return Err(BinaryDbError::corruption(format!(
                "payload append offset changed for {}: expected {offset}, got {append_offset}",
                path.display()
            )));
        }
        Ok(PayloadRange {
            payload_offset: offset,
            payload_len: u32::try_from(bytes.len())
                .map_err(|_| format!("payload length overflows u32: {}", bytes.len()))?,
        })
    }

    fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        let path = self.file_path_for_index(&index)?;
        let bytes = self.read_index_file_bytes(&index)?;
        Self::index_candidates_from_file_bytes(&index, key, bytes.as_deref(), &path)
    }

    fn lookup_index_in_read_txn(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        cache: &mut BinaryDbReadCache,
    ) -> StoreResult<Vec<u32>> {
        let path = self.file_path_for_index(&index)?;
        if !cache.index_files.contains_key(&index) {
            let bytes = self.read_index_file_bytes(&index)?;
            cache.index_files.insert(index.clone(), bytes);
        }
        if let Some(key_size) = index.fixed_key_size() {
            let key_size = usize::try_from(key_size)
                .map_err(|_| format!("fixed index key size overflows usize: {key_size}"))?;
            if key.len() != key_size {
                return Err(BinaryDbError::invalid_domain_data(format!(
                    "fixed index key length {} does not match configured size {key_size}",
                    key.len()
                )));
            }
        }
        if !cache.parsed_index_candidates.contains_key(&index) {
            let parsed = {
                let bytes = cache
                    .index_files
                    .get(&index)
                    .and_then(|bytes| bytes.as_deref());
                Self::index_candidate_map_from_file_bytes(&index, bytes, &path)?
            };
            cache.parsed_index_candidates.insert(index.clone(), parsed);
        }
        Ok(cache
            .parsed_index_candidates
            .get(&index)
            .and_then(|rows| rows.get(key))
            .cloned()
            .unwrap_or_default())
    }
}

impl<S> LocalBinaryDb for LocalBinaryDbFs<S>
where
    S: BinaryDbFileStore,
{
    fn local_repo_root(&self) -> &StorePath {
        &self.local_repo_root
    }

    fn local_authority_id(&self) -> &AuthorityId {
        &self.local_authority_id
    }

    fn current_line_state_scope(&self) -> LocalStateScope {
        self.current_line_state_scope
    }
}
