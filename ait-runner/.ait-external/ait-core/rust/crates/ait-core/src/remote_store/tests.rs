use super::{
    add_remote_with_remote_store, list_remotes_with_remote_store, remote_by_name_with_remote_store,
    remote_exists_with_remote_store, ConfigRemoteStore, RemoteAddRecord, RemoteRecord, RemoteStore,
    RemoteStoreResult,
};
use crate::json_support::{JsonCodec, JsonValue};
use std::cell::RefCell;
use std::fs;
use tempfile::TempDir;

#[derive(Default)]
struct FakeRemoteStore {
    added: RefCell<Vec<RemoteAddRecord>>,
}

impl RemoteStore for FakeRemoteStore {
    fn remote_exists(&self, name: &str) -> RemoteStoreResult<bool> {
        Ok(name == "origin")
    }

    fn list_remotes(&self) -> RemoteStoreResult<Vec<RemoteRecord>> {
        Ok(vec![fixture_remote_record("origin", 1)])
    }

    fn remote_by_name(&self, name: &str) -> RemoteStoreResult<Option<RemoteRecord>> {
        Ok((name == "origin").then(|| fixture_remote_record("origin", 1)))
    }

    fn add_remote(&self, request: &RemoteAddRecord) -> RemoteStoreResult<()> {
        self.added.borrow_mut().push(request.clone());
        Ok(())
    }
}

#[test]
fn remote_store_helpers_accept_trait_object() {
    let store = FakeRemoteStore::default();
    let remote_store: &dyn RemoteStore = &store;

    assert!(remote_exists_with_remote_store(remote_store, "origin").unwrap());
    assert_eq!(
        list_remotes_with_remote_store(remote_store).unwrap(),
        vec![fixture_remote_record("origin", 1)]
    );
    assert_eq!(
        remote_by_name_with_remote_store(remote_store, "origin").unwrap(),
        Some(fixture_remote_record("origin", 1))
    );
}

#[test]
fn add_remote_helper_accepts_trait_object() {
    let store = FakeRemoteStore::default();
    let remote_store: &dyn RemoteStore = &store;
    let request = RemoteAddRecord {
        name: "origin".to_string(),
        url: "https://example.test/ait".to_string(),
        repo_name: Some("ait-core".to_string()),
        make_default: true,
        created_at: "2026-07-04T04:00:00Z".to_string(),
    };

    add_remote_with_remote_store(remote_store, &request).expect("add remote through store");
    assert_eq!(store.added.borrow().as_slice(), &[request]);
}

#[test]
fn config_remote_store_adds_lists_and_reads_remotes() {
    let temp = TempDir::new().expect("tempdir");
    let config_path = temp.path().join(".ait/config.json");
    fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    fs::write(
        &config_path,
        r#"{
  "default_remote": "upstream",
  "repo_id": "REPO-1",
  "remotes": {
    "upstream": {
      "remote_id": 7,
      "url": "http://example.test/upstream",
      "repo_name": "upstream-repo",
      "created_at": "2026-07-04T00:00:00Z"
    }
  }
}
"#,
    )
    .unwrap();

    let store = ConfigRemoteStore::new(&config_path).unwrap();
    assert!(store.remote_exists("upstream").unwrap());
    assert!(!store.remote_exists("origin").unwrap());

    store
        .add_remote(&RemoteAddRecord {
            name: "origin".to_string(),
            url: "http://example.test/origin".to_string(),
            repo_name: None,
            make_default: true,
            created_at: "2026-07-04T01:00:00Z".to_string(),
        })
        .unwrap();

    let rows = store.list_remotes().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "origin");
    assert_eq!(rows[0].repo_name, None);
    assert_eq!(rows[0].is_default_push, 1);
    assert_eq!(rows[0].is_default_pull, 1);
    assert_eq!(rows[1].name, "upstream");
    assert_eq!(rows[1].is_default_push, 0);
    assert_eq!(rows[1].is_default_pull, 0);

    let origin = store.remote_by_name("origin").unwrap().unwrap();
    assert_eq!(origin.url, "http://example.test/origin");
    assert_eq!(origin.created_at, "2026-07-04T01:00:00Z");

    let config_text = fs::read_to_string(&config_path).unwrap();
    let config = JsonCodec::parse_value(&config_text, "test config").unwrap();
    assert_eq!(config["default_remote"], JsonValue::String("origin".into()));
    assert_eq!(
        config["remotes"]["origin"]["url"],
        JsonValue::String("http://example.test/origin".into())
    );
}

fn fixture_remote_record(name: &str, remote_id: i64) -> RemoteRecord {
    RemoteRecord {
        remote_id,
        name: name.to_string(),
        url: format!("http://example.test/{name}"),
        repo_name: Some(format!("{name}-repo")),
        is_default_push: 0,
        is_default_pull: 0,
        created_at: "2026-07-04T00:00:00Z".to_string(),
    }
}
