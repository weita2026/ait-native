use crate::json_support::{parse_value_option, write_pretty_value};
use crate::runtime::RepoRuntime;
use ait_core::json_support::{json, JsonValue};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

const LOCK_TOKEN_ENV: &str = ait_core::environment_contract::names::AIT_WORKSPACE_LOCK_OWNER_TOKEN;

pub fn run_locked_workspace_command<T, F>(
    repo: &RepoRuntime,
    command_name: &str,
    operation: F,
) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String>,
{
    let _lock = WorkspaceCommandLock::acquire(repo, command_name)?;
    operation()
}

pub fn workspace_command_lock_path(repo: &RepoRuntime) -> PathBuf {
    let workspace_root = workspace_root(repo);
    let mut hasher = Sha256::new();
    hasher.update(workspace_root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    workspace_root
        .join(".ait")
        .join("workspace")
        .join("locks")
        .join(format!("{}.lock", &hex[..16]))
}

fn workspace_root(repo: &RepoRuntime) -> PathBuf {
    repo.root
        .canonicalize()
        .unwrap_or_else(|_| repo.root.clone())
}

pub struct WorkspaceCommandLock {
    state: WorkspaceCommandLockState,
    metadata: JsonValue,
}

enum WorkspaceCommandLockState {
    Owned {
        file: File,
        _env_guard: WorkspaceLockEnvGuard,
    },
    Borrowed,
}

impl WorkspaceCommandLock {
    pub fn acquire(repo: &RepoRuntime, command_name: &str) -> Result<Self, String> {
        let workspace_root = workspace_root(repo);
        let lock_path = workspace_command_lock_path(repo);
        let parent = lock_path
            .parent()
            .ok_or_else(|| "Workspace command lock path has no parent.".to_string())?;
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        if let Some(metadata) = borrowable_workspace_lock_metadata(&lock_path, &workspace_root) {
            return Ok(Self {
                state: WorkspaceCommandLockState::Borrowed,
                metadata,
            });
        }
        let mut file = OpenOptions::new()
            .create(true)
            // Preserve metadata until this process owns the lock.
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|err| err.to_string())?;
        file.lock_exclusive().map_err(|err| err.to_string())?;
        let owner_token = new_owner_token(&workspace_root);
        let metadata = json!({
            "command": command_name,
            "pid": process::id(),
            "repo_root": workspace_root,
            "workspace_root": workspace_root,
            "lock_path": lock_path,
            "owner_token": owner_token,
            "started_at": current_timestamp_string(),
        });
        file.set_len(0).map_err(|err| err.to_string())?;
        file.seek(SeekFrom::Start(0))
            .map_err(|err| err.to_string())?;
        write_pretty_value(&mut file, &metadata, "Failed to encode JSON").map_err(|err| {
            err.strip_prefix("Failed to encode JSON: ")
                .unwrap_or(&err)
                .to_string()
        })?;
        file.write_all(b"\n").map_err(|err| err.to_string())?;
        file.flush().map_err(|err| err.to_string())?;
        let env_guard = WorkspaceLockEnvGuard::set(&owner_token);
        Ok(Self {
            state: WorkspaceCommandLockState::Owned {
                file,
                _env_guard: env_guard,
            },
            metadata,
        })
    }

    pub fn metadata(&self) -> &JsonValue {
        &self.metadata
    }

    pub fn is_borrowed(&self) -> bool {
        matches!(self.state, WorkspaceCommandLockState::Borrowed)
    }
}

impl Drop for WorkspaceCommandLock {
    fn drop(&mut self) {
        match &mut self.state {
            WorkspaceCommandLockState::Owned { file, .. } => {
                let _ = file.set_len(0);
                let _ = file.seek(SeekFrom::Start(0));
                let _ = file.flush();
                let _ = file.unlock();
            }
            WorkspaceCommandLockState::Borrowed => {}
        }
    }
}

struct WorkspaceLockEnvGuard {
    previous: Option<String>,
}

impl WorkspaceLockEnvGuard {
    fn set(owner_token: &str) -> Self {
        let previous = env::var(LOCK_TOKEN_ENV).ok();
        env::set_var(LOCK_TOKEN_ENV, owner_token);
        Self { previous }
    }
}

impl Drop for WorkspaceLockEnvGuard {
    fn drop(&mut self) {
        match self.previous.as_deref() {
            Some(value) => env::set_var(LOCK_TOKEN_ENV, value),
            None => env::remove_var(LOCK_TOKEN_ENV),
        }
    }
}

fn borrowable_workspace_lock_metadata(
    lock_path: &Path,
    workspace_root: &Path,
) -> Option<JsonValue> {
    let metadata = read_lock_metadata(lock_path)?;
    if can_borrow_token_lock(&metadata, lock_path, workspace_root)
        || can_borrow_legacy_process_lock(&metadata, lock_path, workspace_root)
    {
        Some(metadata)
    } else {
        None
    }
}

fn read_lock_metadata(lock_path: &Path) -> Option<JsonValue> {
    let text = fs::read_to_string(lock_path).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    parse_value_option(&text)
}

fn can_borrow_token_lock(metadata: &JsonValue, lock_path: &Path, workspace_root: &Path) -> bool {
    let Ok(env_token) = env::var(LOCK_TOKEN_ENV) else {
        return false;
    };
    let Some(owner_pid) = metadata_u32(metadata, "pid") else {
        return false;
    };
    metadata_string(metadata, "workspace_root")
        .map(|root| path_string_eq(root, workspace_root))
        .unwrap_or(false)
        && metadata_string(metadata, "lock_path")
            .map(|path| path_string_eq(path, lock_path))
            .unwrap_or(false)
        && metadata_string(metadata, "owner_token") == Some(env_token.as_str())
        && process_is_alive(owner_pid)
}

fn can_borrow_legacy_process_lock(
    metadata: &JsonValue,
    lock_path: &Path,
    workspace_root: &Path,
) -> bool {
    let Some(holder_pid) = metadata_u32(metadata, "pid") else {
        return false;
    };
    (holder_pid == parent_pid() || holder_pid == process::id())
        && process_is_alive(holder_pid)
        && metadata_string(metadata, "workspace_root")
            .map(|root| path_string_eq(root, workspace_root))
            .unwrap_or(false)
        && lock_is_held_by_another_process(lock_path)
}

fn metadata_string<'a>(metadata: &'a JsonValue, key: &str) -> Option<&'a str> {
    metadata.get(key).and_then(JsonValue::as_str)
}

fn metadata_u32(metadata: &JsonValue, key: &str) -> Option<u32> {
    metadata
        .get(key)
        .and_then(JsonValue::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn path_string_eq(value: &str, path: &Path) -> bool {
    value == path.to_string_lossy()
}

fn lock_is_held_by_another_process(lock_path: &Path) -> bool {
    let Ok(file) = OpenOptions::new().read(true).write(true).open(lock_path) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => {
            let _ = file.unlock();
            false
        }
        Err(err) if is_lock_would_block(&err) => true,
        Err(_) => false,
    }
}

fn is_lock_would_block(err: &std::io::Error) -> bool {
    if err.kind() == ErrorKind::WouldBlock {
        return true;
    }
    #[cfg(unix)]
    {
        matches!(
            err.raw_os_error(),
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
        )
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn new_owner_token(workspace_root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace_root.to_string_lossy().as_bytes());
    hasher.update(process::id().to_string().as_bytes());
    hasher.update(current_timestamp_string().as_bytes());
    if let Ok(duration) = SystemTime::now().duration_since(UNIX_EPOCH) {
        hasher.update(duration.as_nanos().to_string().as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[cfg(unix)]
fn parent_pid() -> u32 {
    unsafe { libc::getppid() as u32 }
}

#[cfg(not(unix))]
fn parent_pid() -> u32 {
    0
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    pid > 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn process_is_alive(pid: u32) -> bool {
    pid > 0
}

fn current_timestamp_string() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

#[cfg(test)]
mod tests;
