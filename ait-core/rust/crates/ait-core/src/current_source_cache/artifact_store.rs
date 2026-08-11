use super::artifact_ports::CurrentSourceNativeCacheArtifactStore;
use crate::json_support::{JsonMap, JsonValue};
use std::path::Path;

pub(super) struct FilesystemCurrentSourceNativeCacheArtifactStore;

impl CurrentSourceNativeCacheArtifactStore for FilesystemCurrentSourceNativeCacheArtifactStore {
    fn artifact_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn artifact_is_executable(&self, path: &Path) -> bool {
        super::is_executable(path)
    }

    fn artifact_mtime_ns(&self, path: &Path) -> Result<u64, String> {
        super::path_mtime_ns(path)
    }

    fn artifact_sha256_hex(&self, path: &Path) -> Result<String, String> {
        super::artifact_sha256_hex(path)
    }

    fn load_metadata(&self, path: &Path) -> JsonMap<String, JsonValue> {
        super::load_json_object(path)
    }

    fn publish_artifact(
        &self,
        source: &Path,
        target: &Path,
        repair_extension_install_name: bool,
    ) -> Result<(), String> {
        super::publish_artifact(source, target, repair_extension_install_name)
    }

    fn ensure_local_extension_init(&self, init_path: &Path) -> Result<(), String> {
        super::ensure_local_extension_init(init_path)
    }

    fn write_metadata(&self, metadata_path: &Path, payload: &JsonValue) -> Result<(), String> {
        super::atomic_write_json(metadata_path, payload)
    }
}
