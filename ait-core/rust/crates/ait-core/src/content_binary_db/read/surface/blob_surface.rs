use super::*;

#[derive(Default)]
struct BinaryBlobReadSession {
    pack_readers: BTreeMap<String, crate::pack_substrate::PackEntryArchive>,
    resolved_blobs: BTreeMap<String, Vec<u8>>,
    resolving_blobs: BTreeSet<String>,
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbBlobStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn read_blob_record(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        blob_index: u32,
    ) -> StoreResult<BinaryBlobRecord> {
        read_blob_record_at::<B, WRITE_LAYOUT>(read, blob_index)
    }

    pub fn blob_view_at(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        blob_index: u32,
    ) -> StoreResult<BinaryBlobView> {
        blob_view_at::<B, WRITE_LAYOUT>(read, blob_index)
    }

    pub fn get_blob_view(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        blob_id: &str,
    ) -> StoreResult<Option<BinaryBlobView>> {
        get_blob_view_by_id::<B, WRITE_LAYOUT>(read, blob_id)
    }

    pub fn legacy_physical_blob_index_by_id(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StoreResult<BTreeMap<String, u32>> {
        let count = read.record_count(Self::blob_file())?;
        let mut indexes = BTreeMap::new();
        for blob_index in 0..count {
            let view = self.blob_view_at(read, blob_index)?;
            if view.record.is_tombstone() {
                continue;
            }
            let key = view.blob_id.to_ascii_lowercase();
            if let Some(existing_index) = indexes.get(&key).copied() {
                let existing = self.blob_view_at(read, existing_index)?;
                if existing.record.sha256 != view.record.sha256
                    || existing.size_bytes != view.size_bytes
                {
                    return Err(format!(
                        "legacy physical Blob identity {} has conflicting records {} and {}",
                        view.blob_id, existing_index, blob_index
                    )
                    .into());
                }
                continue;
            }
            indexes.insert(key, blob_index);
        }
        Ok(indexes)
    }

    pub fn read_blob_bytes_for_id(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        blob_id: &str,
    ) -> StoreResult<Option<Vec<u8>>> {
        let mut session = BinaryBlobReadSession::default();
        self.read_blob_bytes_for_id_with_session(read, blob_id, &mut session, 0, None)
    }

    pub fn read_blob_bytes_for_id_with_legacy_physical_catalog(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        blob_index_by_id: &BTreeMap<String, u32>,
        blob_id: &str,
    ) -> StoreResult<Option<Vec<u8>>> {
        let mut session = BinaryBlobReadSession::default();
        self.read_blob_bytes_for_id_with_session(
            read,
            blob_id,
            &mut session,
            0,
            Some(blob_index_by_id),
        )
    }

    fn read_blob_bytes_for_id_with_session(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        blob_id: &str,
        session: &mut BinaryBlobReadSession,
        depth: usize,
        legacy_blob_index_by_id: Option<&BTreeMap<String, u32>>,
    ) -> StoreResult<Option<Vec<u8>>> {
        if let Some(bytes) = session.resolved_blobs.get(blob_id) {
            return Ok(Some(bytes.clone()));
        }
        let view = if let Some(indexes) = legacy_blob_index_by_id {
            indexes
                .get(&blob_id.to_ascii_lowercase())
                .copied()
                .map(|blob_index| self.blob_view_at(read, blob_index))
                .transpose()?
        } else {
            self.get_blob_view(read, blob_id)?
        };
        let Some(view) = view else {
            return Ok(None);
        };
        self.read_blob_bytes_for_view_with_session(
            read,
            &view,
            session,
            depth,
            legacy_blob_index_by_id,
        )
        .map(Some)
    }

    pub fn read_blob_bytes_for_view(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        view: &BinaryBlobView,
        depth: usize,
    ) -> StoreResult<Vec<u8>> {
        let mut session = BinaryBlobReadSession::default();
        self.read_blob_bytes_for_view_with_session(read, view, &mut session, depth, None)
    }

    fn read_blob_bytes_for_view_with_session(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        view: &BinaryBlobView,
        session: &mut BinaryBlobReadSession,
        depth: usize,
        legacy_blob_index_by_id: Option<&BTreeMap<String, u32>>,
    ) -> StoreResult<Vec<u8>> {
        if let Some(bytes) = session.resolved_blobs.get(&view.blob_id) {
            return Ok(bytes.clone());
        }
        if depth > crate::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH {
            return Err(format!(
                "Binary DB blob delta chain exceeded safety read limit {} for {}",
                crate::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH,
                view.blob_id
            )
            .into());
        }
        if !session.resolving_blobs.insert(view.blob_id.clone()) {
            return Err(format!("cyclic Binary DB blob delta chain for {}", view.blob_id).into());
        }
        let resolved =
            self.resolve_blob_bytes_for_view(read, view, session, depth, legacy_blob_index_by_id);
        session.resolving_blobs.remove(&view.blob_id);
        let bytes = resolved?;
        session
            .resolved_blobs
            .insert(view.blob_id.clone(), bytes.clone());
        Ok(bytes)
    }

    fn resolve_blob_bytes_for_view(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        view: &BinaryBlobView,
        session: &mut BinaryBlobReadSession,
        depth: usize,
        legacy_blob_index_by_id: Option<&BTreeMap<String, u32>>,
    ) -> StoreResult<Vec<u8>> {
        if view.record.is_tombstone() {
            return Err(format!("blob {} is tombstoned", view.blob_id).into());
        }
        if view.record.is_pruned() {
            return Err(format!("blob {} payload has been pruned", view.blob_id).into());
        }
        let Some(member_index) = view.record.pack_member_index() else {
            return Err(format!("blob {} has no object-pack member pointer", view.blob_id).into());
        };
        let member = object_pack_member_view_at::<B, WRITE_LAYOUT>(read, member_index)?;
        if member.record.is_tombstone() {
            return Err(format!("object-pack member {member_index} is tombstoned").into());
        }
        if member.blob_id != view.blob_id {
            return Err(format!(
                "blob {} points to object-pack member for {}",
                view.blob_id, member.blob_id
            )
            .into());
        }
        if usize::from(member.record.delta_chain_depth)
            > crate::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH
        {
            return Err(format!(
                "Binary DB blob {} persisted delta chain depth {} exceeds safety read limit {}",
                view.blob_id,
                member.record.delta_chain_depth,
                crate::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH
            )
            .into());
        }
        let pack = object_pack_view_at::<B, WRITE_LAYOUT>(read, member.record.pack_index)?;
        let pack_path = absolute_repo_path(self.repo_root(), &pack.pack_path)?;
        let pack_path_text = path_to_str(&pack_path)?.to_string();
        let pack_reader_key = format!("{}\n{}", pack.pack_format, pack_path_text);
        if !session.pack_readers.contains_key(&pack_reader_key) {
            let reader = crate::pack_substrate::PackEntryArchive::open_with_format(
                &pack_path_text,
                &pack.pack_format,
            )?;
            session.pack_readers.insert(pack_reader_key.clone(), reader);
        }
        let mut base_map = BTreeMap::new();
        if let Some(base_blob_id) = member.base_blob_id.as_ref() {
            if !base_blob_id.is_empty() {
                let base_entry_name = format!("blobs/{base_blob_id}");
                let base_is_in_same_pack = session
                    .pack_readers
                    .get_mut(&pack_reader_key)
                    .ok_or_else(|| format!("missing object-pack reader for {}", pack.pack_id))?
                    .has_entry(&base_entry_name);
                if !base_is_in_same_pack {
                    if let Some(base) = self.read_blob_bytes_for_id_with_session(
                        read,
                        base_blob_id,
                        session,
                        depth + 1,
                        legacy_blob_index_by_id,
                    )? {
                        base_map.insert(base_blob_id.clone(), base);
                    }
                }
            }
        }
        let bytes = session
            .pack_readers
            .get_mut(&pack_reader_key)
            .ok_or_else(|| format!("missing object-pack reader for {}", pack.pack_id))?
            .read_entry(
                &member.entry_name,
                (!base_map.is_empty()).then_some(&base_map),
                crate::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH,
            )?;
        if u64::try_from(bytes.len()).ok() != Some(view.size_bytes) {
            return Err(format!(
                "blob {} size mismatch: expected {}, got {}",
                view.blob_id,
                view.size_bytes,
                bytes.len()
            )
            .into());
        }
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        if digest.as_slice() != view.record.sha256 {
            return Err(format!("blob {} sha256 mismatch", view.blob_id).into());
        }
        Ok(bytes)
    }

    pub fn append_blob_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryBlobRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinaryBlobCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::blob_file(), &bytes)
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbBlobStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    pub fn append_blob_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        blob_id: &str,
        blob_index: u32,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let key = blob_id_index_key(blob_id)?;
        write.append_index_candidate(Self::blob_id_index(), &key, blob_index)
    }

    pub fn append_blob_with_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryBlobRecord,
    ) -> StoreResult<(u32, String)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let index = self.append_blob_record(write, record)?;
        let blob_id = blob_id_from_sha256(&record.sha256);
        self.append_blob_id_index(write, &blob_id, index)?;
        Ok((index, blob_id))
    }
}

impl<B, const WRITE_LAYOUT: u32> BlobReader for BinaryDbBlobStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Option<Vec<u8>>, String> {
        let read = self.begin_read_txn();
        Ok(self.read_blob_bytes_for_id(&read, blob_id)?)
    }

    fn read_blob_bytes_batch(
        &self,
        blob_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let read = self.begin_read_txn();
        let mut session = BinaryBlobReadSession::default();
        let mut payload = BTreeMap::new();
        for blob_id in blob_ids {
            if payload.contains_key(blob_id) {
                continue;
            }
            if let Some(bytes) =
                self.read_blob_bytes_for_id_with_session(&read, blob_id, &mut session, 0, None)?
            {
                payload.insert(blob_id.clone(), bytes);
            }
        }
        Ok(payload)
    }
}

impl<B, const WRITE_LAYOUT: u32> BlobStore for BinaryDbBlobStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn get_blob(&self, blob_id: &str) -> ContentStoreResult<Option<BlobRecord>> {
        let read = self.begin_read_txn();
        let Some(view) = self.get_blob_view(&read, blob_id)? else {
            return Ok(None);
        };
        let mut object_pack_locator = None;
        let mut pack_entry_name = None;
        let mut base_blob_id = None;
        let mut pack_entry_type = None;
        let mut pack_chain_depth = None;
        if let Some(member_index) = view.pack_member_index {
            let member = object_pack_member_view_at::<B, WRITE_LAYOUT>(&read, member_index)?;
            object_pack_locator = Some(ObjectPackLocator {
                pack_id: member.pack_id,
            });
            pack_entry_name = Some(member.entry_name);
            base_blob_id = member.base_blob_id;
            pack_entry_type = Some(match member.record.member_kind() {
                BinaryObjectPackMemberKind::Full => "full".to_string(),
                BinaryObjectPackMemberKind::Delta => "delta".to_string(),
                BinaryObjectPackMemberKind::Reserved(value) => format!("reserved:{value}"),
            });
            pack_chain_depth = Some(i64::from(member.record.delta_chain_depth));
        }
        Ok(Some(BlobRecord {
            blob_id: view.blob_id,
            sha256: view.sha256,
            size_bytes: i64::try_from(view.size_bytes)
                .map_err(|_| format!("blob size overflows i64: {}", view.size_bytes))?,
            object_pack_locator,
            pack_entry_name,
            base_blob_id,
            pack_entry_type,
            pack_chain_depth,
        }))
    }

    fn read_blob_bytes(&self, blob_id: &str) -> ContentStoreResult<Vec<u8>> {
        let read = self.begin_read_txn();
        self.read_blob_bytes_for_id(&read, blob_id)?
            .ok_or_else(|| format!("blob `{blob_id}` was not found"))
    }

    fn ensure_blob_bytes(&self, _input: EnsureBlobInput<'_>) -> ContentStoreResult<BlobRecord> {
        Err("BinaryDbBlobStore::ensure_blob_bytes requires an explicit object-pack ingest transaction".to_string())
    }
}

pub(super) fn read_blob_record_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    blob_index: u32,
) -> StoreResult<BinaryBlobRecord>
where
    B: BinaryDb,
{
    let layout = required_content_layout(read, BLOB_BIN, BLOB_RECORD_SIZE, "blob")?;
    let raw = read.read_record(
        content_record_file(BLOB_BIN, BLOB_RECORD_SIZE, layout, "blob")?,
        blob_index,
    )?;
    decode_blob_record(layout, &raw)
}

pub(super) fn blob_view_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    blob_index: u32,
) -> StoreResult<BinaryBlobView>
where
    B: BinaryDb,
{
    let record = read_blob_record_at::<B, WRITE_LAYOUT>(read, blob_index)?;
    Ok(BinaryBlobView {
        blob_index,
        blob_id: blob_id_from_sha256(&record.sha256),
        sha256: hex_lower(&record.sha256),
        size_bytes: record.size_bytes,
        pack_member_index: record.pack_member_index(),
        record,
    })
}

pub(super) fn get_blob_view_by_id<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    blob_id: &str,
) -> StoreResult<Option<BinaryBlobView>>
where
    B: BinaryDb,
{
    let key = blob_id_index_key(blob_id)?;
    let Some(layout) = persisted_content_layout(read, BLOB_BIN, BLOB_RECORD_SIZE, "blob")? else {
        return Ok(None);
    };
    for index in index_candidates(
        read,
        content_index(BLOB_ID_IDX, layout, "blob", Some((10, true)))?,
        &key,
    )? {
        let view = blob_view_at::<B, WRITE_LAYOUT>(read, index)?;
        if view.blob_id.eq_ignore_ascii_case(blob_id) && !view.record.is_tombstone() {
            return Ok(Some(view));
        }
    }
    Ok(None)
}
