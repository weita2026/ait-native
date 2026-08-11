use super::{
    prune_runtime_temp_namespace_json, validated_ci_ram_runtime_root_with_source,
    RuntimeTempPruneRequest, RUNTIME_SEQUENCE,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct CiRuntimePaths {
    pub workspace_path: PathBuf,
    pub output_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub rust_owned: bool,
}

pub(super) fn nonempty_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn ci_runtime_paths_from_request(
    request: &JsonMap<String, JsonValue>,
    kind: &str,
    key: &str,
) -> Result<CiRuntimePaths, String> {
    if let Some(workspace_path) = optional_path(request, "workspace_path") {
        let output_dir = optional_path(request, "output_dir")
            .unwrap_or_else(|| workspace_path.join(".ait/generated").join(kind));
        let temp_dir =
            optional_path(request, "temp_dir").unwrap_or_else(|| workspace_path.join(".tmp"));
        let rust_owned =
            if managed_runtime_manifest_matches(&workspace_path, &output_dir, &temp_dir, kind, key)
            {
                true
            } else if workspace_path
                .parent()
                .is_some_and(|base| !base.join("ci-runtime.json").exists())
            {
                validated_ci_ram_runtime_root_with_source()
                    .ok()
                    .map(|(root, _)| {
                        reinitialize_pruned_managed_runtime_paths(
                            request,
                            &root,
                            &workspace_path,
                            &output_dir,
                            &temp_dir,
                            kind,
                            key,
                        )
                    })
                    .transpose()?
                    .unwrap_or(false)
            } else {
                false
            };
        return Ok(CiRuntimePaths {
            workspace_path,
            output_dir,
            temp_dir,
            rust_owned,
        });
    }

    let scope_root = runtime_scope_root(request)?;
    let namespace_root = match explicit_namespace_root(request) {
        Some(root) => root,
        None => validated_ci_ram_runtime_root_with_source()?
            .0
            .join("ci-runs")
            .join(sanitize_segment(kind)),
    };
    fs::create_dir_all(&namespace_root).map_err(|exc| {
        format!(
            "Failed to create CI runtime temp namespace `{}`: {exc}",
            path_string(&namespace_root)
        )
    })?;
    let _ = prune_runtime_temp_namespace_json(&RuntimeTempPruneRequest::default_for_namespace(
        namespace_root.clone(),
    ));

    let base = namespace_root.join(format!(
        "ait-{kind}-{}-{}-pid{}-seq{}",
        sanitize_segment(key),
        unix_millis(),
        std::process::id(),
        RUNTIME_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let workspace_path = base.join("workspace");
    let output_dir = base.join("output");
    let temp_dir = workspace_path.join(".tmp");
    fs::create_dir_all(&temp_dir).map_err(|exc| {
        format!(
            "Failed to create CI runtime temp dir `{}`: {exc}",
            path_string(&temp_dir)
        )
    })?;
    fs::create_dir_all(&output_dir).map_err(|exc| {
        format!(
            "Failed to create CI runtime output dir `{}`: {exc}",
            path_string(&output_dir)
        )
    })?;
    write_manifest(
        &base,
        kind,
        key,
        &scope_root,
        &workspace_path,
        &output_dir,
        &temp_dir,
    )?;
    Ok(CiRuntimePaths {
        workspace_path,
        output_dir,
        temp_dir,
        rust_owned: true,
    })
}

pub(super) fn reinitialize_pruned_managed_runtime_paths(
    request: &JsonMap<String, JsonValue>,
    ram_runtime_root: &Path,
    workspace_path: &Path,
    output_dir: &Path,
    temp_dir: &Path,
    kind: &str,
    key: &str,
) -> Result<bool, String> {
    if workspace_path.file_name().and_then(|value| value.to_str()) != Some("workspace") {
        return Ok(false);
    }
    let Some(base) = workspace_path.parent() else {
        return Ok(false);
    };
    if output_dir != base.join("output") || temp_dir != workspace_path.join(".tmp") {
        return Ok(false);
    }
    let Some(base_name) = base.file_name().and_then(|value| value.to_str()) else {
        return Ok(false);
    };
    if !managed_runtime_base_name_matches(base_name, kind, key) {
        return Ok(false);
    }
    let expected_namespace = ram_runtime_root
        .join("ci-runs")
        .join(sanitize_segment(kind));
    let Some(namespace) = base.parent() else {
        return Ok(false);
    };
    let Ok(expected_namespace) = fs::canonicalize(&expected_namespace) else {
        return Ok(false);
    };
    let Ok(namespace) = fs::canonicalize(namespace) else {
        return Ok(false);
    };
    if namespace != expected_namespace {
        return Ok(false);
    }
    if fs::symlink_metadata(base)
        .ok()
        .is_some_and(|metadata| metadata.file_type().is_symlink())
    {
        return Ok(false);
    }
    fs::create_dir_all(temp_dir).map_err(|exc| {
        format!(
            "Failed to recreate pruned CI runtime temp dir `{}`: {exc}",
            path_string(temp_dir)
        )
    })?;
    fs::create_dir_all(output_dir).map_err(|exc| {
        format!(
            "Failed to recreate pruned CI runtime output dir `{}`: {exc}",
            path_string(output_dir)
        )
    })?;
    let scope_root = runtime_scope_root(request)?;
    write_manifest(
        base,
        kind,
        key,
        &scope_root,
        workspace_path,
        output_dir,
        temp_dir,
    )?;
    Ok(true)
}

fn managed_runtime_base_name_matches(name: &str, kind: &str, key: &str) -> bool {
    let prefix = format!("ait-{}-{}-", sanitize_segment(kind), sanitize_segment(key));
    let Some(suffix) = name.strip_prefix(&prefix) else {
        return false;
    };
    let Some((created_at, pid_and_sequence)) = suffix.rsplit_once("-pid") else {
        return false;
    };
    let Some((pid, sequence)) = pid_and_sequence.split_once("-seq") else {
        return false;
    };
    !created_at.is_empty()
        && created_at.chars().all(|value| value.is_ascii_digit())
        && !pid.is_empty()
        && pid.chars().all(|value| value.is_ascii_digit())
        && !sequence.is_empty()
        && sequence.chars().all(|value| value.is_ascii_digit())
}

fn managed_runtime_manifest_matches(
    workspace_path: &Path,
    output_dir: &Path,
    temp_dir: &Path,
    kind: &str,
    key: &str,
) -> bool {
    if workspace_path.file_name().and_then(|value| value.to_str()) != Some("workspace") {
        return false;
    }
    let Some(base) = workspace_path.parent() else {
        return false;
    };
    let Ok(bytes) = fs::read(base.join("ci-runtime.json")) else {
        return false;
    };
    let Ok(manifest) = serde_json::from_slice::<JsonValue>(&bytes) else {
        return false;
    };
    let Some(manifest) = manifest.as_object() else {
        return false;
    };
    manifest.get("contract").and_then(JsonValue::as_str) == Some("ait.server.ci_runtime_temp.v1")
        && manifest.get("kind").and_then(JsonValue::as_str) == Some(kind)
        && manifest.get("key").and_then(JsonValue::as_str) == Some(key)
        && manifest_path_matches(manifest, "workspace_path", workspace_path)
        && manifest_path_matches(manifest, "output_dir", output_dir)
        && manifest_path_matches(manifest, "temp_dir", temp_dir)
}

fn manifest_path_matches(
    manifest: &JsonMap<String, JsonValue>,
    key: &str,
    expected: &Path,
) -> bool {
    manifest.get(key).and_then(JsonValue::as_str).map(Path::new) == Some(expected)
}

fn runtime_scope_root(request: &JsonMap<String, JsonValue>) -> Result<PathBuf, String> {
    optional_path(request, "server_data_root")
        .or_else(|| optional_path(request, "runtime_scope_root"))
        .or_else(|| optional_path(request, "repo_root"))
        .or_else(|| env::current_dir().ok())
        .ok_or_else(|| {
            "CI runtime temp resolution requires server_data_root, runtime_scope_root, repo_root, or a current directory."
                .to_string()
        })
}

fn explicit_namespace_root(request: &JsonMap<String, JsonValue>) -> Option<PathBuf> {
    optional_path(request, "ci_temp_root").or_else(|| optional_path(request, "runtime_root"))
}

pub(super) fn detect_memory_root() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        for candidate in ["/Volumes/AIT_RAM", "/Volumes/RAMDisk", "/Volumes/RAM"] {
            let path = PathBuf::from(candidate);
            if path.exists() {
                return Some(path);
            }
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        let path = PathBuf::from("/dev/shm");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

fn sanitize_segment(value: &str) -> String {
    let normalized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if normalized.is_empty() {
        "runtime".to_string()
    } else {
        normalized
    }
}

fn write_manifest(
    base: &Path,
    kind: &str,
    key: &str,
    scope_root: &Path,
    workspace_path: &Path,
    output_dir: &Path,
    temp_dir: &Path,
) -> Result<(), String> {
    let manifest = json!({
        "contract": "ait.server.ci_runtime_temp.v1",
        "kind": kind,
        "key": key,
        "pid": std::process::id(),
        "created_at_millis": unix_millis(),
        "scope_root": path_string(scope_root),
        "workspace_path": path_string(workspace_path),
        "output_dir": path_string(output_dir),
        "temp_dir": path_string(temp_dir),
    });
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|exc| format!("Failed to encode CI runtime manifest: {exc}"))?;
    let manifest_path = base.join("ci-runtime.json");
    let pending_path = base.join(format!("ci-runtime.json.tmp-{}", std::process::id()));
    fs::write(&pending_path, bytes).map_err(|exc| {
        format!(
            "Failed to write pending CI runtime manifest `{}`: {exc}",
            path_string(&pending_path)
        )
    })?;
    fs::rename(&pending_path, &manifest_path).map_err(|exc| {
        format!(
            "Failed to activate CI runtime manifest `{}`: {exc}",
            path_string(&manifest_path)
        )
    })
}

fn optional_path(value: &JsonMap<String, JsonValue>, key: &str) -> Option<PathBuf> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(super) fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(super) fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
