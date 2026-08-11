use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use ait_core::external::lockfile::{ExternalLockCodec, ExternalLockNode, TomlExternalLockCodec};
use ait_core::external::materializer::{
    ExternalContentSource, ExternalMaterializationOptions, ExternalMaterializer,
    FilesystemExternalMaterializer,
};
use ait_core::external::{ExternalError, ExternalResult};
use ait_core::pack_substrate::{
    MAX_DELTA_CHAIN_READ_DEPTH, PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    pack_index_checksum_with_format, read_pack_entry_with_format, read_pack_index_with_format,
    read_tree_pack_index_with_format, read_tree_pack_tree_by_ordinal_with_format,
    read_tree_pack_tree_with_format, tree_pack_index_checksum_with_format,
};
use ait_core::server_operational::RepositoryIndex;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};

use crate::RunnerError;

pub const IMPORT_MANIFEST_CONTRACT: &str = "ait.remote_sync.zstd_bulk.import_manifest.v1";
pub const MAX_IMPORT_MANIFEST_BYTES: usize = 64 * 1024 * 1024;

const MAX_MANIFEST_RECORDS: usize = 2_000_000;
const MAX_EXTERNAL_NODES: usize = 256;
const MAX_TREE_DEPTH: usize = 1024;
const MAX_MATERIALIZED_FILES: u64 = 10_000_000;
const MAX_MATERIALIZED_BYTES: u64 = 512 * 1024 * 1024 * 1024;
const PACK_DOWNLOAD_ALLOWANCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PACK_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const PACK_DOWNLOAD_CONCURRENCY: usize = 8;
const PACK_DECODE_CONCURRENCY: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteSnapshotReference {
    pub repository_index: Option<RepositoryIndex>,
    pub repository_name: String,
    pub legacy_repo_id: Option<String>,
    pub snapshot_id: String,
    pub external_repository_indexes: BTreeMap<String, RepositoryIndex>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemotePackKind {
    Object,
    Tree,
}

pub trait RemoteSnapshotProvider: Send + Sync {
    fn fetch_import_manifest(
        &self,
        source: &RemoteSnapshotReference,
    ) -> Result<Vec<u8>, RunnerError>;

    fn download_pack(
        &self,
        source: &RemoteSnapshotReference,
        kind: RemotePackKind,
        pack_id: &str,
        destination: &Path,
        maximum_bytes: u64,
    ) -> Result<u64, RunnerError>;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MaterializationStats {
    pub file_count: u64,
    pub total_bytes: u64,
    pub entry_count: u64,
}

impl MaterializationStats {
    pub fn add(&mut self, other: &Self) -> Result<(), RunnerError> {
        self.file_count = self
            .file_count
            .checked_add(other.file_count)
            .ok_or_else(|| {
                RunnerError::InvalidRequest("materialized file count overflowed u64".to_string())
            })?;
        self.total_bytes = self
            .total_bytes
            .checked_add(other.total_bytes)
            .ok_or_else(|| {
                RunnerError::InvalidRequest("materialized byte count overflowed u64".to_string())
            })?;
        self.entry_count = self
            .entry_count
            .checked_add(other.entry_count)
            .ok_or_else(|| {
                RunnerError::InvalidRequest("materialized entry count overflowed u64".to_string())
            })?;
        enforce_materialization_bounds(self)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RemoteMaterialization {
    pub stats: MaterializationStats,
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct ImportManifest {
    contract: String,
    repo_name: String,
    snapshot_id: String,
    snapshots: Vec<SnapshotRow>,
    object_packs: Vec<ObjectPackRow>,
    tree_packs: Vec<TreePackRow>,
    blob_locators: Vec<BlobLocatorRow>,
    tree_locators: Vec<TreeLocatorRow>,
}

#[derive(Clone, Debug, Deserialize)]
struct SnapshotRow {
    snapshot_id: String,
    root_tree_pack_id: String,
    root_entry_ordinal: u64,
    file_count: u64,
    total_bytes: u64,
}

#[derive(Clone, Debug, Deserialize)]
struct ObjectPackRow {
    pack_id: String,
    pack_format: String,
    member_count: u64,
    total_bytes: u64,
    pack_index_checksum: String,
}

#[derive(Clone, Debug, Deserialize)]
struct TreePackRow {
    pack_id: String,
    pack_format: String,
    tree_count: u64,
    total_bytes: u64,
    pack_index_checksum: String,
}

#[derive(Clone, Debug, Deserialize)]
struct BlobLocatorRow {
    blob_id: String,
    sha256: String,
    size_bytes: u64,
    pack_id: String,
    pack_entry_name: String,
    pack_entry_type: String,
    #[serde(default)]
    pack_base_blob_id: Option<String>,
    pack_chain_depth: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct TreeLocatorRow {
    tree_id: String,
    entry_count: u64,
    tree_pack_id: String,
    tree_pack_checksum: String,
}

#[derive(Debug)]
struct DownloadedPack {
    path: PathBuf,
    index: JsonValue,
}

#[derive(Clone, Debug)]
struct PlannedFile {
    path: PathBuf,
    blob_id: String,
    declared_size: u64,
    mode: String,
}

#[derive(Clone, Debug)]
struct PackDownloadTask {
    kind: RemotePackKind,
    pack_id: String,
    path: PathBuf,
    maximum_bytes: u64,
}

struct SnapshotMaterializer<'a> {
    provider: &'a dyn RemoteSnapshotProvider,
    source: RemoteSnapshotReference,
    pack_root: PathBuf,
    snapshot: SnapshotRow,
    object_packs: BTreeMap<String, ObjectPackRow>,
    tree_packs: BTreeMap<String, TreePackRow>,
    blob_locators: BTreeMap<String, BlobLocatorRow>,
    tree_locators: BTreeMap<String, TreeLocatorRow>,
    downloaded_object_packs: BTreeMap<String, DownloadedPack>,
    downloaded_tree_packs: BTreeMap<String, DownloadedPack>,
    tree_stack: BTreeSet<String>,
    stats: MaterializationStats,
}

pub fn materialize_remote_snapshot(
    provider: &dyn RemoteSnapshotProvider,
    source: &RemoteSnapshotReference,
    workspace: &Path,
    pack_root: &Path,
) -> Result<RemoteMaterialization, RunnerError> {
    ensure_empty_directory(workspace, "remote snapshot workspace")?;
    fs::create_dir_all(pack_root)
        .map_err(|error| RunnerError::fs("create remote pack root", pack_root, error))?;

    let mut root = SnapshotMaterializer::load(provider, source, pack_root.to_path_buf())?;
    root.materialize(workspace)?;
    let mut stats = root.stats.clone();

    let environment = materialize_declared_externals(provider, workspace, pack_root, &mut stats)?;
    Ok(RemoteMaterialization { stats, environment })
}

impl<'a> SnapshotMaterializer<'a> {
    fn load(
        provider: &'a dyn RemoteSnapshotProvider,
        source: &RemoteSnapshotReference,
        pack_root: PathBuf,
    ) -> Result<Self, RunnerError> {
        let manifest_bytes = provider.fetch_import_manifest(source)?;
        if manifest_bytes.len() > MAX_IMPORT_MANIFEST_BYTES {
            return Err(RunnerError::Server(format!(
                "remote Snapshot import manifest is {} bytes; maximum is {MAX_IMPORT_MANIFEST_BYTES}",
                manifest_bytes.len()
            )));
        }
        let manifest: ImportManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| {
                RunnerError::InvalidRequest(format!(
                    "remote Snapshot import manifest is not valid typed JSON: {error}"
                ))
            })?;
        validate_manifest_identity(&manifest, source)?;
        validate_manifest_record_bounds(&manifest)?;

        if manifest.snapshots.len() != 1 {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot import manifest requires exactly one snapshot row, got {}",
                manifest.snapshots.len()
            )));
        }
        let snapshot = manifest
            .snapshots
            .into_iter()
            .next()
            .expect("length checked");
        if snapshot.snapshot_id != source.snapshot_id {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot row identity `{}` does not match requested `{}`",
                snapshot.snapshot_id, source.snapshot_id
            )));
        }
        if snapshot.file_count > MAX_MATERIALIZED_FILES
            || snapshot.total_bytes > MAX_MATERIALIZED_BYTES
        {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot declares {} files and {} bytes, exceeding runner materialization bounds",
                snapshot.file_count, snapshot.total_bytes
            )));
        }

        let object_packs = unique_by(
            manifest.object_packs,
            |row| row.pack_id.clone(),
            "object pack",
        )?;
        let tree_packs = unique_by(manifest.tree_packs, |row| row.pack_id.clone(), "tree pack")?;
        let blob_locators = unique_by(
            manifest.blob_locators,
            |row| row.blob_id.clone(),
            "blob locator",
        )?;
        let tree_locators = unique_by(
            manifest.tree_locators,
            |row| row.tree_id.clone(),
            "tree locator",
        )?;

        if !tree_packs.contains_key(&snapshot.root_tree_pack_id) {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot root references missing tree pack `{}`",
                snapshot.root_tree_pack_id
            )));
        }
        Ok(Self {
            provider,
            source: source.clone(),
            pack_root,
            snapshot,
            object_packs,
            tree_packs,
            blob_locators,
            tree_locators,
            downloaded_object_packs: BTreeMap::new(),
            downloaded_tree_packs: BTreeMap::new(),
            tree_stack: BTreeSet::new(),
            stats: MaterializationStats::default(),
        })
    }

    fn materialize(&mut self, destination: &Path) -> Result<(), RunnerError> {
        let mut planned_files = Vec::new();
        let root_pack_id = self.snapshot.root_tree_pack_id.clone();
        let root_ordinal = usize::try_from(self.snapshot.root_entry_ordinal).map_err(|_| {
            RunnerError::InvalidRequest(format!(
                "root tree ordinal {} exceeds platform capacity",
                self.snapshot.root_entry_ordinal
            ))
        })?;
        let (root_pack_path, root_pack_index) = {
            let root_pack = self.ensure_tree_pack(&root_pack_id)?;
            (root_pack.path.clone(), root_pack.index.clone())
        };
        let root_payload = read_tree_pack_tree_by_ordinal_with_format(
            path_text(&root_pack_path)?,
            root_ordinal,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .map_err(|error| invalid_pack("read root tree", &root_pack_id, error))?;
        let root_tree_id = required_text(&root_payload, "tree_id", "root tree payload")?;
        let rows = root_payload.get("rows").cloned().ok_or_else(|| {
            RunnerError::InvalidRequest(
                "root tree payload is missing array field `rows`".to_string(),
            )
        })?;
        self.validate_tree_locator(&root_tree_id, &root_pack_id, &root_pack_index)?;
        self.plan_tree_rows(&root_tree_id, &rows, destination, 0, &mut planned_files)?;
        self.validate_planned_inventory(&planned_files)?;
        self.prefetch_planned_object_packs(&planned_files)?;
        self.materialize_planned_files(&planned_files)?;

        if self.stats.file_count != self.snapshot.file_count
            || self.stats.total_bytes != self.snapshot.total_bytes
        {
            return Err(RunnerError::InvalidRequest(format!(
                "materialized Snapshot inventory mismatch: expected {} files/{} bytes, reconstructed {} files/{} bytes",
                self.snapshot.file_count,
                self.snapshot.total_bytes,
                self.stats.file_count,
                self.stats.total_bytes
            )));
        }
        Ok(())
    }

    fn plan_tree(
        &mut self,
        tree_id: &str,
        destination: &Path,
        depth: usize,
        planned_files: &mut Vec<PlannedFile>,
    ) -> Result<(), RunnerError> {
        let locator = self.tree_locators.get(tree_id).cloned().ok_or_else(|| {
            RunnerError::InvalidRequest(format!(
                "tree `{tree_id}` has no locator in the import manifest"
            ))
        })?;
        let (pack_path, pack_index) = {
            let pack = self.ensure_tree_pack(&locator.tree_pack_id)?;
            (pack.path.clone(), pack.index.clone())
        };
        self.validate_tree_locator(tree_id, &locator.tree_pack_id, &pack_index)?;
        let rows = read_tree_pack_tree_with_format(
            path_text(&pack_path)?,
            tree_id,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .map_err(|error| invalid_pack("read tree", &locator.tree_pack_id, error))?;
        self.plan_tree_rows(tree_id, &rows, destination, depth, planned_files)
    }

    fn plan_tree_rows(
        &mut self,
        tree_id: &str,
        rows: &JsonValue,
        destination: &Path,
        depth: usize,
        planned_files: &mut Vec<PlannedFile>,
    ) -> Result<(), RunnerError> {
        if depth > MAX_TREE_DEPTH {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot tree depth exceeds {MAX_TREE_DEPTH}"
            )));
        }
        if !self.tree_stack.insert(tree_id.to_string()) {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot tree cycle detected at `{tree_id}`"
            )));
        }
        let result = self.plan_tree_rows_inner(rows, destination, depth, planned_files);
        self.tree_stack.remove(tree_id);
        result
    }

    fn plan_tree_rows_inner(
        &mut self,
        rows: &JsonValue,
        destination: &Path,
        depth: usize,
        planned_files: &mut Vec<PlannedFile>,
    ) -> Result<(), RunnerError> {
        let rows = rows.as_array().ok_or_else(|| {
            RunnerError::InvalidRequest("tree pack member payload must be an array".to_string())
        })?;
        let mut names = BTreeSet::new();
        let mut child_trees = Vec::new();
        for row in rows {
            let name = required_text(row, "entry_name", "tree entry")?;
            validate_tree_entry_name(&name)?;
            if !names.insert(name.clone()) {
                return Err(RunnerError::InvalidRequest(format!(
                    "tree contains duplicate entry name `{name}`"
                )));
            }
            self.stats.entry_count = self.stats.entry_count.checked_add(1).ok_or_else(|| {
                RunnerError::InvalidRequest("materialized entry count overflowed u64".to_string())
            })?;
            enforce_materialization_bounds(&self.stats)?;

            let entry_type = required_text(row, "entry_type", "tree entry")?;
            let target_id = required_text(row, "target_id", "tree entry")?;
            let output = destination.join(&name);
            if fs::symlink_metadata(&output).is_ok() {
                return Err(RunnerError::InvalidRequest(format!(
                    "tree entry `{name}` collides with an existing materialized path"
                )));
            }
            match entry_type.as_str() {
                "tree" => {
                    fs::create_dir(&output).map_err(|error| {
                        RunnerError::fs("create Snapshot directory", &output, error)
                    })?;
                    child_trees.push((target_id, output));
                }
                "blob" => {
                    let declared_size = row
                        .get("size_bytes")
                        .and_then(JsonValue::as_u64)
                        .ok_or_else(|| {
                            RunnerError::InvalidRequest(format!(
                                "blob tree entry `{name}` requires non-negative size_bytes"
                            ))
                        })?;
                    let mode = required_text(row, "mode", "blob tree entry")?;
                    parse_file_permissions(&mode)?;
                    planned_files.push(PlannedFile {
                        path: output,
                        blob_id: target_id,
                        declared_size,
                        mode,
                    });
                }
                other => {
                    return Err(RunnerError::InvalidRequest(format!(
                        "tree entry `{name}` has unsupported entry_type `{other}`"
                    )));
                }
            }
        }
        let child_pack_ids = child_trees
            .iter()
            .map(|(tree_id, _)| {
                self.tree_locators
                    .get(tree_id)
                    .map(|locator| locator.tree_pack_id.clone())
                    .ok_or_else(|| {
                        RunnerError::InvalidRequest(format!(
                            "tree `{tree_id}` has no locator in the import manifest"
                        ))
                    })
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        self.prefetch_tree_packs(&child_pack_ids)?;
        for (tree_id, output) in child_trees {
            self.plan_tree(&tree_id, &output, depth + 1, planned_files)?;
        }
        Ok(())
    }

    fn validate_planned_inventory(&self, planned_files: &[PlannedFile]) -> Result<(), RunnerError> {
        let file_count = u64::try_from(planned_files.len()).map_err(|_| {
            RunnerError::InvalidRequest("planned file count exceeds u64".to_string())
        })?;
        let total_bytes = planned_files.iter().try_fold(0u64, |total, file| {
            total.checked_add(file.declared_size).ok_or_else(|| {
                RunnerError::InvalidRequest("planned byte count overflowed u64".to_string())
            })
        })?;
        if file_count != self.snapshot.file_count || total_bytes != self.snapshot.total_bytes {
            return Err(RunnerError::InvalidRequest(format!(
                "planned Snapshot inventory mismatch: expected {} files/{} bytes, found {file_count} files/{total_bytes} bytes",
                self.snapshot.file_count, self.snapshot.total_bytes
            )));
        }
        Ok(())
    }

    fn materialize_planned_files(
        &mut self,
        planned_files: &[PlannedFile],
    ) -> Result<(), RunnerError> {
        for files in planned_files.chunks(PACK_DECODE_CONCURRENCY) {
            let decoded = self.decode_planned_batch(files)?;
            for (file, bytes) in files.iter().zip(decoded) {
                if bytes.len() as u64 != file.declared_size {
                    return Err(RunnerError::InvalidRequest(format!(
                        "blob `{}` tree size {} differs from decoded size {}",
                        file.blob_id,
                        file.declared_size,
                        bytes.len()
                    )));
                }
                write_materialized_file(&file.path, &bytes, &file.mode)?;
                self.stats.file_count = self.stats.file_count.checked_add(1).ok_or_else(|| {
                    RunnerError::InvalidRequest(
                        "materialized file count overflowed u64".to_string(),
                    )
                })?;
                self.stats.total_bytes = self
                    .stats
                    .total_bytes
                    .checked_add(bytes.len() as u64)
                    .ok_or_else(|| {
                        RunnerError::InvalidRequest(
                            "materialized byte count overflowed u64".to_string(),
                        )
                    })?;
                enforce_materialization_bounds(&self.stats)?;
            }
        }
        Ok(())
    }

    fn decode_planned_batch(
        &self,
        planned_files: &[PlannedFile],
    ) -> Result<Vec<Vec<u8>>, RunnerError> {
        let results = Mutex::new(Vec::<(usize, Result<Vec<u8>, RunnerError>)>::new());
        thread::scope(|scope| -> Result<(), RunnerError> {
            let mut workers = Vec::with_capacity(planned_files.len());
            for (index, file) in planned_files.iter().enumerate() {
                let results = &results;
                let materializer = self;
                workers.push(scope.spawn(move || {
                    let mut resolving = BTreeSet::new();
                    let mut decoded = BTreeMap::new();
                    let result = materializer.decode_blob_readonly(
                        &file.blob_id,
                        &mut resolving,
                        &mut decoded,
                    );
                    if let Ok(mut output) = results.lock() {
                        output.push((index, result));
                    }
                }));
            }
            for worker in workers {
                worker.join().map_err(|_| {
                    RunnerError::Process("remote pack decode worker panicked".to_string())
                })?;
            }
            Ok(())
        })?;
        let mut results = results
            .into_inner()
            .map_err(|_| RunnerError::Process("remote pack decode lock poisoned".to_string()))?;
        results.sort_by_key(|(index, _)| *index);
        if results.len() != planned_files.len() {
            return Err(RunnerError::Process(
                "remote pack decode worker did not return every file".to_string(),
            ));
        }
        results
            .into_iter()
            .map(|(_, result)| result)
            .collect::<Result<Vec<_>, _>>()
    }

    fn decode_blob_readonly(
        &self,
        blob_id: &str,
        resolving: &mut BTreeSet<String>,
        decoded: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<u8>, RunnerError> {
        if let Some(bytes) = decoded.get(blob_id) {
            return Ok(bytes.clone());
        }
        if !resolving.insert(blob_id.to_string()) {
            return Err(RunnerError::InvalidRequest(format!(
                "object pack delta cycle detected at blob `{blob_id}`"
            )));
        }
        let result = self.decode_blob_readonly_inner(blob_id, resolving, decoded);
        resolving.remove(blob_id);
        let bytes = result?;
        decoded.insert(blob_id.to_string(), bytes.clone());
        Ok(bytes)
    }

    fn decode_blob_readonly_inner(
        &self,
        blob_id: &str,
        resolving: &mut BTreeSet<String>,
        decoded: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<u8>, RunnerError> {
        let locator = self.blob_locators.get(blob_id).ok_or_else(|| {
            RunnerError::InvalidRequest(format!(
                "blob `{blob_id}` has no locator in the import manifest"
            ))
        })?;
        validate_blob_locator(locator)?;
        let mut base_map = BTreeMap::new();
        if let Some(base_blob_id) = locator.pack_base_blob_id.as_deref() {
            let base = self.decode_blob_readonly(base_blob_id, resolving, decoded)?;
            base_map.insert(base_blob_id.to_string(), base);
        }
        let pack = self
            .downloaded_object_packs
            .get(&locator.pack_id)
            .ok_or_else(|| {
                RunnerError::Process(format!(
                    "required object pack `{}` was not prefetched",
                    locator.pack_id
                ))
            })?;
        validate_object_entry(&pack.index, locator)?;
        let bytes = read_pack_entry_with_format(
            path_text(&pack.path)?,
            &locator.pack_entry_name,
            (!base_map.is_empty()).then_some(&base_map),
            MAX_DELTA_CHAIN_READ_DEPTH,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .map_err(|error| invalid_pack("read object", &locator.pack_id, error))?;
        if bytes.len() as u64 != locator.size_bytes {
            return Err(RunnerError::InvalidRequest(format!(
                "blob `{blob_id}` decoded size {} differs from locator size {}",
                bytes.len(),
                locator.size_bytes
            )));
        }
        let actual_sha256 = sha256_hex(&bytes);
        if actual_sha256 != locator.sha256.to_ascii_lowercase() {
            return Err(RunnerError::InvalidRequest(format!(
                "blob `{blob_id}` sha256 mismatch"
            )));
        }
        Ok(bytes)
    }

    fn prefetch_tree_packs(&mut self, pack_ids: &BTreeSet<String>) -> Result<(), RunnerError> {
        let tasks = pack_ids
            .iter()
            .filter(|pack_id| !self.downloaded_tree_packs.contains_key(*pack_id))
            .map(|pack_id| {
                let row = self.tree_packs.get(pack_id).ok_or_else(|| {
                    RunnerError::InvalidRequest(format!(
                        "tree locator references missing tree pack `{pack_id}`"
                    ))
                })?;
                validate_tree_pack_row(row)?;
                Ok(PackDownloadTask {
                    kind: RemotePackKind::Tree,
                    pack_id: pack_id.clone(),
                    path: tree_pack_path(&self.pack_root, pack_id),
                    maximum_bytes: pack_download_limit(row.total_bytes),
                })
            })
            .collect::<Result<Vec<_>, RunnerError>>()?;
        download_pack_tasks(self.provider, &self.source, &tasks)?;
        for task in tasks {
            self.ensure_tree_pack(&task.pack_id)?;
        }
        Ok(())
    }

    fn prefetch_planned_object_packs(
        &mut self,
        planned_files: &[PlannedFile],
    ) -> Result<(), RunnerError> {
        let mut pack_ids = BTreeSet::new();
        let mut visited_blobs = BTreeSet::new();
        let mut visiting_blobs = BTreeSet::new();
        for file in planned_files {
            self.collect_required_object_packs(
                &file.blob_id,
                0,
                &mut visited_blobs,
                &mut visiting_blobs,
                &mut pack_ids,
            )?;
        }
        let tasks = pack_ids
            .iter()
            .filter(|pack_id| !self.downloaded_object_packs.contains_key(*pack_id))
            .map(|pack_id| {
                let row = self.object_packs.get(pack_id).ok_or_else(|| {
                    RunnerError::InvalidRequest(format!(
                        "blob locator references missing object pack `{pack_id}`"
                    ))
                })?;
                validate_object_pack_row(row)?;
                Ok(PackDownloadTask {
                    kind: RemotePackKind::Object,
                    pack_id: pack_id.clone(),
                    path: object_pack_path(&self.pack_root, pack_id),
                    maximum_bytes: pack_download_limit(row.total_bytes),
                })
            })
            .collect::<Result<Vec<_>, RunnerError>>()?;
        download_pack_tasks(self.provider, &self.source, &tasks)?;
        for task in tasks {
            self.ensure_object_pack(&task.pack_id)?;
        }
        Ok(())
    }

    fn collect_required_object_packs(
        &self,
        blob_id: &str,
        depth: usize,
        visited: &mut BTreeSet<String>,
        visiting: &mut BTreeSet<String>,
        pack_ids: &mut BTreeSet<String>,
    ) -> Result<(), RunnerError> {
        if visited.contains(blob_id) {
            return Ok(());
        }
        if depth > MAX_DELTA_CHAIN_READ_DEPTH {
            return Err(RunnerError::InvalidRequest(format!(
                "blob `{blob_id}` delta chain exceeds maximum {MAX_DELTA_CHAIN_READ_DEPTH}"
            )));
        }
        if !visiting.insert(blob_id.to_string()) {
            return Err(RunnerError::InvalidRequest(format!(
                "object pack delta cycle detected at blob `{blob_id}`"
            )));
        }
        let locator = self.blob_locators.get(blob_id).ok_or_else(|| {
            RunnerError::InvalidRequest(format!(
                "blob `{blob_id}` has no locator in the import manifest"
            ))
        })?;
        validate_blob_locator(locator)?;
        pack_ids.insert(locator.pack_id.clone());
        if let Some(base_blob_id) = locator.pack_base_blob_id.as_deref() {
            self.collect_required_object_packs(
                base_blob_id,
                depth + 1,
                visited,
                visiting,
                pack_ids,
            )?;
        }
        visiting.remove(blob_id);
        visited.insert(blob_id.to_string());
        Ok(())
    }

    fn ensure_object_pack(&mut self, pack_id: &str) -> Result<&DownloadedPack, RunnerError> {
        if !self.downloaded_object_packs.contains_key(pack_id) {
            let row = self.object_packs.get(pack_id).cloned().ok_or_else(|| {
                RunnerError::InvalidRequest(format!(
                    "blob locator references missing object pack `{pack_id}`"
                ))
            })?;
            validate_object_pack_row(&row)?;
            let path = object_pack_path(&self.pack_root, pack_id);
            if !regular_file_exists(&path)? {
                self.provider.download_pack(
                    &self.source,
                    RemotePackKind::Object,
                    pack_id,
                    &path,
                    pack_download_limit(row.total_bytes),
                )?;
            }
            let index = read_pack_index_with_format(path_text(&path)?, &row.pack_format)
                .map_err(|error| invalid_pack("read object pack index", pack_id, error))?;
            validate_pack_index_common(
                &index,
                pack_id,
                &row.pack_format,
                "member_count",
                row.member_count,
                row.total_bytes,
            )?;
            let checksum = pack_index_checksum_with_format(path_text(&path)?, &row.pack_format)
                .map_err(|error| invalid_pack("hash object pack index", pack_id, error))?
                .ok_or_else(|| {
                    RunnerError::InvalidRequest(format!(
                        "object pack `{pack_id}` has no index checksum"
                    ))
                })?;
            if checksum != row.pack_index_checksum.to_ascii_lowercase() {
                return Err(RunnerError::InvalidRequest(format!(
                    "object pack `{pack_id}` index checksum mismatch"
                )));
            }
            self.downloaded_object_packs
                .insert(pack_id.to_string(), DownloadedPack { path, index });
        }
        Ok(self
            .downloaded_object_packs
            .get(pack_id)
            .expect("inserted or already present"))
    }

    fn ensure_tree_pack(&mut self, pack_id: &str) -> Result<&DownloadedPack, RunnerError> {
        if !self.downloaded_tree_packs.contains_key(pack_id) {
            let row = self.tree_packs.get(pack_id).cloned().ok_or_else(|| {
                RunnerError::InvalidRequest(format!(
                    "tree locator references missing tree pack `{pack_id}`"
                ))
            })?;
            validate_tree_pack_row(&row)?;
            let path = tree_pack_path(&self.pack_root, pack_id);
            if !regular_file_exists(&path)? {
                self.provider.download_pack(
                    &self.source,
                    RemotePackKind::Tree,
                    pack_id,
                    &path,
                    pack_download_limit(row.total_bytes),
                )?;
            }
            let index = read_tree_pack_index_with_format(path_text(&path)?, &row.pack_format)
                .map_err(|error| invalid_pack("read tree pack index", pack_id, error))?;
            validate_pack_index_common(
                &index,
                pack_id,
                &row.pack_format,
                "tree_count",
                row.tree_count,
                row.total_bytes,
            )?;
            let checksum =
                tree_pack_index_checksum_with_format(path_text(&path)?, &row.pack_format)
                    .map_err(|error| invalid_pack("hash tree pack index", pack_id, error))?
                    .ok_or_else(|| {
                        RunnerError::InvalidRequest(format!(
                            "tree pack `{pack_id}` has no index checksum"
                        ))
                    })?;
            if checksum != row.pack_index_checksum.to_ascii_lowercase() {
                return Err(RunnerError::InvalidRequest(format!(
                    "tree pack `{pack_id}` index checksum mismatch"
                )));
            }
            self.downloaded_tree_packs
                .insert(pack_id.to_string(), DownloadedPack { path, index });
        }
        Ok(self
            .downloaded_tree_packs
            .get(pack_id)
            .expect("inserted or already present"))
    }

    fn validate_tree_locator(
        &self,
        tree_id: &str,
        pack_id: &str,
        index: &JsonValue,
    ) -> Result<(), RunnerError> {
        let locator = self.tree_locators.get(tree_id).ok_or_else(|| {
            RunnerError::InvalidRequest(format!(
                "tree `{tree_id}` has no locator in the import manifest"
            ))
        })?;
        if locator.tree_pack_id != pack_id {
            return Err(RunnerError::InvalidRequest(format!(
                "tree `{tree_id}` locator points to pack `{}`, not `{pack_id}`",
                locator.tree_pack_id
            )));
        }
        validate_sha256(&locator.tree_pack_checksum, "tree locator checksum")?;
        let entry = index
            .get("trees")
            .and_then(JsonValue::as_array)
            .and_then(|entries| {
                entries
                    .iter()
                    .find(|entry| entry.get("tree_id").and_then(JsonValue::as_str) == Some(tree_id))
            })
            .ok_or_else(|| {
                RunnerError::InvalidRequest(format!(
                    "tree pack `{pack_id}` index does not contain tree `{tree_id}`"
                ))
            })?;
        if entry.get("entry_count").and_then(JsonValue::as_u64) != Some(locator.entry_count)
            || entry.get("checksum").and_then(JsonValue::as_str)
                != Some(locator.tree_pack_checksum.as_str())
        {
            return Err(RunnerError::InvalidRequest(format!(
                "tree `{tree_id}` locator differs from downloaded tree pack index"
            )));
        }
        Ok(())
    }
}

fn download_pack_tasks(
    provider: &dyn RemoteSnapshotProvider,
    source: &RemoteSnapshotReference,
    tasks: &[PackDownloadTask],
) -> Result<(), RunnerError> {
    if tasks.is_empty() {
        return Ok(());
    }
    let next = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::<(usize, RunnerError)>::new());
    let worker_count = PACK_DOWNLOAD_CONCURRENCY.min(tasks.len());
    thread::scope(|scope| -> Result<(), RunnerError> {
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            workers.push(scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(task) = tasks.get(index) else {
                        break;
                    };
                    if let Err(error) = provider.download_pack(
                        source,
                        task.kind,
                        &task.pack_id,
                        &task.path,
                        task.maximum_bytes,
                    ) {
                        if let Ok(mut failures) = errors.lock() {
                            failures.push((index, error));
                        }
                        break;
                    }
                }
            }));
        }
        for worker in workers {
            worker.join().map_err(|_| {
                RunnerError::Process("remote pack download worker panicked".to_string())
            })?;
        }
        Ok(())
    })?;
    let mut errors = errors
        .into_inner()
        .map_err(|_| RunnerError::Process("remote pack error lock poisoned".to_string()))?;
    errors.sort_by_key(|(index, _)| *index);
    if let Some((_, error)) = errors.into_iter().next() {
        return Err(error);
    }
    Ok(())
}

fn object_pack_path(pack_root: &Path, pack_id: &str) -> PathBuf {
    pack_root.join(format!("object-{}.zstpack", stable_path_digest(pack_id)))
}

fn tree_pack_path(pack_root: &Path, pack_id: &str) -> PathBuf {
    pack_root.join(format!("tree-{}.zstpack", stable_path_digest(pack_id)))
}

fn regular_file_exists(path: &Path) -> Result<bool, RunnerError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(RunnerError::InvalidRequest(format!(
            "downloaded pack path `{}` is not a regular file",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(RunnerError::fs("inspect downloaded pack", path, error)),
    }
}

fn validate_object_pack_row(row: &ObjectPackRow) -> Result<(), RunnerError> {
    if row.pack_format != PACK_FORMAT_ZSTD_CHUNKED_V1 {
        return Err(RunnerError::InvalidRequest(format!(
            "object pack `{}` uses unsupported format `{}`",
            row.pack_id, row.pack_format
        )));
    }
    validate_sha256(&row.pack_index_checksum, "object pack index checksum")
}

fn validate_tree_pack_row(row: &TreePackRow) -> Result<(), RunnerError> {
    if row.pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
        return Err(RunnerError::InvalidRequest(format!(
            "tree pack `{}` uses unsupported format `{}`",
            row.pack_id, row.pack_format
        )));
    }
    validate_sha256(&row.pack_index_checksum, "tree pack index checksum")
}

fn materialize_declared_externals(
    provider: &dyn RemoteSnapshotProvider,
    workspace: &Path,
    pack_root: &Path,
    aggregate_stats: &mut MaterializationStats,
) -> Result<BTreeMap<String, String>, RunnerError> {
    let lock_path = workspace.join("ait-external.lock");
    let lock_bytes = match fs::read(&lock_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(error) => return Err(RunnerError::fs("read ait-external.lock", &lock_path, error)),
    };
    let lockfile = TomlExternalLockCodec
        .parse_lockfile(&lock_bytes)
        .map_err(|error| {
            RunnerError::InvalidRequest(format!("invalid ait-external.lock: {error}"))
        })?;
    if lockfile.nodes.len() > MAX_EXTERNAL_NODES {
        return Err(RunnerError::InvalidRequest(format!(
            "ait-external.lock contains {} nodes; maximum is {MAX_EXTERNAL_NODES}",
            lockfile.nodes.len()
        )));
    }
    let external_stats = Arc::new(Mutex::new(MaterializationStats::default()));
    let content_source = RemoteExternalContentSource {
        provider,
        pack_root: pack_root.join("externals"),
        sequence: AtomicU64::new(0),
        stats: Arc::clone(&external_stats),
    };
    let materializer = FilesystemExternalMaterializer::new(workspace, content_source)
        .map_err(external_materialization_error)?;
    materializer
        .materialize_lockfile(
            &lockfile,
            &ExternalMaterializationOptions::recursive()
                .with_locked(true)
                .with_release_ready(true),
        )
        .map_err(external_materialization_error)?;
    let stats = external_stats
        .lock()
        .map_err(|_| {
            RunnerError::Process("external materialization statistics lock poisoned".to_string())
        })?
        .clone();
    aggregate_stats.add(&stats)?;

    let mut environment = BTreeMap::new();
    for node in lockfile
        .nodes
        .iter()
        .filter(|node| node.parent_path.is_empty())
    {
        let key = external_environment_key(&node.name)?;
        let value = workspace.join(&node.materialize_to);
        let value = value.to_str().ok_or_else(|| {
            RunnerError::InvalidRequest(format!(
                "external materialization path for `{}` is not valid UTF-8",
                node.name
            ))
        })?;
        if environment.insert(key.clone(), value.to_string()).is_some() {
            return Err(RunnerError::InvalidRequest(format!(
                "external names map to duplicate environment key `{key}`"
            )));
        }
    }
    Ok(environment)
}

struct RemoteExternalContentSource<'a> {
    provider: &'a dyn RemoteSnapshotProvider,
    pack_root: PathBuf,
    sequence: AtomicU64,
    stats: Arc<Mutex<MaterializationStats>>,
}

impl ExternalContentSource for RemoteExternalContentSource<'_> {
    fn materialize_content(
        &self,
        node: &ExternalLockNode,
        destination: &Path,
    ) -> ExternalResult<()> {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let packs = self.pack_root.join(format!("node-{sequence}"));
        fs::create_dir_all(&packs).map_err(|error| {
            ExternalError::with_code(
                "runner_external_pack_root",
                format!("failed to create external pack root: {error}"),
            )
        })?;
        let source = declared_external_snapshot_reference(node);
        let mut materializer =
            SnapshotMaterializer::load(self.provider, &source, packs).map_err(runner_external)?;
        materializer
            .materialize(destination)
            .map_err(runner_external)?;
        let mut stats = self.stats.lock().map_err(|_| {
            ExternalError::with_code(
                "runner_external_stats",
                "external materialization statistics lock poisoned",
            )
        })?;
        stats.add(&materializer.stats).map_err(runner_external)
    }
}

fn declared_external_snapshot_reference(node: &ExternalLockNode) -> RemoteSnapshotReference {
    RemoteSnapshotReference {
        repository_index: Some(RepositoryIndex::new(node.repository_index)),
        repository_name: node.repo_name.clone(),
        legacy_repo_id: None,
        snapshot_id: node.snapshot.clone(),
        external_repository_indexes: BTreeMap::new(),
    }
}

fn validate_manifest_identity(
    manifest: &ImportManifest,
    source: &RemoteSnapshotReference,
) -> Result<(), RunnerError> {
    if manifest.contract != IMPORT_MANIFEST_CONTRACT {
        return Err(RunnerError::InvalidRequest(format!(
            "remote Snapshot manifest contract must be `{IMPORT_MANIFEST_CONTRACT}`, got `{}`",
            manifest.contract
        )));
    }
    if manifest.repo_name != source.repository_name || manifest.snapshot_id != source.snapshot_id {
        return Err(RunnerError::InvalidRequest(format!(
            "remote Snapshot manifest identity `{}/{}` does not match requested `{}/{}`",
            manifest.repo_name, manifest.snapshot_id, source.repository_name, source.snapshot_id
        )));
    }
    Ok(())
}

fn validate_manifest_record_bounds(manifest: &ImportManifest) -> Result<(), RunnerError> {
    for (label, count) in [
        ("snapshot", manifest.snapshots.len()),
        ("object pack", manifest.object_packs.len()),
        ("tree pack", manifest.tree_packs.len()),
        ("blob locator", manifest.blob_locators.len()),
        ("tree locator", manifest.tree_locators.len()),
    ] {
        if count > MAX_MANIFEST_RECORDS {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot manifest contains {count} {label} records; maximum is {MAX_MANIFEST_RECORDS}"
            )));
        }
    }
    Ok(())
}

fn unique_by<T>(
    values: Vec<T>,
    key: impl Fn(&T) -> String,
    label: &str,
) -> Result<BTreeMap<String, T>, RunnerError> {
    let mut result = BTreeMap::new();
    for value in values {
        let identity = key(&value);
        if identity.trim().is_empty() {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot manifest contains an empty {label} identity"
            )));
        }
        if result.insert(identity.clone(), value).is_some() {
            return Err(RunnerError::InvalidRequest(format!(
                "remote Snapshot manifest contains duplicate {label} `{identity}`"
            )));
        }
    }
    Ok(result)
}

fn validate_pack_index_common(
    index: &JsonValue,
    pack_id: &str,
    pack_format: &str,
    count_field: &str,
    expected_count: u64,
    expected_total_bytes: u64,
) -> Result<(), RunnerError> {
    if index.get("pack_id").and_then(JsonValue::as_str) != Some(pack_id)
        || index.get("pack_format").and_then(JsonValue::as_str) != Some(pack_format)
        || index.get(count_field).and_then(JsonValue::as_u64) != Some(expected_count)
        || index.get("total_bytes").and_then(JsonValue::as_u64) != Some(expected_total_bytes)
    {
        return Err(RunnerError::InvalidRequest(format!(
            "downloaded pack `{pack_id}` index metadata differs from import manifest"
        )));
    }
    Ok(())
}

fn validate_object_entry(index: &JsonValue, locator: &BlobLocatorRow) -> Result<(), RunnerError> {
    let entry = index
        .get("entries")
        .and_then(JsonValue::as_array)
        .and_then(|entries| {
            entries.iter().find(|entry| {
                entry.get("entry_name").and_then(JsonValue::as_str)
                    == Some(locator.pack_entry_name.as_str())
            })
        })
        .ok_or_else(|| {
            RunnerError::InvalidRequest(format!(
                "object pack `{}` index does not contain entry `{}`",
                locator.pack_id, locator.pack_entry_name
            ))
        })?;
    if entry.get("blob_id").and_then(JsonValue::as_str) != Some(locator.blob_id.as_str())
        || entry.get("entry_type").and_then(JsonValue::as_str)
            != Some(locator.pack_entry_type.as_str())
        || entry.get("chain_depth").and_then(JsonValue::as_u64)
            != Some(locator.pack_chain_depth as u64)
    {
        return Err(RunnerError::InvalidRequest(format!(
            "blob `{}` locator differs from downloaded object pack index",
            locator.blob_id
        )));
    }
    let indexed_base = entry.get("base_blob_id").and_then(JsonValue::as_str);
    if indexed_base != locator.pack_base_blob_id.as_deref() {
        return Err(RunnerError::InvalidRequest(format!(
            "blob `{}` base locator differs from downloaded object pack index",
            locator.blob_id
        )));
    }
    Ok(())
}

fn validate_blob_locator(locator: &BlobLocatorRow) -> Result<(), RunnerError> {
    validate_sha256(&locator.sha256, "blob locator sha256")?;
    if locator.pack_chain_depth > MAX_DELTA_CHAIN_READ_DEPTH {
        return Err(RunnerError::InvalidRequest(format!(
            "blob `{}` delta chain depth {} exceeds maximum {MAX_DELTA_CHAIN_READ_DEPTH}",
            locator.blob_id, locator.pack_chain_depth
        )));
    }
    match locator.pack_entry_type.as_str() {
        "full" if locator.pack_base_blob_id.is_none() && locator.pack_chain_depth == 0 => Ok(()),
        "delta" if locator.pack_base_blob_id.is_some() && locator.pack_chain_depth > 0 => Ok(()),
        _ => Err(RunnerError::InvalidRequest(format!(
            "blob `{}` has incoherent pack entry type/base/depth metadata",
            locator.blob_id
        ))),
    }
}

fn validate_tree_entry_name(name: &str) -> Result<(), RunnerError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 255
        || name.contains(['/', '\\', '\0'])
    {
        return Err(RunnerError::InvalidRequest(format!(
            "tree entry name `{name}` is not one confined path component"
        )));
    }
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<(), RunnerError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RunnerError::InvalidRequest(format!(
            "{label} must be exactly 64 hexadecimal characters"
        )));
    }
    Ok(())
}

fn required_text(value: &JsonValue, field: &str, label: &str) -> Result<String, RunnerError> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            RunnerError::InvalidRequest(format!("{label} requires non-empty field `{field}`"))
        })
}

fn write_materialized_file(path: &Path, bytes: &[u8], mode: &str) -> Result<(), RunnerError> {
    let permissions = parse_file_permissions(mode)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| RunnerError::fs("create materialized Snapshot file", path, error))?;
    file.write_all(bytes)
        .map_err(|error| RunnerError::fs("write materialized Snapshot file", path, error))?;
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(permissions))
            .map_err(|error| RunnerError::fs("set Snapshot file permissions", path, error))?;
    }
    #[cfg(not(unix))]
    {
        let _ = permissions;
    }
    Ok(())
}

fn parse_file_permissions(mode: &str) -> Result<u32, RunnerError> {
    let digits = mode.strip_prefix("0o").ok_or_else(|| {
        RunnerError::InvalidRequest(format!("blob materialization uses malformed mode `{mode}`"))
    })?;
    if digits.len() != 3 || !digits.bytes().all(|byte| matches!(byte, b'0'..=b'7')) {
        return Err(RunnerError::InvalidRequest(format!(
            "blob materialization uses unsupported mode `{mode}`"
        )));
    }
    u32::from_str_radix(digits, 8).map_err(|_| {
        RunnerError::InvalidRequest(format!(
            "blob materialization uses unsupported mode `{mode}`"
        ))
    })
}

fn ensure_empty_directory(path: &Path, label: &str) -> Result<(), RunnerError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| RunnerError::fs("inspect materialization directory", path, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(RunnerError::InvalidRequest(format!(
            "{label} must be a regular directory"
        )));
    }
    if fs::read_dir(path)
        .map_err(|error| RunnerError::fs("read materialization directory", path, error))?
        .next()
        .is_some()
    {
        return Err(RunnerError::InvalidRequest(format!(
            "{label} must be empty before materialization"
        )));
    }
    Ok(())
}

fn enforce_materialization_bounds(stats: &MaterializationStats) -> Result<(), RunnerError> {
    if stats.file_count > MAX_MATERIALIZED_FILES
        || stats.entry_count > MAX_MATERIALIZED_FILES.saturating_mul(2)
        || stats.total_bytes > MAX_MATERIALIZED_BYTES
    {
        return Err(RunnerError::InvalidRequest(format!(
            "materialization exceeds runner bounds: {} files, {} entries, {} bytes",
            stats.file_count, stats.entry_count, stats.total_bytes
        )));
    }
    Ok(())
}

fn pack_download_limit(total_bytes: u64) -> u64 {
    total_bytes
        .saturating_mul(2)
        .saturating_add(PACK_DOWNLOAD_ALLOWANCE_BYTES)
        .min(MAX_PACK_DOWNLOAD_BYTES)
}

fn stable_path_digest(value: &str) -> String {
    sha256_hex(value.as_bytes())[..24].to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn path_text(path: &Path) -> Result<&str, RunnerError> {
    path.to_str().ok_or_else(|| {
        RunnerError::InvalidRequest(format!(
            "runner materialization path `{}` is not valid UTF-8",
            path.display()
        ))
    })
}

fn invalid_pack(operation: &str, pack_id: &str, error: String) -> RunnerError {
    RunnerError::InvalidRequest(format!(
        "{operation} failed for remote pack `{pack_id}`: {error}"
    ))
}

fn external_environment_key(name: &str) -> Result<String, RunnerError> {
    let name = name.strip_prefix("ait-").unwrap_or(name);
    let normalized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    if normalized.is_empty() || normalized.bytes().all(|byte| byte == b'_') {
        return Err(RunnerError::InvalidRequest(format!(
            "external name `{name}` cannot form a portable environment key"
        )));
    }
    Ok(format!("AIT_EXTERNAL_{normalized}_REPO_ROOT"))
}

fn runner_external(error: RunnerError) -> ExternalError {
    ExternalError::with_code("runner_remote_materialization", error.to_string())
}

fn external_materialization_error(error: ExternalError) -> RunnerError {
    RunnerError::InvalidRequest(format!(
        "remote external dependency materialization failed: {error}"
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ait_core::pack_substrate::{
        build_tree_pack_members, read_tree_pack_index_with_format, tree_pack_checksums_by_tree_id,
        write_pack_archive_with_format, write_tree_pack_archive_with_format,
    };
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::executor::{ExecutorConfig, NativeExecutor};
    use crate::protocol::{
        CommandSpec, NATIVE_JOB_CONTRACT, NativeJobRequest, SourceSpec, TerminalStatus,
    };

    fn external_lock_node(repo_name: &str, repository_index: u32) -> ExternalLockNode {
        ExternalLockNode {
            name: format!("dependency-{repository_index}"),
            repo_name: repo_name.to_string(),
            repository_index,
            remote: "origin".to_string(),
            line: "main".to_string(),
            snapshot: format!("SNP-{repository_index:012X}"),
            parent_path: String::new(),
            materialize_to: format!(".ait-external/dependency-{repository_index}"),
            license: "Apache-2.0".to_string(),
            version: None,
            bindings: Vec::new(),
        }
    }

    #[test]
    fn external_snapshot_routing_uses_each_lock_nodes_numeric_identity() {
        let first =
            declared_external_snapshot_reference(&external_lock_node("duplicate-display-name", 11));
        let second =
            declared_external_snapshot_reference(&external_lock_node("duplicate-display-name", 12));

        assert_eq!(first.repository_index, Some(RepositoryIndex::new(11)));
        assert_eq!(second.repository_index, Some(RepositoryIndex::new(12)));
        assert_eq!(first.repository_name, second.repository_name);
        assert!(first.external_repository_indexes.is_empty());
    }

    struct FixtureProvider {
        manifest: Vec<u8>,
        object_pack: PathBuf,
        tree_pack: PathBuf,
        calls: Mutex<Vec<(RemotePackKind, String)>>,
    }

    impl RemoteSnapshotProvider for FixtureProvider {
        fn fetch_import_manifest(
            &self,
            _source: &RemoteSnapshotReference,
        ) -> Result<Vec<u8>, RunnerError> {
            Ok(self.manifest.clone())
        }

        fn download_pack(
            &self,
            _source: &RemoteSnapshotReference,
            kind: RemotePackKind,
            pack_id: &str,
            destination: &Path,
            maximum_bytes: u64,
        ) -> Result<u64, RunnerError> {
            let source = match kind {
                RemotePackKind::Object => &self.object_pack,
                RemotePackKind::Tree => &self.tree_pack,
            };
            let bytes = fs::read(source)
                .map_err(|error| RunnerError::fs("read fixture pack", source, error))?;
            if bytes.len() as u64 > maximum_bytes {
                return Err(RunnerError::Server(
                    "fixture exceeds download bound".to_string(),
                ));
            }
            fs::write(destination, &bytes)
                .map_err(|error| RunnerError::fs("write fixture pack", destination, error))?;
            self.calls
                .lock()
                .expect("fixture calls")
                .push((kind, pack_id.to_string()));
            Ok(bytes.len() as u64)
        }
    }

    fn executable_snapshot_fixture(packs: &TempDir, entry_name: &str) -> FixtureProvider {
        let script =
            b"#!/bin/sh\nset -eu\ntest -n \"$AIT_RUNNER_ATTEMPT_ROOT\"\nprintf remote-ok\n";
        let blob_sha = sha256_hex(script);
        let blob_id = format!("BLB-{}", &blob_sha[..20]);
        let object_pack_id = "PCK-RUNNER-TEST";
        let tree_pack_id = "TPK-RUNNER-TEST";
        let child_tree_id = "TRE-RUNNER-CI";
        let root_tree_id = "TRE-RUNNER-ROOT";
        let object_pack = packs.path().join("object.zstpack");
        let tree_pack = packs.path().join("tree.zstpack");
        let object_members = json!([{
            "entry_name": format!("blobs/{blob_id}"),
            "blob_id": blob_id,
            "data": script.as_slice(),
            "entry_type": "full",
            "chain_depth": 0
        }]);
        let object_stats = write_pack_archive_with_format(
            path_text(&object_pack).unwrap(),
            object_pack_id,
            "1970-01-01T00:00:00Z",
            &object_members,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .expect("object pack");
        let tree_members = build_tree_pack_members(
            &json!([
                {"tree_id": child_tree_id, "entry_count": 1},
                {"tree_id": root_tree_id, "entry_count": 1}
            ]),
            &json!([
                {
                    "tree_id": root_tree_id,
                    "entry_name": "ci",
                    "entry_type": "tree",
                    "target_id": child_tree_id,
                    "size_bytes": null,
                    "mode": "tree"
                },
                {
                    "tree_id": child_tree_id,
                    "entry_name": entry_name,
                    "entry_type": "blob",
                    "target_id": blob_id,
                    "size_bytes": script.len(),
                    "mode": "0o755"
                }
            ]),
        )
        .expect("tree members");
        let tree_stats = write_tree_pack_archive_with_format(
            path_text(&tree_pack).unwrap(),
            tree_pack_id,
            "1970-01-01T00:00:00Z",
            &tree_members,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .expect("tree pack");
        let tree_index = read_tree_pack_index_with_format(
            path_text(&tree_pack).unwrap(),
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .expect("tree index");
        let root_ordinal = tree_index["trees"]
            .as_array()
            .expect("trees")
            .iter()
            .find(|entry| entry["tree_id"] == root_tree_id)
            .and_then(|entry| entry["entry_ordinal"].as_u64())
            .expect("root ordinal");
        let tree_checksums = tree_pack_checksums_by_tree_id(&tree_stats).expect("tree checksums");
        let manifest = json!({
            "contract": IMPORT_MANIFEST_CONTRACT,
            "repo_name": "fixture",
            "snapshot_id": "SNP-FIXTURE",
            "snapshots": [{
                "snapshot_id": "SNP-FIXTURE",
                "root_tree_pack_id": tree_pack_id,
                "root_entry_ordinal": root_ordinal,
                "file_count": 1,
                "total_bytes": script.len()
            }],
            "object_packs": [{
                "pack_id": object_pack_id,
                "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
                "member_count": object_stats["member_count"],
                "total_bytes": object_stats["total_bytes"],
                "pack_index_checksum": object_stats["pack_index_checksum"]
            }],
            "tree_packs": [{
                "pack_id": tree_pack_id,
                "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                "tree_count": tree_stats["tree_count"],
                "total_bytes": tree_stats["total_bytes"],
                "pack_index_checksum": tree_stats["pack_index_checksum"]
            }],
            "blob_locators": [{
                "blob_id": blob_id,
                "sha256": blob_sha,
                "size_bytes": script.len(),
                "pack_id": object_pack_id,
                "pack_entry_name": format!("blobs/{blob_id}"),
                "pack_entry_type": "full",
                "pack_base_blob_id": null,
                "pack_chain_depth": 0
            }],
            "tree_locators": [
                {
                    "tree_id": child_tree_id,
                    "entry_count": 1,
                    "tree_pack_id": tree_pack_id,
                    "tree_pack_checksum": tree_checksums[child_tree_id]
                },
                {
                    "tree_id": root_tree_id,
                    "entry_count": 1,
                    "tree_pack_id": tree_pack_id,
                    "tree_pack_checksum": tree_checksums[root_tree_id]
                }
            ],
            "line_update": null
        });
        FixtureProvider {
            manifest: serde_json::to_vec(&manifest).expect("manifest"),
            object_pack,
            tree_pack,
            calls: Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn external_environment_names_follow_the_ait_core_contract() {
        assert_eq!(
            external_environment_key("ait-core").unwrap(),
            "AIT_EXTERNAL_CORE_REPO_ROOT"
        );
        assert_eq!(
            external_environment_key("company-sdk").unwrap(),
            "AIT_EXTERNAL_COMPANY_SDK_REPO_ROOT"
        );
    }

    #[test]
    fn tree_entries_and_pack_limits_are_bounded() {
        assert!(validate_tree_entry_name("../escape").is_err());
        assert!(validate_tree_entry_name("nested/file").is_err());
        assert!(validate_tree_entry_name("file.rs").is_ok());
        assert_eq!(pack_download_limit(u64::MAX), MAX_PACK_DOWNLOAD_BYTES);
    }

    #[test]
    fn snapshot_file_modes_accept_portable_permission_bits_only() {
        assert_eq!(parse_file_permissions("0o600").unwrap(), 0o600);
        assert_eq!(parse_file_permissions("0o644").unwrap(), 0o644);
        assert_eq!(parse_file_permissions("0o755").unwrap(), 0o755);
        assert!(parse_file_permissions("0o4755").is_err());
        assert!(parse_file_permissions("0755").is_err());
        assert!(parse_file_permissions("0o888").is_err());
    }

    #[test]
    fn remote_snapshot_executes_and_removes_every_attempt_owned_path() {
        let packs = tempfile::tempdir().expect("packs");
        let provider = executable_snapshot_fixture(&packs, "run.sh");
        let attempts = tempfile::tempdir().expect("attempts");
        let source_root = tempfile::tempdir().expect("unused local source");
        let executor = NativeExecutor::new(ExecutorConfig {
            source_root: source_root.path().to_path_buf(),
            attempt_root: attempts.path().to_path_buf(),
        });
        let request = NativeJobRequest {
            contract: NATIVE_JOB_CONTRACT.to_string(),
            label: None,
            source: SourceSpec::RemoteSnapshot {
                repository_index: RepositoryIndex::new(4),
                repository_name: "fixture".to_string(),
                snapshot_id: "SNP-FIXTURE".to_string(),
                external_repository_indexes: BTreeMap::new(),
            },
            command: CommandSpec {
                argv: vec!["./ci/run".to_string(), "patchset".to_string()],
                working_directory: ".".to_string(),
                environment: BTreeMap::new(),
            },
            timeout_ms: 5_000,
            suite_id: Some("fixture".to_string()),
        };
        let result = executor
            .execute_with_provider(&request, Some(&provider))
            .expect("remote execution");
        assert_eq!(result.status, TerminalStatus::Succeeded);
        assert_eq!(
            result.suite_results[0]
                .execution
                .materialization
                .source_kind,
            "remote_snapshot"
        );
        assert!(result.cleanup.attempt_root_removed);
        assert_eq!(
            fs::read_dir(attempts.path())
                .expect("attempt parent")
                .count(),
            0
        );
        assert_eq!(provider.calls.lock().expect("calls").len(), 2);
    }

    #[test]
    fn escaping_tree_entry_is_rejected_before_process_start_and_cleaned() {
        let packs = tempfile::tempdir().expect("packs");
        let provider = executable_snapshot_fixture(&packs, "../run.sh");
        let attempts = tempfile::tempdir().expect("attempts");
        let source_root = tempfile::tempdir().expect("unused local source");
        let executor = NativeExecutor::new(ExecutorConfig {
            source_root: source_root.path().to_path_buf(),
            attempt_root: attempts.path().to_path_buf(),
        });
        let request = NativeJobRequest {
            contract: NATIVE_JOB_CONTRACT.to_string(),
            label: None,
            source: SourceSpec::RemoteSnapshot {
                repository_index: RepositoryIndex::new(4),
                repository_name: "fixture".to_string(),
                snapshot_id: "SNP-FIXTURE".to_string(),
                external_repository_indexes: BTreeMap::new(),
            },
            command: CommandSpec {
                argv: vec!["./ci/run".to_string()],
                working_directory: ".".to_string(),
                environment: BTreeMap::new(),
            },
            timeout_ms: 5_000,
            suite_id: None,
        };
        let error = executor
            .execute_with_provider(&request, Some(&provider))
            .expect_err("escape must fail closed");
        assert!(error.to_string().contains("one confined path component"));
        assert_eq!(
            fs::read_dir(attempts.path())
                .expect("attempt parent")
                .count(),
            0
        );
    }
}
