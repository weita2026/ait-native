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

pub(super) fn load_json_object(path: &Path) -> JsonMap<String, JsonValue> {
    CurrentSourceCacheJson::filesystem().load_object_or_empty(path)
}

pub(super) fn atomic_write_json(path: &Path, payload: &JsonValue) -> Result<(), String> {
    CurrentSourceCacheJson::filesystem().write_pretty_json_atomically(path, payload)
}
