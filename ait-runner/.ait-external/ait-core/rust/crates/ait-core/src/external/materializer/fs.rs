use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::external::lockfile::{ExternalLockNode, ExternalLockfile};
use crate::external::materializer::{
    ExternalContentSource, ExternalMaterializationEntry, ExternalMaterializationOptions,
    ExternalMaterializationReport, ExternalMaterializationState, ExternalMaterializer,
    ExternalMaterializerMarkerFileEntry, ExternalMaterializerMarkerJson,
    EXTERNAL_MATERIALIZER_MARKER,
};
use crate::external::{ExternalError, ExternalResult};

#[derive(Debug, Clone)]
pub struct FilesystemExternalMaterializer<C> {
    repo_root: PathBuf,
    content_source: C,
}

impl<C> FilesystemExternalMaterializer<C>
where
    C: ExternalContentSource,
{
    pub fn new(repo_root: impl Into<PathBuf>, content_source: C) -> ExternalResult<Self> {
        let repo_root = repo_root.into();
        if repo_root.as_os_str().is_empty() {
            return Err(ExternalError::with_code(
                "external_materializer_repo_root",
                "external materializer repo root must not be empty",
            ));
        }
        Ok(Self {
            repo_root,
            content_source,
        })
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }
}

impl<C> ExternalMaterializer for FilesystemExternalMaterializer<C>
where
    C: ExternalContentSource,
{
    fn materialize_lockfile(
        &self,
        lockfile: &ExternalLockfile,
        options: &ExternalMaterializationOptions,
    ) -> ExternalResult<ExternalMaterializationReport> {
        lockfile.validate()?;
        options.reject_forbidden_local_links()?;

        let linked_roots = linked_root_materialization_paths(lockfile, options);
        let mut entries = Vec::new();
        for node in lockfile.sorted_nodes() {
            if node_is_under_linked_root(&node, &linked_roots) {
                entries.push(ExternalMaterializationEntry::from_node(
                    &node,
                    ExternalMaterializationState::SkippedLocalLink,
                ));
                continue;
            }
            if options.no_recursive && !node.parent_path.is_empty() {
                entries.push(ExternalMaterializationEntry::from_node(
                    &node,
                    ExternalMaterializationState::SkippedNoRecursive,
                ));
                continue;
            }
            self.materialize_node(&node)?;
            entries.push(ExternalMaterializationEntry::from_node(
                &node,
                ExternalMaterializationState::Materialized,
            ));
        }
        Ok(ExternalMaterializationReport { entries })
    }
}

fn linked_root_materialization_paths(
    lockfile: &ExternalLockfile,
    options: &ExternalMaterializationOptions,
) -> Vec<String> {
    lockfile
        .nodes
        .iter()
        .filter(|node| {
            node.parent_path.is_empty()
                && options
                    .local_link_overrides
                    .iter()
                    .any(|link| link.name == node.name)
        })
        .map(|node| node.materialize_to.clone())
        .collect()
}

fn node_is_under_linked_root(node: &ExternalLockNode, linked_roots: &[String]) -> bool {
    linked_roots.iter().any(|root| {
        node.materialize_to == *root
            || node.parent_path == *root
            || node.parent_path.starts_with(&format!("{root}/"))
    })
}

impl<C> FilesystemExternalMaterializer<C>
where
    C: ExternalContentSource,
{
    fn materialize_node(&self, node: &ExternalLockNode) -> ExternalResult<()> {
        let destination = self.safe_destination(&node.materialize_to)?;
        prepare_generated_destination(&destination, &node.materialize_to)?;
        fs::create_dir_all(&destination).map_err(|err| {
            ExternalError::with_code(
                "external_materializer_create_dir",
                format!(
                    "failed to create external materialization directory {:?}: {err}",
                    node.materialize_to
                ),
            )
        })?;
        self.content_source
            .materialize_content(node, &destination)?;
        write_marker(node, &destination)
    }

    fn safe_destination(&self, materialize_to: &str) -> ExternalResult<PathBuf> {
        let relative = validate_repo_relative_path(materialize_to, "materialize_to")?;
        let mut destination = self.repo_root.clone();
        for component in relative.components() {
            if let Component::Normal(part) = component {
                destination.push(part);
            }
        }
        ensure_existing_ancestors_are_not_symlinks(&self.repo_root, &destination, materialize_to)?;
        Ok(destination)
    }
}

fn prepare_generated_destination(destination: &Path, display_path: &str) -> ExternalResult<()> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(ExternalError::with_code(
                    "external_materializer_symlink",
                    format!("external materialization path {display_path:?} must not be a symlink"),
                ));
            }
            if metadata.is_file() {
                return Err(ExternalError::with_code(
                    "external_materializer_not_directory",
                    format!(
                        "external materialization path {display_path:?} is a file, not a generated directory"
                    ),
                ));
            }
            if !metadata.is_dir() {
                return Err(ExternalError::with_code(
                    "external_materializer_not_directory",
                    format!("external materialization path {display_path:?} is not a directory"),
                ));
            }
            if !generated_marker_is_regular_file(destination)? {
                return Err(ExternalError::with_code(
                    "external_materializer_dirty_directory",
                    format!(
                        "external materialization path {display_path:?} exists but is not marked as generated by AIT"
                    ),
                ));
            }
            ensure_generated_tree_contains_no_symlinks(destination, display_path)?;
            fs::remove_dir_all(destination).map_err(|err| {
                ExternalError::with_code(
                    "external_materializer_remove_dir",
                    format!(
                        "failed to replace generated external directory {display_path:?}: {err}"
                    ),
                )
            })?;
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(ExternalError::with_code(
            "external_materializer_stat",
            format!("failed to inspect external materialization path {display_path:?}: {err}"),
        )),
    }
}

fn generated_marker_is_regular_file(destination: &Path) -> ExternalResult<bool> {
    match fs::symlink_metadata(destination.join(EXTERNAL_MATERIALIZER_MARKER)) {
        Ok(metadata) => Ok(metadata.is_file() && !metadata.file_type().is_symlink()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(ExternalError::with_code(
            "external_materializer_stat",
            format!("failed to inspect external materialization marker: {err}"),
        )),
    }
}

fn ensure_generated_tree_contains_no_symlinks(
    directory: &Path,
    display_path: &str,
) -> ExternalResult<()> {
    for entry in fs::read_dir(directory).map_err(|err| {
        ExternalError::with_code(
            "external_materializer_stat",
            format!("failed to inspect generated external directory {display_path:?}: {err}"),
        )
    })? {
        let entry = entry.map_err(|err| {
            ExternalError::with_code(
                "external_materializer_stat",
                format!("failed to inspect generated external directory {display_path:?}: {err}"),
            )
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|err| {
            ExternalError::with_code(
                "external_materializer_stat",
                format!(
                    "failed to inspect generated external directory entry {:?}: {err}",
                    path
                ),
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ExternalError::with_code(
                "external_materializer_symlink",
                format!(
                    "external generated directory {display_path:?} contains symlink {:?}",
                    path
                ),
            ));
        }
        if metadata.is_dir() {
            ensure_generated_tree_contains_no_symlinks(&path, display_path)?;
        }
    }
    Ok(())
}

fn write_marker(node: &ExternalLockNode, destination: &Path) -> ExternalResult<()> {
    let files = collect_materialized_file_entries(destination)?;
    ExternalMaterializerMarkerJson::filesystem().write_marker(
        &destination.join(EXTERNAL_MATERIALIZER_MARKER),
        node,
        &files,
    )
}

fn collect_materialized_file_entries(
    destination: &Path,
) -> ExternalResult<Vec<ExternalMaterializerMarkerFileEntry>> {
    let mut entries = Vec::new();
    collect_materialized_file_entries_recursive(destination, destination, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn collect_materialized_file_entries_recursive(
    root: &Path,
    cursor: &Path,
    entries: &mut Vec<ExternalMaterializerMarkerFileEntry>,
) -> ExternalResult<()> {
    for entry in fs::read_dir(cursor).map_err(|err| {
        ExternalError::with_code(
            "external_materializer_stat",
            format!(
                "failed to inspect generated external directory {:?}: {err}",
                cursor
            ),
        )
    })? {
        let entry = entry.map_err(|err| {
            ExternalError::with_code(
                "external_materializer_stat",
                format!(
                    "failed to inspect generated external directory {:?}: {err}",
                    cursor
                ),
            )
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|err| {
            ExternalError::with_code(
                "external_materializer_stat",
                format!(
                    "failed to inspect generated external directory entry {:?}: {err}",
                    path
                ),
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ExternalError::with_code(
                "external_materializer_symlink",
                format!("external generated directory contains symlink {:?}", path),
            ));
        }
        if metadata.is_dir() {
            collect_materialized_file_entries_recursive(root, &path, entries)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(ExternalError::with_code(
                "external_materializer_stat",
                format!(
                    "generated external directory contains unsupported entry {:?}",
                    path
                ),
            ));
        }
        if path.file_name() == Some(OsStr::new(EXTERNAL_MATERIALIZER_MARKER)) {
            continue;
        }
        let data = fs::read(&path).map_err(|err| {
            ExternalError::with_code(
                "external_materializer_stat",
                format!("failed to read generated external file {:?}: {err}", path),
            )
        })?;
        let relative = path.strip_prefix(root).map_err(|_| {
            ExternalError::with_code(
                "external_materializer_stat",
                format!(
                    "generated external file {:?} is outside the materialized root",
                    path
                ),
            )
        })?;
        entries.push(ExternalMaterializerMarkerFileEntry::new(
            normalize_marker_relative_path(relative),
            sha256_hex(&data),
        ));
    }
    Ok(())
}

fn normalize_marker_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn validate_repo_relative_path(path: &str, field: &str) -> ExternalResult<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return Err(ExternalError::with_code(
            "external_materializer_path",
            format!("{field} must not be empty"),
        ));
    }
    let parsed = Path::new(path);
    if parsed.is_absolute() {
        return Err(ExternalError::with_code(
            "external_materializer_path",
            format!("{field} must be repository-relative, got absolute path {path:?}"),
        ));
    }

    let mut normalized = PathBuf::new();
    let mut has_normal = false;
    for component in parsed.components() {
        match component {
            Component::Normal(part) => {
                has_normal = true;
                normalized.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ExternalError::with_code(
                    "external_materializer_path",
                    format!("{field} must not escape the repository, got {path:?}"),
                ));
            }
        }
    }
    if !has_normal {
        return Err(ExternalError::with_code(
            "external_materializer_path",
            format!("{field} must contain a repository-relative path component"),
        ));
    }
    Ok(normalized)
}

fn ensure_existing_ancestors_are_not_symlinks(
    repo_root: &Path,
    destination: &Path,
    display_path: &str,
) -> ExternalResult<()> {
    let relative = destination.strip_prefix(repo_root).map_err(|_| {
        ExternalError::with_code(
            "external_materializer_path",
            format!("external materialization path {display_path:?} is outside the repository"),
        )
    })?;

    let mut cursor = repo_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        cursor.push(part);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(ExternalError::with_code(
                    "external_materializer_symlink",
                    format!(
                        "external materialization path {display_path:?} crosses symlink {:?}",
                        cursor
                    ),
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(ExternalError::with_code(
                    "external_materializer_stat",
                    format!(
                        "failed to inspect external materialization path {display_path:?}: {err}"
                    ),
                ));
            }
        }
    }
    Ok(())
}
