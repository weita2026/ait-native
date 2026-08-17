use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use ait_core::file_io::{FileIoLockMode, FileIoLockStore, FileIoLockWait, FilesystemFileIoStore};
use ait_core::json_support::{
    json, write_pretty_json_atomically_with_newline_with_file_io_store, JsonValue,
};

use crate::json_support::parse_value;

use super::{
    agent_default_worker_manifest_config_json, agent_normalize_worker_manifest_document_json,
    agent_upsert_worker_manifest_worker_json,
};

const MANIFEST_PUBLISH_LABEL: &str = "ait-agent worker manifest";
pub const AGENT_WORKER_MANIFEST_STORE_CONTRACT: &str = "ait.agent.worker_manifest_store.v1";

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

#[derive(Debug, Clone, PartialEq)]
pub struct AgentWorkerManifestMutation {
    pub config: JsonValue,
    pub worker_key: String,
    pub worker: JsonValue,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentWorkerManifestRemoval {
    pub removed: bool,
    pub worker_key: String,
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
        match self.with_process_lock(FileIoLockMode::Shared, || Ok(self.load_unlocked())) {
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
            Err(err) => {
                return self.invalid_document(format!(
                    "Invalid JSON in worker config at {}: {err}",
                    self.path.display()
                ));
            }
        };
        let payload = match parse_value(
            &raw,
            &format!("Invalid JSON in worker config at {}", self.path.display()),
        ) {
            Ok(payload) => payload,
            Err(err) => return self.invalid_document(err),
        };
        normalized_document(&payload, &self.path)
    }

    pub fn save(&self, payload: &JsonValue) -> Result<AgentWorkerManifestDocument, String> {
        self.with_process_lock(FileIoLockMode::Exclusive, || self.save_unlocked(payload))
    }

    fn save_unlocked(&self, payload: &JsonValue) -> Result<AgentWorkerManifestDocument, String> {
        let document = normalized_document(payload, &self.path);
        write_pretty_json_atomically_with_newline_with_file_io_store(
            &self.file_io,
            &self.path,
            &document.config,
            MANIFEST_PUBLISH_LABEL,
        )?;
        Ok(document)
    }

    pub fn upsert(
        &self,
        worker: JsonValue,
        updated_at: Option<&str>,
    ) -> Result<AgentWorkerManifestMutation, String> {
        self.with_process_lock(FileIoLockMode::Exclusive, || {
            self.upsert_unlocked(worker, updated_at)
        })
    }

    fn upsert_unlocked(
        &self,
        worker: JsonValue,
        updated_at: Option<&str>,
    ) -> Result<AgentWorkerManifestMutation, String> {
        let loaded = self.load_unlocked();
        let mut request = ait_core::json_support::json!({
            "config": loaded.config,
            "worker": worker,
            "path": self.path.to_string_lossy(),
        });
        if let Some(updated_at) = clean_optional_text(updated_at) {
            request["updated_at"] = JsonValue::String(updated_at);
        }
        let result = agent_upsert_worker_manifest_worker_json(&request)?;
        let config = result.get("config").cloned().ok_or_else(|| {
            "Rust ait-agent worker manifest upsert contract omitted config.".to_string()
        })?;
        let worker = result.get("worker").cloned().ok_or_else(|| {
            "Rust ait-agent worker manifest upsert contract omitted worker.".to_string()
        })?;
        let worker_key = result
            .get("worker_key")
            .and_then(JsonValue::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "Rust ait-agent worker manifest upsert contract omitted worker_key.".to_string()
            })?
            .to_string();
        let mut issues = loaded.issues;
        issues.extend(string_array(result.get("issues")));
        let saved = self.save_unlocked(&config)?;
        issues.extend(saved.issues);
        Ok(AgentWorkerManifestMutation {
            config: saved.config,
            worker_key,
            worker,
            issues,
        })
    }

    pub fn remove(&self, kind: &str, name: &str) -> Result<AgentWorkerManifestRemoval, String> {
        self.with_process_lock(FileIoLockMode::Exclusive, || {
            self.remove_unlocked(kind, name)
        })
    }

    fn remove_unlocked(
        &self,
        kind: &str,
        name: &str,
    ) -> Result<AgentWorkerManifestRemoval, String> {
        let worker_key = worker_key(kind, name)?;
        let mut config = self.load_unlocked().config;
        let workers = config
            .get_mut("workers")
            .and_then(JsonValue::as_object_mut)
            .ok_or_else(|| "Normalized worker manifest omitted workers map.".to_string())?;
        let removed = workers.remove(&worker_key).is_some();
        self.save_unlocked(&config)?;
        Ok(AgentWorkerManifestRemoval {
            removed,
            worker_key,
        })
    }

    fn with_process_lock<T>(
        &self,
        mode: FileIoLockMode,
        operation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let lock_path = self.lock_path();
        let local_lock = local_path_lock(&lock_path);
        let _local_guard = local_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut file_guard = self
            .file_io
            .acquire_process_lock(&lock_path, mode, FileIoLockWait::Blocking)
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

pub fn agent_worker_manifest_store_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "ait-agent worker manifest store request must be an object".to_string())?;
    let path = required_text(request.get("path"), "path")?;
    let operation = required_text(request.get("operation"), "operation")?;
    let store = AgentWorkerManifestStore::filesystem(path);
    let result = match operation.as_str() {
        "load" => manifest_document_json(store.load()),
        "save" => {
            let payload = request
                .get("config")
                .or_else(|| request.get("payload"))
                .ok_or_else(|| {
                    "ait-agent worker manifest save operation requires config".to_string()
                })?;
            manifest_document_json(store.save(payload)?)
        }
        "upsert" => {
            let worker = request.get("worker").cloned().ok_or_else(|| {
                "ait-agent worker manifest upsert operation requires worker".to_string()
            })?;
            let updated_at = optional_text(request.get("updated_at"), "updated_at")?;
            let mutation = store.upsert(worker, updated_at.as_deref())?;
            json!({
                "config": mutation.config,
                "worker_key": mutation.worker_key,
                "worker": mutation.worker,
                "issues": mutation.issues,
            })
        }
        "remove" => {
            let kind = required_text(request.get("kind"), "kind")?;
            let name = required_text(request.get("name"), "name")?;
            let removal = store.remove(&kind, &name)?;
            json!({
                "removed": removal.removed,
                "worker_key": removal.worker_key,
            })
        }
        other => {
            return Err(format!(
                "unsupported ait-agent worker manifest store operation '{other}'"
            ))
        }
    };
    Ok(json!({
        "contract": AGENT_WORKER_MANIFEST_STORE_CONTRACT,
        "operation": operation,
        "result": result,
        "python_file_read_allowed": false,
        "python_file_mutation_allowed": false,
    }))
}

fn manifest_document_json(document: AgentWorkerManifestDocument) -> JsonValue {
    json!({
        "config": document.config,
        "issues": document.issues,
    })
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

fn worker_key(kind: &str, name: &str) -> Result<String, String> {
    let kind = clean_optional_text(Some(kind))
        .ok_or_else(|| "Worker kind must not be empty.".to_string())?
        .to_ascii_lowercase();
    let name = clean_optional_text(Some(name))
        .ok_or_else(|| "Worker name must not be empty.".to_string())?;
    if kind.contains('/') {
        return Err("Worker kind must not contain '/'.".to_string());
    }
    if name.contains('/') {
        return Err("Worker name must not contain '/'.".to_string());
    }
    Ok(format!("{kind}/{name}"))
}

fn clean_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    optional_text(value, field)?
        .ok_or_else(|| format!("ait-agent worker manifest store request requires {field}"))
}

fn optional_text(value: Option<&JsonValue>, field: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(clean_optional_text(Some(value))),
        Some(_) => Err(format!(
            "ait-agent worker manifest store request field `{field}` must be a string or null"
        )),
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
    use std::process::Command;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{Duration, Instant};

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
    fn invalid_manifest_fails_safe_with_diagnostic() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent-workers.json");
        fs::write(&path, "{not-json").unwrap();
        let store = AgentWorkerManifestStore::filesystem(&path);

        let document = store.load();

        assert_eq!(document.config, json!({"version": 1, "workers": {}}));
        assert_eq!(document.issues.len(), 1);
        assert!(document.issues[0].contains("Invalid JSON in worker config"));
        assert!(document.issues[0].contains(path.to_string_lossy().as_ref()));
    }

    #[test]
    fn upsert_and_remove_publish_normalized_manifest_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".ait/agent-workers.json");
        let store = AgentWorkerManifestStore::filesystem(&path);

        let first = store
            .upsert(
                json!({
                    "kind": "telegram",
                    "name": "main",
                    "token": "secret-token",
                    "username": "bot"
                }),
                Some("2026-07-16T00:00:00Z"),
            )
            .unwrap();
        let second = store
            .upsert(
                json!({
                    "kind": "telegram",
                    "name": "main",
                    "token": "new-token",
                    "username": "bot"
                }),
                Some("2026-07-16T01:00:00Z"),
            )
            .unwrap();

        assert_eq!(first.worker_key, "telegram/main");
        assert_eq!(first.worker["created_at"], "2026-07-16T00:00:00Z");
        assert_eq!(second.worker["created_at"], "2026-07-16T00:00:00Z");
        assert_eq!(second.worker["updated_at"], "2026-07-16T01:00:00Z");
        assert_eq!(second.worker["token"], "new-token");
        let persisted = parse_value(&fs::read_to_string(&path).unwrap(), "manifest").unwrap();
        assert_eq!(persisted["workers"]["telegram/main"]["token"], "new-token");
        assert!(fs::read_dir(path.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("tmp-")));

        let removal = store.remove("telegram", "main").unwrap();
        assert!(removal.removed);
        assert_eq!(removal.worker_key, "telegram/main");
        assert_eq!(store.load().config, json!({"version": 1, "workers": {}}));
    }

    #[test]
    fn removal_rejects_unsafe_worker_names() {
        let temp = tempfile::tempdir().unwrap();
        let store = AgentWorkerManifestStore::filesystem(temp.path().join("workers.json"));

        assert_eq!(
            store.remove("telegram", "bad/name").unwrap_err(),
            "Worker name must not contain '/'."
        );
    }

    #[test]
    fn concurrent_upserts_hold_one_lock_across_read_modify_write() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".ait/agent-workers.json");
        let barrier = Arc::new(Barrier::new(9));
        let mut threads = Vec::new();
        for index in 0..8 {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                AgentWorkerManifestStore::filesystem(path)
                    .upsert(
                        json!({
                            "kind": "telegram",
                            "name": format!("worker-{index}"),
                            "token": format!("token-{index}"),
                        }),
                        Some("2026-07-17T00:00:00Z"),
                    )
                    .unwrap();
            }));
        }
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }

        let document = AgentWorkerManifestStore::filesystem(path).load();
        assert!(document.issues.is_empty());
        assert_eq!(
            document.config["workers"]
                .as_object()
                .map(|workers| workers.len()),
            Some(8)
        );
    }

    #[test]
    fn worker_manifest_store_child_process_upsert() {
        let Ok(path) = std::env::var("AIT_TEST_WORKER_MANIFEST_CHILD_PATH") else {
            return;
        };
        let name = std::env::var("AIT_TEST_WORKER_MANIFEST_CHILD_NAME").expect("child name");
        let barrier_dir = std::env::var("AIT_TEST_WORKER_MANIFEST_BARRIER_DIR")
            .map(PathBuf::from)
            .expect("barrier dir");
        fs::write(barrier_dir.join(format!("{name}.ready")), "ready").expect("ready marker");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !barrier_dir.join("go").exists() {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for process barrier"
            );
            thread::sleep(Duration::from_millis(5));
        }
        AgentWorkerManifestStore::filesystem(path)
            .upsert(
                json!({"kind": "line", "name": name, "token": "secret"}),
                Some("2026-07-17T00:00:00Z"),
            )
            .expect("child upsert");
    }

    #[test]
    fn worker_manifest_processes_cannot_overwrite_each_other() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".ait/agent-workers.json");
        let barrier_dir = temp.path().join("barrier");
        fs::create_dir(&barrier_dir).expect("barrier");
        let executable = std::env::current_exe().expect("test executable");
        let test_name = "manifest::store::tests::worker_manifest_store_child_process_upsert";
        let mut children = Vec::new();
        for index in 0..6 {
            let name = format!("child-{index}");
            children.push(
                Command::new(&executable)
                    .arg("--exact")
                    .arg(test_name)
                    .arg("--nocapture")
                    .env("AIT_TEST_WORKER_MANIFEST_CHILD_PATH", &path)
                    .env("AIT_TEST_WORKER_MANIFEST_CHILD_NAME", &name)
                    .env("AIT_TEST_WORKER_MANIFEST_BARRIER_DIR", &barrier_dir)
                    .spawn()
                    .expect("child process"),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let ready = fs::read_dir(&barrier_dir)
                .expect("barrier entries")
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().ends_with(".ready"))
                .count();
            if ready == children.len() {
                break;
            }
            assert!(Instant::now() < deadline, "children did not reach barrier");
            thread::sleep(Duration::from_millis(10));
        }
        fs::write(barrier_dir.join("go"), "go").expect("release children");
        for mut child in children {
            assert!(child.wait().expect("child status").success());
        }

        let document = AgentWorkerManifestStore::filesystem(path).load();
        assert!(document.issues.is_empty());
        assert_eq!(
            document.config["workers"]
                .as_object()
                .map(|workers| workers.len()),
            Some(6)
        );
    }

    #[test]
    fn versioned_store_contract_executes_filesystem_operations() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".ait/agent-workers.json");
        let upsert = agent_worker_manifest_store_execute_json(&json!({
            "operation": "upsert",
            "path": path,
            "worker": {"kind": "discord", "name": "main", "bot_token": "secret"},
            "updated_at": "2026-07-17T00:00:00Z",
        }))
        .unwrap();
        assert_eq!(upsert["contract"], AGENT_WORKER_MANIFEST_STORE_CONTRACT);
        assert_eq!(upsert["operation"], "upsert");
        assert_eq!(upsert["result"]["worker_key"], "discord/main");
        assert_eq!(upsert["python_file_read_allowed"], false);
        assert_eq!(upsert["python_file_mutation_allowed"], false);

        let loaded = agent_worker_manifest_store_execute_json(&json!({
            "operation": "load",
            "path": path,
        }))
        .unwrap();
        assert_eq!(
            loaded["result"]["config"]["workers"]["discord/main"]["bot_token"],
            "secret"
        );

        let removed = agent_worker_manifest_store_execute_json(&json!({
            "operation": "remove",
            "path": path,
            "kind": "discord",
            "name": "main",
        }))
        .unwrap();
        assert_eq!(removed["result"]["removed"], true);
        assert_eq!(removed["result"]["worker_key"], "discord/main");
    }
}
