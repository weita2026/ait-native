use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_COMPLETED_RUN_BASE_RETENTION_SECONDS: u64 = 6 * 60 * 60;
pub const DEFAULT_ABANDONED_RUN_BASE_RETENTION_SECONDS: u64 = 15 * 60;

#[derive(Debug, Clone)]
pub struct RuntimeTempPruneRequest {
    pub namespace_root: PathBuf,
    pub now_millis: Option<u128>,
    pub completed_run_base_retention_seconds: u64,
    pub abandoned_run_base_retention_seconds: u64,
    pub manifest_owned_only: bool,
    pub dry_run: bool,
}

impl RuntimeTempPruneRequest {
    pub fn default_for_namespace(namespace_root: PathBuf) -> Self {
        Self {
            namespace_root,
            now_millis: None,
            completed_run_base_retention_seconds: DEFAULT_COMPLETED_RUN_BASE_RETENTION_SECONDS,
            abandoned_run_base_retention_seconds: DEFAULT_ABANDONED_RUN_BASE_RETENTION_SECONDS,
            manifest_owned_only: false,
            dry_run: false,
        }
    }
}

pub fn cleanup_runtime_workspace(kind: &str, workspace_path: &Path) -> Result<JsonValue, String> {
    if let Some(run_base) = runtime_base_for_workspace(workspace_path) {
        let namespace_root = run_base.parent().map(Path::to_path_buf);
        let removed = run_base.exists();
        if removed {
            remove_tree(kind, &run_base, "managed run base")?;
        }
        return Ok(json!({
            "status": "cleaned",
            "workspace_path": path_string(workspace_path),
            "run_base_path": path_string(&run_base),
            "namespace_root": namespace_root.as_ref().map(|path| path_string(path)),
            "removed": removed,
            "removed_scope": "managed_run_base",
            "run_base_policy": "remove_on_terminal_state",
        }));
    }
    let removed = workspace_path.exists();
    if removed {
        remove_tree(kind, workspace_path, "workspace")?;
    }
    Ok(json!({
        "status": "cleaned",
        "workspace_path": path_string(workspace_path),
        "removed": removed,
        "removed_scope": "external_workspace",
    }))
}

pub fn finalize_runtime_workspace_cleanup(
    kind: &str,
    workspace_path: &Path,
    cleanup_workspace: bool,
    result: Result<JsonValue, String>,
) -> Result<JsonValue, String> {
    if !cleanup_workspace {
        return result;
    }
    let cleanup = cleanup_runtime_workspace(kind, workspace_path);
    match (result, cleanup) {
        (Ok(mut value), Ok(cleanup_value)) => {
            value["cleanup"] = attach_runtime_temp_prune(cleanup_value, workspace_path);
            Ok(value)
        }
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(run_error), Ok(_)) => Err(run_error),
        (Err(run_error), Err(cleanup_error)) => Err(format!(
            "{run_error}; additionally failed to cleanup {kind} CI workspace `{}`: {cleanup_error}",
            path_string(workspace_path)
        )),
    }
}

pub fn prune_runtime_temp_namespace_json(
    request: &RuntimeTempPruneRequest,
) -> Result<JsonValue, String> {
    let namespace_root = &request.namespace_root;
    let now_millis = request.now_millis.unwrap_or_else(unix_millis);
    let completed_retention_millis =
        seconds_to_millis(request.completed_run_base_retention_seconds);
    let abandoned_retention_millis =
        seconds_to_millis(request.abandoned_run_base_retention_seconds);
    let mut removed_completed = Vec::<String>::new();
    let mut removed_abandoned = Vec::<String>::new();
    let mut kept_recent = Vec::<String>::new();
    let mut kept_active = Vec::<String>::new();
    let mut skipped_unmanaged = Vec::<String>::new();
    if !namespace_root.exists() {
        return Ok(json!({
            "status": "cleaned",
            "namespace_root": path_string(namespace_root),
            "dry_run": request.dry_run,
            "manifest_owned_only": request.manifest_owned_only,
            "removed_completed": removed_completed,
            "removed_abandoned": removed_abandoned,
            "kept_recent": kept_recent,
            "kept_active": kept_active,
            "skipped_unmanaged": skipped_unmanaged,
        }));
    }
    let mut children = fs::read_dir(namespace_root)
        .map_err(|exc| {
            format!(
                "Failed to read CI runtime temp namespace `{}`: {exc}",
                path_string(namespace_root)
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        let name = child
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_string();
        let Some(manifest) = load_ci_runtime_manifest(&child.join("ci-runtime.json")) else {
            if request.manifest_owned_only {
                skipped_unmanaged.push(name);
                continue;
            }
            if !legacy_runtime_base_name_matches(namespace_root, &name) {
                if legacy_snapshot_materialization_group(namespace_root, &child)
                    && now_millis.saturating_sub(path_mtime_millis(&child).unwrap_or(now_millis))
                        >= completed_retention_millis
                {
                    if !request.dry_run {
                        remove_tree("runtime", &child, "legacy snapshot materialization group")?;
                    }
                    removed_completed.push(name);
                    continue;
                }
                skipped_unmanaged.push(name);
                continue;
            }
            let created_at = path_mtime_millis(&child).unwrap_or(now_millis);
            let age_millis = now_millis.saturating_sub(created_at);
            let has_active_materialization = child.join("workspace").exists()
                || child.join("seed-source").exists()
                || child.join("revision-source").exists();
            if has_active_materialization && legacy_runtime_base_pid(&name).is_some_and(pid_is_live)
            {
                kept_active.push(name);
                continue;
            }
            if has_active_materialization {
                if age_millis >= abandoned_retention_millis {
                    if !request.dry_run {
                        remove_tree("runtime", &child, "legacy abandoned run base")?;
                    }
                    removed_abandoned.push(name);
                } else {
                    kept_recent.push(name);
                }
                continue;
            }
            if age_millis >= completed_retention_millis {
                if !request.dry_run {
                    remove_tree("runtime", &child, "legacy completed run base")?;
                }
                removed_completed.push(name);
            } else {
                kept_recent.push(name);
            }
            continue;
        };
        let created_at = manifest
            .get("created_at_millis")
            .and_then(json_u128)
            .or_else(|| path_mtime_millis(&child).ok())
            .unwrap_or(now_millis);
        let age_millis = now_millis.saturating_sub(created_at);
        let workspace_path =
            manifest_path(&manifest, "workspace_path").unwrap_or_else(|| child.join("workspace"));
        let has_active_materialization = workspace_path.exists()
            || child.join("seed-source").exists()
            || child.join("revision-source").exists();
        if has_active_materialization && manifest_pid_is_live(&manifest) {
            kept_active.push(name);
            continue;
        }
        if has_active_materialization {
            if age_millis >= abandoned_retention_millis {
                if !request.dry_run {
                    remove_tree("runtime", &child, "abandoned run base")?;
                }
                removed_abandoned.push(name);
            } else {
                kept_recent.push(name);
            }
            continue;
        }
        if age_millis >= completed_retention_millis {
            if !request.dry_run {
                remove_tree("runtime", &child, "completed run base")?;
            }
            removed_completed.push(name);
        } else {
            kept_recent.push(name);
        }
    }
    Ok(json!({
        "status": "cleaned",
        "namespace_root": path_string(namespace_root),
        "dry_run": request.dry_run,
        "manifest_owned_only": request.manifest_owned_only,
        "completed_run_base_retention_seconds": request.completed_run_base_retention_seconds,
        "abandoned_run_base_retention_seconds": request.abandoned_run_base_retention_seconds,
        "removed_completed": removed_completed,
        "removed_abandoned": removed_abandoned,
        "kept_recent": kept_recent,
        "kept_active": kept_active,
        "skipped_unmanaged": skipped_unmanaged,
    }))
}

fn attach_runtime_temp_prune(mut cleanup_value: JsonValue, workspace_path: &Path) -> JsonValue {
    let namespace_root = cleanup_value
        .get("namespace_root")
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .or_else(|| {
            runtime_base_for_workspace(workspace_path)
                .and_then(|base| base.parent().map(Path::to_path_buf))
        });
    let Some(namespace_root) = namespace_root else {
        return cleanup_value;
    };
    match prune_runtime_temp_namespace_json(&RuntimeTempPruneRequest::default_for_namespace(
        namespace_root,
    )) {
        Ok(prune) => cleanup_value["runtime_temp_prune"] = prune,
        Err(error) => {
            cleanup_value["runtime_temp_prune"] = json!({
                "status": "error",
                "error": error,
            });
        }
    }
    cleanup_value
}

fn runtime_base_for_workspace(workspace_path: &Path) -> Option<PathBuf> {
    if workspace_path.file_name().and_then(|value| value.to_str()) != Some("workspace") {
        return None;
    }
    let base = workspace_path.parent()?.to_path_buf();
    let manifest = load_ci_runtime_manifest(&base.join("ci-runtime.json"))?;
    (manifest_path(&manifest, "workspace_path").as_deref() == Some(workspace_path)).then_some(base)
}

fn legacy_runtime_base_name_matches(namespace_root: &Path, name: &str) -> bool {
    let Some(namespace) = namespace_root.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    name.starts_with(&format!("ait-{namespace}-"))
}

fn legacy_snapshot_materialization_group(namespace_root: &Path, child: &Path) -> bool {
    namespace_root.file_name().and_then(|value| value.to_str()) == Some("snapshot-materialize")
        && child.read_dir().ok().is_some_and(|mut entries| {
            entries.any(|entry| entry.is_ok_and(|entry| entry.path().is_dir()))
        })
}

fn legacy_runtime_base_pid(name: &str) -> Option<u32> {
    name.split('-')
        .find_map(|value| value.strip_prefix("pid")?.parse::<u32>().ok())
        .or_else(|| name.rsplit('-').find_map(|value| value.parse::<u32>().ok()))
}

fn load_ci_runtime_manifest(path: &Path) -> Option<JsonMap<String, JsonValue>> {
    let bytes = fs::read(path).ok()?;
    let value = serde_json::from_slice::<JsonValue>(&bytes).ok()?;
    let object = value.as_object()?;
    if object.get("contract").and_then(JsonValue::as_str) != Some("ait.server.ci_runtime_temp.v1") {
        return None;
    }
    Some(object.clone())
}

fn manifest_path(manifest: &JsonMap<String, JsonValue>, key: &str) -> Option<PathBuf> {
    manifest
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn manifest_pid_is_live(manifest: &JsonMap<String, JsonValue>) -> bool {
    manifest
        .get("pid")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .map(pid_is_live)
        .unwrap_or(false)
}

fn pid_is_live(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn remove_tree(kind: &str, path: &Path, noun: &str) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            make_tree_removable(path).map_err(|cleanup_error| {
                format!(
                    "Failed to prepare {kind} CI {noun} `{}` for cleanup after remove_dir_all failed with {first_error}: {cleanup_error}",
                    path_string(path)
                )
            })?;
            fs::remove_dir_all(path).map_err(|second_error| {
                format!(
                    "Failed to cleanup {kind} CI {noun} `{}` after making it removable: {second_error}; initial error: {first_error}",
                    path_string(path)
                )
            })
        }
    }
}

fn make_tree_removable(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|exc| {
        format!(
            "Failed to inspect cleanup path `{}`: {exc}",
            path_string(path)
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    ensure_cleanup_permissions(path, &metadata)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path).map_err(|exc| {
            format!(
                "Failed to read cleanup directory `{}`: {exc}",
                path_string(path)
            )
        })? {
            let entry = entry.map_err(|exc| {
                format!(
                    "Failed to inspect cleanup directory entry under `{}`: {exc}",
                    path_string(path)
                )
            })?;
            make_tree_removable(&entry.path())?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_cleanup_permissions(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let required = if metadata.is_dir() { 0o700 } else { 0o600 };
    let mode = metadata.permissions().mode();
    if mode & required == required {
        return Ok(());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode | required)).map_err(|exc| {
        format!(
            "Failed to make cleanup path `{}` writable: {exc}",
            path_string(path)
        )
    })
}

#[cfg(not(unix))]
fn ensure_cleanup_permissions(path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
    let mut permissions = metadata.permissions();
    if !permissions.readonly() {
        return Ok(());
    }
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).map_err(|exc| {
        format!(
            "Failed to make cleanup path `{}` writable: {exc}",
            path_string(path)
        )
    })
}

fn json_u128(value: &JsonValue) -> Option<u128> {
    value
        .as_u64()
        .map(u128::from)
        .or_else(|| value.as_str()?.parse::<u128>().ok())
}

fn path_mtime_millis(path: &Path) -> Result<u128, String> {
    let metadata = fs::metadata(path).map_err(|exc| {
        format!(
            "Failed to inspect cleanup path `{}` mtime: {exc}",
            path_string(path)
        )
    })?;
    let modified = metadata.modified().map_err(|exc| {
        format!(
            "Failed to read cleanup path `{}` mtime: {exc}",
            path_string(path)
        )
    })?;
    Ok(modified
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis())
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn seconds_to_millis(seconds: u64) -> u128 {
    u128::from(seconds).saturating_mul(1000)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn temp_root(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "ait-server-ci-workspace-cleanup-{label}-{}-{}",
            std::process::id(),
            unix_millis()
        ))
    }

    fn write_manifest(base: &Path, workspace: &Path, created_at_millis: u128, pid: Option<u32>) {
        let mut manifest = JsonMap::new();
        manifest.insert(
            "contract".to_string(),
            json!("ait.server.ci_runtime_temp.v1"),
        );
        manifest.insert("kind".to_string(), json!("repo-ci"));
        manifest.insert("key".to_string(), json!("ait"));
        manifest.insert("created_at_millis".to_string(), json!(created_at_millis));
        manifest.insert("workspace_path".to_string(), json!(path_string(workspace)));
        manifest.insert(
            "output_dir".to_string(),
            json!(path_string(&base.join("output"))),
        );
        manifest.insert(
            "temp_dir".to_string(),
            json!(path_string(&workspace.join(".tmp"))),
        );
        if let Some(pid) = pid {
            manifest.insert("pid".to_string(), json!(pid));
        }
        fs::write(
            base.join("ci-runtime.json"),
            serde_json::to_vec_pretty(&JsonValue::Object(manifest)).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn finalize_cleanup_removes_current_managed_run_base_and_prunes_old_completed_run_bases() {
        let root = temp_root("completed");
        let namespace = root.join("namespace");
        let stale_base = namespace.join("ait-repo-ci-old-0-1");
        let current_base = namespace.join("ait-repo-ci-current-0-1");
        let stale_workspace = stale_base.join("workspace");
        let current_workspace = current_base.join("workspace");
        fs::create_dir_all(stale_base.join("output")).unwrap();
        fs::write(stale_base.join("output/log.txt"), "old").unwrap();
        write_manifest(&stale_base, &stale_workspace, 0, None);
        fs::create_dir_all(current_workspace.join(".tmp")).unwrap();
        fs::create_dir_all(current_base.join("output")).unwrap();
        fs::write(current_base.join("output/log.txt"), "current").unwrap();
        write_manifest(
            &current_base,
            &current_workspace,
            unix_millis(),
            Some(std::process::id()),
        );

        let value = finalize_runtime_workspace_cleanup(
            "repo",
            &current_workspace,
            true,
            Ok(json!({"status": "pass"})),
        )
        .unwrap();

        assert!(!stale_base.exists());
        assert!(!current_base.exists());
        assert_eq!(value["cleanup"]["removed_scope"], json!("managed_run_base"));
        assert_eq!(
            value["cleanup"]["runtime_temp_prune"]["removed_completed"],
            json!(["ait-repo-ci-old-0-1"])
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prune_removes_old_abandoned_workspace_but_keeps_live_workspace() {
        let root = temp_root("abandoned");
        let namespace = root.join("namespace");
        let abandoned_base = namespace.join("ait-repo-ci-abandoned-0-1");
        let live_base = namespace.join("ait-repo-ci-live-0-1");
        let abandoned_workspace = abandoned_base.join("workspace");
        let live_workspace = live_base.join("workspace");
        fs::create_dir_all(abandoned_workspace.join(".tmp")).unwrap();
        fs::create_dir_all(abandoned_base.join("output")).unwrap();
        fs::create_dir_all(live_workspace.join(".tmp")).unwrap();
        fs::create_dir_all(live_base.join("output")).unwrap();
        write_manifest(&abandoned_base, &abandoned_workspace, 0, None);
        write_manifest(&live_base, &live_workspace, 0, Some(std::process::id()));

        let value = prune_runtime_temp_namespace_json(&RuntimeTempPruneRequest {
            namespace_root: namespace.clone(),
            now_millis: Some(seconds_to_millis(60)),
            completed_run_base_retention_seconds: 60 * 60,
            abandoned_run_base_retention_seconds: 10,
            manifest_owned_only: false,
            dry_run: false,
        })
        .unwrap();

        assert!(!abandoned_base.exists());
        assert!(live_workspace.exists());
        assert_eq!(
            value["removed_abandoned"],
            json!(["ait-repo-ci-abandoned-0-1"])
        );
        assert_eq!(value["kept_active"], json!(["ait-repo-ci-live-0-1"]));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn managed_revision_sources_use_abandoned_retention_and_live_pid_guard() {
        let root = temp_root("managed-revision-source");
        let namespace = root.join("namespace");
        let abandoned_base = namespace.join("ait-patchset-ci-abandoned-0-1");
        let live_base = namespace.join("ait-patchset-ci-live-0-1");
        let abandoned_workspace = abandoned_base.join("workspace");
        let live_workspace = live_base.join("workspace");
        fs::create_dir_all(abandoned_base.join("revision-source")).unwrap();
        fs::create_dir_all(live_base.join("revision-source")).unwrap();
        write_manifest(&abandoned_base, &abandoned_workspace, 0, None);
        write_manifest(&live_base, &live_workspace, 0, Some(std::process::id()));

        let value = prune_runtime_temp_namespace_json(&RuntimeTempPruneRequest {
            namespace_root: namespace,
            now_millis: Some(seconds_to_millis(60)),
            completed_run_base_retention_seconds: 60 * 60,
            abandoned_run_base_retention_seconds: 10,
            manifest_owned_only: false,
            dry_run: false,
        })
        .unwrap();

        assert!(!abandoned_base.exists());
        assert!(live_base.exists());
        assert_eq!(
            value["removed_abandoned"],
            json!(["ait-patchset-ci-abandoned-0-1"])
        );
        assert_eq!(value["kept_active"], json!(["ait-patchset-ci-live-0-1"]));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prune_adopts_matching_legacy_run_bases_but_skips_unrelated_directories() {
        let root = temp_root("legacy");
        let namespace = root.join("repo-ci");
        let legacy = namespace.join("ait-repo-ci-old-0-424242");
        let unrelated = namespace.join("keep-me");
        fs::create_dir_all(legacy.join("output")).unwrap();
        fs::create_dir_all(&unrelated).unwrap();

        let value = prune_runtime_temp_namespace_json(&RuntimeTempPruneRequest {
            namespace_root: namespace.clone(),
            now_millis: Some(unix_millis().saturating_add(seconds_to_millis(60))),
            completed_run_base_retention_seconds: 0,
            abandoned_run_base_retention_seconds: 0,
            manifest_owned_only: false,
            dry_run: false,
        })
        .unwrap();

        assert!(!legacy.exists());
        assert!(unrelated.exists());
        assert_eq!(
            value["removed_completed"],
            json!(["ait-repo-ci-old-0-424242"])
        );
        assert_eq!(value["skipped_unmanaged"], json!(["keep-me"]));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn prune_adopts_old_nested_snapshot_materialization_groups() {
        let root = temp_root("legacy-snapshot-materialization");
        let namespace = root.join("snapshot-materialize");
        let legacy_repo_group = namespace.join("ait");
        fs::create_dir_all(legacy_repo_group.join("snp-old-sm-old")).unwrap();

        let value = prune_runtime_temp_namespace_json(&RuntimeTempPruneRequest {
            namespace_root: namespace,
            now_millis: Some(unix_millis().saturating_add(seconds_to_millis(60))),
            completed_run_base_retention_seconds: 0,
            abandoned_run_base_retention_seconds: 0,
            manifest_owned_only: false,
            dry_run: false,
        })
        .unwrap();

        assert!(!legacy_repo_group.exists());
        assert_eq!(value["removed_completed"], json!(["ait"]));

        let _ = fs::remove_dir_all(root);
    }
}
