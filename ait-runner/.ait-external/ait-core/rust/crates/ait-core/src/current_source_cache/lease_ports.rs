use crate::json_support::JsonValue;
use std::path::{Path, PathBuf};

pub(super) trait CurrentSourceNativeCacheLeaseStore {
    fn ensure_leases_dir(&self, leases_dir: &Path) -> Result<(), String>;
    fn write_lease(&self, lease_path: &Path, payload: &JsonValue) -> Result<(), String>;
    fn release_lease(&self, lease_path: &Path) -> Result<(), String>;
    fn live_lease_paths(&self, leases_dir: &Path) -> Vec<PathBuf>;
}

pub(super) fn ensure_leases_dir_with_current_source_native_cache_lease_store<S>(
    store: &S,
    leases_dir: &Path,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheLeaseStore + ?Sized,
{
    store.ensure_leases_dir(leases_dir)
}

pub(super) fn write_lease_with_current_source_native_cache_lease_store<S>(
    store: &S,
    lease_path: &Path,
    payload: &JsonValue,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheLeaseStore + ?Sized,
{
    store.write_lease(lease_path, payload)
}

pub(super) fn release_lease_with_current_source_native_cache_lease_store<S>(
    store: &S,
    lease_path: &Path,
) -> Result<(), String>
where
    S: CurrentSourceNativeCacheLeaseStore + ?Sized,
{
    store.release_lease(lease_path)
}

pub(super) fn live_lease_paths_with_current_source_native_cache_lease_store<S>(
    store: &S,
    leases_dir: &Path,
) -> Vec<PathBuf>
where
    S: CurrentSourceNativeCacheLeaseStore + ?Sized,
{
    store.live_lease_paths(leases_dir)
}
