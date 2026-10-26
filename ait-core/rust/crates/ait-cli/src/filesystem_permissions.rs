use std::fs;
use std::io;
use std::path::Path;

/// Return the portable permission bits used by AIT's Snapshot and release
/// surfaces. Unix retains the exact filesystem bits. Platforms without POSIX
/// modes retain the caller's logical mode while reflecting the native
/// read-only attribute in the write bits.
pub(crate) fn portable_mode(metadata: &fs::Metadata, _non_unix_fallback: u32) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o777
    }
    #[cfg(not(unix))]
    {
        non_unix_mode(_non_unix_fallback, metadata.permissions().readonly())
    }
}

/// Apply logical permission bits to an existing path. Windows has no POSIX
/// executable bits, so only the writeability boundary is projected there.
pub(crate) fn set_portable_mode(path: &Path, mode: u32) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        permissions.set_mode(mode & 0o777);
    }
    #[cfg(not(unix))]
    {
        permissions.set_readonly(mode & 0o222 == 0);
    }
    fs::set_permissions(path, permissions)
}

/// Restore independently mutable link permissions without following its target.
pub(crate) fn set_portable_symlink_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        // SAFETY: the NUL-terminated path remains live for this path-only syscall.
        let result = unsafe {
            libc::fchmodat(
                libc::AT_FDCWD,
                path.as_ptr(),
                (mode & 0o777) as libc::mode_t,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

#[cfg(any(not(unix), test))]
fn non_unix_mode(fallback: u32, readonly: bool) -> u32 {
    let mode = fallback & 0o777;
    if readonly {
        mode & !0o222
    } else {
        mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_unix_mode_preserves_logical_bits_and_native_readonly() {
        assert_eq!(non_unix_mode(0o755, false), 0o755);
        assert_eq!(non_unix_mode(0o755, true), 0o555);
        assert_eq!(non_unix_mode(0o644, false), 0o644);
        assert_eq!(non_unix_mode(0o644, true), 0o444);
    }

    #[cfg(unix)]
    #[test]
    fn portable_mode_round_trips_exact_unix_bits() {
        let file = tempfile::NamedTempFile::new().expect("temporary file");
        set_portable_mode(file.path(), 0o751).expect("set exact Unix mode");
        let metadata = fs::metadata(file.path()).expect("read metadata");
        assert_eq!(portable_mode(&metadata, 0o644), 0o751);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn symlink_mode_changes_leave_target_permissions_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        fs::write(&target, b"target").unwrap();
        set_portable_mode(&target, 0o600).unwrap();
        for (name, destination, mode) in
            [("alias", "target", 0o700), ("dangling", "missing", 0o711)]
        {
            let link = temp.path().join(name);
            std::os::unix::fs::symlink(destination, &link).unwrap();
            set_portable_symlink_mode(&link, mode).unwrap();
            assert_eq!(
                portable_mode(&fs::symlink_metadata(&link).unwrap(), 0o644),
                mode
            );
        }
        assert_eq!(portable_mode(&fs::metadata(&target).unwrap(), 0o644), 0o600);
    }
}
