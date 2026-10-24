use super::manifest_ports::CurrentSourceNativeCacheManifestStore;
use crate::json_support::JsonValue;
use std::fs;
use std::path::Path;

pub(super) struct FilesystemCurrentSourceNativeCacheManifestStore;

impl CurrentSourceNativeCacheManifestStore for FilesystemCurrentSourceNativeCacheManifestStore {
    fn ensure_cache_root(&self, cache_root: &Path) -> Result<(), String> {
        fs::create_dir_all(cache_root).map_err(|err| {
            format!(
                "Failed to create current-source cache root {}: {err}",
                cache_root.display()
            )
        })
    }

    fn cache_size_bytes(&self, cache_root: &Path) -> u64 {
        super::current_source_cache_size_bytes(cache_root)
    }

    fn write_manifest(&self, manifest_path: &Path, payload: &JsonValue) -> Result<(), String> {
        super::atomic_write_json(manifest_path, payload)
    }
}
