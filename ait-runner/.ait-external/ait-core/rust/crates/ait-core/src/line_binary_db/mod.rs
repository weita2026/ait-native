use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbErrorKind, BinaryDbFsyncPolicy,
    BinaryDbIndexAppender, BinaryDbReadScope, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId,
    BinaryIndexId, BinaryPayloadFileId, StoreResult,
};
use crate::content_binary_db::{
    snapshot_id_from_hash48, snapshot_id_index_key, BinarySnapshotCodec,
};
use crate::line_store::{LineRecord, LineStore, LineStoreResult};
use chrono::{DateTime, SecondsFormat, Utc};
use std::str;

pub const BINARY_DB_LINE_LAYOUT_ID: u32 = 1;
pub const LINE_RECORD_SIZE: u32 = 40;
pub const LINE_NAME_INDEX_RECORD_SIZE: u32 = 12;
pub const LINE_BIN: &str = "line.bin";
pub const LINE_NAME_PAYLOAD_BIN: &str = "line_name_payload.bin";
pub const LINE_NAME_IDX: &str = "line_name.idx";

const LINE_RECORD_SIZE_USIZE: usize = LINE_RECORD_SIZE as usize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryLineRecord {
    pub line_meta: u8,
    pub reserved0: u8,
    pub line_name_len: u16,
    pub line_name_offset: u64,
    pub head_snapshot_index_plus1: u32,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    pub archived_at_s: u64,
}

impl BinaryLineRecord {
    pub const META_ARCHIVED: u8 = 0b0000_0001;
    pub const META_TOMBSTONE: u8 = 0b0000_0010;
    const META_KNOWN: u8 = Self::META_ARCHIVED | Self::META_TOMBSTONE;

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

pub struct BinaryLineCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> BinaryLineCodec<LAYOUT> {
    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(LINE_BIN, LAYOUT, LINE_RECORD_SIZE)
    }

    pub fn payload_file() -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(LINE_NAME_PAYLOAD_BIN, LAYOUT)
    }

    pub fn name_index() -> BinaryIndexId {
        BinaryIndexId::new_fixed(LINE_NAME_IDX, LAYOUT, 8, false)
    }

    pub fn encode_record(record: &BinaryLineRecord) -> StoreResult<Vec<u8>> {
        require_supported_layout::<LAYOUT>()?;
        validate_line_record(record)?;
        let mut out = Vec::with_capacity(LINE_RECORD_SIZE_USIZE);
        out.push(record.line_meta);
        out.push(record.reserved0);
        out.extend_from_slice(&record.line_name_len.to_le_bytes());
        out.extend_from_slice(&record.line_name_offset.to_le_bytes());
        out.extend_from_slice(&record.head_snapshot_index_plus1.to_le_bytes());
        out.extend_from_slice(&record.created_at_s.to_le_bytes());
        out.extend_from_slice(&record.updated_at_s.to_le_bytes());
        out.extend_from_slice(&record.archived_at_s.to_le_bytes());
        if out.len() != LINE_RECORD_SIZE_USIZE {
            return Err("BinaryLineRecord encoding produced wrong length".into());
        }
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<BinaryLineRecord> {
        require_supported_layout::<LAYOUT>()?;
        if raw.len() != LINE_RECORD_SIZE_USIZE {
            return Err(format!(
                "BinaryLineRecord requires {LINE_RECORD_SIZE_USIZE} bytes, got {}",
                raw.len()
            )
            .into());
        }
        let record = BinaryLineRecord {
            line_meta: raw[0],
            reserved0: raw[1],
            line_name_len: u16::from_le_bytes([raw[2], raw[3]]),
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

fn require_supported_persisted_line_layout(layout: u32) -> StoreResult<()> {
    if layout == BINARY_DB_LINE_LAYOUT_ID {
        Ok(())
    } else {
        Err(BinaryDbError::layout_mismatch(format!(
            "unsupported persisted Binary DB line layout: {layout}; supported layout is {BINARY_DB_LINE_LAYOUT_ID}"
        )))
    }
}

fn persisted_line_layout<B: BinaryDb>(read: &BinaryDbReadTxn<'_, B>) -> StoreResult<Option<u32>> {
    let layout = match read.layout_id(BinaryLineCodec::<BINARY_DB_LINE_LAYOUT_ID>::record_file()) {
        Ok(layout) => layout,
        Err(error) if error.kind() == BinaryDbErrorKind::MissingData => return Ok(None),
        Err(error) => return Err(error),
    };
    require_supported_persisted_line_layout(layout)?;
    Ok(Some(layout))
}

fn line_record_file_for_layout(layout: u32) -> StoreResult<BinaryFileId> {
    require_supported_persisted_line_layout(layout)?;
    Ok(BinaryFileId::new(LINE_BIN, layout, LINE_RECORD_SIZE))
}

fn line_payload_file_for_layout(layout: u32) -> StoreResult<BinaryPayloadFileId> {
    require_supported_persisted_line_layout(layout)?;
    Ok(BinaryPayloadFileId::new(LINE_NAME_PAYLOAD_BIN, layout))
}

fn line_index_for_layout(layout: u32) -> StoreResult<BinaryIndexId> {
    require_supported_persisted_line_layout(layout)?;
    Ok(BinaryIndexId::new_fixed(LINE_NAME_IDX, layout, 8, false))
}

fn decode_line_record_for_layout(layout: u32, raw: &[u8]) -> StoreResult<BinaryLineRecord> {
    match layout {
        BINARY_DB_LINE_LAYOUT_ID => BinaryLineCodec::<BINARY_DB_LINE_LAYOUT_ID>::decode_record(raw),
        _ => {
            require_supported_persisted_line_layout(layout)?;
            unreachable!("supported persisted Line layout must have a codec")
        }
    }
}

#[derive(Clone, Debug)]
pub struct BinaryDbLineStore<B: BinaryDb, const WRITE_LAYOUT: u32> {
    db: B,
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbLineStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn new(db: B) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &B {
        &self.db
    }

    pub fn line_file() -> BinaryFileId {
        BinaryLineCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn line_name_payload_file() -> BinaryPayloadFileId {
        BinaryLineCodec::<WRITE_LAYOUT>::payload_file()
    }

    pub fn line_name_index() -> BinaryIndexId {
        BinaryLineCodec::<WRITE_LAYOUT>::name_index()
    }

    pub fn list_lines_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> LineStoreResult<Vec<LineRecord>> {
        let Some(layout) = persisted_line_layout(read).map_err(String::from)? else {
            return Ok(Vec::new());
        };
        let mut lines = Vec::new();
        let file = line_record_file_for_layout(layout).map_err(String::from)?;
        let count = read.record_count(file.clone())?;
        for line_index in 0..count {
            let raw = read.read_record(file.clone(), line_index)?;
            let record = decode_line_record_for_layout(layout, &raw)?;
            if record.is_tombstone() {
                continue;
            }
            lines.push(line_record_view_for_layout(
                read, line_index, &record, layout,
            )?);
        }
        Ok(lines)
    }

    pub fn line_count_with_read(&self, read: &BinaryDbReadTxn<'_, B>) -> LineStoreResult<usize> {
        let Some(layout) = persisted_line_layout(read).map_err(String::from)? else {
            return Ok(0);
        };
        let file = line_record_file_for_layout(layout).map_err(String::from)?;
        let count = read.record_count(file.clone())?;
        let mut logical_count = 0_usize;
        for line_index in 0..count {
            let raw = read.read_record(file.clone(), line_index)?;
            let record = decode_line_record_for_layout(layout, &raw)?;
            if !record.is_tombstone() {
                logical_count += 1;
            }
        }
        Ok(logical_count)
    }

    pub fn line_index_by_name_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        line_name: &str,
    ) -> LineStoreResult<Option<u32>> {
        Ok(find_line_with_persisted_layout(read, line_name)?.map(|(index, _)| index))
    }

    pub fn line_by_name_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        line_name: &str,
    ) -> LineStoreResult<Option<LineRecord>> {
        let Some(layout) = persisted_line_layout(read).map_err(String::from)? else {
            return Ok(None);
        };
        let Some((line_index, record)) = find_line_for_layout(read, line_name, layout)? else {
            return Ok(None);
        };
        Ok(Some(line_record_view_for_layout(
            read, line_index, &record, layout,
        )?))
    }
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbLineStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    #[expect(
        clippy::too_many_arguments,
        reason = "arguments map directly to the fixed Binary DB line record"
    )]
    fn append_line_in_txn<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        line_name: &str,
        line_meta: u8,
        created_at_s: u64,
        updated_at_s: u64,
        archived_at_s: u64,
        head_snapshot_index_plus1: u32,
    ) -> LineStoreResult<(u32, BinaryLineRecord)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let line_name_bytes = line_name.as_bytes();
        let line_name_len = u16::try_from(line_name_bytes.len())
            .map_err(|_| "line_name exceeds u16::MAX bytes".to_string())?;
        let range = tx.append_payload(Self::line_name_payload_file(), line_name_bytes)?;
        let record = BinaryLineRecord {
            line_meta,
            reserved0: 0,
            line_name_len,
            line_name_offset: range.payload_offset,
            head_snapshot_index_plus1,
            created_at_s,
            updated_at_s,
            archived_at_s,
        };
        let bytes = BinaryLineCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        let line_index = tx.append_record(Self::line_file(), &bytes)?;
        tx.append_index_candidate(
            Self::line_name_index(),
            &line_name_hash64(line_name_bytes).to_le_bytes(),
            line_index,
        )?;
        Ok((line_index, record))
    }

    fn mutate_line<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, B, F>,
        line_index: u32,
        record: &BinaryLineRecord,
    ) -> LineStoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinaryLineCodec::<WRITE_LAYOUT>::encode_record(record)?;
        tx.overwrite_record(Self::line_file(), line_index, &bytes)?;
        Ok(())
    }

    pub fn append_line_for_bootstrap(
        &self,
        line_name: &str,
        status: &str,
        created_at: Option<&str>,
        updated_at: Option<&str>,
        archived_at: Option<&str>,
        head_snapshot_id: Option<&str>,
    ) -> LineStoreResult<u32> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let line_name = normalize_line_name(line_name)?;
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        if find_line(&tx, line_name)?.is_some() {
            return Err(format!("Line already exists: {line_name}"));
        }
        let (line_meta, archived_at_s) = status_fields(status, archived_at)?;
        let created_at_s = parse_epoch_seconds(created_at)?;
        let updated_at_s = parse_epoch_seconds(updated_at)?;
        let head_snapshot_index_plus1 = resolve_snapshot_index_plus1(&tx, head_snapshot_id)?;
        let (line_index, _) = self.append_line_in_txn(
            &mut tx,
            line_name,
            line_meta,
            created_at_s,
            updated_at_s,
            archived_at_s,
            head_snapshot_index_plus1,
        )?;
        tx.commit()?;
        Ok(line_index)
    }
}

impl<B, const WRITE_LAYOUT: u32> LineStore for BinaryDbLineStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    fn list_lines(&self) -> LineStoreResult<Vec<LineRecord>> {
        let read = BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Content);
        self.list_lines_with_read(&read)
    }

    fn line_count(&self) -> LineStoreResult<usize> {
        let read = BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Content);
        self.line_count_with_read(&read)
    }

    fn line_by_name(&self, line_name: &str) -> LineStoreResult<Option<LineRecord>> {
        let read = BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Content);
        self.line_by_name_with_read(&read, line_name)
    }

    fn create_line(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        created_at: &str,
    ) -> LineStoreResult<LineRecord> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let line_name = normalize_line_name(line_name)?;
        let created_at_s = parse_required_epoch_seconds(created_at, "created_at")?;
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        if find_line(&tx, line_name)?.is_some() {
            return Err(format!("Line already exists: {line_name}"));
        }
        let head_snapshot_index_plus1 = resolve_snapshot_index_plus1(&tx, head_snapshot_id)?;
        let (line_index, record) = self.append_line_in_txn(
            &mut tx,
            line_name,
            0,
            created_at_s,
            created_at_s,
            0,
            head_snapshot_index_plus1,
        )?;
        let view = line_record_view(&tx, line_index, &record)?;
        tx.commit()?;
        Ok(view)
    }

    fn archive_line(&self, line_name: &str, archived_at: &str) -> LineStoreResult<LineRecord> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let line_name = normalize_line_name(line_name)?;
        let archived_at_s = parse_required_epoch_seconds(archived_at, "archived_at")?;
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        let (line_index, mut record) =
            find_line(&tx, line_name)?.ok_or_else(|| format!("Unknown line: {line_name}"))?;
        if record.is_archived() {
            return line_record_view(&tx, line_index, &record);
        }
        record.line_meta |= BinaryLineRecord::META_ARCHIVED;
        record.archived_at_s = archived_at_s;
        record.updated_at_s = archived_at_s;
        self.mutate_line(&mut tx, line_index, &record)?;
        let view = line_record_view(&tx, line_index, &record)?;
        tx.commit()?;
        Ok(view)
    }

    fn rename_line(
        &self,
        old_line_name: &str,
        new_line_name: &str,
        updated_at: &str,
    ) -> LineStoreResult<LineRecord> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let old_line_name = normalize_line_name(old_line_name)?;
        let new_line_name = normalize_line_name(new_line_name)?;
        if old_line_name == new_line_name {
            return Err("old and new line names must differ".to_string());
        }
        let updated_at_s = parse_required_epoch_seconds(updated_at, "updated_at")?;
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        if find_line(&tx, new_line_name)?.is_some() {
            return Err(format!("Line already exists: {new_line_name}"));
        }
        let (line_index, mut record) = find_line(&tx, old_line_name)?
            .ok_or_else(|| format!("Unknown line: {old_line_name}"))?;
        let new_name_bytes = new_line_name.as_bytes();
        let new_name_len = u16::try_from(new_name_bytes.len())
            .map_err(|_| "line_name exceeds u16::MAX bytes".to_string())?;
        let range = tx.append_payload(Self::line_name_payload_file(), new_name_bytes)?;
        record.line_name_len = new_name_len;
        record.line_name_offset = range.payload_offset;
        record.updated_at_s = updated_at_s;
        self.mutate_line(&mut tx, line_index, &record)?;
        let new_key = line_name_hash64(new_name_bytes).to_le_bytes();
        if !tx
            .lookup_index(Self::line_name_index(), &new_key)?
            .contains(&line_index)
        {
            tx.append_index_candidate(Self::line_name_index(), &new_key, line_index)?;
        }
        let view = line_record_view(&tx, line_index, &record)?;
        tx.commit()?;
        Ok(view)
    }

    fn delete_line(&self, line_name: &str, deleted_at: &str) -> LineStoreResult<LineRecord> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let line_name = normalize_line_name(line_name)?;
        let deleted_at_s = parse_required_epoch_seconds(deleted_at, "deleted_at")?;
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        let (line_index, mut record) =
            find_line(&tx, line_name)?.ok_or_else(|| format!("Unknown line: {line_name}"))?;
        record.line_meta |= BinaryLineRecord::META_TOMBSTONE | BinaryLineRecord::META_ARCHIVED;
        record.archived_at_s = deleted_at_s;
        record.updated_at_s = deleted_at_s;
        self.mutate_line(&mut tx, line_index, &record)?;
        let view = line_record_view(&tx, line_index, &record)?;
        tx.commit()?;
        Ok(view)
    }

    fn set_line_head(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        updated_at: &str,
    ) -> LineStoreResult<LineRecord> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let line_name = normalize_line_name(line_name)?;
        let updated_at_s = parse_required_epoch_seconds(updated_at, "updated_at")?;
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        let (line_index, mut record) =
            find_line(&tx, line_name)?.ok_or_else(|| format!("Unknown line: {line_name}"))?;
        record.head_snapshot_index_plus1 = resolve_snapshot_index_plus1(&tx, head_snapshot_id)?;
        record.updated_at_s = updated_at_s;
        self.mutate_line(&mut tx, line_index, &record)?;
        let view = line_record_view(&tx, line_index, &record)?;
        tx.commit()?;
        Ok(view)
    }

    fn compare_and_swap_line_head(
        &self,
        line_name: &str,
        expected_head_snapshot_id: Option<&str>,
        head_snapshot_id: Option<&str>,
        updated_at: &str,
    ) -> LineStoreResult<LineRecord> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let line_name = normalize_line_name(line_name)?;
        let updated_at_s = parse_required_epoch_seconds(updated_at, "updated_at")?;
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        let (line_index, mut record) =
            find_line(&tx, line_name)?.ok_or_else(|| format!("Unknown line: {line_name}"))?;
        let current = line_record_view(&tx, line_index, &record)?;
        if current.head_snapshot_id.as_deref() != expected_head_snapshot_id {
            let actual = current
                .head_snapshot_id
                .as_deref()
                .unwrap_or("none")
                .to_string();
            tx.abort()?;
            return Err(format!(
                "Line {line_name} compare-and-swap expected head {} but found {actual}.",
                expected_head_snapshot_id.unwrap_or("none")
            ));
        }
        record.head_snapshot_index_plus1 = resolve_snapshot_index_plus1(&tx, head_snapshot_id)?;
        record.updated_at_s = updated_at_s;
        self.mutate_line(&mut tx, line_index, &record)?;
        let view = line_record_view(&tx, line_index, &record)?;
        tx.commit()?;
        Ok(view)
    }

    fn line_updated_at(&self, line_name: &str) -> LineStoreResult<Option<String>> {
        Ok(self
            .line_by_name(line_name)?
            .and_then(|line| line.updated_at))
    }

    fn set_line_updated_at(
        &self,
        line_name: &str,
        updated_at: Option<&str>,
    ) -> LineStoreResult<()> {
        require_supported_layout::<WRITE_LAYOUT>()?;
        let line_name = normalize_line_name(line_name)?;
        let updated_at_s = parse_epoch_seconds(updated_at)?;
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        let (line_index, mut record) =
            find_line(&tx, line_name)?.ok_or_else(|| format!("Unknown line: {line_name}"))?;
        record.updated_at_s = updated_at_s;
        self.mutate_line(&mut tx, line_index, &record)?;
        tx.commit()?;
        Ok(())
    }

    fn touch_line_updated_at(&self, line_name: &str, updated_at: &str) -> LineStoreResult<()> {
        let _ = parse_required_epoch_seconds(updated_at, "updated_at")?;
        self.set_line_updated_at(line_name, Some(updated_at))
    }
}

trait LineReadAccess {
    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<Vec<u8>>;
    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>>;
    fn lookup_index(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>>;
}

impl<B: BinaryDb> LineReadAccess for BinaryDbReadTxn<'_, B> {
    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<Vec<u8>> {
        BinaryDbReadTxn::read_record(self, file, record_index)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        BinaryDbReadTxn::read_payload(self, file, offset, len)
    }

    fn lookup_index(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>> {
        BinaryDbReadTxn::lookup_index(self, index, key)
    }
}

impl<B, F> LineReadAccess for BinaryDbWriteTxn<'_, B, F>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<Vec<u8>> {
        BinaryDbWriteTxn::read_record(self, file, record_index)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        BinaryDbWriteTxn::read_payload(self, file, offset, len)
    }

    fn lookup_index(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>> {
        BinaryDbWriteTxn::lookup_index(self, index, key)
    }
}

fn find_line<A: LineReadAccess>(
    access: &A,
    line_name: &str,
) -> StoreResult<Option<(u32, BinaryLineRecord)>> {
    find_line_for_layout(access, line_name, BINARY_DB_LINE_LAYOUT_ID)
}

fn find_line_with_persisted_layout<B: BinaryDb>(
    read: &BinaryDbReadTxn<'_, B>,
    line_name: &str,
) -> StoreResult<Option<(u32, BinaryLineRecord)>> {
    let Some(layout) = persisted_line_layout(read)? else {
        return Ok(None);
    };
    find_line_for_layout(read, line_name, layout)
}

fn find_line_for_layout<A: LineReadAccess>(
    access: &A,
    line_name: &str,
    layout: u32,
) -> StoreResult<Option<(u32, BinaryLineRecord)>> {
    let line_name = line_name.trim();
    if line_name.is_empty() {
        return Ok(None);
    }
    let key = line_name_hash64(line_name.as_bytes()).to_le_bytes();
    let candidates = access.lookup_index(line_index_for_layout(layout)?, &key)?;
    for line_index in candidates {
        let raw = access.read_record(line_record_file_for_layout(layout)?, line_index)?;
        let record = decode_line_record_for_layout(layout, &raw)?;
        if record.is_tombstone() {
            continue;
        }
        let stored_name = read_line_name_for_layout(access, &record, layout)?;
        if stored_name == line_name {
            return Ok(Some((line_index, record)));
        }
    }
    Ok(None)
}

fn line_record_view<A: LineReadAccess>(
    access: &A,
    line_index: u32,
    record: &BinaryLineRecord,
) -> LineStoreResult<LineRecord> {
    line_record_view_for_layout(access, line_index, record, BINARY_DB_LINE_LAYOUT_ID)
}

fn line_record_view_for_layout<A: LineReadAccess>(
    access: &A,
    line_index: u32,
    record: &BinaryLineRecord,
    layout: u32,
) -> LineStoreResult<LineRecord> {
    Ok(LineRecord {
        line_id: line_identity_from_index(line_index),
        line_name: read_line_name_for_layout(access, record, layout)?,
        status: if record.is_tombstone() {
            "deleted".to_string()
        } else if record.is_archived() {
            "archived".to_string()
        } else {
            "active".to_string()
        },
        archived_at: format_epoch_seconds(record.archived_at_s)?,
        created_at: format_epoch_seconds(record.created_at_s)?,
        updated_at: format_epoch_seconds(record.updated_at_s)?,
        head_snapshot_id: record
            .head_snapshot_index()
            .map(|index| snapshot_id_at(access, index))
            .transpose()?,
    })
}

fn line_identity_from_index(line_index: u32) -> String {
    format!("LNE-{:08X}", line_index.saturating_add(1))
}

fn read_line_name_for_layout<A: LineReadAccess>(
    access: &A,
    record: &BinaryLineRecord,
    layout: u32,
) -> StoreResult<String> {
    let bytes = access.read_payload(
        line_payload_file_for_layout(layout)?,
        record.line_name_offset,
        u32::from(record.line_name_len),
    )?;
    Ok(str::from_utf8(&bytes)
        .map_err(|err| format!("invalid line_name payload UTF-8: {err}"))?
        .to_string())
}

fn snapshot_id_at<A: LineReadAccess>(access: &A, index: u32) -> LineStoreResult<String> {
    let raw = access.read_record(BinarySnapshotCodec::<1>::record_file(), index)?;
    let record = BinarySnapshotCodec::<1>::decode_record(&raw)?;
    if record.is_tombstone() {
        return Err(format!(
            "line head references tombstoned snapshot index {index}"
        ));
    }
    Ok(snapshot_id_from_hash48(record.snapshot_hash48()))
}

fn resolve_snapshot_index_plus1<A: LineReadAccess>(
    access: &A,
    snapshot_id: Option<&str>,
) -> LineStoreResult<u32> {
    let Some(snapshot_id) = snapshot_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    let key = snapshot_id_index_key(snapshot_id)?;
    for index in access.lookup_index(BinarySnapshotCodec::<1>::id_index(), &key)? {
        if snapshot_id_at(access, index)?.eq_ignore_ascii_case(snapshot_id) {
            return index
                .checked_add(1)
                .ok_or_else(|| "snapshot index overflow".to_string());
        }
    }
    Err(format!("Unknown snapshot: {snapshot_id}"))
}

pub fn line_name_hash64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    bytes.iter().fold(FNV_OFFSET_BASIS, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(FNV_PRIME)
    })
}

pub fn binary_line_index_by_name<B: BinaryDb>(
    read: &BinaryDbReadTxn<'_, B>,
    line_name: &str,
) -> StoreResult<Option<u32>> {
    find_line_with_persisted_layout(read, line_name).map(|value| value.map(|(index, _)| index))
}

pub(crate) fn binary_line_index_by_name_in_write<B, F>(
    write: &BinaryDbWriteTxn<'_, B, F>,
    line_name: &str,
) -> StoreResult<Option<u32>>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    find_line(write, line_name).map(|value| value.map(|(index, _)| index))
}

pub fn binary_line_name_at<B: BinaryDb>(
    read: &BinaryDbReadTxn<'_, B>,
    line_index: u32,
) -> StoreResult<String> {
    let layout = persisted_line_layout(read)?
        .ok_or_else(|| BinaryDbError::missing_data("canonical Binary DB line file is missing"))?;
    let raw = read.read_record(line_record_file_for_layout(layout)?, line_index)?;
    let record = decode_line_record_for_layout(layout, &raw)?;
    // A line tombstone removes only the live ref. Historical Snapshots retain
    // their stable line ordinal and must still be able to resolve its name.
    read_line_name_for_layout(read, &record, layout)
}

fn require_supported_layout<const LAYOUT: u32>() -> StoreResult<()> {
    if LAYOUT == BINARY_DB_LINE_LAYOUT_ID {
        Ok(())
    } else {
        Err(format!("unsupported Binary DB line layout: {LAYOUT}").into())
    }
}

fn validate_line_record(record: &BinaryLineRecord) -> StoreResult<()> {
    if record.line_meta & !BinaryLineRecord::META_KNOWN != 0 {
        return Err("BinaryLineRecord has unsupported line_meta bits".into());
    }
    if record.reserved0 != 0 {
        return Err("BinaryLineRecord reserved0 must be zero".into());
    }
    if record.line_name_len == 0 {
        return Err("BinaryLineRecord line_name_len must not be zero".into());
    }
    if record.line_name_offset < 4 {
        return Err("BinaryLineRecord line_name_offset must follow the layout header".into());
    }
    if record.is_archived() != (record.archived_at_s != 0) {
        return Err("BinaryLineRecord archived bit and archived_at_s must be set together".into());
    }
    Ok(())
}

fn normalize_line_name(line_name: &str) -> LineStoreResult<&str> {
    let line_name = line_name.trim();
    if line_name.is_empty() {
        Err("line_name must not be empty".to_string())
    } else {
        Ok(line_name)
    }
}

fn status_fields(status: &str, archived_at: Option<&str>) -> LineStoreResult<(u8, u64)> {
    match status.trim() {
        "" | "active" => {
            if parse_epoch_seconds(archived_at)? != 0 {
                return Err("active line must not have archived_at".to_string());
            }
            Ok((0, 0))
        }
        "archived" => {
            let archived_at_s = parse_epoch_seconds(archived_at)?;
            if archived_at_s == 0 {
                return Err("archived line requires archived_at".to_string());
            }
            Ok((BinaryLineRecord::META_ARCHIVED, archived_at_s))
        }
        other => Err(format!("unsupported line status: {other}")),
    }
}

fn parse_required_epoch_seconds(value: &str, field: &str) -> LineStoreResult<u64> {
    let parsed = parse_epoch_seconds(Some(value))?;
    if parsed == 0 {
        Err(format!("{field} must not be empty or zero"))
    } else {
        Ok(parsed)
    }
}

fn parse_epoch_seconds(value: Option<&str>) -> LineStoreResult<u64> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(0);
    };
    if let Ok(seconds) = value.parse::<u64>() {
        return Ok(seconds);
    }
    let timestamp = DateTime::parse_from_rfc3339(value)
        .map_err(|err| format!("invalid Binary DB line timestamp `{value}`: {err}"))?
        .with_timezone(&Utc)
        .timestamp();
    u64::try_from(timestamp)
        .map_err(|_| format!("Binary DB line timestamp `{value}` is before the Unix epoch"))
}

fn format_epoch_seconds(value: u64) -> LineStoreResult<Option<String>> {
    if value == 0 {
        return Ok(None);
    }
    let value = i64::try_from(value)
        .map_err(|_| "Binary DB line timestamp exceeds the RFC 3339 range".to_string())?;
    DateTime::<Utc>::from_timestamp(value, 0)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
        .map(Some)
        .ok_or_else(|| "Binary DB line timestamp exceeds the RFC 3339 range".to_string())
}

#[cfg(test)]
mod tests;
