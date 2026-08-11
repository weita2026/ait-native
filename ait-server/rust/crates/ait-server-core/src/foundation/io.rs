use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PostgresSchemaReadyKey {
    pub dsn: String,
    pub schema: String,
}

#[derive(Debug, Default, Clone)]
pub struct PostgresSchemaReadyCache {
    ready: HashSet<PostgresSchemaReadyKey>,
}

pub fn postgres_schema_ready_key(
    backend: &str,
    dsn: Option<&str>,
    schema: Option<&str>,
) -> Option<PostgresSchemaReadyKey> {
    if backend.trim() != "postgres" {
        return None;
    }
    let normalized_dsn = dsn.unwrap_or_default().trim();
    let normalized_schema = schema.unwrap_or_default().trim();
    if normalized_dsn.is_empty() || normalized_schema.is_empty() {
        return None;
    }
    Some(PostgresSchemaReadyKey {
        dsn: normalized_dsn.to_string(),
        schema: normalized_schema.to_string(),
    })
}

impl PostgresSchemaReadyCache {
    pub fn reset(&mut self) {
        self.ready.clear();
    }

    pub fn mark_ready(&mut self, key: Option<PostgresSchemaReadyKey>) {
        if let Some(key) = key {
            self.ready.insert(key);
        }
    }

    pub fn is_ready(&self, key: Option<&PostgresSchemaReadyKey>) -> bool {
        match key {
            Some(key) => self.ready.contains(key),
            None => true,
        }
    }
}

pub fn require_schema_ready(ready: bool) -> Result<(), String> {
    if ready {
        return Ok(());
    }
    Err(
        "Content schema bootstrap has not run in this process. Call server_content.initialize(ctx) during ait-server startup before serving request-time content helpers.".to_string(),
    )
}

pub fn manifest_path_for_tree(tree_pack_path: Option<&str>, tree_id: &str) -> String {
    let normalized_tree_id = tree_id.trim();
    let tree_entry_name = format!("trees/{normalized_tree_id}.json");
    match tree_pack_path
        .map(str::trim)
        .filter(|pack_path| !pack_path.is_empty())
    {
        Some(pack_path) => format!("{pack_path}#{tree_entry_name}"),
        None => format!("trees/{normalized_tree_id}"),
    }
}

pub fn snapshot_manifest_map_from_rows(
    rows: &[JsonValue],
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut out = JsonMap::new();
    for row in rows {
        let row_obj = row
            .as_object()
            .ok_or_else(|| "snapshot row must be an object.".to_string())?;
        let path = required_text_field(row_obj, "path")?;
        let entry = json!({
            "blob_id": required_text_field(row_obj, "blob_id")?,
            "size_bytes": row_obj.get("size_bytes").cloned().unwrap_or(JsonValue::Null),
            "mode": required_text_field(row_obj, "mode")?,
            "sha256": required_text_field(row_obj, "sha256")?,
        });
        out.insert(path, entry);
    }
    Ok(out)
}

fn required_text_field(
    obj: &JsonMap<String, JsonValue>,
    field_name: &str,
) -> Result<String, String> {
    obj.get(field_name)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{field_name} is required"))
}
