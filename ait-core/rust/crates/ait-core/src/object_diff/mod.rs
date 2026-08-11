use crate::json_support::JsonValue;
use crate::snapshot_json::SnapshotJson;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

mod diff_impl;

#[cfg(test)]
use self::diff_impl::{
    coerce_snapshot_manifest, maybe_add_text_diff_from_blob_bytes, safe_decode_text, to_mode_int,
};
pub use self::diff_impl::{
    diff_snapshot_manifests, snapshot_diff_from_manifests, snapshot_diff_from_object_reader,
    snapshot_diff_from_readers, workspace_diff_from_entries,
};
pub(crate) use self::diff_impl::{
    diff_snapshot_manifests_impl, snapshot_diff_from_manifests_impl,
    snapshot_diff_from_object_reader_impl, snapshot_diff_from_readers_impl,
    snapshot_manifest_from_object_reader_impl,
};

pub const DEFAULT_SNAPSHOT_DIFF_MAX_BYTES: usize = 128_000;

pub use crate::object_diff_ports::{BlobReader, ObjectReader, SnapshotReader};

pub struct ObjectBackedSnapshotReader<R> {
    object_reader: R,
}

impl<R> ObjectBackedSnapshotReader<R> {
    pub fn new(object_reader: R) -> Self {
        Self { object_reader }
    }
}

impl<R: ObjectReader> SnapshotReader for ObjectBackedSnapshotReader<R> {
    fn read_snapshot_manifest(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        snapshot_manifest_from_object_reader_impl(&self.object_reader, snapshot_id)
    }

    fn read_snapshot_payload(&self, snapshot_id: &str) -> Result<Option<JsonValue>, String> {
        self.object_reader.read_object_json(snapshot_id)
    }

    fn read_snapshot_root_tree_payload(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<JsonValue>, String> {
        let Some(payload) = self.read_snapshot_payload(snapshot_id)? else {
            return Ok(None);
        };
        let Some(payload_obj) = payload.as_object() else {
            return Ok(None);
        };
        Ok(payload_obj
            .get("root_tree")
            .or_else(|| payload_obj.get("root_tree_payload"))
            .filter(|value| value.is_object())
            .cloned())
    }

    fn read_tree_payload(&self, tree_id: &str) -> Result<Option<JsonValue>, String> {
        self.object_reader.read_object_json(tree_id)
    }
}

#[derive(Clone, Debug)]
struct SnapshotFileRow {
    path: String,
    blob_id: Option<String>,
    size_bytes: Option<i64>,
    mode_raw: JsonValue,
    mode_int: i64,
}

#[derive(Clone, Debug)]
struct SnapshotTreeEntry {
    entry_type: String,
    target_id: String,
    size_bytes: Option<i64>,
    mode_raw: JsonValue,
    mode_int: i64,
}

#[derive(Default)]
struct SnapshotTreeDiffState {
    old_rows: BTreeMap<String, SnapshotFileRow>,
    new_rows: BTreeMap<String, SnapshotFileRow>,
    added: Vec<String>,
    deleted: Vec<String>,
    modified: Vec<String>,
    mode_changed: Vec<String>,
    file_entries: Vec<JsonValue>,
}

#[derive(Clone, Debug)]
struct RenameHint {
    blob_id: String,
    old_path: String,
    new_path: String,
    old_parent_path: String,
    new_parent_path: String,
    size_bytes: i64,
}

#[derive(Clone, Debug)]
struct TextDiffPayload {
    status: &'static str,
    insertions: usize,
    deletions: usize,
    text: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceDiffEntry {
    pub path: String,
    pub status: String,
    pub old_bytes: Option<Vec<u8>>,
    pub new_bytes: Option<Vec<u8>>,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
}

pub fn artifact_blob_id(markdown: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(markdown.as_bytes());
    let digest = hasher.finalize();
    let mut prefix = String::with_capacity(20);
    for byte in digest.iter().take(10) {
        prefix.push_str(&format!("{byte:02x}"));
    }
    format!("BLB-{prefix}")
}

pub fn snapshot_manifest_from_object_reader<R: ObjectReader + ?Sized>(
    object_reader: &R,
    snapshot_id: &str,
) -> Result<JsonValue, String> {
    SnapshotJson::stateless().snapshot_manifest_from_object_reader(object_reader, snapshot_id)
}

#[cfg(test)]
mod tests;
