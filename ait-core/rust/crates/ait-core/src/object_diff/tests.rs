use super::*;
use crate::json_support::json;

struct MemoryObjectReader {
    objects: BTreeMap<String, JsonValue>,
}

impl ObjectReader for MemoryObjectReader {
    fn read_object_json(&self, object_id: &str) -> Result<Option<JsonValue>, String> {
        Ok(self.objects.get(object_id).cloned())
    }
}

struct MemoryBlobReader {
    blobs: BTreeMap<String, Vec<u8>>,
}

impl BlobReader for MemoryBlobReader {
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Option<Vec<u8>>, String> {
        Ok(self.blobs.get(blob_id).cloned())
    }
}

struct StrictBlobReader {
    blobs: BTreeMap<String, Vec<u8>>,
}

impl BlobReader for StrictBlobReader {
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Option<Vec<u8>>, String> {
        self.blobs
            .get(blob_id)
            .cloned()
            .map(Some)
            .ok_or_else(|| format!("unexpected blob read `{blob_id}`"))
    }
}

struct MemorySnapshotReader {
    snapshots: BTreeMap<String, JsonValue>,
}

#[test]
fn workspace_diff_reports_conservative_changed_content_bytes() {
    let entries = vec![
        WorkspaceDiffEntry {
            path: "modified.txt".to_string(),
            status: "modified".to_string(),
            old_bytes: Some(b"old".to_vec()),
            new_bytes: Some(b"newer".to_vec()),
            old_mode: Some("0o644".to_string()),
            new_mode: Some("0o644".to_string()),
        },
        WorkspaceDiffEntry {
            path: "deleted.txt".to_string(),
            status: "missing".to_string(),
            old_bytes: Some(b"gone".to_vec()),
            new_bytes: None,
            old_mode: Some("0o644".to_string()),
            new_mode: None,
        },
    ];
    let payload = workspace_diff_from_entries(&entries, "base", "workspace", false, 0);
    assert_eq!(payload["summary"]["changed_bytes"], json!(12));
    assert_eq!(payload["files"][0]["old_size_bytes"], json!(3));
    assert_eq!(payload["files"][0]["new_size_bytes"], json!(5));
    assert_eq!(payload["files"][1]["old_size_bytes"], json!(4));
    assert_eq!(payload["files"][1]["new_size_bytes"], json!(0));
}

impl SnapshotReader for MemorySnapshotReader {
    fn read_snapshot_manifest(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        self.snapshots
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| format!("missing snapshot `{snapshot_id}`"))
    }
}

struct TreeBackedMemorySnapshotReader {
    root_tree_payloads: BTreeMap<String, JsonValue>,
    tree_payloads: BTreeMap<String, JsonValue>,
}

impl SnapshotReader for TreeBackedMemorySnapshotReader {
    fn read_snapshot_manifest(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        Err(format!(
            "snapshot manifest port must not be used for tree-backed snapshot `{snapshot_id}`"
        ))
    }

    fn read_snapshot_root_tree_payload(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<JsonValue>, String> {
        Ok(self.root_tree_payloads.get(snapshot_id).cloned())
    }

    fn read_tree_payload(&self, tree_id: &str) -> Result<Option<JsonValue>, String> {
        Ok(self.tree_payloads.get(tree_id).cloned())
    }
}

#[test]
fn diff_snapshot_manifests_tracks_added_modified_deleted_and_mode_changed() {
    let old_files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "A", "size_bytes": 3, "mode": "0o644"},
        "b.txt": {"path": "b.txt", "blob_id": "B", "size_bytes": 1, "mode": "0o644"},
        "c.txt": {"path": "c.txt", "blob_id": "C", "size_bytes": 1, "mode": "0o644"},
    });
    let new_files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "A", "size_bytes": 3, "mode": "0o644"},
        "b.txt": {"path": "b.txt", "blob_id": "B", "size_bytes": 1, "mode": "0o755"},
        "d.txt": {"path": "d.txt", "blob_id": "D", "size_bytes": 2, "mode": "0o644"},
        "c.txt": {"path": "c.txt", "blob_id": "C2", "size_bytes": 1, "mode": "0o644"},
    });

    let result = diff_snapshot_manifests(&old_files, &new_files, Some("S1"), Some("S2")).unwrap();
    assert_eq!(result["added"], json!(["d.txt"]));
    assert_eq!(result["deleted"], json!([]));
    assert_eq!(result["modified"], json!(["c.txt"]));
    assert_eq!(result["mode_changed"], json!(["b.txt"]));
    assert_eq!(result["summary"]["files_changed"], json!(3));
}

#[test]
fn diff_snapshot_manifests_ignores_missing_size_bytes_when_blob_and_mode_match() {
    let old_files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "A", "size_bytes": null, "mode": "0o644"},
        "b.txt": {"path": "b.txt", "blob_id": "B", "size_bytes": null, "mode": "0o644"},
    });
    let new_files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "A", "size_bytes": 10, "mode": "0o644"},
        "b.txt": {"path": "b.txt", "blob_id": "B2", "size_bytes": 7, "mode": "0o644"},
    });

    let result = diff_snapshot_manifests(&old_files, &new_files, Some("S1"), Some("S2")).unwrap();
    assert_eq!(result["modified"], json!(["b.txt"]));
    assert_eq!(result["mode_changed"], json!([]));
    assert_eq!(result["summary"]["files_changed"], json!(1));
}

#[test]
fn diff_snapshot_manifests_reports_exact_rename_hints() {
    let old_files = json!({
        "docs/old-name.md": {"path": "docs/old-name.md", "blob_id": "BLB-1", "size_bytes": 12, "mode": "0o644"},
    });
    let new_files = json!({
        "guides/new-name.md": {"path": "guides/new-name.md", "blob_id": "BLB-1", "size_bytes": 12, "mode": "0o644"},
    });

    let result = diff_snapshot_manifests(&old_files, &new_files, Some("S1"), Some("S2")).unwrap();
    assert_eq!(result["rename_hints"][0]["blob_id"], json!("BLB-1"));
    assert_eq!(result["directory_move_hints"], json!([]));
}

#[test]
fn diff_snapshot_manifests_groups_unambiguous_directory_moves() {
    let old_files = json!({
        "src/a.py": {"path": "src/a.py", "blob_id": "BLB-a", "size_bytes": 10, "mode": "0o644"},
        "src/b.py": {"path": "src/b.py", "blob_id": "BLB-b", "size_bytes": 20, "mode": "0o644"},
    });
    let new_files = json!({
        "pkg/a.py": {"path": "pkg/a.py", "blob_id": "BLB-a", "size_bytes": 10, "mode": "0o644"},
        "pkg/b.py": {"path": "pkg/b.py", "blob_id": "BLB-b", "size_bytes": 20, "mode": "0o644"},
    });

    let result = diff_snapshot_manifests(&old_files, &new_files, None, None).unwrap();
    assert_eq!(result["directory_move_hints"][0]["rename_count"], json!(2));
}

#[test]
fn snapshot_diff_from_manifests_adds_text_payloads() {
    let old_files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"},
    });
    let new_files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "BLB-NEW", "size_bytes": 10, "mode": "0o644"},
    });
    let blobs = BTreeMap::from([
        ("BLB-OLD".to_string(), b"hello\nfoo\n".to_vec()),
        ("BLB-NEW".to_string(), b"hello\nbar\n".to_vec()),
    ]);

    let result = snapshot_diff_from_manifests(
        &old_files,
        &new_files,
        &blobs,
        Some("S1"),
        Some("S2"),
        true,
        1_000_000,
    )
    .unwrap();

    assert_eq!(result["summary"]["files_changed"], json!(1));
    assert_eq!(result["summary"]["old_snapshot_id"], json!("S1"));
    assert_eq!(result["files"][0]["diff"]["status"], json!("text"));
    assert!(result["files"][0]["diff"]["text"]
        .as_str()
        .unwrap_or("")
        .contains("+bar"));
}

#[test]
fn snapshot_diff_from_manifests_reports_binary_and_too_large_boundaries() {
    let files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "BLB-A", "size_bytes": 4, "mode": "0o644"},
    });
    let other = json!({
        "a.txt": {"path": "a.txt", "blob_id": "BLB-B", "size_bytes": 4, "mode": "0o644"},
    });
    let binary_blobs = BTreeMap::from([
        ("BLB-A".to_string(), b"abc\n".to_vec()),
        ("BLB-B".to_string(), b"a\0c\n".to_vec()),
    ]);
    let binary =
        snapshot_diff_from_manifests(&files, &other, &binary_blobs, None, None, true, 10).unwrap();
    assert_eq!(binary["files"][0]["diff"]["status"], json!("binary"));

    let big_blobs = BTreeMap::from([
        ("BLB-A".to_string(), b"abc\n".to_vec()),
        ("BLB-B".to_string(), b"0123456789".to_vec()),
    ]);
    let too_large =
        snapshot_diff_from_manifests(&files, &other, &big_blobs, None, None, true, 4).unwrap();
    assert_eq!(too_large["files"][0]["diff"]["status"], json!("too_large"));
}

#[test]
fn snapshot_manifest_from_object_reader_accepts_nested_files_payload() {
    let reader = MemoryObjectReader {
        objects: BTreeMap::from([(
            "SNP-1".to_string(),
            json!({
                "snapshot_id": "SNP-1",
                "files": {
                    "a.txt": {"path": "a.txt", "blob_id": "BLB-A", "size_bytes": 4, "mode": "0o644"}
                }
            }),
        )]),
    };

    let result = snapshot_manifest_from_object_reader(&reader, "SNP-1").unwrap();
    assert_eq!(result["a.txt"]["blob_id"], json!("BLB-A"));
}

#[test]
fn snapshot_manifest_from_object_reader_derives_rows_from_tree_payloads() {
    let reader = MemoryObjectReader {
        objects: BTreeMap::from([
            (
                "SNP-1".to_string(),
                json!({
                    "snapshot_id": "SNP-1",
                    "root_tree": {
                        "tree_id": "TRE-ROOT",
                        "entries": [
                            {"entry_name": "README.md", "entry_type": "blob", "target_id": "BLB-README", "size_bytes": 5, "mode": "0o644"},
                            {"entry_name": "src", "entry_type": "tree", "target_id": "TRE-SRC", "mode": "tree"},
                        ]
                    },
                }),
            ),
            (
                "TRE-ROOT".to_string(),
                json!({
                    "tree_id": "TRE-ROOT",
                    "entries": [
                        {"entry_name": "README.md", "entry_type": "blob", "target_id": "BLB-README", "size_bytes": 5, "mode": "0o644"},
                        {"entry_name": "src", "entry_type": "tree", "target_id": "TRE-SRC", "mode": "tree"},
                    ]
                }),
            ),
            (
                "TRE-SRC".to_string(),
                json!({
                    "tree_id": "TRE-SRC",
                    "entries": [
                        {"entry_name": "lib.rs", "entry_type": "blob", "target_id": "BLB-LIB", "size_bytes": 11, "mode": "0o644"},
                    ]
                }),
            ),
        ]),
    };

    let result = snapshot_manifest_from_object_reader(&reader, "SNP-1").unwrap();
    assert_eq!(result["README.md"]["blob_id"], json!("BLB-README"));
    assert_eq!(result["src/lib.rs"]["blob_id"], json!("BLB-LIB"));
}

#[test]
fn snapshot_diff_from_readers_matches_file_map_entrypoint() {
    let old_files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"},
    });
    let new_files = json!({
        "a.txt": {"path": "a.txt", "blob_id": "BLB-NEW", "size_bytes": 10, "mode": "0o644"},
    });
    let snapshot_reader = MemorySnapshotReader {
        snapshots: BTreeMap::from([
            ("S1".to_string(), old_files.clone()),
            ("S2".to_string(), new_files.clone()),
        ]),
    };
    let blob_reader = MemoryBlobReader {
        blobs: BTreeMap::from([
            ("BLB-OLD".to_string(), b"hello\nfoo\n".to_vec()),
            ("BLB-NEW".to_string(), b"hello\nbar\n".to_vec()),
        ]),
    };

    let from_maps = snapshot_diff_from_manifests(
        &old_files,
        &new_files,
        &blob_reader.blobs,
        Some("S1"),
        Some("S2"),
        true,
        1024,
    )
    .unwrap();
    let from_readers = snapshot_diff_from_readers(
        &snapshot_reader,
        Some(&blob_reader),
        Some("S1"),
        Some("S2"),
        true,
        1024,
    )
    .unwrap();

    assert_eq!(from_readers, from_maps);
}

#[test]
fn object_diff_reader_entrypoints_accept_trait_objects() {
    let object_storage = MemoryObjectReader {
        objects: BTreeMap::from([
            (
                "S1".to_string(),
                json!({"files": {"a.txt": {"path": "a.txt", "blob_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"}}}),
            ),
            (
                "S2".to_string(),
                json!({"files": {"a.txt": {"path": "a.txt", "blob_id": "BLB-NEW", "size_bytes": 10, "mode": "0o644"}}}),
            ),
        ]),
    };
    let snapshot_storage = MemorySnapshotReader {
        snapshots: BTreeMap::from([
            (
                "S1".to_string(),
                json!({"a.txt": {"path": "a.txt", "blob_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"}}),
            ),
            (
                "S2".to_string(),
                json!({"a.txt": {"path": "a.txt", "blob_id": "BLB-NEW", "size_bytes": 10, "mode": "0o644"}}),
            ),
        ]),
    };
    let blob_storage = MemoryBlobReader {
        blobs: BTreeMap::from([
            ("BLB-OLD".to_string(), b"hello\nfoo\n".to_vec()),
            ("BLB-NEW".to_string(), b"hello\nbar\n".to_vec()),
        ]),
    };

    let object_reader: &dyn ObjectReader = &object_storage;
    let snapshot_reader: &dyn SnapshotReader = &snapshot_storage;
    let blob_reader: &dyn BlobReader = &blob_storage;

    let manifest = snapshot_manifest_from_object_reader(object_reader, "S1").unwrap();
    assert_eq!(manifest["a.txt"]["blob_id"], json!("BLB-OLD"));

    let from_snapshot_reader = snapshot_diff_from_readers(
        snapshot_reader,
        Some(blob_reader),
        Some("S1"),
        Some("S2"),
        true,
        1024,
    )
    .unwrap();
    assert_eq!(
        from_snapshot_reader["files"][0]["diff"]["status"],
        json!("text")
    );

    let from_object_reader = snapshot_diff_from_object_reader(
        object_reader,
        Some(blob_reader),
        Some("S1"),
        Some("S2"),
        true,
        1024,
    )
    .unwrap();
    assert_eq!(
        from_object_reader["files"][0]["diff"]["status"],
        json!("text")
    );
}

#[test]
fn snapshot_diff_from_object_reader_uses_object_and_blob_ports() {
    let object_reader = MemoryObjectReader {
        objects: BTreeMap::from([
            (
                "S1".to_string(),
                json!({"files": {"a.txt": {"path": "a.txt", "blob_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"}}}),
            ),
            (
                "S2".to_string(),
                json!({"files": {"a.txt": {"path": "a.txt", "blob_id": "BLB-NEW", "size_bytes": 10, "mode": "0o644"}}}),
            ),
        ]),
    };
    let blob_reader = MemoryBlobReader {
        blobs: BTreeMap::from([
            ("BLB-OLD".to_string(), b"hello\nfoo\n".to_vec()),
            ("BLB-NEW".to_string(), b"hello\nbar\n".to_vec()),
        ]),
    };

    let result = snapshot_diff_from_object_reader(
        &object_reader,
        Some(&blob_reader),
        Some("S1"),
        Some("S2"),
        true,
        1024,
    )
    .unwrap();

    assert_eq!(result["files"][0]["diff"]["status"], json!("text"));
    assert!(result["files"][0]["diff"]["text"]
        .as_str()
        .unwrap_or("")
        .contains("+bar"));
}

#[test]
fn snapshot_diff_from_object_reader_uses_tree_payloads_without_nested_files() {
    let object_reader = MemoryObjectReader {
        objects: BTreeMap::from([
            (
                "S1".to_string(),
                json!({
                    "snapshot_id": "S1",
                    "root_tree": {
                        "tree_id": "TRE-OLD",
                        "entries": [
                            {"entry_name": "a.txt", "entry_type": "blob", "target_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"}
                        ]
                    },
                }),
            ),
            (
                "S2".to_string(),
                json!({
                    "snapshot_id": "S2",
                    "root_tree": {
                        "tree_id": "TRE-NEW",
                        "entries": [
                            {"entry_name": "a.txt", "entry_type": "blob", "target_id": "BLB-NEW", "size_bytes": 10, "mode": "0o644"}
                        ]
                    },
                }),
            ),
            (
                "TRE-OLD".to_string(),
                json!({
                    "tree_id": "TRE-OLD",
                    "entries": [
                        {"entry_name": "a.txt", "entry_type": "blob", "target_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"}
                    ]
                }),
            ),
            (
                "TRE-NEW".to_string(),
                json!({
                    "tree_id": "TRE-NEW",
                    "entries": [
                        {"entry_name": "a.txt", "entry_type": "blob", "target_id": "BLB-NEW", "size_bytes": 10, "mode": "0o644"}
                    ]
                }),
            ),
        ]),
    };
    let blob_reader = MemoryBlobReader {
        blobs: BTreeMap::from([
            ("BLB-OLD".to_string(), b"hello\nfoo\n".to_vec()),
            ("BLB-NEW".to_string(), b"hello\nbar\n".to_vec()),
        ]),
    };

    let result = snapshot_diff_from_object_reader(
        &object_reader,
        Some(&blob_reader),
        Some("S1"),
        Some("S2"),
        true,
        1024,
    )
    .unwrap();

    assert_eq!(result["modified"], json!(["a.txt"]));
    assert_eq!(result["files"][0]["diff"]["status"], json!("text"));
}

#[test]
fn snapshot_diff_from_readers_uses_tree_payloads_without_snapshot_manifests() {
    let snapshot_reader = TreeBackedMemorySnapshotReader {
        root_tree_payloads: BTreeMap::from([
            (
                "S1".to_string(),
                json!({
                    "tree_id": "TRE-OLD",
                    "entries": [
                        {"entry_name": "docs", "entry_type": "tree", "target_id": "TRE-OLD-DOCS", "mode": "tree"}
                    ]
                }),
            ),
            (
                "S2".to_string(),
                json!({
                    "tree_id": "TRE-NEW",
                    "entries": [
                        {"entry_name": "docs", "entry_type": "tree", "target_id": "TRE-NEW-DOCS", "mode": "tree"}
                    ]
                }),
            ),
        ]),
        tree_payloads: BTreeMap::from([
            (
                "TRE-OLD".to_string(),
                json!({
                    "tree_id": "TRE-OLD",
                    "entries": [
                        {"entry_name": "docs", "entry_type": "tree", "target_id": "TRE-OLD-DOCS", "mode": "tree"}
                    ]
                }),
            ),
            (
                "TRE-OLD-DOCS".to_string(),
                json!({
                    "tree_id": "TRE-OLD-DOCS",
                    "entries": [
                        {"entry_name": "a.md", "entry_type": "blob", "target_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"}
                    ]
                }),
            ),
            (
                "TRE-NEW".to_string(),
                json!({
                    "tree_id": "TRE-NEW",
                    "entries": [
                        {"entry_name": "docs", "entry_type": "tree", "target_id": "TRE-NEW-DOCS", "mode": "tree"}
                    ]
                }),
            ),
            (
                "TRE-NEW-DOCS".to_string(),
                json!({
                    "tree_id": "TRE-NEW-DOCS",
                    "entries": [
                        {"entry_name": "a.md", "entry_type": "blob", "target_id": "BLB-OLD", "size_bytes": 10, "mode": "0o755"},
                        {"entry_name": "b.md", "entry_type": "blob", "target_id": "BLB-NEW", "size_bytes": 7, "mode": "0o644"}
                    ]
                }),
            ),
        ]),
    };

    let result = snapshot_diff_from_readers(
        &snapshot_reader,
        Option::<&MemoryBlobReader>::None,
        Some("S1"),
        Some("S2"),
        false,
        1024,
    )
    .unwrap();

    assert_eq!(result["added"], json!(["docs/b.md"]));
    assert_eq!(result["mode_changed"], json!(["docs/a.md"]));
}

#[test]
fn snapshot_diff_from_tree_reader_text_reads_only_modified_blobs() {
    let snapshot_reader = TreeBackedMemorySnapshotReader {
        root_tree_payloads: BTreeMap::from([
            (
                "S1".to_string(),
                json!({
                    "tree_id": "TRE-OLD",
                    "entries": [
                        {"entry_name": "same.md", "entry_type": "blob", "target_id": "BLB-SAME", "size_bytes": 5, "mode": "0o644"},
                        {"entry_name": "deleted.md", "entry_type": "blob", "target_id": "BLB-DELETED", "size_bytes": 8, "mode": "0o644"},
                        {"entry_name": "modified.md", "entry_type": "blob", "target_id": "BLB-MOD-OLD", "size_bytes": 10, "mode": "0o644"}
                    ]
                }),
            ),
            (
                "S2".to_string(),
                json!({
                    "tree_id": "TRE-NEW",
                    "entries": [
                        {"entry_name": "same.md", "entry_type": "blob", "target_id": "BLB-SAME", "size_bytes": 5, "mode": "0o644"},
                        {"entry_name": "added.md", "entry_type": "blob", "target_id": "BLB-ADDED", "size_bytes": 6, "mode": "0o644"},
                        {"entry_name": "modified.md", "entry_type": "blob", "target_id": "BLB-MOD-NEW", "size_bytes": 10, "mode": "0o644"}
                    ]
                }),
            ),
        ]),
        tree_payloads: BTreeMap::from([
            (
                "TRE-OLD".to_string(),
                json!({
                    "tree_id": "TRE-OLD",
                    "entries": [
                        {"entry_name": "same.md", "entry_type": "blob", "target_id": "BLB-SAME", "size_bytes": 5, "mode": "0o644"},
                        {"entry_name": "deleted.md", "entry_type": "blob", "target_id": "BLB-DELETED", "size_bytes": 8, "mode": "0o644"},
                        {"entry_name": "modified.md", "entry_type": "blob", "target_id": "BLB-MOD-OLD", "size_bytes": 10, "mode": "0o644"}
                    ]
                }),
            ),
            (
                "TRE-NEW".to_string(),
                json!({
                    "tree_id": "TRE-NEW",
                    "entries": [
                        {"entry_name": "same.md", "entry_type": "blob", "target_id": "BLB-SAME", "size_bytes": 5, "mode": "0o644"},
                        {"entry_name": "added.md", "entry_type": "blob", "target_id": "BLB-ADDED", "size_bytes": 6, "mode": "0o644"},
                        {"entry_name": "modified.md", "entry_type": "blob", "target_id": "BLB-MOD-NEW", "size_bytes": 10, "mode": "0o644"}
                    ]
                }),
            ),
        ]),
    };
    let blob_reader = StrictBlobReader {
        blobs: BTreeMap::from([
            ("BLB-MOD-OLD".to_string(), b"hello\nold\n".to_vec()),
            ("BLB-MOD-NEW".to_string(), b"hello\nnew\n".to_vec()),
        ]),
    };

    let result = snapshot_diff_from_readers(
        &snapshot_reader,
        Some(&blob_reader),
        Some("S1"),
        Some("S2"),
        true,
        1024,
    )
    .unwrap();

    assert_eq!(result["modified"], json!(["modified.md"]));
    let modified_row = result["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["path"] == json!("modified.md"))
        .unwrap();
    assert_eq!(modified_row["diff"]["status"], json!("text"));
}

#[test]
fn snapshot_diff_from_readers_requires_blob_reader_for_text_enrichment() {
    let snapshot_reader = MemorySnapshotReader {
        snapshots: BTreeMap::from([
            (
                "S1".to_string(),
                json!({"a.txt": {"path": "a.txt", "blob_id": "BLB-OLD", "size_bytes": 10, "mode": "0o644"}}),
            ),
            (
                "S2".to_string(),
                json!({"a.txt": {"path": "a.txt", "blob_id": "BLB-NEW", "size_bytes": 10, "mode": "0o644"}}),
            ),
        ]),
    };

    let error = snapshot_diff_from_readers(
        &snapshot_reader,
        Option::<&MemoryBlobReader>::None,
        Some("S1"),
        Some("S2"),
        true,
        1024,
    )
    .unwrap_err();
    assert!(error.contains("blob_reader is required"));
}

#[test]
fn coerce_snapshot_manifest_rejects_invalid_shapes_and_mode_inputs() {
    assert_eq!(to_mode_int(&JsonValue::Null).unwrap(), 0);
    assert_eq!(to_mode_int(&json!("0o755")).unwrap(), 0o755);
    assert_eq!(to_mode_int(&json!("0x1ed")).unwrap(), 0x1ed);
    assert_eq!(to_mode_int(&json!("755")).unwrap(), 755);
    assert!(to_mode_int(&json!("bad-mode")).is_err());

    assert!(coerce_snapshot_manifest(&json!({"a.txt": "bad"})).is_err());
    assert!(coerce_snapshot_manifest(&json!(["bad-row"])).is_err());
    assert!(coerce_snapshot_manifest(&json!([{"blob_id": "A"}])).is_err());
}

#[test]
fn safe_decode_text_and_missing_blob_paths_cover_boundary_states() {
    assert_eq!(
        safe_decode_text(b"hello\n", 32),
        (true, Some("hello\n".to_string()), None)
    );
    assert_eq!(safe_decode_text(b"a\0b", 32), (false, None, Some("binary")));
    assert_eq!(
        safe_decode_text(b"12345", 4),
        (false, None, Some("too_large"))
    );

    let old_row = SnapshotFileRow {
        path: "a.txt".to_string(),
        blob_id: Some("BLB-OLD".to_string()),
        size_bytes: Some(4),
        mode_raw: json!("0o644"),
        mode_int: 0o644,
    };
    let new_row = SnapshotFileRow {
        path: "a.txt".to_string(),
        blob_id: Some("BLB-NEW".to_string()),
        size_bytes: Some(4),
        mode_raw: json!("0o644"),
        mode_int: 0o644,
    };
    let unavailable = maybe_add_text_diff_from_blob_bytes(
        &BTreeMap::new(),
        "a.txt",
        &old_row,
        &new_row,
        Some("S1"),
        Some("S2"),
        32,
    );
    assert_eq!(unavailable.status, "unavailable");
    assert_eq!(unavailable.insertions, 0);
    assert_eq!(unavailable.deletions, 0);
    assert_eq!(unavailable.text, None);
}
