use super::*;

pub fn register_current_source_native_cache_lease_json(
    paths: &CurrentSourceNativeCachePaths,
    worker_id: &str,
) -> Result<JsonValue, String> {
    register_current_source_native_cache_lease_for_owner_json(paths, worker_id, std::process::id())
}

pub fn register_current_source_native_cache_lease_for_owner_json(
    paths: &CurrentSourceNativeCachePaths,
    worker_id: &str,
    owner_pid: u32,
) -> Result<JsonValue, String> {
    let store = FilesystemCurrentSourceNativeCacheLeaseStore;
    register_current_source_native_cache_lease_with_store(&store, paths, worker_id, owner_pid)
}

pub(super) fn register_current_source_native_cache_lease_with_store<S>(
    store: &S,
    paths: &CurrentSourceNativeCachePaths,
    worker_id: &str,
    owner_pid: u32,
) -> Result<JsonValue, String>
where
    S: CurrentSourceNativeCacheLeaseStore + ?Sized,
{
    if owner_pid == 0 || i32::try_from(owner_pid).is_err() {
        return Err("current-source cache lease owner PID must be a positive i32.".to_string());
    }
    ensure_leases_dir_with_current_source_native_cache_lease_store(store, &paths.leases_dir)?;
    let worker_id = normalized_text(Some(worker_id)).unwrap_or_else(|| "main".to_string());
    let lease_path = paths
        .leases_dir
        .join(format!("{owner_pid}-{worker_id}.json"));
    let payload = json!({
        "pid": owner_pid,
        "worker_id": worker_id,
        "created_at": now_seconds(),
        "build_key": paths.build_key,
    });
    write_lease_with_current_source_native_cache_lease_store(store, &lease_path, &payload)?;
    Ok(json!({
        "lease_path": path_text(&lease_path),
        "namespace_root": path_text(&paths.namespace_root),
        "build_key": paths.build_key,
    }))
}

pub fn release_current_source_native_cache_lease_json(
    lease_path: &Path,
    namespace_root: &Path,
    remove_unleased_ready: bool,
) -> Result<JsonValue, String> {
    let store = FilesystemCurrentSourceNativeCacheLeaseStore;
    release_current_source_native_cache_lease_with_store(
        &store,
        lease_path,
        namespace_root,
        remove_unleased_ready,
    )
}

pub(super) fn release_current_source_native_cache_lease_with_store<S>(
    store: &S,
    lease_path: &Path,
    namespace_root: &Path,
    remove_unleased_ready: bool,
) -> Result<JsonValue, String>
where
    S: CurrentSourceNativeCacheLeaseStore + ?Sized,
{
    release_lease_with_current_source_native_cache_lease_store(store, lease_path)?;
    let summary = prune_current_source_native_caches_with_lease_store(
        store,
        &CurrentSourceNativeCachePruneRequest {
            namespace_root: namespace_root.to_path_buf(),
            now: None,
            idle_ttl_seconds: CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS,
            build_stale_seconds: CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS,
            max_bytes: CURRENT_SOURCE_CACHE_MAX_BYTES,
            remove_unleased_ready,
        },
    )?;
    Ok(json!({
        "released": path_text(lease_path),
        "prune": summary,
    }))
}

pub(super) fn prune_dead_leases(leases_dir: &Path) -> Vec<PathBuf> {
    if !leases_dir.exists() {
        return Vec::new();
    }
    let mut lease_paths = fs::read_dir(leases_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    lease_paths.sort();
    let mut live = Vec::new();
    for lease_path in lease_paths {
        let payload = load_json_object(&lease_path);
        let pid = metadata_u64(&payload, "pid").and_then(|value| i32::try_from(value).ok());
        if pid_is_live(pid) {
            live.push(lease_path);
        } else {
            let _ = fs::remove_file(&lease_path);
        }
    }
    live
}

pub(super) fn pid_is_live(pid: Option<i32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    #[cfg(unix)]
    {
        unsafe {
            libc::kill(pid, 0) == 0
                || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
        }
    }
    #[cfg(not(unix))]
    {
        pid == std::process::id() as i32
    }
}
