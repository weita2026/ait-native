//! Shared Agent Teams operation lock and durable receipts.
use ait_core::file_io::{FileIoDurabilityStore, FileIoStore, FilesystemFileIoStore};
use fs2::FileExt;
use std::{fs, path::Path};

pub(crate) fn operation_lock(root: &Path) -> Result<(std::path::PathBuf, fs::File), String> {
    let operational = root.join(".ait/team-session-bootstrap");
    fs::create_dir_all(&operational).map_err(|e| e.to_string())?;
    FilesystemFileIoStore
        .sync_dir(&root.join(".ait"))
        .map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(operational.join("bootstrap.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock_exclusive().map_err(|_| {
        "operation_busy: Another team preparation is running; retry after it completes".to_string()
    })?;
    Ok((operational, lock))
}

pub(crate) fn fence(path: &Path, id: &str) -> Result<(), String> {
    FilesystemFileIoStore
        .write_string_atomically(path, id, "team session receipt")
        .map_err(|e| e.to_string())?;
    FilesystemFileIoStore
        .sync_dir(path.parent().unwrap())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
