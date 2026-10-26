use super::*;

impl RepoRuntime {
    pub fn discover() -> Result<Self, String> {
        match Self::discover_from(&env::current_dir().map_err(|err| err.to_string())?) {
            Ok(ctx) => Ok(ctx),
            Err(primary_err) => {
                for var_name in REPO_DISCOVERY_ENV_VARS {
                    let Some(raw) = env::var_os(var_name) else {
                        continue;
                    };
                    let candidate = PathBuf::from(raw);
                    if candidate.as_os_str().is_empty() {
                        continue;
                    }
                    if let Ok(ctx) = Self::discover_from(&candidate) {
                        return Ok(ctx);
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
        let mut cur = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
        loop {
            let ait_dir = cur.join(APP_DIR);
            let worktree_config_path = cur.join(WORKTREE_CONFIG_NAME);
            if ait_dir.is_dir() {
                let mut config = read_json_object(&ait_dir.join(CONFIG_NAME));
                if worktree_config_path.exists() {
                    let overlay = read_json_object(&worktree_config_path);
                    for (key, value) in overlay {
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
        Err("No .ait directory found in current path or parents.".to_string())
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

pub(super) fn command_executable_basename() -> Option<String> {
    let argv0 = env::args().next()?;
    let basename = PathBuf::from(argv0)
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.trim().to_string())?;
    nonempty(Some(basename))
}
