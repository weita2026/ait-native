use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbErrorKind, BinaryDbFsyncPolicy,
    BinaryDbReadScope, BinaryDbReadTxn, BinaryDbWriteTxn, BinaryFileId, StoreResult,
};
use crate::content_binary_db::{
    snapshot_id_from_hash48, BinaryDbSnapshotStore, BinarySnapshotCodec, BinarySnapshotKind,
    BinarySnapshotRecord,
};
use crate::stash_store::{
    DroppedStashRecord, NewStashRecord, StashRecord, StashStore, StashStoreResult,
};
use chrono::{DateTime, SecondsFormat, Utc};

pub const BINARY_DB_STASH_LAYOUT_ID: u32 = 1;
pub const STASH_RECORD_SIZE: u32 = 8;
pub const STASH_BIN: &str = "stash.bin";

const STASH_RECORD_SIZE_USIZE: usize = STASH_RECORD_SIZE as usize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryStashRecord {
    pub stash_meta: u8,
    pub reserved0: u8,
    pub reserved1: u16,
    pub stash_snapshot_index: u32,
}

impl BinaryStashRecord {
    pub const META_WORKSPACE_CLEARED: u8 = 0b0000_0001;
    pub const META_TOMBSTONE: u8 = 0b1000_0000;
    const META_KNOWN: u8 = Self::META_WORKSPACE_CLEARED | Self::META_TOMBSTONE;

    pub fn workspace_cleared(&self) -> bool {
        self.stash_meta & Self::META_WORKSPACE_CLEARED != 0
    }

    pub fn is_tombstone(&self) -> bool {
        self.stash_meta & Self::META_TOMBSTONE != 0
    }
}

pub struct BinaryStashCodec<const LAYOUT: u32>;

impl<const LAYOUT: u32> BinaryStashCodec<LAYOUT> {
    pub fn record_file() -> BinaryFileId {
        BinaryFileId::new(STASH_BIN, LAYOUT, STASH_RECORD_SIZE)
    }

    pub fn encode_record(record: &BinaryStashRecord) -> StoreResult<Vec<u8>> {
        require_supported_layout::<LAYOUT>()?;
        validate_stash_record(record)?;
        let mut out = Vec::with_capacity(STASH_RECORD_SIZE_USIZE);
        out.push(record.stash_meta);
        out.push(record.reserved0);
        out.extend_from_slice(&record.reserved1.to_le_bytes());
        out.extend_from_slice(&record.stash_snapshot_index.to_le_bytes());
        if out.len() != STASH_RECORD_SIZE_USIZE {
            return Err("BinaryStashRecord encoding produced wrong length".into());
        }
        Ok(out)
    }

    pub fn decode_record(raw: &[u8]) -> StoreResult<BinaryStashRecord> {
        require_supported_layout::<LAYOUT>()?;
        if raw.len() != STASH_RECORD_SIZE_USIZE {
            return Err(format!(
                "BinaryStashRecord requires {STASH_RECORD_SIZE_USIZE} bytes, got {}",
                raw.len()
            )
            .into());
        }
        let record = BinaryStashRecord {
            stash_meta: raw[0],
            reserved0: raw[1],
            reserved1: u16::from_le_bytes([raw[2], raw[3]]),
            stash_snapshot_index: u32::from_le_bytes(raw[4..8].try_into().unwrap()),
        };
        validate_stash_record(&record)?;
        Ok(record)
    }
}

#[derive(Clone, Debug)]
pub struct BinaryDbStashStore<B: BinaryDb + Clone, const WRITE_LAYOUT: u32> {
    db: B,
    snapshots: BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbStashStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + Clone,
{
    pub fn new(db: B) -> Self {
        Self {
            snapshots: BinaryDbSnapshotStore::new(db.clone(), "."),
            db,
        }
    }

    pub fn db(&self) -> &B {
        &self.db
    }

    pub fn record_file() -> BinaryFileId {
        BinaryStashCodec::<WRITE_LAYOUT>::record_file()
    }

    pub fn append_stash_record<F>(
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &BinaryStashRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = BinaryStashCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::record_file(), &bytes)
    }

    pub fn list_stashes_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
    ) -> StashStoreResult<Vec<StashRecord>> {
        let Some(layout) = persisted_stash_layout(read).map_err(String::from)? else {
            return Ok(Vec::new());
        };
        let file = stash_record_file_for_layout(layout).map_err(String::from)?;
        let count = read.record_count(file.clone())?;
        let mut rows = Vec::new();
        for stash_index in 0..count {
            let raw = read.read_record(file.clone(), stash_index)?;
            let record = decode_stash_record_for_layout(layout, &raw)?;
            if record.is_tombstone() {
                continue;
            }
            let stash = self.stash_view(read, stash_index, &record)?;
            let created_at_s = self
                .snapshots
                .read_snapshot_record(read, record.stash_snapshot_index)?
                .created_at_s;
            rows.push((created_at_s, stash_index, stash));
        }
        rows.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        Ok(rows.into_iter().map(|(_, _, stash)| stash).collect())
    }

    pub fn stash_by_id_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        stash_id: &str,
    ) -> StashStoreResult<Option<StashRecord>> {
        let Some(stash_index) = stash_index_from_id(stash_id) else {
            return Ok(None);
        };
        let Some(layout) = persisted_stash_layout(read).map_err(String::from)? else {
            return Ok(None);
        };
        let file = stash_record_file_for_layout(layout).map_err(String::from)?;
        if stash_index >= read.record_count(file.clone())? {
            return Ok(None);
        }
        let raw = read.read_record(file, stash_index)?;
        let record = decode_stash_record_for_layout(layout, &raw)?;
        if record.is_tombstone() {
            return Ok(None);
        }
        self.stash_view(read, stash_index, &record).map(Some)
    }

    fn stash_view(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        stash_index: u32,
        record: &BinaryStashRecord,
    ) -> StashStoreResult<StashRecord> {
        let snapshot = self
            .snapshots
            .snapshot_view_at(read, record.stash_snapshot_index)?;
        if snapshot.record.is_tombstone() {
            return Err(format!(
                "stash {} references tombstoned snapshot index {}",
                stash_id_from_index(stash_index)?,
                record.stash_snapshot_index
            ));
        }
        if snapshot.record.kind() != BinarySnapshotKind::Stash {
            return Err(format!(
                "stash {} references non-stash snapshot {}",
                stash_id_from_index(stash_index)?,
                snapshot.snapshot_id
            ));
        }
        let created_at = format_epoch_seconds(snapshot.created_at_s)?;
        Ok(StashRecord {
            stash_id: stash_id_from_index(stash_index)?,
            snapshot_id: snapshot.snapshot_id,
            source_line_name: snapshot.payload.line_name,
            base_snapshot_id: snapshot.parent_snapshot_id.clone(),
            message: snapshot.payload.message,
            workspace_cleared: record.workspace_cleared(),
            created_at: created_at.clone(),
            snapshot_created_at: created_at,
            snapshot_kind: "stash".to_string(),
            parent_snapshot_id: snapshot.parent_snapshot_id,
            file_count: i64::try_from(snapshot.file_count)
                .map_err(|_| "stash snapshot file_count overflows i64".to_string())?,
            total_bytes: i64::try_from(snapshot.total_bytes)
                .map_err(|_| "stash snapshot total_bytes overflows i64".to_string())?,
        })
    }

    fn validate_new_stash_snapshot(&self, record: NewStashRecord<'_>) -> StashStoreResult<u32> {
        let read = BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Content);
        let snapshot = self
            .snapshots
            .get_snapshot_view(&read, record.snapshot_id)?
            .ok_or_else(|| format!("Unknown snapshot: {}", record.snapshot_id))?;
        if snapshot.snapshot_kind != "stash" {
            return Err(format!(
                "Snapshot {} is not a stash snapshot.",
                record.snapshot_id
            ));
        }
        if snapshot.payload.line_name != record.source_line_name {
            return Err(format!(
                "stash source line {} does not match snapshot line {}",
                record.source_line_name, snapshot.payload.line_name
            ));
        }
        if snapshot.parent_snapshot_id.as_deref() != record.base_snapshot_id {
            return Err(format!(
                "stash base snapshot does not match parent of {}",
                record.snapshot_id
            ));
        }
        if snapshot.payload.message.as_deref() != record.message {
            return Err(format!(
                "stash message does not match snapshot {}",
                record.snapshot_id
            ));
        }
        Ok(snapshot.snapshot_index)
    }

    fn snapshot_is_live_stash<F>(
        write: &BinaryDbWriteTxn<'_, B, F>,
        snapshot_index: u32,
    ) -> StoreResult<BinarySnapshotRecord>
    where
        F: BinaryDbFsyncPolicy,
    {
        let raw = write.read_record(
            BinarySnapshotCodec::<WRITE_LAYOUT>::record_file(),
            snapshot_index,
        )?;
        let record = BinarySnapshotCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        if record.is_tombstone() || record.kind() != BinarySnapshotKind::Stash {
            return Err(
                format!("stash references non-live stash snapshot index {snapshot_index}").into(),
            );
        }
        Ok(record)
    }
}

impl<B, const WRITE_LAYOUT: u32> StashStore for BinaryDbStashStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + Clone,
{
    fn create_stash(&self, record: NewStashRecord<'_>) -> StashStoreResult<StashRecord> {
        require_supported_layout::<WRITE_LAYOUT>().map_err(String::from)?;
        let snapshot_index = self.validate_new_stash_snapshot(record)?;
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        Self::snapshot_is_live_stash(&write, snapshot_index)?;
        let stash_index = Self::append_stash_record(
            &mut write,
            &BinaryStashRecord {
                stash_meta: if record.workspace_cleared {
                    BinaryStashRecord::META_WORKSPACE_CLEARED
                } else {
                    0
                },
                reserved0: 0,
                reserved1: 0,
                stash_snapshot_index: snapshot_index,
            },
        )?;
        write.commit()?;
        let stash_id = stash_id_from_index(stash_index)?;
        self.stash_by_id(&stash_id)?
            .ok_or_else(|| format!("newly appended stash {stash_id} disappeared"))
    }

    fn list_stashes(&self) -> StashStoreResult<Vec<StashRecord>> {
        let read = BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Content);
        self.list_stashes_with_read(&read)
    }

    fn stash_by_id(&self, stash_id: &str) -> StashStoreResult<Option<StashRecord>> {
        let read = BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Content);
        self.stash_by_id_with_read(&read, stash_id)
    }

    fn drop_stash(&self, stash_id: &str) -> StashStoreResult<Option<DroppedStashRecord>> {
        require_supported_layout::<WRITE_LAYOUT>().map_err(String::from)?;
        let Some(stash_index) = stash_index_from_id(stash_id) else {
            return Ok(None);
        };
        let Some(stash) = self.stash_by_id(stash_id)? else {
            return Ok(None);
        };
        let mut write = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::ContentWrite)?;
        let count = write.record_count(Self::record_file())?;
        if stash_index >= count {
            write.abort()?;
            return Ok(None);
        }
        let raw = write.read_record(Self::record_file(), stash_index)?;
        let mut record = BinaryStashCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        if record.is_tombstone() {
            write.abort()?;
            return Ok(None);
        }
        let mut snapshot_record =
            Self::snapshot_is_live_stash(&write, record.stash_snapshot_index)?;
        if snapshot_id_from_hash48(snapshot_record.snapshot_hash48()) != stash.snapshot_id {
            return Err(format!("stash {stash_id} snapshot changed while dropping"));
        }
        let mut has_other_live_reference = false;
        for candidate_index in 0..count {
            if candidate_index == stash_index {
                continue;
            }
            let candidate = BinaryStashCodec::<WRITE_LAYOUT>::decode_record(
                &write.read_record(Self::record_file(), candidate_index)?,
            )?;
            if !candidate.is_tombstone()
                && candidate.stash_snapshot_index == record.stash_snapshot_index
            {
                has_other_live_reference = true;
                break;
            }
        }
        record.stash_meta |= BinaryStashRecord::META_TOMBSTONE;
        write.overwrite_record(
            Self::record_file(),
            stash_index,
            &BinaryStashCodec::<WRITE_LAYOUT>::encode_record(&record)?,
        )?;
        let snapshot_deleted = !has_other_live_reference;
        if snapshot_deleted {
            snapshot_record.snapshot_meta |= BinarySnapshotRecord::META_TOMBSTONE;
            write.overwrite_record(
                BinarySnapshotCodec::<WRITE_LAYOUT>::record_file(),
                record.stash_snapshot_index,
                &BinarySnapshotCodec::<WRITE_LAYOUT>::encode_record(&snapshot_record)?,
            )?;
        }
        write.commit()?;
        Ok(Some(DroppedStashRecord {
            stash,
            snapshot_deleted,
        }))
    }
}

pub fn stash_id_from_index(stash_index: u32) -> StashStoreResult<String> {
    let ordinal = stash_index
        .checked_add(1)
        .ok_or_else(|| "stash index cannot be rendered as a public ID".to_string())?;
    Ok(format!("STH-{ordinal:06}"))
}

pub fn stash_index_from_id(stash_id: &str) -> Option<u32> {
    let digits = stash_id.strip_prefix("STH-")?;
    if digits.len() < 6 || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let ordinal = digits.parse::<u32>().ok()?;
    let index = ordinal.checked_sub(1)?;
    (stash_id_from_index(index).ok()?.as_str() == stash_id).then_some(index)
}

fn persisted_stash_layout<B: BinaryDb>(read: &BinaryDbReadTxn<'_, B>) -> StoreResult<Option<u32>> {
    let layout = match read.layout_id(BinaryStashCodec::<BINARY_DB_STASH_LAYOUT_ID>::record_file())
    {
        Ok(layout) => layout,
        Err(error) if error.kind() == BinaryDbErrorKind::MissingData => return Ok(None),
        Err(error) => return Err(error),
    };
    require_supported_persisted_layout(layout)?;
    Ok(Some(layout))
}

fn stash_record_file_for_layout(layout: u32) -> StoreResult<BinaryFileId> {
    require_supported_persisted_layout(layout)?;
    Ok(BinaryFileId::new(STASH_BIN, layout, STASH_RECORD_SIZE))
}

fn decode_stash_record_for_layout(layout: u32, raw: &[u8]) -> StoreResult<BinaryStashRecord> {
    match layout {
        BINARY_DB_STASH_LAYOUT_ID => {
            BinaryStashCodec::<BINARY_DB_STASH_LAYOUT_ID>::decode_record(raw)
        }
        _ => {
            require_supported_persisted_layout(layout)?;
            unreachable!("supported persisted stash layout must have a codec")
        }
    }
}

fn require_supported_persisted_layout(layout: u32) -> StoreResult<()> {
    if layout == BINARY_DB_STASH_LAYOUT_ID {
        Ok(())
    } else {
        Err(BinaryDbError::layout_mismatch(format!(
            "unsupported persisted Binary DB stash layout: {layout}; supported layout is {BINARY_DB_STASH_LAYOUT_ID}"
        )))
    }
}

fn require_supported_layout<const LAYOUT: u32>() -> StoreResult<()> {
    if LAYOUT == BINARY_DB_STASH_LAYOUT_ID {
        Ok(())
    } else {
        Err(format!("unsupported Binary DB stash layout: {LAYOUT}").into())
    }
}

fn validate_stash_record(record: &BinaryStashRecord) -> StoreResult<()> {
    if record.stash_meta & !BinaryStashRecord::META_KNOWN != 0 {
        return Err("BinaryStashRecord has unsupported stash_meta bits".into());
    }
    if record.reserved0 != 0 {
        return Err("BinaryStashRecord reserved0 must be zero".into());
    }
    if record.reserved1 != 0 {
        return Err("BinaryStashRecord reserved1 must be zero".into());
    }
    Ok(())
}

fn format_epoch_seconds(value: u64) -> StashStoreResult<String> {
    let seconds = i64::try_from(value)
        .map_err(|_| format!("stash snapshot epoch seconds cannot be rendered: {value}"))?;
    let timestamp = DateTime::<Utc>::from_timestamp(seconds, 0)
        .ok_or_else(|| format!("invalid stash snapshot epoch seconds: {value}"))?;
    Ok(timestamp.to_rfc3339_opts(SecondsFormat::Secs, true))
}

#[cfg(test)]
mod tests;
