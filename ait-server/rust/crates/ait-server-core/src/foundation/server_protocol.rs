use std::env;
use std::path::PathBuf;

pub const RUNTIME_DATA_ENV: &str = "AIT_RUNTIME_DATA";
pub const LEGACY_SERVER_DATA_ENV: &str = "AIT_NATIVE_SERVER_DATA";

pub const STORAGE_INGEST_MODE_DEFAULT: &str = "default";
pub const STORAGE_INGEST_MODE_PACK_FULL: &str = "pack_full";
pub const STORAGE_INGEST_MODE_PACK_DELTA: &str = "pack_delta";

pub const TASK_STATUS_COMPLETED: &str = "completed";
pub const TASK_STATUS_ABANDONED: &str = "abandoned";
pub const TASK_STATUS_LEGACY_CANCELED: &str = "canceled";
pub const TASK_STATUS_LATER_PROMOTION_EXCLUDED: &str = "later_promotion_excluded";

pub fn encode_ref_name(name: &str) -> String {
    let mut encoded = String::new();
    for byte in name.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

pub fn decode_ref_name(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3]) {
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    decoded.push(value);
                    index += 3;
                    continue;
                }
            }
        }
        decoded.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

pub fn storage_ingest_mode_values(include_default: bool) -> Vec<&'static str> {
    let mut values = Vec::new();
    if include_default {
        values.push(STORAGE_INGEST_MODE_DEFAULT);
    }
    values.push(STORAGE_INGEST_MODE_PACK_FULL);
    values.push(STORAGE_INGEST_MODE_PACK_DELTA);
    values
}

pub fn normalize_storage_ingest_mode(
    value: Option<&str>,
    allow_default: bool,
) -> Result<String, String> {
    let fallback = if allow_default {
        STORAGE_INGEST_MODE_DEFAULT
    } else {
        STORAGE_INGEST_MODE_PACK_DELTA
    };
    let mut resolved = value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_string();
    if !allow_default && resolved == STORAGE_INGEST_MODE_DEFAULT {
        resolved = STORAGE_INGEST_MODE_PACK_DELTA.to_string();
    }
    if storage_ingest_mode_values(allow_default)
        .iter()
        .any(|candidate| *candidate == resolved)
    {
        Ok(resolved)
    } else {
        Err(format!(
            "Unknown storage_ingest_mode: {}. Expected one of: {}",
            value.unwrap_or("None"),
            storage_ingest_mode_values(allow_default).join(", ")
        ))
    }
}

pub fn runtime_data_env_name() -> Option<&'static str> {
    configured_runtime_data_env().map(|(name, _)| name)
}

pub fn runtime_data_env_value() -> Option<String> {
    configured_runtime_data_env().map(|(_, value)| value)
}

pub fn resolve_server_runtime_root_with_source(
    path: Option<&str>,
) -> Result<(PathBuf, &'static str), String> {
    if let Some(raw) = path {
        return Ok((expand_user(raw), "explicit"));
    }
    if let Some(value) = runtime_data_env_value() {
        return Ok((expand_user(&value), "env"));
    }
    Err(
        "AIT_NATIVE_SERVER_DATA is required for server runtime access; platform default runtime roots are no longer supported."
            .to_string(),
    )
}

pub fn resolve_server_runtime_root(path: Option<&str>) -> Result<PathBuf, String> {
    resolve_server_runtime_root_with_source(path).map(|(root, _)| root)
}

pub fn task_close_allowed_statuses(scope: &str) -> Result<Vec<&'static str>, String> {
    match scope.trim().to_lowercase().as_str() {
        "local" => Ok(vec![
            TASK_STATUS_COMPLETED,
            TASK_STATUS_ABANDONED,
            TASK_STATUS_LATER_PROMOTION_EXCLUDED,
            TASK_STATUS_LEGACY_CANCELED,
        ]),
        "remote" => Ok(vec![
            TASK_STATUS_COMPLETED,
            TASK_STATUS_ABANDONED,
            TASK_STATUS_LEGACY_CANCELED,
        ]),
        _ => Err("Task close scope must be `local` or `remote`.".to_string()),
    }
}

fn configured_runtime_data_env() -> Option<(&'static str, String)> {
    let runtime_value = env::var(RUNTIME_DATA_ENV).unwrap_or_default();
    let runtime_value = runtime_value.trim();
    if !runtime_value.is_empty() {
        return Some((RUNTIME_DATA_ENV, runtime_value.to_string()));
    }
    let legacy_value = env::var(LEGACY_SERVER_DATA_ENV).unwrap_or_default();
    let legacy_value = legacy_value.trim();
    if !legacy_value.is_empty() {
        return Some((LEGACY_SERVER_DATA_ENV, legacy_value.to_string()));
    }
    None
}

fn expand_user(raw: &str) -> PathBuf {
    if raw == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_names_round_trip_percent_encoded_values() {
        let encoded = encode_ref_name("repo/main branch");
        assert_eq!(encoded, "repo%2Fmain%20branch");
        assert_eq!(decode_ref_name(&encoded), "repo/main branch");
    }

    #[test]
    fn storage_ingest_mode_normalizes_default_without_allowing_default() {
        assert_eq!(
            normalize_storage_ingest_mode(Some("default"), false).unwrap(),
            STORAGE_INGEST_MODE_PACK_DELTA
        );
        assert_eq!(
            storage_ingest_mode_values(false),
            vec![
                STORAGE_INGEST_MODE_PACK_FULL,
                STORAGE_INGEST_MODE_PACK_DELTA
            ]
        );
    }

    #[test]
    fn task_close_allowed_statuses_keep_later_promotion_local_only() {
        assert!(task_close_allowed_statuses("local")
            .unwrap()
            .contains(&TASK_STATUS_LATER_PROMOTION_EXCLUDED));
        assert!(!task_close_allowed_statuses("remote")
            .unwrap()
            .contains(&TASK_STATUS_LATER_PROMOTION_EXCLUDED));
    }

    #[test]
    fn explicit_runtime_root_has_source() {
        let (root, source) =
            resolve_server_runtime_root_with_source(Some("/tmp/ait-server")).unwrap();
        assert_eq!(root, PathBuf::from("/tmp/ait-server"));
        assert_eq!(source, "explicit");
    }
}
