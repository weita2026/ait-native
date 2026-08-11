use crate::json_support::{json, JsonMap, JsonValue};
use crate::snapshot_store::{SnapshotRecord, SnapshotStore};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SnapshotAuthoringOptions {
    pub allow_unchanged_tree: bool,
}

mod export;
mod snapshot;
mod tree_rows;
mod types;
mod util;

pub use export::*;
pub(crate) use snapshot::*;
pub use tree_rows::*;
use types::*;
pub(crate) use types::{SnapshotFileEntry, TreeEntryRow, TreeRow};
use util::*;

pub trait LocalSnapshotWriteStore {
    fn create_snapshot(
        &self,
        repo_name: &str,
        line_name: &str,
        message: Option<&str>,
        is_worktree: bool,
    ) -> Result<JsonValue, String>;

    fn create_snapshot_with_parents(
        &self,
        repo_name: &str,
        line_name: &str,
        parent_snapshot_ids: &[String],
        message: Option<&str>,
        is_worktree: bool,
    ) -> Result<JsonValue, String> {
        if parent_snapshot_ids.len() > 1 {
            return Err(
                "Ordered multi-parent Snapshot authoring is unavailable for this store."
                    .to_string(),
            );
        }
        self.create_snapshot(repo_name, line_name, message, is_worktree)
    }
}

pub trait LocalSnapshotReadStore {
    fn get_snapshot(&self, snapshot_id: &str) -> Result<JsonValue, String>;
    fn list_snapshots(&self) -> Result<JsonValue, String>;
    fn get_line(&self, line_name: &str) -> Result<JsonValue, String>;
}

pub trait LocalSnapshotBlobReadStore {
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Vec<u8>, String>;

    fn read_blob_bytes_batch(
        &self,
        blob_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let mut bytes_by_blob_id = BTreeMap::new();
        for blob_id in blob_ids {
            if bytes_by_blob_id.contains_key(blob_id) {
                continue;
            }
            bytes_by_blob_id.insert(blob_id.clone(), self.read_blob_bytes(blob_id)?);
        }
        Ok(bytes_by_blob_id)
    }
}

pub trait LocalSnapshotTreeReadStore {
    fn snapshot_tree_root_locator(
        &self,
        snapshot_id: &str,
    ) -> Result<SnapshotTreeRootLocator, String>;
    fn snapshot_tree_manifest_path(&self, snapshot_id: &str) -> Result<String, String>;
    fn snapshot_tree_path_delta(
        &self,
        old_snapshot_id: Option<&str>,
        new_snapshot_id: Option<&str>,
    ) -> Result<SnapshotPathDelta, String>;
    fn snapshot_tree_file_rows(
        &self,
        snapshot_id: Option<&str>,
    ) -> Result<Vec<SnapshotFileRow>, String>;
    fn snapshot_tree_path_file_rows(
        &self,
        snapshot_id: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, SnapshotFileRow>, String>;
    fn snapshot_tree_path_rows(
        &self,
        snapshot_id: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, JsonValue>, String>;
    fn snapshot_tree_path_rows_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &str,
    ) -> Result<BTreeMap<String, JsonValue>, String>;
    fn snapshot_tree_path_blob_rows_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &str,
    ) -> Result<Vec<SnapshotPathBlobRow>, String>;
    fn visit_snapshot_tree_path_blobs_reverse(
        &self,
        snapshot_ids: &[String],
        path: &str,
        visitor: &mut dyn FnMut(usize, Option<String>) -> Result<bool, String>,
    ) -> Result<(), String> {
        let rows = self.snapshot_tree_path_blob_rows_for_snapshots(snapshot_ids, path)?;
        let mut blob_by_snapshot_index = rows
            .into_iter()
            .map(|row| (row.snapshot_index, row.blob_id))
            .collect::<BTreeMap<_, _>>();
        for snapshot_index in (0..snapshot_ids.len()).rev() {
            if !visitor(
                snapshot_index,
                blob_by_snapshot_index.remove(&snapshot_index),
            )? {
                break;
            }
        }
        Ok(())
    }
    fn snapshot_tree_path_row(
        &self,
        snapshot_id: &str,
        path: &str,
    ) -> Result<Option<JsonValue>, String>;
}

pub trait LocalSnapshotOperationStore:
    LocalSnapshotWriteStore
    + LocalSnapshotReadStore
    + LocalSnapshotBlobReadStore
    + LocalSnapshotTreeReadStore
{
}

impl<T> LocalSnapshotOperationStore for T where
    T: LocalSnapshotWriteStore
        + LocalSnapshotReadStore
        + LocalSnapshotBlobReadStore
        + LocalSnapshotTreeReadStore
        + ?Sized
{
}

pub fn create_snapshot_with_local_snapshot_operation_store<S>(
    store: &S,
    repo_name: &str,
    line_name: &str,
    message: Option<&str>,
    is_worktree: bool,
) -> Result<JsonValue, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    create_snapshot_with_local_snapshot_write_store(
        store,
        repo_name,
        line_name,
        message,
        is_worktree,
    )
}

pub fn create_snapshot_with_local_snapshot_write_store<S>(
    store: &S,
    repo_name: &str,
    line_name: &str,
    message: Option<&str>,
    is_worktree: bool,
) -> Result<JsonValue, String>
where
    S: LocalSnapshotWriteStore + ?Sized,
{
    store.create_snapshot(repo_name, line_name, message, is_worktree)
}

pub fn create_snapshot_with_parents_with_local_snapshot_write_store<S>(
    store: &S,
    repo_name: &str,
    line_name: &str,
    parent_snapshot_ids: &[String],
    message: Option<&str>,
    is_worktree: bool,
) -> Result<JsonValue, String>
where
    S: LocalSnapshotWriteStore + ?Sized,
{
    store.create_snapshot_with_parents(
        repo_name,
        line_name,
        parent_snapshot_ids,
        message,
        is_worktree,
    )
}

pub fn read_blob_bytes_with_local_snapshot_operation_store<S>(
    store: &S,
    blob_id: &str,
) -> Result<Vec<u8>, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    read_blob_bytes_with_local_snapshot_blob_read_store(store, blob_id)
}

pub fn read_blob_bytes_with_local_snapshot_blob_read_store<S>(
    store: &S,
    blob_id: &str,
) -> Result<Vec<u8>, String>
where
    S: LocalSnapshotBlobReadStore + ?Sized,
{
    store.read_blob_bytes(blob_id)
}

pub fn get_snapshot_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<JsonValue, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    get_snapshot_with_local_snapshot_read_store(store, snapshot_id)
}

pub fn get_snapshot_with_local_snapshot_read_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<JsonValue, String>
where
    S: LocalSnapshotReadStore + ?Sized,
{
    store.get_snapshot(snapshot_id)
}

pub fn list_snapshots_with_local_snapshot_operation_store<S>(store: &S) -> Result<JsonValue, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    list_snapshots_with_local_snapshot_read_store(store)
}

pub fn list_snapshots_with_local_snapshot_read_store<S>(store: &S) -> Result<JsonValue, String>
where
    S: LocalSnapshotReadStore + ?Sized,
{
    store.list_snapshots()
}

pub fn get_line_with_local_snapshot_operation_store<S>(
    store: &S,
    line_name: &str,
) -> Result<JsonValue, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    get_line_with_local_snapshot_read_store(store, line_name)
}

pub fn get_line_with_local_snapshot_read_store<S>(
    store: &S,
    line_name: &str,
) -> Result<JsonValue, String>
where
    S: LocalSnapshotReadStore + ?Sized,
{
    store.get_line(line_name)
}

pub fn snapshot_tree_path_delta_with_local_snapshot_operation_store<S>(
    store: &S,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
) -> Result<SnapshotPathDelta, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_path_delta_with_local_snapshot_tree_read_store(
        store,
        old_snapshot_id,
        new_snapshot_id,
    )
}

pub fn snapshot_tree_path_delta_with_local_snapshot_tree_read_store<S>(
    store: &S,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
) -> Result<SnapshotPathDelta, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_path_delta(old_snapshot_id, new_snapshot_id)
}

pub fn snapshot_tree_root_locator_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<SnapshotTreeRootLocator, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_root_locator_with_local_snapshot_tree_read_store(store, snapshot_id)
}

pub fn snapshot_tree_root_locator_with_local_snapshot_tree_read_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<SnapshotTreeRootLocator, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_root_locator(snapshot_id)
}

pub fn snapshot_tree_manifest_path_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<String, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_manifest_path_with_local_snapshot_tree_read_store(store, snapshot_id)
}

pub fn snapshot_tree_manifest_path_with_local_snapshot_tree_read_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<String, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_manifest_path(snapshot_id)
}

pub fn snapshot_tree_file_rows_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_id: Option<&str>,
) -> Result<Vec<SnapshotFileRow>, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_file_rows_with_local_snapshot_tree_read_store(store, snapshot_id)
}

pub fn snapshot_tree_file_rows_with_local_snapshot_tree_read_store<S>(
    store: &S,
    snapshot_id: Option<&str>,
) -> Result<Vec<SnapshotFileRow>, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_file_rows(snapshot_id)
}

pub fn snapshot_tree_path_file_rows_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_id: &str,
    paths: &[String],
) -> Result<BTreeMap<String, SnapshotFileRow>, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_path_file_rows_with_local_snapshot_tree_read_store(store, snapshot_id, paths)
}

pub fn snapshot_tree_path_file_rows_with_local_snapshot_tree_read_store<S>(
    store: &S,
    snapshot_id: &str,
    paths: &[String],
) -> Result<BTreeMap<String, SnapshotFileRow>, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_path_file_rows(snapshot_id, paths)
}

pub fn snapshot_tree_path_rows_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_id: &str,
    paths: &[String],
) -> Result<BTreeMap<String, JsonValue>, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_path_rows_with_local_snapshot_tree_read_store(store, snapshot_id, paths)
}

pub fn snapshot_tree_path_rows_with_local_snapshot_tree_read_store<S>(
    store: &S,
    snapshot_id: &str,
    paths: &[String],
) -> Result<BTreeMap<String, JsonValue>, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_path_rows(snapshot_id, paths)
}

pub fn snapshot_tree_path_rows_for_snapshots_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_ids: &[String],
    path: &str,
) -> Result<BTreeMap<String, JsonValue>, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_path_rows_for_snapshots_with_local_snapshot_tree_read_store(
        store,
        snapshot_ids,
        path,
    )
}

pub fn snapshot_tree_path_rows_for_snapshots_with_local_snapshot_tree_read_store<S>(
    store: &S,
    snapshot_ids: &[String],
    path: &str,
) -> Result<BTreeMap<String, JsonValue>, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_path_rows_for_snapshots(snapshot_ids, path)
}

pub fn snapshot_tree_path_blob_rows_for_snapshots_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_ids: &[String],
    path: &str,
) -> Result<Vec<SnapshotPathBlobRow>, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_path_blob_rows_for_snapshots_with_local_snapshot_tree_read_store(
        store,
        snapshot_ids,
        path,
    )
}

pub fn snapshot_tree_path_blob_rows_for_snapshots_with_local_snapshot_tree_read_store<S>(
    store: &S,
    snapshot_ids: &[String],
    path: &str,
) -> Result<Vec<SnapshotPathBlobRow>, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_path_blob_rows_for_snapshots(snapshot_ids, path)
}

pub fn snapshot_tree_path_row_with_local_snapshot_operation_store<S>(
    store: &S,
    snapshot_id: &str,
    path: &str,
) -> Result<Option<JsonValue>, String>
where
    S: LocalSnapshotOperationStore + ?Sized,
{
    snapshot_tree_path_row_with_local_snapshot_tree_read_store(store, snapshot_id, path)
}

pub fn snapshot_tree_path_row_with_local_snapshot_tree_read_store<S>(
    store: &S,
    snapshot_id: &str,
    path: &str,
) -> Result<Option<JsonValue>, String>
where
    S: LocalSnapshotTreeReadStore + ?Sized,
{
    store.snapshot_tree_path_row(snapshot_id, path)
}
