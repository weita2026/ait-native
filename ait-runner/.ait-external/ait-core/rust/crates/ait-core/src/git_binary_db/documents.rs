use super::*;
use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId,
    BinaryIndexId, LocalBinaryDbFs, StoreResult,
};
use crate::content_binary_db::{
    adapters::local::{RawBlobBatchSession, RawTreeSession},
    LocalContentBinaryDb,
};
use crate::plan_binary_db::{
    BinaryDbPlanStore, PlanCodec, PlanPayload, PlanRecord, PlanRevisionPayload, PlanRevisionRecord,
};
use std::collections::BTreeMap;
mod recovery;

pub struct RawGitDocumentInput<'a> {
    pub path: &'a str,
    pub mode: u16,
    pub body: &'a [u8],
}

pub struct RawGitDocumentObjectInput<'a> {
    pub path: &'a str,
    pub mode: u16,
    pub object_id: [u8; 20],
}

/// Exact source plans and immutable document sets. Raw bodies, artifact roots
/// and descriptors precede owners; no ordinary PlanItems or Tasks are created.
pub struct RawGitDocumentStore {
    interop: LocalBinaryDbFs,
    content: LocalContentBinaryDb<1>,
    blobs: RawBlobBatchSession,
    trees: RawTreeSession,
    plans: BTreeMap<(String, String, String), u32>,
    revisions: BTreeMap<(u32, u16, u64, [u8; 32]), u32>,
}
#[cfg(test)]
type DocumentOpenHook = Box<dyn FnOnce(&mut crate::content_binary_db::BinaryBlobReadSession)>;
#[cfg(test)]
thread_local! { static DOCUMENT_OPEN_HOOK: std::cell::RefCell<Option<DocumentOpenHook>> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn set_document_open_hook(hook: DocumentOpenHook) {
    DOCUMENT_OPEN_HOOK.with(|slot| {
        assert!(slot.borrow().is_none());
        *slot.borrow_mut() = Some(hook);
    });
}
// Read locks protect ordinary writers; final stamps reject out-of-band changes
// while the shared reader retains verified chunks or cached metadata.
type DocumentAuthorityStamps = BTreeMap<std::path::PathBuf, (u64, Option<std::time::SystemTime>)>;
fn document_authority_stamps(
    interop: &LocalBinaryDbFs,
    content: &LocalContentBinaryDb<1>,
) -> StoreResult<DocumentAuthorityStamps> {
    let mut result = BTreeMap::new();
    for root in [interop.authority_root(), content.db().authority_root()] {
        let entries = match std::fs::read_dir(root.as_path()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.to_string().into()),
        };
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "bin" || e == "idx") {
                let meta = entry.metadata().map_err(|e| e.to_string())?;
                result.insert(path, (meta.len(), meta.modified().ok()));
            }
        }
    }
    Ok(result)
}
type DocumentIndexRows = Vec<(&'static str, u32, Vec<(Vec<u8>, u32)>)>;

impl RawGitDocumentStore {
    pub fn document_set_files(&self, tree_index: u32) -> StoreResult<Vec<(String, u16, Vec<u8>)>> {
        self.document_set_files_with_session(tree_index, &mut Default::default())
    }

    pub(crate) fn document_set_files_with_session(
        &self,
        tree_index: u32,
        body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
    ) -> StoreResult<Vec<(String, u16, Vec<u8>)>> {
        let read = BinaryDbReadTxn::new(self.content.db());
        let mut cache = crate::content_binary_db::BinaryDbTreeReadCache::default();
        self.document_set_files_with_read_scope(tree_index, body_read, &read, &mut cache)
    }

    pub(crate) fn document_set_files_with_read_scope(
        &self,
        tree_index: u32,
        body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
        read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
        cache: &mut crate::content_binary_db::BinaryDbTreeReadCache,
    ) -> StoreResult<Vec<(String, u16, Vec<u8>)>> {
        if read.db().authority_root() != self.content.db().authority_root() {
            return Err(BinaryDbError::invalid_domain_data(
                "DocSet read scope must share the content authority",
            ));
        }
        let plans = BinaryDbPlanStore::<_, 1>::new(self.content.db().clone());
        let root = self
            .content
            .trees()
            .tree_view_at_with_cache(read, tree_index, cache)?;
        let mut pending = vec![(
            String::new(),
            root.tree_id,
            std::collections::BTreeSet::new(),
        )];
        let mut output = Vec::new();
        while let Some((prefix, id, mut ancestors)) = pending.pop() {
            if !ancestors.insert(id.clone()) {
                return Err(BinaryDbError::corruption("cyclic DocSet Tree"));
            }
            for entry in self
                .content
                .trees()
                .list_tree_entry_views_with_cache(read, &id, cache)?
            {
                let path = if prefix.is_empty() {
                    entry.entry_name.clone()
                } else {
                    format!("{prefix}/{}", entry.entry_name)
                };
                validate_git_artifact_path(&path)?;
                if entry.entry_type == "tree" {
                    pending.push((path, entry.target_id, ancestors.clone()));
                    continue;
                }
                if entry.entry_type != "blob"
                    || entry.mode.as_deref() != Some("0644")
                    || !path.to_ascii_lowercase().ends_with(".md")
                {
                    return Err(BinaryDbError::corruption("invalid DocSet descriptor leaf"));
                }
                let bytes = self
                    .content
                    .blobs()
                    .read_blob_bytes_for_id_with_session(
                        read,
                        &entry.target_id,
                        body_read,
                        0,
                        None,
                    )?
                    .ok_or("DocSet descriptor Blob missing")?;
                let descriptor = GitDocumentVersionDescriptor::decode(&bytes)?;
                let (revision, payload) =
                    plans.read_plan_revision(read, descriptor.plan_revision_index)?;
                if !revision.is_raw_git_artifact()
                    || revision.is_tombstone()
                    || payload.artifact_path_text()? != path
                {
                    return Err(BinaryDbError::corruption(
                        "DocSet exact PlanRevision/path mismatch",
                    ));
                }
                let (mode, body) = read_raw_artifact_with_session(
                    &self.content,
                    read,
                    &revision,
                    &path,
                    &payload.artifact_blob_id_text()?,
                    body_read,
                    cache,
                )?;
                if mode != descriptor.git_mode {
                    return Err(BinaryDbError::corruption("DocSet artifact mode mismatch"));
                }
                descriptor.verify_body(&body)?;
                output.push((path, mode, body));
            }
        }
        output.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(output)
    }

    pub fn open(interop: LocalBinaryDbFs, content: LocalContentBinaryDb<1>) -> StoreResult<Self> {
        let store = Self::from_authority(interop, content)?;
        store.validate_indexes()?;
        Ok(store)
    }
    fn from_authority(
        interop: LocalBinaryDbFs,
        content: LocalContentBinaryDb<1>,
    ) -> StoreResult<Self> {
        Self::from_authority_inner(
            interop,
            content,
            #[cfg(test)]
            None,
            #[cfg(test)]
            None,
        )
    }
    fn from_authority_inner(
        interop: LocalBinaryDbFs,
        content: LocalContentBinaryDb<1>,
        #[cfg(test)] profile: Option<(usize, bool)>,
        #[cfg(test)] selected_ordinals: Option<std::collections::BTreeSet<u32>>,
    ) -> StoreResult<Self> {
        let store = BinaryDbPlanStore::<_, 1>::new(content.db().clone());
        let read = BinaryDbReadTxn::new(content.db());
        let authority_stamps = document_authority_stamps(&interop, &content)?;
        // Ordinary32MiB chunks, plus stamps that survive reader eviction.
        let mut shared_bodies =
            crate::content_binary_db::BinaryBlobReadSession::for_git_alias_batch();
        #[cfg(test)]
        let mut selected = 0usize;
        let mut plans = BTreeMap::new();
        let mut revisions = BTreeMap::new();
        // Reuse bounded verified Trees/archives only within this immutable read
        // transaction. Each artifact retains its path/mode/body/ordinal checks.
        let mut tree_cache = crate::content_binary_db::BinaryDbTreeReadCache::for_git_admission();
        for ordinal in
            0..read.record_count(BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_revision_file())?
        {
            #[cfg(test)]
            if profile.is_some_and(|(limit, _)| selected >= limit) {
                break;
            }
            #[cfg(test)]
            if selected_ordinals
                .as_ref()
                .is_some_and(|set| !set.contains(&ordinal))
            {
                continue;
            }
            let (record, payload) = { store.read_plan_revision(&read, ordinal)? };
            if !record.is_raw_git_artifact() {
                continue;
            }
            if record.is_tombstone() || record.item_count != 0 {
                return Err(BinaryDbError::corruption("invalid raw Git PlanRevision"));
            }
            let selector = payload.artifact_selector_text()?;
            if !selector.starts_with("git-source/") {
                return Err(BinaryDbError::corruption(
                    "raw Git PlanRevision lacks source selector",
                ));
            }
            let path = payload.artifact_path_text()?;
            validate_git_artifact_path(&path)?;
            let plan = store.read_plan_record(&read, record.plan_index)?;
            if plan.state() != crate::plan_binary_db::PlanState::Archived {
                return Err(BinaryDbError::corruption(
                    "Git source Plan must remain archived",
                ));
            }
            let title = payload.title_snapshot_text()?;
            let blob_id = payload.artifact_blob_id_text()?;
            // Full artifact and mode validation is shared with descriptor reads.
            #[cfg(test)]
            let artifact = if profile.is_none_or(|(_, shared)| shared) {
                shared_bodies.clear_logical_values();
                let value = read_raw_artifact_with_session(
                    &content,
                    &read,
                    &record,
                    &path,
                    &blob_id,
                    &mut shared_bodies,
                    &mut tree_cache,
                )?;
                crate::binary_db::read_profile::note_max(
                    "document_cached_chunk_peak",
                    shared_bodies.profile_cached_chunk_bytes() as u64,
                );
                value
            } else {
                read_raw_artifact(&content, &read, &record, &path, &blob_id, &mut tree_cache)?
            };
            #[cfg(not(test))]
            let artifact = {
                shared_bodies.clear_logical_values();
                read_raw_artifact_with_session(
                    &content,
                    &read,
                    &record,
                    &path,
                    &blob_id,
                    &mut shared_bodies,
                    &mut tree_cache,
                )?
            };
            #[cfg(test)]
            if profile.is_none_or(|(_, shared)| shared) {
                DOCUMENT_OPEN_HOOK.with(|slot| {
                    if let Some(hook) = slot.borrow_mut().take() {
                        hook(&mut shared_bodies);
                    }
                });
            }
            #[cfg(test)]
            {
                selected += 1;
                crate::binary_db::read_profile::note(
                    "document_revisions_validated",
                    artifact.1.len() as u64,
                );
            }
            if plans
                .insert((selector, title, path), record.plan_index)
                .is_some_and(|old| old != record.plan_index)
            {
                return Err(BinaryDbError::corruption(
                    "duplicate raw source Plan binding",
                ));
            }
            let key = (
                record.plan_index,
                artifact.0,
                artifact.1.len() as u64,
                super::records::body_sha256(&artifact.1),
            );
            if revisions.insert(key, ordinal).is_some() {
                return Err(BinaryDbError::corruption(
                    "duplicate raw Git document revision",
                ));
            }
        }
        shared_bodies.clear_logical_values();
        shared_bodies.verify_admission_stamps()?;
        tree_cache.verify_admission_stamps()?;
        if document_authority_stamps(&interop, &content)? != authority_stamps {
            return Err(BinaryDbError::retryable_busy(
                "Git document metadata authority changed during opening",
            ));
        }
        drop(read);
        Ok(Self {
            interop,
            content,
            blobs: Default::default(),
            trees: Default::default(),
            plans,
            revisions,
        })
    }

    fn index_rows(&self) -> StoreResult<DocumentIndexRows> {
        let read = BinaryDbReadTxn::new(&self.interop);
        let mut sources = BTreeMap::new();
        for ordinal in 0..read.record_count(super::store::repository_file())? {
            let (kind, fingerprint, identity) =
                super::store::repository_identity_at(&self.interop, ordinal)?;
            if kind != GitRepositoryKind::ImportSource {
                continue;
            }
            let key = (
                format!("git-source/{}", hex(&fingerprint).to_ascii_uppercase()),
                format!("Git source: {identity}"),
            );
            if sources.insert(key, ordinal).is_some() {
                return Err(BinaryDbError::corruption(
                    "duplicate Git document source identity",
                ));
            }
        }
        let mut plans = Vec::new();
        for ((selector, title, path), ordinal) in &self.plans {
            let source = sources
                .get(&(selector.clone(), title.clone()))
                .ok_or("raw Plan source repository/full identity absent")?;
            let mut key = source.to_le_bytes().to_vec();
            key.extend_from_slice(&super::records::body_sha256(path.as_bytes()));
            plans.push((key, *ordinal));
        }
        let revisions = self
            .revisions
            .iter()
            .map(|((plan, mode, length, hash), ordinal)| {
                let mut key = plan.to_le_bytes().to_vec();
                key.extend_from_slice(&mode.to_le_bytes());
                key.extend_from_slice(&length.to_le_bytes());
                key.extend_from_slice(hash);
                (key, *ordinal)
            })
            .collect();
        Ok(vec![
            ("git_document_plan.idx", 36, plans),
            ("git_document_revision.idx", 46, revisions),
        ])
    }
    fn validate_indexes(&self) -> StoreResult<()> {
        let read = BinaryDbReadTxn::new(&self.interop);
        for (name, width, rows) in self.index_rows()? {
            let length = self
                .interop
                .metadata_len(&self.interop.authority_root().as_path().join(name))?;
            if length != Some(4 + rows.len() as u64 * u64::from(width + 4))
                && !(rows.is_empty() && length.is_none())
            {
                return Err(BinaryDbError::corruption(format!(
                    "{name} requires explicit document-index recovery"
                )));
            }
            for (key, ordinal) in rows {
                if read.lookup_index(BinaryIndexId::new_fixed(name, 1, width, true), &key)?
                    != [ordinal]
                {
                    return Err(BinaryDbError::corruption(format!(
                        "{name} requires explicit document-index recovery"
                    )));
                }
            }
        }
        Ok(())
    }
    pub fn rebuild_indexes(
        interop: &LocalBinaryDbFs,
        content: &LocalContentBinaryDb<1>,
    ) -> StoreResult<()> {
        let store = Self::from_authority(interop.clone(), content.clone())?;
        for (name, width, mut rows) in store.index_rows()? {
            rows.sort_by_key(|(_, ordinal)| *ordinal);
            let mut bytes = 1u32.to_le_bytes().to_vec();
            for (key, ordinal) in rows {
                bytes.extend_from_slice(&key);
                bytes.extend_from_slice(
                    &ordinal
                        .checked_add(1)
                        .ok_or("document index ordinal overflow")?
                        .to_le_bytes(),
                );
            }
            BinaryDbWriteTxn::begin(interop, BinaryDbCommandScope::General)?
                .commit_with_atomic_file_replacement(
                    BinaryFileId::new(name, 1, width + 4),
                    &bytes,
                    "git-document-index-recovery",
                )?;
        }
        Ok(())
    }

    /// Share verified immutable pack chunks only while materializing one DocSet.
    /// The reader is dropped before returning and never crosses Git commits.
    pub fn ensure_document_set_from_objects(
        &mut self,
        objects: &LocalGitObjectStore,
        source_repository_index: u32,
        fingerprint: [u8; 12],
        source_identity: &str,
        inputs: &[RawGitDocumentObjectInput<'_>],
    ) -> StoreResult<u32> {
        if !objects.shares_authorities(&self.interop, &self.content) {
            return Err(BinaryDbError::invalid_domain_data(
                "Git document read session requires the same object/content authorities",
            ));
        }
        let mut body_read = Default::default();
        let mut bodies = Vec::with_capacity(inputs.len());
        let mut source_blob_indices = Vec::with_capacity(inputs.len());
        {
            let mut object_read = objects.read_session(&mut body_read);
            for batch in inputs.chunks(128) {
                for input in batch {
                    let (record, body) = object_read
                        .get(source_repository_index, 1, &input.object_id)?
                        .ok_or("native source document Blob absent")?;
                    if record.kind != GitObjectKind::Blob {
                        return Err(BinaryDbError::invalid_domain_data(
                            "native source document is not a Blob",
                        ));
                    }
                    source_blob_indices.push(record.content_blob_index);
                    bodies.push(body);
                }
            }
            object_read.verify_metadata_authority()?;
            object_read.body_read.verify_admission_stamps()?;
        } // Drop both immutable metadata transactions before any Plan/content write.
        let raw = inputs
            .iter()
            .zip(&bodies)
            .map(|(input, body)| RawGitDocumentInput {
                path: input.path,
                mode: input.mode,
                body,
            })
            .collect::<Vec<_>>();
        self.ensure_document_set_with_read_session(
            source_repository_index,
            fingerprint,
            source_identity,
            &raw,
            Some(&source_blob_indices),
            &mut body_read,
        )
    }

    pub fn ensure_document_set(
        &mut self,
        source_repository_index: u32,
        fingerprint: [u8; 12],
        source_identity: &str,
        inputs: &[RawGitDocumentInput<'_>],
    ) -> StoreResult<u32> {
        self.ensure_document_set_with_read_session(
            source_repository_index,
            fingerprint,
            source_identity,
            inputs,
            None,
            &mut Default::default(),
        )
    }

    fn ensure_document_set_with_read_session(
        &mut self,
        source_repository_index: u32,
        fingerprint: [u8; 12],
        source_identity: &str,
        inputs: &[RawGitDocumentInput<'_>],
        source_blob_indices: Option<&[u32]>,
        body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
    ) -> StoreResult<u32> {
        super::store::verify_source_repository(
            &self.interop,
            source_repository_index,
            fingerprint,
            source_identity,
        )?;
        let selector = format!("git-source/{}", hex(&fingerprint).to_ascii_uppercase());
        let title = format!("Git source: {source_identity}");
        let mut paths = std::collections::BTreeSet::new();
        for input in inputs {
            validate_git_artifact_path(input.path)?;
            if !input.path.to_ascii_lowercase().ends_with(".md") || !paths.insert(input.path) {
                return Err(BinaryDbError::invalid_domain_data(
                    "DocSet requires unique case-insensitive Markdown paths",
                ));
            }
            GitDocumentVersionDescriptor {
                plan_revision_index: 0,
                git_mode: input.mode,
                raw_body_length: input.body.len() as u64,
                raw_body_sha256: super::records::body_sha256(input.body),
            }
            .verify_body(input.body)?;
        }
        // Dependencies are packed before taking the Plan/content writer locks.
        let raw_indices = if let Some(indices) = source_blob_indices {
            if indices.len() != inputs.len() {
                return Err(BinaryDbError::invalid_domain_data(
                    "source document Blob count differs",
                ));
            }
            // Object reads already verified these exact bytes under the same
            // authorities. Recheck current fixed metadata after releasing the
            // source reader; no transaction, locator or body cache crosses calls.
            let read = BinaryDbReadTxn::new(self.content.db());
            self.blobs
                .refresh(&self.content, &read)
                .map_err(BinaryDbError::from)?;
            for (input, index) in inputs.iter().zip(indices) {
                let view = self.content.blobs().blob_view_at(&read, *index)?;
                if view.record.is_tombstone()
                    || view.record.is_pruned()
                    || view.size_bytes != input.body.len() as u64
                    || view.record.sha256 != super::records::body_sha256(input.body)
                {
                    return Err(BinaryDbError::corruption(
                        "verified source document Blob changed",
                    ));
                }
            }
            indices.to_vec()
        } else {
            self.content
                .ensure_blob_bytes_batch_with_read_session(
                    &inputs.iter().map(|input| input.body).collect::<Vec<_>>(),
                    &mut self.blobs,
                    body_read,
                )
                .map_err(BinaryDbError::from)?
        };
        let artifact_roots = inputs
            .iter()
            .zip(&raw_indices)
            .map(|(input, index)| vec![(input.path.to_string(), *index, input.mode)])
            .collect::<Vec<_>>();
        let roots = self
            .content
            .ensure_raw_trees(&artifact_roots, &mut self.trees)
            .map_err(BinaryDbError::from)?;
        let plan_store = BinaryDbPlanStore::<_, 1>::new(self.content.db().clone());
        let mut write =
            BinaryDbWriteTxn::begin(self.content.db(), BinaryDbCommandScope::PlanImport)?;
        let count = write.record_count(BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_file())?;
        let mut plan_bytes = 1u32.to_le_bytes().to_vec();
        {
            // The exclusive PlanImport guard keeps this file immutable while
            // staging its current roots. Use the existing validated file cache
            // only for this scan, and release it before any Plan writes.
            let mut cache = crate::binary_db::BinaryDbReadCache::default();
            for ordinal in 0..count {
                plan_bytes.extend_from_slice(&write.db().read_record_in_read_txn(
                    BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_file(),
                    ordinal,
                    &mut cache,
                )?);
            }
        }
        let now_s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "native timestamp predates epoch")?
            .as_secs();
        let mut plan_updates = Vec::new();
        let mut revision_updates = Vec::new();
        let mut selected = Vec::new();
        let mut changed = false;
        for ((input, raw_index), root) in inputs.iter().zip(&raw_indices).zip(&roots) {
            let key = (selector.clone(), title.clone(), input.path.to_string());
            let plan_index = match self.plans.get(&key).copied() {
                Some(index) => index,
                None => {
                    let index = u32::try_from((plan_bytes.len() - 4) / 48)
                        .map_err(|_| BinaryDbError::invalid_domain_data("Plan index overflow"))?;
                    let range = plan_store.append_plan_payload(
                        &mut write,
                        &PlanPayload {
                            title_bytes: title.as_bytes().to_vec(),
                        },
                    )?;
                    let record = PlanRecord {
                        actor_index_plus1: 0,
                        plan_meta: 1,
                        reserved0: 0,
                        payload_len: u16::try_from(range.payload_len)
                            .map_err(|_| "source Plan title exceeds u16")?,
                        payload_offset: range.payload_offset,
                        latest_revision_index_plus1: 0,
                        published_plan_index_plus1: 0,
                        published_latest_revision_index_plus1: 0,
                        created_at_s: now_s,
                        updated_at_s: now_s,
                        published_at_s: 0,
                    };
                    plan_bytes.extend_from_slice(&PlanCodec::<1>::encode_record(&record)?);
                    plan_updates.push((key, index));
                    changed = true;
                    index
                }
            };
            let hash = super::records::body_sha256(input.body);
            let revision_key = (plan_index, input.mode, input.body.len() as u64, hash);
            let revision = match self.revisions.get(&revision_key).copied() {
                Some(index) => index,
                None => {
                    let offset = 4 + plan_index as usize * 48;
                    let mut plan = PlanCodec::<1>::decode_record(&plan_bytes[offset..offset + 48])?;
                    let previous = plan.latest_revision_index_plus1;
                    let number = if let Some(index) = previous.checked_sub(1) {
                        crate::plan_binary_db::PlanRevisionCodec::<1>::decode_record(
                            &write.read_record(
                                BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_revision_file(),
                                index,
                            )?,
                        )?
                        .revision_number
                        .checked_add(1)
                        .ok_or("raw Plan revision number overflow")?
                    } else {
                        1
                    };
                    let blob_id = crate::content_binary_db::blob_id_from_sha256(&hash);
                    let _ = raw_index;
                    let payload = PlanRevisionPayload {
                        title_snapshot_bytes: title.as_bytes().to_vec(),
                        summary_bytes: Vec::new(),
                        artifact_path_bytes: input.path.as_bytes().to_vec(),
                        artifact_selector_bytes: selector.as_bytes().to_vec(),
                        artifact_heading_bytes: Vec::new(),
                        artifact_blob_id_bytes: blob_id.as_bytes().to_vec(),
                    };
                    let record = PlanRevisionRecord {
                        actor_index_plus1: 0,
                        revision_meta: 32,
                        reserved0: 0,
                        payload_len: 0,
                        revision_number: number,
                        item_count: 0,
                        payload_offset: 0,
                        plan_index,
                        previous_revision_index_plus1: previous,
                        item_start_index: 0,
                        published_revision_index_plus1: 0,
                        root_tree_pack_index_plus1: root
                            .tree_pack_index
                            .checked_add(1)
                            .ok_or("Tree pack index overflow")?,
                        root_entry_ordinal: root.root_entry_ordinal,
                        created_at_s: now_s,
                        published_at_s: 0,
                    };
                    let (index, _) =
                        plan_store.append_plan_revision(&mut write, record, &payload)?;
                    plan.latest_revision_index_plus1 =
                        index.checked_add(1).ok_or("revision index overflow")?;
                    plan.updated_at_s = now_s;
                    plan_bytes[offset..offset + 48]
                        .copy_from_slice(&PlanCodec::<1>::encode_record(&plan)?);
                    revision_updates.push((revision_key, index));
                    changed = true;
                    index
                }
            };
            selected.push((
                input.path.to_string(),
                GitDocumentVersionDescriptor {
                    plan_revision_index: revision,
                    git_mode: input.mode,
                    raw_body_length: input.body.len() as u64,
                    raw_body_sha256: hash,
                },
            ));
        }
        if changed {
            #[cfg(test)]
            if let Some(marker) = std::env::var_os("AIT_RAW_GIT_DOC_TEST_PAUSE") {
                for name in [
                    "plan_payload.bin",
                    "plan_revision.bin",
                    "plan_revision_payload.bin",
                ] {
                    std::fs::File::open(self.content.db().authority_root().as_path().join(name))
                        .and_then(|file| file.sync_all())
                        .map_err(|error| error.to_string())?;
                }
                std::fs::File::open(self.content.db().authority_root().as_path())
                    .and_then(|file| file.sync_all())
                    .map_err(|error| error.to_string())?;
                std::fs::write(marker, b"complete revisions before Plan root")
                    .map_err(|error| error.to_string())?;
                loop {
                    std::thread::park();
                }
            }
            write.commit_with_atomic_file_replacement(
                BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_file(),
                &plan_bytes,
                "raw Git Plan roots",
            )?;
        } else {
            write.commit()?;
        }
        drop(write);
        let mut interop_write =
            BinaryDbWriteTxn::begin(&self.interop, BinaryDbCommandScope::General)?;
        for (key, index) in &plan_updates {
            let mut bytes = source_repository_index.to_le_bytes().to_vec();
            bytes.extend_from_slice(&super::records::body_sha256(key.2.as_bytes()));
            interop_write.append_index_candidate(
                BinaryIndexId::new_fixed("git_document_plan.idx", 1, 36, true),
                &bytes,
                *index,
            )?;
        }
        for (key, index) in &revision_updates {
            let mut bytes = key.0.to_le_bytes().to_vec();
            bytes.extend_from_slice(&key.1.to_le_bytes());
            bytes.extend_from_slice(&key.2.to_le_bytes());
            bytes.extend_from_slice(&key.3);
            interop_write.append_index_candidate(
                BinaryIndexId::new_fixed("git_document_revision.idx", 1, 46, true),
                &bytes,
                *index,
            )?;
        }
        interop_write.commit()?;
        drop(interop_write);
        self.plans.extend(plan_updates);
        self.revisions.extend(revision_updates);
        let descriptor_bodies = selected
            .iter()
            .map(|(_, descriptor)| descriptor.encode().map(|body| body.to_vec()))
            .collect::<StoreResult<Vec<_>>>()?;
        let indices = self
            .content
            .ensure_blob_bytes_batch_with_read_session(
                &descriptor_bodies
                    .iter()
                    .map(Vec::as_slice)
                    .collect::<Vec<_>>(),
                &mut self.blobs,
                body_read,
            )
            .map_err(BinaryDbError::from)?;
        let leaves = selected
            .iter()
            .zip(indices)
            .map(|((path, _), index)| (path.clone(), index, 0o644))
            .collect::<Vec<_>>();
        Ok(self
            .content
            .ensure_raw_trees(&[leaves], &mut self.trees)
            .map_err(BinaryDbError::from)?[0]
            .tree_index)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_raw_artifact(
    content: &LocalContentBinaryDb<1>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    revision: &PlanRevisionRecord,
    path: &str,
    blob_id: &str,
    tree_cache: &mut crate::content_binary_db::BinaryDbTreeReadCache,
) -> StoreResult<(u16, Vec<u8>)> {
    read_raw_artifact_with_session(
        content,
        read,
        revision,
        path,
        blob_id,
        &mut Default::default(),
        tree_cache,
    )
}

fn read_raw_artifact_with_session(
    content: &LocalContentBinaryDb<1>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    revision: &PlanRevisionRecord,
    path: &str,
    blob_id: &str,
    body_read: &mut crate::content_binary_db::BinaryBlobReadSession,
    tree_cache: &mut crate::content_binary_db::BinaryDbTreeReadCache,
) -> StoreResult<(u16, Vec<u8>)> {
    let pack_index = revision
        .root_tree_pack_index()
        .ok_or_else(|| BinaryDbError::corruption("raw artifact has no packed root"))?;
    let pack =
        crate::content_binary_db::BinaryTreePackCodec::<1>::decode_record(&read.read_record(
            crate::content_binary_db::BinaryTreePackCodec::<1>::record_file(),
            pack_index,
        )?)?;
    if !pack.is_ready() || pack.is_tombstone() || revision.root_entry_ordinal >= pack.tree_count {
        return Err(BinaryDbError::corruption(
            "invalid raw artifact packed root",
        ));
    }
    let tree_index = pack
        .first_tree_index
        .checked_add(revision.root_entry_ordinal)
        .ok_or("artifact Tree ordinal overflow")?;
    let mut tree = content
        .trees()
        .tree_view_at_with_cache(read, tree_index, tree_cache)?
        .tree_id;
    let parts = path.split('/').collect::<Vec<_>>();
    let mut mode = None;
    for (position, part) in parts.iter().enumerate() {
        let entries = content
            .trees()
            .list_tree_entry_views_with_cache(read, &tree, tree_cache)?;
        if entries.len() != 1 || entries[0].entry_name != *part {
            return Err(BinaryDbError::corruption(
                "raw artifact root path/content mismatch",
            ));
        }
        let leaf = &entries[0];
        if position + 1 < parts.len() {
            if leaf.entry_type != "tree" {
                return Err(BinaryDbError::corruption("artifact directory is not Tree"));
            }
            tree = leaf.target_id.clone();
        } else {
            if leaf.entry_type != "blob" || leaf.target_id != blob_id {
                return Err(BinaryDbError::corruption("artifact leaf Blob mismatch"));
            }
            mode = Some(
                u16::from_str_radix(leaf.mode.as_deref().ok_or("artifact mode missing")?, 8)
                    .map_err(|_| "invalid artifact mode")?,
            );
        }
    }
    let blob = content
        .blobs()
        .get_blob_view(read, blob_id)?
        .ok_or("raw artifact Blob missing")?;
    let bytes = {
        content
            .blobs()
            .read_blob_bytes_for_view_with_session(read, &blob, body_read, 0, None)?
    };
    Ok((mode.ok_or("artifact mode missing")?, bytes))
}
