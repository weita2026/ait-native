use super::*;

type StateJsonReader = fn(&str) -> Result<JsonValue, String>;
type StateJsonWriter = fn(&str, &JsonValue) -> Result<(), String>;
use crate::file_io::{FileIoResult, FileIoStore};
use crate::json_support::json;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct FakeFileIoStore {
    home_dir: Option<PathBuf>,
    files: RefCell<BTreeMap<PathBuf, String>>,
    reads: RefCell<Vec<PathBuf>>,
    writes: RefCell<Vec<(PathBuf, String)>>,
    read_error: Option<String>,
    write_error: Option<String>,
}

impl FakeFileIoStore {
    fn with_home(home_dir: PathBuf) -> Self {
        Self {
            home_dir: Some(home_dir),
            ..Self::default()
        }
    }

    fn insert_file(&self, path: PathBuf, text: &str) {
        self.files.borrow_mut().insert(path, text.to_string());
    }
}

impl FileIoStore for FakeFileIoStore {
    fn home_dir(&self) -> Option<PathBuf> {
        self.home_dir.clone()
    }

    fn path_exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path)
    }

    fn read_to_string(&self, path: &Path) -> FileIoResult<String> {
        self.reads.borrow_mut().push(path.to_path_buf());
        if let Some(err) = &self.read_error {
            return Err(err.clone().into());
        }
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing fake file {}", path.display()).into())
    }

    fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()> {
        let path = path.to_path_buf();
        let text = text.to_string();
        self.writes.borrow_mut().push((path.clone(), text.clone()));
        if let Some(err) = &self.write_error {
            return Err(err.clone().into());
        }
        self.files.borrow_mut().insert(path, text);
        Ok(())
    }

    fn write_string_atomically(
        &self,
        _path: &Path,
        _text: &str,
        _publish_label: &str,
    ) -> FileIoResult<()> {
        Err("repo state tests do not use atomic writes".into())
    }
}

fn unique_temp_path(name: &str) -> PathBuf {
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    std::env::temp_dir().join(format!("ait-repo-state-json-{name}-{nonce}.json"))
}

#[test]
fn missing_state_json_reads_as_null() {
    let path = unique_temp_path("missing");
    assert_eq!(
        read_repo_config_json_file(path.to_str().unwrap()).unwrap(),
        JsonValue::Null
    );
}

#[test]
fn state_json_write_and_read_round_trips_pretty_sorted_json() {
    let path = unique_temp_path("roundtrip");
    let payload = json!({"z": 1, "a": {"b": true}});

    write_worktree_metadata_json_file(path.to_str().unwrap(), &payload).unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}"
    );
    assert_eq!(
        read_worktree_metadata_json_file(path.to_str().unwrap()).unwrap(),
        payload
    );
    let _ = fs::remove_file(path);
}

#[test]
fn invalid_state_json_reports_label() {
    let path = unique_temp_path("invalid");
    fs::write(&path, "{").unwrap();
    let err = read_worktree_config_json_file(path.to_str().unwrap()).unwrap_err();
    assert!(err.contains("Invalid worktree config JSON"));
    let _ = fs::remove_file(path);
}

#[test]
fn invalid_state_json_preserves_exact_public_error_prefixes() {
    let cases: [(&str, StateJsonReader); 3] = [
        ("repo config", read_repo_config_json_file),
        ("worktree config", read_worktree_config_json_file),
        ("worktree metadata", read_worktree_metadata_json_file),
    ];

    for (label, read_fn) in cases {
        let path = unique_temp_path(label.replace(' ', "-").as_str());
        fs::write(&path, "{").unwrap();

        let err = read_fn(path.to_str().unwrap()).unwrap_err();

        assert!(
            err.starts_with(&format!("Invalid {label} JSON {}:", path.display())),
            "{err}"
        );
        let _ = fs::remove_file(path);
    }
}

#[test]
fn failed_state_json_read_preserves_public_error_prefix() {
    let store = FakeFileIoStore {
        read_error: Some("permission denied".to_string()),
        ..FakeFileIoStore::default()
    };
    let path = PathBuf::from("/repo/config.json");
    store.insert_file(path.clone(), "{}");

    let err =
        read_state_json_file_with_file_io_store(&store, path.to_str().unwrap(), "repo config")
            .unwrap_err();

    assert_eq!(
        err,
        "Failed to read repo config JSON /repo/config.json: permission denied"
    );
}

#[test]
fn repo_state_json_wrapper_preserves_trait_object_read_error_prefix() {
    let store = FakeFileIoStore {
        read_error: Some("permission denied".to_string()),
        ..FakeFileIoStore::default()
    };
    let path = PathBuf::from("/repo/worktree-metadata.json");
    store.insert_file(path.clone(), "{}");
    let store_port: &dyn FileIoStore = &store;
    let state_json = RepoStateJson::new(store_port);

    let err = state_json
        .read_state_json_file(path.to_str().unwrap(), "worktree metadata")
        .unwrap_err();

    assert_eq!(
        err,
        "Failed to read worktree metadata JSON /repo/worktree-metadata.json: permission denied"
    );
}

struct FailingSerialize;

impl serde::Serialize for FailingSerialize {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(<S::Error as serde::ser::Error>::custom(
            "cannot encode repo state",
        ))
    }
}

#[test]
fn failed_state_json_encode_preserves_public_error_prefix() {
    let err = encode_state_json(
        &FailingSerialize,
        Path::new("/repo/config.json"),
        "repo config",
    )
    .unwrap_err();

    assert_eq!(
        err,
        "Failed to encode repo config JSON /repo/config.json: cannot encode repo state"
    );
}

#[test]
fn failed_state_json_write_preserves_public_error_prefix() {
    let store = FakeFileIoStore {
        write_error: Some("disk full".to_string()),
        ..FakeFileIoStore::default()
    };

    let err = write_state_json_file_with_file_io_store(
        &store,
        "/repo/worktree.json",
        &json!({"ok": true}),
        "worktree config",
    )
    .unwrap_err();

    assert_eq!(
        err,
        "Failed to write worktree config JSON /repo/worktree.json: disk full"
    );
}

#[test]
fn public_state_json_write_labels_preserve_exact_error_prefixes() {
    let cases: [(&str, StateJsonWriter); 3] = [
        ("repo config", write_repo_config_json_file),
        ("worktree config", write_worktree_config_json_file),
        ("worktree metadata", write_worktree_metadata_json_file),
    ];

    for (label, write_fn) in cases {
        let path = unique_temp_path(label.replace(' ', "-").as_str()).join("state.json");

        let err = write_fn(path.to_str().unwrap(), &json!({"ok": true})).unwrap_err();

        assert!(
            err.starts_with(&format!("Failed to write {label} JSON {}:", path.display())),
            "{err}"
        );
    }
}

#[test]
fn file_io_store_trait_object_expands_home_and_reads_json() {
    let store = FakeFileIoStore::with_home(PathBuf::from("/home/ait"));
    let expected_path = PathBuf::from("/home/ait/.ait/config.json");
    store.insert_file(expected_path.clone(), r#"{"ok":true}"#);
    let store_port: &dyn FileIoStore = &store;

    let value =
        read_state_json_file_with_file_io_store(store_port, "~/.ait/config.json", "repo config")
            .unwrap();

    assert_eq!(value, json!({"ok": true}));
    assert_eq!(*store.reads.borrow(), vec![expected_path]);
}

#[test]
fn file_io_store_trait_object_missing_path_returns_null_without_reading() {
    let store = FakeFileIoStore::default();
    let store_port: &dyn FileIoStore = &store;

    let value =
        read_state_json_file_with_file_io_store(store_port, "/missing/config.json", "repo config")
            .unwrap();

    assert_eq!(value, JsonValue::Null);
    assert!(store.reads.borrow().is_empty());
}

#[test]
fn file_io_store_trait_object_expands_home_and_writes_pretty_json() {
    let store = FakeFileIoStore::with_home(PathBuf::from("/home/ait"));
    let store_port: &dyn FileIoStore = &store;
    let payload = json!({"z": 1, "a": {"b": true}});
    let state_json = RepoStateJson::new(store_port);

    state_json
        .write_state_json_file("~/.ait/worktree.json", &payload, "worktree config")
        .unwrap();

    let writes = store.writes.borrow();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, PathBuf::from("/home/ait/.ait/worktree.json"));
    assert_eq!(
        writes[0].1,
        "{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}"
    );
}
