//! Retained object/tree pack and storage identity SDK operations.
use crate::{workspace::native_count, JsonInput};
use ait_core::{json_support::JsonValue, pack_substrate as pack, plan_storage_sdk as identity};
use napi::{bindgen_prelude::Buffer, Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct BuildPackMembersOptions {
    pub max_delta_chain_depth: Option<JsonInput>,
    pub initial_by_path: Option<JsonInput>,
}

#[napi]
pub fn plan_pack_substrate_build_pack_members(
    blob_items: JsonInput,
    options: Option<BuildPackMembersOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    pack::build_pack_members(
        &blob_items,
        native_count(
            options.max_delta_chain_depth,
            pack::DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            "max_delta_chain_depth",
        )?,
        options.initial_by_path.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_build_tree_pack_members(
    tree_rows: JsonInput,
    tree_entry_rows: JsonInput,
) -> Result<JsonValue> {
    pack::build_tree_pack_members(&tree_rows, &tree_entry_rows).map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_build_git_binary_delta(
    base_data: Buffer,
    target_data: Buffer,
) -> Buffer {
    pack::build_git_binary_delta(&base_data, &target_data).into()
}

#[napi]
pub fn plan_pack_substrate_build_git_binary_delta_member(
    entry_name: String,
    blob_id: String,
    base_blob_id: String,
    base_data: Buffer,
    target_data: Buffer,
    chain_depth: JsonInput,
) -> Result<JsonValue> {
    Ok(pack::build_git_binary_delta_member(
        &entry_name,
        &blob_id,
        &base_blob_id,
        &base_data,
        &target_data,
        native_count(Some(chain_depth), 0, "chain_depth")?,
    ))
}

#[napi]
pub fn plan_pack_substrate_apply_git_binary_delta(
    base_data: Buffer,
    delta_data: Buffer,
) -> Result<Buffer> {
    pack::apply_git_binary_delta(&base_data, &delta_data)
        .map(Buffer::from)
        .map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_apply_pack_delta(
    base_data: Buffer,
    delta_data: Buffer,
    algorithm: String,
) -> Result<Buffer> {
    pack::apply_pack_delta(&base_data, &delta_data, &algorithm)
        .map(Buffer::from)
        .map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_write_pack_archive(
    pack_path: String,
    pack_id: String,
    created_at: String,
    members: JsonInput,
) -> Result<JsonValue> {
    pack::write_pack_archive(&pack_path, &pack_id, &created_at, &members)
        .map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_read_pack_index(pack_path: String) -> Result<JsonValue> {
    pack::read_pack_index(&pack_path).map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_pack_has_entry(pack_path: String, entry_name: String) -> bool {
    pack::pack_has_entry(&pack_path, &entry_name)
}

#[napi]
pub fn plan_pack_substrate_write_tree_pack_archive(
    pack_path: String,
    pack_id: String,
    created_at: String,
    members: JsonInput,
) -> Result<JsonValue> {
    pack::write_tree_pack_archive(&pack_path, &pack_id, &created_at, &members)
        .map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_read_tree_pack_index(pack_path: String) -> Result<JsonValue> {
    pack::read_tree_pack_index(&pack_path).map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_read_tree_pack_tree(
    pack_path: String,
    tree_id: String,
) -> Result<JsonValue> {
    pack::read_tree_pack_tree(&pack_path, &tree_id).map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_summarize_pack_archives(
    root: String,
    pack_rows: JsonInput,
) -> Result<JsonValue> {
    pack::summarize_pack_archives(&root, &pack_rows).map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_summarize_tree_pack_archives(
    root: String,
    pack_rows: JsonInput,
) -> Result<JsonValue> {
    pack::summarize_tree_pack_archives(&root, &pack_rows).map_err(Error::from_reason)
}

#[napi]
pub fn plan_pack_substrate_tree_pack_manifest_path(
    pack_path: String,
    entry_name: String,
) -> String {
    pack::tree_pack_manifest_path(&pack_path, &entry_name)
}

#[napi]
pub fn plan_pack_substrate_read_tree_pack_tree_by_ordinal(
    pack_path: String,
    entry_ordinal: JsonInput,
) -> Result<JsonValue> {
    pack::read_tree_pack_tree_by_ordinal(
        &pack_path,
        native_count(Some(entry_ordinal), 0, "entry_ordinal")?,
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
pub struct StorageValidationOptions {
    pub packed_blob_count: JsonInput,
    pub packed_full_blob_count: JsonInput,
    pub packed_delta_blob_count: JsonInput,
    pub pack_count: JsonInput,
    pub pack_index_error_count: JsonInput,
    pub tree_pack_index_error_count: Option<JsonInput>,
    pub storage_savings_ratio: f64,
    pub unreferenced_blob_count: Option<JsonInput>,
    pub unreferenced_tree_count: Option<JsonInput>,
    pub signals_summary: Option<JsonInput>,
}

#[napi]
pub fn plan_pack_substrate_build_storage_validation_summary(
    options: StorageValidationOptions,
) -> Result<JsonValue> {
    Ok(pack::build_storage_validation_summary(
        native_count(Some(options.packed_blob_count), 0, "packed_blob_count")?,
        native_count(
            Some(options.packed_full_blob_count),
            0,
            "packed_full_blob_count",
        )?,
        native_count(
            Some(options.packed_delta_blob_count),
            0,
            "packed_delta_blob_count",
        )?,
        native_count(Some(options.pack_count), 0, "pack_count")?,
        native_count(
            Some(options.pack_index_error_count),
            0,
            "pack_index_error_count",
        )?,
        native_count(
            options.tree_pack_index_error_count,
            0,
            "tree_pack_index_error_count",
        )?,
        options.storage_savings_ratio,
        native_count(
            options.unreferenced_blob_count,
            0,
            "unreferenced_blob_count",
        )?,
        native_count(
            options.unreferenced_tree_count,
            0,
            "unreferenced_tree_count",
        )?,
        options.signals_summary.as_deref(),
    ))
}

#[napi]
pub fn plan_pack_substrate_build_tree_records(file_entries: JsonInput) -> Result<JsonValue> {
    identity::build_tree_records(&file_entries).map_err(Error::from_reason)
}

#[napi(object)]
pub struct BuildSnapshotIdOptions {
    pub repo_name: String,
    pub line_name: String,
    pub parent_snapshot_id: Option<String>,
    pub message: Option<String>,
    pub root_tree_id: String,
    pub snapshot_kind: Option<String>,
}

#[napi]
pub fn plan_pack_substrate_build_snapshot_id(options: BuildSnapshotIdOptions) -> JsonValue {
    identity::build_snapshot_id(
        &options.repo_name,
        &options.line_name,
        options.parent_snapshot_id.as_deref(),
        options.message.as_deref(),
        &options.root_tree_id,
        options.snapshot_kind.as_deref().unwrap_or("line"),
    )
}
