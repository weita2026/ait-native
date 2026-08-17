use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ait_core::file_io::{FileIoLockMode, FileIoLockStore, FileIoLockWait, FilesystemFileIoStore};
use ait_core::json_support::JsonValue;

use crate::json_support::parse_value;

use super::{
    agent_default_worker_manifest_config_json, agent_normalize_worker_manifest_document_json,
};

type LocalPathLock = Arc<Mutex<()>>;

fn local_path_locks() -> &'static Mutex<BTreeMap<PathBuf, LocalPathLock>> {
    static LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, LocalPathLock>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn local_path_lock(path: &Path) -> LocalPathLock {
    let mut locks = local_path_locks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentWorkerManifestDocument {
    pub config: JsonValue,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentWorkerManifestStore<S = FilesystemFileIoStore> {
    path: PathBuf,
    file_io: S,
}

impl AgentWorkerManifestStore<FilesystemFileIoStore> {
    pub fn filesystem(path: impl Into<PathBuf>) -> Self {
        Self::new(path, FilesystemFileIoStore)
    }
}

impl<S> AgentWorkerManifestStore<S>
where
    S: FileIoLockStore,
{
    pub fn new(path: impl Into<PathBuf>, file_io: S) -> Self {
        Self {
            path: path.into(),
            file_io,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lock_path(&self) -> PathBuf {
        let mut value = self.path.as_os_str().to_os_string();
        value.push(".lock");
        PathBuf::from(value)
    }

    pub fn load(&self) -> AgentWorkerManifestDocument {
        match self.with_shared_lock(|| Ok(self.load_unlocked())) {
            Ok(document) => document,
            Err(error) => self.invalid_document(error),
        }
    }

    fn load_unlocked(&self) -> AgentWorkerManifestDocument {
        if !self.file_io.path_exists(&self.path) {
            return AgentWorkerManifestDocument {
                config: agent_default_worker_manifest_config_json(),
                issues: Vec::new(),
            };
        }
        let raw = match self.file_io.read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) => {
                return self.invalid_document(format!(
                    "Invalid JSON in worker config at {}: {error}",
                    self.path.display()
                ));
            }
        };
        let payload = match parse_value(
            &raw,
            &format!("Invalid JSON in worker config at {}", self.path.display()),
        ) {
            Ok(payload) => payload,
            Err(error) => return self.invalid_document(error),
        };
        normalized_document(&payload, &self.path)
    }

    fn with_shared_lock<T>(
        &self,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let lock_path = self.lock_path();
        let local_lock = local_path_lock(&lock_path);
        let _local_guard = local_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut file_guard = self
            .file_io
            .acquire_process_lock(&lock_path, FileIoLockMode::Shared, FileIoLockWait::Blocking)
            .map_err(|error| {
                format!(
                    "failed to acquire ait-agent worker manifest lock '{}': {error}",
                    lock_path.display()
                )
            })?
            .ok_or_else(|| {
                format!(
                    "ait-agent worker manifest lock '{}' is busy",
                    lock_path.display()
                )
            })?;
        let result = operation();
        let release = file_guard.release().map_err(|error| {
            format!(
                "failed to release ait-agent worker manifest lock '{}': {error}",
                lock_path.display()
            )
        });
        match (result, release) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(release_error)) => {
                Err(format!("{error}; additionally, {release_error}"))
            }
        }
    }

    fn invalid_document(&self, issue: String) -> AgentWorkerManifestDocument {
        AgentWorkerManifestDocument {
            config: agent_default_worker_manifest_config_json(),
            issues: vec![issue],
        }
    }
}

fn normalized_document(payload: &JsonValue, path: &Path) -> AgentWorkerManifestDocument {
    let document = agent_normalize_worker_manifest_document_json(
        payload,
        Some(path.to_string_lossy().as_ref()),
    );
    AgentWorkerManifestDocument {
        config: document
            .get("config")
            .cloned()
            .unwrap_or_else(agent_default_worker_manifest_config_json),
        issues: string_array(document.get("issues")),
    }
}

fn string_array(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ait_core::json_support::json;

    use super::*;

    #[test]
    fn missing_manifest_loads_default_without_creating_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".ait/agent-workers.json");
        let store = AgentWorkerManifestStore::filesystem(&path);

        let document = store.load();

        assert_eq!(document.config, json!({"version": 1, "workers": {}}));
        assert!(document.issues.is_empty());
        assert!(!path.exists());
    }

    #[test]
    fn valid_manifest_is_normalized_for_runtime_consumers() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-workers.json");
        fs::write(
            &path,
            r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main","token":"secret"}}}"#,
        )
        .unwrap();

        let document = AgentWorkerManifestStore::filesystem(&path).load();

        assert!(document.issues.is_empty());
        assert_eq!(document.config["workers"]["telegram/main"]["name"], "main");
    }

    #[test]
    fn invalid_manifest_fails_safe_with_diagnostic() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-workers.json");
        fs::write(&path, "{not-json").unwrap();

        let document = AgentWorkerManifestStore::filesystem(&path).load();

        assert_eq!(document.config, json!({"version": 1, "workers": {}}));
        assert_eq!(document.issues.len(), 1);
        assert!(document.issues[0].contains("Invalid JSON in worker config"));
    }
}
