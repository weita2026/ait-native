// HTTP compatibility metadata derived in memory from the canonical compact
// Plan records and repository authority binding. This module defines no
// persisted file and does not extend the layout-1 Plan schema.

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ServerPlanMeta {
    pub(super) plan_index: u32,
    pub(super) repo_id: String,
    pub(super) title: String,
    pub(super) status: String,
    pub(super) created_by: Option<String>,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}
