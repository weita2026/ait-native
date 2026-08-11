use super::source_ports::{
    CurrentSourceNativeCacheSourceEntry, CurrentSourceNativeCacheSourceStore,
};
use super::{path_mtime_ns, resolve_path_strict_false};
use std::fs;
use std::path::{Path, PathBuf};

pub(super) struct FilesystemCurrentSourceNativeCacheSourceStore;

impl CurrentSourceNativeCacheSourceStore for FilesystemCurrentSourceNativeCacheSourceStore {
    fn resolve_path_strict_false(&self, path: &Path) -> PathBuf {
        resolve_path_strict_false(path)
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn path_is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn read_source_file(&self, path: &Path) -> Result<Vec<u8>, String> {
        fs::read(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))
    }

    fn path_mtime_ns(&self, path: &Path) -> Result<u64, String> {
        path_mtime_ns(path)
    }

    fn read_source_dir(
        &self,
        dir: &Path,
    ) -> Result<Vec<CurrentSourceNativeCacheSourceEntry>, String> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(dir)
            .map_err(|err| format!("Failed to read source directory {}: {err}", dir.display()))?
        {
            let entry = entry.map_err(|err| {
                format!("Failed to read source entry in {}: {err}", dir.display())
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|err| {
                format!("Failed to inspect source path {}: {err}", path.display())
            })?;
            if file_type.is_dir() {
                entries.push(CurrentSourceNativeCacheSourceEntry::directory(path));
            } else if file_type.is_file() {
                entries.push(CurrentSourceNativeCacheSourceEntry::file(path));
            } else {
                entries.push(CurrentSourceNativeCacheSourceEntry::other(path));
            }
        }
        Ok(entries)
    }
}
