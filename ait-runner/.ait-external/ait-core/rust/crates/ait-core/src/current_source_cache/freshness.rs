use super::*;

pub fn current_source_extension_is_fresh_json(
    request: &CurrentSourceExtensionFreshnessRequest,
) -> JsonValue {
    let store = FilesystemCurrentSourceNativeCacheArtifactStore;
    json!({
        "fresh": current_source_extension_is_fresh_with_artifact_store(
            &store,
            &request.metadata_path,
            &request.extension_path,
            request.source_mtime_ns,
            &request.source_fingerprint,
        ),
        "metadata_path": path_text(&request.metadata_path),
        "extension_path": path_text(&request.extension_path),
    })
}

pub fn current_source_binary_is_fresh_json(
    request: &CurrentSourceBinaryFreshnessRequest,
) -> JsonValue {
    let store = FilesystemCurrentSourceNativeCacheArtifactStore;
    json!({
        "fresh": current_source_binary_is_fresh_with_artifact_store(&store, request),
        "metadata_path": path_text(&request.metadata_path),
        "binary_path": path_text(&request.binary_path),
    })
}

pub(super) fn current_source_extension_is_fresh_with_artifact_store<S>(
    store: &S,
    metadata_path: &Path,
    extension_path: &Path,
    source_mtime_ns: u64,
    source_fingerprint: &str,
) -> bool
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    if !artifact_is_fresh_with_artifact_store(store, extension_path, source_mtime_ns) {
        return false;
    }
    let metadata =
        load_metadata_with_current_source_native_cache_artifact_store(store, metadata_path);
    metadata_u64(&metadata, "core_source_mtime_ns") == Some(source_mtime_ns)
        && metadata_text(&metadata, "core_source_fingerprint").as_deref()
            == Some(source_fingerprint)
}

pub(super) fn current_source_binary_is_fresh_with_artifact_store<S>(
    store: &S,
    request: &CurrentSourceBinaryFreshnessRequest,
) -> bool
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    if !artifact_exists_with_current_source_native_cache_artifact_store(store, &request.binary_path)
        || !artifact_is_executable_with_current_source_native_cache_artifact_store(
            store,
            &request.binary_path,
        )
    {
        return false;
    }
    let metadata = load_metadata_with_current_source_native_cache_artifact_store(
        store,
        &request.metadata_path,
    );
    if metadata_text(&metadata, &request.metadata_fingerprint_key).as_deref()
        != Some(request.source_fingerprint.as_str())
    {
        return false;
    }
    if metadata_u64(&metadata, &request.metadata_source_mtime_key) != Some(request.source_mtime_ns)
    {
        return false;
    }
    let Some(recorded_mtime) = metadata_u64(&metadata, &request.metadata_mtime_key) else {
        return false;
    };
    let Some(recorded_sha256) = metadata_text(&metadata, &request.metadata_sha_key) else {
        return false;
    };
    let Ok(artifact_mtime_ns) = artifact_mtime_ns_with_current_source_native_cache_artifact_store(
        store,
        &request.binary_path,
    ) else {
        return false;
    };
    if recorded_mtime != artifact_mtime_ns {
        return false;
    }
    artifact_sha256_hex_with_current_source_native_cache_artifact_store(store, &request.binary_path)
        .map(|sha| sha == recorded_sha256)
        .unwrap_or(false)
}

pub(super) fn artifact_is_fresh_with_artifact_store<S>(
    store: &S,
    path: &Path,
    source_mtime_ns: u64,
) -> bool
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    artifact_exists_with_current_source_native_cache_artifact_store(store, path)
        && artifact_mtime_ns_with_current_source_native_cache_artifact_store(store, path)
            .map(|mtime| mtime >= source_mtime_ns)
            .unwrap_or(false)
}

pub(super) fn metadata_text(metadata: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn metadata_u64(metadata: &JsonMap<String, JsonValue>, key: &str) -> Option<u64> {
    let value = metadata.get(key)?;
    if let Some(number) = value.as_u64() {
        return Some(number);
    }
    if let Some(number) = value.as_i64() {
        return u64::try_from(number).ok();
    }
    value.as_str()?.trim().parse::<u64>().ok()
}
