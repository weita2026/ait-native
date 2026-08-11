use std::fs;
use std::path::{Path, PathBuf};

use super::config::PrewarmConfig;
use super::helpers::path_string;
use super::paths::is_excluded;

pub(super) fn copy_source_repo(config: &PrewarmConfig, destination: &Path) -> Result<(), String> {
    let source = config
        .source_repo_path
        .as_ref()
        .ok_or_else(|| "source_repo_path is required for source copy.".to_string())?;
    if !source.is_dir() {
        return Err(format!(
            "source_repo_path `{}` must be an existing directory.",
            path_string(source)
        ));
    }
    copy_dir_recursive(source, destination, Path::new(""), &config.copy_excludes)
}

fn copy_dir_recursive(
    source_root: &Path,
    destination_root: &Path,
    relative: &Path,
    excludes: &[PathBuf],
) -> Result<(), String> {
    let source_dir = source_root.join(relative);
    let destination_dir = destination_root.join(relative);
    fs::create_dir_all(&destination_dir).map_err(|exc| {
        format!(
            "Failed to create copied directory `{}`: {exc}",
            path_string(&destination_dir)
        )
    })?;
    for entry in fs::read_dir(&source_dir).map_err(|exc| {
        format!(
            "Failed to read source directory `{}`: {exc}",
            path_string(&source_dir)
        )
    })? {
        let entry = entry.map_err(|exc| format!("Failed to read source directory entry: {exc}"))?;
        let file_name = entry.file_name();
        let child_relative = relative.join(&file_name);
        if is_excluded(&child_relative, excludes) {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination_root.join(&child_relative);
        let metadata = fs::symlink_metadata(&source_path).map_err(|exc| {
            format!(
                "Failed to inspect source path `{}`: {exc}",
                path_string(&source_path)
            )
        })?;
        if metadata.file_type().is_symlink() {
            copy_symlink(&source_path, &destination_path)?;
        } else if metadata.is_dir() {
            copy_dir_recursive(source_root, destination_root, &child_relative, excludes)?;
        } else if metadata.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).map_err(|exc| {
                    format!(
                        "Failed to create file parent `{}`: {exc}",
                        path_string(parent)
                    )
                })?;
            }
            fs::copy(&source_path, &destination_path).map_err(|exc| {
                format!(
                    "Failed to copy `{}` to `{}`: {exc}",
                    path_string(&source_path),
                    path_string(&destination_path)
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> Result<(), String> {
    let target = fs::read_link(source).map_err(|exc| {
        format!(
            "Failed to read symlink `{}` for main-seed copy: {exc}",
            path_string(source)
        )
    })?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            format!(
                "Failed to create symlink parent `{}`: {exc}",
                path_string(parent)
            )
        })?;
    }
    std::os::unix::fs::symlink(&target, destination).map_err(|exc| {
        format!(
            "Failed to copy symlink `{}` to `{}`: {exc}",
            path_string(source),
            path_string(destination)
        )
    })
}

#[cfg(not(unix))]
fn copy_symlink(source: &Path, destination: &Path) -> Result<(), String> {
    let target = fs::canonicalize(source).map_err(|exc| {
        format!(
            "Failed to resolve symlink `{}` for main-seed copy: {exc}",
            path_string(source)
        )
    })?;
    if target.is_dir() {
        copy_dir_recursive(&target, destination, Path::new(""), &[])
    } else {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|exc| {
                format!(
                    "Failed to create symlink-copy parent `{}`: {exc}",
                    path_string(parent)
                )
            })?;
        }
        fs::copy(&target, destination).map(|_| ()).map_err(|exc| {
            format!(
                "Failed to copy symlink target `{}` to `{}`: {exc}",
                path_string(&target),
                path_string(destination)
            )
        })
    }
}
