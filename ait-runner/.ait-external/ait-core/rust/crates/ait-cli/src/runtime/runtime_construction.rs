use super::*;

impl RepoRuntime {
    pub fn discover() -> Result<Self, String> {
        Self::discover_with_config_read(false).map_err(|error| error.to_string())
    }

    pub(crate) fn discover_for_session_ensure() -> std::io::Result<Self> {
        Self::discover_with_config_read(true)
    }

    fn discover_with_config_read(immediate: bool) -> std::io::Result<Self> {
        match Self::discover_from_with_config_read(&env::current_dir()?, immediate) {
            Ok(ctx) => Ok(ctx),
            Err(error) if immediate && error.kind() == std::io::ErrorKind::WouldBlock => Err(error),
            Err(primary_err) => {
                for var_name in REPO_DISCOVERY_ENV_VARS {
                    let Some(raw) = env::var_os(var_name) else {
                        continue;
                    };
                    let candidate = PathBuf::from(raw);
                    if candidate.as_os_str().is_empty() {
                        continue;
                    }
                    match Self::discover_from_with_config_read(&candidate, immediate) {
                        Ok(ctx) => return Ok(ctx),
                        Err(error)
                            if immediate && error.kind() == std::io::ErrorKind::WouldBlock =>
                        {
                            return Err(error)
                        }
                        Err(_) => {}
                    }
                }
                Err(primary_err)
            }
        }
    }

    pub fn discover_from_path(start: &Path) -> Result<Self, String> {
        Self::discover_from(start)
    }

    pub(super) fn discover_from(start: &Path) -> Result<Self, String> {
        Self::discover_from_with_config_read(start, false).map_err(|error| error.to_string())
    }

    fn discover_from_with_config_read(start: &Path, immediate: bool) -> std::io::Result<Self> {
        let mut cur = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
        loop {
            let ait_dir = cur.join(APP_DIR);
            let worktree_config_path = cur.join(WORKTREE_CONFIG_NAME);
            if ait_dir.is_dir() {
                let config_path = ait_dir.join(CONFIG_NAME);
                let bytes = if immediate {
                    ait_core::repo_state_json::try_read_root_config_bytes(&config_path)
                } else {
                    ait_core::repo_state_json::read_root_config_bytes(&config_path)
                };
                let mut config = match bytes {
                    Ok(bytes) => {
                        let content = String::from_utf8(bytes).map_err(|error| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("failed to read {}: {error}", config_path.display()),
                            )
                        })?;
                        crate::json_support::parse_value(&content, "repository config")
                            .map_err(|error| {
                                std::io::Error::new(std::io::ErrorKind::InvalidData, error)
                            })?
                            .as_object()
                            .cloned()
                            .ok_or_else(|| {
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    format!("{} must be a JSON object", config_path.display()),
                                )
                            })?
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => JsonMap::new(),
                    Err(error) if immediate && error.kind() == std::io::ErrorKind::WouldBlock => {
                        return Err(error)
                    }
                    Err(error) => {
                        return Err(std::io::Error::new(
                            error.kind(),
                            format!("failed to read {}: {error}", config_path.display()),
                        ))
                    }
                };
                if worktree_config_path.exists() {
                    let overlay = read_json_object(&worktree_config_path);
                    for (key, value) in overlay {
                        if key == "agent_teams" {
                            continue;
                        }
                        if !value.is_null() {
                            config.insert(key, value);
                        }
                    }
                }
                return Ok(Self {
                    root: cur.clone(),
                    ait_dir,
                    config,
                    worktree_config_path: worktree_config_path
                        .exists()
                        .then_some(worktree_config_path),
                });
            }
            let Some(parent) = cur.parent() else {
                break;
            };
            if parent == cur {
                break;
            }
            cur = parent.to_path_buf();
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No .ait directory found in current path or parents.",
        ))
    }
}

pub(super) fn read_json_object(path: &Path) -> JsonMap<String, JsonValue> {
    let Ok(content) = fs::read_to_string(path) else {
        return JsonMap::new();
    };
    parse_object_or_empty(&content)
}

pub(super) fn as_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

pub(super) fn as_nonempty_string(value: &JsonValue) -> Option<String> {
    nonempty(as_string(value))
}

pub(super) fn nonempty(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

pub(super) fn env_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|raw| nonempty(Some(raw)))
}

pub(super) fn plan_task_binding_mode(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Object(obj) => obj
            .get("mode")
            .and_then(as_nonempty_string)
            .and_then(|mode| match mode.as_str() {
                "off" | "advisory" | "strict" | "required" => Some(mode),
                _ => None,
            }),
        _ => None,
    }
}
