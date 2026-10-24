//! Native legacy workflow transformation used by whole-generation startup admission.
//! The caller supplies a reviewed, complete source fingerprint and initial-Change
//! cohort; row titles and worktree names never establish conversion authority.
use crate::foundation::remote_binary_db::{
    BinaryDbReadLockSet, FilesystemServerRemoteBinaryDb, RepoId, RepoName,
    ServerBinaryDbFilesystemStore, ServerBinaryDbLockMode, ServerBinaryDbLockStore,
    ServerBinaryDbLockWait, StoreGeneration, StorePath,
};
use crate::foundation::server_binary_db_schema_registry::{
    server_binary_db_fixed_record_size, SERVER_BINARY_DB_BIN_SCHEMAS,
    SERVER_BINARY_DB_INDEX_SCHEMAS,
};
use crate::foundation::server_binary_lifecycle::{canonical_real_directory, sha256};
use crate::foundation::server_content_binary_db::{
    validate_server_snapshot_dag_v0, validate_server_tree_authority_v0,
};
use crate::foundation::workflow_binary_v0::*;
use crate::foundation::workflow_binary_v0_adapter::validate_frozen_server_workflow_v0;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

mod progress;
use progress::report_progress;
pub use progress::{WorkflowUpgradePhase, WorkflowUpgradeProgress};

#[derive(Clone, Debug)]
pub struct WorkflowUpgradeRequest {
    pub source_repository: PathBuf,
    pub staged_repository: PathBuf,
    /// Every immutable source file, keyed by its repository-relative path.
    pub expected_source_files: BTreeMap<String, String>,
    /// Exact admitted bootstrap rows. Other Changes are retained, never inferred.
    pub initial_change_indices: BTreeSet<u32>,
    /// Incoming published refs must be supplied by coordinated source admission.
    pub incoming_change_indices: BTreeSet<u32>,
    /// In-memory, source-bound coordination only; never a stored admission rule.
    pub historical_disposition: WorkflowHistoricalDisposition,
}

/// Explicit historical decisions supplied by native coordination, not inferred
/// from names, ordinals, timestamps, or a persisted runtime switch.
#[derive(Clone, Debug, Default)]
pub struct WorkflowHistoricalDisposition {
    pub legacy_history_patchset_indices: BTreeSet<u32>,
    pub retired_pseudo_land_indices: BTreeSet<u32>,
    pub retired_duplicate_land_indices: BTreeSet<u32>,
    pub retained_terminal_task_indices: BTreeSet<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkflowUpgradeReport {
    pub source_files: BTreeMap<String, String>,
    pub target_files: BTreeMap<String, String>,
    pub change_index_map: Vec<Option<u32>>,
    pub removed_initial_change_indices: BTreeSet<u32>,
}

type Files = BTreeMap<String, Vec<u8>>;

fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(format!("Not a regular authority file: {}", path.display()));
    }
    fs::read(path).map_err(|error| error.to_string())
}
fn write_new_sync(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())
}
fn sync_directory(path: &Path) -> Result<(), String> {
    crate::foundation::remote_binary_db::sync_filesystem_directory(path)
        .map_err(|error| error.to_string())
}

pub fn stage_repository_workflow(
    request: WorkflowUpgradeRequest,
) -> Result<WorkflowUpgradeReport, String> {
    let source = canonical_real_directory(&request.source_repository)?;
    if request.staged_repository.exists() {
        return Err(
            "Workflow conversion target already exists; existing evidence is never overwritten"
                .into(),
        );
    }
    let parent = canonical_real_directory(
        request
            .staged_repository
            .parent()
            .ok_or("Workflow target has no parent")?,
    )?;
    let name = request
        .staged_repository
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Workflow target name is invalid")?;
    let target = parent.join(name);
    if target == source || target.starts_with(&source) || source.starts_with(&target) {
        return Err("Workflow target must be independent of its retained source".into());
    }
    let _freeze = BinaryDbReadLockSet::try_acquire(&StorePath::new(source.clone()))
        .map_err(|error| error.to_string())?;
    let _queue_freeze = if source.join("worker_job.bin").exists() {
        Some(ServerBinaryDbFilesystemStore.acquire_process_lock(
            &source.join(crate::foundation::server_operational_worker_jobs::WORKER_QUEUE_LOCK_FILE_NAME),
            ServerBinaryDbLockMode::Shared,
            ServerBinaryDbLockWait::Nonblocking,
        ).map_err(|error| error.to_string())?.ok_or("Workflow source Worker queue is busy")?)
    } else {
        None
    };
    let source_files = read_files(&source)?;
    let source_fingerprints = fingerprints(&source_files);
    if source_fingerprints != request.expected_source_files {
        return Err("Workflow source file closure/fingerprint differs from exact admission".into());
    }
    history::validate_source_plan_bindings(&source, &source_files)?;
    let (files, change_index_map) = convert_with_disposition(
        &source_files,
        &request.initial_change_indices,
        &request.incoming_change_indices,
        &request.historical_disposition,
    )?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let staging = parent.join(format!(
        ".{name}.workflow-staging-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&staging).map_err(|error| error.to_string())?;
    let result = (|| {
        for (relative, bytes) in &files {
            let path = staging.join(relative);
            fs::create_dir_all(path.parent().ok_or("Staged file has no parent")?)
                .map_err(|error| error.to_string())?;
            write_new_sync(&path, bytes)?;
        }
        let db = FilesystemServerRemoteBinaryDb::serving_authority(
            RepoId::new("workflow-conversion"),
            RepoName::new("workflow-conversion"),
            StorePath::new(staging.clone()),
            StoreGeneration::new(1),
        );
        validate_frozen_server_workflow_v0(&db)?;
        validate_server_snapshot_dag_v0(&db).map_err(|error| error.to_string())?;
        validate_server_tree_authority_v0(&db).map_err(|error| error.to_string())?;
        if read_files(&source)? != source_files {
            return Err("Workflow source changed before inactive publication".into());
        }
        sync_directories(&staging)?;
        if target.exists() {
            return Err("Workflow target appeared during conversion".into());
        }
        fs::rename(&staging, &target).map_err(|error| error.to_string())?;
        sync_directory(&parent)?;
        Ok(WorkflowUpgradeReport {
            source_files: source_fingerprints,
            target_files: fingerprints(&files),
            change_index_map,
            removed_initial_change_indices: request.initial_change_indices,
        })
    })();
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn fingerprints(files: &Files) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(name, bytes)| (name.clone(), sha256(bytes)))
        .collect()
}

fn read_files(root: &Path) -> Result<Files, String> {
    read_authority_files(root, true)
}
fn read_authority_files(root: &Path, include_packs: bool) -> Result<Files, String> {
    fn visit(
        root: &Path,
        relative: &Path,
        files: &mut Files,
        include_packs: bool,
    ) -> Result<(), String> {
        let directory = root.join(relative);
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = relative.join(entry.file_name());
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            // Metadata admission reuses the content root in place. Do not walk
            // immutable packs to prepare or fingerprint a metadata-only batch.
            if !include_packs && relative.as_os_str().is_empty() && entry.file_name() == ".ait" {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err("Workflow content root is not a real directory".into());
                }
                continue;
            }
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Workflow source contains a symlink: {}",
                    path.display()
                ));
            }
            if relative.as_os_str().is_empty()
                && ((entry.file_name() == ".locks" && metadata.is_dir())
                    || (entry.file_name() == crate::foundation::server_operational_worker_jobs::WORKER_QUEUE_LOCK_FILE_NAME && metadata.is_file())) {
                continue;
            }
            if metadata.is_dir() {
                if ![
                    ".ait",
                    ".ait/objects",
                    ".ait/objects/packs",
                    ".ait/objects/tree-packs",
                ]
                .iter()
                .any(|allowed| path == Path::new(allowed))
                {
                    return Err(format!(
                        "Undeclared workflow source directory: {}",
                        path.display()
                    ));
                }
                visit(root, &path, files, include_packs)?;
            } else if metadata.is_file() {
                let name = path
                    .to_str()
                    .ok_or("Workflow source path is not UTF-8")?
                    .to_string();
                if !relative.as_os_str().is_empty() {
                    let prefix = match relative.to_str() {
                        Some(".ait/objects/packs") => "PCK-",
                        Some(".ait/objects/tree-packs") => "TPK-",
                        _ => return Err(format!("Undeclared workflow object path: {name}")),
                    };
                    let file_name = entry.file_name();
                    let hash = file_name
                        .to_str()
                        .and_then(|name| name.strip_prefix(prefix))
                        .and_then(|name| name.strip_suffix(".zstpack"));
                    if hash.is_none_or(|hash| {
                        hash.len() != 12
                            || !hash
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
                    }) {
                        return Err(format!("Undeclared workflow pack path: {name}"));
                    }
                }
                if include_packs || relative.as_os_str().is_empty() {
                    files.insert(name, read_regular_file(&entry.path())?);
                }
            } else {
                return Err("Workflow source contains a non-regular authority path".into());
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, Path::new(""), &mut files, include_packs)?;
    Ok(files)
}

#[cfg(test)]
fn convert(
    source: &Files,
    removed: &BTreeSet<u32>,
    incoming: &BTreeSet<u32>,
) -> Result<(Files, Vec<Option<u32>>), String> {
    convert_with_disposition(
        source,
        removed,
        incoming,
        &WorkflowHistoricalDisposition::default(),
    )
}

fn convert_with_disposition(
    source: &Files,
    removed: &BTreeSet<u32>,
    incoming: &BTreeSet<u32>,
    disposition: &WorkflowHistoricalDisposition,
) -> Result<(Files, Vec<Option<u32>>), String> {
    let expected = SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
        .chain(["snapshot_link_payload.bin"])
        .collect::<BTreeSet<_>>();
    for name in &expected {
        let bytes = source
            .get(*name)
            .ok_or_else(|| format!("Incomplete legacy workflow authority: {name}"))?;
        if bytes.get(..4) != Some(&1u32.to_le_bytes()) {
            return Err(format!(
                "Mixed/unsupported legacy layout at {name}; exact u64 workflow source required"
            ));
        }
        let width = match *name {
            "snapshot_link.bin" => Some(40),
            "task.bin" => Some(60),
            "change.bin" => Some(68),
            "snapshot.bin" => Some(88),
            "land.bin" => Some(48),
            "plan.bin" => Some(48),
            "plan_revision.bin" => Some(56),
            _ => server_binary_db_fixed_record_size(name).map(|width| width as usize),
        };
        if width.is_some_and(|width| (bytes.len() - 4) % width != 0) {
            return Err(format!("Legacy record width mismatch at {name}"));
        }
    }
    for name in source.keys().filter(|name| !name.contains('/')) {
        if !expected.contains(name.as_str())
            && !["worker_job.bin", "worker_ready.idx", "worker_state.idx"].contains(&name.as_str())
        {
            return Err(format!("Undeclared legacy workflow authority: {name}"));
        }
    }
    let operational = ["worker_job.bin", "worker_ready.idx", "worker_state.idx"];
    if operational.iter().any(|name| source.contains_key(*name)) {
        for (name, width) in operational.into_iter().zip([52, 12, 8]) {
            if source.get(name).and_then(|bytes| bytes.get(..4)) != Some(&1u32.to_le_bytes()) {
                return Err(format!("Incomplete or mixed Worker Job authority: {name}"));
            }
            records(source, name, width)?;
        }
        use crate::foundation::operational_binary_v0::ServerOperationalBinaryV0Codec;
        for raw in records(source, "worker_job.bin", 52)? {
            ServerOperationalBinaryV0Codec::decode_worker_job(&raw)
                .map_err(|error| error.to_string())?;
        }
        for raw in records(source, "worker_ready.idx", 12)? {
            ServerOperationalBinaryV0Codec::decode_worker_ready_index(&raw)
                .map_err(|error| error.to_string())?;
        }
        for raw in records(source, "worker_state.idx", 8)? {
            ServerOperationalBinaryV0Codec::decode_worker_state_index(&raw)
                .map_err(|error| error.to_string())?;
        }
    }
    let tasks = records(source, "task.bin", 60)?;
    let old_changes = records(source, "change.bin", 68)?;
    let old_patchsets = records(source, "patchset.bin", 65)?;
    let old_lands = records(source, "land.bin", 48)?;
    let old_links = records(source, "snapshot_link.bin", 40)?;
    let history = history::validate_source_history(source, disposition)?;
    let removed_initial_tasks = removed
        .iter()
        .map(|index| {
            let raw = old_changes
                .get(*index as usize)
                .ok_or("Conversion admission names an unknown Change")?;
            if raw[4] != 0 || u32_at(raw, 20) != 0 {
                return Err(format!(
                    "Change {index} is outside the admitted initial bootstrap shape"
                ));
            }
            Ok(u32_at(raw, 16))
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    let mut changes = Vec::new();
    let mut change_map = Vec::new();
    let mut title_payload = 1u32.to_le_bytes().to_vec();
    let mut source_change_rows = Vec::new();
    let mut title_ranges = Vec::new();
    let mut change_rows = Vec::new();
    let mut last_change = BTreeMap::<u32, u32>::new();
    for (index, raw) in old_changes.iter().enumerate() {
        let task = u32_at(raw, 16);
        let ordinal = raw[4];
        if task as usize >= tasks.len() {
            return Err(format!("Change {index} has an unknown Task"));
        }
        source_change_rows.push((index as u32, task, ordinal, u32_at(raw, 20)));
        let mut raw = raw.clone();
        set_u32(&mut raw, 24, 0);
        WorkflowBinaryV0Codec::decode_change_prefix(&raw).map_err(|error| error.to_string())?;
        let payload = payload(
            source,
            "change_payload.bin",
            u64_at(&raw, 8),
            u16_at(&raw, 2) as usize,
        )?;
        WorkflowBinaryV0Codec::decode_single_text_payload(payload, "Change title")
            .map_err(|error| error.to_string())?;
        title_ranges.push((u64_at(&raw, 8), u16_at(&raw, 2) as u64));
        if removed.contains(&(index as u32)) {
            if ordinal != 0 || u32_at(&raw, 20) != 0 {
                return Err(format!(
                    "Change {index} is outside the admitted initial bootstrap shape"
                ));
            }
            change_map.push(None);
            continue;
        }
        let new_index = changes.len() as u32;
        change_map.push(Some(new_index));
        if removed_initial_tasks.contains(&task) {
            // Only the first old C01 is skipped. Preserve every later revision
            // in order, starting at new C01; current metadata never runs here.
            raw[4] = ordinal
                .checked_sub(1)
                .ok_or("Duplicate initial Change ordinal")?;
        }
        set_u32(&mut raw, 20, *last_change.get(&task).unwrap_or(&0));
        last_change.insert(task, new_index + 1);
        set_u64(&mut raw, 8, title_payload.len() as u64);
        title_payload.extend_from_slice(payload);
        change_rows.push((new_index, task, raw[4], u32_at(&raw, 20)));
        changes.push(raw);
    }
    validate_source_ranges("Change title", &mut title_ranges)?;
    if removed
        .iter()
        .any(|index| *index as usize >= old_changes.len())
        || incoming
            .iter()
            .any(|index| *index as usize >= old_changes.len())
    {
        return Err("Conversion admission names an unknown Change".into());
    }
    let incoming_conflicts = incoming
        .iter()
        .copied()
        .filter(|index| {
            change_map[*index as usize].is_none_or(|new_index| {
                changes[new_index as usize][4] != old_changes[*index as usize][4]
            })
        })
        .collect::<Vec<_>>();
    if !incoming_conflicts.is_empty() {
        return Err(format!("INCOMING_PUBLICATION_REFERENCE_CONFLICT: removed or renumbered Change indices {incoming_conflicts:?}; explicit historical disposition required"));
    }
    check_index(
        source,
        "task_change_index.bin",
        tasks.len(),
        &source_change_rows,
        true,
    )?;
    let owner = |change_index: u32, cached: u8| -> Result<(u32, Option<u32>, u8), String> {
        let change = old_changes
            .get(change_index as usize)
            .ok_or("Legacy workflow owner Change is out of range")?;
        if change[4] != cached {
            return Err("Legacy cached Change ordinal disagrees with numeric owner".into());
        }
        let new_index = change_map[change_index as usize];
        Ok((
            u32_at(change, 16),
            new_index,
            new_index.map_or(0, |index| changes[index as usize][4] + 1),
        ))
    };
    let mut patchsets = Vec::new();
    let mut patch_task_rows = Vec::new();
    let mut patch_old_change_rows = Vec::new();
    let mut patch_change_rows = Vec::new();
    let mut allocated = BTreeSet::new();
    let mut typed_history_patchsets = Vec::new();
    for (index, raw) in old_patchsets.iter().enumerate() {
        let (task, change, cached) = owner(u32_at(raw, 4), raw[2])?;
        if !allocated.insert((task, raw[1])) {
            return Err(format!("UNRESOLVED_TASK_PATCHSET_ORDINAL: Task index {task}, ordinal {}; historical identity disposition required",raw[1]));
        }
        patch_task_rows.push((index as u32, task, raw[1], u32_at(raw, 8)));
        patch_old_change_rows.push((index as u32, u32_at(raw, 4), raw[1], u32_at(raw, 12)));
        let mut raw = raw.clone();
        set_u32(&mut raw, 4, task);
        raw[2] = cached;
        if let Some(change) = change {
            patch_change_rows.push((index as u32, change, raw[1], u32_at(&raw, 12)));
        } else {
            set_u32(&mut raw, 12, 0);
        }
        WorkflowBinaryV0Codec::decode_frozen_patchset(&raw).map_err(|error| error.to_string())?;
        let summary = payload(
            source,
            "patchset_summary_payload.bin",
            u64_at(&raw, 51),
            u16_at(&raw, 59) as usize,
        )?;
        let summary =
            WorkflowBinaryV0Codec::decode_single_text_payload(summary, "Patchset summary")
                .map_err(|error| error.to_string())?;
        if [
            "ait-local-land-receipt/v1 ",
            "ait-history-promotion/v1 ",
            "ait-history-promotion-stage/v1 ",
        ]
        .iter()
        .any(|prefix| summary.starts_with(prefix))
        {
            typed_history_patchsets.push(index as u32);
        }
        patchsets.push(raw);
    }
    check_index(
        source,
        "task_patchset_index.bin",
        tasks.len(),
        &patch_task_rows,
        false,
    )?;
    check_index(
        source,
        "change_patchset_index.bin",
        old_changes.len(),
        &patch_old_change_rows,
        true,
    )?;
    for (index, change) in old_changes.iter().enumerate() {
        let selected = u32_at(change, 24);
        if let Some(selected_index) = selected.checked_sub(1) {
            if old_patchsets
                .get(selected_index as usize)
                .is_none_or(|row| u32_at(row, 4) != index as u32)
            {
                return Err(format!(
                    "Change {index} selected Patchset has a foreign owner"
                ));
            }
        }
        let task = u32_at(change, 16) as usize;
        if tasks[task][0] & (TASK_META_COMPLETED | TASK_META_CANCELED) == 0 {
            let task_latest = index_latest(source, "task_patchset_index.bin", task)?;
            let change_latest = index_latest(source, "change_patchset_index.bin", index)?;
            if selected != 0 && (selected != task_latest || selected != change_latest)
                || change_latest != 0 && selected == 0
            {
                return Err(format!(
                    "UNRESOLVED_SELECTED_CANDIDATE: Task index {task}, Change index {index}"
                ));
            }
        }
    }
    let mut lands = Vec::new();
    let mut land_task_rows = Vec::new();
    let mut land_old_change_rows = Vec::new();
    let mut land_change_rows = Vec::new();
    let mut successful = BTreeMap::<u32, Vec<usize>>::new();
    let mut land_ordinals = BTreeSet::new();
    let mut conflicts = Vec::new();
    if typed_history_patchsets
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        != disposition.legacy_history_patchset_indices
    {
        conflicts.push(format!("typed history Patchset indices {typed_history_patchsets:?} require an exact receipt/reference disposition; immutable manifest identities are not rewritten implicitly"));
    }
    for (index, raw) in old_lands.iter().enumerate() {
        let (task, change, cached) = owner(u32_at(raw, 4), raw[2])?;
        let patchset = old_patchsets
            .get(u32_at(raw, 8) as usize)
            .ok_or("Legacy Land Patchset is out of range")?;
        if u32_at(patchset, 4) != u32_at(raw, 4) {
            return Err("Legacy Land/Patchset exact Change owner disagrees".into());
        }
        if !land_ordinals.insert((task, raw[1])) {
            conflicts.push(format!(
                "Task index {task} has colliding Land ordinal {}",
                raw[1]
            ));
        }
        let retired = disposition
            .retired_pseudo_land_indices
            .contains(&(index as u32))
            || disposition
                .retired_duplicate_land_indices
                .contains(&(index as u32));
        if raw[0] & LAND_STATUS_MASK == LAND_STATUS_SUCCEEDED
            && raw[0] & LAND_TOMBSTONE == 0
            && !retired
        {
            successful.entry(task).or_default().push(index);
            let summary = payload(
                source,
                "patchset_summary_payload.bin",
                u64_at(patchset, 51),
                u16_at(patchset, 59) as usize,
            )?;
            let summary =
                WorkflowBinaryV0Codec::decode_single_text_payload(summary, "Patchset summary")
                    .map_err(|error| error.to_string())?;
            if summary.starts_with("ait-local-land-receipt/v1 ") {
                conflicts.push(format!(
                    "Task index {task}, Land index {index} is an imported local pseudo-success"
                ));
            }
        }
        land_task_rows.push((index as u32, task, raw[1], u32_at(raw, 12)));
        land_old_change_rows.push((index as u32, u32_at(raw, 4), raw[1], u32_at(raw, 16)));
        let mut raw = raw.clone();
        if retired {
            raw[0] |= LAND_TOMBSTONE;
        }
        set_u32(&mut raw, 4, task);
        raw[2] = cached;
        if let Some(change) = change {
            land_change_rows.push((index as u32, change, raw[1], u32_at(&raw, 16)));
        } else {
            set_u32(&mut raw, 16, 0);
        }
        WorkflowBinaryV0Codec::decode_land_prefix(&raw).map_err(|error| error.to_string())?;
        lands.push(raw);
    }
    for (task, rows) in &successful {
        if rows.len() > 1 {
            conflicts.push(format!(
                "Task index {task} has multiple succeeded Land rows {rows:?}"
            ));
        }
    }
    let mut missing_receipts = BTreeSet::new();
    for (task, raw) in tasks.iter().enumerate() {
        if raw[0] & TASK_META_COMPLETED != 0 && !successful.contains_key(&(task as u32)) {
            missing_receipts.insert(task as u32);
        }
    }
    if missing_receipts != disposition.retained_terminal_task_indices {
        conflicts.push(format!("Completed/no-live-receipt Tasks {missing_receipts:?} differ from exact admitted historical terminal membership"));
    }
    history::validate_land_disposition(source, disposition, &history)?;
    if !conflicts.is_empty() {
        return Err(format!(
            "UNRESOLVED_HISTORICAL_DISPOSITION: {}",
            conflicts.join("; ")
        ));
    }
    check_index(
        source,
        "task_land_index.bin",
        tasks.len(),
        &land_task_rows,
        false,
    )?;
    check_index(
        source,
        "change_land_index.bin",
        old_changes.len(),
        &land_old_change_rows,
        true,
    )?;
    let mut links = Vec::new();
    let mut link_task_rows = Vec::new();
    let mut link_old_change_rows = Vec::new();
    let mut link_change_rows = Vec::new();
    let mut link_payload_ranges = Vec::new();
    for (index, raw) in old_links.iter().enumerate() {
        if raw[0] & 0x40 != 0 {
            return Err("Legacy Snapshot Link has reserved metadata".into());
        }
        let task = u32_at(raw, 12);
        let old_change = u32_at(raw, 16).checked_sub(1);
        if (raw[0] & SNAPSHOT_LINK_HAS_CHANGE != 0) != old_change.is_some() {
            return Err("Legacy Snapshot Link Change bit disagrees".into());
        }
        let change = old_change
            .map(|old| {
                let original = old_changes
                    .get(old as usize)
                    .ok_or("Legacy Snapshot Change is out of range")?;
                if u32_at(original, 16) != task {
                    return Err("Legacy Snapshot has a foreign Task/Change".to_string());
                }
                Ok(change_map[old as usize])
            })
            .transpose()?
            .flatten();
        let legacy_payload = payload(
            source,
            "snapshot_link_payload.bin",
            u64_at(raw, 4),
            u16_at(raw, 2) as usize,
        )?;
        validate_retired_payload(legacy_payload, raw[0])?;
        link_payload_ranges.push((u64_at(raw, 4), u16_at(raw, 2) as u64));
        link_task_rows.push((index as u32, task, raw[1], u32_at(raw, 24)));
        if let Some(old) = old_change {
            link_old_change_rows.push((index as u32, old, 0, u32_at(raw, 28)));
        } else if u32_at(raw, 28) != 0 {
            return Err("Task-only legacy Snapshot retains a Change predecessor".into());
        }
        if let Some(change) = change {
            link_change_rows.push((index as u32, change, 0, 0));
        }
        let record = V0SnapshotLinkRecord {
            link_meta: (raw[0] & 0x87) & !SNAPSHOT_LINK_HAS_CHANGE | u8::from(change.is_some()),
            snapshot_ordinal: raw[1],
            task_index: task,
            change_index_plus1: change.map_or(0, |index| index + 1),
            content_snapshot_index: u32_at(raw, 20),
            created_at_s: u64_at(raw, 32),
        };
        links.push(
            WorkflowBinaryV0Codec::encode_snapshot_link(record)
                .map_err(|error| error.to_string())?,
        );
    }
    validate_source_ranges("retired Snapshot Link", &mut link_payload_ranges)?;
    check_index(
        source,
        "task_snapshot_index.bin",
        tasks.len(),
        &link_task_rows,
        true,
    )?;
    check_index(
        source,
        "change_snapshot_index.bin",
        old_changes.len(),
        &link_old_change_rows,
        false,
    )?;
    let mut files = source.clone();
    for (name, mut rows) in [
        ("attest.bin", records(source, "attest.bin", 24)?),
        ("review.bin", records(source, "review.bin", 40)?),
        ("policy.bin", records(source, "policy.bin", 32)?),
        ("waiver.bin", records(source, "waiver.bin", 44)?),
    ] {
        let patchset_offset = if name == "review.bin" { 8 } else { 4 };
        for row in &mut rows {
            let old = old_patchsets
                .get(u32_at(row, patchset_offset) as usize)
                .ok_or("Legacy gate Patchset is out of range")?;
            if row[2] != old[1] || row[3] != old[2] {
                return Err(format!(
                    "Legacy {name} exact Patchset/Change cache disagrees"
                ));
            }
            row[3] = patchsets[u32_at(row, patchset_offset) as usize][2];
        }
        files.insert(name.into(), record_file(2, &rows));
    }
    files.insert("change.bin".into(), record_file(2, &changes));
    files.insert("change_payload.bin".into(), title_payload);
    files.insert("patchset.bin".into(), record_file(2, &patchsets));
    files.insert("land.bin".into(), record_file(2, &lands));
    files.insert("snapshot_link.bin".into(), record_file(2, &links));
    files.remove("snapshot_link_payload.bin");
    files.insert(
        "task_change_index.bin".into(),
        index_file(tasks.len(), &change_rows, true)?,
    );
    files.insert(
        "change_patchset_index.bin".into(),
        index_file(changes.len(), &patch_change_rows, true)?,
    );
    files.insert(
        "change_land_index.bin".into(),
        index_file(changes.len(), &land_change_rows, true)?,
    );
    // No predecessor bytes survive in Link2. The owner index retains physical order.
    let mut link_change_previous = BTreeMap::new();
    for row in &mut link_change_rows {
        row.3 = *link_change_previous.get(&row.1).unwrap_or(&0);
        link_change_previous.insert(row.1, row.0 + 1);
    }
    files.insert(
        "change_snapshot_index.bin".into(),
        index_file(changes.len(), &link_change_rows, false)?,
    );
    let (plans, revisions) = crate::foundation::server_plan_binary_db::remote_actor_profile::convert_remote_plan_metadata(&files["plan.bin"], &files["plan_revision.bin"])
        .map_err(|error| error.to_string())?;
    files.insert("plan.bin".into(), plans);
    files.insert("plan_revision.bin".into(), revisions);
    for family in crate::foundation::remote_actor_metadata::RemoteWorkflowFile::ALL {
        let bytes = crate::foundation::remote_actor_metadata::convert_remote_workflow_file(
            crate::foundation::remote_actor_metadata::RemoteActorProfile::PlanOnly,
            family,
            &files[family.name()],
        )
        .map_err(|error| error.to_string())?;
        if let Some(bytes) = bytes {
            files.insert(family.name().into(), bytes);
        }
    }
    Ok((files, change_map))
}

fn validate_retired_payload(raw: &[u8], meta: u8) -> Result<(), String> {
    if raw.len() < 10 {
        return Err("Truncated retired Snapshot payload".into());
    }
    let lengths = (0..5)
        .map(|field| u16_at(raw, field * 2) as usize)
        .collect::<Vec<_>>();
    let mut cursor = 10;
    for length in &lengths {
        let text = raw
            .get(cursor..cursor + length)
            .ok_or("Truncated retired Snapshot text")?;
        std::str::from_utf8(text).map_err(|error| error.to_string())?;
        cursor += length;
    }
    let model = raw.get(cursor..).ok_or("Truncated retired model text")?;
    std::str::from_utf8(model).map_err(|error| error.to_string())?;
    let flags = if lengths[0] > 0 { 8 } else { 0 }
        | if lengths[1] > 0 { 16 } else { 0 }
        | if lengths[4] > 0 || !model.is_empty() {
            32
        } else {
            0
        };
    if meta & 0x38 != flags {
        return Err("Retired Snapshot payload presence disagrees".into());
    }
    Ok(())
}

fn validate_source_ranges(label: &str, ranges: &mut [(u64, u64)]) -> Result<(), String> {
    ranges.sort_unstable();
    let mut end = 4;
    for &(offset, length) in ranges.iter() {
        if offset < end {
            return Err(format!("Overlapping legacy {label} payload ranges"));
        }
        end = offset
            .checked_add(length)
            .ok_or("Legacy payload range overflow")?;
    }
    Ok(())
}
fn records(files: &Files, name: &str, width: usize) -> Result<Vec<Vec<u8>>, String> {
    let bytes = files.get(name).ok_or_else(|| format!("Missing {name}"))?;
    if bytes.len() < 4 || (bytes.len() - 4) % width != 0 {
        return Err(format!("Misaligned {name}"));
    }
    Ok(bytes[4..]
        .chunks_exact(width)
        .map(|raw| raw.to_vec())
        .collect())
}
fn payload<'a>(
    files: &'a Files,
    name: &str,
    offset: u64,
    length: usize,
) -> Result<&'a [u8], String> {
    let offset = usize::try_from(offset).map_err(|error| error.to_string())?;
    if offset < 4 {
        return Err(format!("Invalid {name} payload offset"));
    }
    files
        .get(name)
        .and_then(|bytes| bytes.get(offset..offset.checked_add(length)?))
        .ok_or_else(|| format!("Invalid {name} payload range"))
}
fn record_file(layout: u32, rows: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = layout.to_le_bytes().to_vec();
    for row in rows {
        bytes.extend_from_slice(row);
    }
    bytes
}
fn u16_at(raw: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(raw[offset..offset + 2].try_into().expect("bounded record"))
}
fn u32_at(raw: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(raw[offset..offset + 4].try_into().expect("bounded record"))
}
fn u64_at(raw: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(raw[offset..offset + 8].try_into().expect("bounded record"))
}
fn set_u32(raw: &mut [u8], offset: usize, value: u32) {
    raw[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn set_u64(raw: &mut [u8], offset: usize, value: u64) {
    raw[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn index_latest(files: &Files, name: &str, owner: usize) -> Result<u32, String> {
    let rows = records(files, name, 8)?;
    rows.get(owner)
        .map(|row| u32_at(row, 0))
        .ok_or_else(|| format!("Missing {name} owner {owner}"))
}
fn check_index(
    files: &Files,
    name: &str,
    count: usize,
    rows: &[(u32, u32, u8, u32)],
    ordinal: bool,
) -> Result<(), String> {
    if files.get(name) != Some(&index_file(count, rows, ordinal)?) {
        return Err(format!("Legacy {name} owner inventory disagrees"));
    }
    Ok(())
}
fn index_file(
    count: usize,
    rows: &[(u32, u32, u8, u32)],
    ordinal: bool,
) -> Result<Vec<u8>, String> {
    let mut groups = vec![Vec::new(); count];
    for row in rows {
        groups
            .get_mut(row.1 as usize)
            .ok_or("Workflow index has a foreign owner")?
            .push(*row);
    }
    let mut output = 1u32.to_le_bytes().to_vec();
    for mut rows in groups {
        if ordinal {
            rows.sort_by_key(|row| row.2);
        } else {
            rows.sort_by_key(|row| row.0);
        }
        let mut previous = 0;
        let mut ordinals = BTreeSet::new();
        for &(physical, _, order, prior) in &rows {
            if prior != previous || ordinal && !ordinals.insert(order) {
                return Err("Legacy owner chain/ordinal disagrees".into());
            }
            previous = physical + 1;
        }
        let count = u16::try_from(rows.len()).map_err(|error| error.to_string())?;
        output.extend_from_slice(&previous.to_le_bytes());
        output.extend_from_slice(&count.to_le_bytes());
        let tail = if ordinal {
            rows.last().map_or(0, |row| row.2.saturating_add(1))
        } else {
            0
        };
        output.push(tail);
        output.push(0);
    }
    Ok(output)
}
fn sync_directories(root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            sync_directories(&entry.path())?;
        }
    }
    sync_directory(root)
}

#[cfg(test)]
#[path = "server_workflow_upgrade_tests.rs"]
mod tests;

mod batch;
pub use batch::{
    admit_workflow_generation, admit_workflow_generation_with_admission,
    admit_workflow_generation_with_progress, validate_workflow_completion,
    workflow_generation_fingerprint, WorkflowGenerationAdmission,
    WORKFLOW_UPGRADE_COMPLETION_SCHEMA,
};

#[path = "server_workflow_upgrade/history.rs"]
mod history;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkflowProfile {
    Legacy,
    Current,
}

fn profile(files: &Files) -> Result<WorkflowProfile, String> {
    use crate::foundation::server_plan_binary_db::remote_actor_profile::{
        classify_remote_plan_metadata, RemotePlanMetadataProfile,
    };
    let plan_profile = classify_remote_plan_metadata(
        files.get("plan.bin").ok_or("Incomplete Plan authority")?,
        files
            .get("plan_revision.bin")
            .ok_or("Incomplete Plan revision authority")?,
    )
    .map_err(|error| error.to_string())?;
    let mut version = None;
    for schema in SERVER_BINARY_DB_BIN_SCHEMAS {
        let bytes = files
            .get(schema.path)
            .ok_or_else(|| format!("Incomplete Repository authority: {}", schema.path))?;
        let header = bytes.get(..4).ok_or("Truncated Repository layout header")?;
        if matches!(schema.path, "plan.bin" | "plan_revision.bin") {
            continue;
        }
        if schema.layout_id == 2 {
            let this = if header == 1u32.to_le_bytes() {
                WorkflowProfile::Legacy
            } else if header == 2u32.to_le_bytes() {
                WorkflowProfile::Current
            } else {
                return Err(format!("Unsupported workflow layout at {}", schema.path));
            };
            if version
                .replace(this)
                .is_some_and(|previous| previous != this)
            {
                return Err(
                    "Partial workflow layout profile; whole-generation admission required".into(),
                );
            }
        } else if header != 1u32.to_le_bytes() {
            return Err(format!(
                "Unchanged authority layout differs at {}",
                schema.path
            ));
        }
    }
    let profile = version.ok_or("No declared workflow profile")?;
    if files.contains_key("snapshot_link_payload.bin") != (profile == WorkflowProfile::Legacy) {
        return Err("Snapshot Link payload/profile closure disagrees".into());
    }
    for schema in SERVER_BINARY_DB_INDEX_SCHEMAS {
        let bytes = files
            .get(schema.path)
            .ok_or("Incomplete Repository index authority")?;
        if bytes.get(..4) != Some(&schema.layout_id.to_le_bytes())
            || (bytes.len() - 4) % schema.record_size as usize != 0
        {
            return Err(format!(
                "Repository index layout/width mismatch at {}",
                schema.path
            ));
        }
    }
    for schema in SERVER_BINARY_DB_BIN_SCHEMAS {
        if let Some(mut width) = server_binary_db_fixed_record_size(schema.path) {
            if plan_profile == RemotePlanMetadataProfile::Legacy {
                if schema.path == "plan.bin" {
                    width = 48;
                }
                if schema.path == "plan_revision.bin" {
                    width = 56;
                }
            }
            if profile == WorkflowProfile::Legacy {
                width = match schema.path {
                    "task.bin" => 60,
                    "change.bin" => 68,
                    "snapshot.bin" => 88,
                    "land.bin" => 48,
                    _ => width,
                };
            }
            if schema.path == "snapshot_link.bin" && profile == WorkflowProfile::Legacy {
                width = 40;
            }
            if !(files[schema.path].len() - 4).is_multiple_of(width as usize) {
                return Err(format!(
                    "Repository record width mismatch at {}",
                    schema.path
                ));
            }
        }
    }
    let expected = SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
        .chain([
            "worker_job.bin",
            "worker_ready.idx",
            "worker_state.idx",
            "snapshot_link_payload.bin",
        ])
        .collect::<BTreeSet<_>>();
    if files
        .keys()
        .any(|name| !name.contains('/') && !expected.contains(name.as_str()))
    {
        return Err("Undeclared Repository file in generation admission".into());
    }
    for (name, width) in [
        ("worker_job.bin", 52),
        ("worker_ready.idx", 12),
        ("worker_state.idx", 8),
    ] {
        let bytes = files
            .get(name)
            .ok_or_else(|| format!("Incomplete operational authority: {name}"))?;
        if bytes.get(..4) != Some(&1u32.to_le_bytes()) || (bytes.len() - 4) % width != 0 {
            return Err(format!(
                "Operational profile layout/width mismatch at {name}"
            ));
        }
    }
    Ok(profile)
}
