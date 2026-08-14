use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinarySnapshotRecord {
    pub snapshot_meta: u8,
    pub history_flags: u8,
    pub payload_len: u16,
    pub payload_offset: u64,
    pub snapshot_hash48: u64,
    pub parent_snapshot_index_plus1: u32,
    pub root_tree_pack_index_plus1: u32,
    pub root_entry_ordinal: u32,
    pub line_index_plus1: u32,
    pub manifest_hash: [u8; 32],
    pub file_count: u32,
    pub total_bytes: u64,
    pub created_at_s: u64,
}

impl ServerBinarySnapshotRecord {
    pub const META_KIND_MASK: u8 = 0b0000_0011;
    pub const META_HAS_MESSAGE: u8 = 0b0000_0100;
    pub const META_HAS_LINE_NAME_PAYLOAD: u8 = 0b0000_1000;
    pub const META_PARENT_EDGES_AUTHORITY: u8 = 0b0001_0000;
    pub const META_HAS_ROOT_LOCATOR: u8 = 0b0010_0000;
    pub const META_TOMBSTONE: u8 = 0b1000_0000;
    pub const HISTORY_REMOTE_HEAD_BOUNDARY: u8 = 0b0000_0001;

    pub fn is_tombstone(&self) -> bool {
        self.snapshot_meta & Self::META_TOMBSTONE != 0
    }

    pub fn has_message(&self) -> bool {
        self.snapshot_meta & Self::META_HAS_MESSAGE != 0
    }

    pub fn has_line_name_payload(&self) -> bool {
        self.snapshot_meta & Self::META_HAS_LINE_NAME_PAYLOAD != 0
    }

    pub fn has_root_locator(&self) -> bool {
        self.snapshot_meta & Self::META_HAS_ROOT_LOCATOR != 0
    }

    pub fn has_parent_edges_authority(&self) -> bool {
        self.snapshot_meta & Self::META_PARENT_EDGES_AUTHORITY != 0
    }

    pub fn is_remote_head_history_boundary(&self) -> bool {
        self.history_flags & Self::HISTORY_REMOTE_HEAD_BOUNDARY != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinarySnapshotPayload {
    pub line_name: String,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinarySnapshotCatalogEntry {
    pub snapshot_index: u32,
    pub snapshot_id: String,
    pub record: ServerBinarySnapshotRecord,
    pub parent_snapshot_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerBinarySnapshotParentEdgeRecord {
    pub child_snapshot_index: u32,
    pub parent_snapshot_index: u32,
    pub parent_ordinal: u16,
    pub flags: u16,
}

pub struct ServerBinarySnapshotParentEdgeCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> ServerBinarySnapshotParentEdgeCodec<LAYOUT> {
    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(
            SERVER_SNAPSHOT_PARENT_EDGE_BIN,
            LAYOUT,
            SERVER_SNAPSHOT_PARENT_EDGE_RECORD_SIZE,
            BinaryDbFileFamily::Content,
        )
    }

    pub fn encode_record(record: ServerBinarySnapshotParentEdgeRecord) -> StoreResult<Vec<u8>> {
        require_layout::<LAYOUT>("snapshot parent edge")?;
        if record.flags != 0 || record.parent_ordinal >= 1_024 {
            return Err("snapshot parent edge flags/ordinal are invalid".into());
        }
        if record.child_snapshot_index == record.parent_snapshot_index {
            return Err("snapshot cannot be its own parent".into());
        }
        let mut out = Vec::with_capacity(SERVER_SNAPSHOT_PARENT_EDGE_RECORD_SIZE as usize);
        out.extend_from_slice(&record.child_snapshot_index.to_le_bytes());
        out.extend_from_slice(&record.parent_snapshot_index.to_le_bytes());
        out.extend_from_slice(&record.parent_ordinal.to_le_bytes());
        out.extend_from_slice(&record.flags.to_le_bytes());
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<ServerBinarySnapshotParentEdgeRecord> {
        require_layout::<LAYOUT>("snapshot parent edge")?;
        require_len(
            raw,
            SERVER_SNAPSHOT_PARENT_EDGE_RECORD_SIZE as usize,
            "ServerBinarySnapshotParentEdgeRecord",
        )?;
        let record = ServerBinarySnapshotParentEdgeRecord {
            child_snapshot_index: u32::from_le_bytes(raw[0..4].try_into().unwrap()),
            parent_snapshot_index: u32::from_le_bytes(raw[4..8].try_into().unwrap()),
            parent_ordinal: u16::from_le_bytes(raw[8..10].try_into().unwrap()),
            flags: u16::from_le_bytes(raw[10..12].try_into().unwrap()),
        };
        if record.flags != 0 || record.parent_ordinal >= 1_024 {
            return Err("snapshot parent edge flags/ordinal are corrupt".into());
        }
        if record.child_snapshot_index == record.parent_snapshot_index {
            return Err("snapshot parent edge is self-referential".into());
        }
        Ok(record)
    }
}

pub struct ServerBinarySnapshotCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> ServerBinarySnapshotCodec<LAYOUT> {
    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(
            SERVER_SNAPSHOT_BIN,
            LAYOUT,
            SERVER_SNAPSHOT_RECORD_SIZE,
            BinaryDbFileFamily::Content,
        )
    }

    pub fn payload_file() -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(
            SERVER_SNAPSHOT_PAYLOAD_BIN,
            LAYOUT,
            BinaryDbFileFamily::Content,
        )
    }

    pub fn id_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(
            SERVER_SNAPSHOT_ID_IDX,
            LAYOUT,
            8,
            true,
            BinaryDbFileFamily::Content,
        )
    }

    pub fn encode_record(record: &ServerBinarySnapshotRecord) -> StoreResult<Vec<u8>> {
        require_layout::<LAYOUT>("snapshot")?;
        validate_snapshot_record(record)?;
        let mut out = Vec::with_capacity(SERVER_SNAPSHOT_RECORD_SIZE as usize);
        out.push(record.snapshot_meta);
        out.push(record.history_flags);
        out.extend_from_slice(&record.payload_len.to_le_bytes());
        out.extend_from_slice(&record.payload_offset.to_le_bytes());
        out.extend_from_slice(&record.snapshot_hash48.to_le_bytes());
        out.extend_from_slice(&record.parent_snapshot_index_plus1.to_le_bytes());
        out.extend_from_slice(&record.root_tree_pack_index_plus1.to_le_bytes());
        out.extend_from_slice(&record.root_entry_ordinal.to_le_bytes());
        out.extend_from_slice(&record.line_index_plus1.to_le_bytes());
        out.extend_from_slice(&record.manifest_hash);
        out.extend_from_slice(&record.file_count.to_le_bytes());
        out.extend_from_slice(&record.total_bytes.to_le_bytes());
        out.extend_from_slice(&record.created_at_s.to_le_bytes());
        require_len(
            &out,
            SERVER_SNAPSHOT_RECORD_SIZE as usize,
            "ServerBinarySnapshotRecord",
        )?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<ServerBinarySnapshotRecord> {
        require_layout::<LAYOUT>("snapshot")?;
        require_len(
            raw,
            SERVER_SNAPSHOT_RECORD_SIZE as usize,
            "ServerBinarySnapshotRecord",
        )?;
        let record = ServerBinarySnapshotRecord {
            snapshot_meta: raw[0],
            history_flags: raw[1],
            payload_len: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
            payload_offset: u64::from_le_bytes(raw[4..12].try_into().unwrap()),
            snapshot_hash48: u64::from_le_bytes(raw[12..20].try_into().unwrap()),
            parent_snapshot_index_plus1: u32::from_le_bytes(raw[20..24].try_into().unwrap()),
            root_tree_pack_index_plus1: u32::from_le_bytes(raw[24..28].try_into().unwrap()),
            root_entry_ordinal: u32::from_le_bytes(raw[28..32].try_into().unwrap()),
            line_index_plus1: u32::from_le_bytes(raw[32..36].try_into().unwrap()),
            manifest_hash: raw[36..68].try_into().unwrap(),
            file_count: u32::from_le_bytes(raw[68..72].try_into().unwrap()),
            total_bytes: u64::from_le_bytes(raw[72..80].try_into().unwrap()),
            created_at_s: u64::from_le_bytes(raw[80..88].try_into().unwrap()),
        };
        validate_snapshot_record(&record)?;
        Ok(record)
    }

    pub fn encode_payload(payload: &ServerBinarySnapshotPayload) -> StoreResult<Vec<u8>> {
        require_layout::<LAYOUT>("snapshot")?;
        let message = payload.message.as_deref().unwrap_or("").as_bytes();
        let message_len = u16::try_from(message.len())
            .map_err(|_| "snapshot message exceeds u16::MAX bytes".to_string())?;
        let mut out = Vec::with_capacity(2 + message.len() + payload.line_name.len());
        out.extend_from_slice(&message_len.to_le_bytes());
        out.extend_from_slice(message);
        out.extend_from_slice(payload.line_name.as_bytes());
        if out.len() > usize::from(u16::MAX) {
            return Err(format!("snapshot payload exceeds u16::MAX bytes: {}", out.len()).into());
        }
        Ok(out)
    }

    pub fn decode_payload(
        raw: &[u8],
        has_line_name_payload: bool,
    ) -> StoreResult<ServerBinarySnapshotPayload> {
        require_layout::<LAYOUT>("snapshot")?;
        if raw.len() < 2 {
            return Err("snapshot payload is truncated".into());
        }
        let message_len = usize::from(u16::from_le_bytes(raw[0..2].try_into().unwrap()));
        let message_end = 2_usize
            .checked_add(message_len)
            .ok_or_else(|| "snapshot message length overflow".to_string())?;
        if message_end > raw.len() {
            return Err("snapshot message bytes are truncated".into());
        }
        let line_name_bytes = &raw[message_end..];
        if !has_line_name_payload && !line_name_bytes.is_empty() {
            return Err("snapshot has line-name bytes without metadata flag".into());
        }
        let message = if message_len == 0 {
            None
        } else {
            Some(
                String::from_utf8(raw[2..message_end].to_vec())
                    .map_err(|err| format!("snapshot message is not UTF-8: {err}"))?,
            )
        };
        let line_name = String::from_utf8(line_name_bytes.to_vec())
            .map_err(|err| format!("snapshot line_name is not UTF-8: {err}"))?;
        Ok(ServerBinarySnapshotPayload { line_name, message })
    }
}

#[derive(Clone, Debug)]
pub struct ServerBinaryDbSnapshotStore<B, const WRITE_LAYOUT: u32>
where
    B: ServerRemoteBinaryDb,
{
    db: B,
}

impl<B, const WRITE_LAYOUT: u32> ServerBinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: ServerRemoteBinaryDb,
{
    pub fn new(db: B) -> Self {
        Self { db }
    }

    pub fn snapshot_by_id(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        snapshot_id: &str,
    ) -> StoreResult<Option<(u32, ServerBinarySnapshotRecord)>> {
        let Some(layout) = persisted_content_layout(
            read,
            ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            "snapshot",
        )?
        else {
            return Ok(None);
        };
        let key = server_snapshot_id_index_key(snapshot_id)?;
        for index in read.lookup_index(snapshot_index_for_layout(layout)?, &key)? {
            let raw = read.read_record(snapshot_record_file_for_layout(layout)?, index)?;
            let record = decode_snapshot_record_for_layout(layout, &raw)?;
            if !record.is_tombstone()
                && server_snapshot_id_from_hash48(record.snapshot_hash48)
                    .eq_ignore_ascii_case(snapshot_id)
            {
                return Ok(Some((index, record)));
            }
        }
        Ok(None)
    }

    pub fn snapshot_payload(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        record: &ServerBinarySnapshotRecord,
    ) -> StoreResult<ServerBinarySnapshotPayload> {
        let layout = persisted_content_layout(
            read,
            ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            "snapshot",
        )?
        .ok_or_else(|| BinaryDbError::missing_data("canonical snapshot file is missing"))?;
        let raw = read.read_payload(
            snapshot_payload_file_for_layout(layout)?,
            record.payload_offset,
            u32::from(record.payload_len),
        )?;
        let payload =
            decode_snapshot_payload_for_layout(layout, &raw, record.has_line_name_payload())?;
        if record.has_message() != payload.message.is_some() {
            return Err("snapshot message flag does not match payload".into());
        }
        validate_snapshot_line_name_from_persisted_layout(read, record, &payload)?;
        Ok(payload)
    }

    pub fn all_snapshots(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StoreResult<Vec<(u32, ServerBinarySnapshotRecord)>> {
        let Some(layout) = persisted_content_layout(
            read,
            ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            "snapshot",
        )?
        else {
            return Ok(Vec::new());
        };
        let file = snapshot_record_file_for_layout(layout)?;
        let count = read.record_count(file.clone())?;
        let mut snapshots = Vec::with_capacity(count as usize);
        for (index, raw) in read.read_records(file, 0, count)?.into_iter().enumerate() {
            let index = u32::try_from(index)
                .map_err(|_| BinaryDbError::corruption("snapshot index exceeds u32"))?;
            let record = decode_snapshot_record_for_layout(layout, &raw)?;
            if !record.is_tombstone() {
                snapshots.push((index, record));
            }
        }
        Ok(snapshots)
    }

    pub fn snapshot_catalog(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StoreResult<Vec<ServerBinarySnapshotCatalogEntry>> {
        let snapshots = self.all_snapshots(read)?;
        let mut snapshot_ids_by_index = std::collections::BTreeMap::new();
        let mut snapshot_indexes_by_id = std::collections::BTreeMap::new();
        for (snapshot_index, record) in &snapshots {
            let snapshot_id = server_snapshot_id_from_hash48(record.snapshot_hash48);
            if let Some(existing_index) =
                snapshot_indexes_by_id.insert(snapshot_id.to_ascii_uppercase(), *snapshot_index)
            {
                return Err(BinaryDbError::corruption(format!(
                    "live snapshots {existing_index} and {snapshot_index} have duplicate ID {snapshot_id}"
                )));
            }
            snapshot_ids_by_index.insert(*snapshot_index, snapshot_id);
        }

        let edge_file =
            ServerBinarySnapshotParentEdgeCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file();
        let edge_count = read.record_count(edge_file.clone())?;
        let mut edges_by_child =
            std::collections::BTreeMap::<u32, Vec<ServerBinarySnapshotParentEdgeRecord>>::new();
        for raw in read.read_records(edge_file, 0, edge_count)? {
            let edge = ServerBinarySnapshotParentEdgeCodec::<
                SERVER_CONTENT_BINARY_LAYOUT_ID,
            >::decode_record(&raw)?;
            edges_by_child
                .entry(edge.child_snapshot_index)
                .or_default()
                .push(edge);
        }

        snapshots
            .into_iter()
            .map(|(snapshot_index, record)| {
                let parent_indexes = validated_snapshot_parent_indexes(
                    snapshot_index,
                    &record,
                    edges_by_child.remove(&snapshot_index).unwrap_or_default(),
                )?;
                let parent_snapshot_ids = parent_indexes
                    .into_iter()
                    .map(|parent_index| {
                        snapshot_ids_by_index.get(&parent_index).cloned().ok_or_else(|| {
                            BinaryDbError::corruption(format!(
                                "snapshot {snapshot_index} parent references missing or tombstoned snapshot index {parent_index}"
                            ))
                        })
                    })
                    .collect::<StoreResult<Vec<_>>>()?;
                let snapshot_id = snapshot_ids_by_index
                    .get(&snapshot_index)
                    .cloned()
                    .ok_or_else(|| {
                        BinaryDbError::corruption(format!(
                            "snapshot catalog lost live snapshot index {snapshot_index}"
                        ))
                    })?;
                Ok(ServerBinarySnapshotCatalogEntry {
                    snapshot_index,
                    snapshot_id,
                    record,
                    parent_snapshot_ids,
                })
            })
            .collect()
    }

    pub fn snapshot_id_at(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        snapshot_index: u32,
    ) -> StoreResult<String> {
        let layout = persisted_content_layout(
            read,
            ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            "snapshot",
        )?
        .ok_or_else(|| BinaryDbError::missing_data("canonical snapshot file is missing"))?;
        let raw = read.read_record(snapshot_record_file_for_layout(layout)?, snapshot_index)?;
        let record = decode_snapshot_record_for_layout(layout, &raw)?;
        if record.is_tombstone() {
            return Err(BinaryDbError::corruption(format!(
                "canonical line head references tombstoned snapshot index {snapshot_index}"
            )));
        }
        Ok(server_snapshot_id_from_hash48(record.snapshot_hash48))
    }

    pub fn snapshot_parent_indexes(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        snapshot_index: u32,
        record: &ServerBinarySnapshotRecord,
    ) -> StoreResult<Vec<u32>> {
        if !record.has_parent_edges_authority() {
            return Ok(record
                .parent_snapshot_index_plus1
                .checked_sub(1)
                .into_iter()
                .collect());
        }
        let count = read.record_count(ServerBinarySnapshotParentEdgeCodec::<
            SERVER_CONTENT_BINARY_LAYOUT_ID,
        >::record_file())?;
        let mut edges = Vec::new();
        for edge_index in 0..count {
            let edge = ServerBinarySnapshotParentEdgeCodec::<
                SERVER_CONTENT_BINARY_LAYOUT_ID,
            >::decode_record(&read.read_record(
                ServerBinarySnapshotParentEdgeCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
                edge_index,
            )?)?;
            if edge.child_snapshot_index == snapshot_index {
                edges.push(edge);
            }
        }
        validated_snapshot_parent_indexes(snapshot_index, record, edges)
    }
}

fn validated_snapshot_parent_indexes(
    snapshot_index: u32,
    record: &ServerBinarySnapshotRecord,
    mut edges: Vec<ServerBinarySnapshotParentEdgeRecord>,
) -> StoreResult<Vec<u32>> {
    if !record.has_parent_edges_authority() {
        return Ok(record
            .parent_snapshot_index_plus1
            .checked_sub(1)
            .into_iter()
            .collect());
    }
    edges.sort_by_key(|edge| edge.parent_ordinal);
    for (expected, edge) in edges.iter().enumerate() {
        if usize::from(edge.parent_ordinal) != expected {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {snapshot_index} parent ordinals are not contiguous at {expected}"
            )));
        }
    }
    let parents = edges
        .into_iter()
        .map(|edge| edge.parent_snapshot_index)
        .collect::<Vec<_>>();
    let mut unique_parents = std::collections::BTreeSet::new();
    for parent_index in &parents {
        if !unique_parents.insert(*parent_index) {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {snapshot_index} contains duplicate parent index {parent_index}"
            )));
        }
    }
    let cached = record.parent_snapshot_index_plus1.checked_sub(1);
    if parents.first().copied() != cached {
        return Err(BinaryDbError::corruption(format!(
            "snapshot {snapshot_index} first-parent cache disagrees with ordered edges"
        )));
    }
    if record.is_remote_head_history_boundary() && !parents.is_empty() {
        return Err(BinaryDbError::corruption(format!(
            "remote-head history boundary snapshot {snapshot_index} has local parents"
        )));
    }
    Ok(parents)
}

pub fn validate_server_snapshot_dag_v0<B>(db: &B) -> StoreResult<()>
where
    B: ServerRemoteBinaryDb,
{
    let read = BinaryDbReadTxn::new(db);
    let snapshot_file = ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file();
    let edge_file =
        ServerBinarySnapshotParentEdgeCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file();
    let snapshot_count = read.record_count(snapshot_file.clone())?;
    let mut snapshots = Vec::with_capacity(snapshot_count as usize);
    for index in 0..snapshot_count {
        snapshots.push(
            ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(
                &read.read_record(snapshot_file.clone(), index)?,
            )?,
        );
    }
    let mut parents = vec![Vec::<(u16, u32)>::new(); snapshot_count as usize];
    let edge_count = read.record_count(edge_file.clone())?;
    for edge_index in 0..edge_count {
        let edge =
            ServerBinarySnapshotParentEdgeCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(
                &read.read_record(edge_file.clone(), edge_index)?,
            )?;
        if edge.child_snapshot_index >= snapshot_count
            || edge.parent_snapshot_index >= snapshot_count
        {
            return Err(BinaryDbError::corruption(format!(
                "snapshot parent edge {edge_index} references an out-of-range Snapshot"
            )));
        }
        let child = &snapshots[edge.child_snapshot_index as usize];
        let parent = &snapshots[edge.parent_snapshot_index as usize];
        if !child.has_parent_edges_authority() {
            return Err(BinaryDbError::corruption(format!(
                "snapshot parent edge {edge_index} belongs to a child without edge authority"
            )));
        }
        if child.is_tombstone() || parent.is_tombstone() {
            return Err(BinaryDbError::corruption(format!(
                "snapshot parent edge {edge_index} references a tombstoned Snapshot"
            )));
        }
        parents[edge.child_snapshot_index as usize]
            .push((edge.parent_ordinal, edge.parent_snapshot_index));
    }
    for (index, record) in snapshots.iter().enumerate() {
        if record.is_tombstone() {
            continue;
        }
        if !record.has_parent_edges_authority() {
            return Err(BinaryDbError::corruption(format!(
                "live snapshot {index} lacks v0 ordered parent-edge authority"
            )));
        }
        let rows = &mut parents[index];
        if rows.len() > 1_024 {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {index} exceeds 1,024 parent edges"
            )));
        }
        rows.sort_by_key(|(ordinal, _)| *ordinal);
        for (expected, (ordinal, _)) in rows.iter().enumerate() {
            if usize::from(*ordinal) != expected {
                return Err(BinaryDbError::corruption(format!(
                    "snapshot {index} parent ordinal {ordinal} is not contiguous at {expected}"
                )));
            }
        }
        let first = rows.first().map(|(_, parent)| parent + 1).unwrap_or(0);
        if record.parent_snapshot_index_plus1 != first {
            return Err(BinaryDbError::corruption(format!(
                "snapshot {index} first-parent cache disagrees with ordered edges"
            )));
        }
        if record.is_remote_head_history_boundary() && !rows.is_empty() {
            return Err(BinaryDbError::corruption(format!(
                "remote-head history boundary snapshot {index} has local parent edges"
            )));
        }
    }

    fn visit(
        index: usize,
        snapshots: &[ServerBinarySnapshotRecord],
        parents: &[Vec<(u16, u32)>],
        state: &mut [u8],
    ) -> StoreResult<()> {
        match state[index] {
            1 => {
                return Err(BinaryDbError::corruption(format!(
                    "snapshot parent DAG contains a cycle at Snapshot {index}"
                )))
            }
            2 => return Ok(()),
            _ => {}
        }
        if snapshots[index].is_tombstone() {
            state[index] = 2;
            return Ok(());
        }
        state[index] = 1;
        for (_, parent) in &parents[index] {
            visit(*parent as usize, snapshots, parents, state)?;
        }
        state[index] = 2;
        Ok(())
    }

    let mut state = vec![0_u8; snapshots.len()];
    for index in 0..snapshots.len() {
        visit(index, &snapshots, &parents, &mut state)?;
    }
    Ok(())
}

impl<B, const WRITE_LAYOUT: u32> ServerBinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: ServerRemoteBinaryDb + BinaryDbIndexAppender,
{
    pub(crate) fn snapshot_by_id_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, B, F>,
        snapshot_id: &str,
    ) -> StoreResult<Option<(u32, ServerBinarySnapshotRecord)>>
    where
        F: BinaryDbFsyncPolicy,
    {
        require_layout::<WRITE_LAYOUT>("snapshot write")?;
        find_snapshot_in_write::<B, _, WRITE_LAYOUT>(write, snapshot_id)
    }

    pub(crate) fn snapshot_chain_contains_ancestor_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, B, F>,
        ancestor_snapshot_index: u32,
        descendant_snapshot_index: u32,
    ) -> StoreResult<bool>
    where
        F: BinaryDbFsyncPolicy,
    {
        if ancestor_snapshot_index == descendant_snapshot_index {
            return Ok(true);
        }
        let file = ServerBinarySnapshotCodec::<WRITE_LAYOUT>::record_file();
        let edge_file = ServerBinarySnapshotParentEdgeCodec::<WRITE_LAYOUT>::record_file();
        let edge_count = write.record_count(edge_file.clone())?;
        let mut parents_by_child = std::collections::BTreeMap::<u32, Vec<(u16, u32)>>::new();
        for edge_index in 0..edge_count {
            let edge = ServerBinarySnapshotParentEdgeCodec::<WRITE_LAYOUT>::decode_record(
                &write.read_record(edge_file.clone(), edge_index)?,
            )?;
            parents_by_child
                .entry(edge.child_snapshot_index)
                .or_default()
                .push((edge.parent_ordinal, edge.parent_snapshot_index));
        }
        let mut seen = std::collections::BTreeSet::new();
        let mut pending = vec![descendant_snapshot_index];
        while let Some(cursor) = pending.pop() {
            if !seen.insert(cursor) {
                continue;
            }
            let record = ServerBinarySnapshotCodec::<WRITE_LAYOUT>::decode_record(
                &write.read_record(file.clone(), cursor)?,
            )?;
            let parents = if record.has_parent_edges_authority() {
                let mut rows = parents_by_child.remove(&cursor).unwrap_or_default();
                rows.sort_by_key(|(ordinal, _)| *ordinal);
                for (expected, (ordinal, _)) in rows.iter().enumerate() {
                    if usize::from(*ordinal) != expected {
                        return Err(BinaryDbError::corruption(format!(
                            "snapshot {cursor} parent ordinals are not contiguous at {expected}"
                        )));
                    }
                }
                let parents = rows
                    .into_iter()
                    .map(|(_, parent)| parent)
                    .collect::<Vec<_>>();
                if parents.first().copied() != record.parent_snapshot_index_plus1.checked_sub(1) {
                    return Err(BinaryDbError::corruption(format!(
                        "snapshot {cursor} first-parent cache disagrees with ordered edges"
                    )));
                }
                parents
            } else {
                record
                    .parent_snapshot_index_plus1
                    .checked_sub(1)
                    .into_iter()
                    .collect()
            };
            if parents.contains(&ancestor_snapshot_index) {
                return Ok(true);
            }
            pending.extend(parents);
        }
        Ok(false)
    }

    pub(crate) fn snapshot_id_at_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, B, F>,
        snapshot_index: u32,
    ) -> StoreResult<String>
    where
        F: BinaryDbFsyncPolicy,
    {
        let raw = write.db().read_record(
            ServerBinarySnapshotCodec::<WRITE_LAYOUT>::record_file(),
            snapshot_index,
        )?;
        let record = ServerBinarySnapshotCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        Ok(server_snapshot_id_from_hash48(record.snapshot_hash48))
    }

    pub fn append_snapshot(
        &self,
        snapshot_id: &str,
        record: ServerBinarySnapshotRecord,
        payload: &ServerBinarySnapshotPayload,
    ) -> StoreResult<u32> {
        self.append_snapshot_internal(snapshot_id, record, payload)
    }

    fn append_snapshot_internal(
        &self,
        snapshot_id: &str,
        record: ServerBinarySnapshotRecord,
        payload: &ServerBinarySnapshotPayload,
    ) -> StoreResult<u32> {
        require_layout::<WRITE_LAYOUT>("snapshot write")?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerContent)?;
        let index = self.append_snapshot_internal_in_tx(&mut tx, snapshot_id, record, payload)?;
        tx.commit()?;
        Ok(index)
    }

    pub(crate) fn append_snapshot_in_tx<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        snapshot_id: &str,
        record: ServerBinarySnapshotRecord,
        payload: &ServerBinarySnapshotPayload,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        require_layout::<WRITE_LAYOUT>("snapshot write")?;
        self.append_snapshot_internal_in_tx(tx, snapshot_id, record, payload)
    }

    fn append_snapshot_internal_in_tx<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        snapshot_id: &str,
        mut record: ServerBinarySnapshotRecord,
        payload: &ServerBinarySnapshotPayload,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        require_layout::<WRITE_LAYOUT>("snapshot write")?;
        let hash48 = server_snapshot_hash48_from_id(snapshot_id)?;
        if record.snapshot_hash48 != hash48 {
            return Err(format!(
                "snapshot id {snapshot_id} does not match record hash48 {:012X}",
                record.snapshot_hash48
            )
            .into());
        }
        if find_snapshot_in_write::<B, _, WRITE_LAYOUT>(tx, snapshot_id)?.is_some() {
            return Err(format!("snapshot already exists: {snapshot_id}").into());
        }
        validate_optional_record_link(
            tx,
            ServerBinarySnapshotCodec::<WRITE_LAYOUT>::record_file(),
            record.parent_snapshot_index_plus1,
            "parent snapshot",
        )?;
        validate_optional_record_link(
            tx,
            ServerBinaryLineCodec::<WRITE_LAYOUT>::record_file(),
            record.line_index_plus1,
            "line",
        )?;
        validate_snapshot_line_name::<_, WRITE_LAYOUT>(tx, &record, payload)?;
        let payload_bytes = ServerBinarySnapshotCodec::<WRITE_LAYOUT>::encode_payload(payload)?;
        let range = tx.append_payload(
            ServerBinarySnapshotCodec::<WRITE_LAYOUT>::payload_file(),
            &payload_bytes,
        )?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16::try_from(range.payload_len)
            .map_err(|_| "snapshot payload length exceeds u16::MAX".to_string())?;
        set_payload_flags(&mut record, payload);
        let expected_index =
            tx.record_count(ServerBinarySnapshotCodec::<WRITE_LAYOUT>::record_file())?;
        record.snapshot_meta |= ServerBinarySnapshotRecord::META_PARENT_EDGES_AUTHORITY;
        if let Some(parent_snapshot_index) = record.parent_snapshot_index_plus1.checked_sub(1) {
            let edge = ServerBinarySnapshotParentEdgeRecord {
                child_snapshot_index: expected_index,
                parent_snapshot_index,
                parent_ordinal: 0,
                flags: 0,
            };
            tx.append_record(
                ServerBinarySnapshotParentEdgeCodec::<WRITE_LAYOUT>::record_file(),
                &ServerBinarySnapshotParentEdgeCodec::<WRITE_LAYOUT>::encode_record(edge)?,
            )?;
            tx.fsync_policy().sync_file_data(
                &ServerRemoteBinaryDb::authority_root(&self.db)
                    .as_path()
                    .join(SERVER_SNAPSHOT_PARENT_EDGE_BIN),
            )?;
        }
        let raw = ServerBinarySnapshotCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        let index = tx.append_record(
            ServerBinarySnapshotCodec::<WRITE_LAYOUT>::record_file(),
            &raw,
        )?;
        if index != expected_index {
            return Err(BinaryDbError::corruption(format!(
                "snapshot append index {index} disagrees with expected index {expected_index}"
            )));
        }
        tx.append_index_candidate(
            ServerBinarySnapshotCodec::<WRITE_LAYOUT>::id_index(),
            &server_snapshot_id_index_key(snapshot_id)?,
            index,
        )?;
        Ok(index)
    }
}
