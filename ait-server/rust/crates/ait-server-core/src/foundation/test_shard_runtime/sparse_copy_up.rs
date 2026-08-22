use serde_json::{json, Value as JsonValue};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::helpers::{
    create_symlink, path_is_copy_up_or_child, path_string, remove_existing_with_metadata,
};
use super::materialization::PrepareOneShardInput;
use super::process::apfs_clone_or_copy;

pub(super) fn prepare_sparse_copy_up(
    input: &PrepareOneShardInput<'_>,
) -> Result<JsonValue, String> {
    let repo_dir = input.shard_path.join("repo");
    let copy_up_set = input
        .copy_up_paths
        .iter()
        .map(|path| path_string(path))
        .collect::<BTreeSet<_>>();
    let implicit_link_paths = if input.implicit_immutable_seed_links {
        collect_seed_immutable_link_paths(&input.seed_path, input.copy_up_paths)?
    } else {
        Vec::new()
    };
    let immutable_link_paths: Vec<&PathBuf> = if input.implicit_immutable_seed_links {
        implicit_link_paths.iter().collect()
    } else {
        input.immutable_link_paths.iter().collect()
    };
    let mut copied = Vec::new();
    let mut linked = Vec::new();
    if !input.dry_run {
        for relative_path in input.copy_up_paths {
            copied.push(if input.preserve_repo_dir {
                stage_copy_up_path_for_overlay(&input.seed_path, &repo_dir, relative_path)?
            } else {
                copy_up_path(
                    &input.seed_path,
                    &repo_dir,
                    relative_path,
                    input.platform,
                    input.prefer_apfs_clone,
                )?
            });
        }
        for relative_path in &immutable_link_paths {
            if copy_up_set.contains(&path_string(relative_path)) {
                continue;
            }
            linked.push(link_immutable_path(
                &input.seed_path,
                &repo_dir,
                relative_path.as_path(),
            )?);
        }
    } else {
        copied = input
            .copy_up_paths
            .iter()
            .map(|path| {
                json!({
                    "relative_path": path_string(path),
                    "source": path_string(&input.seed_path.join(path)),
                    "destination": path_string(&repo_dir.join(path)),
                    "dry_run": true
                })
            })
            .collect();
        linked = immutable_link_paths
            .iter()
            .filter(|path| !copy_up_set.contains(&path_string(path)))
            .map(|path| {
                json!({
                    "relative_path": path_string(path),
                    "source": path_string(&input.seed_path.join(path)),
                    "destination": path_string(&repo_dir.join(path)),
                    "dry_run": true
                })
            })
            .collect();
    }

    Ok(json!({
        "strategy": input.strategy.name(),
        "platform": input.platform,
        "repo_dir": path_string(&repo_dir),
        "copy_up_paths": copied,
        "immutable_links": linked,
        "copy_up_semantics": "patch_touched_files_are_real_shard_local_files",
        "selective_links_only": true,
        "implicit_immutable_seed_links": input.implicit_immutable_seed_links,
        "whole_seed_directory_symlink": false,
        "copy_entire_main_seed": false,
        "prefer_apfs_clone": input.prefer_apfs_clone
    }))
}

fn stage_copy_up_path_for_overlay(
    seed_path: &Path,
    repo_dir: &Path,
    relative_path: &Path,
) -> Result<JsonValue, String> {
    let source = seed_path.join(relative_path);
    let destination = repo_dir.join(relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create parent directory `{}` for deferred copy-up: {exc}",
                path_string(parent)
            )
        })?;
    }
    if !source.exists() {
        return Ok(json!({
            "relative_path": path_string(relative_path),
            "source": path_string(&source),
            "destination": path_string(&destination),
            "source_missing": true,
            "deferred_to_revision_overlay": true
        }));
    }
    let metadata = fs::metadata(&source).map_err(|exc| {
        format!(
            "Failed to inspect deferred copy-up source `{}`: {exc}",
            path_string(&source)
        )
    })?;
    if metadata.is_dir() {
        if let Ok(existing) = fs::symlink_metadata(&destination) {
            if existing.file_type().is_symlink() || existing.is_file() {
                remove_existing_with_metadata(&destination, &existing)?;
            }
        }
        fs::create_dir_all(&destination).map_err(|exc| {
            format!(
                "Failed to create deferred copy-up directory `{}`: {exc}",
                path_string(&destination)
            )
        })?;
        return Ok(json!({
            "relative_path": path_string(relative_path),
            "source": path_string(&source),
            "destination": path_string(&destination),
            "kind": "directory",
            "method": "mkdir",
            "deferred_to_revision_overlay": true
        }));
    }
    Ok(json!({
        "relative_path": path_string(relative_path),
        "source": path_string(&source),
        "destination": path_string(&destination),
        "kind": "file",
        "method": "preserve_existing_until_revision_overlay",
        "deferred_to_revision_overlay": true
    }))
}

fn collect_seed_immutable_link_paths(
    seed_path: &Path,
    copy_up_paths: &[PathBuf],
) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    collect_seed_immutable_link_paths_inner(seed_path, Path::new(""), copy_up_paths, &mut paths)?;
    paths.sort_by_key(|left| path_string(left));
    Ok(paths)
}

fn collect_seed_immutable_link_paths_inner(
    seed_path: &Path,
    relative_dir: &Path,
    copy_up_paths: &[PathBuf],
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let dir = seed_path.join(relative_dir);
    let mut entries = fs::read_dir(&dir)
        .map_err(|exc| {
            format!(
                "Failed to read main_seed directory `{}`: {exc}",
                path_string(&dir)
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|exc| {
            format!(
                "Failed to read main_seed directory entry under `{}`: {exc}",
                path_string(&dir)
            )
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_name = PathBuf::from(entry.file_name());
        let relative_path = if relative_dir.as_os_str().is_empty() {
            file_name
        } else {
            relative_dir.join(file_name)
        };
        if path_is_copy_up_or_child(&relative_path, copy_up_paths) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(|exc| {
            format!(
                "Failed to inspect main_seed path `{}`: {exc}",
                path_string(&entry.path())
            )
        })?;
        if metadata.file_type().is_dir() {
            collect_seed_immutable_link_paths_inner(
                seed_path,
                &relative_path,
                copy_up_paths,
                paths,
            )?;
        } else {
            paths.push(relative_path);
        }
    }
    Ok(())
}

fn copy_up_path(
    seed_path: &Path,
    repo_dir: &Path,
    relative_path: &Path,
    platform: &str,
    prefer_apfs_clone: bool,
) -> Result<JsonValue, String> {
    let source = seed_path.join(relative_path);
    let destination = repo_dir.join(relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create parent directory `{}` for copy-up: {exc}",
                path_string(parent)
            )
        })?;
    }
    if !source.exists() {
        return Ok(json!({
            "relative_path": path_string(relative_path),
            "source": path_string(&source),
            "destination": path_string(&destination),
            "source_missing": true,
            "created_parent_for_added_file": true
        }));
    }
    let metadata = fs::metadata(&source).map_err(|exc| {
        format!(
            "Failed to inspect copy-up source `{}`: {exc}",
            path_string(&source)
        )
    })?;
    if metadata.is_dir() {
        if let Ok(existing) = fs::symlink_metadata(&destination) {
            if existing.file_type().is_symlink() || existing.is_file() {
                remove_existing_with_metadata(&destination, &existing)?;
            }
        }
        fs::create_dir_all(&destination).map_err(|exc| {
            format!(
                "Failed to create copy-up directory `{}`: {exc}",
                path_string(&destination)
            )
        })?;
        return Ok(json!({
            "relative_path": path_string(relative_path),
            "source": path_string(&source),
            "destination": path_string(&destination),
            "kind": "directory",
            "method": "mkdir"
        }));
    }
    if let Ok(existing) = fs::symlink_metadata(&destination) {
        remove_existing_with_metadata(&destination, &existing)?;
    }
    let method = if platform == "macos" && prefer_apfs_clone {
        apfs_clone_or_copy(&source, &destination)?
    } else {
        fs::copy(&source, &destination).map_err(|exc| {
            format!(
                "Failed to copy-up `{}` to `{}`: {exc}",
                path_string(&source),
                path_string(&destination)
            )
        })?;
        "std_copy".to_string()
    };
    Ok(json!({
        "relative_path": path_string(relative_path),
        "source": path_string(&source),
        "destination": path_string(&destination),
        "kind": "file",
        "method": method,
        "real_shard_local_file": true
    }))
}

fn link_immutable_path(
    seed_path: &Path,
    repo_dir: &Path,
    relative_path: &Path,
) -> Result<JsonValue, String> {
    let source = seed_path.join(relative_path);
    let destination = repo_dir.join(relative_path);
    if !source.exists() {
        return Ok(json!({
            "relative_path": path_string(relative_path),
            "source": path_string(&source),
            "destination": path_string(&destination),
            "source_missing": true,
            "linked": false
        }));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create parent directory `{}` for immutable link: {exc}",
                path_string(parent)
            )
        })?;
    }
    if let Ok(existing) = fs::symlink_metadata(&destination) {
        if existing.file_type().is_symlink()
            && fs::read_link(&destination).ok().as_deref() == Some(source.as_path())
        {
            return Ok(json!({
                "relative_path": path_string(relative_path),
                "source": path_string(&source),
                "destination": path_string(&destination),
                "linked": true,
                "immutable": true,
                "reused_existing_link": true
            }));
        }
        remove_existing_with_metadata(&destination, &existing)?;
    }
    create_symlink(&source, &destination)?;
    Ok(json!({
        "relative_path": path_string(relative_path),
        "source": path_string(&source),
        "destination": path_string(&destination),
        "linked": true,
        "immutable": true
    }))
}
