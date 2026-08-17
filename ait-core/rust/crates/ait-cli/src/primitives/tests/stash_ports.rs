use super::*;

#[derive(Default)]
struct FakeStashStore {
    stashes: RefCell<BTreeMap<String, StashRecord>>,
}

impl FakeStashStore {
    fn insert(&self, stash: StashRecord) {
        self.stashes
            .borrow_mut()
            .insert(stash.stash_id.clone(), stash);
    }
}

impl StashStore for FakeStashStore {
    fn create_stash(&self, record: NewStashRecord<'_>) -> StashStoreResult<StashRecord> {
        let stash = StashRecord {
            stash_id: record.stash_id.to_string(),
            snapshot_id: record.snapshot_id.to_string(),
            source_line_name: record.source_line_name.to_string(),
            base_snapshot_id: record.base_snapshot_id.map(ToString::to_string),
            message: record.message.map(ToString::to_string),
            workspace_cleared: record.workspace_cleared,
            created_at: record.created_at.to_string(),
            snapshot_created_at: "2026-07-04T01:59:00Z".to_string(),
            snapshot_kind: "stash".to_string(),
            parent_snapshot_id: record.base_snapshot_id.map(ToString::to_string),
            file_count: 3,
            total_bytes: 512,
        };
        self.insert(stash.clone());
        Ok(stash)
    }

    fn list_stashes(&self) -> StashStoreResult<Vec<StashRecord>> {
        Ok(self.stashes.borrow().values().cloned().collect())
    }

    fn stash_by_id(&self, stash_id: &str) -> StashStoreResult<Option<StashRecord>> {
        Ok(self.stashes.borrow().get(stash_id).cloned())
    }

    fn drop_stash(&self, stash_id: &str) -> StashStoreResult<Option<DroppedStashRecord>> {
        Ok(self
            .stashes
            .borrow_mut()
            .remove(stash_id)
            .map(|stash| DroppedStashRecord {
                stash,
                snapshot_deleted: true,
            }))
    }
}

#[test]
fn stash_helpers_accept_stash_store_trait() {
    let store = FakeStashStore::default();
    store
        .create_stash(NewStashRecord {
            stash_id: "STH-0001",
            snapshot_id: "SNP-STASH",
            source_line_name: "feature/demo",
            base_snapshot_id: Some("SNP-BASE"),
            message: Some("park demo changes"),
            workspace_cleared: true,
            created_at: "2026-07-04T02:00:00Z",
        })
        .expect("seed fake stash");

    let listed = stash_list_with_stash_store(&store).expect("list stashes through store");
    assert_eq!(
        listed,
        json!([{
            "stash_id": "STH-0001",
            "snapshot_id": "SNP-STASH",
            "source_line_name": "feature/demo",
            "base_snapshot_id": "SNP-BASE",
            "message": "park demo changes",
            "workspace_cleared": true,
            "created_at": "2026-07-04T02:00:00Z",
            "snapshot_created_at": "2026-07-04T01:59:00Z",
            "snapshot_kind": "stash",
            "parent_snapshot_id": "SNP-BASE",
            "file_count": 3,
            "total_bytes": 512
        }])
    );

    let shown = stash_show_with_stash_store(&store, "STH-0001").expect("show stash through store");
    assert_eq!(shown["stash_id"], json!("STH-0001"));
    assert_eq!(shown["snapshot_id"], json!("SNP-STASH"));
    assert_eq!(shown["workspace_cleared"], json!(true));

    let missing =
        stash_show_with_stash_store(&store, "STH-MISSING").expect_err("missing stash should fail");
    assert_eq!(missing, "Unknown stash: STH-MISSING");

    let dropped =
        drop_stash_record_with_stash_store(&store, "STH-0001").expect("drop stash through store");
    assert_eq!(dropped["stash_id"], json!("STH-0001"));
    assert_eq!(dropped["dropped"], json!(true));
    assert_eq!(dropped["snapshot_deleted"], json!(true));
    assert_eq!(
        drop_stash_record_with_stash_store(&store, "STH-0001")
            .expect_err("dropped stash should now be missing"),
        "Unknown stash: STH-0001"
    );
}

#[test]
fn stash_source_line_guard_rejects_cross_line_restore_for_every_operation() {
    guard_stash_source_line("STH-0001", "feature/source", "feature/source", "apply")
        .expect("same-Line stash restore should remain valid");

    for operation in ["apply", "pop"] {
        let error =
            guard_stash_source_line("STH-0001", "feature/source", "feature/current", operation)
                .expect_err("cross-Line stash restore must fail closed");
        assert!(
            error.contains(&format!("Cannot {operation} stash STH-0001")),
            "{error}"
        );
        assert!(error.contains("saved from Line feature/source"), "{error}");
        assert!(error.contains("current Line is feature/current"), "{error}");
        assert!(error.contains("--force only overwrites"), "{error}");
        assert!(error.contains("cannot bypass this Line check"), "{error}");
    }
}
