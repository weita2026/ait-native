//! Private fixture binary selection. Native owns candidate/family verification.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub fn select(cargo_binary: &Path, supplied: Option<&OsStr>) -> Result<PathBuf, String> {
    let Some(supplied) = supplied else {
        return Ok(cargo_binary.to_path_buf());
    };
    let path = PathBuf::from(supplied);
    if !path.is_absolute() {
        return Err("AIT_TEST_CLI_BIN must be a nonempty absolute executable path".into());
    }
    let metadata = std::fs::metadata(&path)
        .map_err(|error| format!("Invalid AIT_TEST_CLI_BIN {}: {error}", path.display()))?;
    if !metadata.is_file() || !executable(&path, &metadata) {
        return Err(format!(
            "AIT_TEST_CLI_BIN is not an executable file: {}",
            path.display()
        ));
    }
    Ok(path)
}

pub fn selected(cargo_binary: &str) -> PathBuf {
    select(
        Path::new(cargo_binary),
        std::env::var_os("AIT_TEST_CLI_BIN").as_deref(),
    )
    .unwrap_or_else(|error| panic!("{error}; supplied input never falls back to Cargo or PATH"))
}

#[cfg(unix)]
fn executable(_path: &Path, metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(windows)]
fn executable(path: &Path, _metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetBinaryTypeW;
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut kind = 0;
    // A terminated input and writable u32 remain valid for this synchronous call.
    // SCS_32BIT_BINARY=0 and SCS_64BIT_BINARY=6 are native Windows executables.
    unsafe { GetBinaryTypeW(path.as_ptr(), &mut kind) != 0 && matches!(kind, 0 | 6) }
}

#[cfg(not(any(unix, windows)))]
fn executable(_path: &Path, _metadata: &std::fs::Metadata) -> bool {
    false
}
