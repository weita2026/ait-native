use std::collections::BTreeMap;

use crate::binary_db::{BinaryDb, BinaryDbReadTxn, StoreResult};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryTreeRootLocator {
    pub tree_id: String,
}

impl BinaryTreeRootLocator {
    pub fn new(tree_id: impl Into<String>) -> Self {
        Self {
            tree_id: tree_id.into(),
        }
    }
}

impl From<&str> for BinaryTreeRootLocator {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

pub trait BinaryTreeRootResolver {
    fn resolve_snapshot_root(
        &self,
        snapshot_id: &str,
    ) -> StoreResult<Option<BinaryTreeRootLocator>>;
}

pub trait BinaryTreeRootReadResolver<B: BinaryDb + ?Sized>: BinaryTreeRootResolver {
    fn resolve_snapshot_root_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, B>,
        snapshot_id: &str,
    ) -> StoreResult<Option<BinaryTreeRootLocator>>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StaticBinaryTreeRootResolver {
    roots: BTreeMap<String, BinaryTreeRootLocator>,
}

impl StaticBinaryTreeRootResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_snapshot_root(
        mut self,
        snapshot_id: impl Into<String>,
        root: impl Into<BinaryTreeRootLocator>,
    ) -> Self {
        self.insert_snapshot_root(snapshot_id, root);
        self
    }

    pub fn insert_snapshot_root(
        &mut self,
        snapshot_id: impl Into<String>,
        root: impl Into<BinaryTreeRootLocator>,
    ) {
        self.roots.insert(snapshot_id.into(), root.into());
    }
}

impl BinaryTreeRootResolver for StaticBinaryTreeRootResolver {
    fn resolve_snapshot_root(
        &self,
        snapshot_id: &str,
    ) -> StoreResult<Option<BinaryTreeRootLocator>> {
        Ok(self.roots.get(snapshot_id).cloned())
    }
}

impl<B> BinaryTreeRootReadResolver<B> for StaticBinaryTreeRootResolver
where
    B: BinaryDb + ?Sized,
{
    fn resolve_snapshot_root_with_read(
        &self,
        _read: &BinaryDbReadTxn<'_, B>,
        snapshot_id: &str,
    ) -> StoreResult<Option<BinaryTreeRootLocator>> {
        self.resolve_snapshot_root(snapshot_id)
    }
}
