use super::*;

pub(super) fn canonical_root(repo_root: &str) -> Result<PathBuf, PlanFilesystemError> {
    let root = expanduser_path(repo_root);
    root.canonicalize()
        .map_err(|err| io_error_for_path("resolve repository root", &root, err))
}

pub(super) fn expanduser_path(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

pub(super) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => output.push(prefix.as_os_str()),
            Component::RootDir => output.push(Path::new("/")),
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            Component::Normal(part) => output.push(part),
        }
    }
    output
}

pub(super) fn resolve_path_strict_false(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    if let Ok(canonical) = normalized.canonicalize() {
        return canonical;
    }
    let mut cursor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        if cursor.exists() {
            if let Ok(canonical_parent) = cursor.canonicalize() {
                let mut resolved = canonical_parent;
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return lexical_normalize(&resolved);
            }
        }
        let Some(file_name) = cursor.file_name() else {
            return normalized;
        };
        missing.push(file_name.to_os_string());
        let Some(parent) = cursor.parent() else {
            return normalized;
        };
        cursor = parent;
    }
}

pub(super) fn normalized_relative_path(
    root: &Path,
    path_value: &str,
) -> Result<PathBuf, PlanFilesystemError> {
    let raw = expanduser_path(path_value);
    let rel_path = if raw.is_absolute() {
        let normalized = resolve_path_strict_false(&raw);
        normalized
            .strip_prefix(root)
            .map_err(|_| {
                PlanFilesystemError::Invalid(format!(
                    "Path must live inside the repository root: {}",
                    path_value
                ))
            })?
            .to_path_buf()
    } else {
        PathBuf::from(path_value)
    };
    Ok(PathBuf::from(rel_path.to_string_lossy().replace('\\', "/")))
}

pub(super) fn normalized_runtime_root(
    root: &Path,
    runtime_root: &str,
) -> Result<PathBuf, PlanFilesystemError> {
    let raw = expanduser_path(runtime_root);
    let resolved = if raw.is_absolute() {
        resolve_path_strict_false(&raw)
    } else {
        resolve_path_strict_false(&root.join(raw))
    };
    if !resolved.starts_with(root) {
        return Ok(resolved);
    }
    Ok(resolved)
}

pub(super) fn io_error_for_path(
    action: &str,
    path: &Path,
    err: std::io::Error,
) -> PlanFilesystemError {
    match err.kind() {
        std::io::ErrorKind::NotFound => {
            PlanFilesystemError::NotFound(format!("Failed to {}: {}", action, path.display()))
        }
        _ => PlanFilesystemError::Io(format!("Failed to {} {}: {}", action, path.display(), err)),
    }
}

pub(super) fn file_io_error_for_path(
    action: &str,
    path: &Path,
    err: FileIoError,
) -> PlanFilesystemError {
    match err.kind() {
        FileIoErrorKind::NotFound => {
            PlanFilesystemError::NotFound(format!("Failed to {}: {}", action, path.display()))
        }
        _ => PlanFilesystemError::Io(format!("Failed to {} {}: {}", action, path.display(), err)),
    }
}
