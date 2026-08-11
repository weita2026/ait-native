use super::generation_activation::{
    binary_db_activation_lock_root, configured_repo_name, fingerprint_direct_authority_contents,
    validate_generation_file_path, verify_generation, CLIENT_MANIFEST_SCHEMA,
};
use super::generation_content_indexes::{
    is_content_identity_index, rebuild_content_identity_indexes,
};
use super::generation_manifest::write_json;
use super::{GenerationFileManifest, GenerationResult, Path, PathBuf};
use crate::binary_db::{
    AuthorityId, BinaryDbReadLockSet, LocalBinaryDbFs, LocalStateScope,
    REPOSITORY_BINARY_DB_BIN_PATHS, REPOSITORY_BINARY_DB_INDEX_PATHS,
};
use crate::content_binary_db::{object_pack_id_from_hash48, tree_pack_id_from_hash48};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};

pub const U32_TIME_V0_SOURCE_SELECTOR: &str = "u32-time-v0";
pub const U64_SECOND_V0_TARGET_SELECTOR: &str = "u64-second-v0";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageBinaryDbU64SecondUpgradeOptions {
    pub repo_root: PathBuf,
    pub output_root: PathBuf,
    pub source_time_width: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StageBinaryDbU64SecondUpgradeReport {
    #[serde(skip)]
    pub output_root: PathBuf,
    pub repo_name: String,
    pub source_time_width: String,
    pub target_time_width: String,
    pub source_authority_fingerprint: String,
    pub content_fingerprint: String,
    pub converted_file_count: usize,
    pub rebuilt_index_file_count: usize,
    pub copied_file_count: usize,
    pub source_bytes: u64,
    pub target_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
struct TimeWidthPlan {
    name: &'static str,
    source_record_size: usize,
    target_record_size: usize,
    source_time_offsets: &'static [usize],
}

const TIME_WIDTH_PLANS: &[TimeWidthPlan] = &[
    TimeWidthPlan {
        name: "task.bin",
        source_record_size: 44,
        target_record_size: 64,
        source_time_offsets: &[24, 28, 32, 36, 40],
    },
    TimeWidthPlan {
        name: "change.bin",
        source_record_size: 52,
        target_record_size: 68,
        source_time_offsets: &[32, 36, 40, 48],
    },
    TimeWidthPlan {
        name: "land.bin",
        source_record_size: 36,
        target_record_size: 44,
        source_time_offsets: &[24, 28],
    },
    TimeWidthPlan {
        name: "line.bin",
        source_record_size: 28,
        target_record_size: 40,
        source_time_offsets: &[16, 20, 24],
    },
    TimeWidthPlan {
        name: "blob.bin",
        source_record_size: 56,
        target_record_size: 64,
        source_time_offsets: &[16, 20],
    },
    TimeWidthPlan {
        name: "snapshot.bin",
        source_record_size: 84,
        target_record_size: 88,
        source_time_offsets: &[80],
    },
    TimeWidthPlan {
        name: "object_pack.bin",
        source_record_size: 28,
        target_record_size: 32,
        source_time_offsets: &[24],
    },
    TimeWidthPlan {
        name: "tree_pack.bin",
        source_record_size: 28,
        target_record_size: 32,
        source_time_offsets: &[24],
    },
    TimeWidthPlan {
        name: "plan.bin",
        source_record_size: 36,
        target_record_size: 48,
        source_time_offsets: &[24, 28, 32],
    },
    TimeWidthPlan {
        name: "plan_revision.bin",
        source_record_size: 48,
        target_record_size: 56,
        source_time_offsets: &[40, 44],
    },
];

#[derive(Serialize)]
struct UpgradeManifest {
    schema: String,
    label: String,
    source_repo_root: String,
    source_authority_root: String,
    source_authority_fingerprint: String,
    source_time_width: String,
    target_time_width: String,
    worker_count: usize,
    layout_ids: BTreeMap<String, u32>,
    content_fingerprint: String,
    rebuilt_index_file_count: usize,
    files: Vec<GenerationFileManifest>,
    validation: UpgradeValidation,
}

#[derive(Serialize)]
struct UpgradeValidation {
    status: String,
    checks: Vec<String>,
    table_record_counts: BTreeMap<String, u64>,
}

pub fn stage_binary_db_u64_second_upgrade(
    options: StageBinaryDbU64SecondUpgradeOptions,
) -> GenerationResult<StageBinaryDbU64SecondUpgradeReport> {
    if options.source_time_width != U32_TIME_V0_SOURCE_SELECTOR {
        return Err(format!(
            "unsupported Binary DB source time width {:?}; expected exact selector {:?}",
            options.source_time_width, U32_TIME_V0_SOURCE_SELECTOR
        ));
    }
    if options.output_root.exists() {
        return Err(format!(
            "u64-second upgrade output root must not already exist: {}",
            options.output_root.display()
        ));
    }

    let repo_root = options.repo_root.canonicalize().map_err(|error| {
        format!(
            "invalid repository root {}: {error}",
            options.repo_root.display()
        )
    })?;
    let repo_name = configured_repo_name(&repo_root)?;
    let authority_root = repo_root.join(".ait/binary-db");
    let authority_metadata = fs::symlink_metadata(&authority_root).map_err(|error| {
        format!(
            "repository has no Binary DB authority at {}: {error}",
            authority_root.display()
        )
    })?;
    if authority_metadata.file_type().is_symlink() || !authority_metadata.file_type().is_dir() {
        return Err(format!(
            "u64-second upgrade requires a direct Binary DB authority directory: {}",
            authority_root.display()
        ));
    }

    let _activation_guard =
        BinaryDbReadLockSet::try_acquire(&binary_db_activation_lock_root(&repo_root))
            .map_err(|error| format!("Binary DB generation activation is active: {error}"))?;
    let db = LocalBinaryDbFs::new(
        authority_root.clone(),
        repo_root.clone(),
        AuthorityId::new(format!("u64-second-upgrade:{repo_name}")),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
    let read = db.begin_read_txn();
    read.read_lock_paths().map_err(|error| {
        format!("cannot stage u64-second upgrade while a Binary DB writer is active: {error}")
    })?;

    let source_authority_fingerprint = fingerprint_direct_authority_contents(&authority_root)?;
    let pack_paths = referenced_pack_paths(&authority_root)?;
    let source_files = source_authority_files(&authority_root)?;
    let output_parent = options
        .output_root
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_parent).map_err(|error| {
        format!(
            "failed to create u64-second output parent {}: {error}",
            output_parent.display()
        )
    })?;
    let output_name = options
        .output_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "u64-second output root name is not UTF-8".to_string())?;
    let staging_root = output_parent.join(format!(
        ".{output_name}.u64-second-staging-{}",
        std::process::id()
    ));
    if staging_root.exists() {
        return Err(format!(
            "u64-second upgrade staging root already exists: {}",
            staging_root.display()
        ));
    }
    fs::create_dir_all(staging_root.join("local"))
        .and_then(|_| fs::create_dir_all(staging_root.join(".ait/objects/packs")))
        .and_then(|_| fs::create_dir_all(staging_root.join(".ait/objects/tree-packs")))
        .map_err(|error| {
            format!(
                "failed to create u64-second staging root {}: {error}",
                staging_root.display()
            )
        })?;

    let staged = (|| -> GenerationResult<_> {
        let mut files = Vec::new();
        let mut source_bytes = 0_u64;
        let mut target_bytes = 0_u64;
        let mut converted_file_count = 0_usize;
        let mut rebuilt_index_file_count = 0_usize;
        let mut copied_file_count = 0_usize;
        let mut source_content_identity_indexes = BTreeSet::new();

        for (name, source) in source_files {
            let relative_path = format!("local/{name}");
            validate_generation_file_path(&relative_path)?;
            let destination = staging_root.join(&relative_path);
            let source_size = fs::metadata(&source)
                .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?
                .len();
            source_bytes = source_bytes
                .checked_add(source_size)
                .ok_or_else(|| "u64-second source byte count overflow".to_string())?;
            if is_content_identity_index(&name) {
                source_content_identity_indexes.insert(name);
                continue;
            }
            let manifest = if let Some(plan) = time_width_plan(&name) {
                converted_file_count += 1;
                widen_time_record_file(&source, &destination, plan)?
            } else {
                copied_file_count += 1;
                copy_exact_file(&source, &destination, relative_path.clone())?
            };
            target_bytes = target_bytes
                .checked_add(manifest.byte_size)
                .ok_or_else(|| "u64-second target byte count overflow".to_string())?;
            files.push(manifest);
        }

        let rebuilt_content_indexes =
            rebuild_content_identity_indexes(&staging_root.join("local"))?;
        let rebuilt_content_index_names = rebuilt_content_indexes
            .iter()
            .filter_map(|manifest| {
                Path::new(&manifest.relative_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            })
            .collect::<BTreeSet<_>>();
        let orphaned_source_indexes = source_content_identity_indexes
            .difference(&rebuilt_content_index_names)
            .cloned()
            .collect::<Vec<_>>();
        if !orphaned_source_indexes.is_empty() {
            return Err(format!(
                "u32-time-v0 source contains content indexes without authoritative record files: {}",
                orphaned_source_indexes.join(", ")
            ));
        }
        for manifest in rebuilt_content_indexes {
            rebuilt_index_file_count += 1;
            target_bytes = target_bytes
                .checked_add(manifest.byte_size)
                .ok_or_else(|| "u64-second target byte count overflow".to_string())?;
            files.push(manifest);
        }

        for relative_path in pack_paths {
            validate_generation_file_path(&relative_path)?;
            let source = repo_root.join(&relative_path);
            let destination = staging_root.join(&relative_path);
            let manifest = copy_exact_file(&source, &destination, relative_path)?;
            source_bytes = source_bytes
                .checked_add(manifest.byte_size)
                .ok_or_else(|| "u64-second source byte count overflow".to_string())?;
            target_bytes = target_bytes
                .checked_add(manifest.byte_size)
                .ok_or_else(|| "u64-second target byte count overflow".to_string())?;
            copied_file_count += 1;
            files.push(manifest);
        }

        files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let content_fingerprint = fingerprint_files(&files);
        let table_record_counts = files
            .iter()
            .filter_map(|file| {
                file.record_count.map(|count| {
                    let name = Path::new(&file.relative_path)
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or(&file.relative_path)
                        .to_string();
                    (name, count)
                })
            })
            .collect();
        let manifest = UpgradeManifest {
            schema: CLIENT_MANIFEST_SCHEMA.to_string(),
            label: repo_name.clone(),
            source_repo_root: path_text(&repo_root)?,
            source_authority_root: path_text(&authority_root)?,
            source_authority_fingerprint: source_authority_fingerprint.clone(),
            source_time_width: U32_TIME_V0_SOURCE_SELECTOR.to_string(),
            target_time_width: U64_SECOND_V0_TARGET_SELECTOR.to_string(),
            worker_count: 1,
            layout_ids: required_layout_ids(),
            content_fingerprint: content_fingerprint.clone(),
            rebuilt_index_file_count,
            files,
            validation: UpgradeValidation {
                status: "passed".to_string(),
                checks: vec![
                    "exact u32-time-v0 source selector accepted".to_string(),
                    "all source fixed files have layout_id 1 and explicit predecessor widths"
                        .to_string(),
                    "every declared timestamp was zero-extended from little-endian u32 to u64"
                        .to_string(),
                    "every non-index, non-time source byte was preserved in declaration order"
                        .to_string(),
                    "every rebuildable content identity index was regenerated from converted authoritative records using canonical fixed keys"
                        .to_string(),
                    "complete source authority remained read-locked through staging".to_string(),
                    "only authority-referenced canonical pack files were copied exactly"
                        .to_string(),
                    "corrected records and the complete cross-file content closure validated before publication"
                        .to_string(),
                    "still-locked source authority fingerprint matched the staging precondition"
                        .to_string(),
                ],
                table_record_counts,
            },
        };
        write_json(&staging_root.join("client-manifest.json"), &manifest)?;
        sync_directory(&staging_root.join("local"))?;
        sync_directory(&staging_root.join(".ait/objects/packs"))?;
        sync_directory(&staging_root.join(".ait/objects/tree-packs"))?;
        sync_directory(&staging_root.join(".ait/objects"))?;
        sync_directory(&staging_root.join(".ait"))?;
        sync_directory(&staging_root)?;
        verify_generation(&staging_root, &repo_name)?;
        let final_source_fingerprint = fingerprint_direct_authority_contents(&authority_root)?;
        if final_source_fingerprint != source_authority_fingerprint {
            return Err(format!(
                "Binary DB source authority changed during u64-second staging: expected {source_authority_fingerprint}, found {final_source_fingerprint}"
            ));
        }
        Ok((
            content_fingerprint,
            converted_file_count,
            rebuilt_index_file_count,
            copied_file_count,
            source_bytes,
            target_bytes,
        ))
    })();

    let (
        content_fingerprint,
        converted_file_count,
        rebuilt_index_file_count,
        copied_file_count,
        source_bytes,
        target_bytes,
    ) = match staged {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging_root);
            return Err(error);
        }
    };
    drop(read);

    if options.output_root.exists() {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(format!(
            "u64-second output root appeared before publication: {}",
            options.output_root.display()
        ));
    }
    fs::rename(&staging_root, &options.output_root).map_err(|error| {
        let _ = fs::remove_dir_all(&staging_root);
        format!(
            "failed to atomically publish u64-second generation {}: {error}",
            options.output_root.display()
        )
    })?;
    sync_directory(output_parent)?;

    Ok(StageBinaryDbU64SecondUpgradeReport {
        output_root: options.output_root,
        repo_name,
        source_time_width: U32_TIME_V0_SOURCE_SELECTOR.to_string(),
        target_time_width: U64_SECOND_V0_TARGET_SELECTOR.to_string(),
        source_authority_fingerprint,
        content_fingerprint,
        converted_file_count,
        rebuilt_index_file_count,
        copied_file_count,
        source_bytes,
        target_bytes,
    })
}

fn source_authority_files(authority_root: &Path) -> GenerationResult<Vec<(String, PathBuf)>> {
    let mut entries = fs::read_dir(authority_root)
        .map_err(|error| format!("failed to inventory {}: {error}", authority_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inventory {}: {error}", authority_root.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_dir() && entry.file_name() == ".locks" {
            continue;
        }
        if !file_type.is_file() {
            return Err(format!(
                "Binary DB source authority contains an undeclared non-file path: {}",
                path.display()
            ));
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("Binary DB source path is not UTF-8: {}", path.display()))?;
        let allowed = if name.ends_with(".bin") {
            REPOSITORY_BINARY_DB_BIN_PATHS.contains(&name.as_str())
        } else if name.ends_with(".idx") {
            REPOSITORY_BINARY_DB_INDEX_PATHS.contains(&name.as_str())
        } else {
            false
        };
        if !allowed {
            return Err(format!(
                "Binary DB source authority contains an undeclared file: {name:?}"
            ));
        }
        validate_layout_one_header(&path)?;
        files.push((name, path));
    }
    Ok(files)
}

fn referenced_pack_paths(authority_root: &Path) -> GenerationResult<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    for (name, directory, tree) in [
        ("object_pack.bin", "packs", false),
        ("tree_pack.bin", "tree-packs", true),
    ] {
        let path = authority_root.join(name);
        if !path.is_file() {
            continue;
        }
        let plan = time_width_plan(name).ok_or_else(|| format!("missing width plan for {name}"))?;
        let mut file = fs::File::open(&path)
            .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
        let mut header = [0_u8; 4];
        file.read_exact(&mut header)
            .map_err(|error| format!("failed to read {} header: {error}", path.display()))?;
        if u32::from_le_bytes(header) != 1 {
            return Err(format!("{name} does not have layout_id 1"));
        }
        let body_size = fs::metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
            .len()
            .checked_sub(4)
            .ok_or_else(|| format!("{name} is shorter than its layout header"))?;
        let record_size = plan.source_record_size;
        if body_size % record_size as u64 != 0 {
            return Err(format!(
                "{name} is not aligned to explicit {record_size}-byte records"
            ));
        }
        let mut raw = vec![0_u8; record_size];
        for index in 0..body_size / record_size as u64 {
            file.read_exact(&mut raw).map_err(|error| {
                format!("failed to read {name} record {index} for pack closure: {error}")
            })?;
            if raw[0] & 0b1000_0000 != 0 {
                continue;
            }
            let hi = u16::from_le_bytes(raw[2..4].try_into().unwrap());
            let lo = u32::from_le_bytes(raw[4..8].try_into().unwrap());
            let hash48 = (u64::from(hi) << 32) | u64::from(lo);
            let id = if tree {
                tree_pack_id_from_hash48(hash48)
            } else {
                object_pack_id_from_hash48(hash48)
            };
            paths.insert(format!(".ait/objects/{directory}/{id}.zstpack"));
        }
    }
    Ok(paths)
}

fn time_width_plan(name: &str) -> Option<TimeWidthPlan> {
    TIME_WIDTH_PLANS
        .iter()
        .find(|plan| plan.name == name)
        .copied()
}

fn widen_time_record_file(
    source: &Path,
    destination: &Path,
    plan: TimeWidthPlan,
) -> GenerationResult<GenerationFileManifest> {
    let mut input = fs::File::open(source)
        .map_err(|error| format!("failed to open {}: {error}", source.display()))?;
    let mut header = [0_u8; 4];
    input
        .read_exact(&mut header)
        .map_err(|error| format!("failed to read {} header: {error}", source.display()))?;
    if u32::from_le_bytes(header) != 1 {
        return Err(format!(
            "{} source layout is not layout_id 1",
            source.display()
        ));
    }
    validate_time_width_plan(plan)?;
    let body_size = fs::metadata(source)
        .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?
        .len()
        .checked_sub(4)
        .ok_or_else(|| format!("{} is shorter than its layout header", source.display()))?;
    if body_size == 0 || body_size % plan.source_record_size as u64 != 0 {
        return Err(format!(
            "{} is not a non-empty explicit {}-byte u32-time-v0 record file",
            plan.name, plan.source_record_size
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    output
        .write_all(&header)
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(header);
    let record_count = body_size / plan.source_record_size as u64;
    let mut raw = vec![0_u8; plan.source_record_size];
    for index in 0..record_count {
        input.read_exact(&mut raw).map_err(|error| {
            format!(
                "failed to read {} record {index}: {error}",
                source.display()
            )
        })?;
        let widened = widen_time_record(&raw, plan)?;
        output
            .write_all(&widened)
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
        hasher.update(&widened);
    }
    output
        .sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", destination.display()))?;
    let byte_size = 4_u64
        .checked_add(
            record_count
                .checked_mul(plan.target_record_size as u64)
                .ok_or_else(|| format!("{} target byte count overflow", plan.name))?,
        )
        .ok_or_else(|| format!("{} target byte count overflow", plan.name))?;
    Ok(GenerationFileManifest {
        relative_path: format!("local/{}", plan.name),
        byte_size,
        sha256: hex_lower(&hasher.finalize()),
        record_count: Some(record_count),
    })
}

fn widen_time_record(raw: &[u8], plan: TimeWidthPlan) -> GenerationResult<Vec<u8>> {
    if raw.len() != plan.source_record_size {
        return Err(format!(
            "{} source record has {} bytes, expected {}",
            plan.name,
            raw.len(),
            plan.source_record_size
        ));
    }
    validate_time_width_plan(plan)?;
    let mut output = Vec::with_capacity(plan.target_record_size);
    let mut cursor = 0_usize;
    for offset in plan.source_time_offsets {
        output.extend_from_slice(&raw[cursor..*offset]);
        let seconds = u32::from_le_bytes(raw[*offset..*offset + 4].try_into().unwrap());
        output.extend_from_slice(&u64::from(seconds).to_le_bytes());
        cursor = *offset + 4;
    }
    output.extend_from_slice(&raw[cursor..]);
    if output.len() != plan.target_record_size {
        return Err(format!(
            "{} widened record has {} bytes, expected {}",
            plan.name,
            output.len(),
            plan.target_record_size
        ));
    }
    Ok(output)
}

fn validate_time_width_plan(plan: TimeWidthPlan) -> GenerationResult<()> {
    let mut previous_end = 0_usize;
    for offset in plan.source_time_offsets {
        if *offset < previous_end || offset.saturating_add(4) > plan.source_record_size {
            return Err(format!(
                "{} has an invalid timestamp offset {offset}",
                plan.name
            ));
        }
        previous_end = *offset + 4;
    }
    let expected_target = plan
        .source_record_size
        .checked_add(plan.source_time_offsets.len() * 4)
        .ok_or_else(|| format!("{} target record size overflow", plan.name))?;
    if expected_target != plan.target_record_size {
        return Err(format!(
            "{} target record size {} disagrees with {} declared timestamps",
            plan.name,
            plan.target_record_size,
            plan.source_time_offsets.len()
        ));
    }
    Ok(())
}

fn copy_exact_file(
    source: &Path,
    destination: &Path,
    relative_path: String,
) -> GenerationResult<GenerationFileManifest> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "upgrade input is not a regular file: {}",
            source.display()
        ));
    }
    let mut input = fs::File::open(source)
        .map_err(|error| format!("failed to open {}: {error}", source.display()))?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut hasher = Sha256::new();
    let mut byte_size = 0_u64;
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
        hasher.update(&buffer[..count]);
        byte_size = byte_size
            .checked_add(count as u64)
            .ok_or_else(|| format!("file size overflow: {}", source.display()))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", destination.display()))?;
    Ok(GenerationFileManifest {
        record_count: target_record_count(&relative_path, byte_size)?,
        relative_path,
        byte_size,
        sha256: hex_lower(&hasher.finalize()),
    })
}

fn target_record_count(relative_path: &str, byte_size: u64) -> GenerationResult<Option<u64>> {
    let Some(name) = relative_path.strip_prefix("local/") else {
        return Ok(None);
    };
    let record_size = match name {
        "task_change_index.bin" | "task_land_index.bin" | "change_land_index.bin" => 8,
        "plan_item.bin" => 16,
        "object_pack_member.bin" => 16,
        "tree.bin" => 20,
        "stash.bin" => 8,
        _ => return Ok(None),
    };
    let body = byte_size
        .checked_sub(4)
        .ok_or_else(|| format!("{relative_path} is shorter than its layout header"))?;
    if body % record_size != 0 {
        return Err(format!(
            "{relative_path} is misaligned for {record_size}-byte records"
        ));
    }
    Ok(Some(body / record_size))
}

fn validate_layout_one_header(path: &Path) -> GenerationResult<()> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut header = [0_u8; 4];
    file.read_exact(&mut header)
        .map_err(|error| format!("failed to read {} layout header: {error}", path.display()))?;
    let layout = u32::from_le_bytes(header);
    if layout != 1 {
        return Err(format!(
            "u64-second upgrade requires layout_id 1, found {layout} at {}",
            path.display()
        ));
    }
    Ok(())
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

    #[test]
    fn every_predecessor_width_plan_is_exact() {
        for plan in TIME_WIDTH_PLANS {
            validate_time_width_plan(*plan).unwrap();
        }
    }

    #[test]
    fn widening_zero_extends_each_time_and_preserves_every_other_byte() {
        for plan in TIME_WIDTH_PLANS {
            let mut source = (0..plan.source_record_size)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>();
            for (ordinal, offset) in plan.source_time_offsets.iter().enumerate() {
                let value = if ordinal == 0 {
                    u32::MAX
                } else {
                    0x0102_0304_u32.wrapping_add(ordinal as u32)
                };
                source[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            let target = widen_time_record(&source, *plan).unwrap();
            let mut source_cursor = 0_usize;
            let mut target_cursor = 0_usize;
            for offset in plan.source_time_offsets {
                let unchanged = offset - source_cursor;
                assert_eq!(
                    &target[target_cursor..target_cursor + unchanged],
                    &source[source_cursor..*offset],
                    "{} changed non-time bytes",
                    plan.name
                );
                target_cursor += unchanged;
                let source_time =
                    u32::from_le_bytes(source[*offset..*offset + 4].try_into().unwrap());
                let target_time = u64::from_le_bytes(
                    target[target_cursor..target_cursor + 8].try_into().unwrap(),
                );
                assert_eq!(target_time, u64::from(source_time), "{} time", plan.name);
                source_cursor = *offset + 4;
                target_cursor += 8;
            }
            assert_eq!(&target[target_cursor..], &source[source_cursor..]);
            assert_eq!(target.len(), plan.target_record_size);
        }
    }

    #[test]
    fn source_selector_is_exact_and_stable() {
        assert_eq!(U32_TIME_V0_SOURCE_SELECTOR, "u32-time-v0");
        assert_eq!(U64_SECOND_V0_TARGET_SELECTOR, "u64-second-v0");
    }

    #[test]
    fn staging_rejects_every_non_exact_source_selector_before_reading_authority() {
        for selector in ["", "u32", "u32-time", "u64-second-v0"] {
            let error = stage_binary_db_u64_second_upgrade(StageBinaryDbU64SecondUpgradeOptions {
                repo_root: PathBuf::from("does-not-exist"),
                output_root: PathBuf::from("does-not-exist-output"),
                source_time_width: selector.to_string(),
            })
            .unwrap_err();
            assert!(
                error.contains("expected exact selector"),
                "{selector:?}: {error}"
            );
        }
    }

    #[test]
    fn file_conversion_rejects_wrong_layout_and_misaligned_predecessor_without_output() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = temp.path().join("task.bin");
        let target = temp.path().join("target-task.bin");
        let plan = TIME_WIDTH_PLANS
            .iter()
            .find(|plan| plan.name == "task.bin")
            .copied()
            .unwrap();

        let mut wrong_layout = 2_u32.to_le_bytes().to_vec();
        wrong_layout.extend(vec![0_u8; plan.source_record_size]);
        fs::write(&source, wrong_layout).unwrap();
        assert!(widen_time_record_file(&source, &target, plan)
            .unwrap_err()
            .contains("source layout is not layout_id 1"));
        assert!(!target.exists());

        let mut misaligned = 1_u32.to_le_bytes().to_vec();
        misaligned.extend(vec![0_u8; plan.source_record_size - 1]);
        fs::write(&source, misaligned).unwrap();
        assert!(widen_time_record_file(&source, &target, plan)
            .unwrap_err()
            .contains("non-empty explicit 44-byte u32-time-v0"));
        assert!(!target.exists());
    }
}
