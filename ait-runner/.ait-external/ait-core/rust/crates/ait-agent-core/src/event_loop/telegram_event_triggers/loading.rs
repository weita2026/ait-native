use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ait_core::json_support::{json, JsonCodec, JsonMap as Map, JsonValue};

use super::{DefaultTelegramEventTriggerPlanner, TelegramEventTriggerPlanner};

const EVENT_TRIGGER_CONTRACT: &str = "ait_agent_core.event_loop.TelegramEventTrigger.v1";
const EVENT_TRIGGER_STAGE: &str = "rust_agent_telegram_event_trigger";
const EVENT_TRIGGER_DIRECTORY: &str = "docs/event_trigger";
const FRESH_TOPIC_PATH: &str = "docs/event_trigger/fresh_topic.md";
const PLANNING_MODE_PATH: &str = "docs/event_trigger/planning_mode.md";
const MAX_REPO_ROOT_BYTES: usize = 512 * 1_024;
const MAX_TRIGGER_FILES: usize = 128;
const MAX_TRIGGER_FILE_BYTES: u64 = 1_048_576;
const MAX_REGISTRY_BYTES: usize = 8 * 1_048_576;
const MAX_TRIGGER_TEXT_BYTES: usize = 16 * 1_024;
const MAX_HANDLER_PARTS: usize = 128;
const MAX_MATCH_VALUES: usize = 1_024;

pub struct NativeTelegramEventTriggerRegistryLoader<P = DefaultTelegramEventTriggerPlanner> {
    planner: P,
}

impl NativeTelegramEventTriggerRegistryLoader<DefaultTelegramEventTriggerPlanner> {
    pub fn new() -> Self {
        Self::with_planner(DefaultTelegramEventTriggerPlanner)
    }
}

impl Default for NativeTelegramEventTriggerRegistryLoader<DefaultTelegramEventTriggerPlanner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P> NativeTelegramEventTriggerRegistryLoader<P> {
    pub fn with_planner(planner: P) -> Self {
        Self { planner }
    }
}

impl<P> fmt::Debug for NativeTelegramEventTriggerRegistryLoader<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramEventTriggerRegistryLoader")
            .field("native_planner", &true)
            .field("repo_root_exposed", &false)
            .field("trigger_payload_exposed", &false)
            .finish()
    }
}

impl<P> NativeTelegramEventTriggerRegistryLoader<P>
where
    P: TelegramEventTriggerPlanner,
{
    pub fn load(&self, repo_root: &Path) -> Result<JsonValue, String> {
        validate_repo_root(repo_root)?;
        let fresh_topic = load_optional_config(repo_root.join(FRESH_TOPIC_PATH))?;
        let planning_mode = load_optional_config(repo_root.join(PLANNING_MODE_PATH))?;
        let operational_triggers = load_operational_sources(repo_root)?;
        let planned = self
            .planner
            .plan_json(&json!({
                "stage": "normalize_registry",
                "fresh_topic": fresh_topic,
                "planning_mode": planning_mode,
                "operational_triggers": operational_triggers,
            }))
            .map_err(|_| registry_loading_error())?;
        validate_registry_plan(&planned)
    }
}

fn load_operational_sources(repo_root: &Path) -> Result<Vec<JsonValue>, String> {
    let directory = repo_root.join(EVENT_TRIGGER_DIRECTORY);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(Vec::new());
        }
        Err(_) => return Err(registry_loading_error()),
    };
    let mut paths = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension == "md")
        })
        .collect::<Vec<_>>();
    paths.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    if paths.len() > MAX_TRIGGER_FILES {
        return Err(registry_loading_error());
    }

    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(payload) = load_optional_config(path.clone())? else {
            continue;
        };
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        sources.push(json!({
            "source_path": format!("{EVENT_TRIGGER_DIRECTORY}/{file_name}"),
            "payload": payload,
        }));
    }
    Ok(sources)
}

fn load_optional_config(path: PathBuf) -> Result<Option<JsonValue>, String> {
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(None),
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > MAX_TRIGGER_FILE_BYTES {
        return Err(registry_loading_error());
    }
    let markdown = match fs::read_to_string(path) {
        Ok(markdown) => markdown,
        Err(_) => return Ok(None),
    };
    let Some(source) = json_code_block(&markdown) else {
        return Ok(None);
    };
    let parsed = match JsonCodec::parse_value(source.trim(), "Telegram event trigger") {
        Ok(parsed) => parsed,
        Err(_) => return Ok(None),
    };
    Ok(parsed.is_object().then_some(parsed))
}

fn json_code_block(markdown: &str) -> Option<&str> {
    const START: &[u8] = b"```json";
    const END: &[u8] = b"```";
    let bytes = markdown.as_bytes();
    let start = bytes
        .windows(START.len())
        .position(|window| window.eq_ignore_ascii_case(START))?
        + START.len();
    let end = bytes[start..]
        .windows(END.len())
        .position(|window| window == END)?
        + start;
    markdown.get(start..end)
}

fn validate_registry_plan(planned: &JsonValue) -> Result<JsonValue, String> {
    let object = planned.as_object().ok_or_else(registry_loading_error)?;
    if text(object, "migration_stage") != Some(EVENT_TRIGGER_STAGE)
        || text(object, "event_trigger_contract") != Some(EVENT_TRIGGER_CONTRACT)
        || text(object, "stage") != Some("normalize_registry")
        || text(object, "transport") != Some("telegram")
        || bool_field(object, "rust_event_loop_required") != Some(true)
        || bool_field(object, "python_event_trigger_allowed") != Some(false)
    {
        return Err(registry_loading_error());
    }
    let registry = object
        .get("registry")
        .and_then(JsonValue::as_object)
        .ok_or_else(registry_loading_error)?;
    if registry.len() != 3
        || !registry
            .get("fresh_topic")
            .is_some_and(JsonValue::is_object)
        || !registry
            .get("planning_mode")
            .is_some_and(JsonValue::is_object)
    {
        return Err(registry_loading_error());
    }
    let triggers = registry
        .get("telegram_operational")
        .and_then(JsonValue::as_array)
        .ok_or_else(registry_loading_error)?;
    if triggers.len() > MAX_TRIGGER_FILES || triggers.iter().any(|trigger| !valid_trigger(trigger))
    {
        return Err(registry_loading_error());
    }
    let registry = JsonValue::Object(registry.clone());
    if registry.to_string().len() > MAX_REGISTRY_BYTES {
        return Err(registry_loading_error());
    }
    Ok(registry)
}

fn valid_trigger(trigger: &JsonValue) -> bool {
    let Some(object) = trigger.as_object() else {
        return false;
    };
    if object.len() != 6 {
        return false;
    }
    let Some(trigger_id) = object.get("trigger_id").and_then(JsonValue::as_str) else {
        return false;
    };
    let Some(display_trigger) = object.get("display_trigger").and_then(JsonValue::as_str) else {
        return false;
    };
    let Some(source_path) = object.get("source_path").and_then(JsonValue::as_str) else {
        return false;
    };
    let Some(handler) = object.get("handler_command").and_then(JsonValue::as_array) else {
        return false;
    };
    let Some(matches) = object.get("match").and_then(JsonValue::as_object) else {
        return false;
    };
    let source_file = source_path.strip_prefix("docs/event_trigger/");
    valid_text(trigger_id, false)
        && valid_text(display_trigger, false)
        && valid_text(source_path, false)
        && source_file.is_some_and(|value| {
            !value.is_empty()
                && value.ends_with(".md")
                && !value.contains('/')
                && !value.contains('\\')
        })
        && !handler.is_empty()
        && handler.len() <= MAX_HANDLER_PARTS
        && handler
            .iter()
            .all(|value| value.as_str().is_some_and(|value| valid_text(value, false)))
        && valid_text_array(matches.get("phrases"))
        && valid_text_array(matches.get("commands"))
        && matches.len() == 6
        && matches.get("pattern").is_some_and(|value| {
            value.is_null() || value.as_str().is_some_and(|value| valid_text(value, false))
        })
        && bool_field(matches, "allow_trailing_punctuation").is_some()
        && bool_field(matches, "reply_only").is_some()
        && bool_field(matches, "case_sensitive").is_some()
        && object.get("priority").and_then(JsonValue::as_i64).is_some()
}

fn valid_text_array(value: Option<&JsonValue>) -> bool {
    value.and_then(JsonValue::as_array).is_some_and(|values| {
        values.len() <= MAX_MATCH_VALUES
            && values
                .iter()
                .all(|value| value.as_str().is_some_and(|value| valid_text(value, false)))
    })
}

fn valid_text(value: &str, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.trim() == value
        && value.len() <= MAX_TRIGGER_TEXT_BYTES
        && !value.chars().any(char::is_control)
}

fn validate_repo_root(repo_root: &Path) -> Result<(), String> {
    let value = repo_root.to_string_lossy();
    if repo_root.as_os_str().is_empty()
        || value.len() > MAX_REPO_ROOT_BYTES
        || value
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
    {
        return Err(registry_loading_error());
    }
    Ok(())
}

fn text<'a>(object: &'a Map<String, JsonValue>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(JsonValue::as_str)
}

fn bool_field(object: &Map<String, JsonValue>, key: &str) -> Option<bool> {
    object.get(key).and_then(JsonValue::as_bool)
}

fn registry_loading_error() -> String {
    "Telegram event-trigger registry loading failed.".to_string()
}

#[cfg(test)]
mod tests;
