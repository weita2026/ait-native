//! Canonically ordered command/read lock mapping and guard lifecycles.

use super::*;

#[derive(Debug)]
pub struct BinaryDbCommandLockSet {
    scope: BinaryDbCommandScope,
    paths: Vec<PathBuf>,
    locks: Vec<BinaryDbHeldCommandLock>,
    released: bool,
}

#[derive(Debug)]
struct BinaryDbHeldCommandLock {
    guard: BoxedFileIoProcessLockGuard,
}

impl BinaryDbCommandLockSet {
    pub(crate) fn detached_generation_noop(scope: BinaryDbCommandScope) -> Self {
        Self {
            scope,
            paths: Vec::new(),
            locks: Vec::new(),
            released: false,
        }
    }

    pub fn lock_root(authority_root: &StorePath) -> PathBuf {
        authority_root.as_path().join(".locks").join("binary-db")
    }

    pub fn lock_paths(authority_root: &StorePath, scope: BinaryDbCommandScope) -> Vec<PathBuf> {
        scope
            .lock_file_names()
            .iter()
            .map(|name| Self::lock_root(authority_root).join(name))
            .collect()
    }

    pub fn acquire(authority_root: &StorePath, scope: BinaryDbCommandScope) -> StoreResult<Self> {
        Self::acquire_with_file_io_store(&FilesystemFileIoStore, authority_root, scope)
    }

    pub fn acquire_with_file_io_store<S>(
        files: &S,
        authority_root: &StorePath,
        scope: BinaryDbCommandScope,
    ) -> StoreResult<Self>
    where
        S: FileIoByteStore + FileIoLockStore + ?Sized,
    {
        Self::open_and_lock(files, authority_root, scope, false)?.ok_or_else(|| {
            BinaryDbError::retryable_busy(
                "blocking Binary DB command lock acquisition returned no lock",
            )
        })
    }

    pub fn try_acquire(
        authority_root: &StorePath,
        scope: BinaryDbCommandScope,
    ) -> StoreResult<Option<Self>> {
        Self::try_acquire_with_file_io_store(&FilesystemFileIoStore, authority_root, scope)
    }

    pub fn try_acquire_with_file_io_store<S>(
        files: &S,
        authority_root: &StorePath,
        scope: BinaryDbCommandScope,
    ) -> StoreResult<Option<Self>>
    where
        S: FileIoByteStore + FileIoLockStore + ?Sized,
    {
        Self::open_and_lock(files, authority_root, scope, true)
    }

    pub fn command_scope(&self) -> BinaryDbCommandScope {
        self.scope
    }

    pub fn scope(&self) -> BinaryDbCommandScope {
        self.scope
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn release(&mut self) -> StoreResult<()> {
        if self.released {
            return Ok(());
        }
        for lock in &mut self.locks {
            lock.guard
                .clear_contents_and_flush()
                .map_err(|e| file_io_error_to_binary("clear Binary DB lock metadata", e))?;
            lock.guard
                .release()
                .map_err(|e| file_io_error_to_binary("release Binary DB command lock", e))?;
        }
        self.released = true;
        Ok(())
    }

    fn open_and_lock<S>(
        files: &S,
        authority_root: &StorePath,
        scope: BinaryDbCommandScope,
        nonblocking: bool,
    ) -> StoreResult<Option<Self>>
    where
        S: FileIoByteStore + FileIoLockStore + ?Sized,
    {
        let lock_root = Self::lock_root(authority_root);
        files
            .create_parent_dirs(&lock_root.join(".lock-root"))
            .map_err(|e| file_io_error_to_binary("create Binary DB lock directory", e))?;
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut locks: Vec<BinaryDbHeldCommandLock> = Vec::new();
        let wait = if nonblocking {
            FileIoLockWait::Nonblocking
        } else {
            FileIoLockWait::Blocking
        };
        for path in Self::lock_paths(authority_root, scope) {
            let mut guard = match files
                .acquire_process_lock(&path, FileIoLockMode::Exclusive, wait)
                .map_err(|e| {
                    file_io_error_to_binary(format!("open Binary DB lock {}", path.display()), e)
                })? {
                Some(guard) => guard,
                None => {
                    for lock in &mut locks {
                        let _ = lock.guard.clear_contents_and_flush();
                        let _ = lock.guard.release();
                    }
                    return Ok(None);
                }
            };
            if let Err(err) = guard.replace_contents_and_flush(&lock_metadata_bytes(scope, &path)) {
                for lock in &mut locks {
                    let _ = lock.guard.clear_contents_and_flush();
                    let _ = lock.guard.release();
                }
                let _ = guard.release();
                return Err(file_io_error_to_binary(
                    "write Binary DB lock metadata",
                    err,
                ));
            }
            paths.push(path);
            locks.push(BinaryDbHeldCommandLock { guard });
        }
        Ok(Some(Self {
            scope,
            paths,
            locks,
            released: false,
        }))
    }
}

impl Drop for BinaryDbCommandLockSet {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[derive(Debug)]
pub struct BinaryDbReadLockSet {
    paths: Vec<PathBuf>,
    locks: Vec<BinaryDbHeldReadLock>,
    released: bool,
}

#[derive(Debug)]
struct BinaryDbHeldReadLock {
    guard: BoxedFileIoProcessLockGuard,
}

impl BinaryDbReadLockSet {
    pub(crate) fn detached_generation_noop() -> Self {
        Self {
            paths: Vec::new(),
            locks: Vec::new(),
            released: false,
        }
    }

    pub fn try_acquire(authority_root: &StorePath) -> StoreResult<Self> {
        Self::try_acquire_for_scope(authority_root, BinaryDbReadScope::All)
    }

    pub fn try_acquire_for_scope(
        authority_root: &StorePath,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<Self> {
        Self::try_acquire_for_scope_with_file_io_store(
            &FilesystemFileIoStore,
            authority_root,
            read_scope,
        )
    }

    pub fn try_acquire_with_file_io_store<S>(
        files: &S,
        authority_root: &StorePath,
    ) -> StoreResult<Self>
    where
        S: FileIoByteStore + FileIoLockStore + ?Sized,
    {
        Self::try_acquire_for_scope_with_file_io_store(
            files,
            authority_root,
            BinaryDbReadScope::All,
        )
    }

    pub fn try_acquire_for_scope_with_file_io_store<S>(
        files: &S,
        authority_root: &StorePath,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<Self>
    where
        S: FileIoByteStore + FileIoLockStore + ?Sized,
    {
        let lock_root = BinaryDbCommandLockSet::lock_root(authority_root);
        files
            .create_parent_dirs(&lock_root.join(".lock-root"))
            .map_err(|e| file_io_error_to_binary("create Binary DB read lock directory", e))?;
        let mut paths: Vec<PathBuf> = Vec::new();
        let mut locks: Vec<BinaryDbHeldReadLock> = Vec::new();
        for name in read_scope.lock_file_names() {
            let path = lock_root.join(name);
            let guard = match files
                .acquire_process_lock(&path, FileIoLockMode::Shared, FileIoLockWait::Nonblocking)
                .map_err(|e| {
                    file_io_error_to_binary(
                        format!("open Binary DB read lock {}", path.display()),
                        e,
                    )
                })? {
                Some(guard) => guard,
                None => {
                    for lock in &mut locks {
                        let _ = lock.guard.release();
                    }
                    return Err(BinaryDbError::retryable_busy(
                        "Binary DB writer is active; retry read after writer commits",
                    ));
                }
            };
            paths.push(path);
            locks.push(BinaryDbHeldReadLock { guard });
        }
        Ok(Self {
            paths,
            locks,
            released: false,
        })
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn release(&mut self) -> StoreResult<()> {
        if self.released {
            return Ok(());
        }
        for lock in &mut self.locks {
            lock.guard
                .release()
                .map_err(|e| file_io_error_to_binary("release Binary DB read lock", e))?;
        }
        self.released = true;
        Ok(())
    }
}

impl Drop for BinaryDbReadLockSet {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

/// API-level read-only transaction surface for Binary DB access.
fn lock_metadata_bytes(scope: BinaryDbCommandScope, path: &Path) -> Vec<u8> {
    let started_at_s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let metadata = format!(
        "scope={:?}\npid={}\nstarted_at_s={}\npath={}\n",
        scope,
        process::id(),
        started_at_s,
        path.display()
    );
    metadata.into_bytes()
}
