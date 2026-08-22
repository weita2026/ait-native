use super::*;
use crate::foundation::native_repositories::zstd_bulk::validate_zstd_pack_index_metadata;
fn current_object_pack_index(blob_id: &str, checksum: &str, entry_type: &str) -> JsonValue {
    json!({
        "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
        "index_entry_name": "zstd-chunked-object-index",
        "entries": [{
            "entry_name": format!("blobs/{blob_id}"),
            "blob_id": blob_id,
            "entry_type": entry_type,
            "byte_length": 1,
            "uncompressed_byte_length": 1,
            "base_blob_id": if entry_type == "delta" { json!("BLB-BASE") } else { JsonValue::Null },
            "chain_depth": if entry_type == "delta" { 1 } else { 0 },
            "checksum": checksum,
            "delta_algorithm": if entry_type == "delta" {
                json!(crate::foundation::pack_substrate::PACK_DELTA_GIT_BINARY_V1)
            } else {
                JsonValue::Null
            },
        }],
    })
}

fn current_tree_pack_index(
    tree_id: &str,
    entry_ordinal: usize,
    entry_count: usize,
    checksum: &str,
) -> JsonValue {
    json!({
        "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        "index_entry_name": "zstd-chunked-tree-index",
        "trees": [{
            "tree_id": tree_id,
            "entry_ordinal": entry_ordinal,
            "entry_name": format!("trees/{tree_id}.json"),
            "entry_count": entry_count,
            "byte_length": 1,
            "checksum": checksum,
        }],
    })
}
#[test]
fn remote_sync_plan_json_capabilities_match_server_contract() {
    assert_eq!(
        RemoteSyncPlanJson::stateless().capabilities_payload(),
        json!({
            "capabilities": [
                "remote_sync.pack_bulk.zstd.v1",
                "remote_sync.pack_bulk.zstd.download.v1",
                "remote_sync.pack_bulk.zstd.pull_manifest.v1",
            ],
            "remote_sync_capabilities": {
                "zstd_pack_bulk": true,
                "zstd_pack_bulk_download": true,
                "zstd_pull_manifest": true,
            },
            "repository_pack_storage": {
                "contract": "ait.repository.pack_storage.v1",
                "payload_field": "pack_storage",
                "missing_payload_default": "zstd_only",
            },
        })
    );
}

#[test]
fn zstd_pull_manifest_request_rejects_invalid_contract_and_duplicate_have_ids() {
    let valid = RemoteSyncPlanJson::stateless()
        .zstd_pull_manifest_request(&json!({
            "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
            "head_snapshot_id": "SNP-HEAD",
            "have_snapshot_ids": ["SNP-A", "SNP-B"],
        }))
        .expect("valid pull-manifest request");
    assert_eq!(valid.head_snapshot_id, "SNP-HEAD");
    assert_eq!(
        valid.have_snapshot_ids,
        BTreeSet::from(["SNP-A".to_string(), "SNP-B".to_string()])
    );

    let invalid_contract = RemoteSyncPlanJson::stateless()
        .zstd_pull_manifest_request(&json!({
            "contract": "ait.remote_sync.zstd_bulk.pull_manifest.request.v0",
            "head_snapshot_id": "SNP-HEAD",
            "have_snapshot_ids": [],
        }))
        .expect_err("unknown pull-manifest contract must fail");
    assert_eq!(invalid_contract.kind, NativeRepositoryErrorKind::BadRequest);

    let duplicate_have = RemoteSyncPlanJson::stateless()
        .zstd_pull_manifest_request(&json!({
            "contract": REMOTE_SYNC_ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_V1,
            "head_snapshot_id": "SNP-HEAD",
            "have_snapshot_ids": ["SNP-A", "SNP-A"],
        }))
        .expect_err("duplicate have IDs must fail");
    assert_eq!(duplicate_have.kind, NativeRepositoryErrorKind::BadRequest);
}

#[test]
fn repository_pack_storage_capability_matches_contract() {
    assert_eq!(
        repository_pack_storage_capability_json(),
        json!({
            "contract": "ait.repository.pack_storage.v1",
            "payload_field": "pack_storage",
            "missing_payload_default": "zstd_only",
        })
    );
}

#[test]
fn remote_sync_plan_json_parses_strings_objects_nulls_and_missing_fields() {
    let request = RemoteSyncPlanJson::stateless()
        .zstd_bulk_plan_request(&json!({
            "contract": "ait.remote_sync.zstd_bulk.plan.v1",
            "snapshot_ids": [" SNP-A ", "SNP-B"],
            "object_packs": [" OPK-A ", {"pack_id": " OPK-B "}],
            "tree_packs": null
        }))
        .expect("plan request should parse");

    assert_eq!(
        request,
        RemoteSyncZstdBulkPlanRequest {
            snapshot_ids: vec!["SNP-A".to_string(), "SNP-B".to_string()],
            object_pack_ids: vec!["OPK-A".to_string(), "OPK-B".to_string()],
            tree_pack_ids: Vec::new(),
        }
    );

    let missing_fields = RemoteSyncPlanJson::stateless()
        .zstd_bulk_plan_request(&json!({}))
        .expect("missing arrays should default to empty");
    assert_eq!(
        missing_fields,
        RemoteSyncZstdBulkPlanRequest {
            snapshot_ids: Vec::new(),
            object_pack_ids: Vec::new(),
            tree_pack_ids: Vec::new(),
        }
    );
}

#[test]
fn remote_sync_plan_json_response_preserves_request_order_and_duplicates() {
    let request = RemoteSyncZstdBulkPlanRequest {
        snapshot_ids: vec![
            "SNP-B".to_string(),
            "SNP-A".to_string(),
            "SNP-B".to_string(),
        ],
        object_pack_ids: vec!["OPK-B".to_string(), "OPK-A".to_string()],
        tree_pack_ids: vec!["TPK-A".to_string(), "TPK-B".to_string()],
    };
    let response = RemoteSyncPlanJson::stateless().zstd_bulk_plan_response(
        "repo-a",
        &request,
        &RemoteSyncZstdBulkPlanPresence {
            present_snapshot_ids: BTreeSet::from(["SNP-B".to_string()]),
            present_object_pack_ids: BTreeSet::from(["OPK-A".to_string()]),
            present_tree_pack_ids: BTreeSet::from(["TPK-B".to_string()]),
        },
    );

    assert_eq!(
        response,
        json!({
            "repo_name": "repo-a",
            "checked_snapshot_ids": ["SNP-B", "SNP-A", "SNP-B"],
            "present_snapshot_ids": ["SNP-B", "SNP-B"],
            "missing_snapshot_ids": ["SNP-A"],
            "present_object_pack_ids": ["OPK-A"],
            "missing_object_pack_ids": ["OPK-B"],
            "present_tree_pack_ids": ["TPK-B"],
            "missing_tree_pack_ids": ["TPK-A"],
        })
    );
}

#[test]
fn remote_sync_plan_json_keeps_stable_bad_request_errors() {
    let cases = [
        (json!(["SNP-A"]), "zstd bulk plan payload must be an object"),
        (
            json!({"snapshot_ids": "SNP-A"}),
            "zstd bulk `snapshot_ids` must be an array",
        ),
        (
            json!({"snapshot_ids": [" "]}),
            "zstd bulk `snapshot_ids` entries must be non-empty strings",
        ),
        (
            json!({"object_packs": [1]}),
            "zstd bulk `object_packs` entries must be strings or objects",
        ),
        (
            json!({"object_packs": [{}]}),
            "Field `pack_id` must be a non-empty string.",
        ),
    ];

    for (payload, expected_message) in cases {
        let error = RemoteSyncPlanJson::stateless()
            .zstd_bulk_plan_request(&payload)
            .expect_err("invalid payload should fail");
        assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
        assert_eq!(error.message, expected_message);
    }
}

#[test]
fn remote_sync_commit_json_parses_arrays_and_line_update() {
    let payload = json!({
        "object_packs": [{"pack_id": "OPK-A"}],
        "tree_packs": null,
        "blob_locators": [],
        "line_update": {
            "line_name": "main",
            "head_snapshot_id": "SNP-HEAD",
            "expected_head_snapshot_id": "SNP-OLD"
        }
    });
    let contract = RemoteSyncCommitJson::stateless();
    let object = contract
        .zstd_bulk_commit_object(&payload)
        .expect("commit payload should be an object");

    assert_eq!(
        contract
            .zstd_bulk_commit_values(object, "object_packs")
            .expect("object packs should parse")
            .len(),
        1
    );
    assert!(contract
        .zstd_bulk_commit_values(object, "tree_packs")
        .expect("null tree packs should default empty")
        .is_empty());
    assert!(contract
        .zstd_bulk_commit_values(object, "tree_locators")
        .expect("missing tree locators should default empty")
        .is_empty());
    let (line_name, line_update) = contract
        .line_update_request(object)
        .expect("line update should parse")
        .expect("line update should be present");
    assert_eq!(line_name, "main");
    assert_eq!(line_update.head_snapshot_id.as_deref(), Some("SNP-HEAD"));
    assert_eq!(
        line_update.expected_head_snapshot_id.as_deref(),
        Some("SNP-OLD")
    );
}

#[test]
fn remote_sync_line_authority_allows_bootstrap_replay_and_feature_but_rejects_default_advance() {
    for (line_name, current, requested) in [
        ("main", None, Some("SNP-NEW")),
        ("main", Some("SNP-HEAD"), Some("SNP-HEAD")),
        ("feature/task", Some("SNP-OLD"), Some("SNP-NEW")),
    ] {
        require_remote_sync_line_update_authority("main", line_name, current, requested)
            .expect("allowed remote-sync Line transition");
    }

    for requested in [Some("SNP-NEW"), None] {
        let error =
            require_remote_sync_line_update_authority("main", "main", Some("SNP-OLD"), requested)
                .expect_err("initialized default Line must require authoritative Land");
        assert_eq!(error.kind, NativeRepositoryErrorKind::Conflict);
        assert!(error.message.contains("GOVERNED_TARGET_LINE_REQUIRES_LAND"));
        assert!(error.message.contains("authoritative remote Task Land"));
    }
}

#[test]
fn remote_sync_commit_json_preserves_invalid_pack_id_segment_error() {
    let error = RemoteSyncCommitJson::stateless()
        .validate_zstd_pack_id_segment("../bad")
        .expect_err("invalid pack_id should fail");

    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "invalid zstd pack_id path segment: ../bad");
}

#[test]
fn remote_sync_commit_json_keeps_stable_bad_request_errors() {
    let contract = RemoteSyncCommitJson::stateless();
    let error = contract
        .zstd_bulk_commit_object(&json!(["not-object"]))
        .expect_err("non-object payload should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "zstd bulk commit payload must be an object");

    let payload = json!({"object_packs": "OPK-A"});
    let object = contract
        .zstd_bulk_commit_object(&payload)
        .expect("payload should be object");
    let error = contract
        .zstd_bulk_commit_values(object, "object_packs")
        .expect_err("wrong array type should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "zstd bulk `object_packs` must be an array");

    let payload = json!({"line_update": "main"});
    let object = contract
        .zstd_bulk_commit_object(&payload)
        .expect("payload should be object");
    let error = contract
        .line_update_request(object)
        .expect_err("wrong line_update type should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "zstd bulk commit line_update must be an object or null"
    );
}

#[test]
fn remote_sync_commit_json_builds_response_shape() {
    let response = RemoteSyncCommitJson::stateless().zstd_bulk_commit_response(
        RemoteSyncZstdBulkCommitResponse {
            repo_name: "repo-a".to_string(),
            repo_id: "REPO-A".to_string(),
            upserted_object_packs: 1,
            skipped_object_packs: 2,
            upserted_tree_packs: 3,
            skipped_tree_packs: 4,
            upserted_blobs: 5,
            upserted_trees: 6,
            upserted_snapshots: 7,
            skipped_snapshots: 8,
            remote_line: json!({"line_name": "main"}),
            line_head_updated_after_ingest: true,
        },
    );

    assert_eq!(
        response,
        json!({
            "repo_name": "repo-a",
            "repo_id": "REPO-A",
            "upserted_object_packs": 1,
            "skipped_object_packs": 2,
            "upserted_tree_packs": 3,
            "skipped_tree_packs": 4,
            "upserted_blobs": 5,
            "upserted_trees": 6,
            "upserted_snapshots": 7,
            "skipped_snapshots": 8,
            "remote_line": {"line_name": "main"},
            "line_head_updated_after_ingest": true,
            "raw_binary_upload": true,
        })
    );
}

#[test]
fn zstd_pack_index_metadata_validation_keeps_stable_mismatch_errors() {
    let object = JsonMap::from_iter([
        ("member_count".to_string(), json!(2)),
        ("total_bytes".to_string(), json!(99)),
    ]);

    let error = validate_zstd_pack_index_metadata(
        &json!({
            "pack_id": "OPK-OTHER",
            "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
            "member_count": 2,
            "total_bytes": 99,
        }),
        &object,
        "OPK-A",
        false,
    )
    .expect_err("pack_id mismatch should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "zstd pack OPK-A index pack_id mismatch");

    let error = validate_zstd_pack_index_metadata(
        &json!({
            "pack_id": "OPK-A",
            "pack_format": "unsupported-object-pack",
            "member_count": 2,
            "total_bytes": 99,
        }),
        &object,
        "OPK-A",
        false,
    )
    .expect_err("pack_format mismatch should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "zstd pack OPK-A index pack_format mismatch");

    let error = validate_zstd_pack_index_metadata(
        &json!({
            "pack_id": "OPK-A",
            "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
            "member_count": 1,
            "total_bytes": 99,
        }),
        &object,
        "OPK-A",
        false,
    )
    .expect_err("member_count mismatch should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "zstd pack OPK-A member count mismatch");

    let error = validate_zstd_pack_index_metadata(
        &json!({
            "pack_id": "TPK-A",
            "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            "tree_count": 2,
            "total_bytes": 10,
        }),
        &JsonMap::from_iter([
            ("tree_count".to_string(), json!(2)),
            ("total_bytes".to_string(), json!(11)),
        ]),
        "TPK-A",
        true,
    )
    .expect_err("total_bytes mismatch should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "zstd pack TPK-A total_bytes mismatch");
}

#[test]
fn object_pack_entry_validation_requires_complete_current_index_metadata() {
    let indexes = BTreeMap::from([(
        "OPK-A".to_string(),
        current_object_pack_index("BLB-A", "sha256-a", "full"),
    )]);
    validate_object_pack_entry(&indexes, "OPK-A", "BLB-A", "sha256-a", "full")
        .expect("complete current object-pack row should validate");

    let error = validate_object_pack_entry(&BTreeMap::new(), "OPK-A", "BLB-A", "sha256-a", "full")
        .expect_err("unknown object pack should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "Blob BLB-A references unknown object pack OPK-A"
    );

    let error = validate_object_pack_entry(
        &BTreeMap::from([(
            "OPK-A".to_string(),
            json!({
                "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
                "index_entry_name": "zstd-chunked-object-index",
            }),
        )]),
        "OPK-A",
        "BLB-A",
        "sha256-a",
        "full",
    )
    .expect_err("missing entries should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "Invalid pack index: missing entries list");

    let error = validate_object_pack_entry(
        &BTreeMap::from([(
            "OPK-A".to_string(),
            current_object_pack_index("BLB-OTHER", "sha256-a", "full"),
        )]),
        "OPK-A",
        "BLB-A",
        "sha256-a",
        "full",
    )
    .expect_err("missing blob should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "Object pack OPK-A is missing blob BLB-A");

    let error = validate_object_pack_entry(
        &BTreeMap::from([(
            "OPK-A".to_string(),
            current_object_pack_index("BLB-A", "sha256-other", "full"),
        )]),
        "OPK-A",
        "BLB-A",
        "sha256-a",
        "full",
    )
    .expect_err("checksum mismatch should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "Object pack OPK-A checksum mismatch for blob BLB-A"
    );

    let error = validate_object_pack_entry(
        &BTreeMap::from([(
            "OPK-A".to_string(),
            current_object_pack_index("BLB-A", "sha256-a", "delta"),
        )]),
        "OPK-A",
        "BLB-A",
        "sha256-a",
        "full",
    )
    .expect_err("entry_type mismatch should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "Object pack OPK-A entry_type mismatch for blob BLB-A"
    );

    let missing_metadata = validate_object_pack_entry(
        &BTreeMap::from([(
            "OPK-A".to_string(),
            json!({
                "entries": [{
                    "blob_id": "BLB-A",
                    "checksum": "sha256-a"
                }]
            }),
        )]),
        "OPK-A",
        "BLB-A",
        "sha256-a",
        "full",
    )
    .expect_err("missing current index metadata must fail closed");
    assert_eq!(missing_metadata.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        missing_metadata.message,
        "missing required field: pack_format"
    );
}

#[test]
fn tree_pack_entry_validation_requires_complete_current_index_metadata() {
    let indexes = BTreeMap::from([(
        "TPK-A".to_string(),
        current_tree_pack_index("TRE-A", 0, 2, "tree-checksum"),
    )]);
    validate_tree_pack_entry(&indexes, "TPK-A", "TRE-A", 2, "tree-checksum")
        .expect("complete current tree-pack row should validate");

    let error = validate_tree_pack_entry(&BTreeMap::new(), "TPK-A", "TRE-A", 2, "tree-checksum")
        .expect_err("unknown tree pack should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "Tree TRE-A references unknown tree pack TPK-A"
    );

    let error = validate_tree_pack_entry(
        &BTreeMap::from([(
            "TPK-A".to_string(),
            json!({
                "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                "index_entry_name": "zstd-chunked-tree-index",
            }),
        )]),
        "TPK-A",
        "TRE-A",
        2,
        "tree-checksum",
    )
    .expect_err("missing trees should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "Invalid tree pack index: missing trees list");

    let error = validate_tree_pack_entry(
        &BTreeMap::from([(
            "TPK-A".to_string(),
            current_tree_pack_index("TRE-OTHER", 0, 2, "tree-checksum"),
        )]),
        "TPK-A",
        "TRE-A",
        2,
        "tree-checksum",
    )
    .expect_err("missing tree should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "Tree pack TPK-A is missing tree TRE-A");

    let error = validate_tree_pack_entry(
        &BTreeMap::from([(
            "TPK-A".to_string(),
            current_tree_pack_index("TRE-A", 0, 1, "tree-checksum"),
        )]),
        "TPK-A",
        "TRE-A",
        2,
        "tree-checksum",
    )
    .expect_err("entry_count mismatch should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "Tree pack TPK-A entry_count mismatch for tree TRE-A"
    );

    let error = validate_tree_pack_entry(
        &BTreeMap::from([(
            "TPK-A".to_string(),
            current_tree_pack_index("TRE-A", 0, 2, "other-checksum"),
        )]),
        "TPK-A",
        "TRE-A",
        2,
        "tree-checksum",
    )
    .expect_err("checksum mismatch should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "Tree pack TPK-A checksum mismatch for tree TRE-A"
    );

    let missing_metadata = validate_tree_pack_entry(
        &BTreeMap::from([(
            "TPK-A".to_string(),
            json!({
                "trees": [{
                    "tree_id": "TRE-A",
                    "entry_count": 2,
                    "checksum": "tree-checksum"
                }]
            }),
        )]),
        "TPK-A",
        "TRE-A",
        2,
        "tree-checksum",
    )
    .expect_err("missing current tree-pack metadata must fail closed");
    assert_eq!(missing_metadata.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        missing_metadata.message,
        "missing required field: pack_format"
    );
}

#[test]
fn root_tree_locator_validation_keeps_stable_mismatch_errors() {
    validate_root_tree_locator_index(
        &current_tree_pack_index("TRE-ROOT", 0, 1, "tree-checksum"),
        "TPK-ROOT",
        0,
    )
    .expect("complete current root tree row should validate");

    let error = validate_root_tree_locator_index(
        &json!({
            "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            "index_entry_name": "zstd-chunked-tree-index",
        }),
        "TPK-ROOT",
        0,
    )
    .expect_err("missing trees should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(error.message, "Invalid tree pack index: missing trees list");

    let error = validate_root_tree_locator_index(
        &current_tree_pack_index("TRE-OTHER", 1, 1, "tree-checksum"),
        "TPK-ROOT",
        0,
    )
    .expect_err("missing root entry should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "Tree pack TPK-ROOT is missing root entry ordinal 0"
    );
}

#[test]
fn remote_sync_commit_helpers_keep_uploaded_index_and_root_locator_behavior() {
    let contract = RemoteSyncCommitJson::stateless();

    assert_eq!(
        contract.uploaded_zstd_pack_index(
            json!({
                "pack_id": "OPK-A",
                "pack_format": REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1,
                "member_count": 1,
                "total_bytes": 9,
            })
            .to_string()
            .as_bytes(),
        ),
        Some(json!({
            "pack_id": "OPK-A",
            "pack_format": REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1,
            "member_count": 1,
            "total_bytes": 9,
        }))
    );
    assert_eq!(
        contract.uploaded_zstd_pack_index(
            json!({
                "trees": [{
                    "entry_ordinal": 0
                }]
            })
            .to_string()
            .as_bytes(),
        ),
        None
    );

    let root_index = contract
        .uploaded_tree_pack_root_index(
            current_tree_pack_index("TRE-ROOT", 0, 1, "tree-checksum")
                .to_string()
                .as_bytes(),
        )
        .expect("current tree root index should parse");
    contract
        .validate_uploaded_root_tree_locator(&root_index, "TPK-ROOT", 0)
        .expect("complete current root tree row should validate");

    let error = contract
        .validate_uploaded_root_tree_locator(&root_index, "TPK-ROOT", -1)
        .expect_err("negative root ordinal should fail");
    assert_eq!(error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        error.message,
        "Tree pack TPK-ROOT is missing root entry ordinal -1"
    );

    contract
        .validate_uploaded_zstd_pack_index_metadata(
            &json!({
                "pack_id": "OPK-A",
                "pack_format": REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1,
                "member_count": 1,
                "total_bytes": 9,
            }),
            &JsonMap::from_iter([
                ("member_count".to_string(), json!(1)),
                ("total_bytes".to_string(), json!(9)),
            ]),
            "OPK-A",
            false,
        )
        .expect("route upload metadata should keep transport pack_format");
}

#[test]
fn zstd_mirror_contract_covers_current_manifest_and_pack_validation() {
    let zstd_manifest = RemoteSyncZstdImportManifestJson::stateless()
        .zstd_import_manifest_response(
            "repo-a",
            "SNP-ZSTD-MIRROR-CONTRACT",
            json!({
                "snapshot_id": "SNP-ZSTD-MIRROR-CONTRACT",
                "parent_snapshot_id": JsonValue::Null,
                "root_tree_pack_id": "TPK-ZSTD-MIRROR-CONTRACT",
                "root_entry_ordinal": 0,
                "manifest_hash": "current-manifest-hash",
                "message": "current zstd import",
                "line_name": "main",
                "snapshot_kind": "line",
                "file_count": 1,
                "total_bytes": 6,
                "created_at": "2026-07-07T00:00:00Z"
            }),
            vec![json!({
                "pack_id": "OPK-ZSTD-MIRROR-CONTRACT",
                "pack_format": REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1,
                "member_count": 1,
                "total_bytes": 6,
                "pack_index_entry_name": "zstd-chunked-object-index",
                "pack_index_checksum": "object-index-ok",
                "created_at": "2026-07-07T00:00:00Z"
            })],
            vec![json!({
                "pack_id": "TPK-ZSTD-MIRROR-CONTRACT",
                "pack_format": REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1,
                "tree_count": 1,
                "total_bytes": 18,
                "pack_index_entry_name": "zstd-chunked-tree-index",
                "pack_index_checksum": "tree-index-ok",
                "created_at": "2026-07-07T00:00:01Z"
            })],
            vec![json!({
                "blob_id": "BLB-ZSTD-MIRROR-CONTRACT",
                "sha256": sha256_hex(b"hello\n"),
                "size_bytes": 6,
                "pack_id": "OPK-ZSTD-MIRROR-CONTRACT",
                "pack_entry_name": "objects/BLB-ZSTD-MIRROR-CONTRACT",
                "pack_entry_type": "full",
                "pack_base_blob_id": JsonValue::Null,
                "pack_chain_depth": 0,
                "created_at": "2026-07-07T00:00:00Z"
            })],
            vec![json!({
                "tree_id": "TRE-ZSTD-MIRROR-CONTRACT",
                "entry_count": 1,
                "tree_pack_id": "TPK-ZSTD-MIRROR-CONTRACT",
                "tree_pack_checksum": "tree-checksum",
                "created_at": "2026-07-07T00:00:01Z"
            })],
        );
    assert_eq!(
        zstd_manifest["contract"],
        REMOTE_SYNC_ZSTD_IMPORT_MANIFEST_CONTRACT_V1
    );
    assert_eq!(zstd_manifest["repo_name"], "repo-a");
    assert_eq!(zstd_manifest["snapshot_id"], "SNP-ZSTD-MIRROR-CONTRACT");
    assert_eq!(
        zstd_manifest["snapshots"]
            .as_array()
            .expect("manifest snapshots should be an array")
            .len(),
        1
    );
    assert_eq!(
        zstd_manifest["snapshots"][0]["message"],
        "current zstd import"
    );
    assert_eq!(zstd_manifest["snapshots"][0]["snapshot_kind"], "line");
    assert_eq!(
        zstd_manifest["blob_locators"][0]["pack_entry_name"],
        "objects/BLB-ZSTD-MIRROR-CONTRACT"
    );
    assert_eq!(
        zstd_manifest["object_packs"][0]["created_at"],
        "2026-07-07T00:00:00Z"
    );
    assert!(zstd_manifest["object_packs"][0].get("pack_path").is_none());
    assert!(zstd_manifest["tree_packs"][0].get("pack_path").is_none());
    assert!(zstd_manifest["line_update"].is_null());

    let manifest_bytes = serde_json::to_vec_pretty(&zstd_manifest).expect("manifest should encode");
    assert!(!manifest_bytes.ends_with(b"\n"));
    let decoded_manifest: JsonValue =
        serde_json::from_slice(&manifest_bytes).expect("manifest should decode");
    assert_eq!(decoded_manifest, zstd_manifest);

    let contract = RemoteSyncCommitJson::stateless();
    let object_metadata = JsonMap::from_iter([
        ("member_count".to_string(), json!(1)),
        ("total_bytes".to_string(), json!(6)),
        ("pack_index_checksum".to_string(), json!("object-index-ok")),
    ]);
    contract
        .validate_uploaded_zstd_pack_index_metadata(
            &json!({
                "pack_id": "OPK-ZSTD-MIRROR-CONTRACT",
                "pack_format": REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1,
                "member_count": 1,
                "total_bytes": 6,
                "pack_index_checksum": "object-index-ok"
            }),
            &object_metadata,
            "OPK-ZSTD-MIRROR-CONTRACT",
            false,
        )
        .expect("zstd object-pack index metadata should validate");

    let checksum_error = contract
        .validate_uploaded_zstd_pack_index_metadata(
            &json!({
                "pack_id": "OPK-ZSTD-MIRROR-CONTRACT",
                "pack_format": REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1,
                "member_count": 1,
                "total_bytes": 6,
                "pack_index_checksum": "actual-object-index"
            }),
            &object_metadata,
            "OPK-ZSTD-MIRROR-CONTRACT",
            false,
        )
        .expect_err("checksum mismatch should fail");
    assert_eq!(checksum_error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        checksum_error.message,
        "zstd pack OPK-ZSTD-MIRROR-CONTRACT index checksum mismatch"
    );

    let size_error = contract
        .validate_uploaded_zstd_pack_index_metadata(
            &json!({
                "pack_id": "OPK-ZSTD-MIRROR-CONTRACT",
                "pack_format": REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1,
                "member_count": 1,
                "total_bytes": 5,
                "pack_index_checksum": "object-index-ok"
            }),
            &object_metadata,
            "OPK-ZSTD-MIRROR-CONTRACT",
            false,
        )
        .expect_err("size mismatch should fail");
    assert_eq!(size_error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        size_error.message,
        "zstd pack OPK-ZSTD-MIRROR-CONTRACT total_bytes mismatch"
    );

    let malformed_error = contract
        .zstd_bulk_commit_object(&json!(["not-object"]))
        .expect_err("malformed zstd commit body should fail with stable bad request");
    assert_eq!(malformed_error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        malformed_error.message,
        "zstd bulk commit payload must be an object"
    );

    let root_index = contract
        .uploaded_tree_pack_root_index(
            json!({
                "pack_id": "TPK-ZSTD-MIRROR-CONTRACT",
                "pack_format": REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1,
                "index_entry_name": "zstd-chunked-tree-index",
                "tree_count": 1,
                "total_bytes": 18,
                "trees": [{
                    "tree_id": "TRE-ZSTD-MIRROR-CONTRACT",
                    "entry_ordinal": 0,
                    "entry_name": "trees/TRE-ZSTD-MIRROR-CONTRACT.json",
                    "entry_count": 1,
                    "byte_length": 18,
                    "checksum": "tree-checksum"
                }]
            })
            .to_string()
            .as_bytes(),
        )
        .expect("tree root index should parse");
    contract
        .validate_uploaded_root_tree_locator(&root_index, "TPK-ZSTD-MIRROR-CONTRACT", 0)
        .expect("requested snapshot root ordinal should validate");
    let root_error = contract
        .validate_uploaded_root_tree_locator(&root_index, "TPK-ZSTD-MIRROR-CONTRACT", 1)
        .expect_err("missing root ordinal should fail");
    assert_eq!(root_error.kind, NativeRepositoryErrorKind::BadRequest);
    assert_eq!(
        root_error.message,
        "Tree pack TPK-ZSTD-MIRROR-CONTRACT is missing root entry ordinal 1"
    );
}
