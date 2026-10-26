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
}
