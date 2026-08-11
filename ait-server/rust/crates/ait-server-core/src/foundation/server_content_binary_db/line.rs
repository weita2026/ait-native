use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryLineRecord {
    pub line_meta: u8,
    pub reserved0: u8,
    pub line_name_len: u16,
    pub line_name_offset: u64,
    pub head_snapshot_index_plus1: u32,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    pub archived_at_s: u64,
}

impl ServerBinaryLineRecord {
    pub const META_ARCHIVED: u8 = 0b0000_0001;
    pub const META_TOMBSTONE: u8 = 0b0000_0010;
    pub(super) const META_KNOWN: u8 = Self::META_ARCHIVED | Self::META_TOMBSTONE;

    pub fn is_archived(&self) -> bool {
        self.line_meta & Self::META_ARCHIVED != 0
    }

    pub fn is_tombstone(&self) -> bool {
        self.line_meta & Self::META_TOMBSTONE != 0
    }

    pub fn head_snapshot_index(&self) -> Option<u32> {
        self.head_snapshot_index_plus1.checked_sub(1)
    }
}

pub struct ServerBinaryLineCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> ServerBinaryLineCodec<LAYOUT> {
    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(
            SERVER_LINE_BIN,
            LAYOUT,
            SERVER_LINE_RECORD_SIZE,
            BinaryDbFileFamily::Content,
        )
    }

    pub fn payload_file() -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(
            SERVER_LINE_NAME_PAYLOAD_BIN,
            LAYOUT,
            BinaryDbFileFamily::Content,
        )
    }

    pub fn name_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(
            SERVER_LINE_NAME_IDX,
            LAYOUT,
            8,
            false,
            BinaryDbFileFamily::Content,
        )
    }

    pub fn encode_record(record: &ServerBinaryLineRecord) -> StoreResult<Vec<u8>> {
        require_layout::<LAYOUT>("line")?;
        validate_line_record(record)?;
        let mut out = Vec::with_capacity(SERVER_LINE_RECORD_SIZE as usize);
        out.push(record.line_meta);
        out.push(record.reserved0);
        out.extend_from_slice(&record.line_name_len.to_le_bytes());
        out.extend_from_slice(&record.line_name_offset.to_le_bytes());
        out.extend_from_slice(&record.head_snapshot_index_plus1.to_le_bytes());
        out.extend_from_slice(&record.created_at_s.to_le_bytes());
        out.extend_from_slice(&record.updated_at_s.to_le_bytes());
        out.extend_from_slice(&record.archived_at_s.to_le_bytes());
        require_len(
            &out,
            SERVER_LINE_RECORD_SIZE as usize,
            "ServerBinaryLineRecord",
        )?;
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<ServerBinaryLineRecord> {
        require_layout::<LAYOUT>("line")?;
        require_len(
            raw,
            SERVER_LINE_RECORD_SIZE as usize,
            "ServerBinaryLineRecord",
        )?;
        let record = ServerBinaryLineRecord {
            line_meta: raw[0],
            reserved0: raw[1],
            line_name_len: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
            line_name_offset: u64::from_le_bytes(raw[4..12].try_into().unwrap()),
            head_snapshot_index_plus1: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            created_at_s: u64::from_le_bytes(raw[16..24].try_into().unwrap()),
            updated_at_s: u64::from_le_bytes(raw[24..32].try_into().unwrap()),
            archived_at_s: u64::from_le_bytes(raw[32..40].try_into().unwrap()),
        };
        validate_line_record(&record)?;
        Ok(record)
    }
}

#[derive(Clone, Debug)]
pub struct ServerBinaryDbLineStore<B, const WRITE_LAYOUT: u32>
where
    B: ServerRemoteBinaryDb,
{
    db: B,
}

impl<B, const WRITE_LAYOUT: u32> ServerBinaryDbLineStore<B, WRITE_LAYOUT>
where
    B: ServerRemoteBinaryDb,
{
    pub fn new(db: B) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &B {
        &self.db
    }

    pub fn line_by_name(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        line_name: &str,
    ) -> StoreResult<Option<(u32, ServerBinaryLineRecord)>> {
        find_line::<B, WRITE_LAYOUT>(read, line_name)
    }

    pub fn line_name(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        record: &ServerBinaryLineRecord,
    ) -> StoreResult<String> {
        let layout = persisted_content_layout(
            read,
            ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            "line",
        )?
        .ok_or_else(|| BinaryDbError::missing_data("canonical line file is missing"))?;
        let bytes = read.read_payload(
            line_payload_file_for_layout(layout)?,
            record.line_name_offset,
            u32::from(record.line_name_len),
        )?;
        String::from_utf8(bytes).map_err(|err| format!("line_name is not UTF-8: {err}").into())
    }

    pub fn all_lines(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StoreResult<Vec<(u32, String, ServerBinaryLineRecord)>> {
        let Some(layout) = persisted_content_layout(
            read,
            ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            "line",
        )?
        else {
            return Ok(Vec::new());
        };
        let file = line_record_file_for_layout(layout)?;
        let count = read.record_count(file.clone())?;
        let mut lines = Vec::with_capacity(count as usize);
        for index in 0..count {
            let record =
                decode_line_record_for_layout(layout, &read.read_record(file.clone(), index)?)?;
            if !record.is_tombstone() {
                let line_name = self.line_name(read, &record)?;
                lines.push((index, line_name, record));
            }
        }
        Ok(lines)
    }
}

impl<B, const WRITE_LAYOUT: u32> ServerBinaryDbLineStore<B, WRITE_LAYOUT>
where
    B: ServerRemoteBinaryDb + BinaryDbIndexAppender,
{
    pub(crate) fn line_by_name_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, B, F>,
        line_name: &str,
    ) -> StoreResult<Option<(u32, ServerBinaryLineRecord)>>
    where
        F: BinaryDbFsyncPolicy,
    {
        require_layout::<WRITE_LAYOUT>("line write")?;
        find_line_in_write::<B, _, WRITE_LAYOUT>(write, normalize_line_name(line_name)?)
    }

    pub(crate) fn set_line_head_in_tx<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        line_name: &str,
        expected_head_snapshot_index: Option<u32>,
        head_snapshot_index: u32,
        updated_at_s: u64,
    ) -> StoreResult<ServerBinaryLineRecord>
    where
        F: BinaryDbFsyncPolicy,
    {
        require_layout::<WRITE_LAYOUT>("line write")?;
        let line_name = normalize_line_name(line_name)?;
        let (index, mut record) = find_line_in_write::<B, _, WRITE_LAYOUT>(write, line_name)?
            .ok_or_else(|| format!("unknown canonical line: {line_name}"))?;
        if record.head_snapshot_index() != expected_head_snapshot_index {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "canonical line {line_name} advanced under the ServerLand write lock"
            )));
        }
        if record.is_archived() {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "canonical line {line_name} is archived and cannot move"
            )));
        }
        let head_snapshot_index_plus1 = head_snapshot_index
            .checked_add(1)
            .ok_or_else(|| "canonical snapshot index exceeds u32".to_string())?;
        validate_snapshot_link::<_, WRITE_LAYOUT>(write, head_snapshot_index_plus1)?;
        record.head_snapshot_index_plus1 = head_snapshot_index_plus1;
        record.updated_at_s = updated_at_s;
        let raw = ServerBinaryLineCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        write.overwrite_record(
            ServerBinaryLineCodec::<WRITE_LAYOUT>::record_file(),
            index,
            &raw,
        )?;
        Ok(record)
    }

    pub fn create_line(
        &self,
        line_name: &str,
        head_snapshot_index_plus1: u32,
        created_at_s: u64,
    ) -> StoreResult<u32> {
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerContent)?;
        let index =
            self.create_line_in_tx(&mut tx, line_name, head_snapshot_index_plus1, created_at_s)?;
        tx.commit()?;
        Ok(index)
    }

    pub(crate) fn create_line_in_tx<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        line_name: &str,
        head_snapshot_index_plus1: u32,
        created_at_s: u64,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        require_layout::<WRITE_LAYOUT>("line write")?;
        let line_name = normalize_line_name(line_name)?;
        if find_line_in_write::<B, _, WRITE_LAYOUT>(tx, line_name)?.is_some() {
            return Err(format!("line already exists: {line_name}").into());
        }
        validate_snapshot_link::<_, WRITE_LAYOUT>(tx, head_snapshot_index_plus1)?;
        let line_name_len = u16::try_from(line_name.len())
            .map_err(|_| "line_name exceeds u16::MAX bytes".to_string())?;
        let range = tx.append_payload(
            ServerBinaryLineCodec::<WRITE_LAYOUT>::payload_file(),
            line_name.as_bytes(),
        )?;
        let record = ServerBinaryLineRecord {
            line_meta: 0,
            reserved0: 0,
            line_name_len,
            line_name_offset: range.payload_offset,
            head_snapshot_index_plus1,
            created_at_s,
            updated_at_s: created_at_s,
            archived_at_s: 0,
        };
        let raw = ServerBinaryLineCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        let index = tx.append_record(ServerBinaryLineCodec::<WRITE_LAYOUT>::record_file(), &raw)?;
        tx.append_index_candidate(
            ServerBinaryLineCodec::<WRITE_LAYOUT>::name_index(),
            &server_line_name_hash64(line_name.as_bytes()).to_le_bytes(),
            index,
        )?;
        Ok(index)
    }

    pub fn set_line_head(
        &self,
        line_name: &str,
        head_snapshot_index_plus1: u32,
        updated_at_s: u64,
    ) -> StoreResult<ServerBinaryLineRecord> {
        require_layout::<WRITE_LAYOUT>("line write")?;
        let line_name = normalize_line_name(line_name)?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerContent)?;
        let (index, mut record) = find_line_in_write::<B, _, WRITE_LAYOUT>(&tx, line_name)?
            .ok_or_else(|| format!("unknown line: {line_name}"))?;
        validate_snapshot_link::<_, WRITE_LAYOUT>(&tx, head_snapshot_index_plus1)?;
        record.head_snapshot_index_plus1 = head_snapshot_index_plus1;
        record.updated_at_s = updated_at_s;
        let raw = ServerBinaryLineCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        tx.overwrite_record(
            ServerBinaryLineCodec::<WRITE_LAYOUT>::record_file(),
            index,
            &raw,
        )?;
        tx.commit()?;
        Ok(record)
    }

    pub fn set_line_head_if_current(
        &self,
        line_name: &str,
        expected: &ServerBinaryLineRecord,
        head_snapshot_index_plus1: u32,
        updated_at_s: u64,
    ) -> StoreResult<ServerBinaryLineRecord> {
        require_layout::<WRITE_LAYOUT>("line write")?;
        let line_name = normalize_line_name(line_name)?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerContent)?;
        let (index, mut record) = find_line_in_write::<B, _, WRITE_LAYOUT>(&tx, line_name)?
            .ok_or_else(|| format!("unknown line: {line_name}"))?;
        if &record != expected {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "line {line_name} advanced under the ServerContent write lock"
            )));
        }
        if record.is_archived() {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "line {line_name} is archived and cannot move"
            )));
        }
        validate_snapshot_link::<_, WRITE_LAYOUT>(&tx, head_snapshot_index_plus1)?;
        record.head_snapshot_index_plus1 = head_snapshot_index_plus1;
        record.updated_at_s = updated_at_s;
        let raw = ServerBinaryLineCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        tx.overwrite_record(
            ServerBinaryLineCodec::<WRITE_LAYOUT>::record_file(),
            index,
            &raw,
        )?;
        tx.commit()?;
        Ok(record)
    }

    pub fn archive_line(
        &self,
        line_name: &str,
        archived_at_s: u64,
    ) -> StoreResult<ServerBinaryLineRecord> {
        require_layout::<WRITE_LAYOUT>("line write")?;
        if archived_at_s == 0 {
            return Err("archived_at_s must not be zero".into());
        }
        let line_name = normalize_line_name(line_name)?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerContent)?;
        let (index, mut record) = find_line_in_write::<B, _, WRITE_LAYOUT>(&tx, line_name)?
            .ok_or_else(|| format!("unknown line: {line_name}"))?;
        record.line_meta |= ServerBinaryLineRecord::META_ARCHIVED;
        record.archived_at_s = archived_at_s;
        record.updated_at_s = archived_at_s;
        let raw = ServerBinaryLineCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        tx.overwrite_record(
            ServerBinaryLineCodec::<WRITE_LAYOUT>::record_file(),
            index,
            &raw,
        )?;
        tx.commit()?;
        Ok(record)
    }

    pub fn archive_line_if_current(
        &self,
        line_name: &str,
        expected: &ServerBinaryLineRecord,
        archived_at_s: u64,
    ) -> StoreResult<ServerBinaryLineRecord> {
        require_layout::<WRITE_LAYOUT>("line write")?;
        if archived_at_s == 0 {
            return Err("archived_at_s must not be zero".into());
        }
        let line_name = normalize_line_name(line_name)?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerContent)?;
        let (index, mut record) = find_line_in_write::<B, _, WRITE_LAYOUT>(&tx, line_name)?
            .ok_or_else(|| format!("unknown line: {line_name}"))?;
        if &record != expected {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "line {line_name} advanced under the ServerContent write lock"
            )));
        }
        record.line_meta |= ServerBinaryLineRecord::META_ARCHIVED;
        record.archived_at_s = archived_at_s;
        record.updated_at_s = archived_at_s;
        let raw = ServerBinaryLineCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        tx.overwrite_record(
            ServerBinaryLineCodec::<WRITE_LAYOUT>::record_file(),
            index,
            &raw,
        )?;
        tx.commit()?;
        Ok(record)
    }
}
