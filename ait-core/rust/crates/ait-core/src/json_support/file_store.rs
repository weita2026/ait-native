use crate::file_io::FileIoStore;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::path::{Path, PathBuf};

pub fn expand_home_path_with_file_io_store<S>(store: &S, path_value: &str) -> PathBuf
where
    S: FileIoStore + ?Sized,
{
    if path_value == "~" {
        if let Some(home) = store.home_dir() {
            return home;
        }
    }
    if let Some(suffix) = path_value.strip_prefix("~/") {
        if let Some(home) = store.home_dir() {
            return home.join(suffix);
        }
    }
    PathBuf::from(path_value)
}

pub fn read_json_or_null_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    label: &str,
) -> Result<JsonValue, String>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    if !store.path_exists(&path) {
        return Ok(JsonValue::Null);
    }
    let text = store
        .read_to_string(&path)
        .map_err(|err| format!("Failed to read {label} JSON {}: {err}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|err| format!("Invalid {label} JSON {}: {err}", path.display()))
}

pub fn write_pretty_json_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    payload: &JsonValue,
    label: &str,
) -> Result<(), String>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    let text = serde_json::to_string_pretty(payload)
        .map_err(|err| format!("Failed to encode {label} JSON {}: {err}", path.display()))?;
    store
        .write_string(&path, &text)
        .map_err(|err| format!("Failed to write {label} JSON {}: {err}", path.display()))
}

pub fn read_json_object_or_empty_with_file_io_store<S>(
    store: &S,
    path: &Path,
) -> JsonMap<String, JsonValue>
where
    S: FileIoStore + ?Sized,
{
    let Ok(text) = store.read_to_string(path) else {
        return JsonMap::new();
    };
    serde_json::from_str::<JsonValue>(&text)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

pub fn write_pretty_json_atomically_with_newline_with_file_io_store<S>(
    store: &S,
    path: &Path,
    payload: &JsonValue,
    publish_label: &str,
) -> Result<(), String>
where
    S: FileIoStore + ?Sized,
{
    let mut text = serde_json::to_string_pretty(payload)
        .map_err(|err| format!("Failed to encode {}: {err}", path.display()))?;
    text.push('\n');
    store
        .write_string_atomically(path, &text, publish_label)
        .map_err(|err| err.to_string())
}

pub trait JsonFileStore: FileIoStore {
    fn expand_home_path(&self, path_value: &str) -> PathBuf {
        expand_home_path_with_file_io_store(self, path_value)
    }

    fn read_json_or_null(&self, path_value: &str, label: &str) -> Result<JsonValue, String> {
        read_json_or_null_with_file_io_store(self, path_value, label)
    }

    fn write_pretty_json(
        &self,
        path_value: &str,
        payload: &JsonValue,
        label: &str,
    ) -> Result<(), String> {
        write_pretty_json_with_file_io_store(self, path_value, payload, label)
    }

    fn read_json_object_or_empty(&self, path: &Path) -> JsonMap<String, JsonValue> {
        read_json_object_or_empty_with_file_io_store(self, path)
    }

    fn write_pretty_json_atomically_with_newline(
        &self,
        path: &Path,
        payload: &JsonValue,
        publish_label: &str,
    ) -> Result<(), String> {
        write_pretty_json_atomically_with_newline_with_file_io_store(
            self,
            path,
            payload,
            publish_label,
        )
    }
}

impl<T> JsonFileStore for T where T: FileIoStore + ?Sized {}
