use super::*;

pub fn prune_current_source_native_caches_json(
    request: &CurrentSourceNativeCachePruneRequest,
) -> Result<JsonValue, String> {
    let store = FilesystemCurrentSourceNativeCacheLeaseStore;
    prune_current_source_native_caches_with_lease_store(&store, request)
}

pub(super) fn prune_current_source_native_caches_with_lease_store<S>(
    store: &S,
    request: &CurrentSourceNativeCachePruneRequest,
) -> Result<JsonValue, String>
where
    S: CurrentSourceNativeCacheLeaseStore + ?Sized,
{
    let _prune_range = crate::perfetto_range!("ait.core.current_source_cache.prune");
    let root =
        resolve_path_strict_false(&request.namespace_root).join(CURRENT_SOURCE_CACHE_NAMESPACE);
    let current_time = request.now.unwrap_or_else(now_seconds);
    let mut removed_idle = Vec::<String>::new();
    let mut removed_abandoned_builds = Vec::<String>::new();
    let mut removed_budget = Vec::<String>::new();
    let mut removed_unleased_ready = Vec::<String>::new();
    let mut retained = Vec::<(f64, u64, PathBuf)>::new();
    if !root.exists() {
        return Ok(json!({
            "removed_idle": removed_idle,
            "removed_abandoned_builds": removed_abandoned_builds,
            "removed_budget": removed_budget,
            "removed_unleased_ready": removed_unleased_ready,
            "total_bytes": 0u64,
        }));
    }
    let mut build_dirs = fs::read_dir(&root)
        .map_err(|err| {
            format!(
                "Failed to read current-source cache root {}: {err}",
                root.display()
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    build_dirs.sort();
    for build_dir in build_dirs {
        let manifest = load_json_object(&build_dir.join("manifest.json"));
        let live_leases = live_lease_paths_with_current_source_native_cache_lease_store(
            store,
            &build_dir.join("leases"),
        );
        let last_used_at = manifest
            .get("last_used_at")
            .and_then(JsonValue::as_f64)
            .or_else(|| path_mtime_seconds(&build_dir).ok())
            .unwrap_or(current_time);
        let state = manifest
            .get("state")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !live_leases.is_empty() {
            let size_bytes = manifest
                .get("size_bytes")
                .and_then(JsonValue::as_u64)
                .unwrap_or_else(|| current_source_cache_size_bytes(&build_dir));
            retained.push((last_used_at, size_bytes, build_dir));
            continue;
        }
        if state == "building" && current_time - last_used_at >= request.build_stale_seconds as f64
        {
            remove_tree(&build_dir);
            removed_abandoned_builds.push(relative_path_text(&build_dir, &root));
            continue;
        }
        if request.remove_unleased_ready && state == "ready" {
            remove_tree(&build_dir);
            removed_unleased_ready.push(relative_path_text(&build_dir, &root));
            continue;
        }
        if current_time - last_used_at >= request.idle_ttl_seconds as f64 {
            remove_tree(&build_dir);
            removed_idle.push(relative_path_text(&build_dir, &root));
            continue;
        }
        let size_bytes = manifest
            .get("size_bytes")
            .and_then(JsonValue::as_u64)
            .unwrap_or_else(|| current_source_cache_size_bytes(&build_dir));
        retained.push((last_used_at, size_bytes, build_dir));
    }
    let mut total_bytes = retained.iter().map(|(_, size, _)| *size).sum::<u64>();
    if total_bytes > request.max_bytes {
        retained.sort_by(|left, right| left.0.total_cmp(&right.0));
        for (_, size_bytes, build_dir) in retained {
            if total_bytes <= request.max_bytes {
                break;
            }
            if !live_lease_paths_with_current_source_native_cache_lease_store(
                store,
                &build_dir.join("leases"),
            )
            .is_empty()
            {
                continue;
            }
            remove_tree(&build_dir);
            removed_budget.push(relative_path_text(&build_dir, &root));
            total_bytes = total_bytes.saturating_sub(size_bytes);
        }
    }
    prune_empty_dirs(&root);
    Ok(json!({
        "removed_idle": removed_idle,
        "removed_abandoned_builds": removed_abandoned_builds,
        "removed_budget": removed_budget,
        "removed_unleased_ready": removed_unleased_ready,
        "total_bytes": total_bytes,
    }))
}

pub(super) fn remove_tree(root: &Path) {
    let _ = fs::remove_dir_all(root);
}

pub(super) fn prune_empty_dirs(root: &Path) {
    let mut current = root.to_path_buf();
    while current.exists() {
        match fs::read_dir(&current) {
            Ok(entries) => {
                let mut entries = entries;
                if entries.next().is_some() {
                    break;
                }
                if fs::remove_dir(&current).is_err() {
                    break;
                }
            }
            _ => break,
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
}
