use super::*;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepositoryPackInventory {
    pub repo_name: String,
    pub object_packs: Vec<RepositoryObjectPackInventoryRow>,
    pub tree_packs: Vec<RepositoryTreePackInventoryRow>,
    pub blob_locators: Vec<RepositoryBlobLocatorInventoryRow>,
    pub tree_locators: Vec<RepositoryTreeLocatorInventoryRow>,
    pub snapshots: Vec<RepositorySnapshotInventoryRow>,
    pub line_heads: Vec<RepositoryLineHeadInventoryRow>,
}
