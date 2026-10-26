use super::*;
use crate::foundation::operational_binary_v0::{
    SERVER_GLOBAL_OPERATIONAL_BIN_PATHS, SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS,
};
use crate::foundation::remote_binary_db::{
    BinaryDbCommandLockSet, BinaryDbCommandScope, BoxedServerBinaryDbProcessLockGuard,
};
use crate::foundation::server_binary_lifecycle::{
    ServerBinaryActivation, ServerBinaryLifecycleConfig,
};
use crate::foundation::server_operational_job_domain::FrozenBinaryV0WorkerJobAuthority;
use crate::foundation::server_operational_repository_registry::{
    OperationalRepositoryEntry, ServerOperationalRepositoryRegistry, REGISTRY_LOCK_FILE_NAME,
};
use crate::foundation::server_operational_worker_jobs::{
    ServerOperationalWorkerJobStore, WorkerJobDomainAuthority, WORKER_QUEUE_LOCK_FILE_NAME,
};
use crate::foundation::server_plan_binary_db::remote_actor_profile::{
    classify_remote_plan_metadata, convert_remote_plan_metadata, RemotePlanMetadataProfile,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

mod plan_conversion;

pub const WORKFLOW_UPGRADE_COMPLETION_SCHEMA: &str =
    "ait.server.binary_v0.workflow_upgrade.complete.v1";
const REPORT_SCHEMA: &str = "ait.server.binary_v0.workflow_upgrade.report.v1";
const SOURCE_SELECTOR: &str = "remote-u64-workflow-v1";
const TARGET_SELECTOR: &str = "task-owned-workflow-v2";
const REPORT_FILE: &str = "conversion-report.json";

/// A transient, explicitly reviewed source disposition. It is not a persisted
/// switch or a rule inferred from row names. Startup rechecks its complete seal.
pub struct WorkflowGenerationAdmission {
    pub source_fingerprint: String,
    pub initial_changes_by_repository: BTreeMap<u32, BTreeSet<u32>>,
    pub incoming_changes_by_repository: BTreeMap<u32, BTreeSet<u32>>,
    pub historical_dispositions_by_repository: BTreeMap<u32, WorkflowHistoricalDisposition>,
}

pub fn workflow_generation_fingerprint(root: &Path) -> Result<String, String> {
    inventory_fingerprint(&inventory(&canonical_real_directory(root)?)?)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct SealedFile {
    relative_path: String,
    byte_size: u64,
    sha256: String,
}
#[derive(Deserialize, Serialize)]
struct BatchReport {
    schema: String,
    layout_id: u32,
    status: String,
    source_selector: String,
    target_selector: String,
    source_generation: String,
    source_authority_fingerprint: String,
    target_authority_fingerprint: String,
    repository_indexes: Vec<u32>,
    authority_files: Vec<SealedFile>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    schema: String,
    layout_id: u32,
    status: String,
    report_sha256: String,
}

/// Startup caller holds exclusive lifecycle admission and the existing service
/// restore lock for the active generation. No request path invokes conversion.
pub fn admit_workflow_generation(
    config: &ServerBinaryLifecycleConfig,
    active: &ServerBinaryActivation,
) -> Result<ServerBinaryActivation, String> {
    admit_with_hook(config, active, |_| Ok(()))
}

pub fn admit_workflow_generation_with_progress(
    config: &ServerBinaryLifecycleConfig,
    active: &ServerBinaryActivation,
    observer: impl FnMut(WorkflowUpgradeProgress),
) -> Result<ServerBinaryActivation, String> {
    admit_with_policy(config, active, None, |_| Ok(()), observer)
}
pub fn admit_workflow_generation_with_admission(
    config: &ServerBinaryLifecycleConfig,
    active: &ServerBinaryActivation,
    admission: &WorkflowGenerationAdmission,
) -> Result<ServerBinaryActivation, String> {
    admit_with_policy(config, active, Some(admission), |_| Ok(()), |_| {})
}
fn admit_with_hook(
    config: &ServerBinaryLifecycleConfig,
    active: &ServerBinaryActivation,
    hook: impl FnMut(&str) -> Result<(), String>,
) -> Result<ServerBinaryActivation, String> {
    admit_with_policy(config, active, None, hook, |_| {})
}
fn admit_with_policy(
    config: &ServerBinaryLifecycleConfig,
    active: &ServerBinaryActivation,
    admission: Option<&WorkflowGenerationAdmission>,
    mut hook: impl FnMut(&str) -> Result<(), String>,
    mut observer: impl FnMut(WorkflowUpgradeProgress),
) -> Result<ServerBinaryActivation, String> {
    if config.activation()?.as_ref() != Some(active) {
        return Err("Startup activation changed before workflow admission".into());
    }
    let source = canonical_real_directory(&active.generation_root)?;
    let manifest = read_regular_file(&source.join("generation.json"))?;
    let value: Value = serde_json::from_slice(&manifest).map_err(|error| error.to_string())?;
    if value["schema"] != "ait.server.binary_v0.operational_generation.v1"
        || value["layout_id"] != 1
        || value["status"] != "validated_inactive"
        || value["global_registry"] != "global"
        || value["repository_authorities"] != "repositories"
        || value["repository_count"].as_u64().is_none()
    {
        return Err("Unsupported generation profile at workflow admission".into());
    }
    let global = source.join("global");
    let repositories = source.join("repositories");
    let _global_lock = shared_lock(&global.join(REGISTRY_LOCK_FILE_NAME))?;
    let registry = ServerOperationalRepositoryRegistry::new(&global, &repositories)
        .map_err(|error| error.to_string())?;
    let physical_entries = registry.list().map_err(|error| error.to_string())?;
    config.repository_scope.validate_known(&physical_entries)?;
    if physical_entries.len() < value["repository_count"].as_u64().unwrap() as usize {
        return Err("Repository registry is shorter than generation baseline".into());
    }
    let entries = physical_entries
        .iter()
        .filter(|entry| config.repository_scope.includes(entry))
        .cloned()
        .collect::<Vec<_>>();
    if entries.is_empty() {
        report_progress(&mut observer, WorkflowUpgradePhase::Complete, 0, Some(0));
        return Ok(active.clone());
    }
    if plan_conversion::convert_if_current(
        config,
        active,
        &source,
        &entries,
        _global_lock,
        &mut hook,
        &mut observer,
    )? {
        return Ok(active.clone());
    }
    // Only the pre-existing legacy workflow conversion reaches this path.
    let _global_lock = shared_lock(&global.join(REGISTRY_LOCK_FILE_NAME))?;
    registry.validate().map_err(|error| error.to_string())?;
    let global_files = read_global(&global)?;
    let names = fs::read_dir(&repositories)
        .map_err(|error| error.to_string())?
        .map(|entry| {
            entry
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .map_err(|error| error.to_string())
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if names
        != physical_entries
            .iter()
            .map(|entry| entry.repository_index.to_string())
            .collect()
    {
        return Err("Unadmitted Repository directory in complete generation inventory".into());
    }
    validate_root(&source)?;
    let mut locks = Vec::new();
    let mut queue_locks = Vec::new();
    let mut metadata = Vec::new();
    let mut generation_profile = None;
    let mut generation_plan_profile = None;
    report_progress(
        &mut observer,
        WorkflowUpgradePhase::ReadMetadata,
        0,
        Some(entries.len()),
    );
    for entry in &entries {
        let root = repositories.join(entry.repository_index.to_string());
        canonical_real_directory(&root)?;
        queue_locks.push(shared_lock(&root.join(WORKER_QUEUE_LOCK_FILE_NAME))?);
        locks.push(
            BinaryDbReadLockSet::try_acquire(&StorePath::new(root.clone()))
                .map_err(|error| error.to_string())?,
        );
        let files = read_authority_files(&root, false)?;
        let this = profile(&files)
            .map_err(|error| format!("Repository {}: {error}", entry.repository_index))?;
        let plan_profile =
            classify_remote_plan_metadata(&files["plan.bin"], &files["plan_revision.bin"])
                .map_err(|error| format!("Repository {}: {error}", entry.repository_index))?;
        if this == WorkflowProfile::Legacy && plan_profile == RemotePlanMetadataProfile::Actor {
            return Err("Unsupported combined profile: Actor Plan with legacy workflow".into());
        }
        if generation_plan_profile
            .replace(plan_profile)
            .is_some_and(|previous| previous != plan_profile)
        {
            return Err("Partial generation Plan profile; both files and all included authorities form one batch".into());
        }
        if generation_profile
            .replace(this)
            .is_some_and(|previous| previous != this)
        {
            return Err(
                "Partial generation workflow profile; repositories cannot activate separately"
                    .into(),
            );
        }
        metadata.push(files);
        report_progress(
            &mut observer,
            WorkflowUpgradePhase::ReadMetadata,
            metadata.len(),
            Some(entries.len()),
        );
    }
    let generation_profile =
        generation_profile.ok_or("Generation has no managed Repository authorities")?;
    if generation_profile == WorkflowProfile::Current
        && generation_plan_profile == Some(RemotePlanMetadataProfile::Actor)
    {
        report_progress(&mut observer, WorkflowUpgradePhase::Complete, 0, Some(0));
        return Ok(active.clone());
    }
    let before = inventory_for_entries(&source, &entries)?;
    let source_fingerprint = inventory_fingerprint(&before)?;
    if generation_profile == WorkflowProfile::Legacy {
        for (entry, files) in entries.iter().zip(&metadata) {
            history::validate_source_namespace(files, &entry.record.namespace_ascii)?;
            history::validate_source_plan_bindings(
                &repositories.join(entry.repository_index.to_string()),
                files,
            )?;
        }
    }
    // Native startup derives proven membership; no production input is added.
    let derived;
    let admission = if generation_profile == WorkflowProfile::Legacy {
        Some(match admission {
            Some(value) => value,
            None => {
                derived = derive_admission(&entries, &metadata, source_fingerprint.clone())?;
                &derived
            }
        })
    } else {
        None
    };
    if let Some(admission) = admission {
        let indexes = entries
            .iter()
            .map(|entry| entry.repository_index)
            .collect::<BTreeSet<_>>();
        if admission.source_fingerprint != source_fingerprint
            || admission
                .initial_changes_by_repository
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                != indexes
            || admission
                .historical_dispositions_by_repository
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                != indexes
            || admission
                .incoming_changes_by_repository
                .keys()
                .copied()
                .collect::<BTreeSet<_>>()
                != indexes
        {
            return Err("Explicit generation disposition does not match the complete source fingerprint/Repository closure".into());
        }
    }
    for (entry, files) in entries.iter().zip(&metadata) {
        for (name, bytes) in files {
            let path = format!("repositories/{}/{name}", entry.repository_index);
            if !before.iter().any(|file| {
                file.relative_path == path
                    && file.sha256 == sha256(bytes)
                    && file.byte_size == bytes.len() as u64
            }) {
                return Err("Repository metadata drifted before source sealing".into());
            }
        }
    }
    let mut converted_count = 0;
    report_progress(
        &mut observer,
        WorkflowUpgradePhase::Convert,
        0,
        Some(entries.len()),
    );
    let converted_metadata = entries
        .iter()
        .zip(&metadata)
        .map(|(entry, files)| {
            let mut converted = if let Some(admission) = admission {
                convert_with_disposition(
                    files,
                    &admission.initial_changes_by_repository[&entry.repository_index],
                    &admission.incoming_changes_by_repository[&entry.repository_index],
                    &admission.historical_dispositions_by_repository[&entry.repository_index],
                )
                .map(|(files, _)| files)
                .map_err(|error| format!("Repository {}: {error}", entry.repository_index))?
            } else {
                files.clone()
            };
            let (plans, revisions) = convert_remote_plan_metadata(
                &converted["plan.bin"],
                &converted["plan_revision.bin"],
            )
            .map_err(|error| format!("Repository {}: {error}", entry.repository_index))?;
            converted.insert("plan.bin".into(), plans);
            converted.insert("plan_revision.bin".into(), revisions);
            converted_count += 1;
            report_progress(
                &mut observer,
                WorkflowUpgradePhase::Convert,
                converted_count,
                Some(entries.len()),
            );
            Ok::<Files, String>(converted)
        })
        .collect::<Result<Vec<_>, _>>()?;

    // This temporary native validation view contains metadata only. It is never
    // activated, retained as a backup, or used to resume an interrupted upgrade.
    let outputs = MetadataOutputs::new(&config.generations_root)?;
    for entry in &physical_entries {
        fs::create_dir_all(
            outputs
                .0
                .join("repositories")
                .join(entry.repository_index.to_string()),
        )
        .map_err(|error| error.to_string())?;
    }
    report_progress(
        &mut observer,
        WorkflowUpgradePhase::Stage,
        0,
        Some(before.len()),
    );
    let mut staged_count = 0;
    for file in &before {
        let relative = Path::new(&file.relative_path);
        let parts = relative.iter().collect::<Vec<_>>();
        let (replacement, changed) = if parts.len() == 3 && parts[0] == "repositories" {
            let index = parts[1]
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
                .ok_or("Invalid metadata Repository path")?;
            let position = entries
                .iter()
                .position(|entry| entry.repository_index == index);
            let name = parts[2].to_str().ok_or("Non-UTF8 metadata filename")?;
            if let Some(position) = position {
                if name == "snapshot_link_payload.bin" {
                    staged_count += 1;
                    report_progress(
                        &mut observer,
                        WorkflowUpgradePhase::Stage,
                        staged_count,
                        Some(before.len()),
                    );
                    continue;
                }
                let bytes = &converted_metadata[position][name];
                (Some(bytes), metadata[position].get(name) != Some(bytes))
            } else {
                (None, false)
            }
        } else {
            (None, false)
        };
        let output = outputs.0.join(relative);
        fs::create_dir_all(output.parent().ok_or("Metadata output has no parent")?)
            .map_err(|e| e.to_string())?;
        let original;
        let bytes = match replacement {
            Some(bytes) => bytes,
            None => {
                original = read_regular_file(&source.join(relative))?;
                &original
            }
        };
        fs::write(&output, bytes).map_err(|e| e.to_string())?;
        // Replacement must retain the source's access permissions rather than
        // inheriting the temporary output directory's process umask.
        fs::set_permissions(
            &output,
            fs::metadata(source.join(relative))
                .map_err(|e| e.to_string())?
                .permissions(),
        )
        .map_err(|e| e.to_string())?;
        if changed {
            fs::File::open(&output)
                .and_then(|file| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        staged_count += 1;
        report_progress(
            &mut observer,
            WorkflowUpgradePhase::Stage,
            staged_count,
            Some(before.len()),
        );
    }
    hook("after_stage")?;
    report_progress(&mut observer, WorkflowUpgradePhase::Validate, 0, None);
    validate_target(&outputs.0, &physical_entries, &entries, &global_files)?;
    validate_disposition_target(&outputs.0, &entries, &converted_metadata)?;
    validate_unchanged_files(
        &before,
        &inventory_for_entries(&outputs.0, &entries)?,
        &entries,
    )?;

    // Upgrade the shared view to complete native writer exclusion, then verify
    // stability across that lock transition before replacing any metadata.
    drop(locks);
    drop(queue_locks);
    drop(_global_lock);
    report_progress(
        &mut observer,
        WorkflowUpgradePhase::AwaitPublication,
        0,
        None,
    );
    let _registry_write = process_lock(
        &global.join(REGISTRY_LOCK_FILE_NAME),
        ServerBinaryDbLockMode::Exclusive,
    )?;
    let mut writers = Vec::new();
    let mut queues = Vec::new();
    for entry in &entries {
        let directory = repositories.join(entry.repository_index.to_string());
        queues.push(process_lock(
            &directory.join(WORKER_QUEUE_LOCK_FILE_NAME),
            ServerBinaryDbLockMode::Exclusive,
        )?);
        writers.push(
            BinaryDbCommandLockSet::try_acquire(
                &StorePath::new(directory),
                BinaryDbCommandScope::General,
            )
            .map_err(|e| e.to_string())?,
        );
    }
    hook("before_publish")?;
    if config.activation()?.as_ref() != Some(active)
        || inventory_for_entries(&source, &entries)? != before
    {
        return Err("Source metadata drifted before batch publication".into());
    }
    report_progress(
        &mut observer,
        WorkflowUpgradePhase::Publish,
        0,
        Some(entries.len()),
    );
    for (published_count, ((entry, original), converted)) in entries
        .iter()
        .zip(&metadata)
        .zip(&converted_metadata)
        .enumerate()
    {
        let relative = Path::new("repositories").join(entry.repository_index.to_string());
        let directory = source.join(&relative);
        for (name, bytes) in converted {
            if original.get(name) != Some(bytes) {
                fs::rename(outputs.0.join(&relative).join(name), directory.join(name))
                    .map_err(|e| e.to_string())?;
            }
        }
        if original.contains_key("snapshot_link_payload.bin") {
            fs::remove_file(directory.join("snapshot_link_payload.bin"))
                .map_err(|e| e.to_string())?;
        }
        sync_directory(&directory)?;
        report_progress(
            &mut observer,
            WorkflowUpgradePhase::Publish,
            published_count + 1,
            Some(entries.len()),
        );
    }
    // Existing content roots, activation pointer and creation evidence remain
    // in place. Headers classify completion; partial profiles fail closed.
    // This migration provides no cross-file crash atomicity or restoration.
    report_progress(
        &mut observer,
        WorkflowUpgradePhase::Verify,
        0,
        Some(entries.len()),
    );
    for (verified_count, (entry, expected)) in entries.iter().zip(&converted_metadata).enumerate() {
        if read_authority_files(
            &repositories.join(entry.repository_index.to_string()),
            false,
        )? != *expected
        {
            return Err("Published metadata differs from the validated batch".into());
        }
        report_progress(
            &mut observer,
            WorkflowUpgradePhase::Verify,
            verified_count + 1,
            Some(entries.len()),
        );
    }
    hook("after_publish")?;
    report_progress(
        &mut observer,
        WorkflowUpgradePhase::Complete,
        entries.len(),
        Some(entries.len()),
    );
    Ok(active.clone())
}

pub(super) struct MetadataOutputs(pub(super) PathBuf);
impl MetadataOutputs {
    pub(super) fn new(parent: &Path) -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let root = parent.join(format!(".workflow-metadata-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).map_err(|e| e.to_string())?;
        Ok(Self(root))
    }
}
impl Drop for MetadataOutputs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn derive_admission(
    entries: &[OperationalRepositoryEntry],
    metadata: &[Files],
    source_fingerprint: String,
) -> Result<WorkflowGenerationAdmission, String> {
    let mut admission = WorkflowGenerationAdmission {
        source_fingerprint,
        initial_changes_by_repository: BTreeMap::new(),
        incoming_changes_by_repository: BTreeMap::new(),
        historical_dispositions_by_repository: BTreeMap::new(),
    };
    for (entry, files) in entries.iter().zip(metadata) {
        history::validate_source_namespace(files, &entry.record.namespace_ascii)?;
        let disposition = history::derive_approved_disposition(files)?;
        let changes = records(files, "change.bin", 68)?;
        let mut initial = BTreeSet::new();
        let mut initial_tasks = BTreeSet::new();
        for (index, change) in changes.iter().enumerate() {
            if change[4] == 0 {
                if u32_at(change, 20) != 0 || !initial_tasks.insert(u32_at(change, 16)) {
                    return Err(
                        "Invalid or duplicate first source C01 in old-profile admission".into(),
                    );
                }
                initial.insert(index as u32);
            }
        }
        // Owner-directed old-profile cleanup, independent of title/producer.
        // This path is never reached after current-layout metadata is admitted.
        admission
            .initial_changes_by_repository
            .insert(entry.repository_index, initial);
        admission
            .incoming_changes_by_repository
            .insert(entry.repository_index, BTreeSet::new());
        admission
            .historical_dispositions_by_repository
            .insert(entry.repository_index, disposition);
    }
    Ok(admission)
}

fn shared_lock(path: &Path) -> Result<BoxedServerBinaryDbProcessLockGuard, String> {
    process_lock(path, ServerBinaryDbLockMode::Shared)
}
fn process_lock(
    path: &Path,
    mode: ServerBinaryDbLockMode,
) -> Result<BoxedServerBinaryDbProcessLockGuard, String> {
    ServerBinaryDbFilesystemStore
        .acquire_process_lock(path, mode, ServerBinaryDbLockWait::Nonblocking)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Generation writer is active: {}", path.display()))
}
fn read_global(root: &Path) -> Result<Files, String> {
    canonical_real_directory(root)?;
    let expected = SERVER_GLOBAL_OPERATIONAL_BIN_PATHS
        .iter()
        .chain(SERVER_GLOBAL_OPERATIONAL_INDEX_PATHS)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Non-UTF8 global authority path")?;
        if name == REGISTRY_LOCK_FILE_NAME || name == ".locks" {
            continue;
        }
        if !expected.contains(name.as_str()) {
            return Err(format!("Undeclared global authority {name}"));
        }
        let bytes = read_regular_file(&entry.path())?;
        if bytes.get(..4) != Some(&1u32.to_le_bytes()) {
            return Err(format!("Global authority layout mismatch: {name}"));
        }
        files.insert(name, bytes);
    }
    if files.len() != expected.len() {
        return Err("Incomplete global authority inventory".into());
    }
    Ok(files)
}
fn validate_root(root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Non-UTF8 generation path")?;
        let meta = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        let allowed = if meta.is_dir() {
            ["global", "repositories", ".locks"].contains(&name.as_str())
        } else {
            [
                "generation.json",
                "generation-complete.json",
                "conversion-report.json",
                "conversion-complete.json",
                "plan-lineage-repair-receipt.json",
                "fresh-generation-replay-receipt.json",
            ]
            .contains(&name.as_str())
        };
        if meta.file_type().is_symlink() || !allowed {
            return Err(format!("Undeclared generation authority path {name}"));
        }
    }
    Ok(())
}
fn inventory(root: &Path) -> Result<Vec<SealedFile>, String> {
    inventory_with_scope(root, None)
}
fn inventory_for_entries(
    root: &Path,
    entries: &[OperationalRepositoryEntry],
) -> Result<Vec<SealedFile>, String> {
    inventory_with_scope(
        root,
        Some(&entries.iter().map(|entry| entry.repository_index).collect()),
    )
}
fn inventory_with_scope(
    root: &Path,
    scope: Option<&BTreeSet<u32>>,
) -> Result<Vec<SealedFile>, String> {
    fn walk(
        root: &Path,
        path: &Path,
        files: &mut Vec<SealedFile>,
        scope: Option<&BTreeSet<u32>>,
    ) -> Result<(), String> {
        for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            let name = entry.file_name();
            if relative.components().count() == 2 && relative.starts_with("repositories") {
                let index = name
                    .to_str()
                    .and_then(|name| name.parse::<u32>().ok())
                    .ok_or("Invalid Repository directory")?;
                if scope.is_some_and(|scope| !scope.contains(&index)) {
                    continue;
                }
            }
            if name == ".ait"
                || name == ".locks"
                || name == REGISTRY_LOCK_FILE_NAME
                || name == WORKER_QUEUE_LOCK_FILE_NAME
            {
                continue;
            }
            let meta = fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
            if meta.file_type().is_symlink() {
                return Err("Symlink in generation fingerprint".into());
            }
            if meta.is_dir() {
                walk(root, &entry.path(), files, scope)?;
            } else if meta.is_file() {
                let hash = sha256(&read_regular_file(&entry.path())?);
                files.push(SealedFile {
                    relative_path: relative.to_str().ok_or("Non-UTF8 fingerprint path")?.into(),
                    byte_size: meta.len(),
                    sha256: hash,
                });
            } else {
                return Err("Non-file in generation fingerprint".into());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, root, &mut files, scope)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}
fn inventory_fingerprint(files: &[SealedFile]) -> Result<String, String> {
    Ok(sha256(
        &serde_json::to_vec(files).map_err(|error| error.to_string())?,
    ))
}
fn validate_unchanged_files(
    source: &[SealedFile],
    target: &[SealedFile],
    entries: &[OperationalRepositoryEntry],
) -> Result<(), String> {
    let changed = [
        "task.bin",
        "snapshot.bin",
        "plan.bin",
        "plan_revision.bin",
        "change.bin",
        "change_payload.bin",
        "patchset.bin",
        "land.bin",
        "snapshot_link.bin",
        "snapshot_link_payload.bin",
        "attest.bin",
        "review.bin",
        "policy.bin",
        "waiver.bin",
        "task_change_index.bin",
        "change_patchset_index.bin",
        "change_land_index.bin",
        "change_snapshot_index.bin",
    ];
    for file in source {
        let path = Path::new(&file.relative_path);
        if matches!(
            file.relative_path.as_str(),
            "conversion-report.json" | "conversion-complete.json" | "generation-complete.json"
        ) {
            continue;
        }
        if path.components().count() == 3
            && file.relative_path.starts_with("repositories/")
            && path
                .components()
                .nth(1)
                .and_then(|part| part.as_os_str().to_str())
                .and_then(|index| index.parse::<u32>().ok())
                .is_some_and(|index| entries.iter().any(|entry| entry.repository_index == index))
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| changed.contains(&name))
        {
            continue;
        }
        if !target.contains(file) {
            return Err(format!(
                "Unchanged authority bytes differ after staging: {}",
                file.relative_path
            ));
        }
    }
    Ok(())
}
fn validate_target(
    root: &Path,
    physical_entries: &[OperationalRepositoryEntry],
    entries: &[OperationalRepositoryEntry],
    global: &Files,
) -> Result<(), String> {
    if read_global(&root.join("global"))? != *global {
        return Err("Unchanged global authority bytes differ after workflow conversion".into());
    }
    let registry =
        ServerOperationalRepositoryRegistry::new(root.join("global"), root.join("repositories"))
            .map_err(|error| error.to_string())?;
    if registry.validate().map_err(|error| error.to_string())? != physical_entries {
        return Err("Repository registry identities changed during workflow conversion".into());
    }
    for entry in entries {
        let directory = root
            .join("repositories")
            .join(entry.repository_index.to_string());
        let files = read_authority_files(&directory, false)?;
        if profile(&files)? != WorkflowProfile::Current {
            return Err("Target lacks the complete current workflow profile".into());
        }
        if classify_remote_plan_metadata(&files["plan.bin"], &files["plan_revision.bin"])
            .map_err(|error| error.to_string())?
            != RemotePlanMetadataProfile::Actor
        {
            return Err("Target lacks the complete Actor Plan profile".into());
        }
        let db = FilesystemServerRemoteBinaryDb::serving_authority(
            RepoId::new(entry.repository_index.to_string()),
            RepoName::new(entry.repo_name.clone()),
            StorePath::new(directory.clone()),
            StoreGeneration::new(1),
        );
        validate_frozen_server_workflow_v0(&db)?;
        crate::foundation::server_plan_binary_db::BinaryDbServerPlanService::new(db.clone())
            .validate_metadata()?;

        let authority: Arc<dyn WorkerJobDomainAuthority> =
            Arc::new(FrozenBinaryV0WorkerJobAuthority::new(db));
        let jobs =
            ServerOperationalWorkerJobStore::new(entry.repository_index, directory, authority)
                .map_err(|error| error.to_string())?;
        jobs.validate().map_err(|error| error.to_string())?;
    }
    Ok(())
}
fn validate_disposition_target(
    root: &Path,
    entries: &[OperationalRepositoryEntry],
    converted: &[Files],
) -> Result<(), String> {
    for (entry, expected) in entries.iter().zip(converted) {
        let directory = root
            .join("repositories")
            .join(entry.repository_index.to_string());
        if read_authority_files(&directory, false)? != *expected {
            return Err(format!(
                "Sealed workflow target differs from the explicitly admitted disposition for Repository {}",
                entry.repository_index
            ));
        }
    }
    Ok(())
}
pub fn validate_workflow_completion(root: &Path, bytes: &[u8]) -> Result<(), String> {
    read_report(root, bytes).map(|_| ())
}
fn read_report(root: &Path, bytes: &[u8]) -> Result<BatchReport, String> {
    let completion: Completion =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    if completion.schema != WORKFLOW_UPGRADE_COMPLETION_SCHEMA
        || completion.layout_id != 1
        || completion.status != "validated_inactive"
    {
        return Err("Workflow conversion completion envelope is invalid".into());
    }
    let report_bytes = read_regular_file(&root.join(REPORT_FILE))?;
    if sha256(&report_bytes) != completion.report_sha256 {
        return Err("Workflow conversion report changed after completion".into());
    }
    let report: BatchReport =
        serde_json::from_slice(&report_bytes).map_err(|error| error.to_string())?;
    if report.schema != REPORT_SCHEMA
        || report.layout_id != 1
        || report.status != "validated_inactive"
        || report.source_selector != SOURCE_SELECTOR
        || report.target_selector != TARGET_SELECTOR
    {
        return Err("Workflow conversion report profile is invalid".into());
    }
    let manifest = read_regular_file(&root.join("generation.json"))?;
    if !report.authority_files.iter().any(|file| {
        file.relative_path == "generation.json"
            && file.sha256 == sha256(&manifest)
            && file.byte_size == manifest.len() as u64
    }) {
        return Err("Workflow conversion generation manifest differs from its seal".into());
    }
    Ok(report)
}

#[cfg(test)]
mod tests;
