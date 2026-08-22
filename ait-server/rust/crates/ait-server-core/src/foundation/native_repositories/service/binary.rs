use super::*;

#[path = "binary/line_snapshot.rs"]
mod line_snapshot;
#[path = "binary/repository.rs"]
mod repository;
#[path = "binary/serialization.rs"]
mod serialization;
#[path = "binary/snapshot_export.rs"]
mod snapshot_export;

use line_snapshot::*;
use serialization::*;
use snapshot_export::*;

pub(in crate::foundation::native_repositories) use serialization::{
    binary_created_at_value, binary_json_text, binary_snapshot_id,
};

struct BinaryZstdPullCatalogSnapshot {
    snapshot_id: String,
    parent_snapshot_ids: Vec<String>,
    value: JsonValue,
}

struct BinaryZstdPullCatalog {
    revision: [u8; 32],
    manifest_cache: ServerBinaryTreeReadCache,
    snapshots_by_id: BTreeMap<String, BinaryZstdPullCatalogSnapshot>,
    object_pack_rows_by_id: BTreeMap<String, JsonValue>,
    tree_pack_rows_by_id: BTreeMap<String, JsonValue>,
    blob_locator_rows_by_index: BTreeMap<u32, JsonValue>,
    tree_locator_rows_by_index: BTreeMap<u32, JsonValue>,
    tree_entries_by_index: BTreeMap<u32, Vec<ServerBinaryTreeEntryView>>,
}

#[derive(Default)]
struct BinaryZstdPullCatalogCache {
    current: Mutex<Option<Arc<BinaryZstdPullCatalog>>>,
    #[cfg(test)]
    build_count: std::sync::atomic::AtomicU64,
}

#[derive(Clone)]
pub struct BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb,
{
    db: D,
    default_line: String,
    id_namespace_prefix: String,
    created_at: String,
    pull_catalog_cache: Arc<BinaryZstdPullCatalogCache>,
}
