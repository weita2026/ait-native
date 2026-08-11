use std::env;
use std::path::PathBuf;

pub const RUNTIME_DATA_ENV: &str = "AIT_RUNTIME_DATA";
pub const LEGACY_SERVER_DATA_ENV: &str = "AIT_NATIVE_SERVER_DATA";

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

pub fn runtime_data_env_value() -> Option<String> {
    configured_runtime_data_env().map(|(_, value)| value)
}

pub fn runtime_data_env_name() -> Option<&'static str> {
    configured_runtime_data_env().map(|(name, _)| name)
}

pub fn resolve_runtime_data_root_with_source(
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

pub fn resolve_runtime_data_root(path: Option<&str>) -> Result<PathBuf, String> {
    resolve_runtime_data_root_with_source(path).map(|(root, _)| root)
}

pub fn resolve_server_runtime_root_with_source(
    path: Option<&str>,
) -> Result<(PathBuf, &'static str), String> {
    resolve_runtime_data_root_with_source(path)
}

pub fn resolve_server_runtime_root(path: Option<&str>) -> Result<PathBuf, String> {
    resolve_runtime_data_root(path)
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
