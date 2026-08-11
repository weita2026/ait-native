use super::*;

pub(in crate::local_snapshot) fn expanduser_path(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

pub(in crate::local_snapshot) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => output.push(prefix.as_os_str()),
            std::path::Component::RootDir => output.push(Path::new("/")),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            std::path::Component::Normal(part) => output.push(part),
        }
    }
    output
}

pub(in crate::local_snapshot) fn resolve_path_strict_false(path: &Path) -> PathBuf {
    let normalized = lexical_normalize(path);
    if let Ok(canonical) = normalized.canonicalize() {
        return canonical;
    }
    let mut cursor = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        if cursor.exists() {
            if let Ok(canonical_parent) = cursor.canonicalize() {
                let mut resolved = canonical_parent;
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return lexical_normalize(&resolved);
            }
        }
        let Some(file_name) = cursor.file_name() else {
            return normalized;
        };
        missing.push(file_name.to_os_string());
        let Some(parent) = cursor.parent() else {
            return normalized;
        };
        cursor = parent;
    }
}

pub(in crate::local_snapshot) fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(in crate::local_snapshot) fn require_non_empty(
    value: &str,
    field: &str,
) -> Result<String, String> {
    normalize_optional_text(Some(value)).ok_or_else(|| format!("{field} must not be empty"))
}

pub(in crate::local_snapshot) fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
