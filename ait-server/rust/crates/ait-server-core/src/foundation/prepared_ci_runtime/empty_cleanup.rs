use super::*;

/// Physical cleanup only. The caller proves exact runtime/native scope and
/// execution/download exclusion. No missing evidence becomes logical success.
pub fn remove_empty_managed_runtime_container(root: &Path, path: &Path) -> StoreResult<bool> {
    remove_empty_with(root, path, || Ok(()))
}

#[cfg(unix)]
fn owned(file: &fs::File) -> StoreResult<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file
        .metadata()
        .map_err(|e| BinaryDbError::io("inspect runtime container handle", e))?;
    // SAFETY: geteuid has no arguments or memory-safety preconditions.
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err(invalid(
            "Runtime container parent is not exclusively Server-owned",
        ));
    }
    Ok(())
}

/// Open a no-follow root owned by the actual Server process, rather than
/// treating the owner of an arbitrary pre-existing root as Server authority.
#[cfg(unix)]
pub fn open_owned_managed_runtime_root(root: &Path) -> StoreResult<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let root = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)
        .map_err(|e| BinaryDbError::io("open managed runtime root without following links", e))?;
    owned(&root)?;
    Ok(root)
}

#[cfg(not(any(unix, windows)))]
fn remove_empty_with(
    _root: &Path,
    _path: &Path,
    _before_remove: impl FnOnce() -> StoreResult<()>,
) -> StoreResult<bool> {
    // Keep the container when this platform cannot prove handle-relative scope.
    Err(BinaryDbError::retryable_busy(
        "Handle-relative empty runtime cleanup is unavailable on this platform",
    ))
}

#[cfg(windows)]
fn remove_empty_with(
    root: &Path,
    path: &Path,
    before_remove: impl FnOnce() -> StoreResult<()>,
) -> StoreResult<bool> {
    super::windows_fs::remove_empty(root, path, || {
        before_remove().map_err(std::io::Error::other)
    })
    .map_err(|error| BinaryDbError::io("remove exact empty Windows runtime container", error))
}

#[cfg(unix)]
fn remove_empty_with(
    root: &Path,
    path: &Path,
    before_remove: impl FnOnce() -> StoreResult<()>,
) -> StoreResult<bool> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    fn name(value: &std::ffi::OsStr) -> StoreResult<CString> {
        CString::new(value.as_bytes()).map_err(|_| invalid("Runtime container name contains NUL"))
    }
    fn open_child(parent: &fs::File, value: &CString) -> std::io::Result<fs::File> {
        // SAFETY: parent owns a valid fd and value is a live NUL-terminated name.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                value.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: openat returned a new owned descriptor.
            Ok(unsafe { fs::File::from_raw_fd(fd) })
        }
    }

    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid("Runtime container escaped its managed root"))?;
    let mut components = relative.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid(
            "Runtime container path is not a dedicated managed child",
        ));
    }
    let leaf = name(components.pop().unwrap().as_os_str())?;
    let mut parent = open_owned_managed_runtime_root(root)?;
    for component in components {
        match open_child(&parent, &name(component.as_os_str())?) {
            Ok(child) => {
                owned(&child)?;
                parent = child;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                parent
                    .sync_all()
                    .map_err(|e| BinaryDbError::io("sync absent runtime container", e))?;
                return Ok(true);
            }
            Err(e) => {
                return Err(BinaryDbError::io(
                    "open runtime container parent without following links",
                    e,
                ))
            }
        }
    }
    let child = match open_child(&parent, &leaf) {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            parent
                .sync_all()
                .map_err(|e| BinaryDbError::io("sync absent runtime container", e))?;
            return Ok(true);
        }
        Err(e) => {
            return Err(BinaryDbError::io(
                "open runtime container without following links",
                e,
            ))
        }
    };
    owned(&child)?;
    if fs::read_dir(path)
        .map_err(|e| BinaryDbError::io("inspect empty runtime container", e))?
        .next()
        .transpose()
        .map_err(|e| BinaryDbError::io("inspect runtime container entry", e))?
        .is_some()
    {
        return Ok(false);
    }
    before_remove()?;
    // SAFETY: this uses the verified parent descriptor and one component only.
    // AT_REMOVEDIR rejects files/links and atomically fails for a nonempty dir.
    if unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ENOTEMPTY | libc::EEXIST))
            || fs::read_dir(path)
                .ok()
                .is_some_and(|mut entries| entries.next().is_some())
        {
            return Ok(false);
        }
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(BinaryDbError::io(
                "remove empty managed runtime container",
                error,
            ));
        }
    }
    parent
        .sync_all()
        .map_err(|e| BinaryDbError::io("sync removed runtime container", e))?;
    Ok(true)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[test]
    fn retained_runtime_root_requires_process_ownership_and_no_follow() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        assert!(open_owned_managed_runtime_root(root).is_ok());
        let link = root.join("linked-root");
        symlink(root, &link).unwrap();
        assert!(open_owned_managed_runtime_root(&link).is_err());
        // Use actual native metadata. A non-root process can read the system
        // root without changing it. Root can create an owned disposable child
        // and assign a foreign UID, then restore ownership before teardown.
        if unsafe { libc::geteuid() } != 0 {
            assert!(open_owned_managed_runtime_root(Path::new("/"))
                .unwrap_err()
                .to_string()
                .contains("exclusively Server-owned"));
        } else {
            use std::os::unix::ffi::OsStrExt;
            let foreign = root.join("foreign-owned");
            fs::create_dir(&foreign).unwrap();
            let name = std::ffi::CString::new(foreign.as_os_str().as_bytes()).unwrap();
            // SAFETY: name is a live NUL-terminated path inside this fixture.
            assert_eq!(unsafe { libc::chown(name.as_ptr(), 1, 0) }, 0);
            let rejected = open_owned_managed_runtime_root(&foreign).is_err();
            // SAFETY: restore the same fixture before teardown, including failure.
            assert_eq!(unsafe { libc::chown(name.as_ptr(), 0, 0) }, 0);
            assert!(rejected);
        }
    }

    #[test]
    fn empty_runtime_cleanup_preserves_payload_links_foreign_parents_and_raced_files() {
        let root = std::env::temp_dir().join(format!(
            "ait-empty-runtime-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let parent = root.join("1");
        fs::create_dir(&parent).unwrap();
        let child = parent.join("7");
        fs::create_dir(&child).unwrap();
        assert!(remove_empty_managed_runtime_container(&root, &child).unwrap());
        fs::create_dir(&child).unwrap();
        assert!(remove_empty_managed_runtime_container(&root, &child).unwrap());
        fs::create_dir(&child).unwrap();
        assert!(!remove_empty_with(&root, &child, || {
            fs::write(child.join("raced"), b"keep").unwrap();
            Ok(())
        })
        .unwrap());
        assert_eq!(fs::read(child.join("raced")).unwrap(), b"keep");
        let link = parent.join("8");
        symlink(&child, &link).unwrap();
        assert!(remove_empty_managed_runtime_container(&root, &link).is_err());
        let foreign = root.join("foreign");
        fs::create_dir(&foreign).unwrap();
        fs::create_dir(foreign.join("9")).unwrap();
        fs::set_permissions(&foreign, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(remove_empty_managed_runtime_container(&root, &foreign.join("9")).is_err());
        fs::set_permissions(&foreign, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(remove_empty_managed_runtime_container(&root, &root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
