use crate::binary_db::StoreResult;

use crate::plan_binary_db::{
    PlanItemPayload, PlanItemRecord, PlanPayload, PlanRecord, PlanRevisionPayload,
    PlanRevisionRecord,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanSummaryView {
    pub plan_index: u32,
    pub repo_name: Option<String>,
    pub record: PlanRecord,
    pub payload: PlanPayload,
    pub head_revision: Option<PlanRevisionSummaryView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanHeadView {
    pub plan_index: u32,
    pub repo_name: Option<String>,
    pub record: PlanRecord,
    pub payload: PlanPayload,
    pub head_revision: Option<PlanRevisionView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRevisionSummaryView {
    pub revision_index: u32,
    pub record: PlanRevisionRecord,
    pub payload: PlanRevisionPayload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRevisionView {
    pub revision_index: u32,
    pub record: PlanRevisionRecord,
    pub payload: PlanRevisionPayload,
    pub items: Vec<PlanItemView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanItemView {
    pub item_index: u32,
    pub record: PlanItemRecord,
    pub payload: PlanItemPayload,
}
impl PlanHeadView {
    pub fn title_text(&self) -> StoreResult<String> {
        self.payload.title_text()
    }

    pub fn status_name(&self) -> &'static str {
        self.record.status_name()
    }

    pub fn publication_state_name(&self) -> &'static str {
        if self.record.is_published() {
            "published"
        } else {
            "local_draft"
        }
    }

    pub fn head_publication_state_name(&self) -> Option<&'static str> {
        self.head_revision.as_ref().map(|revision| {
            if revision.record.is_published() {
                "published"
            } else {
                "local_draft"
            }
        })
    }

    pub fn head_revision_number(&self) -> Option<u16> {
        self.head_revision
            .as_ref()
            .map(|revision| revision.record.revision_number)
    }

    pub fn head_artifact_path(&self) -> StoreResult<Option<String>> {
        self.head_revision
            .as_ref()
            .map(|revision| revision.payload.artifact_path_text())
            .transpose()
    }

    pub fn head_artifact_selector(&self) -> StoreResult<Option<String>> {
        self.head_revision
            .as_ref()
            .map(|revision| revision.payload.artifact_selector_text())
            .transpose()
    }

    pub fn head_artifact_heading(&self) -> StoreResult<Option<String>> {
        self.head_revision
            .as_ref()
            .map(|revision| revision.payload.artifact_heading_text())
            .transpose()
    }
}
