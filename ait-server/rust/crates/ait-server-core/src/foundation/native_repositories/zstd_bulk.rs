use super::api::{default_main_line, NativeRepositoryError};
use super::service::*;
use crate::foundation::pack_substrate::{
    read_pack_index_checksum_with_format, read_pack_index_with_format,
    read_tree_pack_index_checksum_with_format, read_tree_pack_index_with_format,
    ObjectPackIndexJson, PackIndexEntry, TreePackIndexEntry, TreePackIndexJson,
    PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// Remote-sync metadata names the actual canonical archive formats.  Keeping a
// second HTTP-only spelling makes an object/tree archive fail its typed reader
// even though the uploaded bytes themselves are valid.
pub(super) const REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1: &str = PACK_FORMAT_ZSTD_CHUNKED_V1;
pub(super) const REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1: &str = TREE_PACK_FORMAT_ZSTD_CHUNKED_V1;

#[path = "zstd_bulk/manifest.rs"]
mod manifest;
#[path = "zstd_bulk/pack_io.rs"]
mod pack_io;
#[path = "zstd_bulk/validation.rs"]
mod validation;

pub(super) use manifest::{
    binary_zstd_import_manifest_blob_locator_row, binary_zstd_import_manifest_pack_row,
    binary_zstd_import_manifest_snapshot_row, binary_zstd_import_manifest_tree_locator_row,
};
pub(super) use pack_io::{
    binary_zstd_pack_metadata_object, binary_zstd_pack_upload_response,
    uploaded_tree_pack_root_index, uploaded_zstd_pack_index,
    validate_remote_sync_uploaded_zstd_pack_index_metadata, zstd_pack_index_from_bytes,
    zstd_pack_index_from_path,
};
#[cfg(test)]
pub(in crate::foundation::native_repositories) use validation::validate_zstd_pack_index_metadata;
pub(super) use validation::{
    json_object, json_text_array, json_value_array, optional_i64_field, pack_ids_from_array,
    required_i64_field, validate_object_pack_entry, validate_pack_id_segment,
    validate_root_tree_locator_index, validate_tree_pack_entry,
};
