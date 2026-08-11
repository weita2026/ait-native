use super::*;

pub fn current_source_cache_size_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            let Ok(entries) = fs::read_dir(&path) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                stack.push(entry.path());
            }
        }
    }
    total
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

pub(super) fn artifact_sha256_hex(path: &Path) -> Result<String, String> {
    use std::io::Read;

    let mut file = fs::File::open(path)
        .map_err(|err| format!("Failed to read artifact {}: {err}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|err| format!("Failed to read artifact {}: {err}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn path_mtime_ns(path: &Path) -> Result<u64, String> {
    let modified = fs::metadata(path)
        .map_err(|err| format!("Failed to stat {}: {err}", path.display()))?
        .modified()
        .map_err(|err| format!("Failed to read mtime for {}: {err}", path.display()))?;
    system_time_ns(modified)
}

pub(super) fn path_mtime_seconds(path: &Path) -> Result<f64, String> {
    let modified = fs::metadata(path)
        .map_err(|err| format!("Failed to stat {}: {err}", path.display()))?
        .modified()
        .map_err(|err| format!("Failed to read mtime for {}: {err}", path.display()))?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("mtime for {} predates UNIX_EPOCH: {err}", path.display()))?;
    Ok(duration.as_secs_f64())
}

pub(super) fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

pub(super) fn monotonic_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

pub(super) fn system_time_ns(time: SystemTime) -> Result<u64, String> {
    let nanos = time
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("timestamp predates UNIX_EPOCH: {err}"))?
        .as_nanos();
    u64::try_from(nanos).map_err(|_| "timestamp does not fit in u64 nanoseconds.".to_string())
}

pub(super) fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(super) fn relative_path_text(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

pub(super) fn normalized_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToString::to_string)
}

pub(super) fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub(super) fn resolve_path_strict_false(path: &Path) -> PathBuf {
    let expanded = expand_home(path);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(expanded)
    };
    lexical_normalize(&absolute)
}

pub(super) fn expand_home(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if text == "~" {
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| path.to_path_buf());
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return std::env::var("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|_| path.to_path_buf());
    }
    path.to_path_buf()
}

pub(super) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}
