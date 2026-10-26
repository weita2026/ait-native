use crate::json_support::{json, JsonCodec, JsonEncodeOptions, JsonMap, JsonValue};
use std::fs;
use std::path::{Path, PathBuf};

pub type RemoteStoreResult<T> = Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteRecord {
    pub remote_id: i64,
    pub name: String,
    pub url: String,
    pub repo_name: Option<String>,
    pub is_default_push: i64,
    pub is_default_pull: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAddRecord {
    pub name: String,
    pub url: String,
    pub repo_name: Option<String>,
    pub make_default: bool,
    pub created_at: String,
}

pub trait RemoteStore {
    fn remote_exists(&self, name: &str) -> RemoteStoreResult<bool>;
    fn list_remotes(&self) -> RemoteStoreResult<Vec<RemoteRecord>>;
    fn remote_by_name(&self, name: &str) -> RemoteStoreResult<Option<RemoteRecord>>;
    fn add_remote(&self, request: &RemoteAddRecord) -> RemoteStoreResult<()>;
}

pub fn remote_exists_with_remote_store<S>(store: &S, name: &str) -> RemoteStoreResult<bool>
where
    S: RemoteStore + ?Sized,
{
    store.remote_exists(name)
}

pub fn list_remotes_with_remote_store<S>(store: &S) -> RemoteStoreResult<Vec<RemoteRecord>>
where
    S: RemoteStore + ?Sized,
{
    store.list_remotes()
}

pub fn remote_by_name_with_remote_store<S>(
    store: &S,
    name: &str,
) -> RemoteStoreResult<Option<RemoteRecord>>
where
    S: RemoteStore + ?Sized,
{
    store.remote_by_name(name)
}

pub fn add_remote_with_remote_store<S>(
    store: &S,
    request: &RemoteAddRecord,
) -> RemoteStoreResult<()>
where
    S: RemoteStore + ?Sized,
{
    store.add_remote(request)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigRemoteStore {
    config_path: PathBuf,
}

impl ConfigRemoteStore {
    pub fn new(config_path: impl Into<PathBuf>) -> RemoteStoreResult<Self> {
        let config_path = config_path.into();
        if config_path.as_os_str().is_empty() {
            return Err("Remote config path must not be empty.".to_string());
        }
        Ok(Self { config_path })
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    fn read_config(&self) -> RemoteStoreResult<JsonMap<String, JsonValue>> {
        let text = fs::read_to_string(&self.config_path).map_err(|error| {
            format!(
                "Failed to read remote config {}: {error}",
                self.config_path.display()
            )
        })?;
        JsonCodec::parse_object(&text, ".ait config").map_err(String::from)
    }

    fn write_config(&self, config: JsonMap<String, JsonValue>) -> RemoteStoreResult<()> {
        let parent = self.config_path.parent().ok_or_else(|| {
            format!(
                "Remote config path has no parent: {}",
                self.config_path.display()
            )
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed to create remote config directory {}: {error}",
                parent.display()
            )
        })?;
        let encoded = JsonCodec::encode_value(
            &JsonValue::Object(config),
            JsonEncodeOptions::pretty().with_trailing_newline(),
        )
        .map_err(String::from)?;
        let file_name = self
            .config_path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("config.json");
        let temporary_path = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
        fs::write(&temporary_path, encoded).map_err(|error| {
            format!(
                "Failed to write temporary remote config {}: {error}",
                temporary_path.display()
            )
        })?;
        if let Err(error) = fs::rename(&temporary_path, &self.config_path) {
            let _ = fs::remove_file(&temporary_path);
            return Err(format!(
                "Failed to publish remote config {}: {error}",
                self.config_path.display()
            ));
        }
        Ok(())
    }

    fn records_from_config(
        &self,
        config: &JsonMap<String, JsonValue>,
    ) -> RemoteStoreResult<Vec<RemoteRecord>> {
        let default_remote = config
            .get("default_remote")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let Some(remotes) = config.get("remotes") else {
            return Ok(Vec::new());
        };
        let remotes = remotes
            .as_object()
            .ok_or_else(|| ".ait config remotes must be an object.".to_string())?;
        let mut records = Vec::with_capacity(remotes.len());
        for (ordinal, (name, value)) in remotes.iter().enumerate() {
            let name = require_non_empty(name, "remote name")?;
            let object = value
                .as_object()
                .ok_or_else(|| format!("Remote {name} config must be an object."))?;
            let url = object
                .get("url")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| format!("Remote {name} config is missing url."))?;
            let url = require_non_empty(url, "remote URL")?;
            let remote_id = object
                .get("remote_id")
                .and_then(JsonValue::as_i64)
                .unwrap_or_else(|| i64::try_from(ordinal + 1).unwrap_or(i64::MAX));
            let is_default = i64::from(default_remote == Some(name.as_str()));
            records.push(RemoteRecord {
                remote_id,
                name,
                url,
                repo_name: object
                    .get("repo_name")
                    .and_then(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
                is_default_push: is_default,
                is_default_pull: is_default,
                created_at: object
                    .get("created_at")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        records.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(records)
    }
}

impl RemoteStore for ConfigRemoteStore {
    fn remote_exists(&self, name: &str) -> RemoteStoreResult<bool> {
        let name = require_non_empty(name, "remote name")?;
        Ok(self.remote_by_name(&name)?.is_some())
    }

    fn list_remotes(&self) -> RemoteStoreResult<Vec<RemoteRecord>> {
        let config = self.read_config()?;
        self.records_from_config(&config)
    }

    fn remote_by_name(&self, name: &str) -> RemoteStoreResult<Option<RemoteRecord>> {
        let name = require_non_empty(name, "remote name")?;
        Ok(self
            .list_remotes()?
            .into_iter()
            .find(|record| record.name == name))
    }

    fn add_remote(&self, request: &RemoteAddRecord) -> RemoteStoreResult<()> {
        let name = require_non_empty(&request.name, "remote name")?;
        let url = require_non_empty(&request.url, "remote URL")?;
        let created_at = require_non_empty(&request.created_at, "created_at")?;
        let mut config = self.read_config()?;
        {
            let remotes = config
                .entry("remotes".to_string())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or_else(|| ".ait config remotes must be an object.".to_string())?;
            if remotes.contains_key(&name) {
                return Err(format!("Remote {name} already exists."));
            }
            let next_remote_id = remotes
                .values()
                .filter_map(|value| value.get("remote_id").and_then(JsonValue::as_i64))
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| "Remote id overflow.".to_string())?;
            remotes.insert(
                name.clone(),
                json!({
                    "remote_id": next_remote_id,
                    "url": url,
                    "repo_name": request.repo_name,
                    "created_at": created_at,
                }),
            );
        }
        if request.make_default {
            config.insert("default_remote".to_string(), JsonValue::String(name));
        }
        self.write_config(config)
    }
}

fn require_non_empty(value: &str, field: &str) -> RemoteStoreResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field} must not be empty."));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests;
