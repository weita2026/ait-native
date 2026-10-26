#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRecord {
    pub plan_meta: u8,
    pub reserved0: u8,
    pub payload_len: u16,
    pub payload_offset: u64,
    pub latest_revision_index_plus1: u32,
    pub published_plan_index_plus1: u32,
    pub published_latest_revision_index_plus1: u32,
    pub created_at_s: u64,
    pub updated_at_s: u64,
    pub published_at_s: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRevisionRecord {
    pub revision_meta: u8,
    pub reserved0: u8,
    pub payload_len: u16,
    pub revision_number: u16,
    pub item_count: u16,
    pub payload_offset: u64,
    pub plan_index: u32,
    pub previous_revision_index_plus1: u32,
    pub item_start_index: u32,
    pub published_revision_index_plus1: u32,
    pub root_tree_pack_index_plus1: u32,
    pub root_entry_ordinal: u32,
    pub created_at_s: u64,
    pub published_at_s: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItemRecord {
    pub item_meta: u8,
    pub reserved0: u8,
    pub payload_len: u16,
    pub payload_offset: u64,
    pub line_number: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanState {
    Draft,
    Archived,
    Superseded,
    Reserved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanItemCheckboxState {
    None,
    Open,
    Done,
    Reserved,
}
impl PlanRecord {
    pub fn state(&self) -> PlanState {
        match self.plan_meta & 0b0000_0011 {
            0 => PlanState::Draft,
            1 => PlanState::Archived,
            2 => PlanState::Superseded,
            _ => PlanState::Reserved,
        }
    }

    pub fn status_name(&self) -> &'static str {
        match self.state() {
            PlanState::Draft => "draft",
            PlanState::Archived => "archived",
            PlanState::Superseded => "superseded",
            PlanState::Reserved => "reserved",
        }
    }

    pub fn is_published(&self) -> bool {
        self.plan_meta & 0b0000_0100 != 0
    }

    pub fn is_tombstone(&self) -> bool {
        self.plan_meta & 0b0010_0000 != 0
    }

    pub fn is_active(&self) -> bool {
        !self.is_tombstone() && matches!(self.state(), PlanState::Draft)
    }

    pub fn latest_revision_index(&self) -> Option<u32> {
        self.latest_revision_index_plus1.checked_sub(1)
    }

    pub fn published_plan_index(&self) -> Option<u32> {
        self.published_plan_index_plus1.checked_sub(1)
    }

    pub fn published_latest_revision_index(&self) -> Option<u32> {
        self.published_latest_revision_index_plus1.checked_sub(1)
    }
}

impl PlanRevisionRecord {
    pub fn is_published(&self) -> bool {
        self.revision_meta & 0b0000_0001 != 0
    }

    pub fn is_tombstone(&self) -> bool {
        self.revision_meta & 0b0001_0000 != 0
    }

    pub fn previous_revision_index(&self) -> Option<u32> {
        self.previous_revision_index_plus1.checked_sub(1)
    }

    pub fn published_revision_index(&self) -> Option<u32> {
        self.published_revision_index_plus1.checked_sub(1)
    }

    pub fn root_tree_pack_index(&self) -> Option<u32> {
        self.root_tree_pack_index_plus1.checked_sub(1)
    }
}

impl PlanItemRecord {
    pub fn checkbox_state(&self) -> PlanItemCheckboxState {
        match self.item_meta & 0b0000_0011 {
            0 => PlanItemCheckboxState::None,
            1 => PlanItemCheckboxState::Open,
            2 => PlanItemCheckboxState::Done,
            _ => PlanItemCheckboxState::Reserved,
        }
    }

    pub fn checkbox_state_name(&self) -> &'static str {
        match self.checkbox_state() {
            PlanItemCheckboxState::None => "none",
            PlanItemCheckboxState::Open => "open",
            PlanItemCheckboxState::Done => "done",
            PlanItemCheckboxState::Reserved => "reserved",
        }
    }

    pub fn has_item_ref(&self) -> bool {
        self.item_meta & 0b0000_0100 != 0
    }

    pub fn is_taskable_hint(&self) -> bool {
        self.item_meta & 0b0000_1000 != 0
    }
}
