use super::*;
use crate::json_support::{JsonCodec, JsonEncodeOptions};

pub struct SnapshotSyncWindow {
    pub snapshot_ids: Vec<String>,
    pub sync_scope: &'static str,
    pub sync_reason: &'static str,
    pub remote_head_snapshot_id: Option<String>,
    pub bounded_by_snapshot_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotPathDelta {
    pub affected_paths: Vec<String>,
    pub status_by_path: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotPathBlobRow {
    pub snapshot_index: usize,
    pub blob_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFileRow {
    pub path: String,
    pub blob_id: String,
    pub size_bytes: i64,
    pub mode: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotTreeManifestRow {
    pub path_id: u32,
    pub blob_id: u32,
    pub size_bytes: i64,
    pub mode: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotTreeManifestIndex {
    pub paths: Vec<String>,
    pub path_id_by_path: BTreeMap<String, u32>,
    pub blobs: Vec<String>,
    pub blob_id_by_blob: BTreeMap<String, u32>,
    pub rows: Vec<SnapshotTreeManifestRow>,
}

impl SnapshotTreeManifestIndex {
    pub fn from_file_rows(rows: Vec<SnapshotFileRow>) -> Result<Self, String> {
        let mut paths = Vec::new();
        let mut path_id_by_path = BTreeMap::new();
        let mut blobs = Vec::new();
        let mut blob_id_by_blob = BTreeMap::new();
        let mut indexed_rows = Vec::with_capacity(rows.len());
        for row in rows {
            let path_id = intern_manifest_id(&mut paths, &mut path_id_by_path, row.path)?;
            let blob_id = intern_manifest_id(&mut blobs, &mut blob_id_by_blob, row.blob_id)?;
            indexed_rows.push(SnapshotTreeManifestRow {
                path_id,
                blob_id,
                size_bytes: row.size_bytes,
                mode: row.mode,
                sha256: row.sha256,
            });
        }
        Ok(Self {
            paths,
            path_id_by_path,
            blobs,
            blob_id_by_blob,
            rows: indexed_rows,
        })
    }

    pub fn row_path<'a>(&'a self, row: &SnapshotTreeManifestRow) -> Result<&'a str, String> {
        self.paths
            .get(row.path_id as usize)
            .map(String::as_str)
            .ok_or_else(|| format!("Snapshot manifest path id {} is out of range.", row.path_id))
    }

    pub fn row_blob_id<'a>(&'a self, row: &SnapshotTreeManifestRow) -> Result<&'a str, String> {
        self.blobs
            .get(row.blob_id as usize)
            .map(String::as_str)
            .ok_or_else(|| format!("Snapshot manifest blob id {} is out of range.", row.blob_id))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotTreeRootLocator {
    pub root_tree_id: String,
    pub root_tree_pack_id: String,
    pub root_entry_ordinal: i64,
}

mod manifest_root;

pub(crate) use self::manifest_root::*;

fn intern_manifest_id(
    values: &mut Vec<String>,
    ids: &mut BTreeMap<String, u32>,
    value: String,
) -> Result<u32, String> {
    if let Some(existing) = ids.get(&value) {
        return Ok(*existing);
    }
    let id = u32::try_from(values.len())
        .map_err(|_| "Snapshot manifest index exceeds u32 id capacity.".to_string())?;
    values.push(value.clone());
    ids.insert(value, id);
    Ok(id)
}
