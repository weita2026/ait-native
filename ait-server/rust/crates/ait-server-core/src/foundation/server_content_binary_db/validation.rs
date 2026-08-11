use super::*;

pub(super) trait ContentReadAccess {
    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32>;
    fn read_record(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>>;
    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>>;
    fn lookup_index(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>>;
}

pub(super) fn persisted_content_layout<B: BinaryDb>(
    read: &BinaryDbReadTxn<'_, B>,
    physical_file: BinaryFileId,
    label: &str,
) -> StoreResult<Option<u32>> {
    let layout = match read.layout_id(physical_file) {
        Ok(layout) => layout,
        Err(error) if error.kind() == BinaryDbErrorKind::MissingData => return Ok(None),
        Err(error) => return Err(error),
    };
    require_supported_content_layout(layout, label)?;
    Ok(Some(layout))
}

pub(super) fn require_supported_content_layout(layout: u32, label: &str) -> StoreResult<()> {
    if layout == SERVER_CONTENT_BINARY_LAYOUT_ID {
        Ok(())
    } else {
        Err(BinaryDbError::layout_mismatch(format!(
            "unsupported Binary DB {label} persisted layout: {layout}; supported layout is {SERVER_CONTENT_BINARY_LAYOUT_ID}"
        )))
    }
}

pub(super) fn line_record_file_for_layout(layout: u32) -> StoreResult<BinaryFileId> {
    require_supported_content_layout(layout, "line")?;
    Ok(BinaryFileId::new(
        SERVER_LINE_BIN,
        layout,
        SERVER_LINE_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    ))
}

pub(super) fn line_payload_file_for_layout(layout: u32) -> StoreResult<BinaryPayloadFileId> {
    require_supported_content_layout(layout, "line")?;
    Ok(BinaryPayloadFileId::new(
        SERVER_LINE_NAME_PAYLOAD_BIN,
        layout,
        BinaryDbFileFamily::Content,
    ))
}

pub(super) fn line_index_for_layout(layout: u32) -> StoreResult<BinaryIndexId> {
    require_supported_content_layout(layout, "line")?;
    Ok(BinaryIndexId::new_fixed(
        SERVER_LINE_NAME_IDX,
        layout,
        8,
        false,
        BinaryDbFileFamily::Content,
    ))
}

pub(super) fn snapshot_record_file_for_layout(layout: u32) -> StoreResult<BinaryFileId> {
    require_supported_content_layout(layout, "snapshot")?;
    Ok(BinaryFileId::new(
        SERVER_SNAPSHOT_BIN,
        layout,
        SERVER_SNAPSHOT_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    ))
}

pub(super) fn snapshot_payload_file_for_layout(layout: u32) -> StoreResult<BinaryPayloadFileId> {
    require_supported_content_layout(layout, "snapshot")?;
    Ok(BinaryPayloadFileId::new(
        SERVER_SNAPSHOT_PAYLOAD_BIN,
        layout,
        BinaryDbFileFamily::Content,
    ))
}

pub(super) fn snapshot_index_for_layout(layout: u32) -> StoreResult<BinaryIndexId> {
    require_supported_content_layout(layout, "snapshot")?;
    Ok(BinaryIndexId::new_fixed(
        SERVER_SNAPSHOT_ID_IDX,
        layout,
        8,
        true,
        BinaryDbFileFamily::Content,
    ))
}

pub(super) fn decode_line_record_for_layout(
    layout: u32,
    raw: &[u8],
) -> StoreResult<ServerBinaryLineRecord> {
    match layout {
        SERVER_CONTENT_BINARY_LAYOUT_ID => {
            ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(raw)
        }
        _ => {
            require_supported_content_layout(layout, "line")?;
            unreachable!("supported content line layout must have a decoder")
        }
    }
}

pub(super) fn decode_snapshot_record_for_layout(
    layout: u32,
    raw: &[u8],
) -> StoreResult<ServerBinarySnapshotRecord> {
    match layout {
        SERVER_CONTENT_BINARY_LAYOUT_ID => {
            ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(raw)
        }
        _ => {
            require_supported_content_layout(layout, "snapshot")?;
            unreachable!("supported content snapshot layout must have a decoder")
        }
    }
}

pub(super) fn decode_snapshot_payload_for_layout(
    layout: u32,
    raw: &[u8],
    has_line_name_payload: bool,
) -> StoreResult<ServerBinarySnapshotPayload> {
    match layout {
        SERVER_CONTENT_BINARY_LAYOUT_ID => ServerBinarySnapshotCodec::<
            SERVER_CONTENT_BINARY_LAYOUT_ID,
        >::decode_payload(raw, has_line_name_payload),
        _ => {
            require_supported_content_layout(layout, "snapshot")?;
            unreachable!("supported content snapshot layout must have a payload decoder")
        }
    }
}

impl<B: BinaryDb> ContentReadAccess for BinaryDbReadTxn<'_, B> {
    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.record_count(file)
    }
    fn read_record(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>> {
        self.read_record(file, index)
    }
    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.read_payload(file, offset, len)
    }
    fn lookup_index(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>> {
        self.lookup_index(index, key)
    }
}

impl<B, F> ContentReadAccess for BinaryDbWriteTxn<'_, B, F>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.record_count(file)
    }
    fn read_record(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>> {
        self.read_record(file, index)
    }
    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.read_payload(file, offset, len)
    }
    fn lookup_index(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>> {
        self.lookup_index(index, key)
    }
}

pub(super) fn find_line<B: BinaryDb, const LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    line_name: &str,
) -> StoreResult<Option<(u32, ServerBinaryLineRecord)>> {
    let Some(layout) = persisted_content_layout(
        read,
        ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
        "line",
    )?
    else {
        return Ok(None);
    };
    let line_name = line_name.trim();
    if line_name.is_empty() {
        return Ok(None);
    }
    let key = server_line_name_hash64(line_name.as_bytes()).to_le_bytes();
    for index in read.lookup_index(line_index_for_layout(layout)?, &key)? {
        let raw = read.read_record(line_record_file_for_layout(layout)?, index)?;
        let record = decode_line_record_for_layout(layout, &raw)?;
        if record.is_tombstone() {
            continue;
        }
        let name = read.read_payload(
            line_payload_file_for_layout(layout)?,
            record.line_name_offset,
            u32::from(record.line_name_len),
        )?;
        if name == line_name.as_bytes() {
            return Ok(Some((index, record)));
        }
    }
    Ok(None)
}

pub(super) fn find_line_in_write<B, F, const LAYOUT: u32>(
    write: &BinaryDbWriteTxn<'_, B, F>,
    line_name: &str,
) -> StoreResult<Option<(u32, ServerBinaryLineRecord)>>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    find_line_from_access::<_, LAYOUT>(write, line_name)
}

pub(super) fn find_line_from_access<A: ContentReadAccess, const LAYOUT: u32>(
    access: &A,
    line_name: &str,
) -> StoreResult<Option<(u32, ServerBinaryLineRecord)>> {
    let line_name = line_name.trim();
    if line_name.is_empty() {
        return Ok(None);
    }
    let key = server_line_name_hash64(line_name.as_bytes()).to_le_bytes();
    for index in access.lookup_index(ServerBinaryLineCodec::<LAYOUT>::name_index(), &key)? {
        let raw = access.read_record(ServerBinaryLineCodec::<LAYOUT>::record_file(), index)?;
        let record = ServerBinaryLineCodec::<LAYOUT>::decode_record(&raw)?;
        if record.is_tombstone() {
            continue;
        }
        let name = access.read_payload(
            ServerBinaryLineCodec::<LAYOUT>::payload_file(),
            record.line_name_offset,
            u32::from(record.line_name_len),
        )?;
        if name == line_name.as_bytes() {
            return Ok(Some((index, record)));
        }
    }
    Ok(None)
}

pub(super) fn find_snapshot_in_write<B, F, const LAYOUT: u32>(
    write: &BinaryDbWriteTxn<'_, B, F>,
    snapshot_id: &str,
) -> StoreResult<Option<(u32, ServerBinarySnapshotRecord)>>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    let key = server_snapshot_id_index_key(snapshot_id)?;
    for index in write.lookup_index(ServerBinarySnapshotCodec::<LAYOUT>::id_index(), &key)? {
        let raw = write.read_record(ServerBinarySnapshotCodec::<LAYOUT>::record_file(), index)?;
        let record = ServerBinarySnapshotCodec::<LAYOUT>::decode_record(&raw)?;
        if !record.is_tombstone()
            && server_snapshot_id_from_hash48(record.snapshot_hash48)
                .eq_ignore_ascii_case(snapshot_id)
        {
            return Ok(Some((index, record)));
        }
    }
    Ok(None)
}

pub(super) fn validate_snapshot_link<A: ContentReadAccess, const LAYOUT: u32>(
    access: &A,
    snapshot_index_plus1: u32,
) -> StoreResult<()> {
    let Some(index) = snapshot_index_plus1.checked_sub(1) else {
        return Ok(());
    };
    let file = ServerBinarySnapshotCodec::<LAYOUT>::record_file();
    let count = access.record_count(file.clone())?;
    if index >= count {
        return Err(format!("snapshot index {index} is out of range").into());
    }
    let raw = access.read_record(file, index)?;
    if ServerBinarySnapshotCodec::<LAYOUT>::decode_record(&raw)?.is_tombstone() {
        return Err(format!("snapshot index {index} is tombstoned").into());
    }
    Ok(())
}

pub(super) fn validate_snapshot_line_name<A: ContentReadAccess, const LAYOUT: u32>(
    access: &A,
    record: &ServerBinarySnapshotRecord,
    payload: &ServerBinarySnapshotPayload,
) -> StoreResult<()> {
    let Some(line_index) = record.line_index_plus1.checked_sub(1) else {
        return Ok(());
    };
    let raw = access.read_record(ServerBinaryLineCodec::<LAYOUT>::record_file(), line_index)?;
    let line = ServerBinaryLineCodec::<LAYOUT>::decode_record(&raw)?;
    if line.is_tombstone() {
        return Err(format!("snapshot references tombstoned line index {line_index}").into());
    }
    let line_name = access.read_payload(
        ServerBinaryLineCodec::<LAYOUT>::payload_file(),
        line.line_name_offset,
        u32::from(line.line_name_len),
    )?;
    if line_name != payload.line_name.as_bytes() {
        return Err(format!(
            "snapshot line payload {:?} does not match line index {line_index}",
            payload.line_name
        )
        .into());
    }
    Ok(())
}

pub(super) fn validate_snapshot_line_name_from_persisted_layout<B: BinaryDb>(
    read: &BinaryDbReadTxn<'_, B>,
    record: &ServerBinarySnapshotRecord,
    payload: &ServerBinarySnapshotPayload,
) -> StoreResult<()> {
    let Some(line_index) = record.line_index_plus1.checked_sub(1) else {
        return Ok(());
    };
    let layout = persisted_content_layout(
        read,
        ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
        "line",
    )?
    .ok_or_else(|| BinaryDbError::missing_data("snapshot references a missing line file"))?;
    let raw = read.read_record(line_record_file_for_layout(layout)?, line_index)?;
    let line = decode_line_record_for_layout(layout, &raw)?;
    if line.is_tombstone() {
        return Err(format!("snapshot references tombstoned line index {line_index}").into());
    }
    let line_name = read.read_payload(
        line_payload_file_for_layout(layout)?,
        line.line_name_offset,
        u32::from(line.line_name_len),
    )?;
    if line_name != payload.line_name.as_bytes() {
        return Err(format!(
            "snapshot line payload {:?} does not match line index {line_index}",
            payload.line_name
        )
        .into());
    }
    Ok(())
}

pub(super) fn validate_optional_record_link<A: ContentReadAccess>(
    access: &A,
    file: BinaryFileId,
    record_index_plus1: u32,
    label: &str,
) -> StoreResult<()> {
    let Some(index) = record_index_plus1.checked_sub(1) else {
        return Ok(());
    };
    if index >= access.record_count(file)? {
        return Err(format!("{label} index {index} is out of range").into());
    }
    Ok(())
}

pub(super) fn set_payload_flags(
    record: &mut ServerBinarySnapshotRecord,
    payload: &ServerBinarySnapshotPayload,
) {
    if payload
        .message
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        record.snapshot_meta |= ServerBinarySnapshotRecord::META_HAS_MESSAGE;
    } else {
        record.snapshot_meta &= !ServerBinarySnapshotRecord::META_HAS_MESSAGE;
    }
    if payload.line_name.is_empty() {
        record.snapshot_meta &= !ServerBinarySnapshotRecord::META_HAS_LINE_NAME_PAYLOAD;
    } else {
        record.snapshot_meta |= ServerBinarySnapshotRecord::META_HAS_LINE_NAME_PAYLOAD;
    }
}

pub(super) fn validate_line_record(record: &ServerBinaryLineRecord) -> StoreResult<()> {
    if record.line_meta & !ServerBinaryLineRecord::META_KNOWN != 0 {
        return Err("ServerBinaryLineRecord has unsupported line_meta bits".into());
    }
    if record.reserved0 != 0 {
        return Err("ServerBinaryLineRecord reserved0 must be zero".into());
    }
    if record.line_name_len == 0 {
        return Err("ServerBinaryLineRecord line_name_len must not be zero".into());
    }
    if record.line_name_offset < 4 {
        return Err("ServerBinaryLineRecord line_name_offset must follow the layout header".into());
    }
    if record.is_archived() != (record.archived_at_s != 0) {
        return Err(
            "ServerBinaryLineRecord archived bit and archived_at_s must be set together".into(),
        );
    }
    Ok(())
}

pub(super) fn validate_snapshot_record(record: &ServerBinarySnapshotRecord) -> StoreResult<()> {
    const RESERVED_META_BITS: u8 = 0b0100_0000;
    if record.snapshot_meta & RESERVED_META_BITS != 0 {
        return Err("ServerBinarySnapshotRecord has reserved snapshot_meta bits set".into());
    }
    if record.history_flags & !ServerBinarySnapshotRecord::HISTORY_REMOTE_HEAD_BOUNDARY != 0 {
        return Err("ServerBinarySnapshotRecord has reserved history_flags bits set".into());
    }
    if record.is_remote_head_history_boundary()
        && (!record.has_parent_edges_authority() || record.parent_snapshot_index_plus1 != 0)
    {
        return Err(
            "ServerBinarySnapshotRecord remote history boundary has invalid parent authority"
                .into(),
        );
    }
    if record.snapshot_hash48 >> 48 != 0 {
        return Err("ServerBinarySnapshotRecord snapshot_hash48 high 16 bits must be zero".into());
    }
    if record.payload_len > 0 && record.payload_offset < 4 {
        return Err(
            "ServerBinarySnapshotRecord payload_offset must follow the layout header".into(),
        );
    }
    if record.has_root_locator() != (record.root_tree_pack_index_plus1 != 0) {
        return Err(
            "ServerBinarySnapshotRecord root flag and pack index must be set together".into(),
        );
    }
    Ok(())
}

pub(super) fn require_layout<const LAYOUT: u32>(label: &str) -> StoreResult<()> {
    if LAYOUT == SERVER_CONTENT_BINARY_LAYOUT_ID {
        Ok(())
    } else {
        Err(BinaryDbError::layout_mismatch(format!(
            "unsupported Binary DB {label} layout: {LAYOUT}; supported layout is {SERVER_CONTENT_BINARY_LAYOUT_ID}"
        )))
    }
}

pub(super) fn require_len(raw: &[u8], expected: usize, label: &str) -> StoreResult<()> {
    if raw.len() == expected {
        Ok(())
    } else {
        Err(format!("{label} requires {expected} bytes, got {}", raw.len()).into())
    }
}

pub(super) fn normalize_line_name(line_name: &str) -> StoreResult<&str> {
    let line_name = line_name.trim();
    if line_name.is_empty() {
        Err("line_name must not be empty".into())
    } else {
        Ok(line_name)
    }
}

pub fn server_line_name_hash64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

pub fn server_snapshot_hash48_from_id(snapshot_id: &str) -> StoreResult<u64> {
    let value = snapshot_id.trim();
    let Some(hex) = value.get(4..) else {
        return Err(format!("id `{value}` must start with SNP-").into());
    };
    if !value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("SNP-"))
    {
        return Err(format!("id `{value}` must start with SNP-").into());
    }
    if hex.len() != 12 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("id `{value}` must contain exactly 12 hex chars").into());
    }
    u64::from_str_radix(hex, 16)
        .map_err(|err| format!("invalid snapshot id `{value}`: {err}").into())
}

pub fn server_snapshot_id_from_hash48(hash48: u64) -> String {
    format!("SNP-{hash48:012X}")
}

pub fn server_snapshot_id_index_key(snapshot_id: &str) -> StoreResult<[u8; 8]> {
    Ok(server_snapshot_hash48_from_id(snapshot_id)?.to_le_bytes())
}
