use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::path::{Component, Path, PathBuf};

use super::config::PrewarmConfig;
use super::helpers::{path_string, safe_path_segment};
use super::steps::PrewarmStep;

pub(super) fn verify_required_paths(
    config: &PrewarmConfig,
    seed_path: &Path,
) -> Result<Vec<JsonValue>, String> {
    let mut paths = Vec::new();
    for relative in &config.required_paths {
        let path = seed_path.join(relative);
        let exists = path.exists();
        if !exists {
            return Err(format!(
                "Required prewarm path `{}` is missing under main_seed `{}`.",
                path_string(relative),
                path_string(seed_path)
            ));
        }
        paths.push(json!({
            "relative_path": path_string(relative),
            "path": path_string(&path),
            "exists": exists
        }));
    }
    Ok(paths)
}

pub(super) fn verify_step_required_paths(
    step: &PrewarmStep,
    seed_path: &Path,
) -> Result<Vec<JsonValue>, String> {
    let mut paths = Vec::new();
    for relative in &step.required_paths {
        let path = seed_path.join(relative);
        if !path.exists() {
            return Err(format!(
                "Required prewarm step path `{}` is missing after step `{}`.",
                path_string(relative),
                step.step_id
            ));
        }
        paths.push(json!({
            "relative_path": path_string(relative),
            "path": path_string(&path),
            "exists": true
        }));
    }
    Ok(paths)
}

pub(super) fn relative_path_array_from_either(
    request: &JsonMap<String, JsonValue>,
    prewarm: Option<&JsonMap<String, JsonValue>>,
    key: &str,
) -> Result<Option<Vec<PathBuf>>, String> {
    match relative_path_array(request, key)? {
        Some(values) => Ok(Some(values)),
        None => match prewarm {
            Some(prewarm) => relative_path_array(prewarm, key),
            None => Ok(None),
        },
    }
}

pub(super) fn relative_path_array(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<Vec<PathBuf>>, String> {
    let Some(values) = value.get(key) else {
        return Ok(None);
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
    Ok(Some(paths))
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

pub(super) fn default_copy_excludes() -> Vec<PathBuf> {
    vec![
        PathBuf::from(".ait-worktree-links"),
        PathBuf::from(".ait/cargo-target"),
        PathBuf::from(".ait/generated/ci"),
        PathBuf::from("target"),
        PathBuf::from("node_modules"),
        PathBuf::from(".venv"),
        PathBuf::from("__pycache__"),
    ]
}

pub(super) fn is_excluded(relative: &Path, excludes: &[PathBuf]) -> bool {
    excludes
        .iter()
        .any(|exclude| relative == exclude || relative.starts_with(exclude))
}

pub(super) fn lock_path_for_seed(seed_path: &Path) -> PathBuf {
    let seed_name = seed_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("main-seed");
    seed_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".locks")
        .join(format!("{}.prewarm.lock", safe_path_segment(seed_name)))
}

pub(super) fn staging_path_for_seed(seed_path: &Path) -> PathBuf {
    let seed_name = seed_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("main-seed");
    seed_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(
            ".{}.prewarm-staging-{}",
            safe_path_segment(seed_name),
            std::process::id()
        ))
}
