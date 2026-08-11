use crate::binary_db::{
    BinaryDb, BinaryDbCommandScope, BinaryDbError, BinaryDbFsyncPolicy, BinaryDbReadScope,
    BinaryDbReadTxn, BinaryDbStoreFsyncPolicy, BinaryDbWriteTxn, BinaryFileId, BinaryPayloadFileId,
    PayloadRange, StorePath, StoreResult,
};

pub mod adapters;
pub mod read;
pub mod schema;
pub mod write;

pub use adapters::{
    LocalPlanBinaryDb, LocalRepositoryPlanStore, RemoteFsPlanBinaryDb, RemotePlanBinaryDb,
    RemotePlanSyncArtifactAttachTxn, RemotePlanSyncCommitPoint, RemotePlanSyncPublishTxn,
};
pub use read::{
    PlanHeadScanFilter, PlanHeadView, PlanItemView, PlanRevisionSummaryView, PlanRevisionView,
    PlanSummaryView,
};
pub use schema::{
    PlanCodec, PlanItemCheckboxState, PlanItemCodec, PlanItemPayload, PlanItemRecord, PlanPayload,
    PlanRecord, PlanRevisionCodec, PlanRevisionPayload, PlanRevisionRecord, PlanState, PLAN_BIN,
    PLAN_ITEM_BIN, PLAN_ITEM_PAYLOAD_BIN, PLAN_ITEM_RECORD_SIZE, PLAN_LAYOUT_ID, PLAN_PAYLOAD_BIN,
    PLAN_RECORD_SIZE, PLAN_REVISION_BIN, PLAN_REVISION_PAYLOAD_BIN, PLAN_REVISION_RECORD_SIZE,
};
#[cfg(test)]
pub(crate) use schema::{
    PLAN_ITEM_RECORD_SIZE_USIZE, PLAN_RECORD_SIZE_USIZE, PLAN_REVISION_RECORD_SIZE_USIZE,
};
pub use write::{PlanBinaryDbCommitPoint, PlanBinaryDbWritePurpose, PlanBinaryDbWriteTxn};

pub mod local {
    pub use super::adapters::local::*;
}

pub mod remote {
    pub use super::adapters::remote::*;
}

/// Render the selected repository's zero-based `plan.bin` record ordinal.
/// This is a direct projection and never consults or persists an ID mapping.
pub fn repository_plan_id(plan_ordinal: u32) -> String {
    format!("PR-{plan_ordinal}")
}

/// Parse the canonical repository-local HTTP/CLI Plan identity.
pub fn parse_repository_plan_id(value: &str) -> Result<u32, String> {
    let raw = value.strip_prefix("PR-").ok_or_else(|| {
        format!("Plan Binary DB identity `{value}` is not canonical; use `PR-<plan.bin ordinal>`.")
    })?;
    let ordinal = raw.parse::<u32>().map_err(|_| {
        format!("Plan Binary DB identity `{value}` must contain a u32 plan.bin ordinal.")
    })?;
    if repository_plan_id(ordinal) != value {
        return Err(format!(
            "Plan Binary DB identity `{value}` is not canonical; use `{}`.",
            repository_plan_id(ordinal)
        ));
    }
    Ok(ordinal)
}

/// In-memory update for the root locator already stored inside a canonical
/// `plan_revision.bin` record. This is not a persisted record family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRevisionRootUpdate {
    pub plan_revision_index: u32,
    pub root_tree_pack_index_plus1: u32,
    pub root_entry_ordinal: u32,
}

pub struct BinaryDbPlanStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    db: B,
}

impl<B, const WRITE_LAYOUT: u32> BinaryDbPlanStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub fn new(db: B) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &B {
        &self.db
    }

    pub fn authority_root(&self) -> &StorePath {
        self.db.authority_root()
    }

    pub fn begin_read_txn(&self) -> BinaryDbReadTxn<'_, B> {
        BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Plan)
    }

    pub fn begin_write_txn(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbWriteTxn<'_, B, BinaryDbStoreFsyncPolicy<'_, B>>> {
        Self::ensure_supported_write_layout()?;
        BinaryDbWriteTxn::begin(&self.db, command_scope)
    }

    pub fn begin_write_txn_with_fsync_policy<F>(
        &self,
        command_scope: BinaryDbCommandScope,
        fsync_policy: F,
    ) -> StoreResult<BinaryDbWriteTxn<'_, B, F>>
    where
        F: BinaryDbFsyncPolicy,
    {
        Self::ensure_supported_write_layout()?;
        BinaryDbWriteTxn::begin_with_fsync_policy(&self.db, command_scope, fsync_policy)
    }

    pub fn plan_file() -> BinaryFileId {
        PlanCodec::<PLAN_LAYOUT_ID>::record_file()
    }

    pub fn plan_payload_file() -> BinaryPayloadFileId {
        PlanCodec::<PLAN_LAYOUT_ID>::payload_file()
    }

    pub fn plan_revision_file() -> BinaryFileId {
        PlanRevisionCodec::<PLAN_LAYOUT_ID>::record_file()
    }

    pub fn plan_revision_payload_file() -> BinaryPayloadFileId {
        PlanRevisionCodec::<PLAN_LAYOUT_ID>::payload_file()
    }

    pub fn plan_item_file() -> BinaryFileId {
        PlanItemCodec::<PLAN_LAYOUT_ID>::record_file()
    }

    pub fn plan_item_payload_file() -> BinaryPayloadFileId {
        PlanItemCodec::<PLAN_LAYOUT_ID>::payload_file()
    }

    fn ensure_supported_write_layout() -> StoreResult<()> {
        if WRITE_LAYOUT == PLAN_LAYOUT_ID {
            return Ok(());
        }
        Err(BinaryDbError::unsupported(format!(
            "unsupported Plan Binary DB write layout {WRITE_LAYOUT}; supported layout is {PLAN_LAYOUT_ID}"
        )))
    }

    fn plan_file_for(layout: u32) -> BinaryFileId {
        BinaryFileId::new(PLAN_BIN, layout, PLAN_RECORD_SIZE)
    }

    fn plan_payload_file_for(layout: u32) -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(PLAN_PAYLOAD_BIN, layout)
    }

    fn plan_revision_file_for(layout: u32) -> BinaryFileId {
        BinaryFileId::new(PLAN_REVISION_BIN, layout, PLAN_REVISION_RECORD_SIZE)
    }

    fn plan_revision_payload_file_for(layout: u32) -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(PLAN_REVISION_PAYLOAD_BIN, layout)
    }

    fn plan_item_file_for(layout: u32) -> BinaryFileId {
        BinaryFileId::new(PLAN_ITEM_BIN, layout, PLAN_ITEM_RECORD_SIZE)
    }

    fn plan_item_payload_file_for(layout: u32) -> BinaryPayloadFileId {
        BinaryPayloadFileId::new(PLAN_ITEM_PAYLOAD_BIN, layout)
    }

    fn decode_plan_record(layout: u32, raw: &[u8]) -> StoreResult<PlanRecord> {
        if layout == PLAN_LAYOUT_ID {
            PlanCodec::<PLAN_LAYOUT_ID>::decode_record(raw)
        } else {
            Self::unsupported_layout("plan.bin", layout)
        }
    }

    fn decode_plan_payload(layout: u32, raw: &[u8]) -> StoreResult<PlanPayload> {
        if layout == PLAN_LAYOUT_ID {
            PlanCodec::<PLAN_LAYOUT_ID>::decode_payload(raw)
        } else {
            Self::unsupported_layout("plan_payload.bin", layout)
        }
    }

    fn decode_plan_revision_record(layout: u32, raw: &[u8]) -> StoreResult<PlanRevisionRecord> {
        if layout == PLAN_LAYOUT_ID {
            PlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_record(raw)
        } else {
            Self::unsupported_layout("plan_revision.bin", layout)
        }
    }

    fn decode_plan_revision_payload(layout: u32, raw: &[u8]) -> StoreResult<PlanRevisionPayload> {
        if layout == PLAN_LAYOUT_ID {
            PlanRevisionCodec::<PLAN_LAYOUT_ID>::decode_payload(raw)
        } else {
            Self::unsupported_layout("plan_revision_payload.bin", layout)
        }
    }

    fn decode_plan_item_record(layout: u32, raw: &[u8]) -> StoreResult<PlanItemRecord> {
        if layout == PLAN_LAYOUT_ID {
            PlanItemCodec::<PLAN_LAYOUT_ID>::decode_record(raw)
        } else {
            Self::unsupported_layout("plan_item.bin", layout)
        }
    }

    fn decode_plan_item_payload(layout: u32, raw: &[u8]) -> StoreResult<PlanItemPayload> {
        if layout == PLAN_LAYOUT_ID {
            PlanItemCodec::<PLAN_LAYOUT_ID>::decode_payload(raw)
        } else {
            Self::unsupported_layout("plan_item_payload.bin", layout)
        }
    }

    fn unsupported_layout<T>(file_name: &str, layout: u32) -> StoreResult<T> {
        Err(BinaryDbError::unsupported(format!(
            "unsupported {file_name} layout {layout}; supported layout is {PLAN_LAYOUT_ID}"
        )))
    }

    pub fn append_plan_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &PlanRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = PlanCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::plan_file(), &bytes)
    }

    pub fn append_plan_payload<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        payload: &PlanPayload,
    ) -> StoreResult<PayloadRange>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = PlanCodec::<WRITE_LAYOUT>::encode_payload(payload)?;
        write.append_payload(Self::plan_payload_file(), &bytes)
    }

    pub fn append_plan<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        mut record: PlanRecord,
        payload: &PlanPayload,
    ) -> StoreResult<(u32, PlanRecord)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let range = self.append_plan_payload(write, payload)?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16::try_from(range.payload_len).map_err(|_| {
            format!(
                "plan payload length exceeds u16::MAX: {}",
                range.payload_len
            )
        })?;
        let index = self.append_plan_record(write, &record)?;
        Ok((index, record))
    }

    pub fn overwrite_plan<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        plan_index: u32,
        mut record: PlanRecord,
        payload: &PlanPayload,
    ) -> StoreResult<PlanRecord>
    where
        F: BinaryDbFsyncPolicy,
    {
        let range = self.append_plan_payload(write, payload)?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16::try_from(range.payload_len).map_err(|_| {
            format!(
                "plan payload length exceeds u16::MAX: {}",
                range.payload_len
            )
        })?;
        let bytes = PlanCodec::<WRITE_LAYOUT>::encode_record(&record)?;
        write.overwrite_record(Self::plan_file(), plan_index, &bytes)?;
        Ok(record)
    }

    pub fn overwrite_plan_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        plan_index: u32,
        record: &PlanRecord,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = PlanCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.overwrite_record(Self::plan_file(), plan_index, &bytes)
    }

    pub fn append_plan_revision_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &PlanRevisionRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = PlanRevisionCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::plan_revision_file(), &bytes)
    }

    pub fn append_plan_revision_payload<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        payload: &PlanRevisionPayload,
    ) -> StoreResult<PayloadRange>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = PlanRevisionCodec::<WRITE_LAYOUT>::encode_payload(payload)?;
        write.append_payload(Self::plan_revision_payload_file(), &bytes)
    }

    pub fn append_plan_revision<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        mut record: PlanRevisionRecord,
        payload: &PlanRevisionPayload,
    ) -> StoreResult<(u32, PlanRevisionRecord)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let range = self.append_plan_revision_payload(write, payload)?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16::try_from(range.payload_len).map_err(|_| {
            format!(
                "plan revision payload length exceeds u16::MAX: {}",
                range.payload_len
            )
        })?;
        let index = self.append_plan_revision_record(write, &record)?;
        Ok((index, record))
    }

    pub fn overwrite_plan_revision_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        revision_index: u32,
        record: &PlanRevisionRecord,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = PlanRevisionCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.overwrite_record(Self::plan_revision_file(), revision_index, &bytes)
    }

    pub fn overwrite_plan_revision_root<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        root: &PlanRevisionRootUpdate,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let raw = write.read_record(Self::plan_revision_file(), root.plan_revision_index)?;
        let mut revision = PlanRevisionCodec::<WRITE_LAYOUT>::decode_record(&raw)?;
        revision.root_tree_pack_index_plus1 = root.root_tree_pack_index_plus1;
        revision.root_entry_ordinal = root.root_entry_ordinal;
        self.overwrite_plan_revision_record(write, root.plan_revision_index, &revision)
    }

    pub fn append_plan_item_record<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        record: &PlanItemRecord,
    ) -> StoreResult<u32>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = PlanItemCodec::<WRITE_LAYOUT>::encode_record(record)?;
        write.append_record(Self::plan_item_file(), &bytes)
    }

    pub fn append_plan_item_payload<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        payload: &PlanItemPayload,
    ) -> StoreResult<PayloadRange>
    where
        F: BinaryDbFsyncPolicy,
    {
        let bytes = PlanItemCodec::<WRITE_LAYOUT>::encode_payload(payload)?;
        write.append_payload(Self::plan_item_payload_file(), &bytes)
    }

    pub fn append_plan_item<F>(
        &self,
        write: &mut BinaryDbWriteTxn<'_, B, F>,
        mut record: PlanItemRecord,
        payload: &PlanItemPayload,
    ) -> StoreResult<(u32, PlanItemRecord)>
    where
        F: BinaryDbFsyncPolicy,
    {
        let range = self.append_plan_item_payload(write, payload)?;
        record.payload_offset = range.payload_offset;
        record.payload_len = u16::try_from(range.payload_len).map_err(|_| {
            format!(
                "plan item payload length exceeds u16::MAX: {}",
                range.payload_len
            )
        })?;
        let index = self.append_plan_item_record(write, &record)?;
        Ok((index, record))
    }

    pub fn read_plan_record<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        index: u32,
    ) -> StoreResult<PlanRecord> {
        let layout = read.layout_id(Self::plan_file())?;
        let raw = read.read_record(Self::plan_file_for(layout), index)?;
        Self::decode_plan_record(layout, &raw)
    }

    pub fn read_plan_payload<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        record: &PlanRecord,
    ) -> StoreResult<PlanPayload> {
        let layout = read.layout_id(Self::plan_file())?;
        let raw = read.read_payload(
            Self::plan_payload_file_for(layout),
            record.payload_offset,
            u32::from(record.payload_len),
        )?;
        Self::decode_plan_payload(layout, &raw)
    }

    pub fn read_plan<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        index: u32,
    ) -> StoreResult<(PlanRecord, PlanPayload)> {
        let record = self.read_plan_record(read, index)?;
        let payload = self.read_plan_payload(read, &record)?;
        Ok((record, payload))
    }

    pub fn read_current_plan<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        plan_index: u32,
    ) -> StoreResult<(PlanRecord, PlanPayload)> {
        self.read_plan(read, plan_index)
    }

    pub fn read_plan_revision_record<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        index: u32,
    ) -> StoreResult<PlanRevisionRecord> {
        let layout = read.layout_id(Self::plan_revision_file())?;
        let raw = read.read_record(Self::plan_revision_file_for(layout), index)?;
        Self::decode_plan_revision_record(layout, &raw)
    }

    pub fn read_plan_revision_payload<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        record: &PlanRevisionRecord,
    ) -> StoreResult<PlanRevisionPayload> {
        let layout = read.layout_id(Self::plan_revision_file())?;
        let raw = read.read_payload(
            Self::plan_revision_payload_file_for(layout),
            record.payload_offset,
            u32::from(record.payload_len),
        )?;
        Self::decode_plan_revision_payload(layout, &raw)
    }

    pub fn read_plan_revision<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        index: u32,
    ) -> StoreResult<(PlanRevisionRecord, PlanRevisionPayload)> {
        let record = self.read_plan_revision_record(read, index)?;
        let payload = self.read_plan_revision_payload(read, &record)?;
        Ok((record, payload))
    }

    pub fn read_current_plan_revision<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        revision_index: u32,
    ) -> StoreResult<(PlanRevisionRecord, PlanRevisionPayload)> {
        self.read_plan_revision(read, revision_index)
    }

    pub fn read_plan_item_record<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        index: u32,
    ) -> StoreResult<PlanItemRecord> {
        let layout = read.layout_id(Self::plan_item_file())?;
        let raw = read.read_record(Self::plan_item_file_for(layout), index)?;
        Self::decode_plan_item_record(layout, &raw)
    }

    pub fn read_plan_item_payload<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        record: &PlanItemRecord,
    ) -> StoreResult<PlanItemPayload> {
        let layout = read.layout_id(Self::plan_item_file())?;
        let raw = read.read_payload(
            Self::plan_item_payload_file_for(layout),
            record.payload_offset,
            u32::from(record.payload_len),
        )?;
        Self::decode_plan_item_payload(layout, &raw)
    }

    pub fn read_plan_item<'a>(
        &self,
        read: &BinaryDbReadTxn<'a, B>,
        index: u32,
    ) -> StoreResult<(PlanItemRecord, PlanItemPayload)> {
        let record = self.read_plan_item_record(read, index)?;
        let payload = self.read_plan_item_payload(read, &record)?;
        Ok((record, payload))
    }
}

#[cfg(test)]
mod tests;
