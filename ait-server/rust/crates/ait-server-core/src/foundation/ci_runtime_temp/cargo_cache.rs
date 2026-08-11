use super::{
    path_string, validated_ci_ram_runtime_root_with_source, CARGO_BUILD_DIR_LEASE_NAME,
    CARGO_PROFILE_LOCK_NAMES, CARGO_WORKSPACE_PATH_HASH_TEMPLATE, MAX_CARGO_CACHE_DISCOVERY_DEPTH,
};
use serde_json::{json, Value as JsonValue};
use std::fs::{self, File, OpenOptions};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct CargoBuildDirLease {
    _file: File,
}

pub fn acquire_cargo_build_dir_lease(build_dir: &Path) -> Result<CargoBuildDirLease, String> {
    let lease_root = cargo_build_dir_lease_root(build_dir);
    fs::create_dir_all(&lease_root).map_err(|error| {
        format!(
            "Failed to create Cargo build-dir lease root `{}`: {error}",
            path_string(&lease_root)
        )
    })?;
    let lease_path = lease_root.join(CARGO_BUILD_DIR_LEASE_NAME);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lease_path)
        .map_err(|error| {
            format!(
                "Failed to open Cargo build-dir lease `{}`: {error}",
                path_string(&lease_path)
            )
        })?;
    #[cfg(unix)]
    {
        let status = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if status != 0 {
            return Err(format!(
                "Failed to acquire Cargo build-dir lease `{}`: {}",
                path_string(&lease_path),
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(CargoBuildDirLease { _file: file })
}

fn cargo_build_dir_lease_root(build_dir: &Path) -> PathBuf {
    if build_dir.file_name().and_then(|value| value.to_str())
        == Some(CARGO_WORKSPACE_PATH_HASH_TEMPLATE)
    {
        if let Some(workspaces_root) = build_dir.parent() {
            if workspaces_root.file_name().and_then(|value| value.to_str()) == Some("workspaces") {
                if let Some(lease_root) = workspaces_root.parent() {
                    return lease_root.to_path_buf();
                }
            }
        }
    }
    build_dir.to_path_buf()
}

pub fn prune_obsolete_cargo_incremental_generations() -> Result<JsonValue, String> {
    let (ram_runtime_root, source) = validated_ci_ram_runtime_root_with_source()?;
    let mut result = prune_obsolete_cargo_incremental_generations_in(&ram_runtime_root)?;
    result["ram_runtime_root_source"] = json!(source);
    Ok(result)
}

pub(super) fn prune_obsolete_cargo_incremental_generations_in(
    ram_runtime_root: &Path,
) -> Result<JsonValue, String> {
    let cargo_target_root = ram_runtime_root.join("cargo-target");
    let cargo_build_root = ram_runtime_root.join("cargo-build");
    let mut removed_generations = Vec::<String>::new();
    let mut removed_session_locks = Vec::<String>::new();
    let mut preserved_generations = Vec::<String>::new();
    let mut skipped_locked = Vec::<String>::new();
    let mut cleanup_errors = Vec::<JsonValue>::new();

    for cargo_root in [&cargo_target_root, &cargo_build_root] {
        if !cargo_root.is_dir() {
            continue;
        }
        let mut incremental_paths = cargo_incremental_candidates(cargo_root)?;
        incremental_paths.sort_by(|left, right| left.0.cmp(&right.0));
        for (incremental_path, _) in incremental_paths {
            let Some(profile_path) = incremental_path.parent() else {
                continue;
            };
            let profile_locks = match try_lock_cargo_profile(profile_path) {
                Ok(Some(locks)) => locks,
                Ok(None) => {
                    skipped_locked.push(path_string(&incremental_path));
                    continue;
                }
                Err(error) => {
                    cleanup_errors.push(json!({
                        "incremental_path": path_string(&incremental_path),
                        "error": error,
                    }));
                    continue;
                }
            };

            let crate_entries = match fs::read_dir(&incremental_path) {
                Ok(entries) => entries,
                Err(error) => {
                    cleanup_errors.push(json!({
                        "incremental_path": path_string(&incremental_path),
                        "error": error.to_string(),
                    }));
                    drop(profile_locks);
                    continue;
                }
            };
            for crate_entry in crate_entries.filter_map(Result::ok) {
                let crate_path = crate_entry.path();
                let Ok(crate_metadata) = fs::symlink_metadata(&crate_path) else {
                    continue;
                };
                if !crate_metadata.is_dir() || crate_metadata.file_type().is_symlink() {
                    continue;
                }
                let sessions = match cargo_incremental_session_generations(&crate_path) {
                    Ok(sessions) => sessions,
                    Err(error) => {
                        cleanup_errors.push(json!({
                            "crate_incremental_path": path_string(&crate_path),
                            "error": error,
                        }));
                        continue;
                    }
                };
                let Some((newest, older)) = sessions.split_last() else {
                    continue;
                };
                preserved_generations.push(path_string(&newest.0));
                for (generation_path, _) in older {
                    match fs::remove_dir_all(generation_path) {
                        Ok(()) => {
                            removed_generations.push(path_string(generation_path));
                            if let Some(lock_path) = cargo_incremental_session_lock(generation_path)
                            {
                                if lock_path.is_file() {
                                    match fs::remove_file(&lock_path) {
                                        Ok(()) => {
                                            removed_session_locks.push(path_string(&lock_path))
                                        }
                                        Err(error) => cleanup_errors.push(json!({
                                            "session_lock_path": path_string(&lock_path),
                                            "error": error.to_string(),
                                        })),
                                    }
                                }
                            }
                        }
                        Err(error) => cleanup_errors.push(json!({
                            "generation_path": path_string(generation_path),
                            "error": error.to_string(),
                        })),
                    }
                }
            }
            drop(profile_locks);
        }
    }

    Ok(json!({
        "contract": "ait.server.cargo_incremental_generation_prune.v1",
        "status": if cleanup_errors.is_empty() { "cleaned" } else { "partial" },
        "cargo_target_root": path_string(&cargo_target_root),
        "cargo_build_root": path_string(&cargo_build_root),
        "removed_generation_count": removed_generations.len(),
        "removed_generations": removed_generations,
        "removed_session_locks": removed_session_locks,
        "preserved_generation_count": preserved_generations.len(),
        "preserved_generations": preserved_generations,
        "skipped_locked": skipped_locked,
        "cleanup_errors": cleanup_errors,
        "retention_policy": "newest_generation_per_crate",
        "preserved_scopes": ["build", "deps", "fingerprint", "artifacts", "manifests"],
    }))
}

fn cargo_incremental_session_generations(
    crate_path: &Path,
) -> Result<Vec<(PathBuf, u128)>, String> {
    let mut sessions = fs::read_dir(crate_path)
        .map_err(|error| {
            format!(
                "Failed to inspect Cargo incremental crate `{}`: {error}",
                path_string(crate_path)
            )
        })?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).ok()?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return None;
            }
            let modified = metadata
                .modified()
                .unwrap_or(UNIX_EPOCH)
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            Some((path, modified))
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    Ok(sessions)
}

fn cargo_incremental_session_lock(generation_path: &Path) -> Option<PathBuf> {
    let generation_name = generation_path.file_name()?.to_str()?;
    let (session_name, _) = generation_name.rsplit_once('-')?;
    Some(
        generation_path
            .parent()?
            .join(format!("{session_name}.lock")),
    )
}

pub(super) fn reclaim_cargo_incremental_cache_with_available<F>(
    ram_runtime_root: &Path,
    target_available_bytes: u64,
    mut available_bytes: F,
) -> Result<JsonValue, String>
where
    F: FnMut(&Path) -> Result<u64, String>,
{
    let cargo_target_root = ram_runtime_root.join("cargo-target");
    let cargo_build_root = ram_runtime_root.join("cargo-build");
    let available_before = available_bytes(ram_runtime_root)?;
    let mut available_after = available_before;
    let mut removed_incremental = Vec::<String>::new();
    let mut removed_build_profiles = Vec::<String>::new();
    let mut removed_build_profile_entries = Vec::<String>::new();
    let mut skipped_locked = Vec::<String>::new();
    let mut skipped_leased = Vec::<String>::new();
    let mut cleanup_errors = Vec::<JsonValue>::new();
    if available_before < target_available_bytes {
        let mut candidates = Vec::new();
        for cargo_root in [&cargo_target_root, &cargo_build_root] {
            if cargo_root.is_dir() {
                candidates.extend(cargo_incremental_candidates(cargo_root)?);
            }
        }
        candidates.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        for (incremental_path, _) in candidates {
            if available_after >= target_available_bytes {
                break;
            }
            let Some(profile_path) = incremental_path.parent() else {
                continue;
            };
            let locks = match try_lock_cargo_profile(profile_path) {
                Ok(Some(locks)) => locks,
                Ok(None) => {
                    skipped_locked.push(path_string(&incremental_path));
                    continue;
                }
                Err(error) => {
                    cleanup_errors.push(json!({
                        "incremental_path": path_string(&incremental_path),
                        "error": error,
                    }));
                    continue;
                }
            };
            available_after = available_bytes(ram_runtime_root)?;
            if available_after >= target_available_bytes {
                drop(locks);
                break;
            }
            match fs::remove_dir_all(&incremental_path) {
                Ok(()) => removed_incremental.push(path_string(&incremental_path)),
                Err(error) => cleanup_errors.push(json!({
                    "incremental_path": path_string(&incremental_path),
                    "error": error.to_string(),
                })),
            }
            drop(locks);
            available_after = available_bytes(ram_runtime_root)?;
        }
    }

    if available_after < target_available_bytes && cargo_build_root.is_dir() {
        let mut profiles = cargo_profile_candidates(&cargo_build_root)?;
        profiles.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        for (profile_path, _) in profiles {
            if available_after >= target_available_bytes {
                break;
            }
            let build_cache_lease =
                match try_lock_cargo_build_cache_for_reclaim(&profile_path, &cargo_build_root) {
                    Ok(Some(lease)) => lease,
                    Ok(None) => {
                        skipped_leased.push(path_string(&profile_path));
                        continue;
                    }
                    Err(error) => {
                        cleanup_errors.push(json!({
                            "build_profile_path": path_string(&profile_path),
                            "error": error,
                        }));
                        continue;
                    }
                };
            let locks = match try_lock_cargo_profile(&profile_path) {
                Ok(Some(locks)) => locks,
                Ok(None) => {
                    skipped_locked.push(path_string(&profile_path));
                    drop(build_cache_lease);
                    continue;
                }
                Err(error) => {
                    cleanup_errors.push(json!({
                        "build_profile_path": path_string(&profile_path),
                        "error": error,
                    }));
                    drop(build_cache_lease);
                    continue;
                }
            };
            available_after = available_bytes(ram_runtime_root)?;
            if available_after >= target_available_bytes {
                drop(locks);
                drop(build_cache_lease);
                break;
            }
            match clear_cargo_build_profile_contents(&profile_path) {
                Ok((removed, errors)) => {
                    if !removed.is_empty() {
                        removed_build_profiles.push(path_string(&profile_path));
                        removed_build_profile_entries.extend(removed);
                    }
                    cleanup_errors.extend(errors);
                }
                Err(error) => cleanup_errors.push(json!({
                    "build_profile_path": path_string(&profile_path),
                    "error": error,
                })),
            }
            drop(locks);
            drop(build_cache_lease);
            available_after = available_bytes(ram_runtime_root)?;
        }
    }
    Ok(json!({
        "contract": "ait.server.cargo_incremental_reclamation.v1",
        "status": if available_after >= target_available_bytes { "ready" } else { "insufficient_capacity" },
        "cargo_target_root": path_string(&cargo_target_root),
        "cargo_build_root": path_string(&cargo_build_root),
        "target_available_bytes": target_available_bytes,
        "available_before": available_before,
        "available_after": available_after,
        "removed_incremental_count": removed_incremental.len(),
        "removed_incremental": removed_incremental,
        "removed_build_profile_count": removed_build_profiles.len(),
        "removed_build_profiles": removed_build_profiles,
        "removed_build_profile_entries": removed_build_profile_entries,
        "skipped_locked": skipped_locked,
        "skipped_leased": skipped_leased,
        "cleanup_errors": cleanup_errors,
        "preserved_scopes": ["build", "deps", "fingerprint", "artifacts", "manifests"],
        "reclaimable_build_dir_scopes": ["idle_build_profiles"],
        "reclamation_guards": ["active_ci_build_leases", "active_profiles", "cargo_profile_locks", "final_target_artifacts"],
    }))
}

fn cargo_build_cache_root_for_profile(
    profile_path: &Path,
    cargo_build_root: &Path,
) -> Result<PathBuf, String> {
    let relative = profile_path.strip_prefix(cargo_build_root).map_err(|_| {
        format!(
            "Cargo build profile `{}` is outside build root `{}`.",
            path_string(profile_path),
            path_string(cargo_build_root)
        )
    })?;
    let components = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    if components.len() < 2 {
        return Err(format!(
            "Cargo build profile `{}` does not identify a repository cache below `{}`.",
            path_string(profile_path),
            path_string(cargo_build_root)
        ));
    }
    if components.get(1) == Some(&"ci") {
        return Ok(cargo_build_root.join(components[0]).join("ci"));
    }
    profile_path.parent().map(Path::to_path_buf).ok_or_else(|| {
        format!(
            "Cargo build profile `{}` does not have a cache root.",
            path_string(profile_path)
        )
    })
}

#[cfg(unix)]
fn try_lock_cargo_build_cache_for_reclaim(
    profile_path: &Path,
    cargo_build_root: &Path,
) -> Result<Option<File>, String> {
    let cache_root = cargo_build_cache_root_for_profile(profile_path, cargo_build_root)?;
    let lease_path = cache_root.join(CARGO_BUILD_DIR_LEASE_NAME);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lease_path)
        .map_err(|error| {
            format!(
                "Failed to open Cargo build-dir reclaim lease `{}`: {error}",
                path_string(&lease_path)
            )
        })?;
    let status = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if status == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return Ok(None);
    }
    Err(format!(
        "Failed to acquire Cargo build-dir reclaim lease `{}`: {error}",
        path_string(&lease_path)
    ))
}

#[cfg(not(unix))]
fn try_lock_cargo_build_cache_for_reclaim(
    _profile_path: &Path,
    _cargo_build_root: &Path,
) -> Result<Option<File>, String> {
    Ok(None)
}

fn cargo_profile_candidates(root: &Path) -> Result<Vec<(PathBuf, u128)>, String> {
    let mut candidates = Vec::new();
    collect_cargo_profile_candidates(root, 0, &mut candidates)?;
    Ok(candidates)
}

fn collect_cargo_profile_candidates(
    path: &Path,
    depth: usize,
    candidates: &mut Vec<(PathBuf, u128)>,
) -> Result<(), String> {
    if depth > MAX_CARGO_CACHE_DISCOVERY_DEPTH {
        return Ok(());
    }
    let entries = fs::read_dir(path)
        .map_err(|error| {
            format!(
                "Failed to inspect Cargo build dir `{}`: {error}",
                path_string(path)
            )
        })?
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    let has_profile_lock = entries.iter().any(|entry| {
        entry
            .file_name()
            .to_str()
            .is_some_and(is_cargo_profile_lock_name)
            && entry.path().is_file()
    });
    if has_profile_lock {
        let modified = fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH)
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        candidates.push((path.to_path_buf(), modified));
        return Ok(());
    }
    for entry in entries {
        let candidate = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&candidate) else {
            continue;
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            collect_cargo_profile_candidates(&candidate, depth + 1, candidates)?;
        }
    }
    Ok(())
}

fn clear_cargo_build_profile_contents(
    profile_path: &Path,
) -> Result<(Vec<String>, Vec<JsonValue>), String> {
    let entries = fs::read_dir(profile_path).map_err(|error| {
        format!(
            "Failed to inspect Cargo build profile `{}`: {error}",
            path_string(profile_path)
        )
    })?;
    let mut removed = Vec::new();
    let mut errors = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if entry
            .file_name()
            .to_str()
            .is_some_and(is_cargo_profile_lock_name)
        {
            continue;
        }
        let result = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                fs::remove_dir_all(&path)
            }
            Ok(_) => fs::remove_file(&path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => Err(error),
        };
        match result {
            Ok(()) => removed.push(path_string(&path)),
            Err(error) => errors.push(json!({
                "build_profile_entry": path_string(&path),
                "error": error.to_string(),
            })),
        }
    }
    Ok((removed, errors))
}

fn is_cargo_profile_lock_name(name: &str) -> bool {
    CARGO_PROFILE_LOCK_NAMES.contains(&name)
}

fn cargo_incremental_candidates(root: &Path) -> Result<Vec<(PathBuf, u128)>, String> {
    let mut candidates = Vec::new();
    collect_cargo_incremental_candidates(root, 0, &mut candidates)?;
    Ok(candidates)
}

fn collect_cargo_incremental_candidates(
    path: &Path,
    depth: usize,
    candidates: &mut Vec<(PathBuf, u128)>,
) -> Result<(), String> {
    if depth > MAX_CARGO_CACHE_DISCOVERY_DEPTH {
        return Ok(());
    }
    let entries = fs::read_dir(path).map_err(|error| {
        format!(
            "Failed to inspect shared Cargo cache `{}`: {error}",
            path_string(path)
        )
    })?;
    for entry in entries.filter_map(Result::ok) {
        let candidate = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&candidate) else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        if candidate.file_name().and_then(|value| value.to_str()) == Some("incremental") {
            let modified = metadata
                .modified()
                .unwrap_or(UNIX_EPOCH)
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            candidates.push((candidate, modified));
        } else {
            collect_cargo_incremental_candidates(&candidate, depth + 1, candidates)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn try_lock_cargo_profile(profile_path: &Path) -> Result<Option<Vec<File>>, String> {
    let existing_locks = CARGO_PROFILE_LOCK_NAMES
        .into_iter()
        .map(|name| profile_path.join(name))
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    if existing_locks.is_empty() {
        return Ok(None);
    }
    let mut locked_files = Vec::new();
    for lock_path in existing_locks {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                format!(
                    "Failed to open Cargo profile lock `{}`: {error}",
                    path_string(&lock_path)
                )
            })?;
        let status = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if status != 0 {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::PermissionDenied
            ) {
                return Ok(None);
            }
            return Err(format!(
                "Failed to lock Cargo profile `{}` through `{}`: {error}",
                path_string(profile_path),
                path_string(&lock_path)
            ));
        }
        locked_files.push(file);
    }
    Ok(Some(locked_files))
}

#[cfg(not(unix))]
pub(super) fn try_lock_cargo_profile(_profile_path: &Path) -> Result<Option<Vec<File>>, String> {
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FilesystemCapacity {
    pub(super) available_bytes: u64,
    pub(super) total_bytes: u64,
}

#[cfg(unix)]
pub(super) fn filesystem_capacity_bytes(path: &Path) -> Result<FilesystemCapacity, String> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let encoded = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        format!(
            "CI RAM runtime root contains an interior NUL byte: {}",
            path_string(path)
        )
    })?;
    let mut stat = MaybeUninit::<libc::statvfs>::uninit();
    let status = unsafe { libc::statvfs(encoded.as_ptr(), stat.as_mut_ptr()) };
    if status != 0 {
        return Err(format!(
            "Failed to inspect available bytes for CI RAM runtime root `{}`: {}",
            path_string(path),
            std::io::Error::last_os_error()
        ));
    }
    let stat = unsafe { stat.assume_init() };
    let available_bytes = (stat.f_bavail as u64)
        .checked_mul(stat.f_frsize)
        .ok_or_else(|| "CI RAM runtime available-byte count overflowed u64".to_string())?;
    let total_bytes = (stat.f_blocks as u64)
        .checked_mul(stat.f_frsize)
        .ok_or_else(|| "CI RAM runtime total-byte count overflowed u64".to_string())?;
    Ok(FilesystemCapacity {
        available_bytes,
        total_bytes,
    })
}

#[cfg(unix)]
pub(super) fn filesystem_available_bytes(path: &Path) -> Result<u64, String> {
    Ok(filesystem_capacity_bytes(path)?.available_bytes)
}

#[cfg(not(unix))]
pub(super) fn filesystem_available_bytes(_path: &Path) -> Result<u64, String> {
    Err("CI RAM minimum available-byte validation is unsupported on this platform".to_string())
}

#[cfg(not(unix))]
pub(super) fn filesystem_capacity_bytes(_path: &Path) -> Result<FilesystemCapacity, String> {
    Err("CI RAM capacity validation is unsupported on this platform".to_string())
}
