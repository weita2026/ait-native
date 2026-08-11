use super::api::{
    default_main_line, NativeRepositoryError, RemoteSyncCommitJson, RemoteSyncPlanJson,
    RemoteSyncZstdBulkCommitResponse, RemoteSyncZstdBulkPlanPresence,
    RemoteSyncZstdImportManifestJson, RemoteSyncZstdPullManifestRequest,
};
use super::service::*;
use crate::foundation::pack_substrate::{
    read_pack_index_checksum_with_format, read_pack_index_with_format,
    read_tree_pack_index_checksum_with_format, read_tree_pack_index_with_format,
    read_tree_pack_tree_by_ordinal_with_format, read_tree_pack_tree_with_format,
    ObjectPackIndexJson, PackIndexEntry, TreePackIndexEntry, TreePackIndexJson,
    DEFAULT_MAX_DELTA_CHAIN_DEPTH, PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use serde_json::{json, Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

// Remote-sync metadata names the actual canonical archive formats.  Keeping a
// second HTTP-only spelling makes an object/tree archive fail its typed reader
// even though the uploaded bytes themselves are valid.
pub(super) const REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1: &str = PACK_FORMAT_ZSTD_CHUNKED_V1;
pub(super) const REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1: &str = TREE_PACK_FORMAT_ZSTD_CHUNKED_V1;

#[cfg(feature = "legacy-postgres-runtime")]
#[path = "zstd_bulk/commit.rs"]
mod commit;
#[path = "zstd_bulk/manifest.rs"]
mod manifest;
#[path = "zstd_bulk/pack_io.rs"]
mod pack_io;
#[cfg(feature = "legacy-postgres-runtime")]
#[path = "zstd_bulk/plan.rs"]
mod plan;
#[path = "zstd_bulk/validation.rs"]
mod validation;

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) use commit::zstd_bulk_commit_json;
pub(super) use manifest::{
    binary_zstd_import_manifest_blob_locator_row, binary_zstd_import_manifest_pack_row,
    binary_zstd_import_manifest_snapshot_row, binary_zstd_import_manifest_tree_locator_row,
};
#[cfg(feature = "legacy-postgres-runtime")]
pub(super) use manifest::{get_zstd_import_manifest_json, get_zstd_pull_manifest_json};
pub(super) use pack_io::{
    binary_zstd_pack_metadata_object, binary_zstd_pack_upload_response,
    uploaded_tree_pack_root_index, uploaded_zstd_pack_index,
    validate_remote_sync_uploaded_zstd_pack_index_metadata, zstd_pack_index_from_bytes,
};
#[cfg(feature = "legacy-postgres-runtime")]
pub(super) use pack_io::{
    get_zstd_bulk_object_pack_bytes, get_zstd_bulk_tree_pack_bytes,
    object_pack_has_repository_blob_locator, put_zstd_bulk_object_pack_bytes,
    put_zstd_bulk_tree_pack_bytes, upsert_zstd_object_pack, upsert_zstd_tree_pack,
    zstd_pack_metadata_from_row, zstd_pack_row_path,
};
#[cfg(feature = "legacy-postgres-runtime")]
pub(super) use plan::zstd_bulk_plan_json;
pub(super) use validation::{
    json_object, json_text_array, json_value_array, object_pack_entry_for_blob_id,
    optional_i64_field, pack_ids_from_array, required_i64_field, tree_pack_entry_for_tree_id,
    validate_object_pack_entry, validate_pack_id_segment, validate_root_tree_locator_index,
    validate_tree_pack_entry, validate_zstd_pack_index_metadata,
};

#[cfg(test)]
pub(super) use pack_io::validate_tree_pack_owner_values;
