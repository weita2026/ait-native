use crate::file_io::{FileIoStore, FilesystemFileIoStore};
use crate::json_support::JsonValue;
use crate::json_support::{
    expand_home_path_with_file_io_store, read_json_or_null_with_file_io_store, JsonCodec,
    JsonEncodeOptions,
};
use serde::Serialize;

pub struct RepoStateJson<S> {
    store: S,
}

impl<S> RepoStateJson<S> {
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl RepoStateJson<FilesystemFileIoStore> {
    pub fn filesystem() -> Self {
        Self::new(FilesystemFileIoStore)
    }
}

impl<S> RepoStateJson<S>
where
    S: FileIoStore,
{
    pub fn read_state_json_file(&self, path_value: &str, label: &str) -> Result<JsonValue, String> {
        read_state_json_file_with_store(&self.store, path_value, label)
    }

    pub fn write_state_json_file(
        &self,
        path_value: &str,
        payload: &JsonValue,
        label: &str,
    ) -> Result<(), String> {
        write_state_json_file_with_store(&self.store, path_value, payload, label)
    }
}

#[cfg(test)]
fn read_state_json_file_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    label: &str,
) -> Result<JsonValue, String>
where
    S: FileIoStore + ?Sized,
{
    RepoStateJson::new(store).read_state_json_file(path_value, label)
}

fn read_state_json_file(path_value: &str, label: &str) -> Result<JsonValue, String> {
    RepoStateJson::filesystem().read_state_json_file(path_value, label)
}

#[cfg(test)]
fn write_state_json_file_with_file_io_store<S>(
    store: &S,
    path_value: &str,
    payload: &JsonValue,
    label: &str,
) -> Result<(), String>
where
    S: FileIoStore + ?Sized,
{
    RepoStateJson::new(store).write_state_json_file(path_value, payload, label)
}

fn write_state_json_file(path_value: &str, payload: &JsonValue, label: &str) -> Result<(), String> {
    RepoStateJson::filesystem().write_state_json_file(path_value, payload, label)
}

pub fn read_repo_config_json_file(path_value: &str) -> Result<JsonValue, String> {
    read_state_json_file(path_value, "repo config")
}

pub fn write_repo_config_json_file(path_value: &str, payload: &JsonValue) -> Result<(), String> {
    write_state_json_file(path_value, payload, "repo config")
}

pub fn read_worktree_config_json_file(path_value: &str) -> Result<JsonValue, String> {
    read_state_json_file(path_value, "worktree config")
}

pub fn write_worktree_config_json_file(
    path_value: &str,
    payload: &JsonValue,
) -> Result<(), String> {
    write_state_json_file(path_value, payload, "worktree config")
}

pub fn read_worktree_metadata_json_file(path_value: &str) -> Result<JsonValue, String> {
    read_state_json_file(path_value, "worktree metadata")
}

pub fn write_worktree_metadata_json_file(
    path_value: &str,
    payload: &JsonValue,
) -> Result<(), String> {
    write_state_json_file(path_value, payload, "worktree metadata")
}

fn read_state_json_file_with_store<S>(
    store: &S,
    path_value: &str,
    label: &str,
) -> Result<JsonValue, String>
where
    S: FileIoStore + ?Sized,
{
    read_json_or_null_with_file_io_store(store, path_value, label)
}

fn write_state_json_file_with_store<S>(
    store: &S,
    path_value: &str,
    payload: &JsonValue,
    label: &str,
) -> Result<(), String>
where
    S: FileIoStore + ?Sized,
{
    let path = expand_home_path_with_file_io_store(store, path_value);
    let text = encode_state_json(payload, &path, label)?;
    store
        .write_string(&path, &text)
        .map_err(|err| format!("Failed to write {label} JSON {}: {err}", path.display()))
}

fn encode_state_json<T>(payload: &T, path: &std::path::Path, label: &str) -> Result<String, String>
where
    T: Serialize + ?Sized,
{
    JsonCodec::encode_serializable_with_error_prefix(
        payload,
        JsonEncodeOptions::pretty(),
        &format!("Failed to encode {label} JSON {}", path.display()),
    )
    .map_err(String::from)
}

#[cfg(test)]
mod tests;
