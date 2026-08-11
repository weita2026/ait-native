use serde_json::{json, Map, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
enum TreeNode {
    Tree {
        children: BTreeMap<String, TreeNode>,
    },
    Blob {
        blob_id: String,
        size_bytes: Option<i64>,
        mode: String,
    },
}

#[derive(Clone, Debug)]
struct TreeEntryRow {
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
        .ok_or_else(|| "file_entries must be a JSON array".to_string())?;
    let mut sorted_rows = rows.iter().collect::<Vec<_>>();
    sorted_rows.sort_by_key(|row| {
        row.as_object()
            .and_then(|obj| text_field(obj, "path").ok())
            .unwrap_or_default()
    });

    let mut root: BTreeMap<String, TreeNode> = BTreeMap::new();
    for row in sorted_rows {
        let obj = row
            .as_object()
            .ok_or_else(|| "file entry must be a JSON object".to_string())?;
        let path = text_field(obj, "path")?;
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
                .or_insert_with(|| TreeNode::Tree {
                    children: BTreeMap::new(),
                }) {
                TreeNode::Tree { children } => children,
                TreeNode::Blob { .. } => {
                    return Err(format!(
                        "Path collision while building tree metadata at {path:?}"
                    ));
                }
            };
        }
        cursor.insert(
            parts.last().unwrap().clone(),
            TreeNode::Blob {
                blob_id: text_field(obj, "blob_id")?,
                size_bytes: optional_i64_field(obj, "size_bytes")?,
                mode: text_field(obj, "mode")?,
            },
        );
    }

    let mut tree_rows = BTreeMap::<String, JsonValue>::new();
    let mut tree_entry_rows = BTreeMap::<(String, String), TreeEntryRow>::new();
    let root_tree_id = materialize_tree(&root, &mut tree_rows, &mut tree_entry_rows)?;
    let entries = tree_entry_rows
        .into_values()
        .map(|row| {
            json!({
                "tree_id": row.tree_id,
                "entry_name": row.entry_name,
                "entry_type": row.entry_type,
                "target_id": row.target_id,
                "size_bytes": row.size_bytes,
                "mode": row.mode,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "root_tree_id": root_tree_id,
        "tree_rows": tree_rows.into_values().collect::<Vec<_>>(),
        "tree_entry_rows": entries,
    }))
}

pub fn build_snapshot_id(payload: &JsonValue) -> Result<JsonValue, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "snapshot id payload must be a JSON object".to_string())?;
    let snapshot_kind =
        optional_text_field(obj, "snapshot_kind")?.unwrap_or_else(|| "line".to_string());
    let canonical = json!({
        "repo_name": text_field(obj, "repo_name")?,
        "line_name": text_field(obj, "line_name")?,
        "parent_snapshot_id": optional_text_field(obj, "parent_snapshot_id")?,
        "message": optional_text_field(obj, "message")?,
        "root_tree_id": text_field(obj, "root_tree_id")?,
        "snapshot_kind": snapshot_kind,
    });
    let revision_hash = sha256_hex(
        serde_json::to_string(&canonical)
            .map_err(|err| err.to_string())?
            .as_bytes(),
    );
    Ok(json!({
        "snapshot_id": format!("SNP-{}", revision_hash[..12].to_ascii_uppercase()),
        "revision_hash": revision_hash,
    }))
}

fn materialize_tree(
    children: &BTreeMap<String, TreeNode>,
    tree_rows: &mut BTreeMap<String, JsonValue>,
    tree_entry_rows: &mut BTreeMap<(String, String), TreeEntryRow>,
) -> Result<String, String> {
    let mut serialized_entries = Vec::<JsonValue>::new();
    let mut pending_rows = Vec::<TreeEntryRow>::new();
    for (name, node) in children {
        match node {
            TreeNode::Tree { children } => {
                let child_tree_id = materialize_tree(children, tree_rows, tree_entry_rows)?;
                serialized_entries.push(json!({
                    "name": name,
                    "type": "tree",
                    "target_id": child_tree_id,
                }));
                pending_rows.push(TreeEntryRow {
                    tree_id: String::new(),
                    entry_name: name.clone(),
                    entry_type: "tree".to_string(),
                    target_id: child_tree_id,
                    size_bytes: None,
                    mode: "tree".to_string(),
                });
            }
            TreeNode::Blob {
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
                pending_rows.push(TreeEntryRow {
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
    let digest = sha256_hex(
        serde_json::to_string(&serialized_entries)
            .map_err(|err| err.to_string())?
            .as_bytes(),
    );
    let tree_id = format!("TRE-{}", digest[..20].to_ascii_uppercase());
    tree_rows.entry(tree_id.clone()).or_insert_with(|| {
        json!({
            "tree_id": tree_id,
            "entry_count": serialized_entries.len(),
        })
    });
    let tree_id = format!("TRE-{}", digest[..20].to_ascii_uppercase());
    for mut row in pending_rows {
        row.tree_id = tree_id.clone();
        tree_entry_rows.insert((row.tree_id.clone(), row.entry_name.clone()), row);
    }
    Ok(tree_id)
}

fn text_field(obj: &Map<String, JsonValue>, field: &str) -> Result<String, String> {
    optional_text_field(obj, field)?.ok_or_else(|| format!("{field} must be a non-empty string"))
}

fn optional_text_field(
    obj: &Map<String, JsonValue>,
    field: &str,
) -> Result<Option<String>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => {
            let text = value.trim();
            Ok((!text.is_empty()).then(|| text.to_string()))
        }
        Some(value) => Ok(Some(value.to_string())),
    }
}

fn optional_i64_field(obj: &Map<String, JsonValue>, field: &str) -> Result<Option<i64>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => value
            .as_i64()
            .ok_or_else(|| format!("{field} must fit in i64"))
            .map(Some),
        Some(value) => value
            .to_string()
            .parse::<i64>()
            .map(Some)
            .map_err(|_| format!("{field} must be an integer")),
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
