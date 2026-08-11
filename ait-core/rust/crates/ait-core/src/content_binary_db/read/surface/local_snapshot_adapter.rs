use super::*;

const BLAME_REVERSE_ARCHIVE_RESET_INTERVAL: usize = 128;

impl<B, const WRITE_LAYOUT: u32> LocalSnapshotTreeReadStore
    for BinaryDbSnapshotStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    fn snapshot_tree_root_locator(
        &self,
        snapshot_id: &str,
    ) -> Result<SnapshotTreeRootLocator, String> {
        let read = self.begin_read_txn();
        let view = self
            .get_snapshot_view(&read, snapshot_id)?
            .ok_or_else(|| format!("Unknown snapshot: {snapshot_id}"))?;
        snapshot_tree_root_locator_from_view(&view)
    }

    fn snapshot_tree_manifest_path(&self, snapshot_id: &str) -> Result<String, String> {
        let read = self.begin_read_txn();
        let view = self
            .get_snapshot_view(&read, snapshot_id)?
            .ok_or_else(|| format!("Unknown snapshot: {snapshot_id}"))?;
        snapshot_tree_manifest_path_from_view(&view)
    }

    fn snapshot_tree_path_delta(
        &self,
        old_snapshot_id: Option<&str>,
        new_snapshot_id: Option<&str>,
    ) -> Result<SnapshotPathDelta, String> {
        let old_rows = self
            .snapshot_tree_file_rows(old_snapshot_id)?
            .into_iter()
            .map(|row| (row.path.clone(), row))
            .collect::<BTreeMap<_, _>>();
        let new_rows = self
            .snapshot_tree_file_rows(new_snapshot_id)?
            .into_iter()
            .map(|row| (row.path.clone(), row))
            .collect::<BTreeMap<_, _>>();
        let mut status_by_path = BTreeMap::new();
        for path in old_rows.keys().chain(new_rows.keys()) {
            if status_by_path.contains_key(path) {
                continue;
            }
            match (old_rows.get(path), new_rows.get(path)) {
                (None, Some(_)) => {
                    status_by_path.insert(path.clone(), "added".to_string());
                }
                (Some(_), None) => {
                    status_by_path.insert(path.clone(), "deleted".to_string());
                }
                (Some(old), Some(new))
                    if old.blob_id != new.blob_id
                        || old.sha256 != new.sha256
                        || old.size_bytes != new.size_bytes =>
                {
                    status_by_path.insert(path.clone(), "modified".to_string());
                }
                (Some(old), Some(new)) if old.mode != new.mode => {
                    status_by_path.insert(path.clone(), "mode_changed".to_string());
                }
                _ => {}
            }
        }
        Ok(SnapshotPathDelta {
            affected_paths: status_by_path.keys().cloned().collect(),
            status_by_path,
        })
    }

    fn snapshot_tree_file_rows(
        &self,
        snapshot_id: Option<&str>,
    ) -> Result<Vec<SnapshotFileRow>, String> {
        let Some(snapshot_id) = snapshot_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Vec::new());
        };
        let read = self.begin_read_txn();
        let root_tree_id = self
            .get_snapshot_view(&read, snapshot_id)?
            .and_then(|view| view.root_tree_id)
            .ok_or_else(|| missing_root_tree_pack_locator_error(snapshot_id))?;
        let mut rows = Vec::new();
        let mut visited = BTreeSet::new();
        let mut cache = BinaryDbTreeReadCache::default();
        collect_binary_snapshot_file_rows::<B, WRITE_LAYOUT>(
            &read,
            self.repo_root(),
            &root_tree_id,
            "",
            &mut rows,
            &mut visited,
            &mut cache,
        )?;
        rows.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(rows)
    }

    fn snapshot_tree_path_file_rows(
        &self,
        snapshot_id: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, SnapshotFileRow>, String> {
        let read = self.begin_read_txn();
        let root_tree_id = self
            .get_snapshot_view(&read, snapshot_id)?
            .and_then(|view| view.root_tree_id)
            .ok_or_else(|| missing_root_tree_pack_locator_error(snapshot_id))?;
        let mut requested = BinarySnapshotPathRequest::default();
        for path in paths {
            requested.insert(&require_non_empty(path, "path")?);
        }
        let mut rows = BTreeMap::new();
        let mut visited = BTreeSet::new();
        let mut cache = BinaryDbTreeReadCache::default();
        collect_binary_snapshot_path_file_rows::<B, WRITE_LAYOUT>(
            &read,
            self.repo_root(),
            &root_tree_id,
            &requested,
            &mut rows,
            &mut visited,
            &mut cache,
        )?;
        Ok(rows)
    }

    fn snapshot_tree_path_rows(
        &self,
        snapshot_id: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, JsonValue>, String> {
        Ok(self
            .snapshot_tree_path_file_rows(snapshot_id, paths)?
            .into_iter()
            .map(|(path, row)| (path, snapshot_file_row_json(&row)))
            .collect())
    }

    fn snapshot_tree_path_rows_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &str,
    ) -> Result<BTreeMap<String, JsonValue>, String> {
        let normalized_path = require_non_empty(path, "path")?;
        let mut rows = BTreeMap::new();
        if snapshot_ids.is_empty() {
            return Ok(rows);
        }
        let read = self.begin_read_txn();
        let mut cache = BinaryDbTreeReadCache::default();
        for snapshot_id in snapshot_ids {
            let normalized_snapshot_id = require_non_empty(snapshot_id, "snapshot_id")?;
            let root_tree_id = self
                .get_snapshot_view(&read, &normalized_snapshot_id)?
                .and_then(|view| view.root_tree_id)
                .ok_or_else(|| missing_root_tree_pack_locator_error(&normalized_snapshot_id))?;
            if let Some(row) = binary_snapshot_path_file_row::<B, WRITE_LAYOUT>(
                &read,
                self.repo_root(),
                &root_tree_id,
                &normalized_path,
                &mut cache,
            )? {
                rows.insert(normalized_snapshot_id, snapshot_file_row_json(&row));
            }
        }
        Ok(rows)
    }

    fn snapshot_tree_path_blob_rows_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &str,
    ) -> Result<Vec<SnapshotPathBlobRow>, String> {
        let _range = crate::perfetto_range!("ait.core.blame.path_history_bulk_read");
        let normalized_path = require_non_empty(path, "path")?;
        let mut rows = Vec::new();
        if snapshot_ids.is_empty() {
            return Ok(rows);
        }
        let mut cache = BinaryDbTreeReadCache::default();
        let read = self.begin_read_txn();
        for (snapshot_index, snapshot_id) in snapshot_ids.iter().enumerate() {
            let _snapshot_range = crate::perfetto_range!("ait.core.blame.snapshot_path");
            let normalized_snapshot_id = require_non_empty(snapshot_id, "snapshot_id")?;
            let root_tree_id = {
                let _range = crate::perfetto_range!("ait.core.blame.snapshot_lookup");
                self.get_snapshot_view(&read, &normalized_snapshot_id)?
                    .and_then(|view| view.root_tree_id)
            };
            let Some(root_tree_id) = root_tree_id else {
                continue;
            };
            let entry = {
                let _range = crate::perfetto_range!("ait.core.blame.snapshot_path_entry");
                binary_snapshot_path_entry::<B, WRITE_LAYOUT>(
                    &read,
                    self.repo_root(),
                    &root_tree_id,
                    &normalized_path,
                    &mut cache,
                )?
            };
            if let Some(entry) = entry {
                rows.push(SnapshotPathBlobRow {
                    snapshot_index,
                    blob_id: entry.target_id,
                });
            }
        }
        Ok(rows)
    }

    fn visit_snapshot_tree_path_blobs_reverse(
        &self,
        snapshot_ids: &[String],
        path: &str,
        visitor: &mut dyn FnMut(usize, Option<String>) -> Result<bool, String>,
    ) -> Result<(), String> {
        let _range = crate::perfetto_range!("ait.core.blame.path_history_reverse_visit");
        let normalized_path = require_non_empty(path, "path")?;
        if snapshot_ids.is_empty() {
            return Ok(());
        }
        let mut cache = BinaryDbTreeReadCache::default();
        let read = self.begin_read_txn();
        let mut visited_count = 0_usize;
        for (snapshot_index, snapshot_id) in snapshot_ids.iter().enumerate().rev() {
            let _snapshot_range = crate::perfetto_range!("ait.core.blame.reverse_snapshot_path");
            let normalized_snapshot_id = require_non_empty(snapshot_id, "snapshot_id")?;
            let root_tree_id = {
                let _range = crate::perfetto_range!("ait.core.blame.snapshot_lookup");
                self.get_snapshot_view(&read, &normalized_snapshot_id)?
                    .and_then(|view| view.root_tree_id)
            };
            let blob_id = match root_tree_id {
                Some(root_tree_id) => {
                    let _range = crate::perfetto_range!("ait.core.blame.snapshot_path_entry");
                    binary_snapshot_path_entry::<B, WRITE_LAYOUT>(
                        &read,
                        self.repo_root(),
                        &root_tree_id,
                        &normalized_path,
                        &mut cache,
                    )?
                    .map(|entry| entry.target_id)
                }
                None => None,
            };
            let keep_visiting = visitor(snapshot_index, blob_id)?;
            cache.clear_tree_entries();
            visited_count += 1;
            if visited_count.is_multiple_of(BLAME_REVERSE_ARCHIVE_RESET_INTERVAL) {
                cache.clear_archives();
            }
            if !keep_visiting {
                break;
            }
        }
        Ok(())
    }

    fn snapshot_tree_path_row(
        &self,
        snapshot_id: &str,
        path: &str,
    ) -> Result<Option<JsonValue>, String> {
        let normalized_path = require_non_empty(path, "path")?;
        let read = self.begin_read_txn();
        let root_tree_id = self
            .get_snapshot_view(&read, snapshot_id)?
            .and_then(|view| view.root_tree_id)
            .ok_or_else(|| missing_root_tree_pack_locator_error(snapshot_id))?;
        let mut cache = BinaryDbTreeReadCache::default();
        Ok(binary_snapshot_path_file_row::<B, WRITE_LAYOUT>(
            &read,
            self.repo_root(),
            &root_tree_id,
            &normalized_path,
            &mut cache,
        )?
        .map(|row| snapshot_file_row_json(&row)))
    }
}

pub(super) fn snapshot_tree_root_locator_from_view(
    view: &BinarySnapshotView,
) -> Result<SnapshotTreeRootLocator, String> {
    let root_tree_id = view
        .root_tree_id
        .clone()
        .ok_or_else(|| missing_root_tree_pack_locator_error(&view.snapshot_id))?;
    let root_tree_pack_id = view
        .root_tree_pack_id
        .clone()
        .ok_or_else(|| missing_root_tree_pack_locator_error(&view.snapshot_id))?;
    Ok(SnapshotTreeRootLocator {
        root_tree_id,
        root_tree_pack_id,
        root_entry_ordinal: i64::from(view.root_entry_ordinal),
    })
}

pub(super) fn snapshot_tree_manifest_path_from_view(
    view: &BinarySnapshotView,
) -> Result<String, String> {
    let root_tree_id = view
        .root_tree_id
        .as_deref()
        .ok_or_else(|| missing_root_tree_pack_locator_error(&view.snapshot_id))?;
    let root_tree_pack_path = view
        .root_tree_pack_path
        .as_deref()
        .ok_or_else(|| missing_root_tree_pack_locator_error(&view.snapshot_id))?;
    Ok(crate::pack_substrate::tree_pack_manifest_path(
        root_tree_pack_path,
        &format!("trees/{root_tree_id}.json"),
    ))
}

pub(super) fn collect_binary_snapshot_file_rows<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    repo_root: &StorePath,
    tree_id: &str,
    prefix: &str,
    rows: &mut Vec<SnapshotFileRow>,
    visited: &mut BTreeSet<String>,
    cache: &mut BinaryDbTreeReadCache,
) -> Result<(), String>
where
    B: BinaryDb,
{
    if !visited.insert(tree_id.to_string()) {
        return Err(format!(
            "cycle detected while reading Binary DB snapshot tree {tree_id}"
        ));
    }
    let result = (|| {
        let tree = get_tree_view_by_id_with_cache::<B, WRITE_LAYOUT>(read, tree_id, cache)?
            .ok_or_else(|| format!("Unknown Binary DB tree: {tree_id}"))?;
        for entry in
            list_tree_entry_views_at::<B, WRITE_LAYOUT>(read, repo_root, tree.tree_index, cache)?
        {
            let path = join_path(prefix, &entry.entry_name);
            if entry.entry_type == "tree" {
                collect_binary_snapshot_file_rows::<B, WRITE_LAYOUT>(
                    read,
                    repo_root,
                    &entry.target_id,
                    &path,
                    rows,
                    visited,
                    cache,
                )?;
                continue;
            }
            let blob = get_blob_view_by_id::<B, WRITE_LAYOUT>(read, &entry.target_id)?
                .ok_or_else(|| format!("Unknown Binary DB blob: {}", entry.target_id))?;
            rows.push(SnapshotFileRow {
                path,
                blob_id: blob.blob_id,
                size_bytes: i64::try_from(blob.size_bytes)
                    .map_err(|_| format!("blob size overflows i64: {}", blob.size_bytes))?,
                mode: entry.mode.unwrap_or_default(),
                sha256: blob.sha256,
            });
        }
        Ok(())
    })();
    visited.remove(tree_id);
    result
}

#[derive(Default)]
struct BinarySnapshotPathRequest {
    terminal_paths: BTreeSet<String>,
    children: BTreeMap<String, BinarySnapshotPathRequest>,
}

impl BinarySnapshotPathRequest {
    fn insert(&mut self, normalized_path: &str) {
        let parts = normalized_path
            .split('/')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            return;
        }
        let mut request = self;
        for part in parts {
            request = request.children.entry(part.to_string()).or_default();
        }
        request.terminal_paths.insert(normalized_path.to_string());
    }
}

fn collect_binary_snapshot_path_file_rows<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    repo_root: &StorePath,
    tree_id: &str,
    request: &BinarySnapshotPathRequest,
    rows: &mut BTreeMap<String, SnapshotFileRow>,
    visited: &mut BTreeSet<String>,
    cache: &mut BinaryDbTreeReadCache,
) -> Result<(), String>
where
    B: BinaryDb,
{
    if request.children.is_empty() {
        return Ok(());
    }
    if !visited.insert(tree_id.to_string()) {
        return Err(format!(
            "cycle detected while reading Binary DB snapshot tree {tree_id}"
        ));
    }
    let result = (|| {
        let tree = get_tree_view_by_id_with_cache::<B, WRITE_LAYOUT>(read, tree_id, cache)?
            .ok_or_else(|| format!("Unknown Binary DB tree: {tree_id}"))?;
        let entries =
            list_tree_entry_views_at::<B, WRITE_LAYOUT>(read, repo_root, tree.tree_index, cache)?
                .into_iter()
                .map(|entry| (entry.entry_name.clone(), entry))
                .collect::<BTreeMap<_, _>>();
        for (component, child_request) in &request.children {
            let Some(entry) = entries.get(component) else {
                continue;
            };
            if entry.entry_type == "tree" {
                collect_binary_snapshot_path_file_rows::<B, WRITE_LAYOUT>(
                    read,
                    repo_root,
                    &entry.target_id,
                    child_request,
                    rows,
                    visited,
                    cache,
                )?;
                continue;
            }
            if entry.entry_type != "blob" || child_request.terminal_paths.is_empty() {
                continue;
            }
            let blob = get_blob_view_by_id::<B, WRITE_LAYOUT>(read, &entry.target_id)?
                .ok_or_else(|| format!("Unknown Binary DB blob: {}", entry.target_id))?;
            let size_bytes = i64::try_from(blob.size_bytes)
                .map_err(|_| format!("blob size overflows i64: {}", blob.size_bytes))?;
            for path in &child_request.terminal_paths {
                rows.insert(
                    path.clone(),
                    SnapshotFileRow {
                        path: path.clone(),
                        blob_id: blob.blob_id.clone(),
                        size_bytes,
                        mode: entry.mode.clone().unwrap_or_default(),
                        sha256: blob.sha256.clone(),
                    },
                );
            }
        }
        Ok(())
    })();
    visited.remove(tree_id);
    result
}

pub(super) fn binary_snapshot_path_entry<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    repo_root: &StorePath,
    root_tree_id: &str,
    normalized_path: &str,
    cache: &mut BinaryDbTreeReadCache,
) -> Result<Option<BinaryTreeEntryView>, String>
where
    B: BinaryDb,
{
    let _range = crate::perfetto_range!("ait.core.snapshot.path_entry");
    let parts = normalized_path
        .split('/')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        return Ok(None);
    }
    let mut current_tree_id = root_tree_id.to_string();
    for (index, part) in parts.iter().enumerate() {
        let tree = {
            let _range = crate::perfetto_range!("ait.core.snapshot.path_entry.tree_lookup");
            get_tree_view_by_id_with_cache::<B, WRITE_LAYOUT>(read, &current_tree_id, cache)?
                .ok_or_else(|| format!("Unknown Binary DB tree: {current_tree_id}"))?
        };
        let entries = {
            let _range = crate::perfetto_range!("ait.core.snapshot.path_entry.entries");
            list_tree_entry_views_at::<B, WRITE_LAYOUT>(read, repo_root, tree.tree_index, cache)?
        };
        let Some(entry) = entries.into_iter().find(|entry| entry.entry_name == *part) else {
            return Ok(None);
        };
        if index == parts.len() - 1 {
            if entry.entry_type != "blob" {
                return Ok(None);
            }
            return Ok(Some(entry));
        }
        if entry.entry_type != "tree" {
            return Ok(None);
        }
        current_tree_id = entry.target_id;
    }
    Ok(None)
}

pub(super) fn binary_snapshot_path_file_row<B, const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, B>,
    repo_root: &StorePath,
    root_tree_id: &str,
    normalized_path: &str,
    cache: &mut BinaryDbTreeReadCache,
) -> Result<Option<SnapshotFileRow>, String>
where
    B: BinaryDb,
{
    let Some(entry) = binary_snapshot_path_entry::<B, WRITE_LAYOUT>(
        read,
        repo_root,
        root_tree_id,
        normalized_path,
        cache,
    )?
    else {
        return Ok(None);
    };
    let blob = get_blob_view_by_id::<B, WRITE_LAYOUT>(read, &entry.target_id)?
        .ok_or_else(|| format!("Unknown Binary DB blob: {}", entry.target_id))?;
    Ok(Some(SnapshotFileRow {
        path: normalized_path.to_string(),
        blob_id: blob.blob_id,
        size_bytes: i64::try_from(blob.size_bytes)
            .map_err(|_| format!("blob size overflows i64: {}", blob.size_bytes))?,
        mode: entry.mode.unwrap_or_default(),
        sha256: blob.sha256,
    }))
}

pub(super) fn snapshot_file_row_json(row: &SnapshotFileRow) -> JsonValue {
    SnapshotJson::stateless().snapshot_file_row_payload(row)
}

pub(super) fn require_non_empty(value: &str, label: &str) -> Result<String, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(format!("{label} must not be empty."));
    }
    Ok(normalized.to_string())
}

pub(super) fn missing_root_tree_pack_locator_error(snapshot_id: &str) -> String {
    format!(
        "Snapshot {snapshot_id} is missing root tree pack locator metadata; rerun `ait init --repair-existing` before reading history."
    )
}

pub(super) fn path_to_str(path: &std::path::Path) -> StoreResult<&str> {
    Ok(path
        .to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))?)
}
