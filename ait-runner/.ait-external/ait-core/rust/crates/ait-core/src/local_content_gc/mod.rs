use crate::binary_db::{BinaryDbReadTxn, LocalBinaryDbFs};
use crate::content_binary_db::{
    BinaryDbBlobStore, BinaryDbTreePackStore, BinaryDbTreeStore, BinaryObjectPackMemberKind,
    BinarySnapshotView, LocalContentBinaryDb,
};
use crate::json_support::{json, JsonMap, JsonNumber as Number, JsonValue};
use crate::pack_substrate::{
    build_storage_validation_summary, summarize_pack_archives, summarize_tree_pack_archives,
};
use crate::snapshot_dag::topological_snapshot_order;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Debug)]
struct ReachableState {
    blob_ids: BTreeSet<String>,
    tree_ids: BTreeSet<String>,
}

mod binary_db_prune;
mod binary_db_support;
mod common_support;
mod contracts;
mod validation;

use self::binary_db_prune::*;
use self::binary_db_support::*;
use self::common_support::*;
pub use self::contracts::{
    prune_orphan_packs_with_local_content_maintenance_store,
    storage_stats_with_local_content_maintenance_store,
    validate_with_local_content_maintenance_store, LocalContentMaintenanceStore,
    LocalContentOrphanPackPruneStore, LocalContentStatsOptions, LocalContentStatsStore,
    LocalContentValidationStore,
};
use self::validation::*;

#[cfg(test)]
mod tests;

#[derive(Default)]
struct BlobCounts {
    total_blobs: i64,
    packed_blob_count: i64,
    packed_full_blob_count: i64,
    packed_delta_blob_count: i64,
    total_blob_bytes: i64,
    packed_blob_bytes: i64,
    packed_full_blob_bytes: i64,
    packed_delta_blob_bytes: i64,
}

#[derive(Default)]
struct TreeStats {
    tree_count: i64,
    tree_entry_count: i64,
    reachable_tree_count: i64,
    reachable_tree_entry_count: i64,
    unreachable_tree_count: i64,
    unreachable_tree_entry_count: i64,
    tree_pack_count: i64,
    reachable_tree_pack_count: i64,
    orphan_tree_pack_count: i64,
}
