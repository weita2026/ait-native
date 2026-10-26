use super::*;

#[test]
fn snapshot_transport_source_omits_retired_zip_pack_surface() {
    let sources = [
        (
            "local_snapshot/mod.rs",
            include_str!("../local_snapshot/mod.rs"),
        ),
        (
            "local_snapshot/export.rs",
            include_str!("../local_snapshot/export.rs"),
        ),
        (
            "content_binary_db/adapters/local.rs",
            include_str!("../content_binary_db/adapters/local.rs"),
        ),
        (
            "task_workflow_remote_traits/snapshot_remote_ports.rs",
            include_str!("../task_workflow_remote_traits/snapshot_remote_ports.rs"),
        ),
        (
            "task_workflow_http_adapter/mod.rs",
            include_str!("../task_workflow_http_adapter/mod.rs"),
        ),
        (
            "task_workflow_http_adapter/task_remote.rs",
            include_str!("../task_workflow_http_adapter/task_remote.rs"),
        ),
        (
            "plan_http_client/request_specs/repository_specs.rs",
            include_str!("../plan_http_client/request_specs/repository_specs.rs"),
        ),
        (
            "plan_http_client/task_endpoints.rs",
            include_str!("../plan_http_client/task_endpoints.rs"),
        ),
        (
            "ait-cli/primitives.rs",
            include_str!("../../../ait-cli/src/primitives.rs"),
        ),
        (
            "ait-cli/primitives/snapshot.rs",
            include_str!("../../../ait-cli/src/primitives/snapshot.rs"),
        ),
        (
            "ait-cli/primitives/remote_sync/sync.rs",
            include_str!("../../../ait-cli/src/primitives/remote_sync/sync.rs"),
        ),
        (
            "ait-cli/primitives/remote_sync/backend.rs",
            include_str!("../../../ait-cli/src/primitives/remote_sync/backend.rs"),
        ),
        (
            "ait-cli/main_app/cli_args.rs",
            include_str!("../../../ait-cli/src/main_app/cli_args.rs"),
        ),
        (
            "ait-cli/main_app/repo_commands.rs",
            include_str!("../../../ait-cli/src/main_app/repo_commands.rs"),
        ),
        (
            "ait-py/exports/plan_filesystem_storage.rs",
            include_str!("../../../ait-py/src/exports/plan_filesystem_storage.rs"),
        ),
        (
            "ait-py/exports/remote_clients.rs",
            include_str!("../../../ait-py/src/exports/remote_clients.rs"),
        ),
        (
            "ait-py/exports/task_workflow.rs",
            include_str!("../../../ait-py/src/exports/task_workflow.rs"),
        ),
        (
            "ait-py/exports/workflow_policy.rs",
            include_str!("../../../ait-py/src/exports/workflow_policy.rs"),
        ),
    ];
    let forbidden = [
        "snapshots/{snapshot_id}:pack",
        "snapshot_bundle_binary_pack",
        "SNAPSHOT_BUNDLE_PACK",
        "SnapshotBundlePack",
        "put_remote_snapshot_pack",
        "get_remote_snapshot_pack",
        "zip_snapshot_bundle",
        "snapshot_bundle_export",
        "snapshot_bundle_import",
        "export_snapshot_bundle_with_store",
        "task_workflow_snapshot_bundle_",
        "plan_pack_substrate_write_pack_archive_with_format",
        "plan_pack_substrate_read_pack_index_with_format",
        "plan_pack_substrate_pack_has_entry_with_format",
        "plan_pack_substrate_write_tree_pack_archive_with_format",
        "plan_pack_substrate_read_tree_pack_index_with_format",
        "plan_pack_substrate_read_tree_pack_tree_with_format",
        "plan_pack_substrate_read_tree_pack_index_without_ordinals",
        "plan_pack_substrate_read_tree_pack_tree_by_ordinal_with_format",
        "LocalSnapshotImportTransactionStore",
        "put_remote_snapshot",
        "storage_ingest_mode",
        "server_storage_mode",
        "storage_modes",
    ];

    for (path, source) in sources {
        for token in forbidden {
            assert!(
                !source.contains(token),
                "{path} must not restore retired Snapshot ZIP transport token {token:?}"
            );
        }
    }

    let local_snapshot_sources = [
        include_str!("../local_snapshot/mod.rs"),
        include_str!("../local_snapshot/export.rs"),
        include_str!("../local_snapshot/snapshot.rs"),
        include_str!("../local_snapshot/tree_rows.rs"),
        include_str!("../local_snapshot/types.rs"),
        include_str!("../local_snapshot/util.rs"),
    ];
    for source in local_snapshot_sources {
        assert!(!source.contains("ZipArchive"));
        assert!(!source.contains("ZipWriter"));
    }

    let request_specs = include_str!("../plan_http_client/request_specs/repository_specs.rs");
    assert!(request_specs.contains("object-packs/"));
    assert!(request_specs.contains("tree-packs/"));
    assert!(request_specs.contains("pull-manifests"));
    assert!(request_specs.contains("import-manifests/"));
}

#[test]
fn snapshot_json_builds_remote_snapshot_request_specs() {
    let config = PlanHttpClientConfig {
        base_url: "https://ait.example".to_string(),
        repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
        ..PlanHttpClientConfig::default()
    };
    let codec = SnapshotJson::stateless();

    let get = codec
        .build_get_remote_snapshot_request_spec(
            &config,
            "repo",
            "SNP-1",
            false,
            Some("docs/plan.md"),
        )
        .unwrap();
    assert_eq!(get.method, "GET");
    assert_eq!(
        get.query_pairs,
        vec![
            ("include_content".to_string(), "false".to_string()),
            ("path".to_string(), "docs/plan.md".to_string())
        ]
    );

    let exists = codec
        .build_get_remote_snapshots_existence_request_spec(
            &config,
            "repo",
            &[" SNP-1 ".to_string(), "".to_string(), "SNP-2".to_string()],
        )
        .unwrap();
    assert_eq!(exists.method, "POST");
    assert_eq!(
        exists.body.unwrap()["snapshot_ids"],
        json!(["SNP-1", "SNP-2"])
    );
}

#[test]
fn snapshot_json_owns_manifest_and_diff_facades() {
    let old_files = json!({
        "a.txt": {"blob_id": "BLB-1", "size_bytes": 5, "mode": "100644"},
        "b.txt": {"blob_id": "BLB-2", "size_bytes": 8, "mode": "100644"}
    });
    let new_files = json!({
        "a.txt": {"blob_id": "BLB-3", "size_bytes": 7, "mode": "100644"},
        "c.txt": {"blob_id": "BLB-2", "size_bytes": 8, "mode": "100644"}
    });

    let diff = SnapshotJson::stateless()
        .diff_snapshot_manifests(&old_files, &new_files, Some("SNP-1"), Some("SNP-2"))
        .unwrap();
    assert_eq!(diff["old_snapshot_id"], json!("SNP-1"));
    assert_eq!(diff["new_snapshot_id"], json!("SNP-2"));
    assert_eq!(diff["summary"]["files_changed"], json!(3));
    assert_eq!(diff["added"], json!(["c.txt"]));
    assert_eq!(diff["deleted"], json!(["b.txt"]));
    assert_eq!(diff["modified"], json!(["a.txt"]));
}

#[test]
fn snapshot_json_builds_snapshot_record_payload() {
    let payload = SnapshotJson::stateless().snapshot_record_payload(&SnapshotRecord {
        snapshot_id: "SNP-1".to_string(),
        parent_snapshot_ids: vec!["SNP-0".to_string()],
        primary_parent_snapshot_id: Some("SNP-0".to_string()),
        parent_snapshot_id: Some("SNP-0".to_string()),
        root_tree_pack_id: Some("TPK-1".to_string()),
        root_entry_ordinal: Some(7),
        manifest_hash: "hash".to_string(),
        message: Some("snapshot".to_string()),
        line_name: "main".to_string(),
        snapshot_kind: "line".to_string(),
        file_count: 2,
        total_bytes: 42,
        created_at: "2026-07-06T00:00:00Z".to_string(),
    });

    assert_eq!(payload["snapshot_id"], json!("SNP-1"));
    assert_eq!(payload["root_entry_ordinal"], json!(7));
    assert_eq!(payload["total_bytes"], json!(42));
}
