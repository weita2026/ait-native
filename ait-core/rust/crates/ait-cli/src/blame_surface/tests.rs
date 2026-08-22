use super::*;
use ait_core::binary_db::BinaryDbCommandScope;
use ait_core::content_binary_db::{
    snapshot_id_from_hash48, BinaryDbContentWriteCoordinator, BinaryDbSnapshotWriteInput,
};
use ait_core::line_store::LineStore;
use ait_core::local_snapshot::LocalSnapshotWriteStore;
use ait_core::snapshot_store::{SnapshotParentLink, SnapshotRecord, SnapshotStoreResult};
use std::cell::Cell;
use std::fs;
use tempfile::TempDir;

struct MetadataOnlySnapshotStore {
    snapshot: SnapshotRecord,
    metadata_reads: Cell<usize>,
}

struct MemoryBlobStore {
    bytes_by_blob_id: BTreeMap<String, Vec<u8>>,
}

impl LocalSnapshotBlobReadStore for MemoryBlobStore {
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Vec<u8>, String> {
        self.bytes_by_blob_id
            .get(blob_id)
            .cloned()
            .ok_or_else(|| format!("Unknown test blob: {blob_id}"))
    }
}

struct CountingReversePathStore {
    blob_ids: Vec<Option<String>>,
    visits: Cell<usize>,
}

#[test]
fn patchset_change_identity_preserves_exact_task_scope() {
    let exact = json!({
        "patchset_id": "RT-1/C-01/P-02",
        "change_id": "C-01",
        "change_ref": "RT-1/C-01",
    });
    assert_eq!(
        resolve_patchset_change_identity(&exact, "RT-1/C-01/P-02")
            .expect("exact Patchset identity"),
        (
            "C-01".to_string(),
            "RT-1/C-01".to_string(),
            Some("RT-1".to_string()),
        )
    );

    let derived = json!({
        "patchset_id": "RT-2/C-03/P-04",
        "change_id": "C-03",
    });
    assert_eq!(
        resolve_patchset_change_identity(&derived, "RT-2/C-03/P-04")
            .expect("current-format Patchset identity"),
        (
            "C-03".to_string(),
            "RT-2/C-03".to_string(),
            Some("RT-2".to_string()),
        )
    );
}

#[test]
fn patchset_change_identity_rejects_conflicts_and_keeps_legacy_non_short_ids() {
    let conflict = json!({
        "patchset_id": "RT-1/C-01/P-01",
        "change_id": "C-01",
        "change_ref": "RT-2/C-01",
    });
    let error = resolve_patchset_change_identity(&conflict, "RT-1/C-01/P-01")
        .expect_err("conflicting Patchset scope must fail closed");
    assert!(error.contains("conflicting owning Change identity"));

    let legacy = json!({
        "patchset_id": "RP-1",
        "change_id": "RC-1",
    });
    assert_eq!(
        resolve_patchset_change_identity(&legacy, "RP-1").expect("legacy identity"),
        ("RC-1".to_string(), "RC-1".to_string(), None)
    );
}

#[test]
fn snapshot_overlay_change_identity_never_emits_an_unscoped_short_lookup() {
    assert_eq!(
        overlay_change_reference(&json!({"change_id":"C-01"}).as_object().unwrap().clone())
            .expect("unscoped legacy provenance"),
        None
    );
    assert_eq!(
        overlay_change_reference(
            &json!({"task_id":"RT-1","change_id":"C-01"})
                .as_object()
                .unwrap()
                .clone()
        )
        .expect("scoped provenance"),
        Some("RT-1/C-01".to_string())
    );
}

#[test]
fn current_line_header_prefers_the_selected_line_over_snapshot_authoring_line() {
    let payload = json!({
        "target": {
            "kind": "current_line",
            "line_name": "main",
        },
        "line_name": "feature/task-authoring-line",
        "resolved_snapshot_id": "SNP-LANDED",
    });
    let target = payload["target"].as_object().unwrap();
    assert_eq!(current_line_target_name(&payload, target), "main");

    let legacy_payload = json!({"line_name": "legacy-line"});
    assert_eq!(
        current_line_target_name(&legacy_payload, &JsonMap::new()),
        "legacy-line"
    );
}

#[test]
fn blame_request_validation_rejects_zero_in_every_line_selector() {
    for request in [
        BlameRequest {
            path: "tracked.txt".to_string(),
            line: Some(0),
            ..BlameRequest::default()
        },
        BlameRequest {
            path: "tracked.txt".to_string(),
            start_line: Some(0),
            end_line: Some(1),
            ..BlameRequest::default()
        },
        BlameRequest {
            path: "tracked.txt".to_string(),
            start_line: Some(1),
            end_line: Some(0),
            ..BlameRequest::default()
        },
    ] {
        assert_eq!(
            validate_request(&request).unwrap_err(),
            "Line selections are 1-based and must be positive."
        );
    }
}

impl ReverseSnapshotPathBlobStore for CountingReversePathStore {
    fn visit_reverse_path_blobs(
        &self,
        snapshot_ids: &[String],
        _path: &str,
        visitor: &mut dyn FnMut(usize, Option<String>) -> Result<bool, String>,
    ) -> Result<(), String> {
        assert_eq!(snapshot_ids.len(), self.blob_ids.len());
        for snapshot_index in (0..snapshot_ids.len()).rev() {
            self.visits.set(self.visits.get() + 1);
            if !visitor(snapshot_index, self.blob_ids[snapshot_index].clone())? {
                break;
            }
        }
        Ok(())
    }
}

fn selected_line_test_lineage() -> SnapshotBlameLineage<MemoryBlobStore> {
    let bytes_by_blob_id = BTreeMap::from([
        ("BLB-0".to_string(), b"old\nstable\n".to_vec()),
        ("BLB-1".to_string(), b"old\nstable\n".to_vec()),
        ("BLB-2".to_string(), b"new\nstable\n".to_vec()),
    ]);
    SnapshotBlameLineage {
        blob_store: MemoryBlobStore {
            bytes_by_blob_id: bytes_by_blob_id.clone(),
        },
        chain: vec!["S0".to_string(), "S1".to_string(), "S2".to_string()],
        path_timeline: CompactPathBlobTimeline::from_rows(
            3,
            &[SnapshotPathBlobRow {
                snapshot_index: 2,
                blob_id: "BLB-2".to_string(),
            }],
        )
        .expect("target-only path timeline"),
        blob_bytes_by_id: BTreeMap::from([(
            "BLB-2".to_string(),
            bytes_by_blob_id["BLB-2"].clone(),
        )]),
        blob_lines_cache: BTreeMap::from([(
            "BLB-2".to_string(),
            vec!["new\n".to_string(), "stable\n".to_string()],
        )]),
        target_lines: vec!["new\n".to_string(), "stable\n".to_string()],
    }
}

impl SnapshotStore for MetadataOnlySnapshotStore {
    fn snapshot_exists(&self, _snapshot_id: &str) -> SnapshotStoreResult<bool> {
        panic!("blame metadata loading must use snapshot_by_id")
    }

    fn snapshot_parent_link(
        &self,
        _snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<SnapshotParentLink>> {
        panic!("blame metadata loading must use snapshot_by_id")
    }

    fn snapshot_by_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<SnapshotRecord>> {
        self.metadata_reads.set(self.metadata_reads.get() + 1);
        Ok((snapshot_id == self.snapshot.snapshot_id).then(|| self.snapshot.clone()))
    }

    fn list_line_snapshots(&self) -> SnapshotStoreResult<Vec<SnapshotRecord>> {
        panic!("blame metadata loading must use snapshot_by_id")
    }

    fn snapshot_total_bytes(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<i64>> {
        panic!("blame metadata loading must use snapshot_by_id")
    }

    fn snapshot_root_tree_pack_id(
        &self,
        _snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<String>> {
        panic!("blame metadata loading must use snapshot_by_id")
    }

    fn snapshot_kind(&self, _snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        panic!("blame metadata loading must use snapshot_by_id")
    }

    fn snapshot_chain(&self, _snapshot_id: &str) -> SnapshotStoreResult<Vec<String>> {
        panic!("blame metadata loading must use snapshot_by_id")
    }

    fn set_snapshot_kind(
        &self,
        _snapshot_id: &str,
        _snapshot_kind: &str,
    ) -> SnapshotStoreResult<usize> {
        panic!("blame metadata loading must use snapshot_by_id")
    }
}

#[test]
fn blame_snapshot_metadata_loading_uses_the_narrow_snapshot_record_boundary() {
    let store = MetadataOnlySnapshotStore {
        snapshot: SnapshotRecord {
            snapshot_id: "SNP-METADATA".to_string(),
            parent_snapshot_ids: vec!["SNP-PARENT".to_string()],
            primary_parent_snapshot_id: Some("SNP-PARENT".to_string()),
            parent_snapshot_id: Some("SNP-PARENT".to_string()),
            root_tree_pack_id: Some("TPK-METADATA".to_string()),
            root_entry_ordinal: Some(7),
            manifest_hash: "manifest-hash".to_string(),
            message: Some("metadata only".to_string()),
            line_name: "main".to_string(),
            snapshot_kind: "line".to_string(),
            file_count: 50_000,
            total_bytes: 900_000_000,
            created_at: "2026-07-15T00:00:00Z".to_string(),
        },
        metadata_reads: Cell::new(0),
    };

    let payload = snapshot_metadata_payload_with_store(&store, "SNP-METADATA")
        .expect("metadata-only snapshot payload");

    assert_eq!(store.metadata_reads.get(), 1);
    assert_eq!(payload["snapshot_id"], "SNP-METADATA");
    assert_eq!(payload["parent_snapshot_id"], "SNP-PARENT");
    assert_eq!(payload["line_name"], "main");
    assert_eq!(payload["message"], "metadata only");
    assert_eq!(payload["file_count"], 50_000);
    assert!(payload.get("files").is_none());
    assert!(payload.get("manifest_path").is_none());
}

#[test]
fn blob_text_lines_reuses_process_local_line_cache() {
    let blob_id = "BLB-test";
    let mut blob_bytes_by_id = BTreeMap::from([(blob_id.to_string(), b"one\ntwo\n".to_vec())]);
    let mut line_cache = BTreeMap::new();

    let first = blob_text_lines(blob_id, &blob_bytes_by_id, &mut line_cache, "test blob")
        .expect("first read should decode blob bytes");
    assert_eq!(first, vec!["one\n".to_string(), "two\n".to_string()]);

    blob_bytes_by_id.clear();
    let second = blob_text_lines(blob_id, &blob_bytes_by_id, &mut line_cache, "test blob")
        .expect("second read should use the in-memory line cache");
    assert_eq!(second, first);
}

#[test]
fn blob_text_lines_reports_missing_batch_payload() {
    let mut line_cache = BTreeMap::new();

    let err = blob_text_lines(
        "BLB-missing",
        &BTreeMap::new(),
        &mut line_cache,
        "missing blob",
    )
    .expect_err("missing batch payload should be reported");

    assert!(err.contains("Blob payload missing for `BLB-missing`"));
}

#[test]
fn compact_path_blob_timeline_uses_int_ordinals_for_snapshot_lookup() {
    let rows = vec![
        SnapshotPathBlobRow {
            snapshot_index: 1,
            blob_id: "BLB-a".to_string(),
        },
        SnapshotPathBlobRow {
            snapshot_index: 3,
            blob_id: "BLB-b".to_string(),
        },
        SnapshotPathBlobRow {
            snapshot_index: 4,
            blob_id: "BLB-a".to_string(),
        },
    ];

    let timeline = CompactPathBlobTimeline::from_rows(5, &rows).unwrap();

    assert_eq!(timeline.path_id(), 0);
    assert_eq!(
        timeline.blob_ids(),
        vec!["BLB-a".to_string(), "BLB-b".to_string()]
    );
    assert_eq!(timeline.blob_id_at(0), None);
    assert_eq!(timeline.blob_id_at(1), Some("BLB-a"));
    assert_eq!(timeline.blob_id_at(2), None);
    assert_eq!(timeline.blob_id_at(3), Some("BLB-b"));
    assert_eq!(timeline.blob_id_at(4), Some("BLB-a"));
    assert_eq!(timeline.oldest_existing_snapshot_index(), Some(1));
}

#[test]
fn selected_line_blame_stops_reverse_history_when_owner_is_resolved() {
    let mut lineage = selected_line_test_lineage();
    let tree_store = CountingReversePathStore {
        blob_ids: vec![
            Some("BLB-0".to_string()),
            Some("BLB-1".to_string()),
            Some("BLB-2".to_string()),
        ],
        visits: Cell::new(0),
    };

    let owners = compute_snapshot_selected_line_owners_with_reverse_store(
        &mut lineage,
        &tree_store,
        "S2",
        "tracked.txt",
        1,
        1,
    )
    .expect("selected line owner");

    assert_eq!(owners, vec!["S2".to_string()]);
    assert_eq!(tree_store.visits.get(), 2);
}

#[test]
fn selected_line_blame_walks_to_root_for_unchanged_line() {
    let mut lineage = selected_line_test_lineage();
    let tree_store = CountingReversePathStore {
        blob_ids: vec![
            Some("BLB-0".to_string()),
            Some("BLB-1".to_string()),
            Some("BLB-2".to_string()),
        ],
        visits: Cell::new(0),
    };

    let owners = compute_snapshot_selected_line_owners_with_reverse_store(
        &mut lineage,
        &tree_store,
        "S2",
        "tracked.txt",
        2,
        2,
    )
    .expect("stable line owner");

    assert_eq!(owners, vec!["S0".to_string()]);
    assert_eq!(tree_store.visits.get(), 3);
}

#[test]
fn selected_binary_snapshot_blame_reads_selected_stores_without_retired_backend_fallback() {
    let temp = TempDir::new().unwrap();
    let repo_root = temp.path();
    fs::create_dir_all(repo_root.join(".ait")).unwrap();
    fs::write(
        repo_root.join(".ait/config.json"),
        r#"{"repo_name":"fixture-ait","default_line":"main","snapshot_binary_db_storage":"binary"}"#,
    )
    .unwrap();

    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .lines()
        .create_line("main", None, "2026-07-08T00:00:00Z")
        .expect("create Binary DB line");
    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(repo_root)
        .expect("selected Binary DB snapshot store");

    fs::write(repo_root.join("tracked.txt"), "old line\nstable line\n").unwrap();
    let first = store
        .create_snapshot("fixture-ait", "main", Some("first"), false)
        .expect("create first Binary DB snapshot");
    let first_id = required_string_field(&first, "snapshot_id").expect("first snapshot id");

    fs::write(repo_root.join("tracked.txt"), "new line\nstable line\n").unwrap();
    let second = store
        .create_snapshot("fixture-ait", "main", Some("second"), false)
        .expect("create second Binary DB snapshot");
    let second_id = required_string_field(&second, "snapshot_id").expect("second snapshot id");

    let payload = blame(
        &repo,
        &BlameRequest {
            path: "tracked.txt".to_string(),
            snapshot_id: Some(second_id.clone()),
            ..BlameRequest::default()
        },
    )
    .expect("selected Binary DB blame");
    let lines = payload["lines"].as_array().expect("line rows");

    assert_eq!(
        payload["resolved_snapshot_id"],
        JsonValue::String(second_id.clone())
    );
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["snapshot_id"], JsonValue::String(second_id));
    assert_eq!(lines[0]["start_line"], JsonValue::from(1));
    assert_eq!(lines[1]["snapshot_id"], JsonValue::String(first_id));
    assert_eq!(lines[1]["start_line"], JsonValue::from(2));
}

#[test]
fn merge_snapshot_blame_reports_alternates_and_accepts_explicit_parent() {
    let temp = TempDir::new().unwrap();
    let repo_root = temp.path();
    fs::create_dir_all(repo_root.join(".ait")).unwrap();
    fs::write(
        repo_root.join(".ait/config.json"),
        r#"{"repo_name":"fixture-ait","default_line":"main","snapshot_binary_db_storage":"binary"}"#,
    )
    .unwrap();
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let stores = repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>();
    stores
        .lines()
        .create_line("main", None, "2026-07-19T00:00:00Z")
        .expect("create line");
    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(repo_root)
        .expect("snapshot store");

    fs::write(repo_root.join("tracked.txt"), "root\n").unwrap();
    let root = store
        .create_snapshot("fixture-ait", "main", Some("root"), false)
        .expect("root snapshot");
    let root_id = required_string_field(&root, "snapshot_id").unwrap();

    fs::write(repo_root.join("tracked.txt"), "left\n").unwrap();
    let left = store
        .create_snapshot("fixture-ait", "main", Some("left"), false)
        .expect("left snapshot");
    let left_id = required_string_field(&left, "snapshot_id").unwrap();

    stores
        .lines()
        .set_line_head("main", Some(&root_id), "2026-07-19T00:00:01Z")
        .expect("reset line to root");
    fs::write(repo_root.join("tracked.txt"), "right\n").unwrap();
    let right = store
        .create_snapshot("fixture-ait", "main", Some("right"), false)
        .expect("right snapshot");
    let right_id = required_string_field(&right, "snapshot_id").unwrap();

    let content = stores.content();
    let left_record = content
        .snapshots()
        .snapshot_by_id(&left_id)
        .unwrap()
        .expect("left metadata");
    let left_root = content
        .snapshot_tree_root_locator(&left_id)
        .expect("left root locator");
    let merge_id = snapshot_id_from_hash48(0x0A0B_0C0D_0E0F);
    BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    )
    .record_snapshot(
        BinaryDbCommandScope::ContentWrite,
        &BinaryDbSnapshotWriteInput {
            snapshot_id: merge_id.clone(),
            parent_snapshot_ids: vec![left_id.clone(), right_id.clone()],
            root_tree_pack_id: left_root.root_tree_pack_id,
            root_entry_ordinal: left_root.root_entry_ordinal,
            manifest_hash: "ab".repeat(32),
            message: Some("merge".to_string()),
            line_name: "main".to_string(),
            snapshot_kind: "line".to_string(),
            file_count: left_record.file_count,
            total_bytes: left_record.total_bytes,
            created_at: "2026-07-19T00:00:02Z".to_string(),
        },
    )
    .expect("record merge");

    let primary = blame(
        &repo,
        &BlameRequest {
            path: "tracked.txt".to_string(),
            snapshot_id: Some(merge_id.clone()),
            ..BlameRequest::default()
        },
    )
    .expect("primary-parent blame");
    assert_eq!(primary["lines"][0]["snapshot_id"], left_id);
    assert_eq!(
        primary["parent_selection"]["alternate_parent_snapshot_ids"],
        json!([right_id.clone()])
    );

    let selected = blame(
        &repo,
        &BlameRequest {
            path: "tracked.txt".to_string(),
            snapshot_id: Some(merge_id.clone()),
            via_parent_snapshot_id: Some(right_id.clone()),
            ..BlameRequest::default()
        },
    )
    .expect("alternate-parent blame");
    assert_eq!(selected["lines"][0]["snapshot_id"], merge_id);
    assert_eq!(
        selected["parent_selection"]["selected_parent_snapshot_id"],
        right_id
    );
    assert_eq!(selected["parent_selection"]["mode"], "selected_parent");

    let error = blame(
        &repo,
        &BlameRequest {
            path: "tracked.txt".to_string(),
            snapshot_id: Some(merge_id),
            via_parent_snapshot_id: Some("SNP-NOT-A-PARENT".to_string()),
            ..BlameRequest::default()
        },
    )
    .expect_err("unrelated parent must be rejected");
    assert!(error.contains("does not have selected parent"));
}

#[test]
fn selected_binary_snapshot_blame_resolves_a_deep_path_across_long_ancestry() {
    let temp = TempDir::new().unwrap();
    let repo_root = temp.path();
    fs::create_dir_all(repo_root.join(".ait")).unwrap();
    fs::write(
        repo_root.join(".ait/config.json"),
        r#"{"repo_name":"fixture-deep-blame","default_line":"main","snapshot_binary_db_storage":"binary"}"#,
    )
    .unwrap();

    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .lines()
        .create_line("main", None, "2026-07-16T00:00:00Z")
        .expect("create Binary DB line");
    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(repo_root)
        .expect("selected Binary DB snapshot store");
    let deep_rel_path = "rust/crates/service/src/foundation/workflow/store/change.rs";
    let deep_path = repo_root.join(deep_rel_path);
    fs::create_dir_all(deep_path.parent().expect("deep path parent")).unwrap();
    fs::write(&deep_path, "first owner\nstable line\n").unwrap();
    fs::write(repo_root.join("churn.txt"), "0\n").unwrap();
    let first = store
        .create_snapshot("fixture-deep-blame", "main", Some("first"), false)
        .expect("create first deep-path snapshot");
    let first_id = required_string_field(&first, "snapshot_id").expect("first snapshot id");

    let mut latest_id = first_id.clone();
    for ordinal in 1..=64 {
        fs::write(repo_root.join("churn.txt"), format!("{ordinal}\n")).unwrap();
        let snapshot = store
            .create_snapshot(
                "fixture-deep-blame",
                "main",
                Some(&format!("churn {ordinal}")),
                false,
            )
            .expect("create long ancestry snapshot");
        latest_id = required_string_field(&snapshot, "snapshot_id").expect("latest snapshot id");
    }

    let payload = blame(
        &repo,
        &BlameRequest {
            path: deep_rel_path.to_string(),
            line: Some(1),
            snapshot_id: Some(latest_id.clone()),
            ..BlameRequest::default()
        },
    )
    .expect("deep-path Binary DB blame");
    let lines = payload["lines"].as_array().expect("selected line rows");

    assert_eq!(payload["resolved_snapshot_id"], latest_id);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["snapshot_id"], first_id);
    assert_eq!(lines[0]["start_line"], 1);
    assert_eq!(lines[0]["end_line"], 1);
}

#[test]
fn apply_line_diff_marks_inserted_and_replaced_lines_with_new_owner() {
    let old_lines = vec!["a\n".to_string(), "b\n".to_string(), "c\n".to_string()];
    let new_lines = vec![
        "intro\n".to_string(),
        "a\n".to_string(),
        "B\n".to_string(),
        "c\n".to_string(),
    ];
    let old_owners = vec!["S1".to_string(), "S1".to_string(), "S1".to_string()];

    let owners = apply_line_diff(&old_lines, &new_lines, &old_owners, "S2");

    assert_eq!(
        owners,
        vec![
            "S2".to_string(),
            "S1".to_string(),
            "S2".to_string(),
            "S1".to_string(),
        ]
    );
}

#[test]
fn selected_line_mapper_tracks_equal_lines_and_resolves_changed_lines() {
    let parent_lines = vec![
        "a\n".to_string(),
        "b\n".to_string(),
        "c\n".to_string(),
        "d\n".to_string(),
    ];
    let child_lines = vec![
        "intro\n".to_string(),
        "a\n".to_string(),
        "b\n".to_string(),
        "x\n".to_string(),
        "d\n".to_string(),
    ];
    let tracked = vec![
        SelectedLineTracker {
            output_index: 0,
            current_index: 0,
        },
        SelectedLineTracker {
            output_index: 1,
            current_index: 2,
        },
        SelectedLineTracker {
            output_index: 2,
            current_index: 3,
        },
        SelectedLineTracker {
            output_index: 3,
            current_index: 4,
        },
    ];
    let mut owners = vec![None, None, None, None];

    let mapped =
        map_tracked_lines_to_parent(&parent_lines, &child_lines, &tracked, "S2", &mut owners);

    assert_eq!(
        mapped,
        vec![
            SelectedLineTracker {
                output_index: 1,
                current_index: 1,
            },
            SelectedLineTracker {
                output_index: 3,
                current_index: 3,
            },
        ]
    );
    assert_eq!(owners[0], Some("S2".to_string()));
    assert_eq!(owners[1], None);
    assert_eq!(owners[2], Some("S2".to_string()));
    assert_eq!(owners[3], None);
}

#[test]
fn selected_line_mapper_resolves_multistep_insert_and_replace() {
    let v1_lines = vec!["a\n".to_string(), "b\n".to_string(), "c\n".to_string()];
    let v2_lines = vec![
        "a\n".to_string(),
        "insert\n".to_string(),
        "b\n".to_string(),
        "c\n".to_string(),
    ];
    let v3_lines = vec![
        "a\n".to_string(),
        "insert\n".to_string(),
        "B\n".to_string(),
        "c\n".to_string(),
    ];
    let mut owners = vec![None, None];
    let tracked = vec![
        SelectedLineTracker {
            output_index: 0,
            current_index: 1,
        },
        SelectedLineTracker {
            output_index: 1,
            current_index: 2,
        },
    ];

    let tracked = map_tracked_lines_to_parent(&v2_lines, &v3_lines, &tracked, "S3", &mut owners);
    assert_eq!(
        tracked,
        vec![SelectedLineTracker {
            output_index: 0,
            current_index: 1,
        }]
    );
    assert_eq!(owners[1], Some("S3".to_string()));

    let tracked = map_tracked_lines_to_parent(&v1_lines, &v2_lines, &tracked, "S2", &mut owners);
    assert!(tracked.is_empty());
    assert_eq!(owners[0], Some("S2".to_string()));
}
