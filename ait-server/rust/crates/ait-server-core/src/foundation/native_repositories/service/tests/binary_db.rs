use super::common::*;
use super::*;
use crate::foundation::remote_binary_db::test_support::{
    BinaryDbTestFaultTiming, BinaryDbTestStorageOperation, FaultInjectingServerBinaryDbStore,
};
use crate::foundation::remote_binary_db::{
    ServerBinaryDbAuthorityMode, ServerBinaryDbFilesystemStore,
};
use crate::foundation::server_content_binary_db::{
    validate_server_tree_authority_v0, validate_server_tree_serving_authority_v0,
    ServerBinaryTreeReadCache, SERVER_SNAPSHOT_BIN, SERVER_SNAPSHOT_ID_IDX,
    SERVER_SNAPSHOT_PARENT_EDGE_BIN,
};
use std::io::Write as _;
use std::sync::{Arc, Barrier};

#[test]
fn native_repository_pack_storage_projects_empty_and_committed_inventory() {
    let db = native_line_binary_db("repository-pack-inventory");
    let service = BinaryDbNativeRepositoryService::new(db);
    let empty = service
        .create_repository(RepositoryCreateRequest {
            repo_name: "repo-bin".to_string(),
            default_line: "main".to_string(),
            policy: json!({}),
            id_namespace_prefix: None,
        })
        .expect("empty Binary Repository should project storage");
    assert_eq!(empty["pack_storage"]["object_pack_count"], 0);
    assert_eq!(empty["pack_storage"]["tree_pack_count"], 0);
    assert_eq!(empty["pack_storage"]["zstd_only_verified"], true);

    seed_native_binary_content(&service, "repository-pack-inventory");
    let populated = service
        .get_repository("repo-bin")
        .expect("committed Pack inventory should project from Binary authority");
    assert_eq!(populated["pack_storage"]["object_pack_count"], 1);
    assert_eq!(populated["pack_storage"]["tree_pack_count"], 1);
    assert_eq!(populated["pack_storage"]["zstd_object_pack_count"], 1);
    assert_eq!(populated["pack_storage"]["zstd_tree_pack_count"], 1);
    assert_eq!(populated["pack_storage"]["validation"]["state"], "valid");
}

#[test]
fn native_line_binary_db_list_get_update_and_close_without_postgres() {
    let db = native_line_binary_db("line-crud");
    seed_native_binary_snapshot(&db, "SNP-000000000001");
    let authority_db = db.clone();
    let service = BinaryDbNativeRepositoryService::new(db);

    let repo = service
        .create_repository(RepositoryCreateRequest {
            repo_name: "repo-bin".to_string(),
            default_line: "main".to_string(),
            policy: json!({}),
            id_namespace_prefix: None,
        })
        .expect("binary repository should be creatable");
    assert_eq!(repo["repo_name"], "repo-bin");
    assert_eq!(repo["pack_storage"]["zstd_only_verified"], true);

    let main = service
        .update_line(
            "repo-bin",
            "main",
            LineUpdateRequest {
                head_snapshot_id: Some("SNP-000000000001".to_string()),
                expected_head_snapshot_id: None,
            },
        )
        .expect("main line should update");
    assert_eq!(main["head_snapshot_id"], "SNP-000000000001");
    service
        .ensure_default_line()
        .expect("repeated default-Line ensure should be idempotent");
    let preserved_main = service
        .get_line("repo-bin", "main")
        .expect("existing main Line should remain readable");
    assert_eq!(
        preserved_main["head_snapshot_id"], "SNP-000000000001",
        "default-Line repair must not replace an existing head",
    );

    let feature = service
        .update_line(
            "repo-bin",
            "feature/native-line",
            LineUpdateRequest {
                head_snapshot_id: Some("SNP-000000000001".to_string()),
                expected_head_snapshot_id: None,
            },
        )
        .expect("feature line should create");
    assert_eq!(feature["line_name"], "feature/native-line");
    assert_eq!(feature["status"], "active");

    let listed = service
        .list_lines("repo-bin")
        .expect("lines should list")
        .as_array()
        .expect("lines response should be an array")
        .iter()
        .map(|line| line["line_name"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(listed, vec!["feature/native-line", "main"]);

    let closed = service
        .close_line(
            "repo-bin",
            "feature/native-line",
            LineCloseRequest {
                status: "archived".to_string(),
            },
        )
        .expect("feature line should close");
    assert_eq!(closed["status"], "archived");
    assert!(closed["archived_at"].as_str().is_some());
    assert_eq!(
        authority_db
            .record_count(ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file())
            .expect("canonical line count should read"),
        2
    );
    for path in [
        "workflow_line_projection.bin",
        "workflow_line_projection_payload.bin",
        "line_head.bin",
    ] {
        assert!(
            !ServerRemoteBinaryDb::authority_root(&authority_db)
                .as_path()
                .join(path)
                .exists(),
            "native lines must not create duplicate workflow content file {path}"
        );
    }

    let err = service
        .update_line(
            "repo-bin",
            "feature/native-line",
            LineUpdateRequest {
                head_snapshot_id: Some("SNP-000000000001".to_string()),
                expected_head_snapshot_id: Some("SNP-000000000001".to_string()),
            },
        )
        .expect_err("archived line should not move");
    assert_eq!(err.kind, NativeRepositoryErrorKind::BadRequest);
    assert!(err.message.contains("archived"));

    let err = service
        .close_line(
            "repo-bin",
            "main",
            LineCloseRequest {
                status: "archived".to_string(),
            },
        )
        .expect_err("default line should not close");
    assert_eq!(err.kind, NativeRepositoryErrorKind::BadRequest);
    assert!(err.message.contains("Default line main"));
}

#[test]
fn native_line_binary_db_rejects_unknown_snapshot_and_stale_head() {
    let db = native_line_binary_db("line-cas");
    seed_native_binary_snapshot(&db, "SNP-000000000002");
    seed_native_binary_snapshot(&db, "SNP-000000000003");
    let service = BinaryDbNativeRepositoryService::new(db);
    service
        .create_repository(RepositoryCreateRequest {
            repo_name: "repo-bin".to_string(),
            default_line: "main".to_string(),
            policy: json!({}),
            id_namespace_prefix: None,
        })
        .expect("binary repository should be creatable");

    let missing = service
        .update_line(
            "repo-bin",
            "main",
            LineUpdateRequest {
                head_snapshot_id: Some("SNP-00000000FFFF".to_string()),
                expected_head_snapshot_id: None,
            },
        )
        .expect_err("unknown snapshot should fail closed");
    assert_eq!(missing.kind, NativeRepositoryErrorKind::NotFound);

    service
        .update_line(
            "repo-bin",
            "main",
            LineUpdateRequest {
                head_snapshot_id: Some("SNP-000000000002".to_string()),
                expected_head_snapshot_id: None,
            },
        )
        .expect("main should move to first snapshot");

    let stale = service
        .update_line(
            "repo-bin",
            "main",
            LineUpdateRequest {
                head_snapshot_id: Some("SNP-000000000003".to_string()),
                expected_head_snapshot_id: Some("SNP-00000000FFFE".to_string()),
            },
        )
        .expect_err("stale expected head should fail");
    assert_eq!(stale.kind, NativeRepositoryErrorKind::BadRequest);
    assert!(stale.message.contains("head advanced"));
    assert_eq!(
        service
            .get_line("repo-bin", "main")
            .expect("line should remain on first snapshot")["head_snapshot_id"],
        "SNP-000000000002"
    );

    let moved = service
        .update_line(
            "repo-bin",
            "main",
            LineUpdateRequest {
                head_snapshot_id: Some("SNP-000000000003".to_string()),
                expected_head_snapshot_id: Some("SNP-000000000002".to_string()),
            },
        )
        .expect("matching expected head should move");
    assert_eq!(moved["head_snapshot_id"], "SNP-000000000003");
}

#[test]
fn native_snapshot_binary_db_uses_schema_tree_entries_for_export_and_materialization() {
    let db = native_line_binary_db("snapshot-json");
    let authority_db = db.clone();
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);
    let content = seed_native_binary_content(&service, "snapshot-json");
    let bundle = json!({
        "snapshot_id": "SNP-000000000010",
        "repo_name": "repo-bin",
        "line_name": "main",
        "message": "metadata snapshot",
        "root_tree_pack_id": content.tree_pack_id,
        "root_entry_ordinal": 0,
        "file_count": 1,
        "total_bytes": content.bytes.len(),
        "files": [{
            "path": "README.md",
            "blob_id": content.blob_id,
            "size_bytes": content.bytes.len(),
            "mode": "100644",
            "sha256": sha256_hex(&content.bytes),
        }],
    });

    let committed = service
        .commit_zstd_bulk(
            "repo-bin",
            json!({
                "contract": "ait.remote_sync.zstd_bulk.commit.v1",
                "object_packs": [],
                "tree_packs": [],
                "blob_locators": [],
                "tree_locators": [],
                "snapshots": [bundle],
            }),
        )
        .expect("current snapshot metadata should commit");
    assert_eq!(committed["upserted_snapshots"], 1);

    let exists = service
        .snapshot_existence(
            "repo-bin",
            SnapshotExistsRequest {
                snapshot_ids: vec![
                    "SNP-000000000010".to_string(),
                    "SNP-00000000FFFF".to_string(),
                ],
            },
        )
        .expect("existence should read binary snapshots");
    assert_eq!(exists["present"], json!(["SNP-000000000010"]));
    assert_eq!(exists["missing"], json!(["SNP-00000000FFFF"]));

    let exported = service
        .export_snapshot(
            "repo-bin",
            "SNP-000000000010",
            SnapshotExportQuery::default(),
        )
        .expect("metadata bundle should export");
    assert_eq!(exported["snapshot_id"], "SNP-000000000010");
    assert_eq!(exported["content_included"], false);
    assert_eq!(exported["files"][0]["path"], "README.md");
    assert_eq!(exported["files"][0]["blob_id"], content.blob_id);

    let destination = env::temp_dir().join(format!(
        "ait-server-core-schema-materialize-{}",
        std::process::id()
    ));
    if destination.exists() {
        fs::remove_dir_all(&destination).unwrap();
    }
    let materialized = service
        .materialize_snapshot("repo-bin", "SNP-000000000010", &destination)
        .expect("schema-defined snapshot should materialize");
    assert_eq!(materialized["file_count"], 1);
    assert_eq!(
        fs::read(destination.join("README.md")).unwrap(),
        content.bytes
    );
    fs::remove_dir_all(destination).unwrap();

    let repository_content = ServerBinaryRepositoryContentStore::new(authority_db.clone());
    assert!(repository_content
        .object_pack(&content.object_pack_id)
        .unwrap()
        .is_some());
    assert!(repository_content.tree(&content.tree_id).unwrap().is_some());
    assert_eq!(
        authority_db
            .record_count(
                ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
            )
            .expect("canonical snapshot count should read"),
        1
    );
    for path in [
        "workflow_snapshot_projection.bin",
        "workflow_snapshot_projection_payload.bin",
        "snapshot_head.bin",
    ] {
        assert!(
            !ServerRemoteBinaryDb::authority_root(&authority_db)
                .as_path()
                .join(path)
                .exists(),
            "native snapshots must not create duplicate workflow content file {path}"
        );
    }
}

#[test]
fn binary_zstd_import_manifest_expands_every_selected_tree_pack_member() {
    let db = native_line_binary_db("zstd-reachable-closure");
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);
    let fixture_root = env::temp_dir().join(format!(
        "ait-server-core-zstd-reachable-closure-{}",
        std::process::id()
    ));
    if fixture_root.exists() {
        fs::remove_dir_all(&fixture_root).expect("stale manifest fixture should remove");
    }
    fs::create_dir_all(&fixture_root).expect("manifest fixture root should create");

    let reachable_bytes = b"reachable snapshot content\n".to_vec();
    let reachable_sha256 = sha256_hex(&reachable_bytes);
    let reachable_blob_id = format!("BLB-{}", &reachable_sha256[..20]);
    let reachable_object_pack_id = "PCK-0000000000C0";
    let unrelated_bytes = b"unrelated historical content\n".to_vec();
    let unrelated_sha256 = sha256_hex(&unrelated_bytes);
    let unrelated_blob_id = format!("BLB-{}", &unrelated_sha256[..20]);
    let unrelated_object_pack_id = "PCK-0000000000C1";
    let created_at = "2026-07-19T00:00:00Z";

    for (pack_id, blob_id, sha256, bytes, path_hint) in [
        (
            reachable_object_pack_id,
            reachable_blob_id.as_str(),
            reachable_sha256.as_str(),
            reachable_bytes.as_slice(),
            "nested/README.md",
        ),
        (
            unrelated_object_pack_id,
            unrelated_blob_id.as_str(),
            unrelated_sha256.as_str(),
            unrelated_bytes.as_slice(),
            "UNRELATED.md",
        ),
    ] {
        let pack_path = fixture_root.join(format!("{pack_id}.zstpack"));
        write_rebuilt_zstd_pack_archive(
            pack_path
                .to_str()
                .expect("object pack fixture path should be UTF-8"),
            pack_id,
            created_at,
            vec![ObjectPackRewriteBlob {
                entry_name: format!("blobs/{blob_id}"),
                blob_id: blob_id.to_string(),
                data: bytes.to_vec(),
                path_hint: Some(path_hint.to_string()),
            }],
            0,
        )
        .expect("object pack fixture should write");
        service
            .seed_zstd_pack_batch_for_test(
                "repo-bin",
                vec![(
                    pack_id.to_string(),
                    fs::read(&pack_path).expect("object pack fixture should read"),
                )],
                false,
            )
            .expect("object pack fixture should import");
        service
            .seed_zstd_locator_batch_for_test(
                "repo-bin",
                vec![json!({
                    "blob_id": blob_id,
                    "sha256": sha256,
                    "size_bytes": bytes.len(),
                    "pack_id": pack_id,
                    "pack_entry_name": format!("blobs/{blob_id}"),
                    "pack_entry_type": "full",
                    "pack_base_blob_id": JsonValue::Null,
                    "pack_chain_depth": 0,
                    "created_at": created_at,
                })],
                false,
            )
            .expect("blob locator fixture should import");
    }

    let root_tree_id = "TRE-000000000000000000C0";
    let unrelated_tree_id = "TRE-000000000000000000C1";
    let child_tree_id = "TRE-000000000000000000C2";
    let unrelated_child_tree_id = "TRE-000000000000000000C3";
    let late_parent_tree_id = "TRE-000000000000000000C4";
    let root_tree_pack_id = "TPK-0000000000C0";
    let child_tree_pack_id = "TPK-0000000000C1";
    let unrelated_child_tree_pack_id = "TPK-0000000000C2";
    let root_tree_rows = json!([
        {"tree_id": root_tree_id, "entry_count": 1},
        {"tree_id": unrelated_tree_id, "entry_count": 1},
    ]);
    let root_tree_entry_rows = json!([
        {
            "tree_id": root_tree_id,
            "entry_name": "nested",
            "entry_type": "tree",
            "target_id": child_tree_id,
            "size_bytes": JsonValue::Null,
            "mode": "tree",
        },
        {
            "tree_id": unrelated_tree_id,
            "entry_name": "archive",
            "entry_type": "tree",
            "target_id": unrelated_child_tree_id,
            "size_bytes": JsonValue::Null,
            "mode": "tree",
        },
    ]);
    let root_members = build_tree_pack_members(&root_tree_rows, &root_tree_entry_rows)
        .expect("root tree pack members should build");
    let root_tree_pack_path = fixture_root.join(format!("{root_tree_pack_id}.zstpack"));
    let root_tree_pack_metadata = write_tree_pack_archive_with_format(
        root_tree_pack_path
            .to_str()
            .expect("root tree pack fixture path should be UTF-8"),
        root_tree_pack_id,
        created_at,
        &root_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("root tree pack fixture should write");
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                root_tree_pack_id.to_string(),
                fs::read(&root_tree_pack_path).expect("root tree pack fixture should read"),
            )],
            true,
        )
        .expect("root tree pack fixture should import");
    let root_tree_checksums = root_tree_pack_metadata["pack_index"]["trees"]
        .as_array()
        .expect("root tree pack checksums should exist");

    let child_tree_rows = json!([
        {"tree_id": child_tree_id, "entry_count": 1},
        {"tree_id": late_parent_tree_id, "entry_count": 1},
    ]);
    let child_tree_entry_rows = json!([
        {
            "tree_id": child_tree_id,
            "entry_name": "README.md",
            "entry_type": "blob",
            "target_id": reachable_blob_id,
            "size_bytes": reachable_bytes.len(),
            "mode": "100644",
        },
        {
            "tree_id": late_parent_tree_id,
            "entry_name": "archive",
            "entry_type": "tree",
            "target_id": unrelated_child_tree_id,
            "size_bytes": JsonValue::Null,
            "mode": "tree",
        },
    ]);
    let child_members = build_tree_pack_members(&child_tree_rows, &child_tree_entry_rows)
        .expect("child tree pack members should build");
    let child_tree_pack_path = fixture_root.join(format!("{child_tree_pack_id}.zstpack"));
    let child_tree_pack_metadata = write_tree_pack_archive_with_format(
        child_tree_pack_path
            .to_str()
            .expect("child tree pack fixture path should be UTF-8"),
        child_tree_pack_id,
        created_at,
        &child_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("child tree pack fixture should write");
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                child_tree_pack_id.to_string(),
                fs::read(&child_tree_pack_path).expect("child tree pack fixture should read"),
            )],
            true,
        )
        .expect("child tree pack fixture should import");
    let unrelated_child_tree_rows = json!([{"tree_id": unrelated_child_tree_id, "entry_count": 1}]);
    let unrelated_child_tree_entry_rows = json!([{
        "tree_id": unrelated_child_tree_id,
        "entry_name": "UNRELATED.md",
        "entry_type": "blob",
        "target_id": unrelated_blob_id,
        "size_bytes": unrelated_bytes.len(),
        "mode": "100644",
    }]);
    let unrelated_child_members =
        build_tree_pack_members(&unrelated_child_tree_rows, &unrelated_child_tree_entry_rows)
            .expect("unrelated child tree pack members should build");
    let unrelated_child_tree_pack_path =
        fixture_root.join(format!("{unrelated_child_tree_pack_id}.zstpack"));
    let unrelated_child_tree_pack_metadata = write_tree_pack_archive_with_format(
        unrelated_child_tree_pack_path
            .to_str()
            .expect("unrelated child tree pack fixture path should be UTF-8"),
        unrelated_child_tree_pack_id,
        created_at,
        &unrelated_child_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("unrelated child tree pack fixture should write");
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                unrelated_child_tree_pack_id.to_string(),
                fs::read(&unrelated_child_tree_pack_path)
                    .expect("unrelated child tree pack fixture should read"),
            )],
            true,
        )
        .expect("unrelated child tree pack fixture should import");
    let unrelated_child_tree_index = &unrelated_child_tree_pack_metadata["pack_index"]["trees"][0];
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "tree_id": unrelated_child_tree_id,
                "entry_count": 1,
                "tree_pack_id": unrelated_child_tree_pack_id,
                "tree_pack_checksum": unrelated_child_tree_index["checksum"],
                "created_at": created_at,
            })],
            true,
        )
        .expect("unrelated child tree locator should import");
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            child_tree_pack_metadata["pack_index"]["trees"]
                .as_array()
                .expect("child tree pack checksums should exist")
                .iter()
                .map(|tree| {
                    json!({
                        "tree_id": tree["tree_id"],
                        "entry_count": tree["entry_count"],
                        "tree_pack_id": child_tree_pack_id,
                        "tree_pack_checksum": tree["checksum"],
                        "created_at": created_at,
                    })
                })
                .collect(),
            true,
        )
        .expect("child tree locators should import after their unrelated child tree");
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            root_tree_checksums
                .iter()
                .map(|tree| {
                    json!({
                        "tree_id": tree["tree_id"],
                        "entry_count": tree["entry_count"],
                        "tree_pack_id": root_tree_pack_id,
                        "tree_pack_checksum": tree["checksum"],
                        "created_at": created_at,
                    })
                })
                .collect(),
            true,
        )
        .expect("root tree locators should import after its child tree");

    service
        .commit_zstd_bulk(
            "repo-bin",
            json!({
                "contract": "ait.remote_sync.zstd_bulk.commit.v1",
                "object_packs": [],
                "tree_packs": [],
                "blob_locators": [],
                "tree_locators": [],
                "snapshots": [{
                "snapshot_id": "SNP-0000000000C0",
                "repo_name": "repo-bin",
                "line_name": "main",
                "message": "reachable closure",
                "root_tree_pack_id": root_tree_pack_id,
                "root_entry_ordinal": 0,
                "file_count": 1,
                "total_bytes": reachable_bytes.len(),
                "created_at": created_at,
                "files": [{
                    "path": "nested/README.md",
                    "blob_id": reachable_blob_id,
                    "size_bytes": reachable_bytes.len(),
                    "mode": "100644",
                    "sha256": reachable_sha256,
                }],
                }],
            }),
        )
        .expect("reachable-closure snapshot should commit");

    service.reset_test_zstd_pack_payload_read_count();
    service.reset_test_import_manifest_read_counts();
    let manifest = service
        .get_zstd_import_manifest("repo-bin", "SNP-0000000000C0")
        .expect("reachable-closure manifest should build");
    for (file, minimum_bulk_count) in [
        ("object_pack.bin", 2),
        ("blob.bin", 2),
        ("object_pack_member.bin", 2),
        ("tree_entry_range.bin", 5),
        ("tree_entry.bin", 5),
    ] {
        let reads = service.test_import_manifest_record_read_ranges(file);
        assert_eq!(
            reads.len(),
            1,
            "manifest must issue one bulk fixed-authority read for {file}: {reads:?}"
        );
        assert_eq!(
            reads[0].0, 0,
            "manifest bulk read for {file} must start at zero"
        );
        assert!(
            reads[0].1 >= minimum_bulk_count,
            "manifest bulk read for {file} must cover the nontrivial fixture: {reads:?}"
        );
    }
    for (file, expected_bulk_count) in [("tree_pack.bin", 3), ("tree.bin", 5)] {
        let reads = service.test_import_manifest_record_read_ranges(file);
        assert_eq!(
            reads.last(),
            Some(&(0, expected_bulk_count)),
            "manifest content projection must finish {file} access with one complete bulk read: {reads:?}"
        );
        assert_eq!(
            reads
                .iter()
                .filter(|(_, count)| *count > 1)
                .collect::<Vec<_>>(),
            vec![&(0, expected_bulk_count)],
            "manifest content projection must not split the full {file} authority: {reads:?}"
        );
    }
    let name_reads = service.test_import_manifest_payload_read_ranges("tree_name_payload.bin");
    assert_eq!(
        name_reads.len(),
        1,
        "manifest must read the normalized Tree name payload body once: {name_reads:?}"
    );
    assert_eq!(name_reads[0].0, 4);
    assert!(name_reads[0].1 > 1);
    assert_eq!(
        service.test_import_manifest_zstd_file_read_counts(),
        (2, 3, 0),
        "each selected Object/Tree pack index must be read once and manifest traversal must not read pack chunks"
    );
    assert_eq!(
        manifest["tree_packs"]
            .as_array()
            .expect("tree packs should be an array")
            .iter()
            .map(|row| row["pack_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            root_tree_pack_id,
            child_tree_pack_id,
            unrelated_child_tree_pack_id,
        ])
    );
    let ordered_tree_pack_ids = manifest["tree_packs"]
        .as_array()
        .expect("tree packs should be an array")
        .iter()
        .map(|row| row["pack_id"].as_str().unwrap())
        .collect::<Vec<_>>();
    let root_position = ordered_tree_pack_ids
        .iter()
        .position(|pack_id| *pack_id == root_tree_pack_id)
        .expect("root tree pack should be present");
    for child_pack_id in [child_tree_pack_id, unrelated_child_tree_pack_id] {
        assert!(
            ordered_tree_pack_ids
                .iter()
                .position(|pack_id| *pack_id == child_pack_id)
                .expect("child tree pack should be present")
                < root_position,
            "manifest must order child tree pack {child_pack_id} before parent pack {root_tree_pack_id}"
        );
    }
    assert!(
        ordered_tree_pack_ids
            .iter()
            .position(|pack_id| *pack_id == unrelated_child_tree_pack_id)
            .expect("already visited child tree pack should be present")
            < ordered_tree_pack_ids
                .iter()
                .position(|pack_id| *pack_id == child_tree_pack_id)
                .expect("late parent tree pack should be present"),
        "a child visited before a later pack member must still order before that parent pack"
    );
    assert_eq!(
        manifest["tree_locators"]
            .as_array()
            .expect("tree locators should be an array")
            .iter()
            .map(|row| row["tree_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            root_tree_id,
            unrelated_tree_id,
            child_tree_id,
            unrelated_child_tree_id,
            late_parent_tree_id,
        ]),
        "included tree packs retain every member locator"
    );
    assert_eq!(
        manifest["object_packs"]
            .as_array()
            .expect("object packs should be an array")
            .iter()
            .map(|row| row["pack_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([reachable_object_pack_id, unrelated_object_pack_id]),
        "every tree physically carried by a selected pack must expand object-pack closure"
    );
    assert_eq!(
        manifest["blob_locators"]
            .as_array()
            .expect("blob locators should be an array")
            .iter()
            .map(|row| row["blob_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([reachable_blob_id.as_str(), unrelated_blob_id.as_str()])
    );
    assert_eq!(
        service
            .get_zstd_import_manifest("repo-bin", "SNP-0000000000C0")
            .expect("repeated reachable-closure manifest should build"),
        manifest,
        "manifest ordering and content must be deterministic"
    );
    let blob_manifest = service
        .get_zstd_blob_import_manifest("repo-bin", std::slice::from_ref(&reachable_blob_id))
        .expect("standalone Blob closure manifest should build");
    assert_eq!(
        blob_manifest["object_packs"]
            .as_array()
            .expect("Blob closure Object Packs")
            .iter()
            .map(|row| row["pack_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![reachable_object_pack_id]
    );
    assert_eq!(
        blob_manifest["blob_locators"]
            .as_array()
            .expect("Blob closure locators")
            .iter()
            .map(|row| row["blob_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![reachable_blob_id.as_str()]
    );
    assert_eq!(
        service.test_zstd_pack_payload_read_count(),
        0,
        "import manifests must derive committed pack metadata from bounded zstd index reads"
    );

    service
        .commit_zstd_bulk(
            "repo-bin",
            json!({
                "contract": "ait.remote_sync.zstd_bulk.commit.v1",
                "object_packs": [],
                "tree_packs": [],
                "blob_locators": [],
                "tree_locators": [],
                "snapshots": [{
                "snapshot_id": "SNP-0000000000C1",
                "repo_name": "repo-bin",
                "parent_snapshot_id": "SNP-0000000000C0",
                "line_name": "main",
                "message": "same content child",
                "root_tree_pack_id": root_tree_pack_id,
                "root_entry_ordinal": 0,
                "file_count": 1,
                "total_bytes": reachable_bytes.len(),
                "created_at": created_at,
                "files": [{
                    "path": "nested/README.md",
                    "blob_id": reachable_blob_id,
                    "size_bytes": reachable_bytes.len(),
                    "mode": "100644",
                    "sha256": reachable_sha256,
                }],
                }],
            }),
        )
        .expect("child snapshot should commit");
    service.reset_test_zstd_pack_payload_read_count();
    let bulk = service
        .get_zstd_pull_manifest(
            "repo-bin",
            json!({
                "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                "head_snapshot_id": "SNP-0000000000C1",
                "have_snapshot_ids": [],
            }),
        )
        .expect("bulk pull manifest should build");
    assert_eq!(
        bulk["snapshots"]
            .as_array()
            .expect("bulk snapshots")
            .iter()
            .map(|row| row["snapshot_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["SNP-0000000000C0", "SNP-0000000000C1"]
    );
    assert_eq!(
        bulk["object_packs"].as_array().unwrap().len(),
        2,
        "shared object-pack closure must be emitted once"
    );
    assert_eq!(
        bulk["tree_packs"].as_array().unwrap().len(),
        3,
        "shared tree-pack closure must be emitted once"
    );
    let bounded = service
        .get_zstd_pull_manifest(
            "repo-bin",
            json!({
                "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                "head_snapshot_id": "SNP-0000000000C1",
                "have_snapshot_ids": ["SNP-0000000000C0"],
            }),
        )
        .expect("bounded bulk pull manifest should build");
    assert_eq!(
        bounded["boundary_snapshot_ids"],
        json!(["SNP-0000000000C0"])
    );
    assert_eq!(
        bounded["snapshots"][0]["snapshot_id"],
        json!("SNP-0000000000C1")
    );
    assert_eq!(
        service.test_zstd_pack_payload_read_count(),
        0,
        "pull manifests must derive committed pack metadata from bounded zstd index reads"
    );

    fs::remove_dir_all(fixture_root).expect("manifest fixture should remove");
}

#[test]
fn binary_zstd_pull_catalog_ignores_unreachable_physical_only_tree_pack() {
    let db = native_line_binary_db("zstd-physical-only-tree-pack");
    let authority_db = db.clone();
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);
    let content = seed_native_binary_content(&service, "zstd-physical-only-tree-pack");
    let created_at = "2026-08-21T00:00:00Z";
    let physical_only_pack_id = "TPK-000000000000";
    let fixture_root = env::temp_dir().join(format!(
        "ait-server-core-zstd-physical-only-tree-pack-{}",
        std::process::id()
    ));
    if fixture_root.exists() {
        fs::remove_dir_all(&fixture_root).expect("stale physical-only fixture should remove");
    }
    fs::create_dir_all(&fixture_root).expect("physical-only fixture root should create");

    let tree_rows = json!([{
        "tree_id": "TRE-00000000000000000000",
        "entry_count": 0,
    }]);
    let members = build_tree_pack_members(&tree_rows, &json!([]))
        .expect("physical-only Tree member should build");
    let pack_path = fixture_root.join(format!("{physical_only_pack_id}.zstpack"));
    let mut metadata = write_tree_pack_archive_with_format(
        pack_path
            .to_str()
            .expect("physical-only pack path should be UTF-8"),
        physical_only_pack_id,
        created_at,
        &members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("physical-only Tree archive should write");
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                physical_only_pack_id.to_string(),
                fs::read(&pack_path).expect("physical-only Tree archive should read"),
            )],
            true,
        )
        .expect("physical-only Tree archive should upload");

    let metadata_object = metadata
        .as_object_mut()
        .expect("Tree pack metadata should be an object");
    metadata_object.insert("pack_id".to_string(), json!(physical_only_pack_id));
    metadata_object.insert("created_at".to_string(), json!(created_at));
    metadata_object.insert("tree_count".to_string(), json!(0));
    let pack_index = metadata_object
        .get_mut("pack_index")
        .and_then(JsonValue::as_object_mut)
        .expect("Tree pack metadata should contain an index");
    pack_index.insert("tree_count".to_string(), json!(0));
    pack_index.insert("trees".to_string(), json!([]));

    let store = ServerBinaryRepositoryContentStore::new(authority_db.clone());
    let mut tx = BinaryDbWriteTxn::begin_serving(
        &authority_db,
        BinaryDbCommandScope::ServerRemoteSyncCommit,
    )
    .expect("physical-only Tree pack transaction should begin");
    store
        .append_tree_pack_in_tx(&mut tx, &metadata, &[])
        .expect("physical-only Tree pack record should append");
    tx.commit()
        .expect("physical-only Tree pack record should commit");
    let physical_only = store
        .tree_pack(physical_only_pack_id)
        .expect("physical-only Tree pack should read")
        .expect("physical-only Tree pack should exist");
    assert_eq!(physical_only.record.tree_count, 0);

    let invalid_root = service
        .commit_zstd_bulk(
            "repo-bin",
            json!({
                "contract": "ait.remote_sync.zstd_bulk.commit.v1",
                "object_packs": [],
                "tree_packs": [],
                "blob_locators": [],
                "tree_locators": [],
                "snapshots": [{
                    "snapshot_id": "SNP-0000000000E0",
                    "repo_name": "repo-bin",
                    "line_name": "main",
                    "message": "invalid physical-only root",
                    "root_tree_pack_id": physical_only_pack_id,
                    "root_entry_ordinal": 0,
                    "file_count": 0,
                    "total_bytes": 0,
                    "created_at": created_at,
                    "files": [],
                }],
            }),
        )
        .expect_err("a zero-logical-Tree pack must remain invalid as a Snapshot root");
    assert!(
        invalid_root
            .message
            .contains("has no physical entry ordinal 0"),
        "unexpected zero-logical-Tree root error: {invalid_root:?}"
    );

    service
        .commit_zstd_bulk(
            "repo-bin",
            json!({
                "contract": "ait.remote_sync.zstd_bulk.commit.v1",
                "object_packs": [],
                "tree_packs": [],
                "blob_locators": [],
                "tree_locators": [],
                "snapshots": [{
                    "snapshot_id": "SNP-0000000000E1",
                    "repo_name": "repo-bin",
                    "line_name": "main",
                    "message": "valid root beside physical-only pack",
                    "root_tree_pack_id": content.tree_pack_id,
                    "root_entry_ordinal": 0,
                    "file_count": 1,
                    "total_bytes": content.bytes.len(),
                    "created_at": created_at,
                    "files": [{
                        "path": "README.md",
                        "blob_id": content.blob_id,
                        "size_bytes": content.bytes.len(),
                        "mode": "100644",
                        "sha256": sha256_hex(&content.bytes),
                    }],
                }],
            }),
        )
        .expect("valid Snapshot beside physical-only Tree pack should commit");

    service.reset_test_import_manifest_read_counts();
    let manifest = service
        .get_zstd_pull_manifest(
            "repo-bin",
            json!({
                "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                "head_snapshot_id": "SNP-0000000000E1",
                "have_snapshot_ids": [],
            }),
        )
        .expect("unrelated physical-only Tree pack must not block a valid pull catalog");
    assert_eq!(
        manifest["tree_packs"]
            .as_array()
            .expect("pull manifest Tree packs should be an array")
            .iter()
            .map(|row| row["pack_id"].as_str().expect("Tree pack ID"))
            .collect::<Vec<_>>(),
        vec![content.tree_pack_id.as_str()]
    );
    assert_eq!(
        service.test_import_manifest_zstd_file_read_counts(),
        (1, 1, 0),
        "catalog construction must not open the physical-only Tree archive"
    );

    fs::remove_dir_all(fixture_root).expect("physical-only fixture should remove");
}

#[test]
fn binary_zstd_pull_manifest_reads_snapshot_authority_once_for_a_long_chain() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-zstd-pull-snapshot-catalog-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale pull catalog root should remove");
    }
    fs::create_dir_all(&root).expect("pull catalog root should create");
    for path in SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
    {
        fs::write(root.join(path), SERVER_BINARY_DB_LAYOUT_ID.to_le_bytes())
            .expect("pull catalog Binary DB file should initialize");
    }
    let files = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
    let db = FilesystemServerRemoteBinaryDb::with_file_store(
        files,
        RepoId::new("REPO-ZSTD-PULL-CATALOG"),
        RepoName::new("repo-bin"),
        StorePath::from(root.clone()),
        StoreGeneration::new(1),
        ServerBinaryDbAuthorityMode::TestFixture,
    );
    let service = BinaryDbNativeRepositoryService::new(db.clone());
    create_native_binary_repository(&service);
    let content = seed_native_binary_content(&service, "zstd-pull-snapshot-catalog");
    let created_at = "2026-08-14T00:00:00Z";
    let first_snapshot_id = "SNP-000000000001";
    service
        .commit_zstd_bulk(
            "repo-bin",
            json!({
                "contract": "ait.remote_sync.zstd_bulk.commit.v1",
                "object_packs": [],
                "tree_packs": [],
                "blob_locators": [],
                "tree_locators": [],
                "snapshots": [{
                    "snapshot_id": first_snapshot_id,
                    "repo_name": "repo-bin",
                    "line_name": "main",
                    "message": "catalog chain root",
                    "root_tree_pack_id": content.tree_pack_id,
                    "root_entry_ordinal": 0,
                    "file_count": 1,
                    "total_bytes": content.bytes.len(),
                    "created_at": created_at,
                    "files": [{
                        "path": "README.md",
                        "blob_id": content.blob_id,
                        "size_bytes": content.bytes.len(),
                        "mode": "100644",
                        "sha256": sha256_hex(&content.bytes),
                    }],
                }],
            }),
        )
        .expect("pull catalog root snapshot should commit");

    let snapshots =
        ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
    let read = BinaryDbReadTxn::new_bounded_for_scope(&db, BinaryDbReadScope::CONTENT);
    let (_, template) = snapshots
        .snapshot_by_id(&read, first_snapshot_id)
        .expect("root snapshot lookup should succeed")
        .expect("root snapshot should exist");
    drop(read);
    let mut previous_index = 0_u32;
    let mut head_snapshot_id = first_snapshot_id.to_string();
    for ordinal in 2_u32..=96 {
        head_snapshot_id = format!("SNP-{ordinal:012X}");
        let mut record = template.clone();
        record.snapshot_hash48 = server_snapshot_hash48_from_id(&head_snapshot_id)
            .expect("chain snapshot ID should be canonical");
        record.parent_snapshot_index_plus1 = previous_index + 1;
        record.created_at_s += u64::from(ordinal);
        previous_index = snapshots
            .append_snapshot(
                &head_snapshot_id,
                record,
                &ServerBinarySnapshotPayload {
                    line_name: "main".to_string(),
                    message: Some(format!("catalog chain {ordinal}")),
                },
            )
            .expect("chain snapshot should append");
    }

    let event_offset = db.file_store().events().len();
    let manifest = service
        .get_zstd_pull_manifest(
            "repo-bin",
            json!({
                "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                "head_snapshot_id": head_snapshot_id,
                "have_snapshot_ids": [],
            }),
        )
        .expect("long-chain pull manifest should build");
    let snapshot_rows = manifest["snapshots"]
        .as_array()
        .expect("pull manifest snapshots should be an array");
    assert_eq!(snapshot_rows.len(), 96);
    assert_eq!(snapshot_rows[0]["snapshot_id"], first_snapshot_id);
    assert_eq!(snapshot_rows[95]["snapshot_id"], head_snapshot_id);

    let events = db.file_store().events();
    let events = &events[event_offset..];
    let operation_count = |operation, file_name: &str| {
        events
            .iter()
            .filter(|event| {
                event.operation == operation
                    && event.timing == BinaryDbTestFaultTiming::Before
                    && event.path.file_name().and_then(|name| name.to_str()) == Some(file_name)
            })
            .count()
    };
    assert_eq!(
        operation_count(
            BinaryDbTestStorageOperation::ReadBytes,
            SERVER_SNAPSHOT_ID_IDX
        ),
        0,
        "pull ancestry must not perform one full Snapshot ID index read per Snapshot"
    );
    let maximum_linear_reads = snapshot_rows.len() * 2 + 12;
    let snapshot_reads =
        operation_count(BinaryDbTestStorageOperation::ReadRange, SERVER_SNAPSHOT_BIN);
    let parent_edge_reads = operation_count(
        BinaryDbTestStorageOperation::ReadRange,
        SERVER_SNAPSHOT_PARENT_EDGE_BIN,
    );
    assert!(
        snapshot_reads <= maximum_linear_reads,
        "Snapshot fixed-authority reads must remain linear, got {snapshot_reads} for {} Snapshots",
        snapshot_rows.len()
    );
    assert!(
        parent_edge_reads <= maximum_linear_reads,
        "Snapshot parent-edge reads must remain linear, got {parent_edge_reads} for {} Snapshots",
        snapshot_rows.len()
    );
    for file_name in ["tree_pack.bin", "tree.bin"] {
        let reads = operation_count(BinaryDbTestStorageOperation::ReadRange, file_name);
        assert!(
            reads <= 8,
            "pull manifest must project {file_name} once per request, got {reads} fixed-authority reads for {} Snapshots",
            snapshot_rows.len()
        );
    }

    assert_eq!(service.zstd_pull_catalog_build_count_for_test(), 1);
    let cache_hit_event_offset = db.file_store().events().len();
    let cached_manifest = service
        .get_zstd_pull_manifest(
            "repo-bin",
            json!({
                "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                "head_snapshot_id": head_snapshot_id,
                "have_snapshot_ids": [],
            }),
        )
        .expect("cached long-chain pull manifest should build");
    assert_eq!(
        cached_manifest, manifest,
        "cache hits must preserve exact JSON"
    );
    assert_eq!(
        serde_json::to_vec(&cached_manifest).expect("cached manifest should encode"),
        serde_json::to_vec(&manifest).expect("initial manifest should encode"),
        "cache hits must preserve exact manifest bytes and ordering"
    );
    assert_eq!(service.zstd_pull_catalog_build_count_for_test(), 1);
    let cache_hit_events = db.file_store().events();
    assert!(
        cache_hit_events[cache_hit_event_offset..]
            .iter()
            .all(|event| event.operation != BinaryDbTestStorageOperation::ReadRange),
        "cache hits must not rescan fixed or payload authority ranges"
    );

    service
        .invalidate_zstd_pull_catalog()
        .expect("test should invalidate the shared pull catalog");
    let reader_count = 8;
    let barrier = Arc::new(Barrier::new(reader_count));
    let readers = (0..reader_count)
        .map(|_| {
            let service = service.clone();
            let barrier = barrier.clone();
            let head_snapshot_id = head_snapshot_id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                service.get_zstd_pull_manifest(
                    "repo-bin",
                    json!({
                        "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                        "head_snapshot_id": head_snapshot_id,
                        "have_snapshot_ids": [],
                    }),
                )
            })
        })
        .collect::<Vec<_>>();
    for reader in readers {
        assert_eq!(
            reader
                .join()
                .expect("concurrent pull reader should not panic")
                .expect("concurrent pull reader should succeed"),
            manifest,
            "concurrent readers must share the same immutable catalog"
        );
    }
    assert_eq!(
        service.zstd_pull_catalog_build_count_for_test(),
        2,
        "a concurrent cold miss must build exactly one shared catalog"
    );

    let mutated_head_snapshot_id = "SNP-000000000061";
    service
        .commit_zstd_bulk(
            "repo-bin",
            json!({
                "contract": "ait.remote_sync.zstd_bulk.commit.v1",
                "object_packs": [],
                "tree_packs": [],
                "blob_locators": [],
                "tree_locators": [],
                "snapshots": [{
                    "snapshot_id": mutated_head_snapshot_id,
                    "repo_name": "repo-bin",
                    "line_name": "main",
                    "parent_snapshot_id": head_snapshot_id,
                    "message": "catalog mutation",
                    "root_tree_pack_id": content.tree_pack_id,
                    "root_entry_ordinal": 0,
                    "file_count": 1,
                    "total_bytes": content.bytes.len(),
                    "created_at": "2026-08-14T00:10:00Z",
                    "files": [{
                        "path": "README.md",
                        "blob_id": content.blob_id,
                        "size_bytes": content.bytes.len(),
                        "mode": "100644",
                        "sha256": sha256_hex(&content.bytes),
                    }],
                }],
            }),
        )
        .expect("content mutation should commit and invalidate the pull catalog");
    let mutated_manifest = service
        .get_zstd_pull_manifest(
            "repo-bin",
            json!({
                "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                "head_snapshot_id": mutated_head_snapshot_id,
                "have_snapshot_ids": [],
            }),
        )
        .expect("pull after mutation should rebuild the catalog");
    assert_eq!(mutated_manifest["snapshots"].as_array().unwrap().len(), 97);
    assert_eq!(
        mutated_manifest["snapshots"][96]["snapshot_id"],
        mutated_head_snapshot_id
    );
    assert_eq!(
        service.zstd_pull_catalog_build_count_for_test(),
        3,
        "the first pull after an admitted mutation must build the new revision"
    );

    let tree_index_path = root.join("tree_id.idx");
    let tree_index_bytes = fs::read(&tree_index_path).expect("Tree index should read");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&tree_index_path)
        .expect("Tree index should open for corruption injection")
        .write_all(&[0xff])
        .expect("Tree index corruption byte should append");
    let corrupt = service
        .get_zstd_pull_manifest(
            "repo-bin",
            json!({
                "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                "head_snapshot_id": mutated_head_snapshot_id,
                "have_snapshot_ids": [],
            }),
        )
        .expect_err("a changed corrupt authority revision must fail closed");
    assert_eq!(corrupt.kind, NativeRepositoryErrorKind::Internal);
    fs::write(&tree_index_path, tree_index_bytes).expect("Tree index should restore exactly");
    assert_eq!(
        service
            .get_zstd_pull_manifest(
                "repo-bin",
                json!({
                    "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
                    "head_snapshot_id": mutated_head_snapshot_id,
                    "have_snapshot_ids": [],
                }),
            )
            .expect("exact restored authority should reuse the valid catalog"),
        mutated_manifest
    );

    fs::remove_dir_all(root).expect("pull catalog fixture should remove");
}

#[test]
fn binary_content_pack_relocation_reuses_identical_ids_and_rejects_tree_collision() {
    let db = native_line_binary_db("content-relocation");
    let authority_db = db.clone();
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);
    let content = seed_native_binary_content(&service, "content-relocation");
    let created_at = "2026-01-02T00:00:00Z";
    let root = env::temp_dir().join(format!(
        "ait-server-core-content-relocation-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale relocation root should remove");
    }
    fs::create_dir_all(&root).expect("relocation root should create");

    let relocated_object_pack_id = "PCK-000000000002";
    let object_pack_path = root.join(format!("{relocated_object_pack_id}.zstpack"));
    write_rebuilt_zstd_pack_archive(
        object_pack_path
            .to_str()
            .expect("relocated object pack path should be UTF-8"),
        relocated_object_pack_id,
        created_at,
        vec![ObjectPackRewriteBlob {
            entry_name: format!("blobs/{}", content.blob_id),
            blob_id: content.blob_id.clone(),
            data: content.bytes.clone(),
            path_hint: Some("README.md".to_string()),
        }],
        0,
    )
    .expect("relocated object pack should write");
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                relocated_object_pack_id.to_string(),
                fs::read(&object_pack_path).expect("relocated object pack should read"),
            )],
            false,
        )
        .expect("relocated object pack bytes should import");
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "blob_id": content.blob_id,
                "sha256": sha256_hex(&content.bytes),
                "size_bytes": content.bytes.len(),
                "pack_id": relocated_object_pack_id,
                "pack_entry_name": format!("blobs/{}", content.blob_id),
                "pack_entry_type": "full",
                "pack_base_blob_id": JsonValue::Null,
                "pack_chain_depth": 0,
                "created_at": created_at,
            })],
            false,
        )
        .expect("identical blob should relocate to a new object pack");

    let relocated_tree_pack_id = "TPK-000000000002";
    let tree_rows = json!([{
        "tree_id": content.tree_id,
        "entry_count": 1,
    }]);
    let tree_entry_rows = json!([{
        "tree_id": content.tree_id,
        "entry_name": "README.md",
        "entry_type": "blob",
        "target_id": content.blob_id,
        "size_bytes": content.bytes.len(),
        "mode": "0o100644",
    }]);
    let tree_members = build_tree_pack_members(&tree_rows, &tree_entry_rows)
        .expect("relocated tree members should build");
    let tree_pack_path = root.join(format!("{relocated_tree_pack_id}.zstpack"));
    let tree_pack_metadata = write_tree_pack_archive_with_format(
        tree_pack_path
            .to_str()
            .expect("relocated tree pack path should be UTF-8"),
        relocated_tree_pack_id,
        created_at,
        &tree_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("relocated tree pack should write");
    let tree_checksum = tree_pack_metadata["pack_index"]["trees"][0]["checksum"]
        .as_str()
        .expect("relocated tree checksum should exist")
        .to_string();
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                relocated_tree_pack_id.to_string(),
                fs::read(&tree_pack_path).expect("relocated tree pack should read"),
            )],
            true,
        )
        .expect("relocated tree pack bytes should import");
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "tree_id": content.tree_id,
                "entry_count": 1,
                "tree_pack_id": relocated_tree_pack_id,
                "tree_pack_checksum": tree_checksum,
                "created_at": created_at,
            })],
            true,
        )
        .expect("identical tree should relocate to a new tree pack");

    let repository_content = ServerBinaryRepositoryContentStore::new(authority_db.clone());
    assert_eq!(
        repository_content
            .blob(&content.blob_id)
            .expect("relocated blob should read")
            .expect("relocated blob should exist")
            .pack_id,
        content.object_pack_id
    );
    assert!(repository_content
        .object_pack(relocated_object_pack_id)
        .expect("relocated object pack metadata should read")
        .is_some());
    assert_eq!(
        repository_content
            .tree(&content.tree_id)
            .expect("relocated tree should read")
            .expect("relocated tree should exist")
            .pack_id,
        content.tree_pack_id
    );
    assert!(repository_content
        .tree_pack(relocated_tree_pack_id)
        .expect("relocated tree pack metadata should read")
        .is_some());

    {
        let read =
            BinaryDbReadTxn::new_bounded_for_scope(&authority_db, BinaryDbReadScope::CONTENT);
        let manifest_cache = repository_content
            .manifest_tree_read_cache_with_read(&read)
            .expect("manifest bulk projection must admit repeated active content identities");
        let canonical_blob = manifest_cache
            .projected_blob(&content.blob_id)
            .expect("projected Blob lookup should read")
            .expect("canonical projected Blob should exist");
        assert_eq!(canonical_blob.pack_id, content.object_pack_id);
        let relocated_object_pack = manifest_cache
            .projected_object_pack(relocated_object_pack_id)
            .expect("relocated projected Object Pack lookup should read")
            .expect("relocated projected Object Pack should exist");
        let relocated_blobs = manifest_cache
            .projected_blobs_for_object_pack(&relocated_object_pack)
            .expect("relocated physical Blob member should remain projected");
        assert_eq!(relocated_blobs.len(), 1);
        assert_eq!(relocated_blobs[0].blob_id, content.blob_id);
        assert!(relocated_blobs[0].blob_index > canonical_blob.blob_index);

        let canonical_tree = manifest_cache
            .projected_tree(&content.tree_id)
            .expect("projected Tree lookup should read")
            .expect("canonical projected Tree should exist");
        assert_eq!(canonical_tree.pack_id, content.tree_pack_id);
        let relocated_tree_pack = manifest_cache
            .projected_tree_pack(relocated_tree_pack_id)
            .expect("relocated projected Tree Pack lookup should read")
            .expect("relocated projected Tree Pack should exist");
        let relocated_trees = manifest_cache
            .projected_trees_for_tree_pack(&relocated_tree_pack)
            .expect("relocated physical Tree member should remain projected");
        assert_eq!(relocated_trees.len(), 1);
        assert_eq!(relocated_trees[0].tree_id, content.tree_id);
        assert!(relocated_trees[0].tree_index > canonical_tree.tree_index);

        repository_content
            .validate_manifest_identity_indexes_with_read(
                &read,
                &manifest_cache,
                &BTreeSet::from([
                    content.object_pack_id.to_ascii_uppercase(),
                    relocated_object_pack_id.to_ascii_uppercase(),
                ]),
                &BTreeSet::from([
                    content.tree_pack_id.to_ascii_uppercase(),
                    relocated_tree_pack_id.to_ascii_uppercase(),
                ]),
                &BTreeSet::from([content.blob_id.to_ascii_uppercase()]),
                &BTreeSet::from([content.tree_id.to_ascii_uppercase()]),
            )
            .expect("selected identity indexes must retain canonical and relocated authority");
    }

    let collision_pack_id = "TPK-000000000003";
    let collision_rows = json!([{
        "tree_id": content.tree_id,
        "entry_name": "DIFFERENT.md",
        "entry_type": "blob",
        "target_id": content.blob_id,
        "size_bytes": content.bytes.len(),
        "mode": "100644",
    }]);
    let collision_members = build_tree_pack_members(&tree_rows, &collision_rows)
        .expect("collision tree members should build");
    let collision_path = root.join(format!("{collision_pack_id}.zstpack"));
    let collision_metadata = write_tree_pack_archive_with_format(
        collision_path
            .to_str()
            .expect("collision tree pack path should be UTF-8"),
        collision_pack_id,
        created_at,
        &collision_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("collision tree pack should write");
    let collision_checksum = collision_metadata["pack_index"]["trees"][0]["checksum"]
        .as_str()
        .expect("collision tree checksum should exist")
        .to_string();
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                collision_pack_id.to_string(),
                fs::read(&collision_path).expect("collision tree pack should read"),
            )],
            true,
        )
        .expect("collision pack bytes may stage before metadata validation");
    let collision = service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "tree_id": content.tree_id,
                "entry_count": 1,
                "tree_pack_id": collision_pack_id,
                "tree_pack_checksum": collision_checksum,
                "created_at": created_at,
            })],
            true,
        )
        .expect_err("different content under one tree id must be rejected");
    assert_eq!(collision.kind, NativeRepositoryErrorKind::BadRequest);
    assert!(collision.message.contains("different content"));

    fs::remove_dir_all(root).expect("relocation root should remove");
}

#[test]
fn binary_content_tree_pack_derives_unselected_physical_member_from_pack_index() {
    let db = native_line_binary_db("overlapping-tree-pack");
    let authority_db = db.clone();
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);
    let existing = seed_native_binary_content(&service, "overlapping-tree-pack");
    let created_at = "2026-01-02T00:00:00Z";
    let root = env::temp_dir().join(format!(
        "ait-server-core-overlapping-tree-pack-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale overlapping tree-pack root should remove");
    }
    fs::create_dir_all(&root).expect("overlapping tree-pack root should create");

    let new_tree_id = "TRE-000000000000000000D0";
    let overlapping_pack_id = "TPK-0000000000D0";
    let tree_rows = json!([
        {
            "tree_id": existing.tree_id,
            "entry_count": 1,
        },
        {
            "tree_id": new_tree_id,
            "entry_count": 1,
        },
    ]);
    let tree_entry_rows = json!([
        {
            "tree_id": existing.tree_id,
            "entry_name": "README.md",
            "entry_type": "blob",
            "target_id": existing.blob_id,
            "size_bytes": existing.bytes.len(),
            "mode": "100644",
        },
        {
            "tree_id": new_tree_id,
            "entry_name": "NEW.md",
            "entry_type": "blob",
            "target_id": existing.blob_id,
            "size_bytes": existing.bytes.len(),
            "mode": "100644",
        },
    ]);
    let members = build_tree_pack_members(&tree_rows, &tree_entry_rows)
        .expect("overlapping tree-pack members should build");
    let pack_path = root.join(format!("{overlapping_pack_id}.zstpack"));
    let pack_metadata = write_tree_pack_archive_with_format(
        pack_path
            .to_str()
            .expect("overlapping tree-pack path should be UTF-8"),
        overlapping_pack_id,
        created_at,
        &members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("overlapping tree pack should write");
    let new_tree_checksum = pack_metadata["pack_index"]["trees"][1]["checksum"]
        .as_str()
        .expect("new Tree checksum should exist")
        .to_string();
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                overlapping_pack_id.to_string(),
                fs::read(&pack_path).expect("overlapping tree pack should read"),
            )],
            true,
        )
        .expect("overlapping tree-pack bytes should stage");
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "tree_id": new_tree_id,
                "entry_count": 1,
                "tree_pack_id": overlapping_pack_id,
                "tree_pack_checksum": new_tree_checksum,
                "created_at": created_at,
            })],
            true,
        )
        .expect("one canonical locator should admit both verified physical Trees");

    let content = ServerBinaryRepositoryContentStore::new(authority_db.clone());
    let pack = content
        .tree_pack(overlapping_pack_id)
        .expect("overlapping tree-pack metadata should read")
        .expect("overlapping tree pack should be committed");
    assert_eq!(pack.record.tree_count, 2);
    let read = BinaryDbReadTxn::new_bounded_for_scope(&authority_db, BinaryDbReadScope::CONTENT);
    let physical_trees = content
        .trees_for_tree_pack_with_read(&read, overlapping_pack_id)
        .expect("every physical Tree should have a Binary DB record");
    assert_eq!(
        physical_trees
            .iter()
            .map(|tree| (tree.tree_id.as_str(), tree.record.pack_entry_ordinal))
            .collect::<Vec<_>>(),
        vec![(existing.tree_id.as_str(), 0), (new_tree_id, 1)]
    );
    let duplicate_rows = content
        .tree_entries_for_tree_with_read(&read, &physical_trees[0])
        .expect("physical duplicate Tree rows should read");
    assert_eq!(duplicate_rows.len(), 1);
    assert_eq!(duplicate_rows[0].entry_name, "README.md");
    let new_rows = content
        .tree_entries_for_tree_with_read(&read, &physical_trees[1])
        .expect("new physical Tree rows should read");
    assert_eq!(new_rows.len(), 1);
    assert_eq!(new_rows[0].entry_name, "NEW.md");
    drop(read);
    assert_eq!(
        content
            .tree(&existing.tree_id)
            .expect("existing Tree should read")
            .expect("existing Tree should remain admitted")
            .pack_id,
        existing.tree_pack_id
    );
    assert_eq!(
        content
            .tree(new_tree_id)
            .expect("new Tree should read")
            .expect("new Tree should be admitted")
            .pack_id,
        overlapping_pack_id
    );

    let rejected_pack_id = "TPK-0000000000D1";
    let physical_tree_id = "TRE-000000000000000000D1";
    let absent_tree_id = "TRE-000000000000000000D2";
    let rejected_tree_rows = json!([{
        "tree_id": physical_tree_id,
        "entry_count": 1,
    }]);
    let rejected_entry_rows = json!([{
        "tree_id": physical_tree_id,
        "entry_name": "PHYSICAL.md",
        "entry_type": "blob",
        "target_id": existing.blob_id,
        "size_bytes": existing.bytes.len(),
        "mode": "100644",
    }]);
    let rejected_members = build_tree_pack_members(&rejected_tree_rows, &rejected_entry_rows)
        .expect("rejected tree-pack members should build");
    let rejected_path = root.join(format!("{rejected_pack_id}.zstpack"));
    let rejected_metadata = write_tree_pack_archive_with_format(
        rejected_path
            .to_str()
            .expect("rejected tree-pack path should be UTF-8"),
        rejected_pack_id,
        created_at,
        &rejected_members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("rejected tree pack should write");
    let physical_checksum = rejected_metadata["pack_index"]["trees"][0]["checksum"]
        .as_str()
        .expect("physical Tree checksum should exist")
        .to_string();
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                rejected_pack_id.to_string(),
                fs::read(&rejected_path).expect("rejected tree pack should read"),
            )],
            true,
        )
        .expect("rejected tree-pack bytes may stage");
    let error = service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "tree_id": absent_tree_id,
                "entry_count": 1,
                "tree_pack_id": rejected_pack_id,
                "tree_pack_checksum": physical_checksum,
                "created_at": created_at,
            })],
            true,
        )
        .expect_err("locator absent from the physical Tree pack must fail closed");
    assert!(error
        .message
        .contains("locators for trees absent from its physical index"));
    assert!(content
        .tree_pack(rejected_pack_id)
        .expect("rejected tree-pack metadata lookup should read")
        .is_none());

    fs::remove_dir_all(root).expect("overlapping tree-pack root should remove");
}

#[test]
fn binary_content_object_pack_derives_unselected_physical_member_from_pack_index() {
    let db = native_line_binary_db("overlapping-object-pack");
    let authority_db = db.clone();
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);
    let existing = seed_native_binary_content(&service, "overlapping-object-pack");
    let created_at = "2026-01-02T00:00:00Z";
    let root = env::temp_dir().join(format!(
        "ait-server-core-overlapping-object-pack-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale overlapping pack root should remove");
    }
    fs::create_dir_all(&root).expect("overlapping pack root should create");

    let new_bytes = b"new blob beside a physical duplicate\n".to_vec();
    let new_sha256 = sha256_hex(&new_bytes);
    let new_blob_id = format!("BLB-{}", &new_sha256[..20]);
    let overlapping_pack_id = "PCK-0000000000D0";
    let overlapping_pack_path = root.join(format!("{overlapping_pack_id}.zstpack"));
    write_rebuilt_zstd_pack_archive(
        overlapping_pack_path
            .to_str()
            .expect("overlapping pack path should be UTF-8"),
        overlapping_pack_id,
        created_at,
        vec![
            ObjectPackRewriteBlob {
                entry_name: format!("blobs/{}", existing.blob_id),
                blob_id: existing.blob_id.clone(),
                data: existing.bytes.clone(),
                path_hint: Some("existing.txt".to_string()),
            },
            ObjectPackRewriteBlob {
                entry_name: format!("blobs/{new_blob_id}"),
                blob_id: new_blob_id.clone(),
                data: new_bytes.clone(),
                path_hint: Some("new.txt".to_string()),
            },
        ],
        0,
    )
    .expect("overlapping object pack should write");
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                overlapping_pack_id.to_string(),
                fs::read(&overlapping_pack_path).expect("overlapping pack should read"),
            )],
            false,
        )
        .expect("overlapping object pack bytes should stage");
    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "blob_id": new_blob_id,
                "sha256": new_sha256,
                "size_bytes": new_bytes.len(),
                "pack_id": overlapping_pack_id,
                "pack_entry_name": format!("blobs/{new_blob_id}"),
                "pack_entry_type": "full",
                "pack_base_blob_id": JsonValue::Null,
                "pack_chain_depth": 0,
                "created_at": created_at,
            })],
            false,
        )
        .expect("one canonical locator should admit both verified physical members");

    let content = ServerBinaryRepositoryContentStore::new(authority_db.clone());
    let pack = content
        .object_pack(overlapping_pack_id)
        .expect("overlapping pack metadata should read")
        .expect("overlapping pack should be committed");
    assert_eq!(pack.record.member_count, 2);
    let read = BinaryDbReadTxn::new_bounded_for_scope(&authority_db, BinaryDbReadScope::CONTENT);
    let physical_blob_ids = content
        .blobs_for_object_pack_with_read(&read, overlapping_pack_id)
        .expect("every physical member should have a Binary DB record")
        .into_iter()
        .map(|blob| blob.blob_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        physical_blob_ids,
        BTreeSet::from([existing.blob_id.clone(), new_blob_id.clone()])
    );
    drop(read);
    assert_eq!(
        content
            .blob(&existing.blob_id)
            .expect("existing blob should read")
            .expect("existing blob should remain admitted")
            .pack_id,
        existing.object_pack_id
    );
    assert_eq!(
        content
            .blob(&new_blob_id)
            .expect("new blob should read")
            .expect("new blob should be admitted")
            .pack_id,
        overlapping_pack_id
    );

    let rejected_pack_id = "PCK-0000000000D1";
    let rejected_bytes = b"physical member for locator mismatch\n".to_vec();
    let rejected_sha256 = sha256_hex(&rejected_bytes);
    let rejected_blob_id = format!("BLB-{}", &rejected_sha256[..20]);
    let rejected_pack_path = root.join(format!("{rejected_pack_id}.zstpack"));
    write_rebuilt_zstd_pack_archive(
        rejected_pack_path
            .to_str()
            .expect("rejected pack path should be UTF-8"),
        rejected_pack_id,
        created_at,
        vec![ObjectPackRewriteBlob {
            entry_name: format!("blobs/{rejected_blob_id}"),
            blob_id: rejected_blob_id,
            data: rejected_bytes,
            path_hint: Some("physical.txt".to_string()),
        }],
        0,
    )
    .expect("rejected object pack should write");
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                rejected_pack_id.to_string(),
                fs::read(&rejected_pack_path).expect("rejected pack should read"),
            )],
            false,
        )
        .expect("rejected object pack bytes may stage");
    let absent_bytes = b"locator absent from physical pack\n".to_vec();
    let absent_sha256 = sha256_hex(&absent_bytes);
    let absent_blob_id = format!("BLB-{}", &absent_sha256[..20]);
    let error = service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "blob_id": absent_blob_id,
                "sha256": absent_sha256,
                "size_bytes": absent_bytes.len(),
                "pack_id": rejected_pack_id,
                "pack_entry_name": format!("blobs/{absent_blob_id}"),
                "pack_entry_type": "full",
                "pack_base_blob_id": JsonValue::Null,
                "pack_chain_depth": 0,
                "created_at": created_at,
            })],
            false,
        )
        .expect_err("locator absent from the physical pack must fail closed");
    assert!(error
        .message
        .contains("locators for blobs absent from its physical index"));
    assert!(content
        .object_pack(rejected_pack_id)
        .expect("rejected pack metadata lookup should read")
        .is_none());

    fs::remove_dir_all(root).expect("overlapping pack root should remove");
}

#[test]
fn binary_content_readers_accept_historical_chain_beyond_writer_depth() {
    use crate::foundation::pack_substrate::{
        build_git_binary_delta_member, write_pack_archive_with_format,
        DEFAULT_MAX_DELTA_CHAIN_DEPTH, MAX_DELTA_CHAIN_READ_DEPTH, PACK_FORMAT_ZSTD_CHUNKED_V1,
    };

    let db = native_line_binary_db("historical-delta-read-depth");
    let authority_db = db.clone();
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);

    let historical_depth = DEFAULT_MAX_DELTA_CHAIN_DEPTH + 1;
    assert!(historical_depth < MAX_DELTA_CHAIN_READ_DEPTH);
    let versions = (0..=historical_depth)
        .map(|version| {
            let mut bytes = vec![b'a'; 4096];
            bytes[version] = b'A' + u8::try_from(version).expect("version byte");
            bytes
        })
        .collect::<Vec<_>>();
    let blob_ids = versions
        .iter()
        .map(|bytes| {
            let sha256 = sha256_hex(bytes);
            format!("BLB-{}", &sha256[..20])
        })
        .collect::<Vec<_>>();
    let source_root = env::temp_dir().join(format!(
        "ait-server-core-historical-delta-read-depth-{}",
        std::process::id()
    ));
    if source_root.exists() {
        fs::remove_dir_all(&source_root).expect("stale historical pack root should remove");
    }
    fs::create_dir_all(&source_root).expect("historical pack root should create");

    for depth in 0..=historical_depth {
        let pack_id = format!("PCK-{:012X}", 0x100_u64 + depth as u64);
        let pack_path = source_root.join(format!("{pack_id}.zstpack"));
        let members = if depth == 0 {
            json!([{
                "entry_name": format!("blobs/{}", blob_ids[depth]),
                "blob_id": blob_ids[depth],
                "data": versions[depth].as_slice(),
            }])
        } else {
            json!([build_git_binary_delta_member(
                &format!("blobs/{}", blob_ids[depth]),
                &blob_ids[depth],
                &blob_ids[depth - 1],
                &versions[depth - 1],
                &versions[depth],
                depth,
            )])
        };
        write_pack_archive_with_format(
            pack_path
                .to_str()
                .expect("historical object pack path should be UTF-8"),
            &pack_id,
            "2026-01-01T00:00:00Z",
            &members,
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .expect("historical object pack should write");
        service
            .seed_zstd_pack_batch_for_test(
                "repo-bin",
                vec![(
                    pack_id.clone(),
                    fs::read(&pack_path).expect("historical object pack should read"),
                )],
                false,
            )
            .expect("historical object pack bytes should import");
        service
            .seed_zstd_locator_batch_for_test(
                "repo-bin",
                vec![json!({
                    "blob_id": blob_ids[depth],
                    "sha256": sha256_hex(&versions[depth]),
                    "size_bytes": versions[depth].len(),
                    "pack_id": pack_id,
                    "pack_entry_name": format!("blobs/{}", blob_ids[depth]),
                    "pack_entry_type": if depth == 0 { "full" } else { "delta" },
                    "pack_base_blob_id": if depth == 0 {
                        JsonValue::Null
                    } else {
                        json!(blob_ids[depth - 1])
                    },
                    "pack_chain_depth": depth,
                    "created_at": "2026-01-01T00:00:00Z",
                })],
                false,
            )
            .expect("historical blob locator should import");
    }

    service.reset_test_zstd_pack_payload_read_count();
    assert_eq!(
        service
            .read_binary_blob_content(&blob_ids[historical_depth])
            .expect("native repository reader should accept historical chain"),
        versions[historical_depth]
    );
    assert_eq!(
        service.test_zstd_pack_payload_read_count(),
        0,
        "blob reads must use indexed pack archives without reparsing full pack metadata"
    );
    let content = ServerBinaryRepositoryContentStore::new(authority_db.clone());
    let read = BinaryDbReadTxn::new_bounded_for_scope(&authority_db, BinaryDbReadScope::CONTENT);
    assert_eq!(
        content
            .blob_bytes_with_read(&read, &blob_ids[historical_depth])
            .expect("server content reader should accept historical chain"),
        Some(versions[historical_depth].clone())
    );

    fs::remove_dir_all(source_root).expect("historical source packs should remove");
}

#[test]
fn binary_tree_pack_reads_normalized_v0_members_with_one_session_cache() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-tree-name-payload-batch-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale tree payload batch root should remove");
    }
    let files = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
    let db = FilesystemServerRemoteBinaryDb::with_file_store(
        files,
        RepoId::new("REPO-TREE-PAYLOAD-BATCH"),
        RepoName::new("repo-bin"),
        StorePath::from(root.clone()),
        StoreGeneration::new(1),
        ServerBinaryDbAuthorityMode::TestFixture,
    );
    let service = BinaryDbNativeRepositoryService::new(db.clone());
    create_native_binary_repository(&service);
    let content = seed_native_binary_content(&service, "tree-name-payload-batch");

    let tree_id = "TRE-00000000000000000002";
    let tree_pack_id = "TPK-0000000000B0";
    let tree_rows = json!([{
        "tree_id": tree_id,
        "entry_count": 3,
    }]);
    let tree_entry_rows = json!([
        {
            "tree_id": tree_id,
            "entry_name": "ALPHA.md",
            "entry_type": "blob",
            "target_id": content.blob_id,
            "size_bytes": content.bytes.len(),
            "mode": "0o600",
        },
        {
            "tree_id": tree_id,
            "entry_name": "BETA.md",
            "entry_type": "blob",
            "target_id": content.blob_id,
            "size_bytes": content.bytes.len(),
            "mode": "100644",
        },
        {
            "tree_id": tree_id,
            "entry_name": "GAMMA.md",
            "entry_type": "blob",
            "target_id": content.blob_id,
            "size_bytes": content.bytes.len(),
            "mode": "100644",
        }
    ]);
    let members = build_tree_pack_members(&tree_rows, &tree_entry_rows)
        .expect("batched tree members should build");
    let source = root.join("batched-tree-pack.zstpack");
    let metadata = write_tree_pack_archive_with_format(
        source
            .to_str()
            .expect("batched tree pack path should be UTF-8"),
        tree_pack_id,
        "2026-01-01T00:00:00Z",
        &members,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .expect("batched tree pack should write");
    let checksum = metadata["pack_index"]["trees"][0]["checksum"]
        .as_str()
        .expect("batched tree checksum should exist")
        .to_string();
    service
        .seed_zstd_pack_batch_for_test(
            "repo-bin",
            vec![(
                tree_pack_id.to_string(),
                fs::read(&source).expect("batched tree pack should read"),
            )],
            true,
        )
        .expect("batched tree pack bytes should import");

    service
        .seed_zstd_locator_batch_for_test(
            "repo-bin",
            vec![json!({
                "tree_id": tree_id,
                "entry_count": 3,
                "tree_pack_id": tree_pack_id,
                "tree_pack_checksum": checksum,
                "created_at": "2026-01-01T00:00:00Z",
            })],
            true,
        )
        .expect("batched tree locator should import");
    for authority in [
        "tree_entry.bin",
        "tree_entry_range.bin",
        "tree_name_payload.bin",
    ] {
        assert!(
            db.file_store()
                .events()
                .iter()
                .any(|event| event.path.ends_with(authority)),
            "normalized v0 import must write {authority}"
        );
        assert!(root.join(authority).exists());
    }

    let repository_content = ServerBinaryRepositoryContentStore::new(db.clone());
    validate_server_tree_authority_v0(&db)
        .expect("strict Tree authority validation should accept committed pack payloads");
    let committed_pack = repository_content.tree_pack_path(tree_pack_id);
    let held_pack = committed_pack.with_extension("zstpack-held");
    fs::rename(&committed_pack, &held_pack)
        .expect("test should make the committed pack temporarily unavailable");
    validate_server_tree_serving_authority_v0(&db).expect(
        "serving admission should validate the committed normalized authority without pack opens",
    );
    let unavailable_read = BinaryDbReadTxn::new_bounded_for_scope(&db, BinaryDbReadScope::CONTENT);
    let unavailable_error = repository_content
        .tree_entries_with_read(&unavailable_read, tree_id)
        .expect_err("an ordinary Tree read must still fail closed when its pack is unavailable");
    assert!(
        unavailable_error.to_string().contains("cannot be opened"),
        "{unavailable_error}"
    );
    let strict_error = validate_server_tree_authority_v0(&db)
        .expect_err("strict validation must still require every committed pack payload");
    assert!(
        strict_error.to_string().contains("cannot be opened"),
        "{strict_error}"
    );
    fs::rename(&held_pack, &committed_pack)
        .expect("test should restore the committed pack after validation");

    let read = BinaryDbReadTxn::new_bounded_for_scope(&db, BinaryDbReadScope::CONTENT);
    let mut tree_read_cache = ServerBinaryTreeReadCache::default();
    let entries = repository_content
        .tree_entries_with_read_cache(&read, tree_id, &mut tree_read_cache)
        .expect("equivalent packed and normalized mode spellings should read")
        .into_iter()
        .map(|entry| (entry.entry_name, entry.mode))
        .collect::<Vec<_>>();
    assert_eq!(
        entries,
        vec![
            ("ALPHA.md".to_string(), "000600".to_string()),
            ("BETA.md".to_string(), "100644".to_string()),
            ("GAMMA.md".to_string(), "100644".to_string()),
        ]
    );
    let cached_chunks = tree_read_cache.cached_zstd_chunk_count();
    assert!(cached_chunks > 0);
    for _ in 0..5 {
        repository_content
            .tree_entries_with_read_cache(&read, tree_id, &mut tree_read_cache)
            .expect("repeated normalized tree read should avoid archive reparsing");
    }
    assert_eq!(tree_read_cache.archive_open_count(), 1);
    assert_eq!(tree_read_cache.cached_zstd_chunk_count(), cached_chunks);

    fs::remove_file(source).expect("batched tree source should remove");
}

#[test]
fn same_textual_pack_id_is_repository_local_and_never_cross_resolved() {
    let root_a = env::temp_dir().join(format!(
        "ait-server-core-pack-repo-local-a-{}",
        std::process::id()
    ));
    let root_b = env::temp_dir().join(format!(
        "ait-server-core-pack-repo-local-b-{}",
        std::process::id()
    ));
    for root in [&root_a, &root_b] {
        if root.exists() {
            fs::remove_dir_all(root).expect("stale repository-local pack root should remove");
        }
    }
    let service_a =
        BinaryDbNativeRepositoryService::new(FilesystemServerRemoteBinaryDb::test_fixture(
            RepoId::new("REPO-PACK-LOCAL-A"),
            RepoName::new("repo-bin"),
            StorePath::from(root_a),
            StoreGeneration::new(1),
        ));
    let service_b =
        BinaryDbNativeRepositoryService::new(FilesystemServerRemoteBinaryDb::test_fixture(
            RepoId::new("REPO-PACK-LOCAL-B"),
            RepoName::new("repo-bin"),
            StorePath::from(root_b),
            StoreGeneration::new(1),
        ));
    create_native_binary_repository(&service_a);
    create_native_binary_repository(&service_b);

    let (content_a, content_b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| seed_native_binary_content(&service_a, "repo-local-a"));
        let b = scope.spawn(|| seed_native_binary_content(&service_b, "repo-local-b"));
        (
            a.join().expect("repository A seed should join"),
            b.join().expect("repository B seed should join"),
        )
    });

    assert_eq!(content_a.object_pack_id, content_b.object_pack_id);
    assert_ne!(content_a.blob_id, content_b.blob_id);
    assert_eq!(
        service_a
            .read_binary_blob_content(&content_a.blob_id)
            .expect("repository A blob should read"),
        content_a.bytes
    );
    assert_eq!(
        service_b
            .read_binary_blob_content(&content_b.blob_id)
            .expect("repository B blob should read"),
        content_b.bytes
    );
    assert_eq!(
        service_a
            .read_binary_blob_content(&content_b.blob_id)
            .expect_err("repository A must not resolve repository B blob")
            .kind,
        NativeRepositoryErrorKind::NotFound
    );
    assert_eq!(
        service_b
            .read_binary_blob_content(&content_a.blob_id)
            .expect_err("repository B must not resolve repository A blob")
            .kind,
        NativeRepositoryErrorKind::NotFound
    );
}

#[test]
fn concurrent_same_repository_pack_id_never_overwrites_different_bytes() {
    let db = native_line_binary_db("same-repo-pack-race");
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);

    let pack_id = "PCK-0000000000AB";
    let source_root = env::temp_dir().join(format!(
        "ait-server-core-same-repo-pack-race-source-{}",
        std::process::id()
    ));
    if source_root.exists() {
        fs::remove_dir_all(&source_root).expect("stale pack race source should remove");
    }
    fs::create_dir_all(&source_root).expect("pack race source should create");
    let build_pack = |label: &str, bytes: Vec<u8>| {
        let sha256 = sha256_hex(&bytes);
        let blob_id = format!("BLB-{}", &sha256[..20]);
        let path = source_root.join(format!("{label}.zstpack"));
        write_rebuilt_zstd_pack_archive(
            path.to_str().expect("pack race path should be UTF-8"),
            pack_id,
            "2026-01-01T00:00:00Z",
            vec![ObjectPackRewriteBlob {
                entry_name: format!("blobs/{blob_id}"),
                blob_id,
                data: bytes,
                path_hint: Some(format!("{label}.txt")),
            }],
            0,
        )
        .expect("canonical race pack should write");
        fs::read(path).expect("canonical race pack should read")
    };
    let pack_a = build_pack("a", b"repository-local bytes A\n".to_vec());
    let pack_b = build_pack("b", b"repository-local bytes B are different\n".to_vec());
    assert_ne!(pack_a, pack_b);

    let barrier = Arc::new(std::sync::Barrier::new(3));
    let (result_a, result_b) = std::thread::scope(|scope| {
        let service_a = service.clone();
        let barrier_a = Arc::clone(&barrier);
        let pack_a_for_upload = pack_a.clone();
        let upload_a = scope.spawn(move || {
            barrier_a.wait();
            service_a.put_zstd_bulk_object_pack("repo-bin", pack_id, pack_a_for_upload)
        });
        let service_b = service.clone();
        let barrier_b = Arc::clone(&barrier);
        let pack_b_for_upload = pack_b.clone();
        let upload_b = scope.spawn(move || {
            barrier_b.wait();
            service_b.put_zstd_bulk_object_pack("repo-bin", pack_id, pack_b_for_upload)
        });
        barrier.wait();
        (
            upload_a.join().expect("pack A upload should join"),
            upload_b.join().expect("pack B upload should join"),
        )
    });

    let results = [result_a, result_b];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let conflict = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("one different-byte upload must conflict");
    assert_eq!(conflict.kind, NativeRepositoryErrorKind::Conflict);
    let stored = service
        .get_zstd_bulk_object_pack("repo-bin", pack_id)
        .expect("winning repository-local pack should read");
    assert!(stored == pack_a || stored == pack_b);
    fs::remove_dir_all(source_root).expect("pack race source should remove");
}

#[test]
fn raw_pack_durability_work_stays_outside_repository_pack_lock_boundary() {
    let root = env::temp_dir().join(format!(
        "ait-server-core-pack-lock-durability-boundary-{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("stale pack boundary root should remove");
    }
    let files = FaultInjectingServerBinaryDbStore::new(ServerBinaryDbFilesystemStore);
    let db = FilesystemServerRemoteBinaryDb::with_file_store(
        files,
        RepoId::new("REPO-PACK-LOCK-BOUNDARY"),
        RepoName::new("repo-bin"),
        StorePath::from(root.clone()),
        StoreGeneration::new(1),
        ServerBinaryDbAuthorityMode::TestFixture,
    );
    let service = BinaryDbNativeRepositoryService::new(db.clone());
    create_native_binary_repository(&service);

    let pack_id = "PCK-0000000000AC";
    let source = root.join("source.zstpack");
    let bytes = b"repository pack lock durability boundary\n".to_vec();
    let sha256 = sha256_hex(&bytes);
    let blob_id = format!("BLB-{}", &sha256[..20]);
    write_rebuilt_zstd_pack_archive(
        source
            .to_str()
            .expect("pack boundary source path should be UTF-8"),
        pack_id,
        "2026-01-01T00:00:00Z",
        vec![ObjectPackRewriteBlob {
            entry_name: format!("blobs/{blob_id}"),
            blob_id,
            data: bytes,
            path_hint: Some("boundary.txt".to_string()),
        }],
        0,
    )
    .expect("pack boundary source should write");
    let pack_bytes = fs::read(&source).expect("pack boundary source should read");
    let event_offset = db.file_store().events().len();

    let response = service
        .put_zstd_bulk_object_pack("repo-bin", pack_id, pack_bytes)
        .expect("raw pack should publish");
    assert_eq!(response["status"], "uploaded");

    let events = db.file_store().events();
    let events = &events[event_offset..];
    assert!(events.iter().all(|event| !event
        .path
        .to_string_lossy()
        .ends_with("server-repository-pack.write.journal")));
    let temp_sync = events
        .iter()
        .position(|event| {
            event.operation == BinaryDbTestStorageOperation::SyncFile
                && event.path.to_string_lossy().contains(".zstpack.upload-")
        })
        .expect("temporary pack must be synced");
    let lock_acquire = events
        .iter()
        .position(|event| {
            event.operation == BinaryDbTestStorageOperation::AcquireProcessLock
                && event
                    .path
                    .to_string_lossy()
                    .ends_with("server-repository-pack.write.lock")
        })
        .expect("repository-pack lock must be acquired");
    let lock_release = events
        .iter()
        .position(|event| {
            event.operation == BinaryDbTestStorageOperation::ReleaseProcessLock
                && event
                    .path
                    .to_string_lossy()
                    .ends_with("server-repository-pack.write.lock")
        })
        .expect("repository-pack lock must be released");
    let directory_sync = events
        .iter()
        .position(|event| {
            event.operation == BinaryDbTestStorageOperation::SyncDirectory
                && event.path.ends_with(".ait/objects/packs")
        })
        .expect("pack directory must be synced");
    assert!(temp_sync < lock_acquire);
    assert!(lock_acquire < lock_release);
    assert!(lock_release < directory_sync);
    fs::remove_dir_all(root).expect("pack boundary root should remove");
}

fn staged_object_pack_bytes(root: &Path, pack_id: &str, label: &str, bytes: &[u8]) -> Vec<u8> {
    let source = root.join(format!("staged-{label}.zstpack"));
    let sha256 = sha256_hex(bytes);
    let blob_id = format!("BLB-{}", &sha256[..20]);
    write_rebuilt_zstd_pack_archive(
        source
            .to_str()
            .expect("staged source Pack path should be UTF-8"),
        pack_id,
        "2026-01-01T00:00:00Z",
        vec![ObjectPackRewriteBlob {
            entry_name: format!("blobs/{blob_id}"),
            blob_id,
            data: bytes.to_vec(),
            path_hint: Some(format!("{label}.txt")),
        }],
        0,
    )
    .expect("staged source Pack should write");
    fs::read(source).expect("staged source Pack should read")
}

fn write_staged_pack(upload: &mut NativeZstdPackUpload, bytes: &[u8]) -> (u64, String) {
    let mut file = upload
        .take_file()
        .expect("staging file should be available");
    for chunk in bytes.chunks(17) {
        file.write_all(chunk)
            .expect("staged Pack chunk should write");
    }
    file.sync_all().expect("staged Pack should sync");
    drop(file);
    (bytes.len() as u64, sha256_hex(bytes))
}

#[test]
fn staged_pack_upload_publishes_idempotently_and_conflicts_without_full_file_materialization() {
    let db = native_line_binary_db("staged-pack-publication");
    let root = ServerRemoteBinaryDb::authority_root(&db)
        .as_path()
        .to_path_buf();
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);

    let pack_id = "PCK-0000000000AD";
    let pack_bytes = staged_object_pack_bytes(&root, pack_id, "first", b"first staged bytes\n");
    let mut upload = service
        .begin_zstd_bulk_pack_upload("repo-bin", pack_id, NativeZstdPackKind::Object)
        .expect("staged Pack should begin");
    let temporary_path = upload.temporary_path().to_path_buf();
    let final_path = upload.final_path().to_path_buf();
    assert_eq!(temporary_path.parent(), final_path.parent());
    assert!(temporary_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.contains(".zstpack.upload-") && value.ends_with(".tmp")));
    let (payload_bytes, payload_sha256) = write_staged_pack(&mut upload, &pack_bytes);
    let response = service
        .finish_zstd_bulk_pack_upload("repo-bin", upload, payload_bytes, &payload_sha256)
        .expect("staged Pack should publish");
    assert_eq!(response["status"], "uploaded");
    assert_eq!(
        fs::read(&final_path).expect("published Pack should read"),
        pack_bytes
    );
    assert!(!temporary_path.exists());

    let mut repeated = service
        .begin_zstd_bulk_pack_upload("repo-bin", pack_id, NativeZstdPackKind::Object)
        .expect("idempotent staged Pack should begin");
    let repeated_path = repeated.temporary_path().to_path_buf();
    let (payload_bytes, payload_sha256) = write_staged_pack(&mut repeated, &pack_bytes);
    let response = service
        .finish_zstd_bulk_pack_upload("repo-bin", repeated, payload_bytes, &payload_sha256)
        .expect("identical staged Pack should be idempotent");
    assert_eq!(response["status"], "already_present");
    assert!(!repeated_path.exists());

    let conflicting_bytes =
        staged_object_pack_bytes(&root, pack_id, "second", b"different staged bytes\n");
    let mut conflicting = service
        .begin_zstd_bulk_pack_upload("repo-bin", pack_id, NativeZstdPackKind::Object)
        .expect("conflicting staged Pack should begin");
    let conflicting_path = conflicting.temporary_path().to_path_buf();
    let (payload_bytes, payload_sha256) = write_staged_pack(&mut conflicting, &conflicting_bytes);
    let error = service
        .finish_zstd_bulk_pack_upload("repo-bin", conflicting, payload_bytes, &payload_sha256)
        .expect_err("different staged Pack must conflict");
    assert_eq!(error.kind, NativeRepositoryErrorKind::Conflict);
    assert!(!conflicting_path.exists());
    assert_eq!(
        fs::read(final_path).expect("winning Pack should remain"),
        pack_bytes
    );
}

#[test]
fn invalid_and_abandoned_staged_pack_uploads_are_removed() {
    let db = native_line_binary_db("staged-pack-cleanup");
    let service = BinaryDbNativeRepositoryService::new(db);
    create_native_binary_repository(&service);

    let mut invalid = service
        .begin_zstd_bulk_pack_upload("repo-bin", "PCK-0000000000AE", NativeZstdPackKind::Object)
        .expect("invalid staged Pack should begin");
    let invalid_path = invalid.temporary_path().to_path_buf();
    let invalid_bytes = b"not a zstd Pack";
    let (payload_bytes, payload_sha256) = write_staged_pack(&mut invalid, invalid_bytes);
    let error = service
        .finish_zstd_bulk_pack_upload("repo-bin", invalid, payload_bytes, &payload_sha256)
        .expect_err("invalid staged Pack must fail validation");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert!(!invalid_path.exists());

    let mut abandoned = service
        .begin_zstd_bulk_pack_upload("repo-bin", "TPK-0000000000AF", NativeZstdPackKind::Tree)
        .expect("abandoned staged Pack should begin");
    let abandoned_path = abandoned.temporary_path().to_path_buf();
    let mut abandoned_file = abandoned
        .take_file()
        .expect("abandoned staging file should be available");
    abandoned_file
        .write_all(b"partial")
        .expect("abandoned staging bytes should write");
    drop(abandoned_file);
    std::mem::forget(abandoned);
    assert!(abandoned_path.exists());

    assert_eq!(
        service
            .cleanup_abandoned_zstd_pack_uploads()
            .expect("abandoned staged Pack cleanup should succeed"),
        1
    );
    assert!(!abandoned_path.exists());
    assert_eq!(
        service
            .cleanup_abandoned_zstd_pack_uploads()
            .expect("repeated staged Pack cleanup should be idempotent"),
        0
    );
}
