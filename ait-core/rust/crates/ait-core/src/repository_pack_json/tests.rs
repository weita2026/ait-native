use super::*;
use crate::json_support::json;
use crate::pack_substrate::{PACK_FORMAT_ZSTD_CHUNKED_V1, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1};
use crate::repository_pack_policy::{
    ObjectPackIndexEntryInventory, ObjectPackIndexInventory, RepositoryBlobLocatorInventoryRow,
    RepositoryLineHeadInventoryRow, RepositoryObjectPackInventoryRow,
    RepositorySnapshotInventoryRow, RepositoryTreeLocatorInventoryRow,
    RepositoryTreePackInventoryRow, TreePackIndexEntryInventory, TreePackIndexInventory,
};

fn sample_zstd_import_manifest_payload() -> ZstdImportManifestPayload {
    ZstdImportManifestPayload {
        contract: ZSTD_IMPORT_MANIFEST_CONTRACT_NAME.to_string(),
        repo_name: "repo".to_string(),
        snapshot_id: "SNP-1".to_string(),
        snapshots: vec![ZstdBulkSnapshotRow {
            snapshot_id: "SNP-1".to_string(),
            parent_snapshot_ids: Vec::new(),
            primary_parent_snapshot_id: None,
            parent_snapshot_id: None,
            root_tree_pack_id: Some("TP-1".to_string()),
            root_entry_ordinal: Some(0),
            manifest_hash: Some("hash".to_string()),
            message: Some("message".to_string()),
            line_name: Some("main".to_string()),
            snapshot_kind: Some("line".to_string()),
            file_count: Some(1),
            total_bytes: Some(10),
            created_at: Some("2026-07-06T00:00:00Z".to_string()),
        }],
        object_packs: vec![ZstdBulkObjectPackRow {
            generation_key: None,
            pack_id: "OP-1".to_string(),
            repo_name: None,
            repo_id: None,
            status: None,
            pack_format: Some(PackFormatKind::ZstdChunkedV1),
            member_count: Some(1),
            total_bytes: Some(100),
            pack_path: None,
            pack_index_entry_name: Some("zstd-chunked-object-index".to_string()),
            pack_index_checksum: Some("object-index-checksum".to_string()),
            created_at: Some("2026-07-06T00:00:01Z".to_string()),
            pack_index: None,
        }],
        tree_packs: vec![ZstdBulkTreePackRow {
            generation_key: None,
            pack_id: "TP-1".to_string(),
            repo_name: None,
            repo_id: None,
            status: None,
            pack_format: Some(TreePackFormatKind::ZstdChunkedTreeV1),
            tree_count: Some(1),
            total_bytes: Some(80),
            pack_path: None,
            pack_index_entry_name: Some("zstd-chunked-tree-index".to_string()),
            pack_index_checksum: Some("tree-index-checksum".to_string()),
            created_at: Some("2026-07-06T00:00:02Z".to_string()),
            pack_index: None,
        }],
        blob_locators: vec![ZstdBulkBlobLocatorRow {
            generation_key: None,
            blob_id: "BLB-1".to_string(),
            sha256: Some("blob-sha".to_string()),
            storage_path: None,
            storage_kind: None,
            size_bytes: Some(10),
            pack_id: Some("OP-1".to_string()),
            pack_entry_name: Some("objects/BLB-1".to_string()),
            pack_entry_type: Some("full".to_string()),
            pack_base_blob_id: None,
            pack_chain_depth: Some(0),
            created_at: Some("2026-07-06T00:00:03Z".to_string()),
        }],
        tree_locators: vec![ZstdBulkTreeLocatorRow {
            generation_key: None,
            tree_id: "TREE-1".to_string(),
            entry_count: Some(1),
            tree_pack_id: Some("TP-1".to_string()),
            tree_pack_checksum: Some("tree-checksum".to_string()),
            created_at: Some("2026-07-06T00:00:04Z".to_string()),
        }],
        line_update: None,
    }
}

#[test]
fn zstd_json_payload_wrappers_implement_shared_payload_contract_trait() {
    fn assert_contract<T: JsonPayloadContract<Error = String>>(_value: &T) {}
    assert_contract(&RepositoryPackStorageJson::stateless());
    assert_contract(&ZstdImportManifestJson::stateless());
    assert_contract(&RepositoryPackInventoryJson::stateless());
    assert_contract(&ZstdBulkPlanRequestJson::stateless());
    assert_contract(&ZstdBulkCommitRequestJson::stateless());
}

#[test]
fn zstd_snapshot_transport_preserves_ordered_parents_and_legacy_projection() {
    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.snapshot_id = "SNP-MERGE".to_string();
    let snapshot = &mut manifest.snapshots[0];
    snapshot.snapshot_id = "SNP-MERGE".to_string();
    snapshot.parent_snapshot_ids = vec![
        "SNP-LEFT".to_string(),
        "SNP-RIGHT".to_string(),
        "SNP-OCTOPUS".to_string(),
    ];
    snapshot.primary_parent_snapshot_id = Some("SNP-LEFT".to_string());
    snapshot.parent_snapshot_id = Some("SNP-LEFT".to_string());
    let wrapper = ZstdImportManifestJson::stateless();
    let value = wrapper
        .encode_value(&manifest)
        .expect("encode multi-parent zstd manifest");
    assert_eq!(
        value["snapshots"][0]["parent_snapshot_ids"],
        json!(["SNP-LEFT", "SNP-RIGHT", "SNP-OCTOPUS"])
    );
    assert_eq!(
        value["snapshots"][0]["primary_parent_snapshot_id"],
        json!("SNP-LEFT")
    );
    assert_eq!(
        value["snapshots"][0]["parent_snapshot_id"],
        json!("SNP-LEFT")
    );
    assert_eq!(
        wrapper
            .decode_value(value.clone())
            .expect("decode multi-parent zstd manifest"),
        manifest
    );

    let mut conflicting = value;
    conflicting["snapshots"][0]["primary_parent_snapshot_id"] = json!("SNP-RIGHT");
    let error = wrapper
        .decode_value(conflicting)
        .expect_err("conflicting primary projection must fail");
    assert!(error.contains("primary_parent_snapshot_id projection"));

    let mut legacy = sample_zstd_import_manifest_payload();
    legacy.snapshots[0].parent_snapshot_ids = vec!["SNP-PARENT".to_string()];
    legacy.snapshots[0].primary_parent_snapshot_id = Some("SNP-PARENT".to_string());
    legacy.snapshots[0].parent_snapshot_id = Some("SNP-PARENT".to_string());
    let mut legacy_value = wrapper
        .encode_value(&legacy)
        .expect("encode linear manifest");
    let row = legacy_value["snapshots"][0]
        .as_object_mut()
        .expect("snapshot row object");
    row.remove("parent_snapshot_ids");
    row.remove("primary_parent_snapshot_id");
    assert_eq!(
        wrapper
            .decode_value(legacy_value)
            .expect("legacy single-parent manifest remains readable")
            .snapshots[0]
            .parent_snapshot_ids,
        vec!["SNP-PARENT"]
    );
}

#[test]
fn repository_pack_storage_json_wrapper_owns_pack_storage_payload_shape() {
    let payload = RepositoryPackStoragePayload {
        contract: RepositoryPackStorageContract::V1,
        zstd_only_verified: true,
        object_pack_format: PackFormatKind::ZstdChunkedV1,
        tree_pack_format: TreePackFormatKind::ZstdChunkedTreeV1,
        object_pack_count: 0,
        tree_pack_count: 0,
        zstd_object_pack_count: 0,
        zstd_tree_pack_count: 0,
        requires_zstd_remote_sync: true,
        validation: RepositoryPackStorageValidationPayload {
            state: RepositoryPackStorageValidationState::Valid,
            error_count: 0,
        },
    };
    let wrapper = RepositoryPackStorageJson::stateless();

    let value = wrapper
        .encode_value(&payload)
        .expect("encode storage payload");
    assert_eq!(
        value["contract"].as_str(),
        Some(RepositoryPackStorageContract::NAME)
    );
    assert!(value.get("inventory_mode").is_none());
    assert!(value.get("policy").is_none());
    assert_eq!(value.as_object().expect("storage payload object").len(), 10);
    let decoded = wrapper.decode_value(value).expect("decode storage payload");
    assert_eq!(decoded, payload);
}

#[test]
fn new_empty_repository_pack_storage_payload_is_zstd_only() {
    let inventory = RepositoryPackInventory::new("repo");
    let payload =
        RepositoryPackStoragePayload::from_inventory(&inventory).expect("storage payload");

    assert!(payload.zstd_only_verified);
    assert!(payload.requires_zstd_remote_sync);
    assert_eq!(payload.object_pack_format, PackFormatKind::ZstdChunkedV1);
    assert_eq!(
        payload.tree_pack_format,
        TreePackFormatKind::ZstdChunkedTreeV1
    );
    assert_eq!(
        payload.validation.state,
        RepositoryPackStorageValidationState::Valid
    );
}

#[test]
fn remote_repository_payload_missing_pack_storage_defaults_to_current_format() {
    let repository = json!({
        "repo_name": "repo",
        "repo_id": "REPO-1",
        "default_line": "main",
    });

    let storage = RepositoryPackStoragePayload::from_repository_payload(Some(&repository))
        .expect("missing pack_storage should default");
    assert_eq!(storage, RepositoryPackStoragePayload::current_default());

    let normalized = repository_payload_with_pack_storage_default(repository)
        .expect("repository payload should normalize");
    let normalized_storage =
        RepositoryPackStoragePayload::from_repository_payload(Some(&normalized))
            .expect("normalized pack_storage should decode");
    assert_eq!(
        normalized_storage,
        RepositoryPackStoragePayload::current_default()
    );
}

#[test]
fn server_handshake_pack_storage_capability_payload_names_contract_and_defaults() {
    let capability = repository_pack_storage_capability_payload();

    assert_eq!(
        capability["contract"].as_str(),
        Some(RepositoryPackStorageContract::NAME)
    );
    assert_eq!(
        capability["payload_field"].as_str(),
        Some(REPOSITORY_PACK_STORAGE_PAYLOAD_FIELD)
    );
    assert_eq!(
        capability["missing_payload_default"].as_str(),
        Some(REPOSITORY_PACK_STORAGE_MISSING_PAYLOAD_DEFAULT)
    );
    assert!(capability.get("new_repository_default_policy").is_none());
    assert!(capability.get("supported_policies").is_none());
}

#[test]
fn zstd_import_manifest_json_wrapper_owns_manifest_shape() {
    let manifest = sample_zstd_import_manifest_payload();
    let wrapper = ZstdImportManifestJson::stateless();

    let value = wrapper.encode_value(&manifest).expect("encode manifest");
    assert_eq!(
        value["contract"].as_str(),
        Some(ZSTD_IMPORT_MANIFEST_CONTRACT_NAME)
    );
    assert_eq!(value["snapshots"][0]["message"].as_str(), Some("message"));
    assert_eq!(
        value["snapshots"][0]["snapshot_kind"].as_str(),
        Some("line")
    );
    assert_eq!(
        value["blob_locators"][0]["pack_entry_name"].as_str(),
        Some("objects/BLB-1")
    );
    assert_eq!(
        value["object_packs"][0]["created_at"].as_str(),
        Some("2026-07-06T00:00:01Z")
    );
    assert_eq!(
        value["tree_packs"][0]["created_at"].as_str(),
        Some("2026-07-06T00:00:02Z")
    );
    assert!(value["object_packs"][0].get("pack_path").is_none());
    assert!(value["tree_packs"][0].get("pack_path").is_none());

    let encoded = JsonCodec::encode_value(&value, JsonEncodeOptions::compact())
        .expect("encode manifest value");
    let decoded = wrapper.decode_str(&encoded).expect("decode manifest");
    assert_eq!(decoded, manifest);
}

#[test]
fn zstd_pull_manifest_contract_round_trips_deduplicated_history() {
    let single = sample_zstd_import_manifest_payload();
    let request = ZstdPullManifestRequest {
        contract: ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME.to_string(),
        head_snapshot_id: single.snapshot_id.clone(),
        have_snapshot_ids: vec!["SNP-BOUNDARY".to_string()],
    };
    let request_json = ZstdPullManifestRequestJson::stateless();
    let request_value = request_json.encode_value(&request).expect("encode request");
    assert_eq!(
        request_json
            .decode_value(request_value)
            .expect("decode request"),
        request
    );

    let payload = ZstdPullManifestPayload {
        contract: ZSTD_PULL_MANIFEST_CONTRACT_NAME.to_string(),
        repo_name: single.repo_name,
        head_snapshot_id: single.snapshot_id,
        boundary_snapshot_ids: Vec::new(),
        snapshots: single.snapshots,
        object_packs: single.object_packs,
        tree_packs: single.tree_packs,
        blob_locators: single.blob_locators,
        tree_locators: single.tree_locators,
    };
    let wrapper = ZstdPullManifestJson::stateless();
    let value = wrapper
        .encode_value(&payload)
        .expect("encode pull manifest");
    assert_eq!(
        wrapper.decode_value(value).expect("decode pull manifest"),
        payload
    );
}

#[test]
fn zstd_pull_manifest_rejects_child_before_missing_parent() {
    let single = sample_zstd_import_manifest_payload();
    let mut child = single.snapshots[0].clone();
    child.snapshot_id = "SNP-2".to_string();
    child.parent_snapshot_ids = vec!["SNP-1".to_string()];
    child.primary_parent_snapshot_id = Some("SNP-1".to_string());
    child.parent_snapshot_id = Some("SNP-1".to_string());
    let payload = ZstdPullManifestPayload {
        contract: ZSTD_PULL_MANIFEST_CONTRACT_NAME.to_string(),
        repo_name: single.repo_name,
        head_snapshot_id: child.snapshot_id.clone(),
        boundary_snapshot_ids: Vec::new(),
        snapshots: vec![child, single.snapshots[0].clone()],
        object_packs: single.object_packs,
        tree_packs: single.tree_packs,
        blob_locators: single.blob_locators,
        tree_locators: single.tree_locators,
    };
    let error = ZstdPullManifestJson::stateless()
        .validate_domain(&payload)
        .expect_err("child-first history must fail");
    assert!(error.contains("appears before missing parent"));
}

#[test]
fn zstd_import_manifest_matches_zstd_bulk_commit_row_shapes() {
    let manifest = sample_zstd_import_manifest_payload();
    let commit = ZstdBulkCommitRequest {
        contract: Some("ait.remote_sync.zstd_bulk.commit.v1".to_string()),
        generation_key: None,
        object_packs: manifest.object_packs.clone(),
        tree_packs: manifest.tree_packs.clone(),
        blob_locators: manifest.blob_locators.clone(),
        tree_locators: manifest.tree_locators.clone(),
        snapshots: manifest.snapshots.clone(),
        line_update: manifest.line_update.clone(),
    };

    let manifest_value = ZstdImportManifestJson::stateless()
        .encode_value(&manifest)
        .expect("encode manifest");
    let commit_value = ZstdBulkCommitRequestJson::stateless()
        .encode_value(&commit)
        .expect("encode commit");
    for field in [
        "object_packs",
        "tree_packs",
        "blob_locators",
        "tree_locators",
        "snapshots",
        "line_update",
    ] {
        assert_eq!(manifest_value[field], commit_value[field], "{field}");
    }
}

#[test]
fn zstd_import_manifest_returns_exactly_one_requested_snapshot_row() {
    let wrapper = ZstdImportManifestJson::stateless();
    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.snapshots.push(manifest.snapshots[0].clone());
    assert!(wrapper
        .encode_value(&manifest)
        .expect_err("multiple snapshot rows should be rejected")
        .contains("exactly one snapshot"));

    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.snapshots[0].snapshot_id = "SNP-OTHER".to_string();
    assert!(wrapper
        .encode_value(&manifest)
        .expect_err("mismatched snapshot row should be rejected")
        .contains("match requested snapshot id"));
}

#[test]
fn zstd_import_manifest_excludes_remote_pack_path_and_inventory_fields() {
    let wrapper = ZstdImportManifestJson::stateless();
    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.object_packs[0].pack_path = Some("/server/repo/objects/OP-1.pack".to_string());
    assert!(wrapper
        .encode_value(&manifest)
        .expect_err("object pack_path should be rejected")
        .contains("object_packs[].pack_path"));

    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.tree_packs[0].repo_id = Some("repo-id".to_string());
    assert!(wrapper
        .encode_value(&manifest)
        .expect_err("tree repo_id should be rejected")
        .contains("tree_packs[].repo_id"));

    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.blob_locators[0].storage_path = Some("objects/BLB-1".to_string());
    assert!(wrapper
        .encode_value(&manifest)
        .expect_err("blob storage_path should be rejected")
        .contains("blob_locators[].storage_path"));
}

#[test]
fn zstd_import_manifest_requires_canonical_locator_and_timestamp_fields() {
    let wrapper = ZstdImportManifestJson::stateless();
    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.blob_locators[0].pack_entry_name = None;
    assert!(wrapper
        .encode_value(&manifest)
        .expect_err("pack_entry_name should be required")
        .contains("blob_locators[].pack_entry_name"));

    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.object_packs[0].created_at = None;
    assert!(wrapper
        .encode_value(&manifest)
        .expect_err("object pack created_at should be required")
        .contains("object_packs[].created_at"));

    let mut manifest = sample_zstd_import_manifest_payload();
    manifest.tree_locators[0].created_at = None;
    assert!(wrapper
        .encode_value(&manifest)
        .expect_err("tree locator created_at should be required")
        .contains("tree_locators[].created_at"));
}

#[test]
fn repository_pack_inventory_json_wrapper_owns_inventory_payload_shape() {
    let payload = RepositoryPackInventoryPayload {
        repo_name: "repo".to_string(),
        object_packs: vec![RepositoryObjectPackInventoryRow {
            pack_id: "OP-1".to_string(),
            repo_name: Some("repo".to_string()),
            repo_id: Some("repo-id".to_string()),
            status: "ready".to_string(),
            pack_format: PackFormatKind::ZstdChunkedV1,
            member_count: 1,
            total_bytes: 100,
            pack_path: "objects/OP-1.pack".to_string(),
            pack_index_entry_name: "pack-index.json".to_string(),
            pack_index_checksum: "object-index-checksum".to_string(),
            created_at: "2026-07-06T00:00:00Z".to_string(),
            embedded_index: ObjectPackIndexInventory {
                pack_id: "OP-1".to_string(),
                pack_format: PackFormatKind::ZstdChunkedV1,
                member_count: 1,
                total_bytes: 100,
                entries: vec![ObjectPackIndexEntryInventory {
                    entry_name: "objects/BLB-1".to_string(),
                    blob_id: "BLB-1".to_string(),
                    entry_type: "full".to_string(),
                    checksum: "blob-checksum".to_string(),
                    base_blob_id: None,
                    chain_depth: 0,
                }],
            },
        }],
        tree_packs: vec![RepositoryTreePackInventoryRow {
            pack_id: "TP-1".to_string(),
            repo_name: Some("repo".to_string()),
            repo_id: Some("repo-id".to_string()),
            status: "ready".to_string(),
            pack_format: TreePackFormatKind::ZstdChunkedTreeV1,
            tree_count: 1,
            total_bytes: 80,
            pack_path: "trees/TP-1.pack".to_string(),
            pack_index_entry_name: "tree-pack-index.json".to_string(),
            pack_index_checksum: "tree-index-checksum".to_string(),
            created_at: "2026-07-06T00:00:01Z".to_string(),
            embedded_index: TreePackIndexInventory {
                pack_id: "TP-1".to_string(),
                pack_format: TreePackFormatKind::ZstdChunkedTreeV1,
                tree_count: 1,
                total_bytes: 80,
                trees: vec![TreePackIndexEntryInventory {
                    tree_id: "TRE-1".to_string(),
                    entry_ordinal: 0,
                    entry_count: 1,
                    checksum: "tree-checksum".to_string(),
                }],
            },
        }],
        blob_locators: vec![RepositoryBlobLocatorInventoryRow {
            blob_id: "BLB-1".to_string(),
            sha256: "blob-sha256".to_string(),
            size_bytes: 10,
            pack_id: "OP-1".to_string(),
            pack_entry_name: "objects/BLB-1".to_string(),
            pack_entry_type: "full".to_string(),
            pack_base_blob_id: None,
            pack_chain_depth: 0,
            created_at: "2026-07-06T00:00:02Z".to_string(),
        }],
        tree_locators: vec![RepositoryTreeLocatorInventoryRow {
            tree_id: "TRE-1".to_string(),
            entry_count: 1,
            tree_pack_id: "TP-1".to_string(),
            tree_pack_checksum: "tree-checksum".to_string(),
            created_at: "2026-07-06T00:00:03Z".to_string(),
        }],
        snapshots: vec![RepositorySnapshotInventoryRow {
            snapshot_id: "SNP-1".to_string(),
            parent_snapshot_ids: Vec::new(),
            primary_parent_snapshot_id: None,
            parent_snapshot_id: None,
            root_tree_pack_id: "TP-1".to_string(),
            root_entry_ordinal: 0,
            manifest_hash: "manifest-hash".to_string(),
            message: Some("snapshot".to_string()),
            line_name: Some("main".to_string()),
            snapshot_kind: Some("line".to_string()),
            file_count: 1,
            total_bytes: 10,
            created_at: "2026-07-06T00:00:04Z".to_string(),
        }],
        line_heads: vec![RepositoryLineHeadInventoryRow {
            line_name: "main".to_string(),
            head_snapshot_id: Some("SNP-1".to_string()),
        }],
    };
    let wrapper = RepositoryPackInventoryJson::stateless();

    let value = wrapper
        .encode_value(&payload)
        .expect("encode inventory payload");
    assert_eq!(value["repo_name"].as_str(), Some("repo"));
    assert_eq!(
        value["object_packs"][0]["pack_format"].as_str(),
        Some(PACK_FORMAT_ZSTD_CHUNKED_V1)
    );
    assert_eq!(
        value["object_packs"][0]["pack_index"]["pack_format"].as_str(),
        Some(PACK_FORMAT_ZSTD_CHUNKED_V1)
    );
    assert_eq!(
        value["tree_packs"][0]["pack_format"].as_str(),
        Some(TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
    );
    assert_eq!(
        value["tree_packs"][0]["pack_index"]["pack_format"].as_str(),
        Some(TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
    );
    assert_eq!(
        value["blob_locators"][0]["pack_entry_name"].as_str(),
        Some("objects/BLB-1")
    );
    assert_eq!(value["snapshots"][0]["message"].as_str(), Some("snapshot"));

    let decoded = wrapper
        .decode_value(value)
        .expect("decode inventory payload");
    assert_eq!(decoded, payload);
}

#[test]
fn zstd_bulk_payload_rows_use_typed_pack_format_enums() {
    let request = ZstdBulkPlanRequest {
        snapshot_ids: vec!["SNP-1".to_string()],
        object_packs: vec![ZstdBulkObjectPackRow {
            generation_key: None,
            pack_id: "OP-1".to_string(),
            repo_name: None,
            repo_id: None,
            status: Some("ready".to_string()),
            pack_format: Some(PackFormatKind::ZstdChunkedV1),
            member_count: Some(1),
            total_bytes: Some(100),
            pack_path: None,
            pack_index_entry_name: None,
            pack_index_checksum: None,
            created_at: None,
            pack_index: None,
        }],
        tree_packs: vec![ZstdBulkTreePackRow {
            generation_key: None,
            pack_id: "TP-1".to_string(),
            repo_name: None,
            repo_id: None,
            status: Some("ready".to_string()),
            pack_format: Some(TreePackFormatKind::ZstdChunkedTreeV1),
            tree_count: Some(1),
            total_bytes: Some(100),
            pack_path: None,
            pack_index_entry_name: None,
            pack_index_checksum: None,
            created_at: None,
            pack_index: None,
        }],
    };
    let wrapper = ZstdBulkPlanRequestJson::stateless();

    let value = wrapper.encode_value(&request).expect("encode request");
    assert_eq!(
        value["object_packs"][0]["pack_format"].as_str(),
        Some(PACK_FORMAT_ZSTD_CHUNKED_V1)
    );
    assert_eq!(
        value["tree_packs"][0]["pack_format"].as_str(),
        Some(TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
    );
    let decoded = wrapper.decode_value(value).expect("decode request");
    assert_eq!(decoded, request);
}
