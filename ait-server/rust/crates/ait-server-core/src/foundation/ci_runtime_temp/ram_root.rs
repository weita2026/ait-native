use super::{
    detect_memory_root, filesystem_available_bytes, filesystem_capacity_bytes, nonempty_env_path,
    path_string, prune_runtime_temp_namespace_json, reclaim_cargo_incremental_cache_with_available,
    RuntimeTempPruneRequest, CI_RAM_MIN_AVAILABLE_BYTES_ENV_NAMES,
    CI_RAM_RECLAIM_TARGET_BYTES_ENV_NAMES, CI_RUNTIME_PRESSURE_PRUNE_NAMESPACES,
    PERSISTENT_RUNTIME_ROOT_ENV_NAMES,
};
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub fn ci_ram_runtime_root_with_source() -> Result<(PathBuf, String), String> {
    for name in ["AIT_NATIVE_SERVER_CI_RAM_ROOT", "AIT_CI_RAM_ROOT"] {
        if let Some(path) = nonempty_env_path(name) {
            return Ok((path, name.to_string()));
        }
    }
    for name in ["AIT_NATIVE_SERVER_RAM_SHARD_ROOT", "AIT_RAM_SHARD_ROOT"] {
        if let Some(shard_root) = nonempty_env_path(name) {
            let runtime_root = shard_root.parent().map(Path::to_path_buf).ok_or_else(|| {
                format!("{name} must have a parent directory so the CI RAM runtime root can be derived.")
            })?;
            return Ok((runtime_root, format!("{name}/parent")));
        }
    }
    if let Some(memory_root) = detect_memory_root() {
        return Ok((
            memory_root.join("ait-runtime"),
            "detected_memory_mount/ait-runtime".to_string(),
        ));
    }
    Err(
        "CI requires a memory-backed runtime root. Set AIT_NATIVE_SERVER_CI_RAM_ROOT (preferred) or AIT_NATIVE_SERVER_RAM_SHARD_ROOT; no supported RAM mount was detected."
            .to_string(),
    )
}

pub fn validated_ci_ram_runtime_root_with_source() -> Result<(PathBuf, String), String> {
    let (root, source) = ci_ram_runtime_root_with_source()?;
    validate_ci_ram_runtime_root(root, source)
}

fn validate_ci_ram_runtime_root(
    root: PathBuf,
    source: String,
) -> Result<(PathBuf, String), String> {
    if !root.is_absolute() {
        return Err(format!(
            "CI RAM runtime root from {source} must be absolute: {}",
            path_string(&root)
        ));
    }
    let root = fs::canonicalize(&root).map_err(|exc| {
        format!(
            "CI RAM runtime root from {source} is not an existing mounted directory `{}`: {exc}",
            path_string(&root)
        )
    })?;
    let metadata = fs::metadata(&root).map_err(|exc| {
        format!(
            "Failed to inspect CI RAM runtime root from {source} `{}`: {exc}",
            path_string(&root)
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "CI RAM runtime root from {source} is not a directory: {}",
            path_string(&root)
        ));
    }
    if metadata.permissions().readonly() {
        return Err(format!(
            "CI RAM runtime root from {source} is read-only: {}",
            path_string(&root)
        ));
    }

    let persistent_roots = persistent_runtime_roots();
    validate_ci_ram_root_path_boundary(&root, &source, &persistent_roots)?;
    validate_ci_ram_root_device_boundary(&root, &source, &persistent_roots)?;
    validate_ci_ram_root_available_capacity(&root, &source)?;
    Ok((root, source))
}

fn persistent_runtime_roots() -> Vec<(String, PathBuf)> {
    PERSISTENT_RUNTIME_ROOT_ENV_NAMES
        .into_iter()
        .filter_map(|name| {
            nonempty_env_path(name).map(|path| {
                let resolved = fs::canonicalize(&path).unwrap_or(path);
                (name.to_string(), resolved)
            })
        })
        .collect()
}

pub(super) fn validate_ci_ram_root_path_boundary(
    root: &Path,
    source: &str,
    persistent_roots: &[(String, PathBuf)],
) -> Result<(), String> {
    for (name, persistent_root) in persistent_roots {
        if root == persistent_root || root.starts_with(persistent_root) {
            return Err(format!(
                "CI RAM runtime root from {source} resolves inside persistent authority {name}: {}",
                path_string(root)
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn validate_ci_ram_root_device_boundary(
    root: &Path,
    source: &str,
    persistent_roots: &[(String, PathBuf)],
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let candidate_device = fs::metadata(root)
        .map_err(|exc| format!("Failed to inspect CI RAM root device: {exc}"))?
        .dev();
    let system_device = fs::metadata("/")
        .map_err(|exc| format!("Failed to inspect system filesystem device: {exc}"))?
        .dev();
    if candidate_device == system_device {
        return Err(format!(
            "CI RAM runtime root from {source} is on the persistent system filesystem: {}",
            path_string(root)
        ));
    }
    for (name, persistent_root) in persistent_roots {
        let Ok(metadata) = fs::metadata(persistent_root) else {
            continue;
        };
        if metadata.dev() == candidate_device {
            return Err(format!(
                "CI RAM runtime root from {source} shares a filesystem device with persistent authority {name}: {}",
                path_string(root)
            ));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn validate_ci_ram_root_device_boundary(
    _root: &Path,
    _source: &str,
    _persistent_roots: &[(String, PathBuf)],
) -> Result<(), String> {
    Ok(())
}

fn validate_ci_ram_root_available_capacity(root: &Path, source: &str) -> Result<(), String> {
    let Some((name, raw_minimum)) = CI_RAM_MIN_AVAILABLE_BYTES_ENV_NAMES
        .into_iter()
        .find_map(|name| env::var(name).ok().map(|value| (name, value)))
    else {
        return Ok(());
    };
    let minimum = raw_minimum.trim().parse::<u64>().map_err(|_| {
        format!("{name} must be a non-negative integer byte count; got `{raw_minimum}`")
    })?;
    if minimum == 0 {
        return Ok(());
    }
    let capacity = filesystem_capacity_bytes(root)?;
    let (target, target_source) = ci_ram_reclaim_target_bytes(minimum, capacity.total_bytes)?;
    let mut available = capacity.available_bytes;
    let mut reclamation = JsonValue::Null;
    if available < target {
        reclamation = reclaim_ci_ram_capacity_for_admission(root, target)?;
        reclamation["minimum_available_bytes"] = json!(minimum);
        reclamation["admission_target_bytes"] = json!(target);
        reclamation["admission_target_source"] = json!(target_source);
        reclamation["filesystem_total_bytes"] = json!(capacity.total_bytes);
        available = filesystem_available_bytes(root)?;
        let reclaimed_runtime_bases = reclamation
            .pointer("/runtime_temp_pressure_prune/removed_run_base_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let reclaimed_incremental = reclamation
            .get("removed_incremental_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        let reclaimed_profiles = reclamation
            .get("removed_build_profile_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        if reclaimed_runtime_bases > 0 || reclaimed_incremental > 0 || reclaimed_profiles > 0 {
            eprintln!(
                "ait CI RAM capacity reclaimed managed runtime or Cargo build cache: {}",
                reclamation
            );
        }
    }
    if available < target {
        return Err(format!(
            "CI RAM runtime root from {source} has {available} available bytes after safe managed-runtime and Cargo cache reclamation, below admission target {target} ({target_source}; minimum {name}={minimum}): {}; reclamation={reclamation}",
            path_string(root),
        ));
    }
    Ok(())
}

fn ci_ram_reclaim_target_bytes(
    minimum: u64,
    _filesystem_total_bytes: u64,
) -> Result<(u64, String), String> {
    let Some((name, raw_target)) = CI_RAM_RECLAIM_TARGET_BYTES_ENV_NAMES
        .into_iter()
        .find_map(|name| env::var(name).ok().map(|value| (name, value)))
    else {
        return Ok((
            default_ci_ram_reclaim_target_bytes(minimum),
            "derived:configured_minimum".to_string(),
        ));
    };
    let target = raw_target.trim().parse::<u64>().map_err(|_| {
        format!("{name} must be a non-negative integer byte count; got `{raw_target}`")
    })?;
    Ok((target.max(minimum), format!("explicit:{name}")))
}

pub(super) fn default_ci_ram_reclaim_target_bytes(minimum: u64) -> u64 {
    minimum
}

fn reclaim_ci_ram_capacity_for_admission(
    ram_runtime_root: &Path,
    target_available_bytes: u64,
) -> Result<JsonValue, String> {
    reclaim_ci_ram_capacity_with_available(
        ram_runtime_root,
        target_available_bytes,
        filesystem_available_bytes,
    )
}

pub(super) fn reclaim_ci_ram_capacity_with_available<F>(
    ram_runtime_root: &Path,
    target_available_bytes: u64,
    mut available_bytes: F,
) -> Result<JsonValue, String>
where
    F: FnMut(&Path) -> Result<u64, String>,
{
    let available_before = available_bytes(ram_runtime_root)?;
    let runtime_temp_pressure_prune =
        pressure_prune_completed_ci_runtime_namespaces(ram_runtime_root);
    let available_after_runtime_temp_prune = available_bytes(ram_runtime_root)?;
    let mut reclamation = reclaim_cargo_incremental_cache_with_available(
        ram_runtime_root,
        target_available_bytes,
        |path| available_bytes(path),
    )?;
    reclamation["available_before_admission_reclamation"] = json!(available_before);
    reclamation["available_after_runtime_temp_prune"] = json!(available_after_runtime_temp_prune);
    reclamation["runtime_temp_pressure_prune"] = runtime_temp_pressure_prune;
    reclamation["reclamation_order"] = json!([
        "completed_managed_ci_runs",
        "cargo_incremental",
        "idle_cargo_build_profiles"
    ]);
    Ok(reclamation)
}

fn pressure_prune_completed_ci_runtime_namespaces(ram_runtime_root: &Path) -> JsonValue {
    let ci_runs_root = ram_runtime_root.join("ci-runs");
    let mut namespaces = Vec::<JsonValue>::new();
    let mut removed_run_base_count = 0_u64;
    let mut error_count = 0_u64;
    for namespace in CI_RUNTIME_PRESSURE_PRUNE_NAMESPACES {
        let namespace_root = ci_runs_root.join(namespace);
        let mut request = RuntimeTempPruneRequest::default_for_namespace(namespace_root.clone());
        request.completed_run_base_retention_seconds = 0;
        request.manifest_owned_only = true;
        match prune_runtime_temp_namespace_json(&request) {
            Ok(result) => {
                removed_run_base_count = removed_run_base_count.saturating_add(
                    result
                        .get("removed_completed")
                        .and_then(JsonValue::as_array)
                        .map(|values| values.len() as u64)
                        .unwrap_or(0),
                );
                namespaces.push(json!({
                    "namespace": namespace,
                    "result": result,
                }));
            }
            Err(error) => {
                error_count = error_count.saturating_add(1);
                namespaces.push(json!({
                    "namespace": namespace,
                    "namespace_root": path_string(&namespace_root),
                    "error": error,
                }));
            }
        }
    }
    json!({
        "contract": "ait.server.ci_runtime_pressure_prune.v1",
        "status": if error_count == 0 { "cleaned" } else { "partial" },
        "policy": "completed_manifest_owned_run_bases_only",
        "removed_run_base_count": removed_run_base_count,
        "error_count": error_count,
        "namespaces": namespaces,
    })
}
