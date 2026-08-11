use crate::foundation::operational_binary_v0::{
    OperationalNamespaceIndexRecord, OperationalRepositoryPayload, OperationalRepositoryRecord,
    ServerOperationalBinaryV0Codec, OPERATIONAL_BIN_HEADER_SIZE,
    OPERATIONAL_NAMESPACE_INDEX_RECORD_SIZE, OPERATIONAL_REPOSITORY_RECORD_SIZE,
    OPERATIONAL_V0_LAYOUT_ID, REPOSITORY_LIFECYCLE_ACTIVE, REPOSITORY_LIFECYCLE_PURGED,
    REPOSITORY_LIFECYCLE_RETIRING,
};
use crate::foundation::remote_binary_db::{
    BinaryDbError, BoxedServerBinaryDbProcessLockGuard, ServerBinaryDbByteStore,
    ServerBinaryDbDurabilityStore, ServerBinaryDbFilesystemStore, ServerBinaryDbLockMode,
    ServerBinaryDbLockStore, ServerBinaryDbLockWait, StoreResult,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub const FIXED_REPOSITORY_NAMES: [&str; 4] = ["ait-core", "ait-server", "ait-python", "ait-node"];
pub const PROTOTYPE_POLICY_DEFAULT_FLAGS: u8 = 0b1000_0011;
pub const REGISTRY_LOCK_FILE_NAME: &str = "01-registry.lock";

const REPOSITORY_FILE_NAME: &str = "repository.bin";
const REPOSITORY_PAYLOAD_FILE_NAME: &str = "repository_payload.bin";
const REPOSITORY_NAMESPACE_INDEX_FILE_NAME: &str = "repository_namespace.idx";
const REPOSITORY_NAMESPACE_REBUILD_FILE_NAME: &str = ".repository_namespace.idx.rebuild";
const REPOSITORY_REWRITE_FILE_NAME: &str = ".repository.bin.rewrite";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FreshRepositoryOptions {
    pub namespace_ascii: [u8; 2],
    pub policy_flags: u8,
}

impl Default for FreshRepositoryOptions {
    fn default() -> Self {
        Self {
            namespace_ascii: [0, 0],
            policy_flags: PROTOTYPE_POLICY_DEFAULT_FLAGS,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryCreateSpec {
    pub repo_name: String,
    pub namespace_ascii: [u8; 2],
    pub policy_flags: u8,
    pub created_at_s: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationalRepositoryEntry {
    pub repository_index: u32,
    pub record: OperationalRepositoryRecord,
    pub repo_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryRegistryRecoveryReport {
    pub repository_count: u32,
    pub payload_bytes_truncated: u64,
    pub namespace_index_rows: u32,
}

#[derive(Clone, Debug)]
pub struct ServerOperationalRepositoryRegistry {
    global_root: PathBuf,
    repository_authority_parent: PathBuf,
    files: ServerBinaryDbFilesystemStore,
}

impl ServerOperationalRepositoryRegistry {
    pub fn new(
        global_root: impl Into<PathBuf>,
        repository_authority_parent: impl Into<PathBuf>,
    ) -> StoreResult<Self> {
        let global_root = absolute_path(global_root.into())?;
        let repository_authority_parent = absolute_path(repository_authority_parent.into())?;
        if global_root == repository_authority_parent
            || global_root.starts_with(&repository_authority_parent)
            || repository_authority_parent.starts_with(&global_root)
        {
            return Err(invalid(
                "server-global registry root and Repository authority parent must be distinct and non-nested",
            ));
        }
        Ok(Self {
            global_root,
            repository_authority_parent,
            files: ServerBinaryDbFilesystemStore,
        })
    }

    pub fn global_root(&self) -> &Path {
        &self.global_root
    }

    pub fn repository_authority_parent(&self) -> &Path {
        &self.repository_authority_parent
    }

    pub fn initialize_fresh(
        &self,
        created_at_s: u64,
    ) -> StoreResult<Vec<OperationalRepositoryEntry>> {
        self.initialize_fresh_with_options(created_at_s, [FreshRepositoryOptions::default(); 4])
    }

    pub fn initialize_fresh_with_options(
        &self,
        created_at_s: u64,
        options: [FreshRepositoryOptions; 4],
    ) -> StoreResult<Vec<OperationalRepositoryEntry>> {
        if created_at_s == 0 {
            return Err(invalid(
                "fresh Repository registry creation time must be non-zero",
            ));
        }
        let mut configured_namespaces = BTreeSet::new();
        for (repo_name, option) in FIXED_REPOSITORY_NAMES.iter().zip(options) {
            crate::foundation::operational_binary_v0::validate_namespace(option.namespace_ascii)?;
            ServerOperationalBinaryV0Codec::encode_repository_payload(
                &OperationalRepositoryPayload {
                    repo_name: (*repo_name).to_string(),
                },
            )?;
            if option.namespace_ascii != [0, 0]
                && !configured_namespaces.insert(option.namespace_ascii)
            {
                return Err(invalid(
                    "fresh fixed Repositories cannot share a non-empty namespace",
                ));
            }
        }
        self.prepare_roots()?;
        let mut lock = self.acquire_registry_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.remove_rebuild_temps()?;
        let repository_path = self.repository_path();
        let existing_count = if repository_path.exists() {
            let authority = self.read_authority()?;
            self.validate_directory_closure(&authority.entries)?;
            self.validate_fixed_prefix(&authority.entries, created_at_s, &options)?;
            if authority.entries.len() >= FIXED_REPOSITORY_NAMES.len() {
                self.validate_fixed_slots(&authority.entries)?;
                self.rebuild_namespace_index(&authority.entries)?;
                lock.clear_contents_and_flush()?;
                return Ok(authority.entries);
            }
            authority.entries.len()
        } else {
            self.require_fresh_root_without_repository()?;
            let payload_path = self.repository_payload_path();
            if payload_path.exists() {
                let payload = read_regular_file(&payload_path)?;
                validate_header(&payload, REPOSITORY_PAYLOAD_FILE_NAME)?;
                if payload.len() != OPERATIONAL_BIN_HEADER_SIZE as usize {
                    return Err(corrupt(
                        "interrupted fresh Repository payload has uncommitted bytes",
                    ));
                }
            } else {
                self.write_new_header_file(&payload_path)?;
            }
            self.write_new_header_file(&repository_path)?;
            0
        };
        for (repository_index, (repo_name, options)) in FIXED_REPOSITORY_NAMES
            .iter()
            .zip(options)
            .enumerate()
            .skip(existing_count)
        {
            let expected_index = u32::try_from(repository_index)
                .map_err(|_| corrupt("fixed Repository index exceeds u32"))?;
            let actual_index = self.append_repository_locked(&RepositoryCreateSpec {
                repo_name: (*repo_name).to_string(),
                namespace_ascii: options.namespace_ascii,
                policy_flags: options.policy_flags,
                created_at_s,
            })?;
            if actual_index != expected_index {
                return Err(corrupt(format!(
                    "fixed Repository {repo_name} committed at index {actual_index}, expected {expected_index}"
                )));
            }
        }
        let authority = self.read_authority()?;
        self.validate_fixed_slots(&authority.entries)?;
        self.validate_directory_closure(&authority.entries)?;
        self.rebuild_namespace_index(&authority.entries)?;
        lock.clear_contents_and_flush()?;
        Ok(authority.entries)
    }

    pub fn append_repository(
        &self,
        spec: &RepositoryCreateSpec,
    ) -> StoreResult<OperationalRepositoryEntry> {
        self.append_repository_with_initializer(spec, |_, _| Ok(()))
    }

    pub fn append_repository_with_initializer<F>(
        &self,
        spec: &RepositoryCreateSpec,
        initializer: F,
    ) -> StoreResult<OperationalRepositoryEntry>
    where
        F: FnOnce(u32, &Path) -> StoreResult<()>,
    {
        self.prepare_existing_roots()?;
        let mut lock = self.acquire_registry_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.remove_rebuild_temps()?;
        let authority = self.read_authority()?;
        self.validate_fixed_slots(&authority.entries)?;
        self.validate_directory_closure(&authority.entries)?;
        if u64::try_from(authority.payload_bytes.len())
            .map_err(|_| corrupt("Repository payload file exceeds u64"))?
            != authority.referenced_payload_end
        {
            return Err(corrupt(
                "Repository payload has an unreferenced tail; run registry recovery before appending",
            ));
        }
        self.validate_new_namespace(&authority.entries, spec.namespace_ascii, None)?;
        let index = self.append_repository_locked_with_initializer(spec, initializer)?;
        let authority = self.read_authority()?;
        self.validate_directory_closure(&authority.entries)?;
        self.rebuild_namespace_index(&authority.entries)?;
        let entry = authority
            .entries
            .get(usize::try_from(index).map_err(|_| corrupt("Repository index exceeds usize"))?)
            .cloned()
            .ok_or_else(|| corrupt("committed Repository index is missing"))?;
        lock.clear_contents_and_flush()?;
        Ok(entry)
    }

    pub fn list(&self) -> StoreResult<Vec<OperationalRepositoryEntry>> {
        self.prepare_existing_roots()?;
        let _lock = self.acquire_registry_lock(ServerBinaryDbLockMode::Shared)?;
        let authority = self.read_authority()?;
        self.validate_fixed_slots(&authority.entries)?;
        self.validate_directory_closure(&authority.entries)?;
        Ok(authority.entries)
    }

    pub fn get(&self, repository_index: u32) -> StoreResult<OperationalRepositoryEntry> {
        self.list()?
            .get(
                usize::try_from(repository_index)
                    .map_err(|_| invalid("Repository index exceeds usize"))?,
            )
            .cloned()
            .ok_or_else(|| invalid(format!("unknown Repository index {repository_index}")))
    }

    pub fn discover_by_name(
        &self,
        repo_name: &str,
    ) -> StoreResult<Vec<OperationalRepositoryEntry>> {
        if repo_name.is_empty() {
            return Err(invalid("Repository discovery name is empty"));
        }
        Ok(self
            .list()?
            .into_iter()
            .filter(|entry| entry.repo_name == repo_name)
            .collect())
    }

    pub fn discover_live_namespace(
        &self,
        namespace_ascii: [u8; 2],
    ) -> StoreResult<Vec<OperationalRepositoryEntry>> {
        crate::foundation::operational_binary_v0::validate_namespace(namespace_ascii)?;
        if namespace_ascii == [0, 0] {
            return Ok(Vec::new());
        }
        Ok(self
            .list()?
            .into_iter()
            .filter(|entry| {
                !entry.record.is_tombstoned()
                    && entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
                    && entry.record.namespace_ascii == namespace_ascii
            })
            .collect())
    }

    pub fn update_live_metadata(
        &self,
        repository_index: u32,
        lifecycle_kind: u8,
        namespace_ascii: [u8; 2],
        policy_flags: u8,
        updated_at_s: u64,
    ) -> StoreResult<OperationalRepositoryEntry> {
        self.prepare_existing_roots()?;
        let mut lock = self.acquire_registry_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.remove_rebuild_temps()?;
        let mut authority = self.read_authority()?;
        let index = usize::try_from(repository_index)
            .map_err(|_| invalid("Repository index exceeds usize"))?;
        let current = authority
            .entries
            .get(index)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown Repository index {repository_index}")))?;
        if current.record.is_tombstoned() {
            return Err(invalid("tombstoned Repository metadata is immutable"));
        }
        if lifecycle_kind == REPOSITORY_LIFECYCLE_PURGED
            && current.record.lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
        {
            return Err(BinaryDbError::unsupported(
                "Repository purge requires the registry-plus-queue coordinator",
            ));
        }
        if current.record.lifecycle_kind == REPOSITORY_LIFECYCLE_PURGED
            && lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
        {
            return Err(invalid(
                "purged Repository cannot return to a live lifecycle",
            ));
        }
        let transition_is_valid = lifecycle_kind == current.record.lifecycle_kind
            || matches!(
                (current.record.lifecycle_kind, lifecycle_kind),
                (REPOSITORY_LIFECYCLE_ACTIVE, REPOSITORY_LIFECYCLE_RETIRING)
                    | (REPOSITORY_LIFECYCLE_RETIRING, REPOSITORY_LIFECYCLE_ACTIVE)
            );
        if !transition_is_valid {
            return Err(invalid("Repository lifecycle transition is invalid"));
        }
        if updated_at_s < current.record.created_at_s {
            return Err(invalid(
                "Repository update time precedes immutable creation time",
            ));
        }
        self.validate_new_namespace(&authority.entries, namespace_ascii, Some(repository_index))?;

        let replacement = OperationalRepositoryRecord {
            lifecycle_kind,
            namespace_ascii,
            policy_flags,
            updated_at_s,
            ..current.record
        };
        self.replace_repository_record_locked(
            &mut authority,
            repository_index,
            index,
            replacement,
        )?;
        self.rebuild_namespace_index(&authority.entries)?;
        lock.clear_contents_and_flush()?;
        Ok(authority.entries[index].clone())
    }

    /// Commits the final `retiring -> purged` lifecycle replacement only after
    /// the caller has durably normalized the Repository-local authority.
    ///
    /// The registry lock remains held while `prepare_authority` runs. This
    /// prevents a second server process from reactivating the Repository or
    /// allocating its namespace between queue normalization and the final
    /// lifecycle commit.
    pub fn coordinate_repository_purge<F>(
        &self,
        repository_index: u32,
        updated_at_s: u64,
        prepare_authority: F,
    ) -> StoreResult<OperationalRepositoryEntry>
    where
        F: FnOnce(&OperationalRepositoryEntry) -> StoreResult<()>,
    {
        self.prepare_existing_roots()?;
        let mut lock = self.acquire_registry_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.remove_rebuild_temps()?;
        let mut authority = self.read_authority()?;
        let index = usize::try_from(repository_index)
            .map_err(|_| invalid("Repository index exceeds usize"))?;
        let current = authority
            .entries
            .get(index)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown Repository index {repository_index}")))?;
        if current.record.is_tombstoned() {
            return Err(invalid("tombstoned Repository metadata is immutable"));
        }
        if current.record.lifecycle_kind != REPOSITORY_LIFECYCLE_RETIRING {
            return Err(invalid("Repository purge requires the retiring lifecycle"));
        }
        if updated_at_s < current.record.updated_at_s {
            return Err(invalid(
                "Repository purge time precedes its retirement transition",
            ));
        }

        prepare_authority(&current)?;

        let replacement = OperationalRepositoryRecord {
            lifecycle_kind: REPOSITORY_LIFECYCLE_PURGED,
            updated_at_s,
            ..current.record
        };
        self.replace_repository_record_locked(
            &mut authority,
            repository_index,
            index,
            replacement,
        )?;
        self.rebuild_namespace_index(&authority.entries)?;
        lock.clear_contents_and_flush()?;
        Ok(authority.entries[index].clone())
    }

    pub fn validate(&self) -> StoreResult<Vec<OperationalRepositoryEntry>> {
        self.prepare_existing_roots()?;
        let _lock = self.acquire_registry_lock(ServerBinaryDbLockMode::Shared)?;
        let authority = self.read_authority()?;
        self.validate_fixed_slots(&authority.entries)?;
        self.validate_directory_closure(&authority.entries)?;
        self.validate_namespace_index_if_present(&authority.entries)?;
        Ok(authority.entries)
    }

    pub fn recover(&self) -> StoreResult<RepositoryRegistryRecoveryReport> {
        self.prepare_existing_roots()?;
        let mut lock = self.acquire_registry_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.remove_rebuild_temps()?;
        let authority = self.read_authority()?;
        self.validate_fixed_slots(&authority.entries)?;
        self.recover_uncommitted_tail_directory(&authority.entries)?;
        self.validate_directory_closure(&authority.entries)?;

        let payload_len = u64::try_from(authority.payload_bytes.len())
            .map_err(|_| corrupt("Repository payload file exceeds u64"))?;
        let trailing = payload_len
            .checked_sub(authority.referenced_payload_end)
            .ok_or_else(|| corrupt("Repository payload end exceeds file length"))?;
        if trailing > 0 {
            self.files.truncate_file(
                &self.repository_payload_path(),
                authority.referenced_payload_end,
            )?;
            self.files.sync_file(&self.repository_payload_path())?;
        }
        let namespace_index_rows = self.rebuild_namespace_index(&authority.entries)?;
        lock.clear_contents_and_flush()?;
        Ok(RepositoryRegistryRecoveryReport {
            repository_count: u32::try_from(authority.entries.len())
                .map_err(|_| corrupt("Repository count exceeds u32"))?,
            payload_bytes_truncated: trailing,
            namespace_index_rows,
        })
    }

    pub fn resolve_authority_directory(&self, repository_index: u32) -> StoreResult<PathBuf> {
        let entry = self.get(repository_index)?;
        if entry.record.is_tombstoned() {
            return Err(invalid(format!(
                "Repository index {repository_index} is tombstoned"
            )));
        }
        let path = self
            .repository_authority_parent
            .join(canonical_repository_directory_name(repository_index));
        require_real_directory(&path)?;
        Ok(path)
    }

    fn prepare_roots(&self) -> StoreResult<()> {
        ensure_real_directory(&self.global_root)?;
        ensure_real_directory(&self.repository_authority_parent)?;
        Ok(())
    }

    fn prepare_existing_roots(&self) -> StoreResult<()> {
        require_real_directory(&self.global_root)?;
        require_real_directory(&self.repository_authority_parent)?;
        Ok(())
    }

    fn repository_path(&self) -> PathBuf {
        self.global_root.join(REPOSITORY_FILE_NAME)
    }

    fn repository_payload_path(&self) -> PathBuf {
        self.global_root.join(REPOSITORY_PAYLOAD_FILE_NAME)
    }

    fn namespace_index_path(&self) -> PathBuf {
        self.global_root.join(REPOSITORY_NAMESPACE_INDEX_FILE_NAME)
    }

    fn lock_path(&self) -> PathBuf {
        self.global_root.join(REGISTRY_LOCK_FILE_NAME)
    }

    fn acquire_registry_lock(
        &self,
        mode: ServerBinaryDbLockMode,
    ) -> StoreResult<BoxedServerBinaryDbProcessLockGuard> {
        let lock_path = self.lock_path();
        reject_symlink_or_hardlink_if_present(&lock_path)?;
        let mut lock = self
            .files
            .acquire_process_lock(&lock_path, mode, ServerBinaryDbLockWait::Blocking)?
            .ok_or_else(|| {
                BinaryDbError::retryable_busy(format!(
                    "Repository registry lock is busy at {}",
                    lock_path.display()
                ))
            })?;
        if matches!(mode, ServerBinaryDbLockMode::Exclusive) {
            lock.replace_contents_and_flush(
                format!("root={}\n", self.global_root.display()).as_bytes(),
            )?;
        }
        Ok(lock)
    }

    fn require_fresh_root_without_repository(&self) -> StoreResult<()> {
        for entry in read_dir(&self.global_root)? {
            let entry = entry.map_err(|error| {
                BinaryDbError::io(
                    format!("read directory entry in {}", self.global_root.display()),
                    error,
                )
            })?;
            let name = entry.file_name();
            if name != OsStr::new(REGISTRY_LOCK_FILE_NAME)
                && name != OsStr::new(REPOSITORY_PAYLOAD_FILE_NAME)
            {
                return Err(invalid(format!(
                    "fresh Repository registry root contains unexpected path {:?}",
                    name
                )));
            }
        }
        if read_dir(&self.repository_authority_parent)?
            .next()
            .is_some()
        {
            return Err(invalid("fresh Repository authority parent is not empty"));
        }
        Ok(())
    }

    fn validate_fixed_prefix(
        &self,
        entries: &[OperationalRepositoryEntry],
        created_at_s: u64,
        options: &[FreshRepositoryOptions; 4],
    ) -> StoreResult<()> {
        if entries.len() > FIXED_REPOSITORY_NAMES.len() {
            return Ok(());
        }
        for (index, entry) in entries.iter().enumerate() {
            let expected = options[index];
            if entry.repository_index != index as u32
                || entry.repo_name != FIXED_REPOSITORY_NAMES[index]
                || entry.record.is_tombstoned()
                || entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_ACTIVE
                || entry.record.namespace_ascii != expected.namespace_ascii
                || entry.record.policy_flags != expected.policy_flags
                || entry.record.created_at_s != created_at_s
                || entry.record.updated_at_s != created_at_s
            {
                return Err(corrupt(format!(
                    "interrupted fixed Repository index {index} does not match its fresh initialization input"
                )));
            }
        }
        Ok(())
    }

    fn write_new_header_file(&self, path: &Path) -> StoreResult<()> {
        reject_symlink_or_hardlink_if_present(path)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| {
                BinaryDbError::io(
                    format!("create operational Binary DB file {}", path.display()),
                    error,
                )
            })?;
        file.write_all(&OPERATIONAL_V0_LAYOUT_ID.to_le_bytes())
            .map_err(|error| {
                BinaryDbError::io(
                    format!("write operational Binary DB header {}", path.display()),
                    error,
                )
            })?;
        file.sync_all().map_err(|error| {
            BinaryDbError::io(
                format!("sync operational Binary DB file {}", path.display()),
                error,
            )
        })?;
        self.files.sync_directory(&self.global_root)
    }

    fn append_repository_locked(&self, spec: &RepositoryCreateSpec) -> StoreResult<u32> {
        self.append_repository_locked_with_initializer(spec, |_, _| Ok(()))
    }

    fn append_repository_locked_with_initializer<F>(
        &self,
        spec: &RepositoryCreateSpec,
        initializer: F,
    ) -> StoreResult<u32>
    where
        F: FnOnce(u32, &Path) -> StoreResult<()>,
    {
        if spec.created_at_s == 0 {
            return Err(invalid("Repository creation time must be non-zero"));
        }
        crate::foundation::operational_binary_v0::validate_namespace(spec.namespace_ascii)?;
        let payload_raw = ServerOperationalBinaryV0Codec::encode_repository_payload(
            &OperationalRepositoryPayload {
                repo_name: spec.repo_name.clone(),
            },
        )?;
        let repository_path = self.repository_path();
        let repository_len = required_file_len(&repository_path)?;
        if repository_len < OPERATIONAL_BIN_HEADER_SIZE
            || (repository_len - OPERATIONAL_BIN_HEADER_SIZE)
                % u64::from(OPERATIONAL_REPOSITORY_RECORD_SIZE)
                != 0
        {
            return Err(corrupt("repository.bin is not record-aligned"));
        }
        let index_u64 = (repository_len - OPERATIONAL_BIN_HEADER_SIZE)
            / u64::from(OPERATIONAL_REPOSITORY_RECORD_SIZE);
        if index_u64 >= u64::from(u32::MAX - 1) {
            return Err(invalid(
                "Repository registry has exhausted v0 index capacity",
            ));
        }
        let repository_index =
            u32::try_from(index_u64).map_err(|_| corrupt("Repository index exceeds u32"))?;
        let authority_directory = self
            .repository_authority_parent
            .join(canonical_repository_directory_name(repository_index));
        if authority_directory.exists() {
            return Err(invalid(format!(
                "Repository authority directory already exists before index {repository_index} commits"
            )));
        }
        fs::create_dir(&authority_directory).map_err(|error| {
            BinaryDbError::io(
                format!(
                    "stage numeric Repository authority directory {}",
                    authority_directory.display()
                ),
                error,
            )
        })?;
        self.files
            .sync_directory(&self.repository_authority_parent)?;
        if let Err(error) = initializer(repository_index, &authority_directory)
            .and_then(|_| self.files.sync_directory(&authority_directory))
            .and_then(|_| self.files.sync_directory(&self.repository_authority_parent))
        {
            return match self.remove_staged_authority_directory(&authority_directory) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(BinaryDbError::other(format!(
                    "{error}; additionally failed to remove uncommitted Repository authority {}: {cleanup_error}",
                    authority_directory.display()
                ))),
            };
        }

        let payload_offset = self.append_and_sync(&self.repository_payload_path(), &payload_raw)?;
        let record = OperationalRepositoryRecord {
            repository_meta: 0,
            lifecycle_kind: REPOSITORY_LIFECYCLE_ACTIVE,
            namespace_ascii: spec.namespace_ascii,
            policy_flags: spec.policy_flags,
            payload_len: u32::try_from(payload_raw.len())
                .map_err(|_| invalid("Repository payload exceeds u32"))?,
            payload_offset,
            created_at_s: spec.created_at_s,
            updated_at_s: spec.created_at_s,
        };
        let record_raw = ServerOperationalBinaryV0Codec::encode_repository(record)?;
        let actual_record_offset = self.append_and_sync(&repository_path, &record_raw)?;
        if actual_record_offset != repository_len {
            return Err(corrupt(format!(
                "Repository append offset changed: expected {repository_len}, got {actual_record_offset}"
            )));
        }
        Ok(repository_index)
    }

    fn remove_staged_authority_directory(&self, path: &Path) -> StoreResult<()> {
        require_real_directory(path)?;
        fs::remove_dir_all(path).map_err(|error| {
            BinaryDbError::io(
                format!(
                    "remove uncommitted Repository authority directory {}",
                    path.display()
                ),
                error,
            )
        })?;
        self.files.sync_directory(&self.repository_authority_parent)
    }

    fn append_and_sync(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        reject_symlink_or_hardlink_if_present(path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(path)
            .map_err(|error| {
                BinaryDbError::io(
                    format!("open operational Binary DB file {}", path.display()),
                    error,
                )
            })?;
        let offset = file.seek(SeekFrom::End(0)).map_err(|error| {
            BinaryDbError::io(
                format!("seek operational Binary DB file {}", path.display()),
                error,
            )
        })?;
        file.write_all(bytes).map_err(|error| {
            BinaryDbError::io(
                format!("append operational Binary DB file {}", path.display()),
                error,
            )
        })?;
        file.sync_all().map_err(|error| {
            BinaryDbError::io(
                format!("sync operational Binary DB file {}", path.display()),
                error,
            )
        })?;
        Ok(offset)
    }

    fn read_authority(&self) -> StoreResult<RegistryAuthority> {
        self.validate_global_root_inventory()?;
        let repository_bytes = read_regular_file(&self.repository_path())?;
        validate_header_and_alignment(
            &repository_bytes,
            OPERATIONAL_REPOSITORY_RECORD_SIZE,
            REPOSITORY_FILE_NAME,
        )?;
        let payload_bytes = read_regular_file(&self.repository_payload_path())?;
        validate_header(&payload_bytes, REPOSITORY_PAYLOAD_FILE_NAME)?;

        let mut entries = Vec::new();
        let mut expected_payload_offset = OPERATIONAL_BIN_HEADER_SIZE;
        for (index, raw) in repository_bytes[4..]
            .chunks_exact(OPERATIONAL_REPOSITORY_RECORD_SIZE as usize)
            .enumerate()
        {
            let record = ServerOperationalBinaryV0Codec::decode_repository(raw)?;
            if record.payload_offset != expected_payload_offset {
                return Err(corrupt(format!(
                    "Repository index {index} payload offset is {}, expected contiguous offset {expected_payload_offset}",
                    record.payload_offset
                )));
            }
            let payload_end = record
                .payload_offset
                .checked_add(u64::from(record.payload_len))
                .ok_or_else(|| corrupt("Repository payload range overflow"))?;
            let raw_payload = payload_bytes
                .get(
                    usize::try_from(record.payload_offset)
                        .map_err(|_| corrupt("Repository payload offset exceeds usize"))?
                        ..usize::try_from(payload_end)
                            .map_err(|_| corrupt("Repository payload end exceeds usize"))?,
                )
                .ok_or_else(|| corrupt(format!("Repository index {index} payload is truncated")))?;
            let payload = ServerOperationalBinaryV0Codec::validate_repository_payload_binding(
                record,
                raw_payload,
            )?;
            entries.push(OperationalRepositoryEntry {
                repository_index: u32::try_from(index)
                    .map_err(|_| corrupt("Repository index exceeds u32"))?,
                record,
                repo_name: payload.repo_name,
            });
            expected_payload_offset = payload_end;
        }
        self.validate_live_namespaces(&entries)?;
        Ok(RegistryAuthority {
            repository_bytes,
            payload_bytes,
            referenced_payload_end: expected_payload_offset,
            entries,
        })
    }

    fn validate_global_root_inventory(&self) -> StoreResult<()> {
        for entry in read_dir(&self.global_root)? {
            let entry = entry.map_err(|error| {
                BinaryDbError::io(
                    format!("read directory entry in {}", self.global_root.display()),
                    error,
                )
            })?;
            let name = entry.file_name();
            let admitted = name == OsStr::new(REPOSITORY_FILE_NAME)
                || name == OsStr::new(REPOSITORY_PAYLOAD_FILE_NAME)
                || name == OsStr::new(REPOSITORY_NAMESPACE_INDEX_FILE_NAME)
                || name == OsStr::new(REGISTRY_LOCK_FILE_NAME);
            if !admitted {
                return Err(invalid(format!(
                    "server-global Repository registry contains undeclared path {:?}",
                    name
                )));
            }
        }
        Ok(())
    }

    fn validate_fixed_slots(&self, entries: &[OperationalRepositoryEntry]) -> StoreResult<()> {
        if entries.len() < FIXED_REPOSITORY_NAMES.len() {
            return Err(corrupt(format!(
                "activated Repository registry has {} records; at least four are required",
                entries.len()
            )));
        }
        for (index, expected_name) in FIXED_REPOSITORY_NAMES.iter().enumerate() {
            let entry = &entries[index];
            if entry.repository_index != index as u32
                || entry.repo_name != *expected_name
                || entry.record.is_tombstoned()
            {
                return Err(corrupt(format!(
                    "fixed Repository index {index} is not exact {expected_name}"
                )));
            }
        }
        Ok(())
    }

    fn validate_live_namespaces(&self, entries: &[OperationalRepositoryEntry]) -> StoreResult<()> {
        let mut live = BTreeMap::new();
        for entry in entries {
            if entry.record.is_tombstoned()
                || entry.record.lifecycle_kind == REPOSITORY_LIFECYCLE_PURGED
                || entry.record.namespace_ascii == [0, 0]
            {
                continue;
            }
            if let Some(previous) =
                live.insert(entry.record.namespace_ascii, entry.repository_index)
            {
                return Err(corrupt(format!(
                    "live Repository indexes {previous} and {} share namespace {:?}",
                    entry.repository_index, entry.record.namespace_ascii
                )));
            }
        }
        Ok(())
    }

    fn validate_new_namespace(
        &self,
        entries: &[OperationalRepositoryEntry],
        namespace_ascii: [u8; 2],
        except_index: Option<u32>,
    ) -> StoreResult<()> {
        crate::foundation::operational_binary_v0::validate_namespace(namespace_ascii)?;
        if namespace_ascii == [0, 0] {
            return Ok(());
        }
        if let Some(conflict) = entries.iter().find(|entry| {
            Some(entry.repository_index) != except_index
                && !entry.record.is_tombstoned()
                && entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
                && entry.record.namespace_ascii == namespace_ascii
        }) {
            return Err(invalid(format!(
                "Repository namespace {:?} is already owned by live index {}",
                namespace_ascii, conflict.repository_index
            )));
        }
        Ok(())
    }

    fn validate_directory_closure(
        &self,
        entries: &[OperationalRepositoryEntry],
    ) -> StoreResult<()> {
        let mut present = BTreeSet::new();
        for entry in read_dir(&self.repository_authority_parent)? {
            let entry = entry.map_err(|error| {
                BinaryDbError::io(
                    format!(
                        "read directory entry in {}",
                        self.repository_authority_parent.display()
                    ),
                    error,
                )
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                BinaryDbError::io(
                    format!("inspect Repository authority path {}", path.display()),
                    error,
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(invalid(format!(
                    "Repository authority path {} is not a real directory",
                    path.display()
                )));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("Repository authority directory name is not UTF-8"))?;
            let repository_index = parse_repository_directory_name(&name)?;
            if usize::try_from(repository_index)
                .map_err(|_| corrupt("Repository directory index exceeds usize"))?
                >= entries.len()
            {
                return Err(corrupt(format!(
                    "Repository authority directory {name} has no committed registry record"
                )));
            }
            if !present.insert(repository_index) {
                return Err(corrupt(format!(
                    "duplicate Repository authority directory for index {repository_index}"
                )));
            }
        }
        for entry in entries {
            if !present.contains(&entry.repository_index) {
                return Err(corrupt(format!(
                    "Repository index {} has no numeric authority directory",
                    entry.repository_index
                )));
            }
        }
        Ok(())
    }

    fn recover_uncommitted_tail_directory(
        &self,
        entries: &[OperationalRepositoryEntry],
    ) -> StoreResult<()> {
        let committed_count =
            u32::try_from(entries.len()).map_err(|_| corrupt("Repository count exceeds u32"))?;
        let mut uncommitted = Vec::new();
        for entry in read_dir(&self.repository_authority_parent)? {
            let entry = entry.map_err(|error| {
                BinaryDbError::io(
                    format!(
                        "read directory entry in {}",
                        self.repository_authority_parent.display()
                    ),
                    error,
                )
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                BinaryDbError::io(
                    format!("inspect Repository authority path {}", path.display()),
                    error,
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(invalid(format!(
                    "Repository authority path {} is not a real directory",
                    path.display()
                )));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| invalid("Repository authority directory name is not UTF-8"))?;
            let repository_index = parse_repository_directory_name(&name)?;
            if repository_index >= committed_count {
                uncommitted.push((repository_index, path));
            }
        }
        match uncommitted.as_slice() {
            [] => Ok(()),
            [(repository_index, path)] if *repository_index == committed_count => {
                self.remove_staged_authority_directory(path)
            }
            _ => Err(corrupt(format!(
                "Repository authority directories beyond the committed tail are not an exact recoverable index {committed_count}"
            ))),
        }
    }

    fn namespace_rows(
        &self,
        entries: &[OperationalRepositoryEntry],
    ) -> StoreResult<Vec<OperationalNamespaceIndexRecord>> {
        let mut rows = entries
            .iter()
            .filter(|entry| {
                !entry.record.is_tombstoned()
                    && entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
                    && entry.record.namespace_ascii != [0, 0]
            })
            .map(|entry| {
                Ok(OperationalNamespaceIndexRecord {
                    namespace_ascii: entry.record.namespace_ascii,
                    reserved0: 0,
                    repository_index_plus1: entry
                        .repository_index
                        .checked_add(1)
                        .ok_or_else(|| corrupt("Repository plus-one index overflow"))?,
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;
        rows.sort_by_key(|row| (row.namespace_ascii, row.repository_index_plus1));
        Ok(rows)
    }

    fn rebuild_namespace_index(&self, entries: &[OperationalRepositoryEntry]) -> StoreResult<u32> {
        let rows = self.namespace_rows(entries)?;
        let mut bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
        for row in &rows {
            bytes.extend_from_slice(&ServerOperationalBinaryV0Codec::encode_namespace_index(
                *row,
            )?);
        }
        self.atomic_replace(
            &self.namespace_index_path(),
            &self
                .global_root
                .join(REPOSITORY_NAMESPACE_REBUILD_FILE_NAME),
            &bytes,
        )?;
        u32::try_from(rows.len()).map_err(|_| corrupt("namespace index row count exceeds u32"))
    }

    fn validate_namespace_index_if_present(
        &self,
        entries: &[OperationalRepositoryEntry],
    ) -> StoreResult<()> {
        let path = self.namespace_index_path();
        if !path.exists() {
            return Ok(());
        }
        let bytes = read_regular_file(&path)?;
        validate_header_and_alignment(
            &bytes,
            OPERATIONAL_NAMESPACE_INDEX_RECORD_SIZE,
            REPOSITORY_NAMESPACE_INDEX_FILE_NAME,
        )?;
        let actual = bytes[4..]
            .chunks_exact(OPERATIONAL_NAMESPACE_INDEX_RECORD_SIZE as usize)
            .map(ServerOperationalBinaryV0Codec::decode_namespace_index)
            .collect::<StoreResult<Vec<_>>>()?;
        if actual != self.namespace_rows(entries)? {
            return Err(corrupt("Repository namespace index is stale"));
        }
        Ok(())
    }

    fn replace_repository_record_locked(
        &self,
        authority: &mut RegistryAuthority,
        repository_index: u32,
        index: usize,
        replacement: OperationalRepositoryRecord,
    ) -> StoreResult<()> {
        let replacement_raw = ServerOperationalBinaryV0Codec::encode_repository(replacement)?;
        let record_offset = OPERATIONAL_BIN_HEADER_SIZE
            .checked_add(
                u64::from(repository_index)
                    .checked_mul(u64::from(OPERATIONAL_REPOSITORY_RECORD_SIZE))
                    .ok_or_else(|| corrupt("Repository record offset overflow"))?,
            )
            .ok_or_else(|| corrupt("Repository record offset overflow"))?;
        let start = usize::try_from(record_offset)
            .map_err(|_| corrupt("Repository offset exceeds usize"))?;
        let end = start
            .checked_add(replacement_raw.len())
            .ok_or_else(|| corrupt("Repository replacement range overflow"))?;
        authority
            .repository_bytes
            .get_mut(start..end)
            .ok_or_else(|| corrupt("Repository replacement range is outside the file"))?
            .copy_from_slice(&replacement_raw);
        self.atomic_replace(
            &self.repository_path(),
            &self.global_root.join(REPOSITORY_REWRITE_FILE_NAME),
            &authority.repository_bytes,
        )?;
        authority.entries[index].record = replacement;
        Ok(())
    }

    fn atomic_replace(&self, target: &Path, temporary: &Path, bytes: &[u8]) -> StoreResult<()> {
        reject_symlink_or_hardlink_if_present(target)?;
        if temporary.exists() {
            reject_symlink_or_hardlink_if_present(temporary)?;
            fs::remove_file(temporary).map_err(|error| {
                BinaryDbError::io(
                    format!("remove stale rebuild file {}", temporary.display()),
                    error,
                )
            })?;
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary)
            .map_err(|error| {
                BinaryDbError::io(
                    format!("create rebuild file {}", temporary.display()),
                    error,
                )
            })?;
        file.write_all(bytes).map_err(|error| {
            BinaryDbError::io(format!("write rebuild file {}", temporary.display()), error)
        })?;
        file.sync_all().map_err(|error| {
            BinaryDbError::io(format!("sync rebuild file {}", temporary.display()), error)
        })?;
        fs::rename(temporary, target).map_err(|error| {
            BinaryDbError::io(
                format!(
                    "activate rebuild file {} as {}",
                    temporary.display(),
                    target.display()
                ),
                error,
            )
        })?;
        self.files.sync_directory(&self.global_root)
    }

    fn remove_rebuild_temps(&self) -> StoreResult<()> {
        for name in [
            REPOSITORY_NAMESPACE_REBUILD_FILE_NAME,
            REPOSITORY_REWRITE_FILE_NAME,
        ] {
            let path = self.global_root.join(name);
            if path.exists() {
                reject_symlink_or_hardlink_if_present(&path)?;
                fs::remove_file(&path).map_err(|error| {
                    BinaryDbError::io(
                        format!("remove interrupted rebuild file {}", path.display()),
                        error,
                    )
                })?;
            }
        }
        Ok(())
    }
}

struct RegistryAuthority {
    repository_bytes: Vec<u8>,
    payload_bytes: Vec<u8>,
    referenced_payload_end: u64,
    entries: Vec<OperationalRepositoryEntry>,
}

pub fn canonical_repository_directory_name(repository_index: u32) -> String {
    repository_index.to_string()
}

pub fn parse_repository_directory_name(name: &str) -> StoreResult<u32> {
    if name.is_empty()
        || name.starts_with('+')
        || name.starts_with('-')
        || name.bytes().any(|byte| !byte.is_ascii_digit())
        || (name.len() > 1 && name.starts_with('0'))
    {
        return Err(invalid(format!(
            "Repository authority directory name {name:?} is not canonical unsigned base-10"
        )));
    }
    let value = name.parse::<u32>().map_err(|_| {
        invalid(format!(
            "Repository authority directory name {name:?} exceeds u32"
        ))
    })?;
    if canonical_repository_directory_name(value) != name {
        return Err(invalid(format!(
            "Repository authority directory name {name:?} is not canonical"
        )));
    }
    Ok(value)
}

fn absolute_path(path: PathBuf) -> StoreResult<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| BinaryDbError::io("resolve current directory", error))
    }
}

fn ensure_real_directory(path: &Path) -> StoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(invalid(format!(
                    "{} is not a real directory",
                    path.display()
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| {
                BinaryDbError::io(format!("create directory {}", path.display()), error)
            })?;
            require_real_directory(path)
        }
        Err(error) => Err(BinaryDbError::io(
            format!("inspect directory {}", path.display()),
            error,
        )),
    }
}

fn require_real_directory(path: &Path) -> StoreResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| BinaryDbError::io(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid(format!(
            "{} is not a real directory",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink_or_hardlink_if_present(path: &Path) -> StoreResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(BinaryDbError::io(
                format!("inspect {}", path.display()),
                error,
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid(format!(
            "{} is not a real regular file",
            path.display()
        )));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(invalid(format!(
            "{} is shared through a hard link",
            path.display()
        )));
    }
    Ok(())
}

fn read_regular_file(path: &Path) -> StoreResult<Vec<u8>> {
    reject_symlink_or_hardlink_if_present(path)?;
    fs::read(path).map_err(|error| BinaryDbError::io(format!("read {}", path.display()), error))
}

fn required_file_len(path: &Path) -> StoreResult<u64> {
    reject_symlink_or_hardlink_if_present(path)?;
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| BinaryDbError::io(format!("inspect {}", path.display()), error))
}

fn validate_header(bytes: &[u8], label: &str) -> StoreResult<()> {
    let header: [u8; 4] = bytes
        .get(..4)
        .ok_or_else(|| corrupt(format!("{label} is missing its layout header")))?
        .try_into()
        .expect("four-byte header");
    let layout_id = u32::from_le_bytes(header);
    if layout_id != OPERATIONAL_V0_LAYOUT_ID {
        return Err(BinaryDbError::layout_mismatch(format!(
            "{label} layout is {layout_id}, expected {OPERATIONAL_V0_LAYOUT_ID}"
        )));
    }
    Ok(())
}

fn validate_header_and_alignment(bytes: &[u8], record_size: u32, label: &str) -> StoreResult<()> {
    validate_header(bytes, label)?;
    let body_len = bytes
        .len()
        .checked_sub(4)
        .ok_or_else(|| corrupt(format!("{label} length underflow")))?;
    if body_len % record_size as usize != 0 {
        return Err(corrupt(format!(
            "{label} has an incomplete trailing {record_size}-byte record"
        )));
    }
    Ok(())
}

fn read_dir(path: &Path) -> StoreResult<fs::ReadDir> {
    fs::read_dir(path)
        .map_err(|error| BinaryDbError::io(format!("read directory {}", path.display()), error))
}

fn invalid(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::invalid_domain_data(message)
}

fn corrupt(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::corruption(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(name: &str) -> (PathBuf, ServerOperationalRepositoryRegistry) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ait-server-operational-registry-{name}-{}-{nonce}",
            std::process::id()
        ));
        let registry =
            ServerOperationalRepositoryRegistry::new(root.join("global"), root.join("repos"))
                .unwrap();
        (root, registry)
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fresh_registry_fixes_zero_through_three_and_numeric_directories() {
        let (root, registry) = fixture("fresh");
        let entries = registry.initialize_fresh(100).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.repository_index, entry.repo_name.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (0, "ait-core"),
                (1, "ait-server"),
                (2, "ait-python"),
                (3, "ait-node"),
            ]
        );
        for index in 0..4 {
            assert!(registry
                .repository_authority_parent()
                .join(index.to_string())
                .is_dir());
        }
        assert_eq!(
            &fs::read(registry.global_root().join(REPOSITORY_FILE_NAME)).unwrap()[..4],
            &1_u32.to_le_bytes()
        );
        assert_eq!(registry.validate().unwrap(), entries);
        cleanup(&root);
    }

    #[test]
    fn duplicate_names_are_discovery_only_and_indexes_remain_distinct() {
        let (root, registry) = fixture("duplicates");
        registry.initialize_fresh(100).unwrap();
        let first = registry
            .append_repository(&RepositoryCreateSpec {
                repo_name: "ait-core".to_string(),
                namespace_ascii: *b"x1",
                policy_flags: 0,
                created_at_s: 101,
            })
            .unwrap();
        let second = registry
            .append_repository(&RepositoryCreateSpec {
                repo_name: "ait-core".to_string(),
                namespace_ascii: *b"x2",
                policy_flags: 0,
                created_at_s: 102,
            })
            .unwrap();
        assert_eq!((first.repository_index, second.repository_index), (4, 5));
        let matches = registry.discover_by_name("ait-core").unwrap();
        assert_eq!(
            matches
                .iter()
                .map(|entry| entry.repository_index)
                .collect::<Vec<_>>(),
            [0, 4, 5]
        );
        assert_eq!(
            registry.resolve_authority_directory(4).unwrap(),
            registry.repository_authority_parent().join("4")
        );
        cleanup(&root);
    }

    #[test]
    fn registry_lock_serializes_concurrent_physical_index_allocation() {
        let (root, registry) = fixture("concurrent");
        registry.initialize_fresh(100).unwrap();
        let first_registry = registry.clone();
        let second_registry = registry.clone();
        let first = std::thread::spawn(move || {
            first_registry
                .append_repository(&RepositoryCreateSpec {
                    repo_name: "same".to_string(),
                    namespace_ascii: *b"a1",
                    policy_flags: 0,
                    created_at_s: 101,
                })
                .unwrap()
        });
        let second = std::thread::spawn(move || {
            second_registry
                .append_repository(&RepositoryCreateSpec {
                    repo_name: "same".to_string(),
                    namespace_ascii: *b"a2",
                    policy_flags: 0,
                    created_at_s: 102,
                })
                .unwrap()
        });
        let mut allocated = [
            first.join().unwrap().repository_index,
            second.join().unwrap().repository_index,
        ];
        allocated.sort();
        assert_eq!(allocated, [4, 5]);
        assert_eq!(registry.validate().unwrap().len(), 6);
        cleanup(&root);
    }

    #[test]
    fn fresh_initialization_resumes_an_exact_committed_fixed_prefix() {
        let (root, registry) = fixture("resume");
        registry.prepare_roots().unwrap();
        {
            let mut lock = registry
                .acquire_registry_lock(ServerBinaryDbLockMode::Exclusive)
                .unwrap();
            registry
                .write_new_header_file(&registry.repository_payload_path())
                .unwrap();
            registry
                .write_new_header_file(&registry.repository_path())
                .unwrap();
            assert_eq!(
                registry
                    .append_repository_locked(&RepositoryCreateSpec {
                        repo_name: "ait-core".to_string(),
                        namespace_ascii: [0, 0],
                        policy_flags: PROTOTYPE_POLICY_DEFAULT_FLAGS,
                        created_at_s: 100,
                    })
                    .unwrap(),
                0
            );
            lock.clear_contents_and_flush().unwrap();
        }

        let entries = registry.initialize_fresh(100).unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].repo_name, "ait-core");
        assert_eq!(entries[3].repo_name, "ait-node");
        cleanup(&root);
    }

    #[test]
    fn namespace_index_is_sorted_rebuildable_and_never_repairs_authority() {
        let (root, registry) = fixture("namespace");
        registry
            .initialize_fresh_with_options(
                100,
                [
                    FreshRepositoryOptions {
                        namespace_ascii: *b"z1",
                        policy_flags: 0,
                    },
                    FreshRepositoryOptions {
                        namespace_ascii: [b'a', 0],
                        policy_flags: 0,
                    },
                    FreshRepositoryOptions::default(),
                    FreshRepositoryOptions::default(),
                ],
            )
            .unwrap();
        let index_path = registry.namespace_index_path();
        fs::write(&index_path, [1, 0, 0, 0, 0xff]).unwrap();
        assert!(registry.validate().is_err());
        let report = registry.recover().unwrap();
        assert_eq!(report.namespace_index_rows, 2);
        assert!(registry.validate().is_ok());

        let repository_path = registry.repository_path();
        let mut repository_bytes = fs::read(&repository_path).unwrap();
        repository_bytes[4] = 1;
        fs::write(&repository_path, &repository_bytes).unwrap();
        let before = fs::read(&repository_path).unwrap();
        assert!(registry.recover().is_err());
        assert_eq!(fs::read(&repository_path).unwrap(), before);
        cleanup(&root);
    }

    #[test]
    fn namespace_conflicts_fail_before_allocating_an_index() {
        let (root, registry) = fixture("namespace-conflict");
        registry.initialize_fresh(100).unwrap();
        registry
            .append_repository(&RepositoryCreateSpec {
                repo_name: "one".to_string(),
                namespace_ascii: [b'x', 0],
                policy_flags: 0,
                created_at_s: 101,
            })
            .unwrap();
        let before = registry.list().unwrap().len();
        assert!(registry
            .append_repository(&RepositoryCreateSpec {
                repo_name: "two".to_string(),
                namespace_ascii: [b'x', 0],
                policy_flags: 0,
                created_at_s: 102,
            })
            .is_err());
        assert_eq!(registry.list().unwrap().len(), before);
        assert!(!registry.repository_authority_parent().join("5").exists());
        cleanup(&root);
    }

    #[test]
    fn recovery_discards_only_unreferenced_trailing_payload_bytes() {
        let (root, registry) = fixture("payload-tail");
        registry.initialize_fresh(100).unwrap();
        let payload_path = registry.repository_payload_path();
        let mut payload = OpenOptions::new().append(true).open(&payload_path).unwrap();
        payload.write_all(b"uncommitted").unwrap();
        payload.sync_all().unwrap();
        let report = registry.recover().unwrap();
        assert_eq!(report.payload_bytes_truncated, 11);
        assert!(registry.validate().is_ok());
        cleanup(&root);
    }

    #[test]
    fn initialized_append_populates_authority_before_record_commit() {
        let (root, registry) = fixture("initialized-append");
        registry.initialize_fresh(100).unwrap();
        let repository_path = registry.repository_path();
        let entry = registry
            .append_repository_with_initializer(
                &RepositoryCreateSpec {
                    repo_name: "ait-runner".to_string(),
                    namespace_ascii: [b'R', 0],
                    policy_flags: PROTOTYPE_POLICY_DEFAULT_FLAGS,
                    created_at_s: 101,
                },
                |repository_index, authority_root| {
                    assert_eq!(repository_index, 4);
                    assert_eq!(
                        required_file_len(&repository_path).unwrap(),
                        OPERATIONAL_BIN_HEADER_SIZE
                            + 4 * u64::from(OPERATIONAL_REPOSITORY_RECORD_SIZE)
                    );
                    fs::write(authority_root.join("initialized"), b"ready").unwrap();
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(entry.repository_index, 4);
        assert_eq!(
            fs::read(
                registry
                    .repository_authority_parent()
                    .join("4")
                    .join("initialized")
            )
            .unwrap(),
            b"ready"
        );
        cleanup(&root);
    }

    #[test]
    fn initializer_failure_removes_only_the_uncommitted_authority() {
        let (root, registry) = fixture("initializer-failure");
        registry.initialize_fresh(100).unwrap();
        let repository_before = fs::read(registry.repository_path()).unwrap();
        let payload_before = fs::read(registry.repository_payload_path()).unwrap();
        let error = registry
            .append_repository_with_initializer(
                &RepositoryCreateSpec {
                    repo_name: "ait-runner".to_string(),
                    namespace_ascii: [b'R', 0],
                    policy_flags: PROTOTYPE_POLICY_DEFAULT_FLAGS,
                    created_at_s: 101,
                },
                |_, authority_root| {
                    fs::write(authority_root.join("partial"), b"partial").unwrap();
                    Err(BinaryDbError::other("initializer failed"))
                },
            )
            .unwrap_err();
        assert!(error.contains("initializer failed"));
        assert!(!registry.repository_authority_parent().join("4").exists());
        assert_eq!(
            fs::read(registry.repository_path()).unwrap(),
            repository_before
        );
        assert_eq!(
            fs::read(registry.repository_payload_path()).unwrap(),
            payload_before
        );
        assert_eq!(registry.validate().unwrap().len(), 4);
        cleanup(&root);
    }

    #[test]
    fn recovery_removes_only_the_exact_uncommitted_tail_directory() {
        let (root, registry) = fixture("staged-tail");
        registry.initialize_fresh(100).unwrap();
        let staged = registry.repository_authority_parent().join("4");
        fs::create_dir(&staged).unwrap();
        fs::write(staged.join("partial"), b"partial").unwrap();

        let report = registry.recover().unwrap();
        assert_eq!(report.repository_count, 4);
        assert!(!staged.exists());
        assert!(registry.validate().is_ok());

        fs::create_dir(registry.repository_authority_parent().join("5")).unwrap();
        assert!(registry.recover().is_err());
        assert!(registry.repository_authority_parent().join("5").exists());
        cleanup(&root);
    }

    #[test]
    fn metadata_replacement_preserves_name_and_physical_index() {
        let (root, registry) = fixture("metadata");
        registry.initialize_fresh(100).unwrap();
        let updated = registry
            .update_live_metadata(
                1,
                REPOSITORY_LIFECYCLE_RETIRING,
                [b's', 0],
                0b1010_0101,
                101,
            )
            .unwrap();
        assert_eq!(updated.repository_index, 1);
        assert_eq!(updated.repo_name, "ait-server");
        assert_eq!(updated.record.created_at_s, 100);
        assert_eq!(updated.record.namespace_ascii, [b's', 0]);
        assert_eq!(registry.get(1).unwrap(), updated);
        assert!(registry
            .update_live_metadata(1, REPOSITORY_LIFECYCLE_PURGED, [b's', 0], 0, 102)
            .is_err());
        cleanup(&root);
    }

    #[test]
    fn coordinated_purge_commits_last_and_releases_namespace_for_a_new_index() {
        let (root, registry) = fixture("coordinated-purge");
        registry.initialize_fresh(100).unwrap();
        let retiring = registry
            .update_live_metadata(
                1,
                REPOSITORY_LIFECYCLE_RETIRING,
                [b's', 0],
                0b1010_0101,
                101,
            )
            .unwrap();
        let mut prepared = false;
        let purged = registry
            .coordinate_repository_purge(1, 102, |current| {
                assert_eq!(current, &retiring);
                assert_eq!(current.record.lifecycle_kind, REPOSITORY_LIFECYCLE_RETIRING);
                prepared = true;
                Ok(())
            })
            .unwrap();
        assert!(prepared);
        assert_eq!(purged.record.lifecycle_kind, REPOSITORY_LIFECYCLE_PURGED);
        assert!(registry
            .discover_live_namespace([b's', 0])
            .unwrap()
            .is_empty());

        let replacement = registry
            .append_repository(&RepositoryCreateSpec {
                repo_name: "ait-server".to_string(),
                namespace_ascii: [b's', 0],
                policy_flags: 0b1010_0101,
                created_at_s: 103,
            })
            .unwrap();
        assert_eq!(replacement.repository_index, 4);
        assert_eq!(
            registry
                .discover_live_namespace([b's', 0])
                .unwrap()
                .iter()
                .map(|entry| entry.repository_index)
                .collect::<Vec<_>>(),
            [4]
        );
        assert!(registry.validate().is_ok());
        cleanup(&root);
    }

    #[test]
    fn canonical_directory_parser_rejects_alias_spellings() {
        for valid in ["0", "1", "42", "4294967295"] {
            assert!(parse_repository_directory_name(valid).is_ok());
        }
        for invalid in ["", "00", "01", "+1", "-1", " 1", "1 ", "1x", "4294967296"] {
            assert!(parse_repository_directory_name(invalid).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn authority_directory_symlink_fails_closed() {
        use std::os::unix::fs::symlink;

        let (root, registry) = fixture("symlink");
        registry.initialize_fresh(100).unwrap();
        let real = registry.repository_authority_parent().join("3");
        fs::remove_dir(&real).unwrap();
        symlink(registry.repository_authority_parent().join("2"), &real).unwrap();
        assert!(registry.validate().is_err());
        cleanup(&root);
    }
}
