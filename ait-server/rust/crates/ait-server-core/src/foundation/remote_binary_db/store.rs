use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerBinaryDbLockMode {
    Shared,
    Exclusive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerBinaryDbLockWait {
    Blocking,
    Nonblocking,
}

pub trait ServerBinaryDbProcessLockGuard: fmt::Debug {
    fn replace_contents_and_flush(&mut self, bytes: &[u8]) -> StoreResult<()>;

    fn clear_contents_and_flush(&mut self) -> StoreResult<()>;

    fn release(&mut self) -> StoreResult<()>;
}

pub type BoxedServerBinaryDbProcessLockGuard = Box<dyn ServerBinaryDbProcessLockGuard + Send>;

pub trait ServerBinaryDbFileStore {
    fn path_exists(&self, path: &Path) -> bool;

    fn read_bytes(&self, path: &Path) -> StoreResult<Vec<u8>>;

    fn read_to_string(&self, path: &Path) -> StoreResult<String>;
}

pub trait ServerBinaryDbByteStore: ServerBinaryDbFileStore {
    fn read_range(&self, path: &Path, offset: u64, len: u32) -> StoreResult<Vec<u8>>;

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>>;

    fn create_parent_dirs(&self, path: &Path) -> StoreResult<()>;

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64>;

    fn overwrite_range(&self, _path: &Path, _offset: u64, _bytes: &[u8]) -> StoreResult<()> {
        Err(BinaryDbError::unsupported(
            "Binary DB byte store does not support range overwrite",
        ))
    }

    fn truncate_file(&self, path: &Path, len: u64) -> StoreResult<()>;

    fn remove_file_if_exists(&self, path: &Path) -> StoreResult<()>;
}

pub trait ServerBinaryDbDurabilityStore: ServerBinaryDbFileStore {
    fn sync_file(&self, path: &Path) -> StoreResult<()>;

    /// Persist file contents and the minimum metadata required to read them.
    ///
    /// This is the journal-append ordering primitive. Implementations may use
    /// a data-only sync; the default retains the stronger full-file sync for
    /// stores that do not expose a distinct operation.
    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        self.sync_file(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()>;
}

pub trait ServerBinaryDbLockStore: ServerBinaryDbFileStore {
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: ServerBinaryDbLockMode,
        wait: ServerBinaryDbLockWait,
    ) -> StoreResult<Option<BoxedServerBinaryDbProcessLockGuard>>;
}

pub trait ServerBinaryDbStore:
    ServerBinaryDbByteStore + ServerBinaryDbDurabilityStore + ServerBinaryDbLockStore
{
}

impl<T> ServerBinaryDbStore for T where
    T: ServerBinaryDbByteStore + ServerBinaryDbDurabilityStore + ServerBinaryDbLockStore
{
}

impl<T> ServerBinaryDbFileStore for &T
where
    T: ServerBinaryDbFileStore + ?Sized,
{
    fn path_exists(&self, path: &Path) -> bool {
        (**self).path_exists(path)
    }

    fn read_bytes(&self, path: &Path) -> StoreResult<Vec<u8>> {
        (**self).read_bytes(path)
    }

    fn read_to_string(&self, path: &Path) -> StoreResult<String> {
        (**self).read_to_string(path)
    }
}

impl<T> ServerBinaryDbByteStore for &T
where
    T: ServerBinaryDbByteStore + ?Sized,
{
    fn read_range(&self, path: &Path, offset: u64, len: u32) -> StoreResult<Vec<u8>> {
        (**self).read_range(path, offset, len)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        (**self).metadata_len(path)
    }

    fn create_parent_dirs(&self, path: &Path) -> StoreResult<()> {
        (**self).create_parent_dirs(path)
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        (**self).append_bytes(path, bytes)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> StoreResult<()> {
        (**self).overwrite_range(path, offset, bytes)
    }

    fn truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        (**self).truncate_file(path, len)
    }

    fn remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        (**self).remove_file_if_exists(path)
    }
}

impl<T> ServerBinaryDbDurabilityStore for &T
where
    T: ServerBinaryDbDurabilityStore + ?Sized,
{
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        (**self).sync_file(path)
    }

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        (**self).sync_file_data(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        (**self).sync_directory(path)
    }
}

impl<T> ServerBinaryDbLockStore for &T
where
    T: ServerBinaryDbLockStore + ?Sized,
{
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: ServerBinaryDbLockMode,
        wait: ServerBinaryDbLockWait,
    ) -> StoreResult<Option<BoxedServerBinaryDbProcessLockGuard>> {
        (**self).acquire_process_lock(path, mode, wait)
    }
}
