use crate::activation::{
    canonical_real_directory, read_regular_file, sha256, sync_directory, write_new_sync,
    write_upgrade_completion,
};
use crate::conversion::{inventory_authority_files, FileReport};
use crate::recovery_audit::{audit_generation, authority_fingerprint, AuditGenerationRequest};
use ait_server_core::foundation::operational_binary_v0::{
    ServerOperationalBinaryV0Codec, ServerWorkerReadyIndexRecord, OPERATIONAL_V0_LAYOUT_ID,
    SERVER_GLOBAL_OPERATIONAL_BIN_PATHS, SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS,
    SERVER_REPOSITORY_OPERATIONAL_BIN_PATHS, SERVER_REPOSITORY_OPERATIONAL_INDEX_PATHS,
    SERVER_WORKER_JOB_RECORD_SIZE, WORKER_JOB_STATE_QUEUED,
};
use ait_server_core::foundation::remote_binary_db::{
    BinaryDbReadLockSet, BoxedServerBinaryDbProcessLockGuard, ServerBinaryDbFilesystemStore,
    ServerBinaryDbLockMode, ServerBinaryDbLockStore, ServerBinaryDbLockWait, StorePath,
};
use ait_server_core::foundation::server_binary_db_schema_registry::{
    server_binary_db_fixed_record_size, SERVER_BINARY_DB_BIN_SCHEMAS,
    SERVER_BINARY_DB_INDEX_SCHEMAS,
};
use ait_server_core::foundation::server_operational_repository_registry::REGISTRY_LOCK_FILE_NAME;
use ait_server_core::foundation::server_operational_worker_jobs::WORKER_QUEUE_LOCK_FILE_NAME;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const U32_TIME_V0_SOURCE_SELECTOR: &str = "u32-time-v0";
pub const U64_SECOND_V0_TARGET_SELECTOR: &str = "u64-second-v0";
pub const U64_SECOND_UPGRADE_REPORT_SCHEMA: &str =
    "ait.server.binary_v0.u64_second_upgrade.report.v1";
pub const U64_SECOND_UPGRADE_COMPLETION_SCHEMA: &str =
    "ait.server.binary_v0.u64_second_upgrade.complete.v1";

const GENERATION_SCHEMA: &str = "ait.server.binary_v0.operational_generation.v1";
const POSTGRES_COMPLETION_SCHEMA: &str = "ait.server.postgres_to_binary_v0.complete.v1";
const POSTGRES_REPORT_SCHEMA: &str = "ait.server.postgres_to_binary_v0.report.v1";
const FRESH_COMPLETION_SCHEMA: &str = "ait.server.binary_v0.fresh.complete.v1";
const GENERATION_FILE: &str = "generation.json";
const CONVERSION_REPORT_FILE: &str = "conversion-report.json";
const CONVERSION_COMPLETION_FILE: &str = "conversion-complete.json";
const FRESH_COMPLETION_FILE: &str = "generation-complete.json";
// Historical generations produced by the retired incident replay may contain
// this immutable receipt. The upgrader can copy and seal it, but no replay
// writer remains in the importer.
const LEGACY_FRESH_REPLAY_RECEIPT_FILE: &str = "fresh-generation-replay-receipt.json";
const LIFECYCLE_LOCK_FILE: &str = "lifecycle.lock";

#[derive(Clone, Debug)]
pub struct UpgradeU64SecondsRequest {
    pub source_selector: String,
    pub source_generation: PathBuf,
    pub staged_generation: PathBuf,
    pub report_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeU64SecondsResult {
    pub repository_count: u32,
    pub task_count: u64,
    pub worker_job_count: u64,
    pub source_bytes: u64,
    pub target_bytes: u64,
    pub source_fingerprint: String,
    pub target_fingerprint: String,
    pub staged_generation: PathBuf,
    pub report_path: PathBuf,
}

#[derive(Clone, Copy, Debug)]
struct TimeWidthPlan {
    name: &'static str,
    source_record_size: usize,
    target_record_size: usize,
    source_time_offsets: &'static [usize],
}

const TIME_WIDTH_PLANS: &[TimeWidthPlan] = &[
    plan("repository.bin", 25, 33, &[17, 21]),
    plan("task.bin", 40, 60, &[20, 24, 28, 32, 36]),
    plan("change.bin", 52, 68, &[32, 36, 40, 48]),
    plan("patchset.bin", 57, 65, &[24, 28]),
    plan("attest.bin", 20, 24, &[16]),
    plan("actor.bin", 28, 36, &[20, 24]),
    plan("review.bin", 36, 40, &[32]),
    plan("policy.bin", 28, 32, &[24]),
    plan("land.bin", 40, 48, &[28, 32]),
    plan("snapshot_link.bin", 36, 40, &[32]),
    plan("waiver.bin", 36, 44, &[28, 32]),
    plan("line.bin", 28, 40, &[16, 20, 24]),
    plan("snapshot.bin", 84, 88, &[80]),
    plan("blob.bin", 56, 64, &[16, 20]),
    plan("object_pack.bin", 28, 32, &[24]),
    plan("tree_pack.bin", 28, 32, &[24]),
    plan("plan.bin", 36, 48, &[24, 28, 32]),
    plan("plan_revision.bin", 48, 56, &[40, 44]),
    plan("worker_job.bin", 36, 52, &[20, 24, 28, 32]),
];

const fn plan(
    name: &'static str,
    source_record_size: usize,
    target_record_size: usize,
    source_time_offsets: &'static [usize],
) -> TimeWidthPlan {
    TimeWidthPlan {
        name,
        source_record_size,
        target_record_size,
        source_time_offsets,
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationManifest {
    schema: String,
    layout_id: u32,
    status: String,
    global_registry: String,
    repository_authorities: String,
    repository_count: u32,
}

#[derive(Debug, Deserialize)]
struct SourceCompletion {
    schema: String,
    layout_id: u32,
    status: String,
    report_sha256: Option<String>,
}

#[derive(Debug, Serialize)]
struct UpgradeReport {
    schema: &'static str,
    layout_id: u32,
    status: &'static str,
    source_selector: &'static str,
    target_selector: &'static str,
    source_generation: String,
    source_authority_fingerprint: String,
    target_authority_fingerprint: String,
    source_bytes: u64,
    target_bytes: u64,
    converted_file_count: usize,
    copied_file_count: usize,
    rebuilt_worker_ready_index_count: usize,
    repository_count: u32,
    task_count: u64,
    worker_job_count: u64,
    authority_files: Vec<FileReport>,
    validation: UpgradeValidation,
}

#[derive(Debug, Serialize)]
struct UpgradeValidation {
    status: &'static str,
    checks: Vec<&'static str>,
    record_counts: BTreeMap<String, u64>,
}

struct SourceFreeze {
    _lifecycle: File,
    _registry: BoxedServerBinaryDbProcessLockGuard,
    _repository_reads: Vec<BinaryDbReadLockSet>,
    _worker_queues: Vec<BoxedServerBinaryDbProcessLockGuard>,
}

pub fn upgrade_u64_seconds(
    request: UpgradeU64SecondsRequest,
) -> Result<UpgradeU64SecondsResult, String> {
    if request.source_selector != U32_TIME_V0_SOURCE_SELECTOR {
        return Err(format!(
            "unsupported Binary DB source selector {:?}; expected exact selector {:?}",
            request.source_selector, U32_TIME_V0_SOURCE_SELECTOR
        ));
    }
    if request.staged_generation.exists() {
        return Err(format!(
            "u64-second staged generation must not already exist: {}",
            request.staged_generation.display()
        ));
    }

    let source_generation = canonical_real_directory(&request.source_generation)?;
    let generations_root = source_generation
        .parent()
        .ok_or_else(|| "source generation has no generations parent".to_string())?;
    let output_parent = request
        .staged_generation
        .parent()
        .ok_or_else(|| "staged generation has no parent".to_string())?;
    let output_parent = canonical_real_directory(output_parent)?;
    if output_parent != generations_root {
        return Err("u64-second target must be a sibling inactive generation".to_string());
    }
    let report_path = absolute_new_file_path(&request.report_path)?;
    if report_path.starts_with(generations_root) {
        return Err("u64-second external report must stay outside generation roots".to_string());
    }

    let manifest = read_generation_manifest(&source_generation)?;
    let _freeze = SourceFreeze::acquire(&source_generation, manifest.repository_count, false)?;
    validate_source_evidence(&source_generation)?;
    let source_files = validate_source_inventory(&source_generation, &manifest)?;
    let source_fingerprint = authority_fingerprint(&source_generation)?;

    let output_name = request
        .staged_generation
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "staged generation name is not UTF-8".to_string())?;
    let nonce = unique_nonce()?;
    let staging = output_parent.join(format!(
        ".{output_name}.u64-second-staging-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&staging)
        .map_err(|error| format!("failed to create {}: {error}", staging.display()))?;

    let staged = (|| -> Result<_, String> {
        create_target_directories(&staging, manifest.repository_count)?;
        let mut source_bytes = 0_u64;
        let mut converted_file_count = 0_usize;
        let mut copied_file_count = 0_usize;
        let mut rebuilt_worker_ready_index_count = 0_usize;

        for relative in &source_files {
            let source = source_generation.join(relative);
            let source_size = fs::metadata(&source)
                .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?
                .len();
            source_bytes = source_bytes
                .checked_add(source_size)
                .ok_or_else(|| "u64-second source byte count overflow".to_string())?;
            if relative.file_name().and_then(|value| value.to_str()) == Some("worker_ready.idx") {
                validate_layout_and_alignment(&source, 8, "u32-time-v0 Worker ready index")?;
                rebuilt_worker_ready_index_count += 1;
                continue;
            }
            let destination = staging.join(relative);
            let name = relative
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| format!("source path is not UTF-8: {}", relative.display()))?;
            if is_record_authority_path(relative) {
                if let Some(width) = time_width_plan(name) {
                    widen_time_record_file(&source, &destination, width)?;
                    converted_file_count += 1;
                    continue;
                }
            }
            copy_exact_file(&source, &destination)?;
            copied_file_count += 1;
        }

        for repository_index in 0..manifest.repository_count {
            rebuild_worker_ready_index(
                &staging
                    .join("repositories")
                    .join(repository_index.to_string()),
            )?;
        }
        converted_file_count = converted_file_count
            .checked_add(rebuilt_worker_ready_index_count)
            .ok_or_else(|| "converted file count overflow".to_string())?;

        sync_tree_directories(&staging)?;
        let audit_path = output_parent.join(format!(
            ".{output_name}.u64-second-audit-{}-{nonce}.json",
            std::process::id()
        ));
        let audit = audit_generation(AuditGenerationRequest {
            generation: staging.clone(),
            report_path: audit_path.clone(),
        });
        let _ = fs::remove_file(&audit_path);
        let audit = audit?;

        let final_source_fingerprint = authority_fingerprint(&source_generation)?;
        if final_source_fingerprint != source_fingerprint {
            return Err(format!(
                "Binary DB source changed during u64-second staging: expected {source_fingerprint}, found {final_source_fingerprint}"
            ));
        }

        let authority_files = inventory_authority_files(&staging)?;
        let target_bytes = authority_files.iter().try_fold(0_u64, |total, file| {
            total
                .checked_add(file.byte_size)
                .ok_or_else(|| "u64-second target byte count overflow".to_string())
        })?;
        let record_counts = target_record_counts(&authority_files)?;
        let report = UpgradeReport {
            schema: U64_SECOND_UPGRADE_REPORT_SCHEMA,
            layout_id: 1,
            status: "validated_inactive",
            source_selector: U32_TIME_V0_SOURCE_SELECTOR,
            target_selector: U64_SECOND_V0_TARGET_SELECTOR,
            source_generation: path_text(&source_generation)?,
            source_authority_fingerprint: source_fingerprint.clone(),
            target_authority_fingerprint: audit.source_fingerprint.clone(),
            source_bytes,
            target_bytes,
            converted_file_count,
            copied_file_count,
            rebuilt_worker_ready_index_count,
            repository_count: audit.repository_count,
            task_count: audit.task_count,
            worker_job_count: audit.worker_job_count,
            authority_files,
            validation: UpgradeValidation {
                status: "passed",
                checks: vec![
                    "exact u32-time-v0 source selector and complete predecessor evidence accepted",
                    "global registry and every dense numeric Repository authority read-locked",
                    "all declared layout_id 1 files matched explicit predecessor widths",
                    "every persisted normal Unix second was zero-extended from u32 to u64",
                    "all unaffected authority bytes and pack archives were copied exactly",
                    "Worker ready indexes were rebuilt from converted Worker Job authority",
                    "active codecs and complete cross-file relationships validated inactive",
                    "source authority fingerprint remained unchanged while locks were held",
                ],
                record_counts,
            },
        };
        let mut report_bytes = serde_json::to_vec_pretty(&report)
            .map_err(|error| format!("failed to encode u64-second report: {error}"))?;
        report_bytes.push(b'\n');
        write_new_sync(&staging.join(CONVERSION_REPORT_FILE), &report_bytes)?;
        write_upgrade_completion(&staging, sha256(&report_bytes))?;
        sync_tree_directories(&staging)?;
        Ok((audit, source_bytes, target_bytes, report_bytes))
    })();

    let (audit, source_bytes, target_bytes, report_bytes) = match staged {
        Ok(value) => value,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    if request.staged_generation.exists() {
        let _ = fs::remove_dir_all(&staging);
        return Err(format!(
            "u64-second output appeared before publication: {}",
            request.staged_generation.display()
        ));
    }
    fs::rename(&staging, &request.staged_generation).map_err(|error| {
        let _ = fs::remove_dir_all(&staging);
        format!(
            "failed to atomically publish inactive u64-second generation {}: {error}",
            request.staged_generation.display()
        )
    })?;
    sync_directory(&output_parent)?;
    write_new_sync(&report_path, &report_bytes)?;

    Ok(UpgradeU64SecondsResult {
        repository_count: audit.repository_count,
        task_count: audit.task_count,
        worker_job_count: audit.worker_job_count,
        source_bytes,
        target_bytes,
        source_fingerprint,
        target_fingerprint: audit.source_fingerprint,
        staged_generation: request.staged_generation,
        report_path,
    })
}

pub(crate) fn with_frozen_upgrade_source<T>(
    source_generation: &Path,
    expected_fingerprint: &str,
    action: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let source_generation = canonical_real_directory(source_generation)?;
    let manifest = read_generation_manifest(&source_generation)?;
    let _freeze = SourceFreeze::acquire(&source_generation, manifest.repository_count, true)?;
    validate_source_inventory(&source_generation, &manifest)?;
    let actual = authority_fingerprint(&source_generation)?;
    if actual != expected_fingerprint {
        return Err(format!(
            "u64-second activation source fingerprint changed: expected {expected_fingerprint}, found {actual}"
        ));
    }
    action()
}

impl SourceFreeze {
    fn acquire(
        source_generation: &Path,
        repository_count: u32,
        exclusive_lifecycle: bool,
    ) -> Result<Self, String> {
        let binary_root = source_generation
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| {
                "source generation is not beneath a Binary generations root".to_string()
            })?;
        let lifecycle_path = binary_root.join(LIFECYCLE_LOCK_FILE);
        let lifecycle = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lifecycle_path)
            .map_err(|error| format!("failed to open {}: {error}", lifecycle_path.display()))?;
        let lifecycle_result = if exclusive_lifecycle {
            lifecycle.try_lock_exclusive()
        } else {
            FileExt::try_lock_shared(&lifecycle)
        };
        lifecycle_result.map_err(|error| {
            format!(
                "Binary lifecycle is busy at {}: {error}",
                lifecycle_path.display()
            )
        })?;

        let files = ServerBinaryDbFilesystemStore;
        let registry_path = source_generation
            .join("global")
            .join(REGISTRY_LOCK_FILE_NAME);
        let registry = files
            .acquire_process_lock(
                &registry_path,
                ServerBinaryDbLockMode::Shared,
                ServerBinaryDbLockWait::Nonblocking,
            )
            .map_err(|error| format!("failed to lock Repository registry: {error}"))?
            .ok_or_else(|| "Repository registry writer is active".to_string())?;

        let mut repository_reads = Vec::with_capacity(repository_count as usize);
        let mut worker_queues = Vec::with_capacity(repository_count as usize);
        for repository_index in 0..repository_count {
            let root = source_generation
                .join("repositories")
                .join(repository_index.to_string());
            require_real_directory(&root)?;
            let queue_path = root.join(WORKER_QUEUE_LOCK_FILE_NAME);
            let queue = files
                .acquire_process_lock(
                    &queue_path,
                    ServerBinaryDbLockMode::Shared,
                    ServerBinaryDbLockWait::Nonblocking,
                )
                .map_err(|error| {
                    format!("failed to lock Worker Job queue {repository_index}: {error}")
                })?
                .ok_or_else(|| {
                    format!("Worker Job queue writer is active for Repository {repository_index}")
                })?;
            let read =
                BinaryDbReadLockSet::try_acquire(&StorePath::new(root)).map_err(|error| {
                    format!("Binary DB writer is active for Repository {repository_index}: {error}")
                })?;
            worker_queues.push(queue);
            repository_reads.push(read);
        }
        Ok(Self {
            _lifecycle: lifecycle,
            _registry: registry,
            _repository_reads: repository_reads,
            _worker_queues: worker_queues,
        })
    }
}

fn read_generation_manifest(generation: &Path) -> Result<GenerationManifest, String> {
    let bytes = read_regular_file(&generation.join(GENERATION_FILE))?;
    let manifest: GenerationManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid operational generation manifest: {error}"))?;
    if manifest.schema != GENERATION_SCHEMA
        || manifest.layout_id != 1
        || manifest.status != "validated_inactive"
        || manifest.global_registry != "global"
        || manifest.repository_authorities != "repositories"
    {
        return Err("source is not an exact layout_id 1 operational generation".to_string());
    }
    Ok(manifest)
}

fn validate_source_evidence(generation: &Path) -> Result<(), String> {
    let conversion = generation.join(CONVERSION_COMPLETION_FILE);
    let fresh = generation.join(FRESH_COMPLETION_FILE);
    match (conversion.is_file(), fresh.is_file()) {
        (true, false) => {
            let completion_bytes = read_regular_file(&conversion)?;
            let completion: SourceCompletion = serde_json::from_slice(&completion_bytes)
                .map_err(|error| format!("invalid predecessor completion evidence: {error}"))?;
            let report_bytes = read_regular_file(&generation.join(CONVERSION_REPORT_FILE))?;
            let report: JsonValue = serde_json::from_slice(&report_bytes)
                .map_err(|error| format!("invalid predecessor conversion report: {error}"))?;
            if completion.schema != POSTGRES_COMPLETION_SCHEMA
                || completion.layout_id != 1
                || completion.status != "validated_inactive"
                || completion.report_sha256.as_deref() != Some(sha256(&report_bytes).as_str())
                || report.get("schema").and_then(JsonValue::as_str) != Some(POSTGRES_REPORT_SCHEMA)
                || report.get("layout_id").and_then(JsonValue::as_u64) != Some(1)
                || report.get("status").and_then(JsonValue::as_str) != Some("validated_inactive")
            {
                return Err("predecessor conversion evidence is incomplete or invalid".to_string());
            }
        }
        (false, true) => {
            let completion: SourceCompletion = serde_json::from_slice(&read_regular_file(&fresh)?)
                .map_err(|error| format!("invalid fresh predecessor evidence: {error}"))?;
            if completion.schema != FRESH_COMPLETION_SCHEMA
                || completion.layout_id != 1
                || completion.status != "validated_inactive"
                || completion.report_sha256.is_some()
            {
                return Err("fresh predecessor evidence is incomplete or invalid".to_string());
            }
        }
        _ => {
            return Err(
                "source generation must contain exactly one admitted predecessor completion"
                    .to_string(),
            )
        }
    }
    Ok(())
}

fn validate_source_inventory(
    generation: &Path,
    manifest: &GenerationManifest,
) -> Result<Vec<PathBuf>, String> {
    validate_root_entries(generation)?;
    let global = generation.join("global");
    require_real_directory(&global)?;
    let expected_global = SERVER_GLOBAL_OPERATIONAL_BIN_PATHS
        .iter()
        .chain(SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS)
        .copied()
        .collect::<BTreeSet<_>>();
    validate_immediate_authority(&global, &expected_global, true)?;

    let repositories = generation.join("repositories");
    require_real_directory(&repositories)?;
    validate_dense_repository_directories(&repositories, manifest.repository_count)?;
    let expected_repository = SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
        .chain(SERVER_REPOSITORY_OPERATIONAL_BIN_PATHS.iter().copied())
        .chain(SERVER_REPOSITORY_OPERATIONAL_INDEX_PATHS.iter().copied())
        .collect::<BTreeSet<_>>();

    let mut files = vec![PathBuf::from(GENERATION_FILE)];
    if generation.join(LEGACY_FRESH_REPLAY_RECEIPT_FILE).is_file() {
        files.push(PathBuf::from(LEGACY_FRESH_REPLAY_RECEIPT_FILE));
    }
    for name in &expected_global {
        let relative = PathBuf::from("global").join(name);
        validate_source_authority_file(&generation.join(&relative), &relative)?;
        files.push(relative);
    }
    for repository_index in 0..manifest.repository_count {
        let relative_root = PathBuf::from("repositories").join(repository_index.to_string());
        let root = generation.join(&relative_root);
        validate_immediate_authority(&root, &expected_repository, false)?;
        for name in &expected_repository {
            let relative = relative_root.join(name);
            validate_source_authority_file(&generation.join(&relative), &relative)?;
            files.push(relative);
        }
        files.extend(validate_pack_inventory(generation, &relative_root)?);
    }
    files.sort();
    Ok(files)
}

fn validate_root_entries(generation: &Path) -> Result<(), String> {
    let allowed_files = BTreeSet::from([
        GENERATION_FILE,
        CONVERSION_REPORT_FILE,
        CONVERSION_COMPLETION_FILE,
        FRESH_COMPLETION_FILE,
        LEGACY_FRESH_REPLAY_RECEIPT_FILE,
    ]);
    for entry in sorted_entries(generation)? {
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "generation root contains non-UTF-8 name".to_string())?;
        let allowed = (metadata.is_dir()
            && matches!(name.as_str(), "global" | "repositories" | ".locks"))
            || (metadata.is_file() && allowed_files.contains(name.as_str()));
        if metadata.file_type().is_symlink() || !allowed {
            return Err(format!(
                "source generation contains undeclared root path {name:?}"
            ));
        }
    }
    Ok(())
}

fn validate_immediate_authority(
    root: &Path,
    expected: &BTreeSet<&str>,
    global: bool,
) -> Result<(), String> {
    let mut found = BTreeSet::new();
    for entry in sorted_entries(root)? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| format!("authority path is not UTF-8: {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "source authority contains symlink {}",
                path.display()
            ));
        }
        if metadata.is_file() && expected.contains(name.as_str()) {
            found.insert(name);
            continue;
        }
        let disposable_lock = metadata.is_file()
            && ((global && name == REGISTRY_LOCK_FILE_NAME)
                || (!global && name == WORKER_QUEUE_LOCK_FILE_NAME));
        let allowed_directory =
            metadata.is_dir() && ((!global && name == ".ait") || name == ".locks");
        if !disposable_lock && !allowed_directory {
            return Err(format!(
                "source authority contains undeclared path {}",
                path.display()
            ));
        }
    }
    let expected_owned = expected.iter().map(|value| (*value).to_string()).collect();
    if found != expected_owned {
        return Err(format!(
            "source authority file closure is incomplete at {}: expected={expected:?}, found={found:?}",
            root.display()
        ));
    }
    Ok(())
}

fn validate_dense_repository_directories(root: &Path, count: u32) -> Result<(), String> {
    let mut found = Vec::new();
    for entry in sorted_entries(root)? {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Repository directory name is not UTF-8".to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "Repository authority contains non-directory {name:?}"
            ));
        }
        let index = name
            .parse::<u32>()
            .map_err(|_| format!("non-canonical Repository directory {name:?}"))?;
        if index.to_string() != name {
            return Err(format!("non-canonical Repository directory {name:?}"));
        }
        found.push(index);
    }
    found.sort_unstable();
    let expected = (0..count).collect::<Vec<_>>();
    if found != expected {
        return Err(format!(
            "Repository directories are not the exact dense inventory: expected={expected:?}, found={found:?}"
        ));
    }
    Ok(())
}

fn validate_pack_inventory(
    generation: &Path,
    repository_root: &Path,
) -> Result<Vec<PathBuf>, String> {
    let ait = generation.join(repository_root).join(".ait");
    if !ait.exists() {
        return Ok(Vec::new());
    }
    require_real_directory(&ait)?;
    let objects = ait.join("objects");
    if !objects.exists() {
        if sorted_entries(&ait)?.is_empty() {
            return Ok(Vec::new());
        }
        return Err(format!(
            "source Repository .ait authority lacks its objects directory: {}",
            ait.display()
        ));
    }
    require_real_directory(&objects)?;
    let mut paths = Vec::new();
    for (directory, prefix) in [("packs", "PCK-"), ("tree-packs", "TPK-")] {
        let root = objects.join(directory);
        if !root.exists() {
            continue;
        }
        require_real_directory(&root)?;
        for entry in sorted_entries(&root)? {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| format!("pack path is not UTF-8: {}", path.display()))?;
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || !canonical_pack_name(&name, prefix)
            {
                return Err(format!(
                    "source contains undeclared pack path {}",
                    path.display()
                ));
            }
            paths.push(
                repository_root
                    .join(".ait/objects")
                    .join(directory)
                    .join(name),
            );
        }
    }
    for entry in sorted_entries(&objects)? {
        let name = entry.file_name();
        if name != "packs" && name != "tree-packs" {
            return Err(format!(
                "source object authority contains undeclared path {}",
                entry.path().display()
            ));
        }
    }
    for entry in sorted_entries(&ait)? {
        if entry.file_name() != "objects" {
            return Err(format!(
                "source Repository .ait authority contains undeclared path {}",
                entry.path().display()
            ));
        }
    }
    Ok(paths)
}

fn validate_source_authority_file(path: &Path, relative: &Path) -> Result<(), String> {
    validate_layout_one(path)?;
    let name = relative
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("authority path is not UTF-8: {}", relative.display()))?;
    if let Some(width) = time_width_plan(name) {
        return validate_layout_and_alignment(path, width.source_record_size, "u32-time-v0 record");
    }
    if name == "worker_ready.idx" {
        return validate_layout_and_alignment(path, 8, "u32-time-v0 Worker ready index");
    }
    if let Some(size) = server_binary_db_fixed_record_size(name) {
        return validate_layout_and_alignment(path, size as usize, "fixed record");
    }
    let index_size = SERVER_BINARY_DB_INDEX_SCHEMAS
        .iter()
        .find(|schema| schema.path == name)
        .map(|schema| schema.record_size)
        .or_else(|| {
            SERVER_REPOSITORY_OPERATIONAL_INDEX_PATHS
                .iter()
                .position(|path| *path == name)
                .and_then(|_| (name == "worker_state.idx").then_some(8))
        })
        .or_else(|| (name == "repository_namespace.idx").then_some(8));
    if let Some(size) = index_size {
        validate_layout_and_alignment(path, size as usize, "fixed index")?;
    }
    Ok(())
}

fn validate_layout_one(path: &Path) -> Result<(), String> {
    let mut file =
        File::open(path).map_err(|error| format!("failed to open {}: {error}", path.display()))?;
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

fn validate_layout_and_alignment(
    path: &Path,
    record_size: usize,
    label: &str,
) -> Result<(), String> {
    validate_layout_one(path)?;
    let body = fs::metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?
        .len()
        .checked_sub(4)
        .ok_or_else(|| format!("{} is shorter than its layout header", path.display()))?;
    if body % record_size as u64 != 0 {
        return Err(format!(
            "{} is not aligned to explicit {record_size}-byte {label}",
            path.display()
        ));
    }
    Ok(())
}

fn create_target_directories(root: &Path, repository_count: u32) -> Result<(), String> {
    fs::create_dir(root.join("global"))
        .and_then(|_| fs::create_dir(root.join("repositories")))
        .map_err(|error| format!("failed to create target authority roots: {error}"))?;
    for repository_index in 0..repository_count {
        let repository = root.join("repositories").join(repository_index.to_string());
        fs::create_dir(&repository)
            .and_then(|_| fs::create_dir(repository.join(".ait")))
            .and_then(|_| fs::create_dir(repository.join(".ait/objects")))
            .and_then(|_| fs::create_dir(repository.join(".ait/objects/packs")))
            .and_then(|_| fs::create_dir(repository.join(".ait/objects/tree-packs")))
            .map_err(|error| {
                format!("failed to create target Repository {repository_index}: {error}")
            })?;
    }
    Ok(())
}

fn widen_time_record_file(
    source: &Path,
    destination: &Path,
    width: TimeWidthPlan,
) -> Result<(), String> {
    validate_width_plan(width)?;
    validate_layout_and_alignment(source, width.source_record_size, "u32-time-v0 record")?;
    let mut input = File::open(source)
        .map_err(|error| format!("failed to open {}: {error}", source.display()))?;
    let mut header = [0_u8; 4];
    input
        .read_exact(&mut header)
        .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
    let record_count = (fs::metadata(source)
        .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?
        .len()
        - 4)
        / width.source_record_size as u64;
    let mut output = create_new_file(destination)?;
    output
        .write_all(&header)
        .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    let mut raw = vec![0_u8; width.source_record_size];
    for index in 0..record_count {
        input.read_exact(&mut raw).map_err(|error| {
            format!(
                "failed to read {} record {index}: {error}",
                source.display()
            )
        })?;
        output
            .write_all(&widen_time_record(&raw, width)?)
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", destination.display()))
}

fn widen_time_record(raw: &[u8], width: TimeWidthPlan) -> Result<Vec<u8>, String> {
    if raw.len() != width.source_record_size {
        return Err(format!(
            "{} source record has {} bytes, expected {}",
            width.name,
            raw.len(),
            width.source_record_size
        ));
    }
    validate_width_plan(width)?;
    let mut output = Vec::with_capacity(width.target_record_size);
    let mut cursor = 0_usize;
    for offset in width.source_time_offsets {
        output.extend_from_slice(&raw[cursor..*offset]);
        let value = u32::from_le_bytes(raw[*offset..*offset + 4].try_into().unwrap());
        output.extend_from_slice(&u64::from(value).to_le_bytes());
        cursor = *offset + 4;
    }
    output.extend_from_slice(&raw[cursor..]);
    if output.len() != width.target_record_size {
        return Err(format!(
            "{} widened record has {} bytes, expected {}",
            width.name,
            output.len(),
            width.target_record_size
        ));
    }
    Ok(output)
}

fn validate_width_plan(width: TimeWidthPlan) -> Result<(), String> {
    let mut previous_end = 0_usize;
    for offset in width.source_time_offsets {
        if *offset < previous_end || offset.saturating_add(4) > width.source_record_size {
            return Err(format!(
                "{} has invalid timestamp offset {offset}",
                width.name
            ));
        }
        previous_end = *offset + 4;
    }
    let expected = width
        .source_record_size
        .checked_add(width.source_time_offsets.len() * 4)
        .ok_or_else(|| format!("{} target width overflow", width.name))?;
    if expected != width.target_record_size {
        return Err(format!(
            "{} target width declaration is inconsistent",
            width.name
        ));
    }
    Ok(())
}

fn rebuild_worker_ready_index(repository: &Path) -> Result<(), String> {
    let jobs_path = repository.join("worker_job.bin");
    let bytes = read_regular_file(&jobs_path)?;
    if bytes.len() < 4
        || bytes[..4] != OPERATIONAL_V0_LAYOUT_ID.to_le_bytes()
        || (bytes.len() - 4) % SERVER_WORKER_JOB_RECORD_SIZE as usize != 0
    {
        return Err(format!(
            "converted Worker Job authority is invalid at {}",
            jobs_path.display()
        ));
    }
    let mut ready = Vec::new();
    for (index, raw) in bytes[4..]
        .chunks_exact(SERVER_WORKER_JOB_RECORD_SIZE as usize)
        .enumerate()
    {
        let record = ServerOperationalBinaryV0Codec::decode_worker_job(raw)
            .map_err(|error| format!("failed to decode converted Worker Job {index}: {error}"))?;
        if record.state_kind == WORKER_JOB_STATE_QUEUED {
            let worker_job_index_plus1 = u32::try_from(index)
                .map_err(|_| "Worker Job index exceeds u32".to_string())?
                .checked_add(1)
                .ok_or_else(|| "Worker Job plus-one index overflow".to_string())?;
            ready.push(ServerWorkerReadyIndexRecord {
                available_at_s: record.available_at_s,
                worker_job_index_plus1,
            });
        }
    }
    ready.sort_by_key(|record| (record.available_at_s, record.worker_job_index_plus1));
    let path = repository.join("worker_ready.idx");
    let mut output = create_new_file(&path)?;
    output
        .write_all(&OPERATIONAL_V0_LAYOUT_ID.to_le_bytes())
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    for record in ready {
        output
            .write_all(
                &ServerOperationalBinaryV0Codec::encode_worker_ready_index(record)
                    .map_err(|error| format!("failed to encode Worker ready index: {error}"))?,
            )
            .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", path.display()))
}

fn copy_exact_file(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|error| format!("failed to inspect {}: {error}", source.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "source is not a regular file: {}",
            source.display()
        ));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(format!(
            "source authority file has multiple hard links: {}",
            source.display()
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    #[cfg(target_os = "macos")]
    {
        let source_c = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| format!("source path contains NUL: {}", source.display()))?;
        let destination_c = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| format!("target path contains NUL: {}", destination.display()))?;
        let result = unsafe { libc::clonefile(source_c.as_ptr(), destination_c.as_ptr(), 0) };
        if result == 0 {
            let cloned = fs::symlink_metadata(destination).map_err(|error| {
                format!("failed to inspect clone {}: {error}", destination.display())
            })?;
            if !cloned.is_file()
                || cloned.file_type().is_symlink()
                || cloned.nlink() != 1
                || cloned.len() != metadata.len()
            {
                return Err(format!(
                    "APFS clone did not preserve exact regular-file authority: {}",
                    destination.display()
                ));
            }
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        let unsupported = matches!(
            error.raw_os_error(),
            Some(code)
                if matches!(
                    code,
                    libc::EXDEV | libc::ENOTSUP | libc::EINVAL | libc::ENOSYS | libc::EPERM
                )
        );
        if !unsupported {
            return Err(format!(
                "failed to clone authority file {} to {}: {error}",
                source.display(),
                destination.display()
            ));
        }
    }
    let mut input = File::open(source)
        .map_err(|error| format!("failed to open {}: {error}", source.display()))?;
    let mut output = create_new_file(destination)?;
    let mut buffer = vec![0_u8; 1024 * 1024];
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
    }
    output
        .sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", destination.display()))
}

fn create_new_file(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("failed to create {}: {error}", path.display()))
}

fn target_record_counts(files: &[FileReport]) -> Result<BTreeMap<String, u64>, String> {
    let mut counts = BTreeMap::new();
    for file in files {
        let relative = Path::new(&file.relative_path);
        let Some(name) = relative.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let size = if name == "worker_ready.idx" {
            Some(12)
        } else if name == "repository_namespace.idx" || name == "worker_state.idx" {
            Some(8)
        } else {
            server_binary_db_fixed_record_size(name)
                .map(|value| value as u64)
                .or_else(|| {
                    SERVER_BINARY_DB_INDEX_SCHEMAS
                        .iter()
                        .find(|schema| schema.path == name)
                        .map(|schema| u64::from(schema.record_size))
                })
        };
        let Some(size) = size else {
            continue;
        };
        let body = file
            .byte_size
            .checked_sub(4)
            .ok_or_else(|| format!("{} is shorter than its layout header", file.relative_path))?;
        if body % size != 0 {
            return Err(format!(
                "{} is misaligned after conversion",
                file.relative_path
            ));
        }
        counts.insert(file.relative_path.clone(), body / size);
    }
    Ok(counts)
}

fn is_record_authority_path(relative: &Path) -> bool {
    matches!(
        relative.components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(global), Component::Normal(_)] if *global == "global"
    ) || matches!(
        relative.components().collect::<Vec<_>>().as_slice(),
        [Component::Normal(repositories), Component::Normal(_), Component::Normal(_)] if *repositories == "repositories"
    )
}

fn time_width_plan(name: &str) -> Option<TimeWidthPlan> {
    TIME_WIDTH_PLANS
        .iter()
        .find(|width| width.name == name)
        .copied()
}

fn canonical_pack_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(".zstpack"))
        .is_some_and(|value| {
            value.len() == 12
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
        })
}

fn sorted_entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate {}: {error}", path.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn require_real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect directory {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("path is not a real directory: {}", path.display()));
    }
    Ok(())
}

fn sync_tree_directories(root: &Path) -> Result<(), String> {
    fn visit(path: &Path) -> Result<(), String> {
        for entry in sorted_entries(path)? {
            let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
                format!("failed to inspect {}: {error}", entry.path().display())
            })?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                visit(&entry.path())?;
            }
        }
        sync_directory(path)
    }
    visit(root)
}

fn absolute_new_file_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || fs::symlink_metadata(path).is_ok() {
        return Err(format!(
            "report must identify a new file: {}",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "report path has no parent".to_string())?;
    let parent = canonical_real_directory(parent)?;
    Ok(parent.join(
        path.file_name()
            .ok_or_else(|| "report path has no file name".to_string())?,
    ))
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("path is not UTF-8: {}", path.display()))
}

fn unique_nonce() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|_| "system time precedes Unix epoch".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_predecessor_generation(label: &str) -> (PathBuf, PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ait-server-u64-second-{label}-{}-{nonce}",
            std::process::id()
        ));
        let binary_root = root.join("binary-v0");
        let source = binary_root.join("generations/source-u32");
        let global = source.join("global");
        fs::create_dir_all(&global).unwrap();
        fs::create_dir(source.join("repositories")).unwrap();
        for name in SERVER_GLOBAL_OPERATIONAL_BIN_PATHS
            .iter()
            .chain(SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS)
        {
            fs::write(global.join(name), 1_u32.to_le_bytes()).unwrap();
        }
        let manifest = serde_json::json!({
            "schema": GENERATION_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "global_registry": "global",
            "repository_authorities": "repositories",
            "repository_count": 0,
        });
        fs::write(
            source.join(GENERATION_FILE),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        (root, source)
    }

    #[test]
    fn every_predecessor_width_plan_zero_extends_only_declared_times() {
        for width in TIME_WIDTH_PLANS {
            validate_width_plan(*width).unwrap();
            let mut source = (0..width.source_record_size)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>();
            for (ordinal, offset) in width.source_time_offsets.iter().enumerate() {
                let value = if ordinal == 0 {
                    u32::MAX
                } else {
                    0x0102_0304_u32.wrapping_add(ordinal as u32)
                };
                source[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
            }
            let target = widen_time_record(&source, *width).unwrap();
            let mut source_cursor = 0_usize;
            let mut target_cursor = 0_usize;
            for offset in width.source_time_offsets {
                let unchanged = offset - source_cursor;
                assert_eq!(
                    &target[target_cursor..target_cursor + unchanged],
                    &source[source_cursor..*offset],
                    "{} changed non-time bytes",
                    width.name
                );
                target_cursor += unchanged;
                let source_time =
                    u32::from_le_bytes(source[*offset..*offset + 4].try_into().unwrap());
                let target_time = u64::from_le_bytes(
                    target[target_cursor..target_cursor + 8].try_into().unwrap(),
                );
                assert_eq!(target_time, u64::from(source_time), "{}", width.name);
                source_cursor = *offset + 4;
                target_cursor += 8;
            }
            assert_eq!(&target[target_cursor..], &source[source_cursor..]);
            assert_eq!(target.len(), width.target_record_size);
        }
    }

    #[test]
    fn source_selector_is_exact_and_checked_before_filesystem_access() {
        for selector in ["", "u32", "u32-time", U64_SECOND_V0_TARGET_SELECTOR] {
            let error = upgrade_u64_seconds(UpgradeU64SecondsRequest {
                source_selector: selector.to_string(),
                source_generation: PathBuf::from("does-not-exist"),
                staged_generation: PathBuf::from("does-not-exist-output"),
                report_path: PathBuf::from("does-not-exist-report"),
            })
            .unwrap_err();
            assert!(
                error.contains("expected exact selector"),
                "{selector}: {error}"
            );
        }
    }

    #[test]
    fn activation_source_recheck_rejects_any_authority_change_before_action() {
        let (root, source) = empty_predecessor_generation("source-recheck");
        let expected = authority_fingerprint(&source).unwrap();
        let manifest: JsonValue =
            serde_json::from_slice(&fs::read(source.join(GENERATION_FILE)).unwrap()).unwrap();
        let mut changed_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
        changed_bytes.push(b'\n');
        fs::write(source.join(GENERATION_FILE), changed_bytes).unwrap();

        let mut action_called = false;
        let error = with_frozen_upgrade_source(&source, &expected, || {
            action_called = true;
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("source fingerprint changed"), "{error}");
        assert!(!action_called);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn canonical_pack_names_are_narrow() {
        assert!(canonical_pack_name("PCK-0123456789AF.zstpack", "PCK-"));
        assert!(canonical_pack_name("TPK-ABCDEF012345.zstpack", "TPK-"));
        for name in [
            "PCK-0123456789af.zstpack",
            "PCK-0123456789AF.pack",
            "PCK-0123456789A.zstpack",
            "PCK-0123456789AF.zstpack.extra",
        ] {
            assert!(!canonical_pack_name(name, "PCK-"), "{name}");
        }
    }
}
