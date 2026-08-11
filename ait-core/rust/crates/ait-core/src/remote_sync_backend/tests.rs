use super::*;
use crate::json_support::json;
use crate::plan_http_client::{
    build_commit_remote_zstd_bulk_request_spec, build_get_remote_zstd_import_manifest_request_spec,
    build_get_remote_zstd_object_pack_request_spec, build_get_remote_zstd_tree_pack_request_spec,
    build_plan_remote_zstd_bulk_request_spec, build_put_remote_zstd_object_pack_request_spec,
    build_put_remote_zstd_tree_pack_request_spec, PlanHttpClientConfig,
};
use std::collections::BTreeMap;

fn http_config() -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
        headers: BTreeMap::new(),
        default_timeout_ms: 30_000,
        retry_attempts: 0,
        retry_backoff_ms: 0,
        pool_max_idle_per_host: 1,
    }
}

fn current_inventory() -> RemoteSyncSnapshotInventory {
    RemoteSyncSnapshotInventory::from_pack_formats(
        ["SNP-1"],
        [PACK_FORMAT_ZSTD_CHUNKED_V1],
        [TREE_PACK_FORMAT_ZSTD_CHUNKED_V1],
    )
}

#[test]
fn remote_sync_selects_current_zstd_backend() {
    let negotiated = negotiate_remote_sync_backend(
        &current_inventory(),
        &RemoteSyncCapabilities::with_zstd_pack_bulk(),
    )
    .expect("current inventory and capability negotiate");

    assert_eq!(negotiated.backend, RemoteSyncBackendKind::ZstdPackBulk);
    assert_eq!(
        negotiated.reason,
        "local_inventory_and_remote_support_zstd_pack_bulk"
    );
}

#[test]
fn remote_sync_rejects_unknown_pack_format_and_missing_capability() {
    let unknown = RemoteSyncSnapshotInventory::from_pack_formats(
        ["SNP-1"],
        ["unknown-object"],
        ["unknown-tree"],
    );
    let format_error =
        negotiate_remote_sync_backend(&unknown, &RemoteSyncCapabilities::with_zstd_pack_bulk())
            .expect_err("unknown local formats fail closed");
    assert!(format_error.contains("Unsupported object pack format"));

    let capability_error =
        negotiate_remote_sync_backend(&current_inventory(), &RemoteSyncCapabilities::default())
            .expect_err("the remote must advertise the current backend");
    assert!(capability_error.contains(REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK));
}

#[test]
fn current_remote_capabilities_parse_from_repository_and_health_shapes() {
    let direct = RemoteSyncCapabilities::from_server_payload(Some(&json!({
        "capabilities": [
            REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK,
            REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD,
            REMOTE_SYNC_CAPABILITY_ZSTD_PULL_MANIFEST,
            REMOTE_SYNC_CAPABILITY_SNAPSHOT_DAG_V2
        ]
    })));
    assert!(direct.zstd_pack_bulk);
    assert!(direct.zstd_pack_bulk_download);
    assert!(direct.zstd_pull_manifest);
    assert!(direct.snapshot_dag_v2);

    let nested = RemoteSyncCapabilities::from_server_payload(Some(&json!({
        "ci_capabilities": {
            "remote_sync_capabilities": {
                "zstd_pack_bulk": true,
                "zstd_pack_bulk_download": true,
                "zstd_pull_manifest": true,
                "snapshot_dag_v2": true
            }
        }
    })));
    assert_eq!(nested, direct);
}

#[test]
fn upload_capability_does_not_imply_download_capability() {
    let upload_only = RemoteSyncCapabilities::from_server_payload(Some(&json!({
        "capabilities": [REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK]
    })));
    assert!(upload_only.zstd_pack_bulk);
    assert!(!upload_only.zstd_pack_bulk_download);

    let download_only = RemoteSyncCapabilities::from_server_payload(Some(&json!({
        "capabilities": [REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD]
    })));
    assert!(!download_only.zstd_pack_bulk);
    assert!(download_only.zstd_pack_bulk_download);
}

#[test]
fn backend_payload_contains_only_current_capabilities() {
    let negotiation = RemoteSyncBackendNegotiation {
        backend: RemoteSyncBackendKind::ZstdPackBulk,
        reason: "local_inventory_and_remote_support_zstd_pack_bulk",
        capabilities: RemoteSyncCapabilities::with_zstd_pack_bulk(),
    };
    let diff = RemoteSyncInventoryDiff {
        checked_snapshot_ids: vec!["SNP-A".to_string(), "SNP-B".to_string()],
        present_snapshot_ids: vec!["SNP-A".to_string()],
        missing_snapshot_ids: vec!["SNP-B".to_string()],
    };
    let expected = json!({
        "backend": "zstd_pack_bulk",
        "reason": "local_inventory_and_remote_support_zstd_pack_bulk",
        "capabilities": {
            "zstd_pack_bulk": true,
            "zstd_pack_bulk_download": false,
            "zstd_pull_manifest": false,
            "snapshot_dag_v2": false
        },
        "diff": {
            "checked_snapshot_ids": ["SNP-A", "SNP-B"],
            "present_snapshot_ids": ["SNP-A"],
            "missing_snapshot_ids": ["SNP-B"]
        }
    });

    assert_eq!(
        RemoteSyncPlanJson::stateless().backend_payload(&negotiation, &diff),
        expected
    );
    assert_eq!(remote_sync_backend_payload(&negotiation, &diff), expected);
}

#[test]
fn snapshot_dag_capability_is_explicit() {
    require_snapshot_dag_remote_capability(&RemoteSyncCapabilities::default(), &[])
        .expect("linear uploads need no DAG capability");

    let affected = vec!["SNP-MERGE".to_string()];
    let error = require_snapshot_dag_remote_capability(
        &RemoteSyncCapabilities::with_zstd_pack_bulk(),
        &affected,
    )
    .expect_err("multi-parent upload fails closed");
    assert!(error.contains(REMOTE_SYNC_CAPABILITY_SNAPSHOT_DAG_V2));
    assert!(error.contains("SNP-MERGE"));

    require_snapshot_dag_remote_capability(
        &RemoteSyncCapabilities::with_zstd_pack_bulk().with_snapshot_dag_v2(),
        &affected,
    )
    .expect("advertised DAG capability accepts multi-parent upload");
}

#[test]
fn remote_sync_inventory_diff_preserves_order_and_duplicates() {
    let checked = vec![
        "SNP-A".to_string(),
        "SNP-B".to_string(),
        "SNP-B".to_string(),
        "SNP-C".to_string(),
    ];
    let diff = RemoteSyncInventoryDiff::from_present_snapshot_ids(
        &checked,
        &BTreeSet::from(["SNP-B".to_string()]),
    );

    assert_eq!(diff.checked_snapshot_ids, checked);
    assert_eq!(diff.present_snapshot_ids, vec!["SNP-B", "SNP-B"]);
    assert_eq!(diff.missing_snapshot_ids, vec!["SNP-A", "SNP-C"]);
}

#[test]
fn current_backend_exposes_trait_boundary() {
    let backend: &dyn RemoteSyncBackend = &ZstdPackBulkRemoteBackend;

    assert_eq!(backend.kind(), RemoteSyncBackendKind::ZstdPackBulk);
    assert_eq!(
        backend.required_capability(),
        REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK
    );
}

#[test]
fn zstd_push_and_pull_http_specs_use_current_routes_and_media_types() {
    let config = http_config();
    let plan = build_plan_remote_zstd_bulk_request_spec(
        &config,
        "repo",
        &json!({
            "snapshot_ids": ["SNP-1"],
            "object_packs": [],
            "tree_packs": []
        }),
    )
    .unwrap();
    assert_eq!(
        plan.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/plan"
    );

    let object_upload =
        build_put_remote_zstd_object_pack_request_spec(&config, "repo", "PCK-1", b"object")
            .unwrap();
    assert_eq!(
        object_upload
            .headers
            .get("Content-Type")
            .map(String::as_str),
        Some(ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE)
    );
    let tree_upload =
        build_put_remote_zstd_tree_pack_request_spec(&config, "repo", "TPK-1", b"tree").unwrap();
    assert_eq!(
        tree_upload.headers.get("Content-Type").map(String::as_str),
        Some(ZSTD_BULK_TREE_PACK_MEDIA_TYPE)
    );

    let commit = build_commit_remote_zstd_bulk_request_spec(
        &config,
        "repo",
        &json!({
            "contract": "ait.remote_sync.zstd_bulk.commit.v1",
            "object_packs": [],
            "tree_packs": [],
            "blob_locators": [],
            "tree_locators": [],
            "snapshots": []
        }),
    )
    .unwrap();
    assert_eq!(
        commit.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
    );

    let manifest =
        build_get_remote_zstd_import_manifest_request_spec(&config, "repo", "SNP-1").unwrap();
    assert_eq!(
        manifest.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/import-manifests/SNP-1"
    );
    let object_download =
        build_get_remote_zstd_object_pack_request_spec(&config, "repo", "PCK-1").unwrap();
    assert_eq!(
        object_download.headers.get("Accept").map(String::as_str),
        Some(ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE)
    );
    let tree_download =
        build_get_remote_zstd_tree_pack_request_spec(&config, "repo", "TPK-1").unwrap();
    assert_eq!(
        tree_download.headers.get("Accept").map(String::as_str),
        Some(ZSTD_BULK_TREE_PACK_MEDIA_TYPE)
    );
}
