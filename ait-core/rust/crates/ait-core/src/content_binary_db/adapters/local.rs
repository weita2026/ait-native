use crate::binary_db::{
    AuthorityId, BinaryDbCommandScope, BinaryDbReadTxn, LocalBinaryDbFs, LocalStateScope, StorePath,
};
use crate::content_binary_db::{
    blob_id_from_sha256, tree_pack_id_from_hash48, BinaryDbBlobStore,
    BinaryDbContentWriteCoordinator, BinaryDbObjectPackMemberWriteInput, BinaryDbObjectPackStore,
    BinaryDbObjectPackWriteInput, BinaryDbSnapshotReader, BinaryDbSnapshotStore,
    BinaryDbSnapshotWriteInput, BinaryDbTreeEntryWriteInput, BinaryDbTreePackStore,
    BinaryDbTreePackTreeWriteInput, BinaryDbTreePackWriteInput, BinaryDbTreeStore,
    BinaryObjectPackMemberKind,
};
use crate::content_store::BlobStore;
use crate::json_support::{json, JsonMap, JsonNumber as Number, JsonValue};
use crate::local_snapshot::{
    active_workspace_runtime_root, build_snapshot_id_with_parents, build_snapshot_object_pack_id,
    build_tree_records, workspace_ignore_policy, LocalSnapshotBlobReadStore,
    LocalSnapshotReadStore, LocalSnapshotTreeReadStore, LocalSnapshotWriteStore,
    SnapshotAuthoringOptions, SnapshotFileEntry, SnapshotFileRow, SnapshotPathBlobRow,
    SnapshotPathDelta, SnapshotTreeRootLocator, TreeEntryRow, TreeRow,
};
use crate::snapshot_json::SnapshotJson;
use crate::snapshot_store::{
    normalize_snapshot_parent_set, SnapshotParentLink, SnapshotParentLinkPage, SnapshotRecord,
    SnapshotStore, SnapshotStoreResult,
};
use crate::{
    pack_substrate::{
        build_pack_members, build_tree_pack_members, build_typed_pack_members,
        default_object_pack_relative_path, default_tree_pack_relative_path,
        tree_pack_manifest_path, write_pack_archive_with_format,
        write_tree_pack_archive_with_format, write_typed_pack_archive_with_format,
        ObjectPackWriteMember, PackCandidate, CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
        DEFAULT_MAX_DELTA_CHAIN_DEPTH,
    },
    plan_filesystem::{
        is_generated_worktree_cargo_config, list_visible_workspace_entries,
        path_is_projected_out_for_workspace,
    },
    repository_pack_policy::{
        zstd_only_object_pack_write_format, zstd_only_tree_pack_write_format,
    },
    workspace_hash_cache::{
        load_workspace_hash_cache, workspace_file_fingerprint,
        workspace_file_fingerprint_from_visible_metadata, workspace_hash_cache_entry,
        write_workspace_hash_cache,
    },
};
use chrono::{SecondsFormat, Utc};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Instant;

const WORKTREE_CARGO_CONFIG_RELATIVE_PATH: &str = ".cargo/config.toml";

/// Local content Binary DB adapter.
///
/// The adapter is intentionally a thin bundle over the folder-scoped content
/// stores. It keeps local authority identity at the boundary while leaving
/// pack ingestion policy to command-level transactions.
#[derive(Clone, Debug)]
pub struct LocalContentBinaryDb<const WRITE_LAYOUT: u32> {
    db: LocalBinaryDbFs,
    workspace_root: StorePath,
    pack_root: StorePath,
    blobs: BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    snapshots: BinaryDbSnapshotStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    object_packs: BinaryDbObjectPackStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    tree_packs: BinaryDbTreePackStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    trees: BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
}

mod local_read_adapter;
mod local_write_adapter;
mod path_authority_mapping;
mod snapshot_authoring_support;
mod transaction_coordination;

use self::path_authority_mapping::*;
use self::transaction_coordination::*;
