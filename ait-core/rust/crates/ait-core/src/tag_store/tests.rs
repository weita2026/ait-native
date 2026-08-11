use super::{
    create_tag_with_store, delete_tag_with_store, list_tags_with_store, new_tag_record,
    tag_by_name_with_store, FilesystemTagStore, TagStore,
};
use crate::ref_names::encode_ref_name;
use std::cell::RefCell;
use std::fs;
use tempfile::TempDir;

#[derive(Default)]
struct FakeTagStore {
    records: RefCell<Vec<super::TagRecord>>,
}

impl TagStore for FakeTagStore {
    fn create_tag(
        &self,
        record: &super::TagRecord,
        force: bool,
    ) -> super::TagStoreResult<super::TagRecord> {
        let mut records = self.records.borrow_mut();
        if let Some(existing) = records.iter_mut().find(|row| row.name == record.name) {
            if !force {
                return Err("exists".to_string());
            }
            *existing = record.clone();
        } else {
            records.push(record.clone());
        }
        Ok(record.clone())
    }

    fn list_tags(&self) -> super::TagStoreResult<Vec<super::TagRecord>> {
        Ok(self.records.borrow().clone())
    }

    fn tag_by_name(&self, name: &str) -> super::TagStoreResult<Option<super::TagRecord>> {
        Ok(self
            .records
            .borrow()
            .iter()
            .find(|row| row.name == name)
            .cloned())
    }

    fn delete_tag(&self, name: &str) -> super::TagStoreResult<Option<super::TagRecord>> {
        let mut records = self.records.borrow_mut();
        let Some(index) = records.iter().position(|row| row.name == name) else {
            return Ok(None);
        };
        Ok(Some(records.remove(index)))
    }
}

fn tag_store_fixture() -> (TempDir, FilesystemTagStore) {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(".ait")).unwrap();
    let store = FilesystemTagStore::new(temp.path().to_string_lossy().as_ref()).unwrap();
    (temp, store)
}

#[test]
fn tag_store_helper_accepts_trait_object() {
    let store = FakeTagStore::default();
    let tag_store: &dyn TagStore = &store;
    let record = new_tag_record(
        "stable/baseline",
        "SNP-123",
        "parser rewrite baseline",
        "2026-07-05T00:00:00Z",
    )
    .unwrap();

    create_tag_with_store(tag_store, &record, false).unwrap();
    assert_eq!(
        tag_by_name_with_store(tag_store, "stable/baseline")
            .unwrap()
            .unwrap()
            .snapshot_id,
        "SNP-123"
    );
    assert_eq!(list_tags_with_store(tag_store).unwrap().len(), 1);
    assert_eq!(
        delete_tag_with_store(tag_store, "stable/baseline")
            .unwrap()
            .unwrap()
            .name,
        "stable/baseline"
    );
    assert_eq!(
        tag_by_name_with_store(tag_store, "stable/baseline").unwrap(),
        None
    );
}

#[test]
fn filesystem_tag_store_writes_json_record_with_encoded_name() {
    let (temp, store) = tag_store_fixture();
    let record = new_tag_record(
        "stable/refactor baseline",
        "SNP-ABC",
        "known good parser state",
        "2026-07-05T00:00:00Z",
    )
    .unwrap();

    let persisted = store.create_tag(&record, false).unwrap();

    assert_eq!(persisted, record);
    let path = temp.path().join(".ait/refs/tags").join(format!(
        "{}.json",
        encode_ref_name("stable/refactor baseline")
    ));
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains("\"snapshot_id\": \"SNP-ABC\""));
    assert!(text.contains("\"message\": \"known good parser state\""));
}

#[test]
fn filesystem_tag_store_lists_shows_force_replaces_and_deletes() {
    let (_temp, store) = tag_store_fixture();
    let first = new_tag_record(
        "stable/parser",
        "SNP-1",
        "first baseline",
        "2026-07-05T00:00:00Z",
    )
    .unwrap();
    let second = new_tag_record(
        "stable/parser",
        "SNP-2",
        "second baseline",
        "2026-07-05T00:01:00Z",
    )
    .unwrap();
    let other = new_tag_record(
        "stable/api",
        "SNP-3",
        "api baseline",
        "2026-07-05T00:02:00Z",
    )
    .unwrap();

    store.create_tag(&first, false).unwrap();
    store.create_tag(&other, false).unwrap();
    let duplicate = store.create_tag(&second, false).unwrap_err();
    assert!(duplicate.contains("already exists"));

    store.create_tag(&second, true).unwrap();
    assert_eq!(
        store
            .tag_by_name("stable/parser")
            .unwrap()
            .unwrap()
            .snapshot_id,
        "SNP-2"
    );
    assert_eq!(
        store
            .list_tags()
            .unwrap()
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>(),
        vec!["stable/api".to_string(), "stable/parser".to_string()]
    );

    assert_eq!(
        store
            .delete_tag("stable/parser")
            .unwrap()
            .unwrap()
            .snapshot_id,
        "SNP-2"
    );
    assert_eq!(store.tag_by_name("stable/parser").unwrap(), None);
    assert_eq!(store.delete_tag("stable/parser").unwrap(), None);
}

#[test]
fn tag_record_rejects_empty_or_multiline_message() {
    assert!(
        new_tag_record("stable/parser", "SNP-1", "", "2026-07-05T00:00:00Z")
            .unwrap_err()
            .contains("message must not be empty")
    );
    assert!(new_tag_record(
        "stable/parser",
        "SNP-1",
        "line one\nline two",
        "2026-07-05T00:00:00Z",
    )
    .unwrap_err()
    .contains("message must be a single line"));
}
