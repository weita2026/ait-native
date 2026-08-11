use crate::json_support::{encode_value_pretty_with_newline_error_string, parse_value};
use crate::remote_repository::local_policy_payload;
use crate::runtime::{RemoteRow, RepoRuntime};
use crate::workspace_lock::run_locked_workspace_command;
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::plan_http_client::{
    repository_registration_policy_flags, PlanHttpClientConfig, PlanHttpClientError,
    PlanHttpClientManager,
};
use ait_core::server_repo_retire::{
    validate_remote_authority_relative_path, RemoteExportFile, RemoteExportManifest,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const LOCAL_RETIREMENT_CONTRACT: &str = "ait.local.repository-retirement.v1";
const LOCAL_RESTORE_CONTRACT: &str = "ait.local.repository-restore.v1";
const SERVER_RETIREMENT_ABORT_CONTRACT: &str = "ait.server.repository-retirement-abort.v1";
const RESTORE_RECEIPT_SCHEMA: &str = "ait.remote-restore.v1";
const RESTORE_SESSION_SCHEMA: &str = "ait.remote-restore-session.v1";
const COMPLETE_STATE: &str = "complete";
const UPLOADING_STATE: &str = "uploading";
const COMMITTED_STATE: &str = "committed";
const ACTIVE_REPOSITORY_LIFECYCLE_KIND: u32 = 1;

#[derive(Clone, Debug)]
pub struct RepoRetireRequest {
    pub remote_name: Option<String>,
    pub abort: bool,
    pub replace_export: bool,
}

#[derive(Clone, Debug)]
pub struct RepoRestoreRequest {
    pub remote_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RestoreReceipt {
    restored_at_s: u32,
    repository_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RestoreSession {
    state: String,
    restore_token: String,
    repository_index: Option<u32>,
}

#[derive(Clone, Debug)]
struct ArchivePaths {
    parent: PathBuf,
    slot: PathBuf,
    session: PathBuf,
    remote_name: String,
}

#[derive(Clone, Debug)]
struct CompleteArchive {
    manifest: RemoteExportManifest,
    receipt: Option<RestoreReceipt>,
}

pub fn retire_repository(
    repo: &RepoRuntime,
    request: &RepoRetireRequest,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli repo retire", || {
        let refreshed = RepoRuntime::discover_from_path(&repo.workspace_root())?;
        retire_repository_unlocked(&refreshed, request)
    })
}

pub fn restore_repository(
    repo: &RepoRuntime,
    request: &RepoRestoreRequest,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli repo restore", || {
        let refreshed = RepoRuntime::discover_from_path(&repo.workspace_root())?;
        restore_repository_unlocked(&refreshed, request)
    })
}

pub(crate) fn enforce_fresh_registration_archive_policy(
    repo: &RepoRuntime,
    remote_name: &str,
    discard_export: bool,
) -> Result<bool, String> {
    if repo.repository_index().is_some() {
        return Ok(false);
    }
    let Some(paths) = archive_paths(repo, remote_name, false)? else {
        return Ok(false);
    };
    recover_archive_publication(&paths)?;
    if !path_exists(&paths.slot)? {
        return Ok(false);
    }
    require_real_directory(&paths.slot, "local retirement archive")?;
    if !discard_export {
        return Err(format!(
            "Fresh registration through remote {remote_name:?} would ignore the existing complete \
             retirement archive at {}. Restore it with `ait repo restore --remote {}` or explicitly \
             discard it with `ait remote add {} <url> --discard-export`.",
            paths.slot.display(),
            shell_word(remote_name),
            shell_word(remote_name),
        ));
    }
    remove_directory_tree(&paths.slot, "local retirement archive")?;
    remove_restore_session_artifacts(&paths)?;
    sync_directory(&paths.parent)?;
    Ok(true)
}

fn retire_repository_unlocked(
    repo: &RepoRuntime,
    request: &RepoRetireRequest,
) -> Result<JsonValue, String> {
    if request.abort && request.replace_export {
        return Err(
            "`abort` and `replace_export` are mutually exclusive Repository retirement options."
                .to_string(),
        );
    }
    let remote = repo.remote_row(request.remote_name.as_deref())?;
    let repository_index = repo.require_repository_index()?;
    let mut client = PlanHttpClientManager::new(http_config(repo, &remote))
        .map_err(|error| error.to_string())?;
    if request.abort {
        return abort_repository_retirement_unlocked(
            repo,
            &remote,
            repository_index.get(),
            &mut client,
        );
    }
    let expected_repo_name = remote
        .repo_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| repo.repo_name());
    let status = match client.begin_repository_retirement(repository_index) {
        Ok(status) => status,
        Err(error) if is_already_purged(&error) => {
            return recover_already_purged_archive(
                repo,
                &remote,
                repository_index.get(),
                &expected_repo_name,
            );
        }
        Err(error) => return Err(error.to_string()),
    };
    validate_retirement_status_repository(&status, repository_index.get())?;
    let ready = required_bool(
        &status,
        "ready_for_export",
        "Repository retirement response",
    )?;
    if !ready {
        return Ok(json!({
            "contract": LOCAL_RETIREMENT_CONTRACT,
            "state": "waiting_for_jobs",
            "remote": remote.name,
            "repository_index": repository_index.get(),
            "ready_for_export": false,
            "drain": status.get("drain").cloned().unwrap_or(JsonValue::Null),
            "archive": JsonValue::Null,
            "server": status,
        }));
    }

    let manifest_value = status
        .get("manifest")
        .ok_or_else(|| "Ready Repository retirement response is missing manifest.".to_string())?;
    let manifest = RemoteExportManifest::from_json(manifest_value)?;
    validate_manifest_context(repo, &manifest, &expected_repo_name)?;
    let paths = archive_paths(repo, &remote.name, true)?
        .ok_or_else(|| "Failed to create local retirement archive root.".to_string())?;
    recover_archive_publication(&paths)?;

    let reused = prepare_complete_archive(
        &paths,
        &manifest,
        repository_index.get(),
        request.replace_export,
        |file| {
            client
                .get_repository_retirement_file(repository_index, &file.path)
                .map_err(|error| error.to_string())
        },
    )?;
    clear_restore_receipt(&paths)?;
    remove_restore_session_artifacts(&paths)?;
    let complete = load_complete_archive(&paths)?;
    if complete.manifest != manifest || complete.receipt.is_some() {
        return Err(
            "Published retirement archive does not exactly match the acknowledged manifest."
                .to_string(),
        );
    }

    let purged = client
        .purge_repository_retirement(repository_index, &manifest)
        .map_err(|error| {
            format!(
                "Local retirement archive is complete at {}, but server purge failed: {error}",
                paths.slot.display()
            )
        })?;
    Ok(json!({
        "contract": LOCAL_RETIREMENT_CONTRACT,
        "state": "purged",
        "remote": remote.name,
        "repository_index": repository_index.get(),
        "ready_for_export": true,
        "archive": archive_projection(&paths, &manifest, reused),
        "server": purged,
    }))
}

fn abort_repository_retirement_unlocked(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    repository_index: u32,
    client: &mut PlanHttpClientManager,
) -> Result<JsonValue, String> {
    let repository_index_value =
        ait_core::server_operational::RepositoryIndex::new(repository_index);
    let response = client
        .abort_repository_retirement(repository_index_value)
        .map_err(|error| {
            format!(
                "Repository retirement abort failed before local retirement transients were changed: {error}"
            )
        })?;
    let already_aborted = validate_retirement_abort_response(&response, repository_index)?;
    let archive_preserved = (|| {
        let Some(paths) = archive_paths(repo, &remote.name, false)? else {
            return Ok(false);
        };
        recover_archive_publication(&paths)?;
        path_exists(&paths.slot)
    })()
    .map_err(|error: String| {
        format!(
            "Server Repository index {repository_index} is active, but local retirement transient cleanup failed: {error}"
        )
    })?;
    Ok(json!({
        "contract": LOCAL_RETIREMENT_CONTRACT,
        "state": "active",
        "operation": "aborted",
        "remote": remote.name,
        "repository_index": repository_index,
        "aborted": true,
        "already_aborted": already_aborted,
        "local_transients_cleaned": true,
        "archive_preserved": archive_preserved,
        "server": response,
    }))
}

fn restore_repository_unlocked(
    repo: &RepoRuntime,
    request: &RepoRestoreRequest,
) -> Result<JsonValue, String> {
    let remote = repo.remote_row(request.remote_name.as_deref())?;
    let paths = archive_paths(repo, &remote.name, false)?.ok_or_else(|| {
        format!(
            "No local retirement archive exists for remote {:?}.",
            remote.name
        )
    })?;
    recover_archive_publication(&paths)?;
    let archive = load_complete_archive(&paths)?;
    let expected_repo_name = remote
        .repo_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| repo.repo_name());
    validate_manifest_context(repo, &archive.manifest, &expected_repo_name)?;

    if let Some(receipt) = archive.receipt.as_ref() {
        if repo.repository_index().map(|value| value.get()) == Some(receipt.repository_index) {
            remove_restore_session_artifacts(&paths)?;
            return Ok(restore_projection(
                &remote,
                &paths,
                &archive.manifest,
                receipt.repository_index,
                receipt.restored_at_s,
                true,
            ));
        }
        return Err(format!(
            "Archive {} records a completed restore to repository_index {}, but local config is \
             bound to {}. Refusing to create another Repository from the same restored archive.",
            paths.slot.display(),
            receipt.repository_index,
            repo.repository_index()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "no repository_index".to_string()),
        ));
    }

    let policy = local_policy_payload(repo)?;
    let policy_flags =
        repository_registration_policy_flags(policy.as_ref()).map_err(|error| error.to_string())?;
    let mut client = PlanHttpClientManager::new(http_config(repo, &remote))
        .map_err(|error| error.to_string())?;
    let session = read_restore_session(&paths.session)?;
    let committed_index = match session {
        Some(session) if session.state == COMMITTED_STATE => {
            session.repository_index.ok_or_else(|| {
                "Committed local restore session is missing repository_index.".to_string()
            })?
        }
        existing => {
            let mut session = match existing {
                Some(session) => session,
                None => {
                    begin_restore_session(&mut client, &paths, &archive.manifest, policy_flags)?
                }
            };
            let mut restarted = false;
            loop {
                match upload_and_commit_restore(
                    &mut client,
                    &paths,
                    &archive.manifest,
                    &session.restore_token,
                ) {
                    Ok(response) => {
                        let repository_index =
                            validate_restore_commit_response(repo, &archive.manifest, &response)?;
                        session.state = COMMITTED_STATE.to_string();
                        session.repository_index = Some(repository_index);
                        write_restore_session(&paths, &session)?;
                        break repository_index;
                    }
                    Err(error) if error.remote_status() == Some(404) && !restarted => {
                        remove_regular_file_if_exists(&paths.session, "local restore session")?;
                        sync_directory(&paths.parent)?;
                        session = begin_restore_session(
                            &mut client,
                            &paths,
                            &archive.manifest,
                            policy_flags,
                        )?;
                        restarted = true;
                    }
                    Err(error) => {
                        return Err(format!(
                            "Repository restore session {} failed: {error}",
                            session.restore_token
                        ))
                    }
                }
            }
        }
    };

    replace_config_repository_index(repo, committed_index)?;
    let restored_at_s = now_s()?;
    let receipt = RestoreReceipt {
        restored_at_s,
        repository_index: committed_index,
    };
    write_restore_receipt(&paths, &receipt)?;
    remove_restore_session_artifacts(&paths)?;
    Ok(restore_projection(
        &remote,
        &paths,
        &archive.manifest,
        committed_index,
        restored_at_s,
        false,
    ))
}

fn begin_restore_session(
    client: &mut PlanHttpClientManager,
    paths: &ArchivePaths,
    manifest: &RemoteExportManifest,
    policy_flags: u8,
) -> Result<RestoreSession, String> {
    let response = client
        .begin_repository_restore(manifest, policy_flags)
        .map_err(|error| {
            if error.remote_status() == Some(409) {
                format!(
                    "{error}. If a prior restore commit succeeded but its local receipt was lost, \
                     identify that live Repository before retrying; the local archive intentionally \
                     stores no server instance identity or old Repository index."
                )
            } else {
                error.to_string()
            }
        })?;
    if required_string(&response, "contract", "Repository restore session response")?
        != "ait.server.repository-restore-session.v1"
        || required_string(&response, "state", "Repository restore session response")?
            != UPLOADING_STATE
    {
        return Err(
            "Repository restore session response has an unsupported contract or state.".to_string(),
        );
    }
    let restore_token = required_string(
        &response,
        "restore_token",
        "Repository restore session response",
    )?;
    validate_restore_token(&restore_token)?;
    let session = RestoreSession {
        state: UPLOADING_STATE.to_string(),
        restore_token,
        repository_index: None,
    };
    write_restore_session(paths, &session)?;
    Ok(session)
}

fn upload_and_commit_restore(
    client: &mut PlanHttpClientManager,
    paths: &ArchivePaths,
    manifest: &RemoteExportManifest,
    restore_token: &str,
) -> Result<JsonValue, PlanHttpClientError> {
    for expected in &manifest.files {
        let bytes = read_verified_file(&paths.slot.join("data"), expected)
            .map_err(PlanHttpClientError::Invalid)?;
        match client.upload_repository_restore_file(restore_token, &expected.path, bytes) {
            Ok(_) => {}
            Err(error)
                if error.remote_status() == Some(400)
                    && error
                        .remote_detail()
                        .is_some_and(|detail| detail.contains("already committed")) =>
            {
                return client.commit_repository_restore(restore_token);
            }
            Err(error) => return Err(error),
        }
    }
    client.commit_repository_restore(restore_token)
}

fn validate_restore_commit_response(
    repo: &RepoRuntime,
    manifest: &RemoteExportManifest,
    response: &JsonValue,
) -> Result<u32, String> {
    if required_string(response, "contract", "Repository restore response")?
        != "ait.server.repository-restore.v1"
        || !required_bool(response, "created", "Repository restore response")?
    {
        return Err(
            "Repository restore response must report the exact created contract.".to_string(),
        );
    }
    let repository = response
        .get("repository")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Repository restore response is missing repository object.".to_string())?;
    let repository_index = required_u32_object(
        repository,
        "repository_index",
        "Repository restore repository",
    )?;
    if required_string_object(
        repository,
        "repository_name",
        "Repository restore repository",
    )? != manifest.repo_name
        || required_string_object(repository, "namespace", "Repository restore repository")?
            != manifest.namespace
    {
        return Err(
            "Repository restore response name or namespace does not match the local archive."
                .to_string(),
        );
    }
    if repository
        .get("tombstoned")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return Err("Newly restored Repository must not be tombstoned.".to_string());
    }
    if repo.repository_index().map(|value| value.get()) == Some(repository_index) {
        return Err(format!(
            "Repository restore reused configured repository_index {repository_index}; restore must allocate a new index."
        ));
    }
    Ok(repository_index)
}

fn prepare_complete_archive<F>(
    paths: &ArchivePaths,
    manifest: &RemoteExportManifest,
    repository_index: u32,
    replace_export: bool,
    mut download: F,
) -> Result<bool, String>
where
    F: FnMut(&RemoteExportFile) -> Result<Vec<u8>, String>,
{
    if path_exists(&paths.slot)? {
        require_real_directory(&paths.slot, "local retirement archive")?;
        let existing_manifest = read_manifest_from_slot(&paths.slot);
        let exact_manifest = existing_manifest
            .as_ref()
            .is_ok_and(|existing| existing == manifest);
        let receipt_matches_current = read_restore_receipt(&paths.slot.join("restore.json"))
            .ok()
            .flatten()
            .is_some_and(|receipt| receipt.repository_index == repository_index);
        let complete_exact = exact_manifest
            && load_complete_archive(paths).is_ok_and(|archive| archive.manifest == *manifest);
        if complete_exact {
            return Ok(true);
        }
        if !exact_manifest && !receipt_matches_current && !replace_export {
            return Err(format!(
                "Remote {:?} already has an unrelated local retirement archive at {}. \
                 Pass `--replace-export` to replace it explicitly.",
                paths.remote_name,
                paths.slot.display()
            ));
        }
    }

    let stage = create_archive_stage(paths)?;
    let stage_result = (|| {
        let data = stage.join("data");
        for expected in &manifest.files {
            let bytes = download(expected)?;
            validate_downloaded_bytes(expected, &bytes)?;
            write_staged_authority_file(&data, expected, &bytes)?;
        }
        validate_data_inventory(&data, manifest)?;
        write_new_json_file(&stage.join("remote.json"), &manifest.to_json()?)?;
        sync_directory(&stage)?;
        publish_archive_stage(paths, &stage)
    })();
    if stage_result.is_err() && path_exists(&stage).unwrap_or(false) {
        let _ = remove_directory_tree(&stage, "failed retirement archive stage");
    }
    stage_result.map(|_| false)
}

fn recover_already_purged_archive(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    repository_index: u32,
    expected_repo_name: &str,
) -> Result<JsonValue, String> {
    let paths = archive_paths(repo, &remote.name, false)?.ok_or_else(|| {
        format!(
            "Server Repository index {repository_index} is already purged, but no local archive \
             root exists for remote {:?}.",
            remote.name
        )
    })?;
    recover_archive_publication(&paths)?;
    let archive = load_complete_archive(&paths).map_err(|error| {
        format!(
            "Server Repository index {repository_index} is already purged, but its local archive \
             cannot be verified: {error}"
        )
    })?;
    validate_manifest_context(repo, &archive.manifest, expected_repo_name)?;
    if archive.receipt.is_some() {
        return Err(
            "Already-purged Repository has a stale restore.json receipt; local retirement state is inconsistent."
                .to_string(),
        );
    }
    Ok(json!({
        "contract": LOCAL_RETIREMENT_CONTRACT,
        "state": "already_purged",
        "remote": remote.name,
        "repository_index": repository_index,
        "ready_for_export": true,
        "archive": archive_projection(&paths, &archive.manifest, true),
    }))
}

fn validate_retirement_status_repository(
    status: &JsonValue,
    repository_index: u32,
) -> Result<(), String> {
    if required_string(status, "contract", "Repository retirement response")?
        != "ait.server.repository-retirement.v1"
    {
        return Err("Repository retirement response has an unsupported contract.".to_string());
    }
    let actual = status
        .get("repository")
        .and_then(JsonValue::as_object)
        .and_then(|repository| repository.get("repository_index"))
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            "Repository retirement response is missing numeric repository_index.".to_string()
        })?;
    if actual != repository_index {
        return Err(format!(
            "Repository retirement response index mismatch: configured={repository_index} remote={actual}."
        ));
    }
    Ok(())
}

fn validate_retirement_abort_response(
    response: &JsonValue,
    repository_index: u32,
) -> Result<bool, String> {
    const LABEL: &str = "Repository retirement abort response";
    if required_string(response, "contract", LABEL)? != SERVER_RETIREMENT_ABORT_CONTRACT {
        return Err(format!("{LABEL} has an unsupported contract."));
    }
    if !required_bool(response, "aborted", LABEL)? {
        return Err(format!("{LABEL} must report aborted=true."));
    }
    let already_aborted = required_bool(response, "already_aborted", LABEL)?;
    let repository = response
        .get("repository")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("{LABEL} requires object repository."))?;
    let actual_index = required_u32_object(repository, "repository_index", LABEL)?;
    if actual_index != repository_index {
        return Err(format!(
            "{LABEL} index mismatch: configured={repository_index} remote={actual_index}."
        ));
    }
    let lifecycle_kind = required_u32_object(repository, "lifecycle_kind", LABEL)?;
    if lifecycle_kind != ACTIVE_REPOSITORY_LIFECYCLE_KIND {
        return Err(format!(
            "{LABEL} must report active lifecycle_kind={ACTIVE_REPOSITORY_LIFECYCLE_KIND}, got {lifecycle_kind}."
        ));
    }
    let tombstoned = repository
        .get("tombstoned")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| format!("{LABEL} requires boolean tombstoned."))?;
    if tombstoned {
        return Err(format!("{LABEL} must report tombstoned=false."));
    }
    Ok(already_aborted)
}

fn validate_manifest_context(
    repo: &RepoRuntime,
    manifest: &RemoteExportManifest,
    expected_repo_name: &str,
) -> Result<(), String> {
    if manifest.repo_name != expected_repo_name {
        return Err(format!(
            "Remote export Repository name mismatch: configured={expected_repo_name:?} manifest={:?}.",
            manifest.repo_name
        ));
    }
    let expected_namespace = repo.id_namespace_prefix();
    if manifest.namespace != expected_namespace {
        return Err(format!(
            "Remote export namespace mismatch: configured={expected_namespace:?} manifest={:?}.",
            manifest.namespace
        ));
    }
    Ok(())
}

fn archive_projection(
    paths: &ArchivePaths,
    manifest: &RemoteExportManifest,
    reused: bool,
) -> JsonValue {
    let byte_count = manifest
        .files
        .iter()
        .fold(0_u64, |total, file| total.saturating_add(file.size));
    json!({
        "path": paths.slot.to_string_lossy(),
        "remote_json": paths.slot.join("remote.json").to_string_lossy(),
        "file_count": manifest.files.len(),
        "byte_count": byte_count,
        "reused": reused,
    })
}

fn restore_projection(
    remote: &RemoteRow,
    paths: &ArchivePaths,
    manifest: &RemoteExportManifest,
    repository_index: u32,
    restored_at_s: u32,
    already_restored: bool,
) -> JsonValue {
    json!({
        "contract": LOCAL_RESTORE_CONTRACT,
        "state": COMPLETE_STATE,
        "remote": remote.name,
        "archive": paths.slot.to_string_lossy(),
        "repo_name": manifest.repo_name,
        "namespace": manifest.namespace,
        "repository_index": repository_index,
        "restored_at_s": restored_at_s,
        "already_restored": already_restored,
    })
}

fn archive_paths(
    repo: &RepoRuntime,
    remote_name: &str,
    create_parent: bool,
) -> Result<Option<ArchivePaths>, String> {
    let remote_name = validate_remote_name(remote_name)?;
    let ait_dir = repo.authoritative_repo_root().join(".ait");
    require_real_directory(&ait_dir, "authoritative .ait directory")?;
    let parent = ait_dir.join("remote");
    match fs::symlink_metadata(&parent) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "Local retirement archive root {} must be a real directory.",
                    parent.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_parent => {
            fs::create_dir(&parent).map_err(|error| {
                format!(
                    "Failed to create local retirement archive root {}: {error}",
                    parent.display()
                )
            })?;
            sync_directory(&ait_dir)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "Failed to inspect local retirement archive root {}: {error}",
                parent.display()
            ))
        }
    }
    Ok(Some(ArchivePaths {
        slot: parent.join(&remote_name),
        session: parent.join(format!(".{remote_name}.restore-session.json")),
        parent,
        remote_name,
    }))
}

fn validate_remote_name(value: &str) -> Result<String, String> {
    let value = value.trim();
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(
            "Remote name must be a single non-empty filesystem component for local retirement archives."
                .to_string(),
        );
    }
    Ok(value.to_string())
}

fn recover_archive_publication(paths: &ArchivePaths) -> Result<(), String> {
    require_real_directory(&paths.parent, "local retirement archive root")?;
    let stage_prefix = format!(".{}.staging-", paths.remote_name);
    let backup_prefix = format!(".{}.backup-", paths.remote_name);
    let session_temp_prefix = format!(".{}.restore-session.json-", paths.remote_name);
    let receipt_temp_prefix = format!(".{}.restore-receipt-", paths.remote_name);
    let mut stages = Vec::new();
    let mut backups = Vec::new();
    let mut temporary_files = Vec::new();
    for entry in fs::read_dir(&paths.parent).map_err(|error| {
        format!(
            "Failed to scan local retirement archive root {}: {error}",
            paths.parent.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read local retirement archive entry in {}: {error}",
                paths.parent.display()
            )
        })?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Local retirement archive entry name must be UTF-8.".to_string())?;
        if name.starts_with(&stage_prefix) {
            stages.push(entry.path());
        } else if name.starts_with(&backup_prefix) {
            backups.push(entry.path());
        } else if name.starts_with(&session_temp_prefix) || name.starts_with(&receipt_temp_prefix) {
            temporary_files.push(entry.path());
        }
    }
    stages.sort();
    backups.sort();
    temporary_files.sort();
    let final_exists = path_exists(&paths.slot)?;
    if final_exists {
        require_real_directory(&paths.slot, "local retirement archive")?;
    } else if backups.len() == 1 {
        require_real_directory(&backups[0], "retirement archive backup")?;
        fs::rename(&backups[0], &paths.slot).map_err(|error| {
            format!(
                "Failed to recover local retirement archive {} from {}: {error}",
                paths.slot.display(),
                backups[0].display()
            )
        })?;
        backups.clear();
        sync_directory(&paths.parent)?;
    } else if backups.len() > 1 {
        return Err(format!(
            "Multiple local retirement archive backups exist for remote {:?}; refusing ambiguous recovery.",
            paths.remote_name
        ));
    }
    for stage in stages {
        remove_directory_tree(&stage, "stale retirement archive stage")?;
    }
    for backup in backups {
        remove_directory_tree(&backup, "stale retirement archive backup")?;
    }
    for temporary in temporary_files {
        remove_regular_file_if_exists(&temporary, "stale retirement JSON stage")?;
    }
    sync_directory(&paths.parent)
}

fn create_archive_stage(paths: &ArchivePaths) -> Result<PathBuf, String> {
    let prefix = format!(".{}.staging", paths.remote_name);
    let stage = allocate_unique_path(&paths.parent, &prefix, "")?;
    fs::create_dir(&stage).map_err(|error| {
        format!(
            "Failed to create retirement archive stage {}: {error}",
            stage.display()
        )
    })?;
    if let Err(error) = fs::create_dir(stage.join("data")) {
        let _ = fs::remove_dir(&stage);
        return Err(format!(
            "Failed to create retirement archive data stage {}: {error}",
            stage.join("data").display()
        ));
    }
    sync_directory(&stage)?;
    sync_directory(&paths.parent)?;
    Ok(stage)
}

fn publish_archive_stage(paths: &ArchivePaths, stage: &Path) -> Result<(), String> {
    let backup =
        allocate_unique_path(&paths.parent, &format!(".{}.backup", paths.remote_name), "")?;
    let had_final = path_exists(&paths.slot)?;
    if had_final {
        require_real_directory(&paths.slot, "existing local retirement archive")?;
        fs::rename(&paths.slot, &backup).map_err(|error| {
            format!(
                "Failed to stage existing retirement archive {} for replacement: {error}",
                paths.slot.display()
            )
        })?;
        sync_directory(&paths.parent)?;
    }
    if let Err(error) = fs::rename(stage, &paths.slot) {
        if had_final && !path_exists(&paths.slot).unwrap_or(true) {
            let _ = fs::rename(&backup, &paths.slot);
            let _ = sync_directory(&paths.parent);
        }
        return Err(format!(
            "Failed to atomically publish retirement archive {}: {error}",
            paths.slot.display()
        ));
    }
    sync_directory(&paths.parent)?;
    if had_final {
        remove_directory_tree(&backup, "replaced retirement archive backup")?;
        sync_directory(&paths.parent)?;
    }
    Ok(())
}

fn load_complete_archive(paths: &ArchivePaths) -> Result<CompleteArchive, String> {
    require_real_directory(&paths.slot, "local retirement archive")?;
    let mut entries = BTreeSet::new();
    for entry in fs::read_dir(&paths.slot).map_err(|error| {
        format!(
            "Failed to read local retirement archive {}: {error}",
            paths.slot.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read local retirement archive entry in {}: {error}",
                paths.slot.display()
            )
        })?;
        entries.insert(
            entry
                .file_name()
                .into_string()
                .map_err(|_| "Local retirement archive entry name must be UTF-8.".to_string())?,
        );
    }
    let required = BTreeSet::from(["data".to_string(), "remote.json".to_string()]);
    if !required.is_subset(&entries)
        || entries
            .iter()
            .any(|name| !matches!(name.as_str(), "data" | "remote.json" | "restore.json"))
    {
        return Err(format!(
            "Local retirement archive {} must contain only remote.json, data/, and optional restore.json.",
            paths.slot.display()
        ));
    }
    require_real_directory(&paths.slot.join("data"), "retirement archive data")?;
    require_regular_file(
        &paths.slot.join("remote.json"),
        "retirement archive remote.json",
    )?;
    let manifest = read_manifest_from_slot(&paths.slot)?;
    validate_data_inventory(&paths.slot.join("data"), &manifest)?;
    let receipt = read_restore_receipt(&paths.slot.join("restore.json"))?;
    Ok(CompleteArchive { manifest, receipt })
}

fn read_manifest_from_slot(slot: &Path) -> Result<RemoteExportManifest, String> {
    let value = read_json_file(&slot.join("remote.json"), "retirement archive remote.json")?;
    RemoteExportManifest::from_json(&value)
}

fn validate_data_inventory(
    data_root: &Path,
    manifest: &RemoteExportManifest,
) -> Result<(), String> {
    require_real_directory(data_root, "retirement archive data")?;
    let mut actual_files = Vec::new();
    let mut actual_directories = BTreeSet::new();
    collect_archive_inventory(
        data_root,
        data_root,
        &mut actual_files,
        &mut actual_directories,
    )?;
    actual_files.sort_by(|left, right| left.path.cmp(&right.path));
    if actual_files != manifest.files {
        return Err(
            "Local retirement archive data does not exactly match remote.json file inventory."
                .to_string(),
        );
    }
    let mut expected_directories = BTreeSet::new();
    for file in &manifest.files {
        let mut parent = Path::new(&file.path).parent();
        while let Some(path) = parent {
            if path.as_os_str().is_empty() {
                break;
            }
            expected_directories.insert(path.to_string_lossy().replace('\\', "/"));
            parent = path.parent();
        }
    }
    if actual_directories != expected_directories {
        return Err(
            "Local retirement archive data contains an unexpected or missing directory."
                .to_string(),
        );
    }
    Ok(())
}

fn collect_archive_inventory(
    root: &Path,
    current: &Path,
    files: &mut Vec<RemoteExportFile>,
    directories: &mut BTreeSet<String>,
) -> Result<(), String> {
    for entry in fs::read_dir(current).map_err(|error| {
        format!(
            "Failed to read archive directory {}: {error}",
            current.display()
        )
    })? {
        let entry = entry
            .map_err(|error| format!("Failed to read entry in {}: {error}", current.display()))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!("Failed to inspect archive path {}: {error}", path.display())
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Local retirement archive must not contain symbolic link {}.",
                path.display()
            ));
        }
        let relative = relative_archive_path(root, &path)?;
        if metadata.is_dir() {
            directories.insert(relative);
            collect_archive_inventory(root, &path, files, directories)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(format!(
                "Local retirement archive path {} is not a regular file.",
                path.display()
            ));
        }
        #[cfg(unix)]
        if metadata.nlink() != 1 {
            return Err(format!(
                "Local retirement archive file {} must not be hard-linked.",
                path.display()
            ));
        }
        let (size, sha256) = sha256_file(&path)?;
        files.push(RemoteExportFile {
            path: relative,
            size,
            sha256,
        });
    }
    Ok(())
}

fn relative_archive_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        format!(
            "Archive path {} escapes data root {}.",
            path.display(),
            root.display()
        )
    })?;
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(value) = component else {
            return Err("Archive path is not canonical.".to_string());
        };
        parts.push(
            value
                .to_str()
                .ok_or_else(|| "Archive path must be UTF-8.".to_string())?,
        );
    }
    let value = parts.join("/");
    validate_remote_authority_relative_path(&value)?;
    Ok(value)
}

fn validate_downloaded_bytes(expected: &RemoteExportFile, bytes: &[u8]) -> Result<(), String> {
    let size = u64::try_from(bytes.len())
        .map_err(|_| format!("Downloaded file {} exceeds u64 size.", expected.path))?;
    let sha256 = sha256_bytes(bytes);
    if size != expected.size || sha256 != expected.sha256 {
        return Err(format!(
            "Downloaded authority file {} failed size or SHA-256 verification.",
            expected.path
        ));
    }
    Ok(())
}

fn write_staged_authority_file(
    data_root: &Path,
    expected: &RemoteExportFile,
    bytes: &[u8],
) -> Result<(), String> {
    validate_remote_authority_relative_path(&expected.path)?;
    let target = data_root.join(&expected.path);
    let parent = target.parent().ok_or_else(|| {
        format!(
            "Staged authority file {} has no parent directory.",
            target.display()
        )
    })?;
    ensure_directory_tree(data_root, parent)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|error| {
            format!(
                "Failed to create staged authority file {}: {error}",
                target.display()
            )
        })?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            format!(
                "Failed to durably write staged authority file {}: {error}",
                target.display()
            )
        })?;
    sync_directory(parent)
}

fn ensure_directory_tree(root: &Path, target: &Path) -> Result<(), String> {
    require_real_directory(root, "archive data root")?;
    let relative = target.strip_prefix(root).map_err(|_| {
        format!(
            "Archive directory {} escapes data root {}.",
            target.display(),
            root.display()
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err("Archive directory path is not canonical.".to_string());
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "Archive directory {} must be a real directory.",
                        current.display()
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|error| {
                    format!(
                        "Failed to create archive directory {}: {error}",
                        current.display()
                    )
                })?;
                let parent = current
                    .parent()
                    .ok_or_else(|| "Archive directory has no parent.".to_string())?;
                sync_directory(parent)?;
            }
            Err(error) => {
                return Err(format!(
                    "Failed to inspect archive directory {}: {error}",
                    current.display()
                ))
            }
        }
    }
    Ok(())
}

fn read_verified_file(data_root: &Path, expected: &RemoteExportFile) -> Result<Vec<u8>, String> {
    validate_remote_authority_relative_path(&expected.path)?;
    let path = data_root.join(&expected.path);
    require_regular_file(&path, "retirement archive authority file")?;
    let bytes = fs::read(&path)
        .map_err(|error| format!("Failed to read archive file {}: {error}", path.display()))?;
    validate_downloaded_bytes(expected, &bytes)?;
    Ok(bytes)
}

fn clear_restore_receipt(paths: &ArchivePaths) -> Result<(), String> {
    let receipt = paths.slot.join("restore.json");
    if path_exists(&receipt)? {
        remove_regular_file_if_exists(&receipt, "restore receipt")?;
        sync_directory(&paths.slot)?;
    }
    Ok(())
}

fn read_restore_receipt(path: &Path) -> Result<Option<RestoreReceipt>, String> {
    if !path_exists(path)? {
        return Ok(None);
    }
    let value = read_json_file(path, "restore receipt")?;
    let object = value
        .as_object()
        .ok_or_else(|| "restore.json must contain a JSON object.".to_string())?;
    require_exact_keys(
        object,
        &["schema", "state", "restored_at_s", "repository_index"],
        "restore.json",
    )?;
    if required_string_object(object, "schema", "restore.json")? != RESTORE_RECEIPT_SCHEMA
        || required_string_object(object, "state", "restore.json")? != COMPLETE_STATE
    {
        return Err("restore.json has an unsupported schema or state.".to_string());
    }
    let restored_at_s = required_u32_object(object, "restored_at_s", "restore.json")?;
    if restored_at_s == 0 {
        return Err("restore.json restored_at_s must be non-zero.".to_string());
    }
    Ok(Some(RestoreReceipt {
        restored_at_s,
        repository_index: required_u32_object(object, "repository_index", "restore.json")?,
    }))
}

fn write_restore_receipt(paths: &ArchivePaths, receipt: &RestoreReceipt) -> Result<(), String> {
    let value = json!({
        "schema": RESTORE_RECEIPT_SCHEMA,
        "state": COMPLETE_STATE,
        "restored_at_s": receipt.restored_at_s,
        "repository_index": receipt.repository_index,
    });
    write_json_atomically(
        &paths.slot.join("restore.json"),
        &value,
        &paths.parent,
        &format!(".{}.restore-receipt", paths.remote_name),
    )?;
    sync_directory(&paths.slot)
}

fn read_restore_session(path: &Path) -> Result<Option<RestoreSession>, String> {
    if !path_exists(path)? {
        return Ok(None);
    }
    let value = read_json_file(path, "local restore session")?;
    let object = value
        .as_object()
        .ok_or_else(|| "Local restore session must contain a JSON object.".to_string())?;
    let state = required_string_object(object, "state", "local restore session")?;
    let expected = if state == UPLOADING_STATE {
        vec!["schema", "state", "restore_token"]
    } else if state == COMMITTED_STATE {
        vec!["schema", "state", "restore_token", "repository_index"]
    } else {
        return Err(format!(
            "Local restore session has unsupported state {state:?}."
        ));
    };
    require_exact_keys(object, &expected, "local restore session")?;
    if required_string_object(object, "schema", "local restore session")? != RESTORE_SESSION_SCHEMA
    {
        return Err("Local restore session has an unsupported schema.".to_string());
    }
    let restore_token = required_string_object(object, "restore_token", "local restore session")?;
    validate_restore_token(&restore_token)?;
    let repository_index = if state == COMMITTED_STATE {
        Some(required_u32_object(
            object,
            "repository_index",
            "local restore session",
        )?)
    } else {
        None
    };
    Ok(Some(RestoreSession {
        state,
        restore_token,
        repository_index,
    }))
}

fn write_restore_session(paths: &ArchivePaths, session: &RestoreSession) -> Result<(), String> {
    validate_restore_token(&session.restore_token)?;
    let value = if session.state == UPLOADING_STATE && session.repository_index.is_none() {
        json!({
            "schema": RESTORE_SESSION_SCHEMA,
            "state": UPLOADING_STATE,
            "restore_token": session.restore_token,
        })
    } else if session.state == COMMITTED_STATE {
        json!({
            "schema": RESTORE_SESSION_SCHEMA,
            "state": COMMITTED_STATE,
            "restore_token": session.restore_token,
            "repository_index": session.repository_index.ok_or_else(|| {
                "Committed local restore session requires repository_index.".to_string()
            })?,
        })
    } else {
        return Err("Local restore session state is internally inconsistent.".to_string());
    };
    write_json_atomically(
        &paths.session,
        &value,
        &paths.parent,
        &format!(".{}.restore-session.json", paths.remote_name),
    )
}

fn remove_restore_session_artifacts(paths: &ArchivePaths) -> Result<(), String> {
    remove_regular_file_if_exists(&paths.session, "local restore session")?;
    let prefix = format!(".{}.restore-session.json-", paths.remote_name);
    let receipt_prefix = format!(".{}.restore-receipt-", paths.remote_name);
    for entry in fs::read_dir(&paths.parent).map_err(|error| {
        format!(
            "Failed to scan local restore session directory {}: {error}",
            paths.parent.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "Failed to read local restore session entry in {}: {error}",
                paths.parent.display()
            )
        })?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Local restore session entry name must be UTF-8.".to_string())?;
        if name.starts_with(&prefix) || name.starts_with(&receipt_prefix) {
            remove_regular_file_if_exists(&entry.path(), "stale local restore JSON stage")?;
        }
    }
    sync_directory(&paths.parent)
}

fn replace_config_repository_index(
    repo: &RepoRuntime,
    repository_index: u32,
) -> Result<(), String> {
    let config_path = repo.authoritative_repo_root().join(".ait/config.json");
    let value = read_json_file(&config_path, "repository config")?;
    let mut config = value
        .as_object()
        .cloned()
        .ok_or_else(|| "Repository config must contain a JSON object.".to_string())?;
    if config.get("repository_index").and_then(JsonValue::as_u64)
        == Some(u64::from(repository_index))
    {
        return Ok(());
    }
    if config.get("repository_index").is_some()
        && config
            .get("repository_index")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .is_none()
    {
        return Err(
            "Repository config repository_index must be an unsigned 32-bit integer.".to_string(),
        );
    }
    config.insert(
        "repository_index".to_string(),
        JsonValue::from(repository_index),
    );
    let parent = config_path
        .parent()
        .ok_or_else(|| "Repository config has no parent directory.".to_string())?;
    write_json_atomically(
        &config_path,
        &JsonValue::Object(config),
        parent,
        ".config.json.restore",
    )
}

fn write_new_json_file(path: &Path, value: &JsonValue) -> Result<(), String> {
    let encoded = encode_value_pretty_with_newline_error_string(value)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Failed to create JSON file {}: {error}", path.display()))?;
    file.write_all(encoded.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            format!(
                "Failed to durably write JSON file {}: {error}",
                path.display()
            )
        })
}

fn write_json_atomically(
    target: &Path,
    value: &JsonValue,
    staging_parent: &Path,
    staging_prefix: &str,
) -> Result<(), String> {
    require_real_directory(staging_parent, "JSON staging directory")?;
    if path_exists(target)? {
        require_regular_file(target, "JSON target")?;
    }
    let encoded = encode_value_pretty_with_newline_error_string(value)?;
    let staged = allocate_unique_path(staging_parent, staging_prefix, ".tmp")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .map_err(|error| {
            format!(
                "Failed to create staged JSON file {}: {error}",
                staged.display()
            )
        })?;
    if let Ok(metadata) = fs::metadata(target) {
        if let Err(error) = file.set_permissions(metadata.permissions()) {
            drop(file);
            let _ = fs::remove_file(&staged);
            return Err(format!(
                "Failed to preserve permissions for JSON file {}: {error}",
                target.display()
            ));
        }
    }
    file.write_all(encoded.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| {
            let _ = fs::remove_file(&staged);
            format!(
                "Failed to durably stage JSON file {}: {error}",
                staged.display()
            )
        })?;
    drop(file);
    fs::rename(&staged, target).map_err(|error| {
        let _ = fs::remove_file(&staged);
        format!(
            "Failed to atomically publish JSON file {}: {error}",
            target.display()
        )
    })?;
    let target_parent = target
        .parent()
        .ok_or_else(|| format!("JSON target {} has no parent.", target.display()))?;
    sync_directory(target_parent)?;
    if target_parent != staging_parent {
        sync_directory(staging_parent)?;
    }
    Ok(())
}

fn read_json_file(path: &Path, label: &str) -> Result<JsonValue, String> {
    require_regular_file(path, label)?;
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path.display()))?;
    parse_value(
        &text,
        &format!("Failed to parse {label} {}", path.display()),
    )
}

fn require_exact_keys(
    object: &JsonMap<String, JsonValue>,
    expected: &[&str],
    label: &str,
) -> Result<(), String> {
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{label} fields are not exact."));
    }
    Ok(())
}

fn required_string(value: &JsonValue, field: &str, label: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{label} requires string {field}."))
}

fn required_bool(value: &JsonValue, field: &str, label: &str) -> Result<bool, String> {
    value
        .get(field)
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| format!("{label} requires boolean {field}."))
}

fn required_string_object(
    object: &JsonMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<String, String> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{label} requires string {field}."))
}

fn required_u32_object(
    object: &JsonMap<String, JsonValue>,
    field: &str,
    label: &str,
) -> Result<u32, String> {
    object
        .get(field)
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("{label} requires unsigned 32-bit integer {field}."))
}

fn validate_restore_token(value: &str) -> Result<(), String> {
    if value.len() != 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(
            "Repository restore token must contain exactly 32 lowercase hexadecimal characters."
                .to_string(),
        );
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<(u64, String), String> {
    require_regular_file(path, "archive authority file")?;
    let mut file = File::open(path)
        .map_err(|error| format!("Failed to open archive file {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("Failed to read archive file {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(u64::try_from(read).map_err(|_| "Read size exceeds u64.".to_string())?)
            .ok_or_else(|| format!("Archive file {} exceeds u64 size.", path.display()))?;
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok((size, hex_digest(&digest)))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_digest(&digest)
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push_str(&format!("{byte:02x}"));
    }
    output
}

fn require_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "{label} {} must be a real directory.",
            path.display()
        ));
    }
    Ok(())
}

fn require_regular_file(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "{label} {} must be a regular file.",
            path.display()
        ));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(format!(
            "{label} {} must not be hard-linked.",
            path.display()
        ));
    }
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Failed to inspect {}: {error}", path.display())),
    }
}

fn remove_regular_file_if_exists(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "Refusing to remove {label} {} because it is not a regular file.",
                    path.display()
                ));
            }
            fs::remove_file(path)
                .map_err(|error| format!("Failed to remove {label} {}: {error}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "Failed to inspect {label} {}: {error}",
            path.display()
        )),
    }
}

fn remove_directory_tree(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Failed to inspect removal path {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "Refusing recursive removal because {} is not a real directory.",
            path.display()
        ));
    }
    for entry in fs::read_dir(path)
        .map_err(|error| format!("Failed to scan removal path {}: {error}", path.display()))?
    {
        let entry = entry
            .map_err(|error| format!("Failed to read entry in {}: {error}", path.display()))?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(|error| {
            format!(
                "Failed to inspect removal path {}: {error}",
                child.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            fs::remove_file(&child).map_err(|error| {
                format!(
                    "Failed to remove symbolic link {} from {label}: {error}",
                    child.display()
                )
            })?;
        } else if metadata.is_dir() {
            remove_directory_tree(&child, label)?;
        } else if metadata.is_file() {
            fs::remove_file(&child).map_err(|error| {
                format!(
                    "Failed to remove file {} from {label}: {error}",
                    child.display()
                )
            })?;
        } else {
            return Err(format!(
                "Refusing recursive removal because {} is not a regular file.",
                child.display()
            ));
        }
    }
    fs::remove_dir(path)
        .map_err(|error| format!("Failed to remove {label} {}: {error}", path.display()))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Failed to sync directory {}: {error}", path.display()))
}

#[cfg(windows)]
fn sync_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Failed to inspect directory {}: {error}", path.display()))?;
    if !metadata.is_dir() {
        return Err(format!(
            "Directory sync target {} is not a directory.",
            path.display()
        ));
    }
    // File contents are synced before every rename. std does not expose a
    // Windows directory handle with Unix-style fsync semantics.
    Ok(())
}

fn allocate_unique_path(parent: &Path, prefix: &str, suffix: &str) -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock is before Unix epoch: {error}"))?
        .as_nanos();
    for attempt in 0_u8..=32 {
        let candidate = parent.join(format!(
            "{prefix}-{}-{nonce}-{attempt}{suffix}",
            std::process::id()
        ));
        if !path_exists(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Could not allocate unique local path in {}.",
        parent.display()
    ))
}

fn now_s() -> Result<u32, String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock is before Unix epoch: {error}"))?
        .as_secs();
    u32::try_from(seconds).map_err(|_| "Current Unix time exceeds u32.".to_string())
}

fn http_config(repo: &RepoRuntime, remote: &RemoteRow) -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: remote.url.clone(),
        repository_index: repo.repository_index(),
        headers: repo.auth_headers(),
        ..PlanHttpClientConfig::default()
    }
}

fn is_already_purged(error: &PlanHttpClientError) -> bool {
    error.remote_status() == Some(409)
        && error
            .remote_detail()
            .is_some_and(|detail| detail.contains("already purged"))
}

fn shell_word(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::thread;
    use std::time::Duration;
    use tiny_http::{Header, Response, Server, StatusCode};

    #[derive(Clone, Debug)]
    struct TestResponse {
        status: u16,
        content_type: &'static str,
        body: Vec<u8>,
    }

    #[derive(Clone, Debug)]
    struct ObservedRequest {
        method: String,
        path: String,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }

    fn json_response(status: u16, value: JsonValue) -> TestResponse {
        TestResponse {
            status,
            content_type: "application/json",
            body: value.to_string().into_bytes(),
        }
    }

    fn bytes_response(bytes: &[u8]) -> TestResponse {
        TestResponse {
            status: 200,
            content_type: ait_core::server_repo_retire::REMOTE_AUTHORITY_FILE_MEDIA_TYPE,
            body: bytes.to_vec(),
        }
    }

    fn spawn_test_server(
        responses: Vec<TestResponse>,
    ) -> (
        String,
        thread::JoinHandle<Result<Vec<ObservedRequest>, String>>,
    ) {
        let server = Server::http("127.0.0.1:0").expect("bind test server");
        let address = server.server_addr();
        let handle = thread::spawn(move || {
            let mut observed = Vec::new();
            for response in responses {
                let Some(mut request) = server
                    .recv_timeout(Duration::from_secs(10))
                    .map_err(|error| error.to_string())?
                else {
                    return Err("Timed out waiting for retirement protocol request.".to_string());
                };
                let mut body = Vec::new();
                request
                    .as_reader()
                    .read_to_end(&mut body)
                    .map_err(|error| error.to_string())?;
                let headers = request
                    .headers()
                    .iter()
                    .map(|header| {
                        (
                            header.field.to_string().to_ascii_lowercase(),
                            header.value.as_str().to_string(),
                        )
                    })
                    .collect();
                observed.push(ObservedRequest {
                    method: request.method().as_str().to_string(),
                    path: request.url().to_string(),
                    headers,
                    body,
                });
                let header = Header::from_bytes("Content-Type", response.content_type)
                    .map_err(|_| "Failed to construct test Content-Type.".to_string())?;
                request
                    .respond(
                        Response::from_data(response.body)
                            .with_status_code(StatusCode(response.status))
                            .with_header(header),
                    )
                    .map_err(|error| error.to_string())?;
            }
            Ok(observed)
        });
        (format!("http://{address}"), handle)
    }

    fn write_repo_fixture(
        root: &Path,
        server_url: &str,
        repository_index: Option<u32>,
    ) -> RepoRuntime {
        let ait_dir = root.join(".ait");
        fs::create_dir_all(&ait_dir).expect("create fixture .ait");
        let mut config = json!({
            "repo_name": "duplicate-name",
            "default_line": "main",
            "id_namespace_prefix": "R",
            "policy_profile": "prototype",
            "default_remote": "origin",
            "remotes": {
                "origin": {
                    "remote_id": 1,
                    "url": server_url,
                    "repo_name": "duplicate-name",
                    "created_at": "2026-07-31T00:00:00Z",
                },
            },
        });
        if let Some(repository_index) = repository_index {
            config["repository_index"] = json!(repository_index);
        }
        fs::write(
            ait_dir.join("config.json"),
            encode_value_pretty_with_newline_error_string(&config).expect("encode fixture config"),
        )
        .expect("write fixture config");
        fs::write(
            ait_dir.join("policy.yaml"),
            "version: 1\npolicy_id: prototype\ndefaults:\n  require_attestation: true\n  require_tests: false\n",
        )
        .expect("write fixture policy");
        RepoRuntime::discover_from_path(root).expect("discover fixture")
    }

    #[test]
    fn retirement_then_restore_publishes_exact_slot_and_rebinds_to_new_index() {
        let first_path = "nested/worker job.bin";
        let first_bytes = b"worker-job-authority".to_vec();
        let second_path = "repository.bin";
        let second_bytes = b"repository-authority".to_vec();
        let manifest = RemoteExportManifest {
            schema: ait_core::server_repo_retire::REMOTE_EXPORT_SCHEMA.to_string(),
            state: ait_core::server_repo_retire::REMOTE_EXPORT_STATE_COMPLETE.to_string(),
            repo_name: "duplicate-name".to_string(),
            namespace: "R".to_string(),
            exported_at_s: 1_786_000_000,
            files: vec![
                RemoteExportFile {
                    path: first_path.to_string(),
                    size: first_bytes.len() as u64,
                    sha256: sha256_bytes(&first_bytes),
                },
                RemoteExportFile {
                    path: second_path.to_string(),
                    size: second_bytes.len() as u64,
                    sha256: sha256_bytes(&second_bytes),
                },
            ],
        };
        manifest.validate().expect("valid fixture manifest");
        let token = "0123456789abcdef0123456789abcdef";
        let manifest_json = manifest.to_json().expect("manifest JSON");
        let responses = vec![
            json_response(
                200,
                json!({
                    "contract": "ait.server.repository-retirement.v1",
                    "repository": {
                        "repository_index": 7,
                        "repository_name": "duplicate-name",
                        "namespace": "R",
                    },
                    "drain": {
                        "queued_worker_jobs": 0,
                        "running_worker_jobs": 0,
                    },
                    "ready_for_export": true,
                    "manifest": manifest_json,
                }),
            ),
            bytes_response(&first_bytes),
            bytes_response(&second_bytes),
            json_response(
                200,
                json!({
                    "contract": "ait.server.repository-purge.v1",
                    "repository": {
                        "repository_index": 7,
                        "lifecycle_kind": 3,
                    },
                }),
            ),
            json_response(
                201,
                json!({
                    "contract": "ait.server.repository-restore-session.v1",
                    "restore_token": token,
                    "state": "uploading",
                }),
            ),
            json_response(
                200,
                json!({
                    "contract": "ait.server.repository-restore-session.v1",
                    "restore_token": token,
                    "state": "uploading",
                    "file": manifest.files[0].clone(),
                }),
            ),
            json_response(
                200,
                json!({
                    "contract": "ait.server.repository-restore-session.v1",
                    "restore_token": token,
                    "state": "uploading",
                    "file": manifest.files[1].clone(),
                }),
            ),
            json_response(
                200,
                json!({
                    "contract": "ait.server.repository-restore.v1",
                    "created": true,
                    "repository": {
                        "repository_index": 9,
                        "repository_name": "duplicate-name",
                        "namespace": "R",
                        "tombstoned": false,
                    },
                }),
            ),
        ];
        let (url, server) = spawn_test_server(responses);
        let temp = tempfile::tempdir().expect("temp repository");
        let repo = write_repo_fixture(temp.path(), &url, Some(7));

        let retired = retire_repository_unlocked(
            &repo,
            &RepoRetireRequest {
                remote_name: Some("origin".to_string()),
                abort: false,
                replace_export: false,
            },
        )
        .expect("retire Repository");
        assert_eq!(retired["state"], json!("purged"));
        let slot = temp.path().join(".ait/remote/origin");
        assert!(slot.join("remote.json").is_file());
        assert!(slot.join("data").is_dir());
        assert!(!slot.join("restore.json").exists());
        let remote_json =
            read_json_file(&slot.join("remote.json"), "test remote.json").expect("read manifest");
        assert_eq!(remote_json, manifest.to_json().expect("manifest JSON"));
        for forbidden in [
            "server_instance_id",
            "old_repository_index",
            "archive_sha256",
            "local_export_generation",
        ] {
            assert!(remote_json.get(forbidden).is_none(), "{forbidden}");
        }

        let restored = restore_repository_unlocked(
            &repo,
            &RepoRestoreRequest {
                remote_name: Some("origin".to_string()),
            },
        )
        .expect("restore Repository");
        assert_eq!(restored["repository_index"], json!(9));
        assert_eq!(restored["already_restored"], json!(false));
        let config =
            read_json_file(&temp.path().join(".ait/config.json"), "test config").expect("config");
        assert_eq!(config["repository_index"], json!(9));
        let receipt =
            read_json_file(&slot.join("restore.json"), "test receipt").expect("restore receipt");
        let receipt_keys = receipt
            .as_object()
            .expect("receipt object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            receipt_keys,
            BTreeSet::from(["repository_index", "restored_at_s", "schema", "state"])
        );
        assert_eq!(receipt["repository_index"], json!(9));
        assert!(!temp
            .path()
            .join(".ait/remote/.origin.restore-session.json")
            .exists());

        let observed = server.join().expect("join test server").expect("server");
        assert_eq!(observed.len(), 8);
        assert_eq!(
            observed
                .iter()
                .map(|request| (request.method.as_str(), request.path.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (
                    "POST",
                    "/v1/native/repository-authorities/7/retirement"
                ),
                (
                    "GET",
                    "/v1/native/repository-authorities/7/retirement/files/nested/worker%20job.bin"
                ),
                (
                    "GET",
                    "/v1/native/repository-authorities/7/retirement/files/repository.bin"
                ),
                (
                    "POST",
                    "/v1/native/repository-authorities/7/retirement/purge"
                ),
                ("POST", "/v1/native/repository-restores"),
                (
                    "PUT",
                    "/v1/native/repository-restores/0123456789abcdef0123456789abcdef/files/nested/worker%20job.bin"
                ),
                (
                    "PUT",
                    "/v1/native/repository-restores/0123456789abcdef0123456789abcdef/files/repository.bin"
                ),
                (
                    "POST",
                    "/v1/native/repository-restores/0123456789abcdef0123456789abcdef/commit"
                ),
            ]
        );
        assert_eq!(
            parse_value(
                std::str::from_utf8(&observed[3].body).expect("purge UTF-8"),
                "purge body",
            )
            .expect("purge JSON"),
            manifest.to_json().expect("manifest JSON"),
        );
        assert_eq!(observed[5].body, first_bytes);
        assert_eq!(observed[6].body, second_bytes);
        for request in [&observed[5], &observed[6]] {
            assert_eq!(
                request.headers.get("content-type").map(String::as_str),
                Some(ait_core::server_repo_retire::REMOTE_AUTHORITY_FILE_MEDIA_TYPE)
            );
        }

        let refreshed =
            RepoRuntime::discover_from_path(temp.path()).expect("refresh restored repo");
        let repeated = restore_repository_unlocked(
            &refreshed,
            &RepoRestoreRequest {
                remote_name: Some("origin".to_string()),
            },
        )
        .expect("repeat restore uses local receipt without another server allocation");
        assert_eq!(repeated["repository_index"], json!(9));
        assert_eq!(repeated["already_restored"], json!(true));
    }

    #[test]
    fn retirement_reports_worker_job_blockers_without_creating_or_purging_archive() {
        let (url, server) = spawn_test_server(vec![json_response(
            200,
            json!({
                "contract": "ait.server.repository-retirement.v1",
                "repository": {
                    "repository_index": 7,
                    "repository_name": "duplicate-name",
                    "namespace": "R",
                },
                "drain": {
                    "queued_worker_jobs": 2,
                    "running_worker_jobs": 1,
                },
                "ready_for_export": false,
                "manifest": JsonValue::Null,
            }),
        )]);
        let temp = tempfile::tempdir().expect("temp repository");
        let repo = write_repo_fixture(temp.path(), &url, Some(7));
        let result = retire_repository_unlocked(
            &repo,
            &RepoRetireRequest {
                remote_name: Some("origin".to_string()),
                abort: false,
                replace_export: false,
            },
        )
        .expect("report retirement blockers");
        assert_eq!(result["state"], json!("waiting_for_jobs"));
        assert_eq!(result["ready_for_export"], json!(false));
        assert_eq!(result["drain"]["queued_worker_jobs"], json!(2));
        assert_eq!(result["drain"]["running_worker_jobs"], json!(1));
        assert!(!temp.path().join(".ait/remote").exists());
        let observed = server.join().expect("join test server").expect("server");
        assert_eq!(observed.len(), 1);
        assert_eq!(
            (observed[0].method.as_str(), observed[0].path.as_str()),
            ("POST", "/v1/native/repository-authorities/7/retirement")
        );
    }

    #[test]
    fn retirement_abort_is_idempotent_and_preserves_completed_archive_and_config() {
        let (url, server) = spawn_test_server(vec![json_response(
            200,
            json!({
                "contract": SERVER_RETIREMENT_ABORT_CONTRACT,
                "aborted": true,
                "already_aborted": true,
                "repository": {
                    "repository_index": 7,
                    "repository_name": "duplicate-name",
                    "namespace": "R",
                    "lifecycle_kind": ACTIVE_REPOSITORY_LIFECYCLE_KIND,
                    "tombstoned": false,
                },
            }),
        )]);
        let temp = tempfile::tempdir().expect("temp repository");
        let repo = write_repo_fixture(temp.path(), &url, Some(7));
        let parent = temp.path().join(".ait/remote");
        let slot = parent.join("origin");
        fs::create_dir_all(slot.join("data")).expect("create complete archive fixture");
        let remote_json = b"{\"complete\":\"archive\"}\n";
        let restore_json = b"{\"complete\":\"restore\"}\n";
        let authority = b"preserved authority";
        fs::write(slot.join("remote.json"), remote_json).expect("write remote.json fixture");
        fs::write(slot.join("restore.json"), restore_json).expect("write restore.json fixture");
        fs::write(slot.join("data/repository.bin"), authority).expect("write authority fixture");
        let stage = parent.join(".origin.staging-interrupted");
        let backup = parent.join(".origin.backup-interrupted");
        fs::create_dir_all(&stage).expect("create stale stage");
        fs::create_dir_all(&backup).expect("create stale backup");
        fs::write(stage.join("partial.bin"), b"partial").expect("write stale stage");
        fs::write(backup.join("old.bin"), b"old").expect("write stale backup");
        let session_temp = parent.join(".origin.restore-session.json-interrupted");
        let receipt_temp = parent.join(".origin.restore-receipt-interrupted");
        fs::write(&session_temp, b"partial").expect("write stale session temp");
        fs::write(&receipt_temp, b"partial").expect("write stale receipt temp");
        let config_path = temp.path().join(".ait/config.json");
        let config_before = fs::read(&config_path).expect("read config before abort");

        let result = retire_repository_unlocked(
            &repo,
            &RepoRetireRequest {
                remote_name: Some("origin".to_string()),
                abort: true,
                replace_export: false,
            },
        )
        .expect("abort Repository retirement");
        assert_eq!(result["state"], json!("active"));
        assert_eq!(result["operation"], json!("aborted"));
        assert_eq!(result["aborted"], json!(true));
        assert_eq!(result["already_aborted"], json!(true));
        assert_eq!(result["local_transients_cleaned"], json!(true));
        assert_eq!(result["archive_preserved"], json!(true));
        assert!(!stage.exists());
        assert!(!backup.exists());
        assert!(!session_temp.exists());
        assert!(!receipt_temp.exists());
        assert_eq!(
            fs::read(slot.join("remote.json")).expect("read preserved remote.json"),
            remote_json
        );
        assert_eq!(
            fs::read(slot.join("restore.json")).expect("read preserved restore.json"),
            restore_json
        );
        assert_eq!(
            fs::read(slot.join("data/repository.bin")).expect("read preserved authority"),
            authority
        );
        assert_eq!(
            fs::read(&config_path).expect("read config after abort"),
            config_before
        );

        let observed = server.join().expect("join test server").expect("server");
        assert_eq!(observed.len(), 1);
        assert_eq!(
            (observed[0].method.as_str(), observed[0].path.as_str()),
            (
                "POST",
                "/v1/native/repository-authorities/7/retirement/abort"
            )
        );
        assert_eq!(
            parse_value(
                std::str::from_utf8(&observed[0].body).expect("abort body UTF-8"),
                "abort body",
            )
            .expect("abort body JSON"),
            json!({})
        );
    }

    #[test]
    fn retirement_abort_validation_failure_preserves_local_transients() {
        let (url, server) = spawn_test_server(vec![json_response(
            200,
            json!({
                "contract": SERVER_RETIREMENT_ABORT_CONTRACT,
                "aborted": false,
                "already_aborted": false,
                "repository": {
                    "repository_index": 7,
                    "lifecycle_kind": ACTIVE_REPOSITORY_LIFECYCLE_KIND,
                    "tombstoned": false,
                },
            }),
        )]);
        let temp = tempfile::tempdir().expect("temp repository");
        let repo = write_repo_fixture(temp.path(), &url, Some(7));
        let parent = temp.path().join(".ait/remote");
        let stage = parent.join(".origin.staging-interrupted");
        let backup = parent.join(".origin.backup-interrupted");
        fs::create_dir_all(&stage).expect("create stale stage");
        fs::create_dir_all(&backup).expect("create stale backup");
        fs::write(stage.join("partial.bin"), b"partial").expect("write stale stage");
        fs::write(backup.join("old.bin"), b"old").expect("write stale backup");
        let session_temp = parent.join(".origin.restore-session.json-interrupted");
        fs::write(&session_temp, b"partial").expect("write stale session temp");
        let config_path = temp.path().join(".ait/config.json");
        let config_before = fs::read(&config_path).expect("read config before abort");

        let error = retire_repository_unlocked(
            &repo,
            &RepoRetireRequest {
                remote_name: Some("origin".to_string()),
                abort: true,
                replace_export: false,
            },
        )
        .expect_err("invalid abort response must fail");
        assert!(error.contains("must report aborted=true"), "{error}");
        assert!(stage.is_dir());
        assert!(backup.is_dir());
        assert!(session_temp.is_file());
        assert_eq!(
            fs::read(&config_path).expect("read config after failed abort"),
            config_before
        );

        let observed = server.join().expect("join test server").expect("server");
        assert_eq!(observed.len(), 1);
        assert_eq!(
            observed[0].path,
            "/v1/native/repository-authorities/7/retirement/abort"
        );
    }

    #[test]
    fn restore_resumes_the_recorded_upload_session_without_allocating_another() {
        let bytes = b"repository-authority".to_vec();
        let manifest = RemoteExportManifest {
            schema: ait_core::server_repo_retire::REMOTE_EXPORT_SCHEMA.to_string(),
            state: ait_core::server_repo_retire::REMOTE_EXPORT_STATE_COMPLETE.to_string(),
            repo_name: "duplicate-name".to_string(),
            namespace: "R".to_string(),
            exported_at_s: 1_786_000_000,
            files: vec![RemoteExportFile {
                path: "repository.bin".to_string(),
                size: bytes.len() as u64,
                sha256: sha256_bytes(&bytes),
            }],
        };
        let token = "fedcba9876543210fedcba9876543210";
        let (url, server) = spawn_test_server(vec![
            json_response(
                200,
                json!({
                    "contract": "ait.server.repository-restore-session.v1",
                    "restore_token": token,
                    "state": "uploading",
                    "file": manifest.files[0].clone(),
                }),
            ),
            json_response(
                200,
                json!({
                    "contract": "ait.server.repository-restore.v1",
                    "created": true,
                    "repository": {
                        "repository_index": 9,
                        "repository_name": "duplicate-name",
                        "namespace": "R",
                        "tombstoned": false,
                    },
                }),
            ),
        ]);
        let temp = tempfile::tempdir().expect("temp repository");
        let repo = write_repo_fixture(temp.path(), &url, Some(7));
        let paths = archive_paths(&repo, "origin", true)
            .expect("archive paths")
            .expect("archive root");
        prepare_complete_archive(&paths, &manifest, 7, false, |_| Ok(bytes.clone()))
            .expect("publish archive");
        write_restore_session(
            &paths,
            &RestoreSession {
                state: UPLOADING_STATE.to_string(),
                restore_token: token.to_string(),
                repository_index: None,
            },
        )
        .expect("write interrupted restore session");

        let restored = restore_repository_unlocked(
            &repo,
            &RepoRestoreRequest {
                remote_name: Some("origin".to_string()),
            },
        )
        .expect("resume restore");
        assert_eq!(restored["repository_index"], json!(9));
        let observed = server.join().expect("join test server").expect("server");
        assert_eq!(
            observed
                .iter()
                .map(|request| (request.method.as_str(), request.path.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (
                    "PUT",
                    "/v1/native/repository-restores/fedcba9876543210fedcba9876543210/files/repository.bin"
                ),
                (
                    "POST",
                    "/v1/native/repository-restores/fedcba9876543210fedcba9876543210/commit"
                ),
            ]
        );
    }

    #[test]
    fn fresh_registration_requires_explicit_archive_discard() {
        let temp = tempfile::tempdir().expect("temp repository");
        let repo = write_repo_fixture(temp.path(), "https://example.invalid", None);
        let slot = temp.path().join(".ait/remote/origin");
        fs::create_dir_all(slot.join("data")).expect("create archive fixture");
        fs::write(slot.join("remote.json"), "{}\n").expect("write archive fixture");

        let error = enforce_fresh_registration_archive_policy(&repo, "origin", false).unwrap_err();
        assert!(
            error.contains("ait repo restore --remote origin"),
            "{error}"
        );
        assert!(error.contains("--discard-export"), "{error}");
        assert!(slot.exists());

        assert!(
            enforce_fresh_registration_archive_policy(&repo, "origin", true)
                .expect("explicit discard")
        );
        assert!(!slot.exists());
    }

    #[test]
    fn unrelated_archive_requires_replace_but_current_restore_receipt_allows_next_cycle() {
        let temp = tempfile::tempdir().expect("temp repository");
        let repo = write_repo_fixture(temp.path(), "https://example.invalid", Some(7));
        let paths = archive_paths(&repo, "origin", true)
            .expect("archive paths")
            .expect("archive root");
        let old_bytes = b"old";
        let old_manifest = RemoteExportManifest {
            schema: ait_core::server_repo_retire::REMOTE_EXPORT_SCHEMA.to_string(),
            state: ait_core::server_repo_retire::REMOTE_EXPORT_STATE_COMPLETE.to_string(),
            repo_name: "duplicate-name".to_string(),
            namespace: "R".to_string(),
            exported_at_s: 1_786_000_000,
            files: vec![RemoteExportFile {
                path: "repository.bin".to_string(),
                size: old_bytes.len() as u64,
                sha256: sha256_bytes(old_bytes),
            }],
        };
        prepare_complete_archive(&paths, &old_manifest, 7, false, |_| Ok(old_bytes.to_vec()))
            .expect("publish old archive");
        let new_bytes = b"new";
        let new_manifest = RemoteExportManifest {
            exported_at_s: 1_786_000_100,
            files: vec![RemoteExportFile {
                path: "repository.bin".to_string(),
                size: new_bytes.len() as u64,
                sha256: sha256_bytes(new_bytes),
            }],
            ..old_manifest.clone()
        };
        let error =
            prepare_complete_archive(&paths, &new_manifest, 7, false, |_| Ok(new_bytes.to_vec()))
                .unwrap_err();
        assert!(error.contains("--replace-export"), "{error}");

        write_restore_receipt(
            &paths,
            &RestoreReceipt {
                restored_at_s: 1_786_000_050,
                repository_index: 7,
            },
        )
        .expect("write current restore receipt");
        prepare_complete_archive(&paths, &new_manifest, 7, false, |_| Ok(new_bytes.to_vec()))
            .expect("current restore receipt authorizes next retirement cycle");
        clear_restore_receipt(&paths).expect("clear stale restore receipt");
        let loaded = load_complete_archive(&paths).expect("load replacement archive");
        assert_eq!(loaded.manifest, new_manifest);
        assert!(loaded.receipt.is_none());
    }
}
