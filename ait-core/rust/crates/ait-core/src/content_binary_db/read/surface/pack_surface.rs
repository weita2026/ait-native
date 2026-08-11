use super::*;

impl<B, const WRITE_LAYOUT: u32> BinaryDbObjectPackStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn read_object_pack_record(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        pack_index: u32,
    ) -> StoreResult<BinaryObjectPackRecord> {
        read_object_pack_record_at::<B, WRITE_LAYOUT>(read, pack_index)
    }

    pub fn object_pack_view_at(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        pack_index: u32,
    ) -> StoreResult<BinaryObjectPackView> {
        object_pack_view_at::<B, WRITE_LAYOUT>(read, pack_index)
    }

    pub fn get_object_pack_view(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        pack_id: &str,
    ) -> StoreResult<Option<BinaryObjectPackView>> {
        get_object_pack_view_by_id::<B, WRITE_LAYOUT>(read, pack_id)
    }

    pub fn list_object_pack_views(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StoreResult<Vec<BinaryObjectPackView>> {
        let Some(layout) = persisted_content_layout(
            read,
            OBJECT_PACK_BIN,
            OBJECT_PACK_RECORD_SIZE,
            "object pack",
        )?
        else {
            return Ok(Vec::new());
        };
        let count = read.record_count(content_record_file(
            OBJECT_PACK_BIN,
            OBJECT_PACK_RECORD_SIZE,
            layout,
            "object pack",
        )?)?;
        let mut views = Vec::new();
        for pack_index in 0..count {
            let view = object_pack_view_at::<B, WRITE_LAYOUT>(read, pack_index)?;
            if !view.record.is_tombstone() {
                views.push(view);
            }
        }
        Ok(views)
    }

    pub fn read_object_pack_member_record(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        member_index: u32,
    ) -> StoreResult<BinaryObjectPackMemberRecord> {
        read_object_pack_member_record_at::<B, WRITE_LAYOUT>(read, member_index)
    }

    pub fn object_pack_member_view_at(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        member_index: u32,
    ) -> StoreResult<BinaryObjectPackMemberView> {
        object_pack_member_view_at::<B, WRITE_LAYOUT>(read, member_index)
    }

    pub fn list_object_pack_member_views(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        pack_id: &str,
    ) -> StoreResult<Vec<BinaryObjectPackMemberView>> {
        let Some(pack) = self.get_object_pack_view(read, pack_id)? else {
            return Ok(Vec::new());
        };
        let mut members = Vec::new();
        for offset in 0..pack.record.member_count {
            let member_index = pack
                .record
                .first_member_index
                .checked_add(offset)
                .ok_or_else(|| "object-pack member index overflow".to_string())?;
            let member = object_pack_member_view_at::<B, WRITE_LAYOUT>(read, member_index)?;
            if !member.record.is_tombstone() {
                members.push(member);
            }
        }
        Ok(members)
    }

    pub fn append_object_pack_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryObjectPackRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinaryObjectPackCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::object_pack_file(), &bytes)
    }

    pub fn append_object_pack_member_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryObjectPackMemberRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinaryObjectPackMemberCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::object_pack_member_file(), &bytes)
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbObjectPackStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    pub fn append_object_pack_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        pack_id: &str,
        pack_index: u32,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let key = object_pack_id_index_key(pack_id)?;
        write.append_index_candidate(Self::object_pack_id_index(), &key, pack_index)
    }

    pub fn append_object_pack_with_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryObjectPackRecord,
    ) -> StoreResult<(u32, String)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let index = self.append_object_pack_record(write, record)?;
        let pack_id = object_pack_id_from_hash48(record.pack_hash48());
        self.append_object_pack_id_index(write, &pack_id, index)?;
        Ok((index, pack_id))
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbTreePackStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn read_tree_pack_record(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_pack_index: u32,
    ) -> StoreResult<BinaryTreePackRecord> {
        read_tree_pack_record_at::<B, WRITE_LAYOUT>(read, tree_pack_index)
    }

    pub fn tree_pack_view_at(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_pack_index: u32,
    ) -> StoreResult<BinaryTreePackView> {
        tree_pack_view_at::<B, WRITE_LAYOUT>(read, tree_pack_index)
    }

    pub fn get_tree_pack_view(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        pack_id: &str,
    ) -> StoreResult<Option<BinaryTreePackView>> {
        get_tree_pack_view_by_id::<B, WRITE_LAYOUT>(read, pack_id)
    }

    pub fn list_tree_pack_views(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StoreResult<Vec<BinaryTreePackView>> {
        let Some(layout) =
            persisted_content_layout(read, TREE_PACK_BIN, TREE_PACK_RECORD_SIZE, "tree pack")?
        else {
            return Ok(Vec::new());
        };
        let count = read.record_count(content_record_file(
            TREE_PACK_BIN,
            TREE_PACK_RECORD_SIZE,
            layout,
            "tree pack",
        )?)?;
        let mut views = Vec::new();
        for tree_pack_index in 0..count {
            let view = tree_pack_view_at::<B, WRITE_LAYOUT>(read, tree_pack_index)?;
            if !view.record.is_tombstone() {
                views.push(view);
            }
        }
        Ok(views)
    }

    pub fn read_tree_payload_for_id(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_id: &str,
    ) -> StoreResult<Option<JsonValue>> {
        let mut cache = BinaryDbTreeReadCache::default();
        let Some(tree) =
            get_tree_view_by_id_with_cache::<B, WRITE_LAYOUT>(read, tree_id, &mut cache)?
        else {
            return Ok(None);
        };
        read_tree_pack_payload_at::<B, WRITE_LAYOUT>(
            read,
            self.repo_root(),
            tree.tree_index,
            &mut cache,
        )
        .map(Some)
    }

    pub fn append_tree_pack_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryTreePackRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinaryTreePackCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::tree_pack_file(), &bytes)
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbTreePackStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    pub fn append_tree_pack_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        pack_id: &str,
        tree_pack_index: u32,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let key = tree_pack_id_index_key(pack_id)?;
        write.append_index_candidate(Self::tree_pack_id_index(), &key, tree_pack_index)
    }

    pub fn append_tree_pack_with_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryTreePackRecord,
    ) -> StoreResult<(u32, String)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let index = self.append_tree_pack_record(write, record)?;
        let pack_id = tree_pack_id_from_hash48(record.pack_hash48());
        self.append_tree_pack_id_index(write, &pack_id, index)?;
        Ok((index, pack_id))
    }
}

impl<B, const WRITE_LAYOUT: u32> ObjectPackStore for BinaryDbObjectPackStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn get_object_pack(&self, pack_id: &str) -> ContentStoreResult<Option<ObjectPackRecord>> {
        let read = self.begin_read_txn();
        Ok(self
            .get_object_pack_view(&read, pack_id)?
            .map(object_pack_record_from_view)
            .transpose()?)
    }

    fn list_object_pack_ids(&self) -> ContentStoreResult<Vec<String>> {
        let read = self.begin_read_txn();
        Ok(self
            .list_object_pack_views(&read)?
            .into_iter()
            .map(|view| view.pack_id)
            .collect())
    }

    fn list_object_pack_members(
        &self,
        pack_id: &str,
    ) -> ContentStoreResult<Vec<ObjectPackMemberRecord>> {
        let read = self.begin_read_txn();
        Ok(self
            .list_object_pack_member_views(&read, pack_id)?
            .into_iter()
            .map(object_pack_member_record_from_view)
            .collect::<StoreResult<Vec<_>>>()?)
    }

    fn record_object_pack(
        &self,
        _input: RecordObjectPackInput<'_>,
    ) -> ContentStoreResult<ObjectPackRecord> {
        Err(
            "BinaryDbObjectPackStore::record_object_pack requires explicit Binary DB pack metadata"
                .to_string(),
        )
    }
}

impl<B, const WRITE_LAYOUT: u32> TreePackStore for BinaryDbTreePackStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn get_tree_pack(&self, pack_id: &str) -> ContentStoreResult<Option<TreePackRecord>> {
        let read = self.begin_read_txn();
        Ok(self
            .get_tree_pack_view(&read, pack_id)?
            .map(tree_pack_record_from_view)
            .transpose()?)
    }

    fn read_tree_payload(&self, tree_id: &str) -> ContentStoreResult<Option<JsonValue>> {
        let read = self.begin_read_txn();
        Ok(self.read_tree_payload_for_id(&read, tree_id)?)
    }

    fn record_tree_pack(
        &self,
        _input: RecordTreePackInput<'_>,
    ) -> ContentStoreResult<TreePackRecord> {
        Err("BinaryDbTreePackStore::record_tree_pack requires explicit Binary DB tree-pack metadata".to_string())
    }
}

pub(super) fn read_object_pack_record_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    pack_index: u32,
) -> StoreResult<BinaryObjectPackRecord>
where
    B: BinaryDb,
{
    let layout = required_content_layout(
        read,
        OBJECT_PACK_BIN,
        OBJECT_PACK_RECORD_SIZE,
        "object pack",
    )?;
    let raw = read.read_record(
        content_record_file(
            OBJECT_PACK_BIN,
            OBJECT_PACK_RECORD_SIZE,
            layout,
            "object pack",
        )?,
        pack_index,
    )?;
    decode_object_pack_record(layout, &raw)
}

pub(super) fn object_pack_view_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    pack_index: u32,
) -> StoreResult<BinaryObjectPackView>
where
    B: BinaryDb,
{
    let record = read_object_pack_record_at::<B, WRITE_LAYOUT>(read, pack_index)?;
    let pack_id = object_pack_id_from_hash48(record.pack_hash48());
    let pack_format = object_pack_format_name(record.format_kind())?.to_string();
    let pack_path = object_pack_relative_path(&pack_id, &pack_format)?;
    Ok(BinaryObjectPackView {
        pack_index,
        record,
        pack_id,
        pack_path,
        pack_format,
    })
}

pub(super) fn get_object_pack_view_by_id<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    pack_id: &str,
) -> StoreResult<Option<BinaryObjectPackView>>
where
    B: BinaryDb,
{
    let key = object_pack_id_index_key(pack_id)?;
    let Some(layout) = persisted_content_layout(
        read,
        OBJECT_PACK_BIN,
        OBJECT_PACK_RECORD_SIZE,
        "object pack",
    )?
    else {
        return Ok(None);
    };
    for index in index_candidates(
        read,
        content_index(OBJECT_PACK_ID_IDX, layout, "object pack", Some((8, true)))?,
        &key,
    )? {
        let view = object_pack_view_at::<B, WRITE_LAYOUT>(read, index)?;
        if view.pack_id.eq_ignore_ascii_case(pack_id) && !view.record.is_tombstone() {
            return Ok(Some(view));
        }
    }
    Ok(None)
}

pub(super) fn read_object_pack_member_record_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    member_index: u32,
) -> StoreResult<BinaryObjectPackMemberRecord>
where
    B: BinaryDb,
{
    let layout = required_content_layout(
        read,
        OBJECT_PACK_MEMBER_BIN,
        OBJECT_PACK_MEMBER_RECORD_SIZE,
        "object pack member",
    )?;
    let raw = read.read_record(
        content_record_file(
            OBJECT_PACK_MEMBER_BIN,
            OBJECT_PACK_MEMBER_RECORD_SIZE,
            layout,
            "object pack member",
        )?,
        member_index,
    )?;
    decode_object_pack_member_record(layout, &raw)
}

pub(super) fn object_pack_member_view_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    member_index: u32,
) -> StoreResult<BinaryObjectPackMemberView>
where
    B: BinaryDb,
{
    let record = read_object_pack_member_record_at::<B, WRITE_LAYOUT>(read, member_index)?;
    let pack = object_pack_view_at::<B, WRITE_LAYOUT>(read, record.pack_index)?;
    let blob = blob_view_at::<B, WRITE_LAYOUT>(read, record.blob_index)?;
    let base_blob_id = record
        .base_blob_index()
        .map(|index| blob_view_at::<B, WRITE_LAYOUT>(read, index).map(|view| view.blob_id))
        .transpose()?;
    Ok(BinaryObjectPackMemberView {
        member_index,
        entry_name: format!("blobs/{}", blob.blob_id),
        pack_id: pack.pack_id,
        blob_id: blob.blob_id,
        base_blob_id,
        record,
    })
}

pub(super) fn read_tree_pack_record_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_pack_index: u32,
) -> StoreResult<BinaryTreePackRecord>
where
    B: BinaryDb,
{
    let layout = required_content_layout(read, TREE_PACK_BIN, TREE_PACK_RECORD_SIZE, "tree pack")?;
    let raw = read.read_record(
        content_record_file(TREE_PACK_BIN, TREE_PACK_RECORD_SIZE, layout, "tree pack")?,
        tree_pack_index,
    )?;
    decode_tree_pack_record(layout, &raw)
}

pub(super) fn tree_pack_view_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_pack_index: u32,
) -> StoreResult<BinaryTreePackView>
where
    B: BinaryDb,
{
    let record = read_tree_pack_record_at::<B, WRITE_LAYOUT>(read, tree_pack_index)?;
    let pack_id = tree_pack_id_from_hash48(record.pack_hash48());
    let pack_format = tree_pack_format_name(record.format_kind())?.to_string();
    let pack_path = tree_pack_relative_path(&pack_id, &pack_format)?;
    Ok(BinaryTreePackView {
        tree_pack_index,
        record,
        pack_id,
        pack_path,
        pack_format,
    })
}

pub(super) fn get_tree_pack_view_by_id<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    pack_id: &str,
) -> StoreResult<Option<BinaryTreePackView>>
where
    B: BinaryDb,
{
    let key = tree_pack_id_index_key(pack_id)?;
    let Some(layout) =
        persisted_content_layout(read, TREE_PACK_BIN, TREE_PACK_RECORD_SIZE, "tree pack")?
    else {
        return Ok(None);
    };
    for index in index_candidates(
        read,
        content_index(TREE_PACK_ID_IDX, layout, "tree pack", Some((8, true)))?,
        &key,
    )? {
        let view = tree_pack_view_at::<B, WRITE_LAYOUT>(read, index)?;
        if view.pack_id.eq_ignore_ascii_case(pack_id) && !view.record.is_tombstone() {
            return Ok(Some(view));
        }
    }
    Ok(None)
}

pub(super) fn tree_pack_view_for_tree_index<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_index: u32,
) -> StoreResult<Option<BinaryTreePackView>>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.tree_pack.locate_for_tree");
    let Some(layout) =
        persisted_content_layout(read, TREE_PACK_BIN, TREE_PACK_RECORD_SIZE, "tree pack")?
    else {
        return Ok(None);
    };
    let count = {
        let _range = crate::perfetto_range!("ait.core.tree_pack.locate_for_tree.record_count");
        read.record_count(content_record_file(
            TREE_PACK_BIN,
            TREE_PACK_RECORD_SIZE,
            layout,
            "tree pack",
        )?)?
    };
    let _scan_range = crate::perfetto_range!("ait.core.tree_pack.locate_for_tree.reverse_scan");
    for tree_pack_index in (0..count).rev() {
        let view = tree_pack_view_at::<B, WRITE_LAYOUT>(read, tree_pack_index)?;
        if view.record.is_tombstone() {
            continue;
        }
        let start = view.record.first_tree_index;
        let end = start
            .checked_add(view.record.tree_count)
            .ok_or_else(|| "tree-pack range overflow".to_string())?;
        if (start..end).contains(&tree_index) {
            return Ok(Some(view));
        }
    }
    Ok(None)
}

pub(super) fn tree_pack_view_for_tree_index_with_cache<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_index: u32,
    cache: &mut BinaryDbTreeReadCache,
) -> StoreResult<Option<BinaryTreePackView>>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.tree_pack.locate_for_tree_cached");
    if !cache.tree_pack_index_loaded {
        let _range = crate::perfetto_range!("ait.core.tree_pack.locator_cache_build");
        let Some(layout) =
            persisted_content_layout(read, TREE_PACK_BIN, TREE_PACK_RECORD_SIZE, "tree pack")?
        else {
            cache.tree_pack_index_loaded = true;
            return Ok(None);
        };
        let tree_layout = required_content_layout(read, TREE_BIN, TREE_RECORD_SIZE, "tree")?;
        let tree_count = read.record_count(content_record_file(
            TREE_BIN,
            TREE_RECORD_SIZE,
            tree_layout,
            "tree",
        )?)?;
        let pack_count = read.record_count(content_record_file(
            TREE_PACK_BIN,
            TREE_PACK_RECORD_SIZE,
            layout,
            "tree pack",
        )?)?;
        let tree_count = usize::try_from(tree_count)
            .map_err(|_| BinaryDbError::corruption("tree record count overflows usize"))?;
        let mut views = Vec::new();
        let mut locators = vec![None; tree_count];
        for tree_pack_index in 0..pack_count {
            let view = tree_pack_view_at::<B, WRITE_LAYOUT>(read, tree_pack_index)?;
            if view.record.is_tombstone() {
                continue;
            }
            let start = usize::try_from(view.record.first_tree_index)
                .map_err(|_| BinaryDbError::corruption("tree-pack start overflows usize"))?;
            let end = view
                .record
                .first_tree_index
                .checked_add(view.record.tree_count)
                .ok_or_else(|| BinaryDbError::corruption("tree-pack range overflow"))?;
            let end = usize::try_from(end)
                .map_err(|_| BinaryDbError::corruption("tree-pack end overflows usize"))?;
            if end > locators.len() {
                return Err(BinaryDbError::corruption(format!(
                    "tree-pack range {start}..{end} exceeds tree record count {}",
                    locators.len()
                )));
            }
            let view_index = views.len();
            for locator in &mut locators[start..end] {
                *locator = Some(view_index);
            }
            views.push(view);
        }
        cache.tree_pack_views = views;
        cache.tree_pack_for_tree_index = locators;
        cache.tree_pack_index_loaded = true;
    }
    let tree_index = usize::try_from(tree_index)
        .map_err(|_| BinaryDbError::corruption("tree index overflows usize"))?;
    let Some(view_index) = cache
        .tree_pack_for_tree_index
        .get(tree_index)
        .copied()
        .flatten()
    else {
        return Ok(None);
    };
    Ok(cache.tree_pack_views.get(view_index).cloned())
}

pub(super) fn object_pack_record_from_view(
    view: BinaryObjectPackView,
) -> StoreResult<ObjectPackRecord> {
    let status = if view.record.is_tombstone() {
        "tombstone"
    } else if view.record.is_ready() {
        "ready"
    } else {
        "pending"
    };
    Ok(ObjectPackRecord {
        pack_id: view.pack_id,
        pack_path: view.pack_path,
        pack_format: view.pack_format,
        member_count: i64::from(view.record.member_count),
        total_bytes: i64::try_from(view.record.total_bytes).map_err(|_| {
            format!(
                "object pack total_bytes overflows i64: {}",
                view.record.total_bytes
            )
        })?,
        index_entry_name: None,
        index_checksum: None,
        created_at: (view.record.created_at_s != 0).then(|| view.record.created_at_s.to_string()),
        status: Some(status.to_string()),
    })
}

pub(super) fn object_pack_member_record_from_view(
    view: BinaryObjectPackMemberView,
) -> StoreResult<ObjectPackMemberRecord> {
    Ok(ObjectPackMemberRecord {
        pack_id: view.pack_id,
        blob_id: view.blob_id,
        entry_name: view.entry_name,
        base_blob_id: view.base_blob_id,
    })
}

pub(super) fn tree_pack_record_from_view(view: BinaryTreePackView) -> StoreResult<TreePackRecord> {
    let status = if view.record.is_tombstone() {
        "tombstone"
    } else if view.record.is_ready() {
        "ready"
    } else {
        "pending"
    };
    Ok(TreePackRecord {
        pack_id: view.pack_id,
        pack_path: view.pack_path,
        pack_format: view.pack_format,
        entry_count: Some(i64::from(view.record.tree_count)),
        checksum: None,
        status: Some(status.to_string()),
    })
}
