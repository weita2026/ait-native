use super::*;
use ait_core::json_support::json;
use std::cell::RefCell;
use std::fs;
use tempfile::TempDir;

#[derive(Default)]
struct FakeSnapshotKindStore {
    kind: Option<String>,
    set_calls: RefCell<Vec<(String, String)>>,
}

impl SnapshotStore for FakeSnapshotKindStore {
    fn snapshot_exists(&self, _snapshot_id: &str) -> Result<bool, String> {
        Ok(true)
    }

    fn snapshot_parent_link(
        &self,
        _snapshot_id: &str,
    ) -> Result<Option<ait_core::snapshot_store::SnapshotParentLink>, String> {
        Ok(None)
    }

    fn snapshot_by_id(
        &self,
        _snapshot_id: &str,
    ) -> Result<Option<ait_core::snapshot_store::SnapshotRecord>, String> {
        Ok(None)
    }

    fn list_line_snapshots(&self) -> Result<Vec<ait_core::snapshot_store::SnapshotRecord>, String> {
        Ok(Vec::new())
    }

    fn snapshot_total_bytes(&self, _snapshot_id: &str) -> Result<Option<i64>, String> {
        Ok(None)
    }

    fn snapshot_root_tree_pack_id(&self, _snapshot_id: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn snapshot_kind(&self, _snapshot_id: &str) -> Result<Option<String>, String> {
        Ok(self.kind.clone())
    }

    fn snapshot_chain(&self, snapshot_id: &str) -> Result<Vec<String>, String> {
        Ok(vec![snapshot_id.to_string()])
    }

    fn set_snapshot_kind(&self, snapshot_id: &str, snapshot_kind: &str) -> Result<usize, String> {
        self.set_calls
            .borrow_mut()
            .push((snapshot_id.to_string(), snapshot_kind.to_string()));
        Ok(1)
    }
}

struct FakeStashStore;

impl StashStore for FakeStashStore {
    fn create_stash(&self, record: NewStashRecord<'_>) -> Result<StashRecord, String> {
        Ok(StashRecord {
            stash_id: record.stash_id.to_string(),
            snapshot_id: record.snapshot_id.to_string(),
            source_line_name: record.source_line_name.to_string(),
            base_snapshot_id: record.base_snapshot_id.map(ToString::to_string),
            message: record.message.map(ToString::to_string),
            workspace_cleared: record.workspace_cleared,
            created_at: record.created_at.to_string(),
            snapshot_created_at: "2026-06-20T00:00:01Z".to_string(),
            snapshot_kind: "stash".to_string(),
            parent_snapshot_id: Some("SNP-BASE".to_string()),
            file_count: 3,
            total_bytes: 42,
        })
    }

    fn list_stashes(&self) -> Result<Vec<StashRecord>, String> {
        Ok(Vec::new())
    }

    fn stash_by_id(&self, _stash_id: &str) -> Result<Option<StashRecord>, String> {
        Ok(None)
    }

    fn drop_stash(&self, _stash_id: &str) -> Result<Option<DroppedStashRecord>, String> {
        Ok(None)
    }
}

fn repo_fixture() -> (TempDir, RepoRuntime) {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    fs::write(
        root.join(".ait/config.json"),
        r#"{
  "repo_name": "fixture-ait",
  "default_line": "main"
}
"#,
    )
    .unwrap();
    let repo = RepoRuntime::discover_from_path(root).unwrap();
    (temp, repo)
}

#[test]
fn create_stash_record_reads_snapshot_kind_from_store() {
    let (_temp, repo) = repo_fixture();
    let snapshot_store = FakeSnapshotKindStore {
        kind: Some("stash".to_string()),
        ..Default::default()
    };
    let stash_store = FakeStashStore;

    let stash = create_stash_record_with_stores(
        &repo,
        &snapshot_store,
        &stash_store,
        "SNP-STASH",
        "main",
        Some("SNP-BASE"),
        Some("stash message"),
        true,
    )
    .expect("create stash record");

    assert_eq!(stash["snapshot_id"], json!("SNP-STASH"));
    assert_eq!(stash["snapshot_kind"], json!("stash"));
    assert_eq!(stash["message"], json!("stash message"));
}

#[test]
fn mark_stash_snapshot_kind_accepts_snapshot_store_trait() {
    let snapshot_store = FakeSnapshotKindStore::default();

    mark_stash_snapshot_kind_with_snapshot_store(&snapshot_store, "SNP-STASH")
        .expect("mark stash snapshot kind through snapshot store");

    assert_eq!(
        snapshot_store.set_calls.borrow().as_slice(),
        &[("SNP-STASH".to_string(), "stash".to_string())]
    );
}
