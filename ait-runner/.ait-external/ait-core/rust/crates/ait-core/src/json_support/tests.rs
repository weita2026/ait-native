use super::*;
use crate::file_io::{FileIoResult, FileIoStore, FilesystemFileIoStore};
use crate::json_support::json;
use crate::json_support::JsonValue;
use serde::{Deserialize, Serialize};
use serde_json::Map as JsonMap;
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
    atomic_writes: RefCell<Vec<(PathBuf, String, String)>>,
    atomic_failure: Option<FakeAtomicFailure>,
}

#[derive(Clone, Copy)]
enum FakeAtomicFailure {
    TempWrite,
    Rename,
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
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| format!("missing fake file {}", path.display()).into())
    }

    fn write_string(&self, path: &Path, text: &str) -> FileIoResult<()> {
        self.writes
            .borrow_mut()
            .push((path.to_path_buf(), text.to_string()));
        Ok(())
    }

    fn write_string_atomically(
        &self,
        path: &Path,
        text: &str,
        publish_label: &str,
    ) -> FileIoResult<()> {
        let path = path.to_path_buf();
        let text = text.to_string();
        self.atomic_writes.borrow_mut().push((
            path.clone(),
            text.clone(),
            publish_label.to_string(),
        ));
        match self.atomic_failure {
            Some(FakeAtomicFailure::TempWrite) => Err(format!(
                "Failed to write fake temp for {}: disk full",
                path.display()
            )
            .into()),
            Some(FakeAtomicFailure::Rename) => Err(format!(
                "Failed to publish {publish_label} fake.tmp -> {}: permission denied",
                path.display()
            )
            .into()),
            None => {
                self.files.borrow_mut().insert(path, text);
                Ok(())
            }
        }
    }
}

fn unique_temp_dir(name: &str) -> PathBuf {
    let nonce = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    std::env::temp_dir().join(format!("ait-json-support-{name}-{nonce}"))
}

fn object(value: JsonValue) -> JsonMap<String, JsonValue> {
    value.as_object().unwrap().clone()
}

#[test]
fn json_fields_required_and_optional_text_errors_are_stable() {
    let payload = object(json!({
        "name": "ait",
        "empty": "",
        "none": null,
        "count": 3
    }));

    assert_eq!(required_text_field(&payload, "name").unwrap(), "ait");
    assert_eq!(required_text_field(&payload, "empty").unwrap(), "");
    assert_eq!(
        optional_text_field(&payload, "missing").unwrap(),
        None::<String>
    );
    assert_eq!(
        optional_text_field(&payload, "none").unwrap(),
        None::<String>
    );

    assert_eq!(
        required_text_field(&payload, "missing")
            .unwrap_err()
            .to_string(),
        "Missing required field `missing`."
    );
    assert_eq!(
        required_text_field(&payload, "none")
            .unwrap_err()
            .to_string(),
        "Field `none` must be a JSON string, got null."
    );
    assert_eq!(
        optional_text_field(&payload, "count")
            .unwrap_err()
            .to_string(),
        "Field `count` must be a JSON string or null, got number."
    );
}

#[test]
fn json_fields_bool_integer_object_array_and_path_errors_are_stable() {
    let payload = object(json!({
        "enabled": true,
        "count": 42,
        "float": 1.25,
        "repo": {"name": "ait-core"},
        "items": [1, 2],
        "path": "runtime/report.json",
        "none": null,
        "text": "value"
    }));

    assert!(required_bool_field(&payload, "enabled").unwrap());
    assert_eq!(optional_bool_field(&payload, "none").unwrap(), None);
    assert_eq!(required_integer_field(&payload, "count").unwrap(), 42);
    assert_eq!(optional_integer_field(&payload, "none").unwrap(), None);
    assert_eq!(
        required_object_field(&payload, "repo").unwrap()["name"],
        "ait-core"
    );
    assert_eq!(required_array_field(&payload, "items").unwrap().len(), 2);
    assert_eq!(
        required_path_field(&payload, "path").unwrap(),
        PathBuf::from("runtime/report.json")
    );
    assert_eq!(optional_path_field(&payload, "none").unwrap(), None);

    assert_eq!(
        required_bool_field(&payload, "text")
            .unwrap_err()
            .to_string(),
        "Field `text` must be a JSON boolean, got string."
    );
    assert_eq!(
        required_integer_field(&payload, "float")
            .unwrap_err()
            .to_string(),
        "Field `float` must be a JSON integer, got number."
    );
    assert_eq!(
        required_object_field(&payload, "items")
            .unwrap_err()
            .to_string(),
        "Field `items` must be a JSON object, got array."
    );
    assert_eq!(
        optional_array_field(&payload, "text")
            .unwrap_err()
            .to_string(),
        "Field `text` must be a JSON array or null, got string."
    );
    assert_eq!(
        required_object_value(&json!(null), "payload")
            .unwrap_err()
            .to_string(),
        "payload must be a JSON object, got null."
    );
    assert_eq!(
        required_array_value(&json!("items"), "items")
            .unwrap_err()
            .to_string(),
        "items must be a JSON array, got string."
    );
}

#[test]
fn json_codec_encodes_compact_pretty_and_trailing_newline() {
    let payload = json!({"z": 1, "a": {"b": true}});

    assert_eq!(
        JsonCodec::encode_value(&payload, JsonEncodeOptions::compact()).unwrap(),
        "{\"a\":{\"b\":true},\"z\":1}"
    );
    assert_eq!(
        JsonCodec::encode_value(&payload, JsonEncodeOptions::pretty()).unwrap(),
        "{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}"
    );
    assert_eq!(
        JsonCodec::encode_value(
            &payload,
            JsonEncodeOptions::pretty().with_trailing_newline()
        )
        .unwrap(),
        "{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}\n"
    );
    assert_eq!(
        JsonCodec::encode_value_to_vec(&payload, JsonEncodeOptions::compact()).unwrap(),
        b"{\"a\":{\"b\":true},\"z\":1}".to_vec()
    );
    assert_eq!(
        JsonCodec::encode_value_to_vec_with_error_prefix(
            &payload,
            JsonEncodeOptions::pretty(),
            "Could not encode payload",
        )
        .unwrap(),
        b"{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}".to_vec()
    );
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct JsonCodecTypedFixture {
    name: String,
    count: u32,
}

#[test]
fn json_codec_encodes_and_decodes_typed_payloads_with_stable_prefixes() {
    let payload = JsonCodecTypedFixture {
        name: "ait".to_string(),
        count: 3,
    };

    assert_eq!(
        JsonCodec::encode_serializable(&payload, JsonEncodeOptions::compact()).unwrap(),
        "{\"name\":\"ait\",\"count\":3}"
    );
    assert_eq!(
        JsonCodec::encode_serializable_with_error_prefix(
            &payload,
            JsonEncodeOptions::pretty().with_trailing_newline(),
            "Failed to encode fixture",
        )
        .unwrap(),
        "{\n  \"name\": \"ait\",\n  \"count\": 3\n}\n"
    );
    assert_eq!(
        JsonCodec::encode_serializable_to_vec_with_error_prefix(
            &payload,
            JsonEncodeOptions::compact(),
            "Failed to encode fixture bytes",
        )
        .unwrap(),
        b"{\"name\":\"ait\",\"count\":3}".to_vec()
    );
    let value = JsonCodec::to_value_serializable(&payload).unwrap();
    assert_eq!(value, json!({"name": "ait", "count": 3}));
    assert_eq!(
        JsonCodec::from_value_deserializable::<JsonCodecTypedFixture>(value).unwrap(),
        payload
    );
    assert!(
        JsonCodec::from_value_deserializable::<JsonCodecTypedFixture>(json!({
            "name": "ait",
            "count": "bad",
        }))
        .unwrap_err()
        .to_string()
        .contains("invalid type")
    );
    assert_eq!(
        JsonCodec::parse_deserializable_with_error_prefix::<JsonCodecTypedFixture>(
            r#"{"name":"ait","count":3}"#,
            "Failed to decode fixture",
        )
        .unwrap(),
        payload
    );
    assert!(
        JsonCodec::parse_deserializable_with_error_prefix::<JsonCodecTypedFixture>(
            "{",
            "Failed to decode fixture",
        )
        .unwrap_err()
        .to_string()
        .starts_with("Failed to decode fixture: ")
    );
}

#[test]
fn json_codec_parse_errors_include_stable_labels() {
    let err = JsonCodec::parse_value("{", "plan request").unwrap_err();
    assert!(err.to_string().starts_with("Invalid plan request JSON: "));

    assert_eq!(
        JsonCodec::parse_object("[]", "task response")
            .unwrap_err()
            .to_string(),
        "task response JSON must be an object."
    );
    assert!(
        JsonCodec::parse_value_with_error_prefix("{", "payload must be valid JSON")
            .unwrap_err()
            .to_string()
            .starts_with("payload must be valid JSON: ")
    );
    assert!(
        JsonCodec::parse_slice_with_error_prefix(b"{", "bytes must be valid JSON")
            .unwrap_err()
            .to_string()
            .starts_with("bytes must be valid JSON: ")
    );
    assert_eq!(
        JsonCodec::parse_object_with_error_prefix(
            "[]",
            "payload must be valid JSON",
            "payload must be an object.",
        )
        .unwrap_err()
        .to_string(),
        "payload must be an object."
    );
}

#[test]
fn json_helper_errors_convert_to_plain_cli_and_http_messages() {
    let payload = JsonMap::new();
    let field_error = required_text_field(&payload, "repo_name").unwrap_err();
    let cli_message = format!("invalid request: {field_error}");
    let http_body = json!({"error": field_error.to_string()});
    let native_error_kind = "bad_request";
    let native_error_message = field_error.to_string();

    assert_eq!(
        cli_message,
        "invalid request: Missing required field `repo_name`."
    );
    assert_eq!(
        http_body,
        json!({"error": "Missing required field `repo_name`."})
    );
    assert_eq!(native_error_kind, "bad_request");
    assert_eq!(native_error_message, "Missing required field `repo_name`.");
}

#[test]
fn json_file_store_expands_home_and_reads_json() {
    let store = FakeFileIoStore::with_home(PathBuf::from("/home/ait"));
    let expected_path = PathBuf::from("/home/ait/.ait/config.json");
    store.insert_file(expected_path.clone(), r#"{"ok":true}"#);

    let value = store
        .read_json_or_null("~/.ait/config.json", "repo config")
        .unwrap();

    assert_eq!(value, json!({"ok": true}));
    assert_eq!(*store.reads.borrow(), vec![expected_path]);
}

#[test]
fn json_file_store_trait_object_reads_and_writes_through_shared_methods() {
    let store = FakeFileIoStore::with_home(PathBuf::from("/home/ait"));
    let read_path = PathBuf::from("/home/ait/.ait/config.json");
    let write_path = PathBuf::from("/home/ait/.ait/worktree.json");
    let atomic_path = PathBuf::from("/cache/manifest.json");
    store.insert_file(read_path.clone(), r#"{"ok":true}"#);
    let store_port: &dyn JsonFileStore = &store;

    let value = store_port
        .read_json_or_null("~/.ait/config.json", "repo config")
        .unwrap();
    assert_eq!(value, json!({"ok": true}));

    store_port
        .write_pretty_json(
            "~/.ait/worktree.json",
            &json!({"ok": true}),
            "worktree config",
        )
        .unwrap();
    store_port
        .write_pretty_json_atomically_with_newline(
            &atomic_path,
            &json!({"state": "ready"}),
            "current-source JSON",
        )
        .unwrap();

    assert_eq!(*store.reads.borrow(), vec![read_path]);
    assert_eq!(
        store.writes.borrow().as_slice(),
        &[(write_path, "{\n  \"ok\": true\n}".to_string())]
    );
    assert_eq!(
        store.atomic_writes.borrow().as_slice(),
        &[(
            atomic_path,
            "{\n  \"state\": \"ready\"\n}\n".to_string(),
            "current-source JSON".to_string()
        )]
    );
}

#[test]
fn json_file_store_missing_path_returns_null_without_reading() {
    let store = FakeFileIoStore::default();

    let value = store
        .read_json_or_null("/missing/config.json", "repo config")
        .unwrap();

    assert_eq!(value, JsonValue::Null);
    assert!(store.reads.borrow().is_empty());
}

#[test]
fn json_file_store_writes_pretty_json_without_trailing_newline() {
    let store = FakeFileIoStore::with_home(PathBuf::from("/home/ait"));
    let payload = json!({"z": 1, "a": {"b": true}});

    store
        .write_pretty_json("~/.ait/worktree.json", &payload, "worktree config")
        .unwrap();

    let writes = store.writes.borrow();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, PathBuf::from("/home/ait/.ait/worktree.json"));
    assert_eq!(
        writes[0].1,
        "{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}"
    );
}

#[test]
fn json_file_store_reads_object_or_empty_for_missing_malformed_or_non_object() {
    let store = FakeFileIoStore::default();
    let missing = PathBuf::from("/missing/manifest.json");
    let malformed = PathBuf::from("/tmp/malformed.json");
    let array = PathBuf::from("/tmp/array.json");
    let object = PathBuf::from("/tmp/object.json");
    store.insert_file(malformed.clone(), "{");
    store.insert_file(array.clone(), "[]");
    store.insert_file(object.clone(), r#"{"state":"ready"}"#);

    assert!(store.read_json_object_or_empty(&missing).is_empty());
    assert!(store.read_json_object_or_empty(&malformed).is_empty());
    assert!(store.read_json_object_or_empty(&array).is_empty());
    assert_eq!(
        store
            .read_json_object_or_empty(&object)
            .get("state")
            .and_then(JsonValue::as_str),
        Some("ready")
    );
}

#[test]
fn json_file_store_atomic_pretty_write_adds_trailing_newline() {
    let store = FakeFileIoStore::default();
    let path = PathBuf::from("/cache/manifest.json");
    let payload = json!({"z": 1, "a": {"b": true}});

    store
        .write_pretty_json_atomically_with_newline(&path, &payload, "current-source JSON")
        .unwrap();

    let writes = store.atomic_writes.borrow();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, path);
    assert_eq!(
        writes[0].1,
        "{\n  \"a\": {\n    \"b\": true\n  },\n  \"z\": 1\n}\n"
    );
    assert_eq!(writes[0].2, "current-source JSON");
}

#[test]
fn json_file_store_atomic_pretty_write_propagates_temp_write_failure() {
    let store = FakeFileIoStore {
        atomic_failure: Some(FakeAtomicFailure::TempWrite),
        ..FakeFileIoStore::default()
    };
    let path = PathBuf::from("/cache/manifest.json");
    store.insert_file(path.clone(), "{\"old\":true}\n");

    let err = store
        .write_pretty_json_atomically_with_newline(
            &path,
            &json!({"state": "ready"}),
            "current-source JSON",
        )
        .unwrap_err();

    assert_eq!(
        err,
        "Failed to write fake temp for /cache/manifest.json: disk full"
    );
    let writes = store.atomic_writes.borrow();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, path);
    assert_eq!(writes[0].1, "{\n  \"state\": \"ready\"\n}\n");
    assert_eq!(writes[0].2, "current-source JSON");
    assert_eq!(
        store
            .files
            .borrow()
            .get(&PathBuf::from("/cache/manifest.json")),
        Some(&"{\"old\":true}\n".to_string())
    );
}

#[test]
fn json_file_store_atomic_pretty_write_propagates_rename_failure() {
    let store = FakeFileIoStore {
        atomic_failure: Some(FakeAtomicFailure::Rename),
        ..FakeFileIoStore::default()
    };
    let path = PathBuf::from("/cache/manifest.json");
    store.insert_file(path.clone(), "{\"old\":true}\n");

    let err = store
        .write_pretty_json_atomically_with_newline(
            &path,
            &json!({"state": "ready"}),
            "current-source JSON",
        )
        .unwrap_err();

    assert_eq!(
        err,
        "Failed to publish current-source JSON fake.tmp -> /cache/manifest.json: permission denied"
    );
    let writes = store.atomic_writes.borrow();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].0, path);
    assert_eq!(writes[0].1, "{\n  \"state\": \"ready\"\n}\n");
    assert_eq!(writes[0].2, "current-source JSON");
    assert_eq!(
        store
            .files
            .borrow()
            .get(&PathBuf::from("/cache/manifest.json")),
        Some(&"{\"old\":true}\n".to_string())
    );
}

#[test]
fn filesystem_file_io_store_drives_json_atomic_write() {
    let root = unique_temp_dir("atomic");
    let path = root.join("nested/manifest.json");
    let store = FilesystemFileIoStore;

    store
        .write_pretty_json_atomically_with_newline(
            &path,
            &json!({"state": "ready"}),
            "current-source JSON",
        )
        .unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\n  \"state\": \"ready\"\n}\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn filesystem_file_io_store_json_atomic_rename_failure_removes_temp_file() {
    let root = unique_temp_dir("atomic-rename-failure");
    let path = root.join("manifest.json");
    fs::create_dir_all(&path).unwrap();
    let store = FilesystemFileIoStore;

    let err = store
        .write_pretty_json_atomically_with_newline(
            &path,
            &json!({"state": "ready"}),
            "current-source JSON",
        )
        .unwrap_err();

    assert!(err.contains("Failed to publish current-source JSON"));
    assert!(path.is_dir());
    let leftovers: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("manifest.json.tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "leftover temp files: {leftovers:?}");
    let _ = fs::remove_dir_all(root);
}
