use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use std::{iter, os::windows::ffi::OsStrExt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileIoErrorKind {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    InvalidInput,
    InvalidData,
    Interrupted,
    WouldBlock,
    UnexpectedEof,
    WriteZero,
    Unsupported,
    Lock,
    Durability,
    Utf8,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileIoError {
    kind: FileIoErrorKind,
    message: String,
}

pub type FileIoResult<T> = Result<T, FileIoError>;

impl FileIoError {
    pub fn new(kind: FileIoErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(FileIoErrorKind::Other, message)
    }

    pub fn from_io(err: io::Error) -> Self {
        Self::new(file_io_error_kind(err.kind()), err.to_string())
    }

    pub fn from_io_message(message: impl Into<String>, err: io::Error) -> Self {
        Self::new(file_io_error_kind(err.kind()), message)
    }

    pub fn kind(&self) -> FileIoErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for FileIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FileIoError {}

impl From<io::Error> for FileIoError {
    fn from(err: io::Error) -> Self {
        Self::from_io(err)
    }
}

impl From<String> for FileIoError {
    fn from(message: String) -> Self {
        Self::other(message)
    }
}

impl From<&str> for FileIoError {
    fn from(message: &str) -> Self {
        Self::other(message)
    }
}

fn file_io_error_kind(kind: io::ErrorKind) -> FileIoErrorKind {
    match kind {
        io::ErrorKind::NotFound => FileIoErrorKind::NotFound,
        io::ErrorKind::PermissionDenied => FileIoErrorKind::PermissionDenied,
        io::ErrorKind::AlreadyExists => FileIoErrorKind::AlreadyExists,
        io::ErrorKind::InvalidInput => FileIoErrorKind::InvalidInput,
        io::ErrorKind::InvalidData => FileIoErrorKind::InvalidData,
        io::ErrorKind::Interrupted => FileIoErrorKind::Interrupted,
        io::ErrorKind::WouldBlock => FileIoErrorKind::WouldBlock,
        io::ErrorKind::UnexpectedEof => FileIoErrorKind::UnexpectedEof,
        io::ErrorKind::WriteZero => FileIoErrorKind::WriteZero,
        io::ErrorKind::Unsupported => FileIoErrorKind::Unsupported,
        _ => FileIoErrorKind::Other,
    }
}

#[cfg(unix)]
pub(crate) fn sync_filesystem_directory(path: &Path) -> io::Result<()> {
    File::open(path).and_then(|directory| directory.sync_all())
}

#[cfg(windows)]
pub(crate) fn sync_filesystem_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "directory sync target is not a directory: {}",
                path.display()
            ),
        ));
    }
    // Windows does not expose Unix directory fsync semantics through
    // `std::fs::File`. File contents are synced before publication, and the
    // closed-handle rename is the durability boundary available to this
    // adapter.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn sync_filesystem_directory(path: &Path) -> io::Result<()> {
    File::open(path).and_then(|directory| directory.sync_all())
}

pub trait FileIoStore {
    fn home_dir(&self) -> Option<PathBuf>;
    fn path_exists(&self, path: &Path) -> bool;
    fn list_directory_paths(&self, path: &Path) -> FileIoResult<Vec<PathBuf>> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!("Directory listing is not supported for {}", path.display()),
        ))
    }
    fn read_bytes(&self, path: &Path) -> FileIoResult<Vec<u8>> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!("File byte reads are not supported for {}", path.display()),
        ))
    }
    fn read_to_string(&self, path: &Path) -> FileIoResult<String>;
    fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()>;

    fn write_string_atomically(
        &self,
        path: &Path,
        text: &str,
        publish_label: &str,
    ) -> FileIoResult<()>;
}

impl<T> FileIoStore for &T
where
    T: FileIoStore + ?Sized,
{
    fn home_dir(&self) -> Option<PathBuf> {
        (**self).home_dir()
    }

    fn path_exists(&self, path: &Path) -> bool {
        (**self).path_exists(path)
    }

    fn list_directory_paths(&self, path: &Path) -> FileIoResult<Vec<PathBuf>> {
        (**self).list_directory_paths(path)
    }

    fn read_bytes(&self, path: &Path) -> FileIoResult<Vec<u8>> {
        (**self).read_bytes(path)
    }

    fn read_to_string(&self, path: &Path) -> FileIoResult<String> {
        (**self).read_to_string(path)
    }

    fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()> {
        (**self).write_string(path, text)
    }

    fn write_string_atomically(
        &self,
        path: &Path,
        text: &str,
        publish_label: &str,
    ) -> FileIoResult<()> {
        (**self).write_string_atomically(path, text, publish_label)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileIoLockMode {
    Shared,
    Exclusive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileIoLockWait {
    Blocking,
    Nonblocking,
}

pub trait FileIoProcessLockGuard: fmt::Debug {
    fn replace_contents_and_flush(&mut self, bytes: &[u8]) -> FileIoResult<()>;
    fn clear_contents_and_flush(&mut self) -> FileIoResult<()>;
    fn release(&mut self) -> FileIoResult<()>;
}

pub type BoxedFileIoProcessLockGuard = Box<dyn FileIoProcessLockGuard + Send>;

pub trait FileIoByteStore: FileIoStore {
    fn write_bytes(&self, path: &Path, _bytes: &[u8]) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!("File byte writes are not supported for {}", path.display()),
        ))
    }

    fn write_bytes_atomically(
        &self,
        path: &Path,
        _bytes: &[u8],
        _publish_label: &str,
    ) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!(
                "Atomic file byte writes are not supported for {}",
                path.display()
            ),
        ))
    }

    /// Stages a complete replacement outside the target directory before an
    /// atomic same-filesystem rename.
    ///
    /// An error from this method guarantees that the target was not replaced.
    /// Directory durability is deliberately owned by the caller after the
    /// successful rename so it can distinguish a crossed commit point from a
    /// pre-publication failure.
    fn write_bytes_atomically_from_directory(
        &self,
        path: &Path,
        _staging_directory: &Path,
        _bytes: &[u8],
        _publish_label: &str,
    ) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!(
                "Staged atomic file byte writes are not supported for {}",
                path.display()
            ),
        ))
    }

    fn read_range(&self, path: &Path, offset: u64, len: u32) -> FileIoResult<Vec<u8>> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!(
                "File range reads are not supported for {} at offset {offset} length {len}",
                path.display()
            ),
        ))
    }

    fn metadata_len(&self, path: &Path) -> FileIoResult<Option<u64>> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!(
                "File length metadata is not supported for {}",
                path.display()
            ),
        ))
    }

    fn create_parent_dirs(&self, path: &Path) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!(
                "Parent directory creation is not supported for {}",
                path.display()
            ),
        ))
    }

    fn append_bytes(&self, path: &Path, _bytes: &[u8]) -> FileIoResult<u64> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!("File byte appends are not supported for {}", path.display()),
        ))
    }

    fn overwrite_range(&self, path: &Path, offset: u64, _bytes: &[u8]) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!(
                "File range overwrites are not supported for {} at offset {offset}",
                path.display()
            ),
        ))
    }

    fn truncate_file(&self, path: &Path, len: u64) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!(
                "File truncation is not supported for {} to length {len}",
                path.display()
            ),
        ))
    }

    fn remove_file_if_exists(&self, path: &Path) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!("File removal is not supported for {}", path.display()),
        ))
    }
}

pub trait FileIoDurabilityStore: FileIoStore {
    fn sync_file(&self, path: &Path) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!("File fsync is not supported for {}", path.display()),
        ))
    }

    fn sync_dir(&self, path: &Path) -> FileIoResult<()> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!("Directory fsync is not supported for {}", path.display()),
        ))
    }
}

pub trait FileIoLockStore: FileIoStore {
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: FileIoLockMode,
        wait: FileIoLockWait,
    ) -> FileIoResult<Option<BoxedFileIoProcessLockGuard>> {
        Err(FileIoError::new(
            FileIoErrorKind::Unsupported,
            format!(
                "Process locks are not supported for {} ({mode:?}, {wait:?})",
                path.display()
            ),
        ))
    }
}

impl<T> FileIoByteStore for &T
where
    T: FileIoByteStore + ?Sized,
{
    fn write_bytes(&self, path: &Path, bytes: &[u8]) -> FileIoResult<()> {
        (**self).write_bytes(path, bytes)
    }

    fn write_bytes_atomically(
        &self,
        path: &Path,
        bytes: &[u8],
        publish_label: &str,
    ) -> FileIoResult<()> {
        (**self).write_bytes_atomically(path, bytes, publish_label)
    }

    fn write_bytes_atomically_from_directory(
        &self,
        path: &Path,
        staging_directory: &Path,
        bytes: &[u8],
        publish_label: &str,
    ) -> FileIoResult<()> {
        (**self).write_bytes_atomically_from_directory(
            path,
            staging_directory,
            bytes,
            publish_label,
        )
    }

    fn read_range(&self, path: &Path, offset: u64, len: u32) -> FileIoResult<Vec<u8>> {
        (**self).read_range(path, offset, len)
    }

    fn metadata_len(&self, path: &Path) -> FileIoResult<Option<u64>> {
        (**self).metadata_len(path)
    }

    fn create_parent_dirs(&self, path: &Path) -> FileIoResult<()> {
        (**self).create_parent_dirs(path)
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> FileIoResult<u64> {
        (**self).append_bytes(path, bytes)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> FileIoResult<()> {
        (**self).overwrite_range(path, offset, bytes)
    }

    fn truncate_file(&self, path: &Path, len: u64) -> FileIoResult<()> {
        (**self).truncate_file(path, len)
    }

    fn remove_file_if_exists(&self, path: &Path) -> FileIoResult<()> {
        (**self).remove_file_if_exists(path)
    }
}

impl<T> FileIoDurabilityStore for &T
where
    T: FileIoDurabilityStore + ?Sized,
{
    fn sync_file(&self, path: &Path) -> FileIoResult<()> {
        (**self).sync_file(path)
    }

    fn sync_dir(&self, path: &Path) -> FileIoResult<()> {
        (**self).sync_dir(path)
    }
}

impl<T> FileIoLockStore for &T
where
    T: FileIoLockStore + ?Sized,
{
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: FileIoLockMode,
        wait: FileIoLockWait,
    ) -> FileIoResult<Option<BoxedFileIoProcessLockGuard>> {
        (**self).acquire_process_lock(path, mode, wait)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FilesystemFileIoStore;

impl FileIoStore for FilesystemFileIoStore {
    fn home_dir(&self) -> Option<PathBuf> {
        env::var_os("HOME").map(PathBuf::from)
    }

    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn list_directory_paths(&self, path: &Path) -> FileIoResult<Vec<PathBuf>> {
        let mut paths = fs::read_dir(path)
            .map_err(FileIoError::from)?
            .map(|entry| entry.map(|entry| entry.path()).map_err(FileIoError::from))
            .collect::<FileIoResult<Vec<_>>>()?;
        paths.sort();
        Ok(paths)
    }

    fn read_bytes(&self, path: &Path) -> FileIoResult<Vec<u8>> {
        fs::read(path).map_err(FileIoError::from)
    }

    fn read_to_string(&self, path: &Path) -> FileIoResult<String> {
        fs::read_to_string(path).map_err(FileIoError::from)
    }

    fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()> {
        fs::write(path, text).map_err(FileIoError::from)
    }

    fn write_string_atomically(
        &self,
        path: &Path,
        text: &str,
        publish_label: &str,
    ) -> FileIoResult<()> {
        write_bytes_atomically_to_filesystem(path, text.as_bytes(), publish_label)
    }
}

impl FileIoByteStore for FilesystemFileIoStore {
    fn write_bytes(&self, path: &Path, bytes: &[u8]) -> FileIoResult<()> {
        fs::write(path, bytes).map_err(FileIoError::from)
    }

    fn write_bytes_atomically(
        &self,
        path: &Path,
        bytes: &[u8],
        publish_label: &str,
    ) -> FileIoResult<()> {
        write_bytes_atomically_to_filesystem(path, bytes, publish_label)
    }

    fn write_bytes_atomically_from_directory(
        &self,
        path: &Path,
        staging_directory: &Path,
        bytes: &[u8],
        publish_label: &str,
    ) -> FileIoResult<()> {
        write_bytes_atomically_from_directory_to_filesystem(
            path,
            staging_directory,
            bytes,
            publish_label,
        )
    }

    fn read_range(&self, path: &Path, offset: u64, len: u32) -> FileIoResult<Vec<u8>> {
        let mut file = File::open(path).map_err(FileIoError::from)?;
        file.seek(SeekFrom::Start(offset))
            .map_err(FileIoError::from)?;
        let len = usize::try_from(len).map_err(|_| {
            FileIoError::new(
                FileIoErrorKind::InvalidInput,
                format!("range length overflows address space: {len}"),
            )
        })?;
        let mut bytes = vec![0_u8; len];
        file.read_exact(&mut bytes).map_err(FileIoError::from)?;
        Ok(bytes)
    }

    fn metadata_len(&self, path: &Path) -> FileIoResult<Option<u64>> {
        match fs::metadata(path) {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(FileIoError::from(err)),
        }
    }

    fn create_parent_dirs(&self, path: &Path) -> FileIoResult<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(FileIoError::from)?;
        }
        Ok(())
    }

    fn append_bytes(&self, path: &Path, bytes: &[u8]) -> FileIoResult<u64> {
        self.create_parent_dirs(path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .map_err(FileIoError::from)?;
        let offset = file.seek(SeekFrom::End(0)).map_err(FileIoError::from)?;
        file.write_all(bytes).map_err(FileIoError::from)?;
        Ok(offset)
    }

    fn overwrite_range(&self, path: &Path, offset: u64, bytes: &[u8]) -> FileIoResult<()> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(FileIoError::from)?;
        let file_len = file.metadata().map_err(FileIoError::from)?.len();
        let end = offset
            .checked_add(u64::try_from(bytes.len()).map_err(|_| {
                FileIoError::new(
                    FileIoErrorKind::InvalidInput,
                    format!("overwrite length overflows u64: {}", bytes.len()),
                )
            })?)
            .ok_or_else(|| {
                FileIoError::new(
                    FileIoErrorKind::InvalidInput,
                    format!("overwrite range overflows for {}", path.display()),
                )
            })?;
        if end > file_len {
            return Err(FileIoError::new(
                FileIoErrorKind::InvalidInput,
                format!(
                    "overwrite range {offset}..{end} exceeds {} length {file_len}",
                    path.display()
                ),
            ));
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(FileIoError::from)?;
        file.write_all(bytes).map_err(FileIoError::from)
    }

    fn truncate_file(&self, path: &Path, len: u64) -> FileIoResult<()> {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(FileIoError::from)?;
        file.set_len(len).map_err(FileIoError::from)
    }

    fn remove_file_if_exists(&self, path: &Path) -> FileIoResult<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(FileIoError::from(err)),
        }
    }
}

impl FileIoDurabilityStore for FilesystemFileIoStore {
    fn sync_file(&self, path: &Path) -> FileIoResult<()> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .and_then(|file| file.sync_all())
            .map_err(|err| {
                FileIoError::new(
                    FileIoErrorKind::Durability,
                    format!("Failed to sync file {}: {err}", path.display()),
                )
            })
    }

    fn sync_dir(&self, path: &Path) -> FileIoResult<()> {
        sync_filesystem_directory(path).map_err(|err| {
            FileIoError::new(
                FileIoErrorKind::Durability,
                format!("Failed to sync directory {}: {err}", path.display()),
            )
        })
    }
}

impl FileIoLockStore for FilesystemFileIoStore {
    fn acquire_process_lock(
        &self,
        path: &Path,
        mode: FileIoLockMode,
        wait: FileIoLockWait,
    ) -> FileIoResult<Option<BoxedFileIoProcessLockGuard>> {
        self.create_parent_dirs(path)?;
        let file = OpenOptions::new()
            .create(true)
            // Preserve an existing lock payload until exclusive ownership is acquired.
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(FileIoError::from)?;
        if !lock_file(&file, mode, wait)? {
            return Ok(None);
        }
        Ok(Some(Box::new(FilesystemProcessLockGuard {
            file,
            released: false,
        })))
    }
}

#[derive(Debug)]
struct FilesystemProcessLockGuard {
    file: File,
    released: bool,
}

impl FileIoProcessLockGuard for FilesystemProcessLockGuard {
    fn replace_contents_and_flush(&mut self, bytes: &[u8]) -> FileIoResult<()> {
        self.file.set_len(0).map_err(FileIoError::from)?;
        self.file
            .seek(SeekFrom::Start(0))
            .map_err(FileIoError::from)?;
        self.file.write_all(bytes).map_err(FileIoError::from)?;
        self.file.flush().map_err(FileIoError::from)
    }

    fn clear_contents_and_flush(&mut self) -> FileIoResult<()> {
        self.replace_contents_and_flush(&[])
    }

    fn release(&mut self) -> FileIoResult<()> {
        if self.released {
            return Ok(());
        }
        unlock_file(&self.file)?;
        self.released = true;
        Ok(())
    }
}

impl Drop for FilesystemProcessLockGuard {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn lock_file(file: &File, mode: FileIoLockMode, wait: FileIoLockWait) -> FileIoResult<bool> {
    let result = match (mode, wait) {
        (FileIoLockMode::Shared, FileIoLockWait::Blocking) => fs2::FileExt::lock_shared(file),
        (FileIoLockMode::Exclusive, FileIoLockWait::Blocking) => fs2::FileExt::lock_exclusive(file),
        (FileIoLockMode::Shared, FileIoLockWait::Nonblocking) => {
            fs2::FileExt::try_lock_shared(file)
        }
        (FileIoLockMode::Exclusive, FileIoLockWait::Nonblocking) => {
            fs2::FileExt::try_lock_exclusive(file)
        }
    };
    match result {
        Ok(()) => Ok(true),
        Err(err) if wait == FileIoLockWait::Nonblocking && is_lock_contended(&err) => Ok(false),
        Err(err) => Err(FileIoError::new(
            FileIoErrorKind::Lock,
            format!("Failed to acquire process file lock: {err}"),
        )),
    }
}

fn unlock_file(file: &File) -> FileIoResult<()> {
    fs2::FileExt::unlock(file).map_err(|err| {
        FileIoError::new(
            FileIoErrorKind::Lock,
            format!("Failed to release process file lock: {err}"),
        )
    })
}

fn is_lock_contended(err: &io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    err.kind() == io::ErrorKind::WouldBlock
        || err
            .raw_os_error()
            .is_some_and(|code| expected.raw_os_error() == Some(code))
}

fn monotonic_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

fn write_bytes_atomically_to_filesystem(
    path: &Path,
    bytes: &[u8],
    publish_label: &str,
) -> FileIoResult<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|err| {
            FileIoError::from_io_message(
                format!("Failed to create {}: {err}", parent.display()),
                err,
            )
        })?;
    }
    let staging_directory = path.parent().unwrap_or_else(|| Path::new("."));
    write_bytes_atomically_from_directory_to_filesystem(
        path,
        staging_directory,
        bytes,
        publish_label,
    )
}

fn write_bytes_atomically_from_directory_to_filesystem(
    path: &Path,
    staging_directory: &Path,
    bytes: &[u8],
    publish_label: &str,
) -> FileIoResult<()> {
    fs::create_dir_all(staging_directory).map_err(|err| {
        FileIoError::from_io_message(
            format!(
                "Failed to create atomic staging directory {}: {err}",
                staging_directory.display()
            ),
            err,
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let temp_path = staging_directory.join(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        monotonic_nanos()
    ));
    let write_result = (|| -> FileIoResult<()> {
        let mut file = fs::File::create(&temp_path).map_err(|err| {
            FileIoError::from_io_message(
                format!("Failed to create {}: {err}", temp_path.display()),
                err,
            )
        })?;
        file.write_all(bytes).map_err(|err| {
            FileIoError::from_io_message(
                format!("Failed to write {}: {err}", temp_path.display()),
                err,
            )
        })?;
        file.sync_all().map_err(|err| {
            FileIoError::new(
                FileIoErrorKind::Durability,
                format!("Failed to sync {}: {err}", temp_path.display()),
            )
        })?;
        replace_file_atomically(&temp_path, path).map_err(|err| {
            FileIoError::from_io_message(
                format!(
                    "Failed to publish {publish_label} {} -> {}: {err}",
                    temp_path.display(),
                    path.display()
                ),
                err,
            )
        })?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

#[cfg(not(windows))]
fn replace_file_atomically(source: &Path, target: &Path) -> io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn replace_file_atomically(source: &Path, target: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
