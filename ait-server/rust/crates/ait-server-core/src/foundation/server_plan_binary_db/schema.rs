use crate::foundation::remote_binary_db::{BinaryDbFileFamily, BinaryFileId, BinaryPayloadFileId};

pub(super) const PLAN_LAYOUT_ID: u32 = 1;

pub(super) const PLAN_BIN: &str = "plan.bin";
pub(super) const PLAN_PAYLOAD_BIN: &str = "plan_payload.bin";
pub(super) const PLAN_REVISION_BIN: &str = "plan_revision.bin";
pub(super) const PLAN_REVISION_PAYLOAD_BIN: &str = "plan_revision_payload.bin";
pub(super) const PLAN_ITEM_BIN: &str = "plan_item.bin";
pub(super) const PLAN_ITEM_PAYLOAD_BIN: &str = "plan_item_payload.bin";

pub(super) const PLAN_RECORD_SIZE: u32 = 48;
pub(super) const PLAN_REVISION_RECORD_SIZE: u32 = 56;
pub(super) const PLAN_ITEM_RECORD_SIZE: u32 = 16;

pub(super) const PLAN_STATE_DRAFT_META: u8 = 0;
pub(super) const PLAN_STATE_ARCHIVED_META: u8 = 1;
pub(super) const PLAN_STATE_SUPERSEDED_META: u8 = 2;
pub(super) const PLAN_STATE_MASK: u8 = 0b0000_0011;
pub(super) const ITEM_STATE_OPEN_META: u8 = 1;
pub(super) const ITEM_STATE_DONE_META: u8 = 2;
pub(super) const ITEM_HAS_REF_META: u8 = 0b0000_0100;
pub(super) const ITEM_TASKABLE_HINT_META: u8 = 0b0000_1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompactPlanFile {
    Plan,
    PlanRevision,
    PlanItem,
}

impl CompactPlanFile {
    fn path(self) -> &'static str {
        match self {
            Self::Plan => PLAN_BIN,
            Self::PlanRevision => PLAN_REVISION_BIN,
            Self::PlanItem => PLAN_ITEM_BIN,
        }
    }

    fn record_size(self) -> u32 {
        match self {
            Self::Plan => PLAN_RECORD_SIZE,
            Self::PlanRevision => PLAN_REVISION_RECORD_SIZE,
            Self::PlanItem => PLAN_ITEM_RECORD_SIZE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompactPlanLayoutSpec {
    pub(super) layout_id: u32,
}

impl CompactPlanLayoutSpec {
    fn record_file(self, file: CompactPlanFile) -> BinaryFileId {
        BinaryFileId::new(
            file.path(),
            self.layout_id,
            file.record_size(),
            BinaryDbFileFamily::Plan,
        )
    }
}

pub(super) const COMPACT_PLAN_LAYOUT_V1: CompactPlanLayoutSpec = CompactPlanLayoutSpec {
    layout_id: PLAN_LAYOUT_ID,
};

pub(super) fn compact_plan_layout_for(layout: u32) -> Result<CompactPlanLayoutSpec, String> {
    match layout {
        PLAN_LAYOUT_ID => Ok(COMPACT_PLAN_LAYOUT_V1),
        _ => Err(format!(
            "unsupported compact Plan Binary DB layout {layout}"
        )),
    }
}

pub(super) fn compact_plan_file_for(
    layout: u32,
    file: CompactPlanFile,
) -> Result<BinaryFileId, String> {
    compact_plan_layout_for(layout).map(|spec| spec.record_file(file))
}

pub(super) fn plan_file_for(layout: u32) -> Result<BinaryFileId, String> {
    compact_plan_file_for(layout, CompactPlanFile::Plan)
}

pub(super) fn plan_revision_file_for(layout: u32) -> Result<BinaryFileId, String> {
    compact_plan_file_for(layout, CompactPlanFile::PlanRevision)
}

pub(super) fn plan_item_file_for(layout: u32) -> Result<BinaryFileId, String> {
    compact_plan_file_for(layout, CompactPlanFile::PlanItem)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlanRecord {
    pub(super) plan_meta: u8,
    pub(super) reserved0: u8,
    pub(super) payload_len: u16,
    pub(super) payload_offset: u64,
    pub(super) latest_revision_index_plus1: u32,
    pub(super) published_plan_index_plus1: u32,
    pub(super) published_latest_revision_index_plus1: u32,
    pub(super) created_at_s: u64,
    pub(super) updated_at_s: u64,
    pub(super) published_at_s: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlanRevisionRecord {
    pub(super) revision_meta: u8,
    pub(super) reserved0: u8,
    pub(super) payload_len: u16,
    pub(super) revision_number: u16,
    pub(super) item_count: u16,
    pub(super) payload_offset: u64,
    pub(super) plan_index: u32,
    pub(super) previous_revision_index_plus1: u32,
    pub(super) item_start_index: u32,
    pub(super) published_revision_index_plus1: u32,
    pub(super) root_tree_pack_index_plus1: u32,
    pub(super) root_entry_ordinal: u32,
    pub(super) created_at_s: u64,
    pub(super) published_at_s: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlanItemRecord {
    pub(super) item_meta: u8,
    pub(super) reserved0: u8,
    pub(super) payload_len: u16,
    pub(super) payload_offset: u64,
    pub(super) line_number: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlanRevisionPayload {
    pub(super) title_snapshot: String,
    pub(super) summary: String,
    pub(super) artifact_path: String,
    pub(super) artifact_selector: String,
    pub(super) artifact_heading: String,
    pub(super) artifact_blob_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PlanItemPayload {
    pub(super) plan_item_ref: String,
    pub(super) text: String,
    pub(super) heading_path: Vec<String>,
}

pub(super) fn plan_file() -> BinaryFileId {
    plan_file_for(PLAN_LAYOUT_ID).expect("server Plan Binary DB v1 plan file must be supported")
}

pub(super) fn plan_payload_file() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new(PLAN_PAYLOAD_BIN, PLAN_LAYOUT_ID, BinaryDbFileFamily::Plan)
}

pub(super) fn plan_revision_file() -> BinaryFileId {
    plan_revision_file_for(PLAN_LAYOUT_ID)
        .expect("server Plan Binary DB v1 plan revision file must be supported")
}

pub(super) fn plan_revision_payload_file() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new(
        PLAN_REVISION_PAYLOAD_BIN,
        PLAN_LAYOUT_ID,
        BinaryDbFileFamily::Plan,
    )
}

pub(super) fn plan_item_file() -> BinaryFileId {
    plan_item_file_for(PLAN_LAYOUT_ID)
        .expect("server Plan Binary DB v1 plan item file must be supported")
}

pub(super) fn plan_item_payload_file() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new(
        PLAN_ITEM_PAYLOAD_BIN,
        PLAN_LAYOUT_ID,
        BinaryDbFileFamily::Plan,
    )
}
