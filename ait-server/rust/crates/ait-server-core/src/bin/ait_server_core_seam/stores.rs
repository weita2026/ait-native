use super::json_helpers::{
    bytes_map, optional_f64, optional_text, optional_usize, print_json, required_text,
    required_value,
};
use super::*;
pub(super) fn server_storage_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let payload: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let obj = payload
        .as_object()
        .ok_or_else(|| "server-storage payload must be a JSON object.".to_string())?;
    let result = match operation {
        "build-pack-members" => {
            let max_depth = optional_usize(obj, "max_delta_chain_depth")?.unwrap_or(4);
            pack_substrate::build_pack_members(
                required_value(obj, "blob_items")?,
                max_depth,
                obj.get("initial_by_path").filter(|value| !value.is_null()),
            )
        }
        "write-pack-archive" => pack_substrate::write_pack_archive(
            &required_text(obj, "pack_path")?,
            &required_text(obj, "pack_id")?,
            &required_text(obj, "created_at")?,
            required_value(obj, "members")?,
        ),
        "read-pack-index" => {
            let pack_path = required_text(obj, "pack_path")?;
            match optional_text(obj, "pack_format") {
                Some(pack_format) => {
                    pack_substrate::read_pack_index_with_format(&pack_path, &pack_format)
                }
                None => pack_substrate::read_pack_index(&pack_path),
            }
        }
        "pack-has-entry" => Ok(json!({
            "exists": pack_substrate::pack_has_entry(
                &required_text(obj, "pack_path")?,
                &required_text(obj, "entry_name")?,
            )
        })),
        "read-pack-entry" => {
            let base_map = bytes_map(obj.get("resolve_base_blob_map"))?;
            let pack_path = required_text(obj, "pack_path")?;
            let entry_name = required_text(obj, "entry_name")?;
            let max_chain_depth = optional_usize(obj, "max_chain_depth")?.unwrap_or(4);
            let bytes = match optional_text(obj, "pack_format") {
                Some(pack_format) => pack_substrate::read_pack_entry_with_format(
                    &pack_path,
                    &entry_name,
                    base_map.as_ref(),
                    max_chain_depth,
                    &pack_format,
                ),
                None => pack_substrate::read_pack_entry(
                    &pack_path,
                    &entry_name,
                    base_map.as_ref(),
                    max_chain_depth,
                ),
            }?;
            Ok(json!({"data": bytes}))
        }
        "summarize-pack-archives" => pack_substrate::summarize_pack_archives(
            &required_text(obj, "root")?,
            required_value(obj, "pack_rows")?,
        ),
        "build-storage-validation-summary" => Ok(pack_substrate::build_storage_validation_summary(
            optional_usize(obj, "packed_blob_count")?.unwrap_or(0),
            optional_usize(obj, "packed_full_blob_count")?.unwrap_or(0),
            optional_usize(obj, "packed_delta_blob_count")?.unwrap_or(0),
            optional_usize(obj, "pack_count")?.unwrap_or(0),
            optional_usize(obj, "pack_index_error_count")?.unwrap_or(0),
            optional_usize(obj, "tree_pack_index_error_count")?.unwrap_or(0),
            optional_f64(obj, "storage_savings_ratio")?.unwrap_or(0.0),
            optional_usize(obj, "unreferenced_blob_count")?.unwrap_or(0),
            optional_usize(obj, "unreferenced_tree_count")?.unwrap_or(0),
            obj.get("signals_summary"),
        )),
        "build-tree-pack-members" => pack_substrate::build_tree_pack_members(
            required_value(obj, "tree_rows")?,
            required_value(obj, "tree_entry_rows")?,
        ),
        "write-tree-pack-archive" => pack_substrate::write_tree_pack_archive(
            &required_text(obj, "pack_path")?,
            &required_text(obj, "pack_id")?,
            &required_text(obj, "created_at")?,
            required_value(obj, "members")?,
        ),
        "read-tree-pack-index" => {
            pack_substrate::read_tree_pack_index(&required_text(obj, "pack_path")?)
        }
        "read-tree-pack-index-without-ordinals" => {
            pack_substrate::read_tree_pack_index_without_ordinals(&required_text(obj, "pack_path")?)
        }
        "read-tree-pack-tree" => pack_substrate::read_tree_pack_tree(
            &required_text(obj, "pack_path")?,
            &required_text(obj, "tree_id")?,
        ),
        "read-tree-pack-tree-by-ordinal" => pack_substrate::read_tree_pack_tree_by_ordinal(
            &required_text(obj, "pack_path")?,
            optional_usize(obj, "entry_ordinal")?
                .ok_or_else(|| "entry_ordinal must be an integer".to_string())?,
        ),
        "tree-pack-contains-blob-ids" => pack_substrate::tree_pack_contains_blob_ids(
            &required_text(obj, "pack_path")?,
            required_value(obj, "blob_ids")?,
        ),
        "summarize-tree-pack-archives" => pack_substrate::summarize_tree_pack_archives(
            &required_text(obj, "root")?,
            required_value(obj, "pack_rows")?,
        ),
        "tree-pack-manifest-path" => Ok(json!({
            "manifest_path": pack_substrate::tree_pack_manifest_path(
                &required_text(obj, "pack_path")?,
                &required_text(obj, "entry_name")?,
            )
        })),
        "build-tree-records" => {
            revision_trees::build_tree_records(required_value(obj, "file_entries")?)
        }
        "build-snapshot-id" => revision_trees::build_snapshot_id(&payload),
        _ => Err(format!("Unsupported server-storage operation: {operation}")),
    }?;
    print_json(&result)
}
