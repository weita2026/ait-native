use super::*;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct FilesystemServerRemoteBinaryDb<S = ServerBinaryDbFilesystemStore> {
    files: S,
    repo_id: RepoId,
    repo_name: RepoName,
    authority_root: StorePath,
    storage_generation: StoreGeneration,
    authority_mode: ServerBinaryDbAuthorityMode,
    writer_admission: Arc<BinaryDbWriterAdmission>,
}

impl FilesystemServerRemoteBinaryDb<ServerBinaryDbFilesystemStore> {
    pub fn serving_authority(
        repo_id: RepoId,
        repo_name: RepoName,
        authority_root: StorePath,
        storage_generation: StoreGeneration,
    ) -> Self {
        Self::with_file_store(
            ServerBinaryDbFilesystemStore,
            repo_id,
            repo_name,
            authority_root,
            storage_generation,
            ServerBinaryDbAuthorityMode::ServingAuthority,
        )
    }

    pub fn test_fixture(
        repo_id: RepoId,
        repo_name: RepoName,
        authority_root: StorePath,
        storage_generation: StoreGeneration,
    ) -> Self {
        Self::with_file_store(
            ServerBinaryDbFilesystemStore,
            repo_id,
            repo_name,
            authority_root,
            storage_generation,
            ServerBinaryDbAuthorityMode::TestFixture,
        )
    }
}

impl<S> FilesystemServerRemoteBinaryDb<S>
where
    S: ServerBinaryDbStore,
{
    pub const HEADER_SIZE: u64 = 4;

    pub fn with_file_store(
        files: S,
        repo_id: RepoId,
        repo_name: RepoName,
        authority_root: StorePath,
        storage_generation: StoreGeneration,
        authority_mode: ServerBinaryDbAuthorityMode,
    ) -> Self {
        Self {
            files,
            repo_id,
            repo_name,
            authority_root,
            storage_generation,
            authority_mode,
            writer_admission: Arc::new(BinaryDbWriterAdmission::default()),
        }
    }

    pub fn file_store(&self) -> &S {
        &self.files
    }

    pub fn resolve_record_path(&self, file: &BinaryFileId) -> StoreResult<PathBuf> {
        self.validate_bin_schema(file.as_str(), file.layout_id(), file.family())?;
        Self::resolve_relative_path(self.authority_root.as_path(), file.as_str())
    }

    pub fn resolve_payload_path(&self, file: &BinaryPayloadFileId) -> StoreResult<PathBuf> {
        self.validate_bin_schema(file.as_str(), file.layout_id(), file.family())?;
        Self::resolve_relative_path(self.authority_root.as_path(), file.as_str())
    }

    pub fn resolve_index_path(&self, index: &BinaryIndexId) -> StoreResult<PathBuf> {
        self.validate_bin_schema(index.as_str(), index.layout_id(), index.family())?;
        if self.authority_mode.is_serving_authority() && index.as_str().ends_with(".idx") {
            let schema =
                crate::foundation::server_binary_db_schema_registry::server_binary_db_index_schema(
                    index.as_str(),
                )
                .ok_or_else(|| {
                    BinaryDbError::invalid_domain_data(format!(
                        "undeclared server Binary DB .idx path is forbidden: {}",
                        index.as_str()
                    ))
                })?;
            let entry_size = index
                .fixed_key_size()
                .and_then(|key_size| key_size.checked_add(4))
                .ok_or_else(|| {
                    BinaryDbError::invalid_domain_data(format!(
                        "server Binary DB index '{}' must use its fixed v0 record width",
                        index.as_str()
                    ))
                })?;
            if schema.layout_id != index.layout_id()
                || schema.family != index.family()
                || schema.record_size != entry_size
            {
                return Err(BinaryDbError::layout_mismatch(format!(
                    "server Binary DB index schema mismatch for {}",
                    index.as_str()
                )));
            }
        }
        Self::resolve_relative_path(self.authority_root.as_path(), index.as_str())
    }

    fn validate_bin_schema(
        &self,
        path: &str,
        layout_id: u32,
        family: BinaryDbFileFamily,
    ) -> StoreResult<()> {
        if !path.ends_with(".bin") {
            return Ok(());
        }
        let schema =
            crate::foundation::server_binary_db_schema_registry::server_binary_db_bin_schema(path)
                .ok_or_else(|| {
                    BinaryDbError::invalid_domain_data(format!(
                        "undeclared server Binary DB .bin path is forbidden: {path}"
                    ))
                })?;
        if schema.layout_id != layout_id {
            return Err(BinaryDbError::layout_mismatch(format!(
                "server Binary DB schema layout mismatch for {path}: declared={}, requested={layout_id}",
                schema.layout_id
            )));
        }
        if schema.family != family {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "server Binary DB schema family mismatch for {path}: declared={:?}, requested={family:?}",
                schema.family
            )));
        }
        Ok(())
    }

    pub fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        candidate: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        write.ensure_authorized_family(index.family())?;
        if index.fixed_key_size().is_some()
            && crate::foundation::server_binary_db_schema_registry::server_binary_db_index_schema(
                index.as_str(),
            )
            .is_some()
        {
            return self.merge_fixed_index_candidates(&index, &[(key.to_vec(), candidate)]);
        }
        let path = self.resolve_index_path(&index)?;
        let metadata_len = self.files.metadata_len(&path)?;
        let append_offset = if let Some(size) = metadata_len {
            if size < Self::HEADER_SIZE {
                return Err(BinaryDbError::corruption(
                    "index file is missing binary layout header",
                ));
            }
            let layout_id = self.read_layout_id_from_path(&path)?;
            Self::validate_layout(layout_id, index.layout_id(), "index", index.as_str())?;
            size
        } else {
            let layout_id = index.layout_id();
            let header_offset = self.files.append_bytes(&path, &layout_id.to_le_bytes())?;
            if header_offset != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "index layout header append offset changed for {}: expected 0, got {header_offset}",
                    path.display()
                )));
            }
            Self::HEADER_SIZE
        };

        if let Some(key_size) = index.fixed_key_size() {
            let entry_size = u64::from(key_size)
                .checked_add(4)
                .ok_or_else(|| BinaryDbError::corruption("fixed index entry size overflow"))?;
            if (append_offset - Self::HEADER_SIZE) % entry_size != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "fixed index '{}' length is not aligned to {entry_size}-byte entries",
                    index.as_str()
                )));
            }
        }

        let entry = Self::build_index_entry_for(&index, key, candidate)?;
        let actual_offset = self.files.append_bytes(&path, &entry)?;
        if actual_offset != append_offset {
            return Err(BinaryDbError::corruption(format!(
                "index candidate append offset changed for {}: expected {append_offset}, got {actual_offset}",
                path.display()
            )));
        }
        Ok(())
    }

    pub fn append_index_candidates(
        &self,
        index: BinaryIndexId,
        candidates: &[(Vec<u8>, u32)],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        if candidates.is_empty() {
            return Ok(());
        }
        write.ensure_authorized_family(index.family())?;
        if index.fixed_key_size().is_some()
            && crate::foundation::server_binary_db_schema_registry::server_binary_db_index_schema(
                index.as_str(),
            )
            .is_some()
        {
            return self.merge_fixed_index_candidates(&index, candidates);
        }
        let path = self.resolve_index_path(&index)?;
        let metadata_len = self.files.metadata_len(&path)?;
        let append_offset = if let Some(size) = metadata_len {
            if size < Self::HEADER_SIZE {
                return Err(BinaryDbError::corruption(
                    "index file is missing binary layout header",
                ));
            }
            let layout_id = self.read_layout_id_from_path(&path)?;
            Self::validate_layout(layout_id, index.layout_id(), "index", index.as_str())?;
            size
        } else {
            let layout_id = index.layout_id();
            let header_offset = self.files.append_bytes(&path, &layout_id.to_le_bytes())?;
            if header_offset != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "index layout header append offset changed for {}: expected 0, got {header_offset}",
                    path.display()
                )));
            }
            Self::HEADER_SIZE
        };

        if let Some(key_size) = index.fixed_key_size() {
            let entry_size = u64::from(key_size)
                .checked_add(4)
                .ok_or_else(|| BinaryDbError::corruption("fixed index entry size overflow"))?;
            if (append_offset - Self::HEADER_SIZE) % entry_size != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "fixed index '{}' length is not aligned to {entry_size}-byte entries",
                    index.as_str()
                )));
            }
        }

        let mut entries = Vec::new();
        for (key, candidate) in candidates {
            entries.extend_from_slice(&Self::build_index_entry_for(&index, key, *candidate)?);
        }
        let actual_offset = self.files.append_bytes(&path, &entries)?;
        if actual_offset != append_offset {
            return Err(BinaryDbError::corruption(format!(
                "index candidate batch append offset changed for {}: expected {append_offset}, got {actual_offset}",
                path.display()
            )));
        }
        Ok(())
    }

    fn merge_fixed_index_candidates(
        &self,
        index: &BinaryIndexId,
        candidates: &[(Vec<u8>, u32)],
    ) -> StoreResult<()> {
        let fixed_key_size = usize::try_from(index.fixed_key_size().ok_or_else(|| {
            BinaryDbError::invalid_domain_data("fixed index merge requires a fixed key size")
        })?)
        .map_err(|_| BinaryDbError::invalid_domain_data("fixed index key size overflow"))?;
        let entry_size = fixed_key_size
            .checked_add(4)
            .ok_or_else(|| BinaryDbError::corruption("fixed index entry size overflow"))?;
        let path = self.resolve_index_path(index)?;
        let existing = match self.files.metadata_len(&path)? {
            Some(size) => {
                if size < Self::HEADER_SIZE {
                    return Err(BinaryDbError::corruption(
                        "index file is missing binary layout header",
                    ));
                }
                let bytes = self.files.read_bytes(&path)?;
                if u64::try_from(bytes.len()).ok() != Some(size) {
                    return Err(BinaryDbError::corruption(format!(
                        "fixed index '{}' changed during read",
                        index.as_str()
                    )));
                }
                bytes
            }
            None => {
                let header = index.layout_id().to_le_bytes();
                let offset = self.files.append_bytes(&path, &header)?;
                if offset != 0 {
                    return Err(BinaryDbError::corruption(format!(
                        "index layout header append offset changed for {}: expected 0, got {offset}",
                        path.display()
                    )));
                }
                header.to_vec()
            }
        };
        let persisted_layout = u32::from_le_bytes(
            existing[..usize::try_from(Self::HEADER_SIZE).expect("header size fits usize")]
                .try_into()
                .expect("index header is four bytes"),
        );
        Self::validate_layout(persisted_layout, index.layout_id(), "index", index.as_str())?;
        let body = &existing[usize::try_from(Self::HEADER_SIZE).expect("header size fits usize")..];
        if body.len() % entry_size != 0 {
            return Err(BinaryDbError::corruption(format!(
                "fixed index '{}' length is not aligned to {entry_size}-byte entries",
                index.as_str()
            )));
        }
        let mut rows =
            Vec::<(Vec<u8>, u32)>::with_capacity(body.len() / entry_size + candidates.len());
        for entry in body.chunks_exact(entry_size) {
            let raw = u32::from_le_bytes(
                entry[fixed_key_size..]
                    .try_into()
                    .expect("fixed index target is four bytes"),
            );
            let candidate = if index.stores_record_index_plus_one() {
                raw.checked_sub(1).ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "fixed index '{}' contains zero index-plus-one",
                        index.as_str()
                    ))
                })?
            } else {
                raw
            };
            let row = (entry[..fixed_key_size].to_vec(), candidate);
            if rows.last().is_some_and(|previous| previous >= &row) {
                return Err(BinaryDbError::corruption(format!(
                    "fixed index '{}' is not strictly sorted by key and target",
                    index.as_str()
                )));
            }
            rows.push(row);
        }
        for (key, candidate) in candidates {
            Self::build_index_entry_for(index, key, *candidate)?;
            rows.push((key.clone(), *candidate));
        }
        rows.sort();
        rows.dedup();
        let mut encoded = Vec::with_capacity(Self::HEADER_SIZE as usize + rows.len() * entry_size);
        encoded.extend_from_slice(&index.layout_id().to_le_bytes());
        for (key, candidate) in rows {
            encoded.extend_from_slice(&Self::build_index_entry_for(index, &key, candidate)?);
        }
        if encoded == existing {
            return Ok(());
        }
        let suffix = encoded.get(existing.len()..).ok_or_else(|| {
            BinaryDbError::corruption(format!(
                "fixed index '{}' merge unexpectedly shrank authority",
                index.as_str()
            ))
        })?;
        if !suffix.is_empty() {
            let offset = self.files.append_bytes(&path, suffix)?;
            if offset != existing.len() as u64 {
                return Err(BinaryDbError::corruption(format!(
                    "fixed index '{}' append offset changed: expected {}, got {offset}",
                    index.as_str(),
                    existing.len()
                )));
            }
        }
        if !encoded.starts_with(&existing) {
            self.files.overwrite_range(&path, 0, &encoded)?;
        }
        Ok(())
    }

    pub fn build_index_entry(key: BinaryIndexKeyRef<'_>, candidate: u32) -> StoreResult<Vec<u8>> {
        let key_len = u32::try_from(key.len())
            .map_err(|_| format!("index key length exceeds u32::MAX: {}", key.len()))?;
        let mut out = Vec::with_capacity(8 + key.len());
        out.extend_from_slice(&key_len.to_le_bytes());
        out.extend_from_slice(key);
        out.extend_from_slice(&candidate.to_le_bytes());
        Ok(out)
    }

    fn build_index_entry_for(
        index: &BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        candidate: u32,
    ) -> StoreResult<Vec<u8>> {
        let Some(fixed_key_size) = index.fixed_key_size() else {
            return Self::build_index_entry(key, candidate);
        };
        let fixed_key_size = usize::try_from(fixed_key_size)
            .map_err(|_| BinaryDbError::invalid_domain_data("fixed index key size overflow"))?;
        if key.len() != fixed_key_size {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "fixed index '{}' requires {fixed_key_size} key bytes, got {}",
                index.as_str(),
                key.len()
            )));
        }
        let stored_candidate = if index.stores_record_index_plus_one() {
            candidate.checked_add(1).ok_or_else(|| {
                BinaryDbError::invalid_domain_data("fixed index record index overflow")
            })?
        } else {
            candidate
        };
        let mut out = Vec::with_capacity(fixed_key_size + 4);
        out.extend_from_slice(key);
        out.extend_from_slice(&stored_candidate.to_le_bytes());
        Ok(out)
    }

    fn validate_file_name(raw: &str) -> StoreResult<()> {
        if raw.is_empty() {
            return Err(BinaryDbError::invalid_domain_data(
                "record/ index/ payload name is empty",
            ));
        }
        let path = Path::new(raw);
        Self::validate_relative_path(path)?;
        Ok(())
    }

    fn validate_relative_path(path: &Path) -> StoreResult<()> {
        if path.is_absolute() {
            return Err(BinaryDbError::invalid_domain_data(
                "absolute paths are not allowed",
            ));
        }
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(BinaryDbError::invalid_domain_data(
                "parent traversal in file path is not allowed",
            ));
        }
        Ok(())
    }

    fn resolve_relative_path(root: &Path, relative: &str) -> StoreResult<PathBuf> {
        Self::validate_file_name(relative)?;
        let joined = root.join(relative);
        Ok(joined)
    }

    fn ensure_parent_dir(&self, path: &Path) -> StoreResult<()> {
        self.files.create_parent_dirs(path)
    }

    fn read_layout_id_from_path(&self, path: &Path) -> StoreResult<u32> {
        let size = self.files.metadata_len(path)?.ok_or_else(|| {
            BinaryDbError::missing_data(format!("Binary DB file '{}' is missing", path.display()))
        })?;
        if size < Self::HEADER_SIZE {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB file '{}' has a truncated layout header: expected {} bytes, got {size}",
                path.display(),
                Self::HEADER_SIZE
            )));
        }
        let header = self.files.read_range(
            path,
            0,
            Self::HEADER_SIZE.try_into().expect("header fits u32"),
        )?;
        Ok(u32::from_le_bytes(header.as_slice().try_into().map_err(
            |_| BinaryDbError::corruption("invalid Binary DB layout header"),
        )?))
    }

    fn validate_layout(
        actual_layout_id: u32,
        expected_layout_id: u32,
        kind: &str,
        file_name: &str,
    ) -> StoreResult<()> {
        if actual_layout_id == expected_layout_id {
            Ok(())
        } else {
            Err(BinaryDbError::layout_mismatch(format!(
                "layout mismatch for {kind} '{file_name}': existing={actual_layout_id}, expected={expected_layout_id}"
            )))
        }
    }

    fn fixed_record_size(&self, file: &BinaryFileId, layout_id: u32) -> StoreResult<usize> {
        if layout_id != file.layout_id() {
            return Err(BinaryDbError::layout_mismatch(format!(
                "layout mismatch for file '{}': existing={layout_id}, expected={}",
                file.as_str(),
                file.layout_id()
            )));
        }
        let record_size = usize::try_from(file.record_size())
            .map_err(|_| BinaryDbError::invalid_domain_data("record size exceeds usize"))?;
        if record_size == 0 {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "record size for '{}' must be greater than zero",
                file.as_str()
            )));
        }
        Ok(record_size)
    }

    fn ensure_record_file_layout(
        &self,
        file: &BinaryFileId,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32> {
        write.ensure_active()?;
        self.fixed_record_size(file, file.layout_id())?;
        Ok(file.layout_id())
    }

    fn read_record_bytes(
        &self,
        file: &BinaryFileId,
        record_index: u32,
    ) -> StoreResult<BinaryRecordBytes> {
        Ok(self
            .read_record_range_bytes(file, record_index, 1)?
            .pop()
            .expect("one requested record produces one result"))
    }

    fn read_record_range_bytes(
        &self,
        file: &BinaryFileId,
        first_record_index: u32,
        record_count: u32,
    ) -> StoreResult<Vec<BinaryRecordBytes>> {
        if record_count == 0 {
            return Ok(Vec::new());
        }
        let path = self.resolve_record_path(file)?;
        let size = self.files.metadata_len(&path)?.ok_or_else(|| {
            BinaryDbError::missing_data(format!("record file '{}' is missing", file.as_str()))
        })?;
        if size < Self::HEADER_SIZE {
            return Err(BinaryDbError::corruption(
                "record file is corrupted: missing binary header",
            ));
        }
        let header = self.files.read_range(
            &path,
            0,
            Self::HEADER_SIZE.try_into().expect("header fits u32"),
        )?;
        let layout_id = u32::from_le_bytes(
            header
                .as_slice()
                .try_into()
                .map_err(|_| BinaryDbError::corruption("invalid Binary DB layout header"))?,
        );
        let record_size = self.fixed_record_size(file, layout_id)?;
        let payload_size = size - Self::HEADER_SIZE;
        if payload_size % (record_size as u64) != 0 {
            return Err(BinaryDbError::corruption(format!(
                "record file '{}' has invalid body length for layout {}",
                file.as_str(),
                layout_id
            )));
        }
        let available_count = payload_size / (record_size as u64);
        let end_record_index = first_record_index
            .checked_add(record_count)
            .ok_or_else(|| BinaryDbError::invalid_domain_data("record range index overflow"))?;
        if u64::from(end_record_index) > available_count {
            return Err(BinaryDbError::missing_data(format!(
                "record range {first_record_index}..{end_record_index} out of bounds for file '{}'",
                file.as_str()
            )));
        }
        let offset = Self::HEADER_SIZE
            .checked_add(u64::from(first_record_index) * (record_size as u64))
            .ok_or_else(|| BinaryDbError::invalid_domain_data("record range offset overflow"))?;
        let byte_len = record_size
            .checked_mul(
                usize::try_from(record_count).map_err(|_| {
                    BinaryDbError::invalid_domain_data("record count exceeds usize")
                })?,
            )
            .ok_or_else(|| BinaryDbError::invalid_domain_data("record range length overflow"))?;
        let bytes = self.files.read_range(
            &path,
            offset,
            u32::try_from(byte_len).map_err(|_| {
                BinaryDbError::invalid_domain_data("record range length exceeds u32")
            })?,
        )?;
        if bytes.len() != byte_len {
            return Err(BinaryDbError::corruption(format!(
                "record file '{}' changed during range read",
                file.as_str()
            )));
        }
        Ok(bytes
            .chunks_exact(record_size)
            .map(<[u8]>::to_vec)
            .collect())
    }
}

impl<S> BinaryDbJournalIo for FilesystemServerRemoteBinaryDb<S>
where
    S: ServerBinaryDbStore,
{
    fn journal_append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        self.files.append_bytes(path, bytes)
    }

    fn journal_overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> StoreResult<()> {
        self.files.overwrite_range(path, offset, bytes)
    }

    fn journal_truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        self.files.truncate_file(path, len)
    }

    fn journal_remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        self.files.remove_file_if_exists(path)
    }
}

impl<S> BinaryDb for FilesystemServerRemoteBinaryDb<S>
where
    S: ServerBinaryDbStore,
{
    fn authority_root(&self) -> &StorePath {
        &self.authority_root
    }

    fn acquire_in_process_write_admission(
        &self,
        command_scope: BinaryDbCommandScope,
        max_wait: Option<Duration>,
    ) -> StoreResult<Option<BinaryDbWriterAdmissionGuard>> {
        self.writer_admission
            .acquire(command_scope, max_wait)
            .map(Some)
    }

    fn acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::acquire_with_store(&self.files, &self.authority_root, command_scope)
    }

    fn try_acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::try_acquire_with_store(
            &self.files,
            &self.authority_root,
            command_scope,
        )
    }

    fn try_acquire_command_scope_union(
        &self,
        command_scope: BinaryDbCommandScope,
        scopes: &[BinaryDbCommandScope],
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::try_acquire_scope_union_with_store(
            &self.files,
            &self.authority_root,
            command_scope,
            scopes,
        )
    }

    fn try_acquire_recovery_admission_lock(&self) -> StoreResult<BinaryDbRecoveryAdmissionLock> {
        BinaryDbRecoveryAdmissionLock::try_acquire_with_store(&self.files, &self.authority_root)
    }

    fn acquire_queued_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
        max_wait: Duration,
        retry_interval: Duration,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::acquire_queued_with_store(
            &self.files,
            &self.authority_root,
            command_scope,
            max_wait,
            retry_interval,
        )
    }

    fn acquire_read_lock(&self) -> StoreResult<BinaryDbReadLockSet> {
        BinaryDbReadLockSet::try_acquire_with_store(&self.files, &self.authority_root)
    }

    fn acquire_read_lock_for_scope(
        &self,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<BinaryDbReadLockSet> {
        BinaryDbReadLockSet::try_acquire_for_scope_with_store(
            &self.files,
            &self.authority_root,
            read_scope,
        )
    }

    fn path_exists(&self, path: &Path) -> bool {
        self.files.path_exists(path)
    }

    fn read_to_string(&self, path: &Path) -> StoreResult<String> {
        self.files.read_to_string(path)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        self.files.metadata_len(path)
    }

    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.files.sync_file(path)
    }

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        self.files.sync_file_data(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.files.sync_directory(path)
    }

    fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32> {
        let path = self.resolve_record_path(&file)?;
        let size = self.files.metadata_len(&path)?.ok_or_else(|| {
            BinaryDbError::missing_data(format!("record file '{}' is missing", file.as_str()))
        })?;
        if size == 0 {
            return Err(BinaryDbError::corruption(format!(
                "record file '{}' is empty",
                file.as_str()
            )));
        }
        self.read_layout_id_from_path(&path)
    }

    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        let path = self.resolve_record_path(&file)?;
        let Some(size) = self.files.metadata_len(&path)? else {
            return Ok(0);
        };
        if size < Self::HEADER_SIZE {
            return Err(BinaryDbError::corruption(
                "record file is corrupted: missing binary header",
            ));
        }
        let layout_id = self.read_layout_id_from_path(&path)?;
        let record_size = self.fixed_record_size(&file, layout_id)?;
        let body_size = size - Self::HEADER_SIZE;
        if body_size % (record_size as u64) != 0 {
            return Err(BinaryDbError::corruption(format!(
                "record file '{}' has invalid body length for layout {}",
                file.as_str(),
                layout_id
            )));
        }
        Ok((body_size / (record_size as u64))
            .try_into()
            .map_err(|_| "record count does not fit u32".to_string())?)
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<BinaryRecordBytes> {
        self.read_record_bytes(&file, record_index)
    }

    fn read_records(
        &self,
        file: BinaryFileId,
        first_record_index: u32,
        record_count: u32,
    ) -> StoreResult<Vec<BinaryRecordBytes>> {
        self.read_record_range_bytes(&file, first_record_index, record_count)
    }

    fn append_record(
        &self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32> {
        write.ensure_authorized_family(file.family())?;
        let path = self.resolve_record_path(&file)?;
        self.ensure_parent_dir(&path)?;
        self.ensure_record_file_layout(&file, write)?;
        let layout_id = file.layout_id();
        let record_size = self.fixed_record_size(&file, layout_id)?;
        if record.len() != record_size {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "record body length {} does not match compact-v1 {} layout {} for '{}'",
                record.len(),
                record_size,
                layout_id,
                file.as_str()
            )));
        }
        let existing_size = self.files.metadata_len(&path)?;
        let append_offset = if let Some(existing_size) = existing_size {
            if existing_size < Self::HEADER_SIZE {
                return Err(BinaryDbError::corruption(format!(
                    "record file '{}' is corrupted: missing header",
                    file.as_str()
                )));
            }
            let existing_layout = self.read_layout_id_from_path(&path)?;
            if existing_layout != layout_id {
                return Err(BinaryDbError::layout_mismatch(format!(
                    "layout mismatch for file '{}': existing={existing_layout}, expected={layout_id}",
                    file.as_str()
                )));
            }
            let body_size = existing_size
                .checked_sub(Self::HEADER_SIZE)
                .ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "record file '{}' is corrupted: missing header",
                        file.as_str()
                    ))
                })?;
            if body_size % (record_size as u64) != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "record file '{}' has misaligned body length {body_size} for {record_size}-byte records in layout {layout_id}",
                    file.as_str()
                )));
            }
            existing_size
        } else {
            let header_offset = self.files.append_bytes(&path, &layout_id.to_le_bytes())?;
            if header_offset != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "record layout header append offset changed for {}: expected 0, got {header_offset}",
                    path.display()
                )));
            }
            Self::HEADER_SIZE
        };
        let index = append_offset
            .checked_sub(Self::HEADER_SIZE)
            .map(|payload_size| payload_size / (record_size as u64))
            .ok_or_else(|| "invalid record file state".to_string())?;
        let actual_offset = self.files.append_bytes(&path, record)?;
        if actual_offset != append_offset {
            return Err(BinaryDbError::corruption(format!(
                "record append offset changed for {}: expected {append_offset}, got {actual_offset}",
                path.display()
            )));
        }
        Ok(u32::try_from(index).map_err(|_| "record index overflow".to_string())?)
    }

    fn append_records(
        &self,
        file: BinaryFileId,
        records: &[BinaryRecordBytes],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<Vec<u32>> {
        if records.is_empty() {
            return Ok(Vec::new());
        }
        write.ensure_authorized_family(file.family())?;
        let path = self.resolve_record_path(&file)?;
        self.ensure_parent_dir(&path)?;
        self.ensure_record_file_layout(&file, write)?;
        let layout_id = file.layout_id();
        let record_size = self.fixed_record_size(&file, layout_id)?;
        if let Some(record) = records.iter().find(|record| record.len() != record_size) {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "record body length {} does not match compact-v1 {} layout {} for '{}'",
                record.len(),
                record_size,
                layout_id,
                file.as_str()
            )));
        }
        let existing_size = self.files.metadata_len(&path)?;
        let append_offset = if let Some(existing_size) = existing_size {
            if existing_size < Self::HEADER_SIZE {
                return Err(BinaryDbError::corruption(format!(
                    "record file '{}' is corrupted: missing header",
                    file.as_str()
                )));
            }
            let existing_layout = self.read_layout_id_from_path(&path)?;
            if existing_layout != layout_id {
                return Err(BinaryDbError::layout_mismatch(format!(
                    "layout mismatch for file '{}': existing={existing_layout}, expected={layout_id}",
                    file.as_str()
                )));
            }
            let body_size = existing_size
                .checked_sub(Self::HEADER_SIZE)
                .ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "record file '{}' is corrupted: missing header",
                        file.as_str()
                    ))
                })?;
            if body_size % (record_size as u64) != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "record file '{}' has misaligned body length {body_size} for {record_size}-byte records in layout {layout_id}",
                    file.as_str()
                )));
            }
            existing_size
        } else {
            let header_offset = self.files.append_bytes(&path, &layout_id.to_le_bytes())?;
            if header_offset != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "record layout header append offset changed for {}: expected 0, got {header_offset}",
                    path.display()
                )));
            }
            Self::HEADER_SIZE
        };
        let first_index = append_offset
            .checked_sub(Self::HEADER_SIZE)
            .map(|payload_size| payload_size / (record_size as u64))
            .ok_or_else(|| "invalid record file state".to_string())?;
        let record_count = u64::try_from(records.len())
            .map_err(|_| "record batch count exceeds u64".to_string())?;
        let end_index = first_index
            .checked_add(record_count)
            .ok_or_else(|| "record batch index overflow".to_string())?;
        if end_index > u64::from(u32::MAX) + 1 {
            return Err("record batch index exceeds u32".into());
        }
        let encoded_len = record_size
            .checked_mul(records.len())
            .ok_or_else(|| "record batch byte length overflow".to_string())?;
        let mut encoded = Vec::with_capacity(encoded_len);
        for record in records {
            encoded.extend_from_slice(record);
        }
        let actual_offset = self.files.append_bytes(&path, &encoded)?;
        if actual_offset != append_offset {
            return Err(BinaryDbError::corruption(format!(
                "record batch append offset changed for {}: expected {append_offset}, got {actual_offset}",
                path.display()
            )));
        }
        (first_index..end_index)
            .map(|index| u32::try_from(index).map_err(|_| "record index overflow".into()))
            .collect()
    }

    fn overwrite_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        write.ensure_authorized_family(file.family())?;
        let record_size = usize::try_from(file.record_size())
            .map_err(|_| BinaryDbError::invalid_domain_data("record size does not fit usize"))?;
        if record.len() != record_size {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "record body length {} does not match configured size {} for '{}'",
                record.len(),
                record_size,
                file.as_str()
            )));
        }
        let count = self.record_count(file.clone())?;
        if record_index >= count {
            return Err(BinaryDbError::missing_data(format!(
                "record index {record_index} is out of range for '{}'",
                file.as_str()
            )));
        }
        let path = self.resolve_record_path(&file)?;
        let offset = Self::HEADER_SIZE
            .checked_add(u64::from(record_index) * u64::from(file.record_size()))
            .ok_or_else(|| BinaryDbError::invalid_domain_data("record offset overflow"))?;
        self.files.overwrite_range(&path, offset, record)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        let path = self.resolve_payload_path(&file)?;
        let file_len = self.files.metadata_len(&path)?.ok_or_else(|| {
            BinaryDbError::missing_data(format!("payload file '{}' is missing", file.as_str()))
        })?;
        if file_len < Self::HEADER_SIZE {
            return Err(BinaryDbError::corruption(format!(
                "payload file '{}' is corrupted: missing binary header",
                file.as_str()
            )));
        }
        let layout_id = self.read_layout_id_from_path(&path)?;
        Self::validate_layout(layout_id, file.layout_id(), "payload", file.as_str())?;
        if offset < Self::HEADER_SIZE {
            return Err(BinaryDbError::invalid_domain_data(
                "payload offset must point after layout header",
            ));
        }
        let read_len = u64::from(len);
        if offset
            .checked_add(read_len)
            .ok_or_else(|| "payload read range overflows".to_string())?
            > file_len
        {
            return Err(BinaryDbError::missing_data(
                "payload read range is out of bounds",
            ));
        }
        self.files.read_range(&path, offset, len)
    }

    fn append_payload(
        &self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<PayloadRange> {
        write.ensure_authorized_family(file.family())?;
        let path = self.resolve_payload_path(&file)?;
        let append_offset = if let Some(size) = self.files.metadata_len(&path)? {
            if size >= Self::HEADER_SIZE {
                let layout_id = self.read_layout_id_from_path(&path)?;
                Self::validate_layout(layout_id, file.layout_id(), "payload", file.as_str())?;
                size
            } else if size == 0 {
                let header_offset = self
                    .files
                    .append_bytes(&path, &file.layout_id().to_le_bytes())?;
                if header_offset != 0 {
                    return Err(BinaryDbError::corruption(format!(
                        "payload layout header append offset changed for {}: expected 0, got {header_offset}",
                        path.display()
                    )));
                }
                Self::HEADER_SIZE
            } else {
                return Err(BinaryDbError::corruption(format!(
                    "payload file '{}' is corrupted: header truncated",
                    file.as_str()
                )));
            }
        } else {
            let header_offset = self
                .files
                .append_bytes(&path, &file.layout_id().to_le_bytes())?;
            if header_offset != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "payload layout header append offset changed for {}: expected 0, got {header_offset}",
                    path.display()
                )));
            }
            Self::HEADER_SIZE
        };
        let _data_len = u64::try_from(bytes.len())
            .map_err(|_| "payload bytes length exceeds u64".to_string())?;
        let actual_offset = self.files.append_bytes(&path, bytes)?;
        if actual_offset != append_offset {
            return Err(BinaryDbError::corruption(format!(
                "payload append offset changed for {}: expected {append_offset}, got {actual_offset}",
                path.display()
            )));
        }
        Ok(PayloadRange {
            payload_offset: append_offset,
            payload_len: u32::try_from(bytes.len())
                .map_err(|_| "payload length exceeds u32".to_string())?,
        })
    }

    fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        let mut matches = self.lookup_index_many(index, &[key])?;
        Ok(matches
            .pop()
            .expect("one lookup key produces one aligned result"))
    }

    fn lookup_index_many(
        &self,
        index: BinaryIndexId,
        keys: &[BinaryIndexKeyRef<'_>],
    ) -> StoreResult<Vec<Vec<u32>>> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let path = self.resolve_index_path(&index)?;
        let Some(file_len) = self.files.metadata_len(&path)? else {
            return Ok(vec![Vec::new(); keys.len()]);
        };
        if file_len < Self::HEADER_SIZE {
            return Err(BinaryDbError::corruption(
                "index file is corrupted: missing binary header",
            ));
        }
        let bytes = self.files.read_bytes(&path)?;
        let file_len_usize = usize::try_from(file_len)
            .map_err(|_| BinaryDbError::corruption("index file length exceeds usize"))?;
        if bytes.len() != file_len_usize {
            return Err(BinaryDbError::corruption(format!(
                "index file '{}' changed during read",
                index.as_str()
            )));
        }
        let layout_id = u32::from_le_bytes(
            bytes[..usize::try_from(Self::HEADER_SIZE).expect("header size fits usize")]
                .try_into()
                .expect("index header is four bytes"),
        );
        Self::validate_layout(layout_id, index.layout_id(), "index", index.as_str())?;

        let mut positions_by_key = BTreeMap::<Vec<u8>, Vec<usize>>::new();
        for (position, key) in keys.iter().enumerate() {
            positions_by_key
                .entry((*key).to_vec())
                .or_default()
                .push(position);
        }
        let mut out = vec![Vec::new(); keys.len()];
        if let Some(fixed_key_size) = index.fixed_key_size() {
            let fixed_key_size = usize::try_from(fixed_key_size).map_err(|_| {
                BinaryDbError::corruption("fixed index key size does not fit usize")
            })?;
            for key in keys {
                if key.len() != fixed_key_size {
                    return Err(BinaryDbError::invalid_domain_data(format!(
                        "fixed index '{}' requires {fixed_key_size} key bytes, got {}",
                        index.as_str(),
                        key.len()
                    )));
                }
            }
            let body =
                &bytes[usize::try_from(Self::HEADER_SIZE).expect("header size fits usize")..];
            let entry_size = fixed_key_size
                .checked_add(4)
                .ok_or_else(|| BinaryDbError::corruption("fixed index entry size overflow"))?;
            if body.len() % entry_size != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "fixed index '{}' is truncated",
                    index.as_str()
                )));
            }
            let require_v0_order =
                crate::foundation::server_binary_db_schema_registry::server_binary_db_index_schema(
                    index.as_str(),
                )
                .is_some();
            let mut previous = None::<(Vec<u8>, u32)>;
            for entry in body.chunks_exact(entry_size) {
                let raw = u32::from_le_bytes(
                    entry[fixed_key_size..]
                        .try_into()
                        .expect("fixed index value is four bytes"),
                );
                let candidate = if index.stores_record_index_plus_one() {
                    raw.checked_sub(1).ok_or_else(|| {
                        BinaryDbError::corruption(format!(
                            "fixed index '{}' contains zero index-plus-one",
                            index.as_str()
                        ))
                    })?
                } else {
                    raw
                };
                let current = (entry[..fixed_key_size].to_vec(), candidate);
                if require_v0_order
                    && previous
                        .as_ref()
                        .is_some_and(|previous| previous >= &current)
                {
                    return Err(BinaryDbError::corruption(format!(
                        "fixed index '{}' is not strictly sorted by key and target",
                        index.as_str()
                    )));
                }
                previous = Some(current);
                let Some(positions) = positions_by_key.get(&entry[..fixed_key_size]) else {
                    continue;
                };
                for position in positions {
                    out[*position].push(candidate);
                }
            }
            return Ok(out);
        }
        let mut cursor = usize::try_from(Self::HEADER_SIZE).expect("header size fits usize");
        while cursor < bytes.len() {
            let key_len_end = cursor
                .checked_add(4)
                .ok_or_else(|| "index cursor overflow".to_string())?;
            if key_len_end > bytes.len() {
                return Err(BinaryDbError::corruption("index file is truncated"));
            }
            let key_len = u32::from_le_bytes(
                bytes[cursor..key_len_end]
                    .try_into()
                    .expect("slice length checked"),
            ) as usize;
            cursor = key_len_end;
            let key_end = cursor
                .checked_add(key_len)
                .ok_or_else(|| "index cursor overflow".to_string())?;
            if key_end > bytes.len() {
                return Err(BinaryDbError::corruption("index file is truncated"));
            }
            let key_bytes = &bytes[cursor..key_end];
            cursor = key_end;
            let value_end = cursor
                .checked_add(4)
                .ok_or_else(|| "index cursor overflow".to_string())?;
            if value_end > bytes.len() {
                return Err(BinaryDbError::corruption("index file is truncated"));
            }
            let candidate = u32::from_le_bytes(
                bytes[cursor..value_end]
                    .try_into()
                    .expect("slice length checked"),
            );
            cursor = value_end;
            if let Some(positions) = positions_by_key.get(key_bytes) {
                for position in positions {
                    out[*position].push(candidate);
                }
            }
        }
        Ok(out)
    }
}

impl<S> BinaryDbIndexAppender for FilesystemServerRemoteBinaryDb<S>
where
    S: ServerBinaryDbStore,
{
    fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        Self::append_index_candidate(self, index, key, record_index, write)
    }

    fn append_index_candidates(
        &self,
        index: BinaryIndexId,
        candidates: &[(Vec<u8>, u32)],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        Self::append_index_candidates(self, index, candidates, write)
    }
}

impl<S> RemoteBinaryDb for FilesystemServerRemoteBinaryDb<S>
where
    S: ServerBinaryDbStore,
{
    fn remote_repo_id(&self) -> &RepoId {
        &self.repo_id
    }

    fn remote_repo_name(&self) -> &RepoName {
        &self.repo_name
    }

    fn remote_authority_root(&self) -> &StorePath {
        &self.authority_root
    }
}

impl<S> ServerRemoteBinaryDb for FilesystemServerRemoteBinaryDb<S>
where
    S: ServerBinaryDbStore,
{
    fn repo_id(&self) -> &RepoId {
        &self.repo_id
    }

    fn repo_name(&self) -> &RepoName {
        &self.repo_name
    }

    fn authority_root(&self) -> &StorePath {
        &self.authority_root
    }

    fn storage_generation(&self) -> StoreGeneration {
        self.storage_generation
    }

    fn authority_mode(&self) -> ServerBinaryDbAuthorityMode {
        self.authority_mode
    }
}
