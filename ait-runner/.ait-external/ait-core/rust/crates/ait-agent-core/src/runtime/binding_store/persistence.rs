use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ait_core::file_io::{
    FileIoByteStore, FileIoDurabilityStore, FileIoLockMode, FileIoLockStore, FileIoLockWait,
    FileIoStore,
};
use ait_core::json_support::{
    write_pretty_json_atomically_with_newline_with_file_io_store, JsonCodec, JsonValue,
};
use ait_core::runtime_binding_state::{
    default_runtime_binding_state_payload_json, normalize_runtime_binding_state_document_json,
};

use super::AgentRuntimeBindingStore;

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

impl AgentRuntimeBindingStore {
    pub fn load(&self) -> Result<JsonValue, String> {
        self.with_process_lock(FileIoLockMode::Shared, || self.load_unlocked())
    }

    pub fn save(&self, payload: &JsonValue) -> Result<JsonValue, String> {
        self.with_process_lock(FileIoLockMode::Exclusive, || {
            self.recover_interrupted_writes_unlocked()?;
            let state = normalized_state(payload);
            self.write_unlocked(&state)?;
            Ok(state)
        })
    }

    pub fn recover_interrupted_writes(&self) -> Result<usize, String> {
        self.with_process_lock(FileIoLockMode::Exclusive, || {
            self.recover_interrupted_writes_unlocked()
        })
    }

    pub(super) fn mutate<F>(&self, mutation: F) -> Result<JsonValue, String>
    where
        F: FnOnce(&mut JsonValue) -> Result<(JsonValue, bool), String>,
    {
        self.with_process_lock(FileIoLockMode::Exclusive, || {
            self.recover_interrupted_writes_unlocked()?;
            let mut state = self.load_unlocked()?;
            let (result, changed) = mutation(&mut state)?;
            if changed {
                let normalized = normalized_state(&state);
                self.write_unlocked(&normalized)?;
            }
            Ok(result)
        })
    }

    fn with_process_lock<T>(
        &self,
        mode: FileIoLockMode,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let local_lock = local_path_lock(&self.lock_path());
        let _local_guard = local_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut file_guard = self
            .io
            .acquire_process_lock(&self.lock_path(), mode, FileIoLockWait::Blocking)
            .map_err(|error| {
                format!(
                    "failed to acquire ait-agent runtime binding lock '{}': {error}",
                    self.lock_path().display()
                )
            })?
            .ok_or_else(|| {
                format!(
                    "ait-agent runtime binding lock '{}' is busy",
                    self.lock_path().display()
                )
            })?;
        let result = operation();
        let release = file_guard.release().map_err(|error| {
            format!(
                "failed to release ait-agent runtime binding lock '{}': {error}",
                self.lock_path().display()
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

    fn load_unlocked(&self) -> Result<JsonValue, String> {
        if !self.io.path_exists(&self.path) {
            return Ok(default_runtime_binding_state_payload_json());
        }
        let raw = self.io.read_to_string(&self.path).map_err(|error| {
            format!(
                "failed to read ait-agent runtime binding state '{}': {error}",
                self.path.display()
            )
        })?;
        let payload = JsonCodec::parse_value_with_error_prefix(
            &raw,
            "invalid ait-agent runtime binding state",
        )
        .map_err(String::from)?;
        Ok(normalized_state(&payload))
    }

    fn write_unlocked(&self, payload: &JsonValue) -> Result<(), String> {
        write_pretty_json_atomically_with_newline_with_file_io_store(
            &self.io,
            &self.path,
            payload,
            "ait-agent runtime binding state",
        )
        .map_err(|error| {
            format!(
                "failed to publish ait-agent runtime binding state '{}': {error}",
                self.path.display()
            )
        })?;
        if let Some(parent) = self.path.parent() {
            self.io.sync_dir(parent).map_err(|error| {
                format!(
                    "failed to sync ait-agent runtime binding directory '{}': {error}",
                    parent.display()
                )
            })?;
        }
        Ok(())
    }

    fn recover_interrupted_writes_unlocked(&self) -> Result<usize, String> {
        let Some(parent) = self.path.parent() else {
            return Ok(0);
        };
        let Some(file_name) = self.path.file_name().and_then(|value| value.to_str()) else {
            return Ok(0);
        };
        let prefix = format!("{file_name}.tmp-");
        let paths = match self.io.list_directory_paths(parent) {
            Ok(paths) => paths,
            Err(_error) if !parent.exists() => return Ok(0),
            Err(error) => {
                return Err(format!(
                    "failed to inspect ait-agent runtime binding directory '{}': {error}",
                    parent.display()
                ))
            }
        };
        let mut recovered = 0;
        for path in paths {
            let matches = path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.starts_with(&prefix));
            if !matches {
                continue;
            }
            self.io.remove_file_if_exists(&path).map_err(|error| {
                format!(
                    "failed to remove interrupted ait-agent runtime binding write '{}': {error}",
                    path.display()
                )
            })?;
            recovered += 1;
        }
        Ok(recovered)
    }
}

pub(super) fn normalized_state(payload: &JsonValue) -> JsonValue {
    normalize_runtime_binding_state_document_json(payload)
        .get("state")
        .cloned()
        .unwrap_or_else(default_runtime_binding_state_payload_json)
}
