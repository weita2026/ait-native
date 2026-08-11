use serde_json::{json, Value as JsonValue};
use std::fs;
use std::path::{Path, PathBuf};

use super::helpers::{
    path_is_copy_up_or_child, path_string, remove_existing_with_metadata, remove_path_if_exists,
};

pub(super) fn shard_path_is_reusable(shard_path: &Path) -> Result<bool, String> {
    if !shard_path.exists() {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(shard_path).map_err(|exc| {
        format!(
            "Failed to inspect shard path `{}` before prepare reuse: {exc}",
            path_string(shard_path)
        )
    })?;
    Ok(metadata.is_dir())
}

pub(super) fn reset_prepare_runtime_dirs(paths: &[&Path]) -> Result<(), String> {
    for path in paths {
        let _ = remove_path_if_exists(path)?;
    }
    Ok(())
}

pub(super) fn ensure_reusable_repo_dir(repo_dir: &Path) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(repo_dir) {
        if metadata.file_type().is_symlink() || metadata.is_file() {
            let _ = remove_path_if_exists(repo_dir)?;
        }
    }
    fs::create_dir_all(repo_dir).map_err(|exc| {
        format!(
            "Failed to create reusable shard repo dir `{}`: {exc}",
            path_string(repo_dir)
        )
    })
}

pub(super) fn reset_stale_shard_before_prepare(shard_path: &Path) -> Result<bool, String> {
    if !shard_path.exists() {
        return Ok(false);
    }
    let metadata = fs::symlink_metadata(shard_path).map_err(|exc| {
        format!(
            "Failed to inspect stale shard path `{}` before prepare: {exc}",
            path_string(shard_path)
        )
    })?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(shard_path).map_err(|exc| {
            format!(
                "Failed to remove stale shard file `{}` before prepare: {exc}",
                path_string(shard_path)
            )
        })?;
    } else {
        fs::remove_dir_all(shard_path).map_err(|exc| {
            format!(
                "Failed to remove stale shard directory `{}` before prepare: {exc}",
                path_string(shard_path)
            )
        })?;
    }
    Ok(true)
}

pub(super) fn preserve_repo_dir_cleanup(
    seed_path: &Path,
    shard_path: &Path,
    repo_dir: &Path,
    copy_up_paths: &[PathBuf],
) -> Result<JsonValue, String> {
    let runtime_paths = vec![
        shard_path.join("input"),
        shard_path.join("output"),
        shard_path.join("upper"),
        shard_path.join("work"),
    ];
    let mut removed_runtime_paths = Vec::new();
    for path in &runtime_paths {
        if remove_path_if_exists(path)? {
            removed_runtime_paths.push(path_string(path));
        }
    }
    let pruned_paths = if repo_dir.exists() {
        prune_repo_paths_missing_from_seed(repo_dir, seed_path, copy_up_paths)?
    } else {
        Vec::new()
    };
    Ok(json!({
        "removed_runtime_paths": removed_runtime_paths,
        "pruned_repo_paths": pruned_paths,
        "preserved_copy_up_paths": copy_up_paths
            .iter()
            .map(|path| path_string(path))
            .collect::<Vec<_>>(),
    }))
}

pub(super) fn prune_repo_paths_missing_from_seed(
    repo_dir: &Path,
    seed_path: &Path,
    preserve_paths: &[PathBuf],
) -> Result<Vec<String>, String> {
    let mut removed = Vec::new();
    prune_repo_paths_missing_from_seed_inner(
        repo_dir,
        seed_path,
        Path::new(""),
        preserve_paths,
        &mut removed,
    )?;
    removed.sort();
    Ok(removed)
}

pub(super) fn prune_repo_paths_missing_from_seed_inner(
    repo_dir: &Path,
    seed_path: &Path,
    relative_dir: &Path,
    preserve_paths: &[PathBuf],
    removed: &mut Vec<String>,
) -> Result<(), String> {
    let dir = repo_dir.join(relative_dir);
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&dir).map_err(|exc| {
        format!(
            "Failed to read shard repo dir `{}` while pruning: {exc}",
            path_string(&dir)
        )
    })? {
        let entry = entry.map_err(|exc| {
            format!(
                "Failed to read shard repo entry under `{}` while pruning: {exc}",
                path_string(&dir)
            )
        })?;
        let entry_path = entry.path();
        let relative_path = relative_dir.join(entry.file_name());
        if path_is_copy_up_or_child(&relative_path, preserve_paths) {
            continue;
        }
        let seed_entry = seed_path.join(&relative_path);
        let metadata = fs::symlink_metadata(&entry_path).map_err(|exc| {
            format!(
                "Failed to inspect shard repo path `{}` while pruning: {exc}",
                path_string(&entry_path)
            )
        })?;
        if !seed_entry.exists() {
            remove_existing_with_metadata(&entry_path, &metadata)?;
            removed.push(path_string(&relative_path));
            continue;
        }
        if metadata.is_dir() {
            let seed_metadata = fs::metadata(&seed_entry).map_err(|exc| {
                format!(
                    "Failed to inspect main-seed path `{}` while pruning: {exc}",
                    path_string(&seed_entry)
                )
            })?;
            if !seed_metadata.is_dir() {
                remove_existing_with_metadata(&entry_path, &metadata)?;
                removed.push(path_string(&relative_path));
                continue;
            }
            prune_repo_paths_missing_from_seed_inner(
                repo_dir,
                seed_path,
                &relative_path,
                preserve_paths,
                removed,
            )?;
        }
    }
    Ok(())
}

pub(super) fn guard_shard_path(seed_path: &Path, shard_path: &Path) -> Result<(), String> {
    if shard_path == seed_path {
        return Err("Shard path must not equal main_seed path.".to_string());
    }
    if seed_path.exists() && shard_path.exists() {
        let seed = fs::canonicalize(seed_path).map_err(|exc| {
            format!(
                "Failed to canonicalize main_seed path `{}`: {exc}",
                path_string(seed_path)
            )
        })?;
        let shard = fs::canonicalize(shard_path).map_err(|exc| {
            format!(
                "Failed to canonicalize shard path `{}`: {exc}",
                path_string(shard_path)
            )
        })?;
        if shard == seed || shard.starts_with(&seed) {
            return Err(
                "Shard path must not be the main_seed path or live inside main_seed.".to_string(),
            );
        }
    }
    Ok(())
}
