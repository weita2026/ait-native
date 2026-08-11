use super::lease_ports::CurrentSourceNativeCacheLeaseStore;
use crate::json_support::JsonValue;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) struct FilesystemCurrentSourceNativeCacheLeaseStore;

impl CurrentSourceNativeCacheLeaseStore for FilesystemCurrentSourceNativeCacheLeaseStore {
    fn ensure_leases_dir(&self, leases_dir: &Path) -> Result<(), String> {
        fs::create_dir_all(leases_dir).map_err(|err| {
            format!(
                "Failed to create current-source cache leases directory {}: {err}",
                leases_dir.display()
            )
        })
    }

    fn write_lease(&self, lease_path: &Path, payload: &JsonValue) -> Result<(), String> {
        super::atomic_write_json(lease_path, payload)
    }

    fn release_lease(&self, lease_path: &Path) -> Result<(), String> {
        match fs::remove_file(lease_path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!(
                "Failed to release current-source cache lease {}: {err}",
                lease_path.display()
            )),
        }
    }

    fn live_lease_paths(&self, leases_dir: &Path) -> Vec<PathBuf> {
        super::prune_dead_leases(leases_dir)
    }
}
