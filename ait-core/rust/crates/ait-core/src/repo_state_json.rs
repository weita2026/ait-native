use crate::file_io::{FileIoStore, FilesystemFileIoStore};
use crate::json_support::JsonValue;
use crate::json_support::{
    expand_home_path_with_file_io_store, read_json_or_null_with_file_io_store, JsonCodec,
    JsonEncodeOptions,
};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek};
use std::path::Path;
use std::rc::{Rc, Weak};

const CONFIG_BUSY: &str =
    "operation_busy: Repository configuration is in use; retry after preparation completes";

#[derive(Clone, Eq, Hash, PartialEq)]
struct ConfigFileIdentity {
    #[cfg(unix)]
    inode: (u64, u64),
    #[cfg(windows)]
    inode: (u32, u32, u32),
    #[cfg(not(any(unix, windows)))]
    path: std::path::PathBuf,
}

struct HeldConfigFile {
    file: RefCell<File>,
    identity: ConfigFileIdentity,
}

thread_local! {
    // Only this thread may borrow an owning handle. Another thread must take
    // its own OS lock, including while a writer has temporarily empty bytes.
    static CONFIG_LEASES: RefCell<HashMap<ConfigFileIdentity, Vec<Weak<HeldConfigFile>>>> =
        RefCell::new(HashMap::new());
}

/// Internal coordination shared by native config mutation and admission.
/// Rc deliberately keeps a lease on its owning thread.
#[doc(hidden)]
pub struct RootConfigLease(Rc<HeldConfigFile>);

impl RootConfigLease {
    pub fn try_acquire(path: &Path, exclusive: bool, writable: bool) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(writable)
            .open(path)
            .map_err(|error| error.to_string())?;
        let result = if exclusive {
            fs2::FileExt::try_lock_exclusive(&file)
        } else {
            fs2::FileExt::try_lock_shared(&file)
        };
        result.map_err(|_| CONFIG_BUSY.to_string())?;
        Self::register(file, path).map_err(|error| error.to_string())
    }

    pub fn read(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let identity = config_file_identity(&file, path)?;
        if let Some(owner) = Self::current_owner(&identity) {
            // In particular, do not downgrade an outer exclusive lease or
            // read via a second handle under Windows byte-range locking.
            return Ok(Self(owner));
        }
        fs2::FileExt::lock_shared(&file)?;
        Self::register(file, path)
    }

    /// Internal publication waits for concurrent readers instead of failing a
    /// valid workflow. A lease owned by this thread must still return Busy:
    /// blocking here would upgrade a read or reenter an active mutation.
    pub fn acquire_mutation(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        let identity = config_file_identity(&file, path).map_err(|error| error.to_string())?;
        if Self::current_owner(&identity).is_some() {
            return Err(CONFIG_BUSY.to_string());
        }
        fs2::FileExt::lock_exclusive(&file).map_err(|error| error.to_string())?;
        Self::register(file, path).map_err(|error| error.to_string())
    }

    fn current_owner(identity: &ConfigFileIdentity) -> Option<Rc<HeldConfigFile>> {
        CONFIG_LEASES.with(|leases| {
            leases
                .borrow()
                .get(identity)
                .and_then(|owners| owners.iter().find_map(Weak::upgrade))
        })
    }

    pub fn with_file<T>(&self, use_file: impl FnOnce(&mut File) -> T) -> T {
        use_file(&mut self.0.file.borrow_mut())
    }

    fn register(file: File, path: &Path) -> io::Result<Self> {
        let identity = config_file_identity(&file, path)?;
        let owner = Rc::new(HeldConfigFile {
            file: RefCell::new(file),
            identity,
        });
        CONFIG_LEASES.with(|leases| {
            leases
                .borrow_mut()
                .entry(owner.identity.clone())
                .or_default()
                .push(Rc::downgrade(&owner));
        });
        Ok(Self(owner))
    }
}

impl Drop for RootConfigLease {
    fn drop(&mut self) {
        if Rc::strong_count(&self.0) != 1 {
            return;
        }
        let _ = CONFIG_LEASES.try_with(|leases| {
            let mut leases = leases.borrow_mut();
            if let Some(owners) = leases.get_mut(&self.0.identity) {
                owners.retain(|owner| !owner.ptr_eq(&Rc::downgrade(&self.0)));
                if owners.is_empty() {
                    leases.remove(&self.0.identity);
                }
            }
        });
    }
}

fn config_file_identity(file: &File, _path: &Path) -> io::Result<ConfigFileIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(ConfigFileIdentity {
            inode: (metadata.dev(), metadata.ino()),
        })
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        };
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // The handle stays open, and a successful call initializes this record.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), information.as_mut_ptr()) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        let information = unsafe { information.assume_init() };
        Ok(ConfigFileIdentity {
            inode: (
                information.dwVolumeSerialNumber,
                information.nFileIndexHigh,
                information.nFileIndexLow,
            ),
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Ok(ConfigFileIdentity {
            path: _path.canonicalize()?,
        })
    }
}

#[doc(hidden)]
pub fn read_root_config_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let lease = RootConfigLease::read(path)?;
    lease.with_file(|file| {
        file.rewind()?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

#[cfg(test)]
mod config_lease_tests {
    use super::*;

    #[test]
    fn nested_reads_preserve_outer_ownership_and_other_thread_exclusion() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.json");
        std::fs::write(&path, br#"{"repo_name":"owner"}"#).unwrap();
        for exclusive in [false, true] {
            let outer = RootConfigLease::try_acquire(&path, exclusive, false).unwrap();
            assert_eq!(
                read_root_config_bytes(&path).unwrap(),
                br#"{"repo_name":"owner"}"#
            );
            #[cfg(unix)]
            {
                let alias = temp.path().join(format!("alias-{exclusive}.json"));
                std::fs::hard_link(&path, &alias).unwrap();
                assert_eq!(
                    read_root_config_bytes(&alias).unwrap(),
                    br#"{"repo_name":"owner"}"#
                );
            }
            assert!(RootConfigLease::try_acquire(&path, true, true)
                .err()
                .unwrap()
                .contains("operation_busy"));
            assert!(RootConfigLease::acquire_mutation(&path)
                .err()
                .unwrap()
                .contains("operation_busy"));
            std::thread::scope(|scope| {
                scope
                    .spawn(|| {
                        assert!(RootConfigLease::try_acquire(&path, true, true).is_err());
                        if exclusive {
                            assert!(RootConfigLease::try_acquire(&path, false, false).is_err());
                        }
                    })
                    .join()
                    .unwrap();
            });
            let contender = File::open(&path).unwrap();
            assert!(fs2::FileExt::try_lock_exclusive(&contender).is_err());
            if exclusive {
                assert!(fs2::FileExt::try_lock_shared(&contender).is_err());
            }
            let inner = RootConfigLease::read(&path).unwrap();
            drop(outer);
            assert!(fs2::FileExt::try_lock_exclusive(&contender).is_err());
            drop(inner);
            fs2::FileExt::try_lock_exclusive(&contender).unwrap();
        }
    }
}

pub struct RepoStateJson<S> {
    store: S,
}

impl<S> RepoStateJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RepoStateJson<FilesystemFileIoStore> {
    pub fn filesystem() -> Self {
        Self::new(FilesystemFileIoStore)
    }
}

impl<S> RepoStateJson<S>
where
    S: FileIoStore,
{
    pub fn read_state_json_file(&self, path_value: &str, label: &str) -> Result<JsonValue, String> {
        read_state_json_file_with_store(&self.store, path_value, label)
    }

    pub fn write_state_json_file(
        &self,
        path_value: &str,
        payload: &JsonValue,
        label: &str,
    ) -> Result<(), String> {
        write_state_json_file_with_store(&self.store, path_value, payload, label)
    }
}

#[cfg(test)]
fn read_state_json_file_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    label: &str,
) -> Result<JsonValue, String>
where
    S: FileIoStore + ?Sized,
{
    RepoStateJson::new(store).read_state_json_file(path_value, label)
}

fn read_state_json_file(path_value: &str, label: &str) -> Result<JsonValue, String> {
    RepoStateJson::filesystem().read_state_json_file(path_value, label)
}

#[cfg(test)]
fn write_state_json_file_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    payload: &JsonValue,
    label: &str,
) -> Result<(), String>
where
    S: FileIoStore + ?Sized,
{
    RepoStateJson::new(store).write_state_json_file(path_value, payload, label)
}

fn write_state_json_file(path_value: &str, payload: &JsonValue, label: &str) -> Result<(), String> {
    RepoStateJson::filesystem().write_state_json_file(path_value, payload, label)
}

pub fn read_repo_config_json_file(path_value: &str) -> Result<JsonValue, String> {
    read_state_json_file(path_value, "repo config")
}

pub fn write_repo_config_json_file(path_value: &str, payload: &JsonValue) -> Result<(), String> {
    write_state_json_file(path_value, payload, "repo config")
}

pub fn read_worktree_config_json_file(path_value: &str) -> Result<JsonValue, String> {
    read_state_json_file(path_value, "worktree config")
}

pub fn write_worktree_config_json_file(
    path_value: &str,
    payload: &JsonValue,
) -> Result<(), String> {
    write_state_json_file(path_value, payload, "worktree config")
}

pub fn read_worktree_metadata_json_file(path_value: &str) -> Result<JsonValue, String> {
    read_state_json_file(path_value, "worktree metadata")
}

pub fn write_worktree_metadata_json_file(
    path_value: &str,
    payload: &JsonValue,
) -> Result<(), String> {
    write_state_json_file(path_value, payload, "worktree metadata")
}

fn read_state_json_file_with_store<S>(
    store: &S,
    path_value: &str,
    label: &str,
) -> Result<JsonValue, String>
where
    S: FileIoStore + ?Sized,
{
    read_json_or_null_with_file_io_store(store, path_value, label)
}

fn write_state_json_file_with_store<S>(
    store: &S,
    path_value: &str,
    payload: &JsonValue,
    label: &str,
) -> Result<(), String>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    let text = encode_state_json(payload, &path, label)?;
    store
        .write_string(&path, &text)
        .map_err(|err| format!("Failed to write {label} JSON {}: {err}", path.display()))
}

fn encode_state_json<T>(payload: &T, path: &std::path::Path, label: &str) -> Result<String, String>
where
    T: Serialize + ?Sized,
{
    JsonCodec::encode_serializable_with_error_prefix(
        payload,
        JsonEncodeOptions::pretty(),
        &format!("Failed to encode {label} JSON {}", path.display()),
    )
    .map_err(String::from)
}

#[cfg(test)]
mod tests;
