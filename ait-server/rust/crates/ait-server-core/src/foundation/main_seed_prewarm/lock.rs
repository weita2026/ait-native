use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use super::helpers::path_string;
use super::LOCK_POLL_MS;

pub(super) struct SeedPrewarmLock {
    path: PathBuf,
}

impl SeedPrewarmLock {
    pub(super) fn acquire(path: &Path, timeout_ms: u64) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|exc| {
                format!(
                    "Failed to create main-seed lock parent `{}`: {exc}",
                    path_string(parent)
                )
            })?;
        }
        let started = Instant::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(_) => {
                    fs::write(path, format!("pid={}\n", std::process::id())).map_err(|exc| {
                        format!(
                            "Failed to write main-seed lock `{}`: {exc}",
                            path_string(path)
                        )
                    })?;
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(exc) if exc.kind() == std::io::ErrorKind::AlreadyExists => {
                    if started.elapsed() >= Duration::from_millis(timeout_ms) {
                        return Err(format!(
                            "Timed out waiting for main-seed prewarm lock `{}`.",
                            path_string(path)
                        ));
                    }
                    thread::sleep(Duration::from_millis(LOCK_POLL_MS));
                }
                Err(exc) => {
                    return Err(format!(
                        "Failed to acquire main-seed prewarm lock `{}`: {exc}",
                        path_string(path)
                    ));
                }
            }
        }
    }
}

impl Drop for SeedPrewarmLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
