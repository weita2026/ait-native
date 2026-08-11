use crate::json_support::{JsonMap, JsonValue};
use std::path::{Path, PathBuf};

pub(super) trait CurrentSourceNativeCacheArtifactStore {
    fn artifact_exists(&self, path: &Path) -> bool;
    fn artifact_is_executable(&self, path: &Path) -> bool;
    fn artifact_mtime_ns(&self, path: &Path) -> Result<u64, String>;
    fn artifact_sha256_hex(&self, path: &Path) -> Result<String, String>;
    fn load_metadata(&self, path: &Path) -> JsonMap<String, JsonValue>;
    fn publish_artifact(
        &self,
        source: &Path,
        target: &Path,
        repair_extension_install_name: bool,
    ) -> Result<(), String>;
    fn ensure_local_extension_init(&self, init_path: &Path) -> Result<(), String>;
    fn write_metadata(&self, metadata_path: &Path, payload: &JsonValue) -> Result<(), String>;
}

pub(super) fn artifact_exists_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    path: &Path,
) -> bool
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    store.artifact_exists(path)
}

pub(super) fn artifact_is_executable_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    path: &Path,
) -> bool
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    store.artifact_is_executable(path)
}

pub(super) fn artifact_mtime_ns_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    path: &Path,
) -> Result<u64, String>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    store.artifact_mtime_ns(path)
}

pub(super) fn artifact_sha256_hex_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    path: &Path,
) -> Result<String, String>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    store.artifact_sha256_hex(path)
}

pub(super) fn load_metadata_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    path: &Path,
) -> JsonMap<String, JsonValue>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    store.load_metadata(path)
}

pub(super) fn publish_artifact_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    source: &Path,
    target: &Path,
    repair_extension_install_name: bool,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    store.publish_artifact(source, target, repair_extension_install_name)
}

pub(super) fn ensure_local_extension_init_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    init_path: &Path,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    store.ensure_local_extension_init(init_path)
}

pub(super) fn write_metadata_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    metadata_path: &Path,
    payload: &JsonValue,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    store.write_metadata(metadata_path, payload)
}

pub(super) fn first_existing_artifact_with_current_source_native_cache_artifact_store<S>(
    store: &S,
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Option<PathBuf>
where
    S: CurrentSourceNativeCacheArtifactStore + ?Sized,
{
    candidates
        .into_iter()
        .find(|path| artifact_exists_with_current_source_native_cache_artifact_store(store, path))
}
