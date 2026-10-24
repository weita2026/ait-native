//! Shared Agent Teams operation lock and durable receipts.
use fs2::FileExt;
use std::{fs, io::Write, path::Path};

pub(crate) fn operation_lock(root: &Path) -> Result<(std::path::PathBuf, fs::File), String> {
    let operational = root.join(".ait/team-session-bootstrap");
    fs::create_dir_all(&operational).map_err(|e| e.to_string())?;
    fs::File::open(root.join(".ait"))
        .and_then(|dir| dir.sync_all())
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
    let tmp = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    file.write_all(id.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    fs::rename(tmp, path).map_err(|e| e.to_string())?;
    fs::File::open(path.parent().unwrap())
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
