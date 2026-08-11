use super::*;

pub fn write_current_source_native_cache_manifest_json(
    request: &CurrentSourceNativeCacheManifestRequest,
) -> Result<JsonValue, String> {
    let store = FilesystemCurrentSourceNativeCacheManifestStore;
    write_current_source_native_cache_manifest_with_store(&store, request)
}

pub(super) fn write_current_source_native_cache_manifest_with_store<S>(
    store: &S,
    request: &CurrentSourceNativeCacheManifestRequest,
) -> Result<JsonValue, String>
where
    S: CurrentSourceNativeCacheManifestStore + ?Sized,
{
    ensure_cache_root_with_current_source_native_cache_manifest_store(
        store,
        &request.paths.cache_root,
    )?;
    let mut payload = JsonMap::new();
    payload.insert("state".to_string(), json!(request.state));
    payload.insert("build_key".to_string(), json!(request.paths.build_key));
    payload.insert(
        "source_mtime_ns".to_string(),
        json!(request.source_mtime_ns),
    );
    payload.insert(
        "last_used_at".to_string(),
        json!(request.last_used_at.unwrap_or_else(now_seconds)),
    );
    payload.insert(
        "size_bytes".to_string(),
        json!(request.size_bytes.unwrap_or_else(|| {
            cache_size_bytes_with_current_source_native_cache_manifest_store(
                store,
                &request.paths.cache_root,
            )
        })),
    );
    payload.insert(
        "target_dir".to_string(),
        json!(path_text(&request.paths.target_dir)),
    );
    for (key, value) in &request.extra {
        if !key.trim().is_empty() {
            payload.insert(key.clone(), value.clone());
        }
    }
    let value = JsonValue::Object(payload);
    write_manifest_with_current_source_native_cache_manifest_store(
        store,
        &request.paths.manifest_path,
        &value,
    )?;
    Ok(value)
}

pub fn repair_current_source_native_cache_manifest_after_use_json(
    paths: &CurrentSourceNativeCachePaths,
    source_mtime_ns: u64,
) -> Result<JsonValue, String> {
    let store = FilesystemCurrentSourceNativeCacheManifestStore;
    repair_current_source_native_cache_manifest_after_use_with_store(&store, paths, source_mtime_ns)
}

pub(super) fn repair_current_source_native_cache_manifest_after_use_with_store<S>(
    store: &S,
    paths: &CurrentSourceNativeCachePaths,
    source_mtime_ns: u64,
) -> Result<JsonValue, String>
where
    S: CurrentSourceNativeCacheManifestStore + ?Sized,
{
    let manifest =
        load_manifest_with_current_source_native_cache_manifest_store(store, &paths.manifest_path);
    let state = manifest
        .get("state")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("ready")
        .to_string();
    let mut extra = JsonMap::new();
    for (key, value) in manifest {
        if !matches!(
            key.as_str(),
            "state" | "source_mtime_ns" | "last_used_at" | "size_bytes"
        ) {
            extra.insert(key, value);
        }
    }
    write_current_source_native_cache_manifest_with_store(
        store,
        &CurrentSourceNativeCacheManifestRequest {
            paths: paths.clone(),
            state,
            source_mtime_ns,
            last_used_at: Some(now_seconds()),
            size_bytes: None,
            extra,
        },
    )
}

pub(super) fn load_json_object(path: &Path) -> JsonMap<String, JsonValue> {
    CurrentSourceCacheJson::filesystem().load_object_or_empty(path)
}

pub(super) fn atomic_write_json(path: &Path, payload: &JsonValue) -> Result<(), String> {
    CurrentSourceCacheJson::filesystem().write_pretty_json_atomically(path, payload)
}
