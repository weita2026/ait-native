use super::records::{body_sha256, object_key, require_width, u32_at, u64_at};
use super::*;
use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId,
    BinaryIndexId, BinaryPayloadFileId, LocalBinaryDbFs, StoreResult,
};
use crate::content_binary_db::LocalContentBinaryDb;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

pub const GIT_OBJECT_STORE_BIN_PATHS: &[&str] = &[
    "git_repository.bin",
    "git_repository_payload.bin",
    "git_object.bin",
    "git_generation.bin",
    "git_identity.bin",
    "git_identity_payload.bin",
    "git_commit_mapping.bin",
    "git_ref_mapping.bin",
    "git_ref_mapping_payload.bin",
    "git_operation_checkpoint.bin",
];
pub const GIT_OBJECT_STORE_INDEX_PATHS: &[&str] = &[
    "git_repository_fingerprint.idx",
    "git_object.idx",
    "git_document_plan.idx",
    "git_document_revision.idx",
    "git_generation_id.idx",
    "git_commit_object.idx",
    "git_commit_snapshot.idx",
    "git_ref_lookup.idx",
    "git_operation_id.idx",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GitRepositoryKind {
    ImportSource = 0,
    ExportTarget = 1,
    AitSource = 2,
}

#[derive(Clone, Debug)]
struct RepositoryRecord {
    kind: u8,
    format: u8,
    identity_len: u32,
    identity_offset: u64,
    fingerprint: [u8; 12],
}

impl RepositoryRecord {
    fn decode(bytes: &[u8]) -> StoreResult<Self> {
        require_width(bytes, 28, "GitRepository")?;
        if bytes[0] > 2
            || bytes[2..4] != [0, 0]
            || bytes[1] != if bytes[0] == 2 { 0 } else { 1 }
            || u32_at(bytes, 4) == 0
        {
            return Err(BinaryDbError::corruption(
                "invalid GitRepository format/flags/identity",
            ));
        }
        Ok(Self {
            kind: bytes[0],
            format: bytes[1],
            identity_len: u32_at(bytes, 4),
            identity_offset: u64_at(bytes, 8),
            fingerprint: bytes[16..28].try_into().expect("checked width"),
        })
    }

    fn encode(&self) -> [u8; 28] {
        let mut bytes = [0; 28];
        bytes[0] = self.kind;
        bytes[1] = self.format;
        bytes[4..8].copy_from_slice(&self.identity_len.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.identity_offset.to_le_bytes());
        bytes[16..28].copy_from_slice(&self.fingerprint);
        bytes
    }
}

pub(super) fn repository_file() -> BinaryFileId {
    BinaryFileId::new("git_repository.bin", 1, GIT_REPOSITORY_RECORD_SIZE)
}

fn repository_payload() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new("git_repository_payload.bin", 1)
}

fn repository_index() -> BinaryIndexId {
    BinaryIndexId::new_fixed("git_repository_fingerprint.idx", 1, 12, true)
}

pub(super) fn verify_source_repository(
    db: &LocalBinaryDbFs,
    ordinal: u32,
    fingerprint: [u8; 12],
    identity: &str,
) -> StoreResult<()> {
    let read = BinaryDbReadTxn::new(db);
    let record = RepositoryRecord::decode(&read.read_record(repository_file(), ordinal)?)?;
    let bytes = read.read_payload(
        repository_payload(),
        record.identity_offset,
        record.identity_len,
    )?;
    if record.kind != GitRepositoryKind::ImportSource as u8
        || record.fingerprint != fingerprint
        || bytes != identity.as_bytes()
    {
        return Err(BinaryDbError::invalid_domain_data(
            "Git document source repository/full identity mismatch",
        ));
    }
    Ok(())
}

/// Native zstd bodies live in the existing shared content root. Only optional
/// interoperability records and their rebuildable indexes live in `db`.
///
/// No production CLI calls this adapter until complete mapping/Plan/GC
/// admission is wired. Opening validates the index closure once. An in-memory
/// command index retains keys and ordinals, never raw bodies. Bounded batches
/// use the existing native pack writer and transaction index append mechanism.
pub struct LocalGitObjectStore {
    db: LocalBinaryDbFs,
    content: LocalContentBinaryDb<1>,
    object_keys: Mutex<BTreeMap<[u8; 25], u32>>,
    blob_session: Mutex<crate::content_binary_db::adapters::local::RawBlobBatchSession>,
    #[cfg(test)]
    pub(super) body_reads: std::sync::atomic::AtomicUsize,
}

/// Read-only metadata reuse, scoped to a caller-owned batch or admission.
type GitAuthorityStamps = BTreeMap<std::path::PathBuf, (u64, Option<std::time::SystemTime>)>;

/// One operation's bounded pack readers and authority proofs. Resolved bodies
/// end at each batch; immutable reader stamps survive cache eviction.
pub struct GitObjectBatchReader<'a> {
    objects: &'a LocalGitObjectStore,
    body: crate::content_binary_db::BinaryBlobReadSession,
    stamps: Option<GitAuthorityStamps>,
    git_read: BinaryDbReadTxn<'a, LocalBinaryDbFs>,
    content_read: BinaryDbReadTxn<'a, LocalBinaryDbFs>,
    repositories: BTreeSet<u32>,
}
impl GitObjectBatchReader<'_> {
    pub fn get_record_batch(
        &mut self,
        repository: u32,
        format: u8,
        oids: &[[u8; 20]],
    ) -> StoreResult<Vec<GitObjectRecord>> {
        if oids.len() > 128 {
            return Err(BinaryDbError::invalid_domain_data(
                "Git metadata batch exceeds 128",
            ));
        }
        require_sha1_object_id(format, &[0; 20])?;
        self.verify()?;
        if !self.repositories.contains(&repository) {
            RepositoryRecord::decode(&self.git_read.read_record(repository_file(), repository)?)?;
            self.repositories.insert(repository);
        }
        let keys = self
            .objects
            .object_keys
            .lock()
            .map_err(|_| BinaryDbError::corruption("Git index session poisoned"))?;
        let rows = oids
            .iter()
            .map(|oid| {
                let key = object_key(repository, oid);
                let ordinal = keys.get(&key).copied().ok_or_else(|| {
                    BinaryDbError::missing_data("Git metadata batch object absent")
                })?;
                let row = GitObjectRecord::decode(
                    &self.git_read.read_record(git_object_file(), ordinal)?,
                )?;
                if row.index_key() != key {
                    return Err(BinaryDbError::corruption(
                        "Git object index points to a different key",
                    ));
                }
                Ok(row)
            })
            .collect::<StoreResult<Vec<_>>>()?;
        drop(keys);
        self.verify()?;
        Ok(rows)
    }
    pub fn get_batch(
        &mut self,
        repository: u32,
        format: u8,
        oids: &[[u8; 20]],
    ) -> StoreResult<Vec<(GitObjectRecord, Vec<u8>)>> {
        if oids.len() > 128 {
            return Err(BinaryDbError::invalid_domain_data(
                "Git body read batch exceeds 128",
            ));
        }
        let records = self.get_record_batch(repository, format, oids)?;
        self.body.clear_logical_values();
        let rows = records
            .into_iter()
            .map(|row| {
                let body =
                    self.objects
                        .read_body_with_read(&row, &self.content_read, &mut self.body)?;
                Ok((row, body))
            })
            .collect::<StoreResult<Vec<_>>>()?;
        self.verify()?;
        Ok(rows)
    }
    pub fn verify(&mut self) -> StoreResult<()> {
        self.git_read.read_lock_paths()?;
        self.content_read.read_lock_paths()?;
        let session = self.objects.read_session(&mut self.body);
        *session.authority_stamps.borrow_mut() = self.stamps.clone();
        session.verify_metadata_authority()?;
        self.stamps = session.authority_stamps.into_inner();
        self.body.verify_admission_stamps()
    }
}
#[cfg(test)]
thread_local! {
    static METADATA_BATCH_TEST_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) fn set_metadata_batch_test_hook(hook: Box<dyn FnOnce()>) {
    METADATA_BATCH_TEST_HOOK.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(hook);
    });
}

pub(crate) struct GitObjectReadSession<'a> {
    objects: &'a LocalGitObjectStore,
    git_read: BinaryDbReadTxn<'a, LocalBinaryDbFs>,
    pub(crate) content_read: BinaryDbReadTxn<'a, LocalBinaryDbFs>,
    pub(crate) body_read: &'a mut crate::content_binary_db::BinaryBlobReadSession,
    metadata_count: std::cell::Cell<Option<u32>>,
    repositories: std::cell::RefCell<BTreeSet<u32>>,
    authority_stamps: std::cell::RefCell<Option<GitAuthorityStamps>>,
}
impl GitObjectReadSession<'_> {
    pub(crate) fn git_read(&self) -> &BinaryDbReadTxn<'_, LocalBinaryDbFs> {
        &self.git_read
    }

    #[cfg(test)]
    pub(crate) fn legacy_record_for_profile(
        &self,
        repository: u32,
        format: u8,
        oid: &[u8],
    ) -> StoreResult<Option<GitObjectRecord>> {
        self.objects
            .get_record_with_read(&self.git_read, repository, format, oid)
    }
    pub(crate) fn record(
        &self,
        repository: u32,
        format: u8,
        oid: &[u8],
    ) -> StoreResult<Option<GitObjectRecord>> {
        let oid = require_sha1_object_id(format, oid)?;
        if self.metadata_count.get().is_none() {
            self.verify_metadata_authority()?;
        }
        if !self.repositories.borrow().contains(&repository) {
            RepositoryRecord::decode(&self.git_read.read_record(repository_file(), repository)?)?;
            self.repositories.borrow_mut().insert(repository);
        }
        let key = object_key(repository, &oid);
        let keys = self
            .objects
            .object_keys
            .lock()
            .map_err(|_| BinaryDbError::corruption("Git index session poisoned"))?;
        if keys.len() as u64 != u64::from(self.metadata_count.get().unwrap()) {
            return Err(BinaryDbError::retryable_busy(
                "Git object authority changed; reopen the command index session",
            ));
        }
        let Some(ordinal) = keys.get(&key).copied() else {
            return Ok(None);
        };
        let record =
            GitObjectRecord::decode(&self.git_read.read_record(git_object_file(), ordinal)?)?;
        if record.index_key() != key {
            return Err(BinaryDbError::corruption(
                "Git object index points to a different key",
            ));
        }
        Ok(Some(record))
    }

    /// Equivalent namespaces must map every OID/kind to the same immutable
    /// native Blob ordinal. This includes descendants, not merely the root's
    /// SHA1, so different colliding Git bodies cannot share a binding proof.
    pub(crate) fn namespace_body_signatures(&self) -> StoreResult<BTreeMap<u32, [u8; 32]>> {
        use sha2::{Digest, Sha256};
        let keys = self
            .objects
            .object_keys
            .lock()
            .map_err(|_| BinaryDbError::corruption("Git index session poisoned"))?;
        let mut signatures = BTreeMap::<u32, Sha256>::new();
        for (key, ordinal) in keys.iter() {
            let record =
                GitObjectRecord::decode(&self.git_read.read_record(git_object_file(), *ordinal)?)?;
            if record.index_key() != *key {
                return Err(BinaryDbError::corruption(
                    "Git namespace signature key mismatch",
                ));
            }
            let hash = signatures.entry(record.repository_index).or_default();
            hash.update(record.object_id);
            hash.update([record.kind as u8]);
            hash.update(record.content_blob_index.to_le_bytes());
        }
        Ok(signatures
            .into_iter()
            .map(|(repository, hash)| (repository, hash.finalize().into()))
            .collect())
    }

    /// Read locks protect normal writers. Stamps also reject out-of-band changes
    /// before an admission result is published; no proof survives this session.
    pub(crate) fn verify_metadata_authority(&self) -> StoreResult<()> {
        let mut stamps = BTreeMap::new();
        for root in [
            self.objects.db.authority_root(),
            self.objects.content.db().authority_root(),
        ] {
            let entries = match std::fs::read_dir(root.as_path()) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string().into()),
            };
            for entry in entries {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "bin" || e == "idx") {
                    let metadata = entry.metadata().map_err(|e| e.to_string())?;
                    stamps.insert(path, (metadata.len(), metadata.modified().ok()));
                }
            }
        }
        if let Some(expected) = self.authority_stamps.borrow().as_ref() {
            if *expected != stamps {
                return Err(BinaryDbError::retryable_busy(
                    "Git/native metadata authority changed during read session",
                ));
            }
        }
        let count = self.git_read.record_count(git_object_file())?;
        self.objects
            .require_index_count(git_object_index(), count, 29)?;
        let keys = self
            .objects
            .object_keys
            .lock()
            .map_err(|_| BinaryDbError::corruption("Git index session poisoned"))?;
        if keys.len() as u64 != u64::from(count)
            || self.metadata_count.get().is_some_and(|old| old != count)
        {
            return Err(BinaryDbError::retryable_busy(
                "Git object authority changed; reopen the command index session",
            ));
        }
        self.metadata_count.set(Some(count));
        *self.authority_stamps.borrow_mut() = Some(stamps);
        Ok(())
    }
    pub(crate) fn get(
        &mut self,
        repository: u32,
        format: u8,
        oid: &[u8],
    ) -> StoreResult<Option<(GitObjectRecord, Vec<u8>)>> {
        let Some(record) = self.record(repository, format, oid)? else {
            return Ok(None);
        };
        let body = self
            .objects
            .read_body_with_read(&record, &self.content_read, self.body_read)?;
        Ok(Some((record, body)))
    }
}

pub struct GitObjectInput<'a> {
    pub kind: GitObjectKind,
    pub format: u8,
    pub object_id: &'a [u8],
    pub body: &'a [u8],
}

impl LocalGitObjectStore {
    pub fn batch_reader(&self) -> GitObjectBatchReader<'_> {
        GitObjectBatchReader {
            objects: self,
            body: crate::content_binary_db::BinaryBlobReadSession::for_git_alias_batch(),
            stamps: None,
            git_read: BinaryDbReadTxn::new(&self.db),
            content_read: BinaryDbReadTxn::new(self.content.db()),
            repositories: BTreeSet::new(),
        }
    }
    pub(super) fn shares_authorities(
        &self,
        interop: &LocalBinaryDbFs,
        content: &LocalContentBinaryDb<1>,
    ) -> bool {
        self.db.authority_root() == interop.authority_root()
            && self.content.db().authority_root() == content.db().authority_root()
    }

    pub fn open(db: LocalBinaryDbFs, content: LocalContentBinaryDb<1>) -> StoreResult<Self> {
        Self::open_with_body_admission(db, content, true)
    }

    /// Inactive repack reads and verifies every OID/body in bounded batches,
    /// then seals the complete generation. Normal runtime admission stays strict.
    pub(crate) fn open_for_staged_repack(
        db: LocalBinaryDbFs,
        content: LocalContentBinaryDb<1>,
    ) -> StoreResult<Self> {
        Self::open_with_body_admission(db, content, false)
    }

    fn open_with_body_admission(
        db: LocalBinaryDbFs,
        content: LocalContentBinaryDb<1>,
        admit_bodies: bool,
    ) -> StoreResult<Self> {
        Self::open_admission_inner(
            db,
            content,
            admit_bodies,
            #[cfg(test)]
            None,
        )
    }
    fn open_admission_inner(
        db: LocalBinaryDbFs,
        content: LocalContentBinaryDb<1>,
        admit_bodies: bool,
        #[cfg(test)] limit: Option<usize>,
    ) -> StoreResult<Self> {
        if db.authority_root() == content.db().authority_root() {
            return Err(BinaryDbError::invalid_domain_data(
                "optional Git authority must be separate from shared content authority",
            ));
        }
        let store = Self {
            db,
            content,
            object_keys: Mutex::new(BTreeMap::new()),
            blob_session: Mutex::new(Default::default()),
            #[cfg(test)]
            body_reads: Default::default(),
        };
        let read = BinaryDbReadTxn::new(&store.db);
        let repositories = read.record_count(repository_file())?;
        store.require_index_count(repository_index(), repositories, 16)?;
        for ordinal in 0..repositories {
            let record = RepositoryRecord::decode(&read.read_record(repository_file(), ordinal)?)?;
            let identity = read.read_payload(
                repository_payload(),
                record.identity_offset,
                record.identity_len,
            )?;
            std::str::from_utf8(&identity)
                .map_err(|_| BinaryDbError::corruption("repository identity is not UTF-8"))?;
            let candidates = read.lookup_index(repository_index(), &record.fingerprint)?;
            if candidates
                .iter()
                .filter(|candidate| **candidate == ordinal)
                .count()
                != 1
            {
                return Err(index_recovery_required("repository fingerprint index"));
            }
        }
        let objects = read.record_count(git_object_file())?;
        store.require_index_count(git_object_index(), objects, GIT_OBJECT_INDEX_RECORD_SIZE)?;
        let mut keys = BTreeSet::new();
        let mut body_read = crate::content_binary_db::BinaryBlobReadSession::for_git_admission();
        // Empty/metadata-only opening must not create content-side locks.
        let mut content_read = None;
        // Operation-local, bounded proofs for exact OID/kind/native Blob aliases.
        // Every record/FK/index is still checked; each distinct body is hashed.
        let mut body_proofs = BTreeSet::new();
        #[cfg(test)]
        let mut selected_bytes = 0u64;
        for ordinal in 0..objects {
            #[cfg(test)]
            if limit.is_some_and(|n| ordinal as usize >= n) {
                break;
            }
            #[cfg(test)]
            crate::binary_db::read_profile::note("strict_object_rows", 0);
            if ordinal % 128 == 0 {
                body_read.clear_logical_values();
            }
            let record = GitObjectRecord::decode(&read.read_record(git_object_file(), ordinal)?)?;
            if record.repository_index >= repositories || !keys.insert(record.index_key()) {
                return Err(BinaryDbError::corruption(
                    "invalid GitObject repository or duplicate key",
                ));
            }
            if read.lookup_index(git_object_index(), &record.index_key())? != [ordinal] {
                return Err(index_recovery_required("Git object index"));
            }
            if admit_bodies {
                let content_read =
                    content_read.get_or_insert_with(|| BinaryDbReadTxn::new(store.content.db()));
                let key = (
                    record.content_blob_index,
                    record.kind as u8,
                    record.object_id,
                );
                if !body_proofs.contains(&key) {
                    #[cfg(test)]
                    if limit.is_some() {
                        let size = store
                            .content
                            .blobs()
                            .blob_view_at(content_read, record.content_blob_index)?
                            .size_bytes;
                        if selected_bytes != 0
                            && selected_bytes.saturating_add(size) > 8 * 1024 * 1024
                        {
                            break;
                        }
                        selected_bytes = selected_bytes.saturating_add(size);
                    }
                    let body =
                        { store.read_body_with_read(&record, content_read, &mut body_read)? };
                    #[cfg(test)]
                    crate::binary_db::read_profile::note(
                        "strict_unique_body_reads",
                        body.len() as u64,
                    );
                    #[cfg(not(test))]
                    let _ = body;
                    if body_proofs.len() < 65536 {
                        body_proofs.insert(key);
                    }
                }
            }
            store
                .object_keys
                .lock()
                .map_err(|_| BinaryDbError::corruption("Git index session poisoned"))?
                .insert(record.index_key(), ordinal);
        }
        body_read.verify_admission_stamps()?;
        drop(content_read);
        drop(read);
        Ok(store)
    }

    pub fn db(&self) -> &LocalBinaryDbFs {
        &self.db
    }

    /// Resolve exact stored identity; short fingerprints alone never select an endpoint.
    pub fn repository_identity(
        &self,
        ordinal: u32,
    ) -> StoreResult<(GitRepositoryKind, [u8; 12], String)> {
        repository_identity_at(&self.db, ordinal)
    }

    /// Fingerprints come from existing CLI identity algorithms. Compare full
    /// identities after the fingerprint lookup; a prefix alone is not identity.
    pub fn ensure_repository(
        &self,
        kind: GitRepositoryKind,
        format: u8,
        fingerprint: [u8; 12],
        identity: &str,
    ) -> StoreResult<u32> {
        if format
            != if kind == GitRepositoryKind::AitSource {
                0
            } else {
                1
            }
        {
            return Err(BinaryDbError::unsupported(
                "unsupported Git repository object format",
            ));
        }
        let identity_len = u32::try_from(identity.len())
            .map_err(|_| BinaryDbError::invalid_domain_data("repository identity exceeds u32"))?;
        if identity_len == 0 {
            return Err(BinaryDbError::invalid_domain_data(
                "empty repository identity",
            ));
        }
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)?;
        self.require_index_count(
            repository_index(),
            write.record_count(repository_file())?,
            16,
        )?;
        for ordinal in write.lookup_index(repository_index(), &fingerprint)? {
            let record = RepositoryRecord::decode(&write.read_record(repository_file(), ordinal)?)?;
            if record.fingerprint != fingerprint {
                return Err(BinaryDbError::corruption(
                    "repository index points to a different key",
                ));
            }
            let bytes = write.read_payload(
                repository_payload(),
                record.identity_offset,
                record.identity_len,
            )?;
            if record.kind == kind as u8 && record.format == format && bytes == identity.as_bytes()
            {
                return Ok(ordinal);
            }
        }
        let range = write.append_payload(repository_payload(), identity.as_bytes())?;
        let record = RepositoryRecord {
            kind: kind as u8,
            format,
            identity_len,
            identity_offset: range.payload_offset,
            fingerprint,
        };
        let ordinal = write.append_record(repository_file(), &record.encode())?;
        write.append_index_candidate(repository_index(), &fingerprint, ordinal)?;
        write.commit()?;
        Ok(ordinal)
    }

    pub fn put(
        &self,
        repository: u32,
        kind: GitObjectKind,
        format: u8,
        oid: &[u8],
        body: &[u8],
    ) -> StoreResult<u32> {
        Ok(self.put_batch(
            repository,
            &[GitObjectInput {
                kind,
                format,
                object_id: oid,
                body,
            }],
        )?[0])
    }

    /// Reuses only admitted key/ordinal metadata in this command. Raw content
    /// bodies are bounded caller inputs, never cached across commits. Native
    /// append-only object authority cannot change behind a live session: a
    /// different writer's append requires reopening and validating its index.
    pub fn put_batch(
        &self,
        repository: u32,
        inputs: &[GitObjectInput<'_>],
    ) -> StoreResult<Vec<u32>> {
        self.put_batch_with_hints(repository, inputs, None, None)
    }

    pub fn put_batch_with_delta_hints(
        &self,
        repository: u32,
        inputs: &[GitObjectInput<'_>],
        hints: &[Option<&str>],
    ) -> StoreResult<Vec<u32>> {
        if inputs.len() != hints.len() || inputs.len() > 128 {
            return Err(BinaryDbError::invalid_domain_data(
                "Git delta batch requires one hint per object and at most 128 objects",
            ));
        }
        self.put_batch_with_hints(repository, inputs, Some(hints), None)
    }

    /// Transient read-locality classification; all kinds, bodies and identities are still verified.
    pub fn put_batch_with_delta_hints_and_partitions(
        &self,
        repository: u32,
        inputs: &[GitObjectInput<'_>],
        hints: &[Option<&str>],
        partitions: &[bool],
    ) -> StoreResult<Vec<u32>> {
        if inputs.len() != hints.len()
            || inputs.len() != partitions.len()
            || inputs.len() > 128
            || inputs
                .iter()
                .zip(partitions)
                .any(|(input, p)| *p && input.kind != GitObjectKind::Blob)
        {
            return Err(BinaryDbError::invalid_domain_data("partitioned Git batch requires one hint/class per object; only Blob may be Markdown; at most 128 objects"));
        }
        self.put_batch_with_hints(repository, inputs, Some(hints), Some(partitions))
    }
    fn put_batch_with_hints(
        &self,
        repository: u32,
        inputs: &[GitObjectInput<'_>],
        hints: Option<&[Option<&str>]>,
        partitions: Option<&[bool]>,
    ) -> StoreResult<Vec<u32>> {
        let oids = inputs
            .iter()
            .map(|input| {
                let oid = require_sha1_object_id(input.format, input.object_id)?;
                if git_object_id(input.kind, input.body) != oid {
                    return Err(BinaryDbError::invalid_domain_data(
                        "Git object body/type/OID mismatch",
                    ));
                }
                Ok(oid)
            })
            .collect::<StoreResult<Vec<_>>>()?;
        let priority_oids = partitions.map(|classes| {
            oids.iter()
                .zip(classes)
                .filter(|(_, p)| **p)
                .map(|(oid, _)| *oid)
                .collect::<BTreeSet<_>>()
        });
        #[cfg(test)]
        crate::binary_db::read_profile::note("git_object_write_transactions", 0);
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)?;
        RepositoryRecord::decode(&write.read_record(repository_file(), repository)?)?;
        let mut keys = self
            .object_keys
            .lock()
            .map_err(|_| BinaryDbError::corruption("Git index session poisoned"))?;
        let count = write.record_count(git_object_file())?;
        self.require_index_count(git_object_index(), count, 29)?;
        if keys.len() as u64 != u64::from(count) {
            return Err(BinaryDbError::retryable_busy(
                "Git object authority changed; reopen the command index session",
            ));
        }
        let mut pending = BTreeMap::<[u8; 25], usize>::new();
        let mut body_read = crate::content_binary_db::BinaryBlobReadSession::default();
        let content_read = BinaryDbReadTxn::new(self.content.db());
        for (position, (input, oid)) in inputs.iter().zip(&oids).enumerate() {
            let key = object_key(repository, oid);
            if let Some(ordinal) = keys.get(&key) {
                let record =
                    GitObjectRecord::decode(&write.read_record(git_object_file(), *ordinal)?)?;
                if record.index_key() != key
                    || record.kind != input.kind
                    || self.read_body_with_read(&record, &content_read, &mut body_read)?
                        != input.body
                {
                    return Err(BinaryDbError::corruption(
                        "conflicting Git object identity/body",
                    ));
                }
            } else if let Some(previous) = pending.insert(key, position) {
                if inputs[previous].kind != input.kind || inputs[previous].body != input.body {
                    return Err(BinaryDbError::corruption(
                        "conflicting Git object within input batch",
                    ));
                }
            }
        }
        drop(content_read);
        let mut positions = pending.values().copied().collect::<Vec<_>>();
        // Preserve the caller's graph/history order for bounded base windows.
        positions.sort_unstable();
        let bodies = positions
            .iter()
            .map(|position| inputs[*position].body)
            .collect::<Vec<_>>();
        let blobs = if bodies.is_empty() {
            Vec::new()
        } else {
            #[cfg(test)]
            crate::binary_db::read_profile::note(
                "git_native_blob_ensure_batches",
                bodies.iter().map(|b| b.len() as u64).sum(),
            );
            let mut session = self
                .blob_session
                .lock()
                .map_err(|_| BinaryDbError::corruption("Blob batch session poisoned"))?;
            if let Some(hints) = hints {
                let selected = positions
                    .iter()
                    .map(|position| hints[*position])
                    .collect::<Vec<_>>();
                if partitions.is_some() {
                    let classes = positions
                        .iter()
                        .map(|p| {
                            priority_oids
                                .as_ref()
                                .is_some_and(|set| set.contains(&oids[*p]))
                        })
                        .collect::<Vec<_>>();
                    self.content
                        .ensure_blob_bytes_batch_with_delta_hints_and_partitions(
                            &bodies,
                            &selected,
                            &classes,
                            &mut session,
                        )
                        .map_err(BinaryDbError::from)?
                } else {
                    self.content
                        .ensure_blob_bytes_batch_with_delta_hints(&bodies, &selected, &mut session)
                        .map_err(BinaryDbError::from)?
                }
            } else {
                self.content
                    .ensure_blob_bytes_batch_with_session(&bodies, &mut session)
                    .map_err(BinaryDbError::from)?
            }
        };
        let mut appended = BTreeMap::new();
        for (position, blob_index) in positions.into_iter().zip(blobs) {
            let record = GitObjectRecord {
                kind: inputs[position].kind,
                repository_index: repository,
                content_blob_index: blob_index,
                object_id: oids[position],
            };
            let ordinal = write.append_record(git_object_file(), &record.encode())?;
            write.append_index_candidate(git_object_index(), &record.index_key(), ordinal)?;
            appended.insert(record.index_key(), ordinal);
        }
        write.commit()?;
        keys.extend(appended);
        if let Some(hints) = hints {
            let read = BinaryDbReadTxn::new(&self.db);
            let mut session = self
                .blob_session
                .lock()
                .map_err(|_| BinaryDbError::corruption("Blob batch session poisoned"))?;
            for (hint, oid) in hints.iter().zip(&oids) {
                if let Some(hint) = hint {
                    let ordinal = keys
                        .get(&object_key(repository, oid))
                        .ok_or("Git delta head absent")?;
                    let record =
                        GitObjectRecord::decode(&read.read_record(git_object_file(), *ordinal)?)?;
                    session.note_delta_hint(hint, record.content_blob_index);
                }
            }
        }
        inputs
            .iter()
            .zip(&oids)
            .map(|(_, oid)| {
                keys.get(&object_key(repository, oid))
                    .copied()
                    .ok_or_else(|| {
                        BinaryDbError::corruption("committed Git object absent from index session")
                    })
            })
            .collect()
    }

    pub fn get(
        &self,
        repository: u32,
        format: u8,
        oid: &[u8],
    ) -> StoreResult<Option<(GitObjectRecord, Vec<u8>)>> {
        self.get_with_read_session(repository, format, oid, &mut Default::default())
    }

    pub(crate) fn read_session<'a>(
        &'a self,
        body_read: &'a mut crate::content_binary_db::BinaryBlobReadSession,
    ) -> GitObjectReadSession<'a> {
        GitObjectReadSession {
            objects: self,
            git_read: BinaryDbReadTxn::new(&self.db),
            content_read: BinaryDbReadTxn::new(self.content.db()),
            body_read,
            metadata_count: Default::default(),
            repositories: Default::default(),
            authority_stamps: Default::default(),
        }
    }
    pub(crate) fn get_with_read_session(
        &self,
        repository: u32,
        format: u8,
        oid: &[u8],
        body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
    ) -> StoreResult<Option<(GitObjectRecord, Vec<u8>)>> {
        require_sha1_object_id(format, oid)?;
        let mut session = self.read_session(body_read);
        let result = session.get(repository, format, oid)?;
        session.verify_metadata_authority()?;
        Ok(result)
    }

    /// A caller-bounded set of bytes, with no reader retained after this call.
    pub fn get_batch(
        &self,
        repository: u32,
        format: u8,
        oids: &[[u8; 20]],
    ) -> StoreResult<Vec<(GitObjectRecord, Vec<u8>)>> {
        if oids.len() > 128 {
            return Err(BinaryDbError::invalid_domain_data(
                "Git body read batch exceeds 128",
            ));
        }
        require_sha1_object_id(format, &[0; 20])?;
        let mut body_read = Default::default();
        let mut session = self.read_session(&mut body_read);
        let result = oids
            .iter()
            .map(|oid| {
                session
                    .get(repository, format, oid)?
                    .ok_or_else(|| BinaryDbError::missing_data("Git body batch object absent"))
            })
            .collect::<StoreResult<Vec<_>>>()?;
        session.verify_metadata_authority()?;
        Ok(result)
    }

    /// A bounded metadata-only batch retains all per-record authority guards.
    pub fn get_record_batch(
        &self,
        repository: u32,
        format: u8,
        oids: &[[u8; 20]],
    ) -> StoreResult<Vec<GitObjectRecord>> {
        if oids.len() > 128 {
            return Err(BinaryDbError::invalid_domain_data(
                "Git metadata batch exceeds 128",
            ));
        }
        require_sha1_object_id(format, &[0; 20])?;
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let stamps = self.metadata_batch_stamps()?;
        // One read guard owns this bounded immutable batch. Repository,
        // fixed-table and index-count invariants are constant within it;
        // every selected record still verifies its complete encoding and key.
        RepositoryRecord::decode(&read.read_record(repository_file(), repository)?)?;
        let count = read.record_count(git_object_file())?;
        self.require_index_count(git_object_index(), count, 29)?;
        let keys = self
            .object_keys
            .lock()
            .map_err(|_| BinaryDbError::corruption("Git index session poisoned"))?;
        if keys.len() as u64 != u64::from(count) {
            return Err(BinaryDbError::retryable_busy(
                "Git object authority changed; reopen the command index session",
            ));
        }
        let records = oids
            .iter()
            .map(|oid| {
                let key = object_key(repository, oid);
                let ordinal = keys.get(&key).copied().ok_or_else(|| {
                    BinaryDbError::missing_data("Git metadata batch object absent")
                })?;
                let record =
                    GitObjectRecord::decode(&read.read_record(git_object_file(), ordinal)?)?;
                if record.index_key() != key {
                    return Err(BinaryDbError::corruption(
                        "Git object index points to a different key",
                    ));
                }
                Ok(record)
            })
            .collect::<StoreResult<Vec<_>>>()?;
        drop(keys);
        #[cfg(test)]
        METADATA_BATCH_TEST_HOOK.with(|slot| {
            if let Some(hook) = slot.borrow_mut().take() {
                hook();
            }
        });
        if self.metadata_batch_stamps()? != stamps {
            return Err(BinaryDbError::retryable_busy(
                "Git metadata authority changed during bounded batch",
            ));
        }
        Ok(records)
    }

    fn metadata_batch_stamps(&self) -> StoreResult<GitAuthorityStamps> {
        let mut stamps = BTreeMap::new();
        for name in ["git_repository.bin", "git_object.bin", "git_object.idx"] {
            let path = self.db.authority_root().as_path().join(name);
            match std::fs::metadata(&path) {
                Ok(meta) => {
                    stamps.insert(path, (meta.len(), meta.modified().ok()));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string().into()),
            }
        }
        Ok(stamps)
    }

    /// Command-admitted immutable metadata without decompressing its body.
    /// Opening/putting the object validates bodies; callers still use get when
    /// bytes are needed. The key and changed-authority guard remain identical.
    pub fn get_record(
        &self,
        repository: u32,
        format: u8,
        oid: &[u8],
    ) -> StoreResult<Option<GitObjectRecord>> {
        require_sha1_object_id(format, oid)?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.get_record_with_read(&read, repository, format, oid)
    }
    fn get_record_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
        repository: u32,
        format: u8,
        oid: &[u8],
    ) -> StoreResult<Option<GitObjectRecord>> {
        let oid = require_sha1_object_id(format, oid)?;
        RepositoryRecord::decode(&read.read_record(repository_file(), repository)?)?;
        let count = read.record_count(git_object_file())?;
        self.require_index_count(git_object_index(), count, 29)?;
        let keys = self
            .object_keys
            .lock()
            .map_err(|_| BinaryDbError::corruption("Git index session poisoned"))?;
        if keys.len() as u64 != u64::from(count) {
            return Err(BinaryDbError::retryable_busy(
                "Git object authority changed; reopen the command index session",
            ));
        }
        let key = object_key(repository, &oid);
        let Some(ordinal) = keys.get(&key).copied() else {
            return Ok(None);
        };
        let record = GitObjectRecord::decode(&read.read_record(git_object_file(), ordinal)?)?;
        if record.index_key() != key {
            return Err(BinaryDbError::corruption(
                "Git object index points to a different key",
            ));
        }
        Ok(Some(record))
    }

    fn read_body_with_read(
        &self,
        record: &GitObjectRecord,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
        session: &mut crate::content_binary_db::BinaryBlobReadSession,
    ) -> StoreResult<Vec<u8>> {
        #[cfg(test)]
        self.body_reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let blob = self
            .content
            .blobs()
            .blob_view_at(read, record.content_blob_index)?;
        let bytes = {
            self.content
                .blobs()
                .read_blob_bytes_for_view_with_session(read, &blob, session, 0, None)?
        };
        if bytes.len() as u64 != blob.size_bytes
            || body_sha256(&bytes) != blob.record.sha256
            || git_object_id(record.kind, &bytes) != record.object_id
        {
            return Err(BinaryDbError::corruption(
                "Git object native body hash/length/OID mismatch",
            ));
        }
        Ok(bytes)
    }

    fn require_index_count(
        &self,
        index: BinaryIndexId,
        count: u32,
        row_size: u32,
    ) -> StoreResult<()> {
        #[cfg(test)]
        crate::binary_db::read_profile::note("git_index_count_checks", 0);
        let path = self
            .db
            .authority_root()
            .as_path()
            .join(index.relative_path().as_path());
        let length = self.db.metadata_len(&path)?;
        let expected = 4 + u64::from(count) * u64::from(row_size);
        if length == Some(expected) || (count == 0 && length.is_none()) {
            Ok(())
        } else {
            Err(index_recovery_required("Git index count"))
        }
    }

    /// Explicit bounded index recovery. Indexes are rebuilt from fixed records
    /// under existing transaction locks and atomic replacement, never JSON.
    /// Each replacement is independent; an interruption is safe to retry.
    pub fn rebuild_indexes(db: &LocalBinaryDbFs) -> StoreResult<()> {
        for objects in [false, true] {
            let mut write = BinaryDbWriteTxn::begin(db, BinaryDbCommandScope::General)?;
            let file = if objects {
                git_object_file()
            } else {
                repository_file()
            };
            let mut bytes = 1u32.to_le_bytes().to_vec();
            let mut object_keys = BTreeSet::new();
            for ordinal in 0..write.record_count(file.clone())? {
                let row = write.read_record(file.clone(), ordinal)?;
                if objects {
                    let record = GitObjectRecord::decode(&row)?;
                    RepositoryRecord::decode(
                        &write.read_record(repository_file(), record.repository_index)?,
                    )?;
                    if !object_keys.insert(record.index_key()) {
                        return Err(BinaryDbError::corruption(
                            "duplicate Git object key during index recovery",
                        ));
                    }
                    bytes.extend_from_slice(&record.index_key());
                } else {
                    let record = RepositoryRecord::decode(&row)?;
                    let identity = write.read_payload(
                        repository_payload(),
                        record.identity_offset,
                        record.identity_len,
                    )?;
                    std::str::from_utf8(&identity).map_err(|_| {
                        BinaryDbError::corruption("repository identity is not UTF-8")
                    })?;
                    bytes.extend_from_slice(&record.fingerprint);
                }
                bytes.extend_from_slice(
                    &ordinal
                        .checked_add(1)
                        .ok_or_else(|| BinaryDbError::corruption("index ordinal overflow"))?
                        .to_le_bytes(),
                );
            }
            let index = if objects {
                git_object_index()
            } else {
                repository_index()
            };
            let size = if objects { 29 } else { 16 };
            write.commit_with_atomic_file_replacement(
                BinaryFileId::new(index.relative_path().clone(), 1, size),
                &bytes,
                "git-index-rebuild",
            )?;
        }
        Ok(())
    }
}

fn index_recovery_required(name: &str) -> BinaryDbError {
    BinaryDbError::corruption(format!(
        "{name} is incomplete or inconsistent; rebuild Git indexes before admission"
    ))
}

pub(super) fn repository_identity_at(
    db: &LocalBinaryDbFs,
    ordinal: u32,
) -> StoreResult<(GitRepositoryKind, [u8; 12], String)> {
    let read = BinaryDbReadTxn::new(db);
    let record = RepositoryRecord::decode(&read.read_record(repository_file(), ordinal)?)?;
    let identity = String::from_utf8(read.read_payload(
        repository_payload(),
        record.identity_offset,
        record.identity_len,
    )?)
    .map_err(|_| BinaryDbError::corruption("Git repository identity is not UTF-8"))?;
    let kind = match record.kind {
        0 => GitRepositoryKind::ImportSource,
        1 => GitRepositoryKind::ExportTarget,
        2 => GitRepositoryKind::AitSource,
        _ => unreachable!("validated kind"),
    };
    Ok((kind, record.fingerprint, identity))
}
