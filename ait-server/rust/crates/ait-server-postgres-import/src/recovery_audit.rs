use crate::activation::{canonical_real_directory, read_regular_file, sha256, write_new_sync};
use crate::domain::RepositoryDomain;
use crate::generation_inventory::is_disposable_runtime_file;
use ait_server_core::foundation::remote_binary_db::{
    BinaryDbError, BinaryDbReadTxn, FilesystemServerRemoteBinaryDb, RepoId, RepoName,
    StoreGeneration, StorePath, StoreResult,
};
use ait_server_core::foundation::server_operational_repository_registry::ServerOperationalRepositoryRegistry;
use ait_server_core::foundation::server_operational_worker_jobs::{
    ServerOperationalWorkerJobStore, WorkerJobDomainAuthority,
};
use ait_server_core::foundation::server_workflow_store::{
    ServerWorkflowChangeStore, ServerWorkflowTaskStore,
};
use ait_server_core::foundation::workflow_binary_v0::{
    WorkflowBinaryV0Codec, LAND_MODE_MASK, LAND_STATUS_MASK,
};
use ait_server_core::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store;
use serde::Serialize;
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const AUDIT_SCHEMA: &str = "ait.server.binary_v0.recovery_generation_audit.v1";
const GENERATION_SCHEMA: &str = "ait.server.binary_v0.operational_generation.v1";
const RUNTIME_LOCK_DIRECTORY: &str = ".locks";
const HISTORY_PROMOTION_PREFIX: &str = "ait-history-promotion/v1 ";
const LOCAL_LAND_RECEIPT_PREFIX: &str = "ait-local-land-receipt/v1 ";

#[derive(Clone, Debug)]
pub struct AuditGenerationRequest {
    pub generation: PathBuf,
    pub report_path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditGenerationResult {
    pub repository_count: u32,
    pub task_count: u64,
    pub worker_job_count: u64,
    pub source_fingerprint: String,
    pub report_path: PathBuf,
}

#[derive(Debug, Serialize)]
struct GenerationAudit {
    schema: &'static str,
    layout_id: u32,
    status: &'static str,
    generation: String,
    source_fingerprint: String,
    generation_manifest_sha256: String,
    activation_repository_baseline: u64,
    repository_count: u32,
    task_count: u64,
    worker_job_count: u64,
    repositories: Vec<RepositoryAudit>,
}

#[derive(Debug, Serialize)]
struct RepositoryAudit {
    repository_index: u32,
    repo_name: String,
    namespace_ascii: [u8; 2],
    namespace: String,
    lifecycle_kind: u8,
    policy_flags: u8,
    patchset_codec: &'static str,
    authority_relative_path: String,
    file_sizes: BTreeMap<String, u64>,
    line_names_by_index: BTreeMap<u32, String>,
    main_head_snapshot_id: Option<String>,
    snapshot_count: u32,
    snapshot_ids_by_index: BTreeMap<u32, String>,
    task_count: u32,
    tasks: JsonValue,
    change_count: u32,
    changes: JsonValue,
    patchset_count: u32,
    patchsets: Vec<JsonValue>,
    land_count: u32,
    lands: Vec<JsonValue>,
    history_promotion_count: u32,
    local_land_receipt_count: u32,
    worker_job_count: u32,
    worker_jobs: Vec<JsonValue>,
}

pub fn audit_generation(request: AuditGenerationRequest) -> Result<AuditGenerationResult, String> {
    let generation = canonical_real_directory(&request.generation)?;
    let report_path = absolute_new_file_path(&request.report_path)?;
    if report_path.starts_with(&generation) {
        return Err("recovery audit report must be outside the audited generation".to_string());
    }
    let source_fingerprint = authority_fingerprint(&generation)?;
    let manifest_path = generation.join("generation.json");
    let manifest_bytes = read_regular_file(&manifest_path)?;
    let manifest: JsonValue = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        format!(
            "failed to parse recovery generation manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    validate_generation_manifest(&manifest)?;

    let global_root = canonical_real_directory(&generation.join("global"))?;
    let repositories_root = canonical_real_directory(&generation.join("repositories"))?;
    let registry = ServerOperationalRepositoryRegistry::new(&global_root, &repositories_root)
        .map_err(|error| format!("failed to open audited Repository registry: {error}"))?;
    let entries = registry
        .validate()
        .map_err(|error| format!("audited Repository registry is invalid: {error}"))?;
    let activation_repository_baseline = manifest
        .get("repository_count")
        .and_then(JsonValue::as_u64)
        .expect("validated operational generation has a Repository baseline");
    if activation_repository_baseline > entries.len() as u64 {
        return Err(
            "audited Repository registry is shorter than its activation baseline".to_string(),
        );
    }

    let mut repositories = Vec::with_capacity(entries.len());
    let mut task_count = 0_u64;
    let mut worker_job_count = 0_u64;
    for entry in entries {
        let authority_root = registry
            .resolve_authority_directory(entry.repository_index)
            .map_err(|error| {
                format!(
                    "failed to resolve audited Repository {}: {error}",
                    entry.repository_index
                )
            })?;
        let audit = audit_repository(
            &repositories_root,
            &authority_root,
            entry.repository_index,
            &entry.repo_name,
            entry.record.namespace_ascii,
            entry.record.lifecycle_kind,
            entry.record.policy_flags,
        )?;
        task_count = task_count
            .checked_add(u64::from(audit.task_count))
            .ok_or_else(|| "audited Task count exceeds u64".to_string())?;
        worker_job_count = worker_job_count
            .checked_add(u64::from(audit.worker_job_count))
            .ok_or_else(|| "audited Worker Job count exceeds u64".to_string())?;
        repositories.push(audit);
    }

    let after_fingerprint = authority_fingerprint(&generation)?;
    if after_fingerprint != source_fingerprint {
        return Err("audited generation changed during its read-only inventory".to_string());
    }
    let repository_count = u32::try_from(repositories.len())
        .map_err(|_| "audited Repository count exceeds u32".to_string())?;
    let report = GenerationAudit {
        schema: AUDIT_SCHEMA,
        layout_id: 1,
        status: "audited_immutable",
        generation: generation.display().to_string(),
        source_fingerprint: source_fingerprint.clone(),
        generation_manifest_sha256: sha256(&manifest_bytes),
        activation_repository_baseline,
        repository_count,
        task_count,
        worker_job_count,
        repositories,
    };
    let mut bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("failed to encode recovery generation audit: {error}"))?;
    bytes.push(b'\n');
    write_new_sync(&report_path, &bytes)?;
    Ok(AuditGenerationResult {
        repository_count,
        task_count,
        worker_job_count,
        source_fingerprint,
        report_path,
    })
}

#[allow(clippy::too_many_arguments)]
fn audit_repository(
    repositories_root: &Path,
    authority_root: &Path,
    repository_index: u32,
    repo_name: &str,
    namespace_ascii: [u8; 2],
    lifecycle_kind: u8,
    policy_flags: u8,
) -> Result<RepositoryAudit, String> {
    let repo_id = repository_index.to_string();
    let (domain, patchset_codec) = match RepositoryDomain::load_frozen(
        authority_root,
        &repo_id,
        repo_name,
        1,
        namespace_ascii,
    ) {
        Ok(domain) => (domain, "frozen"),
        Err(frozen_error) => match RepositoryDomain::load(
            authority_root,
            &repo_id,
            repo_name,
            1,
            namespace_ascii,
        ) {
            Ok(domain) => (domain, "transitional"),
            Err(transitional_error) => {
                return Err(format!(
                    "audited Repository {repository_index} ({repo_name}) is invalid under both admitted Patchset codecs: frozen={frozen_error}; transitional={transitional_error}"
                ))
            }
        },
    };
    domain
        .validate_tree_authority(&repo_id, repo_name, 1)
        .map_err(|error| {
            format!("audited Repository {repository_index} ({repo_name}) is invalid: {error}")
        })?;
    let namespace = namespace_text(namespace_ascii)?;
    let db = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new(repo_id),
        RepoName::new(repo_name),
        StorePath::new(authority_root.to_path_buf()),
        StoreGeneration::new(1),
    );
    let workflow = match patchset_codec {
        "frozen" => BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), &namespace)?,
        "transitional" => BinaryDbServerWorkflowV0Store::new_remote(db.clone(), &namespace)?,
        _ => unreachable!("recovery audit selects one of two Patchset codecs"),
    };
    let tasks = workflow.list_tasks(repo_name)?;
    let changes = workflow.list_changes(repo_name)?;
    let task_count = json_array_len(&tasks, "Task")?;
    let change_count = json_array_len(&changes, "Change")?;
    let read = BinaryDbReadTxn::new(&db);

    let mut history_promotion_count = 0_u32;
    let mut local_land_receipt_count = 0_u32;
    let mut patchsets = Vec::with_capacity(domain.patchsets_by_id.len());
    for (patchset_id, identity) in &domain.patchsets_by_id {
        let summary_raw = read
            .read_payload(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                identity.record.summary_offset,
                u32::from(identity.record.summary_len),
            )
            .map_err(|error| format!("failed to read audited Patchset summary: {error}"))?;
        let summary = WorkflowBinaryV0Codec::decode_single_text_payload(
            &summary_raw,
            "audited Patchset summary",
        )
        .map_err(|error| format!("failed to decode audited Patchset summary: {error}"))?;
        let recovery_authority = if let Some(raw) = summary.strip_prefix(HISTORY_PROMOTION_PREFIX) {
            history_promotion_count = history_promotion_count
                .checked_add(1)
                .ok_or_else(|| "history promotion count exceeds u32".to_string())?;
            Some(parse_embedded_json(raw, "history promotion")?)
        } else if let Some(raw) = summary.strip_prefix(LOCAL_LAND_RECEIPT_PREFIX) {
            local_land_receipt_count = local_land_receipt_count
                .checked_add(1)
                .ok_or_else(|| "local Land receipt count exceeds u32".to_string())?;
            Some(parse_embedded_json(raw, "local Land receipt")?)
        } else {
            None
        };
        patchsets.push(json!({
            "patchset_index": identity.index,
            "patchset_id": patchset_id,
            "change_index": identity.record.change_index,
            "base_snapshot_id": domain.snapshot_id(identity.record.base_snapshot_index)?,
            "revision_snapshot_id": domain.snapshot_id(identity.record.revision_snapshot_index)?,
            "created_at_s": identity.record.created_at_s,
            "summary": summary,
            "recovery_authority": recovery_authority,
        }));
    }
    patchsets.sort_by_key(|row| {
        row.get("patchset_index")
            .and_then(JsonValue::as_u64)
            .unwrap_or(u64::MAX)
    });

    let mut lands = domain
        .lands_by_id
        .iter()
        .map(|(land_id, identity)| {
            let record = identity.record;
            let target_line_index = record
                .target_line_index_plus1
                .checked_sub(1)
                .ok_or_else(|| format!("audited Land {land_id} has no target Line"))?;
            let patchset = domain
                .patchsets_by_index
                .get(record.patchset_index as usize)
                .ok_or_else(|| format!("audited Land {land_id} has no Patchset"))?;
            Ok(json!({
                "land_id": land_id,
                "land_ordinal": record.land_ordinal,
                "change_index": record.change_index,
                "patchset_index": record.patchset_index,
                "patchset_revision_snapshot_id": domain.snapshot_id(patchset.revision_snapshot_index)?,
                "pre_land_target_snapshot_id": optional_snapshot_id(&domain, record.pre_land_target_snapshot_index_plus1)?,
                "landed_snapshot_id": optional_snapshot_id(&domain, record.landed_snapshot_index_plus1)?,
                "target_line": domain.line_names_by_index.get(&target_line_index),
                "status_kind": record.land_meta & LAND_STATUS_MASK,
                "mode_kind": (record.land_meta & LAND_MODE_MASK) >> 5,
                "submitted_at_s": record.submitted_at_s,
                "updated_at_s": record.updated_at_s,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    lands.sort_by_key(|row| {
        (
            row.get("change_index")
                .and_then(JsonValue::as_u64)
                .unwrap_or(u64::MAX),
            row.get("land_ordinal")
                .and_then(JsonValue::as_u64)
                .unwrap_or(u64::MAX),
        )
    });

    let job_domain = Arc::new(AuditJobDomain {
        patchset_count: u32::try_from(domain.patchsets_by_index.len())
            .map_err(|_| "audited Patchset count exceeds u32".to_string())?,
        snapshot_indexes: domain.snapshot_ids_by_index.keys().copied().collect(),
    });
    let worker_jobs = ServerOperationalWorkerJobStore::new(
        repository_index,
        authority_root.to_path_buf(),
        job_domain,
    )
    .map_err(|error| format!("failed to open audited Worker Job authority: {error}"))?
    .validate()
    .map_err(|error| format!("audited Worker Job authority is invalid: {error}"))?
    .into_iter()
    .map(|entry| {
        let record = entry.record;
        json!({
            "worker_job_index": entry.key.worker_job_index,
            "job_kind": record.job_kind,
            "state_kind": record.state_kind,
            "outcome_kind": record.outcome_kind,
            "attempt_count": record.attempt_count,
            "max_attempts": record.max_attempts,
            "error_kind": record.error_kind,
            "patchset_index": record.patchset_index_plus1.checked_sub(1),
            "snapshot_index": record.snapshot_index_plus1.checked_sub(1),
            "available_at_s": record.available_at_s,
            "created_at_s": record.created_at_s,
            "updated_at_s": record.updated_at_s,
        })
    })
    .collect::<Vec<_>>();

    let main_head_snapshot_id = domain
        .main_head_snapshot_index
        .map(|index| domain.snapshot_id(index).map(str::to_string))
        .transpose()?;
    let authority_relative_path = authority_root
        .strip_prefix(repositories_root)
        .map_err(|_| "audited Repository escaped its generation root".to_string())?
        .to_string_lossy()
        .into_owned();
    Ok(RepositoryAudit {
        repository_index,
        repo_name: repo_name.to_string(),
        namespace_ascii,
        namespace,
        lifecycle_kind,
        policy_flags,
        patchset_codec,
        authority_relative_path,
        file_sizes: immediate_file_sizes(authority_root)?,
        line_names_by_index: domain.line_names_by_index,
        main_head_snapshot_id,
        snapshot_count: u32::try_from(domain.snapshot_ids_by_index.len())
            .map_err(|_| "audited Snapshot count exceeds u32".to_string())?,
        snapshot_ids_by_index: domain.snapshot_ids_by_index,
        task_count,
        tasks,
        change_count,
        changes,
        patchset_count: u32::try_from(patchsets.len())
            .map_err(|_| "audited Patchset count exceeds u32".to_string())?,
        patchsets,
        land_count: u32::try_from(lands.len())
            .map_err(|_| "audited Land count exceeds u32".to_string())?,
        lands,
        history_promotion_count,
        local_land_receipt_count,
        worker_job_count: u32::try_from(worker_jobs.len())
            .map_err(|_| "audited Worker Job count exceeds u32".to_string())?,
        worker_jobs,
    })
}

struct AuditJobDomain {
    patchset_count: u32,
    snapshot_indexes: BTreeSet<u32>,
}

impl WorkerJobDomainAuthority for AuditJobDomain {
    fn validate_patchset_index(&self, patchset_index: u32) -> StoreResult<()> {
        if patchset_index < self.patchset_count {
            Ok(())
        } else {
            Err(BinaryDbError::invalid_domain_data(format!(
                "Patchset index {patchset_index} is out of range"
            )))
        }
    }

    fn validate_snapshot_index(&self, snapshot_index: u32) -> StoreResult<()> {
        if self.snapshot_indexes.contains(&snapshot_index) {
            Ok(())
        } else {
            Err(BinaryDbError::invalid_domain_data(format!(
                "Snapshot index {snapshot_index} is absent or tombstoned"
            )))
        }
    }
}

fn optional_snapshot_id(
    domain: &RepositoryDomain,
    snapshot_index_plus1: u32,
) -> Result<Option<String>, String> {
    snapshot_index_plus1
        .checked_sub(1)
        .map(|index| domain.snapshot_id(index).map(str::to_string))
        .transpose()
}

fn validate_generation_manifest(manifest: &JsonValue) -> Result<(), String> {
    if manifest.get("schema").and_then(JsonValue::as_str) != Some(GENERATION_SCHEMA)
        || manifest.get("layout_id").and_then(JsonValue::as_u64) != Some(1)
        || manifest.get("status").and_then(JsonValue::as_str) != Some("validated_inactive")
        || manifest.get("global_registry").and_then(JsonValue::as_str) != Some("global")
        || manifest
            .get("repository_authorities")
            .and_then(JsonValue::as_str)
            != Some("repositories")
        || manifest
            .get("repository_count")
            .and_then(JsonValue::as_u64)
            .is_none()
    {
        return Err("recovery audit source is not an exact operational generation".to_string());
    }
    Ok(())
}

fn namespace_text(namespace_ascii: [u8; 2]) -> Result<String, String> {
    let bytes = namespace_ascii
        .into_iter()
        .take_while(|byte| *byte != 0)
        .collect::<Vec<_>>();
    String::from_utf8(bytes).map_err(|_| "Repository namespace is not ASCII".to_string())
}

fn json_array_len(value: &JsonValue, label: &str) -> Result<u32, String> {
    let len = value
        .as_array()
        .ok_or_else(|| format!("audited {label} inventory is not an array"))?
        .len();
    u32::try_from(len).map_err(|_| format!("audited {label} count exceeds u32"))
}

fn parse_embedded_json(raw: &str, label: &str) -> Result<JsonValue, String> {
    serde_json::from_str(raw).map_err(|error| format!("invalid persisted {label}: {error}"))
}

fn immediate_file_sizes(root: &Path) -> Result<BTreeMap<String, u64>, String> {
    let mut entries = fs::read_dir(root)
        .map_err(|error| format!("failed to list {}: {error}", root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate {}: {error}", root.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut sizes = BTreeMap::new();
    for entry in entries {
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("failed to inspect {}: {error}", entry.path().display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "audited authority contains a symbolic link: {}",
                entry.path().display()
            ));
        }
        if metadata.is_file() {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "audited authority has a non-UTF-8 file name".to_string())?;
            sizes.insert(name, metadata.len());
        }
    }
    Ok(sizes)
}

pub(crate) fn authority_fingerprint(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_regular_files(root, root, &mut files)?;
    files.sort();
    let mut authority = Sha256::new();
    for relative in files {
        let absolute = root.join(&relative);
        let metadata = fs::metadata(&absolute)
            .map_err(|error| format!("failed to inspect {}: {error}", absolute.display()))?;
        authority.update(relative.to_string_lossy().as_bytes());
        authority.update([0]);
        authority.update(metadata.len().to_le_bytes());
        authority.update(file_sha256(&absolute)?.as_bytes());
    }
    Ok(format!("{:x}", authority.finalize()))
}

fn collect_regular_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to enumerate {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let absolute = entry.path();
        let metadata = fs::symlink_metadata(&absolute)
            .map_err(|error| format!("failed to inspect {}: {error}", absolute.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "audited generation contains a symbolic link: {}",
                absolute.display()
            ));
        }
        if metadata.is_dir() {
            if directory == root && entry.file_name() == RUNTIME_LOCK_DIRECTORY {
                continue;
            }
            collect_regular_files(root, &absolute, files)?;
        } else if metadata.is_file() {
            let relative = absolute
                .strip_prefix(root)
                .map_err(|_| "audited path escaped generation root".to_string())?
                .to_path_buf();
            if !is_disposable_runtime_file(&relative) {
                files.push(relative);
            }
        } else {
            return Err(format!(
                "audited generation contains a non-file entry: {}",
                absolute.display()
            ));
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn absolute_new_file_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || fs::symlink_metadata(path).is_ok() {
        return Err(format!(
            "recovery audit report must identify a new file: {}",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "recovery audit report has no parent".to_string())?;
    let parent = canonical_real_directory(parent)?;
    Ok(parent.join(
        path.file_name()
            .ok_or_else(|| "recovery audit report has no file name".to_string())?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn authority_fingerprint_excludes_disposable_runtime_locks() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ait-server-recovery-fingerprint-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("authority.bin"), b"authority").unwrap();
        let before = authority_fingerprint(&root).unwrap();

        let locks = root.join(RUNTIME_LOCK_DIRECTORY).join("binary-db");
        fs::create_dir_all(&locks).unwrap();
        fs::write(locks.join("server-content.write.lock"), b"").unwrap();
        assert_eq!(authority_fingerprint(&root).unwrap(), before);
        fs::write(locks.join("server-content.write.lock"), b"runtime-only").unwrap();
        assert_eq!(authority_fingerprint(&root).unwrap(), before);

        let repository = root.join("repositories/2");
        fs::create_dir_all(&repository).unwrap();
        let rebuild = repository.join(".worker_ready.idx.rebuild");
        fs::write(&rebuild, b"interrupted disposable rebuild").unwrap();
        assert_eq!(authority_fingerprint(&root).unwrap(), before);
        fs::write(&rebuild, b"changed disposable rebuild").unwrap();
        assert_eq!(authority_fingerprint(&root).unwrap(), before);

        let worker_queue_lock = repository.join("worker-queue.lock");
        fs::write(&worker_queue_lock, b"runtime diagnostic owner").unwrap();
        assert_eq!(authority_fingerprint(&root).unwrap(), before);
        fs::write(&worker_queue_lock, b"").unwrap();
        assert_eq!(authority_fingerprint(&root).unwrap(), before);

        fs::write(
            repository.join("worker-queue.lock.extra"),
            b"authority lock lookalike",
        )
        .unwrap();
        assert_ne!(authority_fingerprint(&root).unwrap(), before);
        fs::remove_file(repository.join("worker-queue.lock.extra")).unwrap();
        assert_eq!(authority_fingerprint(&root).unwrap(), before);

        fs::write(
            repository.join(".worker_ready.idx.rebuild.extra"),
            b"authority lookalike",
        )
        .unwrap();
        assert_ne!(authority_fingerprint(&root).unwrap(), before);
        fs::remove_file(repository.join(".worker_ready.idx.rebuild.extra")).unwrap();
        assert_eq!(authority_fingerprint(&root).unwrap(), before);

        fs::write(root.join("authority.bin"), b"changed authority").unwrap();
        assert_ne!(authority_fingerprint(&root).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
    }
}
