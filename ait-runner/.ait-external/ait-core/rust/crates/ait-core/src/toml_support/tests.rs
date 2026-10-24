use super::*;
use crate::file_io::{FileIoResult, FileIoStore, FilesystemFileIoStore};
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
    read_failure: Option<String>,
    write_failure: Option<String>,
    atomic_failure: Option<String>,
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
        if let Some(message) = self.read_failure.as_ref() {
            return Err(message.clone().into());
        }
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
        if let Some(message) = self.write_failure.as_ref() {
            return Err(message.clone().into());
        }
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), text.to_string());
        Ok(())
    }

    fn write_string_atomically(
        &self,
        path: &Path,
        text: &str,
        publish_label: &str,
    ) -> FileIoResult<()> {
        self.atomic_writes.borrow_mut().push((
            path.to_path_buf(),
            text.to_string(),
            publish_label.to_string(),
        ));
        if let Some(message) = self.atomic_failure.as_ref() {
            return Err(message.clone().into());
        }
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), text.to_string());
        Ok(())
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
    std::env::temp_dir().join(format!("ait-toml-support-{name}-{nonce}"))
}

fn table(text: &str) -> TomlTable {
    TomlCodec::parse_table(text, "fixture").unwrap()
}

#[test]
fn toml_codec_encodes_pretty_compact_and_trailing_newline() {
    let value = TomlCodec::parse_value("name = \"ait\"\ncount = 3\n", "config").unwrap();

    assert_eq!(
        TomlCodec::encode_value(&value, TomlEncodeOptions::compact()).unwrap(),
        "count = 3\nname = \"ait\"\n"
    );
    assert_eq!(
        TomlCodec::encode_value(&value, TomlEncodeOptions::pretty()).unwrap(),
        "count = 3\nname = \"ait\"\n"
    );
    assert_eq!(
        TomlCodec::encode_value(&value, TomlEncodeOptions::pretty().with_trailing_newline())
            .unwrap(),
        "count = 3\nname = \"ait\"\n"
    );
}

#[test]
fn toml_codec_parse_errors_include_stable_labels() {
    let err = TomlCodec::parse_value("=", "external manifest").unwrap_err();
    assert!(err
        .to_string()
        .starts_with("Invalid external manifest TOML: "));

    assert_eq!(
        TomlCodec::parse_table("name = \"ait\"\n", "external manifest")
            .unwrap()
            .get("name")
            .and_then(toml::Value::as_str),
        Some("ait")
    );
}

#[test]
fn toml_fields_cover_required_optional_and_string_list_values() {
    let payload = table(
        r#"
name = "ait"
enabled = true
count = 42
path = "runtime/report.toml"
items = ["a", "b"]

[repo]
name = "ait-core"
"#,
    );

    assert_eq!(required_text_field(&payload, "name").unwrap(), "ait");
    assert_eq!(optional_text_field(&payload, "missing").unwrap(), None);
    assert!(required_bool_field(&payload, "enabled").unwrap());
    assert_eq!(required_integer_field(&payload, "count").unwrap(), 42);
    assert_eq!(
        required_path_field(&payload, "path").unwrap(),
        PathBuf::from("runtime/report.toml")
    );
    assert_eq!(
        required_string_list_field(&payload, "items").unwrap(),
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(
        required_table_field(&payload, "repo").unwrap()["name"].as_str(),
        Some("ait-core")
    );

    assert_eq!(
        required_text_field(&payload, "missing")
            .unwrap_err()
            .to_string(),
        "Missing required TOML field `missing`."
    );
    assert_eq!(
        optional_integer_field(&payload, "name")
            .unwrap_err()
            .to_string(),
        "Field `name` must be a TOML integer, got string."
    );
}

#[test]
fn toml_file_store_trait_object_reads_and_writes_through_shared_methods() {
    let store = FakeFileIoStore::with_home(PathBuf::from("/home/ait"));
    let read_path = PathBuf::from("/home/ait/.ait/config.toml");
    let write_path = PathBuf::from("/home/ait/.ait/worktree.toml");
    let atomic_path = PathBuf::from("/cache/manifest.toml");
    store.insert_file(read_path.clone(), "ok = true\n");
    let store_port: &dyn TomlFileStore = &store;

    let value = store_port
        .read_toml_value(
            "~/.ait/config.toml",
            "repo config",
            TomlReadOptions::required(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(value["ok"].as_bool(), Some(true));

    store_port
        .write_toml_value(
            "~/.ait/worktree.toml",
            &value,
            "worktree config",
            TomlWriteOptions::direct(),
        )
        .unwrap();
    store_port
        .write_toml_value_at_path(
            &atomic_path,
            &value,
            "external manifest",
            TomlWriteOptions::atomic("external manifest TOML").with_trailing_newline(),
        )
        .unwrap();

    assert_eq!(*store.reads.borrow(), vec![read_path]);
    assert_eq!(
        store.writes.borrow().as_slice(),
        &[(write_path, "ok = true\n".to_string())]
    );
    assert_eq!(
        store.atomic_writes.borrow().as_slice(),
        &[(
            atomic_path,
            "ok = true\n".to_string(),
            "external manifest TOML".to_string()
        )]
    );
}

#[test]
fn toml_file_store_optional_missing_path_returns_none_without_reading() {
    let store = FakeFileIoStore::default();

    let value = store
        .read_toml_value(
            "/missing/config.toml",
            "repo config",
            TomlReadOptions::optional(),
        )
        .unwrap();

    assert_eq!(value, None);
    assert!(store.reads.borrow().is_empty());
}

#[test]
fn toml_file_store_required_missing_path_reports_context() {
    let store = FakeFileIoStore::default();

    let err = store
        .read_toml_value(
            "/missing/config.toml",
            "repo config",
            TomlReadOptions::required(),
        )
        .unwrap_err();

    assert_eq!(
        err.to_string(),
        "Missing repo config TOML /missing/config.toml."
    );
    assert!(store.reads.borrow().is_empty());
}

#[test]
fn toml_file_store_propagates_read_write_and_atomic_errors_with_context() {
    let read_store = FakeFileIoStore {
        read_failure: Some("disk read failed".to_string()),
        ..FakeFileIoStore::default()
    };
    read_store.insert_file(PathBuf::from("/tmp/config.toml"), "ok = true\n");
    let read_err = read_store
        .read_toml_value(
            "/tmp/config.toml",
            "repo config",
            TomlReadOptions::required(),
        )
        .unwrap_err();
    assert_eq!(
        read_err.to_string(),
        "Failed to read repo config TOML /tmp/config.toml: disk read failed"
    );

    let write_store = FakeFileIoStore {
        write_failure: Some("disk full".to_string()),
        ..FakeFileIoStore::default()
    };
    let value = TomlCodec::parse_value("ok = true\n", "fixture").unwrap();
    let write_err = write_store
        .write_toml_value(
            "/tmp/config.toml",
            &value,
            "repo config",
            TomlWriteOptions::direct(),
        )
        .unwrap_err();
    assert_eq!(
        write_err.to_string(),
        "Failed to write repo config TOML /tmp/config.toml: disk full"
    );

    let atomic_store = FakeFileIoStore {
        atomic_failure: Some("rename failed".to_string()),
        ..FakeFileIoStore::default()
    };
    atomic_store.insert_file(PathBuf::from("/tmp/config.toml"), "old = true\n");
    let atomic_err = atomic_store
        .write_toml_value(
            "/tmp/config.toml",
            &value,
            "repo config",
            TomlWriteOptions::atomic("repo config TOML"),
        )
        .unwrap_err();
    assert_eq!(
        atomic_err.to_string(),
        "Failed to write repo config TOML /tmp/config.toml: rename failed"
    );
    assert_eq!(
        atomic_store
            .files
            .borrow()
            .get(&PathBuf::from("/tmp/config.toml")),
        Some(&"old = true\n".to_string())
    );
}

#[test]
fn filesystem_file_io_store_drives_toml_atomic_write() {
    let root = unique_temp_dir("atomic");
    let path = root.join("nested/manifest.toml");
    let store = FilesystemFileIoStore;
    let value = TomlCodec::parse_value("state = \"ready\"\n", "fixture").unwrap();

    store
        .write_toml_value_at_path(
            &path,
            &value,
            "external manifest",
            TomlWriteOptions::atomic("external manifest TOML").with_trailing_newline(),
        )
        .unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "state = \"ready\"\n");
    let _ = fs::remove_dir_all(root);
}
