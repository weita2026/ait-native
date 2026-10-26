//! Shared retained storage SDK identity helpers, extracted unchanged from PyO3.
//! These functions calculate metadata payloads; they do not persist records.
use crate::json_support::{json, JsonCodec, JsonEncodeOptions, JsonMap as Map, JsonValue};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn encode_json_value_compact(value: &JsonValue) -> Result<String, String> {
    JsonCodec::encode_value(value, JsonEncodeOptions::compact()).map_err(|error| {
        error
            .message()
            .strip_prefix("Failed to encode JSON: ")
            .unwrap_or(error.message())
            .to_string()
    })
}

fn encode_json_value_compact_or_default(value: &JsonValue) -> String {
    encode_json_value_compact(value).unwrap_or_default()
}

#[derive(Clone, Debug)]
enum StorageTreeNode {
    Tree {
        children: BTreeMap<String, StorageTreeNode>,
    },
    Blob {
        blob_id: String,
        size_bytes: Option<i64>,
        mode: String,
    },
}

#[derive(Clone, Debug)]
struct StorageTreeEntryRow {
    tree_id: String,
    entry_name: String,
    entry_type: String,
    target_id: String,
    size_bytes: Option<i64>,
    mode: String,
}

pub fn build_tree_records(file_entries: &JsonValue) -> Result<JsonValue, String> {
    let rows = file_entries
        .as_array()
        .ok_or_else(|| "file_entries must be a list.".to_string())?;
    let mut sorted_rows = rows.iter().collect::<Vec<_>>();
    sorted_rows.sort_by_key(|row| {
        row.as_object()
            .and_then(|obj| optional_json_text_field(obj, "path"))
            .unwrap_or_default()
    });
    let mut root: BTreeMap<String, StorageTreeNode> = BTreeMap::new();
    for row in sorted_rows {
        let obj = row
            .as_object()
            .ok_or_else(|| "file entry must be an object.".to_string())?;
        let path = required_json_text_field(obj, "path")?;
        let parts = path
            .trim_matches('/')
            .split('/')
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        let mut cursor = &mut root;
        for part in &parts[..parts.len() - 1] {
            cursor = match cursor
                .entry(part.clone())
                .or_insert_with(|| StorageTreeNode::Tree {
                    children: BTreeMap::new(),
                }) {
                StorageTreeNode::Tree { children } => children,
                StorageTreeNode::Blob { .. } => {
                    return Err(format!(
                        "Path collision while building tree metadata at {path:?}"
                    ));
                }
            };
        }
        cursor.insert(
            parts.last().unwrap().clone(),
            StorageTreeNode::Blob {
                blob_id: required_json_text_field(obj, "blob_id")?,
                size_bytes: optional_json_i64_field(obj, "size_bytes")?,
                mode: required_json_text_field(obj, "mode")?,
            },
        );
    }

    let mut tree_rows = BTreeMap::<String, JsonValue>::new();
    let mut tree_entry_rows = BTreeMap::<(String, String), StorageTreeEntryRow>::new();
    let root_tree_id = materialize_storage_tree(&root, &mut tree_rows, &mut tree_entry_rows)?;
    Ok(json!({
        "root_tree_id": root_tree_id,
        "tree_rows": tree_rows.into_values().collect::<Vec<_>>(),
        "tree_entry_rows": tree_entry_rows.into_values().map(|row| json!({
            "tree_id": row.tree_id,
            "entry_name": row.entry_name,
            "entry_type": row.entry_type,
            "target_id": row.target_id,
            "size_bytes": row.size_bytes,
            "mode": row.mode,
        })).collect::<Vec<_>>(),
    }))
}

fn materialize_storage_tree(
    children: &BTreeMap<String, StorageTreeNode>,
    tree_rows: &mut BTreeMap<String, JsonValue>,
    tree_entry_rows: &mut BTreeMap<(String, String), StorageTreeEntryRow>,
) -> Result<String, String> {
    let mut serialized_entries = Vec::<JsonValue>::new();
    let mut pending_rows = Vec::<StorageTreeEntryRow>::new();
    for (name, node) in children {
        match node {
            StorageTreeNode::Tree { children } => {
                let child_tree_id = materialize_storage_tree(children, tree_rows, tree_entry_rows)?;
                serialized_entries.push(json!({
                    "name": name,
                    "type": "tree",
                    "target_id": child_tree_id,
                }));
                pending_rows.push(StorageTreeEntryRow {
                    tree_id: String::new(),
                    entry_name: name.clone(),
                    entry_type: "tree".to_string(),
                    target_id: child_tree_id,
                    size_bytes: None,
                    mode: "tree".to_string(),
                });
            }
            StorageTreeNode::Blob {
                blob_id,
                size_bytes,
                mode,
            } => {
                serialized_entries.push(json!({
                    "name": name,
                    "type": "blob",
                    "target_id": blob_id,
                    "size_bytes": size_bytes,
                    "mode": mode,
                }));
                pending_rows.push(StorageTreeEntryRow {
                    tree_id: String::new(),
                    entry_name: name.clone(),
                    entry_type: "blob".to_string(),
                    target_id: blob_id.clone(),
                    size_bytes: *size_bytes,
                    mode: mode.clone(),
                });
            }
        }
    }
    let digest_payload = JsonValue::Array(serialized_entries.clone());
    let digest_text = encode_json_value_compact(&digest_payload)?;
    let digest = sha256_hex(digest_text.as_bytes());
    let tree_id = format!("TRE-{}", digest[..20].to_ascii_uppercase());
    let row_tree_id = tree_id.clone();
    tree_rows.entry(tree_id.clone()).or_insert_with(|| {
        json!({
            "tree_id": row_tree_id,
            "entry_count": serialized_entries.len(),
        })
    });
    for mut row in pending_rows {
        row.tree_id = tree_id.clone();
        tree_entry_rows.insert((row.tree_id.clone(), row.entry_name.clone()), row);
    }
    Ok(tree_id)
}

pub fn build_snapshot_id(
    repo_name: &str,
    line_name: &str,
    parent_snapshot_id: Option<&str>,
    message: Option<&str>,
    root_tree_id: &str,
    snapshot_kind: &str,
) -> JsonValue {
    let payload = json!({
        "repo_name": repo_name,
        "line_name": line_name,
        "parent_snapshot_id": parent_snapshot_id,
        "message": message,
        "root_tree_id": root_tree_id,
        "snapshot_kind": snapshot_kind,
    });
    let revision_text = encode_json_value_compact_or_default(&payload);
    let revision_hash = sha256_hex(revision_text.as_bytes());
    json!({
        "snapshot_id": format!("SNP-{}", revision_hash[..12].to_ascii_uppercase()),
        "revision_hash": revision_hash,
    })
}

fn required_json_text_field(obj: &Map<String, JsonValue>, field: &str) -> Result<String, String> {
    optional_json_text_field(obj, field)
        .ok_or_else(|| format!("{field} must be a non-empty string."))
}

fn optional_json_text_field(obj: &Map<String, JsonValue>, field: &str) -> Option<String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::String(value)) => {
            let text = value.trim();
            (!text.is_empty()).then(|| text.to_string())
        }
        Some(value) => Some(value.to_string()),
    }
}

fn optional_json_i64_field(
    obj: &Map<String, JsonValue>,
    field: &str,
) -> Result<Option<i64>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_i64()
            .ok_or_else(|| format!("{field} must fit in i64."))
            .map(Some),
        Some(value) => value
            .to_string()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| format!("{field} must be an integer.")),
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_python_storage_identity_golden() {
        let entries = json!([
            {"path":"a.txt","blob_id":"BLB-A","size_bytes":1,"mode":"0o644"},
            {"path":"nested/b.bin","blob_id":"BLB-B","size_bytes":4,"mode":"0o755"}
        ]);
        let trees = build_tree_records(&entries).unwrap();
        assert_eq!(trees["root_tree_id"], "TRE-59B20845E41B99DD6AF0");
        assert_eq!(trees["tree_entry_rows"].as_array().unwrap().len(), 3);
        assert_eq!(
            build_snapshot_id(
                "fixture",
                "main",
                None,
                Some("SDK"),
                trees["root_tree_id"].as_str().unwrap(),
                "line"
            ),
            json!({
                "snapshot_id":"SNP-2E95AF56F25D",
                "revision_hash":"2e95af56f25d3a1dbc7c62ace62f6bf7fe5bf3f590d6097eae0904a1d85a62e3"
            })
        );
    }

    #[test]
    fn retained_storage_tree_validation_errors() {
        assert_eq!(
            build_tree_records(&json!({})).unwrap_err(),
            "file_entries must be a list."
        );
        assert_eq!(
            build_tree_records(
                &json!([{"path":"a","blob_id":"b","mode":"m","size_bytes":u64::MAX}])
            )
            .unwrap_err(),
            "size_bytes must fit in i64."
        );
        assert!(build_tree_records(&json!([
            {"path":"a","blob_id":"b","mode":"m"},
            {"path":"a/b","blob_id":"b","mode":"m"}
        ]))
        .unwrap_err()
        .contains("Path collision"));
    }
}
