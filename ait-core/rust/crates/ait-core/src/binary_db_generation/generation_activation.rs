use super::generation_content_closure::validate_repository_content_closure;
use super::{GenerationResult, Path, PathBuf};
use crate::binary_db::{
    AuthorityId, BinaryDbCommandLockSet, BinaryDbCommandScope, BinaryDbReadLockSet,
    LocalBinaryDbFs, LocalStateScope, StorePath, REPOSITORY_BINARY_DB_BIN_PATHS,
    REPOSITORY_BINARY_DB_INDEX_PATHS,
};
use crate::workflow_binary_db::{
    BinaryDbWorkflowStore, BINARY_DB_WORKFLOW_LAYOUT_ID, CHANGE_LAND_INDEX_BIN,
    CHANGE_LAND_INDEX_RECORD_SIZE, CHANGE_PAYLOAD_BIN, CHANGE_RECORD_BIN, LAND_RECORD_BIN,
    LOCAL_CHANGE_RECORD_SIZE, LOCAL_LAND_RECORD_SIZE, LOCAL_TASK_RECORD_SIZE,
    TASK_CHANGE_INDEX_BIN, TASK_CHANGE_INDEX_RECORD_SIZE, TASK_LAND_INDEX_BIN,
    TASK_LAND_INDEX_RECORD_SIZE, TASK_PAYLOAD_BIN, TASK_RECORD_BIN,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::os::unix::ffi::OsStrExt;
use std::time::{SystemTime, UNIX_EPOCH};

const CLIENT_MANIFEST: &str = "client-manifest.json";
pub(super) const CLIENT_MANIFEST_SCHEMA: &str = "ait.binary-db-local-generation-manifest.v2";
const ACTIVATION_LOCK_ROOT: &str = ".binary-db-generation-activation";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbGenerationActivationOptions {
    pub repo_root: PathBuf,
    pub generation_root: PathBuf,
    pub expected_current_authority_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BinaryDbGenerationActivationReport {
    #[serde(skip)]
    pub repo_root: PathBuf,
    #[serde(skip)]
    pub generation_root: PathBuf,
    pub repo_name: String,
    pub authority_root: String,
    pub pack_root: String,
    pub content_fingerprint: String,
    pub retired_direct_authority: bool,
    pub retained_previous_authority: Option<String>,
    pub activation_strategy: String,
    pub single_syscall_atomic: bool,
    pub activation_lock_protected: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivatedBinaryDbGeneration {
    pub generation_root: PathBuf,
    pub authority_root: PathBuf,
    pub pack_root: PathBuf,
    pub repo_name: String,
    pub content_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ClientManifest {
    schema: String,
    label: String,
    worker_count: usize,
    layout_ids: BTreeMap<String, u32>,
    source_authority_fingerprint: String,
    content_fingerprint: String,
    files: Vec<ClientManifestFile>,
    validation: ClientManifestValidation,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct ClientManifestFile {
    pub(super) relative_path: String,
    pub(super) byte_size: u64,
    pub(super) sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ClientManifestValidation {
    status: String,
}

pub(super) struct VerifiedGeneration {
    pub(super) generation_root: PathBuf,
    pub(super) source_authority_fingerprint: String,
    pub(super) content_fingerprint: String,
    pub(super) files: Vec<ClientManifestFile>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthorityReplacementStrategy {
    AtomicExchange,
    ActivationLockTwoPhaseRename,
    RenameIntoEmpty,
}

impl AuthorityReplacementStrategy {
    fn persisted_name(self) -> &'static str {
        match self {
            Self::AtomicExchange => "atomic_exchange",
            Self::ActivationLockTwoPhaseRename => "activation_lock_two_phase_rename",
            Self::RenameIntoEmpty => "rename_into_empty",
        }
    }

    fn single_syscall_atomic(self) -> bool {
        matches!(self, Self::AtomicExchange | Self::RenameIntoEmpty)
    }
}

#[derive(Debug)]
struct AuthorityReplacement {
    strategy: AuthorityReplacementStrategy,
    retired_authority: Option<PathBuf>,
}

pub fn activate_binary_db_generation(
    options: BinaryDbGenerationActivationOptions,
) -> GenerationResult<BinaryDbGenerationActivationReport> {
    let repo_root = options.repo_root.canonicalize().map_err(|error| {
        format!(
            "invalid repository root {}: {error}",
            options.repo_root.display()
        )
    })?;
    let repo_name = configured_repo_name(&repo_root)?;
    let authority = repo_root.join(".ait/binary-db");
    let _activation_lock = BinaryDbCommandLockSet::acquire(
        &binary_db_activation_lock_root(&repo_root),
        BinaryDbCommandScope::General,
    )
    .map_err(|error| format!("failed to acquire Binary DB activation lock: {error}"))?;
    let (generation_root, captured_source_fingerprint) =
        read_generation_activation_precondition(&options.generation_root, &repo_name)?;
    reject_generation_inside_retired_authority(&authority, &generation_root)?;
    let expected_current_fingerprint = options
        .expected_current_authority_fingerprint
        .as_deref()
        .unwrap_or(&captured_source_fingerprint);
    if !is_sha256(expected_current_fingerprint) {
        return Err(
            "Binary DB activation expected_current_authority_fingerprint is not SHA-256"
                .to_string(),
        );
    }
    let current_authority_fingerprint = fingerprint_direct_authority_state(&authority)?;
    if current_authority_fingerprint != expected_current_fingerprint {
        return Err(format!(
            "stale Binary DB generation: current authority fingerprint changed after capture: expected={expected_current_fingerprint} actual={current_authority_fingerprint}"
        ));
    }
    let verified = verify_generation(&generation_root, &repo_name)?;
    if verified.source_authority_fingerprint != captured_source_fingerprint {
        return Err(
            "Binary DB generation source fingerprint changed during activation verification"
                .to_string(),
        );
    }

    let ait_root = repo_root.join(".ait");
    fs::create_dir_all(&ait_root)
        .map_err(|error| format!("failed to create {}: {error}", ait_root.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let staged_authority = ait_root.join(format!(
        ".binary-db.activate-{}-{nonce}",
        std::process::id()
    ));
    install_verified_packs(&repo_root, &verified, nonce)?;
    stage_direct_authority(&staged_authority, &verified)?;

    let previous = match fs::symlink_metadata(&authority) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            let _ = remove_path(&staged_authority);
            return Err(format!(
                "failed to inspect Binary DB authority {}: {error}",
                authority.display()
            ));
        }
    };
    let replacement = replace_authority(
        &authority,
        &staged_authority,
        previous.is_some(),
        &ait_root,
        nonce,
    );
    let replacement = match replacement {
        Ok(replacement) => replacement,
        Err(error) => {
            let _ = remove_path(&staged_authority);
            return Err(format!(
                "failed to replace direct Binary DB authority {}: {error}",
                authority.display()
            ));
        }
    };
    if let Err(error) = sync_directory(&ait_root) {
        let rollback =
            rollback_authority_replacement(&authority, &replacement, previous.is_some(), &ait_root);
        return Err(activation_failure(
            format!("failed to sync activated direct Binary DB authority: {error}"),
            rollback,
        ));
    }

    if let Err(error) = admit_activated_binary_db_generation(&authority, &repo_name) {
        let rollback =
            rollback_authority_replacement(&authority, &replacement, previous.is_some(), &ait_root);
        return Err(activation_failure(
            format!("activated direct Binary DB authority failed bounded admission: {error}"),
            rollback,
        ));
    }

    let retired_direct_authority = previous
        .as_ref()
        .is_some_and(|metadata| metadata.file_type().is_dir());
    let retained_previous_authority = replacement
        .retired_authority
        .as_ref()
        .map(|path| path_text(path))
        .transpose()?;

    let pack_root = path_text(&repo_root.join(".ait/objects"))?;
    Ok(BinaryDbGenerationActivationReport {
        repo_root,
        generation_root: verified.generation_root,
        repo_name,
        authority_root: path_text(&authority)?,
        pack_root,
        content_fingerprint: verified.content_fingerprint,
        retired_direct_authority,
        retained_previous_authority,
        activation_strategy: replacement.strategy.persisted_name().to_string(),
        single_syscall_atomic: replacement.strategy.single_syscall_atomic(),
        activation_lock_protected: true,
    })
}

fn replace_authority(
    authority: &Path,
    staged_authority: &Path,
    had_previous: bool,
    ait_root: &Path,
    nonce: u128,
) -> GenerationResult<AuthorityReplacement> {
    if !had_previous {
        fs::rename(staged_authority, authority).map_err(|error| error.to_string())?;
        return Ok(AuthorityReplacement {
            strategy: AuthorityReplacementStrategy::RenameIntoEmpty,
            retired_authority: None,
        });
    }
    match atomic_exchange(staged_authority, authority) {
        Ok(()) => Ok(AuthorityReplacement {
            strategy: AuthorityReplacementStrategy::AtomicExchange,
            retired_authority: Some(staged_authority.to_path_buf()),
        }),
        Err(error) if atomic_exchange_is_unsupported(&error) => {
            let retired_authority =
                ait_root.join(format!(".binary-db.retired-{}-{nonce}", std::process::id()));
            fs::rename(authority, &retired_authority).map_err(|rename_error| {
                format!(
                    "atomic exchange is unsupported ({error}); failed to stage the previous authority for lock-protected replacement: {rename_error}"
                )
            })?;
            if let Err(rename_error) = fs::rename(staged_authority, authority) {
                let restore = fs::rename(&retired_authority, authority);
                return Err(match restore {
                    Ok(()) => format!(
                        "atomic exchange is unsupported ({error}); lock-protected replacement failed: {rename_error}; previous authority restored"
                    ),
                    Err(restore_error) => format!(
                        "atomic exchange is unsupported ({error}); lock-protected replacement failed: {rename_error}; previous authority restore failed: {restore_error}"
                    ),
                });
            }
            Ok(AuthorityReplacement {
                strategy: AuthorityReplacementStrategy::ActivationLockTwoPhaseRename,
                retired_authority: Some(retired_authority),
            })
        }
        Err(error) => Err(error.to_string()),
    }
}

fn atomic_exchange_is_unsupported(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::Unsupported {
        return true;
    }
    error
        .raw_os_error()
        .is_some_and(|code| [libc::ENOTSUP, libc::EOPNOTSUPP, libc::ENOSYS].contains(&code))
}

/*
 * Keep replacement and rollback under the activation command lock. A two-rename
 * filesystem fallback is therefore invisible to conforming runtime admission,
 * but it is reported distinctly from a single-syscall exchange.
 */
fn rollback_authority_replacement(
    authority: &Path,
    replacement: &AuthorityReplacement,
    had_previous: bool,
    ait_root: &Path,
) -> GenerationResult<()> {
    if had_previous {
        let retired_authority = replacement
            .retired_authority
            .as_ref()
            .ok_or_else(|| "authority replacement lost its previous authority path".to_string())?;
        match replacement.strategy {
            AuthorityReplacementStrategy::AtomicExchange => {
                atomic_exchange(retired_authority, authority).map_err(|error| {
                    format!(
                        "failed to restore previous direct Binary DB authority {}: {error}",
                        authority.display()
                    )
                })?;
                remove_path(retired_authority).map_err(|error| {
                    format!(
                        "failed to remove rolled-back Binary DB authority {}: {error}",
                        retired_authority.display()
                    )
                })?;
            }
            AuthorityReplacementStrategy::ActivationLockTwoPhaseRename => {
                remove_path(authority).map_err(|error| {
                    format!(
                        "failed to remove rejected Binary DB authority {}: {error}",
                        authority.display()
                    )
                })?;
                fs::rename(retired_authority, authority).map_err(|error| {
                    format!(
                        "failed to restore previous direct Binary DB authority {}: {error}",
                        authority.display()
                    )
                })?;
            }
            AuthorityReplacementStrategy::RenameIntoEmpty => {
                return Err("replacement strategy lost previous authority state".to_string())
            }
        }
    } else {
        remove_path(authority).map_err(|error| {
            format!(
                "failed to remove rolled-back Binary DB authority {}: {error}",
                authority.display()
            )
        })?;
    }
    sync_directory(ait_root)
}

pub fn binary_db_activation_lock_root(repo_root: &Path) -> StorePath {
    StorePath::from(repo_root.join(".ait").join(ACTIVATION_LOCK_ROOT))
}

pub fn admit_activated_binary_db_generation(
    authority: &Path,
    expected_repo_name: &str,
) -> GenerationResult<ActivatedBinaryDbGeneration> {
    let metadata = fs::symlink_metadata(authority).map_err(|error| {
        format!(
            "direct Binary DB authority {} is missing: {error}",
            authority.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Binary DB authority {} must be a direct directory; symbolic links are forbidden",
            authority.display()
        ));
    }
    if !metadata.file_type().is_dir() {
        return Err(format!(
            "Binary DB authority {} must be a direct directory",
            authority.display()
        ));
    }
    if authority.file_name().and_then(|value| value.to_str()) != Some("binary-db")
        || authority
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            != Some(".ait")
    {
        return Err(format!(
            "Binary DB authority must be the repository .ait/binary-db direct directory: {}",
            authority.display()
        ));
    }
    let authority_root = authority.canonicalize().map_err(|error| {
        format!(
            "direct Binary DB authority {} cannot be resolved: {error}",
            authority.display()
        )
    })?;
    let ait_root = authority_root
        .parent()
        .ok_or_else(|| "direct Binary DB authority has no .ait parent".to_string())?;
    let generation_root = ait_root
        .parent()
        .ok_or_else(|| "direct Binary DB authority has no repository root".to_string())?
        .to_path_buf();
    let configured_name = configured_repo_name(&generation_root)?;
    if configured_name != expected_repo_name {
        return Err(format!(
            "Binary DB authority repository mismatch: expected {expected_repo_name:?}, got {configured_name:?}"
        ));
    }
    let pack_root = generation_root.join(".ait/objects");
    let pack_metadata = fs::symlink_metadata(&pack_root).map_err(|error| {
        format!(
            "repository Binary DB pack authority {} is missing: {error}",
            pack_root.display()
        )
    })?;
    if pack_metadata.file_type().is_symlink() || !pack_metadata.file_type().is_dir() {
        return Err(format!(
            "repository Binary DB pack authority {} must be a direct directory",
            pack_root.display()
        ));
    }
    let content_fingerprint = validate_direct_authority(&authority_root)?;
    Ok(ActivatedBinaryDbGeneration {
        generation_root,
        authority_root,
        pack_root,
        repo_name: configured_name,
        content_fingerprint,
    })
}

pub fn admit_activated_binary_db_generation_for_runtime(
    repo_root: &Path,
    authority: &Path,
    expected_repo_name: &str,
) -> GenerationResult<(
    ActivatedBinaryDbGeneration,
    std::sync::Arc<std::sync::Mutex<BinaryDbReadLockSet>>,
)> {
    let guard = BinaryDbReadLockSet::try_acquire(&binary_db_activation_lock_root(repo_root))
        .map_err(|error| format!("Binary DB generation activation is active: {error}"))?;
    crate::plan_binary_db::repair_plan_binary_db_authority_if_needed(authority)
        .map_err(|error| format!("Selected Binary DB Plan recovery failed: {error}"))?;
    let generation = admit_activated_binary_db_generation(authority, expected_repo_name)?;
    Ok((
        generation,
        std::sync::Arc::new(std::sync::Mutex::new(guard)),
    ))
}

pub fn snapshot_binary_db_authority_fingerprint(repo_root: &Path) -> GenerationResult<String> {
    let repo_root = repo_root
        .canonicalize()
        .map_err(|error| format!("invalid repository root {}: {error}", repo_root.display()))?;
    let _activation_guard =
        BinaryDbReadLockSet::try_acquire(&binary_db_activation_lock_root(&repo_root))
            .map_err(|error| format!("Binary DB generation activation is active: {error}"))?;
    let authority_root = repo_root.join(".ait/binary-db");
    if !authority_root.exists() {
        return fingerprint_direct_authority_state(&authority_root);
    }
    let db = LocalBinaryDbFs::new(
        authority_root.clone(),
        repo_root,
        AuthorityId::new("authority-fingerprint"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS);
    let read = db.begin_read_txn();
    read.read_lock_paths().map_err(|error| {
        format!("cannot fingerprint Binary DB authority while a writer is active: {error}")
    })?;
    fingerprint_direct_authority_state(&authority_root)
}

fn read_generation_activation_precondition(
    generation_root: &Path,
    expected_repo_name: &str,
) -> GenerationResult<(PathBuf, String)> {
    let generation_root = generation_root.canonicalize().map_err(|error| {
        format!(
            "invalid Binary DB generation root {}: {error}",
            generation_root.display()
        )
    })?;
    let manifest_path = generation_root.join(CLIENT_MANIFEST);
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
    let manifest: ClientManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;
    if manifest.schema != CLIENT_MANIFEST_SCHEMA {
        return Err(format!(
            "Binary DB generation manifest schema is unsupported: {:?}",
            manifest.schema
        ));
    }
    if manifest.label != expected_repo_name {
        return Err(format!(
            "Binary DB generation repository mismatch: expected {expected_repo_name:?}, got {:?}",
            manifest.label
        ));
    }
    if !is_sha256(&manifest.source_authority_fingerprint) {
        return Err("Binary DB generation source_authority_fingerprint is not SHA-256".to_string());
    }
    Ok((generation_root, manifest.source_authority_fingerprint))
}

pub(super) fn verify_generation(
    generation_root: &Path,
    expected_repo_name: &str,
) -> GenerationResult<VerifiedGeneration> {
    let generation_root = generation_root.canonicalize().map_err(|error| {
        format!(
            "invalid Binary DB generation root {}: {error}",
            generation_root.display()
        )
    })?;
    let authority_root = generation_root.join("local");
    let pack_root = generation_root.join(".ait/objects");
    if !authority_root.is_dir() {
        return Err(format!(
            "Binary DB generation has no local authority: {}",
            authority_root.display()
        ));
    }
    if !pack_root.is_dir() {
        return Err(format!(
            "Binary DB generation has no pack authority: {}",
            pack_root.display()
        ));
    }
    let manifest_path = generation_root.join(CLIENT_MANIFEST);
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
    let manifest: ClientManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;
    if manifest.schema != CLIENT_MANIFEST_SCHEMA {
        return Err(format!(
            "Binary DB generation manifest schema is unsupported: {:?}",
            manifest.schema
        ));
    }
    if manifest.label != expected_repo_name {
        return Err(format!(
            "Binary DB generation repository mismatch: expected {expected_repo_name:?}, got {:?}",
            manifest.label
        ));
    }
    if manifest.worker_count == 0 {
        return Err("Binary DB generation worker_count must be at least 1".to_string());
    }
    if manifest.layout_ids != required_layout_ids() {
        return Err(format!(
            "Binary DB generation layout map is unsupported: {:?}",
            manifest.layout_ids
        ));
    }
    if manifest.validation.status != "passed" {
        return Err(format!(
            "Binary DB generation validation did not pass: {:?}",
            manifest.validation.status
        ));
    }
    if !is_sha256(&manifest.content_fingerprint) {
        return Err("Binary DB generation content_fingerprint is not SHA-256".to_string());
    }
    if !is_sha256(&manifest.source_authority_fingerprint) {
        return Err("Binary DB generation source_authority_fingerprint is not SHA-256".to_string());
    }

    let mut declared_paths = BTreeSet::new();
    let mut previous = None::<&str>;
    for file in &manifest.files {
        validate_generation_file_path(&file.relative_path)?;
        if previous.is_some_and(|value| value >= file.relative_path.as_str()) {
            return Err("Binary DB generation file inventory must be strictly sorted".to_string());
        }
        previous = Some(&file.relative_path);
        if !declared_paths.insert(file.relative_path.clone()) {
            return Err(format!(
                "duplicate Binary DB generation file inventory path: {:?}",
                file.relative_path
            ));
        }
        if !is_sha256(&file.sha256) {
            return Err(format!(
                "Binary DB generation file has invalid SHA-256: {:?}",
                file.relative_path
            ));
        }
        let path = generation_root.join(&file.relative_path);
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "Binary DB generation manifest path {} is missing: {error}",
                path.display()
            )
        })?;
        if !metadata.file_type().is_file() || metadata.len() != file.byte_size {
            return Err(format!(
                "Binary DB generation file size/type mismatch at {}",
                path.display()
            ));
        }
        if file.relative_path.starts_with("local/")
            && file.relative_path.ends_with(".bin")
            && file.byte_size <= 4
        {
            return Err(format!(
                "Binary DB generation contains forbidden header-only file {:?}",
                file.relative_path
            ));
        }
        if file.relative_path.starts_with("local/") {
            validate_local_binary_file(&path, &file.relative_path, file.byte_size)?;
        }
        let actual_sha256 = sha256_file(&path)?;
        if actual_sha256 != file.sha256 {
            return Err(format!(
                "Binary DB generation checksum mismatch at {}",
                path.display()
            ));
        }
    }
    let actual_paths = collect_generation_paths(&generation_root)?;
    if actual_paths != declared_paths {
        let undeclared = actual_paths.difference(&declared_paths).next().cloned();
        let missing = declared_paths.difference(&actual_paths).next().cloned();
        return Err(format!(
            "Binary DB generation file inventory mismatch: undeclared={undeclared:?} missing={missing:?}"
        ));
    }
    let authority_validation: GenerationResult<()> = (|| {
        validate_generation_workflow_authority(
            &authority_root,
            &generation_root,
            expected_repo_name,
        )?;
        validate_repository_content_closure(&authority_root, &generation_root)?;
        Ok(())
    })();
    let validation_lock_root = authority_root.join(".locks");
    if validation_lock_root.exists() {
        return Err(format!(
            "detached generation verification unexpectedly created runtime locks at {}",
            validation_lock_root.display()
        ));
    }
    authority_validation?;
    let calculated_fingerprint = fingerprint_files(&manifest.files);
    if calculated_fingerprint != manifest.content_fingerprint {
        return Err(format!(
            "Binary DB generation fingerprint mismatch: manifest={} calculated={calculated_fingerprint}",
            manifest.content_fingerprint
        ));
    }
    Ok(VerifiedGeneration {
        generation_root,
        source_authority_fingerprint: manifest.source_authority_fingerprint,
        content_fingerprint: manifest.content_fingerprint,
        files: manifest.files,
    })
}

fn validate_generation_workflow_authority(
    authority_root: &Path,
    generation_root: &Path,
    expected_repo_name: &str,
) -> GenerationResult<()> {
    let task_pair = (
        authority_root.join(TASK_RECORD_BIN).is_file(),
        authority_root.join(TASK_PAYLOAD_BIN).is_file(),
    );
    let change_pair = (
        authority_root.join(CHANGE_RECORD_BIN).is_file(),
        authority_root.join(CHANGE_PAYLOAD_BIN).is_file(),
    );
    for (kind, pair) in [("Task", task_pair), ("Change", change_pair)] {
        if pair.0 != pair.1 {
            return Err(format!(
                "Binary DB generation has an incomplete canonical {kind} record/payload pair"
            ));
        }
    }
    if !task_pair.0 && !change_pair.0 {
        return Ok(());
    }

    let db = LocalBinaryDbFs::new(
        authority_root.to_path_buf(),
        generation_root.to_path_buf(),
        AuthorityId::new(format!("generation-workflow:{expected_repo_name}")),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(REPOSITORY_BINARY_DB_INDEX_PATHS)
    .for_detached_generation_without_locks();
    BinaryDbWorkflowStore::<_, BINARY_DB_WORKFLOW_LAYOUT_ID>::new(db, expected_repo_name)
        .validate_detached_authority()
        .map(|_| ())
        .map_err(|error| format!("Binary DB generation workflow validation failed: {error}"))
}

pub(super) fn validate_generation_file_path(relative_path: &str) -> GenerationResult<()> {
    let path = Path::new(relative_path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "unsafe Binary DB generation manifest path: {relative_path:?}"
        ));
    }
    let components = path.components().count();
    if let Some(local_path) = relative_path.strip_prefix("local/") {
        if components != 2 {
            return Err(format!(
                "Binary DB authority paths must be direct schema leaves: {relative_path:?}"
            ));
        }
        let allowed = if local_path.ends_with(".bin") {
            REPOSITORY_BINARY_DB_BIN_PATHS.contains(&local_path)
        } else if local_path.ends_with(".idx") {
            REPOSITORY_BINARY_DB_INDEX_PATHS.contains(&local_path)
        } else {
            false
        };
        if !allowed {
            return Err(format!(
                "Binary DB generation contains an undeclared authority path: {relative_path:?}"
            ));
        }
        return Ok(());
    }
    if let Some(pack_path) = relative_path.strip_prefix(".ait/objects/") {
        if !is_canonical_pack_path(pack_path) {
            return Err(format!(
                "Binary DB generation contains an undeclared pack path: {relative_path:?}"
            ));
        }
        return Ok(());
    }
    Err(format!(
        "Binary DB generation path is outside local/ and .ait/objects/: {relative_path:?}"
    ))
}

fn is_canonical_pack_path(value: &str) -> bool {
    let (prefix, id_prefix) = if let Some(name) = value.strip_prefix("packs/") {
        (name, "PCK-")
    } else if let Some(name) = value.strip_prefix("tree-packs/") {
        (name, "TPK-")
    } else {
        return false;
    };
    let Some(id) = prefix.strip_suffix(".zstpack") else {
        return false;
    };
    let Some(hex) = id.strip_prefix(id_prefix) else {
        return false;
    };
    hex.len() == 12
        && hex
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'A'..=b'F').contains(&value))
}

fn collect_generation_paths(generation_root: &Path) -> GenerationResult<BTreeSet<String>> {
    let mut paths = BTreeSet::new();
    collect_paths_under(generation_root, &generation_root.join("local"), &mut paths)?;
    collect_paths_under(
        generation_root,
        &generation_root.join(".ait/objects"),
        &mut paths,
    )?;
    Ok(paths)
}

fn collect_paths_under(
    generation_root: &Path,
    current: &Path,
    paths: &mut BTreeSet<String>,
) -> GenerationResult<()> {
    let mut entries = fs::read_dir(current)
        .map_err(|error| format!("failed to inventory {}: {error}", current.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inventory {}: {error}", current.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "Binary DB generation unexpectedly contains a symlink: {}",
                path.display()
            ));
        }
        if file_type.is_dir() {
            collect_paths_under(generation_root, &path, paths)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(generation_root)
                .map_err(|_| format!("generation path escaped root: {}", path.display()))?
                .to_str()
                .ok_or_else(|| format!("generation path is not UTF-8: {}", path.display()))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            validate_generation_file_path(&relative)?;
            paths.insert(relative);
        } else {
            return Err(format!(
                "Binary DB generation contains a non-file entry: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn install_verified_packs(
    repo_root: &Path,
    verified: &VerifiedGeneration,
    nonce: u128,
) -> GenerationResult<()> {
    let repo_pack_root = repo_root.join(".ait/objects");
    ensure_direct_directory(&repo_pack_root)?;
    let mut touched_directories = BTreeSet::new();
    for (index, file) in verified
        .files
        .iter()
        .filter(|file| file.relative_path.starts_with(".ait/objects/"))
        .enumerate()
    {
        let source = verified.generation_root.join(&file.relative_path);
        let destination = repo_root.join(&file.relative_path);
        let parent = destination
            .parent()
            .ok_or_else(|| format!("pack path has no parent: {}", destination.display()))?;
        ensure_direct_directory(parent)?;
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                if !metadata.file_type().is_file()
                    || metadata.len() != file.byte_size
                    || sha256_file(&destination)? != file.sha256
                {
                    return Err(format!(
                        "repository pack authority conflicts with verified generation at {}",
                        destination.display()
                    ));
                }
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect repository pack {}: {error}",
                    destination.display()
                ))
            }
        }
        let file_name = destination
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("pack path is not UTF-8: {}", destination.display()))?;
        let staged = parent.join(format!(
            ".{file_name}.activate-{}-{nonce}-{index}",
            std::process::id()
        ));
        copy_verified_file(&source, &staged, file)?;
        fs::rename(&staged, &destination).map_err(|error| {
            let _ = remove_path(&staged);
            format!(
                "failed to publish repository pack {}: {error}",
                destination.display()
            )
        })?;
        touched_directories.insert(parent.to_path_buf());
    }
    for directory in touched_directories {
        sync_directory(&directory)?;
    }
    sync_directory(&repo_pack_root)
}

fn stage_direct_authority(
    staged_authority: &Path,
    verified: &VerifiedGeneration,
) -> GenerationResult<()> {
    fs::create_dir(staged_authority).map_err(|error| {
        format!(
            "failed to create direct Binary DB staging authority {}: {error}",
            staged_authority.display()
        )
    })?;
    let result = (|| {
        for file in verified
            .files
            .iter()
            .filter(|file| file.relative_path.starts_with("local/"))
        {
            let source = verified.generation_root.join(&file.relative_path);
            let file_name = file
                .relative_path
                .strip_prefix("local/")
                .ok_or_else(|| "verified local path lost its prefix".to_string())?;
            copy_verified_file(&source, &staged_authority.join(file_name), file)?;
        }
        sync_directory(staged_authority)
    })();
    if result.is_err() {
        let _ = remove_path(staged_authority);
    }
    result
}

fn copy_verified_file(
    source: &Path,
    destination: &Path,
    expected: &ClientManifestFile,
) -> GenerationResult<()> {
    let mut input = fs::File::open(source)
        .map_err(|error| format!("failed to open {}: {error}", source.display()))?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .map_err(|error| format!("failed to create {}: {error}", destination.display()))?;
    let mut hasher = Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("failed to read {}: {error}", source.display()))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("failed to write {}: {error}", destination.display()))?;
        hasher.update(&buffer[..read]);
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| format!("copied byte count overflow at {}", source.display()))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("failed to sync {}: {error}", destination.display()))?;
    let actual_sha256 = hex_lower(&hasher.finalize());
    if copied != expected.byte_size || actual_sha256 != expected.sha256 {
        let _ = remove_path(destination);
        return Err(format!(
            "verified generation changed during activation copy at {}",
            source.display()
        ));
    }
    Ok(())
}

fn ensure_direct_directory(path: &Path) -> GenerationResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(format!(
            "Binary DB directory {} must be a direct directory",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .ok_or_else(|| format!("directory has no parent: {}", path.display()))?;
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|error| {
                    format!("failed to create directory {}: {error}", parent.display())
                })?;
            }
            fs::create_dir(path)
                .map_err(|error| format!("failed to create directory {}: {error}", path.display()))
        }
        Err(error) => Err(format!("failed to inspect {}: {error}", path.display())),
    }
}

pub(super) fn fingerprint_direct_authority_contents(
    authority_root: &Path,
) -> GenerationResult<String> {
    let metadata = fs::symlink_metadata(authority_root).map_err(|error| {
        format!(
            "failed to inspect Binary DB authority {}: {error}",
            authority_root.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(format!(
            "Binary DB authority {} must be a direct directory for fingerprinting",
            authority_root.display()
        ));
    }
    let mut entries = fs::read_dir(authority_root)
        .map_err(|error| format!("failed to inventory {}: {error}", authority_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inventory {}: {error}", authority_root.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.path();
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| format!("Binary DB authority path is not UTF-8: {}", path.display()))?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_dir() && name == ".locks" {
            continue;
        }
        if !metadata.file_type().is_file() {
            return Err(format!(
                "Binary DB authority fingerprint encountered a non-file entry: {}",
                path.display()
            ));
        }
        files.push(ClientManifestFile {
            relative_path: name,
            byte_size: metadata.len(),
            sha256: sha256_file(&path)?,
        });
    }
    Ok(fingerprint_files(&files))
}

fn fingerprint_direct_authority_state(authority_root: &Path) -> GenerationResult<String> {
    match fs::symlink_metadata(authority_root) {
        Ok(_) => fingerprint_direct_authority_contents(authority_root),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(hex_lower(
            &Sha256::digest(b"ait.binary-db-authority-state.v1\0absent"),
        )),
        Err(error) => Err(format!(
            "failed to inspect Binary DB authority {}: {error}",
            authority_root.display()
        )),
    }
}

fn validate_direct_authority(authority_root: &Path) -> GenerationResult<String> {
    let mut entries = fs::read_dir(authority_root)
        .map_err(|error| format!("failed to inventory {}: {error}", authority_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inventory {}: {error}", authority_root.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut hasher = Sha256::new();
    for entry in entries {
        let path = entry.path();
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| format!("Binary DB authority path is not UTF-8: {}", path.display()))?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Binary DB authority contains a forbidden symbolic link: {}",
                path.display()
            ));
        }
        if metadata.file_type().is_dir() {
            if name != ".locks" {
                return Err(format!(
                    "Binary DB authority contains an undeclared directory: {}",
                    path.display()
                ));
            }
            validate_lock_directory(&path)?;
            continue;
        }
        if !metadata.file_type().is_file() {
            return Err(format!(
                "Binary DB authority contains a non-file entry: {}",
                path.display()
            ));
        }
        let allowed = if name.ends_with(".bin") {
            REPOSITORY_BINARY_DB_BIN_PATHS.contains(&name.as_str())
        } else if name.ends_with(".idx") {
            REPOSITORY_BINARY_DB_INDEX_PATHS.contains(&name.as_str())
        } else {
            false
        };
        if !allowed {
            return Err(format!(
                "Binary DB authority contains an undeclared schema path: {}",
                path.display()
            ));
        }
        if name.ends_with(".bin") && metadata.len() <= 4 {
            return Err(format!(
                "Binary DB authority contains forbidden header-only file {name:?}"
            ));
        }
        validate_local_binary_file(&path, &format!("local/{name}"), metadata.len())?;
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(metadata.len().to_le_bytes());
        hasher.update([0]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn validate_lock_directory(lock_root: &Path) -> GenerationResult<()> {
    let binary_root = lock_root.join("binary-db");
    let mut root_entries = fs::read_dir(lock_root)
        .map_err(|error| format!("failed to inspect {}: {error}", lock_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect {}: {error}", lock_root.display()))?;
    if root_entries.is_empty() {
        return Ok(());
    }
    if root_entries.len() != 1
        || root_entries
            .pop()
            .is_none_or(|entry| entry.path() != binary_root)
    {
        return Err(format!(
            "Binary DB authority contains an undeclared lock directory entry under {}",
            lock_root.display()
        ));
    }
    let metadata = fs::symlink_metadata(&binary_root)
        .map_err(|error| format!("failed to inspect {}: {error}", binary_root.display()))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return Err(format!(
            "Binary DB lock root {} must be a direct directory",
            binary_root.display()
        ));
    }
    const ACTIVE_LOCK_FILES: &[&str] = &[
        "content.write.lock",
        "gc.write.lock",
        "global.write.lock",
        "plan.write.lock",
        "remote-content.write.lock",
        "remote-plan.write.lock",
        "snapshot.write.lock",
    ];
    let entries = fs::read_dir(&binary_root)
        .map_err(|error| format!("failed to inspect {}: {error}", binary_root.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("failed to inspect {}: {error}", binary_root.display()))?;
    if entries.len() > ACTIVE_LOCK_FILES.len() {
        return Err(format!(
            "Binary DB lock root contains too many entries: {}",
            binary_root.display()
        ));
    }
    for entry in entries {
        let path = entry.path();
        let name = entry
            .file_name()
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| format!("Binary DB lock path is not UTF-8: {}", path.display()))?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
        if !ACTIVE_LOCK_FILES.contains(&name.as_str()) || !metadata.file_type().is_file() {
            return Err(format!(
                "Binary DB lock root contains an undeclared entry: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

pub(super) fn configured_repo_name(repo_root: &Path) -> GenerationResult<String> {
    let config_path = repo_root.join(".ait/config.json");
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(&config_path)
            .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?,
    )
    .map_err(|error| format!("failed to parse {}: {error}", config_path.display()))?;
    value
        .get("repo_name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{} has no non-empty repo_name", config_path.display()))
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

fn fingerprint_files(files: &[ClientManifestFile]) -> String {
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
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
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

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_local_binary_file(
    path: &Path,
    relative_path: &str,
    byte_size: u64,
) -> GenerationResult<()> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let mut header = [0_u8; 4];
    file.read_exact(&mut header).map_err(|error| {
        format!(
            "Binary DB generation file has no complete layout header at {}: {error}",
            path.display()
        )
    })?;
    let layout = u32::from_le_bytes(header);
    if layout != 1 {
        return Err(format!(
            "Binary DB generation file has unsupported layout {layout} at {relative_path:?}"
        ));
    }
    let Some(name) = relative_path.strip_prefix("local/") else {
        return Ok(());
    };
    let record_size = match name {
        "plan.bin" => Some(48_u64),
        "plan_revision.bin" => Some(56),
        "plan_item.bin" => Some(16),
        TASK_RECORD_BIN => Some(u64::from(LOCAL_TASK_RECORD_SIZE)),
        TASK_CHANGE_INDEX_BIN => Some(u64::from(TASK_CHANGE_INDEX_RECORD_SIZE)),
        TASK_LAND_INDEX_BIN => Some(u64::from(TASK_LAND_INDEX_RECORD_SIZE)),
        CHANGE_RECORD_BIN => Some(u64::from(LOCAL_CHANGE_RECORD_SIZE)),
        CHANGE_LAND_INDEX_BIN => Some(u64::from(CHANGE_LAND_INDEX_RECORD_SIZE)),
        LAND_RECORD_BIN => Some(u64::from(LOCAL_LAND_RECORD_SIZE)),
        "blob.bin" => Some(64),
        "snapshot.bin" => Some(88),
        "object_pack.bin" => Some(32),
        "object_pack_member.bin" => Some(16),
        "tree_pack.bin" => Some(32),
        "tree.bin" => Some(20),
        "line.bin" => Some(40),
        "stash.bin" => Some(8),
        _ => None,
    };
    if let Some(record_size) = record_size {
        let body_size = byte_size.checked_sub(4).ok_or_else(|| {
            format!("Binary DB generation file is shorter than its header: {relative_path:?}")
        })?;
        if body_size % record_size != 0 {
            return Err(format!(
                "Binary DB generation file is misaligned for {record_size}-byte records: {relative_path:?}"
            ));
        }
    }
    Ok(())
}

fn reject_generation_inside_retired_authority(
    authority: &Path,
    generation_root: &Path,
) -> GenerationResult<()> {
    if let Ok(authority_root) = authority.canonicalize() {
        if generation_root.starts_with(&authority_root) {
            return Err(format!(
                "Binary DB generation {} must not be inside the authority being retired {}",
                generation_root.display(),
                authority_root.display()
            ));
        }
    }
    Ok(())
}

fn activation_failure(primary: String, rollback: GenerationResult<()>) -> String {
    let mut parts = vec![primary];
    match rollback {
        Ok(()) => parts.push("direct authority rollback completed".to_string()),
        Err(error) => parts.push(format!("direct authority rollback failed: {error}")),
    }
    parts.join("; ")
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> GenerationResult<()> {
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("failed to sync directory {}: {error}", path.display()))
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> GenerationResult<()> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("failed to inspect directory {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "directory sync target is not a directory: {}",
            path.display()
        ));
    }
    // Windows does not expose Unix directory fsync semantics through
    // `std::fs::File`. File contents are synced before publication and the
    // closed-handle rename is the durable boundary available to this adapter.
    Ok(())
}

#[cfg(target_os = "macos")]
fn atomic_exchange(left: &Path, right: &Path) -> std::io::Result<()> {
    let left = CString::new(left.as_os_str().as_bytes())?;
    let right = CString::new(right.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_SWAP,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(target_os = "linux")]
fn atomic_exchange(left: &Path, right: &Path) -> std::io::Result<()> {
    let left = CString::new(left.as_os_str().as_bytes())?;
    let right = CString::new(right.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn atomic_exchange(_left: &Path, _right: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic Binary DB pointer exchange is unsupported on this platform",
    ))
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
    use crate::content_binary_db::{
        BinaryTreeCodec, BinaryTreePackCodec, BinaryTreePackRecord, BinaryTreeRecord,
    };
    use serde_json::json;
    use tempfile::TempDir;

    fn write_repo(root: &Path, repo_name: &str) {
        fs::create_dir_all(root.join(".ait/binary-db")).unwrap();
        fs::write(
            root.join(".ait/config.json"),
            serde_json::to_vec(&json!({"repo_name": repo_name})).unwrap(),
        )
        .unwrap();
        fs::write(root.join(".ait/binary-db/old-authority.marker"), b"old").unwrap();
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "test fixture fields mirror the persisted generation manifest"
    )]
    fn write_generation(
        root: &Path,
        source_authority_root: &Path,
        repo_name: &str,
        file_name: &str,
        validation_status: &str,
        layout_ids: BTreeMap<String, u32>,
        file_layout: u32,
        record_body_len: usize,
        manifest_schema: &str,
    ) {
        fs::create_dir_all(root.join("local")).unwrap();
        fs::create_dir_all(root.join(".ait/objects")).unwrap();
        let mut bytes = file_layout.to_le_bytes().to_vec();
        bytes.resize(4 + record_body_len, 0);
        let relative_path = format!("local/{file_name}");
        fs::write(root.join(&relative_path), &bytes).unwrap();
        let files = vec![ClientManifestFile {
            relative_path: relative_path.clone(),
            byte_size: bytes.len() as u64,
            sha256: hex_lower(&Sha256::digest(&bytes)),
        }];
        let manifest = json!({
            "schema": manifest_schema,
            "label": repo_name,
            "worker_count": 7,
            "layout_ids": layout_ids,
            "source_authority_fingerprint": fingerprint_direct_authority_contents(source_authority_root).unwrap(),
            "content_fingerprint": fingerprint_files(&files),
            "files": [{
                "relative_path": relative_path,
                "byte_size": bytes.len(),
                "sha256": files[0].sha256,
            }],
            "validation": {"status": validation_status},
        });
        fs::write(
            root.join(CLIENT_MANIFEST),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn valid_generation_atomically_replaces_and_retains_direct_authority() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let generation = temp.path().join("generation");
        write_repo(&repo, "fixture");
        write_generation(
            &generation,
            &repo.join(".ait/binary-db"),
            "fixture",
            "line.bin",
            "passed",
            required_layout_ids(),
            1,
            40,
            CLIENT_MANIFEST_SCHEMA,
        );

        let report = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo.clone(),
            generation_root: generation.clone(),
            expected_current_authority_fingerprint: None,
        })
        .unwrap();

        assert!(report.retired_direct_authority);
        let retained = report
            .retained_previous_authority
            .as_ref()
            .map(PathBuf::from)
            .expect("previous authority must be retained for rollback");
        assert_eq!(
            fs::read(retained.join("old-authority.marker")).unwrap(),
            b"old"
        );
        assert!(report.activation_lock_protected);
        assert!(matches!(
            report.activation_strategy.as_str(),
            "atomic_exchange" | "activation_lock_two_phase_rename"
        ));
        assert_eq!(
            report.single_syscall_atomic,
            report.activation_strategy == "atomic_exchange"
        );
        let authority_type = fs::symlink_metadata(repo.join(".ait/binary-db"))
            .unwrap()
            .file_type();
        assert!(authority_type.is_dir());
        assert!(!authority_type.is_symlink());
        assert!(!repo.join(".ait/binary-db/old-authority.marker").exists());
        let admitted =
            admit_activated_binary_db_generation(&repo.join(".ait/binary-db"), "fixture").unwrap();
        let canonical_repo = repo.canonicalize().unwrap();
        assert_eq!(admitted.generation_root, canonical_repo);
        assert_eq!(
            admitted.pack_root,
            repo.canonicalize().unwrap().join(".ait/objects")
        );
        fs::OpenOptions::new()
            .append(true)
            .open(repo.join(".ait/binary-db/line.bin"))
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let error = admit_activated_binary_db_generation(&repo.join(".ait/binary-db"), "fixture")
            .unwrap_err();
        assert!(error.contains("misaligned"));
    }

    #[test]
    fn invalid_generations_never_replace_existing_pointer() {
        for case in [
            "missing-packs",
            "checksum-drift",
            "repo-mismatch",
            "layout-mismatch",
            "failed-validation",
            "undeclared-bin",
            "manifest-schema",
            "file-layout-mismatch",
            "misaligned-record",
        ] {
            let temp = TempDir::new().unwrap();
            let repo = temp.path().join("repo");
            let generation = temp.path().join("generation");
            write_repo(&repo, "fixture");
            let label = if case == "repo-mismatch" {
                "different"
            } else {
                "fixture"
            };
            let mut layouts = required_layout_ids();
            if case == "layout-mismatch" {
                layouts.insert("content".to_string(), 2);
            }
            let file_name = if case == "undeclared-bin" {
                "undeclared_alias.bin"
            } else {
                "line.bin"
            };
            write_generation(
                &generation,
                &repo.join(".ait/binary-db"),
                label,
                file_name,
                if case == "failed-validation" {
                    "failed"
                } else {
                    "passed"
                },
                layouts,
                if case == "file-layout-mismatch" { 2 } else { 1 },
                if case == "misaligned-record" { 39 } else { 40 },
                if case == "manifest-schema" {
                    "ait.binary-db-local-generation-manifest.v999"
                } else {
                    CLIENT_MANIFEST_SCHEMA
                },
            );
            if case == "missing-packs" {
                fs::remove_dir_all(generation.join(".ait/objects")).unwrap();
            }
            if case == "checksum-drift" {
                fs::write(generation.join("local/line.bin"), b"drift").unwrap();
            }

            let error = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
                repo_root: repo.clone(),
                generation_root: generation.clone(),
                expected_current_authority_fingerprint: None,
            })
            .unwrap_err();
            assert!(!error.is_empty(), "{case}");
            assert!(repo.join(".ait/binary-db").is_dir(), "{case}");
            assert_eq!(
                fs::read(repo.join(".ait/binary-db/old-authority.marker")).unwrap(),
                b"old",
                "{case}"
            );
        }
    }

    #[test]
    fn stale_generation_rejects_every_source_authority_change_before_staging() {
        for case in ["same-length", "append", "add-file", "remove-file"] {
            let temp = TempDir::new().unwrap();
            let repo = temp.path().join("repo");
            let generation = temp.path().join("generation");
            write_repo(&repo, "fixture");
            write_generation(
                &generation,
                &repo.join(".ait/binary-db"),
                "fixture",
                "line.bin",
                "passed",
                required_layout_ids(),
                1,
                40,
                CLIENT_MANIFEST_SCHEMA,
            );
            let marker = repo.join(".ait/binary-db/old-authority.marker");
            match case {
                "same-length" => fs::write(&marker, b"new").unwrap(),
                "append" => fs::OpenOptions::new()
                    .append(true)
                    .open(&marker)
                    .unwrap()
                    .write_all(b"!")
                    .unwrap(),
                "add-file" => fs::write(repo.join(".ait/binary-db/added.bin"), b"new").unwrap(),
                "remove-file" => fs::remove_file(&marker).unwrap(),
                _ => unreachable!(),
            }

            let current_bytes = fs::read_dir(repo.join(".ait/binary-db"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .map(|entry| (entry.file_name(), fs::read(entry.path()).unwrap()))
                .collect::<BTreeMap<_, _>>();
            let error = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
                repo_root: repo.clone(),
                generation_root: generation.clone(),
                expected_current_authority_fingerprint: None,
            })
            .unwrap_err();

            assert!(
                error.contains("stale Binary DB generation"),
                "{case}: {error}"
            );
            let after_bytes = fs::read_dir(repo.join(".ait/binary-db"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
                .map(|entry| (entry.file_name(), fs::read(entry.path()).unwrap()))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(after_bytes, current_bytes, "{case}");
            assert!(generation.join("local/line.bin").is_file(), "{case}");
            assert!(
                fs::read_dir(repo.join(".ait"))
                    .unwrap()
                    .filter_map(Result::ok)
                    .all(|entry| !entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".binary-db.activate-")),
                "{case}"
            );
        }
    }

    #[test]
    fn activation_requires_valid_source_authority_fingerprint() {
        for case in ["missing", "invalid"] {
            let temp = TempDir::new().unwrap();
            let repo = temp.path().join("repo");
            let generation = temp.path().join("generation");
            write_repo(&repo, "fixture");
            write_generation(
                &generation,
                &repo.join(".ait/binary-db"),
                "fixture",
                "line.bin",
                "passed",
                required_layout_ids(),
                1,
                40,
                CLIENT_MANIFEST_SCHEMA,
            );
            let manifest_path = generation.join(CLIENT_MANIFEST);
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
            if case == "missing" {
                manifest
                    .as_object_mut()
                    .unwrap()
                    .remove("source_authority_fingerprint");
            } else {
                manifest["source_authority_fingerprint"] = json!("not-a-sha256");
            }
            fs::write(
                &manifest_path,
                serde_json::to_vec_pretty(&manifest).unwrap(),
            )
            .unwrap();

            let error = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
                repo_root: repo.clone(),
                generation_root: generation,
                expected_current_authority_fingerprint: None,
            })
            .unwrap_err();
            assert!(
                error.contains("source_authority_fingerprint"),
                "{case}: {error}"
            );
            assert_eq!(
                fs::read(repo.join(".ait/binary-db/old-authority.marker")).unwrap(),
                b"old",
                "{case}"
            );
        }
    }

    #[test]
    fn explicit_target_fingerprint_preserves_cross_repository_recovery() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let generation = temp.path().join("generation");
        write_repo(&source, "fixture");
        write_repo(&target, "fixture");
        fs::write(
            source.join(".ait/binary-db/old-authority.marker"),
            b"source",
        )
        .unwrap();
        fs::write(
            target.join(".ait/binary-db/old-authority.marker"),
            b"target",
        )
        .unwrap();
        write_generation(
            &generation,
            &source.join(".ait/binary-db"),
            "fixture",
            "line.bin",
            "passed",
            required_layout_ids(),
            1,
            40,
            CLIENT_MANIFEST_SCHEMA,
        );
        let target_fingerprint =
            fingerprint_direct_authority_contents(&target.join(".ait/binary-db")).unwrap();

        let report = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: target.clone(),
            generation_root: generation,
            expected_current_authority_fingerprint: Some(target_fingerprint),
        })
        .unwrap();

        assert!(report.retained_previous_authority.is_some());
        assert!(target.join(".ait/binary-db/line.bin").is_file());
    }

    #[test]
    fn activation_rejects_mismatched_compact_tree_pack_ordinal() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let generation = temp.path().join("generation");
        write_repo(&repo, "fixture");
        fs::create_dir_all(generation.join("local")).unwrap();
        fs::create_dir_all(generation.join(".ait/objects")).unwrap();

        let tree_pack = BinaryTreePackRecord {
            pack_meta: BinaryTreePackRecord::META_READY,
            pack_format_kind: 1,
            pack_hash_hi16: 0,
            pack_hash_lo32: 0,
            first_tree_index: 0,
            tree_count: 1,
            total_bytes: 1,
            created_at_s: 1,
        };
        let tree = BinaryTreeRecord {
            tree_meta: 0,
            reserved0: 0,
            pack_entry_ordinal: 7,
            entry_count: 0,
            tree_hash80: [7; 10],
        };
        let tree_bytes = [
            1_u32.to_le_bytes().as_slice(),
            BinaryTreeCodec::<1>::encode_record(&tree)
                .unwrap()
                .as_slice(),
        ]
        .concat();
        let tree_pack_bytes = [
            1_u32.to_le_bytes().as_slice(),
            BinaryTreePackCodec::<1>::encode_record(&tree_pack)
                .unwrap()
                .as_slice(),
        ]
        .concat();
        fs::write(generation.join("local/tree.bin"), &tree_bytes).unwrap();
        fs::write(generation.join("local/tree_pack.bin"), &tree_pack_bytes).unwrap();
        let files = vec![
            ClientManifestFile {
                relative_path: "local/tree.bin".to_string(),
                byte_size: tree_bytes.len() as u64,
                sha256: hex_lower(&Sha256::digest(&tree_bytes)),
            },
            ClientManifestFile {
                relative_path: "local/tree_pack.bin".to_string(),
                byte_size: tree_pack_bytes.len() as u64,
                sha256: hex_lower(&Sha256::digest(&tree_pack_bytes)),
            },
        ];
        let manifest = json!({
            "schema": CLIENT_MANIFEST_SCHEMA,
            "label": "fixture",
            "worker_count": 3,
            "layout_ids": required_layout_ids(),
            "source_authority_fingerprint": fingerprint_direct_authority_contents(&repo.join(".ait/binary-db")).unwrap(),
            "content_fingerprint": fingerprint_files(&files),
            "files": files.iter().map(|file| json!({
                "relative_path": file.relative_path,
                "byte_size": file.byte_size,
                "sha256": file.sha256,
            })).collect::<Vec<_>>(),
            "validation": {"status": "passed"},
        });
        fs::write(
            generation.join(CLIENT_MANIFEST),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let error = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo.clone(),
            generation_root: generation,
            expected_current_authority_fingerprint: None,
        })
        .unwrap_err();
        assert!(error.contains("compact pack ordinal 7, expected 0"));
        assert_eq!(
            fs::read(repo.join(".ait/binary-db/old-authority.marker")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn activation_rejects_incomplete_exact_v0_workflow_indexes_before_exchange() {
        let temp = TempDir::new().unwrap();
        let repo = temp.path().join("repo");
        let generation = temp.path().join("generation");
        write_repo(&repo, "fixture");
        fs::create_dir_all(generation.join("local")).unwrap();
        fs::create_dir_all(generation.join(".ait/objects")).unwrap();

        let task_payloads = [
            1_u32.to_le_bytes().as_slice(),
            4_u16.to_le_bytes().as_slice(),
            b"Task",
            b"Intent",
        ]
        .concat();
        let mut task_records = 1_u32.to_le_bytes().to_vec();
        task_records.extend_from_slice(
            &crate::workflow_binary_db::LocalTaskRecord {
                task_meta: 0,
                local_meta: 0,
                payload_len: 12,
                payload_offset: 4,
                origin_plan_revision_index_plus1: 0,
                plan_item_index_plus1: 0,
                published_remote_task_index: 0,
                created_at_s: 1,
                updated_at_s: 1,
                plan_linked_at_s: 0,
                published_at_s: 0,
                closed_at_s: 0,
            }
            .encode()
            .unwrap(),
        );
        fs::write(generation.join("local/task.bin"), &task_records).unwrap();
        fs::write(generation.join("local/task_payload.bin"), &task_payloads).unwrap();
        let files = vec![
            ClientManifestFile {
                relative_path: "local/task.bin".to_string(),
                byte_size: task_records.len() as u64,
                sha256: hex_lower(&Sha256::digest(&task_records)),
            },
            ClientManifestFile {
                relative_path: "local/task_payload.bin".to_string(),
                byte_size: task_payloads.len() as u64,
                sha256: hex_lower(&Sha256::digest(&task_payloads)),
            },
        ];
        let manifest = json!({
            "schema": CLIENT_MANIFEST_SCHEMA,
            "label": "fixture",
            "worker_count": 1,
            "layout_ids": required_layout_ids(),
            "source_authority_fingerprint": fingerprint_direct_authority_contents(&repo.join(".ait/binary-db")).unwrap(),
            "content_fingerprint": fingerprint_files(&files),
            "files": files.iter().map(|file| json!({
                "relative_path": file.relative_path,
                "byte_size": file.byte_size,
                "sha256": file.sha256,
            })).collect::<Vec<_>>(),
            "validation": {"status": "passed"},
        });
        fs::write(
            generation.join(CLIENT_MANIFEST),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let error = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
            repo_root: repo.clone(),
            generation_root: generation,
            expected_current_authority_fingerprint: None,
        })
        .unwrap_err();
        assert!(error.contains("task_change_index.bin has 0 records, expected 1"));
        assert_eq!(
            fs::read(repo.join(".ait/binary-db/old-authority.marker")).unwrap(),
            b"old"
        );
    }
}
