use serde_json::{Map as JsonMap, Value as JsonValue};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(super) fn path_is_copy_up_or_child(path: &Path, copy_up_paths: &[PathBuf]) -> bool {
    copy_up_paths
        .iter()
        .any(|copy_up| path == copy_up || path.starts_with(copy_up))
}

pub(super) fn remove_path_if_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            remove_existing_with_metadata(path, &metadata)?;
            Ok(true)
        }
        Err(exc) if exc.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(exc) => Err(format!(
            "Failed to inspect path `{}` before removal: {exc}",
            path_string(path)
        )),
    }
}

pub(super) fn remove_existing_with_metadata(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), String> {
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)
            .map_err(|exc| format!("Failed to remove file `{}`: {exc}", path_string(path)))
    } else {
        fs::remove_dir_all(path)
            .map_err(|exc| format!("Failed to remove directory `{}`: {exc}", path_string(path)))
    }
}

#[cfg(unix)]
pub(super) fn create_symlink(source: &Path, destination: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(source, destination).map_err(|exc| {
        format!(
            "Failed to symlink immutable path `{}` to `{}`: {exc}",
            path_string(source),
            path_string(destination)
        )
    })
}

#[cfg(not(unix))]
pub(super) fn create_symlink(source: &Path, destination: &Path) -> Result<(), String> {
    if source.is_dir() {
        fs::create_dir_all(destination).map_err(|exc| {
            format!(
                "Failed to create immutable directory fallback `{}`: {exc}",
                path_string(destination)
            )
        })
    } else {
        fs::copy(source, destination).map(|_| ()).map_err(|exc| {
            format!(
                "Failed to copy immutable file fallback `{}` to `{}`: {exc}",
                path_string(source),
                path_string(destination)
            )
        })
    }
}

pub(super) fn write_json_file(path: &Path, value: &JsonValue) -> Result<(), String> {
    let content = serde_json::to_string_pretty(value)
        .map_err(|exc| format!("Failed to encode JSON for `{}`: {exc}", path_string(path)))?;
    fs::write(path, format!("{content}\n"))
        .map_err(|exc| format!("Failed to write `{}`: {exc}", path_string(path)))
}

pub(super) fn platform_name(request: &JsonMap<String, JsonValue>) -> String {
    optional_text(request, "platform")
        .map(|value| normalize_platform(&value))
        .unwrap_or_else(|| normalize_platform(std::env::consts::OS))
}

pub(super) fn normalize_platform(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "darwin" | "mac" | "macos" => "macos".to_string(),
        "linux" => "linux".to_string(),
        other => other.to_string(),
    }
}

pub(super) fn relative_path_array(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<PathBuf>, String> {
    let Some(values) = value.get(key) else {
        return Ok(Vec::new());
    };
    let values = values
        .as_array()
        .ok_or_else(|| format!("Field `{key}` must be an array of relative path strings."))?;
    let mut paths = Vec::new();
    for value in values {
        let raw = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Field `{key}` must contain non-empty strings."))?;
        paths.push(validate_relative_path(raw, key)?);
    }
    Ok(paths)
}

pub(super) fn validate_relative_path(value: &str, key: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Err(format!(
            "Field `{key}` must not contain absolute path `{value}`."
        ));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "Field `{key}` must not contain path traversal segment `{value}`."
        ));
    }
    Ok(path)
}

pub(super) fn path_from_json(value: &JsonValue, field: &str) -> Result<PathBuf, String> {
    value
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| format!("Field `{field}` must be a string path."))
}

pub(super) fn required_text(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<String, String> {
    optional_text(value, key).ok_or_else(|| format!("Field `{key}` must be a non-empty string."))
}

pub(super) fn optional_text(value: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn optional_bool(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<bool>, String> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("Field `{key}` must be a boolean.")),
    }
}

pub(super) fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
