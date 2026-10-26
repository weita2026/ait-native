use super::*;

#[derive(Default)]
pub struct BinaryDbTreeReadCache {
    archives: BTreeMap<String, TreePackEntryArchive>,
    tree_entries: BTreeMap<u32, Vec<BinaryTreeEntryView>>,
    pub(super) tree_pack_views: Vec<BinaryTreePackView>,
    pub(super) tree_pack_for_tree_index: Vec<Option<usize>>,
    pub(super) tree_pack_index_loaded: bool,
}

impl BinaryDbTreeReadCache {
    pub fn clear_tree_entries(&mut self) {
        self.tree_entries.clear();
    }

    pub fn clear_archives(&mut self) {
        self.archives.clear();
    }
}

#[cfg(test)]
impl BinaryDbTreeReadCache {
    pub(crate) fn archive_count(&self) -> usize {
        self.archives.len()
    }

    pub(crate) fn cached_zstd_chunk_count(&self) -> usize {
        self.archives
            .values()
            .map(TreePackEntryArchive::cached_zstd_chunk_count)
            .sum()
    }

    pub(crate) fn cached_tree_pack_count(&self) -> usize {
        self.tree_pack_views.len()
    }

    pub(crate) fn cached_tree_entry_count(&self) -> usize {
        self.tree_entries.len()
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbTreeStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn read_tree_record(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_index: u32,
    ) -> StoreResult<BinaryTreeRecord> {
        read_tree_record_at::<B, WRITE_LAYOUT>(read, tree_index)
    }

    pub fn tree_view_at(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_index: u32,
    ) -> StoreResult<BinaryTreeView> {
        tree_view_at::<B, WRITE_LAYOUT>(read, tree_index)
    }

    pub fn tree_view_at_with_cache(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_index: u32,
        cache: &mut BinaryDbTreeReadCache,
    ) -> StoreResult<BinaryTreeView> {
        tree_view_at_with_cache::<B, WRITE_LAYOUT>(read, tree_index, cache)
    }

    pub fn get_tree_view(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_id: &str,
    ) -> StoreResult<Option<BinaryTreeView>> {
        get_tree_view_by_id::<B, WRITE_LAYOUT>(read, tree_id)
    }

    pub fn existing_tree_ids(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StoreResult<BTreeSet<String>> {
        let Some(layout) = persisted_content_layout(read, TREE_BIN, TREE_RECORD_SIZE, "tree")?
        else {
            return Ok(BTreeSet::new());
        };
        let tree_file = content_record_file(TREE_BIN, TREE_RECORD_SIZE, layout, "tree")?;
        let count = read.record_count(tree_file.clone())?;
        let mut tree_ids = BTreeSet::new();
        let mut cache = BinaryDbTreeReadCache::default();
        for tree_index in 0..count {
            let view = self.tree_view_at_with_cache(read, tree_index, &mut cache)?;
            if !view.record.is_tombstone() && view.tree_pack_id.is_some() {
                tree_ids.insert(view.tree_id);
            }
        }
        Ok(tree_ids)
    }

    pub fn list_tree_entry_views(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_id: &str,
    ) -> StoreResult<Vec<BinaryTreeEntryView>> {
        let mut cache = BinaryDbTreeReadCache::default();
        let Some(tree) =
            get_tree_view_by_id_with_cache::<B, WRITE_LAYOUT>(read, tree_id, &mut cache)?
        else {
            return Ok(Vec::new());
        };
        list_tree_entry_views_at::<B, WRITE_LAYOUT>(
            read,
            self.repo_root(),
            tree.tree_index,
            &mut cache,
        )
    }

    pub fn list_tree_entry_views_with_cache(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        tree_id: &str,
        cache: &mut BinaryDbTreeReadCache,
    ) -> StoreResult<Vec<BinaryTreeEntryView>> {
        let Some(tree) = get_tree_view_by_id_with_cache::<B, WRITE_LAYOUT>(read, tree_id, cache)?
        else {
            return Ok(Vec::new());
        };
        list_tree_entry_views_at::<B, WRITE_LAYOUT>(read, self.repo_root(), tree.tree_index, cache)
    }

    pub fn list_tree_entry_views_for_record_in_pack_with_cache(
        &self,
        _read: &BinaryDbReadTxn<'_, B>,
        tree_index: u32,
        tree: &BinaryTreeRecord,
        tree_pack: &BinaryTreePackView,
        cache: &mut BinaryDbTreeReadCache,
    ) -> StoreResult<Vec<BinaryTreeEntryView>> {
        let payload = read_tree_pack_payload_for_record(
            self.repo_root(),
            tree_index,
            tree,
            tree_pack,
            cache,
        )?;
        tree_entry_views_from_payload(&payload)
    }

    pub fn read_tree_payload_json(
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

    pub fn append_tree_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryTreeRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinaryTreeCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::tree_file(), &bytes)
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbTreeStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    pub fn append_tree_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        tree_id: &str,
        tree_index: u32,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let key = tree_id_index_key(tree_id)?;
        write.append_index_candidate(Self::tree_id_index(), &key, tree_index)
    }

    pub fn append_tree_with_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryTreeRecord,
    ) -> StoreResult<(u32, String)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let index = self.append_tree_record(write, record)?;
        let tree_id = tree_id_from_hash80(&record.tree_hash80);
        self.append_tree_id_index(write, &tree_id, index)?;
        Ok((index, tree_id))
    }
}

impl<B, const WRITE_LAYOUT: u32> TreeStore for BinaryDbTreeStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn get_tree(&self, tree_id: &str) -> ContentStoreResult<Option<TreeRecord>> {
        let read = self.begin_read_txn();
        Ok(self
            .get_tree_view(&read, tree_id)?
            .map(tree_record_from_view)
            .transpose()?)
    }

    fn list_tree_entries(&self, tree_id: &str) -> ContentStoreResult<Vec<TreeEntryRecord>> {
        let read = self.begin_read_txn();
        Ok(self
            .list_tree_entry_views(&read, tree_id)?
            .into_iter()
            .map(tree_entry_record_from_view)
            .collect::<StoreResult<Vec<_>>>()?)
    }

    fn snapshot_root_entries(&self, snapshot_id: &str) -> ContentStoreResult<Vec<TreeEntryRecord>> {
        Err(format!(
            "BinaryDbTreeStore has no snapshot root mapping for `{snapshot_id}`; use BinaryDbSnapshotReader"
        ))
    }

    fn snapshot_path_blob(
        &self,
        snapshot_id: &str,
        path: &RepoPath,
    ) -> ContentStoreResult<Option<BlobRecord>> {
        Err(format!(
            "BinaryDbTreeStore has no snapshot path index for `{snapshot_id}` and `{}`; use a snapshot root adapter",
            path.0
        ))
    }

    fn record_tree(&self, _input: RecordTreeInput<'_>) -> ContentStoreResult<TreeRecord> {
        Err("BinaryDbTreeStore::record_tree requires explicit Binary DB tree metadata".to_string())
    }
}

pub(super) fn read_tree_record_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_index: u32,
) -> StoreResult<BinaryTreeRecord>
where
    B: BinaryDb,
{
    let layout = required_content_layout(read, TREE_BIN, TREE_RECORD_SIZE, "tree")?;
    let raw = read.read_record(
        content_record_file(TREE_BIN, TREE_RECORD_SIZE, layout, "tree")?,
        tree_index,
    )?;
    decode_tree_record(layout, &raw)
}

pub(super) fn tree_view_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_index: u32,
) -> StoreResult<BinaryTreeView>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.tree.view");
    let record = {
        let _range = crate::perfetto_range!("ait.core.tree.view.record_read");
        read_tree_record_at::<B, WRITE_LAYOUT>(read, tree_index)?
    };
    let tree_pack_id = {
        let _range = crate::perfetto_range!("ait.core.tree.view.pack_locator");
        tree_pack_view_for_tree_index::<B, WRITE_LAYOUT>(read, tree_index)?.map(|view| view.pack_id)
    };
    Ok(BinaryTreeView {
        tree_index,
        tree_id: tree_id_from_hash80(&record.tree_hash80),
        tree_pack_id,
        record,
    })
}

pub(super) fn tree_view_at_with_cache<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_index: u32,
    cache: &mut BinaryDbTreeReadCache,
) -> StoreResult<BinaryTreeView>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.tree.view_cached");
    let record = read_tree_record_at::<B, WRITE_LAYOUT>(read, tree_index)?;
    let tree_pack_id = {
        let _range = crate::perfetto_range!("ait.core.tree.view_cached.pack_locator");
        tree_pack_view_for_tree_index_with_cache::<B, WRITE_LAYOUT>(read, tree_index, cache)?
            .map(|view| view.pack_id)
    };
    Ok(BinaryTreeView {
        tree_index,
        tree_id: tree_id_from_hash80(&record.tree_hash80),
        tree_pack_id,
        record,
    })
}

pub(super) fn get_tree_view_by_id<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_id: &str,
) -> StoreResult<Option<BinaryTreeView>>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.tree.get_by_id");
    let key = tree_id_index_key(tree_id)?;
    let Some(layout) = persisted_content_layout(read, TREE_BIN, TREE_RECORD_SIZE, "tree")? else {
        return Ok(None);
    };
    let candidates = {
        let _range = crate::perfetto_range!("ait.core.tree.get_by_id.index_lookup");
        index_candidates(
            read,
            content_index(TREE_ID_IDX, layout, "tree", Some((10, true)))?,
            &key,
        )?
    };
    for index in candidates {
        let view = {
            let _range = crate::perfetto_range!("ait.core.tree.get_by_id.candidate_view");
            tree_view_at::<B, WRITE_LAYOUT>(read, index)?
        };
        if view.tree_id.eq_ignore_ascii_case(tree_id) && !view.record.is_tombstone() {
            return Ok(Some(view));
        }
    }
    Ok(None)
}

pub(super) fn get_tree_view_by_id_with_cache<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    tree_id: &str,
    cache: &mut BinaryDbTreeReadCache,
) -> StoreResult<Option<BinaryTreeView>>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.tree.get_by_id_cached");
    let key = tree_id_index_key(tree_id)?;
    let Some(layout) = persisted_content_layout(read, TREE_BIN, TREE_RECORD_SIZE, "tree")? else {
        return Ok(None);
    };
    let candidates = index_candidates(
        read,
        content_index(TREE_ID_IDX, layout, "tree", Some((10, true)))?,
        &key,
    )?;
    for index in candidates {
        let view = tree_view_at_with_cache::<B, WRITE_LAYOUT>(read, index, cache)?;
        if view.tree_id.eq_ignore_ascii_case(tree_id) && !view.record.is_tombstone() {
            return Ok(Some(view));
        }
    }
    Ok(None)
}

pub(super) fn read_tree_pack_payload_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    repo_root: &StorePath,
    tree_index: u32,
    cache: &mut BinaryDbTreeReadCache,
) -> StoreResult<JsonValue>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.tree.payload_at");
    let tree = {
        let _range = crate::perfetto_range!("ait.core.tree.payload_at.record_read");
        read_tree_record_at::<B, WRITE_LAYOUT>(read, tree_index)?
    };
    let tree_id = tree_id_from_hash80(&tree.tree_hash80);
    let tree_pack = {
        let _range = crate::perfetto_range!("ait.core.tree.payload_at.pack_locator");
        tree_pack_view_for_tree_index_with_cache::<B, WRITE_LAYOUT>(read, tree_index, cache)?
            .ok_or_else(|| {
                BinaryDbError::corruption(format!(
                    "Binary DB tree {tree_id} has no authoritative tree-pack locator"
                ))
            })?
    };
    let _range = crate::perfetto_range!("ait.core.tree.payload_at.archive_payload");
    read_tree_pack_payload_for_record(repo_root, tree_index, &tree, &tree_pack, cache)
}

fn read_tree_pack_payload_for_record(
    repo_root: &StorePath,
    tree_index: u32,
    tree: &BinaryTreeRecord,
    tree_pack: &BinaryTreePackView,
    cache: &mut BinaryDbTreeReadCache,
) -> StoreResult<JsonValue> {
    let _range = crate::perfetto_range!("ait.core.tree.pack_payload");
    let tree_id = tree_id_from_hash80(&tree.tree_hash80);
    if !tree_pack.record.has_sparse_physical_ordinals() {
        if tree.pack_entry_ordinal >= tree_pack.record.tree_count {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree {tree_id} ordinal {} is outside tree pack {}",
                tree.pack_entry_ordinal, tree_pack.pack_id
            )));
        }
        let expected_ordinal = tree_index
            .checked_sub(tree_pack.record.first_tree_index)
            .ok_or_else(|| BinaryDbError::corruption("tree index precedes its tree-pack range"))?;
        if tree.pack_entry_ordinal != expected_ordinal {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree {tree_id} ordinal {} does not match tree-pack range ordinal {expected_ordinal}",
                tree.pack_entry_ordinal
            )));
        }
    }
    // A remote-head boundary import may reuse trees already owned by another
    // local pack, leaving a sparse projection of the verified physical
    // archive. For a marked sparse pack, `pack_entry_ordinal` is its physical
    // locator; archive bounds plus the payload tree ID and entry count below
    // remain the integrity checks.

    let pack_path = absolute_repo_path(repo_root, &tree_pack.pack_path)?;
    let pack_path_text = path_to_str(&pack_path)?;
    let cache_key = format!("{pack_path_text}#{}", tree_pack.pack_format);
    let archive = {
        let _range = crate::perfetto_range!("ait.core.tree.pack_payload.archive_select");
        match cache.archives.entry(cache_key) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => entry.insert(
                TreePackEntryArchive::open_with_format(pack_path_text, &tree_pack.pack_format)
                    .map_err(BinaryDbError::corruption)?,
            ),
        }
    };
    let payload = {
        let _range = crate::perfetto_range!("ait.core.tree.pack_payload.archive_read");
        archive
            .read_tree_by_ordinal(tree.pack_entry_ordinal as usize)
            .map_err(BinaryDbError::corruption)?
    };
    let normalized = {
        let _range = crate::perfetto_range!("ait.core.tree.pack_payload.normalize");
        normalize_tree_payload(&tree_id, payload)?
    };
    let actual_tree_id = normalized
        .get("tree_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| BinaryDbError::corruption("tree-pack payload is missing tree_id"))?;
    if !actual_tree_id.eq_ignore_ascii_case(&tree_id) {
        return Err(BinaryDbError::corruption(format!(
            "tree-pack ordinal {} contains {actual_tree_id}, expected {tree_id}",
            tree.pack_entry_ordinal
        )));
    }
    let actual_count = normalized
        .get("entries")
        .and_then(JsonValue::as_array)
        .map(Vec::len)
        .ok_or_else(|| BinaryDbError::corruption("tree-pack payload is missing entries"))?;
    if actual_count != tree.entry_count as usize {
        return Err(BinaryDbError::corruption(format!(
            "tree-pack entry count mismatch for {tree_id}: expected {}, got {actual_count}",
            tree.entry_count
        )));
    }
    Ok(normalized)
}

pub(super) fn list_tree_entry_views_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    repo_root: &StorePath,
    tree_index: u32,
    cache: &mut BinaryDbTreeReadCache,
) -> StoreResult<Vec<BinaryTreeEntryView>>
where
    B: BinaryDb,
{
    if let Some(entries) = cache.tree_entries.get(&tree_index) {
        return Ok(entries.clone());
    }
    let payload = read_tree_pack_payload_at::<B, WRITE_LAYOUT>(read, repo_root, tree_index, cache)?;
    let entries = tree_entry_views_from_payload(&payload)?;
    cache.tree_entries.insert(tree_index, entries.clone());
    Ok(entries)
}

fn tree_entry_views_from_payload(payload: &JsonValue) -> StoreResult<Vec<BinaryTreeEntryView>> {
    let entries = payload
        .get("entries")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| BinaryDbError::corruption("tree-pack payload is missing entries"))?;
    let mut views = entries
        .iter()
        .enumerate()
        .map(|(ordinal, row)| tree_entry_view_from_json(ordinal, row))
        .collect::<StoreResult<Vec<_>>>()?;
    views.sort_by(|left, right| left.entry_name.cmp(&right.entry_name));
    Ok(views)
}

fn tree_entry_view_from_json(ordinal: usize, row: &JsonValue) -> StoreResult<BinaryTreeEntryView> {
    let object = row
        .as_object()
        .ok_or_else(|| BinaryDbError::corruption("tree-pack entry must be an object"))?;
    let entry_name = object
        .get("entry_name")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| BinaryDbError::corruption("tree-pack entry is missing entry_name"))?
        .to_string();
    validate_tree_entry_name(&entry_name)?;
    let entry_type = object
        .get("entry_type")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| BinaryDbError::corruption("tree-pack entry is missing entry_type"))?
        .to_string();
    let target_id = object
        .get("target_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| BinaryDbError::corruption("tree-pack entry is missing target_id"))?
        .to_string();
    match entry_type.as_str() {
        "blob" => {
            blob_id_index_key(&target_id)?;
        }
        "tree" => {
            tree_id_index_key(&target_id)?;
        }
        other => {
            return Err(BinaryDbError::corruption(format!(
                "unsupported tree-pack entry type: {other}"
            )))
        }
    }
    let size_bytes = object.get("size_bytes").and_then(JsonValue::as_u64);
    let mode = object
        .get("mode")
        .and_then(JsonValue::as_str)
        .map(str::to_string);
    Ok(BinaryTreeEntryView {
        entry_ordinal: u32::try_from(ordinal)
            .map_err(|_| BinaryDbError::corruption("tree-pack entry ordinal overflows u32"))?,
        entry_name,
        entry_type,
        target_id,
        size_bytes,
        mode,
    })
}

pub(super) fn normalize_tree_payload(
    expected_tree_id: &str,
    payload: JsonValue,
) -> StoreResult<JsonValue> {
    if let Some(rows) = payload.as_array() {
        return Ok(json!({
            "tree_id": expected_tree_id,
            "entries": rows,
        }));
    }
    let object = payload
        .as_object()
        .ok_or_else(|| BinaryDbError::corruption("tree-pack payload must be an object or array"))?;
    let mut tree_id = object
        .get("tree_id")
        .and_then(JsonValue::as_str)
        .unwrap_or(expected_tree_id);
    let rows = object
        .get("entries")
        .or_else(|| object.get("rows"))
        .ok_or_else(|| BinaryDbError::corruption("tree-pack payload is missing rows"))?;
    let entries = if let Some(entries) = rows.as_array() {
        entries
    } else if let Some(nested) = rows.as_object() {
        tree_id = nested
            .get("tree_id")
            .and_then(JsonValue::as_str)
            .unwrap_or(tree_id);
        nested
            .get("entries")
            .or_else(|| nested.get("rows"))
            .and_then(JsonValue::as_array)
            .ok_or_else(|| BinaryDbError::corruption("nested tree-pack payload is missing rows"))?
    } else {
        return Err(BinaryDbError::corruption(
            "tree-pack rows must be an array or object",
        ));
    };
    let entries = entries
        .iter()
        .cloned()
        .map(|mut entry| {
            if let Some(object) = entry.as_object_mut() {
                object.remove("tree_id");
            }
            entry
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "tree_id": tree_id,
        "entries": entries,
    }))
}

pub(super) fn collect_manifest_rows<B, const WRITE_LAYOUT: u32>(
    tree_store: &BinaryDbTreeStore<B, WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, B>,
    tree_id: &str,
    prefix: &str,
    files: &mut Map<String, JsonValue>,
    visited: &mut BTreeSet<String>,
) -> StoreResult<()>
where
    B: BinaryDb,
{
    let mut cache = BinaryDbTreeReadCache::default();
    collect_manifest_rows_cached::<B, WRITE_LAYOUT>(
        tree_store, read, tree_id, prefix, files, visited, &mut cache,
    )
}

fn collect_manifest_rows_cached<B, const WRITE_LAYOUT: u32>(
    tree_store: &BinaryDbTreeStore<B, WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, B>,
    tree_id: &str,
    prefix: &str,
    files: &mut Map<String, JsonValue>,
    visited: &mut BTreeSet<String>,
    cache: &mut BinaryDbTreeReadCache,
) -> StoreResult<()>
where
    B: BinaryDb,
{
    if !visited.insert(tree_id.to_string()) {
        return Err(format!("cycle detected while reading tree manifest at {tree_id}").into());
    }
    let result = (|| {
        let tree = get_tree_view_by_id_with_cache::<B, WRITE_LAYOUT>(read, tree_id, cache)?
            .ok_or_else(|| BinaryDbError::missing_data(format!("Unknown tree: {tree_id}")))?;
        for entry in list_tree_entry_views_at::<B, WRITE_LAYOUT>(
            read,
            tree_store.repo_root(),
            tree.tree_index,
            cache,
        )? {
            let path = join_path(prefix, &entry.entry_name);
            if entry.entry_type == "tree" {
                collect_manifest_rows_cached::<B, WRITE_LAYOUT>(
                    tree_store,
                    read,
                    &entry.target_id,
                    &path,
                    files,
                    visited,
                    cache,
                )?;
                continue;
            }
            let previous = files.insert(
                path.clone(),
                json!({
                    "path": path,
                    "blob_id": entry.target_id,
                    "size_bytes": entry.size_bytes,
                    "mode": entry.mode,
                }),
            );
            if previous.is_some() {
                return Err(format!("duplicate snapshot manifest path: {path}").into());
            }
        }
        Ok(())
    })();
    visited.remove(tree_id);
    result
}

pub(super) fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

pub(super) fn validate_tree_entry_name(name: &str) -> StoreResult<()> {
    if name.is_empty() {
        return Err("tree entry name must not be empty".into());
    }
    if name == "." || name == ".." {
        return Err(format!("tree entry name must be a single safe path segment: {name}").into());
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(format!("tree entry name must be a single safe path segment: {name}").into());
    }
    Ok(())
}

pub(super) fn tree_record_from_view(view: BinaryTreeView) -> StoreResult<TreeRecord> {
    Ok(TreeRecord {
        tree_id: view.tree_id,
        entry_count: Some(i64::from(view.record.entry_count)),
        tree_pack_id: view.tree_pack_id,
        status: Some(if view.record.is_tombstone() {
            "tombstone".to_string()
        } else {
            "ready".to_string()
        }),
    })
}
