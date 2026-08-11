use super::generation_activation::{
    admit_activated_binary_db_generation, binary_db_activation_lock_root, configured_repo_name,
    fingerprint_direct_authority_contents, validate_generation_file_path, CLIENT_MANIFEST_SCHEMA,
};
use super::generation_manifest::write_json;
use super::{deterministic_parallel_map, GenerationFileManifest, GenerationResult, Path, PathBuf};
use crate::binary_db::{
    AuthorityId, BinaryDbCommandScope, BinaryDbReadLockSet, LocalBinaryDbFs, LocalStateScope,
    REPOSITORY_BINARY_DB_BIN_PATHS, REPOSITORY_BINARY_DB_INDEX_PATHS,
};
use crate::content_binary_db::{
    object_pack_id_from_hash48, tree_id_from_hash80, tree_pack_id_from_hash48,
    BinaryDbContentWriteCoordinator, BinaryDbObjectPackMemberWriteInput,
    BinaryDbObjectPackWriteInput, BinaryDbTreePackTreeWriteInput, BinaryDbTreePackWriteInput,
    BinaryDbTreeReadCache, BinaryObjectPackMemberView, BinarySnapshotCodec, BinaryTreeCodec,
    BinaryTreeEntryView, BinaryTreePackCodec, BinaryTreePackView, LocalContentBinaryDb, BLOB_BIN,
    BLOB_ID_IDX, BLOB_RECORD_SIZE, OBJECT_PACK_BIN, OBJECT_PACK_ID_IDX, OBJECT_PACK_MEMBER_BIN,
    OBJECT_PACK_MEMBER_RECORD_SIZE, OBJECT_PACK_RECORD_SIZE, SNAPSHOT_BIN, SNAPSHOT_RECORD_SIZE,
    TREE_BIN, TREE_ID_IDX, TREE_PACK_BIN, TREE_PACK_ID_IDX, TREE_PACK_RECORD_SIZE,
    TREE_RECORD_SIZE,
};
use crate::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};
use crate::pack_substrate::{
    build_tree_pack_members, default_object_pack_relative_path, default_tree_pack_relative_path,
    write_tree_pack_archive_with_format, write_typed_pack_archive_with_format,
    ObjectPackWriteMember, CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT, PACK_FORMAT_ZSTD_CHUNKED_V1,
    TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use crate::plan_binary_db::{
    BinaryDbPlanStore, PlanRevisionCodec, PLAN_REVISION_BIN, PLAN_REVISION_RECORD_SIZE,
};
use crate::workflow_binary_db::{
    CHANGE_LAND_INDEX_BIN, CHANGE_LAND_INDEX_RECORD_SIZE, CHANGE_RECORD_BIN, LAND_RECORD_BIN,
    LOCAL_CHANGE_RECORD_SIZE, LOCAL_LAND_RECORD_SIZE, LOCAL_TASK_RECORD_SIZE,
    TASK_CHANGE_INDEX_BIN, TASK_CHANGE_INDEX_RECORD_SIZE, TASK_LAND_INDEX_BIN,
    TASK_LAND_INDEX_RECORD_SIZE, TASK_RECORD_BIN,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::{Read, Write};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CaptureBinaryDbGenerationOptions {
    pub repo_root: PathBuf,
    pub output_root: PathBuf,
    pub jobs: usize,
}

impl CaptureBinaryDbGenerationOptions {
    pub fn validate(self) -> GenerationResult<Self> {
        if self.jobs == 0 {
            return Err("--jobs must be at least 1".to_string());
        }
        if self.output_root.exists() {
            return Err(format!(
                "output root must not already exist: {}",
                self.output_root.display()
            ));
        }
        if self.output_root.file_name().is_none() {
            return Err(format!(
                "output root must have a final path component: {}",
                self.output_root.display()
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CaptureBinaryDbGenerationReport {
    #[serde(skip)]
    pub output_root: PathBuf,
    pub repo_name: String,
    pub worker_count: usize,
    pub source_authority_fingerprint: String,
    pub content_fingerprint: String,
    pub file_count: usize,
}

#[derive(Serialize)]
struct CapturedClientManifest {
    schema: String,
    label: String,
    source_repo_root: String,
    source_authority_root: String,
    source_authority_fingerprint: String,
    source_snapshot_parent_representation: String,
    worker_count: usize,
    layout_ids: BTreeMap<String, u32>,
    content_fingerprint: String,
    files: Vec<GenerationFileManifest>,
    content_projection: CapturedContentProjection,
    validation: CapturedValidation,
}

#[derive(Serialize)]
struct CapturedValidation {
    status: String,
    checks: Vec<String>,
    table_record_counts: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct CapturedContentProjection {
    source_blob_records: u64,
    retained_blob_records: u64,
    source_object_pack_records: u64,
    retained_object_pack_records: u64,
    source_object_pack_member_records: u64,
    retained_object_pack_member_records: u64,
    source_tree_pack_records: u64,
    retained_tree_pack_records: u64,
    source_tree_records: u64,
    retained_tree_records: u64,
    source_retained: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct ReachableContentPlan {
    output_blob_indices: BTreeSet<u32>,
    retained_blob_indices: BTreeSet<u32>,
    retained_object_pack_indices: BTreeSet<u32>,
    retained_object_pack_member_indices: BTreeSet<u32>,
    retained_tree_pack_indices: BTreeSet<u32>,
    retained_tree_indices: BTreeSet<u32>,
    rootless_plan_revision_indices: BTreeSet<u32>,
    source_blob_count: u32,
    source_object_pack_count: u32,
    source_object_pack_member_count: u32,
    source_tree_pack_count: u32,
    source_tree_count: u32,
}

#[derive(Clone, Debug)]
struct CanonicalBlobSource {
    source_blob_index: u32,
    source_pack_index: u32,
    member: BinaryObjectPackMemberView,
    record: crate::content_binary_db::BinaryBlobRecord,
}

#[derive(Clone, Debug)]
struct CanonicalTreeSource {
    source_tree_index: u32,
    source_pack_index: u32,
    tree_id: String,
    record: crate::content_binary_db::BinaryTreeRecord,
    entries: Vec<BinaryTreeEntryView>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RepackedTreeLocator {
    tree_pack_index: u32,
    pack_entry_ordinal: u32,
}

#[derive(Clone, Debug)]
struct CopyInput {
    source: PathBuf,
    relative_path: String,
    strategy: CopyStrategy,
}

#[derive(Clone, Debug)]
enum CopyStrategy {
    Exact,
    CanonicalTreeLocators { tree_pack_source: PathBuf },
}

fn capture_db(
    authority_root: PathBuf,
    repo_root: PathBuf,
    authority_id: String,
) -> LocalBinaryDbFs {
    LocalBinaryDbFs::new(
        authority_root,
        repo_root,
        AuthorityId::new(authority_id),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS)
}

pub fn capture_binary_db_generation(
    options: CaptureBinaryDbGenerationOptions,
) -> GenerationResult<CaptureBinaryDbGenerationReport> {
    let options = options.validate()?;
    let repo_root = options.repo_root.canonicalize().map_err(|error| {
        format!(
            "invalid repository root {}: {error}",
            options.repo_root.display()
        )
    })?;
    let repo_name = configured_repo_name(&repo_root)?;
    let _activation_guard =
        BinaryDbReadLockSet::try_acquire(&binary_db_activation_lock_root(&repo_root))
            .map_err(|error| format!("Binary DB generation activation is active: {error}"))?;
    let pointer = repo_root.join(".ait/binary-db");
    let pointer_metadata = fs::symlink_metadata(&pointer).map_err(|error| {
        format!(
            "repository has no Binary DB authority at {}: {error}",
            pointer.display()
        )
    })?;
    let (authority_root, pack_root) = if pointer_metadata.file_type().is_symlink() {
        let generation = admit_activated_binary_db_generation(&pointer, &repo_name)?;
        (generation.authority_root, generation.generation_root)
    } else if pointer_metadata.file_type().is_dir() {
        (
            pointer.canonicalize().map_err(|error| error.to_string())?,
            repo_root.clone(),
        )
    } else {
        return Err(format!(
            "Binary DB authority {} is neither a directory nor an activation pointer",
            pointer.display()
        ));
    };

    let db = capture_db(
        authority_root.clone(),
        repo_root.clone(),
        format!("capture:{repo_name}"),
    );
    let read = db.begin_read_txn();
    read.read_lock_paths().map_err(|error| {
        format!("cannot capture Binary DB generation while a writer is active: {error}")
    })?;
    let content = LocalContentBinaryDb::<1>::from_db_with_roots(
        db.clone(),
        repo_root.clone(),
        pack_root.clone(),
    );
    let inputs = collect_copy_inputs(&authority_root)?;
    let source_authority_fingerprint = fingerprint_direct_authority_contents(&authority_root)?;
    let source_snapshots = read_canonical_source_snapshot_records(&authority_root)?;
    let reachable_content =
        plan_reachable_content(&authority_root, &content, &read, &source_snapshots)?;

    let output_parent = options
        .output_root
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_parent).map_err(|error| {
        format!(
            "failed to create output parent {}: {error}",
            output_parent.display()
        )
    })?;
    let output_name = options
        .output_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "output root name is not UTF-8".to_string())?;
    let staging_root = output_parent.join(format!(
        ".{output_name}.capture-staging-{}",
        std::process::id()
    ));
    if staging_root.exists() {
        return Err(format!(
            "capture staging root already exists: {}",
            staging_root.display()
        ));
    }
    fs::create_dir_all(staging_root.join("local"))
        .and_then(|_| fs::create_dir_all(staging_root.join(".ait/objects")))
        .map_err(|error| {
            format!(
                "failed to create capture staging root {}: {error}",
                staging_root.display()
            )
        })?;

    let copied = deterministic_parallel_map(&inputs, options.jobs, |_index, input| {
        copy_and_hash(input, &staging_root)
    });
    let mut files = match copied {
        Ok(files) => files,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(error);
        }
    };
    let content_projection = match repack_reachable_content(
        &content,
        &read,
        &staging_root,
        &repo_name,
        &reachable_content,
        &mut files,
    ) {
        Ok(projection) => projection,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(error);
        }
    };
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    if let Err(error) = validate_captured_tree_locators(&staging_root, &repo_name, options.jobs) {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(error);
    }
    let content_fingerprint = fingerprint_files(&files);
    let table_record_counts = record_counts(&files)?;
    let file_count = files.len();
    let manifest = CapturedClientManifest {
        schema: CLIENT_MANIFEST_SCHEMA.to_string(),
        label: repo_name.clone(),
        source_repo_root: path_text(&repo_root)?,
        source_authority_root: path_text(&authority_root)?,
        source_authority_fingerprint: source_authority_fingerprint.clone(),
        source_snapshot_parent_representation: "canonical".to_string(),
        worker_count: options.jobs,
        layout_ids: required_layout_ids(),
        content_fingerprint: content_fingerprint.clone(),
        files,
        content_projection,
        validation: CapturedValidation {
            status: "passed".to_string(),
            checks: capture_validation_checks(),
            table_record_counts,
        },
    };
    if let Err(error) = write_json(&staging_root.join("client-manifest.json"), &manifest) {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(error);
    }
    sync_directory(&staging_root.join("local"))?;
    sync_directory(&staging_root.join(".ait/objects"))?;
    sync_directory(&staging_root)?;
    drop(read);

    if options.output_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(format!(
            "output root appeared before capture publication: {}",
            options.output_root.display()
        ));
    }
    fs::rename(&staging_root, &options.output_root).map_err(|error| {
        let _ = fs::remove_dir_all(&staging_root);
        format!(
            "failed to atomically publish captured generation {}: {error}",
            options.output_root.display()
        )
    })?;
    sync_directory(output_parent)?;
    Ok(CaptureBinaryDbGenerationReport {
        output_root: options.output_root,
        repo_name,
        worker_count: options.jobs,
        source_authority_fingerprint,
        content_fingerprint,
        file_count,
    })
}

fn capture_validation_checks() -> Vec<String> {
    let mut checks = vec![
        "all-domain Binary DB read lock held through capture".to_string(),
        "source authority content fingerprint recorded while the all-domain read lock was held"
            .to_string(),
    ];
    checks.extend([
        "authority output matches the exact repository Binary DB schema registry".to_string(),
        "undeclared and retired storage files were rejected before content planning".to_string(),
        "tree.bin locators were canonicalized from declared tree-pack ranges and verified against pack payloads".to_string(),
        "source Snapshot records decoded directly as the canonical typed DAG schema".to_string(),
        "explicit Task and Change authorities were copied only through the declared repository schema".to_string(),
        "Snapshot-root-reachable content was rebuilt into dense live-only object/tree packs and metadata; Snapshot root locators were remapped and disconnected source rows were omitted".to_string(),
        "copy used bounded streaming buffers".to_string(),
    ]);
    checks
}

fn write_captured_bytes(path: &Path, bytes: &[u8]) -> GenerationResult<()> {
    let mut file = fs::OpenOptions::new()
        .create_new(!path.exists())
        .truncate(path.exists())
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to open captured file {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to write captured file {}: {error}", path.display()))?;
    file.sync_data()
        .map_err(|error| format!("failed to sync captured file {}: {error}", path.display()))
}

fn refresh_captured_file_manifest(
    files: &mut Vec<GenerationFileManifest>,
    staging_root: &Path,
    relative_path: &str,
) -> GenerationResult<()> {
    let path = staging_root.join(relative_path);
    let byte_size = fs::metadata(&path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
        .len();
    let entry = GenerationFileManifest {
        relative_path: relative_path.to_string(),
        byte_size,
        sha256: sha256_file(&path)?,
        record_count: fixed_record_count(relative_path, byte_size)?,
    };
    if let Some(existing) = files
        .iter_mut()
        .find(|existing| existing.relative_path == relative_path)
    {
        *existing = entry;
    } else {
        files.push(entry);
    }
    Ok(())
}

fn plan_reachable_content(
    authority_root: &Path,
    content: &LocalContentBinaryDb<1>,
    read: &crate::binary_db::BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    source_snapshots: &[crate::content_binary_db::BinarySnapshotRecord],
) -> GenerationResult<ReachableContentPlan> {
    let mut plan = ReachableContentPlan {
        source_blob_count: source_record_count(authority_root, BLOB_BIN, BLOB_RECORD_SIZE)?,
        source_object_pack_count: source_record_count(
            authority_root,
            OBJECT_PACK_BIN,
            OBJECT_PACK_RECORD_SIZE,
        )?,
        source_object_pack_member_count: source_record_count(
            authority_root,
            OBJECT_PACK_MEMBER_BIN,
            OBJECT_PACK_MEMBER_RECORD_SIZE,
        )?,
        source_tree_pack_count: source_record_count(
            authority_root,
            TREE_PACK_BIN,
            TREE_PACK_RECORD_SIZE,
        )?,
        source_tree_count: source_record_count(authority_root, TREE_BIN, TREE_RECORD_SIZE)?,
        ..ReachableContentPlan::default()
    };

    let tree_pack_views = content
        .tree_packs()
        .list_tree_pack_views(read)
        .map_err(|error| format!("cannot inventory live tree packs for capture: {error}"))?;
    let tree_pack_position_by_index = tree_pack_views
        .iter()
        .enumerate()
        .map(|(position, pack)| (pack.tree_pack_index, position))
        .collect::<BTreeMap<_, _>>();
    let mut tree_pack_index_by_tree = vec![None; plan.source_tree_count as usize];
    for pack in &tree_pack_views {
        let end = pack
            .record
            .first_tree_index
            .checked_add(pack.record.tree_count)
            .ok_or_else(|| format!("tree pack {} range overflows u32", pack.pack_id))?;
        if end > plan.source_tree_count {
            return Err(format!(
                "tree pack {} range [{}..{end}) exceeds {} tree records",
                pack.pack_id, pack.record.first_tree_index, plan.source_tree_count
            ));
        }
        for tree_index in pack.record.first_tree_index..end {
            let slot = &mut tree_pack_index_by_tree[tree_index as usize];
            if let Some(existing) = slot.replace(pack.tree_pack_index) {
                return Err(format!(
                    "tree record {tree_index} is claimed by live tree packs {existing} and {}",
                    pack.tree_pack_index
                ));
            }
        }
    }

    let (tree_records, active_tree_index_by_id) = {
        let mut records = Vec::with_capacity(plan.source_tree_count as usize);
        let mut active_index_by_id = BTreeMap::new();
        for tree_index in 0..plan.source_tree_count {
            let record = content
                .trees()
                .read_tree_record(read, tree_index)
                .map_err(|error| {
                    format!("cannot inventory tree record {tree_index} for capture: {error}")
                })?;
            if !record.is_tombstone() {
                // Tree-ID indexes are append-only candidate lists. Overwriting the map
                // entry while scanning ordinals preserves the normal newest-live-candidate
                // lookup rule without materializing a tree view or rescanning pack ranges.
                active_index_by_id.insert(
                    tree_id_from_hash80(&record.tree_hash80).to_ascii_lowercase(),
                    tree_index,
                );
            }
            records.push(record);
        }
        (records, active_index_by_id)
    };

    let mut tree_queue = VecDeque::new();
    let mut reachable_blob_ids = BTreeSet::new();
    for (snapshot_index, snapshot) in source_snapshots.iter().enumerate() {
        if snapshot.is_tombstone() {
            continue;
        }
        let Some(tree_pack_index) = snapshot.root_tree_pack_index() else {
            continue;
        };
        let pack_position = tree_pack_position_by_index
            .get(&tree_pack_index)
            .copied()
            .ok_or_else(|| {
                format!(
                    "live Snapshot {snapshot_index} references unavailable tree pack {tree_pack_index}"
                )
            })?;
        let pack = &tree_pack_views[pack_position];
        let root_tree_index =
            source_tree_index_for_pack_ordinal(pack, snapshot.root_entry_ordinal, &tree_records)
                .map_err(|error| {
                    format!("live Snapshot {snapshot_index} root is invalid: {error}")
                })?;
        let root_tree = tree_records.get(root_tree_index as usize).ok_or_else(|| {
            format!("live Snapshot {snapshot_index} references missing root tree {root_tree_index}")
        })?;
        if root_tree.is_tombstone() {
            return Err(format!(
                "live Snapshot {snapshot_index} references tombstoned root tree {root_tree_index}"
            ));
        }
        tree_queue.push_back(root_tree_index);
    }

    let plans = BinaryDbPlanStore::<_, 1>::new(content.db().clone());
    let plan_revision_count = read
        .record_count(BinaryDbPlanStore::<LocalBinaryDbFs, 1>::plan_revision_file())
        .map_err(|error| format!("cannot inventory Plan revisions for capture: {error}"))?;
    for revision_index in 0..plan_revision_count {
        let (revision, payload) = plans
            .read_current_plan_revision(read, revision_index)
            .map_err(|error| {
                format!(
                    "cannot read Plan revision {revision_index} artifact while planning capture: {error}"
                )
            })?;
        if revision.is_tombstone() {
            continue;
        }
        let artifact_blob_id = payload.artifact_blob_id_text().map_err(|error| {
            format!("cannot decode Plan revision {revision_index} artifact Blob identity: {error}")
        })?;
        let has_artifact_blob = !artifact_blob_id.is_empty();
        if has_artifact_blob {
            reachable_blob_ids.insert(artifact_blob_id);
        }
        let Some(tree_pack_index) = revision.root_tree_pack_index() else {
            if has_artifact_blob {
                return Err(format!(
                    "live Plan revision {revision_index} with an artifact Blob has no repo-root Tree locator"
                ));
            }
            if revision.root_entry_ordinal != 0 {
                return Err(format!(
                    "rootless live Plan revision {revision_index} has a nonzero root Tree ordinal"
                ));
            }
            plan.rootless_plan_revision_indices.insert(revision_index);
            continue;
        };
        let pack_position = tree_pack_position_by_index
            .get(&tree_pack_index)
            .copied()
            .ok_or_else(|| {
                format!(
                    "live Plan revision {revision_index} references unavailable tree pack {tree_pack_index}"
                )
            })?;
        let pack = &tree_pack_views[pack_position];
        let root_tree_index =
            source_tree_index_for_pack_ordinal(pack, revision.root_entry_ordinal, &tree_records)
                .map_err(|error| {
                    format!("live Plan revision {revision_index} root is invalid: {error}")
                })?;
        tree_queue.push_back(root_tree_index);
    }

    let mut tree_cache = BinaryDbTreeReadCache::default();
    {
        while let Some(tree_index) = tree_queue.pop_front() {
            if !plan.retained_tree_indices.insert(tree_index) {
                continue;
            }
            let tree_pack_index = tree_pack_index_by_tree
                .get(tree_index as usize)
                .and_then(|value| *value)
                .ok_or_else(|| {
                    format!("reachable tree {tree_index} has no live tree-pack range")
                })?;
            plan.retained_tree_pack_indices.insert(tree_pack_index);
            let pack_position = tree_pack_position_by_index
                .get(&tree_pack_index)
                .copied()
                .ok_or_else(|| format!("reachable tree pack {tree_pack_index} is unavailable"))?;
            let pack = &tree_pack_views[pack_position];
            let mut tree = tree_records
                .get(tree_index as usize)
                .cloned()
                .ok_or_else(|| format!("reachable tree {tree_index} is unavailable"))?;
            if tree.is_tombstone() {
                return Err(format!(
                    "reachable tree {tree_index} in pack {} is tombstoned",
                    pack.pack_id
                ));
            }
            if !pack.record.has_sparse_physical_ordinals() {
                tree.pack_entry_ordinal = tree_index
                    .checked_sub(pack.record.first_tree_index)
                    .ok_or_else(|| format!("tree {tree_index} precedes pack {}", pack.pack_id))?;
            }
            let entries = content
                .trees()
                .list_tree_entry_views_for_record_in_pack_with_cache(
                    read,
                    tree_index,
                    &tree,
                    pack,
                    &mut tree_cache,
                )
                .map_err(|error| {
                    format!(
                        "cannot read reachable tree {tree_index} in pack {}: {error}",
                        pack.pack_id
                    )
                })?;
            for entry in entries {
                match entry.entry_type.as_str() {
                    "blob" => {
                        reachable_blob_ids.insert(entry.target_id);
                    }
                    "tree" => {
                        let child_tree_index = active_tree_index_by_id
                            .get(&entry.target_id.to_ascii_lowercase())
                            .copied()
                            .ok_or_else(|| {
                                format!(
                                    "reachable tree {tree_index} entry {:?} references missing active tree {}",
                                    entry.entry_name, entry.target_id
                                )
                            })?;
                        tree_queue.push_back(child_tree_index);
                    }
                    other => {
                        return Err(format!(
                            "reachable tree {tree_index} entry {:?} has unsupported type {other:?}",
                            entry.entry_name
                        ));
                    }
                }
            }
        }
    }

    let object_pack_views = content
        .object_packs()
        .list_object_pack_views(read)
        .map_err(|error| format!("cannot inventory live object packs for capture: {error}"))?;
    let object_pack_position_by_index = object_pack_views
        .iter()
        .enumerate()
        .map(|(position, pack)| (pack.pack_index, position))
        .collect::<BTreeMap<_, _>>();
    let mut blob_queue = VecDeque::new();
    for blob_id in reachable_blob_ids {
        let blob = content
            .blobs()
            .get_blob_view(read, &blob_id)
            .map_err(|error| format!("cannot resolve reachable blob {blob_id}: {error}"))?
            .ok_or_else(|| format!("reachable tree references missing active blob {blob_id}"))?;
        if blob.record.is_pruned() {
            return Err(format!("reachable blob {blob_id} is pruned"));
        }
        let member_index = blob
            .pack_member_index
            .ok_or_else(|| format!("reachable blob {blob_id} has no object-pack member pointer"))?;
        let member = content
            .object_packs()
            .object_pack_member_view_at(read, member_index)
            .map_err(|error| {
                format!("cannot resolve member {member_index} for blob {blob_id}: {error}")
            })?;
        if member.record.is_tombstone() || member.record.blob_index != blob.blob_index {
            return Err(format!(
                "reachable blob {blob_id} and object-pack member {member_index} are not live backreferences"
            ));
        }
        plan.output_blob_indices.insert(blob.blob_index);
        blob_queue.push_back(blob.blob_index);
    }

    while let Some(blob_index) = blob_queue.pop_front() {
        if !plan.retained_blob_indices.insert(blob_index) {
            continue;
        }
        let blob = content
            .blobs()
            .blob_view_at(read, blob_index)
            .map_err(|error| format!("cannot read reachable blob {blob_index}: {error}"))?;
        if blob.record.is_tombstone() || blob.record.is_pruned() {
            return Err(format!("reachable blob record {blob_index} is unavailable"));
        }
        let member_index = blob.pack_member_index.ok_or_else(|| {
            format!("reachable blob record {blob_index} has no object-pack member pointer")
        })?;
        let member = content
            .object_packs()
            .object_pack_member_view_at(read, member_index)
            .map_err(|error| {
                format!("cannot read member {member_index} for blob {blob_index}: {error}")
            })?;
        if member.record.is_tombstone() || member.record.blob_index != blob_index {
            return Err(format!(
                "reachable blob {blob_index} and member {member_index} are not live backreferences"
            ));
        }
        let pack_index = member.record.pack_index;
        let pack_position = object_pack_position_by_index
            .get(&pack_index)
            .copied()
            .ok_or_else(|| format!("reachable object pack {pack_index} is unavailable"))?;
        let pack = &object_pack_views[pack_position];
        let member_end = pack
            .record
            .first_member_index
            .checked_add(pack.record.member_count)
            .ok_or_else(|| format!("object pack {} member range overflows u32", pack.pack_id))?;
        if member_end > plan.source_object_pack_member_count {
            return Err(format!(
                "object pack {} member range [{}..{member_end}) exceeds {} member records",
                pack.pack_id, pack.record.first_member_index, plan.source_object_pack_member_count
            ));
        }
        if member_index < pack.record.first_member_index || member_index >= member_end {
            return Err(format!(
                "reachable member {member_index} is outside object pack {} range",
                pack.pack_id
            ));
        }
        plan.retained_object_pack_indices.insert(pack_index);
        plan.retained_object_pack_member_indices
            .insert(member_index);
        if let Some(base_blob_index) = member.record.base_blob_index() {
            blob_queue.push_back(base_blob_index);
        }
    }

    Ok(plan)
}

fn source_tree_index_for_pack_ordinal(
    pack: &BinaryTreePackView,
    pack_entry_ordinal: u32,
    tree_records: &[crate::content_binary_db::BinaryTreeRecord],
) -> GenerationResult<u32> {
    if !pack.record.has_sparse_physical_ordinals() {
        if pack_entry_ordinal >= pack.record.tree_count {
            return Err(format!(
                "ordinal {pack_entry_ordinal} is outside tree pack {} count {}",
                pack.pack_id, pack.record.tree_count
            ));
        }
        return pack
            .record
            .first_tree_index
            .checked_add(pack_entry_ordinal)
            .ok_or_else(|| format!("tree pack {} root index overflows u32", pack.pack_id));
    }

    let end = pack
        .record
        .first_tree_index
        .checked_add(pack.record.tree_count)
        .ok_or_else(|| format!("tree pack {} range overflows u32", pack.pack_id))?;
    let mut match_index = None;
    for tree_index in pack.record.first_tree_index..end {
        let tree = tree_records.get(tree_index as usize).ok_or_else(|| {
            format!(
                "tree pack {} references missing tree {tree_index}",
                pack.pack_id
            )
        })?;
        if tree.pack_entry_ordinal != pack_entry_ordinal {
            continue;
        }
        if let Some(existing) = match_index.replace(tree_index) {
            return Err(format!(
                "tree pack {} physical ordinal {pack_entry_ordinal} is shared by trees {existing} and {tree_index}",
                pack.pack_id
            ));
        }
    }
    match_index.ok_or_else(|| {
        format!(
            "tree pack {} has no tree at physical ordinal {pack_entry_ordinal}",
            pack.pack_id
        )
    })
}

fn read_canonical_source_snapshot_records(
    authority_root: &Path,
) -> GenerationResult<Vec<crate::content_binary_db::BinarySnapshotRecord>> {
    let path = authority_root.join(SNAPSHOT_BIN);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if bytes.get(..4) != Some(1_u32.to_le_bytes().as_slice()) {
        return Err(format!(
            "Snapshot reachability planning requires canonical layout-1 {}",
            path.display()
        ));
    }
    let body = &bytes[4..];
    if body.len() % SNAPSHOT_RECORD_SIZE as usize != 0 {
        return Err(format!(
            "Snapshot reachability planning found misaligned {}",
            path.display()
        ));
    }
    body.chunks_exact(SNAPSHOT_RECORD_SIZE as usize)
        .enumerate()
        .map(|(index, raw)| {
            BinarySnapshotCodec::<1>::decode_record(raw).map_err(|error| {
                format!("cannot decode canonical Snapshot {index} while planning capture: {error}")
            })
        })
        .collect()
}

fn source_record_count(
    authority_root: &Path,
    name: &str,
    record_size: u32,
) -> GenerationResult<u32> {
    let path = authority_root.join(name);
    if !path.exists() {
        return Ok(0);
    }
    let byte_size = fs::metadata(&path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
        .len();
    let count = fixed_record_count(&format!("local/{name}"), byte_size)?
        .ok_or_else(|| format!("{name} has no fixed-record schema"))?;
    let expected_body = count
        .checked_mul(u64::from(record_size))
        .ok_or_else(|| format!("{name} byte length overflows u64"))?;
    if expected_body + 4 != byte_size {
        return Err(format!(
            "{name} has an inconsistent fixed-record byte length"
        ));
    }
    u32::try_from(count).map_err(|_| format!("{name} record count exceeds u32::MAX"))
}

fn repack_reachable_content(
    source: &LocalContentBinaryDb<1>,
    source_read: &crate::binary_db::BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    staging_root: &Path,
    repo_name: &str,
    plan: &ReachableContentPlan,
    files: &mut Vec<GenerationFileManifest>,
) -> GenerationResult<CapturedContentProjection> {
    let repacked_authority = staging_root.join(".content-repack-local");
    if repacked_authority.exists() {
        return Err(format!(
            "content repack authority already exists: {}",
            repacked_authority.display()
        ));
    }
    fs::create_dir_all(&repacked_authority).map_err(|error| {
        format!(
            "failed to create content repack authority {}: {error}",
            repacked_authority.display()
        )
    })?;
    let repacked_db = capture_db(
        repacked_authority.clone(),
        staging_root.to_path_buf(),
        format!("capture-repack:{repo_name}"),
    );
    let repacked = LocalContentBinaryDb::<1>::from_db_with_roots(
        repacked_db,
        staging_root.to_path_buf(),
        staging_root.to_path_buf(),
    );

    let (blob_count, object_pack_count, object_pack_member_count, blob_sizes_by_id) =
        repack_object_packs(source, source_read, &repacked, staging_root, plan, files)?;
    let (tree_pack_count, tree_count, source_tree_ids, repacked_tree_locators) = repack_tree_packs(
        source,
        source_read,
        &repacked,
        staging_root,
        plan,
        &blob_sizes_by_id,
        files,
    )?;
    rewrite_repacked_snapshot_roots(
        staging_root,
        &source_tree_ids,
        &repacked_tree_locators,
        files,
    )?;
    rewrite_repacked_plan_roots(
        staging_root,
        &source_tree_ids,
        &repacked_tree_locators,
        &plan.rootless_plan_revision_indices,
        files,
    )?;
    install_repacked_content_metadata(&repacked_authority, staging_root, files)?;
    fs::remove_dir_all(&repacked_authority).map_err(|error| {
        format!(
            "failed to remove content repack authority {}: {error}",
            repacked_authority.display()
        )
    })?;

    Ok(CapturedContentProjection {
        source_blob_records: u64::from(plan.source_blob_count),
        retained_blob_records: blob_count,
        source_object_pack_records: u64::from(plan.source_object_pack_count),
        retained_object_pack_records: object_pack_count,
        source_object_pack_member_records: u64::from(plan.source_object_pack_member_count),
        retained_object_pack_member_records: object_pack_member_count,
        source_tree_pack_records: u64::from(plan.source_tree_pack_count),
        retained_tree_pack_records: tree_pack_count,
        source_tree_records: u64::from(plan.source_tree_count),
        retained_tree_records: tree_count,
        source_retained: true,
    })
}

fn repack_object_packs(
    source: &LocalContentBinaryDb<1>,
    source_read: &crate::binary_db::BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    repacked: &LocalContentBinaryDb<1>,
    staging_root: &Path,
    plan: &ReachableContentPlan,
    files: &mut Vec<GenerationFileManifest>,
) -> GenerationResult<(u64, u64, u64, BTreeMap<String, u64>)> {
    let mut canonical_by_blob_id = BTreeMap::<String, CanonicalBlobSource>::new();
    for source_blob_index in &plan.output_blob_indices {
        let blob = source
            .blobs()
            .blob_view_at(source_read, *source_blob_index)
            .map_err(|error| {
                format!("cannot read retained blob {source_blob_index} for repack: {error}")
            })?;
        if blob.record.is_tombstone() || blob.record.is_pruned() {
            return Err(format!(
                "retained blob {source_blob_index} is unavailable during repack"
            ));
        }
        let member_index = blob.pack_member_index.ok_or_else(|| {
            format!("retained blob {source_blob_index} has no object-pack member")
        })?;
        if !plan
            .retained_object_pack_member_indices
            .contains(&member_index)
        {
            return Err(format!(
                "retained blob {source_blob_index} points to unretained member {member_index}"
            ));
        }
        let member = source
            .object_packs()
            .object_pack_member_view_at(source_read, member_index)
            .map_err(|error| {
                format!("cannot read retained object-pack member {member_index}: {error}")
            })?;
        if member.record.is_tombstone() || member.record.blob_index != *source_blob_index {
            return Err(format!(
                "retained blob {source_blob_index} and member {member_index} are not live backreferences"
            ));
        }
        let normalized_blob_id = blob.blob_id.to_ascii_lowercase();
        let candidate = CanonicalBlobSource {
            source_blob_index: *source_blob_index,
            source_pack_index: member.record.pack_index,
            member,
            record: blob.record,
        };
        if let Some(existing) = canonical_by_blob_id.get(&normalized_blob_id) {
            if existing.record.sha256 != candidate.record.sha256
                || existing.record.size_bytes != candidate.record.size_bytes
            {
                return Err(format!(
                    "retained blob identity {} has conflicting source records {} and {}",
                    candidate.member.blob_id,
                    existing.source_blob_index,
                    candidate.source_blob_index
                ));
            }
            if existing.source_blob_index > candidate.source_blob_index {
                continue;
            }
        }
        canonical_by_blob_id.insert(normalized_blob_id, candidate);
    }

    let blob_sizes_by_id = canonical_by_blob_id
        .iter()
        .map(|(blob_id, source)| (blob_id.clone(), source.record.size_bytes))
        .collect::<BTreeMap<_, _>>();
    let mut sources_by_pack = BTreeMap::<u32, Vec<CanonicalBlobSource>>::new();
    for source in canonical_by_blob_id.into_values() {
        sources_by_pack
            .entry(source.source_pack_index)
            .or_default()
            .push(source);
    }
    let coordinator = BinaryDbContentWriteCoordinator::new(
        repacked.blobs(),
        repacked.object_packs(),
        repacked.tree_packs(),
        repacked.trees(),
        repacked.snapshots(),
    );
    let mut seen_pack_members = BTreeMap::<String, Vec<String>>::new();
    let mut output_blob_count = 0_u64;
    let mut output_pack_count = 0_u64;
    for (source_pack_index, mut blob_sources) in sources_by_pack {
        blob_sources.sort_by(|left, right| left.member.blob_id.cmp(&right.member.blob_id));
        let source_pack = source
            .object_packs()
            .object_pack_view_at(source_read, source_pack_index)
            .map_err(|error| {
                format!("cannot read retained object pack {source_pack_index}: {error}")
            })?;
        if source_pack.record.is_tombstone() || !source_pack.record.is_ready() {
            return Err(format!(
                "retained object pack {} is not live and ready",
                source_pack.pack_id
            ));
        }
        let blob_ids = blob_sources
            .iter()
            .map(|source| source.member.blob_id.clone())
            .collect::<Vec<_>>();
        let pack_identity = blob_sources
            .iter()
            .map(|source| {
                format!(
                    "{}:{}",
                    source.member.blob_id,
                    hex_lower(&source.record.sha256)
                )
            })
            .collect::<Vec<_>>();
        let pack_id = object_pack_id_from_hash48(stable_repack_hash48("object", &pack_identity));
        if let Some(existing) = seen_pack_members.insert(pack_id.clone(), pack_identity.clone()) {
            return Err(format!(
                "object repack identity collision for {pack_id}: {existing:?} versus {pack_identity:?}"
            ));
        }
        let bytes_by_blob_id =
            crate::object_diff_ports::BlobReader::read_blob_bytes_batch(source.blobs(), &blob_ids)
                .map_err(|error| {
                    format!(
                        "cannot materialize retained object pack {}: {error}",
                        source_pack.pack_id
                    )
                })?;
        let mut archive_members = Vec::with_capacity(blob_sources.len());
        let mut metadata_members = Vec::with_capacity(blob_sources.len());
        for blob_source in &blob_sources {
            let blob_id = &blob_source.member.blob_id;
            let data = bytes_by_blob_id
                .get(blob_id)
                .cloned()
                .ok_or_else(|| format!("retained blob {blob_id} disappeared while repacking"))?;
            if u64::try_from(data.len()).ok() != Some(blob_source.record.size_bytes) {
                return Err(format!(
                    "retained blob {blob_id} size changed while repacking"
                ));
            }
            archive_members.push(ObjectPackWriteMember {
                entry_name: format!("blobs/{blob_id}"),
                blob_id: blob_id.clone(),
                data,
                logical_data: None,
                entry_type: "full".to_string(),
                base_blob_id: None,
                chain_depth: 0,
                delta_algorithm: None,
            });
            metadata_members.push(BinaryDbObjectPackMemberWriteInput {
                blob_id: blob_id.clone(),
                sha256: hex_lower(&blob_source.record.sha256),
                size_bytes: i64::try_from(blob_source.record.size_bytes)
                    .map_err(|_| format!("retained blob {blob_id} size exceeds i64"))?,
                pack_entry_type: "full".to_string(),
                pack_base_blob_id: None,
                pack_chain_depth: 0,
                created_at: timestamp_from_seconds(blob_source.record.created_at_s)?,
            });
        }
        let relative_path = default_object_pack_relative_path(&pack_id);
        let normalized_path = normalize_pack_relative_path(&relative_path)?;
        let absolute_path = staging_root.join(&normalized_path);
        let archive_stats = write_typed_pack_archive_with_format(
            path_text(&absolute_path)?.as_str(),
            &pack_id,
            CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
            &archive_members,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )?;
        let member_count = required_json_u64(&archive_stats, "member_count")?;
        if member_count != blob_sources.len() as u64 {
            return Err(format!(
                "repacked object pack {pack_id} wrote {member_count} members, expected {}",
                blob_sources.len()
            ));
        }
        coordinator
            .record_object_pack_metadata(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbObjectPackWriteInput {
                    pack_id: pack_id.clone(),
                    pack_rel_path: normalized_path.clone(),
                    pack_format: required_json_text(&archive_stats, "pack_format")?,
                    member_count: i64::try_from(member_count)
                        .map_err(|_| format!("object pack {pack_id} member count exceeds i64"))?,
                    total_bytes: i64::try_from(required_json_u64(&archive_stats, "total_bytes")?)
                        .map_err(|_| {
                        format!("object pack {pack_id} total bytes exceed i64")
                    })?,
                    created_at: timestamp_from_seconds(source_pack.record.created_at_s)?,
                    members: metadata_members,
                },
            )
            .map_err(|error| format!("cannot record repacked object pack {pack_id}: {error}"))?;
        refresh_captured_file_manifest(files, staging_root, &normalized_path)?;
        output_blob_count = output_blob_count
            .checked_add(member_count)
            .ok_or_else(|| "repacked blob count overflows u64".to_string())?;
        output_pack_count = output_pack_count
            .checked_add(1)
            .ok_or_else(|| "repacked object-pack count overflows u64".to_string())?;
    }
    Ok((
        output_blob_count,
        output_pack_count,
        output_blob_count,
        blob_sizes_by_id,
    ))
}

fn canonicalize_repacked_tree_entries(
    tree_id: &str,
    mut entries: Vec<BinaryTreeEntryView>,
    blob_sizes_by_id: &BTreeMap<String, u64>,
) -> GenerationResult<Vec<BinaryTreeEntryView>> {
    for entry in &mut entries {
        match entry.entry_type.as_str() {
            "blob" => {
                let size_bytes = blob_sizes_by_id
                    .get(&entry.target_id.to_ascii_lowercase())
                    .copied()
                    .ok_or_else(|| {
                        format!(
                            "retained tree {tree_id} entry {:?} references blob {} outside the retained canonical closure",
                            entry.entry_name, entry.target_id
                        )
                    })?;
                if entry
                    .size_bytes
                    .is_some_and(|persisted| persisted != size_bytes)
                {
                    return Err(format!(
                        "retained tree {tree_id} entry {:?} size disagrees with canonical blob {}",
                        entry.entry_name, entry.target_id
                    ));
                }
                // A known pre-restoration tree-pack writer omitted blob sizes even
                // though the tree identity included them. The blob record is the
                // exact local authority for restoring that value offline.
                entry.size_bytes = Some(size_bytes);
            }
            "tree" => {
                if entry.size_bytes.is_some() {
                    return Err(format!(
                        "retained tree {tree_id} entry {:?} gives a child tree a byte size",
                        entry.entry_name
                    ));
                }
                if entry.mode.as_deref() != Some("tree") {
                    return Err(format!(
                        "retained tree {tree_id} entry {:?} has non-canonical tree mode {:?}",
                        entry.entry_name, entry.mode
                    ));
                }
            }
            other => {
                return Err(format!(
                    "retained tree {tree_id} entry {:?} has unsupported type {other}",
                    entry.entry_name
                ));
            }
        }
    }
    entries.sort_by(|left, right| left.entry_name.cmp(&right.entry_name));
    let restored_tree_id = tree_id_from_canonical_entries(&entries)?;
    if !restored_tree_id.eq_ignore_ascii_case(tree_id) {
        return Err(format!(
            "retained tree {tree_id} content restores to different identity {restored_tree_id}"
        ));
    }
    Ok(entries)
}

fn tree_id_from_canonical_entries(entries: &[BinaryTreeEntryView]) -> GenerationResult<String> {
    let mut serialized_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry.entry_type.as_str() {
            "blob" => serialized_entries.push(json!({
                "name": entry.entry_name,
                "type": "blob",
                "target_id": entry.target_id,
                "size_bytes": entry.size_bytes.ok_or_else(|| {
                    format!("tree entry {:?} is missing canonical blob size", entry.entry_name)
                })?,
                "mode": entry.mode.as_deref().ok_or_else(|| {
                    format!("tree entry {:?} is missing canonical blob mode", entry.entry_name)
                })?,
            })),
            "tree" => serialized_entries.push(json!({
                "name": entry.entry_name,
                "type": "tree",
                "target_id": entry.target_id,
            })),
            other => {
                return Err(format!(
                    "tree entry {:?} has unsupported type {other}",
                    entry.entry_name
                ));
            }
        }
    }
    let encoded = JsonCodec::encode_serializable(&serialized_entries, JsonEncodeOptions::compact())
        .map_err(|error| format!("cannot encode canonical tree identity: {error}"))?;
    let digest = Sha256::digest(encoded.as_bytes());
    let mut hash80 = [0_u8; 10];
    hash80.copy_from_slice(&digest[..10]);
    Ok(tree_id_from_hash80(&hash80))
}

#[allow(clippy::type_complexity)]
fn repack_tree_packs(
    source: &LocalContentBinaryDb<1>,
    source_read: &crate::binary_db::BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    repacked: &LocalContentBinaryDb<1>,
    staging_root: &Path,
    plan: &ReachableContentPlan,
    blob_sizes_by_id: &BTreeMap<String, u64>,
    files: &mut Vec<GenerationFileManifest>,
) -> GenerationResult<(
    u64,
    u64,
    BTreeMap<(u32, u32), String>,
    BTreeMap<String, RepackedTreeLocator>,
)> {
    let source_packs = source
        .tree_packs()
        .list_tree_pack_views(source_read)
        .map_err(|error| format!("cannot list retained tree packs for repack: {error}"))?;
    let mut pack_position_by_index = BTreeMap::new();
    let mut pack_index_by_tree = vec![None; plan.source_tree_count as usize];
    for (position, pack) in source_packs.iter().enumerate() {
        pack_position_by_index.insert(pack.tree_pack_index, position);
        let end = pack
            .record
            .first_tree_index
            .checked_add(pack.record.tree_count)
            .ok_or_else(|| format!("tree pack {} range overflows", pack.pack_id))?;
        if end > plan.source_tree_count {
            return Err(format!(
                "tree pack {} exceeds source tree record count",
                pack.pack_id
            ));
        }
        for tree_index in pack.record.first_tree_index..end {
            let slot = &mut pack_index_by_tree[tree_index as usize];
            if slot.replace(pack.tree_pack_index).is_some() {
                return Err(format!(
                    "source tree {tree_index} belongs to overlapping tree-pack ranges"
                ));
            }
        }
    }

    let mut canonical_by_tree_id = BTreeMap::<String, CanonicalTreeSource>::new();
    let mut source_tree_ids = BTreeMap::<(u32, u32), String>::new();
    let mut cache = BinaryDbTreeReadCache::default();
    for source_tree_index in &plan.retained_tree_indices {
        let source_pack_index = pack_index_by_tree
            .get(*source_tree_index as usize)
            .and_then(|value| *value)
            .ok_or_else(|| format!("retained tree {source_tree_index} has no source pack"))?;
        let pack = &source_packs[*pack_position_by_index
            .get(&source_pack_index)
            .ok_or_else(|| format!("retained tree pack {source_pack_index} is missing"))?];
        let mut record = source
            .trees()
            .read_tree_record(source_read, *source_tree_index)
            .map_err(|error| format!("cannot read retained tree {source_tree_index}: {error}"))?;
        if record.is_tombstone() {
            return Err(format!("retained tree {source_tree_index} is tombstoned"));
        }
        let tree_id = tree_id_from_hash80(&record.tree_hash80);
        let physical_ordinal = if pack.record.has_sparse_physical_ordinals() {
            record.pack_entry_ordinal
        } else {
            let ordinal = source_tree_index
                .checked_sub(pack.record.first_tree_index)
                .ok_or_else(|| format!("tree {source_tree_index} precedes its source pack"))?;
            record.pack_entry_ordinal = ordinal;
            ordinal
        };
        if let Some(existing) =
            source_tree_ids.insert((source_pack_index, physical_ordinal), tree_id.clone())
        {
            return Err(format!(
                "source tree pack {} physical ordinal {} contains both {} and {}",
                pack.pack_id, physical_ordinal, existing, tree_id
            ));
        }
        let entries = source
            .trees()
            .list_tree_entry_views_for_record_in_pack_with_cache(
                source_read,
                *source_tree_index,
                &record,
                pack,
                &mut cache,
            )
            .map_err(|error| {
                format!(
                    "cannot read retained tree {tree_id} from {}: {error}",
                    pack.pack_id
                )
            })?;
        let entries = canonicalize_repacked_tree_entries(&tree_id, entries, blob_sizes_by_id)?;
        let normalized_tree_id = tree_id.to_ascii_lowercase();
        let candidate = CanonicalTreeSource {
            source_tree_index: *source_tree_index,
            source_pack_index,
            tree_id,
            record,
            entries,
        };
        if let Some(existing) = canonical_by_tree_id.get(&normalized_tree_id) {
            if existing.record.entry_count != candidate.record.entry_count
                || existing.entries != candidate.entries
            {
                return Err(format!(
                    "retained tree identity {} has conflicting source payloads at {} and {}",
                    candidate.tree_id, existing.source_tree_index, candidate.source_tree_index
                ));
            }
            if existing.source_tree_index > candidate.source_tree_index {
                continue;
            }
        }
        canonical_by_tree_id.insert(normalized_tree_id, candidate);
    }

    let mut sources_by_pack = BTreeMap::<u32, Vec<CanonicalTreeSource>>::new();
    for source in canonical_by_tree_id.into_values() {
        sources_by_pack
            .entry(source.source_pack_index)
            .or_default()
            .push(source);
    }
    let coordinator = BinaryDbContentWriteCoordinator::new(
        repacked.blobs(),
        repacked.object_packs(),
        repacked.tree_packs(),
        repacked.trees(),
        repacked.snapshots(),
    );
    let mut seen_pack_members = BTreeMap::<String, Vec<String>>::new();
    let mut output_pack_count = 0_u64;
    let mut output_tree_count = 0_u64;
    let mut repacked_tree_locators = BTreeMap::new();
    for (source_pack_index, mut tree_sources) in sources_by_pack {
        tree_sources.sort_by(|left, right| left.tree_id.cmp(&right.tree_id));
        let source_pack = source
            .tree_packs()
            .tree_pack_view_at(source_read, source_pack_index)
            .map_err(|error| {
                format!("cannot read retained tree pack {source_pack_index}: {error}")
            })?;
        if source_pack.record.is_tombstone() || !source_pack.record.is_ready() {
            return Err(format!(
                "retained tree pack {} is not live and ready",
                source_pack.pack_id
            ));
        }
        let tree_rows = JsonValue::Array(
            tree_sources
                .iter()
                .map(|source| {
                    json!({
                        "tree_id": source.tree_id,
                        "entry_count": source.record.entry_count,
                    })
                })
                .collect(),
        );
        let mut entry_rows = Vec::new();
        for tree_source in &tree_sources {
            for entry in &tree_source.entries {
                let mode = entry.mode.clone().ok_or_else(|| {
                    format!(
                        "retained tree {} entry {:?} has no canonical mode",
                        tree_source.tree_id, entry.entry_name
                    )
                })?;
                entry_rows.push(json!({
                    "tree_id": tree_source.tree_id,
                    "entry_name": entry.entry_name,
                    "entry_type": entry.entry_type,
                    "target_id": entry.target_id,
                    "size_bytes": entry.size_bytes,
                    "mode": mode,
                }));
            }
        }
        let members = build_tree_pack_members(&tree_rows, &JsonValue::Array(entry_rows))?;
        let pack_identity = members
            .as_array()
            .ok_or_else(|| "repacked tree members must be an array".to_string())?
            .iter()
            .map(|member| {
                let tree_id = member
                    .get("tree_id")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| "repacked tree member is missing tree_id".to_string())?;
                let checksum = member
                    .get("checksum")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| "repacked tree member is missing checksum".to_string())?;
                Ok(format!("{tree_id}:{checksum}"))
            })
            .collect::<GenerationResult<Vec<_>>>()?;
        let pack_id = tree_pack_id_from_hash48(stable_repack_hash48("tree", &pack_identity));
        if let Some(existing) = seen_pack_members.insert(pack_id.clone(), pack_identity.clone()) {
            return Err(format!(
                "tree repack identity collision for {pack_id}: {existing:?} versus {pack_identity:?}"
            ));
        }
        let relative_path = default_tree_pack_relative_path(&pack_id);
        let normalized_path = normalize_pack_relative_path(&relative_path)?;
        let absolute_path = staging_root.join(&normalized_path);
        let archive_stats = write_tree_pack_archive_with_format(
            path_text(&absolute_path)?.as_str(),
            &pack_id,
            CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
            &members,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )?;
        let tree_count = required_json_u64(&archive_stats, "tree_count")?;
        if tree_count != tree_sources.len() as u64 {
            return Err(format!(
                "repacked tree pack {pack_id} wrote {tree_count} trees, expected {}",
                tree_sources.len()
            ));
        }
        coordinator
            .record_tree_pack_metadata(
                BinaryDbCommandScope::ContentWrite,
                &BinaryDbTreePackWriteInput {
                    pack_id: pack_id.clone(),
                    pack_rel_path: normalized_path.clone(),
                    pack_format: required_json_text(&archive_stats, "pack_format")?,
                    tree_count: i64::try_from(tree_count)
                        .map_err(|_| format!("tree pack {pack_id} count exceeds i64"))?,
                    total_bytes: i64::try_from(required_json_u64(&archive_stats, "total_bytes")?)
                        .map_err(|_| {
                        format!("tree pack {pack_id} total bytes exceed i64")
                    })?,
                    created_at: timestamp_from_seconds(source_pack.record.created_at_s)?,
                    trees: tree_sources
                        .iter()
                        .map(|source| BinaryDbTreePackTreeWriteInput {
                            tree_id: source.tree_id.clone(),
                            entry_count: i64::from(source.record.entry_count),
                        })
                        .collect(),
                },
            )
            .map_err(|error| format!("cannot record repacked tree pack {pack_id}: {error}"))?;
        let repacked_read = repacked.tree_packs().begin_read_txn();
        let repacked_pack = repacked
            .tree_packs()
            .get_tree_pack_view(&repacked_read, &pack_id)
            .map_err(|error| format!("cannot resolve repacked tree pack {pack_id}: {error}"))?
            .ok_or_else(|| format!("repacked tree pack {pack_id} was not recorded"))?;
        drop(repacked_read);
        for (ordinal, tree_source) in tree_sources.iter().enumerate() {
            let locator = RepackedTreeLocator {
                tree_pack_index: repacked_pack.tree_pack_index,
                pack_entry_ordinal: u32::try_from(ordinal)
                    .map_err(|_| format!("tree pack {pack_id} ordinal exceeds u32"))?,
            };
            if repacked_tree_locators
                .insert(tree_source.tree_id.to_ascii_lowercase(), locator)
                .is_some()
            {
                return Err(format!(
                    "repacked tree {} received more than one locator",
                    tree_source.tree_id
                ));
            }
        }
        refresh_captured_file_manifest(files, staging_root, &normalized_path)?;
        output_tree_count = output_tree_count
            .checked_add(tree_count)
            .ok_or_else(|| "repacked tree count overflows u64".to_string())?;
        output_pack_count = output_pack_count
            .checked_add(1)
            .ok_or_else(|| "repacked tree-pack count overflows u64".to_string())?;
    }
    Ok((
        output_pack_count,
        output_tree_count,
        source_tree_ids,
        repacked_tree_locators,
    ))
}

fn rewrite_repacked_snapshot_roots(
    staging_root: &Path,
    source_tree_ids: &BTreeMap<(u32, u32), String>,
    repacked_tree_locators: &BTreeMap<String, RepackedTreeLocator>,
    files: &mut Vec<GenerationFileManifest>,
) -> GenerationResult<()> {
    let relative_path = format!("local/{SNAPSHOT_BIN}");
    let path = staging_root.join(&relative_path);
    if !path.exists() {
        return Ok(());
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("failed to read {} for root remap: {error}", path.display()))?;
    if bytes.get(..4) != Some(1_u32.to_le_bytes().as_slice())
        || (bytes.len() - 4) % SNAPSHOT_RECORD_SIZE as usize != 0
    {
        return Err(format!(
            "captured Snapshot authority {} is invalid before root remap",
            path.display()
        ));
    }
    let mut output = 1_u32.to_le_bytes().to_vec();
    for (snapshot_index, raw) in bytes[4..]
        .chunks_exact(SNAPSHOT_RECORD_SIZE as usize)
        .enumerate()
    {
        let mut snapshot = BinarySnapshotCodec::<1>::decode_record(raw)
            .map_err(|error| format!("cannot decode Snapshot {snapshot_index}: {error}"))?;
        if snapshot.is_tombstone() {
            snapshot.root_tree_pack_index_plus1 = 0;
            snapshot.root_entry_ordinal = 0;
            snapshot.snapshot_meta &=
                !crate::content_binary_db::BinarySnapshotRecord::META_HAS_ROOT_LOCATOR;
        } else if let Some(source_pack_index) = snapshot.root_tree_pack_index() {
            let tree_id = source_tree_ids
                .get(&(source_pack_index, snapshot.root_entry_ordinal))
                .ok_or_else(|| {
                    format!(
                        "live Snapshot {snapshot_index} root ({source_pack_index}, {}) was not retained",
                        snapshot.root_entry_ordinal
                    )
                })?;
            let locator = repacked_tree_locators
                .get(&tree_id.to_ascii_lowercase())
                .ok_or_else(|| {
                    format!(
                        "live Snapshot {snapshot_index} root tree {tree_id} has no repacked locator"
                    )
                })?;
            snapshot.root_tree_pack_index_plus1 = locator
                .tree_pack_index
                .checked_add(1)
                .ok_or_else(|| "repacked tree-pack index plus-one overflows".to_string())?;
            snapshot.root_entry_ordinal = locator.pack_entry_ordinal;
            snapshot.snapshot_meta |=
                crate::content_binary_db::BinarySnapshotRecord::META_HAS_ROOT_LOCATOR;
        } else {
            snapshot.root_entry_ordinal = 0;
            snapshot.snapshot_meta &=
                !crate::content_binary_db::BinarySnapshotRecord::META_HAS_ROOT_LOCATOR;
        }
        output.extend_from_slice(
            &BinarySnapshotCodec::<1>::encode_record(&snapshot)
                .map_err(|error| format!("cannot encode Snapshot {snapshot_index}: {error}"))?,
        );
    }
    write_captured_bytes(&path, &output)?;
    refresh_captured_file_manifest(files, staging_root, &relative_path)
}

fn rewrite_repacked_plan_roots(
    staging_root: &Path,
    source_tree_ids: &BTreeMap<(u32, u32), String>,
    repacked_tree_locators: &BTreeMap<String, RepackedTreeLocator>,
    rootless_plan_revision_indices: &BTreeSet<u32>,
    files: &mut Vec<GenerationFileManifest>,
) -> GenerationResult<()> {
    let relative_path = format!("local/{PLAN_REVISION_BIN}");
    let path = staging_root.join(&relative_path);
    if !path.exists() {
        return Ok(());
    }
    let bytes = fs::read(&path)
        .map_err(|error| format!("failed to read {} for root remap: {error}", path.display()))?;
    if bytes.get(..4) != Some(1_u32.to_le_bytes().as_slice())
        || (bytes.len() - 4) % PLAN_REVISION_RECORD_SIZE as usize != 0
    {
        return Err(format!(
            "captured Plan revision authority {} is invalid before root remap",
            path.display()
        ));
    }
    let mut output = 1_u32.to_le_bytes().to_vec();
    for (revision_index, raw) in bytes[4..]
        .chunks_exact(PLAN_REVISION_RECORD_SIZE as usize)
        .enumerate()
    {
        let mut revision = PlanRevisionCodec::<1>::decode_record(raw)
            .map_err(|error| format!("cannot decode Plan revision {revision_index}: {error}"))?;
        if revision.is_tombstone() {
            revision.root_tree_pack_index_plus1 = 0;
            revision.root_entry_ordinal = 0;
        } else if rootless_plan_revision_indices.contains(
            &u32::try_from(revision_index)
                .map_err(|_| "Plan revision index exceeds u32::MAX".to_string())?,
        ) {
            if revision.root_tree_pack_index().is_some() || revision.root_entry_ordinal != 0 {
                return Err(format!(
                    "rootless live Plan revision {revision_index} changed before root remap"
                ));
            }
        } else {
            let source_pack_index = revision.root_tree_pack_index().ok_or_else(|| {
                format!("live Plan revision {revision_index} has no repo-root Tree locator")
            })?;
            let tree_id = source_tree_ids
                .get(&(source_pack_index, revision.root_entry_ordinal))
                .ok_or_else(|| {
                    format!(
                        "live Plan revision {revision_index} repo root ({source_pack_index}, {}) was not retained",
                        revision.root_entry_ordinal
                    )
                })?;
            let locator = repacked_tree_locators
                .get(&tree_id.to_ascii_lowercase())
                .ok_or_else(|| {
                    format!(
                        "live Plan revision {revision_index} repo-root Tree {tree_id} has no repacked locator"
                    )
                })?;
            revision.root_tree_pack_index_plus1 = locator
                .tree_pack_index
                .checked_add(1)
                .ok_or_else(|| "repacked Plan root Tree-pack index overflows".to_string())?;
            revision.root_entry_ordinal = locator.pack_entry_ordinal;
        }
        output.extend_from_slice(
            &PlanRevisionCodec::<1>::encode_record(&revision).map_err(|error| {
                format!("cannot encode Plan revision {revision_index}: {error}")
            })?,
        );
    }
    write_captured_bytes(&path, &output)?;
    refresh_captured_file_manifest(files, staging_root, &relative_path)
}

fn install_repacked_content_metadata(
    repacked_authority: &Path,
    staging_root: &Path,
    files: &mut Vec<GenerationFileManifest>,
) -> GenerationResult<()> {
    for name in [
        BLOB_BIN,
        BLOB_ID_IDX,
        OBJECT_PACK_BIN,
        OBJECT_PACK_ID_IDX,
        OBJECT_PACK_MEMBER_BIN,
        TREE_PACK_BIN,
        TREE_PACK_ID_IDX,
        TREE_BIN,
        TREE_ID_IDX,
    ] {
        let relative_path = format!("local/{name}");
        files.retain(|file| file.relative_path != relative_path);
        let source = repacked_authority.join(name);
        let destination = staging_root.join(&relative_path);
        if !source.exists() {
            if destination.exists() {
                fs::remove_file(&destination).map_err(|error| {
                    format!("failed to remove stale {}: {error}", destination.display())
                })?;
            }
            continue;
        }
        let bytes = fs::read(&source)
            .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
        if bytes.len() <= 4 {
            return Err(format!(
                "repacked content metadata {} is forbidden header-only output",
                source.display()
            ));
        }
        write_captured_bytes(&destination, &bytes)?;
        refresh_captured_file_manifest(files, staging_root, &relative_path)?;
    }
    Ok(())
}

fn stable_repack_hash48(domain: &str, member_ids: &[String]) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"ait.binary-db-generation.repack.v1\0");
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    for member_id in member_ids {
        hasher.update((member_id.len() as u64).to_le_bytes());
        hasher.update(member_id.as_bytes());
    }
    let digest = hasher.finalize();
    (u64::from(digest[0]) << 40)
        | (u64::from(digest[1]) << 32)
        | (u64::from(digest[2]) << 24)
        | (u64::from(digest[3]) << 16)
        | (u64::from(digest[4]) << 8)
        | u64::from(digest[5])
}

fn timestamp_from_seconds(seconds: u64) -> GenerationResult<String> {
    let seconds = i64::try_from(seconds)
        .map_err(|_| format!("Binary DB timestamp {seconds} cannot be represented as RFC 3339"))?;
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
        .ok_or_else(|| format!("timestamp {seconds} is outside RFC3339 range"))
}

fn required_json_u64(value: &JsonValue, field: &str) -> GenerationResult<u64> {
    value
        .get(field)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| format!("pack archive stats are missing integer {field}"))
}

fn required_json_text(value: &JsonValue, field: &str) -> GenerationResult<String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("pack archive stats are missing text {field}"))
}

fn collect_copy_inputs(authority_root: &Path) -> GenerationResult<Vec<CopyInput>> {
    let mut inputs = Vec::new();
    let mut seen = BTreeSet::new();
    let mut authority_entries = fs::read_dir(authority_root)
        .map_err(|error| format!("failed to inventory {}: {error}", authority_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inventory {}: {error}", authority_root.display()))?;
    authority_entries.sort_by_key(|entry| entry.file_name());
    for entry in authority_entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_dir() && entry.file_name() == ".locks" {
            continue;
        }
        if !file_type.is_file() {
            return Err(format!(
                "Binary DB authority contains an undeclared non-file path: {}",
                path.display()
            ));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("Binary DB authority path is not UTF-8: {}", path.display()))?;
        let allowed = if name.ends_with(".bin") {
            REPOSITORY_BINARY_DB_BIN_PATHS.contains(&name.as_str())
        } else if name.ends_with(".idx") {
            REPOSITORY_BINARY_DB_INDEX_PATHS.contains(&name.as_str())
        } else {
            false
        };
        if !allowed {
            return Err(format!(
                "Binary DB authority contains an undeclared file: {name:?}"
            ));
        }
        let header_only = name.ends_with(".bin")
            && fs::metadata(&path)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
                .len()
                <= 4;
        if header_only {
            return Err(format!(
                "Binary DB authority contains forbidden header-only file: {name:?}"
            ));
        }
        let strategy = if name == "tree.bin" {
            CopyStrategy::CanonicalTreeLocators {
                tree_pack_source: authority_root.join("tree_pack.bin"),
            }
        } else {
            CopyStrategy::Exact
        };
        push_copy_input(
            &mut inputs,
            &mut seen,
            path,
            format!("local/{name}"),
            strategy,
        )?;
    }
    inputs.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(inputs)
}

fn normalize_pack_relative_path(value: &str) -> GenerationResult<String> {
    let normalized = value.replace('\\', "/");
    validate_generation_file_path(&normalized)
        .map_err(|error| format!("invalid generated pack path {value:?}: {error}"))?;
    if !normalized.starts_with(".ait/objects/") {
        return Err(format!("pack path is outside .ait/objects/: {value:?}"));
    }
    Ok(normalized)
}

fn push_copy_input(
    inputs: &mut Vec<CopyInput>,
    seen: &mut BTreeSet<String>,
    source: PathBuf,
    relative_path: String,
    strategy: CopyStrategy,
) -> GenerationResult<()> {
    if !seen.insert(relative_path.clone()) {
        return Err(format!(
            "Binary DB generation has duplicate source path: {relative_path:?}"
        ));
    }
    let metadata = fs::symlink_metadata(&source)
        .map_err(|error| format!("captured source {} is missing: {error}", source.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "captured source is not a regular file: {}",
            source.display()
        ));
    }
    inputs.push(CopyInput {
        source,
        relative_path,
        strategy,
    });
    Ok(())
}

fn copy_and_hash(
    input: &CopyInput,
    staging_root: &Path,
) -> GenerationResult<GenerationFileManifest> {
    match &input.strategy {
        CopyStrategy::Exact => copy_exact_and_hash(input, staging_root),
        CopyStrategy::CanonicalTreeLocators { tree_pack_source } => {
            canonicalize_tree_locators_and_hash(input, tree_pack_source, staging_root)
        }
    }
}

fn copy_exact_and_hash(
    input: &CopyInput,
    staging_root: &Path,
) -> GenerationResult<GenerationFileManifest> {
    let destination = staging_root.join(&input.relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let mut source = fs::File::open(&input.source)
        .map_err(|error| format!("failed to open {}: {error}", input.source.display()))?;
    let mut target = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut hasher = Sha256::new();
    let mut byte_size = 0_u64;
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {}: {error}", input.source.display()))?;
        if read == 0 {
            break;
        }
        target
            .write_all(&buffer[..read])
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
        hasher.update(&buffer[..read]);
        byte_size = byte_size
            .checked_add(read as u64)
            .ok_or_else(|| format!("captured file size overflow: {}", input.source.display()))?;
    }
    target
        .sync_data()
        .map_err(|error| format!("failed to sync {}: {error}", destination.display()))?;
    Ok(GenerationFileManifest {
        relative_path: input.relative_path.clone(),
        byte_size,
        sha256: hex_lower(&hasher.finalize()),
        record_count: fixed_record_count(&input.relative_path, byte_size)?,
    })
}

fn canonicalize_tree_locators_and_hash(
    input: &CopyInput,
    tree_pack_source: &Path,
    staging_root: &Path,
) -> GenerationResult<GenerationFileManifest> {
    let destination = staging_root.join(&input.relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let tree_bytes = fs::metadata(&input.source)
        .map_err(|error| format!("failed to inspect {}: {error}", input.source.display()))?
        .len();
    let tree_pack_bytes = fs::metadata(tree_pack_source)
        .map_err(|error| format!("failed to inspect {}: {error}", tree_pack_source.display()))?
        .len();
    let tree_count = fixed_record_count("local/tree.bin", tree_bytes)?
        .ok_or_else(|| "tree.bin has no fixed-record schema".to_string())?;
    let tree_pack_count = fixed_record_count("local/tree_pack.bin", tree_pack_bytes)?
        .ok_or_else(|| "tree_pack.bin has no fixed-record schema".to_string())?;

    let mut source = fs::File::open(&input.source)
        .map_err(|error| format!("failed to open {}: {error}", input.source.display()))?;
    let mut tree_packs = fs::File::open(tree_pack_source)
        .map_err(|error| format!("failed to open {}: {error}", tree_pack_source.display()))?;
    let mut target = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;

    let mut tree_header = [0_u8; 4];
    source
        .read_exact(&mut tree_header)
        .map_err(|error| format!("failed to read {} header: {error}", input.source.display()))?;
    let mut tree_pack_header = [0_u8; 4];
    tree_packs
        .read_exact(&mut tree_pack_header)
        .map_err(|error| {
            format!(
                "failed to read {} header: {error}",
                tree_pack_source.display()
            )
        })?;
    let tree_layout = u32::from_le_bytes(tree_header);
    let tree_pack_layout = u32::from_le_bytes(tree_pack_header);
    if tree_layout != 1 || tree_pack_layout != 1 {
        return Err(format!(
            "tree capture requires layout 1 tree.bin and tree_pack.bin, got {tree_layout} and {tree_pack_layout}"
        ));
    }

    target
        .write_all(&tree_header)
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(tree_header);
    let mut next_tree_index = 0_u64;
    let mut raw_pack = vec![0_u8; TREE_PACK_RECORD_SIZE as usize];
    let mut raw_tree = vec![0_u8; TREE_RECORD_SIZE as usize];
    for pack_index in 0..tree_pack_count {
        tree_packs.read_exact(&mut raw_pack).map_err(|error| {
            format!(
                "failed to read tree pack record {pack_index} from {}: {error}",
                tree_pack_source.display()
            )
        })?;
        let pack = BinaryTreePackCodec::<1>::decode_record(&raw_pack)
            .map_err(|error| format!("invalid tree pack record {pack_index}: {error}"))?;
        if u64::from(pack.first_tree_index) != next_tree_index {
            return Err(format!(
                "tree pack record {pack_index} starts at {}, expected dense tree index {next_tree_index}",
                pack.first_tree_index
            ));
        }
        for pack_entry_ordinal in 0..pack.tree_count {
            if next_tree_index >= tree_count {
                return Err(format!(
                    "tree pack record {pack_index} exceeds the {tree_count}-record tree.bin authority"
                ));
            }
            source.read_exact(&mut raw_tree).map_err(|error| {
                format!(
                    "failed to read tree record {next_tree_index} from {}: {error}",
                    input.source.display()
                )
            })?;
            let mut tree = BinaryTreeCodec::<1>::decode_record(&raw_tree)
                .map_err(|error| format!("invalid tree record {next_tree_index}: {error}"))?;
            if !pack.has_sparse_physical_ordinals() {
                tree.pack_entry_ordinal = pack_entry_ordinal;
            }
            let encoded = BinaryTreeCodec::<1>::encode_record(&tree).map_err(|error| {
                format!("failed to encode tree record {next_tree_index}: {error}")
            })?;
            target
                .write_all(&encoded)
                .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
            hasher.update(&encoded);
            next_tree_index = next_tree_index
                .checked_add(1)
                .ok_or_else(|| "tree record count overflow".to_string())?;
        }
    }
    if next_tree_index != tree_count {
        return Err(format!(
            "tree pack ranges cover {next_tree_index} tree records, but tree.bin contains {tree_count}"
        ));
    }
    target
        .sync_data()
        .map_err(|error| format!("failed to sync {}: {error}", destination.display()))?;
    Ok(GenerationFileManifest {
        relative_path: input.relative_path.clone(),
        byte_size: tree_bytes,
        sha256: hex_lower(&hasher.finalize()),
        record_count: Some(tree_count),
    })
}

fn validate_captured_tree_locators(
    staging_root: &Path,
    repo_name: &str,
    jobs: usize,
) -> GenerationResult<()> {
    let authority = staging_root.join("local");
    if !authority.join("tree.bin").is_file() {
        return Ok(());
    }
    let content = captured_content_store(staging_root, repo_name, "inventory");
    let read = content.trees().begin_read_txn();
    let packs = content
        .tree_packs()
        .list_tree_pack_views(&read)
        .map_err(|error| format!("captured tree-pack metadata validation failed: {error}"))?;
    drop(read);
    drop(content);
    let batches = balanced_tree_pack_batches(packs, jobs);
    let validation = deterministic_parallel_map(&batches, jobs, |batch_index, batch| {
        validate_captured_tree_pack_batch(staging_root, repo_name, batch_index, batch)
    })
    .map(|_| ());
    let validation_lock_root = authority.join(".locks");
    let lock_cleanup = if validation_lock_root.exists() {
        fs::remove_dir_all(&validation_lock_root).map_err(|error| {
            format!(
                "failed to remove capture validation locks {}: {error}",
                validation_lock_root.display()
            )
        })
    } else {
        Ok(())
    };
    match (validation, lock_cleanup) {
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; capture validation lock cleanup also failed: {cleanup_error}"
        )),
        (Err(error), _) => Err(error),
        (Ok(()), cleanup) => cleanup,
    }
}

fn balanced_tree_pack_batches(
    mut packs: Vec<BinaryTreePackView>,
    jobs: usize,
) -> Vec<Vec<BinaryTreePackView>> {
    if packs.is_empty() {
        return Vec::new();
    }
    let worker_count = jobs.min(packs.len()).max(1);
    packs.sort_by(|left, right| {
        right
            .record
            .tree_count
            .cmp(&left.record.tree_count)
            .then_with(|| left.tree_pack_index.cmp(&right.tree_pack_index))
    });
    let mut batches = vec![Vec::new(); worker_count];
    let mut tree_totals = vec![0_u64; worker_count];
    for pack in packs {
        let target = tree_totals
            .iter()
            .enumerate()
            .min_by_key(|(index, total)| (**total, *index))
            .map(|(index, _)| index)
            .unwrap_or_default();
        tree_totals[target] = tree_totals[target].saturating_add(u64::from(pack.record.tree_count));
        batches[target].push(pack);
    }
    for batch in &mut batches {
        batch.sort_by_key(|pack| pack.tree_pack_index);
    }
    batches
}

fn captured_content_store(
    staging_root: &Path,
    repo_name: &str,
    worker_label: &str,
) -> LocalContentBinaryDb<1> {
    let db = capture_db(
        staging_root.join("local"),
        staging_root.to_path_buf(),
        format!("capture-validation:{repo_name}:{worker_label}"),
    );
    LocalContentBinaryDb::<1>::from_db_with_roots(
        db,
        staging_root.to_path_buf(),
        staging_root.to_path_buf(),
    )
}

fn validate_captured_tree_pack_batch(
    staging_root: &Path,
    repo_name: &str,
    batch_index: usize,
    packs: &[BinaryTreePackView],
) -> GenerationResult<()> {
    let content = captured_content_store(staging_root, repo_name, &batch_index.to_string());
    let read = content.trees().begin_read_txn();
    let mut cache = BinaryDbTreeReadCache::default();
    for pack in packs {
        for ordinal in 0..pack.record.tree_count {
            let tree_index = pack
                .record
                .first_tree_index
                .checked_add(ordinal)
                .ok_or_else(|| format!("tree index overflow in pack {}", pack.pack_id))?;
            let tree = content
                .trees()
                .read_tree_record(&read, tree_index)
                .map_err(|error| {
                    format!(
                        "captured tree record {tree_index} in pack {} is invalid: {error}",
                        pack.pack_id
                    )
                })?;
            let tree_id = tree_id_from_hash80(&tree.tree_hash80);
            if tree.is_tombstone() {
                continue;
            }
            content
                .trees()
                .list_tree_entry_views_for_record_in_pack_with_cache(
                    &read, tree_index, &tree, pack, &mut cache,
                )
                .map_err(|error| {
                    format!(
                        "captured tree {} does not match pack {} ordinal {ordinal}: {error}",
                        tree_id, pack.pack_id
                    )
                })?;
        }
    }
    Ok(())
}

fn fixed_record_count(relative_path: &str, byte_size: u64) -> GenerationResult<Option<u64>> {
    let Some(name) = relative_path.strip_prefix("local/") else {
        return Ok(None);
    };
    let record_size = match name {
        "plan.bin" => 48,
        "plan_revision.bin" => 56,
        "plan_item.bin" => 16,
        TASK_RECORD_BIN => u64::from(LOCAL_TASK_RECORD_SIZE),
        TASK_CHANGE_INDEX_BIN => u64::from(TASK_CHANGE_INDEX_RECORD_SIZE),
        TASK_LAND_INDEX_BIN => u64::from(TASK_LAND_INDEX_RECORD_SIZE),
        CHANGE_RECORD_BIN => u64::from(LOCAL_CHANGE_RECORD_SIZE),
        CHANGE_LAND_INDEX_BIN => u64::from(CHANGE_LAND_INDEX_RECORD_SIZE),
        LAND_RECORD_BIN => u64::from(LOCAL_LAND_RECORD_SIZE),
        "blob.bin" => 64,
        "snapshot.bin" => 88,
        "object_pack.bin" => 32,
        "object_pack_member.bin" => 16,
        "tree_pack.bin" => 32,
        "tree.bin" => 20,
        "line.bin" => 40,
        "stash.bin" => 8,
        _ => return Ok(None),
    };
    let body = byte_size
        .checked_sub(4)
        .ok_or_else(|| format!("Binary DB file is shorter than its header: {relative_path}"))?;
    if body % record_size != 0 {
        return Err(format!(
            "Binary DB file is misaligned for {record_size}-byte records: {relative_path}"
        ));
    }
    Ok(Some(body / record_size))
}

fn record_counts(files: &[GenerationFileManifest]) -> GenerationResult<BTreeMap<String, u64>> {
    let mut counts = BTreeMap::new();
    for file in files {
        if let Some(count) = file.record_count {
            let name = Path::new(&file.relative_path)
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| format!("manifest path is not UTF-8: {}", file.relative_path))?;
            counts.insert(name.to_string(), count);
        }
    }
    Ok(counts)
}

fn fingerprint_files(files: &[GenerationFileManifest]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        hasher.update(file.relative_path.as_bytes());
        hasher.update([0]);
        hasher.update(file.byte_size.to_le_bytes());
        hasher.update(file.sha256.as_bytes());
        hasher.update([0]);
    }
    hex_lower(&hasher.finalize())
}

fn sha256_file(path: &Path) -> GenerationResult<String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open {} for hashing: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn required_layout_ids() -> BTreeMap<String, u32> {
    BTreeMap::from([
        ("content".to_string(), 1),
        ("line".to_string(), 1),
        ("plan".to_string(), 1),
        ("stash".to_string(), 1),
        ("workflow".to_string(), 1),
    ])
}

fn sync_directory(path: &Path) -> GenerationResult<()> {
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("failed to sync directory {}: {error}", path.display()))
}

fn path_text(path: &Path) -> GenerationResult<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary_db::BinaryDbCommandScope;
    use crate::binary_db_generation::{
        activate_binary_db_generation, BinaryDbGenerationActivationOptions,
    };
    use crate::content_binary_db::{
        snapshot_id_from_hash48, tree_id_from_hash80, tree_pack_id_from_hash48, BinaryDbBlobStore,
        BinaryDbContentWriteCoordinator, BinaryDbObjectPackStore, BinaryDbSnapshotStore,
        BinaryDbSnapshotWriteInput, BinaryDbTreePackStore, BinaryDbTreeStore, BinaryTreePackRecord,
        BinaryTreeRecord, LocalContentBinaryDb,
    };
    use crate::content_store::BlobStore;
    use crate::line_binary_db::BinaryDbLineStore;
    use crate::line_store::LineStore;
    use crate::plan_binary_db::{PlanPayload, PlanRecord, PlanRevisionPayload, PlanRevisionRecord};
    use crate::snapshot_store::SnapshotStore;
    use serde_json::json;
    use tempfile::TempDir;

    fn initialize_capture_fixture(temp: &TempDir) -> (PathBuf, PathBuf) {
        let repo = temp.path().join("repo");
        let authority = repo.join(".ait/binary-db");
        fs::create_dir_all(&authority).unwrap();
        fs::write(
            repo.join(".ait/config.json"),
            serde_json::to_vec(&json!({"repo_name": "fixture"})).unwrap(),
        )
        .unwrap();
        let content = LocalContentBinaryDb::<1>::new(
            authority.clone(),
            repo.clone(),
            AuthorityId::new("fixture"),
            LocalStateScope::Repository,
        );
        content
            .ensure_blob_bytes_content(b"workflow archive fixture", Some("fixture.txt"))
            .unwrap();
        (repo, authority)
    }

    #[test]
    fn fixed_record_counter_omits_retired_tables() {
        assert_eq!(
            fixed_record_count("local/workflow_record.bin", 4 + 2 * 12).unwrap(),
            None
        );
        assert_eq!(
            fixed_record_count("local/snapshot_parent_edge.bin", 4 + 12).unwrap(),
            None
        );
    }

    #[test]
    fn capture_preserves_schema_valid_empty_artifact_plan_revision_without_root() {
        let temp = TempDir::new().unwrap();
        let (repo, authority) = initialize_capture_fixture(&temp);
        let generation = temp.path().join("generation");
        let db = LocalBinaryDbFs::new(
            authority,
            repo.clone(),
            AuthorityId::new("rootless-plan-fixture"),
            LocalStateScope::Repository,
        )
        .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
        .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
        let plans = BinaryDbPlanStore::<_, 1>::new(db);
        let mut write = plans
            .begin_write_txn(BinaryDbCommandScope::PlanSyncLocalPlan)
            .unwrap();
        plans
            .append_plan(
                &mut write,
                PlanRecord {
                    plan_meta: 0,
                    reserved0: 0,
                    payload_len: 0,
                    payload_offset: 0,
                    latest_revision_index_plus1: 1,
                    published_plan_index_plus1: 0,
                    published_latest_revision_index_plus1: 0,
                    created_at_s: 41,
                    updated_at_s: 41,
                    published_at_s: 0,
                },
                &PlanPayload {
                    title_bytes: b"legacy empty artifact".to_vec(),
                },
            )
            .unwrap();
        plans
            .append_plan_revision(
                &mut write,
                PlanRevisionRecord {
                    revision_meta: 0,
                    reserved0: 0,
                    payload_len: 0,
                    revision_number: 1,
                    item_count: 0,
                    payload_offset: 0,
                    plan_index: 0,
                    previous_revision_index_plus1: 0,
                    item_start_index: 0,
                    published_revision_index_plus1: 0,
                    root_tree_pack_index_plus1: 0,
                    root_entry_ordinal: 0,
                    created_at_s: 41,
                    published_at_s: 0,
                },
                &PlanRevisionPayload {
                    title_snapshot_bytes: b"legacy empty artifact".to_vec(),
                    summary_bytes: Vec::new(),
                    artifact_path_bytes: b"docs/sprints/legacy.md".to_vec(),
                    artifact_selector_bytes: Vec::new(),
                    artifact_heading_bytes: b"legacy empty artifact".to_vec(),
                    artifact_blob_id_bytes: Vec::new(),
                },
            )
            .unwrap();
        write.commit().unwrap();

        capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
            repo_root: repo.clone(),
            output_root: generation.clone(),
            jobs: 2,
        })
        .unwrap();

        let bytes = fs::read(generation.join("local").join(PLAN_REVISION_BIN)).unwrap();
        let revision = PlanRevisionCodec::<1>::decode_record(&bytes[4..]).unwrap();
        assert!(revision.root_tree_pack_index().is_none());
        assert_eq!(revision.root_entry_ordinal, 0);
        activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo,
            generation_root: generation,
            expected_current_authority_fingerprint: None,
        })
        .unwrap();
    }

    #[test]
    fn capture_streams_live_referenced_packs_into_activatable_generation() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let authority = repo.join(".ait/binary-db");
        let generation = temp.path().join("generation");
        fs::create_dir_all(&authority).unwrap();
        fs::write(
            repo.join(".ait/config.json"),
            serde_json::to_vec(&json!({"repo_name": "fixture"})).unwrap(),
        )
        .unwrap();
        let content = LocalContentBinaryDb::<1>::new(
            authority,
            repo.clone(),
            AuthorityId::new("fixture"),
            LocalStateScope::Repository,
        );
        let blob_id = content
            .ensure_blob_bytes_content(b"captured bytes", Some("fixture.txt"))
            .unwrap();
        fs::write(repo.join("fixture.txt"), b"captured bytes").unwrap();
        content
            .create_no_parent_snapshot_content("fixture", "main", Some("capture"), false)
            .unwrap();

        let report = capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
            repo_root: repo.clone(),
            output_root: generation.clone(),
            jobs: 7,
        })
        .unwrap();
        assert_eq!(report.worker_count, 7);
        assert_eq!(report.source_authority_fingerprint.len(), 64);
        assert!(generation.join("client-manifest.json").is_file());
        assert!(generation.join("local/blob.bin").is_file());
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(generation.join("client-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["source_snapshot_parent_representation"],
            json!("canonical")
        );
        assert_eq!(
            manifest["source_authority_fingerprint"],
            json!(report.source_authority_fingerprint)
        );
        let packs = fs::read_dir(generation.join(".ait/objects/packs"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(packs.len(), 1);

        activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo.clone(),
            generation_root: generation.clone(),
            expected_current_authority_fingerprint: None,
        })
        .unwrap();
        let admitted =
            admit_activated_binary_db_generation(&repo.join(".ait/binary-db"), "fixture").unwrap();
        let activated = LocalContentBinaryDb::<1>::from_db_with_roots(
            LocalBinaryDbFs::new(
                admitted.authority_root,
                repo.clone(),
                AuthorityId::new("fixture"),
                LocalStateScope::Repository,
            )
            .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
            .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS),
            repo,
            admitted.generation_root,
        );
        assert_eq!(
            activated.blobs().read_blob_bytes(&blob_id).unwrap(),
            b"captured bytes".to_vec()
        );
    }

    #[test]
    fn capture_projects_disconnected_snapshot_content_without_mutating_source() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let authority = repo.join(".ait/binary-db");
        let generation = temp.path().join("generation");
        fs::create_dir_all(&authority).unwrap();
        fs::write(
            repo.join(".ait/config.json"),
            serde_json::to_vec(&json!({"repo_name": "fixture"})).unwrap(),
        )
        .unwrap();
        let content = LocalContentBinaryDb::<1>::new(
            authority.clone(),
            repo.clone(),
            AuthorityId::new("fixture"),
            LocalStateScope::Repository,
        );

        fs::write(repo.join("fixture.txt"), b"retained snapshot").unwrap();
        let retained = content
            .create_no_parent_snapshot_content("fixture", "main", Some("retained"), false)
            .unwrap();
        let retained_id = retained["snapshot_id"].as_str().unwrap().to_string();
        fs::write(repo.join("fixture.txt"), b"disconnected snapshot").unwrap();
        content
            .create_no_parent_snapshot_content("fixture", "abandoned", Some("disconnected"), false)
            .unwrap();

        let snapshot_path = authority.join(SNAPSHOT_BIN);
        let mut source_snapshot_bytes = fs::read(&snapshot_path).unwrap();
        let abandoned_meta_offset = 4 + SNAPSHOT_RECORD_SIZE as usize;
        source_snapshot_bytes[abandoned_meta_offset] |= 0b1000_0000;
        fs::write(&snapshot_path, &source_snapshot_bytes).unwrap();
        let source_tree_pack_paths = fs::read_dir(repo.join(".ait/objects/tree-packs"))
            .unwrap()
            .count();
        let source_object_pack_paths = fs::read_dir(repo.join(".ait/objects/packs"))
            .unwrap()
            .count();
        assert_eq!(source_tree_pack_paths, 2);
        assert_eq!(source_object_pack_paths, 2);

        capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
            repo_root: repo.clone(),
            output_root: generation.clone(),
            jobs: 2,
        })
        .unwrap();

        assert_eq!(fs::read(&snapshot_path).unwrap(), source_snapshot_bytes);
        assert_eq!(
            fs::read_dir(generation.join(".ait/objects/tree-packs"))
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            fs::read_dir(generation.join(".ait/objects/packs"))
                .unwrap()
                .count(),
            1
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(generation.join("client-manifest.json")).unwrap())
                .unwrap();
        assert_eq!(
            manifest["content_projection"]["source_tree_pack_records"],
            json!(2)
        );
        assert_eq!(
            manifest["content_projection"]["retained_tree_pack_records"],
            json!(1)
        );
        assert_eq!(
            manifest["content_projection"]["source_object_pack_records"],
            json!(2)
        );
        assert_eq!(
            manifest["content_projection"]["retained_object_pack_records"],
            json!(1)
        );

        activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo.clone(),
            generation_root: generation.clone(),
            expected_current_authority_fingerprint: None,
        })
        .unwrap();
        assert!(!generation.join("local/.locks").exists());
        let activated = LocalContentBinaryDb::<1>::new(
            authority,
            repo,
            AuthorityId::new("activated"),
            LocalStateScope::Repository,
        );
        assert!(activated
            .snapshots()
            .snapshot_by_id(&retained_id)
            .unwrap()
            .is_some());
        let read = activated.snapshots().begin_read_txn();
        assert_eq!(
            activated
                .snapshots()
                .list_snapshot_views(&read)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn capture_repacks_retained_tree_without_unregistered_physical_member() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let authority = repo.join(".ait/binary-db");
        let generation = temp.path().join("generation");
        fs::create_dir_all(&authority).unwrap();
        fs::write(
            repo.join(".ait/config.json"),
            serde_json::to_vec(&json!({"repo_name": "fixture"})).unwrap(),
        )
        .unwrap();

        let db = LocalBinaryDbFs::new(
            authority.clone(),
            repo.clone(),
            AuthorityId::new("fixture"),
            LocalStateScope::Repository,
        );
        let blob_store = BinaryDbBlobStore::<_, 1>::new(db.clone(), repo.clone());
        let object_pack_store = BinaryDbObjectPackStore::<_, 1>::new(db.clone(), repo.clone());
        let tree_pack_store = BinaryDbTreePackStore::<_, 1>::new(db.clone(), repo.clone());
        let tree_store = BinaryDbTreeStore::<_, 1>::new(db.clone(), repo.clone());
        let snapshot_store = BinaryDbSnapshotStore::<_, 1>::new(db.clone(), repo.clone());
        let line_store = BinaryDbLineStore::<_, 1>::new(db.clone());
        line_store
            .create_line("main", None, "2026-07-20T00:00:00Z")
            .unwrap();

        let root_tree_id = tree_id_from_canonical_entries(&[]).unwrap();
        let (unregistered_tree_id, unregistered_entry) = (0_u32..)
            .find_map(|candidate| {
                let entry = BinaryTreeEntryView {
                    entry_ordinal: 0,
                    entry_name: format!("unregistered-{candidate}"),
                    entry_type: "tree".to_string(),
                    target_id: root_tree_id.clone(),
                    size_bytes: None,
                    mode: Some("tree".to_string()),
                };
                let tree_id = tree_id_from_canonical_entries(std::slice::from_ref(&entry)).ok()?;
                (tree_id > root_tree_id).then_some((tree_id, entry))
            })
            .unwrap();
        let tree_ids = [root_tree_id, unregistered_tree_id];
        let tree_hashes = tree_ids
            .iter()
            .map(|tree_id| crate::content_binary_db::tree_id_index_key(tree_id).unwrap())
            .collect::<Vec<_>>();
        let tree_pack_hash = 0x1122_3344_5566_u64;
        let tree_pack_id = tree_pack_id_from_hash48(tree_pack_hash);
        let mut write = tree_store
            .begin_write_txn(BinaryDbCommandScope::ContentWrite)
            .unwrap();
        for (ordinal, tree_hash80) in tree_hashes.into_iter().enumerate() {
            tree_store
                .append_tree_with_id_index(
                    &mut write,
                    &BinaryTreeRecord {
                        tree_meta: 0,
                        reserved0: 0,
                        pack_entry_ordinal: ordinal as u32,
                        entry_count: u32::from(ordinal == 1),
                        tree_hash80,
                    },
                )
                .unwrap();
        }
        tree_pack_store
            .append_tree_pack_with_id_index(
                &mut write,
                &BinaryTreePackRecord {
                    pack_meta: BinaryTreePackRecord::META_READY,
                    pack_format_kind: 1,
                    pack_hash_hi16: ((tree_pack_hash >> 32) & 0xffff) as u16,
                    pack_hash_lo32: tree_pack_hash as u32,
                    first_tree_index: 0,
                    tree_count: 2,
                    total_bytes: 0,
                    created_at_s: 1,
                },
            )
            .unwrap();
        write.commit().unwrap();

        let members = build_tree_pack_members(
            &json!([
                {"tree_id": tree_ids[0], "entry_count": 0},
                {"tree_id": tree_ids[1], "entry_count": 1},
            ]),
            &json!([{
                "tree_id": tree_ids[1],
                "entry_name": unregistered_entry.entry_name,
                "entry_type": "tree",
                "target_id": tree_ids[0],
                "size_bytes": null,
                "mode": "tree",
            }]),
        )
        .unwrap();
        let pack_path = repo.join(default_tree_pack_relative_path(&tree_pack_id));
        crate::pack_substrate::write_tree_pack_archive_with_format(
            pack_path.to_str().unwrap(),
            &tree_pack_id,
            "2026-07-20T00:00:00Z",
            &members,
            crate::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .unwrap();

        let coordinator = BinaryDbContentWriteCoordinator::new(
            &blob_store,
            &object_pack_store,
            &tree_pack_store,
            &tree_store,
            &snapshot_store,
        );
        let snapshot_id = snapshot_id_from_hash48(0x2233_4455_6677);
        coordinator
            .record_snapshot(
                BinaryDbCommandScope::SnapshotWrite,
                &BinaryDbSnapshotWriteInput {
                    snapshot_id: snapshot_id.clone(),
                    parent_snapshot_ids: Vec::new(),
                    root_tree_pack_id: tree_pack_id.clone(),
                    root_entry_ordinal: 0,
                    manifest_hash: "00".repeat(32),
                    message: Some("retained root".to_string()),
                    line_name: "main".to_string(),
                    snapshot_kind: "line".to_string(),
                    file_count: 0,
                    total_bytes: 0,
                    created_at: "2026-07-20T00:00:01Z".to_string(),
                },
            )
            .unwrap();

        capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
            repo_root: repo.clone(),
            output_root: generation.clone(),
            jobs: 2,
        })
        .unwrap();

        let tree_bytes = fs::read(generation.join("local/tree.bin")).unwrap();
        let captured_trees = tree_bytes[4..]
            .chunks_exact(TREE_RECORD_SIZE as usize)
            .map(|raw| BinaryTreeCodec::<1>::decode_record(raw).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(captured_trees.len(), 1);
        assert!(captured_trees.iter().all(|tree| !tree.is_tombstone()));
        assert_eq!(
            tree_id_from_hash80(&captured_trees[0].tree_hash80),
            tree_ids[0]
        );
        assert_eq!(
            fs::read_dir(generation.join(".ait/objects/tree-packs"))
                .unwrap()
                .count(),
            1
        );
        assert!(!generation
            .join(default_tree_pack_relative_path(&tree_pack_id))
            .exists());
        let generated = LocalContentBinaryDb::<1>::new(
            generation.join("local"),
            generation.clone(),
            AuthorityId::new("generated"),
            LocalStateScope::Repository,
        );
        let generated_read = generated.snapshots().begin_read_txn();
        let generated_snapshot = generated
            .snapshots()
            .get_snapshot_view(&generated_read, &snapshot_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            generated_snapshot.root_tree_id.as_deref(),
            Some(tree_ids[0].as_str())
        );
        assert_eq!(generated_snapshot.root_entry_ordinal, 0);
        let generated_packs = generated
            .tree_packs()
            .list_tree_pack_views(&generated_read)
            .unwrap();
        assert_eq!(generated_packs.len(), 1);
        assert_eq!(generated_packs[0].record.tree_count, 1);
        assert_ne!(generated_packs[0].pack_id, tree_pack_id);
        drop(generated_read);
        drop(generated);
        fs::remove_dir_all(generation.join("local/.locks")).unwrap();

        activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo.clone(),
            generation_root: generation,
            expected_current_authority_fingerprint: None,
        })
        .unwrap();
        let activated = LocalContentBinaryDb::<1>::new(
            authority,
            repo,
            AuthorityId::new("activated"),
            LocalStateScope::Repository,
        );
        assert!(activated
            .snapshots()
            .snapshot_by_id(&snapshot_id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn capture_restores_omitted_tree_blob_size_from_blob_authority() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let authority = repo.join(".ait/binary-db");
        let generation = temp.path().join("generation");
        fs::create_dir_all(&authority).unwrap();
        fs::write(
            repo.join(".ait/config.json"),
            serde_json::to_vec(&json!({"repo_name": "fixture"})).unwrap(),
        )
        .unwrap();
        fs::write(repo.join("fixture.txt"), b"root bytes").unwrap();
        let content = LocalContentBinaryDb::<1>::new(
            authority,
            repo.clone(),
            AuthorityId::new("fixture"),
            LocalStateScope::Repository,
        );
        let created = content
            .create_no_parent_snapshot_content("fixture", "main", Some("root"), false)
            .unwrap();
        let snapshot_id = created["snapshot_id"].as_str().unwrap();
        let read = content.snapshots().begin_read_txn();
        let snapshot = content
            .snapshots()
            .get_snapshot_view(&read, snapshot_id)
            .unwrap()
            .unwrap();
        let tree_id = snapshot.root_tree_id.unwrap();
        let pack_id = snapshot.root_tree_pack_id.unwrap();
        let entries = content
            .trees()
            .list_tree_entry_views(&read, &tree_id)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size_bytes, Some(10));
        let blob_id = entries[0].target_id.clone();
        drop(read);

        let legacy_members = build_tree_pack_members(
            &json!([{"tree_id": tree_id, "entry_count": 1}]),
            &json!([{
                "tree_id": tree_id,
                "entry_name": "fixture.txt",
                "entry_type": "blob",
                "target_id": blob_id,
                "size_bytes": null,
                "mode": "0o644",
            }]),
        )
        .unwrap();
        let pack_path = repo.join(default_tree_pack_relative_path(&pack_id));
        write_tree_pack_archive_with_format(
            pack_path.to_str().unwrap(),
            &pack_id,
            "2026-07-20T00:00:00Z",
            &legacy_members,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .unwrap();
        let immutable_source_pack = fs::read(&pack_path).unwrap();

        capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
            repo_root: repo,
            output_root: generation.clone(),
            jobs: 2,
        })
        .unwrap();
        assert_eq!(fs::read(pack_path).unwrap(), immutable_source_pack);

        let generated = LocalContentBinaryDb::<1>::new(
            generation.join("local"),
            generation.clone(),
            AuthorityId::new("generated"),
            LocalStateScope::Repository,
        );
        let generated_read = generated.trees().begin_read_txn();
        let restored = generated
            .trees()
            .list_tree_entry_views(&generated_read, &tree_id)
            .unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].target_id, blob_id);
        assert_eq!(restored[0].size_bytes, Some(10));
        let generated_packs = generated
            .tree_packs()
            .list_tree_pack_views(&generated_read)
            .unwrap();
        assert_eq!(generated_packs.len(), 1);
        assert_ne!(generated_packs[0].pack_id, pack_id);
    }

    #[test]
    fn capture_rejects_undeclared_and_retired_storage_files_without_mutating_source() {
        for name in [
            "undeclared.bin",
            "change_lifecycle.bin",
            "land_target_line.bin",
            "snapshot_parent_edge.bin",
            "snapshot_parent_child.idx",
            "tree_entry.bin",
            "tree_name_payload.bin",
            "workflow_record.bin",
            "workflow_record_payload.bin",
        ] {
            let temp = TempDir::new().unwrap();
            let (repo, authority) = initialize_capture_fixture(&temp);
            let generation = temp.path().join("generation");
            let undeclared_path = authority.join(name);
            let source_bytes = format!("immutable retired bytes for {name}").into_bytes();
            fs::write(&undeclared_path, &source_bytes).unwrap();

            let error = capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
                repo_root: repo,
                output_root: generation.clone(),
                jobs: 1,
            })
            .unwrap_err();

            assert!(
                error.contains(&format!("undeclared file: {name:?}")),
                "unexpected error for {name}: {error}"
            );
            assert_eq!(fs::read(&undeclared_path).unwrap(), source_bytes);
            assert!(!generation.exists());
        }
    }

    #[test]
    fn capture_rejects_retired_root_activation_bit_without_mutating_source() {
        let temp = TempDir::new().unwrap();
        let (repo, authority) = initialize_capture_fixture(&temp);
        let generation = temp.path().join("generation");
        fs::write(repo.join("fixture.txt"), b"root bytes").unwrap();
        let content = LocalContentBinaryDb::<1>::new(
            authority.clone(),
            repo.clone(),
            AuthorityId::new("fixture"),
            LocalStateScope::Repository,
        );
        content
            .create_no_parent_snapshot_content("fixture", "main", Some("root"), false)
            .unwrap();

        let snapshot_path = authority.join(SNAPSHOT_BIN);
        let mut source_bytes = fs::read(&snapshot_path).unwrap();
        source_bytes[4] |=
            crate::content_binary_db::BinarySnapshotRecord::META_HAS_ADDITIONAL_PARENTS;
        fs::write(&snapshot_path, &source_bytes).unwrap();

        let error = capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
            repo_root: repo,
            output_root: generation.clone(),
            jobs: 1,
        })
        .unwrap_err();

        assert!(
            error.contains("cannot decode canonical Snapshot 0"),
            "unexpected error: {error}"
        );
        assert_eq!(fs::read(snapshot_path).unwrap(), source_bytes);
        assert!(!generation.exists());
    }
}
