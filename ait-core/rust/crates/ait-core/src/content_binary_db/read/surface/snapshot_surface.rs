use super::*;

impl<B, const WRITE_LAYOUT: u32> BinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn read_snapshot_record(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        snapshot_index: u32,
    ) -> StoreResult<BinarySnapshotRecord> {
        read_snapshot_record_at::<B, WRITE_LAYOUT>(read, snapshot_index)
    }

    pub fn snapshot_view_at(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        snapshot_index: u32,
    ) -> StoreResult<BinarySnapshotView> {
        snapshot_view_at::<B, WRITE_LAYOUT>(read, snapshot_index)
    }

    pub fn get_snapshot_view(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        snapshot_id: &str,
    ) -> StoreResult<Option<BinarySnapshotView>> {
        get_snapshot_view_by_id::<B, WRITE_LAYOUT>(read, snapshot_id)
    }

    pub fn list_snapshot_views(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StoreResult<Vec<BinarySnapshotView>> {
        let _range = crate::perfetto_range!("ait.core.snapshot.list_views");
        let Some(layout) =
            persisted_content_layout(read, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, "snapshot")?
        else {
            return Ok(Vec::new());
        };
        let count = read.record_count(content_record_file(
            SNAPSHOT_BIN,
            SNAPSHOT_RECORD_SIZE,
            layout,
            "snapshot",
        )?)?;
        let mut views = Vec::new();
        for snapshot_index in 0..count {
            let view = snapshot_view_at::<B, WRITE_LAYOUT>(read, snapshot_index)?;
            if !view.record.is_tombstone() {
                views.push(view);
            }
        }
        views.sort_by(|left, right| {
            right
                .created_at_s
                .cmp(&left.created_at_s)
                .then_with(|| right.snapshot_index.cmp(&left.snapshot_index))
        });
        Ok(views)
    }

    pub fn list_line_snapshot_records_with_manifest_paths(
        &self,
    ) -> StoreResult<Vec<(SnapshotRecord, String)>> {
        let _range = crate::perfetto_range!("ait.core.snapshot.list_line_with_manifest_paths");
        let read = self.begin_read_txn();
        let views = {
            let _range = crate::perfetto_range!("ait.core.snapshot.list_line.load_views");
            self.list_snapshot_views(&read)?
        };
        let _range = crate::perfetto_range!("ait.core.snapshot.list_line.project");
        views
            .into_iter()
            .filter(|view| view.snapshot_kind == "line")
            .map(|view| {
                let manifest_path = snapshot_tree_manifest_path_from_view(&view)?;
                Ok((snapshot_record_from_view(view)?, manifest_path))
            })
            .collect()
    }

    pub fn update_snapshot_kind(
        &self,
        snapshot_id: &str,
        snapshot_kind: &str,
    ) -> StoreResult<usize> {
        require_non_empty(snapshot_id, "snapshot_id")?;
        let kind_bits = match snapshot_kind.trim() {
            "line" => 0,
            "stash" => 1,
            other => {
                return Err(format!("unsupported Binary DB snapshot kind: {other}").into());
            }
        };
        let mut write = self.begin_write_txn(BinaryDbCommandScope::ContentWrite)?;
        let key = snapshot_id_index_key(snapshot_id)?;
        let mut candidates = write.lookup_index(Self::snapshot_id_index(), &key)?;
        candidates.sort_unstable();
        candidates.dedup();
        candidates.reverse();
        for snapshot_index in candidates {
            let raw = write.read_record(Self::snapshot_file(), snapshot_index)?;
            let mut record = BinarySnapshotCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
            if record.is_tombstone()
                || !snapshot_id_from_hash48(record.snapshot_hash48())
                    .eq_ignore_ascii_case(snapshot_id)
            {
                continue;
            }
            record.snapshot_meta =
                (record.snapshot_meta & !BinarySnapshotRecord::META_KIND_MASK) | kind_bits;
            write.overwrite_record(
                Self::snapshot_file(),
                snapshot_index,
                &BinarySnapshotCodec::<WRITE_LAYOUT>::encode_record(&record)?,
            )?;
            write.commit()?;
            return Ok(1);
        }
        write.abort()?;
        Ok(0)
    }

    pub fn append_snapshot_payload<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        payload: &BinarySnapshotPayload,
    ) -> StoreResult<PayloadRange>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinarySnapshotCodec::<WRITE_LAYOUT>::encode_payload(payload)?;
        write.append_payload(Self::snapshot_payload_file(), &bytes)
    }

    pub fn append_snapshot_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinarySnapshotRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinarySnapshotCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::snapshot_file(), &bytes)
    }

    pub fn append_snapshot<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        mut record: BinarySnapshotRecord,
        payload: &BinarySnapshotPayload,
    ) -> StoreResult<(u32, String, BinarySnapshotRecord)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let payload_range = self.append_snapshot_payload(write, payload)?;
        record.payload_offset = payload_range.payload_offset;
        record.payload_len = u16::try_from(payload_range.payload_len)
            .map_err(|_| "snapshot payload length exceeds u16::MAX".to_string())?;
        if payload
            .message
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            record.snapshot_meta |= BinarySnapshotRecord::META_HAS_MESSAGE;
        } else {
            record.snapshot_meta &= !BinarySnapshotRecord::META_HAS_MESSAGE;
        }
        if payload.line_name.is_empty() {
            record.snapshot_meta &= !BinarySnapshotRecord::META_HAS_LINE_NAME_PAYLOAD;
        } else {
            record.snapshot_meta |= BinarySnapshotRecord::META_HAS_LINE_NAME_PAYLOAD;
        }
        if payload.additional_parent_snapshot_indices.is_empty() {
            record.snapshot_meta &= !BinarySnapshotRecord::META_HAS_ADDITIONAL_PARENTS;
        } else {
            record.snapshot_meta |= BinarySnapshotRecord::META_HAS_ADDITIONAL_PARENTS;
        }
        let index = self.append_snapshot_record(write, &record)?;
        let snapshot_id = snapshot_id_from_hash48(record.snapshot_hash48());
        Ok((index, snapshot_id, record))
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    pub fn append_snapshot_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        snapshot_id: &str,
        snapshot_index: u32,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let key = snapshot_id_index_key(snapshot_id)?;
        write.append_index_candidate(Self::snapshot_id_index(), &key, snapshot_index)
    }

    pub fn append_snapshot_with_id_index<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: BinarySnapshotRecord,
        payload: &BinarySnapshotPayload,
    ) -> StoreResult<(u32, String, BinarySnapshotRecord)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let (index, snapshot_id, record) = self.append_snapshot(write, record, payload)?;
        self.append_snapshot_id_index(write, &snapshot_id, index)?;
        Ok((index, snapshot_id, record))
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryTreeRootResolver for BinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn resolve_snapshot_root(
        &self,
        snapshot_id: &str,
    ) -> StoreResult<Option<BinaryTreeRootLocator>> {
        let read = self.begin_read_txn();
        Ok(self
            .get_snapshot_view(&read, snapshot_id)?
            .and_then(|view| view.root_tree_id.map(BinaryTreeRootLocator::new)))
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryTreeRootReadResolver<B>
    for BinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn resolve_snapshot_root_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        snapshot_id: &str,
    ) -> StoreResult<Option<BinaryTreeRootLocator>> {
        Ok(self
            .get_snapshot_view(read, snapshot_id)?
            .and_then(|view| view.root_tree_id.map(BinaryTreeRootLocator::new)))
    }
}

impl<B, const WRITE_LAYOUT: u32> SnapshotStore for BinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn snapshot_exists(&self, snapshot_id: &str) -> SnapshotStoreResult<bool> {
        require_non_empty(snapshot_id, "snapshot_id")?;
        let read = self.begin_read_txn();
        Ok(get_snapshot_record_by_id::<B, WRITE_LAYOUT>(&read, snapshot_id)?.is_some())
    }

    fn snapshot_parent_link(
        &self,
        snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<SnapshotParentLink>> {
        require_non_empty(snapshot_id, "snapshot_id")?;
        self.snapshot_parent_links(&[snapshot_id.to_string()])?
            .into_iter()
            .next()
            .ok_or_else(|| "Snapshot parent batch returned no row.".to_string())
    }

    fn snapshot_parent_links(
        &self,
        snapshot_ids: &[String],
    ) -> SnapshotStoreResult<Vec<Option<SnapshotParentLink>>> {
        let read = self.begin_read_txn();
        snapshot_ids
            .iter()
            .map(|snapshot_id| {
                require_non_empty(snapshot_id, "snapshot_id")?;
                let Some((snapshot_index, record)) =
                    get_snapshot_record_by_id::<B, WRITE_LAYOUT>(&read, snapshot_id)?
                else {
                    return Ok(None);
                };
                let parent_snapshot_ids =
                    snapshot_parent_records::<B, WRITE_LAYOUT>(&read, snapshot_index, &record)?
                        .into_iter()
                        .map(|(_, parent)| snapshot_id_from_hash48(parent.snapshot_hash48()))
                        .collect::<Vec<_>>();
                let (primary_parent_snapshot_id, parent_snapshot_id) =
                    compatibility_parent_projections(&parent_snapshot_ids);
                Ok(Some(SnapshotParentLink {
                    snapshot_id: snapshot_id_from_hash48(record.snapshot_hash48()),
                    parent_snapshot_ids,
                    primary_parent_snapshot_id,
                    parent_snapshot_id,
                }))
            })
            .collect()
    }

    fn snapshot_parent_link_page(
        &self,
        cursor: usize,
        limit: usize,
    ) -> SnapshotStoreResult<SnapshotParentLinkPage> {
        if limit == 0 {
            return Err("Snapshot parent-link page limit must be greater than zero.".to_string());
        }
        let read = self.begin_read_txn();
        let Some(layout) =
            persisted_content_layout(&read, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, "snapshot")?
        else {
            return Ok(SnapshotParentLinkPage {
                links: Vec::new(),
                next_cursor: None,
            });
        };
        let snapshot_file =
            content_record_file(SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, layout, "snapshot")?;
        let snapshot_count = usize::try_from(read.record_count(snapshot_file)?)
            .map_err(|_| "Snapshot record count does not fit usize.".to_string())?;
        if cursor > snapshot_count {
            return Err(format!(
                "Snapshot parent-link page cursor {cursor} is beyond record count {snapshot_count}."
            ));
        }
        let end = cursor.saturating_add(limit).min(snapshot_count);
        let mut links = Vec::with_capacity(end.saturating_sub(cursor));
        for snapshot_index in cursor..end {
            let snapshot_index = u32::try_from(snapshot_index)
                .map_err(|_| "Snapshot record index does not fit u32.".to_string())?;
            let record = read_snapshot_record_at::<B, WRITE_LAYOUT>(&read, snapshot_index)?;
            if record.is_tombstone() {
                continue;
            }
            let parent_snapshot_ids =
                snapshot_parent_records::<B, WRITE_LAYOUT>(&read, snapshot_index, &record)?
                    .into_iter()
                    .map(|(_, parent)| snapshot_id_from_hash48(parent.snapshot_hash48()))
                    .collect::<Vec<_>>();
            let (primary_parent_snapshot_id, parent_snapshot_id) =
                compatibility_parent_projections(&parent_snapshot_ids);
            links.push(SnapshotParentLink {
                snapshot_id: snapshot_id_from_hash48(record.snapshot_hash48()),
                parent_snapshot_ids,
                primary_parent_snapshot_id,
                parent_snapshot_id,
            });
        }
        Ok(SnapshotParentLinkPage {
            links,
            next_cursor: (end < snapshot_count).then_some(end),
        })
    }

    fn snapshot_by_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<SnapshotRecord>> {
        require_non_empty(snapshot_id, "snapshot_id")?;
        let read = self.begin_read_txn();
        Ok(self
            .get_snapshot_view(&read, snapshot_id)?
            .map(snapshot_record_from_view)
            .transpose()?)
    }

    fn list_line_snapshots(&self) -> SnapshotStoreResult<Vec<SnapshotRecord>> {
        let read = self.begin_read_txn();
        Ok(self
            .list_snapshot_views(&read)?
            .into_iter()
            .filter(|view| view.snapshot_kind == "line")
            .map(snapshot_record_from_view)
            .collect::<StoreResult<Vec<_>>>()?)
    }

    fn snapshot_total_bytes(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<i64>> {
        require_non_empty(snapshot_id, "snapshot_id")?;
        let read = self.begin_read_txn();
        self.get_snapshot_view(&read, snapshot_id)?
            .map(|view| {
                i64::try_from(view.total_bytes).map_err(|_| {
                    format!("snapshot total_bytes overflows i64: {}", view.total_bytes)
                })
            })
            .transpose()
    }

    fn snapshot_root_tree_pack_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        require_non_empty(snapshot_id, "snapshot_id")?;
        let read = self.begin_read_txn();
        Ok(self
            .get_snapshot_view(&read, snapshot_id)?
            .and_then(|view| view.root_tree_pack_id))
    }

    fn snapshot_kind(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        require_non_empty(snapshot_id, "snapshot_id")?;
        let read = self.begin_read_txn();
        Ok(self
            .get_snapshot_view(&read, snapshot_id)?
            .map(|view| view.snapshot_kind))
    }

    fn snapshot_chain(&self, snapshot_id: &str) -> SnapshotStoreResult<Vec<String>> {
        require_non_empty(snapshot_id, "snapshot_id")?;
        let read = self.begin_read_txn();
        let Some((mut current_index, mut current_record)) =
            get_snapshot_record_by_id::<B, WRITE_LAYOUT>(&read, snapshot_id)?
        else {
            return Err(format!("Unknown snapshot: {snapshot_id}"));
        };
        let mut ordered = Vec::new();
        let mut seen = BTreeSet::new();
        loop {
            let value = snapshot_id_from_hash48(current_record.snapshot_hash48());
            if !seen.insert(current_index) {
                return Err(format!("Cycle detected in snapshot chain at {value}"));
            }
            ordered.push(value);
            let Some((parent_index, parent)) = snapshot_primary_parent_record::<B, WRITE_LAYOUT>(
                &read,
                current_index,
                &current_record,
            )?
            else {
                break;
            };
            current_index = parent_index;
            current_record = parent;
        }
        ordered.reverse();
        Ok(ordered)
    }

    fn set_snapshot_kind(
        &self,
        snapshot_id: &str,
        snapshot_kind: &str,
    ) -> SnapshotStoreResult<usize> {
        self.update_snapshot_kind(snapshot_id, snapshot_kind)
            .map_err(String::from)
    }
}

pub(super) fn decode_snapshot_payload(
    layout: u32,
    raw: &[u8],
    has_line_name_payload: bool,
    has_additional_parents: bool,
) -> StoreResult<BinarySnapshotPayload> {
    match layout {
        CONTENT_BINARY_LAYOUT_ID => {
            BinarySnapshotCodec::<CONTENT_BINARY_LAYOUT_ID>::decode_payload(
                raw,
                has_line_name_payload,
                has_additional_parents,
            )
        }
        _ => {
            require_supported_content_layout(layout, "snapshot")?;
            unreachable!("supported content layout must have a snapshot payload decoder")
        }
    }
}

pub(super) fn read_snapshot_record_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    snapshot_index: u32,
) -> StoreResult<BinarySnapshotRecord>
where
    B: BinaryDb,
{
    let layout = required_content_layout(read, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, "snapshot")?;
    let raw = read.read_record(
        content_record_file(SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, layout, "snapshot")?,
        snapshot_index,
    )?;
    decode_snapshot_record(layout, &raw)
}

pub(super) fn snapshot_parent_records<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    snapshot_index: u32,
    record: &BinarySnapshotRecord,
) -> StoreResult<Vec<(u32, BinarySnapshotRecord)>>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.snapshot.snapshot_parent_records");
    let additional_parent_snapshot_indices = if record.has_additional_parents() {
        let _range = crate::perfetto_range!("ait.core.snapshot.read_parent_extension");
        let layout = required_content_layout(read, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, "snapshot")?;
        let payload_raw = read.read_payload(
            content_payload(SNAPSHOT_PAYLOAD_BIN, layout, "snapshot")?,
            record.payload_offset,
            u32::from(record.payload_len),
        )?;
        decode_snapshot_payload(layout, &payload_raw, record.has_line_name_payload(), true)?
            .additional_parent_snapshot_indices
    } else {
        Vec::new()
    };
    snapshot_parent_records_from_indices::<B, WRITE_LAYOUT>(
        read,
        snapshot_index,
        record,
        &additional_parent_snapshot_indices,
    )
}

fn snapshot_parent_records_from_indices<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    snapshot_index: u32,
    record: &BinarySnapshotRecord,
    additional_parent_snapshot_indices: &[u32],
) -> StoreResult<Vec<(u32, BinarySnapshotRecord)>>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.snapshot.validate_parent_indices");
    if record.has_additional_parents() == additional_parent_snapshot_indices.is_empty() {
        return Err(BinaryDbError::corruption(format!(
            "snapshot {snapshot_index} additional-parent flag does not match its payload"
        )));
    }
    if record.is_remote_head_history_boundary()
        && (record.parent_snapshot_index().is_some()
            || !additional_parent_snapshot_indices.is_empty())
    {
        return Err(BinaryDbError::corruption(format!(
            "snapshot {snapshot_index} is a remote-head history boundary but has local parents"
        )));
    }
    let Some(primary_parent_snapshot_index) = record.parent_snapshot_index() else {
        return Ok(Vec::new());
    };
    let mut parent_snapshot_indices =
        Vec::with_capacity(1_usize.saturating_add(additional_parent_snapshot_indices.len()));
    parent_snapshot_indices.push(primary_parent_snapshot_index);
    parent_snapshot_indices.extend_from_slice(additional_parent_snapshot_indices);
    if parent_snapshot_indices.len() > MAX_SNAPSHOT_PARENT_COUNT {
        return Err(BinaryDbError::corruption(format!(
            "snapshot {snapshot_index} parent count {} exceeds {MAX_SNAPSHOT_PARENT_COUNT}",
            parent_snapshot_indices.len()
        )));
    }
    let layout = required_content_layout(read, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, "snapshot")?;
    let snapshot_count = read.record_count(content_record_file(
        SNAPSHOT_BIN,
        SNAPSHOT_RECORD_SIZE,
        layout,
        "snapshot",
    )?)?;
    let mut seen = BTreeSet::new();
    let mut parents = Vec::with_capacity(parent_snapshot_indices.len());
    for (ordinal, parent_snapshot_index) in parent_snapshot_indices.into_iter().enumerate() {
        if parent_snapshot_index >= snapshot_count {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {snapshot_index} parent ordinal {ordinal} points at missing snapshot {parent_snapshot_index}"
            )));
        }
        if parent_snapshot_index >= snapshot_index {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {snapshot_index} parent ordinal {ordinal} must reference an earlier record, got {parent_snapshot_index}"
            )));
        }
        if !seen.insert(parent_snapshot_index) {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {snapshot_index} contains duplicate parent snapshot {parent_snapshot_index}"
            )));
        }
        let parent = read_snapshot_record_at::<B, WRITE_LAYOUT>(read, parent_snapshot_index)?;
        if parent.is_tombstone() {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {snapshot_index} references tombstoned parent snapshot {parent_snapshot_index}"
            )));
        }
        parents.push((parent_snapshot_index, parent));
    }
    Ok(parents)
}

fn snapshot_primary_parent_record<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    snapshot_index: u32,
    record: &BinarySnapshotRecord,
) -> StoreResult<Option<(u32, BinarySnapshotRecord)>>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.snapshot.snapshot_primary_parent_record");
    if record.is_remote_head_history_boundary() {
        if record.parent_snapshot_index().is_some() || record.has_additional_parents() {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {snapshot_index} is a remote-head history boundary but has local parents"
            )));
        }
        return Ok(None);
    }
    let Some(parent_snapshot_index) = record.parent_snapshot_index() else {
        return Ok(None);
    };
    if parent_snapshot_index >= snapshot_index {
        return Err(BinaryDbError::corruption(format!(
            "snapshot {snapshot_index} primary parent must reference an earlier record, got {parent_snapshot_index}"
        )));
    }
    let parent = read_snapshot_record_at::<B, WRITE_LAYOUT>(read, parent_snapshot_index)?;
    if parent.is_tombstone() {
        return Err(BinaryDbError::corruption(format!(
            "snapshot {snapshot_index} references tombstoned parent snapshot {parent_snapshot_index}"
        )));
    }
    Ok(Some((parent_snapshot_index, parent)))
}

pub(super) fn snapshot_view_at<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    snapshot_index: u32,
) -> StoreResult<BinarySnapshotView>
where
    B: BinaryDb,
{
    let layout = required_content_layout(read, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, "snapshot")?;
    let raw = read.read_record(
        content_record_file(SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, layout, "snapshot")?,
        snapshot_index,
    )?;
    let record = decode_snapshot_record(layout, &raw)?;
    let payload_raw = read.read_payload(
        content_payload(SNAPSHOT_PAYLOAD_BIN, layout, "snapshot")?,
        record.payload_offset,
        u32::from(record.payload_len),
    )?;
    let mut payload = decode_snapshot_payload(
        layout,
        &payload_raw,
        record.has_line_name_payload(),
        record.has_additional_parents(),
    )?;
    if record.has_message() != payload.message.is_some() {
        return Err("snapshot message metadata flag does not match payload".into());
    }
    match record.line_index() {
        Some(line_index) => {
            payload.line_name = binary_line_name_at(read, line_index)?;
        }
        None if !record.has_line_name_payload() => {
            return Err("snapshot has neither line index nor line-name payload".into());
        }
        None => {}
    }
    let parent_snapshot_ids = snapshot_parent_records_from_indices::<B, WRITE_LAYOUT>(
        read,
        snapshot_index,
        &record,
        &payload.additional_parent_snapshot_indices,
    )?
    .into_iter()
    .map(|(_, parent)| snapshot_id_from_hash48(parent.snapshot_hash48()))
    .collect::<Vec<_>>();
    let (primary_parent_snapshot_id, parent_snapshot_id) =
        compatibility_parent_projections(&parent_snapshot_ids);
    let snapshot_id = snapshot_id_from_hash48(record.snapshot_hash48());
    validate_snapshot_parent_set(
        Some(&snapshot_id),
        &parent_snapshot_ids,
        primary_parent_snapshot_id.as_deref(),
        parent_snapshot_id.as_deref(),
    )?;
    let (root_tree_pack_id, root_tree_pack_path, root_tree_id, root_tree_index) =
        match record.root_tree_pack_index() {
            Some(root_tree_pack_index) => {
                let pack = tree_pack_view_at::<B, WRITE_LAYOUT>(read, root_tree_pack_index)?;
                if record.root_entry_ordinal >= pack.record.tree_count {
                    return Err(format!(
                        "snapshot root tree ordinal {} is outside tree pack {} count {}",
                        record.root_entry_ordinal, pack.pack_id, pack.record.tree_count
                    )
                    .into());
                }
                let root_tree_index = pack
                    .record
                    .first_tree_index
                    .checked_add(record.root_entry_ordinal)
                    .ok_or_else(|| "snapshot root tree index overflow".to_string())?;
                let root_tree = read_tree_record_at::<B, WRITE_LAYOUT>(read, root_tree_index)?;
                (
                    Some(pack.pack_id),
                    Some(pack.pack_path),
                    Some(tree_id_from_hash80(&root_tree.tree_hash80)),
                    Some(root_tree_index),
                )
            }
            None => (None, None, None, None),
        };
    Ok(BinarySnapshotView {
        snapshot_index,
        snapshot_id,
        parent_snapshot_ids,
        primary_parent_snapshot_id,
        parent_snapshot_id,
        root_tree_pack_id,
        root_tree_pack_path,
        root_tree_id,
        root_tree_index,
        root_entry_ordinal: record.root_entry_ordinal,
        manifest_hash: hex_lower(&record.manifest_hash),
        snapshot_kind: snapshot_kind_name(record.kind())?.to_string(),
        file_count: u64::from(record.file_count),
        total_bytes: record.total_bytes,
        created_at_s: record.created_at_s,
        payload,
        record,
    })
}

pub(super) fn get_snapshot_view_by_id<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    snapshot_id: &str,
) -> StoreResult<Option<BinarySnapshotView>>
where
    B: BinaryDb,
{
    let key = snapshot_id_index_key(snapshot_id)?;
    let Some(layout) =
        persisted_content_layout(read, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, "snapshot")?
    else {
        return Ok(None);
    };
    for index in index_candidates(
        read,
        content_index(SNAPSHOT_ID_IDX, layout, "snapshot", Some((8, true)))?,
        &key,
    )? {
        let view = snapshot_view_at::<B, WRITE_LAYOUT>(read, index)?;
        if view.snapshot_id.eq_ignore_ascii_case(snapshot_id) && !view.record.is_tombstone() {
            return Ok(Some(view));
        }
    }
    Ok(None)
}

fn get_snapshot_record_by_id<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    snapshot_id: &str,
) -> StoreResult<Option<(u32, BinarySnapshotRecord)>>
where
    B: BinaryDb,
{
    let key = snapshot_id_index_key(snapshot_id)?;
    let Some(layout) =
        persisted_content_layout(read, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE, "snapshot")?
    else {
        return Ok(None);
    };
    for index in index_candidates(
        read,
        content_index(SNAPSHOT_ID_IDX, layout, "snapshot", Some((8, true)))?,
        &key,
    )? {
        let record = read_snapshot_record_at::<B, WRITE_LAYOUT>(read, index)?;
        if snapshot_id_from_hash48(record.snapshot_hash48()).eq_ignore_ascii_case(snapshot_id)
            && !record.is_tombstone()
        {
            return Ok(Some((index, record)));
        }
    }
    Ok(None)
}

pub(super) fn tree_entry_record_from_view(
    view: BinaryTreeEntryView,
) -> StoreResult<TreeEntryRecord> {
    Ok(TreeEntryRecord {
        path: view.entry_name,
        blob_id: (view.entry_type == "blob").then(|| view.target_id.clone()),
        tree_id: (view.entry_type == "tree").then(|| view.target_id.clone()),
        mode: view.mode,
        size_bytes: view
            .size_bytes
            .map(i64::try_from)
            .transpose()
            .map_err(|_| "tree entry size_bytes overflows i64".to_string())?,
    })
}

pub(super) fn snapshot_record_from_view(view: BinarySnapshotView) -> StoreResult<SnapshotRecord> {
    Ok(SnapshotRecord {
        snapshot_id: view.snapshot_id,
        parent_snapshot_ids: view.parent_snapshot_ids,
        primary_parent_snapshot_id: view.primary_parent_snapshot_id,
        parent_snapshot_id: view.parent_snapshot_id,
        root_tree_pack_id: view.root_tree_pack_id,
        root_entry_ordinal: Some(i64::from(view.root_entry_ordinal)),
        manifest_hash: view.manifest_hash,
        message: view.payload.message,
        line_name: view.payload.line_name,
        snapshot_kind: view.snapshot_kind,
        file_count: i64::try_from(view.file_count)
            .map_err(|_| format!("snapshot file_count overflows i64: {}", view.file_count))?,
        total_bytes: i64::try_from(view.total_bytes)
            .map_err(|_| format!("snapshot total_bytes overflows i64: {}", view.total_bytes))?,
        created_at: view.created_at_s.to_string(),
    })
}

pub(super) fn snapshot_kind_name(kind: BinarySnapshotKind) -> StoreResult<&'static str> {
    match kind {
        BinarySnapshotKind::Line => Ok("line"),
        BinarySnapshotKind::Stash => Ok("stash"),
        BinarySnapshotKind::Reserved(value) => {
            Err(format!("unsupported snapshot kind: {value}").into())
        }
    }
}
