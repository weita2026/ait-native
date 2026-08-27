use crate::fresh_generation::{initialize_fresh_generation, initialize_repository_authority};
use crate::repository_retirement::{
    collect_export_files, copy_manifest_files, create_restore_session_directory,
    finish_purge_journal, namespace_ascii, prepare_purge_journal, read_export_file,
    recover_purge_journal, restore_staging_parent, validate_staged_archive, write_staged_file,
    RemoteExportManifest, REMOTE_EXPORT_SCHEMA, REMOTE_EXPORT_STATE_COMPLETE,
};
use ait_server_core::foundation::native_repositories::BinaryDbNativeRepositoryService;
use ait_server_core::foundation::operational_binary_v0::{
    REPOSITORY_LIFECYCLE_ACTIVE, REPOSITORY_LIFECYCLE_PURGED, REPOSITORY_LIFECYCLE_RETIRING,
    WORKER_JOB_ERROR_RETRYABLE_EXECUTION, WORKER_JOB_ERROR_TERMINAL_EXECUTION,
    WORKER_JOB_KIND_PATCHSET_CI, WORKER_JOB_KIND_REPO_CI, WORKER_JOB_OUTCOME_COMPLETED,
    WORKER_JOB_STATE_FAILED, WORKER_JOB_STATE_QUEUED, WORKER_JOB_STATE_RUNNING,
    WORKER_JOB_STATE_SUCCEEDED,
};
use ait_server_core::foundation::remote_binary_db::{
    sync_filesystem_directory, BinaryDbError, BinaryDbErrorKind, BinaryDbReadTxn,
    FilesystemServerRemoteBinaryDb, RepoId, RepoName, StoreGeneration, StorePath, StoreResult,
};
use ait_server_core::foundation::server_binary_lifecycle::{
    ServerBinaryActivation, ServerBinaryLifecycleConfig, SERVER_FRESH_COMPLETION_FILE,
    SERVER_LEGACY_CONVERSION_COMPLETION_FILE,
};
use ait_server_core::foundation::server_content_binary_db::{
    validate_server_snapshot_dag_v0, validate_server_tree_serving_authority_v0,
    ServerBinaryRepositoryContentStore,
};
use ait_server_core::foundation::server_operational_job_domain::{
    FrozenBinaryV0WorkerJobAuthority, WorkerJobCompactCiEvidence, WorkerJobExecutionAuthority,
    WorkerJobKind,
};
use ait_server_core::foundation::server_operational_repository_registry::{
    OperationalRepositoryEntry, RepositoryCreateSpec, ServerOperationalRepositoryRegistry,
    FIXED_REPOSITORY_NAMES,
};
use ait_server_core::foundation::server_operational_runtime_leases::{
    RuntimeLeaseGrant, RuntimeLeaseToken, ServerOperationalRuntimeLeases,
};
use ait_server_core::foundation::server_operational_worker_jobs::{
    merge_ready_candidates, ServerOperationalWorkerJobStore, WorkerJobDomainAuthority,
    WorkerJobEntry,
};
use ait_server_core::foundation::workflow_binary_v0::{
    CI_STATUS_ERROR, CI_STATUS_FAIL, CI_STATUS_NONE, CI_STATUS_PASS,
};
use ait_server_core::foundation::workflow_binary_v0_adapter::{
    validate_frozen_server_workflow_v0, BinaryDbServerWorkflowV0Store,
};
use fs2::FileExt;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

const GENERATION_SCHEMA: &str = "ait.server.binary_v0.operational_generation.v1";
const POSTGRES_CONVERSION_COMPLETION_SCHEMA: &str = "ait.server.postgres_to_binary_v0.complete.v1";
const POSTGRES_CONVERSION_REPORT_SCHEMA: &str = "ait.server.postgres_to_binary_v0.report.v1";
const U64_SECOND_UPGRADE_COMPLETION_SCHEMA: &str =
    "ait.server.binary_v0.u64_second_upgrade.complete.v1";
const U64_SECOND_UPGRADE_REPORT_SCHEMA: &str = "ait.server.binary_v0.u64_second_upgrade.report.v1";
// Read-only startup compatibility for a generation produced by the retired
// incident converter. The release server has no command or writer for it.
const LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA: &str =
    "ait.server.binary_v0.plan_lineage_repair.complete.v1";
const LEGACY_PLAN_LINEAGE_REPORT_SCHEMA: &str =
    "ait.server.binary_v0.plan_lineage_repair.report.v1";
const LEGACY_PLAN_LINEAGE_RECEIPT_FILE: &str = "plan-lineage-repair-receipt.json";
const U32_TIME_V0_SELECTOR: &str = "u32-time-v0";
const U64_SECOND_V0_SELECTOR: &str = "u64-second-v0";
const FRESH_COMPLETION_SCHEMA: &str = "ait.server.binary_v0.fresh.complete.v1";
const OPERATIONAL_CAPABILITY_CONTRACT: &str = "ait.server.operational-capabilities.v1";
const WORKER_JOB_SERVICE_CONTRACT: &str = "ait.server.worker-job.service.v1";
pub(crate) const NATIVE_JOB_V3_CONTRACT: &str = "ait.runner.native-job.v3";
pub(crate) const NATIVE_JOB_REPOSITORY_CI_ARGV0: &str = "./ci/run";
pub(crate) const NATIVE_JOB_REPOSITORY_CI_UNIX_PATH: &str = "ci/run.sh";
pub(crate) const NATIVE_JOB_REPOSITORY_CI_WINDOWS_PATH: &str = "ci/run.ps1";
#[cfg(test)]
const NATIVE_JOB_V2_CONTRACT: &str = "ait.runner.native-job.v2";
const NATIVE_RESULT_CONTRACT: &str = "ait.runner.native-result.v1";
const DEFAULT_LEASE_DURATION_S: u32 = 60;
const DEFAULT_RETRY_DELAY_S: u32 = 15;
const DEFAULT_NATIVE_TIMEOUT_MS: u64 = 15 * 60 * 1000;
const MAX_LIST_LIMIT: usize = 1_000;
const MAX_CLAIM_CANDIDATES: usize = 1_024;
const MAX_FAILURE_DETAIL_CHARS: usize = 4_096;
const MAX_RESTORE_SESSIONS: usize = 64;
const DEFAULT_RESTORE_SESSION_TTL_S: u64 = 24 * 60 * 60;
const DEFAULT_RESTORE_STAGING_MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const RESTORE_STAGING_LOCK_FILE: &str = ".restore-sessions.lock";

pub(crate) type OperationalDb = FilesystemServerRemoteBinaryDb;
type FrozenAuthority = FrozenBinaryV0WorkerJobAuthority<OperationalDb>;

fn ensure_runtime_activation(
    config: &ServerBinaryLifecycleConfig,
) -> Result<ServerBinaryActivation, String> {
    if let Some(activation) = config.activation()? {
        return Ok(activation);
    }
    Err(format!(
        "Binary server activation is missing at {}. Refusing to create an empty generation during startup. Use `ait-server init --data {}` or `ait-server --data {} --init-if-missing` for an intentional new installation.",
        config.activation_pointer.display(),
        config.server_data_root.display(),
        config.server_data_root.display(),
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledRuntimeInitialization {
    pub runtime_root: PathBuf,
    pub activation_pointer: PathBuf,
    pub generation_root: PathBuf,
    pub created: bool,
}

pub fn initialize_installed_runtime(
    config: &ServerBinaryLifecycleConfig,
) -> Result<InstalledRuntimeInitialization, String> {
    if let Some(activation) = config.activation()? {
        return Ok(installed_initialization_report(config, activation, false));
    }
    require_empty_fresh_runtime_root(&config.server_data_root)?;
    let created_at_s = now_s()?;
    let activation = config.ensure_fresh_activation(|generation_root| {
        initialize_fresh_generation(generation_root, created_at_s)
    })?;
    Ok(installed_initialization_report(config, activation, true))
}

fn installed_initialization_report(
    config: &ServerBinaryLifecycleConfig,
    activation: ServerBinaryActivation,
    created: bool,
) -> InstalledRuntimeInitialization {
    InstalledRuntimeInitialization {
        runtime_root: config.server_data_root.clone(),
        activation_pointer: activation.activation_pointer,
        generation_root: activation.generation_root,
        created,
    }
}

fn require_empty_fresh_runtime_root(root: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to inspect first-use server runtime root {}: {error}",
                root.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "first-use server runtime root must not be a symbolic link: {}",
            root.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "first-use server runtime root is not a directory: {}",
            root.display()
        ));
    }
    let mut entries = fs::read_dir(root).map_err(|error| {
        format!(
            "failed to inspect first-use server runtime root {}: {error}",
            root.display()
        )
    })?;
    if entries
        .next()
        .transpose()
        .map_err(|error| {
            format!(
                "failed to inspect first-use server runtime root {}: {error}",
                root.display()
            )
        })?
        .is_some()
    {
        return Err(format!(
            "first-use server runtime root is non-empty but has no valid Binary activation: {}. Refusing to create an empty authority over possible existing data.",
            root.display()
        ));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GenerationManifest {
    schema: String,
    layout_id: u32,
    status: String,
    global_registry: String,
    repository_authorities: String,
    repository_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionEvidence {
    schema: String,
    layout_id: u32,
    status: String,
    report_sha256: String,
}

#[derive(Debug, Deserialize)]
struct ConversionReportEvidence {
    schema: String,
    layout_id: u32,
    status: String,
    #[serde(default)]
    source_selector: Option<String>,
    #[serde(default)]
    target_selector: Option<String>,
    #[serde(default)]
    repository_indexes: Vec<u32>,
    #[serde(default)]
    sealed_files: Vec<LegacySealedFileEvidence>,
}

#[derive(Debug, Deserialize)]
struct LegacySealedFileEvidence {
    relative_path: String,
    byte_size: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FreshCompletionEvidence {
    schema: String,
    layout_id: u32,
    status: String,
    generation_manifest_sha256: String,
    repository_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimRequest {
    accepted_runtime_contracts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimNextRequest {
    accepted_job_kinds: Vec<u8>,
    #[serde(default)]
    repository_indexes: Vec<u32>,
    accepted_runtime_contracts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseRequest {
    attempt_count: u16,
    lease_token: String,
    #[serde(default)]
    detail: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreStartRequest {
    manifest: RemoteExportManifest,
    policy_flags: u8,
}

#[derive(Clone)]
struct RestoreSession {
    manifest: RemoteExportManifest,
    policy_flags: u8,
    session_root: PathBuf,
    reserved_bytes: u64,
    expires_at_s: u64,
    committed: Option<JsonValue>,
}

#[derive(Clone)]
struct OperationalRepositoryRuntime {
    entry: OperationalRepositoryEntry,
    authority: Arc<FrozenAuthority>,
    jobs: ServerOperationalWorkerJobStore,
    native: BinaryDbNativeRepositoryService<OperationalDb>,
}

#[derive(Clone)]
pub(crate) struct OperationalBinaryRuntime {
    registry: ServerOperationalRepositoryRegistry,
    repositories: Arc<RwLock<BTreeMap<u32, OperationalRepositoryRuntime>>>,
    registration_lock: Arc<Mutex<()>>,
    retirement_exports: Arc<Mutex<BTreeMap<u32, RemoteExportManifest>>>,
    restore_sessions: Arc<Mutex<BTreeMap<String, RestoreSession>>>,
    restore_staging_parent: PathBuf,
    _restore_staging_lock: Arc<File>,
    restore_session_ttl_s: u64,
    restore_staging_max_bytes: u64,
    leases: ServerOperationalRuntimeLeases,
    lease_duration_s: u32,
    retry_delay_s: u32,
}

impl OperationalBinaryRuntime {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn repository_indexes(&self) -> Vec<u32> {
        self.repositories
            .read()
            .expect("Repository runtime lock is poisoned")
            .keys()
            .copied()
            .collect()
    }

    pub(crate) fn ensure_from_config(config: &ServerBinaryLifecycleConfig) -> Result<Self, String> {
        let activation = ensure_runtime_activation(config)?;
        Self::from_activation_with_restore_limits(
            &activation,
            config.runtime_lease_replica.clone(),
            DEFAULT_LEASE_DURATION_S,
            DEFAULT_RETRY_DELAY_S,
            DEFAULT_RESTORE_SESSION_TTL_S,
            DEFAULT_RESTORE_STAGING_MAX_BYTES,
        )
    }

    #[cfg(test)]
    pub(crate) fn from_activation(
        activation: &ServerBinaryActivation,
        lease_replica: PathBuf,
        lease_duration_s: u32,
        retry_delay_s: u32,
    ) -> Result<Self, String> {
        Self::from_activation_with_restore_limits(
            activation,
            lease_replica,
            lease_duration_s,
            retry_delay_s,
            DEFAULT_RESTORE_SESSION_TTL_S,
            DEFAULT_RESTORE_STAGING_MAX_BYTES,
        )
    }

    fn from_activation_with_restore_limits(
        activation: &ServerBinaryActivation,
        lease_replica: PathBuf,
        lease_duration_s: u32,
        retry_delay_s: u32,
        restore_session_ttl_s: u64,
        restore_staging_max_bytes: u64,
    ) -> Result<Self, String> {
        let completion_bytes = read_regular_file(&activation.completion_file)?;
        if sha256(&completion_bytes) != activation.completion_sha256 {
            return Err(
                "Binary v0 lifecycle completion hash changed before runtime open".to_string(),
            );
        }
        validate_completion(
            &activation.generation_root,
            &activation.completion_file,
            &completion_bytes,
        )?;
        Self::open_generation_with_restore_limits(
            activation.generation_root.clone(),
            lease_replica,
            lease_duration_s,
            retry_delay_s,
            restore_session_ttl_s,
            restore_staging_max_bytes,
        )
    }

    #[cfg(test)]
    pub(crate) fn open_generation(
        generation_root: PathBuf,
        lease_replica: PathBuf,
        lease_duration_s: u32,
        retry_delay_s: u32,
    ) -> Result<Self, String> {
        Self::open_generation_with_restore_limits(
            generation_root,
            lease_replica,
            lease_duration_s,
            retry_delay_s,
            DEFAULT_RESTORE_SESSION_TTL_S,
            DEFAULT_RESTORE_STAGING_MAX_BYTES,
        )
    }

    fn open_generation_with_restore_limits(
        generation_root: PathBuf,
        lease_replica: PathBuf,
        lease_duration_s: u32,
        retry_delay_s: u32,
        restore_session_ttl_s: u64,
        restore_staging_max_bytes: u64,
    ) -> Result<Self, String> {
        require_positive_runtime_durations(lease_duration_s, retry_delay_s)?;
        if restore_session_ttl_s == 0 {
            return Err("Repository restore session TTL must be positive".to_string());
        }
        if restore_staging_max_bytes == 0 {
            return Err("Repository restore staging byte ceiling must be positive".to_string());
        }
        let generation_root = canonical_real_directory(&generation_root)?;
        let manifest_bytes = read_regular_file(&generation_root.join("generation.json"))?;
        let manifest: GenerationManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|error| {
                format!(
                    "failed to parse Binary v0 generation manifest {}: {error}",
                    generation_root.join("generation.json").display()
                )
            })?;
        if manifest.schema != GENERATION_SCHEMA
            || manifest.layout_id != 1
            || manifest.status != "validated_inactive"
            || manifest.global_registry != "global"
            || manifest.repository_authorities != "repositories"
        {
            return Err("Binary v0 generation manifest is invalid".to_string());
        }

        let global_root = canonical_real_directory(&generation_root.join("global"))?;
        let repositories_root = canonical_real_directory(&generation_root.join("repositories"))?;
        let registry = ServerOperationalRepositoryRegistry::new(&global_root, &repositories_root)
            .map_err(|error| format!("open Binary Repository registry: {error}"))?;
        registry
            .recover()
            .map_err(|error| format!("recover Binary Repository registry: {error}"))?;
        let entries = registry
            .validate()
            .map_err(|error| format!("validate Binary Repository registry: {error}"))?;
        if entries.len() < manifest.repository_count {
            return Err(format!(
                "Binary v0 generation Repository registry is shorter than its activation baseline: manifest={}, registry={}",
                manifest.repository_count,
                entries.len()
            ));
        }

        let mut repositories = BTreeMap::new();
        for entry in entries {
            let authority_root = registry
                .resolve_authority_directory(entry.repository_index)
                .map_err(|error| {
                    format!(
                        "resolve Binary Repository {} purge recovery root: {error}",
                        entry.repository_index
                    )
                })?;
            recover_purge_journal(&authority_root, entry.record.lifecycle_kind).map_err(
                |error| {
                    format!(
                        "recover Binary Repository {} purge journal: {error}",
                        entry.repository_index
                    )
                },
            )?;
            let repository = open_repository_runtime(&registry, entry)?;
            repositories.insert(repository.entry.repository_index, repository);
        }

        let restore_staging_parent = restore_staging_parent(&generation_root)
            .map_err(|error| format!("prepare Repository restore staging: {error}"))?;
        let restore_staging_lock = acquire_restore_staging_lock(&restore_staging_parent)?;
        cleanup_abandoned_restore_staging(&restore_staging_parent)?;
        let (leases, _) =
            ServerOperationalRuntimeLeases::open(lease_replica, [generation_root.clone()])
                .map_err(|error| format!("open Binary runtime lease replica: {error}"))?;
        let runtime = Self {
            registry,
            repositories: Arc::new(RwLock::new(repositories)),
            registration_lock: Arc::new(Mutex::new(())),
            retirement_exports: Arc::new(Mutex::new(BTreeMap::new())),
            restore_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            restore_staging_parent,
            _restore_staging_lock: Arc::new(restore_staging_lock),
            restore_session_ttl_s,
            restore_staging_max_bytes,
            leases,
            lease_duration_s,
            retry_delay_s,
        };
        runtime.reconcile(now_s()?)?;
        Ok(runtime)
    }

    pub(crate) fn capabilities(&self) -> JsonValue {
        json!({
            "operational_capabilities": {
                "contract": OPERATIONAL_CAPABILITY_CONTRACT,
                "repository_identity": "binary-repository-index.v0",
                "worker_job_identity": "binary-worker-job-key.v0",
                "runner_contracts": [NATIVE_JOB_V3_CONTRACT],
            }
        })
    }

    pub(crate) fn repository_db(
        &self,
        repository_index: u32,
    ) -> StoreResult<(OperationalRepositoryEntry, OperationalDb)> {
        let repository = self.repository(repository_index)?;
        Ok((repository.entry.clone(), repository.authority.db().clone()))
    }

    pub(crate) fn repository_native_service(
        &self,
        repository_index: u32,
    ) -> StoreResult<(
        OperationalRepositoryEntry,
        BinaryDbNativeRepositoryService<OperationalDb>,
    )> {
        let repository = self.repository(repository_index)?;
        Ok((repository.entry.clone(), repository.native.clone()))
    }

    pub(crate) fn serving_repository_indexes(&self) -> Vec<u32> {
        self.repositories
            .read()
            .expect("Repository runtime lock is poisoned")
            .values()
            .filter(|repository| {
                !repository.entry.record.is_tombstoned()
                    && repository.entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
            })
            .map(|repository| repository.entry.repository_index)
            .collect()
    }

    pub(crate) fn register_repository(
        &self,
        repository_name: &str,
        namespace_ascii: [u8; 2],
        policy_flags: u8,
    ) -> StoreResult<JsonValue> {
        if repository_name.is_empty() {
            return Err(invalid("Repository name is empty"));
        }
        ait_server_core::foundation::operational_binary_v0::validate_namespace(namespace_ascii)?;
        let _registration = self
            .registration_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Repository registration lock is poisoned"))?;

        if namespace_ascii != [0, 0] {
            if let Some(existing) = self
                .registry
                .discover_live_namespace(namespace_ascii)?
                .into_iter()
                .next()
            {
                if existing.record.lifecycle_kind == REPOSITORY_LIFECYCLE_RETIRING {
                    return Err(invalid(format!(
                        "Repository namespace {:?} is retiring at index {}; finish retirement before registering it again",
                        namespace_ascii, existing.repository_index
                    )));
                }
                if existing.repo_name != repository_name
                    || existing.record.policy_flags != policy_flags
                {
                    return Err(invalid(format!(
                        "Repository namespace {:?} is already owned by index {} with different registration metadata",
                        namespace_ascii, existing.repository_index
                    )));
                }
                self.ensure_repository_runtime(existing.clone())?;
                return Ok(repository_registration_projection(existing, false));
            }
        }

        let spec = RepositoryCreateSpec {
            repo_name: repository_name.to_string(),
            namespace_ascii,
            policy_flags,
            created_at_s: now_s_store()?,
        };
        let entry = self.registry.append_repository_with_initializer(
            &spec,
            |repository_index, authority_root| {
                initialize_repository_authority(authority_root, repository_index, repository_name)
                    .map_err(BinaryDbError::other)
            },
        )?;
        self.ensure_repository_runtime(entry.clone())?;
        Ok(repository_registration_projection(entry, true))
    }

    pub(crate) fn enqueue_patchset_ci(
        &self,
        repository_index: u32,
        patchset_index: u32,
        starts_new_run: bool,
        max_attempts: u16,
    ) -> StoreResult<JsonValue> {
        self.require_active_repository(repository_index)?;
        let repository = self.repository(repository_index)?;
        let patchset = repository.authority.frozen_patchset_at(patchset_index)?;
        if let Some(worker_job_index) = minus_one(patchset.ci_worker_job_index_plus1) {
            let selected = repository.jobs.get(worker_job_index)?;
            if selected.record.job_kind == WORKER_JOB_KIND_PATCHSET_CI
                && selected.record.patchset_index_plus1 == patchset_index.saturating_add(1)
                && matches!(
                    selected.record.state_kind,
                    WORKER_JOB_STATE_QUEUED | WORKER_JOB_STATE_RUNNING
                )
            {
                return self.job_projection(&repository, selected);
            }
        }
        let now = now_s_store()?;
        let entry = repository.authority.enqueue_patchset_ci_job(
            &repository.jobs,
            patchset_index,
            max_attempts,
            now,
            now,
            starts_new_run,
        )?;
        self.job_projection(&repository, entry)
    }

    pub(crate) fn snapshot_index_for_id(
        &self,
        repository_index: u32,
        snapshot_id: &str,
    ) -> StoreResult<u32> {
        self.repository(repository_index)?
            .authority
            .snapshot_index_for_id(snapshot_id)
    }

    pub(crate) fn patchset_ci_jobs(
        &self,
        repository_index: u32,
        patchset_index: u32,
        limit: usize,
    ) -> StoreResult<JsonValue> {
        if !(1..=MAX_LIST_LIMIT).contains(&limit) {
            return Err(invalid(format!(
                "Patchset CI Job list limit must be between 1 and {MAX_LIST_LIMIT}"
            )));
        }
        let repository = self.repository(repository_index)?;
        let patchset = repository.authority.frozen_patchset_at(patchset_index)?;
        let selected_worker_job_index = minus_one(patchset.ci_worker_job_index_plus1);
        let mut entries = repository
            .jobs
            .list()?
            .into_iter()
            .filter(|entry| {
                entry.record.job_kind == WORKER_JOB_KIND_PATCHSET_CI
                    && entry.record.patchset_index_plus1 == patchset_index.saturating_add(1)
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.key.worker_job_index));
        let jobs = entries
            .into_iter()
            .take(limit)
            .map(|entry| self.job_projection(&repository, entry))
            .collect::<StoreResult<Vec<_>>>()?;
        let latest_job = selected_worker_job_index
            .and_then(|selected| {
                jobs.iter()
                    .find(|job| job["worker_job_index"].as_u64() == Some(u64::from(selected)))
            })
            .cloned()
            .unwrap_or(JsonValue::Null);
        Ok(json!({
            "repository_index": repository_index,
            "patchset_index": patchset_index,
            "selected_worker_job_index": selected_worker_job_index,
            "latest_job": latest_job,
            "recent_jobs": jobs,
        }))
    }

    pub(crate) fn enqueue_repo_ci(
        &self,
        repository_index: u32,
        snapshot_index: u32,
        max_attempts: u16,
    ) -> StoreResult<JsonValue> {
        self.require_active_repository(repository_index)?;
        let repository = self.repository(repository_index)?;
        let now = now_s_store()?;
        let entry = repository.authority.enqueue_repo_ci_job(
            &repository.jobs,
            snapshot_index,
            max_attempts,
            now,
            now,
        )?;
        self.job_projection(&repository, entry)
    }

    pub(crate) fn repository_authority(&self, repository_index: u32) -> StoreResult<JsonValue> {
        let repository = self.repository(repository_index)?;
        let pack_storage =
            ServerBinaryRepositoryContentStore::new(repository.authority.db().clone())
                .repository_pack_storage_payload()?;
        Ok(json!({
            "contract": "ait.server.repository-authority.v1",
            "repository": repository_projection(&repository.entry),
            "pack_storage": pack_storage,
        }))
    }

    pub(crate) fn repository_authorities(
        &self,
        repository_name: Option<&str>,
    ) -> StoreResult<JsonValue> {
        if repository_name.is_some_and(str::is_empty) {
            return Err(invalid("Repository discovery name cannot be empty"));
        }
        let repositories = self.repositories_snapshot()?;
        let repositories = repositories
            .values()
            .filter(|repository| {
                repository.entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
                    && repository_name.is_none_or(|name| repository.entry.repo_name == name)
            })
            .map(|repository| repository_projection(&repository.entry))
            .collect::<Vec<_>>();
        Ok(json!({
            "contract": "ait.server.repository-authority.v1",
            "repositories": repositories,
            "count": repositories.len(),
            "discovery": {
                "repository_name": repository_name,
                "routing_authority": false,
            },
        }))
    }

    pub(crate) fn require_active_repository(&self, repository_index: u32) -> StoreResult<()> {
        let entry = self.registry.get(repository_index)?;
        if entry.record.is_tombstoned()
            || entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_ACTIVE
        {
            let lifecycle = match entry.record.lifecycle_kind {
                REPOSITORY_LIFECYCLE_ACTIVE => "active",
                REPOSITORY_LIFECYCLE_RETIRING => "retiring",
                REPOSITORY_LIFECYCLE_PURGED => "purged",
                _ => "unsupported",
            };
            return Err(invalid(format!(
                "Repository index {repository_index} is {lifecycle} and does not admit new mutations"
            )));
        }
        Ok(())
    }

    pub(crate) fn begin_repository_retirement(
        &self,
        repository_index: u32,
    ) -> StoreResult<JsonValue> {
        let _registration = self
            .registration_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Repository registration lock is poisoned"))?;
        let now = now_s_store()?;
        self.reconcile(now)?;
        let current = self.registry.get(repository_index)?;
        let entry = match current.record.lifecycle_kind {
            REPOSITORY_LIFECYCLE_ACTIVE => self.registry.update_live_metadata(
                repository_index,
                REPOSITORY_LIFECYCLE_RETIRING,
                current.record.namespace_ascii,
                current.record.policy_flags,
                now.max(current.record.updated_at_s),
            )?,
            REPOSITORY_LIFECYCLE_RETIRING => current,
            REPOSITORY_LIFECYCLE_PURGED => {
                return Err(invalid(format!(
                    "Repository index {repository_index} is already purged"
                )))
            }
            other => {
                return Err(corrupt(format!(
                    "Repository index {repository_index} has unsupported lifecycle kind {other}"
                )))
            }
        };
        self.update_repository_entry(entry.clone())?;
        self.retirement_exports
            .lock()
            .map_err(|_| BinaryDbError::other("Repository export cache lock is poisoned"))?
            .remove(&repository_index);
        self.retirement_projection(entry)
    }

    pub(crate) fn repository_retirement(&self, repository_index: u32) -> StoreResult<JsonValue> {
        let _registration = self
            .registration_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Repository registration lock is poisoned"))?;
        self.reconcile(now_s_store()?)?;
        let entry = self.registry.get(repository_index)?;
        if entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_RETIRING {
            return Err(invalid(format!(
                "Repository index {repository_index} is not retiring"
            )));
        }
        self.update_repository_entry(entry.clone())?;
        self.retirement_projection(entry)
    }

    pub(crate) fn abort_repository_retirement(
        &self,
        repository_index: u32,
    ) -> StoreResult<JsonValue> {
        let _registration = self
            .registration_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Repository registration lock is poisoned"))?;
        let now = now_s_store()?;
        let current = self.registry.get(repository_index)?;
        if current.record.is_tombstoned() {
            return Err(invalid(format!(
                "Repository index {repository_index} is tombstoned and cannot abort retirement"
            )));
        }
        let (entry, already_aborted) = match current.record.lifecycle_kind {
            REPOSITORY_LIFECYCLE_ACTIVE => (current, true),
            REPOSITORY_LIFECYCLE_RETIRING => (
                self.registry.update_live_metadata(
                    repository_index,
                    REPOSITORY_LIFECYCLE_ACTIVE,
                    current.record.namespace_ascii,
                    current.record.policy_flags,
                    now.max(current.record.updated_at_s),
                )?,
                false,
            ),
            REPOSITORY_LIFECYCLE_PURGED => {
                return Err(invalid(format!(
                    "Repository index {repository_index} is purged and its retirement cannot be aborted"
                )))
            }
            other => {
                return Err(corrupt(format!(
                    "Repository index {repository_index} has unsupported lifecycle kind {other}"
                )))
            }
        };
        self.update_repository_entry(entry.clone())?;
        self.retirement_exports
            .lock()
            .map_err(|_| BinaryDbError::other("Repository export cache lock is poisoned"))?
            .remove(&repository_index);
        Ok(repository_retirement_abort_projection(
            entry,
            already_aborted,
        ))
    }

    pub(crate) fn repository_retirement_file(
        &self,
        repository_index: u32,
        file_path: &str,
    ) -> StoreResult<Vec<u8>> {
        let _registration = self
            .registration_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Repository registration lock is poisoned"))?;
        let entry = self.registry.get(repository_index)?;
        if entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_RETIRING {
            return Err(invalid(format!(
                "Repository index {repository_index} is not retiring"
            )));
        }
        let repository = self.repository_any(repository_index)?;
        if !repository.jobs.retirement_blockers()?.is_drained() {
            return Err(invalid(
                "Repository Worker Jobs must drain before authority files can be exported",
            ));
        }
        let manifest = self.retirement_manifest(&repository, &entry, true)?;
        let expected = manifest
            .files
            .iter()
            .find(|file| file.path == file_path)
            .cloned()
            .ok_or_else(|| {
                BinaryDbError::missing_data(format!(
                    "remote export manifest has no authority file {file_path:?}"
                ))
            })?;
        let read = BinaryDbReadTxn::new(repository.authority.db());
        read.read_lock_paths()?;
        read_export_file(repository.jobs.authority_root(), &expected)
    }

    pub(crate) fn purge_retired_repository(
        &self,
        repository_index: u32,
        payload: &JsonValue,
    ) -> StoreResult<JsonValue> {
        let acknowledged: RemoteExportManifest =
            decode_request(payload, "Repository retirement purge acknowledgement")?;
        acknowledged.validate()?;
        let _registration = self
            .registration_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Repository registration lock is poisoned"))?;
        self.reconcile(now_s_store()?)?;
        let current = self.registry.get(repository_index)?;
        if current.record.lifecycle_kind == REPOSITORY_LIFECYCLE_PURGED {
            if acknowledged.repo_name != current.repo_name
                || namespace_ascii(&acknowledged.namespace)? != current.record.namespace_ascii
            {
                return Err(invalid(
                    "purged Repository acknowledgement metadata does not match the registry",
                ));
            }
            recover_purge_journal(
                &self
                    .registry
                    .resolve_authority_directory(repository_index)?,
                REPOSITORY_LIFECYCLE_PURGED,
            )?;
            if let Ok(repository) = self.repository_any(repository_index) {
                repository
                    .native
                    .invalidate_zstd_pull_catalog()
                    .map_err(|error| BinaryDbError::other(error.to_string()))?;
            }
            return Ok(repository_purge_projection(current, true));
        }
        if current.record.lifecycle_kind != REPOSITORY_LIFECYCLE_RETIRING {
            return Err(invalid(format!(
                "Repository index {repository_index} is not retiring"
            )));
        }
        let repository = self.repository_any(repository_index)?;
        if !repository.jobs.retirement_blockers()?.is_drained() {
            return Err(invalid("Repository Worker Jobs must drain before purge"));
        }
        let actual = self.retirement_manifest(&repository, &current, false)?;
        if actual != acknowledged {
            return Err(invalid(
                "Repository purge acknowledgement does not exactly match the current complete export manifest",
            ));
        }

        let authority_root = repository.jobs.authority_root().to_path_buf();
        let purge_time = now_s_store()?.max(current.record.updated_at_s);
        let coordinated =
            self.registry
                .coordinate_repository_purge(repository_index, purge_time, |_| {
                    prepare_purge_journal(&authority_root)?;
                    let mutation = (|| {
                        repository.authority.clear_all_patchset_ci_job_locators()?;
                        repository.jobs.tombstone_all(purge_time)?;
                        Ok(())
                    })();
                    if let Err(error) = mutation {
                        recover_purge_journal(&authority_root, REPOSITORY_LIFECYCLE_RETIRING)?;
                        repository.jobs.recover()?;
                        return Err(error);
                    }
                    Ok(())
                });
        let purged = match coordinated {
            Ok(entry) => entry,
            Err(error) => {
                let lifecycle = self
                    .registry
                    .get(repository_index)
                    .map(|entry| entry.record.lifecycle_kind)
                    .unwrap_or(REPOSITORY_LIFECYCLE_RETIRING);
                if let Err(recovery_error) = recover_purge_journal(&authority_root, lifecycle) {
                    return Err(BinaryDbError::other(format!(
                        "{error}; additionally failed to recover Repository purge: {recovery_error}"
                    )));
                }
                if lifecycle == REPOSITORY_LIFECYCLE_RETIRING {
                    repository.jobs.recover()?;
                }
                return Err(error);
            }
        };
        repository
            .native
            .invalidate_zstd_pull_catalog()
            .map_err(|error| BinaryDbError::other(error.to_string()))?;
        self.update_repository_entry(purged.clone())?;
        finish_purge_journal(&authority_root)?;
        Ok(repository_purge_projection(purged, false))
    }

    pub(crate) fn begin_repository_restore(&self, payload: &JsonValue) -> StoreResult<JsonValue> {
        let request: RestoreStartRequest = decode_request(payload, "Repository restore")?;
        request.manifest.validate()?;
        let reserved_bytes = restore_manifest_bytes(&request.manifest)?;
        if reserved_bytes > self.restore_staging_max_bytes {
            return Err(invalid(format!(
                "Repository restore declares {reserved_bytes} bytes, above the configured staging ceiling {}",
                self.restore_staging_max_bytes
            )));
        }
        let namespace = namespace_ascii(&request.manifest.namespace)?;
        if let Some(existing) = self
            .registry
            .discover_live_namespace(namespace)?
            .into_iter()
            .next()
        {
            return Err(invalid(format!(
                "Repository namespace {:?} is already owned by live index {}",
                namespace, existing.repository_index
            )));
        }
        let now = now_s_store()?;
        let mut sessions = self
            .restore_sessions
            .lock()
            .map_err(|_| BinaryDbError::other("Repository restore session lock is poisoned"))?;
        self.cleanup_expired_restore_sessions_locked(&mut sessions, now)?;
        if sessions
            .values()
            .filter(|session| session.committed.is_none())
            .count()
            >= MAX_RESTORE_SESSIONS
        {
            return Err(BinaryDbError::retryable_busy(
                "Repository restore session capacity is exhausted",
            ));
        }
        let reserved_in_use = sessions
            .values()
            .filter(|session| session.committed.is_none())
            .try_fold(0_u64, |total, session| {
                total.checked_add(session.reserved_bytes).ok_or_else(|| {
                    corrupt("Repository restore staging reservation total overflowed")
                })
            })?;
        if reserved_in_use
            .checked_add(reserved_bytes)
            .is_none_or(|total| total > self.restore_staging_max_bytes)
        {
            return Err(BinaryDbError::retryable_busy(format!(
                "Repository restore staging byte capacity is exhausted: reserved={reserved_in_use}, requested={reserved_bytes}, ceiling={}",
                self.restore_staging_max_bytes
            )));
        }
        let token = loop {
            let token = random_restore_token()?;
            if !sessions.contains_key(&token) && !self.restore_staging_parent.join(&token).exists()
            {
                break token;
            }
        };
        let session_root = create_restore_session_directory(&self.restore_staging_parent, &token)?;
        sessions.insert(
            token.clone(),
            RestoreSession {
                manifest: request.manifest,
                policy_flags: request.policy_flags,
                session_root,
                reserved_bytes,
                expires_at_s: now.saturating_add(self.restore_session_ttl_s),
                committed: None,
            },
        );
        Ok(json!({
            "contract": "ait.server.repository-restore-session.v1",
            "restore_token": token,
            "state": "uploading",
            "reserved_bytes": reserved_bytes,
            "expires_at_s": now.saturating_add(self.restore_session_ttl_s),
        }))
    }

    pub(crate) fn upload_repository_restore_file(
        &self,
        restore_token: &str,
        file_path: &str,
        bytes: &[u8],
    ) -> StoreResult<JsonValue> {
        require_restore_token(restore_token)?;
        let now = now_s_store()?;
        let mut sessions = self
            .restore_sessions
            .lock()
            .map_err(|_| BinaryDbError::other("Repository restore session lock is poisoned"))?;
        self.cleanup_expired_restore_sessions_locked(&mut sessions, now)?;
        let (expected, session_root) = {
            let session = sessions.get(restore_token).ok_or_else(|| {
                BinaryDbError::missing_data("unknown or expired Repository restore session")
            })?;
            if session.committed.is_some() {
                return Err(invalid("Repository restore session is already committed"));
            }
            let expected = session
                .manifest
                .files
                .iter()
                .find(|file| file.path == file_path)
                .cloned()
                .ok_or_else(|| {
                    BinaryDbError::missing_data(format!(
                        "restore manifest has no authority file {file_path:?}"
                    ))
                })?;
            (expected, session.session_root.clone())
        };
        if let Err(error) = write_staged_file(&session_root.join("data"), &expected, bytes) {
            return Err(self.discard_failed_restore_session(&mut sessions, restore_token, error));
        }
        let expires_at_s = now.saturating_add(self.restore_session_ttl_s);
        sessions
            .get_mut(restore_token)
            .expect("restore session remains present while its lock is held")
            .expires_at_s = expires_at_s;
        Ok(json!({
            "contract": "ait.server.repository-restore-session.v1",
            "restore_token": restore_token,
            "state": "uploading",
            "file": expected,
            "expires_at_s": expires_at_s,
        }))
    }

    pub(crate) fn commit_repository_restore(&self, restore_token: &str) -> StoreResult<JsonValue> {
        require_restore_token(restore_token)?;
        let _registration = self
            .registration_lock
            .lock()
            .map_err(|_| BinaryDbError::other("Repository registration lock is poisoned"))?;
        let now = now_s_store()?;
        let mut sessions = self
            .restore_sessions
            .lock()
            .map_err(|_| BinaryDbError::other("Repository restore session lock is poisoned"))?;
        self.cleanup_expired_restore_sessions_locked(&mut sessions, now)?;
        let (data_root, namespace, repository_name, manifest, policy_flags) = {
            let session = sessions.get(restore_token).ok_or_else(|| {
                BinaryDbError::missing_data("unknown or expired Repository restore session")
            })?;
            if let Some(committed) = &session.committed {
                return Ok(committed.clone());
            }
            (
                session.session_root.join("data"),
                namespace_ascii(&session.manifest.namespace)?,
                session.manifest.repo_name.clone(),
                session.manifest.clone(),
                session.policy_flags,
            )
        };
        if let Err(error) = validate_staged_archive(&data_root, &manifest) {
            return Err(self.discard_failed_restore_session(&mut sessions, restore_token, error));
        }
        let created_at_s = now;
        let spec = RepositoryCreateSpec {
            repo_name: repository_name.clone(),
            namespace_ascii: namespace,
            policy_flags,
            created_at_s,
        };
        let entry = match self.registry.append_repository_with_initializer(
            &spec,
            |repository_index, authority_root| {
                copy_manifest_files(&data_root, authority_root, &manifest)?;
                normalize_restored_authority(
                    authority_root,
                    repository_index,
                    &repository_name,
                    namespace,
                    created_at_s,
                )
            },
        ) {
            Ok(entry) => entry,
            Err(error) => {
                return Err(self.discard_failed_restore_session(
                    &mut sessions,
                    restore_token,
                    error,
                ));
            }
        };
        self.ensure_repository_runtime(entry.clone())?;
        let response = json!({
            "contract": "ait.server.repository-restore.v1",
            "created": true,
            "repository": repository_projection(&entry),
        });
        let session = sessions
            .get_mut(restore_token)
            .expect("restore session remains present while its lock is held");
        session.committed = Some(response.clone());
        session.expires_at_s = now.saturating_add(self.restore_session_ttl_s);
        if let Err(error) =
            remove_restore_session_directory(&self.restore_staging_parent, &session.session_root)
        {
            eprintln!(
                "warning: failed to remove committed Repository restore staging {}: {error}",
                session.session_root.display()
            );
        }
        Ok(response)
    }

    pub(crate) fn abort_repository_restore(&self, restore_token: &str) -> StoreResult<JsonValue> {
        require_restore_token(restore_token)?;
        let now = now_s_store()?;
        let mut sessions = self
            .restore_sessions
            .lock()
            .map_err(|_| BinaryDbError::other("Repository restore session lock is poisoned"))?;
        self.cleanup_expired_restore_sessions_locked(&mut sessions, now)?;
        let Some(session) = sessions.get(restore_token) else {
            return Ok(repository_restore_abort_projection(
                restore_token,
                "absent",
                true,
                true,
            ));
        };
        if session.committed.is_some() {
            return Ok(repository_restore_abort_projection(
                restore_token,
                "committed",
                false,
                true,
            ));
        }
        let session_root = session.session_root.clone();
        remove_restore_session_directory(&self.restore_staging_parent, &session_root)?;
        sessions.remove(restore_token);
        Ok(repository_restore_abort_projection(
            restore_token,
            "aborted",
            true,
            false,
        ))
    }

    fn cleanup_expired_restore_sessions_locked(
        &self,
        sessions: &mut BTreeMap<String, RestoreSession>,
        now_s: u64,
    ) -> StoreResult<()> {
        let expired = sessions
            .iter()
            .filter(|(_, session)| now_s >= session.expires_at_s)
            .map(|(token, session)| (token.clone(), session.session_root.clone()))
            .collect::<Vec<_>>();
        for (token, session_root) in expired {
            remove_restore_session_directory(&self.restore_staging_parent, &session_root)?;
            sessions.remove(&token);
        }
        Ok(())
    }

    fn discard_failed_restore_session(
        &self,
        sessions: &mut BTreeMap<String, RestoreSession>,
        restore_token: &str,
        cause: BinaryDbError,
    ) -> BinaryDbError {
        let Some(session) = sessions.get(restore_token) else {
            return cause;
        };
        let session_root = session.session_root.clone();
        match remove_restore_session_directory(&self.restore_staging_parent, &session_root) {
            Ok(()) => {
                sessions.remove(restore_token);
                cause
            }
            Err(cleanup_error) => BinaryDbError::other(format!(
                "{cause}; additionally failed to discard Repository restore staging: {cleanup_error}"
            )),
        }
    }

    pub(crate) fn list_worker_jobs(
        &self,
        repository_index: u32,
        state_kind: Option<u8>,
        limit: usize,
    ) -> StoreResult<JsonValue> {
        validate_state_filter(state_kind)?;
        if !(1..=MAX_LIST_LIMIT).contains(&limit) {
            return Err(invalid(format!(
                "Worker Job list limit must be between 1 and {MAX_LIST_LIMIT}"
            )));
        }
        let repository = self.repository(repository_index)?;
        let jobs = repository
            .jobs
            .list_recent(state_kind, limit)?
            .into_iter()
            .map(|entry| self.job_projection(&repository, entry))
            .collect::<StoreResult<Vec<_>>>()?;
        Ok(json!({
            "contract": WORKER_JOB_SERVICE_CONTRACT,
            "repository_index": repository_index,
            "jobs": jobs,
            "count": jobs.len(),
        }))
    }

    pub(crate) fn worker_job(
        &self,
        repository_index: u32,
        worker_job_index: u32,
    ) -> StoreResult<JsonValue> {
        let repository = self.repository(repository_index)?;
        let entry = repository.jobs.get(worker_job_index)?;
        Ok(json!({
            "contract": WORKER_JOB_SERVICE_CONTRACT,
            "job": self.job_projection(&repository, entry)?,
        }))
    }

    pub(crate) fn claim_worker_job(
        &self,
        repository_index: u32,
        worker_job_index: u32,
        payload: &JsonValue,
    ) -> StoreResult<JsonValue> {
        let request: ClaimRequest = decode_request(payload, "Worker Job claim")?;
        require_native_v3(&request.accepted_runtime_contracts)?;
        let now = now_s_store()?;
        self.reconcile(now)?;
        let repository = self.repository(repository_index)?;
        let entry = repository.jobs.get(worker_job_index)?;
        require_external_runner_kind(entry.record.job_kind)?;
        let runtime_request = self.runtime_request(&repository, entry)?;
        let grant = self.leases.claim(
            &repository.jobs,
            worker_job_index,
            now,
            self.lease_duration_s,
        )?;
        Ok(claim_response(&repository, grant, runtime_request, "claim"))
    }

    pub(crate) fn claim_next_worker_job(&self, payload: &JsonValue) -> StoreResult<JsonValue> {
        let request: ClaimNextRequest = decode_request(payload, "Worker Job claim-next")?;
        require_native_v3(&request.accepted_runtime_contracts)?;
        let accepted_kinds = validate_claim_kinds(&request.accepted_job_kinds)?;
        let repositories = self.repositories_snapshot()?;
        let repository_filter =
            validate_repository_filter(&request.repository_indexes, &repositories)?;
        let now = now_s_store()?;
        self.reconcile(now)?;
        let candidates = merge_ready_candidates(
            repositories
                .values()
                .filter(|repository| {
                    repository.entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
                        && repository_filter.as_ref().is_none_or(|filter| {
                            filter.contains(&repository.entry.repository_index)
                        })
                })
                .map(|repository| repository.jobs.ready_candidates(now, MAX_CLAIM_CANDIDATES))
                .collect::<StoreResult<Vec<_>>>()?
                .into_iter()
                .flatten()
                .filter(|entry| accepted_kinds.contains(&entry.record.job_kind)),
            MAX_CLAIM_CANDIDATES,
        );
        for entry in candidates {
            let repository = self.repository(entry.key.repository_index)?;
            let runtime_request = self.runtime_request(&repository, entry)?;
            match self.leases.claim(
                &repository.jobs,
                entry.key.worker_job_index,
                now,
                self.lease_duration_s,
            ) {
                Ok(grant) => {
                    return Ok(claim_response(&repository, grant, runtime_request, "claim"))
                }
                Err(error)
                    if error.kind() == BinaryDbErrorKind::InvalidDomainData
                        && (error.contains("queued")
                            || error.contains("available")
                            || error.contains("attempt")) =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(json!({
            "contract": WORKER_JOB_SERVICE_CONTRACT,
            "operation": "claim",
            "claimed_job": JsonValue::Null,
        }))
    }

    pub(crate) fn heartbeat_worker_job(
        &self,
        repository_index: u32,
        worker_job_index: u32,
        payload: &JsonValue,
    ) -> StoreResult<JsonValue> {
        let request: LeaseRequest = decode_request(payload, "Worker Job heartbeat")?;
        if request.detail.is_some() {
            return Err(invalid("Worker Job heartbeat does not accept detail"));
        }
        let token = RuntimeLeaseToken::parse_hex(&request.lease_token)?;
        let repository = self.repository(repository_index)?;
        let grant = self.leases.heartbeat(
            &repository.jobs,
            worker_job_index,
            request.attempt_count,
            token,
            now_s_store()?,
            self.lease_duration_s,
        )?;
        Ok(lease_response("heartbeat", grant.worker_job))
    }

    pub(crate) fn complete_worker_job(
        &self,
        repository_index: u32,
        worker_job_index: u32,
        payload: &JsonValue,
    ) -> StoreResult<JsonValue> {
        let request: LeaseRequest = decode_request(payload, "Worker Job completion")?;
        let detail = request
            .detail
            .as_ref()
            .ok_or_else(|| invalid("Worker Job completion requires detail"))?;
        let token = RuntimeLeaseToken::parse_hex(&request.lease_token)?;
        let repository = self.repository(repository_index)?;
        let current = repository.jobs.get(worker_job_index)?;
        require_external_runner_kind(current.record.job_kind)?;
        require_detail_job_kind(detail, current.record.job_kind)?;
        let now = now_s_store()?;
        let completed = match current.record.job_kind {
            WORKER_JOB_KIND_PATCHSET_CI => {
                let patchset_index = minus_one_required(
                    current.record.patchset_index_plus1,
                    "patchset.ci Patchset",
                )?;
                let closure = repository.authority.patchset_closure(patchset_index)?;
                let evidence = compact_native_result(detail, closure.ci_run_seq, now)?;
                self.leases
                    .complete_with_domain_commit(
                        &repository.jobs,
                        worker_job_index,
                        request.attempt_count,
                        token,
                        WORKER_JOB_OUTCOME_COMPLETED,
                        now,
                        |_| {
                            repository.authority.commit_patchset_ci_evidence(
                                patchset_index,
                                worker_job_index,
                                evidence,
                            )
                        },
                    )?
                    .0
            }
            WORKER_JOB_KIND_REPO_CI => self.leases.complete_after_domain_commit(
                &repository.jobs,
                worker_job_index,
                request.attempt_count,
                token,
                WORKER_JOB_OUTCOME_COMPLETED,
                now,
            )?,
            _ => unreachable!("external runner kind was validated"),
        };
        Ok(lease_response("complete", completed))
    }

    pub(crate) fn fail_worker_job(
        &self,
        repository_index: u32,
        worker_job_index: u32,
        payload: &JsonValue,
    ) -> StoreResult<JsonValue> {
        let request: LeaseRequest = decode_request(payload, "Worker Job failure")?;
        let detail = request
            .detail
            .as_ref()
            .ok_or_else(|| invalid("Worker Job failure requires detail"))?;
        let retryable = detail
            .get("retryable")
            .and_then(JsonValue::as_bool)
            .ok_or_else(|| invalid("Worker Job failure detail requires boolean retryable"))?;
        let error = detail
            .get("error")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("Worker Job failure detail requires error text"))?;
        if error.chars().count() > MAX_FAILURE_DETAIL_CHARS {
            return Err(invalid(format!(
                "Worker Job failure detail exceeds {MAX_FAILURE_DETAIL_CHARS} characters"
            )));
        }
        let token = RuntimeLeaseToken::parse_hex(&request.lease_token)?;
        let repository = self.repository(repository_index)?;
        let current = repository.jobs.get(worker_job_index)?;
        require_external_runner_kind(current.record.job_kind)?;
        let now = now_s_store()?;
        let can_retry = retryable && current.record.attempt_count < current.record.max_attempts;
        let retry_available_at_s = if can_retry {
            Some(
                now.checked_add(u64::from(self.retry_delay_s))
                    .ok_or_else(|| invalid("Worker Job retry time exceeds u64"))?,
            )
        } else {
            None
        };
        let error_kind = if can_retry {
            WORKER_JOB_ERROR_RETRYABLE_EXECUTION
        } else {
            WORKER_JOB_ERROR_TERMINAL_EXECUTION
        };
        let failed = self.leases.fail_attempt(
            &repository.jobs,
            worker_job_index,
            request.attempt_count,
            token,
            error_kind,
            retry_available_at_s,
            now,
        )?;
        Ok(lease_response("fail", failed))
    }

    fn repository(&self, repository_index: u32) -> StoreResult<OperationalRepositoryRuntime> {
        let repository = self.repository_any(repository_index)?;
        if repository.entry.record.lifecycle_kind == REPOSITORY_LIFECYCLE_PURGED {
            return Err(BinaryDbError::missing_data(format!(
                "Binary Repository index {repository_index} is purged"
            )));
        }
        Ok(repository)
    }

    fn repository_any(&self, repository_index: u32) -> StoreResult<OperationalRepositoryRuntime> {
        self.repositories
            .read()
            .map_err(|_| BinaryDbError::other("Repository runtime lock is poisoned"))?
            .get(&repository_index)
            .cloned()
            .ok_or_else(|| {
                BinaryDbError::missing_data(format!(
                    "unknown Binary Repository index {repository_index}"
                ))
            })
    }

    fn update_repository_entry(&self, entry: OperationalRepositoryEntry) -> StoreResult<()> {
        let mut repositories = self
            .repositories
            .write()
            .map_err(|_| BinaryDbError::other("Repository runtime lock is poisoned"))?;
        let repository = repositories
            .get_mut(&entry.repository_index)
            .ok_or_else(|| {
                BinaryDbError::missing_data(format!(
                    "unknown Binary Repository index {}",
                    entry.repository_index
                ))
            })?;
        repository.entry = entry;
        Ok(())
    }

    fn retirement_projection(&self, entry: OperationalRepositoryEntry) -> StoreResult<JsonValue> {
        let repository = self.repository_any(entry.repository_index)?;
        let blockers = repository.jobs.retirement_blockers()?;
        let manifest = if blockers.is_drained() {
            serde_json::to_value(self.retirement_manifest(&repository, &entry, true)?)
                .map_err(|error| BinaryDbError::other(format!("encode export manifest: {error}")))?
        } else {
            JsonValue::Null
        };
        Ok(json!({
            "contract": "ait.server.repository-retirement.v1",
            "repository": repository_projection(&entry),
            "drain": {
                "queued_worker_jobs": blockers.queued,
                "running_worker_jobs": blockers.running,
            },
            "ready_for_export": blockers.is_drained(),
            "manifest": manifest,
        }))
    }

    fn retirement_manifest(
        &self,
        repository: &OperationalRepositoryRuntime,
        entry: &OperationalRepositoryEntry,
        use_cache: bool,
    ) -> StoreResult<RemoteExportManifest> {
        if use_cache {
            if let Some(manifest) = self
                .retirement_exports
                .lock()
                .map_err(|_| BinaryDbError::other("Repository export cache lock is poisoned"))?
                .get(&entry.repository_index)
                .cloned()
            {
                return Ok(manifest);
            }
        }
        if entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_RETIRING {
            return Err(invalid("only a retiring Repository can be exported"));
        }
        if !repository.jobs.retirement_blockers()?.is_drained() {
            return Err(invalid("Repository Worker Jobs must drain before export"));
        }
        let read = BinaryDbReadTxn::new(repository.authority.db());
        read.read_lock_paths()?;
        let manifest = RemoteExportManifest {
            schema: REMOTE_EXPORT_SCHEMA.to_string(),
            state: REMOTE_EXPORT_STATE_COMPLETE.to_string(),
            repo_name: entry.repo_name.clone(),
            namespace: namespace_text(entry.record.namespace_ascii),
            exported_at_s: entry.record.updated_at_s,
            files: collect_export_files(repository.jobs.authority_root())?,
        };
        manifest.validate()?;
        if use_cache {
            self.retirement_exports
                .lock()
                .map_err(|_| BinaryDbError::other("Repository export cache lock is poisoned"))?
                .insert(entry.repository_index, manifest.clone());
        }
        Ok(manifest)
    }

    fn repositories_snapshot(&self) -> StoreResult<BTreeMap<u32, OperationalRepositoryRuntime>> {
        self.repositories
            .read()
            .map(|repositories| repositories.clone())
            .map_err(|_| BinaryDbError::other("Repository runtime lock is poisoned"))
    }

    fn ensure_repository_runtime(&self, entry: OperationalRepositoryEntry) -> StoreResult<()> {
        if self
            .repositories
            .read()
            .map_err(|_| BinaryDbError::other("Repository runtime lock is poisoned"))?
            .contains_key(&entry.repository_index)
        {
            return Ok(());
        }
        let repository =
            open_repository_runtime(&self.registry, entry).map_err(BinaryDbError::other)?;
        self.repositories
            .write()
            .map_err(|_| BinaryDbError::other("Repository runtime lock is poisoned"))?
            .entry(repository.entry.repository_index)
            .or_insert(repository);
        Ok(())
    }

    fn stores(&self) -> Vec<ServerOperationalWorkerJobStore> {
        self.repositories
            .read()
            .expect("Repository runtime lock is poisoned")
            .values()
            .filter(|repository| {
                repository.entry.record.lifecycle_kind != REPOSITORY_LIFECYCLE_PURGED
            })
            .map(|repository| repository.jobs.clone())
            .collect()
    }

    fn reconcile(&self, now_s: u64) -> StoreResult<()> {
        self.leases
            .reconcile(&self.stores(), now_s, self.retry_delay_s)
            .map(|_| ())
    }

    fn runtime_request(
        &self,
        repository: &OperationalRepositoryRuntime,
        entry: WorkerJobEntry,
    ) -> StoreResult<JsonValue> {
        let (run_mode, snapshot_index) = match entry.record.job_kind {
            WORKER_JOB_KIND_PATCHSET_CI => {
                let patchset_index =
                    minus_one_required(entry.record.patchset_index_plus1, "patchset.ci Patchset")?;
                let patchset = repository.authority.patchset_closure(patchset_index)?;
                ("patchset", patchset.revision_snapshot_index)
            }
            WORKER_JOB_KIND_REPO_CI => (
                "repo",
                minus_one_required(
                    entry.record.snapshot_index_plus1,
                    "repo.ci selected Snapshot",
                )?,
            ),
            other => {
                return Err(invalid(format!(
                    "Worker Job kind {other} is not executable by the external native runner"
                )))
            }
        };
        let snapshot_id = repository.authority.snapshot_id_at(snapshot_index)?;
        let external_repository_indexes = fixed_external_repository_indexes();
        Ok(json!({
            "contract": NATIVE_JOB_V3_CONTRACT,
            "label": format!(
                "{}/{}",
                entry.key.repository_index, entry.key.worker_job_index
            ),
            "source": {
                "kind": "remote_snapshot",
                "repository_index": entry.key.repository_index,
                "repository_name": repository.entry.repo_name,
                "snapshot_id": snapshot_id,
                "external_repository_indexes": external_repository_indexes,
            },
            "command": {
                "argv": [NATIVE_JOB_REPOSITORY_CI_ARGV0, run_mode],
            },
            "timeout_ms": DEFAULT_NATIVE_TIMEOUT_MS,
        }))
    }

    fn job_projection(
        &self,
        repository: &OperationalRepositoryRuntime,
        entry: WorkerJobEntry,
    ) -> StoreResult<JsonValue> {
        let kind = WorkerJobKind::try_from(entry.record.job_kind)?;
        let state = worker_job_state_name(entry.record.state_kind);
        let mut projection = json!({
            "repository_index": entry.key.repository_index,
            "worker_job_index": entry.key.worker_job_index,
            "job_kind": entry.record.job_kind,
            "job_type": kind.as_str(),
            "state_kind": entry.record.state_kind,
            "state": state,
            "diagnostic_status": state,
            "retry_pending": entry.record.state_kind == WORKER_JOB_STATE_QUEUED
                && entry.record.attempt_count > 0,
            "outcome_kind": entry.record.outcome_kind,
            "attempt_count": entry.record.attempt_count,
            "max_attempts": entry.record.max_attempts,
            "error_kind": entry.record.error_kind,
            "patchset_index": minus_one(entry.record.patchset_index_plus1),
            "snapshot_index": minus_one(entry.record.snapshot_index_plus1),
            "available_at_s": entry.record.available_at_s,
            "locked_at_s": entry.record.locked_at_s,
            "created_at_s": entry.record.created_at_s,
            "updated_at_s": entry.record.updated_at_s,
            "tombstoned": entry.record.is_tombstoned(),
        });
        if entry.record.job_kind == WORKER_JOB_KIND_PATCHSET_CI {
            let patchset_index =
                minus_one_required(entry.record.patchset_index_plus1, "patchset.ci Patchset")?;
            let patchset = repository.authority.frozen_patchset_at(patchset_index)?;
            projection["patchset_ci"] = json!({
                "patchset_index": patchset_index,
                "ci_worker_job_index": minus_one(patchset.ci_worker_job_index_plus1),
                "ci_run_seq": patchset.ci_run_seq,
                "ci_completed_at_s": patchset.ci_completed_at_s,
                "selected_suite_count": patchset.ci_selected_suite_count,
                "suite_result_count": patchset.ci_suite_result_count,
                "blocking_failure_count": patchset.ci_blocking_failure_count,
                "overall_status": ci_status_name(patchset.ci_status_bits & 0b11),
                "tests_status": ci_status_name((patchset.ci_status_bits >> 2) & 0b11),
                "lint_status": ci_status_name((patchset.ci_status_bits >> 4) & 0b11),
            });
        }
        Ok(projection)
    }
}

fn fixed_external_repository_indexes() -> BTreeMap<String, u32> {
    FIXED_REPOSITORY_NAMES
        .iter()
        .enumerate()
        .map(|(repository_index, repository_name)| {
            (
                (*repository_name).to_string(),
                u32::try_from(repository_index)
                    .expect("the four fixed Repository indexes always fit u32"),
            )
        })
        .collect()
}

fn normalize_restored_authority(
    authority_root: &Path,
    repository_index: u32,
    repository_name: &str,
    namespace_ascii: [u8; 2],
    updated_at_s: u64,
) -> StoreResult<()> {
    let db = OperationalDb::serving_authority(
        RepoId::new(repository_index.to_string()),
        RepoName::new(repository_name.to_string()),
        StorePath::new(authority_root.to_path_buf()),
        StoreGeneration::new(1),
    );
    validate_frozen_server_workflow_v0(&db)?;
    validate_server_snapshot_dag_v0(&db)?;
    validate_server_tree_serving_authority_v0(&db)?;
    let authority = Arc::new(FrozenAuthority::new(db.clone()));
    let domain: Arc<dyn WorkerJobDomainAuthority> = authority.clone();
    let jobs = ServerOperationalWorkerJobStore::new(
        repository_index,
        authority_root.to_path_buf(),
        domain,
    )?;
    jobs.validate()?;

    authority.clear_all_patchset_ci_job_locators()?;
    jobs.tombstone_all(updated_at_s)?;

    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(
        db.clone(),
        &namespace_text(namespace_ascii),
    )
    .map_err(BinaryDbError::other)?;
    workflow.repair_frozen_patchsets_for_activation(&BTreeMap::new())?;
    validate_frozen_server_workflow_v0(&db)?;
    validate_server_snapshot_dag_v0(&db)?;
    validate_server_tree_serving_authority_v0(&db)?;
    jobs.validate()?;
    Ok(())
}

fn open_repository_runtime(
    registry: &ServerOperationalRepositoryRegistry,
    entry: OperationalRepositoryEntry,
) -> Result<OperationalRepositoryRuntime, String> {
    let authority_root = registry
        .resolve_authority_directory(entry.repository_index)
        .map_err(|error| {
            format!(
                "resolve Binary Repository {} authority: {error}",
                entry.repository_index
            )
        })?;
    let db = OperationalDb::serving_authority(
        RepoId::new(entry.repository_index.to_string()),
        RepoName::new(entry.repo_name.clone()),
        StorePath::new(authority_root.clone()),
        StoreGeneration::new(1),
    );
    let native = BinaryDbNativeRepositoryService::new(db.clone()).with_default_line("main");
    native.ensure_default_line().map_err(|error| {
        format!(
            "ensure default main Line for Binary Repository {}: {error}",
            entry.repository_index
        )
    })?;
    let abandoned_uploads = native
        .cleanup_abandoned_zstd_pack_uploads()
        .map_err(|error| {
            format!(
                "clean abandoned zstd Pack uploads for Repository {}: {error}",
                entry.repository_index
            )
        })?;
    if abandoned_uploads != 0 {
        eprintln!(
            "removed {abandoned_uploads} abandoned zstd Pack upload(s) for Repository {}",
            entry.repository_index
        );
    }
    let authority = Arc::new(FrozenAuthority::new(db.clone()));
    let domain: Arc<dyn WorkerJobDomainAuthority> = authority.clone();
    let jobs = ServerOperationalWorkerJobStore::new(entry.repository_index, authority_root, domain)
        .map_err(|error| {
            format!(
                "open Binary Worker Job store for Repository {}: {error}",
                entry.repository_index
            )
        })?;
    let ci_job_locators = jobs
        .patchset_ci_locators_for_activation_repair()
        .map_err(|error| {
            format!(
                "inspect Binary Patchset CI Job locators for Repository {}: {error}",
                entry.repository_index
            )
        })?;
    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(
        db.clone(),
        &namespace_text(entry.record.namespace_ascii),
    )?;
    workflow
        .repair_frozen_patchsets_for_activation(&ci_job_locators)
        .map_err(|error| {
            format!(
                "repair frozen Binary Patchsets for Repository {}: {error}",
                entry.repository_index
            )
        })?;
    validate_frozen_server_workflow_v0(&db).map_err(|error| {
        format!(
            "validate frozen Binary workflow for Repository {}: {error}",
            entry.repository_index
        )
    })?;
    jobs.recover().map_err(|error| {
        format!(
            "recover Binary Worker Job store for Repository {}: {error}",
            entry.repository_index
        )
    })?;
    Ok(OperationalRepositoryRuntime {
        entry,
        authority,
        jobs,
        native,
    })
}

fn repository_registration_projection(
    entry: OperationalRepositoryEntry,
    created: bool,
) -> JsonValue {
    json!({
        "contract": "ait.server.repository-registration.v1",
        "created": created,
        "repository": repository_projection(&entry),
    })
}

fn repository_purge_projection(
    entry: OperationalRepositoryEntry,
    already_purged: bool,
) -> JsonValue {
    json!({
        "contract": "ait.server.repository-purge.v1",
        "purged": true,
        "already_purged": already_purged,
        "repository": repository_projection(&entry),
    })
}

fn repository_retirement_abort_projection(
    entry: OperationalRepositoryEntry,
    already_aborted: bool,
) -> JsonValue {
    json!({
        "contract": "ait.server.repository-retirement-abort.v1",
        "aborted": true,
        "already_aborted": already_aborted,
        "repository": repository_projection(&entry),
    })
}

fn repository_restore_abort_projection(
    restore_token: &str,
    state: &str,
    aborted: bool,
    already_terminal: bool,
) -> JsonValue {
    json!({
        "contract": "ait.server.repository-restore-abort.v1",
        "restore_token": restore_token,
        "state": state,
        "aborted": aborted,
        "already_terminal": already_terminal,
    })
}

fn repository_projection(entry: &OperationalRepositoryEntry) -> JsonValue {
    json!({
        "repository_index": entry.repository_index,
        "repository_name": entry.repo_name,
        "lifecycle_kind": entry.record.lifecycle_kind,
        "namespace": namespace_text(entry.record.namespace_ascii),
        "policy_flags": entry.record.policy_flags,
        "created_at_s": entry.record.created_at_s,
        "updated_at_s": entry.record.updated_at_s,
        "tombstoned": entry.record.is_tombstoned(),
    })
}

fn random_restore_token() -> StoreResult<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        BinaryDbError::other(format!("generate restore session token: {error}"))
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn require_restore_token(token: &str) -> StoreResult<()> {
    if token.len() == 32
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(invalid("Repository restore session token is invalid"))
    }
}

fn restore_manifest_bytes(manifest: &RemoteExportManifest) -> StoreResult<u64> {
    manifest.files.iter().try_fold(0_u64, |total, file| {
        total
            .checked_add(file.size)
            .ok_or_else(|| invalid("Repository restore manifest byte total overflows u64"))
    })
}

fn acquire_restore_staging_lock(staging_parent: &Path) -> Result<File, String> {
    let lock_path = staging_parent.join(RESTORE_STAGING_LOCK_FILE);
    if let Ok(metadata) = fs::symlink_metadata(&lock_path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "Repository restore staging lock is not a regular file: {}",
                lock_path.display()
            ));
        }
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| {
            format!(
                "open Repository restore staging lock {}: {error}",
                lock_path.display()
            )
        })?;
    FileExt::try_lock_exclusive(&lock).map_err(|error| {
        format!(
            "acquire exclusive Repository restore staging lock {}: {error}",
            lock_path.display()
        )
    })?;
    lock.sync_all().map_err(|error| {
        format!(
            "sync Repository restore staging lock {}: {error}",
            lock_path.display()
        )
    })?;
    sync_filesystem_directory(staging_parent).map_err(|error| {
        format!(
            "sync Repository restore staging directory {}: {error}",
            staging_parent.display()
        )
    })?;
    Ok(lock)
}

fn cleanup_abandoned_restore_staging(staging_parent: &Path) -> Result<(), String> {
    for entry in fs::read_dir(staging_parent).map_err(|error| {
        format!(
            "read Repository restore staging directory {}: {error}",
            staging_parent.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "read Repository restore staging entry in {}: {error}",
                staging_parent.display()
            )
        })?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Repository restore staging contains a non-UTF-8 entry".to_string())?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "inspect Repository restore staging {}: {error}",
                path.display()
            )
        })?;
        if name == RESTORE_STAGING_LOCK_FILE {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "Repository restore staging lock is not a regular file: {}",
                    path.display()
                ));
            }
            continue;
        }
        require_restore_token(&name).map_err(|error| {
            format!(
                "Repository restore staging contains an unmanaged entry {}: {error}",
                path.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "Repository restore staging session is not a real directory: {}",
                path.display()
            ));
        }
        fs::remove_dir_all(&path).map_err(|error| {
            format!(
                "remove abandoned Repository restore staging {}: {error}",
                path.display()
            )
        })?;
    }
    sync_filesystem_directory(staging_parent).map_err(|error| {
        format!(
            "sync cleaned Repository restore staging directory {}: {error}",
            staging_parent.display()
        )
    })
}

fn remove_restore_session_directory(staging_parent: &Path, session_root: &Path) -> StoreResult<()> {
    if session_root.parent() != Some(staging_parent) {
        return Err(corrupt(format!(
            "Repository restore session escaped its staging parent: {}",
            session_root.display()
        )));
    }
    let token = session_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| corrupt("Repository restore session path has no UTF-8 token"))?;
    require_restore_token(token)?;
    match fs::symlink_metadata(session_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(corrupt(format!(
                    "Repository restore session is not a real directory: {}",
                    session_root.display()
                )));
            }
            fs::remove_dir_all(session_root).map_err(|error| {
                BinaryDbError::io(
                    format!(
                        "remove Repository restore staging {}",
                        session_root.display()
                    ),
                    error,
                )
            })?;
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(BinaryDbError::io(
                format!(
                    "inspect Repository restore staging {}",
                    session_root.display()
                ),
                error,
            ));
        }
    }
    sync_filesystem_directory(staging_parent).map_err(|error| {
        BinaryDbError::io(
            format!(
                "sync Repository restore staging directory {}",
                staging_parent.display()
            ),
            error,
        )
    })
}

fn claim_response(
    repository: &OperationalRepositoryRuntime,
    grant: RuntimeLeaseGrant,
    runtime_request: JsonValue,
    operation: &str,
) -> JsonValue {
    let job_kind = grant.worker_job.record.job_kind;
    let job_type = WorkerJobKind::try_from(job_kind)
        .expect("claimed fixed Worker Job kind was validated")
        .as_str();
    json!({
        "contract": WORKER_JOB_SERVICE_CONTRACT,
        "operation": operation,
        "claimed_job": {
            "repository_index": grant.worker_job.key.repository_index,
            "worker_job_index": grant.worker_job.key.worker_job_index,
            "attempt_count": grant.attempt_count,
            "lease_token": grant.lease_token.to_hex(),
            "lease_expires_at_s": grant.expires_at_s,
            "job_kind": job_kind,
            "job_type": job_type,
            "state_kind": grant.worker_job.record.state_kind,
            "runtime_request": runtime_request,
        },
        "repository": {
            "repository_index": repository.entry.repository_index,
            "repository_name": repository.entry.repo_name,
        },
    })
}

fn lease_response(operation: &str, entry: WorkerJobEntry) -> JsonValue {
    json!({
        "contract": WORKER_JOB_SERVICE_CONTRACT,
        "operation": operation,
        "job": {
            "repository_index": entry.key.repository_index,
            "worker_job_index": entry.key.worker_job_index,
            "attempt_count": entry.record.attempt_count,
            "state_kind": entry.record.state_kind,
            "outcome_kind": entry.record.outcome_kind,
            "error_kind": entry.record.error_kind,
            "available_at_s": entry.record.available_at_s,
            "updated_at_s": entry.record.updated_at_s,
        },
    })
}

fn compact_native_result(
    detail: &JsonValue,
    ci_run_seq: u32,
    completed_at_s: u64,
) -> StoreResult<WorkerJobCompactCiEvidence> {
    let result = detail
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| invalid("patchset.ci completion requires object result"))?;
    if result.get("contract").and_then(JsonValue::as_str) != Some(NATIVE_RESULT_CONTRACT) {
        return Err(invalid(
            "patchset.ci completion result contract is unsupported",
        ));
    }
    let terminal = result
        .get("status")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid("patchset.ci completion result requires status"))?;
    let tests = result
        .get("tests_status")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| invalid("patchset.ci completion result requires tests_status"))?;
    let suite_results = result
        .get("suite_results")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| invalid("patchset.ci completion result requires suite_results"))?;
    let declared_count = result
        .get("suite_result_count")
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| invalid("patchset.ci completion result requires suite_result_count"))?;
    if usize::try_from(declared_count).ok() != Some(suite_results.len()) {
        return Err(invalid(
            "patchset.ci completion suite_result_count disagrees with suite_results",
        ));
    }
    let suite_result_count = u16::try_from(suite_results.len())
        .map_err(|_| invalid("patchset.ci completion has more than u16 suite results"))?;
    let mut blocking_failure_count = 0_u16;
    let mut lint_status = CI_STATUS_NONE;
    for suite in suite_results {
        let status = match suite.get("status").and_then(JsonValue::as_str) {
            Some("pass") => CI_STATUS_PASS,
            Some("fail") => CI_STATUS_FAIL,
            _ => return Err(invalid("patchset.ci suite status must be pass or fail")),
        };
        if suite.get("blocking").and_then(JsonValue::as_bool) == Some(true)
            && status != CI_STATUS_PASS
        {
            blocking_failure_count = blocking_failure_count
                .checked_add(1)
                .ok_or_else(|| invalid("patchset.ci blocking failure count exceeds u16"))?;
        }
        let suite_id = suite
            .get("suite_id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(suite_id.as_str(), "cargo_fmt" | "rustfmt") {
            lint_status = match (lint_status, status) {
                (CI_STATUS_FAIL, _) | (_, CI_STATUS_FAIL) => CI_STATUS_FAIL,
                _ => CI_STATUS_PASS,
            };
        }
    }
    let tests_status = match tests {
        "pass" => CI_STATUS_PASS,
        "fail" => CI_STATUS_FAIL,
        _ => return Err(invalid("patchset.ci tests_status must be pass or fail")),
    };
    let overall_status = match terminal {
        "succeeded" if tests_status == CI_STATUS_PASS && blocking_failure_count == 0 => {
            CI_STATUS_PASS
        }
        "succeeded" | "command_failed" => CI_STATUS_FAIL,
        "timed_out" => CI_STATUS_ERROR,
        _ => return Err(invalid("patchset.ci terminal status is invalid")),
    };
    Ok(WorkerJobCompactCiEvidence {
        ci_completed_at_s: completed_at_s,
        ci_run_seq,
        selected_suite_count: suite_result_count,
        suite_result_count,
        blocking_failure_count,
        overall_status,
        tests_status,
        lint_status,
    })
}

fn require_detail_job_kind(detail: &JsonValue, expected: u8) -> StoreResult<()> {
    let actual = detail
        .get("job_kind")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or_else(|| invalid("Worker Job completion detail requires job_kind"))?;
    if actual != expected {
        return Err(invalid(
            "Worker Job completion detail job_kind disagrees with fixed authority",
        ));
    }
    Ok(())
}

fn validate_claim_kinds(values: &[u8]) -> StoreResult<BTreeSet<u8>> {
    let kinds = values.iter().copied().collect::<BTreeSet<_>>();
    if kinds.len() != values.len() {
        return Err(invalid(
            "Worker Job claim accepted_job_kinds contains duplicates",
        ));
    }
    for kind in &kinds {
        require_external_runner_kind(*kind)?;
    }
    Ok(kinds)
}

fn validate_repository_filter(
    values: &[u32],
    repositories: &BTreeMap<u32, OperationalRepositoryRuntime>,
) -> StoreResult<Option<BTreeSet<u32>>> {
    if values.is_empty() {
        return Ok(None);
    }
    let filter = values.iter().copied().collect::<BTreeSet<_>>();
    if filter.len() != values.len() {
        return Err(invalid(
            "Worker Job claim repository_indexes contains duplicates",
        ));
    }
    if let Some(index) = filter
        .iter()
        .find(|index| !repositories.contains_key(index))
    {
        return Err(BinaryDbError::missing_data(format!(
            "unknown Binary Repository index {index}"
        )));
    }
    Ok(Some(filter))
}

fn require_external_runner_kind(kind: u8) -> StoreResult<()> {
    if matches!(kind, WORKER_JOB_KIND_PATCHSET_CI | WORKER_JOB_KIND_REPO_CI) {
        Ok(())
    } else {
        Err(invalid(format!(
            "Worker Job kind {kind} is not assigned to the external native runner"
        )))
    }
}

fn require_native_v3(contracts: &[String]) -> StoreResult<()> {
    if contracts
        .iter()
        .any(|value| value == NATIVE_JOB_V3_CONTRACT)
    {
        Ok(())
    } else {
        Err(invalid(format!(
            "Worker Job claim requires accepted runtime contract {NATIVE_JOB_V3_CONTRACT}"
        )))
    }
}

fn validate_state_filter(state_kind: Option<u8>) -> StoreResult<()> {
    if state_kind.is_none_or(|state| (1..=4).contains(&state)) {
        Ok(())
    } else {
        Err(invalid("Worker Job state_kind filter must be 1 through 4"))
    }
}

fn decode_request<T>(payload: &JsonValue, label: &str) -> StoreResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(payload.clone())
        .map_err(|error| invalid(format!("{label} payload is invalid: {error}")))
}

fn minus_one(value: u32) -> Option<u32> {
    value.checked_sub(1)
}

fn minus_one_required(value: u32, label: &str) -> StoreResult<u32> {
    minus_one(value).ok_or_else(|| invalid(format!("{label} reference is missing")))
}

fn namespace_text(namespace: [u8; 2]) -> String {
    namespace
        .into_iter()
        .take_while(|byte| *byte != 0)
        .map(char::from)
        .collect()
}

fn ci_status_name(status: u8) -> &'static str {
    match status {
        CI_STATUS_PASS => "pass",
        CI_STATUS_FAIL => "fail",
        CI_STATUS_ERROR => "error",
        _ => "none",
    }
}

fn worker_job_state_name(state: u8) -> &'static str {
    match state {
        WORKER_JOB_STATE_QUEUED => "queued",
        WORKER_JOB_STATE_RUNNING => "running",
        WORKER_JOB_STATE_SUCCEEDED => "succeeded",
        WORKER_JOB_STATE_FAILED => "failed",
        _ => "unknown",
    }
}

fn require_positive_runtime_durations(
    lease_duration_s: u32,
    retry_delay_s: u32,
) -> Result<(), String> {
    if lease_duration_s == 0 || retry_delay_s == 0 {
        return Err("Binary Worker Job lease and retry durations must be non-zero".to_string());
    }
    Ok(())
}

fn now_s() -> Result<u64, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system time precedes the Unix epoch".to_string())?
        .as_secs();
    Ok(seconds)
}

fn now_s_store() -> StoreResult<u64> {
    now_s().map_err(BinaryDbError::other)
}

fn canonical_real_directory(path: &Path) -> Result<PathBuf, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{} is not a real directory", path.display()));
    }
    fs::canonicalize(path)
        .map_err(|error| format!("failed to canonicalize {}: {error}", path.display()))
}

fn read_regular_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("{} is not a real regular file", path.display()));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(format!("{} must have one filesystem link", path.display()));
    }
    fs::read(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

fn validate_completion(
    generation_root: &Path,
    completion_path: &Path,
    completion_bytes: &[u8],
) -> Result<(), String> {
    match completion_path.file_name().and_then(|value| value.to_str()) {
        Some(SERVER_FRESH_COMPLETION_FILE) => {
            let completion: FreshCompletionEvidence = serde_json::from_slice(completion_bytes)
                .map_err(|error| {
                    format!(
                        "failed to parse fresh Binary v0 completion {}: {error}",
                        completion_path.display()
                    )
                })?;
            if completion.schema != FRESH_COMPLETION_SCHEMA
                || completion.layout_id != 1
                || completion.status != "validated_inactive"
                || !is_sha256(&completion.generation_manifest_sha256)
            {
                return Err("fresh Binary v0 completion envelope is invalid".to_string());
            }
            let manifest_bytes = read_regular_file(&generation_root.join("generation.json"))?;
            if sha256(&manifest_bytes) != completion.generation_manifest_sha256 {
                return Err(
                    "fresh Binary v0 generation manifest changed after completion".to_string(),
                );
            }
            let manifest: GenerationManifest =
                serde_json::from_slice(&manifest_bytes).map_err(|error| {
                    format!("failed to parse fresh Binary v0 generation manifest: {error}")
                })?;
            if manifest.schema != GENERATION_SCHEMA
                || manifest.layout_id != 1
                || manifest.status != "validated_inactive"
                || manifest.repository_count != completion.repository_count
            {
                return Err(
                    "fresh Binary v0 completion disagrees with generation manifest".to_string(),
                );
            }
            Ok(())
        }
        Some(SERVER_LEGACY_CONVERSION_COMPLETION_FILE) => {
            let completion: CompletionEvidence =
                serde_json::from_slice(completion_bytes).map_err(|error| {
                    format!(
                        "failed to parse converted Binary v0 completion {}: {error}",
                        completion_path.display()
                    )
                })?;
            let expected_report_schema = match completion.schema.as_str() {
                POSTGRES_CONVERSION_COMPLETION_SCHEMA => POSTGRES_CONVERSION_REPORT_SCHEMA,
                U64_SECOND_UPGRADE_COMPLETION_SCHEMA => U64_SECOND_UPGRADE_REPORT_SCHEMA,
                LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA => LEGACY_PLAN_LINEAGE_REPORT_SCHEMA,
                _ => return Err("converted Binary v0 completion envelope is invalid".to_string()),
            };
            if completion.layout_id != 1
                || completion.status != "validated_inactive"
                || !is_sha256(&completion.report_sha256)
            {
                return Err("converted Binary v0 completion envelope is invalid".to_string());
            }
            let report_bytes = read_regular_file(&generation_root.join("conversion-report.json"))?;
            if sha256(&report_bytes) != completion.report_sha256 {
                return Err("converted Binary v0 report changed after completion".to_string());
            }
            let report: ConversionReportEvidence = serde_json::from_slice(&report_bytes)
                .map_err(|error| format!("failed to parse converted Binary v0 report: {error}"))?;
            let selectors_are_exact = if completion.schema == U64_SECOND_UPGRADE_COMPLETION_SCHEMA {
                report.source_selector.as_deref() == Some(U32_TIME_V0_SELECTOR)
                    && report.target_selector.as_deref() == Some(U64_SECOND_V0_SELECTOR)
            } else {
                true
            };
            if report.schema != expected_report_schema
                || report.layout_id != 1
                || report.status != "validated_inactive"
                || !selectors_are_exact
            {
                return Err("converted Binary v0 report envelope is invalid".to_string());
            }
            if completion.schema == LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA {
                validate_legacy_plan_lineage_runtime_evidence(generation_root, &report)?;
            }
            Ok(())
        }
        _ => Err(format!(
            "unsupported Binary v0 completion evidence file {}",
            completion_path.display()
        )),
    }
}

fn validate_legacy_plan_lineage_runtime_evidence(
    generation_root: &Path,
    report: &ConversionReportEvidence,
) -> Result<(), String> {
    if report.repository_indexes != [0, 1] || report.sealed_files.is_empty() {
        return Err("legacy Plan-lineage report scope is invalid".to_string());
    }
    let expected = expected_legacy_plan_lineage_sealed_paths();
    let mut actual = BTreeSet::new();
    let mut previous: Option<&str> = None;
    for file in &report.sealed_files {
        let relative = Path::new(&file.relative_path);
        if relative.is_absolute()
            || file.relative_path.is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            || previous.is_some_and(|previous| previous.as_bytes() >= file.relative_path.as_bytes())
            || !actual.insert(file.relative_path.clone())
            || !is_sha256(&file.sha256)
        {
            return Err("legacy Plan-lineage sealed file inventory is invalid".to_string());
        }
        previous = Some(&file.relative_path);
        if matches!(
            file.relative_path.as_str(),
            "generation.json" | LEGACY_PLAN_LINEAGE_RECEIPT_FILE
        ) {
            let bytes = read_regular_file(&generation_root.join(relative))?;
            if bytes.len() as u64 != file.byte_size || sha256(&bytes) != file.sha256 {
                return Err(format!(
                    "legacy Plan-lineage immutable evidence changed after completion: {}",
                    file.relative_path
                ));
            }
        }
    }
    if actual != expected {
        return Err(format!(
            "legacy Plan-lineage sealed scope differs: expected={expected:?}, actual={actual:?}"
        ));
    }
    Ok(())
}

fn expected_legacy_plan_lineage_sealed_paths() -> BTreeSet<String> {
    let mut paths = BTreeSet::from([
        "generation.json".to_string(),
        "global/repository.bin".to_string(),
        "global/repository_payload.bin".to_string(),
        LEGACY_PLAN_LINEAGE_RECEIPT_FILE.to_string(),
    ]);
    for repository_index in [0_u32, 1_u32] {
        for name in [
            "plan.bin",
            "plan_payload.bin",
            "plan_revision.bin",
            "plan_revision_payload.bin",
            "plan_item.bin",
            "plan_item_payload.bin",
            "task.bin",
        ] {
            paths.insert(format!("repositories/{repository_index}/{name}"));
        }
    }
    paths
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn invalid(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::invalid_domain_data(message)
}

fn corrupt(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::corruption(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ait_server_core::foundation::pack_substrate::{
        build_pack_members, build_tree_pack_members, read_pack_index, read_tree_pack_index,
        write_pack_archive, write_tree_pack_archive, DEFAULT_MAX_DELTA_CHAIN_DEPTH,
        PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    };
    use ait_server_core::foundation::remote_binary_db::{
        BinaryDbCommandScope, BinaryDbReadTxn, BinaryDbWriteTxn,
    };
    use ait_server_core::foundation::server_content_binary_db::{
        server_snapshot_hash48_from_id, ServerBinaryDbLineStore, ServerBinaryDbSnapshotStore,
        ServerBinarySnapshotPayload, ServerBinarySnapshotRecord, SERVER_CONTENT_BINARY_LAYOUT_ID,
    };
    use ait_server_core::foundation::server_plan_binary_db::BinaryDbServerPlanService;
    use ait_server_core::foundation::server_workflow_store::{
        ServerWorkflowChangeStore, ServerWorkflowPatchsetStore, ServerWorkflowPolicyStore,
        ServerWorkflowTaskStore,
    };
    use ait_server_core::foundation::workflow_binary_v0::WorkflowBinaryV0Codec;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Barrier;

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "ait-server-{label}-{}-{}",
                std::process::id(),
                TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create isolated Binary runtime test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove isolated Binary runtime test directory");
        }
    }

    fn lifecycle_config(directory: &TestDirectory) -> ServerBinaryLifecycleConfig {
        let server_data = directory.0.join("server-data");
        ServerBinaryLifecycleConfig::from_server_data_root(server_data)
            .expect("construct isolated lifecycle config")
    }

    #[test]
    fn production_upgrade_without_activation_fails_before_creating_any_layout() {
        let directory = TestDirectory::new("missing-activation-upgrade");
        let config = lifecycle_config(&directory);

        let error = ensure_runtime_activation(&config).unwrap_err();

        assert!(error.contains("Refusing to create an empty generation"));
        assert!(!config.server_data_root.exists());
        assert!(!config.binary_root.exists());
        assert!(!config.generations_root.exists());
        assert!(!config.activation_pointer.exists());
    }

    #[test]
    fn explicit_installed_initialization_creates_then_reopens_activation() {
        let fresh_directory = TestDirectory::new("explicit-fresh-bootstrap");
        let fresh = lifecycle_config(&fresh_directory);
        let initialized = initialize_installed_runtime(&fresh)
            .expect("explicitly initialize a fresh installation");
        assert!(initialized.created);
        assert!(initialized.generation_root.exists());
        assert!(fresh.activation_pointer.exists());

        let reopened = ensure_runtime_activation(&fresh)
            .expect("existing activation does not require bootstrap admission");
        assert_eq!(reopened.generation_root, initialized.generation_root);
    }

    #[test]
    fn compact_native_result_derives_only_fixed_patchset_ci_evidence() {
        let detail = json!({
            "job_kind": WORKER_JOB_KIND_PATCHSET_CI,
            "result": {
                "contract": NATIVE_RESULT_CONTRACT,
                "status": "succeeded",
                "tests_status": "pass",
                "suite_result_count": 2,
                "suite_results": [
                    {"suite_id": "rust_tests", "status": "pass", "blocking": true},
                    {"suite_id": "cargo_fmt", "status": "pass", "blocking": true}
                ],
                "cleanup": {}
            }
        });
        let evidence = compact_native_result(&detail, 3, 1_786_000_000).unwrap();
        assert_eq!(
            evidence,
            WorkerJobCompactCiEvidence {
                ci_completed_at_s: 1_786_000_000,
                ci_run_seq: 3,
                selected_suite_count: 2,
                suite_result_count: 2,
                blocking_failure_count: 0,
                overall_status: CI_STATUS_PASS,
                tests_status: CI_STATUS_PASS,
                lint_status: CI_STATUS_PASS,
            }
        );
        assert!(!serde_json::to_string(&evidence.ci_run_seq)
            .unwrap()
            .contains("result"));
    }

    #[test]
    fn claim_filters_accept_only_pair_key_native_runner_kinds() {
        assert_eq!(
            validate_claim_kinds(&[WORKER_JOB_KIND_PATCHSET_CI, WORKER_JOB_KIND_REPO_CI]).unwrap(),
            BTreeSet::from([WORKER_JOB_KIND_PATCHSET_CI, WORKER_JOB_KIND_REPO_CI])
        );
        assert!(validate_claim_kinds(&[WORKER_JOB_KIND_PATCHSET_CI; 2]).is_err());
        assert!(validate_claim_kinds(&[5]).is_err());
    }

    #[test]
    fn namespace_projection_preserves_empty_and_one_byte_encodings() {
        assert_eq!(namespace_text([0, 0]), "");
        assert_eq!(namespace_text([b'R', 0]), "R");
        assert_eq!(namespace_text([b'R', b'T']), "RT");
    }

    #[test]
    fn startup_normalizes_active_patchset_before_ci_job_selection() {
        let directory = TestDirectory::new("startup-patchset-repair");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let authority_root = generation.join("repositories").join("0");
        let db = OperationalDb::serving_authority(
            RepoId::new("0"),
            RepoName::new("ait-core"),
            StorePath::new(authority_root),
            StoreGeneration::new(1),
        );
        let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
        let snapshots =
            ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
        let line_index = lines.create_line("main", 0, 1).expect("create main Line");
        let payload = ServerBinarySnapshotPayload {
            line_name: "main".to_string(),
            message: Some("activation repair fixture".to_string()),
        };
        let snapshot_record =
            |snapshot_id: &str, parent_snapshot_index_plus1: u32| ServerBinarySnapshotRecord {
                snapshot_meta: 0,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: server_snapshot_hash48_from_id(snapshot_id)
                    .expect("valid fixture Snapshot id"),
                parent_snapshot_index_plus1,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: line_index + 1,
                manifest_hash: [0; 32],
                file_count: 0,
                total_bytes: 0,
                created_at_s: 1_786_000_000,
            };
        let base = "SNP-0000000000A1";
        let revision = "SNP-0000000000A2";
        let base_index = snapshots
            .append_snapshot(base, snapshot_record(base, 0), &payload)
            .expect("append base Snapshot");
        snapshots
            .append_snapshot(
                revision,
                snapshot_record(revision, base_index + 1),
                &payload,
            )
            .expect("append revision Snapshot");
        lines
            .set_line_head("main", base_index + 1, 1_786_000_001)
            .expect("set main Line head");

        let active = BinaryDbServerWorkflowV0Store::new_remote(db.clone(), "C")
            .expect("construct active workflow adapter");
        active
            .create_task(
                "ait-core",
                &json!({
                    "title": "Post-activation history promotion",
                    "intent": "Exercise active Patchset locator normalization"
                }),
            )
            .expect("create active Task");
        active
            .create_change(
                "ait-core",
                &json!({
                    "task_id": "RCT-0001",
                    "title": "Post-activation aggregate Change",
                    "base_line": "main"
                }),
            )
            .expect("create active Change");
        active
            .publish_patchset(
                "RCT-0001/C-01",
                &json!({
                    "base_snapshot_id": base,
                    "revision_snapshot_id": revision,
                    "summary": "active aggregate Patchset locator fixture",
                    "author_mode": "ai_with_human_review"
                }),
            )
            .expect("publish active Patchset");
        let raw = BinaryDbReadTxn::new(&db)
            .read_record(WorkflowBinaryV0Codec::patchset_file(), 0)
            .expect("read active Patchset");
        let active_patchset = WorkflowBinaryV0Codec::decode_frozen_patchset(&raw)
            .expect("decode the unified active Patchset authority");
        assert_eq!(active_patchset.ci_worker_job_index_plus1, 0);

        let runtime = OperationalBinaryRuntime::open_generation(
            generation,
            directory.0.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("repair and open activated frozen generation");
        let (_, repaired_db) = runtime.repository_db(0).expect("read repaired Repository");
        let frozen = BinaryDbServerWorkflowV0Store::new_remote_frozen(repaired_db.clone(), "C")
            .expect("construct frozen workflow adapter");
        let ci = frozen
            .run_patchset_ci("RCT-0001/C-01/P-01", &json!({"trigger": "manual_rerun"}))
            .expect("start frozen Patchset CI");
        assert_eq!(ci["ci_run_seq"], 1);
        let job = runtime
            .enqueue_patchset_ci(0, 0, false, 3)
            .expect("select repository-local Patchset CI Job");
        assert_eq!(job["repository_index"], 0);
        assert_eq!(job["worker_job_index"], 0);
        let rejected = runtime
            .claim_worker_job(
                0,
                0,
                &json!({"accepted_runtime_contracts": [NATIVE_JOB_V2_CONTRACT]}),
            )
            .expect_err("v2-only runner admission must fail closed");
        assert!(rejected.to_string().contains(NATIVE_JOB_V3_CONTRACT));
        assert_eq!(
            runtime.worker_job(0, 0).expect("read rejected Job state")["job"]["state"],
            json!("queued")
        );

        let claimed = runtime
            .claim_worker_job(
                0,
                0,
                &json!({"accepted_runtime_contracts": [NATIVE_JOB_V3_CONTRACT]}),
            )
            .expect("claim native Job v3 request");
        let request = &claimed["claimed_job"]["runtime_request"];
        assert_eq!(request["contract"], json!(NATIVE_JOB_V3_CONTRACT));
        assert_eq!(
            request["command"]["argv"],
            json!([NATIVE_JOB_REPOSITORY_CI_ARGV0, "patchset"])
        );
        assert_eq!(
            request["source"]["external_repository_indexes"],
            json!({
                "ait-core": 0,
                "ait-server": 1,
                "ait-python": 2,
                "ait-node": 3,
            })
        );
        let repaired = WorkflowBinaryV0Codec::decode_frozen_patchset(
            &BinaryDbReadTxn::new(&repaired_db)
                .read_record(WorkflowBinaryV0Codec::patchset_file(), 0)
                .expect("read frozen Patchset"),
        )
        .expect("decode frozen Patchset");
        assert_eq!(repaired.ci_run_seq, 1);
        assert_eq!(repaired.ci_worker_job_index_plus1, 1);
    }

    #[test]
    fn repository_authority_projects_committed_pack_inventory() {
        let directory = TestDirectory::new("repository-pack-inventory");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let runtime = OperationalBinaryRuntime::open_generation(
            generation,
            directory.0.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("open frozen Binary generation");
        let (_, db) = runtime.repository_db(0).expect("read Repository DB");
        let content = ServerBinaryRepositoryContentStore::new(db.clone());

        let created_at = "2026-08-17T00:00:00Z";
        let object_pack_id = "PCK-000000000001";
        let object_pack_path = content.object_pack_path(object_pack_id);
        fs::create_dir_all(object_pack_path.parent().expect("object Pack parent"))
            .expect("create object Pack directory");
        let bytes = b"pack inventory fixture\n";
        let blob_sha256 = sha256(bytes);
        let blob_id = format!("BLB-{}", &blob_sha256[..20]);
        let object_members = build_pack_members(
            &json!([{
                "entry_name": format!("blobs/{blob_id}"),
                "blob_id": blob_id,
                "data": bytes,
                "path_hint": "README.md"
            }]),
            DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            None,
        )
        .expect("build object Pack members");
        write_pack_archive(
            object_pack_path.to_str().expect("UTF-8 object Pack path"),
            object_pack_id,
            created_at,
            &object_members,
        )
        .expect("write object Pack");
        let object_index =
            read_pack_index(object_pack_path.to_str().expect("UTF-8 object Pack path"))
                .expect("read object Pack index");

        let tree_pack_id = "TPK-000000000002";
        let tree_id = "TRE-00000000000000000002";
        let tree_pack_path = content.tree_pack_path(tree_pack_id);
        fs::create_dir_all(tree_pack_path.parent().expect("Tree Pack parent"))
            .expect("create Tree Pack directory");
        let tree_members = build_tree_pack_members(
            &json!([{"tree_id": tree_id, "entry_count": 1}]),
            &json!([{
                "tree_id": tree_id,
                "entry_name": "README.md",
                "entry_type": "blob",
                "target_id": blob_id,
                "size_bytes": bytes.len(),
                "mode": "0o644"
            }]),
        )
        .expect("build Tree Pack members");
        write_tree_pack_archive(
            tree_pack_path.to_str().expect("UTF-8 Tree Pack path"),
            tree_pack_id,
            created_at,
            &tree_members,
        )
        .expect("write Tree Pack");
        let tree_index =
            read_tree_pack_index(tree_pack_path.to_str().expect("UTF-8 Tree Pack path"))
                .expect("read Tree Pack index");

        let mut tx = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerRemoteSyncCommit)
            .expect("begin Pack inventory fixture transaction");
        content
            .append_object_pack_in_tx(
                &mut tx,
                &json!({
                    "pack_id": object_pack_id,
                    "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
                    "pack_index": object_index,
                    "total_bytes": fs::metadata(&object_pack_path).expect("object Pack metadata").len(),
                    "created_at": created_at
                }),
                &[],
            )
            .expect("append Object Pack authority");
        content
            .append_tree_pack_in_tx(
                &mut tx,
                &json!({
                    "pack_id": tree_pack_id,
                    "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                    "pack_index": tree_index,
                    "total_bytes": fs::metadata(&tree_pack_path).expect("Tree Pack metadata").len(),
                    "created_at": created_at
                }),
                &[],
            )
            .expect("append Tree Pack authority");
        tx.commit().expect("commit Pack inventory fixture");

        let repository = runtime
            .repository_authority(0)
            .expect("project Repository Pack inventory");
        assert_eq!(repository["pack_storage"]["object_pack_count"], 1);
        assert_eq!(repository["pack_storage"]["tree_pack_count"], 1);
        assert_eq!(repository["pack_storage"]["zstd_object_pack_count"], 1);
        assert_eq!(repository["pack_storage"]["zstd_tree_pack_count"], 1);
        assert_eq!(repository["pack_storage"]["zstd_only_verified"], true);
        assert_eq!(repository["pack_storage"]["validation"]["state"], "valid");
    }

    #[test]
    fn live_registration_is_immediate_idempotent_and_restart_safe() {
        let directory = TestDirectory::new("live-registration");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let lease_replica = directory.0.join("runtime-worker-leases.bin");
        let runtime = OperationalBinaryRuntime::open_generation(
            generation.clone(),
            lease_replica.clone(),
            60,
            15,
        )
        .expect("open frozen Binary generation");

        let created = runtime
            .register_repository("ait-runner", [b'R', 0], 0b1000_0011)
            .expect("register live Repository");
        assert_eq!(created["created"], true);
        assert_eq!(created["repository"]["repository_index"], 4);
        assert_eq!(runtime.repository_indexes(), vec![0, 1, 2, 3, 4]);
        assert_eq!(
            runtime
                .repository_authority(4)
                .expect("read live Repository")["repository"]["repository_name"],
            "ait-runner"
        );

        let repeated = runtime
            .register_repository("ait-runner", [b'R', 0], 0b1000_0011)
            .expect("repeat exact namespace registration");
        assert_eq!(repeated["created"], false);
        assert_eq!(repeated["repository"]["repository_index"], 4);

        let duplicate_name = runtime
            .register_repository("ait-runner", *b"R2", 0b1000_0011)
            .expect("register duplicate discovery name");
        assert_eq!(duplicate_name["repository"]["repository_index"], 5);
        assert!(runtime
            .register_repository("different", [b'R', 0], 0b1000_0011)
            .is_err());
        drop(runtime);

        let completion_file = generation.join(SERVER_FRESH_COMPLETION_FILE);
        let completion_bytes = fs::read(&completion_file).expect("read fresh completion");
        let activation = ServerBinaryActivation {
            activation_pointer: directory.0.join("active.json"),
            generation_root: generation,
            completion_file,
            completion_sha256: sha256(&completion_bytes),
        };
        let reopened =
            OperationalBinaryRuntime::from_activation(&activation, lease_replica, 60, 15)
                .expect("reopen activated generation above immutable manifest baseline");
        assert_eq!(reopened.repository_indexes(), vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn retirement_exports_exact_authority_purges_then_restores_at_a_new_index() {
        let directory = TestDirectory::new("retirement-restore");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let lease_replica = directory.0.join("runtime-worker-leases.bin");
        let runtime = OperationalBinaryRuntime::open_generation(
            generation.clone(),
            lease_replica.clone(),
            60,
            15,
        )
        .expect("open frozen Binary generation");

        let status = runtime
            .begin_repository_retirement(1)
            .expect("begin Repository retirement");
        assert_eq!(status["ready_for_export"], true);
        let manifest: RemoteExportManifest =
            serde_json::from_value(status["manifest"].clone()).expect("decode export manifest");
        assert_eq!(manifest.repo_name, "ait-server");
        assert_eq!(manifest.namespace, "SE");
        assert!(manifest
            .files
            .iter()
            .any(|file| file.path == "patchset.bin"));
        assert!(manifest
            .files
            .iter()
            .any(|file| file.path == "worker_job.bin"));
        assert!(manifest.files.iter().all(|file| {
            !file.path.ends_with(".lock")
                && !file.path.ends_with(".journal")
                && !file.path.ends_with(".rewrite")
        }));
        let exported = manifest
            .files
            .iter()
            .map(|file| {
                (
                    file.path.clone(),
                    runtime
                        .repository_retirement_file(1, &file.path)
                        .expect("read manifested authority file"),
                )
            })
            .collect::<BTreeMap<_, _>>();

        let purged = runtime
            .purge_retired_repository(
                1,
                &serde_json::to_value(&manifest).expect("encode acknowledgement"),
            )
            .expect("purge acknowledged Repository");
        assert_eq!(
            purged["repository"]["lifecycle_kind"],
            REPOSITORY_LIFECYCLE_PURGED
        );
        assert!(runtime.repository_authority(1).is_err());
        assert!(runtime
            .repository_authorities(Some("ait-server"))
            .expect("discover live duplicate name")["repositories"]
            .as_array()
            .expect("Repository discovery array")
            .is_empty());

        let interrupted_session = runtime
            .begin_repository_restore(&json!({
                "manifest": manifest,
                "policy_flags": 0b1000_0011,
            }))
            .expect("begin Repository restore");
        let interrupted_token = interrupted_session["restore_token"]
            .as_str()
            .expect("restore token")
            .to_string();
        let (first_path, first_bytes) =
            exported.first_key_value().expect("exported authority file");
        runtime
            .upload_repository_restore_file(&interrupted_token, first_path, first_bytes)
            .expect("stage one file before interruption");
        let interrupted_root = runtime.restore_staging_parent.join(&interrupted_token);
        assert!(interrupted_root.is_dir());
        drop(runtime);

        let runtime = OperationalBinaryRuntime::open_generation(
            generation.clone(),
            lease_replica.clone(),
            60,
            15,
        )
        .expect("reopen after interrupted restore upload");
        assert!(
            !interrupted_root.exists(),
            "runtime activation must remove non-resumable restore staging"
        );
        assert!(runtime
            .commit_repository_restore(&interrupted_token)
            .expect_err("memory-local interrupted session must not be published")
            .contains("unknown or expired Repository restore session"));
        assert!(runtime.registry.get(4).is_err());

        let session = runtime
            .begin_repository_restore(&json!({
                "manifest": manifest,
                "policy_flags": 0b1000_0011,
            }))
            .expect("restart Repository restore");
        let token = session["restore_token"]
            .as_str()
            .expect("replacement restore token")
            .to_string();
        for (path, bytes) in &exported {
            runtime
                .upload_repository_restore_file(&token, path, bytes)
                .expect("upload exact authority file");
        }
        let restored = runtime
            .commit_repository_restore(&token)
            .expect("commit Repository restore");
        assert_eq!(restored["repository"]["repository_index"], 4);
        assert_eq!(restored["repository"]["repository_name"], "ait-server");
        assert_eq!(restored["repository"]["namespace"], "SE");
        assert_eq!(
            runtime
                .commit_repository_restore(&token)
                .expect("repeat committed restore"),
            restored
        );
        let committed_abort = runtime
            .abort_repository_restore(&token)
            .expect("abort committed restore is terminal-idempotent");
        assert_eq!(committed_abort["aborted"], false);
        assert_eq!(committed_abort["state"], "committed");
        assert_eq!(
            runtime
                .register_repository("ait-server", *b"SE", 0b1000_0011)
                .expect("same restored namespace is idempotent")["repository"]["repository_index"],
            4
        );
        drop(runtime);

        let reopened = OperationalBinaryRuntime::open_generation(generation, lease_replica, 60, 15)
            .expect("reopen purged and restored registry");
        assert_eq!(reopened.repository_indexes(), vec![0, 1, 2, 3, 4]);
        assert_eq!(
            reopened
                .repository_authority(4)
                .expect("read restored Repository")["repository"]["repository_name"],
            "ait-server"
        );
        assert_eq!(
            reopened.registry.get(1).unwrap().record.lifecycle_kind,
            REPOSITORY_LIFECYCLE_PURGED
        );
    }

    #[test]
    fn restore_staging_enforces_quota_abort_expiry_failure_cleanup_and_process_lock() {
        let directory = TestDirectory::new("restore-staging-bounds");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let lease_replica = directory.0.join("runtime-worker-leases.bin");
        let runtime = OperationalBinaryRuntime::open_generation_with_restore_limits(
            generation.clone(),
            lease_replica.clone(),
            60,
            15,
            60,
            8,
        )
        .expect("open frozen Binary generation with bounded restore staging");
        let second_open_error = match OperationalBinaryRuntime::open_generation_with_restore_limits(
            generation.clone(),
            lease_replica.clone(),
            60,
            15,
            60,
            8,
        ) {
            Ok(_) => panic!("a second runtime must not own the same restore staging"),
            Err(error) => error,
        };
        assert!(second_open_error.contains("exclusive Repository restore staging lock"));

        let bytes = b"hello";
        let manifest = RemoteExportManifest {
            schema: REMOTE_EXPORT_SCHEMA.to_string(),
            state: REMOTE_EXPORT_STATE_COMPLETE.to_string(),
            repo_name: "restored".to_string(),
            namespace: "RZ".to_string(),
            exported_at_s: 1_786_000_000,
            files: vec![crate::repository_retirement::RemoteExportFile {
                path: "worker_job.bin".to_string(),
                size: bytes.len() as u64,
                sha256: sha256(bytes),
            }],
        };
        let payload = json!({"manifest": manifest, "policy_flags": 0});
        let oversized = json!({
            "manifest": RemoteExportManifest {
                files: vec![crate::repository_retirement::RemoteExportFile {
                    path: "worker_job.bin".to_string(),
                    size: 9,
                    sha256: sha256(b"123456789"),
                }],
                ..manifest.clone()
            },
            "policy_flags": 0,
        });
        assert!(runtime
            .begin_repository_restore(&oversized)
            .expect_err("one manifest cannot exceed the staging byte ceiling")
            .contains("above the configured staging ceiling"));

        let failed = runtime
            .begin_repository_restore(&payload)
            .expect("begin failed-upload fixture");
        let failed_token = failed["restore_token"].as_str().unwrap().to_string();
        let failed_root = runtime.restore_staging_parent.join(&failed_token);
        assert!(runtime
            .upload_repository_restore_file(&failed_token, "worker_job.bin", b"wrong")
            .expect_err("checksum mismatch must fail and discard the session")
            .contains("does not match"));
        assert!(!failed_root.exists());
        assert!(!runtime
            .restore_sessions
            .lock()
            .unwrap()
            .contains_key(&failed_token));

        let first = runtime
            .begin_repository_restore(&payload)
            .expect("reserve first restore");
        assert!(runtime
            .begin_repository_restore(&payload)
            .expect_err("aggregate reservations must honor the byte ceiling")
            .contains("byte capacity is exhausted"));
        let first_token = first["restore_token"].as_str().unwrap().to_string();
        let aborted = runtime
            .abort_repository_restore(&first_token)
            .expect("abort live restore");
        assert_eq!(aborted["state"], "aborted");
        assert_eq!(aborted["aborted"], true);
        let repeated = runtime
            .abort_repository_restore(&first_token)
            .expect("repeat abort is idempotent");
        assert_eq!(repeated["state"], "absent");
        assert_eq!(repeated["already_terminal"], true);

        let expired = runtime
            .begin_repository_restore(&payload)
            .expect("begin expiry fixture");
        let expired_token = expired["restore_token"].as_str().unwrap().to_string();
        let expired_root = runtime.restore_staging_parent.join(&expired_token);
        runtime
            .restore_sessions
            .lock()
            .unwrap()
            .get_mut(&expired_token)
            .unwrap()
            .expires_at_s = 0;
        let replacement = runtime
            .begin_repository_restore(&payload)
            .expect("expired reservation is reclaimed before admission");
        assert!(!expired_root.exists());
        let replacement_token = replacement["restore_token"].as_str().unwrap().to_string();
        let replacement_root = runtime.restore_staging_parent.join(&replacement_token);
        assert!(replacement_root.is_dir());
        drop(runtime);

        let reopened = OperationalBinaryRuntime::open_generation_with_restore_limits(
            generation,
            lease_replica,
            60,
            15,
            60,
            8,
        )
        .expect("reopen and clean abandoned restore staging");
        assert!(!replacement_root.exists());
        assert!(reopened.restore_sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn retirement_file_reads_do_not_recover_unrelated_repository_queues() {
        let directory = TestDirectory::new("retirement-file-target-local");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let runtime = OperationalBinaryRuntime::open_generation(
            generation,
            directory.0.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("open frozen Binary generation");

        let status = runtime
            .begin_repository_retirement(1)
            .expect("begin Repository retirement");
        let manifest: RemoteExportManifest =
            serde_json::from_value(status["manifest"].clone()).expect("decode export manifest");
        let expected = manifest.files.first().expect("manifested authority file");

        let unrelated_worker_jobs = runtime
            .registry
            .resolve_authority_directory(0)
            .expect("resolve unrelated Repository authority")
            .join("worker_job.bin");
        let original =
            fs::read(&unrelated_worker_jobs).expect("read unrelated Worker Job authority");
        fs::write(&unrelated_worker_jobs, b"invalid")
            .expect("make unrelated Worker Job authority temporarily invalid");

        let exported = runtime
            .repository_retirement_file(1, &expected.path)
            .expect("read target Repository authority without global reconciliation");

        fs::write(&unrelated_worker_jobs, original)
            .expect("restore unrelated Worker Job authority fixture");
        assert_eq!(
            u64::try_from(exported.len()).expect("export length fits u64"),
            expected.size
        );
        assert_eq!(sha256(&exported), expected.sha256);
    }

    #[test]
    fn retirement_abort_is_idempotent_and_preserves_repository_authority() {
        let directory = TestDirectory::new("retirement-abort");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let runtime = OperationalBinaryRuntime::open_generation(
            generation,
            directory.0.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("open frozen Binary generation");
        let before = runtime.registry.get(1).expect("read active Repository");
        let authority_root = runtime
            .registry
            .resolve_authority_directory(1)
            .expect("resolve Repository authority");
        let authority_before =
            collect_export_files(&authority_root).expect("inventory authority before retirement");

        runtime
            .begin_repository_retirement(1)
            .expect("begin Repository retirement");
        assert!(runtime.require_active_repository(1).is_err());
        let aborted = runtime
            .abort_repository_retirement(1)
            .expect("abort Repository retirement");
        assert_eq!(
            aborted["contract"],
            "ait.server.repository-retirement-abort.v1"
        );
        assert_eq!(aborted["aborted"], true);
        assert_eq!(aborted["already_aborted"], false);
        assert_eq!(
            aborted["repository"]["lifecycle_kind"],
            REPOSITORY_LIFECYCLE_ACTIVE
        );
        runtime
            .require_active_repository(1)
            .expect("aborted Repository admits mutations");

        let after = runtime.registry.get(1).expect("read aborted Repository");
        assert_eq!(after.repository_index, before.repository_index);
        assert_eq!(after.repo_name, before.repo_name);
        assert_eq!(after.record.namespace_ascii, before.record.namespace_ascii);
        assert_eq!(after.record.policy_flags, before.record.policy_flags);
        assert_eq!(after.record.created_at_s, before.record.created_at_s);
        assert!(!after.record.is_tombstoned());
        assert_eq!(
            collect_export_files(&authority_root)
                .expect("inventory authority after retirement rollback"),
            authority_before
        );

        let repeated = runtime
            .abort_repository_retirement(1)
            .expect("repeat Repository retirement abort");
        assert_eq!(repeated["already_aborted"], true);
        assert_eq!(
            repeated["repository"]["updated_at_s"],
            aborted["repository"]["updated_at_s"]
        );
    }

    #[test]
    fn retirement_abort_and_purge_have_one_linearized_winner() {
        let directory = TestDirectory::new("retirement-abort-purge-race");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let runtime = Arc::new(
            OperationalBinaryRuntime::open_generation(
                generation,
                directory.0.join("runtime-worker-leases.bin"),
                60,
                15,
            )
            .expect("open frozen Binary generation"),
        );
        let status = runtime
            .begin_repository_retirement(1)
            .expect("begin Repository retirement");
        let manifest = status["manifest"].clone();
        let barrier = Arc::new(Barrier::new(3));

        let abort_runtime = runtime.clone();
        let abort_barrier = barrier.clone();
        let abort = std::thread::spawn(move || {
            abort_barrier.wait();
            abort_runtime.abort_repository_retirement(1)
        });
        let purge_runtime = runtime.clone();
        let purge_barrier = barrier.clone();
        let purge = std::thread::spawn(move || {
            purge_barrier.wait();
            purge_runtime.purge_retired_repository(1, &manifest)
        });
        barrier.wait();
        let abort = abort.join().expect("join abort transaction");
        let purge = purge.join().expect("join purge transaction");
        assert_ne!(
            abort.is_ok(),
            purge.is_ok(),
            "exactly one terminal transaction branch must commit"
        );

        let lifecycle = runtime
            .registry
            .get(1)
            .expect("read terminal Repository")
            .record
            .lifecycle_kind;
        if abort.is_ok() {
            assert_eq!(lifecycle, REPOSITORY_LIFECYCLE_ACTIVE);
            assert!(purge
                .expect_err("purge must lose after abort")
                .contains("not retiring"));
        } else {
            assert_eq!(lifecycle, REPOSITORY_LIFECYCLE_PURGED);
            assert!(abort
                .expect_err("abort must lose after purge")
                .contains("purged"));
        }
    }

    #[test]
    fn generation_open_rejects_registry_shorter_than_manifest_baseline() {
        let directory = TestDirectory::new("short-registry-baseline");
        let generation = directory.0.join("generation");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize frozen Binary generation");
        let manifest_path = generation.join("generation.json");
        let mut manifest: JsonValue =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read generation manifest"))
                .expect("decode generation manifest");
        manifest["repository_count"] = json!(5);
        let mut manifest_bytes =
            serde_json::to_vec_pretty(&manifest).expect("encode generation manifest");
        manifest_bytes.push(b'\n');
        fs::write(&manifest_path, manifest_bytes).expect("write larger activation baseline");

        let error = OperationalBinaryRuntime::open_generation(
            generation,
            directory.0.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .err()
        .expect("reject Registry shorter than manifest baseline");
        assert!(
            error.contains("shorter than its activation baseline"),
            "{error}"
        );
    }

    #[test]
    fn converted_generation_opens_without_postgres_runtime_state() {
        let directory = TestDirectory::new("converted-generation");
        let generation = directory.0.join("converted");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize compatible converted authority");
        fs::remove_file(generation.join(SERVER_FRESH_COMPLETION_FILE))
            .expect("replace fresh completion with conversion completion");

        let report = b"{\"schema\":\"ait.server.postgres_to_binary_v0.report.v1\",\"layout_id\":1,\"status\":\"validated_inactive\"}\n";
        fs::write(generation.join("conversion-report.json"), report)
            .expect("write converted generation report");
        let completion = serde_json::to_vec_pretty(&json!({
            "schema": POSTGRES_CONVERSION_COMPLETION_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "report_sha256": sha256(report),
        }))
        .expect("encode converted completion");
        let mut completion = completion;
        completion.push(b'\n');
        let completion_file = generation.join(SERVER_LEGACY_CONVERSION_COMPLETION_FILE);
        fs::write(&completion_file, &completion).expect("write converted completion");

        let activation = ServerBinaryActivation {
            activation_pointer: directory.0.join("active.json"),
            generation_root: generation,
            completion_file,
            completion_sha256: sha256(&completion),
        };
        let runtime = OperationalBinaryRuntime::from_activation(
            &activation,
            directory.0.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("open converted Binary generation");

        assert_eq!(runtime.repository_indexes(), vec![0, 1, 2, 3]);
        assert_eq!(
            runtime
                .repository_authority(1)
                .expect("read converted Repository authority")["repository"]["repository_name"],
            "ait-server"
        );
    }

    #[test]
    fn u64_second_upgrade_generation_opens_with_exact_completion_pair() {
        let directory = TestDirectory::new("u64-second-upgrade-generation");
        let generation = directory.0.join("converted");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize compatible upgraded authority");
        fs::remove_file(generation.join(SERVER_FRESH_COMPLETION_FILE))
            .expect("replace fresh completion with u64-second upgrade completion");

        let mut report = serde_json::to_vec_pretty(&json!({
            "schema": U64_SECOND_UPGRADE_REPORT_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "source_selector": U32_TIME_V0_SELECTOR,
            "target_selector": U64_SECOND_V0_SELECTOR,
        }))
        .expect("encode u64-second upgrade report");
        report.push(b'\n');
        fs::write(generation.join("conversion-report.json"), &report)
            .expect("write u64-second upgrade report");
        let mut completion = serde_json::to_vec_pretty(&json!({
            "schema": U64_SECOND_UPGRADE_COMPLETION_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "report_sha256": sha256(&report),
        }))
        .expect("encode u64-second upgrade completion");
        completion.push(b'\n');
        let completion_file = generation.join(SERVER_LEGACY_CONVERSION_COMPLETION_FILE);
        fs::write(&completion_file, &completion).expect("write u64-second upgrade completion");

        let activation = ServerBinaryActivation {
            activation_pointer: directory.0.join("active.json"),
            generation_root: generation,
            completion_file,
            completion_sha256: sha256(&completion),
        };
        let runtime = OperationalBinaryRuntime::from_activation(
            &activation,
            directory.0.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("open exact u64-second upgraded Binary generation");

        assert_eq!(runtime.repository_indexes(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn legacy_plan_lineage_generation_restarts_after_legitimate_plan_mutation() {
        let directory = TestDirectory::new("plan-lineage-repair-generation");
        let generation = directory.0.join("converted");
        initialize_fresh_generation(&generation, 1_786_000_000)
            .expect("initialize compatible repaired authority");
        fs::remove_file(generation.join(SERVER_FRESH_COMPLETION_FILE))
            .expect("replace fresh completion with Plan-lineage repair completion");
        fs::write(
            generation.join(LEGACY_PLAN_LINEAGE_RECEIPT_FILE),
            b"{\"status\":\"validated_inactive\"}\n",
        )
        .expect("write Plan-lineage repair receipt");

        let sealed_files = expected_legacy_plan_lineage_sealed_paths()
            .into_iter()
            .map(|relative_path| {
                let bytes = fs::read(generation.join(&relative_path))
                    .expect("read Plan-lineage repair sealed file");
                json!({
                    "relative_path": relative_path,
                    "byte_size": bytes.len(),
                    "sha256": sha256(&bytes),
                })
            })
            .collect::<Vec<_>>();
        let mut report = serde_json::to_vec_pretty(&json!({
            "schema": LEGACY_PLAN_LINEAGE_REPORT_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "repository_indexes": [0, 1],
            "sealed_files": sealed_files,
        }))
        .expect("encode Plan-lineage repair report");
        report.push(b'\n');
        fs::write(generation.join("conversion-report.json"), &report)
            .expect("write Plan-lineage repair report");
        let mut completion = serde_json::to_vec_pretty(&json!({
            "schema": LEGACY_PLAN_LINEAGE_COMPLETION_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "report_sha256": sha256(&report),
        }))
        .expect("encode Plan-lineage repair completion");
        completion.push(b'\n');
        let completion_file = generation.join(SERVER_LEGACY_CONVERSION_COMPLETION_FILE);
        fs::write(&completion_file, &completion).expect("write Plan-lineage repair completion");
        let activation = ServerBinaryActivation {
            activation_pointer: directory.0.join("active.json"),
            generation_root: generation.clone(),
            completion_file,
            completion_sha256: sha256(&completion),
        };
        let runtime = OperationalBinaryRuntime::from_activation(
            &activation,
            directory.0.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("open scoped Plan-lineage repaired Binary generation");
        assert_eq!(runtime.repository_indexes(), vec![0, 1, 2, 3]);

        let (_, repository_db) = runtime
            .repository_db(1)
            .expect("open mutable ait-server Repository authority");
        let plans = BinaryDbServerPlanService::new(repository_db);
        let created = plans
            .create_plan(
                "ait-server",
                &json!({
                    "title": "Post-activation Plan",
                    "status": "draft",
                    "artifact_path": "docs/sprints/post_activation.md",
                    "artifact_selector": "post-activation/root",
                    "artifact_heading": "Post-activation Plan",
                    "items": [],
                    "actor_type": "repository",
                }),
            )
            .expect("create legitimate Plan after activation");
        let created_plan_id = created["plan_id"]
            .as_str()
            .expect("created Plan identity")
            .to_string();
        drop(runtime);

        let reopened = OperationalBinaryRuntime::from_activation(
            &activation,
            directory.0.join("reopened-runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("restart after legitimate mutable Plan authority write");
        let (_, reopened_db) = reopened
            .repository_db(1)
            .expect("reopen mutable ait-server Repository authority");
        assert_eq!(
            BinaryDbServerPlanService::new(reopened_db)
                .get_plan(&created_plan_id)
                .expect("read post-activation Plan after restart")["status"],
            "draft"
        );
        drop(reopened);

        let receipt_path = generation.join(LEGACY_PLAN_LINEAGE_RECEIPT_FILE);
        let mut receipt_bytes = fs::read(&receipt_path).expect("read immutable repair receipt");
        receipt_bytes.push(b' ');
        fs::write(receipt_path, receipt_bytes).expect("tamper immutable repair receipt");
        let error = OperationalBinaryRuntime::from_activation(
            &activation,
            directory.0.join("tampered-runtime-worker-leases.bin"),
            60,
            15,
        )
        .err()
        .expect("reject changed Plan-lineage immutable evidence");
        assert!(error.contains("immutable evidence changed"), "{error}");
    }

    #[test]
    fn converted_completion_rejects_cross_pairs_and_selector_mismatches() {
        let directory = TestDirectory::new("converted-completion-pairing");
        let generation = directory.0.join("generation");
        fs::create_dir(&generation).expect("create converted generation fixture");
        let completion_path = generation.join(SERVER_LEGACY_CONVERSION_COMPLETION_FILE);

        let assert_rejected = |completion_schema: &str, report: JsonValue| {
            let mut report_bytes =
                serde_json::to_vec_pretty(&report).expect("encode rejected conversion report");
            report_bytes.push(b'\n');
            fs::write(generation.join("conversion-report.json"), &report_bytes)
                .expect("write rejected conversion report");
            let mut completion_bytes = serde_json::to_vec_pretty(&json!({
                "schema": completion_schema,
                "layout_id": 1,
                "status": "validated_inactive",
                "report_sha256": sha256(&report_bytes),
            }))
            .expect("encode rejected conversion completion");
            completion_bytes.push(b'\n');
            let error = validate_completion(&generation, &completion_path, &completion_bytes)
                .expect_err("reject mismatched conversion evidence");
            assert!(error.contains("envelope is invalid"), "{error}");
        };

        assert_rejected(
            POSTGRES_CONVERSION_COMPLETION_SCHEMA,
            json!({
                "schema": U64_SECOND_UPGRADE_REPORT_SCHEMA,
                "layout_id": 1,
                "status": "validated_inactive",
                "source_selector": U32_TIME_V0_SELECTOR,
                "target_selector": U64_SECOND_V0_SELECTOR,
            }),
        );
        assert_rejected(
            U64_SECOND_UPGRADE_COMPLETION_SCHEMA,
            json!({
                "schema": POSTGRES_CONVERSION_REPORT_SCHEMA,
                "layout_id": 1,
                "status": "validated_inactive",
            }),
        );
        assert_rejected(
            U64_SECOND_UPGRADE_COMPLETION_SCHEMA,
            json!({
                "schema": U64_SECOND_UPGRADE_REPORT_SCHEMA,
                "layout_id": 1,
                "status": "validated_inactive",
                "source_selector": U64_SECOND_V0_SELECTOR,
                "target_selector": U64_SECOND_V0_SELECTOR,
            }),
        );
        assert_rejected(
            "ait.server.binary_v0.unknown.complete.v1",
            json!({
                "schema": U64_SECOND_UPGRADE_REPORT_SCHEMA,
                "layout_id": 1,
                "status": "validated_inactive",
                "source_selector": U32_TIME_V0_SELECTOR,
                "target_selector": U64_SECOND_V0_SELECTOR,
            }),
        );
    }

    #[test]
    fn converted_completion_rejects_report_changed_after_hashing() {
        let directory = TestDirectory::new("converted-completion-report-hash");
        let generation = directory.0.join("generation");
        fs::create_dir(&generation).expect("create converted generation fixture");
        let original_report = b"{\"schema\":\"ait.server.binary_v0.u64_second_upgrade.report.v1\",\"layout_id\":1,\"status\":\"validated_inactive\",\"source_selector\":\"u32-time-v0\",\"target_selector\":\"u64-second-v0\"}\n";
        fs::write(
            generation.join("conversion-report.json"),
            [original_report.as_slice(), b"\n"].concat(),
        )
        .expect("change u64-second report after completion");
        let mut completion = serde_json::to_vec_pretty(&json!({
            "schema": U64_SECOND_UPGRADE_COMPLETION_SCHEMA,
            "layout_id": 1,
            "status": "validated_inactive",
            "report_sha256": sha256(original_report),
        }))
        .expect("encode u64-second completion");
        completion.push(b'\n');

        let error = validate_completion(
            &generation,
            &generation.join(SERVER_LEGACY_CONVERSION_COMPLETION_FILE),
            &completion,
        )
        .expect_err("reject report changed after completion");
        assert!(error.contains("changed after completion"), "{error}");
    }
}
