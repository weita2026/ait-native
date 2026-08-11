use crate::json_support::{JsonMap, JsonValue};
use std::path::Path;

pub(super) trait CurrentSourceNativeCacheManifestStore {
    fn ensure_cache_root(&self, cache_root: &Path) -> Result<(), String>;
    fn cache_size_bytes(&self, cache_root: &Path) -> u64;
    fn load_manifest(&self, manifest_path: &Path) -> JsonMap<String, JsonValue>;
    fn write_manifest(&self, manifest_path: &Path, payload: &JsonValue) -> Result<(), String>;
}

pub(super) fn ensure_cache_root_with_current_source_native_cache_manifest_store<S>(
    store: &S,
    cache_root: &Path,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheManifestStore + ?Sized,
{
    store.ensure_cache_root(cache_root)
}

pub(super) fn cache_size_bytes_with_current_source_native_cache_manifest_store<S>(
    store: &S,
    cache_root: &Path,
) -> u64
where
    S: CurrentSourceNativeCacheManifestStore + ?Sized,
{
    store.cache_size_bytes(cache_root)
}

pub(super) fn load_manifest_with_current_source_native_cache_manifest_store<S>(
    store: &S,
    manifest_path: &Path,
) -> JsonMap<String, JsonValue>
where
    S: CurrentSourceNativeCacheManifestStore + ?Sized,
{
    store.load_manifest(manifest_path)
}

pub(super) fn write_manifest_with_current_source_native_cache_manifest_store<S>(
    store: &S,
    manifest_path: &Path,
    payload: &JsonValue,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheManifestStore + ?Sized,
{
    store.write_manifest(manifest_path, payload)
}
