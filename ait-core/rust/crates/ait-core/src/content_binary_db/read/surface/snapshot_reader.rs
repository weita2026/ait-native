use super::*;

#[derive(Clone, Debug)]
pub struct BinaryDbSnapshotReader<B, const WRITE_LAYOUT: u32, R = StaticBinaryTreeRootResolver>
where
    B: BinaryDb,
    R: BinaryTreeRootResolver,
{
    tree_store: BinaryDbTreeStore<B, WRITE_LAYOUT>,
    root_resolver: R,
}

impl<B, const WRITE_LAYOUT: u32>
    BinaryDbSnapshotReader<B, WRITE_LAYOUT, StaticBinaryTreeRootResolver>
where
    B: BinaryDb,
{
    pub fn new(tree_store: BinaryDbTreeStore<B, WRITE_LAYOUT>) -> Self {
        Self {
            tree_store,
            root_resolver: StaticBinaryTreeRootResolver::new(),
        }
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
        self.root_resolver.insert_snapshot_root(snapshot_id, root);
    }
}

impl<B, const WRITE_LAYOUT: u32, R> BinaryDbSnapshotReader<B, WRITE_LAYOUT, R>
where
    B: BinaryDb,
    R: BinaryTreeRootResolver,
{
    pub fn with_root_resolver(
        tree_store: BinaryDbTreeStore<B, WRITE_LAYOUT>,
        root_resolver: R,
    ) -> Self {
        Self {
            tree_store,
            root_resolver,
        }
    }

    pub fn tree_store(&self) -> &BinaryDbTreeStore<B, WRITE_LAYOUT> {
        &self.tree_store
    }

    pub fn root_resolver(&self) -> &R {
        &self.root_resolver
    }
}

impl<B, const WRITE_LAYOUT: u32, R> SnapshotReader for BinaryDbSnapshotReader<B, WRITE_LAYOUT, R>
where
    B: BinaryDb,
    R: BinaryTreeRootReadResolver<B>,
{
    fn read_snapshot_manifest(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        let read = self.tree_store.begin_read_txn();
        let Some(root) = self
            .root_resolver
            .resolve_snapshot_root_with_read(&read, snapshot_id)?
        else {
            return Err(format!("snapshot `{snapshot_id}` was not found"));
        };
        let mut files = Map::new();
        let mut visited = BTreeSet::new();
        collect_manifest_rows::<B, WRITE_LAYOUT>(
            &self.tree_store,
            &read,
            &root.tree_id,
            "",
            &mut files,
            &mut visited,
        )?;
        Ok(JsonValue::Object(files))
    }

    fn read_snapshot_root_tree_payload(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<JsonValue>, String> {
        let read = self.tree_store.begin_read_txn();
        let Some(root) = self
            .root_resolver
            .resolve_snapshot_root_with_read(&read, snapshot_id)?
        else {
            return Ok(None);
        };
        Ok(self
            .tree_store
            .read_tree_payload_json(&read, &root.tree_id)?)
    }

    fn read_tree_payload(&self, tree_id: &str) -> Result<Option<JsonValue>, String> {
        let read = self.tree_store.begin_read_txn();
        Ok(self.tree_store.read_tree_payload_json(&read, tree_id)?)
    }
}
