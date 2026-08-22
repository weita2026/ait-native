use super::api::*;
use super::zstd_bulk::*;
use crate::foundation::pack_substrate::{
    read_tree_pack_index_with_format, PackEntryArchive, PACK_FORMAT_ZSTD_CHUNKED_V1,
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
    ServerBinarySnapshotRecord, ServerBinaryTreeEntryView, ServerBinaryTreeReadCache,
    SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[path = "service/binary.rs"]
mod binary;
#[path = "service/pack_policy.rs"]
mod pack_policy;

pub use binary::BinaryDbNativeRepositoryService;
pub(super) use binary::{binary_created_at_value, binary_json_text, binary_snapshot_id};
pub(super) use pack_policy::{
    normalize_status, remote_sync_capabilities, repository_json,
    repository_pack_storage_capability_json, REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD,
};

#[derive(Debug, Clone)]
pub struct ServerRuntimePaths {
    pub root: PathBuf,
    pub pack_dir: PathBuf,
    pub tree_pack_dir: PathBuf,
    pub ref_root: PathBuf,
}

impl ServerRuntimePaths {
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

pub(super) fn path_to_string(path: &Path) -> Result<String, NativeRepositoryError> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        NativeRepositoryError::internal(format!("path is not valid UTF-8: {}", path.display()))
    })
}

pub(super) fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
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

#[cfg(test)]
#[path = "service/tests.rs"]
mod tests;
