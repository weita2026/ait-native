use super::*;

pub(super) fn default_main_seed_root() -> Result<PathBuf, String> {
    #[cfg(test)]
    let root = crate::foundation::ci_runtime_temp::ci_ram_runtime_root_with_source()?.0;
    #[cfg(not(test))]
    let root = crate::foundation::ci_runtime_temp::validated_ci_ram_runtime_root_with_source()?.0;
    Ok(root.join("main-seeds"))
}

pub(super) fn normalize_plane(value: Option<String>) -> Result<String, String> {
    let plane = value.unwrap_or_else(|| DEFAULT_REPO_CI_PLANE.to_string());
    let plane = plane.trim();
    if REPO_CI_PLANES.iter().any(|allowed| plane == *allowed) {
        Ok(plane.to_string())
    } else {
        Err(format!(
            "Unsupported repo CI plane `{plane}`. Expected one of: {}.",
            REPO_CI_PLANES.join(", ")
        ))
    }
}

pub(super) fn required_text(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<String, String> {
    optional_text(value, key).ok_or_else(|| format!("Field `{key}` must be a non-empty string."))
}

pub(super) fn optional_text(value: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn optional_path(value: &JsonMap<String, JsonValue>, key: &str) -> Option<PathBuf> {
    optional_text(value, key).map(PathBuf::from)
}

pub(super) fn optional_bool(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<bool>, String> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("Field `{key}` must be a boolean.")),
    }
}

pub(super) fn optional_i64(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<i64>, String> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => number
            .as_i64()
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("Field `{key}` must be a positive integer."))
            .map(Some),
        Some(_) => Err(format!("Field `{key}` must be a positive integer.")),
    }
}

pub(super) fn string_array(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(raw) = value.get(key) else {
        return Ok(Vec::new());
    };
    let values = raw
        .as_array()
        .ok_or_else(|| format!("Field `{key}` must be an array of non-empty strings."))?;
    let mut parsed = Vec::new();
    for item in values {
        let text = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Field `{key}` must contain non-empty strings."))?;
        parsed.push(text.to_string());
    }
    Ok(parsed)
}

pub(super) fn string_array_from_value(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::trim))
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn safe_path_segment(value: &str) -> String {
    let segment = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if segment.is_empty() || segment == "." || segment == ".." {
        "repo".to_string()
    } else {
        segment
    }
}

pub(super) fn duration_seconds(started: Instant) -> f64 {
    let millis = started.elapsed().as_millis() as f64;
    (millis / 1000.0 * 1000.0).round() / 1000.0
}

pub(super) fn path_has_parent_escape(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

pub(super) fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

#[cfg(test)]
mod tests {
    use super::default_main_seed_root;
    use std::env;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("repo CI env lock should not poison")
    }

    fn restore_env_var(name: &str, value: Option<String>) {
        if let Some(value) = value {
            env::set_var(name, value);
        } else {
            env::remove_var(name);
        }
    }

    #[test]
    fn default_main_seed_root_uses_the_current_ci_ram_root() {
        let _guard = env_lock();
        let ci_ram_root_env = crate::environment_contract::names::AIT_NATIVE_SERVER_CI_RAM_ROOT;
        let previous_ci_ram_root = env::var(ci_ram_root_env).ok();
        let ram_root = PathBuf::from("/tmp/ait-server-ram-runtime-test");

        env::set_var(ci_ram_root_env, &ram_root);

        let root = default_main_seed_root().expect("CI RAM main-seed root should resolve");

        restore_env_var(ci_ram_root_env, previous_ci_ram_root);

        assert_eq!(root, ram_root.join("main-seeds"));
    }
}
