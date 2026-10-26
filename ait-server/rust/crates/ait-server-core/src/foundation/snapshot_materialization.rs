use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

/// A final-tree entry. Symlink content is the link target, not its contents.
pub struct SnapshotTreeFile<'a> {
    pub path: &'a str,
    pub mode: &'a str,
    pub content: &'a [u8],
}

/// Reconcile a Server-owned source directory with the complete final manifest.
/// Package bytes and execution metadata must live outside this directory.
pub fn materialize_exact_snapshot_tree<'a>(
    destination: &Path,
    files: impl IntoIterator<Item = SnapshotTreeFile<'a>>,
) -> Result<(), String> {
    let mut entries = BTreeMap::new();
    let mut directories = BTreeSet::new();
    for file in files {
        let path = relative_path(file.path)?;
        let mode = tree_mode(file.mode)?;
        validate_link_content(mode, file.content)?;
        if entries.insert(path.clone(), file).is_some() {
            return Err(format!(
                "duplicate snapshot materialization path {}",
                path.display()
            ));
        }
        let mut parent = path.parent();
        while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
            directories.insert(path.to_path_buf());
            parent = path.parent();
        }
    }
    if let Some(path) = entries.keys().find(|path| directories.contains(*path)) {
        return Err(format!(
            "snapshot path {} is both a file and directory",
            path.display()
        ));
    }

    ensure_directory(destination)?;
    prune_tree(
        destination,
        Path::new(""),
        &entries.keys().cloned().collect(),
        &directories,
    )?;
    for file in entries.values() {
        write_snapshot_tree_file(destination, file.path, file.mode, file.content)?;
    }
    Ok(())
}

/// Prepare the exact path set before streaming pack members into the tree.
pub(crate) fn prepare_exact_snapshot_paths<'a>(
    destination: &Path,
    files: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<(), String> {
    let mut entries = BTreeSet::new();
    let mut directories = BTreeSet::new();
    for (path, mode) in files {
        tree_mode(mode)?;
        let path = relative_path(path)?;
        if !entries.insert(path.clone()) {
            return Err("duplicate snapshot materialization path".into());
        }
        let mut parent = path.parent();
        while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
            directories.insert(path.to_path_buf());
            parent = path.parent();
        }
    }
    if entries.iter().any(|path| directories.contains(path)) {
        return Err("snapshot path is both a file and directory".into());
    }
    ensure_directory(destination)?;
    prune_tree(destination, Path::new(""), &entries, &directories)
}

/// Common read-only representability check for original and tested files.
pub(crate) fn validate_snapshot_tree_file(
    path: &str,
    mode: &str,
    content: &[u8],
) -> Result<(), String> {
    relative_path(path)?;
    validate_link_content(tree_mode(mode)?, content)
}

/// Write one manifest entry without following a destination symlink.
/// This does not prune unrelated entries, so selected-path exports stay additive.
pub(crate) fn write_snapshot_tree_file(
    destination: &Path,
    path: &str,
    mode: &str,
    content: &[u8],
) -> Result<(), String> {
    let relative = relative_path(path)?;
    let mode = tree_mode(mode)?;
    validate_link_content(mode, content)?;
    ensure_directory(destination)?;
    let mut parent = destination.to_path_buf();
    if let Some(relative_parent) = relative.parent() {
        for component in relative_parent.components() {
            parent.push(component);
            ensure_directory(&parent)?;
        }
    }
    let output = destination.join(relative);
    match fs::symlink_metadata(&output) {
        Ok(metadata) if metadata.is_dir() => {
            return Err(format!(
                "snapshot output {} is a directory",
                output.display()
            ));
        }
        Ok(_) => fs::remove_file(&output)
            .map_err(|error| format!("remove snapshot output {}: {error}", output.display()))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "inspect snapshot output {}: {error}",
                output.display()
            ))
        }
    }
    if mode & 0o170000 == 0o120000 {
        return create_symlink(content, &output);
    }
    fs::write(&output, content)
        .map_err(|error| format!("write snapshot output {}: {error}", output.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&output, fs::Permissions::from_mode(mode & 0o7777))
            .map_err(|error| format!("set snapshot mode {}: {error}", output.display()))?;
    }
    Ok(())
}

fn relative_path(path: &str) -> Result<PathBuf, String> {
    let relative = PathBuf::from(path);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!(
            "snapshot materialized path {path:?} must be relative and stay inside its destination"
        ));
    }
    Ok(relative)
}

fn tree_mode(text: &str) -> Result<u32, String> {
    let mode = u32::from_str_radix(text.trim().trim_start_matches("0o"), 8)
        .map_err(|_| format!("invalid snapshot file mode {text:?}"))?;
    if mode & !0o177777 != 0 || !matches!(mode & 0o170000, 0 | 0o100000 | 0o120000) {
        return Err(format!("unsupported snapshot file mode {text:?}"));
    }
    Ok(mode)
}

fn validate_link_content(mode: u32, content: &[u8]) -> Result<(), String> {
    if mode & 0o170000 == 0o120000 && (content.is_empty() || content.contains(&0)) {
        return Err("snapshot symlink target must be nonempty and contain no NUL".to_string());
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(format!(
            "snapshot directory {} is not a real directory",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path)
            .map_err(|error| format!("create snapshot directory {}: {error}", path.display())),
        Err(error) => Err(format!(
            "inspect snapshot directory {}: {error}",
            path.display()
        )),
    }
}

fn prune_tree(
    root: &Path,
    relative: &Path,
    entries: &BTreeSet<PathBuf>,
    directories: &BTreeSet<PathBuf>,
) -> Result<(), String> {
    let directory = root.join(relative);
    for child in fs::read_dir(&directory)
        .map_err(|error| format!("read snapshot directory {}: {error}", directory.display()))?
    {
        let child = child.map_err(|error| format!("read snapshot directory entry: {error}"))?;
        let path = relative.join(child.file_name());
        let metadata = fs::symlink_metadata(child.path()).map_err(|error| {
            format!("inspect snapshot entry {}: {error}", child.path().display())
        })?;
        if directories.contains(&path) && metadata.is_dir() && !metadata.file_type().is_symlink() {
            prune_tree(root, &path, entries, directories)?;
        } else if !entries.contains(&path) || metadata.is_dir() {
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(child.path())
            } else {
                fs::remove_file(child.path())
            }
            .map_err(|error| {
                format!(
                    "remove obsolete snapshot entry {}: {error}",
                    child.path().display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn create_symlink(content: &[u8], output: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;
    std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(content), output)
        .map_err(|error| format!("create snapshot symlink {}: {error}", output.display()))
}

#[cfg(windows)]
fn create_symlink(content: &[u8], output: &Path) -> Result<(), String> {
    let target = std::str::from_utf8(content)
        .map_err(|error| format!("snapshot symlink target is not valid UTF-8: {error}"))?;
    std::os::windows::fs::symlink_file(target, output)
        .map_err(|error| format!("create snapshot symlink {}: {error}", output.display()))
}

#[cfg(not(any(unix, windows)))]
fn create_symlink(_: &[u8], _: &Path) -> Result<(), String> {
    Err("snapshot symlink materialization is unsupported on this platform".to_string())
}

#[cfg(test)]
#[path = "snapshot_materialization/tests.rs"]
mod tests;
