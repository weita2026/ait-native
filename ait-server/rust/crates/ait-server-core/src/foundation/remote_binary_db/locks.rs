use super::*;

const BINARY_DB_RECOVERY_ADMISSION_LOCK: &str = "recovery-admission.lock";

impl ServerBinaryDbLockStore for ServerBinaryDbFilesystemStore {
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: ServerBinaryDbLockMode,
        wait: ServerBinaryDbLockWait,
    ) -> StoreResult<Option<BoxedServerBinaryDbProcessLockGuard>> {
        self.create_parent_dirs(path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .map_err(|err| {
                BinaryDbError::io(format!("open Binary DB lock file {}", path.display()), err)
            })?;
        let base = match mode {
            ServerBinaryDbLockMode::Shared => libc::LOCK_SH,
            ServerBinaryDbLockMode::Exclusive => libc::LOCK_EX,
        };
        let operation = match wait {
            ServerBinaryDbLockWait::Blocking => base,
            ServerBinaryDbLockWait::Nonblocking => base | libc::LOCK_NB,
        };
        if let Err(err) = try_flock(&file, operation) {
            if is_lock_busy(&err) {
                return Ok(None);
            }
            return Err(BinaryDbError::io(
                format!("acquire Binary DB process lock {}", path.display()),
                err,
            ));
        }
        Ok(Some(Box::new(ServerBinaryDbFilesystemProcessLockGuard {
            file,
            path: path.to_path_buf(),
            released: false,
        })))
    }
}

#[derive(Debug)]
struct ServerBinaryDbFilesystemProcessLockGuard {
    file: File,
    path: PathBuf,
    released: bool,
}

impl ServerBinaryDbProcessLockGuard for ServerBinaryDbFilesystemProcessLockGuard {
    fn replace_contents_and_flush(&mut self, bytes: &[u8]) -> StoreResult<()> {
        self.file.set_len(0).map_err(|err| {
            BinaryDbError::io(
                format!("truncate Binary DB lock file {}", self.path.display()),
                err,
            )
        })?;
        self.file.seek(SeekFrom::Start(0)).map_err(|err| {
            BinaryDbError::io(
                format!("seek Binary DB lock file {}", self.path.display()),
                err,
            )
        })?;
        self.file.write_all(bytes).map_err(|err| {
            BinaryDbError::io(
                format!("write Binary DB lock file {}", self.path.display()),
                err,
            )
        })?;
        // The kernel flock is the exclusion authority. These bytes are
        // best-effort live diagnostics, not durable state, so forcing the lock
        // inode to stable storage on every acquire/release only lengthens the
        // protected critical section.
        self.file.flush().map_err(|err| {
            BinaryDbError::io(
                format!("flush Binary DB lock file {}", self.path.display()),
                err,
            )
        })
    }

    fn clear_contents_and_flush(&mut self) -> StoreResult<()> {
        self.replace_contents_and_flush(&[])
    }

    fn release(&mut self) -> StoreResult<()> {
        if self.released {
            return Ok(());
        }
        try_flock(&self.file, libc::LOCK_UN).map_err(|err| {
            BinaryDbError::io(
                format!("release Binary DB process lock {}", self.path.display()),
                err,
            )
        })?;
        self.released = true;
        Ok(())
    }
}

impl Drop for ServerBinaryDbFilesystemProcessLockGuard {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[derive(Debug)]
pub struct BinaryDbReadLockSet {
    locks: Vec<BinaryDbHeldProcessLock>,
    paths: Vec<PathBuf>,
}

impl BinaryDbReadLockSet {
    pub fn try_acquire(root: &StorePath) -> StoreResult<Self> {
        Self::try_acquire_for_scope(root, BinaryDbReadScope::ALL)
    }

    pub fn try_acquire_for_scope(
        root: &StorePath,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<Self> {
        Self::try_acquire_for_scope_with_store(&ServerBinaryDbFilesystemStore, root, read_scope)
    }

    pub fn try_acquire_with_store<S>(files: &S, root: &StorePath) -> StoreResult<Self>
    where
        S: ServerBinaryDbByteStore + ServerBinaryDbLockStore + ?Sized,
    {
        Self::try_acquire_for_scope_with_store(files, root, BinaryDbReadScope::ALL)
    }

    pub fn try_acquire_for_scope_with_store<S>(
        files: &S,
        root: &StorePath,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<Self>
    where
        S: ServerBinaryDbByteStore + ServerBinaryDbLockStore + ?Sized,
    {
        let lock_root = BinaryDbCommandLockSet::prepare_lock_root_with_store(files, root)?;
        let mut locks = Vec::new();
        let mut paths = Vec::new();
        for name in BinaryDbCommandScope::all_write_lock_file_names() {
            if !read_scope.includes_lock_file_name(name) {
                continue;
            }
            let path = lock_root.join(name);
            match BinaryDbHeldProcessLock::try_acquire_shared_with_store(files, &path) {
                Ok(Some(lock)) => {
                    paths.push(path);
                    locks.push(lock);
                }
                Ok(None) => {
                    let mut acquired = Self { locks, paths };
                    let _ = acquired.release();
                    return Err(BinaryDbError::retryable_busy(format!(
                        "Binary DB writer is already active at {}; retry read after writer commits; {}",
                        path.display(),
                        lock_holder_diagnostic(files, &path),
                    )));
                }
                Err(err) => {
                    let mut acquired = Self { locks, paths };
                    let _ = acquired.release();
                    return Err(err);
                }
            }
        }
        Ok(Self { locks, paths })
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn release(&mut self) -> StoreResult<()> {
        let mut first_error = None;
        for lock in self.locks.iter_mut().rev() {
            if let Err(err) = lock.release() {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        self.locks.clear();
        self.paths.clear();
        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

impl Drop for BinaryDbReadLockSet {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[derive(Debug)]
pub struct BinaryDbRecoveryAdmissionLock {
    path: PathBuf,
    lock: BinaryDbHeldProcessLock,
}

impl BinaryDbRecoveryAdmissionLock {
    pub fn try_acquire(root: &StorePath) -> StoreResult<Self> {
        Self::try_acquire_with_store(&ServerBinaryDbFilesystemStore, root)
    }

    pub fn try_acquire_with_store<S>(files: &S, root: &StorePath) -> StoreResult<Self>
    where
        S: ServerBinaryDbByteStore + ServerBinaryDbLockStore + ?Sized,
    {
        let lock_root = BinaryDbCommandLockSet::prepare_lock_root_with_store(files, root)?;
        let path = lock_root.join(BINARY_DB_RECOVERY_ADMISSION_LOCK);
        let Some(lock) = BinaryDbHeldProcessLock::try_acquire_with_store(
            files,
            &path,
            ServerBinaryDbLockMode::Exclusive,
            ServerBinaryDbLockWait::Nonblocking,
            None,
        )?
        else {
            return Err(BinaryDbError::retryable_busy(format!(
                "Binary DB recovery admission is already active at {}",
                path.display()
            )));
        };
        Ok(Self { path, lock })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn release(&mut self) -> StoreResult<()> {
        self.lock.release()
    }
}

impl Drop for BinaryDbRecoveryAdmissionLock {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[derive(Debug)]
pub struct BinaryDbCommandLockSet {
    scope: BinaryDbCommandScope,
    paths: Vec<PathBuf>,
    locks: Vec<BinaryDbHeldProcessLock>,
}

impl BinaryDbCommandLockSet {
    pub fn acquire(root: &StorePath, command_scope: BinaryDbCommandScope) -> StoreResult<Self> {
        Self::acquire_with_store(&ServerBinaryDbFilesystemStore, root, command_scope)
    }

    pub fn acquire_with_store<S>(
        files: &S,
        root: &StorePath,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<Self>
    where
        S: ServerBinaryDbByteStore + ServerBinaryDbLockStore + ?Sized,
    {
        let retry_interval = Duration::from_millis(10);
        loop {
            match Self::try_acquire_with_store(files, root, command_scope) {
                Ok(lock) => return Ok(lock),
                Err(err) if err.is_retryable_busy() => {
                    thread::sleep(retry_interval);
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub fn try_acquire(root: &StorePath, command_scope: BinaryDbCommandScope) -> StoreResult<Self> {
        Self::try_acquire_with_store(&ServerBinaryDbFilesystemStore, root, command_scope)
    }

    pub fn try_acquire_with_store<S>(
        files: &S,
        root: &StorePath,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<Self>
    where
        S: ServerBinaryDbByteStore + ServerBinaryDbLockStore + ?Sized,
    {
        Self::try_acquire_scope_union_with_store(files, root, command_scope, &[command_scope])
    }

    pub fn try_acquire_scope_union(
        root: &StorePath,
        command_scope: BinaryDbCommandScope,
        scopes: &[BinaryDbCommandScope],
    ) -> StoreResult<Self> {
        Self::try_acquire_scope_union_with_store(
            &ServerBinaryDbFilesystemStore,
            root,
            command_scope,
            scopes,
        )
    }

    pub fn try_acquire_scope_union_with_store<S>(
        files: &S,
        root: &StorePath,
        command_scope: BinaryDbCommandScope,
        scopes: &[BinaryDbCommandScope],
    ) -> StoreResult<Self>
    where
        S: ServerBinaryDbByteStore + ServerBinaryDbLockStore + ?Sized,
    {
        let lock_root = Self::prepare_lock_root_with_store(files, root)?;
        let mut lock_names = BTreeSet::new();
        for scope in std::iter::once(&command_scope).chain(scopes.iter()) {
            lock_names.extend(scope.lock_file_names().iter().copied());
        }
        let mut locks = Vec::new();
        let mut paths = Vec::new();
        for name in lock_names {
            let path = lock_root.join(name);
            match BinaryDbHeldProcessLock::try_acquire_exclusive_with_store(
                files,
                &path,
                command_scope,
            ) {
                Ok(Some(lock)) => {
                    paths.push(path);
                    locks.push(lock);
                }
                Ok(None) => {
                    let mut acquired = Self {
                        scope: command_scope,
                        paths,
                        locks,
                    };
                    let _ = acquired.release();
                    return Err(BinaryDbError::retryable_busy(format!(
                        "Binary DB {:?} writer is already active at {}; {}",
                        command_scope,
                        path.display(),
                        lock_holder_diagnostic(files, &path),
                    )));
                }
                Err(err) => {
                    let mut acquired = Self {
                        scope: command_scope,
                        paths,
                        locks,
                    };
                    let _ = acquired.release();
                    return Err(err);
                }
            }
        }
        Ok(Self {
            scope: command_scope,
            paths,
            locks,
        })
    }

    pub fn acquire_queued(
        root: &StorePath,
        command_scope: BinaryDbCommandScope,
        max_wait: Duration,
        retry_interval: Duration,
    ) -> StoreResult<Self> {
        Self::acquire_queued_with_store(
            &ServerBinaryDbFilesystemStore,
            root,
            command_scope,
            max_wait,
            retry_interval,
        )
    }

    pub fn acquire_queued_with_store<S>(
        files: &S,
        root: &StorePath,
        command_scope: BinaryDbCommandScope,
        max_wait: Duration,
        retry_interval: Duration,
    ) -> StoreResult<Self>
    where
        S: ServerBinaryDbByteStore + ServerBinaryDbLockStore + ?Sized,
    {
        let started = Instant::now();
        let retry_interval = retry_interval.max(Duration::from_millis(1));
        loop {
            match Self::try_acquire_with_store(files, root, command_scope) {
                Ok(lock) => return Ok(lock),
                Err(err) if err.is_retryable_busy() && started.elapsed() < max_wait => {
                    thread::sleep(retry_interval);
                }
                Err(err) if err.is_retryable_busy() => {
                    return Err(BinaryDbError::retryable_busy(format!(
                        "timed out waiting for Binary DB {:?} writer lock after {:?}: {}",
                        command_scope, max_wait, err
                    )));
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub fn command_scope(&self) -> BinaryDbCommandScope {
        self.scope
    }

    pub fn scope(&self) -> BinaryDbCommandScope {
        self.command_scope()
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    pub fn lock_paths(&self) -> &[PathBuf] {
        self.paths()
    }

    pub fn release(&mut self) -> StoreResult<()> {
        let mut first_error = None;
        for lock in self.locks.iter_mut().rev() {
            if let Err(err) = lock.release() {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        self.locks.clear();
        self.paths.clear();
        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn prepare_lock_root_with_store<S>(files: &S, root: &StorePath) -> StoreResult<PathBuf>
    where
        S: ServerBinaryDbByteStore + ?Sized,
    {
        let lock_root = binary_db_lock_root(root);
        files.create_parent_dirs(&lock_root.join(".lock-root"))?;
        Ok(lock_root)
    }
}

impl Drop for BinaryDbCommandLockSet {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[derive(Debug)]
struct BinaryDbHeldProcessLock {
    guard: BoxedServerBinaryDbProcessLockGuard,
    active: bool,
    clear_on_release: bool,
}

impl BinaryDbHeldProcessLock {
    fn try_acquire_shared_with_store<S>(files: &S, path: &Path) -> StoreResult<Option<Self>>
    where
        S: ServerBinaryDbLockStore + ?Sized,
    {
        Self::try_acquire_with_store(
            files,
            path,
            ServerBinaryDbLockMode::Shared,
            ServerBinaryDbLockWait::Nonblocking,
            None,
        )
    }

    fn try_acquire_exclusive_with_store<S>(
        files: &S,
        path: &Path,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<Option<Self>>
    where
        S: ServerBinaryDbLockStore + ?Sized,
    {
        Self::try_acquire_with_store(
            files,
            path,
            ServerBinaryDbLockMode::Exclusive,
            ServerBinaryDbLockWait::Nonblocking,
            Some(command_scope),
        )
    }

    fn try_acquire_with_store<S>(
        files: &S,
        path: &Path,
        mode: ServerBinaryDbLockMode,
        wait: ServerBinaryDbLockWait,
        command_scope: Option<BinaryDbCommandScope>,
    ) -> StoreResult<Option<Self>>
    where
        S: ServerBinaryDbLockStore + ?Sized,
    {
        let Some(mut guard) = files.acquire_process_lock(path, mode, wait)? else {
            return Ok(None);
        };
        if let Some(command_scope) = command_scope {
            let contents = format!(
                "scope={command_scope:?}\npid={}\nacquired_unix_ms={}\n",
                std::process::id(),
                unix_time_millis(),
            );
            if let Err(err) = guard.replace_contents_and_flush(contents.as_bytes()) {
                let _ = guard.release();
                return Err(err);
            }
        }
        Ok(Some(Self {
            guard,
            active: true,
            clear_on_release: command_scope.is_some(),
        }))
    }

    pub(super) fn release(&mut self) -> StoreResult<()> {
        if !self.active {
            return Ok(());
        }
        let mut first_error = None;
        if self.clear_on_release {
            if let Err(err) = self.guard.clear_contents_and_flush() {
                first_error = Some(err);
            }
        }
        if let Err(err) = self.guard.release() {
            if first_error.is_none() {
                first_error = Some(err);
            }
        }
        self.active = false;
        match first_error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }
}

fn try_flock(file: &File, operation: libc::c_int) -> std::io::Result<()> {
    let rc = unsafe { libc::flock(file.as_raw_fd(), operation) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn is_lock_busy(err: &std::io::Error) -> bool {
    err.kind() == ErrorKind::WouldBlock
        || err.raw_os_error() == Some(libc::EWOULDBLOCK)
        || err.raw_os_error() == Some(libc::EAGAIN)
}

fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn lock_holder_diagnostic<S>(files: &S, path: &Path) -> String
where
    S: ServerBinaryDbFileStore + ?Sized,
{
    let Ok(contents) = files.read_to_string(path) else {
        return "holder_scope=unknown holder_pid=unknown holder_acquired_unix_ms=unknown holder_held_ms=unknown".to_string();
    };
    let value = |key: &str| {
        contents
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
    };
    let acquired = value("acquired_unix_ms=");
    let held = acquired
        .parse::<u128>()
        .ok()
        .map(|started| unix_time_millis().saturating_sub(started).to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "holder_scope={} holder_pid={} holder_acquired_unix_ms={acquired} holder_held_ms={held}",
        value("scope="),
        value("pid="),
    )
}

impl Drop for BinaryDbHeldProcessLock {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[derive(Debug)]
pub(super) struct BinaryDbWriteLock {
    locks: BinaryDbCommandLockSet,
    acquired_at: Instant,
}

impl BinaryDbWriteLock {
    pub(super) fn try_acquire_scope_union<B>(
        db: &B,
        command_scope: BinaryDbCommandScope,
        scopes: &[BinaryDbCommandScope],
    ) -> StoreResult<Self>
    where
        B: BinaryDb + ?Sized,
    {
        let locks = db.try_acquire_command_scope_union(command_scope, scopes)?;
        Ok(Self {
            locks,
            acquired_at: Instant::now(),
        })
    }

    pub(super) fn held_duration(&self) -> Duration {
        self.acquired_at.elapsed()
    }

    pub(super) fn release(&mut self) -> StoreResult<()> {
        self.locks.release()
    }

    #[cfg(test)]
    pub(super) fn paths(&self) -> &[PathBuf] {
        self.locks.paths()
    }
}

impl Drop for BinaryDbWriteLock {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn binary_db_lock_root(root: &StorePath) -> PathBuf {
    root.as_path().join(".locks").join("binary-db")
}
