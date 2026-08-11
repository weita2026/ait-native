use super::api::*;
#[cfg(feature = "legacy-postgres-runtime")]
use super::materialize::*;
#[cfg(feature = "legacy-postgres-runtime")]
use super::snapshot_export::*;
use super::zstd_bulk::*;
#[cfg(feature = "legacy-postgres-runtime")]
use crate::foundation::db::{
    read_server_plane, write_server_plane, NativePostgresDriver, PostgresConnectionPoolRegistry,
    PostgresTimeoutScope,
};
use crate::foundation::pack_substrate::{
    read_pack_entry_with_format, read_tree_pack_index_with_format,
    read_tree_pack_tree_by_ordinal_with_format, read_tree_pack_tree_with_format,
    tree_pack_manifest_path, PackEntryArchive, PACK_FORMAT_ZSTD_CHUNKED_V1,
    TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use crate::foundation::remote_binary_db::{
    BinaryDbCommandScope, BinaryDbError, BinaryDbErrorKind, BinaryDbFsyncPolicy,
    BinaryDbIndexAppender, BinaryDbReadScope, BinaryDbReadTxn, BinaryDbWriteTxn,
    ServerRemoteBinaryDb,
};
#[cfg(test)]
use crate::foundation::server_content_binary_db::ServerBinaryLineCodec;
use crate::foundation::server_content_binary_db::{
    server_snapshot_hash48_from_id, server_snapshot_id_from_hash48, ServerBinaryDbLineStore,
    ServerBinaryDbSnapshotStore, ServerBinaryLineRecord, ServerBinaryRemoteSyncLineWrite,
    ServerBinaryRepositoryContentStore, ServerBinarySnapshotCodec, ServerBinarySnapshotPayload,
    ServerBinarySnapshotRecord, SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use crate::foundation::server_protocol::resolve_server_runtime_root;
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[path = "service/binary.rs"]
mod binary;
#[path = "service/pack_policy.rs"]
mod pack_policy;
#[cfg(feature = "legacy-postgres-runtime")]
#[path = "service/postgres.rs"]
mod postgres;

pub use binary::BinaryDbNativeRepositoryService;
pub(super) use binary::{binary_created_at_value, binary_json_text, binary_snapshot_id};
#[cfg(feature = "legacy-postgres-runtime")]
pub(super) use pack_policy::{
    ensure_zstd_only_repository_flow_allowed, line_json, repository_json_with_pack_storage,
};
#[cfg(test)]
pub(super) use pack_policy::{
    ensure_zstd_only_repository_flow_allowed_for_pack_storage, object_pack_format_summary,
    tree_pack_format_summary,
};
pub(super) use pack_policy::{
    normalize_namespace_prefix, normalize_policy_json, normalize_status, parse_policy_json,
    remote_sync_capabilities, repository_json, repository_pack_storage_capability_json,
    ZstdOnlyRepositoryFlow, REPOSITORY_PACK_STORAGE_CONTRACT,
    REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD,
};
#[cfg(feature = "legacy-postgres-runtime")]
pub use postgres::PostgresNativeRepositoryService;
#[cfg(feature = "legacy-postgres-runtime")]
pub(super) use postgres::{
    blob_bytes_for_blob_id, require_blob_locator_for_repo, runtime_storage_path, select_blob_by_id,
    select_blob_locator_for_repo, select_repository_row, select_snapshot_row,
    snapshot_json_from_row, tree_pack_locator_for_id, tree_pack_locator_for_tree_id,
    update_line_json, validate_existing_snapshot, walk_tree_rows,
};
#[cfg(all(test, feature = "legacy-postgres-runtime"))]
pub(super) use postgres::{
    native_blob_resolver_delta_chain_exceeded, CONTENT_SCHEMA_SQL, REPOSITORY_METADATA_SCHEMA_SQL,
};

const NATIVE_BLOB_RESOLVER_MAX_DELTA_CHAIN_DEPTH: usize = 1024;
#[derive(Debug, Clone)]
pub struct ServerRuntimePaths {
    pub root: PathBuf,
    pub pack_dir: PathBuf,
    pub tree_pack_dir: PathBuf,
    pub ref_root: PathBuf,
}

impl ServerRuntimePaths {
    pub fn discover_from_env() -> Result<Self, String> {
        let root = resolve_server_runtime_root(None)?;
        Self::new(root)
    }

    pub fn new(root: PathBuf) -> Result<Self, String> {
        Ok(Self {
            root: root.clone(),
            pack_dir: root.join("objects").join("packs"),
            tree_pack_dir: root.join("objects").join("tree-packs"),
            ref_root: root.join("refs"),
        })
    }
}

pub(super) fn normalize_required_text(
    value: &str,
    field: &str,
) -> Result<String, NativeRepositoryError> {
    let text = value.trim();
    if text.is_empty() {
        return Err(NativeRepositoryError::bad_request(format!(
            "Field `{field}` must be a non-empty string."
        )));
    }
    Ok(text.to_string())
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

pub(super) fn required_json_text(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<String, String> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Field `{field}` must be a non-empty string."))
}

pub(super) fn optional_json_text(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Option<String> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub(super) fn json_i64(value: &JsonValue) -> Result<i64, NativeRepositoryError> {
    match value {
        JsonValue::Number(number) => number
            .as_i64()
            .ok_or_else(|| NativeRepositoryError::bad_request("numeric field must fit in i64")),
        JsonValue::String(text) => text
            .parse::<i64>()
            .map_err(|_| NativeRepositoryError::bad_request("numeric field must be an integer")),
        JsonValue::Null => Err(NativeRepositoryError::bad_request(
            "numeric field must not be null",
        )),
        _ => Err(NativeRepositoryError::bad_request(
            "numeric field must be an integer",
        )),
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(super) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub(super) fn new_identifier(prefix: &str, seed: &str) -> String {
    let digest = sha256_hex(format!("{seed}|{}|{}", now_rfc3339(), std::process::id()).as_bytes());
    format!("{prefix}-{}", digest[..24].to_ascii_uppercase())
}

pub(super) fn zstd_object_pack_archive_path(paths: &ServerRuntimePaths, pack_id: &str) -> PathBuf {
    paths.pack_dir.join(format!("{pack_id}.zstpack"))
}

pub(super) fn zstd_tree_pack_archive_path(paths: &ServerRuntimePaths, pack_id: &str) -> PathBuf {
    paths.tree_pack_dir.join(format!("{pack_id}.zstpack"))
}

pub(super) fn blob_packref_path(paths: &ServerRuntimePaths, blob_id: &str) -> PathBuf {
    paths.pack_dir.join(format!("{blob_id}.packref"))
}

pub(super) fn stored_path_string(
    paths: &ServerRuntimePaths,
    path: &Path,
) -> Result<String, NativeRepositoryError> {
    if let Ok(relative) = path.strip_prefix(&paths.root) {
        return path_to_string(relative);
    }
    path_to_string(path)
}

pub(super) fn path_to_string(path: &Path) -> Result<String, NativeRepositoryError> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        NativeRepositoryError::internal(format!("path is not valid UTF-8: {}", path.display()))
    })
}

pub(super) fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(super) fn db_internal(exc: ::postgres::Error) -> NativeRepositoryError {
    NativeRepositoryError::internal(exc.to_string())
}

#[derive(Debug, Clone)]
pub(super) struct RepositoryRow {
    pub(super) repo_name: String,
    pub(super) repo_id: String,
    pub(super) default_line: String,
    pub(super) lifecycle_state: String,
    pub(super) id_namespace_prefix: String,
    pub(super) policy_json: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct SnapshotRow {
    pub(super) snapshot_id: String,
    pub(super) repo_name: String,
    pub(super) repo_id: String,
    pub(super) parent_snapshot_id: Option<String>,
    pub(super) root_tree_pack_id: String,
    pub(super) root_entry_ordinal: usize,
    pub(super) manifest_hash: String,
    pub(super) message: Option<String>,
    pub(super) line_name: Option<String>,
    pub(super) file_count: i32,
    pub(super) total_bytes: i64,
    pub(super) created_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct BlobRow {
    pub(super) sha256: String,
    pub(super) pack_id: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct BlobLocatorRow {
    pub(super) blob_id: String,
    pub(super) sha256: String,
    pub(super) size_bytes: i64,
    pub(super) pack_id: String,
    pub(super) pack_entry_type: Option<String>,
    pub(super) pack_base_blob_id: Option<String>,
    pub(super) pack_chain_depth: Option<i64>,
    pub(super) created_at: String,
}

#[derive(Debug, Clone)]
pub(super) struct SnapshotFileEntry {
    pub(super) path: String,
    pub(super) blob_id: String,
    pub(super) size_bytes: i64,
    pub(super) mode: String,
    pub(super) sha256: String,
}

#[cfg(test)]
#[path = "service/tests.rs"]
mod tests;
