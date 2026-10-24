use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CurrentSourceNativeCacheSourceEntryKind {
    File,
    Directory,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CurrentSourceNativeCacheSourceEntry {
    pub(super) path: PathBuf,
    pub(super) kind: CurrentSourceNativeCacheSourceEntryKind,
}

impl CurrentSourceNativeCacheSourceEntry {
    pub(super) fn file(path: PathBuf) -> Self {
        Self {
            path,
            kind: CurrentSourceNativeCacheSourceEntryKind::File,
        }
    }

    pub(super) fn directory(path: PathBuf) -> Self {
        Self {
            path,
            kind: CurrentSourceNativeCacheSourceEntryKind::Directory,
        }
    }

    pub(super) fn other(path: PathBuf) -> Self {
        Self {
            path,
            kind: CurrentSourceNativeCacheSourceEntryKind::Other,
        }
    }
}

pub(super) trait CurrentSourceNativeCacheSourceStore {
    fn resolve_path_strict_false(&self, path: &Path) -> PathBuf;
    fn path_exists(&self, path: &Path) -> bool;
    fn path_is_dir(&self, path: &Path) -> bool;
    fn read_source_file(&self, path: &Path) -> Result<Vec<u8>, String>;
    fn path_mtime_ns(&self, path: &Path) -> Result<u64, String>;
    fn read_source_dir(
        &self,
        dir: &Path,
    ) -> Result<Vec<CurrentSourceNativeCacheSourceEntry>, String>;
}

pub(super) fn resolve_path_with_current_source_native_cache_source_store<S>(
    store: &S,
    path: &Path,
) -> PathBuf
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    store.resolve_path_strict_false(path)
}

pub(super) fn path_exists_with_current_source_native_cache_source_store<S>(
    store: &S,
    path: &Path,
) -> bool
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    store.path_exists(path)
}

pub(super) fn path_is_dir_with_current_source_native_cache_source_store<S>(
    store: &S,
    path: &Path,
) -> bool
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    store.path_is_dir(path)
}

pub(super) fn read_source_file_with_current_source_native_cache_source_store<S>(
    store: &S,
    path: &Path,
) -> Result<Vec<u8>, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    store.read_source_file(path)
}

pub(super) fn path_mtime_ns_with_current_source_native_cache_source_store<S>(
    store: &S,
    path: &Path,
) -> Result<u64, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    store.path_mtime_ns(path)
}

pub(super) fn read_source_dir_with_current_source_native_cache_source_store<S>(
    store: &S,
    dir: &Path,
) -> Result<Vec<CurrentSourceNativeCacheSourceEntry>, String>
where
    S: CurrentSourceNativeCacheSourceStore + ?Sized,
{
    store.read_source_dir(dir)
}
