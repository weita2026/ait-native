use crate::activation::{
    atomic_replace, canonical_real_directory, read_regular_file, sha256, sync_directory,
    write_completion, write_new_sync,
};
use crate::domain::{PatchsetIdentity, RepositoryDomain};
use crate::generation_inventory::is_disposable_runtime_file;
use crate::json::parse_object_without_duplicates;
use crate::legacy_alias_source::{LegacyPatchsetCatalog, LegacyPatchsetIdentity};
use crate::recovery_job_policy::{
    RecoveryJobAudit, RecoveryJobClassification, RecoveryJobPolicy, ATTACHED_TERMINAL_EXACT,
    DIAGNOSTIC_ATTACHED, LEGACY_PATCHSET_OMISSION, MAIN_SEED_LANDED_SNAPSHOT,
    MISSING_REMOTE_PREFIX, NON_MAIN_TARGET_OMISSION, REPO_CI_RESULT_SNAPSHOT,
    SNAPSHOT_ONLY_NO_PATCHSET_OMISSION, UNPROVABLE_ATTACHED_OMISSION,
};
use crate::types::{
    SourceJobRow, SourceManifest, SourceRepositoryRow, SourceSnapshot, SOURCE_DATABASE,
};
use ait_server_core::foundation::operational_binary_v0::{
    OperationalNamespaceIndexRecord, OperationalRepositoryPayload, OperationalRepositoryRecord,
    ServerOperationalBinaryV0Codec, ServerWorkerJobRecord, ServerWorkerReadyIndexRecord,
    ServerWorkerStateIndexRecord, OPERATIONAL_V0_LAYOUT_ID, REPOSITORY_LIFECYCLE_ACTIVE,
    REPOSITORY_LIFECYCLE_PURGED, REPOSITORY_LIFECYCLE_RETIRING, WORKER_JOB_ERROR_LEASE_EXPIRED,
    WORKER_JOB_ERROR_NONE, WORKER_JOB_ERROR_RETRYABLE_EXECUTION,
    WORKER_JOB_ERROR_TERMINAL_EXECUTION, WORKER_JOB_KIND_CONTENT_GC,
    WORKER_JOB_KIND_CONTENT_OPTIMIZE, WORKER_JOB_KIND_CONTENT_PACK, WORKER_JOB_KIND_LAND_PROCESS,
    WORKER_JOB_KIND_MAIN_SEED_REFRESH, WORKER_JOB_KIND_PATCHSET_CI,
    WORKER_JOB_KIND_PATCHSET_CI_AGGREGATE, WORKER_JOB_KIND_POLICY_EVALUATE,
    WORKER_JOB_KIND_RECONCILE_REPO, WORKER_JOB_KIND_REPO_CI, WORKER_JOB_OUTCOME_ATTACHED,
    WORKER_JOB_OUTCOME_COMPLETED, WORKER_JOB_OUTCOME_FAILED, WORKER_JOB_OUTCOME_NONE,
    WORKER_JOB_OUTCOME_SKIPPED, WORKER_JOB_OUTCOME_SUPERSEDED, WORKER_JOB_STATE_FAILED,
    WORKER_JOB_STATE_QUEUED, WORKER_JOB_STATE_SUCCEEDED,
};
use ait_server_core::foundation::remote_binary_db::{BinaryDbError, StoreResult};
use ait_server_core::foundation::server_operational_repository_registry::{
    ServerOperationalRepositoryRegistry, FIXED_REPOSITORY_NAMES,
};
use ait_server_core::foundation::server_operational_worker_jobs::{
    ServerOperationalWorkerJobStore, WorkerJobDomainAuthority,
};
use ait_server_core::foundation::transport::normalize_async_job_payload;
use ait_server_core::foundation::workflow_binary_v0::{
    V0FrozenPatchsetRecord, WorkflowBinaryV0Codec, PATCHSET_RECORD_SIZE,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(target_os = "macos")]
use std::ffi::CString;
use std::fs;
#[cfg(target_os = "macos")]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const SOURCE_MANIFEST_SCHEMA: &str = "ait.server.binary_registry.generation.v1";
const REPOSITORY_ORDER_SCHEMA: &str = "ait.server.postgres_to_binary_v0.repository_order.v1";
const GENERATION_SCHEMA: &str = "ait.server.binary_v0.operational_generation.v1";
const REPORT_SCHEMA: &str = "ait.server.postgres_to_binary_v0.report.v1";
const LEASE_EXPIRED_REQUEUED: &str = "Worker lease expired; job returned to queue";
const LEASE_EXPIRED_FAILED: &str = "Worker lease expired after max attempts";
const HISTORICAL_ATTACHED_DIAGNOSTIC: &str =
    "server worker executor admission waited after durable claim: 3642 conflicts with running job 3638";
const LEGACY_SNAPSHOT_PATCHSET_ALIASES: &[(&str, &str)] = &[
    (
        "manual-ait-core-main-seed-refresh-snp-67aaa1063113",
        "operator-main-seed-refresh",
    ),
    (
        "operator-ait-core-main-seed-refresh-snp-67aaa1063113-p3",
        "operator-main-seed-refresh",
    ),
    (
        "operator-main-seed-refresh-snp-4b5ba09ce296-p3",
        "operator-main-seed-refresh",
    ),
    ("P-SEC-0127-SNAPSHOT-ONLY-VERIFY", "SEC-0127"),
    ("P-SEC-0127-SNAPSHOT-ONLY-VERIFY-2", "SEC-0127"),
];

#[derive(Clone)]
pub struct StageRequest {
    pub dsn: String,
    pub source_manifest: PathBuf,
    pub source_root: Option<PathBuf>,
    pub repository_order: Option<PathBuf>,
    pub legacy_alias_manifest: Option<PathBuf>,
    pub legacy_alias_root: Option<PathBuf>,
    pub legacy_id_index_manifest: Option<PathBuf>,
    pub legacy_id_index_root: Option<PathBuf>,
    pub recovery_job_manifest: Option<PathBuf>,
    pub staged_generation: PathBuf,
    pub report_path: PathBuf,
}

impl std::fmt::Debug for StageRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StageRequest")
            .field("dsn", &"[REDACTED]")
            .field("source_manifest", &self.source_manifest)
            .field("source_root", &self.source_root)
            .field("repository_order", &self.repository_order)
            .field("legacy_alias_manifest", &self.legacy_alias_manifest)
            .field("legacy_alias_root", &self.legacy_alias_root)
            .field("legacy_id_index_manifest", &self.legacy_id_index_manifest)
            .field("legacy_id_index_root", &self.legacy_id_index_root)
            .field("recovery_job_manifest", &self.recovery_job_manifest)
            .field("staged_generation", &self.staged_generation)
            .field("report_path", &self.report_path)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StageResult {
    pub repository_count: u32,
    pub worker_job_count: u64,
    pub staged_generation: PathBuf,
    pub report_path: PathBuf,
}

#[derive(Clone, Debug)]
struct PreparedRepository {
    source: SourceRepositoryRow,
    repository_index: u32,
    namespace_ascii: [u8; 2],
    policy_flags: u8,
    lifecycle_kind: u8,
    created_at_s: u64,
    updated_at_s: u64,
    storage_generation: u64,
    domain: RepositoryDomain,
    jobs: Vec<PreparedJob>,
}

#[derive(Clone, Debug)]
struct PreparedJob {
    source_job_id: i64,
    source_patchset_id: Option<String>,
    worker_job_index: u32,
    record: ServerWorkerJobRecord,
    patchset_index: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourcePatchsetIdentityKind {
    Canonical,
    LegacyManifestAlias,
    LegacySnapshotAlias,
}

#[derive(Clone, Debug, Serialize)]
struct ConversionReport {
    schema: &'static str,
    layout_id: u32,
    status: &'static str,
    source_database: String,
    source_tables: [&'static str; 2],
    source_inventory_sha256: String,
    legacy_alias_manifest_sha256: Option<String>,
    legacy_id_index_manifest_sha256: Option<String>,
    recovery_job_manifest_sha256: Option<String>,
    source_repository_count: u64,
    source_job_count: u64,
    target_repository_count: u32,
    target_worker_job_count: u64,
    excluded_source_rows_serialized: u64,
    recovery_job_classifications: Vec<RecoveryJobClassification>,
    repository_mappings: Vec<RepositoryMappingReport>,
    worker_job_mappings: Vec<WorkerJobMappingReport>,
    authority_files: Vec<FileReport>,
}

#[derive(Clone, Debug, Serialize)]
struct RepositoryMappingReport {
    source_repo_id: String,
    source_repo_name: String,
    repository_index: u32,
    target_directory: String,
}

#[derive(Clone, Debug, Serialize)]
struct WorkerJobMappingReport {
    source_job_id: i64,
    repository_index: u32,
    worker_job_index: u32,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct FileReport {
    pub(crate) relative_path: String,
    pub(crate) byte_size: u64,
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryOrderDocument {
    schema: String,
    repositories: Vec<RepositoryOrderEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryOrderEntry {
    repository_index: u32,
    repo_id: String,
    repo_name: String,
}

pub fn stage_snapshot(
    snapshot: &SourceSnapshot,
    request: StageRequest,
) -> Result<StageResult, String> {
    validate_snapshot_envelope(snapshot)?;
    let (source_manifest, source_generation_root) =
        load_source_manifest(&request.source_manifest, request.source_root.as_deref())?;
    let repository_order = request
        .repository_order
        .as_deref()
        .map(load_repository_order)
        .transpose()?;
    if request.legacy_alias_root.is_some() && request.legacy_alias_manifest.is_none() {
        return Err("--legacy-alias-root requires --legacy-alias-manifest".to_string());
    }
    if request.legacy_id_index_root.is_some() && request.legacy_id_index_manifest.is_none() {
        return Err("--legacy-id-index-root requires --legacy-id-index-manifest".to_string());
    }
    if request.legacy_id_index_manifest.is_some() && request.legacy_alias_manifest.is_none() {
        return Err("--legacy-id-index-manifest requires --legacy-alias-manifest".to_string());
    }
    let mut legacy_alias_catalog = match request.legacy_alias_manifest.as_deref() {
        Some(path) => Some(LegacyPatchsetCatalog::load(
            path,
            request.legacy_alias_root.as_deref(),
        )?),
        None => None,
    };
    if let Some(path) = request.legacy_id_index_manifest.as_deref() {
        legacy_alias_catalog
            .as_mut()
            .ok_or_else(|| "legacy ID index source has no alias catalog".to_string())?
            .apply_id_index_source(path, request.legacy_id_index_root.as_deref())?;
    }
    if let Some(catalog) = legacy_alias_catalog.as_ref() {
        catalog.validate_production_cutover(&snapshot.jobs)?;
    }
    let recovery_job_policy = request
        .recovery_job_manifest
        .as_deref()
        .map(|path| RecoveryJobPolicy::load(path, snapshot))
        .transpose()?;
    let mut repositories = prepare_repositories(
        snapshot,
        &source_manifest,
        &source_generation_root,
        repository_order.as_ref(),
    )?;
    if let Some(catalog) = legacy_alias_catalog.as_mut() {
        for repository in &repositories {
            if catalog.has_repository(&repository.source.repo_id) {
                catalog.bind_v0_repository(
                    &repository.source.repo_id,
                    &repository.source.repo_name,
                    &repository.domain,
                )?;
            }
        }
        catalog.validate_v0_bindings_complete()?;
    }
    let provisional_job_map = assign_job_indexes(snapshot, &mut repositories, &BTreeSet::new())?;
    let provisional_audit = prepare_jobs(
        snapshot,
        &provisional_job_map,
        &mut repositories,
        legacy_alias_catalog.as_ref(),
        recovery_job_policy.is_some(),
    )?;
    if let Some(policy) = recovery_job_policy.as_ref() {
        policy.validate_audit(&provisional_audit)?;
    } else if !provisional_audit.classifications().is_empty() {
        return Err("Worker Job recovery classification occurred without a manifest".to_string());
    }
    let omitted_job_ids = provisional_audit.omitted_ids();
    let recovery_audit = if omitted_job_ids.is_empty() {
        provisional_audit
    } else {
        let dense_job_map = assign_job_indexes(snapshot, &mut repositories, &omitted_job_ids)?;
        let dense_audit = prepare_jobs(
            snapshot,
            &dense_job_map,
            &mut repositories,
            legacy_alias_catalog.as_ref(),
            recovery_job_policy.is_some(),
        )?;
        if dense_audit.classifications() != provisional_audit.classifications() {
            return Err(
                "Worker Job recovery classification changed after dense omission remap".to_string(),
            );
        }
        if let Some(policy) = recovery_job_policy.as_ref() {
            policy.validate_audit(&dense_audit)?;
        }
        dense_audit
    };
    for repository in &repositories {
        repository
            .domain
            .validate_tree_authority(
                &repository.source.repo_id,
                &repository.source.repo_name,
                repository.storage_generation,
            )
            .map_err(|error| {
                format!(
                    "source Repository {} ({}) authority is invalid: {error}",
                    repository.source.repo_id, repository.source.repo_name
                )
            })?;
    }

    let staged_generation = create_empty_staged_generation(&request.staged_generation)?;
    let global_root = staged_generation.join("global");
    let repositories_root = staged_generation.join("repositories");
    fs::create_dir(&global_root)
        .map_err(|error| format!("failed to create {}: {error}", global_root.display()))?;
    fs::create_dir(&repositories_root)
        .map_err(|error| format!("failed to create {}: {error}", repositories_root.display()))?;
    write_repository_registry(&global_root, &repositories)?;
    for repository in &repositories {
        write_repository_authority(&repositories_root, repository)?;
    }
    write_generation_manifest(&staged_generation, &repositories)?;
    validate_target(&global_root, &repositories_root, &repositories)?;

    let authority_files = inventory_authority_files(&staged_generation)?;
    let inventory_bytes = serde_json::to_vec(&snapshot.inventory_before)
        .map_err(|error| format!("failed to encode source inventory evidence: {error}"))?;
    let report = ConversionReport {
        schema: REPORT_SCHEMA,
        layout_id: 1,
        status: "validated_inactive",
        source_database: snapshot.database_name.clone(),
        source_tables: ["ait_native_content.repositories", "ait_native_control.jobs"],
        source_inventory_sha256: sha256(&inventory_bytes),
        legacy_alias_manifest_sha256: legacy_alias_catalog
            .as_ref()
            .map(|catalog| catalog.manifest_sha256().to_string()),
        legacy_id_index_manifest_sha256: legacy_alias_catalog
            .as_ref()
            .and_then(|catalog| catalog.id_index_manifest_sha256().map(str::to_string)),
        recovery_job_manifest_sha256: recovery_job_policy
            .as_ref()
            .map(|policy| policy.manifest_sha256().to_string()),
        source_repository_count: snapshot.inventory_before.repository_count,
        source_job_count: snapshot.inventory_before.job_count,
        target_repository_count: u32::try_from(repositories.len())
            .map_err(|_| "target Repository count exceeds u32".to_string())?,
        target_worker_job_count: repositories.iter().try_fold(0_u64, |count, repository| {
            count
                .checked_add(repository.jobs.len() as u64)
                .ok_or_else(|| "target Worker Job count exceeds u64".to_string())
        })?,
        excluded_source_rows_serialized: recovery_audit.omitted_ids().len() as u64,
        recovery_job_classifications: recovery_audit.classifications(),
        repository_mappings: repositories
            .iter()
            .map(|repository| RepositoryMappingReport {
                source_repo_id: repository.source.repo_id.clone(),
                source_repo_name: repository.source.repo_name.clone(),
                repository_index: repository.repository_index,
                target_directory: format!("repositories/{}", repository.repository_index),
            })
            .collect(),
        worker_job_mappings: repositories
            .iter()
            .flat_map(|repository| {
                repository
                    .jobs
                    .iter()
                    .map(move |job| WorkerJobMappingReport {
                        source_job_id: job.source_job_id,
                        repository_index: repository.repository_index,
                        worker_job_index: job.worker_job_index,
                    })
            })
            .collect(),
        authority_files,
    };
    let mut report_bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("failed to encode conversion report: {error}"))?;
    report_bytes.push(b'\n');
    let report_path = absolute_new_file_path(&request.report_path)?;
    if report_path.starts_with(&staged_generation) {
        return Err(
            "external conversion report path must be outside the staged generation".to_string(),
        );
    }
    write_new_sync(&report_path, &report_bytes)?;
    write_new_sync(
        &staged_generation.join("conversion-report.json"),
        &report_bytes,
    )?;
    write_completion(&staged_generation, sha256(&report_bytes))?;
    sync_directory(&staged_generation)?;

    Ok(StageResult {
        repository_count: report.target_repository_count,
        worker_job_count: report.target_worker_job_count,
        staged_generation,
        report_path,
    })
}

fn validate_snapshot_envelope(snapshot: &SourceSnapshot) -> Result<(), String> {
    if snapshot.database_name != SOURCE_DATABASE {
        return Err(format!(
            "conversion source database must be {SOURCE_DATABASE:?}"
        ));
    }
    if snapshot.inventory_before != snapshot.inventory_after {
        return Err("source inventory changed across the conversion snapshot".to_string());
    }
    if snapshot.inventory_before.repository_count != snapshot.repositories.len() as u64
        || snapshot.inventory_before.job_count != snapshot.jobs.len() as u64
    {
        return Err("source inventory counts disagree with supplied rows".to_string());
    }
    Ok(())
}

fn load_source_manifest(
    path: &Path,
    source_root: Option<&Path>,
) -> Result<(SourceManifest, PathBuf), String> {
    let path = fs::canonicalize(path).map_err(|error| {
        format!(
            "failed to canonicalize source manifest {}: {error}",
            path.display()
        )
    })?;
    let manifest: SourceManifest =
        serde_json::from_slice(&read_regular_file(&path)?).map_err(|error| {
            format!(
                "failed to parse source manifest {}: {error}",
                path.display()
            )
        })?;
    if manifest.schema != SOURCE_MANIFEST_SCHEMA
        || manifest.status != "current"
        || manifest.authority_backend != "remote-binary-db"
        || manifest.layout_id != 1
        || manifest.repositories.is_empty()
    {
        return Err(
            "source manifest is not a current layout-1 remote Binary authority".to_string(),
        );
    }
    let root = source_root.unwrap_or(
        path.parent()
            .ok_or_else(|| "source manifest has no parent".to_string())?,
    );
    Ok((manifest, canonical_real_directory(root)?))
}

fn load_repository_order(path: &Path) -> Result<RepositoryOrderDocument, String> {
    let path = fs::canonicalize(path).map_err(|error| {
        format!(
            "failed to canonicalize Repository order {}: {error}",
            path.display()
        )
    })?;
    let order: RepositoryOrderDocument = serde_json::from_slice(&read_regular_file(&path)?)
        .map_err(|error| {
            format!(
                "failed to parse Repository order {}: {error}",
                path.display()
            )
        })?;
    if order.schema != REPOSITORY_ORDER_SCHEMA {
        return Err(format!(
            "Repository order schema must be exact {REPOSITORY_ORDER_SCHEMA:?}"
        ));
    }
    Ok(order)
}

fn prepare_repositories(
    snapshot: &SourceSnapshot,
    manifest: &SourceManifest,
    source_generation_root: &Path,
    repository_order: Option<&RepositoryOrderDocument>,
) -> Result<Vec<PreparedRepository>, String> {
    if snapshot.repositories.len() >= u32::MAX as usize {
        return Err("source Repository count exceeds Binary DB v0 capacity".to_string());
    }
    let mut by_id = BTreeMap::new();
    let mut reserved = BTreeMap::<&str, Vec<SourceRepositoryRow>>::new();
    let mut namespaces = BTreeMap::<[u8; 2], String>::new();
    for source in &snapshot.repositories {
        validate_non_empty_exact(&source.repo_id, "Repository repo_id")?;
        validate_repo_id(&source.repo_id)?;
        validate_non_empty_exact(&source.repo_name, "Repository repo_name")?;
        if source.default_line != "main" {
            return Err(format!(
                "Repository {} default_line must be exact main",
                source.repo_id
            ));
        }
        let namespace_ascii = parse_namespace(&source.id_namespace_prefix)?;
        if namespace_ascii != [0, 0] {
            if let Some(previous) = namespaces.insert(namespace_ascii, source.repo_id.clone()) {
                return Err(format!(
                    "source Repositories {previous} and {} share non-empty namespace {:?}",
                    source.repo_id, namespace_ascii
                ));
            }
        }
        if by_id
            .insert(source.repo_id.clone(), source.clone())
            .is_some()
        {
            return Err(format!("duplicate source Repository ID {}", source.repo_id));
        }
        if let Some(name) = FIXED_REPOSITORY_NAMES
            .iter()
            .find(|name| source.repo_name == **name)
        {
            reserved.entry(name).or_default().push(source.clone());
        }
    }
    for name in FIXED_REPOSITORY_NAMES {
        if reserved.get(name).map(Vec::len) != Some(1) {
            return Err(format!(
                "source must contain exactly one reserved Repository name {name}"
            ));
        }
    }

    let ordered = ordered_source_repositories(snapshot, &by_id, &reserved, repository_order)?;

    let mut manifest_by_id = BTreeMap::new();
    let mut manifest_roots = BTreeSet::new();
    for entry in &manifest.repositories {
        validate_relative_path(&entry.authority_relative_path)?;
        if entry.storage_generation == 0
            || manifest_by_id
                .insert(entry.repo_id.clone(), entry.clone())
                .is_some()
        {
            return Err(format!(
                "source manifest has duplicate or invalid Repository {}",
                entry.repo_id
            ));
        }
    }
    if manifest_by_id.len() != ordered.len() {
        return Err("source manifest and PostgreSQL Repository inventories differ".to_string());
    }

    ordered
        .into_iter()
        .enumerate()
        .map(|(index, source)| {
            let manifest = manifest_by_id.remove(&source.repo_id).ok_or_else(|| {
                format!(
                    "source Repository {} has no exact manifest authority",
                    source.repo_id
                )
            })?;
            if manifest.repo_name != source.repo_name {
                return Err(format!(
                    "source manifest name disagrees for Repository {}",
                    source.repo_id
                ));
            }
            let source_root = source_generation_root.join(&manifest.authority_relative_path);
            let canonical_source_root = canonical_real_directory(&source_root)?;
            if !manifest_roots.insert(canonical_source_root.clone()) {
                return Err(format!(
                    "multiple source Repositories resolve to authority {}",
                    canonical_source_root.display()
                ));
            }
            let namespace_ascii = parse_namespace(&source.id_namespace_prefix)?;
            let policy_flags = parse_policy_flags(&source.policy_json)?;
            let lifecycle_kind = parse_lifecycle(&source.lifecycle_state)?;
            let created_at_s = timestamp_s(source.created_at, "Repository created_at")?;
            let updated_at_s = timestamp_s(source.updated_at, "Repository updated_at")?;
            if updated_at_s < created_at_s {
                return Err(format!(
                    "Repository {} update time precedes creation time",
                    source.repo_id
                ));
            }
            let domain = RepositoryDomain::load(
                &canonical_source_root,
                &source.repo_id,
                &source.repo_name,
                manifest.storage_generation,
                namespace_ascii,
            )
            .map_err(|error| {
                format!(
                    "source Repository {} ({}) authority is invalid: {error}",
                    source.repo_id, source.repo_name
                )
            })?;
            Ok(PreparedRepository {
                source,
                repository_index: u32::try_from(index)
                    .map_err(|_| "Repository index exceeds u32".to_string())?,
                namespace_ascii,
                policy_flags,
                lifecycle_kind,
                created_at_s,
                updated_at_s,
                storage_generation: manifest.storage_generation,
                domain,
                jobs: Vec::new(),
            })
        })
        .collect()
}

fn ordered_source_repositories(
    snapshot: &SourceSnapshot,
    by_id: &BTreeMap<String, SourceRepositoryRow>,
    reserved: &BTreeMap<&str, Vec<SourceRepositoryRow>>,
    repository_order: Option<&RepositoryOrderDocument>,
) -> Result<Vec<SourceRepositoryRow>, String> {
    let Some(order) = repository_order else {
        let mut ordered = FIXED_REPOSITORY_NAMES
            .iter()
            .map(|name| reserved.get(name).unwrap()[0].clone())
            .collect::<Vec<_>>();
        let mut remainder = snapshot
            .repositories
            .iter()
            .filter(|source| !FIXED_REPOSITORY_NAMES.contains(&source.repo_name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        remainder.sort_by(|left, right| left.repo_id.as_bytes().cmp(right.repo_id.as_bytes()));
        ordered.extend(remainder);
        return Ok(ordered);
    };

    if order.repositories.len() != snapshot.repositories.len() {
        return Err(format!(
            "explicit Repository order has {} entries for {} source Repositories",
            order.repositories.len(),
            snapshot.repositories.len()
        ));
    }
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::with_capacity(order.repositories.len());
    for (expected_index, entry) in order.repositories.iter().enumerate() {
        let expected_index = u32::try_from(expected_index)
            .map_err(|_| "explicit Repository order exceeds u32".to_string())?;
        if entry.repository_index != expected_index {
            return Err(format!(
                "explicit Repository order must be dense: expected index {expected_index}, got {}",
                entry.repository_index
            ));
        }
        if !seen.insert(entry.repo_id.clone()) {
            return Err(format!(
                "explicit Repository order repeats Repository ID {}",
                entry.repo_id
            ));
        }
        let source = by_id.get(&entry.repo_id).ok_or_else(|| {
            format!(
                "explicit Repository order references unknown Repository ID {}",
                entry.repo_id
            )
        })?;
        if source.repo_name != entry.repo_name {
            return Err(format!(
                "explicit Repository order name disagrees for Repository {}",
                entry.repo_id
            ));
        }
        if let Some(expected_name) = FIXED_REPOSITORY_NAMES.get(expected_index as usize) {
            if source.repo_name != *expected_name {
                return Err(format!(
                    "explicit Repository index {expected_index} must remain reserved for {expected_name}"
                ));
            }
        }
        ordered.push(source.clone());
    }
    Ok(ordered)
}

fn assign_job_indexes(
    snapshot: &SourceSnapshot,
    repositories: &mut [PreparedRepository],
    excluded_job_ids: &BTreeSet<i64>,
) -> Result<BTreeMap<i64, (u32, u32)>, String> {
    let repository_indexes = repositories
        .iter()
        .map(|repository| {
            (
                repository.source.repo_id.clone(),
                (
                    repository.repository_index,
                    repository.source.repo_name.clone(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut by_repository = BTreeMap::<u32, Vec<&SourceJobRow>>::new();
    let mut source_job_ids = BTreeSet::new();
    for job in &snapshot.jobs {
        if job.job_id <= 0 || !source_job_ids.insert(job.job_id) {
            return Err(format!(
                "source Job ID {} is non-positive or duplicated",
                job.job_id
            ));
        }
        validate_non_empty_exact(&job.repo_id, "Job repo_id")?;
        validate_non_empty_exact(&job.repo_name, "Job repo_name")?;
        let (repository_index, expected_name) = repository_indexes
            .get(&job.repo_id)
            .ok_or_else(|| format!("Job {} references unknown Repository ID", job.job_id))?;
        if job.repo_name != *expected_name {
            return Err(format!(
                "Job {} duplicated Repository name disagrees with Repository {}",
                job.job_id, job.repo_id
            ));
        }
        if !excluded_job_ids.contains(&job.job_id) {
            by_repository
                .entry(*repository_index)
                .or_default()
                .push(job);
        }
    }
    let mut map = BTreeMap::new();
    for repository in repositories {
        let mut jobs = by_repository
            .remove(&repository.repository_index)
            .unwrap_or_default();
        jobs.sort_by_key(|job| job.job_id);
        if jobs.len() >= u32::MAX as usize {
            return Err(format!(
                "Repository {} Job count exceeds Binary DB v0 capacity",
                repository.source.repo_id
            ));
        }
        for (index, job) in jobs.into_iter().enumerate() {
            let worker_job_index =
                u32::try_from(index).map_err(|_| "Worker Job index exceeds u32".to_string())?;
            map.insert(job.job_id, (repository.repository_index, worker_job_index));
        }
    }
    Ok(map)
}

fn prepare_jobs(
    snapshot: &SourceSnapshot,
    job_map: &BTreeMap<i64, (u32, u32)>,
    repositories: &mut [PreparedRepository],
    legacy_alias_catalog: Option<&LegacyPatchsetCatalog>,
    recovery_enabled: bool,
) -> Result<RecoveryJobAudit, String> {
    let mut conversion_errors = Vec::<(i64, String)>::new();
    let mut recovery_audit = RecoveryJobAudit::default();
    let source_by_id = snapshot
        .jobs
        .iter()
        .map(|job| (job.job_id, job))
        .collect::<BTreeMap<_, _>>();
    let repository_index_by_id = repositories
        .iter()
        .map(|repository| {
            (
                repository.source.repo_id.clone(),
                repository.repository_index,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let jobs_by_repository = snapshot.jobs.iter().fold(
        BTreeMap::<u32, Vec<&SourceJobRow>>::new(),
        |mut grouped, job| {
            let repository_index = repository_index_by_id[&job.repo_id];
            grouped.entry(repository_index).or_default().push(job);
            grouped
        },
    );
    for repository in repositories {
        let mut jobs = jobs_by_repository
            .get(&repository.repository_index)
            .cloned()
            .unwrap_or_default();
        jobs.sort_by_key(|job| job.job_id);
        let mut prepared = Vec::with_capacity(jobs.len());
        let mut legacy_aliases = BTreeMap::<String, (u32, i64)>::new();
        for job in jobs {
            let mapped_worker_job_index = job_map
                .get(&job.job_id)
                .map(|(_, worker_job_index)| *worker_job_index);
            let worker_job_index = mapped_worker_job_index.unwrap_or(0);
            let converted = match convert_job(
                job,
                worker_job_index,
                repository,
                &source_by_id,
                job_map,
                legacy_alias_catalog,
                recovery_enabled,
                &mut recovery_audit,
            ) {
                Ok(converted) => converted,
                Err(error) => {
                    if recovery_enabled {
                        if let Some(category) = recovery_omission_category(job, &error) {
                            recovery_audit.omit(category, job.job_id);
                            continue;
                        }
                    }
                    conversion_errors.push((job.job_id, error));
                    continue;
                }
            };
            if mapped_worker_job_index.is_none() {
                conversion_errors.push((
                    job.job_id,
                    format!(
                        "Job {} converted after its provisional recovery omission",
                        job.job_id
                    ),
                ));
                continue;
            }
            if let (Some(source_patchset_id), Some(patchset_index)) = (
                converted.source_patchset_id.as_deref(),
                converted.patchset_index,
            ) {
                if legacy_patchset_coordinates(source_patchset_id).is_some() {
                    match legacy_aliases
                        .insert(source_patchset_id.to_string(), (patchset_index, job.job_id))
                    {
                        Some((previous_index, previous_job_id))
                            if previous_index != patchset_index =>
                        {
                            conversion_errors.push((
                                job.job_id,
                                format!(
                                    "legacy Patchset {source_patchset_id:?} resolves inconsistently between Jobs {previous_job_id} and {}",
                                    job.job_id
                                ),
                            ));
                            continue;
                        }
                        _ => {}
                    }
                }
            }
            if repository.lifecycle_kind == REPOSITORY_LIFECYCLE_PURGED
                && converted.record.state_kind == WORKER_JOB_STATE_QUEUED
            {
                conversion_errors.push((
                    job.job_id,
                    format!(
                        "purged Repository {} retains live queued Job {}",
                        repository.source.repo_id, job.job_id
                    ),
                ));
                continue;
            }
            prepared.push(converted);
        }
        repository.jobs = prepared;
    }
    if conversion_errors.is_empty() {
        Ok(recovery_audit)
    } else {
        conversion_errors.sort_by_key(|(job_id, _)| *job_id);
        let count = conversion_errors.len();
        Err(format!(
            "Worker Job conversion rejected {count} source row(s):\n{}",
            conversion_errors
                .into_iter()
                .map(|(_, error)| error)
                .collect::<Vec<_>>()
                .join("\n")
        ))
    }
}

fn recovery_omission_category(job: &SourceJobRow, error: &str) -> Option<&'static str> {
    if error.contains("explicit Patchset")
        && error.contains("at physical index")
        && error.contains("absent from the frozen v0 prefix mapping")
        && error.contains("no exact appended v0 identity")
    {
        return Some(LEGACY_PATCHSET_OMISSION);
    }
    if job.result_json.contains("\"status\":\"attached\"")
        || job.result_json.contains("\"status\": \"attached\"")
    {
        if error.contains("result lacks a valid related Job ID")
            || error.contains("historical attached outcome has no distinct related Job")
        {
            return Some(UNPROVABLE_ATTACHED_OMISSION);
        }
    }
    if error.contains("legacy Snapshot-only Patchset alias")
        && error.contains("resolved to 0 candidates")
    {
        return Some(SNAPSHOT_ONLY_NO_PATCHSET_OMISSION);
    }
    if error.contains("Job target_line must be absent or exact main") {
        return Some(NON_MAIN_TARGET_OMISSION);
    }
    None
}

fn convert_job(
    job: &SourceJobRow,
    worker_job_index: u32,
    repository: &PreparedRepository,
    source_by_id: &BTreeMap<i64, &SourceJobRow>,
    job_map: &BTreeMap<i64, (u32, u32)>,
    legacy_alias_catalog: Option<&LegacyPatchsetCatalog>,
    recovery_enabled: bool,
    recovery_audit: &mut RecoveryJobAudit,
) -> Result<PreparedJob, String> {
    validate_non_empty_exact(&job.job_type, "Job job_type")?;
    validate_non_empty_exact(&job.state, "Job state")?;
    if job.locked_at.is_some() || job.locked_by.is_some() {
        return Err(format!(
            "Job {} is not quiesced: locked_at and locked_by must both be SQL NULL",
            job.job_id
        ));
    }
    if job.state == "running" {
        return Err(format!(
            "Job {} is running; running Jobs cannot be converted",
            job.job_id
        ));
    }
    let job_kind = job_kind(&job.job_type)?;
    let payload = parse_object_without_duplicates(
        &job.payload_json,
        &format!("Job {} payload_json", job.job_id),
    )?;
    validate_source_payload_contract(&job.job_type, &payload)?;
    validate_discardable_runtime_context(&payload)?;
    validate_repository_payload_identity(job, repository, &payload)?;
    let source_patchset_id = optional_exact_text(&payload, "patchset_id")?;
    let result = parse_object_without_duplicates(
        &job.result_json,
        &format!("Job {} result_json", job.job_id),
    )?;
    let (patchset_index, snapshot_index) = resolve_job_domain_references(
        job.job_id,
        job_kind,
        &payload,
        &result,
        repository,
        legacy_alias_catalog,
        recovery_enabled,
        recovery_audit,
    )
    .map_err(|error| format!("Job {} domain reference is invalid: {error}", job.job_id))?;
    let (state_kind, outcome_kind, error_kind) = convert_state(
        job,
        &result,
        repository.repository_index,
        source_by_id,
        job_map,
        recovery_enabled,
        recovery_audit,
    )?;
    validate_result_domain_values(
        job,
        &result,
        &payload,
        repository,
        patchset_index,
        snapshot_index,
        source_patchset_id,
        recovery_enabled,
    )
    .map_err(|error| format!("Job {} result domain is invalid: {error}", job.job_id))?;

    let attempt_count = u16::try_from(job.attempt_count)
        .map_err(|_| format!("Job {} attempt_count is outside u16", job.job_id))?;
    let max_attempts = u16::try_from(job.max_attempts)
        .map_err(|_| format!("Job {} max_attempts is outside u16", job.job_id))?;
    let record = ServerWorkerJobRecord {
        job_meta: 0,
        job_kind,
        state_kind,
        outcome_kind,
        attempt_count,
        max_attempts,
        error_kind,
        reserved0: 0,
        patchset_index_plus1: plus_one(patchset_index, "Patchset")?,
        snapshot_index_plus1: plus_one(snapshot_index, "Snapshot")?,
        available_at_s: timestamp_s(job.available_at, "Job available_at")?,
        locked_at_s: 0,
        created_at_s: timestamp_s(job.created_at, "Job created_at")?,
        updated_at_s: timestamp_s(job.updated_at, "Job updated_at")?,
    };
    ServerOperationalBinaryV0Codec::encode_worker_job(record)
        .map_err(|error| format!("Job {} cannot encode: {error}", job.job_id))?;
    Ok(PreparedJob {
        source_job_id: job.job_id,
        source_patchset_id: source_patchset_id.map(str::to_string),
        worker_job_index,
        record,
        patchset_index,
    })
}

fn validate_source_payload_contract(
    job_type: &str,
    payload: &JsonMap<String, JsonValue>,
) -> Result<(), String> {
    let normalized = normalize_async_job_payload(job_type, payload)
        .map_err(|error| format!("source Async Job contract rejected payload: {error}"))?;
    for (field, value) in payload {
        if normalized.get(field) != Some(value) {
            return Err(format!(
                "{job_type} payload field {field:?} requires coercion or normalization"
            ));
        }
    }
    Ok(())
}

fn validate_discardable_runtime_context(
    payload: &JsonMap<String, JsonValue>,
) -> Result<(), String> {
    for field in ["runtime_payload", "runner_context"] {
        if payload.get(field).is_some_and(|value| !value.is_null()) {
            return Err(format!(
                "source Async Job field {field:?} is non-null and has no admitted reconstruction gate"
            ));
        }
    }
    Ok(())
}

fn validate_repository_payload_identity(
    job: &SourceJobRow,
    repository: &PreparedRepository,
    payload: &JsonMap<String, JsonValue>,
) -> Result<(), String> {
    if let Some(value) = optional_exact_text(payload, "repo_id")? {
        if value != repository.source.repo_id {
            return Err(format!("Job {} payload repo_id disagrees", job.job_id));
        }
    }
    if let Some(value) = optional_exact_text(payload, "repo_name")? {
        if value != repository.source.repo_name {
            return Err(format!("Job {} payload repo_name disagrees", job.job_id));
        }
    }
    Ok(())
}

fn resolve_job_domain_references(
    source_job_id: i64,
    job_kind: u8,
    payload: &JsonMap<String, JsonValue>,
    result: &JsonMap<String, JsonValue>,
    repository: &PreparedRepository,
    legacy_alias_catalog: Option<&LegacyPatchsetCatalog>,
    recovery_enabled: bool,
    recovery_audit: &mut RecoveryJobAudit,
) -> Result<(Option<u32>, Option<u32>), String> {
    match job_kind {
        WORKER_JOB_KIND_CONTENT_GC
        | WORKER_JOB_KIND_CONTENT_OPTIMIZE
        | WORKER_JOB_KIND_CONTENT_PACK
        | WORKER_JOB_KIND_RECONCILE_REPO => Ok((None, None)),
        WORKER_JOB_KIND_LAND_PROCESS => {
            validate_direct_main_land(payload)?;
            let revision_snapshot_hint =
                patchset_revision_snapshot_hint(job_kind, payload, result, true)?;
            let submission_id = required_exact_text(payload, "submission_id")?;
            let patchset_index = repository.domain.direct_main_land_patchset(submission_id)?;
            let patchset = repository
                .domain
                .patchsets_by_index
                .get(patchset_index as usize)
                .copied()
                .ok_or_else(|| "Land accepted Patchset is missing".to_string())?;
            let change = repository
                .domain
                .patchsets_by_id
                .values()
                .find(|identity| identity.index == patchset_index)
                .copied()
                .ok_or_else(|| "Land accepted Patchset identity is ambiguous".to_string())?;
            let (source_patchset, source_kind) = resolve_patchset(
                source_job_id,
                payload,
                repository,
                legacy_alias_catalog,
                revision_snapshot_hint.as_deref(),
                recovery_enabled,
                recovery_audit,
            )?;
            if source_patchset.index != change.index {
                return Err(
                    "land.process patchset_id disagrees with the Land accepted Patchset"
                        .to_string(),
                );
            }
            validate_patchset_redundancy(payload, repository, change, source_kind, false)?;
            if let Some(land_seq) = optional_positive_u32(payload, "land_seq")? {
                let land = repository.domain.lands_by_id[submission_id].record;
                if land_seq != u32::from(land.land_ordinal) + 1 {
                    return Err("land.process land_seq disagrees with Land authority".to_string());
                }
            }
            let _ = patchset;
            Ok((Some(patchset_index), None))
        }
        WORKER_JOB_KIND_MAIN_SEED_REFRESH => {
            validate_logical_main(payload)?;
            let revision_snapshot_hint =
                patchset_revision_snapshot_hint(job_kind, payload, result, true)?;
            let normal = (|| {
                let (patchset, source_kind) = resolve_patchset(
                    source_job_id,
                    payload,
                    repository,
                    legacy_alias_catalog,
                    revision_snapshot_hint.as_deref(),
                    recovery_enabled,
                    recovery_audit,
                )?;
                validate_patchset_redundancy(payload, repository, patchset, source_kind, false)?;
                let snapshot_id = required_exact_text(payload, "snapshot_id")?;
                if repository
                    .domain
                    .snapshot_id(patchset.record.revision_snapshot_index)?
                    != snapshot_id
                {
                    return Err(
                        "main-seed.refresh snapshot_id is not the Patchset revision Snapshot"
                            .to_string(),
                    );
                }
                let previous = optional_exact_text(payload, "previous_snapshot_id")?
                    .map(|snapshot_id| repository.domain.snapshot(snapshot_id))
                    .transpose()?;
                Ok((Some(patchset.index), previous))
            })();
            match normal {
                Ok(value) => Ok(value),
                Err(_normal_error) if recovery_enabled => {
                    let fallback_hint =
                        patchset_revision_snapshot_hint(job_kind, payload, result, false)?;
                    let (patchset, source_kind) = resolve_patchset(
                        source_job_id,
                        payload,
                        repository,
                        legacy_alias_catalog,
                        fallback_hint.as_deref(),
                        recovery_enabled,
                        recovery_audit,
                    )?;
                    validate_patchset_redundancy(payload, repository, patchset, source_kind, true)?;
                    let landed_snapshot_id = required_exact_text(payload, "snapshot_id")?;
                    repository.domain.validate_successful_main_land_snapshot(
                        patchset.index,
                        landed_snapshot_id,
                    )?;
                    let previous = optional_exact_text(payload, "previous_snapshot_id")?
                        .map(|snapshot_id| repository.domain.snapshot(snapshot_id))
                        .transpose()?;
                    recovery_audit.normalize(MAIN_SEED_LANDED_SNAPSHOT, source_job_id);
                    Ok((Some(patchset.index), previous))
                }
                Err(error) => Err(error),
            }
        }
        WORKER_JOB_KIND_PATCHSET_CI
        | WORKER_JOB_KIND_PATCHSET_CI_AGGREGATE
        | WORKER_JOB_KIND_POLICY_EVALUATE => {
            let revision_snapshot_hint =
                patchset_revision_snapshot_hint(job_kind, payload, result, true)?;
            let (patchset, source_kind) = resolve_patchset(
                source_job_id,
                payload,
                repository,
                legacy_alias_catalog,
                revision_snapshot_hint.as_deref(),
                recovery_enabled,
                recovery_audit,
            )?;
            validate_patchset_redundancy(payload, repository, patchset, source_kind, false)?;
            Ok((Some(patchset.index), None))
        }
        WORKER_JOB_KIND_REPO_CI => {
            validate_logical_main(payload)?;
            let snapshot = match optional_exact_text(payload, "snapshot_id")? {
                Some(snapshot_id) => repository.domain.snapshot(snapshot_id)?,
                None => {
                    let main_head =
                        repository.domain.main_head_snapshot_index.ok_or_else(|| {
                            "repo.ci cannot derive an absent logical main head".to_string()
                        })?;
                    let result_snapshot = optional_exact_text(result, "snapshot_id")?
                        .map(|snapshot_id| repository.domain.snapshot(snapshot_id))
                        .transpose()?;
                    match result_snapshot {
                        Some(snapshot) if recovery_enabled && snapshot != main_head => {
                            recovery_audit.normalize(REPO_CI_RESULT_SNAPSHOT, source_job_id);
                            snapshot
                        }
                        _ => main_head,
                    }
                }
            };
            Ok((None, Some(snapshot)))
        }
        _ => Err("Worker Job kind is unassigned".to_string()),
    }
}

fn patchset_revision_snapshot_hint(
    job_kind: u8,
    payload: &JsonMap<String, JsonValue>,
    result: &JsonMap<String, JsonValue>,
    include_main_seed_snapshot: bool,
) -> Result<Option<String>, String> {
    fn nested_exact_text<'a>(
        object: &'a JsonMap<String, JsonValue>,
        path: &[&str],
    ) -> Result<Option<&'a str>, String> {
        let mut value = object.get(path[0]);
        for segment in &path[1..] {
            value = match value {
                None | Some(JsonValue::Null) => return Ok(None),
                Some(JsonValue::Object(object)) => object.get(*segment),
                Some(_) => {
                    return Err(format!(
                        "result_json.{} parent must be an Object",
                        path.join(".")
                    ))
                }
            };
        }
        match value {
            None | Some(JsonValue::Null) => Ok(None),
            Some(JsonValue::String(value)) if !value.is_empty() && value.trim() == value => {
                Ok(Some(value))
            }
            Some(_) => Err(format!(
                "result_json.{} must be null or exact non-empty Text",
                path.join(".")
            )),
        }
    }

    let mut snapshots = BTreeSet::new();
    if let Some(value) = optional_exact_text(payload, "revision_snapshot_id")? {
        snapshots.insert(value.to_string());
    }
    for path in [
        &["revision_snapshot_id"][..],
        &["patchset_ci_detail", "revision_snapshot_id"][..],
        &[
            "attestation",
            "detail",
            "patchset_ci",
            "revision_snapshot_id",
        ][..],
        &[
            "attestation_update",
            "detail",
            "patchset_ci",
            "revision_snapshot_id",
        ][..],
    ] {
        if let Some(value) = nested_exact_text(result, path)? {
            snapshots.insert(value.to_string());
        }
    }
    if job_kind == WORKER_JOB_KIND_MAIN_SEED_REFRESH && include_main_seed_snapshot {
        snapshots.insert(required_exact_text(payload, "snapshot_id")?.to_string());
    }
    if snapshots.len() > 1 {
        return Err(format!(
            "Patchset Job carries conflicting revision Snapshot hints: {}",
            snapshots.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(snapshots.into_iter().next())
}

fn resolve_patchset(
    source_job_id: i64,
    payload: &JsonMap<String, JsonValue>,
    repository: &PreparedRepository,
    legacy_alias_catalog: Option<&LegacyPatchsetCatalog>,
    revision_snapshot_hint: Option<&str>,
    recovery_enabled: bool,
    recovery_audit: &mut RecoveryJobAudit,
) -> Result<(PatchsetIdentity, SourcePatchsetIdentityKind), String> {
    let source_patchset_id = required_exact_text(payload, "patchset_id")?;
    let catalog =
        legacy_alias_catalog.filter(|catalog| catalog.has_repository(&repository.source.repo_id));
    let post_production_cutover = catalog
        .and_then(|catalog| catalog.is_post_production_cutover(source_job_id))
        .unwrap_or(false);
    let canonical = match repository.domain.patchset(source_patchset_id).ok() {
        Some(canonical) => Some(canonical),
        None if recovery_enabled
            && repository.source.repo_name == "ait"
            && source_patchset_id.starts_with("T-") =>
        {
            let corrected = format!("R{source_patchset_id}");
            let canonical = repository.domain.patchset(&corrected).ok();
            if canonical.is_some() {
                recovery_audit.normalize(MISSING_REMOTE_PREFIX, source_job_id);
            }
            canonical
        }
        None => None,
    };
    let explicit = catalog
        .map(|catalog| {
            catalog.exact_patchset(
                &repository.source.repo_id,
                &repository.source.repo_name,
                source_patchset_id,
            )
        })
        .transpose()?
        .flatten();

    if let Some(revision_snapshot_hint) =
        revision_snapshot_hint.filter(|_| canonical.is_some() || explicit.is_some())
    {
        let canonical_matches = canonical
            .map(|patchset| {
                repository
                    .domain
                    .snapshot_id(patchset.record.revision_snapshot_index)
                    .map(|value| value == revision_snapshot_hint)
            })
            .transpose()?
            .unwrap_or(false);
        let explicit_matches = explicit
            .as_ref()
            .is_some_and(|patchset| patchset.revision_snapshot_id == revision_snapshot_hint);
        match (canonical_matches, explicit_matches) {
            (true, false) => {
                return Ok((
                    canonical.expect("matching canonical Patchset"),
                    SourcePatchsetIdentityKind::Canonical,
                ))
            }
            (false, true) if post_production_cutover => {
                return Err(format!(
                    "post-production-cutover Job {source_job_id} Patchset {source_patchset_id:?} matches only pre-v0 explicit authority"
                ))
            }
            (false, true) => {
                let catalog = catalog.expect("matching explicit Patchset has a catalog");
                let explicit = explicit.as_ref().expect("matching explicit Patchset");
                return Ok((
                    historical_patchset_from_catalog(repository, catalog, explicit)?,
                    SourcePatchsetIdentityKind::LegacyManifestAlias,
                ));
            }
            (true, true) => {
                let canonical = canonical.expect("matching canonical Patchset");
                let explicit = explicit.as_ref().expect("matching explicit Patchset");
                if legacy_patchset_matches_identity(repository, explicit, canonical)? {
                    return Ok((canonical, SourcePatchsetIdentityKind::Canonical));
                }
                let historical = historical_patchset_from_catalog(
                    repository,
                    catalog.expect("matching explicit Patchset has a catalog"),
                    explicit,
                )?;
                if historical.index == canonical.index {
                    return Ok((canonical, SourcePatchsetIdentityKind::Canonical));
                }
                return Err(format!(
                    "Patchset {source_patchset_id:?} and revision Snapshot {revision_snapshot_hint:?} ambiguously select distinct explicit and v0 identities"
                ));
            }
            (false, false) => {
                return Err(format!(
                    "Patchset {source_patchset_id:?} has no explicit or v0 identity matching revision Snapshot {revision_snapshot_hint:?}"
                ))
            }
        }
    }

    match (canonical, explicit.as_ref()) {
        (Some(canonical), None) => {
            return Ok((canonical, SourcePatchsetIdentityKind::Canonical))
        }
        (None, Some(_)) if post_production_cutover => {
            return Err(format!(
                "post-production-cutover Job {source_job_id} references pre-v0-only Patchset {source_patchset_id:?}"
            ))
        }
        (None, Some(explicit)) => {
            return Ok((
                historical_patchset_from_catalog(
                    repository,
                    catalog.expect("explicit Patchset has a catalog"),
                    explicit,
                )?,
                SourcePatchsetIdentityKind::LegacyManifestAlias,
            ))
        }
        (Some(canonical), Some(_)) if post_production_cutover => {
            return Ok((canonical, SourcePatchsetIdentityKind::Canonical))
        }
        (Some(canonical), Some(explicit)) => {
            if legacy_patchset_matches_identity(repository, explicit, canonical)? {
                return Ok((canonical, SourcePatchsetIdentityKind::Canonical));
            }
            let historical = historical_patchset_from_catalog(
                repository,
                catalog.expect("explicit Patchset has a catalog"),
                explicit,
            )?;
            if historical.index == canonical.index {
                return Ok((canonical, SourcePatchsetIdentityKind::Canonical));
            }
            return Err(format!(
                "pre-production-cutover Patchset {source_patchset_id:?} is ambiguous without a revision Snapshot hint"
            ));
        }
        (None, None) => {}
    }

    if let Some((legacy_task_number, patchset_number)) =
        legacy_patchset_coordinates(source_patchset_id)
    {
        if post_production_cutover {
            return Err(format!(
                "post-v0 Job {source_job_id} references legacy Patchset alias {source_patchset_id:?}"
            ));
        }
        let canonical_patchset_id = legacy_canonical_patchset_id(
            source_patchset_id,
            repository.namespace_ascii,
            legacy_task_number,
            patchset_number,
        )?;
        if let Some(change_id) = optional_exact_text(payload, "change_id")? {
            validate_legacy_source_change_id(change_id, source_patchset_id)?;
        }
        let catalog = catalog.ok_or_else(|| {
            format!(
                "legacy Patchset {source_patchset_id:?} requires an explicit --legacy-alias-manifest"
            )
        })?;
        let legacy = catalog.patchset(
            &repository.source.repo_id,
            &repository.source.repo_name,
            source_patchset_id,
            &canonical_patchset_id,
            patchset_number,
            revision_snapshot_hint,
        )?;
        if legacy.patchset_number != patchset_number {
            return Err(format!(
                "legacy Patchset {source_patchset_id:?} ordinal disagrees with explicit alias authority"
            ));
        }
        let patchset = historical_patchset_from_catalog(repository, catalog, &legacy)?;
        return Ok((patchset, SourcePatchsetIdentityKind::LegacyManifestAlias));
    }

    if post_production_cutover {
        return Err(format!(
            "post-v0 Job {source_job_id} references unknown same-Repository Patchset {source_patchset_id:?}"
        ));
    }

    let expected_change_id = LEGACY_SNAPSHOT_PATCHSET_ALIASES
        .iter()
        .find_map(|(patchset_id, change_id)| {
            (*patchset_id == source_patchset_id).then_some(*change_id)
        })
        .ok_or_else(|| format!("unknown same-Repository Patchset {source_patchset_id:?}"))?;
    if required_exact_text(payload, "change_id")? != expected_change_id {
        return Err(format!(
            "legacy Snapshot-only Patchset alias {source_patchset_id:?} has an unexpected change_id"
        ));
    }
    let snapshot_id = required_exact_text(payload, "snapshot_id")?;
    let snapshot_index = repository.domain.snapshot(snapshot_id)?;
    let ordinal_hint = source_patchset_id
        .rsplit_once("-p")
        .and_then(|(_, value)| parse_positive_decimal(value));
    let candidates = repository
        .domain
        .patchsets_by_index
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, patchset)| patchset.revision_snapshot_index == snapshot_index)
        .filter(|(_, patchset)| {
            ordinal_hint.is_none_or(|ordinal| u32::from(patchset.patch_ordinal) + 1 == ordinal)
        })
        .map(|(index, _)| {
            repository
                .domain
                .patchsets_by_id
                .values()
                .find(|identity| identity.index == index as u32)
                .copied()
                .ok_or_else(|| {
                    format!(
                        "legacy Snapshot-only alias {source_patchset_id:?} has no Patchset identity"
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if candidates.len() != 1 {
        return Err(format!(
            "legacy Snapshot-only Patchset alias {source_patchset_id:?} resolved to {} candidates",
            candidates.len()
        ));
    }
    Ok((
        candidates[0],
        SourcePatchsetIdentityKind::LegacySnapshotAlias,
    ))
}

fn legacy_patchset_matches_identity(
    repository: &PreparedRepository,
    legacy: &LegacyPatchsetIdentity,
    current: PatchsetIdentity,
) -> Result<bool, String> {
    Ok(legacy.patchset_number == current.patchset_number()
        && legacy.base_snapshot_id
            == repository
                .domain
                .snapshot_id(current.record.base_snapshot_index)?
        && legacy.revision_snapshot_id
            == repository
                .domain
                .snapshot_id(current.record.revision_snapshot_index)?)
}

fn validate_patchset_redundancy(
    payload: &JsonMap<String, JsonValue>,
    repository: &PreparedRepository,
    patchset: PatchsetIdentity,
    source_kind: SourcePatchsetIdentityKind,
    allow_landed_snapshot: bool,
) -> Result<(), String> {
    let source_patchset_id = required_exact_text(payload, "patchset_id")?;
    if let Some(value) = optional_positive_u32(payload, "patchset_number")? {
        if value != patchset.patchset_number() {
            return Err("payload patchset_number disagrees with Patchset authority".to_string());
        }
    }
    if let Some(value) = optional_positive_u32(payload, "change_seq")? {
        if source_kind == SourcePatchsetIdentityKind::Canonical
            && value != patchset.change_sequence()
        {
            return Err("payload change_seq disagrees with Change authority".to_string());
        }
    }
    if let Some(value) = optional_exact_text(payload, "change_id")? {
        validate_source_patchset_change_id(value, source_patchset_id)?;
    }
    let revision_snapshot_id = repository
        .domain
        .snapshot_id(patchset.record.revision_snapshot_index)?;
    if let Some(value) = optional_exact_text(payload, "revision_snapshot_id")? {
        if value != revision_snapshot_id {
            return Err(
                "payload revision_snapshot_id disagrees with Patchset authority".to_string(),
            );
        }
    }
    if let Some(value) = optional_exact_text(payload, "snapshot_id")? {
        if !allow_landed_snapshot && value != revision_snapshot_id {
            return Err("payload snapshot_id disagrees with Patchset authority".to_string());
        }
    }
    Ok(())
}

fn historical_patchset_from_catalog(
    repository: &PreparedRepository,
    catalog: &LegacyPatchsetCatalog,
    legacy: &crate::legacy_alias_source::LegacyPatchsetIdentity,
) -> Result<PatchsetIdentity, String> {
    let target_index = catalog.target_patchset_index(
        &repository.source.repo_id,
        &repository.source.repo_name,
        legacy.physical_index,
    )?;
    let identity = match target_index {
        Some(target_index) => repository
            .domain
            .patchsets_by_id
            .values()
            .find(|identity| identity.index == target_index)
            .copied()
            .ok_or_else(|| {
                format!(
                    "explicit Patchset {:?} maps to missing v0 target index {target_index}",
                    legacy.canonical_patchset_id
                )
            })?,
        None => repository
            .domain
            .patchset_by_immutable_identity(
                &legacy.base_snapshot_id,
                &legacy.revision_snapshot_id,
                legacy.patchset_number,
            )?
            .ok_or_else(|| {
                format!(
                    "explicit Patchset {:?} at physical index {} is absent from the frozen v0 prefix mapping and has no exact appended v0 identity matching its immutable Snapshot pair and Patchset ordinal",
                    legacy.canonical_patchset_id, legacy.physical_index
                )
            })?,
    };
    let target_base_snapshot_id = repository
        .domain
        .snapshot_id(identity.record.base_snapshot_index)?;
    let target_revision_snapshot_id = repository
        .domain
        .snapshot_id(identity.record.revision_snapshot_index)?;
    if target_base_snapshot_id != legacy.base_snapshot_id
        || target_revision_snapshot_id != legacy.revision_snapshot_id
        || identity.patchset_number() != legacy.patchset_number
    {
        return Err(format!(
            "explicit Patchset {:?} mapped v0 identity disagrees with its immutable Snapshot pair or ordinal",
            legacy.canonical_patchset_id
        ));
    }
    Ok(identity)
}

fn legacy_canonical_patchset_id(
    source_patchset_id: &str,
    namespace_ascii: [u8; 2],
    task_number: u32,
    patchset_number: u32,
) -> Result<String, String> {
    let (prefix_and_task, _) = source_patchset_id
        .rsplit_once('-')
        .ok_or_else(|| "legacy Patchset has no ordinal suffix".to_string())?;
    let (legacy_prefix, _) = prefix_and_task
        .rsplit_once('-')
        .ok_or_else(|| "legacy Patchset has no Task suffix".to_string())?;
    let legacy_prefix = legacy_prefix.strip_prefix("P-").unwrap_or(legacy_prefix);
    let locality = match legacy_prefix.as_bytes().first() {
        Some(b'R') => "R",
        Some(b'L') => "L",
        _ => "",
    };
    let namespace = namespace_ascii
        .into_iter()
        .take_while(|byte| *byte != 0)
        .map(|byte| byte.to_ascii_uppercase())
        .collect::<Vec<_>>();
    let namespace = String::from_utf8(namespace)
        .map_err(|_| "Repository namespace is not ASCII".to_string())?;
    Ok(format!(
        "{locality}{namespace}T-{task_number:04}/C-01/P-{patchset_number:02}"
    ))
}

fn validate_legacy_source_change_id(
    source_change_id: &str,
    source_patchset_id: &str,
) -> Result<(), String> {
    let (prefix_and_task, _) = source_patchset_id
        .rsplit_once('-')
        .ok_or_else(|| format!("legacy Patchset ID is not recognized: {source_patchset_id:?}"))?;
    let (patchset_prefix, task_number) = prefix_and_task
        .rsplit_once('-')
        .ok_or_else(|| format!("legacy Patchset ID is not recognized: {source_patchset_id:?}"))?;
    let change_prefix = if let Some(change_prefix) = patchset_prefix.strip_prefix("P-") {
        change_prefix.to_string()
    } else if let Some(change_prefix) = patchset_prefix.strip_suffix('P') {
        format!("{change_prefix}C")
    } else {
        return Err(format!(
            "legacy Patchset prefix has no exact Change counterpart: {source_patchset_id:?}"
        ));
    };
    let expected_change_id = format!("{change_prefix}-{task_number}");
    if source_change_id != expected_change_id {
        return Err(format!(
            "legacy change_id {source_change_id:?} disagrees with source Patchset {source_patchset_id:?}"
        ));
    }
    Ok(())
}

fn validate_source_patchset_change_id(
    source_change_id: &str,
    source_patchset_id: &str,
) -> Result<(), String> {
    if let Some((source_change_ref, patch_ordinal)) = source_patchset_id.rsplit_once("/P-") {
        if parse_positive_decimal(patch_ordinal).is_none() {
            return Err(format!(
                "source Patchset ID has an invalid Patchset ordinal: {source_patchset_id:?}"
            ));
        }
        let short_change_id = source_change_ref
            .rsplit_once('/')
            .map(|(_, value)| value)
            .unwrap_or(source_change_ref);
        if source_change_id == source_change_ref || source_change_id == short_change_id {
            return Ok(());
        }
        return Err(format!(
            "change_id {source_change_id:?} disagrees with source Patchset {source_patchset_id:?}"
        ));
    }
    if let Some(expected_change_id) =
        LEGACY_SNAPSHOT_PATCHSET_ALIASES
            .iter()
            .find_map(|(patchset_id, change_id)| {
                (*patchset_id == source_patchset_id).then_some(*change_id)
            })
    {
        if source_change_id == expected_change_id {
            return Ok(());
        }
        return Err(format!(
            "snapshot-only change_id {source_change_id:?} disagrees with source Patchset {source_patchset_id:?}"
        ));
    }
    validate_legacy_source_change_id(source_change_id, source_patchset_id)
}

fn legacy_patchset_coordinates(value: &str) -> Option<(u32, u32)> {
    let (change_prefix, patchset_number) = value.rsplit_once('-')?;
    let (_, change_sequence) = change_prefix.rsplit_once('-')?;
    Some((
        parse_positive_decimal(change_sequence)?,
        parse_positive_decimal(patchset_number)?,
    ))
}

fn parse_positive_decimal(value: &str) -> Option<u32> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u32>().ok().filter(|value| *value != 0)
}

fn validate_logical_main(payload: &JsonMap<String, JsonValue>) -> Result<(), String> {
    if let Some(target_line) = optional_exact_text(payload, "target_line")? {
        if target_line != "main" {
            return Err("Job target_line must be absent or exact main".to_string());
        }
    }
    Ok(())
}

fn validate_direct_main_land(payload: &JsonMap<String, JsonValue>) -> Result<(), String> {
    validate_logical_main(payload)?;
    if let Some(mode) = optional_exact_text(payload, "mode")? {
        if mode != "direct" {
            return Err("land.process mode must be absent or exact direct".to_string());
        }
    }
    Ok(())
}

fn validate_result_domain_values(
    job: &SourceJobRow,
    result: &JsonMap<String, JsonValue>,
    payload: &JsonMap<String, JsonValue>,
    repository: &PreparedRepository,
    patchset_index: Option<u32>,
    snapshot_index: Option<u32>,
    source_patchset_id: Option<&str>,
    recovery_enabled: bool,
) -> Result<(), String> {
    if let Some(value) = optional_exact_text(result, "repo_id")? {
        if value != repository.source.repo_id {
            return Err("result_json repo_id disagrees with owning Repository".to_string());
        }
    }
    if let Some(value) = optional_exact_text(result, "repo_name")? {
        if value != repository.source.repo_name {
            return Err("result_json repo_name disagrees with owning Repository".to_string());
        }
    }

    let patchset = patchset_index
        .map(|index| {
            repository
                .domain
                .patchsets_by_id
                .values()
                .find(|identity| identity.index == index)
                .copied()
                .ok_or_else(|| format!("result_json selects unknown Patchset index {index}"))
        })
        .transpose()?;
    if let Some(value) = optional_exact_text(result, "patchset_id")? {
        let patchset = patchset
            .ok_or_else(|| "result_json patchset_id has no fixed Patchset reference".to_string())?;
        let canonical_id = repository
            .domain
            .patchsets_by_id
            .iter()
            .find_map(|(id, identity)| (identity.index == patchset.index).then_some(id.as_str()))
            .ok_or_else(|| "result_json selected Patchset has no public identity".to_string())?;
        if Some(value) != source_patchset_id && value != canonical_id {
            return Err("result_json patchset_id disagrees with fixed Patchset".to_string());
        }
    }
    if let Some(value) = optional_exact_text(result, "change_id")? {
        validate_source_patchset_change_id(
            value,
            source_patchset_id.ok_or_else(|| {
                "result_json change_id has no source Patchset reference".to_string()
            })?,
        )?;
    }
    if let Some(value) = optional_positive_u32(result, "patchset_number")? {
        if patchset.is_none_or(|patchset| value != patchset.patchset_number()) {
            return Err("result_json patchset_number disagrees with fixed Patchset".to_string());
        }
    }
    if let Some(value) = optional_positive_u32(result, "change_seq")? {
        if patchset.is_none_or(|patchset| value != patchset.change_sequence()) {
            return Err("result_json change_seq disagrees with fixed Change".to_string());
        }
    }
    if let Some(value) = optional_exact_text(result, "revision_snapshot_id")? {
        let patchset = patchset.ok_or_else(|| {
            "result_json revision_snapshot_id has no fixed Patchset reference".to_string()
        })?;
        if value
            != repository
                .domain
                .snapshot_id(patchset.record.revision_snapshot_index)?
        {
            return Err(
                "result_json revision_snapshot_id disagrees with fixed Patchset".to_string(),
            );
        }
    }
    if let Some(value) = optional_exact_text(result, "snapshot_id")? {
        let expected_index = patchset
            .map(|patchset| patchset.record.revision_snapshot_index)
            .or(snapshot_index)
            .ok_or_else(|| "result_json snapshot_id has no fixed Snapshot reference".to_string())?;
        if value != repository.domain.snapshot_id(expected_index)? {
            let admitted_landed_snapshot = recovery_enabled
                && job.job_type == "main-seed.refresh"
                && patchset.is_some_and(|patchset| {
                    repository
                        .domain
                        .validate_successful_main_land_snapshot(patchset.index, value)
                        .is_ok()
                });
            if !admitted_landed_snapshot {
                return Err("result_json snapshot_id disagrees with fixed Snapshot".to_string());
            }
        }
    }
    if let Some(value) = optional_exact_text(result, "previous_snapshot_id")? {
        if patchset.is_none() {
            return Err(
                "result_json previous_snapshot_id is not valid for this Job kind".to_string(),
            );
        }
        let expected_index = snapshot_index.ok_or_else(|| {
            "result_json previous_snapshot_id has no fixed prior Snapshot".to_string()
        })?;
        if value != repository.domain.snapshot_id(expected_index)? {
            return Err(
                "result_json previous_snapshot_id disagrees with fixed prior Snapshot".to_string(),
            );
        }
    }
    if let Some(value) = optional_exact_text(result, "target_line")? {
        if value != "main" {
            return Err("result_json target_line must be exact main".to_string());
        }
    }
    if let (Some(result_change_id), Some(payload_change_id)) = (
        optional_exact_text(result, "change_id")?,
        optional_exact_text(payload, "change_id")?,
    ) {
        if result_change_id != payload_change_id {
            return Err("result_json change_id disagrees with payload_json".to_string());
        }
    }
    Ok(())
}

fn convert_state(
    job: &SourceJobRow,
    result: &JsonMap<String, JsonValue>,
    repository_index: u32,
    source_by_id: &BTreeMap<i64, &SourceJobRow>,
    job_map: &BTreeMap<i64, (u32, u32)>,
    recovery_enabled: bool,
    recovery_audit: &mut RecoveryJobAudit,
) -> Result<(u8, u8, u16), String> {
    let diagnostic_attached = recovery_enabled
        && job.state == "succeeded"
        && job.last_error.as_deref() == Some(HISTORICAL_ATTACHED_DIAGNOSTIC)
        && result.get("status").and_then(JsonValue::as_str) == Some("attached")
        && result
            .get("scheduler")
            .and_then(JsonValue::as_object)
            .is_some();
    let error_kind = match job.last_error.as_deref() {
        None => WORKER_JOB_ERROR_NONE,
        Some(error) if error.is_empty() || error.trim() != error => {
            return Err(format!(
                "Job {} last_error must be SQL NULL or exact non-empty text",
                job.job_id
            ))
        }
        Some(LEASE_EXPIRED_REQUEUED) if job.state == "queued" => WORKER_JOB_ERROR_LEASE_EXPIRED,
        Some(LEASE_EXPIRED_FAILED) if job.state == "failed" => WORKER_JOB_ERROR_LEASE_EXPIRED,
        Some(LEASE_EXPIRED_REQUEUED | LEASE_EXPIRED_FAILED) => {
            return Err(format!(
                "Job {} lease-expiry message disagrees with state {}",
                job.job_id, job.state
            ))
        }
        Some(_) if diagnostic_attached => WORKER_JOB_ERROR_NONE,
        Some(_) if job.state == "queued" => WORKER_JOB_ERROR_RETRYABLE_EXECUTION,
        Some(_) if job.state == "failed" => WORKER_JOB_ERROR_TERMINAL_EXECUTION,
        Some(_) => {
            return Err(format!(
                "Job {} state {} cannot retain last_error",
                job.job_id, job.state
            ))
        }
    };
    match job.state.as_str() {
        "queued" => Ok((WORKER_JOB_STATE_QUEUED, WORKER_JOB_OUTCOME_NONE, error_kind)),
        "failed" => {
            if error_kind == WORKER_JOB_ERROR_NONE {
                return Err(format!("failed Job {} lacks last_error", job.job_id));
            }
            Ok((
                WORKER_JOB_STATE_FAILED,
                WORKER_JOB_OUTCOME_FAILED,
                error_kind,
            ))
        }
        "succeeded" => {
            if error_kind != WORKER_JOB_ERROR_NONE {
                return Err(format!("succeeded Job {} retains last_error", job.job_id));
            }
            let outcome = match result.get("status").and_then(JsonValue::as_str) {
                Some("skipped") => {
                    if result.get("reason").and_then(JsonValue::as_str)
                        == Some("equivalent_later_patchset_ci_succeeded")
                    {
                        validate_related_result_job(
                            job,
                            result.get("successor_job_id"),
                            repository_index,
                            source_by_id,
                            job_map,
                            RelatedJobRule::LaterSucceeded,
                        )?;
                        WORKER_JOB_OUTCOME_SUPERSEDED
                    } else {
                        WORKER_JOB_OUTCOME_SKIPPED
                    }
                }
                Some("attached") => {
                    if recovery_enabled
                        && result
                            .get("executor")
                            .and_then(JsonValue::as_object)
                            .is_some()
                    {
                        let executor = result
                            .get("executor")
                            .and_then(JsonValue::as_object)
                            .expect("checked executor object");
                        validate_historical_attached_projection(job, executor, true)?;
                        validate_historical_attached_result_job(
                            job,
                            executor.get("active_job_id"),
                            source_by_id,
                            HistoricalAttachedOrdering::Earlier,
                        )?;
                        recovery_audit.normalize(ATTACHED_TERMINAL_EXACT, job.job_id);
                    } else if diagnostic_attached {
                        let scheduler = result
                            .get("scheduler")
                            .and_then(JsonValue::as_object)
                            .expect("checked diagnostic scheduler object");
                        validate_historical_attached_projection(job, scheduler, false)?;
                        validate_historical_attached_result_job(
                            job,
                            scheduler.get("active_job_id"),
                            source_by_id,
                            HistoricalAttachedOrdering::Later,
                        )?;
                        recovery_audit.normalize(DIAGNOSTIC_ATTACHED, job.job_id);
                    } else {
                        let active = result
                            .get("scheduler")
                            .and_then(JsonValue::as_object)
                            .and_then(|scheduler| scheduler.get("active_job_id"));
                        validate_related_result_job(
                            job,
                            active,
                            repository_index,
                            source_by_id,
                            job_map,
                            RelatedJobRule::EarlierActive,
                        )?;
                    }
                    WORKER_JOB_OUTCOME_ATTACHED
                }
                Some(status) if status.trim() != status || status.is_empty() => {
                    return Err(format!(
                        "Job {} result status is not exact text",
                        job.job_id
                    ))
                }
                _ => WORKER_JOB_OUTCOME_COMPLETED,
            };
            Ok((WORKER_JOB_STATE_SUCCEEDED, outcome, WORKER_JOB_ERROR_NONE))
        }
        state => Err(format!("Job {} has unassigned state {state:?}", job.job_id)),
    }
}

#[derive(Clone, Copy)]
enum HistoricalAttachedOrdering {
    Earlier,
    Later,
}

fn validate_historical_attached_projection(
    job: &SourceJobRow,
    projection: &JsonMap<String, JsonValue>,
    executor: bool,
) -> Result<(), String> {
    let allowed = if executor {
        BTreeSet::from(["active_job_id", "job_id", "kind", "singleflight_key"])
    } else {
        BTreeSet::from(["active_job_id", "decision", "singleflight_key"])
    };
    reject_extra_keys(projection, &allowed, "historical attached projection")?;
    if executor {
        if projection.get("kind").and_then(JsonValue::as_str) != Some("attached")
            || projection.get("job_id").and_then(exact_i64) != Some(job.job_id)
        {
            return Err(format!(
                "Job {} historical executor projection is not exact attached self authority",
                job.job_id
            ));
        }
    } else if projection.get("decision").and_then(JsonValue::as_str) != Some("attach") {
        return Err(format!(
            "Job {} historical scheduler projection is not exact attach authority",
            job.job_id
        ));
    }
    required_exact_text(projection, "singleflight_key")?;
    Ok(())
}

fn validate_historical_attached_result_job(
    job: &SourceJobRow,
    value: Option<&JsonValue>,
    source_by_id: &BTreeMap<i64, &SourceJobRow>,
    ordering: HistoricalAttachedOrdering,
) -> Result<(), String> {
    let related_id = value
        .and_then(exact_i64)
        .ok_or_else(|| format!("Job {} result lacks a valid related Job ID", job.job_id))?;
    if related_id == job.job_id {
        return Err(format!(
            "Job {} historical attached outcome has no distinct related Job",
            job.job_id
        ));
    }
    let related = source_by_id.get(&related_id).copied().ok_or_else(|| {
        format!(
            "Job {} historical attached outcome references absent Job {related_id}",
            job.job_id
        )
    })?;
    if related.repo_id != job.repo_id || related.repo_name != job.repo_name {
        return Err(format!(
            "Job {} historical attached outcome references a cross-Repository Job",
            job.job_id
        ));
    }
    let ordered = match ordering {
        HistoricalAttachedOrdering::Earlier => related_id < job.job_id,
        HistoricalAttachedOrdering::Later => related_id > job.job_id,
    };
    if !ordered || related.state != "succeeded" || related.job_type != job.job_type {
        return Err(format!(
            "Job {} historical attached related Job has the wrong ordering, state, or kind",
            job.job_id
        ));
    }
    let current_payload = parse_object_without_duplicates(
        &job.payload_json,
        &format!("Job {} payload_json", job.job_id),
    )?;
    let related_payload = parse_object_without_duplicates(
        &related.payload_json,
        &format!("Job {related_id} payload_json"),
    )?;
    for field in [
        "repo_id",
        "repo_name",
        "patchset_id",
        "revision_snapshot_id",
        "snapshot_id",
        "suite_ids",
    ] {
        if current_payload.get(field) != related_payload.get(field) {
            return Err(format!(
                "Job {} historical attached related Job disagrees on {field}",
                job.job_id
            ));
        }
    }
    Ok(())
}

enum RelatedJobRule {
    LaterSucceeded,
    EarlierActive,
}

fn validate_related_result_job(
    job: &SourceJobRow,
    value: Option<&JsonValue>,
    repository_index: u32,
    source_by_id: &BTreeMap<i64, &SourceJobRow>,
    job_map: &BTreeMap<i64, (u32, u32)>,
    rule: RelatedJobRule,
) -> Result<(), String> {
    let related_id = value
        .and_then(exact_i64)
        .ok_or_else(|| format!("Job {} result lacks a valid related Job ID", job.job_id))?;
    if related_id == job.job_id
        || job_map.get(&related_id).map(|(index, _)| *index) != Some(repository_index)
    {
        return Err(format!(
            "Job {} result related Job is self, absent, or cross-Repository",
            job.job_id
        ));
    }
    let related = source_by_id[&related_id];
    let valid = match rule {
        RelatedJobRule::LaterSucceeded => {
            let current_payload = parse_object_without_duplicates(
                &job.payload_json,
                &format!("Job {} payload_json", job.job_id),
            )?;
            let related_payload = parse_object_without_duplicates(
                &related.payload_json,
                &format!("Job {related_id} payload_json"),
            )?;
            job.job_type == "patchset.ci"
                && related_id > job.job_id
                && related.state == "succeeded"
                && related.job_type == job.job_type
                && current_payload.get("patchset_id") == related_payload.get("patchset_id")
        }
        RelatedJobRule::EarlierActive => {
            related_id < job.job_id && matches!(related.state.as_str(), "queued" | "running")
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "Job {} result related Job does not satisfy the required ordering/state",
            job.job_id
        ))
    }
}

fn write_repository_registry(
    global_root: &Path,
    repositories: &[PreparedRepository],
) -> Result<(), String> {
    let mut payload_bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
    let mut repository_bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
    let mut namespace_rows = Vec::new();
    for repository in repositories {
        let payload = ServerOperationalBinaryV0Codec::encode_repository_payload(
            &OperationalRepositoryPayload {
                repo_name: repository.source.repo_name.clone(),
            },
        )
        .map_err(|error| format!("failed to encode Repository payload: {error}"))?;
        let payload_offset = payload_bytes.len() as u64;
        payload_bytes.extend_from_slice(&payload);
        let record = OperationalRepositoryRecord {
            repository_meta: 0,
            lifecycle_kind: repository.lifecycle_kind,
            namespace_ascii: repository.namespace_ascii,
            policy_flags: repository.policy_flags,
            payload_len: u32::try_from(payload.len())
                .map_err(|_| "Repository payload exceeds u32".to_string())?,
            payload_offset,
            created_at_s: repository.created_at_s,
            updated_at_s: repository.updated_at_s,
        };
        repository_bytes.extend_from_slice(
            &ServerOperationalBinaryV0Codec::encode_repository(record)
                .map_err(|error| format!("failed to encode Repository record: {error}"))?,
        );
        if repository.namespace_ascii != [0, 0] {
            namespace_rows.push(OperationalNamespaceIndexRecord {
                namespace_ascii: repository.namespace_ascii,
                reserved0: 0,
                repository_index_plus1: repository
                    .repository_index
                    .checked_add(1)
                    .ok_or_else(|| "Repository plus-one index overflow".to_string())?,
            });
        }
    }
    namespace_rows.sort_by_key(|row| (row.namespace_ascii, row.repository_index_plus1));
    let mut namespace_bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
    for row in namespace_rows {
        namespace_bytes.extend_from_slice(
            &ServerOperationalBinaryV0Codec::encode_namespace_index(row)
                .map_err(|error| format!("failed to encode namespace index: {error}"))?,
        );
    }
    write_new_sync(&global_root.join("repository.bin"), &repository_bytes)?;
    write_new_sync(&global_root.join("repository_payload.bin"), &payload_bytes)?;
    write_new_sync(
        &global_root.join("repository_namespace.idx"),
        &namespace_bytes,
    )?;
    sync_directory(global_root)
}

fn write_repository_authority(
    repositories_root: &Path,
    repository: &PreparedRepository,
) -> Result<(), String> {
    let target = repositories_root.join(repository.repository_index.to_string());
    fs::create_dir(&target)
        .map_err(|error| format!("failed to create {}: {error}", target.display()))?;
    copy_directory_contents(&repository.domain.source_root, &target)?;
    for forbidden in [
        "worker_job.bin",
        "worker_ready.idx",
        "worker_state.idx",
        "worker_job_payload.bin",
        "worker_job_input_payload.bin",
        "worker_job_request_payload.bin",
        "worker_job_result_payload.bin",
        "worker_job_error_payload.bin",
        "worker_job_lease_owner_payload.bin",
        "job.bin",
    ] {
        if target.join(forbidden).exists() {
            return Err(format!(
                "source authority contains forbidden operational path {forbidden}"
            ));
        }
    }
    rewrite_patchsets(&target, repository)?;
    write_worker_jobs(&target, &repository.jobs)?;
    sync_directory(&target)?;
    sync_directory(repositories_root)
}

fn rewrite_patchsets(target: &Path, repository: &PreparedRepository) -> Result<(), String> {
    let path = target.join("patchset.bin");
    let raw = read_regular_file(&path)?;
    if raw.len() < 4
        || raw[..4] != 1_u32.to_le_bytes()
        || (raw.len() - 4) % PATCHSET_RECORD_SIZE as usize != 0
    {
        return Err(format!(
            "source patchset.bin is not aligned: {}",
            path.display()
        ));
    }
    if (raw.len() - 4) / PATCHSET_RECORD_SIZE as usize != repository.domain.patchsets_by_index.len()
    {
        return Err("source Patchset count changed after domain validation".to_string());
    }
    let mut latest_ci_job = BTreeMap::<u32, (i64, u32)>::new();
    for job in &repository.jobs {
        if job.record.job_kind == WORKER_JOB_KIND_PATCHSET_CI {
            let patchset_index = job
                .patchset_index
                .ok_or_else(|| "Patchset CI Job lacks Patchset index".to_string())?;
            let candidate = (job.source_job_id, job.worker_job_index);
            if latest_ci_job
                .get(&patchset_index)
                .is_none_or(|current| candidate.0 > current.0)
            {
                latest_ci_job.insert(patchset_index, candidate);
            }
        }
    }
    let mut corrected = 1_u32.to_le_bytes().to_vec();
    for (index, legacy) in repository
        .domain
        .patchsets_by_index
        .iter()
        .copied()
        .enumerate()
    {
        let ci_completed_at_s = legacy.ci_completed_at_s;
        let locator = latest_ci_job
            .get(&(index as u32))
            .map(|(_, worker_job_index)| {
                worker_job_index
                    .checked_add(1)
                    .ok_or_else(|| "Worker Job plus-one locator overflow".to_string())
            })
            .transpose()?
            .unwrap_or(0);
        let frozen = V0FrozenPatchsetRecord {
            patchset_meta: legacy.patchset_meta,
            patch_ordinal: legacy.patch_ordinal,
            change_ordinal: legacy.change_ordinal,
            reserved0: legacy.reserved0,
            change_index: legacy.change_index,
            previous_task_patchset_index_plus1: legacy.previous_task_patchset_index_plus1,
            previous_change_patchset_index_plus1: legacy.previous_change_patchset_index_plus1,
            base_snapshot_index: legacy.base_snapshot_index,
            revision_snapshot_index: legacy.revision_snapshot_index,
            created_at_s: legacy.created_at_s,
            ci_completed_at_s,
            ci_run_seq: legacy.ci_run_seq,
            ci_selected_suite_count: legacy.ci_selected_suite_count,
            ci_suite_result_count: legacy.ci_suite_result_count,
            ci_blocking_failure_count: legacy.ci_blocking_failure_count,
            ci_status_bits: legacy.ci_status_bits,
            summary_offset: legacy.summary_offset,
            summary_len: legacy.summary_len,
            ci_worker_job_index_plus1: locator,
        };
        corrected.extend_from_slice(
            &WorkflowBinaryV0Codec::encode_frozen_patchset(frozen)
                .map_err(|error| format!("failed to encode corrected Patchset {index}: {error}"))?,
        );
    }
    atomic_replace(&path, &corrected)
}

fn write_worker_jobs(target: &Path, jobs: &[PreparedJob]) -> Result<(), String> {
    let mut fixed = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
    for job in jobs {
        fixed.extend_from_slice(
            &ServerOperationalBinaryV0Codec::encode_worker_job(job.record)
                .map_err(|error| format!("failed to encode Worker Job: {error}"))?,
        );
    }
    let mut ready = jobs
        .iter()
        .filter(|job| job.record.state_kind == WORKER_JOB_STATE_QUEUED)
        .map(|job| ServerWorkerReadyIndexRecord {
            available_at_s: job.record.available_at_s,
            worker_job_index_plus1: job.worker_job_index + 1,
        })
        .collect::<Vec<_>>();
    ready.sort_by_key(|row| (row.available_at_s, row.worker_job_index_plus1));
    let mut ready_bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
    for row in ready {
        ready_bytes.extend_from_slice(
            &ServerOperationalBinaryV0Codec::encode_worker_ready_index(row)
                .map_err(|error| format!("failed to encode Worker ready index: {error}"))?,
        );
    }
    let mut state = jobs
        .iter()
        .map(|job| ServerWorkerStateIndexRecord {
            state_kind: job.record.state_kind,
            reserved0: 0,
            reserved1: 0,
            worker_job_index_plus1: job.worker_job_index + 1,
        })
        .collect::<Vec<_>>();
    state.sort_by_key(|row| (row.state_kind, row.worker_job_index_plus1));
    let mut state_bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
    for row in state {
        state_bytes.extend_from_slice(
            &ServerOperationalBinaryV0Codec::encode_worker_state_index(row)
                .map_err(|error| format!("failed to encode Worker state index: {error}"))?,
        );
    }
    write_new_sync(&target.join("worker_job.bin"), &fixed)?;
    write_new_sync(&target.join("worker_ready.idx"), &ready_bytes)?;
    write_new_sync(&target.join("worker_state.idx"), &state_bytes)
}

fn write_generation_manifest(
    generation: &Path,
    repositories: &[PreparedRepository],
) -> Result<(), String> {
    let value = json!({
        "schema": GENERATION_SCHEMA,
        "layout_id": 1,
        "status": "validated_inactive",
        "global_registry": "global",
        "repository_authorities": "repositories",
        "repository_count": repositories.len(),
    });
    let mut bytes = serde_json::to_vec_pretty(&value)
        .map_err(|error| format!("failed to encode generation manifest: {error}"))?;
    bytes.push(b'\n');
    write_new_sync(&generation.join("generation.json"), &bytes)
}

fn validate_target(
    global_root: &Path,
    repositories_root: &Path,
    repositories: &[PreparedRepository],
) -> Result<(), String> {
    let registry = ServerOperationalRepositoryRegistry::new(global_root, repositories_root)
        .map_err(|error| format!("target Repository registry cannot open: {error}"))?;
    let entries = registry
        .validate()
        .map_err(|error| format!("target Repository registry validation failed: {error}"))?;
    if entries.len() != repositories.len() {
        return Err("target Repository registry count changed".to_string());
    }
    for repository in repositories {
        let domain = Arc::new(TargetDomain {
            patchset_count: u32::try_from(repository.domain.patchsets_by_index.len())
                .map_err(|_| "Patchset count exceeds u32".to_string())?,
            snapshot_indexes: repository
                .domain
                .snapshot_ids_by_index
                .keys()
                .copied()
                .collect(),
        });
        let store = ServerOperationalWorkerJobStore::new(
            repository.repository_index,
            repositories_root.join(repository.repository_index.to_string()),
            domain,
        )
        .map_err(|error| format!("target Worker Job store cannot open: {error}"))?;
        let jobs = store
            .validate()
            .map_err(|error| format!("target Worker Job validation failed: {error}"))?;
        if jobs.len() != repository.jobs.len() {
            return Err("target Worker Job count changed".to_string());
        }
        validate_corrected_patchsets(
            &repositories_root.join(repository.repository_index.to_string()),
            repository,
        )?;
    }
    Ok(())
}

struct TargetDomain {
    patchset_count: u32,
    snapshot_indexes: BTreeSet<u32>,
}

impl WorkerJobDomainAuthority for TargetDomain {
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

fn validate_corrected_patchsets(
    root: &Path,
    repository: &PreparedRepository,
) -> Result<(), String> {
    let raw = read_regular_file(&root.join("patchset.bin"))?;
    for (index, record) in raw[4..]
        .chunks_exact(PATCHSET_RECORD_SIZE as usize)
        .enumerate()
    {
        let record = WorkflowBinaryV0Codec::decode_frozen_patchset(record)
            .map_err(|error| format!("corrected Patchset {index} is invalid: {error}"))?;
        if let Some(job_index) = record.ci_worker_job_index_plus1.checked_sub(1) {
            let job = repository
                .jobs
                .get(job_index as usize)
                .ok_or_else(|| format!("Patchset {index} CI Job locator is out of range"))?;
            if job.record.job_kind != WORKER_JOB_KIND_PATCHSET_CI
                || job.patchset_index != Some(index as u32)
            {
                return Err(format!(
                    "Patchset {index} CI Job locator does not exact-match same-Repository Job"
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn create_empty_staged_generation(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("staged generation path is empty".to_string());
    }
    if fs::symlink_metadata(path).is_ok() {
        return Err(format!(
            "staged generation path already exists: {}",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| "staged generation path has no parent".to_string())?;
    let parent = canonical_real_directory(parent)?;
    let name = path
        .file_name()
        .ok_or_else(|| "staged generation path has no name".to_string())?;
    let target = parent.join(name);
    fs::create_dir(&target).map_err(|error| {
        format!(
            "failed to create staged generation {}: {error}",
            target.display()
        )
    })?;
    sync_directory(&parent)?;
    canonical_real_directory(&target)
}

pub(crate) fn absolute_new_file_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() || fs::symlink_metadata(path).is_ok() {
        return Err(format!(
            "report path must identify a new file: {}",
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

pub(crate) fn copy_directory_contents(source: &Path, target: &Path) -> Result<(), String> {
    let mut entries = fs::read_dir(source)
        .map_err(|error| {
            format!(
                "failed to read source directory {}: {error}",
                source.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inventory {}: {error}", source.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().into_string().map_err(|_| {
            format!(
                "source authority has non-UTF-8 path under {}",
                source.display()
            )
        })?;
        let source_path = entry.path();
        let target_path = target.join(&name);
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|error| format!("failed to inspect {}: {error}", source_path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "source authority contains symlink {}",
                source_path.display()
            ));
        }
        if metadata.is_dir() {
            fs::create_dir(&target_path)
                .map_err(|error| format!("failed to create {}: {error}", target_path.display()))?;
            copy_directory_contents(&source_path, &target_path)?;
            sync_directory(&target_path)?;
        } else if metadata.is_file() {
            #[cfg(unix)]
            if metadata.nlink() != 1 {
                return Err(format!(
                    "source authority file has multiple hard links: {}",
                    source_path.display()
                ));
            }
            copy_regular_file(&source_path, &target_path, &metadata)?;
        } else {
            return Err(format!(
                "source authority contains special path {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

fn copy_regular_file(
    source: &Path,
    target: &Path,
    source_metadata: &fs::Metadata,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let source_c = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| format!("source authority path contains NUL: {}", source.display()))?;
        let target_c = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| format!("target authority path contains NUL: {}", target.display()))?;
        let result = unsafe { libc::clonefile(source_c.as_ptr(), target_c.as_ptr(), 0) };
        if result == 0 {
            let target_metadata = fs::symlink_metadata(target).map_err(|error| {
                format!("failed to inspect clone {}: {error}", target.display())
            })?;
            if !target_metadata.is_file()
                || target_metadata.file_type().is_symlink()
                || target_metadata.nlink() != 1
                || target_metadata.len() != source_metadata.len()
            {
                return Err(format!(
                    "APFS clone did not preserve exact regular-file authority: {}",
                    target.display()
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
                target.display()
            ));
        }
    }

    let bytes = read_regular_file(source)?;
    write_new_sync(target, &bytes)
}

pub(crate) fn inventory_authority_files(generation: &Path) -> Result<Vec<FileReport>, String> {
    fn visit(root: &Path, current: &Path, files: &mut Vec<FileReport>) -> Result<(), String> {
        let mut entries = fs::read_dir(current)
            .map_err(|error| format!("failed to inventory {}: {error}", current.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("failed to inventory {}: {error}", current.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                visit(root, &path, files)?;
            } else if metadata.is_file() && !metadata.file_type().is_symlink() {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| "authority inventory escaped generation root".to_string())?
                    .to_str()
                    .ok_or_else(|| "authority inventory path is not UTF-8".to_string())?
                    .to_string();
                if is_disposable_runtime_file(Path::new(&relative)) {
                    continue;
                }
                let bytes = read_regular_file(&path)?;
                files.push(FileReport {
                    relative_path: relative,
                    byte_size: bytes.len() as u64,
                    sha256: sha256(&bytes),
                });
            } else {
                return Err(format!(
                    "staged generation contains non-regular path {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    visit(generation, generation, &mut files)?;
    files.sort_by(|left, right| {
        left.relative_path
            .as_bytes()
            .cmp(right.relative_path.as_bytes())
    });
    Ok(files)
}

fn parse_namespace(value: &str) -> Result<[u8; 2], String> {
    if value.len() > 2 || !value.is_ascii() {
        return Err("Repository namespace must contain zero, one, or two ASCII bytes".to_string());
    }
    let mut namespace = [0_u8; 2];
    for (index, byte) in value.bytes().enumerate() {
        if !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')) {
            return Err(format!(
                "Repository namespace contains forbidden byte {byte}"
            ));
        }
        namespace[index] = byte;
    }
    Ok(namespace)
}

fn parse_policy_flags(raw: &str) -> Result<u8, String> {
    let object = parse_object_without_duplicates(raw, "Repository policy_json")?;
    let allowed = BTreeSet::from(["policy_id", "version", "defaults", "class_overrides"]);
    reject_extra_keys(&object, &allowed, "policy_json")?;
    if object.get("policy_id").and_then(JsonValue::as_str) != Some("prototype") {
        return Err("policy_json policy_id must be exact prototype".to_string());
    }
    if let Some(version) = object.get("version") {
        if version.as_u64() != Some(1) {
            return Err("policy_json version must be exact integer 1".to_string());
        }
    }
    let defaults = object
        .get("defaults")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "policy_json requires defaults object".to_string())?;
    let names = [
        "require_attestation",
        "require_tests",
        "require_lint",
        "require_security_scan",
        "require_license_scan",
        "require_ai_provenance",
        "require_code_review_summary",
    ];
    reject_extra_keys(
        defaults,
        &names.into_iter().collect(),
        "policy_json.defaults",
    )?;
    let mut flags = 0_u8;
    for (index, name) in names.iter().enumerate() {
        let default = index < 2;
        let enabled = match defaults.get(*name) {
            None => default,
            Some(JsonValue::Bool(value)) => *value,
            Some(_) => {
                return Err(format!(
                    "policy_json.defaults.{name} must be a JSON boolean"
                ))
            }
        };
        if enabled {
            flags |= 1 << index;
        }
    }
    let docs_override = match object.get("class_overrides") {
        None => true,
        Some(JsonValue::Array(values)) if values.is_empty() => false,
        Some(JsonValue::Array(values)) if values.len() == 1 => {
            validate_docs_override(&values[0])?;
            true
        }
        Some(_) => return Err(
            "policy_json class_overrides must be absent, empty, or the exact docs-only override"
                .to_string(),
        ),
    };
    if docs_override {
        flags |= 1 << 7;
    }
    Ok(flags)
}

fn validate_docs_override(value: &JsonValue) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| "docs-only override must be an object".to_string())?;
    reject_extra_keys(
        object,
        &BTreeSet::from(["when", "set"]),
        "class_overrides[0]",
    )?;
    let when = object
        .get("when")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "docs-only override requires when object".to_string())?;
    if when.len() != 1 || when.get("content_class").and_then(JsonValue::as_str) != Some("docs_only")
    {
        return Err("docs-only override when object is not exact".to_string());
    }
    let set = object
        .get("set")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "docs-only override requires set object".to_string())?;
    let required = BTreeSet::from([
        "require_tests",
        "require_lint",
        "require_security_scan",
        "require_license_scan",
    ]);
    reject_extra_keys(set, &required, "class_overrides[0].set")?;
    if set.len() != required.len()
        || required
            .iter()
            .any(|name| set.get(*name) != Some(&JsonValue::Bool(false)))
    {
        return Err("docs-only override set object is not exact".to_string());
    }
    Ok(())
}

fn reject_extra_keys(
    object: &JsonMap<String, JsonValue>,
    allowed: &BTreeSet<&str>,
    label: &str,
) -> Result<(), String> {
    let extra = object
        .keys()
        .filter(|key| !allowed.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if extra.is_empty() {
        Ok(())
    } else {
        Err(format!("{label} contains unsupported keys {extra:?}"))
    }
}

fn parse_lifecycle(value: &str) -> Result<u8, String> {
    match value {
        "active" => Ok(REPOSITORY_LIFECYCLE_ACTIVE),
        "retiring" => Ok(REPOSITORY_LIFECYCLE_RETIRING),
        "purged" => Ok(REPOSITORY_LIFECYCLE_PURGED),
        _ => Err(format!(
            "Repository lifecycle_state is unassigned: {value:?}"
        )),
    }
}

fn job_kind(value: &str) -> Result<u8, String> {
    match value {
        "content.gc" => Ok(WORKER_JOB_KIND_CONTENT_GC),
        "content.optimize" => Ok(WORKER_JOB_KIND_CONTENT_OPTIMIZE),
        "content.pack" => Ok(WORKER_JOB_KIND_CONTENT_PACK),
        "land.process" => Ok(WORKER_JOB_KIND_LAND_PROCESS),
        "main-seed.refresh" => Ok(WORKER_JOB_KIND_MAIN_SEED_REFRESH),
        "patchset.ci" => Ok(WORKER_JOB_KIND_PATCHSET_CI),
        "patchset.ci.aggregate" => Ok(WORKER_JOB_KIND_PATCHSET_CI_AGGREGATE),
        "policy.evaluate" => Ok(WORKER_JOB_KIND_POLICY_EVALUATE),
        "reconcile.repo" => Ok(WORKER_JOB_KIND_RECONCILE_REPO),
        "repo.ci" => Ok(WORKER_JOB_KIND_REPO_CI),
        _ => Err(format!(
            "Job type {value:?} is unassigned; agent.turn.submit and opaque Jobs are rejected"
        )),
    }
}

fn timestamp_s(value: chrono::DateTime<chrono::Utc>, label: &str) -> Result<u64, String> {
    let value =
        u64::try_from(value.timestamp()).map_err(|_| format!("{label} precedes the Unix epoch"))?;
    if value == 0 {
        Err(format!("{label} must be non-zero"))
    } else {
        Ok(value)
    }
}

fn plus_one(value: Option<u32>, label: &str) -> Result<u32, String> {
    value
        .map(|value| {
            value
                .checked_add(1)
                .ok_or_else(|| format!("{label} plus-one index overflow"))
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn required_exact_text<'a>(
    object: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<&'a str, String> {
    optional_exact_text(object, field)?
        .ok_or_else(|| format!("payload requires exact non-empty string field {field:?}"))
}

fn optional_exact_text<'a>(
    object: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<&'a str>, String> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) if !value.is_empty() && value.trim() == value => {
            Ok(Some(value))
        }
        Some(_) => Err(format!(
            "payload field {field:?} must be null or exact non-empty string"
        )),
    }
}

fn optional_positive_u32(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<u32>, String> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| format!("payload field {field:?} must be a positive u32")),
    }
}

fn exact_i64(value: &JsonValue) -> Option<i64> {
    if let Some(value) = value.as_i64() {
        return Some(value);
    }
    let text = value.as_str()?;
    if text.is_empty() || text.trim() != text || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn validate_non_empty_exact(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.trim() != value {
        Err(format!("{label} must be non-empty without normalization"))
    } else {
        Ok(())
    }
}

fn validate_repo_id(value: &str) -> Result<(), String> {
    let hex = value
        .strip_prefix("REPO-")
        .ok_or_else(|| format!("Repository ID {value:?} lacks exact REPO- prefix"))?;
    if !matches!(hex.len(), 24 | 32)
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'A'..=b'F'))
    {
        return Err(format!(
            "Repository ID {value:?} must contain 24 or 32 uppercase hexadecimal digits"
        ));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "source authority_relative_path is unsafe: {value:?}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn authority_inventory_excludes_only_exact_runtime_rebuild_temporaries() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ait-server-authority-inventory-{}-{nonce}",
            std::process::id()
        ));
        let repository = root.join("repositories/2");
        fs::create_dir_all(&repository).unwrap();
        fs::write(root.join("generation.json"), b"authority").unwrap();
        fs::write(repository.join(".worker_ready.idx.rebuild"), b"disposable").unwrap();
        fs::write(
            repository.join(".worker_ready.idx.rebuild.extra"),
            b"authority lookalike",
        )
        .unwrap();

        let paths = inventory_authority_files(&root)
            .unwrap()
            .into_iter()
            .map(|file| file.relative_path)
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            [
                "generation.json".to_string(),
                "repositories/2/.worker_ready.idx.rebuild.extra".to_string(),
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn job(job_id: i64, state: &str, last_error: Option<&str>) -> SourceJobRow {
        let time = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        SourceJobRow {
            job_id,
            repo_name: "ait-core".to_string(),
            repo_id: "REPO-0123456789ABCDEF01234567".to_string(),
            job_type: "content.gc".to_string(),
            state: state.to_string(),
            payload_json: r#"{"repo_name":"ait-core"}"#.to_string(),
            result_json: "{}".to_string(),
            attempt_count: 0,
            max_attempts: 3,
            available_at: time,
            locked_at: None,
            locked_by: None,
            last_error: last_error.map(str::to_string),
            created_at: time,
            updated_at: time,
        }
    }

    fn repository(repo_id: &str, repo_name: &str) -> SourceRepositoryRow {
        let time = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        SourceRepositoryRow {
            repo_id: repo_id.to_string(),
            repo_name: repo_name.to_string(),
            default_line: "main".to_string(),
            id_namespace_prefix: String::new(),
            policy_json: r#"{"policy_id":"prototype","defaults":{}}"#.to_string(),
            created_at: time,
            updated_at: time,
            lifecycle_state: "active".to_string(),
        }
    }

    fn order_entry(index: u32, repository: &SourceRepositoryRow) -> RepositoryOrderEntry {
        RepositoryOrderEntry {
            repository_index: index,
            repo_id: repository.repo_id.clone(),
            repo_name: repository.repo_name.clone(),
        }
    }

    #[test]
    fn repository_index_order_is_unsigned_utf8_after_fixed_slots() {
        let mut values = [
            "REPO-FFFFFFFFFFFFFFFFFFFFFFFF",
            "REPO-000000000000000000000000",
        ];
        values.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        assert_eq!(values[0], "REPO-000000000000000000000000");
    }

    #[test]
    fn explicit_repository_order_is_dense_complete_and_keeps_fixed_slots() {
        let repositories = vec![
            repository("REPO-000000000000000000000000", "ait-core"),
            repository("REPO-111111111111111111111111", "ait-server"),
            repository("REPO-222222222222222222222222", "ait-python"),
            repository("REPO-333333333333333333333333", "ait-node"),
            repository("REPO-FFFFFFFFFFFFFFFFFFFFFFFF", "other"),
            repository("REPO-EEEEEEEEEEEEEEEEEEEEEEEE", "ait-runner"),
        ];
        let snapshot = SourceSnapshot {
            database_name: SOURCE_DATABASE.to_string(),
            inventory_before: crate::types::SourceInventory {
                columns: Vec::new(),
                constraints: Vec::new(),
                repository_count: repositories.len() as u64,
                job_count: 0,
            },
            inventory_after: crate::types::SourceInventory {
                columns: Vec::new(),
                constraints: Vec::new(),
                repository_count: repositories.len() as u64,
                job_count: 0,
            },
            repositories: repositories.clone(),
            jobs: Vec::new(),
        };
        let by_id = repositories
            .iter()
            .cloned()
            .map(|repository| (repository.repo_id.clone(), repository))
            .collect::<BTreeMap<_, _>>();
        let reserved = FIXED_REPOSITORY_NAMES
            .iter()
            .map(|name| {
                (
                    *name,
                    repositories
                        .iter()
                        .filter(|repository| repository.repo_name == *name)
                        .cloned()
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let order = RepositoryOrderDocument {
            schema: REPOSITORY_ORDER_SCHEMA.to_string(),
            repositories: [0_usize, 1, 2, 3, 5, 4]
                .into_iter()
                .enumerate()
                .map(|(index, source_index)| order_entry(index as u32, &repositories[source_index]))
                .collect(),
        };

        let ordered =
            ordered_source_repositories(&snapshot, &by_id, &reserved, Some(&order)).unwrap();
        assert_eq!(ordered[4].repo_name, "ait-runner");
        assert_eq!(ordered[5].repo_name, "other");

        let mut sparse = order.clone();
        sparse.repositories[4].repository_index = 6;
        assert!(
            ordered_source_repositories(&snapshot, &by_id, &reserved, Some(&sparse))
                .unwrap_err()
                .contains("must be dense")
        );

        let mut wrong_fixed = order;
        wrong_fixed.repositories.swap(0, 4);
        wrong_fixed.repositories[0].repository_index = 0;
        wrong_fixed.repositories[4].repository_index = 4;
        assert!(
            ordered_source_repositories(&snapshot, &by_id, &reserved, Some(&wrong_fixed))
                .unwrap_err()
                .contains("must remain reserved")
        );
    }

    #[test]
    fn namespace_encoding_is_exact_and_trailing_zero_based() {
        assert_eq!(parse_namespace("").unwrap(), [0, 0]);
        assert_eq!(parse_namespace("R").unwrap(), [b'R', 0]);
        assert_eq!(parse_namespace("_-").unwrap(), [b'_', b'-']);
        assert!(parse_namespace("ABC").is_err());
        assert!(parse_namespace("é").is_err());
    }

    #[test]
    fn prototype_policy_defaults_and_docs_override_are_closed() {
        assert_eq!(
            parse_policy_flags(r#"{"policy_id":"prototype","defaults":{}}"#).unwrap(),
            0b1000_0011
        );
        assert_eq!(
            parse_policy_flags(
                r#"{"policy_id":"prototype","version":1,"defaults":{},"class_overrides":[]}"#
            )
            .unwrap(),
            0b0000_0011
        );
        assert!(
            parse_policy_flags(r#"{"policy_id":"prototype","defaults":{},"extra":true}"#).is_err()
        );
        assert!(parse_policy_flags(r#"{"policy_id":"prototype","defaults":{"x":true}}"#).is_err());
    }

    #[test]
    fn only_ten_job_types_are_assigned() {
        assert_eq!(
            job_kind("patchset.ci").unwrap(),
            WORKER_JOB_KIND_PATCHSET_CI
        );
        assert!(job_kind("agent.turn.submit").is_err());
        assert!(job_kind("opaque").is_err());
    }

    #[test]
    fn legacy_patchset_coordinates_require_two_positive_decimal_suffixes() {
        assert_eq!(legacy_patchset_coordinates("RCP-0079-1"), Some((79, 1)));
        assert_eq!(legacy_patchset_coordinates("P-CC-0222-17"), Some((222, 17)));
        assert_eq!(legacy_patchset_coordinates("LP-1639-1"), Some((1639, 1)));
        assert_eq!(legacy_patchset_coordinates("RSEP-0135-2"), Some((135, 2)));
        assert_eq!(legacy_patchset_coordinates("RCP-0000-1"), None);
        assert_eq!(legacy_patchset_coordinates("RCP-0079-0"), None);
        assert_eq!(legacy_patchset_coordinates("RCP-X-1"), None);
        assert_eq!(legacy_patchset_coordinates("RCT-0001/C-01/P-01"), None);
        assert_eq!(
            legacy_patchset_coordinates("P-SEC-0127-SNAPSHOT-ONLY-VERIFY-2"),
            None
        );
    }

    #[test]
    fn legacy_source_change_id_must_match_the_source_patchset_prefix_and_sequence() {
        assert!(validate_legacy_source_change_id("RSEC-0169", "RSEP-0169-3").is_ok());
        assert!(validate_legacy_source_change_id("RCC-0079", "RCP-0079-1").is_ok());
        assert!(validate_legacy_source_change_id("LCC-0044", "P-LCC-0044-3").is_ok());
        assert!(validate_legacy_source_change_id("RSEC-0170", "RSEP-0169-3").is_err());
        assert!(validate_legacy_source_change_id("RSEC-0169", "RCP-0169-3").is_err());
    }

    #[test]
    fn land_process_fixed_selector_is_direct_main_only() {
        let direct_main = serde_json::from_value(json!({
            "target_line": "main",
            "mode": "direct"
        }))
        .unwrap();
        assert!(validate_direct_main_land(&direct_main).is_ok());

        let gate_mode = serde_json::from_value(json!({"mode": "gate"})).unwrap();
        assert!(validate_direct_main_land(&gate_mode).is_err());

        let other_line = serde_json::from_value(json!({"target_line": "feature"})).unwrap();
        assert!(validate_direct_main_land(&other_line).is_err());
    }

    #[test]
    fn legacy_patchset_revision_hint_reads_nested_result_and_rejects_conflicts() {
        let payload = serde_json::from_value(json!({"patchset_id": "LCP-0012-1"})).unwrap();
        let result = serde_json::from_value(json!({
            "attestation": {
                "detail": {
                    "patchset_ci": {"revision_snapshot_id": "SNP-53DA16E6F603"}
                }
            }
        }))
        .unwrap();
        assert_eq!(
            patchset_revision_snapshot_hint(WORKER_JOB_KIND_PATCHSET_CI, &payload, &result, false,)
                .unwrap()
                .as_deref(),
            Some("SNP-53DA16E6F603")
        );

        let main_seed = serde_json::from_value(json!({
            "patchset_id": "RSEP-0169-3",
            "snapshot_id": "SNP-3CC45D21CADE"
        }))
        .unwrap();
        assert!(patchset_revision_snapshot_hint(
            WORKER_JOB_KIND_MAIN_SEED_REFRESH,
            &main_seed,
            &result,
            true,
        )
        .unwrap_err()
        .contains("conflicting revision Snapshot hints"));
    }

    #[test]
    fn snapshot_only_patchset_aliases_are_exact_and_unique() {
        let aliases = LEGACY_SNAPSHOT_PATCHSET_ALIASES
            .iter()
            .map(|(patchset_id, _)| *patchset_id)
            .collect::<BTreeSet<_>>();
        assert_eq!(aliases.len(), LEGACY_SNAPSHOT_PATCHSET_ALIASES.len());
        assert!(aliases.contains("manual-ait-core-main-seed-refresh-snp-67aaa1063113"));
        assert!(!aliases.contains("P-SEC-0127-SNAPSHOT-ONLY-VERIFY-3"));
    }

    #[test]
    fn repo_id_gate_requires_uppercase_exact_width() {
        assert!(validate_repo_id("REPO-0123456789ABCDEF01234567").is_ok());
        assert!(validate_repo_id("REPO-0123456789abcdef01234567").is_err());
        assert!(validate_repo_id("REPO-1234").is_err());
    }

    #[test]
    fn source_job_state_mapping_is_closed_and_lease_message_specific() {
        let queued = job(1, "queued", Some(LEASE_EXPIRED_REQUEUED));
        let jobs = BTreeMap::from([(1, &queued)]);
        let mapping = BTreeMap::from([(1, (0, 0))]);
        let mut recovery_audit = RecoveryJobAudit::default();
        assert_eq!(
            convert_state(
                &queued,
                &JsonMap::new(),
                0,
                &jobs,
                &mapping,
                false,
                &mut recovery_audit,
            )
            .unwrap(),
            (
                WORKER_JOB_STATE_QUEUED,
                WORKER_JOB_OUTCOME_NONE,
                WORKER_JOB_ERROR_LEASE_EXPIRED
            )
        );

        let wrong = job(2, "queued", Some(LEASE_EXPIRED_FAILED));
        let jobs = BTreeMap::from([(2, &wrong)]);
        let mapping = BTreeMap::from([(2, (0, 0))]);
        let mut recovery_audit = RecoveryJobAudit::default();
        assert!(convert_state(
            &wrong,
            &JsonMap::new(),
            0,
            &jobs,
            &mapping,
            false,
            &mut recovery_audit,
        )
        .is_err());

        let succeeded = job(3, "succeeded", None);
        let jobs = BTreeMap::from([(3, &succeeded)]);
        let mapping = BTreeMap::from([(3, (0, 0))]);
        let mut recovery_audit = RecoveryJobAudit::default();
        assert_eq!(
            convert_state(
                &succeeded,
                &JsonMap::new(),
                0,
                &jobs,
                &mapping,
                false,
                &mut recovery_audit,
            )
            .unwrap(),
            (
                WORKER_JOB_STATE_SUCCEEDED,
                WORKER_JOB_OUTCOME_COMPLETED,
                WORKER_JOB_ERROR_NONE
            )
        );
    }

    #[test]
    fn historical_attached_validation_uses_frozen_source_after_dense_omission() {
        let active = job(1, "succeeded", None);
        let attached = job(2, "succeeded", None);
        let source_by_id = BTreeMap::from([(1, &active), (2, &attached)]);

        validate_historical_attached_result_job(
            &attached,
            Some(&json!("1")),
            &source_by_id,
            HistoricalAttachedOrdering::Earlier,
        )
        .unwrap();

        let mut cross_repository = active.clone();
        cross_repository.repo_id = "REPO-FFFFFFFFFFFFFFFFFFFFFFFF".to_string();
        let source_by_id = BTreeMap::from([(1, &cross_repository), (2, &attached)]);
        assert!(validate_historical_attached_result_job(
            &attached,
            Some(&json!("1")),
            &source_by_id,
            HistoricalAttachedOrdering::Earlier,
        )
        .unwrap_err()
        .contains("cross-Repository"));
    }

    #[test]
    fn authority_copy_preserves_independent_regular_files() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ait-server-import-authority-copy-{}-{nonce}",
            std::process::id()
        ));
        let source = root.join("source");
        let target = root.join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(source.join("nested/authority.bin"), b"source-authority").unwrap();

        copy_directory_contents(&source, &target).unwrap();
        assert_eq!(
            fs::read(target.join("nested/authority.bin")).unwrap(),
            b"source-authority"
        );
        fs::write(target.join("nested/authority.bin"), b"target-authority").unwrap();
        assert_eq!(
            fs::read(source.join("nested/authority.bin")).unwrap(),
            b"source-authority"
        );

        fs::remove_dir_all(&root).unwrap();
    }
}
