use super::*;
use crate::primitives::workflow::{workflow_find_bound_task_worktree_metadata, workflow_root_repo};

const MAIN_SEED_HASH_WORKERS: usize = 9;

fn normalize_workspace_cargo_projection_target(workspace_path: &Path, projection: &str) -> String {
    let relative_target = format!("target-dir = \".ait/{SHARED_CARGO_TARGET_DIRNAME}\"\n");
    let generated = generated_worktree_cargo_config_text(workspace_path);
    let canonical_target = generated
        .lines()
        .find(|line| line.starts_with("target-dir = "))
        .map(|line| format!("{line}\n"));
    canonical_target.map_or_else(
        || projection.to_string(),
        |canonical_target| projection.replacen(&relative_target, &canonical_target, 1),
    )
}

fn validate_workspace_cargo_projection_for_snapshot(
    repo: &RepoRuntime,
    target_snapshot_id: &str,
    workspace_path: &Path,
    is_worktree: bool,
) -> Result<bool, String> {
    if !is_worktree {
        return Ok(false);
    }
    let repo_workspace_root = repo.workspace_root();
    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&repo_workspace_root)?;
    let Some(row) =
        store.snapshot_tree_path_row(target_snapshot_id, WORKTREE_CARGO_CONFIG_RELATIVE_PATH)?
    else {
        return Ok(false);
    };
    let Some(blob_id) = file_map_row_blob_id(&row) else {
        return Err(format!(
            "Snapshot {target_snapshot_id} Cargo configuration has no Blob identity."
        ));
    };
    let source = read_snapshot_blob_text(repo, &blob_id)?;
    let Some(expected) = upgrade_generated_worktree_cargo_config_text(workspace_path, &source)
    else {
        return Ok(false);
    };
    let config_path = workspace_path.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    let metadata = fs::symlink_metadata(&config_path).map_err(|err| {
        format!(
            "Managed Cargo file metadata is unavailable at {}: {err}",
            config_path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Managed Cargo path must be a physical file at {}.",
            config_path.display()
        ));
    }
    let actual = fs::read_to_string(&config_path)
        .map_err(|err| format!("Failed to read {}: {err}", config_path.display()))?;
    let Some(current_actual) =
        upgrade_generated_worktree_cargo_config_text(workspace_path, &actual)
    else {
        return Err(format!(
            "Managed Cargo file at {} is not recognized as a Task-worktree file.",
            config_path.display()
        ));
    };
    if actual != current_actual
        || normalize_workspace_cargo_projection_target(workspace_path, &actual)
            != normalize_workspace_cargo_projection_target(workspace_path, &expected)
    {
        return Err(format!(
            "Managed Cargo file does not match Snapshot {target_snapshot_id}: expected_sha256={}, actual_sha256={}.",
            sha256_hex_bytes(expected.as_bytes()),
            sha256_hex_bytes(actual.as_bytes())
        ));
    }
    Ok(true)
}

pub(in crate::primitives) fn main_seed_aligned_result(
    line_name: &str,
    target_snapshot_id: &str,
    seed_path: &Path,
    seed_state: &MainSeedState,
    root_source: Option<&str>,
) -> MainSeedMirrorResult {
    MainSeedMirrorResult {
        status: "aligned".to_string(),
        name: seed_state
            .worktree_name
            .clone()
            .unwrap_or_else(|| main_seed_worktree_name(line_name)),
        path: seed_path.to_path_buf(),
        line_name: line_name.to_string(),
        seed_snapshot_id: target_snapshot_id.to_string(),
        root_source: root_source.and_then(|value| normalized_text(Some(value))),
        seed_refreshed_at: seed_state.seed_refreshed_at.clone(),
        baseline_seed_snapshot_id: None,
        refresh_strategy: "already_aligned".to_string(),
        copy_strategy: None,
        copy_error: None,
        materialized_write_count: None,
        materialized_remove_count: None,
        materialized_unchanged_count: None,
        phase_timings_ms: None,
        error: None,
    }
}

pub(in crate::primitives) fn validate_workspace_for_snapshot(
    repo: &RepoRuntime,
    target_snapshot_id: &str,
    workspace_path: &Path,
    is_worktree: bool,
) -> Result<(usize, JsonValue), String> {
    let total_started = Instant::now();
    let cargo_projection_started = Instant::now();
    let managed_cargo_projection = validate_workspace_cargo_projection_for_snapshot(
        repo,
        target_snapshot_id,
        workspace_path,
        is_worktree,
    )?;
    let cargo_projection_elapsed = elapsed_ms(cargo_projection_started);

    let snapshot_rules_started = Instant::now();
    let snapshot_rules = snapshot_rules_state_from_snapshot_id(repo, Some(target_snapshot_id))?;
    let ignore_rules =
        workspace_snapshot_ignore_rules(workspace_path, snapshot_rules.text.as_deref())?;
    let snapshot_rules_elapsed = elapsed_ms(snapshot_rules_started);

    let snapshot_manifest_started = Instant::now();
    let snapshot_manifest = filtered_snapshot_manifest_index_for_workspace(
        repo,
        workspace_path,
        is_worktree,
        Some(target_snapshot_id),
        ignore_rules.as_deref(),
    )?;
    let snapshot_manifest_elapsed = elapsed_ms(snapshot_manifest_started);

    let workspace_walk_started = Instant::now();
    let entries = list_visible_workspace_entries(
        workspace_path.to_string_lossy().as_ref(),
        ignore_rules.as_deref(),
        None,
    )
    .map_err(|err| format!("{err:?}"))?;
    let workspace_root_text = workspace_path.to_string_lossy().to_string();
    let actual_paths = entries
        .files
        .into_iter()
        .filter(|path| {
            !(path_is_projected_out_for_workspace(&workspace_root_text, path, is_worktree)
                || managed_cargo_projection && path == WORKTREE_CARGO_CONFIG_RELATIVE_PATH)
        })
        .collect::<BTreeSet<_>>();
    let expected_paths = snapshot_manifest
        .rows
        .iter()
        .map(|row| snapshot_manifest.row_path(row).map(ToString::to_string))
        .collect::<Result<BTreeSet<_>, String>>()?;
    let missing_paths = expected_paths
        .difference(&actual_paths)
        .take(20)
        .cloned()
        .collect::<Vec<_>>();
    let untracked_paths = actual_paths
        .difference(&expected_paths)
        .take(20)
        .cloned()
        .collect::<Vec<_>>();
    if !missing_paths.is_empty() || !untracked_paths.is_empty() {
        return Err(format!(
            "Workspace file set does not match Snapshot {target_snapshot_id}: missing={missing_paths:?}, untracked={untracked_paths:?}."
        ));
    }
    let workspace_walk_elapsed = elapsed_ms(workspace_walk_started);

    let metadata_compare_started = Instant::now();
    let mut hash_targets = Vec::with_capacity(snapshot_manifest.rows.len());
    for row in &snapshot_manifest.rows {
        let relative_path = snapshot_manifest.row_path(row)?.to_string();
        let absolute_path = workspace_path.join(&relative_path);
        let fingerprint = workspace_file_fingerprint(&absolute_path)?;
        let expected_mode_bits = parse_mode_bits(Some(row.mode.as_str()))?;
        let expected_is_symlink = expected_mode_bits & 0o170000 == 0o120000;
        if (fingerprint.file_kind == "symlink") != expected_is_symlink {
            return Err(format!(
                "Workspace file kind does not match Snapshot for `{relative_path}`."
            ));
        }
        if !expected_is_symlink && fingerprint.file_kind != "file" {
            return Err(format!(
                "Workspace path is not a file for `{relative_path}`."
            ));
        }
        if row.size_bytes < 0 || fingerprint.size_bytes != row.size_bytes as u64 {
            return Err(format!(
                "Workspace size does not match Snapshot for `{relative_path}`."
            ));
        }
        let actual_mode = if expected_is_symlink {
            format!("{:#o}", 0o120000 | (fingerprint.mode_bits & 0o777))
        } else {
            format!("{:#o}", fingerprint.mode_bits & 0o777)
        };
        if actual_mode != row.mode {
            return Err(format!(
                "Workspace mode does not match Snapshot for `{relative_path}`: expected {}, got {actual_mode}.",
                row.mode
            ));
        }
        hash_targets.push((
            relative_path,
            absolute_path,
            expected_is_symlink,
            fingerprint,
            row.sha256.clone(),
        ));
    }
    let metadata_compare_elapsed = elapsed_ms(metadata_compare_started);

    let content_hash_started = Instant::now();
    if !hash_targets.is_empty() {
        let worker_count = MAIN_SEED_HASH_WORKERS.min(hash_targets.len());
        let chunk_size = hash_targets.len().div_ceil(worker_count);
        std::thread::scope(|scope| -> Result<(), String> {
            let mut workers = Vec::with_capacity(worker_count);
            for chunk in hash_targets.chunks(chunk_size) {
                workers.push(scope.spawn(move || -> Result<(), String> {
                    for (
                        relative_path,
                        absolute_path,
                        is_symlink,
                        fingerprint_before,
                        expected_sha256,
                    ) in chunk
                    {
                        let bytes = if *is_symlink {
                            let target = fs::read_link(absolute_path).map_err(|err| {
                                format!(
                                    "Failed to read workspace symlink {}: {err}",
                                    absolute_path.display()
                                )
                            })?;
                            #[cfg(unix)]
                            {
                                target.as_os_str().as_bytes().to_vec()
                            }
                            #[cfg(windows)]
                            {
                                target
                                    .to_str()
                                    .ok_or_else(|| {
                                        format!(
                                            "Windows symlink target is not valid Unicode: {}",
                                            target.display()
                                        )
                                    })?
                                    .as_bytes()
                                    .to_vec()
                            }
                        } else {
                            fs::read(absolute_path).map_err(|err| {
                                format!(
                                    "Failed to read workspace path {}: {err}",
                                    absolute_path.display()
                                )
                            })?
                        };
                        if sha256_hex_bytes(&bytes) != *expected_sha256 {
                            return Err(format!(
                                "Workspace content does not match Snapshot for `{relative_path}`."
                            ));
                        }
                        let fingerprint_after = workspace_file_fingerprint(absolute_path)?;
                        if &fingerprint_after != fingerprint_before {
                            return Err(format!(
                                "Workspace path `{relative_path}` changed during validation."
                            ));
                        }
                    }
                    Ok(())
                }));
            }
            for worker in workers {
                worker
                    .join()
                    .map_err(|_| "Workspace content validation worker panicked.".to_string())??;
            }
            Ok(())
        })?;
    }
    let content_hash_elapsed = elapsed_ms(content_hash_started);

    Ok((
        snapshot_manifest.rows.len(),
        json!({
            "cargo_projection": cargo_projection_elapsed,
            "snapshot_rules": snapshot_rules_elapsed,
            "snapshot_manifest": snapshot_manifest_elapsed,
            "workspace_walk": workspace_walk_elapsed,
            "metadata_compare": metadata_compare_elapsed,
            "content_hash": content_hash_elapsed,
            "content_row_count": snapshot_manifest.rows.len(),
            "total": elapsed_ms(total_started),
        }),
    ))
}

fn workspace_snapshot_ignore_rules(
    workspace_path: &Path,
    snapshot_rules_text: Option<&str>,
) -> Result<Option<String>, String> {
    let mut parts = Vec::new();
    let ignore_path = workspace_path.join(WORKSPACE_IGNORE_FILE);
    if ignore_path.is_file() {
        let text = fs::read_to_string(&ignore_path)
            .map_err(|err| format!("Failed to read {}: {err}", ignore_path.display()))?;
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(text) = snapshot_rules_text {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    parts.push("/docs/".to_string());
    Ok(Some(parts.join("\n") + "\n"))
}

fn materialize_main_seed_cargo_projection(
    repo: &RepoRuntime,
    target_snapshot_id: &str,
    staging_path: &Path,
) -> Result<(), String> {
    let repo_workspace_root = repo.workspace_root();
    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&repo_workspace_root)?;
    let Some(row) =
        store.snapshot_tree_path_row(target_snapshot_id, WORKTREE_CARGO_CONFIG_RELATIVE_PATH)?
    else {
        return Ok(());
    };
    let Some(blob_id) = file_map_row_blob_id(&row) else {
        return Ok(());
    };
    let source = read_snapshot_blob_text(repo, &blob_id)?;
    let Some(projected) = upgrade_generated_worktree_cargo_config_text(staging_path, &source)
    else {
        return Ok(());
    };
    let config_path = staging_path.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    if path_exists_or_directory_link(&config_path) {
        let metadata = fs::symlink_metadata(&config_path).map_err(|err| {
            format!(
                "Failed to inspect staged managed Cargo file {}: {err}",
                config_path.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "Staged managed Cargo path must be a physical file at {}.",
                config_path.display()
            ));
        }
        fs::remove_file(&config_path).map_err(|err| {
            format!(
                "Failed to replace staged managed Cargo file {}: {err}",
                config_path.display()
            )
        })?;
    }
    fs::write(&config_path, projected).map_err(|err| err.to_string())?;
    let mode = readonly_file_mode(parse_mode_bits(
        row.get("mode").and_then(JsonValue::as_str),
    )?);
    set_portable_mode(&config_path, mode).map_err(|err| err.to_string())
}

fn unique_main_seed_sibling_path(seed_path: &Path, role: &str) -> PathBuf {
    let seed_name = seed_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("main-seed");
    let nonce = system_event_timestamp().replace([':', '-', '.'], "");
    seed_path.parent().unwrap_or(seed_path).join(format!(
        ".{seed_name}.{role}-{}-{nonce}",
        std::process::id()
    ))
}

fn main_seed_backup_path(seed_path: &Path) -> PathBuf {
    let seed_name = seed_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("main-seed");
    seed_path
        .parent()
        .unwrap_or(seed_path)
        .join(format!(".{seed_name}.backup"))
}

fn prepare_main_seed_backup_slot(
    repo: &RepoRuntime,
    seed_path: &Path,
) -> Result<Option<&'static str>, String> {
    let backup_path = main_seed_backup_path(seed_path);
    if !path_exists_or_directory_link(&backup_path) {
        return Ok(None);
    }
    if !path_exists_or_directory_link(seed_path) {
        fs::rename(&backup_path, seed_path).map_err(|err| {
            format!(
                "Failed to restore retained CLI main-seed backup {} to {}: {err}",
                backup_path.display(),
                seed_path.display()
            )
        })?;
        let restored_state = main_seed_state(seed_path);
        let restored_is_valid = restored_state
            .seed_line_name
            .as_deref()
            .zip(restored_state.seed_snapshot_id.as_deref())
            .is_some_and(|(line_name, snapshot_id)| {
                is_seed_state_aligned(repo, &restored_state, seed_path, line_name, snapshot_id)
            });
        if restored_is_valid {
            return Ok(Some("restored_retained_backup"));
        }
        let rollback_error = fs::rename(seed_path, &backup_path).err();
        return Err(match rollback_error {
            Some(rollback_error) => format!(
                "Retained CLI main-seed backup failed validation after restore, and moving it back to {} failed: {rollback_error}",
                backup_path.display()
            ),
            None => format!(
                "Retained CLI main-seed backup failed validation after restore and remains at {}.",
                backup_path.display()
            ),
        });
    }

    let current_state = main_seed_state(seed_path);
    let current_is_valid = current_state
        .seed_line_name
        .as_deref()
        .zip(current_state.seed_snapshot_id.as_deref())
        .is_some_and(|(line_name, snapshot_id)| {
            is_seed_state_aligned(repo, &current_state, seed_path, line_name, snapshot_id)
        });
    if !current_is_valid {
        return Err(format!(
            "CLI main-seed swap recovery requires inspection: both current seed {} and retained backup {} exist, but the current seed does not validate.",
            seed_path.display(),
            backup_path.display()
        ));
    }
    remove_tree_force(&backup_path).map_err(|err| {
        format!(
            "Failed to remove redundant retained CLI main-seed backup {}: {err}",
            backup_path.display()
        )
    })?;
    Ok(Some("removed_redundant_retained_backup"))
}

fn local_line_generation_matches(
    repo: &RepoRuntime,
    line_name: &str,
    target_snapshot_id: &str,
) -> Result<bool, String> {
    Ok(local_line_head_snapshot_id(repo, line_name)?.as_deref() == Some(target_snapshot_id))
}

fn atomic_install_seed_directory_with<R, V>(
    staging_path: &Path,
    seed_path: &Path,
    backup_path: &Path,
    mut rename: R,
    validate: V,
) -> Result<bool, String>
where
    R: FnMut(&Path, &Path) -> std::io::Result<()>,
    V: FnOnce() -> Result<(), String>,
{
    let had_previous_seed = path_exists_or_directory_link(seed_path);
    if path_exists_or_directory_link(backup_path) {
        return Err(format!(
            "Main-seed backup path already exists: {}",
            backup_path.display()
        ));
    }
    if had_previous_seed {
        rename(seed_path, backup_path).map_err(|err| {
            format!(
                "Failed to preserve previous main seed {} at {}: {err}",
                seed_path.display(),
                backup_path.display()
            )
        })?;
    }
    if let Err(error) = rename(staging_path, seed_path) {
        let restore_error = if had_previous_seed {
            rename(backup_path, seed_path).err()
        } else {
            None
        };
        return Err(match restore_error {
            Some(restore_error) => format!(
                "Failed to install staged main seed {} at {}: {error}; restoring the previous seed also failed: {restore_error}",
                staging_path.display(),
                seed_path.display()
            ),
            None => format!(
                "Failed to install staged main seed {} at {}: {error}",
                staging_path.display(),
                seed_path.display()
            ),
        });
    }
    if let Err(validation_error) = validate() {
        let staged_restore_error = rename(seed_path, staging_path).err();
        let previous_restore_error = if had_previous_seed {
            rename(backup_path, seed_path).err()
        } else {
            None
        };
        let mut rollback_errors = Vec::new();
        if let Some(error) = staged_restore_error {
            rollback_errors.push(format!("new seed rollback failed: {error}"));
        }
        if let Some(error) = previous_restore_error {
            rollback_errors.push(format!("previous seed restore failed: {error}"));
        }
        return Err(if rollback_errors.is_empty() {
            format!("Installed main seed failed post-swap validation: {validation_error}")
        } else {
            format!(
                "Installed main seed failed post-swap validation: {validation_error}; {}",
                rollback_errors.join("; ")
            )
        });
    }
    Ok(had_previous_seed)
}

fn atomic_install_seed_directory<V>(
    staging_path: &Path,
    seed_path: &Path,
    backup_path: &Path,
    validate: V,
) -> Result<bool, String>
where
    V: FnOnce() -> Result<(), String>,
{
    atomic_install_seed_directory_with(
        staging_path,
        seed_path,
        backup_path,
        |source, target| fs::rename(source, target),
        validate,
    )
}

fn restore_promoted_worktree_source(
    staging_path: &Path,
    original_path: &Path,
    original_marker: &[u8],
    original_cargo_config: Option<&[u8]>,
) -> Result<(), String> {
    if !path_exists_or_directory_link(staging_path) {
        return Ok(());
    }
    set_tree_writeable(staging_path)?;
    fs::write(staging_path.join(WORKTREE_CONFIG_NAME), original_marker)
        .map_err(|err| err.to_string())?;
    let cargo_config_path = staging_path.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    match original_cargo_config {
        Some(contents) => {
            if let Some(parent) = cargo_config_path.parent() {
                fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
            fs::write(&cargo_config_path, contents).map_err(|err| err.to_string())?;
        }
        None if path_exists_or_directory_link(&cargo_config_path) => {
            remove_path_entry(&cargo_config_path)?;
        }
        None => {}
    }
    fs::rename(staging_path, original_path).map_err(|err| {
        format!(
            "Failed to restore promoted worktree source {} to {}: {err}",
            staging_path.display(),
            original_path.display()
        )
    })
}

struct PromotedSeedMarker<'a> {
    seed_path: &'a Path,
    seed_name: &'a str,
    line_name: &'a str,
    target_snapshot_id: &'a str,
    refreshed_at: &'a str,
    root_source: Option<&'a str>,
    snapshot_rules: &'a SnapshotRulesState,
    content_fingerprint: &'a str,
    content_row_count: usize,
}

fn write_promoted_seed_marker(
    staging_path: &Path,
    marker: &PromotedSeedMarker<'_>,
) -> Result<(), String> {
    let marker_path = staging_path.join(WORKTREE_CONFIG_NAME);
    if marker_path.is_file() {
        let metadata = fs::metadata(&marker_path).map_err(|err| err.to_string())?;
        let mode = portable_mode(&metadata, 0o644);
        set_portable_mode(&marker_path, mode | 0o200).map_err(|err| err.to_string())?;
    }
    let marker = json!({
        "worktree_name": marker.seed_name,
        "current_line": marker.line_name,
        "repo_root": staging_path
            .join(APP_DIR)
            .canonicalize()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| staging_path.to_path_buf())
            .to_string_lossy()
            .to_string(),
        "workspace_root": marker.seed_path.to_string_lossy().to_string(),
        "created_at": marker.refreshed_at,
        "internal_role": INTERNAL_WORKTREE_ROLE_MAIN_SEED,
        "seed_line_name": marker.line_name,
        "seed_snapshot_id": marker.target_snapshot_id,
        "seed_refreshed_at": marker.refreshed_at,
        "root_source": marker.root_source.and_then(|value| normalized_text(Some(value))),
        "layout_version": MAIN_SEED_LAYOUT_VERSION,
        "seed_ignore_rules_blob_id": marker.snapshot_rules.blob_id,
        "seed_content_fingerprint": marker.content_fingerprint,
        "seed_content_row_count": marker.content_row_count,
        "materialized_write_count": 0,
        "materialized_remove_count": 0,
        "materialized_unchanged_count": marker.content_row_count,
    });
    write_json_pretty(&marker_path, &marker)?;
    let metadata = fs::metadata(&marker_path).map_err(|err| err.to_string())?;
    let mode = readonly_file_mode(portable_mode(&metadata, 0o644));
    set_portable_mode(&marker_path, mode).map_err(|err| err.to_string())
}

fn promote_bound_worktree_to_main_seed(
    repo: &RepoRuntime,
    bound_worktree: &JsonValue,
    line_name: &str,
    target_snapshot_id: &str,
    seed_name: &str,
    seed_path: &Path,
    root_source: Option<&str>,
) -> Result<JsonValue, String> {
    let total_started = Instant::now();
    let worktree_name = required_string_field(bound_worktree, "name")?;
    let worktree_path = required_path_field(bound_worktree, "path")?;
    let worktree_metadata = fs::symlink_metadata(&worktree_path).map_err(|err| {
        format!(
            "Failed to inspect bound worktree {}: {err}",
            worktree_path.display()
        )
    })?;
    if worktree_metadata.file_type().is_symlink() || !worktree_metadata.is_dir() {
        return Err(format!(
            "Bound worktree must be a physical directory before promotion: {}",
            worktree_path.display()
        ));
    }
    if resolve_path_strict_false(&worktree_path) == resolve_path_strict_false(seed_path) {
        return Err("Bound worktree is already the CLI main-seed path.".to_string());
    }

    let original_marker = fs::read(worktree_path.join(WORKTREE_CONFIG_NAME)).map_err(|err| {
        format!(
            "Failed to preserve bound worktree marker {}: {err}",
            worktree_path.join(WORKTREE_CONFIG_NAME).display()
        )
    })?;
    let cargo_config_path = worktree_path.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    let original_cargo_config = fs::read(&cargo_config_path).ok();
    let validation_started = Instant::now();
    let (content_row_count, validation_timings) =
        validate_workspace_for_snapshot(repo, target_snapshot_id, &worktree_path, true)?;
    let validation_elapsed = elapsed_ms(validation_started);
    if !local_line_generation_matches(repo, line_name, target_snapshot_id)? {
        return Err(format!(
            "Local Line {line_name} advanced while validating {target_snapshot_id}; refusing stale main-seed promotion."
        ));
    }

    let cargo_cleanup_started = Instant::now();
    let cargo_build_cache_cleanup =
        cleanup_registered_worktree_cargo_build_dir(&worktree_path, &worktree_name)?;
    let cargo_cleanup_elapsed = elapsed_ms(cargo_cleanup_started);
    let parent = seed_path.parent().ok_or_else(|| {
        format!(
            "CLI main-seed path has no parent directory: {}",
            seed_path.display()
        )
    })?;
    fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    let staging_path = unique_main_seed_sibling_path(seed_path, "promote");
    let backup_path = main_seed_backup_path(seed_path);
    if path_exists_or_directory_link(&staging_path) {
        remove_tree_force(&staging_path)?;
    }

    let source_move_started = Instant::now();
    if let Err(error) = fs::rename(&worktree_path, &staging_path) {
        let class = if error.raw_os_error() == Some(18) {
            "cross_filesystem"
        } else {
            "rename_failed"
        };
        return Err(format!(
            "Bound worktree promotion source move failed ({class}) from {} to {}: {error}",
            worktree_path.display(),
            staging_path.display()
        ));
    }
    let source_move_elapsed = elapsed_ms(source_move_started);

    let refreshed_at = system_event_timestamp();
    let preparation_started = Instant::now();
    let preparation_result = (|| -> Result<(String, usize), String> {
        let snapshot_rules = snapshot_rules_state_from_snapshot_id(repo, Some(target_snapshot_id))?;
        write_promoted_seed_marker(
            &staging_path,
            &PromotedSeedMarker {
                seed_path,
                seed_name,
                line_name,
                target_snapshot_id,
                refreshed_at: &refreshed_at,
                root_source,
                snapshot_rules: &snapshot_rules,
                content_fingerprint: "",
                content_row_count,
            },
        )?;
        materialize_main_seed_cargo_projection(repo, target_snapshot_id, &staging_path)?;
        set_tree_readonly(&staging_path)?;
        let (content_fingerprint, fingerprint_row_count) =
            main_seed_content_fingerprint(&staging_path)?;
        if fingerprint_row_count != content_row_count {
            return Err(format!(
                "Promoted main-seed visible row count changed during preparation: expected {content_row_count}, got {fingerprint_row_count}."
            ));
        }
        write_promoted_seed_marker(
            &staging_path,
            &PromotedSeedMarker {
                seed_path,
                seed_name,
                line_name,
                target_snapshot_id,
                refreshed_at: &refreshed_at,
                root_source,
                snapshot_rules: &snapshot_rules,
                content_fingerprint: &content_fingerprint,
                content_row_count,
            },
        )?;
        Ok((content_fingerprint, fingerprint_row_count))
    })();
    let preparation_elapsed = elapsed_ms(preparation_started);
    let (_content_fingerprint, fingerprint_row_count) = match preparation_result {
        Ok(result) => result,
        Err(error) => {
            let rollback = restore_promoted_worktree_source(
                &staging_path,
                &worktree_path,
                &original_marker,
                original_cargo_config.as_deref(),
            );
            return Err(match rollback {
                Ok(()) => format!("Failed to prepare bound worktree main seed: {error}"),
                Err(rollback_error) => format!(
                    "Failed to prepare bound worktree main seed: {error}; source rollback failed: {rollback_error}"
                ),
            });
        }
    };

    if !local_line_generation_matches(repo, line_name, target_snapshot_id)? {
        restore_promoted_worktree_source(
            &staging_path,
            &worktree_path,
            &original_marker,
            original_cargo_config.as_deref(),
        )?;
        return Err(format!(
            "Local Line {line_name} advanced before the main-seed swap; refusing stale promotion of {target_snapshot_id}."
        ));
    }

    let swap_started = Instant::now();
    let backup_recovery = match prepare_main_seed_backup_slot(repo, seed_path) {
        Ok(recovery) => recovery,
        Err(error) => {
            let rollback = restore_promoted_worktree_source(
                &staging_path,
                &worktree_path,
                &original_marker,
                original_cargo_config.as_deref(),
            );
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => {
                    format!("{error}; bound worktree source rollback failed: {rollback_error}")
                }
            });
        }
    };
    let swap_result = atomic_install_seed_directory(&staging_path, seed_path, &backup_path, || {
        if !local_line_generation_matches(repo, line_name, target_snapshot_id)? {
            return Err(format!(
                "Local Line {line_name} advanced during the main-seed swap; refusing stale promotion of {target_snapshot_id}."
            ));
        }
        let installed_state = main_seed_state(seed_path);
        if is_seed_state_aligned(
            repo,
            &installed_state,
            seed_path,
            line_name,
            target_snapshot_id,
        ) {
            Ok(())
        } else {
            let integrity_detail = RepoRuntime::discover_from_path(seed_path)
                .and_then(|seed_repo| {
                    let rules =
                        snapshot_rules_state_from_snapshot_id(repo, Some(target_snapshot_id))?;
                    validate_main_seed_baseline(
                        &seed_repo,
                        target_snapshot_id,
                        rules.text.as_deref(),
                    )
                    .map(|integrity| {
                        format!(
                            "clean={}, changed_count={}, untracked_paths={:?}",
                            integrity.clean, integrity.changed_count, integrity.untracked_paths
                        )
                    })
                    .and_then(|integrity| {
                        validate_main_seed_cargo_projection(&seed_repo, target_snapshot_id)
                            .map(|()| format!("{integrity}; cargo_projection=valid"))
                            .or_else(|error| {
                                Ok(format!("{integrity}; cargo_projection_error={error}"))
                            })
                    })
                })
                .unwrap_or_else(|error| format!("integrity probe failed: {error}"));
            Err(format!(
                "installed CLI main seed does not validate against Snapshot {target_snapshot_id}; state={installed_state:?}; {integrity_detail}"
            ))
        }
    });
    let swap_elapsed = elapsed_ms(swap_started);
    let had_previous_seed = match swap_result {
        Ok(value) => value,
        Err(error) => {
            let rollback = restore_promoted_worktree_source(
                &staging_path,
                &worktree_path,
                &original_marker,
                original_cargo_config.as_deref(),
            );
            return Err(match rollback {
                Ok(()) => error,
                Err(rollback_error) => {
                    format!("{error}; bound worktree source rollback failed: {rollback_error}")
                }
            });
        }
    };

    let backup_cleanup_started = Instant::now();
    let backup_cleanup_error = if had_previous_seed && path_exists_or_directory_link(&backup_path) {
        remove_tree_force(&backup_path).err()
    } else {
        None
    };
    let backup_cleanup_elapsed = elapsed_ms(backup_cleanup_started);
    let registration_cleanup_started = Instant::now();
    let worktree_cleanup = finalize_promoted_worktree_registration(
        repo,
        &worktree_name,
        &worktree_path,
        seed_path,
        cargo_build_cache_cleanup,
    )
    .unwrap_or_else(|error| {
        json!({
            "status": "failed",
            "reason": "promoted_registration_cleanup_failed",
            "name": worktree_name,
            "path": worktree_path.to_string_lossy().to_string(),
            "promoted_path": seed_path.to_string_lossy().to_string(),
            "removed": false,
            "error": error,
        })
    });
    let registration_cleanup_elapsed = elapsed_ms(registration_cleanup_started);

    Ok(json!({
        "status": "promoted",
        "name": seed_name,
        "path": seed_path.to_string_lossy().to_string(),
        "line_name": line_name,
        "seed_snapshot_id": target_snapshot_id,
        "root_source": root_source.and_then(|value| normalized_text(Some(value))),
        "seed_refreshed_at": refreshed_at,
        "refresh_strategy": "validated_task_worktree_atomic_promotion",
        "materialized_write_count": 0,
        "materialized_remove_count": 0,
        "materialized_unchanged_count": fingerprint_row_count,
        "fallback_used": false,
        "backup_recovery": backup_recovery,
        "backup_cleanup_error": backup_cleanup_error,
        "worktree_cleanup": worktree_cleanup,
        "phase_timings_ms": {
            "snapshot_validation": validation_timings,
            "snapshot_validation_wall": validation_elapsed,
            "cargo_build_cache_cleanup": cargo_cleanup_elapsed,
            "source_move": source_move_elapsed,
            "preparation": preparation_elapsed,
            "atomic_swap_and_validation": swap_elapsed,
            "backup_cleanup": backup_cleanup_elapsed,
            "registry_cleanup": registration_cleanup_elapsed,
            "total": elapsed_ms(total_started),
        },
    }))
}

pub(in crate::primitives) fn sync_main_seed_after_task_land(
    repo: &RepoRuntime,
    task_id: Option<&str>,
    task_status: Option<&str>,
    target_line: &str,
    target_snapshot_id: &str,
) -> JsonValue {
    let total_started = Instant::now();
    let result = (|| -> Result<JsonValue, String> {
        let root_repo = workflow_root_repo(repo)?;
        let default_line = root_repo.default_line_name();
        if target_line != default_line {
            return Ok(main_seed_skipped_result(
                "target_not_default_line",
                &default_line,
                target_line,
                Some(target_snapshot_id),
                None,
                None,
            ));
        }
        let seed_name = main_seed_worktree_name(target_line);
        let Some(seed_location) =
            task_worktree_layout::resolve_main_seed_mirror_location(&root_repo, &seed_name)
        else {
            let budget_status = task_worktree_layout::main_seed_ram_budget_status(&root_repo);
            return Ok(
                if let Some(status) = budget_status.filter(|status| status.exceeded) {
                    main_seed_skipped_result(
                        "main_seed_ram_budget_exceeded",
                        &default_line,
                        target_line,
                        Some(target_snapshot_id),
                        Some(status.seed_snapshot_total_bytes),
                        Some(status.main_seed_ram_max_bytes),
                    )
                } else {
                    main_seed_skipped_result(
                        "managed_ephemeral_root_unavailable",
                        &default_line,
                        target_line,
                        Some(target_snapshot_id),
                        None,
                        None,
                    )
                },
            );
        };
        let seed_path = seed_location.target_path;
        let lock_started = Instant::now();
        let _lock =
            RepoFileLock::acquire_blocking(&main_seed_refresh_lock_path(&root_repo, target_line))
                .map_err(|err| format!("failed to acquire CLI main-seed refresh lock: {err}"))?;
        let lock_elapsed = elapsed_ms(lock_started);
        if !local_line_generation_matches(&root_repo, target_line, target_snapshot_id)? {
            return Ok(json!({
                "status": "skipped",
                "reason": "stale_local_line_generation",
                "line_name": target_line,
                "default_line": default_line,
                "seed_snapshot_id": target_snapshot_id,
                "current_snapshot_id": local_line_head_snapshot_id(&root_repo, target_line)?,
                "phase_timings_ms": {
                    "lock_wait": lock_elapsed,
                    "total": elapsed_ms(total_started),
                },
            }));
        }
        let seed_state = main_seed_state(&seed_path);
        if is_seed_state_aligned(
            &root_repo,
            &seed_state,
            &seed_path,
            target_line,
            target_snapshot_id,
        ) {
            let mut aligned = main_seed_aligned_result(
                target_line,
                target_snapshot_id,
                &seed_path,
                &seed_state,
                Some(seed_location.root_source.as_str()),
            )
            .to_json();
            aligned["phase_timings_ms"] = json!({
                "lock_wait": lock_elapsed,
                "total": elapsed_ms(total_started),
            });
            return Ok(aligned);
        }

        let mut promotion_error = None;
        let normalized_task_status = normalized_text(task_status);
        if normalized_task_status.as_deref() == Some("completed") {
            if let Some(task_id) = normalized_text(task_id) {
                match workflow_find_bound_task_worktree_metadata(&root_repo, &task_id)? {
                    Some(bound_worktree) => match promote_bound_worktree_to_main_seed(
                        &root_repo,
                        &bound_worktree,
                        target_line,
                        target_snapshot_id,
                        &seed_name,
                        &seed_path,
                        Some(seed_location.root_source.as_str()),
                    ) {
                        Ok(mut promoted) => {
                            if let Some(timings) = promoted
                                .get_mut("phase_timings_ms")
                                .and_then(JsonValue::as_object_mut)
                            {
                                timings.insert("lock_wait".to_string(), json!(lock_elapsed));
                                timings.insert(
                                    "land_seed_sync_total".to_string(),
                                    json!(elapsed_ms(total_started)),
                                );
                            }
                            return Ok(promoted);
                        }
                        Err(error) => promotion_error = Some(error),
                    },
                    None => {
                        promotion_error =
                            Some("No registered bound Task worktree was available.".to_string())
                    }
                }
            } else {
                promotion_error =
                    Some("Task finish output did not preserve a Task ID.".to_string());
            }
        }
        let fallback_reason = if normalized_task_status.as_deref() == Some("completed") {
            "task_worktree_promotion_unavailable"
        } else {
            "task_not_completed"
        };
        if !local_line_generation_matches(&root_repo, target_line, target_snapshot_id)? {
            return Ok(json!({
                "status": "skipped",
                "reason": "stale_local_line_generation",
                "line_name": target_line,
                "default_line": default_line,
                "seed_snapshot_id": target_snapshot_id,
                "current_snapshot_id": local_line_head_snapshot_id(&root_repo, target_line)?,
                "promotion_error": promotion_error,
                "phase_timings_ms": {
                    "lock_wait": lock_elapsed,
                    "total": elapsed_ms(total_started),
                },
            }));
        }
        let mut refreshed = refresh_main_seed_mirror(
            &root_repo,
            target_line,
            target_snapshot_id,
            &seed_name,
            &seed_path,
            Some(seed_location.root_source.as_str()),
        )
        .to_json();
        refreshed["fallback_used"] = JsonValue::Bool(true);
        refreshed["fallback_reason"] = JsonValue::String(fallback_reason.to_string());
        refreshed["promotion_error"] = promotion_error
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null);
        let refresh_timings = refreshed
            .get("phase_timings_ms")
            .cloned()
            .unwrap_or_else(|| json!({}));
        refreshed["phase_timings_ms"] = json!({
            "lock_wait": lock_elapsed,
            "fallback_refresh": refresh_timings,
            "land_seed_sync_total": elapsed_ms(total_started),
        });
        Ok(refreshed)
    })();
    result.unwrap_or_else(|error| {
        json!({
            "status": "failed",
            "reason": "post_land_cli_main_seed_sync_failed",
            "line_name": target_line,
            "seed_snapshot_id": target_snapshot_id,
            "error": error,
            "phase_timings_ms": {
                "total": elapsed_ms(total_started),
            },
        })
    })
}

pub(in crate::primitives) fn main_seed_skipped_result(
    reason: &str,
    default_line: &str,
    line_name: &str,
    seed_snapshot_id: Option<&str>,
    seed_snapshot_total_bytes: Option<i64>,
    main_seed_ram_max_bytes: Option<i64>,
) -> JsonValue {
    let mut payload = ait_core::json_support::JsonMap::new();
    payload.insert(
        "status".to_string(),
        JsonValue::String("skipped".to_string()),
    );
    payload.insert("reason".to_string(), JsonValue::String(reason.to_string()));
    payload.insert(
        "default_line".to_string(),
        JsonValue::String(default_line.to_string()),
    );
    payload.insert(
        "line_name".to_string(),
        JsonValue::String(line_name.to_string()),
    );
    if let Some(snapshot_id) = normalized_text(seed_snapshot_id) {
        payload.insert(
            "seed_snapshot_id".to_string(),
            JsonValue::String(snapshot_id),
        );
    }
    if let Some(total_bytes) = seed_snapshot_total_bytes {
        payload.insert(
            "seed_snapshot_total_bytes".to_string(),
            JsonValue::Number(total_bytes.into()),
        );
    }
    if let Some(max_bytes) = main_seed_ram_max_bytes {
        payload.insert(
            "main_seed_ram_max_bytes".to_string(),
            JsonValue::Number(max_bytes.into()),
        );
    }
    JsonValue::Object(payload)
}

pub(in crate::primitives) fn refresh_main_seed_mirror(
    repo: &RepoRuntime,
    line_name: &str,
    target_snapshot_id: &str,
    seed_name: &str,
    seed_path: &Path,
    root_source: Option<&str>,
) -> MainSeedMirrorResult {
    let total_started = Instant::now();
    let refreshed_at = system_event_timestamp();
    let staging_name = format!(
        ".{seed_name}.tmp-{}-{}",
        std::process::id(),
        refreshed_at.replace([':', '-'], "")
    );
    let staging_path = seed_path.parent().unwrap_or(seed_path).join(staging_name);
    let backup_path = main_seed_backup_path(seed_path);
    let baseline_seed_state = main_seed_state(seed_path);
    let baseline_seed_snapshot_id =
        main_seed_refresh_baseline_snapshot_id(&baseline_seed_state, line_name);
    let snapshot_rules_started = Instant::now();
    let target_snapshot_rules =
        snapshot_rules_state_from_snapshot_id(repo, Some(target_snapshot_id));
    let target_snapshot_rules_elapsed = elapsed_ms(snapshot_rules_started);
    let mut baseline_snapshot_id: Option<String> = None;
    let mut copy_strategy: Option<String> = None;
    let mut copy_error: Option<String> = None;
    let mut refresh_strategy = "full_snapshot_materialization".to_string();
    let mut materialization_plan: Option<MainSeedMaterializationPlan> = None;
    let mut phase_timings_ms = JsonMap::new();

    let refresh_result = (|| -> Result<MainSeedMirrorResult, String> {
        let target_snapshot_rules = target_snapshot_rules?;
        let baseline_snapshot_available = baseline_seed_snapshot_id
            .as_deref()
            .map(|snapshot_id| local_snapshot_exists(repo, snapshot_id))
            .transpose()?
            .unwrap_or(false);
        let usable_baseline_seed_snapshot_id = if baseline_snapshot_available {
            baseline_seed_snapshot_id.clone()
        } else {
            None
        };
        let visibility_can_reuse_existing_seed = usable_baseline_seed_snapshot_id.is_some()
            && baseline_seed_state.seed_ignore_rules_blob_id == target_snapshot_rules.blob_id;
        let staging_cleanup_started = Instant::now();
        if path_exists_or_directory_link(&staging_path) {
            remove_tree_force(&staging_path)?;
        }
        if let Some(parent) = staging_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        phase_timings_ms.insert(
            "staging_prepare".to_string(),
            json!(elapsed_ms(staging_cleanup_started)),
        );
        if visibility_can_reuse_existing_seed {
            if let Some(existing_seed_snapshot_id) = usable_baseline_seed_snapshot_id.clone() {
                let copy_started = Instant::now();
                match copy_seed_tree(seed_path, &staging_path, MAIN_SEED_COPY_EXCLUDE_NAMES) {
                    Ok(strategy) => {
                        copy_strategy = Some(strategy);
                        phase_timings_ms.insert(
                            "copy_seed_tree".to_string(),
                            json!(elapsed_ms(copy_started)),
                        );
                        let writable_started = Instant::now();
                        set_tree_directories_writeable(&staging_path)?;
                        phase_timings_ms.insert(
                            "set_tree_directories_writeable".to_string(),
                            json!(elapsed_ms(writable_started)),
                        );
                        baseline_snapshot_id = Some(existing_seed_snapshot_id);
                        refresh_strategy = "delta_from_existing_seed".to_string();
                    }
                    Err(err) => {
                        copy_error = Some(err);
                        refresh_strategy =
                            "full_snapshot_materialization_after_copy_fallback".to_string();
                        if path_exists_or_directory_link(&staging_path) {
                            let fallback_cleanup_started = Instant::now();
                            remove_tree_force(&staging_path)?;
                            phase_timings_ms.insert(
                                "copy_fallback_cleanup".to_string(),
                                json!(elapsed_ms(fallback_cleanup_started)),
                            );
                        }
                    }
                }
            }
        } else if baseline_seed_snapshot_id.is_some() {
            refresh_strategy = if baseline_snapshot_available {
                "full_snapshot_materialization_after_visibility_reset".to_string()
            } else {
                "full_snapshot_materialization_after_missing_baseline".to_string()
            };
        }
        let extra_config = json!({
            "worktree_name": seed_name,
            "current_line": line_name,
            "created_at": refreshed_at,
            "internal_role": INTERNAL_WORKTREE_ROLE_MAIN_SEED,
            "seed_line_name": line_name,
            "seed_snapshot_id": target_snapshot_id,
            "seed_refreshed_at": refreshed_at,
            "root_source": root_source.and_then(|value| normalized_text(Some(value))),
        });
        let runtime_layout_started = Instant::now();
        let seed_repo = materialize_worktree_runtime_layout(
            repo,
            seed_name,
            &staging_path,
            line_name,
            &refreshed_at,
            Some(&extra_config),
        )?;
        phase_timings_ms.insert(
            "materialize_worktree_runtime_layout".to_string(),
            json!(elapsed_ms(runtime_layout_started)),
        );
        let materialize_started = Instant::now();
        let plan = materialize_main_seed_snapshot(
            &seed_repo,
            target_snapshot_id,
            baseline_snapshot_id.as_deref(),
            &target_snapshot_rules,
            target_snapshot_rules_elapsed,
        )?;
        if plan.baseline_integrity_reset {
            refresh_strategy = "full_snapshot_materialization_after_integrity_reset".to_string();
        }
        materialization_plan = Some(plan);
        phase_timings_ms.insert(
            "materialize_main_seed_snapshot".to_string(),
            json!(elapsed_ms(materialize_started)),
        );
        let cargo_projection_started = Instant::now();
        materialize_main_seed_cargo_projection(repo, target_snapshot_id, &staging_path)?;
        phase_timings_ms.insert(
            "materialize_cargo_projection".to_string(),
            json!(elapsed_ms(cargo_projection_started)),
        );
        let mut seed_config = read_json_object_value(&staging_path.join(WORKTREE_CONFIG_NAME));
        seed_config.insert(
            "workspace_root".to_string(),
            JsonValue::String(seed_path.to_string_lossy().to_string()),
        );
        seed_config.insert(
            "seed_snapshot_id".to_string(),
            JsonValue::String(target_snapshot_id.to_string()),
        );
        seed_config.insert(
            "layout_version".to_string(),
            JsonValue::from(MAIN_SEED_LAYOUT_VERSION),
        );
        seed_config.insert(
            "seed_refreshed_at".to_string(),
            JsonValue::String(refreshed_at.clone()),
        );
        seed_config.insert(
            "seed_ignore_rules_blob_id".to_string(),
            target_snapshot_rules
                .blob_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        );
        seed_config.insert(
            "seed_content_fingerprint".to_string(),
            materialization_plan
                .as_ref()
                .map(|plan| JsonValue::String(plan.content_fingerprint.clone()))
                .unwrap_or(JsonValue::Null),
        );
        seed_config.insert(
            "seed_content_row_count".to_string(),
            JsonValue::from(
                materialization_plan
                    .as_ref()
                    .map(|plan| plan.content_row_count)
                    .unwrap_or_default() as u64,
            ),
        );
        seed_config.insert(
            "materialized_write_count".to_string(),
            JsonValue::from(
                materialization_plan
                    .as_ref()
                    .map(|plan| plan.write_count)
                    .unwrap_or_default() as u64,
            ),
        );
        seed_config.insert(
            "materialized_remove_count".to_string(),
            JsonValue::from(
                materialization_plan
                    .as_ref()
                    .map(|plan| plan.remove_count)
                    .unwrap_or_default() as u64,
            ),
        );
        seed_config.insert(
            "materialized_unchanged_count".to_string(),
            JsonValue::from(
                materialization_plan
                    .as_ref()
                    .map(|plan| plan.unchanged_count)
                    .unwrap_or_default() as u64,
            ),
        );
        let config_write_started = Instant::now();
        write_json_pretty(
            &staging_path.join(WORKTREE_CONFIG_NAME),
            &JsonValue::Object(seed_config),
        )?;
        phase_timings_ms.insert(
            "write_worktree_config".to_string(),
            json!(elapsed_ms(config_write_started)),
        );
        let readonly_started = Instant::now();
        set_tree_directories_readonly(&staging_path)?;
        phase_timings_ms.insert(
            "set_tree_directories_readonly".to_string(),
            json!(elapsed_ms(readonly_started)),
        );
        let swap_started = Instant::now();
        let backup_recovery_started = Instant::now();
        let _backup_recovery = prepare_main_seed_backup_slot(repo, seed_path)?;
        phase_timings_ms.insert(
            "backup_recovery".to_string(),
            json!(elapsed_ms(backup_recovery_started)),
        );
        let had_previous_seed = atomic_install_seed_directory(
            &staging_path,
            seed_path,
            &backup_path,
            || {
                if !local_line_generation_matches(repo, line_name, target_snapshot_id)? {
                    return Err(format!(
                        "Local Line {line_name} advanced during CLI main-seed refresh; refusing stale installation of {target_snapshot_id}."
                    ));
                }
                let installed_state = main_seed_state(seed_path);
                if is_seed_state_aligned(
                    repo,
                    &installed_state,
                    seed_path,
                    line_name,
                    target_snapshot_id,
                ) {
                    Ok(())
                } else {
                    Err(format!(
                        "refreshed CLI main seed does not validate against Snapshot {target_snapshot_id}"
                    ))
                }
            },
        )?;
        phase_timings_ms.insert(
            "atomic_swap_and_validation".to_string(),
            json!(elapsed_ms(swap_started)),
        );
        if had_previous_seed && path_exists_or_directory_link(&backup_path) {
            let backup_cleanup_started = Instant::now();
            if let Err(error) = remove_tree_force(&backup_path) {
                copy_error = Some(format!(
                    "CLI main-seed refresh succeeded, but previous-seed backup cleanup failed at {}: {error}",
                    backup_path.display()
                ));
            }
            phase_timings_ms.insert(
                "backup_cleanup".to_string(),
                json!(elapsed_ms(backup_cleanup_started)),
            );
        }
        phase_timings_ms.insert("total".to_string(), json!(elapsed_ms(total_started)));
        Ok(MainSeedMirrorResult {
            status: "refreshed".to_string(),
            name: seed_name.to_string(),
            path: seed_path.to_path_buf(),
            line_name: line_name.to_string(),
            seed_snapshot_id: target_snapshot_id.to_string(),
            root_source: root_source.and_then(|value| normalized_text(Some(value))),
            seed_refreshed_at: Some(refreshed_at),
            baseline_seed_snapshot_id: baseline_seed_snapshot_id.clone(),
            refresh_strategy: refresh_strategy.clone(),
            copy_strategy: copy_strategy.clone(),
            copy_error: copy_error.clone(),
            materialized_write_count: materialization_plan.as_ref().map(|plan| plan.write_count),
            materialized_remove_count: materialization_plan.as_ref().map(|plan| plan.remove_count),
            materialized_unchanged_count: materialization_plan
                .as_ref()
                .map(|plan| plan.unchanged_count),
            phase_timings_ms: Some(json!({
                "refresh": JsonValue::Object(phase_timings_ms.clone()),
                "materialization": materialization_plan
                    .as_ref()
                    .map(|plan| plan.phase_timings_ms.clone())
                    .unwrap_or_else(|| json!({})),
            })),
            error: None,
        })
    })();

    match refresh_result {
        Ok(result) => result,
        Err(err) => {
            if path_exists_or_directory_link(&staging_path) {
                let _ = remove_tree_force(&staging_path);
            }
            MainSeedMirrorResult {
                status: "failed".to_string(),
                name: seed_name.to_string(),
                path: seed_path.to_path_buf(),
                line_name: line_name.to_string(),
                seed_snapshot_id: target_snapshot_id.to_string(),
                root_source: root_source.and_then(|value| normalized_text(Some(value))),
                seed_refreshed_at: None,
                baseline_seed_snapshot_id,
                refresh_strategy,
                copy_strategy,
                copy_error,
                materialized_write_count: materialization_plan
                    .as_ref()
                    .map(|plan| plan.write_count),
                materialized_remove_count: materialization_plan
                    .as_ref()
                    .map(|plan| plan.remove_count),
                materialized_unchanged_count: materialization_plan
                    .as_ref()
                    .map(|plan| plan.unchanged_count),
                phase_timings_ms: Some(json!({
                    "refresh": JsonValue::Object(phase_timings_ms.clone()),
                    "materialization": materialization_plan
                        .as_ref()
                        .map(|plan| plan.phase_timings_ms.clone())
                        .unwrap_or_else(|| json!({})),
                })),
                error: Some(err),
            }
        }
    }
}

pub(in crate::primitives) fn ensure_main_seed_mirror_for_snapshot(
    repo: &RepoRuntime,
    line_name: &str,
    target_snapshot_id: &str,
    root_source: Option<&str>,
) -> Option<MainSeedMirrorResult> {
    let default_line = repo.default_line_name();
    if line_name != default_line {
        return None;
    }
    let seed_name = main_seed_worktree_name(line_name);
    let seed_location = task_worktree_layout::resolve_main_seed_mirror_location(repo, &seed_name)?;
    let seed_path = seed_location.target_path;
    let seed_state = main_seed_state(&seed_path);
    if is_seed_state_aligned(repo, &seed_state, &seed_path, line_name, target_snapshot_id) {
        return Some(main_seed_aligned_result(
            line_name,
            target_snapshot_id,
            &seed_path,
            &seed_state,
            root_source,
        ));
    }
    let _lock =
        RepoFileLock::acquire_blocking(&main_seed_refresh_lock_path(repo, line_name)).ok()?;
    let locked_seed_state = main_seed_state(&seed_path);
    if is_seed_state_aligned(
        repo,
        &locked_seed_state,
        &seed_path,
        line_name,
        target_snapshot_id,
    ) {
        return Some(main_seed_aligned_result(
            line_name,
            target_snapshot_id,
            &seed_path,
            &locked_seed_state,
            root_source,
        ));
    }
    Some(refresh_main_seed_mirror(
        repo,
        line_name,
        target_snapshot_id,
        &seed_name,
        &seed_path,
        root_source,
    ))
}

pub fn task_resolve_main_seed_mirror_location(
    repo: &RepoRuntime,
    seed_name: &str,
) -> Result<JsonValue, String> {
    Ok(
        task_worktree_layout::resolve_main_seed_mirror_location(repo, seed_name)
            .map(|location| location.to_json())
            .unwrap_or(JsonValue::Null),
    )
}

pub fn task_ensure_main_seed_mirror(
    repo: &RepoRuntime,
    force_refresh: bool,
    line_name: Option<&str>,
) -> Result<JsonValue, String> {
    let default_line = repo.default_line_name();
    let effective_line_name = normalized_text(line_name).unwrap_or_else(|| default_line.clone());
    if effective_line_name != default_line {
        return Ok(main_seed_skipped_result(
            "target_not_default_line",
            &default_line,
            &effective_line_name,
            None,
            None,
            None,
        ));
    }
    let line_row = match local_line_row(repo, &effective_line_name) {
        Ok(row) => row,
        Err(_) => {
            return Ok(main_seed_skipped_result(
                "line_missing",
                &default_line,
                &effective_line_name,
                None,
                None,
                None,
            ));
        }
    };
    let target_snapshot_id = match string_field(&line_row, "head_snapshot_id") {
        Some(snapshot_id) => snapshot_id,
        None => {
            return Ok(main_seed_skipped_result(
                "line_head_missing",
                &default_line,
                &effective_line_name,
                None,
                None,
                None,
            ));
        }
    };
    let budget_status = task_worktree_layout::main_seed_ram_budget_status(repo);
    let seed_name = main_seed_worktree_name(&effective_line_name);
    let Some(seed_location) =
        task_worktree_layout::resolve_main_seed_mirror_location(repo, &seed_name)
    else {
        if let Some(status) = budget_status.as_ref().filter(|status| status.exceeded) {
            return Ok(main_seed_skipped_result(
                "main_seed_ram_budget_exceeded",
                &default_line,
                &effective_line_name,
                Some(target_snapshot_id.as_str()),
                Some(status.seed_snapshot_total_bytes),
                Some(status.main_seed_ram_max_bytes),
            ));
        }
        return Ok(main_seed_skipped_result(
            "managed_ephemeral_root_unavailable",
            &default_line,
            &effective_line_name,
            Some(target_snapshot_id.as_str()),
            None,
            None,
        ));
    };
    let seed_path = seed_location.target_path.clone();
    let seed_state = main_seed_state(&seed_path);
    if !force_refresh
        && is_seed_state_aligned(
            repo,
            &seed_state,
            &seed_path,
            &effective_line_name,
            &target_snapshot_id,
        )
    {
        return Ok(main_seed_aligned_result(
            &effective_line_name,
            &target_snapshot_id,
            &seed_path,
            &seed_state,
            Some(seed_location.root_source.as_str()),
        )
        .to_json());
    }
    let _lock =
        RepoFileLock::acquire_blocking(&main_seed_refresh_lock_path(repo, &effective_line_name))
            .map_err(|err| format!("failed to acquire main-seed refresh lock: {err}"))?;
    let locked_seed_state = main_seed_state(&seed_path);
    if !force_refresh
        && is_seed_state_aligned(
            repo,
            &locked_seed_state,
            &seed_path,
            &effective_line_name,
            &target_snapshot_id,
        )
    {
        return Ok(main_seed_aligned_result(
            &effective_line_name,
            &target_snapshot_id,
            &seed_path,
            &locked_seed_state,
            Some(seed_location.root_source.as_str()),
        )
        .to_json());
    }
    Ok(refresh_main_seed_mirror(
        repo,
        &effective_line_name,
        &target_snapshot_id,
        &seed_name,
        &seed_path,
        Some(seed_location.root_source.as_str()),
    )
    .to_json())
}

#[cfg(test)]
mod selected_binary_main_seed_tests {
    use super::*;
    use ait_core::line_store::LineStore;
    use ait_core::local_snapshot::LocalSnapshotWriteStore;
    use std::path::Path;
    use tempfile::TempDir;

    struct WritableTempDir(TempDir);

    impl WritableTempDir {
        fn new() -> Self {
            Self(TempDir::new().expect("promotion tempdir"))
        }

        fn new_in(parent: &Path) -> Self {
            Self(TempDir::new_in(parent).expect("promotion tempdir in requested parent"))
        }

        fn path(&self) -> &Path {
            self.0.path()
        }
    }

    impl Drop for WritableTempDir {
        fn drop(&mut self) {
            let _ = set_tree_writeable(self.0.path());
        }
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, content).expect("write file");
    }

    fn binary_snapshot_repo() -> (WritableTempDir, RepoRuntime) {
        let temp = WritableTempDir::new();
        let root = temp.path();
        fs::create_dir_all(root.join(".ait")).expect("create .ait");
        write_file(
            &root.join(".ait/config.json"),
            r#"{"repo_name":"fixture-ait","default_line":"main","snapshot_binary_db_storage":"binary"}"#,
        );
        let repo = RepoRuntime::discover_from_path(root).expect("discover runtime");
        repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .lines()
            .create_line("main", None, "2026-07-08T00:00:00Z")
            .expect("create Binary DB line");
        (temp, repo)
    }

    fn binary_promotion_fixture_with_files(
        task_id: &str,
        generated_file_count: usize,
        temp_parent: Option<&Path>,
    ) -> (WritableTempDir, RepoRuntime, String, PathBuf, PathBuf) {
        let temp = match temp_parent {
            Some(parent) => WritableTempDir::new_in(parent),
            None => WritableTempDir::new(),
        };
        let repo_root = temp.path().join("repo");
        let ephemeral_root = temp.path().join("client-runtime");
        fs::create_dir_all(repo_root.join(".ait/worktrees")).expect("create authority");
        fs::create_dir_all(&ephemeral_root).expect("create client runtime");
        write_file(
            &repo_root.join(".ait/config.json"),
            &json!({
                "repo_name": "fixture-ait",
                "default_line": "main",
                "snapshot_binary_db_storage": "binary",
                "task_worktree": {
                    "ephemeral_root": ephemeral_root.to_string_lossy().to_string(),
                },
            })
            .to_string(),
        );
        let repo = RepoRuntime::discover_from_path(&repo_root).expect("discover runtime");
        let lines = repo
            .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .lines();
        lines
            .create_line("main", None, "2026-07-28T00:00:00Z")
            .expect("create main line");
        write_file(&repo_root.join("plain.txt"), "landed content\n");
        write_file(
            &repo_root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH),
            "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n",
        );
        for index in 0..generated_file_count {
            write_file(
                &repo_root
                    .join("generated")
                    .join(format!("{:02}", index % 32))
                    .join(format!("file-{index:04}.txt")),
                &format!("generated main-seed benchmark row {index:04}\n"),
            );
        }
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&repo_root)
            .expect("selected snapshot store");
        let snapshot = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("landed revision"), false)
            .expect("create landed snapshot");
        let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
        lines
            .set_line_head("main", Some(&snapshot_id), "2026-07-28T00:00:01Z")
            .expect("set main head");
        let feature_line = format!("feature/{}", task_id.to_ascii_lowercase());
        lines
            .create_line(&feature_line, Some(&snapshot_id), "2026-07-28T00:00:02Z")
            .expect("create feature line");

        let worktree_name = task_id.to_ascii_lowercase();
        let worktree_path = ephemeral_root.join(&worktree_name);
        let worktree_repo = materialize_worktree_runtime_layout(
            &repo,
            &worktree_name,
            &worktree_path,
            &feature_line,
            "2026-07-28T00:00:03Z",
            None,
        )
        .expect("materialize task worktree");
        task_start_snapshot_restore(&worktree_repo, Some(&snapshot_id), None)
            .expect("restore task Snapshot");
        write_json_pretty(
            &worktree_registry_path(&repo, &worktree_name),
            &json!({
                "name": worktree_name,
                "path": worktree_path.to_string_lossy().to_string(),
                "repo_root": repo_root.to_string_lossy().to_string(),
                "line_name": feature_line,
                "bound_task_id": task_id,
                "auto_created_for_task": true,
                "creation_kind": "task_auto_created",
                "cleanup_policy": "after_remote_land",
                "created_at": "2026-07-28T00:00:03Z",
            }),
        )
        .expect("register task worktree");
        let seed_path = task_worktree_layout::resolve_main_seed_mirror_location(&repo, "main-seed")
            .expect("resolve CLI main seed")
            .target_path;
        (temp, repo, snapshot_id, worktree_path, seed_path)
    }

    fn binary_promotion_fixture(
        task_id: &str,
    ) -> (WritableTempDir, RepoRuntime, String, PathBuf, PathBuf) {
        binary_promotion_fixture_with_files(task_id, 0, None)
    }

    #[test]
    fn exact_workspace_snapshot_validation_rejects_dirty_content() {
        let (_temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        write_file(&root.join("plain.txt"), "plain seed\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let snapshot = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("workspace validation"), false)
            .expect("create Binary DB snapshot");
        let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");

        let (row_count, timings) =
            validate_workspace_for_snapshot(&repo, &snapshot_id, &root, false)
                .expect("exact workspace should validate");
        assert_eq!(row_count, 1);
        assert!(timings["total"].as_f64().is_some());

        write_file(&root.join("plain.txt"), "dirty seed\n");
        let error = validate_workspace_for_snapshot(&repo, &snapshot_id, &root, false)
            .expect_err("dirty workspace must not promote");
        assert!(error.contains("size") || error.contains("content"));
    }

    #[test]
    fn exact_workspace_snapshot_validation_verifies_hidden_cargo_projection_content() {
        let (_temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        let source = "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n";
        write_file(&root.join("plain.txt"), "plain seed\n");
        write_file(&root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH), source);
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let snapshot = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("workspace projection"), false)
            .expect("create Binary DB snapshot");
        let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");

        write_file(
            &root.join(WORKTREE_CONFIG_NAME),
            &json!({"worktree_name": "task-one"}).to_string(),
        );
        let projected =
            upgrade_generated_worktree_cargo_config_text(&root, source).expect("project source");
        write_file(&root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH), &projected);
        let (row_count, _) = validate_workspace_for_snapshot(&repo, &snapshot_id, &root, true)
            .expect("exact managed projection should validate");
        assert_eq!(row_count, 1);

        let tampered = projected.replace(
            "managed-test = [\"test\", \"--profile\", \"ait-ci\"]",
            "managed-test = [\"test\", \"--ignored-change\"]",
        );
        write_file(&root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH), &tampered);
        let error = validate_workspace_for_snapshot(&repo, &snapshot_id, &root, true)
            .expect_err("hidden managed projection must still match Snapshot source");
        assert!(error.contains("Managed Cargo file does not match Snapshot"));
    }

    #[test]
    fn atomic_seed_swap_rolls_back_previous_seed_after_validation_failure() {
        let temp = TempDir::new().expect("swap tempdir");
        let seed_path = temp.path().join("main-seed");
        let staging_path = temp.path().join("staging");
        let backup_path = temp.path().join("backup");
        fs::create_dir_all(&seed_path).expect("create old seed");
        fs::create_dir_all(&staging_path).expect("create staged seed");
        write_file(&seed_path.join("value.txt"), "old\n");
        write_file(&staging_path.join("value.txt"), "new\n");

        let error = atomic_install_seed_directory(&staging_path, &seed_path, &backup_path, || {
            Err("injected post-swap failure".to_string())
        })
        .expect_err("validation failure must roll back");

        assert!(error.contains("post-swap validation"));
        assert_eq!(
            fs::read_to_string(seed_path.join("value.txt")).expect("restored old seed"),
            "old\n"
        );
        assert_eq!(
            fs::read_to_string(staging_path.join("value.txt")).expect("restored staged seed"),
            "new\n"
        );
        assert!(!backup_path.exists());
    }

    #[test]
    fn atomic_seed_swap_restores_previous_seed_when_install_rename_fails() {
        let temp = TempDir::new().expect("swap tempdir");
        let seed_path = temp.path().join("main-seed");
        let staging_path = temp.path().join("staging");
        let backup_path = temp.path().join("backup");
        fs::create_dir_all(&seed_path).expect("create old seed");
        fs::create_dir_all(&staging_path).expect("create staged seed");
        write_file(&seed_path.join("value.txt"), "old\n");
        let mut rename_count = 0_usize;

        let error = atomic_install_seed_directory_with(
            &staging_path,
            &seed_path,
            &backup_path,
            |source, target| {
                rename_count += 1;
                if rename_count == 2 {
                    return Err(std::io::Error::from_raw_os_error(18));
                }
                fs::rename(source, target)
            },
            || Ok(()),
        )
        .expect_err("injected cross-filesystem install must fall back");

        assert!(error.contains("Failed to install staged main seed"));
        assert!(seed_path.join("value.txt").is_file());
        assert!(staging_path.is_dir());
        assert!(!backup_path.exists());
    }

    #[test]
    fn retained_seed_backup_is_bounded_and_recovers_missing_seed() {
        let (_temp, repo, snapshot_id, _worktree_path, seed_path) =
            binary_promotion_fixture("RCT-BACKUP");
        let promoted = sync_main_seed_after_task_land(
            &repo,
            Some("RCT-BACKUP"),
            Some("completed"),
            "main",
            &snapshot_id,
        );
        assert_eq!(promoted["status"], json!("promoted"));
        let backup_path = main_seed_backup_path(&seed_path);

        fs::rename(&seed_path, &backup_path).expect("simulate interrupted swap");
        let recovery =
            prepare_main_seed_backup_slot(&repo, &seed_path).expect("restore retained backup");
        assert_eq!(recovery, Some("restored_retained_backup"));
        assert!(seed_path.is_dir());
        assert!(!backup_path.exists());

        fs::create_dir_all(&backup_path).expect("simulate redundant retained backup");
        write_file(&backup_path.join("old.txt"), "old seed\n");
        let recovery =
            prepare_main_seed_backup_slot(&repo, &seed_path).expect("remove redundant backup");
        assert_eq!(recovery, Some("removed_redundant_retained_backup"));
        assert!(!backup_path.exists());
        assert!(
            is_seed_state_aligned(
                &repo,
                &main_seed_state(&seed_path),
                &seed_path,
                "main",
                &snapshot_id
            ),
            "recovered seed must remain valid"
        );
    }

    #[test]
    fn completed_task_promotes_exact_bound_worktree_and_is_idempotent() {
        let (_temp, repo, snapshot_id, worktree_path, seed_path) =
            binary_promotion_fixture("RCT-PROMOTE");

        let promoted = sync_main_seed_after_task_land(
            &repo,
            Some("RCT-PROMOTE"),
            Some("completed"),
            "main",
            &snapshot_id,
        );

        assert_eq!(
            promoted["status"],
            json!("promoted"),
            "promotion payload: {promoted:#?}"
        );
        assert_eq!(
            promoted["refresh_strategy"],
            json!("validated_task_worktree_atomic_promotion")
        );
        assert_eq!(promoted["fallback_used"], json!(false));
        assert!(!worktree_path.exists());
        assert!(seed_path.is_dir());
        assert_eq!(
            promoted["worktree_cleanup"]["status"],
            json!("promoted_and_consumed")
        );
        assert!(!worktree_registry_path(&repo, "rct-promote").exists());
        assert!(
            is_seed_state_aligned(
                &repo,
                &main_seed_state(&seed_path),
                &seed_path,
                "main",
                &snapshot_id
            ),
            "promotion payload: {promoted:#?}; seed path: {}",
            seed_path.display()
        );

        let repeated = sync_main_seed_after_task_land(
            &repo,
            Some("RCT-PROMOTE"),
            Some("completed"),
            "main",
            &snapshot_id,
        );
        assert_eq!(repeated["status"], json!("aligned"));
        assert_eq!(repeated["refresh_strategy"], json!("already_aligned"));
    }

    #[test]
    #[ignore = "manual same-filesystem approximately-1,000-file p95 benchmark"]
    fn completed_task_promotion_stays_below_two_second_p95_for_one_thousand_files() {
        let mut samples_ms = Vec::with_capacity(20);
        for iteration in 0..20 {
            let task_id = format!("RCT-BENCH-{iteration:02}");
            let (_temp, repo, snapshot_id, worktree_path, seed_path) =
                binary_promotion_fixture_with_files(&task_id, 998, None);
            let promoted = sync_main_seed_after_task_land(
                &repo,
                Some(&task_id),
                Some("completed"),
                "main",
                &snapshot_id,
            );
            assert_eq!(
                promoted["status"],
                json!("promoted"),
                "promotion payload: {promoted:#?}"
            );
            assert!(!worktree_path.exists());
            assert!(seed_path.is_dir());
            samples_ms.push(
                promoted["phase_timings_ms"]["land_seed_sync_total"]
                    .as_f64()
                    .expect("land seed sync timing"),
            );
        }
        samples_ms.sort_by(f64::total_cmp);
        let p95_ms = samples_ms[18];
        eprintln!(
            "ait_main_seed_promotion_1000_files samples_ms={samples_ms:?} p95_ms={p95_ms:.3}"
        );
        assert!(
            p95_ms < 2_000.0,
            "approximately-1,000-file promotion p95 was {p95_ms:.3} ms"
        );
    }

    #[test]
    #[ignore = "manual approximately-1,000-file Snapshot refresh fallback benchmark"]
    fn dirty_task_worktree_reports_one_thousand_file_fallback_refresh_timing() {
        let (_temp, repo, snapshot_id, worktree_path, seed_path) =
            binary_promotion_fixture_with_files("RCT-BENCH-FALLBACK", 998, None);
        write_file(&worktree_path.join("plain.txt"), "dirty after ready\n");

        let refreshed = sync_main_seed_after_task_land(
            &repo,
            Some("RCT-BENCH-FALLBACK"),
            Some("completed"),
            "main",
            &snapshot_id,
        );

        assert_eq!(
            refreshed["status"],
            json!("refreshed"),
            "fallback payload: {refreshed:#?}"
        );
        assert_eq!(refreshed["fallback_used"], json!(true));
        assert!(worktree_path.is_dir());
        assert!(seed_path.is_dir());
        eprintln!(
            "ait_main_seed_fallback_refresh_1000_files timings={}",
            refreshed["phase_timings_ms"]
        );
    }

    #[test]
    fn dirty_completed_worktree_falls_back_to_local_snapshot_refresh() {
        let (_temp, repo, snapshot_id, worktree_path, seed_path) =
            binary_promotion_fixture("RCT-DIRTY");
        write_file(&worktree_path.join("plain.txt"), "dirty content\n");

        let refreshed = sync_main_seed_after_task_land(
            &repo,
            Some("RCT-DIRTY"),
            Some("completed"),
            "main",
            &snapshot_id,
        );

        assert_eq!(refreshed["status"], json!("refreshed"));
        assert_eq!(refreshed["fallback_used"], json!(true));
        assert_eq!(
            refreshed["fallback_reason"],
            json!("task_worktree_promotion_unavailable")
        );
        assert!(refreshed["promotion_error"]
            .as_str()
            .is_some_and(|error| error.contains("Workspace")));
        assert!(worktree_path.is_dir());
        assert!(
            is_seed_state_aligned(
                &repo,
                &main_seed_state(&seed_path),
                &seed_path,
                "main",
                &snapshot_id
            ),
            "fallback payload: {refreshed:#?}; seed path: {}",
            seed_path.display()
        );
    }

    #[test]
    fn delta_refresh_replaces_copied_readonly_cargo_projection() {
        let (temp, repo) = binary_snapshot_repo();
        let temp_path = temp.path().to_path_buf();
        let root = repo.workspace_root();
        let cargo_source = "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n";
        write_file(&root.join("plain.txt"), "first seed\n");
        write_file(
            &root.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH),
            cargo_source,
        );
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let first = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("first seed"), false)
            .expect("create first seed snapshot");
        let first_snapshot_id =
            required_string_field(&first, "snapshot_id").expect("first snapshot id");
        let seed_path = temp.path().join("main-seed");

        let first_refresh = refresh_main_seed_mirror(
            &repo,
            "main",
            &first_snapshot_id,
            "main-seed",
            &seed_path,
            Some("test"),
        );
        assert_eq!(
            first_refresh.status, "refreshed",
            "first seed refresh failed: {:?}",
            first_refresh.error
        );
        let first_projection_mode = portable_mode(
            &fs::metadata(seed_path.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH))
                .expect("first projected Cargo metadata"),
            0o644,
        );
        assert_eq!(first_projection_mode & 0o222, 0);

        write_file(&root.join("plain.txt"), "second seed\n");
        let second = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("second seed"), false)
            .expect("create second seed snapshot");
        let second_snapshot_id =
            required_string_field(&second, "snapshot_id").expect("second snapshot id");
        let second_refresh = refresh_main_seed_mirror(
            &repo,
            "main",
            &second_snapshot_id,
            "main-seed",
            &seed_path,
            Some("test"),
        );

        assert_eq!(
            second_refresh.status, "refreshed",
            "delta seed refresh failed: {:?}",
            second_refresh.error
        );
        assert_eq!(second_refresh.refresh_strategy, "delta_from_existing_seed");
        let installed_projection = seed_path.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
        let installed_projection_mode = portable_mode(
            &fs::metadata(&installed_projection).expect("installed projected Cargo metadata"),
            0o644,
        );
        assert_eq!(installed_projection_mode & 0o222, 0);
        assert!(
            is_seed_state_aligned(
                &repo,
                &main_seed_state(&seed_path),
                &seed_path,
                "main",
                &second_snapshot_id
            ),
            "delta refresh must install an aligned read-only seed"
        );
        drop(snapshot_store);
        drop(repo);
        drop(temp);
        assert!(
            !temp_path.exists(),
            "read-only main-seed fixture leaked after test cleanup: {}",
            temp_path.display()
        );
    }

    #[test]
    fn active_task_uses_snapshot_refresh_without_consuming_worktree() {
        let (_temp, repo, snapshot_id, worktree_path, seed_path) =
            binary_promotion_fixture("RCT-ACTIVE");

        let refreshed = sync_main_seed_after_task_land(
            &repo,
            Some("RCT-ACTIVE"),
            Some("active"),
            "main",
            &snapshot_id,
        );

        assert_eq!(refreshed["status"], json!("refreshed"));
        assert_eq!(refreshed["fallback_used"], json!(true));
        assert_eq!(refreshed["fallback_reason"], json!("task_not_completed"));
        assert!(worktree_path.is_dir());
        assert!(worktree_registry_path(&repo, "rct-active").is_file());
        assert!(
            seed_path.is_dir(),
            "fallback payload: {refreshed:#?}; seed path: {}",
            seed_path.display()
        );
    }

    #[test]
    fn stale_land_generation_never_replaces_newer_cli_seed() {
        let (_temp, repo, stale_snapshot_id, worktree_path, seed_path) =
            binary_promotion_fixture("RCT-STALE");
        write_file(&repo.workspace_root().join("plain.txt"), "newer content\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                &repo.workspace_root(),
            )
            .expect("selected snapshot store");
        let newer = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("newer revision"), false)
            .expect("create newer snapshot");
        let newer_snapshot_id =
            required_string_field(&newer, "snapshot_id").expect("newer snapshot id");
        repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .lines()
            .set_line_head("main", Some(&newer_snapshot_id), "2026-07-28T00:00:04Z")
            .expect("advance main");

        let skipped = sync_main_seed_after_task_land(
            &repo,
            Some("RCT-STALE"),
            Some("completed"),
            "main",
            &stale_snapshot_id,
        );

        assert_eq!(skipped["status"], json!("skipped"));
        assert_eq!(skipped["reason"], json!("stale_local_line_generation"));
        assert_eq!(skipped["current_snapshot_id"], json!(newer_snapshot_id));
        assert!(worktree_path.is_dir());
        assert!(!seed_path.exists());
    }

    #[test]
    fn selected_binary_main_seed_refresh_persists_promoted_workspace_root() {
        let (temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        write_file(&root.join("hello.txt"), "promoted seed\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let snapshot = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("promoted seed"), false)
            .expect("create Binary DB snapshot");
        let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
        let seed_path = temp.path().join("main-seed");

        let result = refresh_main_seed_mirror(
            &repo,
            "main",
            &snapshot_id,
            "main-seed",
            &seed_path,
            Some("test"),
        );

        assert_eq!(
            result.status, "refreshed",
            "seed refresh failed: {:?}",
            result.error
        );
        let seed_config = read_json_object_value(&seed_path.join(WORKTREE_CONFIG_NAME));
        let workspace_root = seed_config
            .get("workspace_root")
            .and_then(JsonValue::as_str)
            .expect("promoted seed workspace root");
        assert_eq!(Path::new(workspace_root), seed_path.as_path());
        assert!(!workspace_root.contains(".main-seed.tmp-"));

        let promoted_repo = RepoRuntime::discover_from_path(&seed_path)
            .expect("discover promoted main-seed runtime");
        assert_eq!(promoted_repo.workspace_root(), seed_path);
        let promoted_state = main_seed_state(&seed_path);
        assert!(is_seed_state_aligned(
            &repo,
            &promoted_state,
            &seed_path,
            "main",
            &snapshot_id
        ));
        let mut stale_staging_state = promoted_state;
        stale_staging_state.workspace_root = Some(
            seed_path
                .with_file_name(".main-seed.tmp-stale")
                .to_string_lossy()
                .to_string(),
        );
        assert!(!is_seed_state_aligned(
            &repo,
            &stale_staging_state,
            &seed_path,
            "main",
            &snapshot_id
        ));
        set_tree_writeable(&seed_path).expect("make promoted seed removable after test");
    }

    #[test]
    fn selected_binary_main_seed_materialization_reads_blobs_without_retired_backend_fallback() {
        let (temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        write_file(&root.join("hello.txt"), "seed hello\n");
        write_file(&root.join("nested/file.txt"), "seed nested\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let snapshot = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("seed target"), false)
            .expect("create Binary DB snapshot");
        let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
        let snapshot_rules =
            snapshot_rules_state_from_snapshot_id(&repo, Some(&snapshot_id)).expect("rules");
        let seed_path = temp.path().join("main-seed");
        let seed_repo = materialize_worktree_runtime_layout(
            &repo,
            "main-seed",
            &seed_path,
            "main",
            "2026-07-08T00:01:00Z",
            None,
        )
        .expect("materialize seed runtime");

        let plan =
            materialize_main_seed_snapshot(&seed_repo, &snapshot_id, None, &snapshot_rules, 0.0)
                .expect("materialize selected Binary DB main seed");

        assert_eq!(plan.write_count, 2);
        assert_eq!(
            fs::read_to_string(seed_path.join("hello.txt")).expect("seed hello"),
            "seed hello\n"
        );
        assert_eq!(
            fs::read_to_string(seed_path.join("nested/file.txt")).expect("seed nested"),
            "seed nested\n"
        );
    }

    #[test]
    fn selected_binary_main_seed_rebuilds_when_cached_baseline_is_corrupt() {
        let (_temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        write_file(&root.join("changed.txt"), "baseline changed\n");
        write_file(&root.join("stable.txt"), "stable\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let baseline = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("seed baseline"), false)
            .expect("create baseline snapshot");
        let baseline_id =
            required_string_field(&baseline, "snapshot_id").expect("baseline snapshot id");
        let baseline_rules = snapshot_rules_state_from_snapshot_id(&repo, Some(&baseline_id))
            .expect("baseline rules");
        let seed_temp = TempDir::new().expect("seed tempdir");
        let seed_path = seed_temp.path().join("main-seed-corrupt");
        let seed_repo = materialize_worktree_runtime_layout(
            &repo,
            "main-seed-corrupt",
            &seed_path,
            "main",
            "2026-07-08T00:01:00Z",
            None,
        )
        .expect("materialize seed runtime");
        materialize_main_seed_snapshot(&seed_repo, &baseline_id, None, &baseline_rules, 0.0)
            .expect("materialize baseline seed");
        let clean_plan = materialize_main_seed_snapshot(
            &seed_repo,
            &baseline_id,
            Some(&baseline_id),
            &baseline_rules,
            0.0,
        )
        .expect("reuse clean cached baseline");
        assert!(!clean_plan.baseline_integrity_reset);
        assert_eq!(clean_plan.write_count, 0);
        assert_eq!(clean_plan.remove_count, 0);
        let aligned_state = MainSeedState {
            exists: true,
            internal_role: Some(INTERNAL_WORKTREE_ROLE_MAIN_SEED.to_string()),
            workspace_root: Some(seed_path.to_string_lossy().to_string()),
            seed_line_name: Some("main".to_string()),
            seed_snapshot_id: Some(baseline_id.clone()),
            seed_content_fingerprint: Some(clean_plan.content_fingerprint.clone()),
            seed_content_row_count: Some(clean_plan.content_row_count),
            layout_version: Some(MAIN_SEED_LAYOUT_VERSION),
            ..MainSeedState::default()
        };
        assert!(is_seed_state_aligned(
            &repo,
            &aligned_state,
            &seed_path,
            "main",
            &baseline_id
        ));

        set_portable_mode(&seed_path.join("stable.txt"), 0o644)
            .expect("make cached stable file writable");
        write_file(&seed_path.join("stable.txt"), "corrupt\n");
        write_file(&seed_path.join("rogue.txt"), "rogue\n");
        assert!(!is_seed_state_aligned(
            &repo,
            &aligned_state,
            &seed_path,
            "main",
            &baseline_id
        ));
        let (forged_fingerprint, forged_row_count) =
            main_seed_content_fingerprint(&seed_path).expect("fingerprint corrupt seed");
        let forged_aligned_state = MainSeedState {
            seed_content_fingerprint: Some(forged_fingerprint),
            seed_content_row_count: Some(forged_row_count),
            ..aligned_state.clone()
        };
        assert!(!is_seed_state_aligned(
            &repo,
            &forged_aligned_state,
            &seed_path,
            "main",
            &baseline_id
        ));
        write_file(&root.join("changed.txt"), "target changed\n");
        let target = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("seed target"), false)
            .expect("create target snapshot");
        let target_id = required_string_field(&target, "snapshot_id").expect("target snapshot id");
        let target_rules =
            snapshot_rules_state_from_snapshot_id(&repo, Some(&target_id)).expect("target rules");

        let plan = materialize_main_seed_snapshot(
            &seed_repo,
            &target_id,
            Some(&baseline_id),
            &target_rules,
            0.0,
        )
        .expect("rebuild corrupt cached baseline");

        assert!(plan.baseline_integrity_reset);
        assert_eq!(plan.write_count, 2);
        assert_eq!(plan.remove_count, 1);
        assert_eq!(
            plan.phase_timings_ms
                .get("visible_row_count_strategy")
                .and_then(JsonValue::as_str),
            Some("baseline_integrity_reset_full_projection")
        );
        assert_eq!(
            fs::read_to_string(seed_path.join("changed.txt")).expect("target changed"),
            "target changed\n"
        );
        assert_eq!(
            fs::read_to_string(seed_path.join("stable.txt")).expect("repaired stable"),
            "stable\n"
        );
        assert!(!seed_path.join("rogue.txt").exists());
    }
}
