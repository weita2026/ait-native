use super::*;

impl RepositoryPackInventory {
    pub fn new(repo_name: impl Into<String>) -> Self {
        Self {
            repo_name: repo_name.into(),
            ..Self::default()
        }
    }

    pub fn is_empty_pack_inventory(&self) -> bool {
        self.object_packs.is_empty() && self.tree_packs.is_empty()
    }

    pub fn object_and_tree_pack_formats_are_all_zstd(&self) -> bool {
        self.object_packs
            .iter()
            .all(|row| row.pack_format == PackFormatKind::ZstdChunkedV1)
            && self
                .tree_packs
                .iter()
                .all(|row| row.pack_format == TreePackFormatKind::ZstdChunkedTreeV1)
    }
}
