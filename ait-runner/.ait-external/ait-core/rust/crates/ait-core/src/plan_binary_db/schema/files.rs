pub const PLAN_LAYOUT_ID: u32 = 1;

pub const PLAN_BIN: &str = "plan.bin";
pub const PLAN_PAYLOAD_BIN: &str = "plan_payload.bin";
pub const PLAN_REVISION_BIN: &str = "plan_revision.bin";
pub const PLAN_REVISION_PAYLOAD_BIN: &str = "plan_revision_payload.bin";
pub const PLAN_ITEM_BIN: &str = "plan_item.bin";
pub const PLAN_ITEM_PAYLOAD_BIN: &str = "plan_item_payload.bin";

pub const PLAN_RECORD_SIZE: u32 = 48;
pub const PLAN_REVISION_RECORD_SIZE: u32 = 56;
pub const PLAN_ITEM_RECORD_SIZE: u32 = 16;

pub(crate) const PLAN_RECORD_SIZE_USIZE: usize = PLAN_RECORD_SIZE as usize;
pub(crate) const PLAN_REVISION_RECORD_SIZE_USIZE: usize = PLAN_REVISION_RECORD_SIZE as usize;
pub(crate) const PLAN_ITEM_RECORD_SIZE_USIZE: usize = PLAN_ITEM_RECORD_SIZE as usize;
