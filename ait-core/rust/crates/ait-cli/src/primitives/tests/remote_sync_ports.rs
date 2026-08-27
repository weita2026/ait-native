use super::*;
use ait_core::binary_db::{BinaryDbCommandScope, LocalBinaryDbFs};
use ait_core::content_binary_db::{
    snapshot_id_from_hash48, BinaryDbContentWriteCoordinator, BinaryDbSnapshotWriteInput,
};
use ait_core::remote_sync_local_store::{
    RemoteSyncLocalStoreContext, RemoteSyncZstdImportSource, ZstdImportHistoryMode,
};
use ait_core::repository_pack_json::{
    JsonPayloadContract, ZstdBulkCommitRequest, ZstdImportManifestJson, ZstdImportManifestPayload,
    ZstdPullManifestJson, ZstdPullManifestPayload, ZSTD_IMPORT_MANIFEST_CONTRACT_NAME,
    ZSTD_PULL_MANIFEST_CONTRACT_NAME,
};
use ait_core::server_operational::RepositoryIndex;
use std::collections::{BTreeMap, BTreeSet};
use tempfile::TempDir;

fn zstd_only_capabilities() -> RemoteSyncCapabilities {
    RemoteSyncCapabilities::with_zstd_pack_bulk()
}

fn zstd_only_download_capabilities() -> RemoteSyncCapabilities {
    RemoteSyncCapabilities::with_zstd_pack_bulk_download()
}

fn unavailable_remote_sync_capabilities() -> RemoteSyncCapabilities {
    RemoteSyncCapabilities::default()
}

#[test]
fn external_hydration_rebinds_http_authority_to_the_resolved_repository() {
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_string(), "Bearer fixture".to_string());
    let consumer_config = PlanHttpClientConfig {
        base_url: "https://ait.example.test".to_string(),
        repository_index: Some(RepositoryIndex::new(2)),
        headers: headers.clone(),
        retry_attempts: 2,
        ..PlanHttpClientConfig::default()
    };

    let rebound = remote_sync::remote_repository_authority_http_config(
        consumer_config.clone(),
        &json!({
            "repository": {
                "repository_name": "ait-core",
                "repository_index": 0,
            }
        }),
        RepositoryIndex::new(0),
        "ait-core",
    )
    .expect("external repository authority should be accepted");

    assert_eq!(rebound.repository_index, Some(RepositoryIndex::new(0)));
    assert_eq!(rebound.base_url, consumer_config.base_url);
    assert_eq!(rebound.headers, headers);
    assert_eq!(rebound.retry_attempts, consumer_config.retry_attempts);

    let duplicate_name = remote_sync::remote_repository_authority_http_config(
        consumer_config.clone(),
        &json!({
            "repository": {
                "repository_name": "duplicate-display-name",
                "repository_index": 0,
            }
        }),
        RepositoryIndex::new(0),
        "ait-core",
    )
    .expect("Repository name is display data, not authority identity");
    assert_eq!(
        duplicate_name.repository_index,
        Some(RepositoryIndex::new(0))
    );
}

#[test]
fn external_hydration_repository_authority_fails_closed_on_index_drift() {
    let config = PlanHttpClientConfig {
        repository_index: Some(RepositoryIndex::new(2)),
        ..PlanHttpClientConfig::default()
    };

    for (repository, expected_message) in [
        (
            json!({"repository": {"repository_name": "ait-core"}}),
            "missing its canonical repository_index",
        ),
        (
            json!({"repository": {
                "repository_name": "ait-core",
                "repository_index": "0"
            }}),
            "must be an unsigned JSON integer",
        ),
        (
            json!({"repository": {
                "repository_name": "ait-core",
                "repository_index": 1
            }}),
            "repository index drifted",
        ),
    ] {
        let error = remote_sync::remote_repository_authority_http_config(
            config.clone(),
            &repository,
            RepositoryIndex::new(0),
            "ait-core",
        )
        .expect_err("invalid external repository authority must fail closed");
        assert!(error.contains(expected_message), "{error}");
    }
}

#[test]
fn remote_sync_pull_defensively_validates_workspace_options_before_remote_access() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");

    let err = remote_sync::pull(&repo, None, Some("main"), false, false, true)
        .expect_err("force without restore should fail before remote lookup");

    assert_eq!(err, "--force only applies together with --restore");

    let err = remote_sync::pull(&repo, None, Some("main"), true, false, false)
        .expect_err("merge without restore should fail before remote lookup");
    assert_eq!(
        err,
        "--merge requires --restore because a divergent merge changes workspace files."
    );

    let mut remote = FakeLineSnapshotRemote::default();
    let err = remote_sync::pull_line_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        true,
        false,
        false,
        &zstd_only_download_capabilities(),
    )
    .expect_err("direct pull primitive must require restore for merge");
    assert_eq!(
        err,
        "--merge requires --restore because a divergent merge changes workspace files."
    );
}

struct FakeRemoteSyncLocalSnapshotSource {
    rows: Vec<remote_sync::RemoteSyncLocalSnapshotParent>,
}

struct FakeRemoteSyncLocalInventorySource {
    calls: std::cell::RefCell<Vec<Vec<String>>>,
    metadata: remote_sync::RemoteSyncLocalInventoryMetadata,
}

struct FakeRemoteSyncZstdLocalPlanSource {
    calls: std::cell::RefCell<Vec<(Vec<String>, Vec<String>)>>,
    plan: remote_sync::ZstdBulkLocalPlan,
}

impl FakeRemoteSyncLocalInventorySource {
    fn new(metadata: remote_sync::RemoteSyncLocalInventoryMetadata) -> Self {
        Self {
            calls: std::cell::RefCell::new(Vec::new()),
            metadata,
        }
    }
}

impl FakeRemoteSyncZstdLocalPlanSource {
    fn new(plan: remote_sync::ZstdBulkLocalPlan) -> Self {
        Self {
            calls: std::cell::RefCell::new(Vec::new()),
            plan,
        }
    }
}

impl remote_sync::RemoteSyncLocalInventorySource for FakeRemoteSyncLocalInventorySource {
    fn snapshot_inventory_metadata(
        &self,
        _ctx: &remote_sync::RemoteSyncLocalStoreContext,
        snapshot_ids: &[String],
    ) -> Result<remote_sync::RemoteSyncLocalInventoryMetadata, String> {
        self.calls.borrow_mut().push(snapshot_ids.to_vec());
        Ok(self.metadata.clone())
    }
}

impl remote_sync::RemoteSyncZstdLocalPlanSource for FakeRemoteSyncZstdLocalPlanSource {
    fn zstd_bulk_local_plan(
        &self,
        _ctx: &remote_sync::RemoteSyncLocalStoreContext,
        snapshot_ids: &[String],
        present_set: &BTreeSet<String>,
    ) -> Result<remote_sync::ZstdBulkLocalPlan, String> {
        self.calls.borrow_mut().push((
            snapshot_ids.to_vec(),
            present_set.iter().cloned().collect::<Vec<_>>(),
        ));
        Ok(self.plan.clone())
    }
}

impl remote_sync::RemoteSyncLocalSnapshotSource for FakeRemoteSyncLocalSnapshotSource {
    fn snapshot_parent_rows(
        &self,
        _ctx: &remote_sync::RemoteSyncLocalStoreContext,
    ) -> Result<Vec<remote_sync::RemoteSyncLocalSnapshotParent>, String> {
        Ok(self.rows.clone())
    }

    fn snapshot_content_complete(
        &self,
        _ctx: &remote_sync::RemoteSyncLocalStoreContext,
        snapshot_id: &str,
    ) -> Result<bool, String> {
        Ok(self.rows.iter().any(|row| row.snapshot_id == snapshot_id))
    }
}

#[test]
fn remote_sync_snapshot_ordering_reads_through_local_snapshot_source_trait_object() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let source = FakeRemoteSyncLocalSnapshotSource {
        rows: vec![
            remote_sync::RemoteSyncLocalSnapshotParent {
                snapshot_id: "SNP-CHILD".to_string(),
                parent_snapshot_ids: vec!["SNP-PARENT".to_string()],
                primary_parent_snapshot_id: Some("SNP-PARENT".to_string()),
                parent_snapshot_id: Some("SNP-PARENT".to_string()),
            },
            remote_sync::RemoteSyncLocalSnapshotParent {
                snapshot_id: "SNP-PARENT".to_string(),
                parent_snapshot_ids: Vec::new(),
                primary_parent_snapshot_id: None,
                parent_snapshot_id: None,
            },
        ],
    };

    let ordered = remote_sync::local_repo_snapshot_ids_topological_with_source(&source, &repo)
        .expect("snapshot order");

    assert_eq!(ordered, ["SNP-PARENT", "SNP-CHILD"]);
}

#[test]
fn remote_sync_snapshot_ordering_reports_cycle_from_local_snapshot_source() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let source = FakeRemoteSyncLocalSnapshotSource {
        rows: vec![
            remote_sync::RemoteSyncLocalSnapshotParent {
                snapshot_id: "SNP-A".to_string(),
                parent_snapshot_ids: vec!["SNP-B".to_string()],
                primary_parent_snapshot_id: Some("SNP-B".to_string()),
                parent_snapshot_id: Some("SNP-B".to_string()),
            },
            remote_sync::RemoteSyncLocalSnapshotParent {
                snapshot_id: "SNP-B".to_string(),
                parent_snapshot_ids: vec!["SNP-A".to_string()],
                primary_parent_snapshot_id: Some("SNP-A".to_string()),
                parent_snapshot_id: Some("SNP-A".to_string()),
            },
        ],
    };

    let err = remote_sync::local_repo_snapshot_ids_topological_with_source(&source, &repo)
        .expect_err("cycle should fail");

    assert_eq!(err, "Cycle detected in Snapshot DAG at SNP-A.");
}

#[test]
fn remote_sync_multi_parent_capability_guard_fails_before_remote_mutation() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let source = FakeRemoteSyncLocalSnapshotSource {
        rows: vec![remote_sync::RemoteSyncLocalSnapshotParent {
            snapshot_id: "SNP-MERGE".to_string(),
            parent_snapshot_ids: vec!["SNP-LEFT".to_string(), "SNP-RIGHT".to_string()],
            primary_parent_snapshot_id: Some("SNP-LEFT".to_string()),
            parent_snapshot_id: Some("SNP-LEFT".to_string()),
        }],
    };
    let error = remote_sync::require_snapshot_dag_upload_capability_with_source(
        &source,
        &repo,
        &["SNP-MERGE".to_string()],
        &RemoteSyncCapabilities::with_zstd_pack_bulk(),
    )
    .expect_err("remote without DAG capability must fail before any uploader is called");
    assert!(error.contains(REMOTE_SYNC_CAPABILITY_SNAPSHOT_DAG_V2));
    assert!(error.contains("before mutation"));

    remote_sync::require_snapshot_dag_upload_capability_with_source(
        &source,
        &repo,
        &["SNP-MERGE".to_string()],
        &RemoteSyncCapabilities::with_zstd_pack_bulk().with_snapshot_dag_v2(),
    )
    .expect("DAG-capable remote accepts multi-parent snapshot metadata");
    remote_sync::require_snapshot_dag_upload_capability_with_source(
        &source,
        &repo,
        &["SNP-UNRELATED".to_string()],
        &RemoteSyncCapabilities::default(),
    )
    .expect("linear or unrelated upload does not require DAG capability");
}

#[test]
fn remote_sync_inventory_reads_through_local_inventory_source_trait_object() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let source =
        FakeRemoteSyncLocalInventorySource::new(remote_sync::RemoteSyncLocalInventoryMetadata {
            object_pack_formats: BTreeSet::from([
                ait_core::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
            ]),
            tree_pack_formats: BTreeSet::from([
                ait_core::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1.to_string(),
            ]),
        });
    let snapshot_ids = vec!["SNP-A".to_string(), "SNP-B".to_string()];

    let inventory = remote_sync::local_remote_sync_inventory_for_snapshots_with_source(
        &source,
        &repo,
        &snapshot_ids,
    )
    .expect("inventory");

    assert_eq!(
        source.calls.borrow().as_slice(),
        std::slice::from_ref(&snapshot_ids)
    );
    assert_eq!(inventory.snapshot_ids, snapshot_ids);
    assert!(inventory
        .object_pack_formats
        .contains(ait_core::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1));
    assert!(inventory
        .tree_pack_formats
        .contains(ait_core::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1));
}

#[test]
fn remote_sync_zstd_local_plan_reads_through_local_plan_source_trait_object() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let returned_plan = remote_sync::ZstdBulkLocalPlan {
        snapshot_order: vec!["SNP-B".to_string()],
        snapshots: BTreeMap::from([(
            "SNP-B".to_string(),
            json!({
                "snapshot_id": "SNP-B",
                "root_tree_pack_id": "TP-1",
                "root_entry_ordinal": 0,
            }),
        )]),
        object_packs: BTreeMap::from([(
            "OP-1".to_string(),
            remote_sync::ZstdBulkLocalPack {
                pack_id: "OP-1".to_string(),
                pack_abs_path: PathBuf::from("/tmp/op-1.pack"),
                metadata: json!({"pack_id": "OP-1"}),
            },
        )]),
        tree_packs: BTreeMap::new(),
        tree_pack_order: Vec::new(),
        blob_locators: BTreeMap::new(),
        tree_locators: BTreeMap::new(),
    };
    let source = FakeRemoteSyncZstdLocalPlanSource::new(returned_plan);
    let snapshot_ids = vec!["SNP-A".to_string(), "SNP-B".to_string()];
    let present_set = BTreeSet::from(["SNP-A".to_string()]);

    let plan = remote_sync::build_zstd_bulk_local_plan_with_source(
        &source,
        &repo,
        &snapshot_ids,
        &present_set,
    )
    .expect("zstd local plan");

    assert_eq!(
        source.calls.borrow().as_slice(),
        &[(snapshot_ids, vec!["SNP-A".to_string()])]
    );
    assert_eq!(plan.snapshot_order, ["SNP-B"]);
    assert!(plan.snapshots.contains_key("SNP-B"));
    assert_eq!(plan.object_packs["OP-1"].pack_id, "OP-1");
}

#[test]
fn remote_sync_orders_delta_base_object_packs_before_dependents() {
    let plan = remote_sync::ZstdBulkLocalPlan {
        snapshot_order: Vec::new(),
        snapshots: BTreeMap::new(),
        object_packs: BTreeMap::from([
            (
                "PCK-DEPENDENT".to_string(),
                remote_sync::ZstdBulkLocalPack {
                    pack_id: "PCK-DEPENDENT".to_string(),
                    pack_abs_path: PathBuf::from("/tmp/dependent.zstpack"),
                    metadata: json!({"pack_id": "PCK-DEPENDENT"}),
                },
            ),
            (
                "PCK-BASE".to_string(),
                remote_sync::ZstdBulkLocalPack {
                    pack_id: "PCK-BASE".to_string(),
                    pack_abs_path: PathBuf::from("/tmp/base.zstpack"),
                    metadata: json!({"pack_id": "PCK-BASE"}),
                },
            ),
        ]),
        tree_packs: BTreeMap::new(),
        tree_pack_order: Vec::new(),
        blob_locators: BTreeMap::from([
            (
                "BLB-BASE".to_string(),
                json!({"blob_id": "BLB-BASE", "pack_id": "PCK-BASE"}),
            ),
            (
                "BLB-DELTA".to_string(),
                json!({
                    "blob_id": "BLB-DELTA",
                    "pack_id": "PCK-DEPENDENT",
                    "pack_base_blob_id": "BLB-BASE"
                }),
            ),
        ]),
        tree_locators: BTreeMap::new(),
    };

    let ordered = remote_sync::ordered_object_pack_metadata(&plan).expect("ordered packs");
    assert_eq!(ordered[0]["pack_id"], "PCK-BASE");
    assert_eq!(ordered[1]["pack_id"], "PCK-DEPENDENT");
}

#[test]
fn remote_sync_commit_plan_reuses_the_negotiated_local_plan_without_rescanning_storage() {
    let repo_tmp = tempdir().expect("repo tempdir");
    init_repo(&InitRequest {
        root: repo_tmp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_tmp.path()).expect("repo runtime");
    let pack = |pack_id: &str| remote_sync::ZstdBulkLocalPack {
        pack_id: pack_id.to_string(),
        pack_abs_path: PathBuf::from(format!("/tmp/{pack_id}.pack")),
        metadata: json!({"pack_id": pack_id}),
    };
    let planned = remote_sync::ZstdBulkLocalPlan {
        snapshot_order: vec!["SNP-A".to_string(), "SNP-B".to_string()],
        snapshots: BTreeMap::from([
            ("SNP-A".to_string(), json!({"snapshot_id": "SNP-A"})),
            ("SNP-B".to_string(), json!({"snapshot_id": "SNP-B"})),
        ]),
        object_packs: BTreeMap::from([
            ("OP-A".to_string(), pack("OP-A")),
            ("OP-B".to_string(), pack("OP-B")),
        ]),
        tree_packs: BTreeMap::from([
            ("TP-A".to_string(), pack("TP-A")),
            ("TP-B".to_string(), pack("TP-B")),
        ]),
        tree_pack_order: vec!["TP-A".to_string(), "TP-B".to_string()],
        blob_locators: BTreeMap::from([
            (
                "BLB-A".to_string(),
                json!({"blob_id": "BLB-A", "pack_id": "OP-A"}),
            ),
            (
                "BLB-B".to_string(),
                json!({"blob_id": "BLB-B", "pack_id": "OP-B"}),
            ),
        ]),
        tree_locators: BTreeMap::from([
            (
                "TRE-A".to_string(),
                json!({"tree_id": "TRE-A", "tree_pack_id": "TP-A"}),
            ),
            (
                "TRE-B".to_string(),
                json!({"tree_id": "TRE-B", "tree_pack_id": "TP-B"}),
            ),
        ]),
    };

    // The fixture repository does not contain either synthetic snapshot. This
    // succeeds only when the already-built negotiated plan is reused instead
    // of opening storage and rebuilding the closure a second time.
    let commit = remote_sync::zstd_bulk_commit_local_plan(
        &repo,
        &planned,
        &["SNP-A".to_string(), "SNP-B".to_string()],
        &["SNP-B".to_string()],
        &["OP-B".to_string()],
        &["TP-B".to_string()],
    )
    .expect("reuse negotiated commit plan");

    assert_eq!(commit.snapshot_order, ["SNP-B"]);
    assert_eq!(
        commit.snapshots.keys().cloned().collect::<Vec<_>>(),
        ["SNP-B"]
    );
    assert_eq!(
        commit.object_packs.keys().cloned().collect::<Vec<_>>(),
        ["OP-B"]
    );
    assert_eq!(
        commit.tree_packs.keys().cloned().collect::<Vec<_>>(),
        ["TP-B"]
    );
    assert_eq!(commit.tree_pack_order, ["TP-B"]);
    assert_eq!(
        commit.blob_locators.keys().cloned().collect::<Vec<_>>(),
        ["BLB-B"]
    );
    assert_eq!(
        commit.tree_locators.keys().cloned().collect::<Vec<_>>(),
        ["TRE-B"]
    );
}

#[test]
fn remote_sync_preserves_delta_pack_when_base_is_already_remote() {
    let plan = remote_sync::ZstdBulkLocalPlan {
        snapshot_order: Vec::new(),
        snapshots: BTreeMap::new(),
        object_packs: BTreeMap::from([(
            "PCK-DEPENDENT".to_string(),
            remote_sync::ZstdBulkLocalPack {
                pack_id: "PCK-DEPENDENT".to_string(),
                pack_abs_path: PathBuf::from("/tmp/dependent.zstpack"),
                metadata: json!({"pack_id": "PCK-DEPENDENT"}),
            },
        )]),
        tree_packs: BTreeMap::new(),
        tree_pack_order: Vec::new(),
        blob_locators: BTreeMap::from([(
            "BLB-DELTA".to_string(),
            json!({
                "blob_id": "BLB-DELTA",
                "pack_id": "PCK-DEPENDENT",
                "pack_base_blob_id": "BLB-MISSING"
            }),
        )]),
        tree_locators: BTreeMap::new(),
    };

    let ordered = remote_sync::ordered_object_pack_metadata(&plan).expect("ordered packs");
    assert_eq!(ordered.len(), 1);
    assert_eq!(ordered[0]["pack_id"], "PCK-DEPENDENT");
}

#[test]
fn remote_sync_accepts_physical_object_pack_member_selected_from_another_pack() {
    let plan = remote_sync::ZstdBulkLocalPlan {
        snapshot_order: Vec::new(),
        snapshots: BTreeMap::new(),
        object_packs: BTreeMap::from([(
            "PCK-PARTIAL".to_string(),
            remote_sync::ZstdBulkLocalPack {
                pack_id: "PCK-PARTIAL".to_string(),
                pack_abs_path: PathBuf::from("/tmp/partial.zstpack"),
                metadata: json!({
                    "pack_id": "PCK-PARTIAL",
                    "pack_index": {
                        "entries": [
                            {"blob_id": "BLB-A"},
                            {"blob_id": "BLB-B"}
                        ]
                    }
                }),
            },
        )]),
        tree_packs: BTreeMap::new(),
        tree_pack_order: Vec::new(),
        blob_locators: BTreeMap::from([(
            "BLB-A".to_string(),
            json!({"blob_id": "BLB-A", "pack_id": "PCK-PARTIAL"}),
        )]),
        tree_locators: BTreeMap::new(),
    };

    remote_sync::validate_zstd_object_pack_locator_coverage(&plan)
        .expect("a physical duplicate may have its canonical locator in another pack");
}

#[test]
fn remote_sync_rejects_locator_selecting_pack_without_physical_member() {
    let plan = remote_sync::ZstdBulkLocalPlan {
        snapshot_order: Vec::new(),
        snapshots: BTreeMap::new(),
        object_packs: BTreeMap::from([(
            "PCK-SELECTED".to_string(),
            remote_sync::ZstdBulkLocalPack {
                pack_id: "PCK-SELECTED".to_string(),
                pack_abs_path: PathBuf::from("/tmp/selected.zstpack"),
                metadata: json!({
                    "pack_id": "PCK-SELECTED",
                    "pack_index": {
                        "entries": [{"blob_id": "BLB-A"}]
                    }
                }),
            },
        )]),
        tree_packs: BTreeMap::new(),
        tree_pack_order: Vec::new(),
        blob_locators: BTreeMap::from([(
            "BLB-MISSING".to_string(),
            json!({"blob_id": "BLB-MISSING", "pack_id": "PCK-SELECTED"}),
        )]),
        tree_locators: BTreeMap::new(),
    };

    let error = remote_sync::validate_zstd_object_pack_locator_coverage(&plan)
        .expect_err("a selected locator must name a physical member of its pack");
    assert!(error.contains("members absent from the physical pack"));
    assert!(error.contains("BLB-MISSING"));
}

#[test]
fn remote_sync_rejects_duplicate_physical_object_pack_members() {
    let plan = remote_sync::ZstdBulkLocalPlan {
        snapshot_order: Vec::new(),
        snapshots: BTreeMap::new(),
        object_packs: BTreeMap::from([(
            "PCK-DUPLICATE".to_string(),
            remote_sync::ZstdBulkLocalPack {
                pack_id: "PCK-DUPLICATE".to_string(),
                pack_abs_path: PathBuf::from("/tmp/duplicate.zstpack"),
                metadata: json!({
                    "pack_id": "PCK-DUPLICATE",
                    "pack_index": {
                        "entries": [
                            {"blob_id": "BLB-A"},
                            {"blob_id": "BLB-A"}
                        ]
                    }
                }),
            },
        )]),
        tree_packs: BTreeMap::new(),
        tree_pack_order: Vec::new(),
        blob_locators: BTreeMap::from([(
            "BLB-A".to_string(),
            json!({"blob_id": "BLB-A", "pack_id": "PCK-DUPLICATE"}),
        )]),
        tree_locators: BTreeMap::new(),
    };

    let error = remote_sync::validate_zstd_object_pack_locator_coverage(&plan)
        .expect_err("duplicate physical blob members remain invalid");
    assert!(error.contains("duplicate blob entry BLB-A"));
}

#[test]
fn remote_sync_uses_planned_child_first_tree_pack_order_without_pack_scan() {
    const PARENT_PACK_ID: &str = "TPK-000000000001";
    const CHILD_PACK_ID: &str = "TPK-FFFFFFFFFFFF";
    const PARENT_TREE_ID: &str = "TRE-00000000000000000001";
    const CHILD_TREE_ID: &str = "TRE-FFFFFFFFFFFFFFFFFFFF";
    let plan = remote_sync::ZstdBulkLocalPlan {
        snapshot_order: Vec::new(),
        snapshots: BTreeMap::new(),
        object_packs: BTreeMap::new(),
        tree_packs: BTreeMap::from([
            (
                PARENT_PACK_ID.to_string(),
                remote_sync::ZstdBulkLocalPack {
                    pack_id: PARENT_PACK_ID.to_string(),
                    pack_abs_path: PathBuf::from("/does-not-exist/parent.zstpack"),
                    metadata: json!({"pack_id": PARENT_PACK_ID}),
                },
            ),
            (
                CHILD_PACK_ID.to_string(),
                remote_sync::ZstdBulkLocalPack {
                    pack_id: CHILD_PACK_ID.to_string(),
                    pack_abs_path: PathBuf::from("/does-not-exist/child.zstpack"),
                    metadata: json!({"pack_id": CHILD_PACK_ID}),
                },
            ),
        ]),
        tree_pack_order: vec![CHILD_PACK_ID.to_string(), PARENT_PACK_ID.to_string()],
        blob_locators: BTreeMap::new(),
        tree_locators: BTreeMap::from([
            (
                PARENT_TREE_ID.to_string(),
                json!({"tree_id": PARENT_TREE_ID, "tree_pack_id": PARENT_PACK_ID}),
            ),
            (
                CHILD_TREE_ID.to_string(),
                json!({"tree_id": CHILD_TREE_ID, "tree_pack_id": CHILD_PACK_ID}),
            ),
        ]),
    };

    let ordered = remote_sync::ordered_tree_pack_metadata(&plan).expect("ordered tree packs");
    assert_eq!(ordered[0]["pack_id"], CHILD_PACK_ID);
    assert_eq!(ordered[1]["pack_id"], PARENT_PACK_ID);

    let mut invalid_order = plan;
    invalid_order.tree_pack_order[0] = "TPK-EEEEEEEEEEEE".to_string();
    let error = remote_sync::ordered_tree_pack_metadata(&invalid_order)
        .expect_err("unknown ordered tree pack must fail during local preflight");
    assert!(error.contains("references unknown pack"));
}

fn init_remote_sync_backend_dispatch_repo() -> (tempfile::TempDir, RepoRuntime) {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    (repo_tmp, repo)
}

#[derive(Default)]
struct FakeRemoteSyncBackend {
    remote_context_calls: usize,
    pull_line_calls: usize,
    push_line_calls: usize,
    upload_snapshot_chain_calls: usize,
    sync_patchset_revision_snapshot_calls: usize,
}

impl remote_sync::RemoteSyncBackend for FakeRemoteSyncBackend {
    fn remote_context(
        &mut self,
        _repo: &RepoRuntime,
        remote_name: Option<&str>,
    ) -> Result<(RemoteRow, String), String> {
        self.remote_context_calls += 1;
        let name = remote_name.unwrap_or("origin").to_string();
        Ok((
            RemoteRow {
                name,
                url: "https://ait.example".to_string(),
                repo_name: Some("fixture-ait".to_string()),
            },
            "fixture-ait".to_string(),
        ))
    }

    fn pull_line(
        &mut self,
        _repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
        merge: bool,
        restore: bool,
        force: bool,
    ) -> Result<JsonValue, String> {
        self.pull_line_calls += 1;
        Ok(json!({
            "dispatch": "pull_line",
            "remote": remote_row.name,
            "repo_name": repo_name,
            "line": line_name,
            "merge": merge,
            "restore": restore,
            "force": force,
        }))
    }

    fn push_line(
        &mut self,
        _repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
    ) -> Result<JsonValue, String> {
        self.push_line_calls += 1;
        Ok(json!({
            "dispatch": "push_line",
            "remote": remote_row.name,
            "repo_name": repo_name,
            "line": line_name,
        }))
    }

    fn upload_snapshot_chain(
        &mut self,
        _repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        snapshot_id: &str,
        line_name: Option<&str>,
        reason: Option<&str>,
    ) -> Result<JsonValue, String> {
        self.upload_snapshot_chain_calls += 1;
        Ok(json!({
            "dispatch": "upload_snapshot_chain",
            "remote": remote_row.name,
            "repo_name": repo_name,
            "snapshot_id": snapshot_id,
            "line": line_name,
            "reason": reason,
        }))
    }

    fn sync_patchset_revision_snapshot(
        &mut self,
        _repo: &RepoRuntime,
        remote_row: &RemoteRow,
        repo_name: &str,
        line_name: &str,
        revision_snapshot_id: &str,
        base_line: &str,
    ) -> Result<JsonValue, String> {
        self.sync_patchset_revision_snapshot_calls += 1;
        Ok(json!({
            "dispatch": "sync_patchset_revision_snapshot",
            "remote": remote_row.name,
            "repo_name": repo_name,
            "line": line_name,
            "revision_snapshot_id": revision_snapshot_id,
            "base_line": base_line,
        }))
    }
}

#[test]
fn remote_sync_mutation_dispatch_accepts_substitute_backend() {
    let (_repo_tmp, repo) = init_remote_sync_backend_dispatch_repo();
    let mut backend = FakeRemoteSyncBackend::default();

    let pulled = remote_sync::pull_with_remote_sync_backend(
        &mut backend,
        &repo,
        Some("backup"),
        Some("feature/sync"),
        false,
        true,
        true,
    )
    .expect("dispatch pull to fake backend");
    assert_eq!(pulled["dispatch"], json!("pull_line"));
    assert_eq!(pulled["restore"], json!(true));
    assert_eq!(pulled["force"], json!(true));

    let pushed = remote_sync::push_with_remote_sync_backend(
        &mut backend,
        &repo,
        Some("backup"),
        Some("feature/sync"),
    )
    .expect("dispatch push to fake backend");
    assert_eq!(pushed["dispatch"], json!("push_line"));

    let uploaded = remote_sync::upload_snapshot_chain_with_remote_sync_backend(
        &mut backend,
        &repo,
        Some("backup"),
        "SNP-REMOTE",
        Some("feature/sync"),
        Some("patchset revision"),
    )
    .expect("dispatch upload snapshot chain to fake backend");
    assert_eq!(uploaded["dispatch"], json!("upload_snapshot_chain"));
    assert_eq!(uploaded["snapshot_id"], json!("SNP-REMOTE"));
    assert_eq!(uploaded["line"], json!("feature/sync"));
    assert_eq!(uploaded["reason"], json!("patchset revision"));

    let remote_row = RemoteRow {
        name: "backup".to_string(),
        url: "https://ait.example".to_string(),
        repo_name: Some("fixture-ait".to_string()),
    };
    let synced = remote_sync::sync_patchset_revision_snapshot_with_remote_sync_backend(
        &mut backend,
        &repo,
        &remote_row,
        "fixture-ait",
        "feature/sync",
        "SNP-REVISION",
        "main",
    )
    .expect("dispatch patchset revision snapshot sync to fake backend");
    assert_eq!(synced["dispatch"], json!("sync_patchset_revision_snapshot"));
    assert_eq!(synced["revision_snapshot_id"], json!("SNP-REVISION"));
    assert_eq!(synced["base_line"], json!("main"));

    assert_eq!(backend.remote_context_calls, 3);
    assert_eq!(backend.pull_line_calls, 1);
    assert_eq!(backend.push_line_calls, 1);
    assert_eq!(backend.upload_snapshot_chain_calls, 1);
    assert_eq!(backend.sync_patchset_revision_snapshot_calls, 1);
}
#[test]
fn remote_sync_snapshot_chain_upload_accepts_line_and_snapshot_remote_traits() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "snapshot chain upload").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("snapshot chain upload fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote::default();

    let uploaded = remote_sync::upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "fixture-ait",
        &snapshot_id,
        Some("main"),
        &zstd_only_capabilities(),
    )
    .expect("upload snapshot chain through reusable flow");

    assert_eq!(uploaded["repo_name"], json!("fixture-ait"));
    assert_eq!(uploaded["checked_snapshots"], json!(1));
    assert_eq!(uploaded["uploaded_snapshots"], json!(1));
    assert_eq!(uploaded["skipped_snapshots"], json!(0));
    assert!(uploaded["remote_head_snapshot_id"].is_null());
    assert_eq!(
        uploaded["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 1);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 1);

    remote.remote_snapshots.insert(
        snapshot_id.clone(),
        json!({
            "repo_name": "fixture-ait",
            "snapshot_id": snapshot_id,
        }),
    );
    let skipped = remote_sync::upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "fixture-ait",
        &snapshot_id,
        None,
        &zstd_only_capabilities(),
    )
    .expect("present remote snapshot should be skipped");
    assert_eq!(skipped["checked_snapshots"], json!(1));
    assert_eq!(skipped["uploaded_snapshots"], json!(0));
    assert_eq!(skipped["skipped_snapshots"], json!(1));
    assert_eq!(remote.zstd_plan_requests.len(), 2);
    assert_eq!(remote.zstd_commit_requests.len(), 2);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 1);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 1);

    remote.lines = vec![json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": snapshot_id
    })];
    let already_current =
        remote_sync::upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
            &repo,
            &mut remote,
            "fixture-ait",
            &snapshot_id,
            Some("main"),
            &zstd_only_capabilities(),
        )
        .expect("already-current remote line should require no upload");
    assert_eq!(already_current["checked_snapshots"], json!(0));
    assert_eq!(already_current["uploaded_snapshots"], json!(0));
    assert_eq!(already_current["skipped_snapshots"], json!(0));
    assert_eq!(
        already_current["remote_head_snapshot_id"],
        json!(snapshot_id)
    );
    assert_eq!(remote.zstd_plan_requests.len(), 2);
    assert_eq!(remote.zstd_commit_requests.len(), 2);
}

#[test]
fn remote_sync_push_line_accepts_line_and_snapshot_remote_traits() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "push line").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("push line fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote::default();

    let pushed = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect("push line through reusable flow");

    assert_eq!(pushed["remote"], json!("origin"));
    assert_eq!(pushed["repo_name"], json!("fixture-ait"));
    assert_eq!(pushed["line"], json!("main"));
    assert_eq!(pushed["head_snapshot_id"], json!(snapshot_id));
    assert_eq!(pushed["checked_snapshots"], json!(1));
    assert_eq!(pushed["uploaded_snapshots"], json!(1));
    assert_eq!(pushed["skipped_snapshots"], json!(0));
    assert_eq!(pushed["pushed_snapshots"], json!(1));
    assert_eq!(
        pushed["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(
        pushed["remote_line"]["head_snapshot_id"],
        json!(snapshot_id)
    );
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 1);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 1);

    let no_op = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect("already-current remote line should be reusable");
    assert_eq!(no_op["checked_snapshots"], json!(0));
    assert_eq!(no_op["uploaded_snapshots"], json!(0));
    assert_eq!(no_op["skipped_snapshots"], json!(0));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
}

#[test]
fn remote_sync_push_missing_feature_line_with_present_head_uses_line_update_only() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "feature line push").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("feature line push fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    remote_sync::set_or_create_local_line_head(&repo, "feature/rt-1", Some(&snapshot_id))
        .expect("create local feature line");
    let mut remote = FakeLineSnapshotRemote {
        remote_snapshots: BTreeMap::from([(
            snapshot_id.clone(),
            json!({
                "repo_name": "fixture-ait",
                "snapshot_id": snapshot_id.clone(),
            }),
        )]),
        ..FakeLineSnapshotRemote::default()
    };

    let pushed = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "feature/rt-1",
        &zstd_only_capabilities(),
    )
    .expect("present snapshot should allow line-only feature push");

    assert_eq!(pushed["remote"], json!("origin"));
    assert_eq!(pushed["repo_name"], json!("fixture-ait"));
    assert_eq!(pushed["line"], json!("feature/rt-1"));
    assert_eq!(pushed["head_snapshot_id"], json!(snapshot_id));
    assert_eq!(pushed["checked_snapshots"], json!(1));
    assert_eq!(pushed["uploaded_snapshots"], json!(0));
    assert_eq!(pushed["skipped_snapshots"], json!(1));
    assert_eq!(pushed["pushed_snapshots"], json!(0));
    assert_eq!(pushed["sync_scope"], json!("line_only"));
    assert_eq!(
        pushed["sync_reason"],
        json!("remote_line_missing_head_snapshot_present")
    );
    assert_eq!(
        pushed["remote_line"]["head_snapshot_id"],
        json!(snapshot_id)
    );
    assert_eq!(remote.line_update_calls, 1);
    assert_eq!(remote.zstd_plan_requests.len(), 0);
    assert_eq!(remote.zstd_commit_requests.len(), 0);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 0);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 0);
}

#[test]
fn remote_sync_push_stale_feature_line_with_present_head_uses_line_update_only() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "base line push").expect("fixture file");
    let base_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("base line push fixture"),
        false,
    )
    .expect("create base snapshot");
    let base_snapshot_id =
        required_string_field(&base_snapshot, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "feature line push").expect("fixture file");
    let feature_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("feature line push fixture"),
        false,
    )
    .expect("create feature snapshot");
    let feature_snapshot_id =
        required_string_field(&feature_snapshot, "snapshot_id").expect("feature snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    remote_sync::set_or_create_local_line_head(&repo, "feature/rt-1", Some(&feature_snapshot_id))
        .expect("create local feature line");
    let mut remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "feature/rt-1",
            "status": "active",
            "head_snapshot_id": base_snapshot_id.clone(),
        })],
        remote_snapshots: BTreeMap::from([(
            feature_snapshot_id.clone(),
            json!({
                "repo_name": "fixture-ait",
                "snapshot_id": feature_snapshot_id.clone(),
            }),
        )]),
        ..FakeLineSnapshotRemote::default()
    };

    let pushed = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "feature/rt-1",
        &zstd_only_capabilities(),
    )
    .expect("present snapshot should allow stale feature line update");

    assert_eq!(pushed["remote"], json!("origin"));
    assert_eq!(pushed["repo_name"], json!("fixture-ait"));
    assert_eq!(pushed["line"], json!("feature/rt-1"));
    assert_eq!(pushed["head_snapshot_id"], json!(feature_snapshot_id));
    assert_eq!(pushed["checked_snapshots"], json!(1));
    assert_eq!(pushed["uploaded_snapshots"], json!(0));
    assert_eq!(pushed["skipped_snapshots"], json!(1));
    assert_eq!(pushed["pushed_snapshots"], json!(0));
    assert_eq!(pushed["sync_scope"], json!("line_only"));
    assert_eq!(
        pushed["sync_reason"],
        json!("remote_line_stale_head_snapshot_present")
    );
    assert_eq!(pushed["remote_head_snapshot_id"], json!(base_snapshot_id));
    assert_eq!(
        pushed["remote_line"]["head_snapshot_id"],
        json!(feature_snapshot_id)
    );
    assert_eq!(remote.line_update_calls, 1);
    assert_eq!(remote.zstd_plan_requests.len(), 0);
    assert_eq!(remote.zstd_commit_requests.len(), 0);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 0);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 0);
}

#[test]
fn remote_sync_solo_local_push_rejects_initialized_default_line_advance() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "base").expect("base fixture file");
    let base = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("base fixture"),
        false,
    )
    .expect("create base snapshot");
    let base_snapshot_id = required_string_field(&base, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "local landed head").expect("head fixture file");
    let head = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local landed head fixture"),
        false,
    )
    .expect("create head snapshot");
    let head_snapshot_id = required_string_field(&head, "snapshot_id").expect("head snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    assert_eq!(repo.effective_workflow_mode(), "solo_local");
    let mut remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": base_snapshot_id.clone(),
        })],
        ..FakeLineSnapshotRemote::default()
    };

    let error = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect_err("solo-local push must not advance an initialized remote default Line");

    assert!(error.contains("Refusing to advance initialized remote `origin` target Line `main`"));
    assert!(error.contains("only authoritative remote Task Land may move this governed Line"));
    assert!(error.contains("ait workflow ready <local-change-id> --apply --remote origin"));
    assert!(error.contains(&base_snapshot_id));
    assert!(error.contains(&head_snapshot_id));
    assert_eq!(remote.line_update_calls, 0);
    assert_eq!(remote.zstd_plan_requests.len(), 0);
    assert_eq!(remote.zstd_commit_requests.len(), 0);
    assert_eq!(remote.lines[0]["head_snapshot_id"], json!(base_snapshot_id));
}

#[test]
fn remote_sync_solo_local_push_rejects_null_default_line_initialization() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "base").expect("base fixture file");
    let base = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("base fixture"),
        false,
    )
    .expect("create base snapshot");
    let base_snapshot_id = required_string_field(&base, "snapshot_id").expect("base snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let task_store = repo.task_store().expect("task store");
    let change_store = repo.change_store().expect("change store");
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Completed local Task",
        "Require governed remote promotion",
        None,
        None,
        None,
        None,
    )
    .expect("create local Task");
    let task_id = required_string_field(&task, "task_id").expect("task id");
    let change = change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        &task_id,
        "Completed local Change",
        "main",
        None,
        Some(&base_snapshot_id),
    )
    .expect("create local Change");
    let change_ref = required_string_field(&change, "change_ref").expect("change ref");
    fs::write(repo_root.join("src.txt"), "completed local head").expect("head fixture file");
    let head = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("completed local head fixture"),
        false,
    )
    .expect("create local head snapshot");
    let head_snapshot_id = required_string_field(&head, "snapshot_id").expect("head snapshot id");
    workflow_local_change_land_with_change_store(
        &change_store,
        &change_ref,
        "main",
        &head_snapshot_id,
        Some(&base_snapshot_id),
    )
    .expect("land local Change");
    workflow_local_task_close_with_task_store(&task_store, &task_id, "completed")
        .expect("complete local Task");
    assert_eq!(repo.effective_workflow_mode(), "solo_local");
    let mut remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": null,
        })],
        ..FakeLineSnapshotRemote::default()
    };

    let error = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect_err("solo-local push must not initialize a null remote default Line");

    assert!(error.contains("Refusing to initialize null remote `origin` target Line `main`"));
    assert!(error.contains("only authoritative remote Task Land may move this governed Line"));
    assert!(error.contains("ait workflow ready <local-change-id> --apply --remote origin"));
    assert!(error.contains("none"));
    assert!(error.contains(&head_snapshot_id));
    assert!(error.contains(&change_ref));
    assert_eq!(remote.line_update_calls, 0);
    assert_eq!(remote.zstd_plan_requests.len(), 0);
    assert_eq!(remote.zstd_commit_requests.len(), 0);
    assert!(remote.lines[0]["head_snapshot_id"].is_null());
}

fn rewrite_repo_packs_as_zstd(repo_root: &Path, _snapshot_id: &str) {
    set_repo_pack_format_kinds(repo_root, 1);
}

fn set_repo_pack_format_kinds(repo_root: &Path, format_kind: u8) {
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let content = repo.binary_db_stores::<1>().content();
    let object_packs = content.object_packs();
    let mut write = object_packs
        .begin_write_txn(BinaryDbCommandScope::ContentWrite)
        .expect("begin Binary DB pack metadata rewrite");

    let object_pack_file = ait_core::content_binary_db::BinaryDbObjectPackStore::<
        LocalBinaryDbFs,
        1,
    >::object_pack_file();
    let object_pack_count = write
        .record_count(object_pack_file.clone())
        .expect("count Binary DB object packs");
    for index in 0..object_pack_count {
        let mut record = ait_core::content_binary_db::BinaryObjectPackCodec::<1>::decode_record(
            &write
                .read_record(object_pack_file.clone(), index)
                .expect("read Binary DB object pack"),
        )
        .expect("decode Binary DB object pack");
        record.pack_format_kind = format_kind;
        write
            .overwrite_record(
                object_pack_file.clone(),
                index,
                &ait_core::content_binary_db::BinaryObjectPackCodec::<1>::encode_record(&record)
                    .expect("encode Binary DB object pack"),
            )
            .expect("rewrite Binary DB object pack");
    }

    let tree_pack_file =
        ait_core::content_binary_db::BinaryDbTreePackStore::<LocalBinaryDbFs, 1>::tree_pack_file();
    let tree_pack_count = write
        .record_count(tree_pack_file.clone())
        .expect("count Binary DB tree packs");
    for index in 0..tree_pack_count {
        let mut record = ait_core::content_binary_db::BinaryTreePackCodec::<1>::decode_record(
            &write
                .read_record(tree_pack_file.clone(), index)
                .expect("read Binary DB tree pack"),
        )
        .expect("decode Binary DB tree pack");
        record.pack_format_kind = format_kind;
        write
            .overwrite_record(
                tree_pack_file.clone(),
                index,
                &ait_core::content_binary_db::BinaryTreePackCodec::<1>::encode_record(&record)
                    .expect("encode Binary DB tree pack"),
            )
            .expect("rewrite Binary DB tree pack");
    }

    write
        .commit()
        .expect("commit Binary DB pack metadata rewrite");
}
fn remove_manifest_forbidden_fields(row: &mut JsonValue, fields: &[&str]) {
    if let Some(object) = row.as_object_mut() {
        for field in fields {
            object.remove(*field);
        }
    }
}

fn zstd_pack_entry_names_by_blob_id(
    local_plan: &remote_sync::ZstdBulkLocalPlan,
) -> BTreeMap<String, String> {
    let mut entry_names = BTreeMap::new();
    for pack in local_plan.object_packs.values() {
        let Some(entries) = pack
            .metadata
            .get("pack_index")
            .and_then(|index| index.get("entries"))
            .and_then(JsonValue::as_array)
        else {
            continue;
        };
        for entry in entries {
            let Some(blob_id) = entry.get("blob_id").and_then(JsonValue::as_str) else {
                continue;
            };
            let Some(entry_name) = entry.get("entry_name").and_then(JsonValue::as_str) else {
                continue;
            };
            entry_names.insert(blob_id.to_string(), entry_name.to_string());
        }
    }
    entry_names
}

fn snapshot_blob_ids(repo: &RepoRuntime, snapshot_id: &str) -> BTreeSet<String> {
    repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content()
        .snapshot_tree_file_rows(Some(snapshot_id))
        .expect("Binary DB snapshot tree file rows")
        .into_iter()
        .map(|row| row.blob_id)
        .collect()
}

fn zstd_import_manifest_from_local_plan(
    repo: &RepoRuntime,
    repo_name: &str,
    snapshot_id: &str,
    local_plan: &remote_sync::ZstdBulkLocalPlan,
) -> ZstdImportManifestPayload {
    let entry_names = zstd_pack_entry_names_by_blob_id(local_plan);
    let snapshot_blob_ids = snapshot_blob_ids(repo, snapshot_id);

    let object_packs = local_plan
        .object_packs
        .values()
        .map(|pack| {
            let mut row = pack.metadata.clone();
            remove_manifest_forbidden_fields(
                &mut row,
                &[
                    "generation_key",
                    "repo_name",
                    "repo_id",
                    "status",
                    "pack_path",
                    "pack_index",
                ],
            );
            row
        })
        .collect::<Vec<_>>();
    let tree_packs = local_plan
        .tree_packs
        .values()
        .map(|pack| {
            let mut row = pack.metadata.clone();
            remove_manifest_forbidden_fields(
                &mut row,
                &[
                    "generation_key",
                    "repo_name",
                    "repo_id",
                    "status",
                    "pack_path",
                    "pack_index",
                ],
            );
            row
        })
        .collect::<Vec<_>>();
    let blob_locators = local_plan
        .blob_locators
        .iter()
        .filter(|(blob_id, _)| snapshot_blob_ids.contains(*blob_id))
        .map(|(blob_id, locator)| {
            let mut row = locator.clone();
            remove_manifest_forbidden_fields(
                &mut row,
                &["generation_key", "storage_path", "storage_kind"],
            );
            row.as_object_mut().expect("blob locator object").insert(
                "pack_entry_name".to_string(),
                json!(entry_names
                    .get(blob_id)
                    .expect("blob locator pack entry name")),
            );
            row
        })
        .collect::<Vec<_>>();
    let tree_locators = local_plan
        .tree_locators
        .values()
        .map(|locator| {
            let mut row = locator.clone();
            remove_manifest_forbidden_fields(&mut row, &["generation_key"]);
            row
        })
        .collect::<Vec<_>>();
    let mut snapshot_row = local_plan
        .snapshots
        .get(snapshot_id)
        .expect("snapshot row")
        .clone();
    snapshot_row
        .as_object_mut()
        .expect("snapshot object")
        .entry("snapshot_kind".to_string())
        .or_insert_with(|| json!("line"));

    let commit = ZstdBulkCommitRequest::from_json_rows(
        Some("ait.remote_sync.zstd_bulk.commit.v1".to_string()),
        None,
        object_packs,
        tree_packs,
        blob_locators,
        tree_locators,
        vec![snapshot_row],
        None,
    )
    .expect("canonical zstd rows");

    let manifest = ZstdImportManifestPayload {
        contract: ZSTD_IMPORT_MANIFEST_CONTRACT_NAME.to_string(),
        repo_name: repo_name.to_string(),
        snapshot_id: snapshot_id.to_string(),
        snapshots: commit.snapshots,
        object_packs: commit.object_packs,
        tree_packs: commit.tree_packs,
        blob_locators: commit.blob_locators,
        tree_locators: commit.tree_locators,
        line_update: None,
    };
    ZstdImportManifestJson::stateless()
        .validate_domain(&manifest)
        .expect("valid zstd import manifest fixture");
    manifest
}

fn seed_fake_zstd_download_remote_from_repo(
    remote: &mut FakeLineSnapshotRemote,
    repo: &RepoRuntime,
    repo_name: &str,
    snapshot_ids: &[String],
) {
    for snapshot_id in snapshot_ids {
        let local_plan = remote_sync::build_zstd_bulk_local_plan(
            repo,
            std::slice::from_ref(snapshot_id),
            &BTreeSet::new(),
        )
        .expect("zstd local plan");
        let manifest =
            zstd_import_manifest_from_local_plan(repo, repo_name, snapshot_id, &local_plan);
        for pack in local_plan.object_packs.values() {
            remote
                .zstd_object_packs
                .entry(pack.pack_id.clone())
                .or_insert_with(|| fs::read(&pack.pack_abs_path).expect("object pack bytes"));
        }
        for pack in local_plan.tree_packs.values() {
            remote
                .zstd_tree_packs
                .entry(pack.pack_id.clone())
                .or_insert_with(|| fs::read(&pack.pack_abs_path).expect("tree pack bytes"));
        }
        remote
            .zstd_import_manifests
            .insert(snapshot_id.clone(), manifest);
    }
}

struct TwoSnapshotZstdFixture {
    _tmp: TempDir,
    repo: RepoRuntime,
    parent_id: String,
    child_id: String,
}

struct DiamondZstdFixture {
    _tmp: TempDir,
    repo: RepoRuntime,
    root_id: String,
    left_id: String,
    right_id: String,
    merge_id: String,
}

fn init_fixture_repo(root: &Path) {
    init_repo(&InitRequest {
        root: root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init fixture repo");
    set_runtime_repository_index(root, 7);
}

fn create_two_snapshot_zstd_source() -> TwoSnapshotZstdFixture {
    let tmp = tempdir().expect("source repo tempdir");
    let root = tmp.path();
    init_fixture_repo(root);
    fs::write(root.join("src.txt"), "zstd parent").expect("parent file");
    let parent = create_local_snapshot(
        root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("zstd parent"),
        false,
    )
    .expect("create parent snapshot");
    let parent_id = required_string_field(&parent, "snapshot_id").expect("parent snapshot id");
    fs::write(root.join("src.txt"), "zstd child").expect("child file");
    let child = create_local_snapshot(
        root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("zstd child"),
        false,
    )
    .expect("create child snapshot");
    let child_id = required_string_field(&child, "snapshot_id").expect("child snapshot id");
    rewrite_repo_packs_as_zstd(root, &parent_id);
    rewrite_repo_packs_as_zstd(root, &child_id);
    let repo = RepoRuntime::discover_from_path(root).expect("source repo runtime");
    TwoSnapshotZstdFixture {
        _tmp: tmp,
        repo,
        parent_id,
        child_id,
    }
}

fn create_diamond_zstd_source() -> DiamondZstdFixture {
    let tmp = tempdir().expect("diamond source repo tempdir");
    let root = tmp.path();
    init_fixture_repo(root);
    let repo = RepoRuntime::discover_from_path(root).expect("diamond source runtime");

    fs::write(root.join("src.txt"), "root\n").unwrap();
    let root_snapshot = create_local_snapshot(
        root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("root"),
        false,
    )
    .unwrap();
    let root_id = required_string_field(&root_snapshot, "snapshot_id").unwrap();

    fs::write(root.join("src.txt"), "left\n").unwrap();
    let left_snapshot = create_local_snapshot(
        root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("left"),
        false,
    )
    .unwrap();
    let left_id = required_string_field(&left_snapshot, "snapshot_id").unwrap();

    repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .lines()
        .set_line_head("main", Some(&root_id), "2026-07-19T00:00:01Z")
        .unwrap();
    fs::write(root.join("src.txt"), "right\n").unwrap();
    let right_snapshot = create_local_snapshot(
        root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("right"),
        false,
    )
    .unwrap();
    let right_id = required_string_field(&right_snapshot, "snapshot_id").unwrap();

    let content = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content();
    let left_record = content
        .snapshots()
        .snapshot_by_id(&left_id)
        .unwrap()
        .expect("left snapshot metadata");
    let left_root = content.snapshot_tree_root_locator(&left_id).unwrap();
    let merge_id = snapshot_id_from_hash48(0x0C0D_0E0F_1011);
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
            manifest_hash: "ef".repeat(32),
            message: Some("merge".to_string()),
            line_name: "main".to_string(),
            snapshot_kind: "line".to_string(),
            file_count: left_record.file_count,
            total_bytes: left_record.total_bytes,
            created_at: "2026-07-19T00:00:02Z".to_string(),
        },
    )
    .expect("record diamond merge");

    for snapshot_id in [&root_id, &left_id, &right_id, &merge_id] {
        rewrite_repo_packs_as_zstd(root, snapshot_id);
    }
    DiamondZstdFixture {
        _tmp: tmp,
        repo,
        root_id,
        left_id,
        right_id,
        merge_id,
    }
}

fn create_empty_fixture_repo() -> (TempDir, RepoRuntime) {
    let tmp = tempdir().expect("target repo tempdir");
    init_fixture_repo(tmp.path());
    let repo = RepoRuntime::discover_from_path(tmp.path()).expect("target repo runtime");
    (tmp, repo)
}

fn snapshot_exists_in_repo(repo: &RepoRuntime, snapshot_id: &str) -> bool {
    repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content()
        .snapshot_exists(snapshot_id)
        .expect("read Binary DB snapshot existence")
}

fn tree_pack_owner_map(repo: &RepoRuntime) -> BTreeMap<String, String> {
    let content = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content();
    let trees = content.trees();
    let read = trees.begin_read_txn();
    trees
        .existing_tree_ids(&read)
        .expect("list existing trees")
        .into_iter()
        .map(|tree_id| {
            let owner = trees
                .get_tree_view(&read, &tree_id)
                .expect("read tree owner")
                .and_then(|tree| tree.tree_pack_id)
                .expect("tree pack owner");
            (tree_id, owner)
        })
        .collect()
}

fn blob_exists_in_repo(repo: &RepoRuntime, blob_id: &str) -> bool {
    let blobs = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content()
        .blobs()
        .clone();
    let read = blobs.begin_read_txn();
    blobs
        .get_blob_view(&read, blob_id)
        .expect("read Binary DB blob existence")
        .is_some()
}

fn assert_repo_pack_metadata_zstd_only(repo: &RepoRuntime) {
    let content = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content();
    let object_packs = content.object_packs();
    let read = object_packs.begin_read_txn();
    let object_pack_views = object_packs
        .list_object_pack_views(&read)
        .expect("list Binary DB object packs");
    let tree_pack_views = content
        .tree_packs()
        .list_tree_pack_views(&read)
        .expect("list Binary DB tree packs");

    assert!(object_pack_views
        .iter()
        .all(|pack| { pack.pack_format == ait_core::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1 }));
    assert!(tree_pack_views.iter().all(|pack| {
        pack.pack_format == ait_core::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1
    }));
    assert!(
        !object_pack_views.is_empty(),
        "stress fixture should contain object packs"
    );
    assert!(
        !tree_pack_views.is_empty(),
        "stress fixture should contain tree packs"
    );
}

fn remote_with_zstd_manifests(
    source_repo: &RepoRuntime,
    snapshot_ids: &[String],
) -> FakeLineSnapshotRemote {
    let mut remote = FakeLineSnapshotRemote::default();
    seed_fake_zstd_download_remote_from_repo(&mut remote, source_repo, "fixture-ait", snapshot_ids);
    remote
}

fn enable_bulk_pull_manifest(
    remote: &mut FakeLineSnapshotRemote,
    snapshot_ids: &[String],
    head_snapshot_id: &str,
) {
    enable_bulk_pull_manifest_with_boundaries(remote, snapshot_ids, head_snapshot_id, Vec::new());
}

fn enable_bulk_pull_manifest_with_boundaries(
    remote: &mut FakeLineSnapshotRemote,
    snapshot_ids: &[String],
    head_snapshot_id: &str,
    boundary_snapshot_ids: Vec<String>,
) {
    let manifests = snapshot_ids
        .iter()
        .map(|snapshot_id| {
            remote
                .zstd_import_manifests
                .get(snapshot_id)
                .cloned()
                .expect("snapshot manifest for bulk pull")
        })
        .collect::<Vec<_>>();
    let snapshots = manifests
        .iter()
        .flat_map(|manifest| manifest.snapshots.iter().cloned())
        .collect::<Vec<_>>();
    let object_packs = manifests
        .iter()
        .flat_map(|manifest| manifest.object_packs.iter().cloned())
        .map(|pack| (pack.pack_id.clone(), pack))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();
    let tree_packs = manifests
        .iter()
        .flat_map(|manifest| manifest.tree_packs.iter().cloned())
        .map(|pack| (pack.pack_id.clone(), pack))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();
    let blob_locators = manifests
        .iter()
        .flat_map(|manifest| manifest.blob_locators.iter().cloned())
        .map(|locator| (locator.blob_id.clone(), locator))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();
    let tree_locators = manifests
        .iter()
        .flat_map(|manifest| manifest.tree_locators.iter().cloned())
        .map(|locator| (locator.tree_id.clone(), locator))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect::<Vec<_>>();
    let manifest = ZstdPullManifestPayload {
        contract: ZSTD_PULL_MANIFEST_CONTRACT_NAME.to_string(),
        repo_name: "fixture-ait".to_string(),
        head_snapshot_id: head_snapshot_id.to_string(),
        boundary_snapshot_ids,
        snapshots,
        object_packs,
        tree_packs,
        blob_locators,
        tree_locators,
    };
    ZstdPullManifestJson::stateless()
        .validate_domain(&manifest)
        .expect("valid bulk pull manifest fixture");
    remote.zstd_pull_manifest = Some(manifest);
}

fn corrupt_first_zstd_data_chunk_preserving_index(pack_path: &Path) {
    let mut bytes = fs::read(pack_path).expect("read zstd pack");
    let trailer_len = 60usize;
    assert!(bytes.len() > trailer_len);
    let trailer_start = bytes.len() - trailer_len;
    assert_eq!(&bytes[trailer_start..trailer_start + 8], b"AITZSTP1");
    let mut index_offset_bytes = [0u8; 8];
    index_offset_bytes.copy_from_slice(&bytes[trailer_start + 12..trailer_start + 20]);
    let index_offset = u64::from_le_bytes(index_offset_bytes) as usize;
    assert!(index_offset > 0);
    bytes[0] ^= 0x40;
    fs::write(pack_path, bytes).expect("rewrite corrupted zstd data chunk");
}

#[test]
fn zstd_only_snapshot_hydration_uses_zstd_pack_download_or_fails_explicitly() {
    let source_tmp = tempdir().expect("source repo tempdir");
    let source_root = source_tmp.path();
    init_fixture_repo(source_root);
    fs::write(source_root.join("src.txt"), "zstd hydration").expect("fixture file");
    let snapshot = create_local_snapshot(
        source_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("zstd hydration"),
        false,
    )
    .expect("create remote snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(source_root, &snapshot_id);
    let source_repo = RepoRuntime::discover_from_path(source_root).expect("source repo runtime");

    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut legacy_remote =
        remote_with_zstd_manifests(&source_repo, std::slice::from_ref(&snapshot_id));
    let err = remote_sync::hydrate_remote_snapshot_chain_with_task_remote_and_capabilities(
        &target_repo,
        &mut legacy_remote,
        "origin",
        "fixture-ait",
        &snapshot_id,
        &unavailable_remote_sync_capabilities(),
    )
    .expect_err("hydration requires the current download capability");
    assert!(err.contains("Remote sync requires capability"));
    assert!(err.contains(REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD));
    assert!(legacy_remote.zstd_import_manifest_reads.is_empty());
    assert!(legacy_remote.zstd_object_pack_downloads.is_empty());
    assert!(legacy_remote.zstd_tree_pack_downloads.is_empty());

    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote = remote_with_zstd_manifests(&source_repo, std::slice::from_ref(&snapshot_id));
    let hydrated = remote_sync::hydrate_remote_snapshot_chain_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        &snapshot_id,
        &zstd_only_download_capabilities(),
    )
    .expect("zstd hydration should download raw zstd packs");

    assert_eq!(
        hydrated["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(hydrated["imported_snapshots"], json!(1));
    assert_eq!(hydrated["imported_snapshot_ids"], json!([snapshot_id]));
    assert!(
        hydrated["zstd_bulk"]["downloaded_object_packs"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert!(
        hydrated["zstd_bulk"]["downloaded_tree_packs"]
            .as_i64()
            .unwrap()
            > 0
    );
    assert_eq!(remote.zstd_import_manifest_reads, vec![snapshot_id.clone()]);
    assert!(!remote.zstd_object_pack_downloads.is_empty());
    assert!(!remote.zstd_tree_pack_downloads.is_empty());
    assert!(snapshot_exists_in_repo(&target_repo, &snapshot_id));
}

#[test]
fn zstd_pull_walks_parent_manifests_until_local_ancestor() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote = remote_with_zstd_manifests(
        &fixture.repo,
        &[fixture.parent_id.clone(), fixture.child_id.clone()],
    );
    remote.lines = vec![json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": fixture.child_id,
    })];

    let pulled = remote_sync::pull_line_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        false,
        false,
        false,
        &zstd_only_download_capabilities(),
    )
    .expect("pull should walk parent manifests");

    assert_eq!(
        remote.zstd_import_manifest_reads,
        vec![fixture.child_id.clone(), fixture.parent_id.clone()]
    );
    assert_eq!(
        pulled["imported_snapshot_ids"],
        json!([fixture.parent_id, fixture.child_id])
    );
}

#[test]
fn zstd_pull_uses_one_deduplicated_manifest_and_stages_pack_batches() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let snapshot_ids = vec![fixture.parent_id.clone(), fixture.child_id.clone()];
    let mut remote = remote_with_zstd_manifests(&fixture.repo, &snapshot_ids);
    enable_bulk_pull_manifest(&mut remote, &snapshot_ids, &fixture.child_id);
    remote.lines = vec![json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": fixture.child_id,
    })];
    let capabilities = zstd_only_download_capabilities().with_zstd_pull_manifest();

    let pulled = remote_sync::pull_line_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        false,
        false,
        false,
        &capabilities,
    )
    .expect("bulk pull manifest should import complete ancestry");

    assert_eq!(remote.zstd_pull_manifest_requests.len(), 1);
    assert_eq!(
        remote.zstd_pull_manifest_requests[0].head_snapshot_id,
        fixture.child_id
    );
    assert!(remote.zstd_pull_manifest_requests[0]
        .have_snapshot_ids
        .is_empty());
    assert!(
        remote.zstd_import_manifest_reads.is_empty(),
        "capability-gated bulk pull must avoid per-Snapshot manifests"
    );
    assert_eq!(
        remote
            .zstd_object_pack_downloads
            .iter()
            .collect::<BTreeSet<_>>()
            .len(),
        remote.zstd_object_pack_downloads.len(),
        "each deduplicated object pack should download once"
    );
    assert_eq!(
        remote
            .zstd_tree_pack_downloads
            .iter()
            .collect::<BTreeSet<_>>()
            .len(),
        remote.zstd_tree_pack_downloads.len(),
        "each deduplicated tree pack should download once"
    );
    assert_eq!(pulled["imported_snapshots"], json!(2));
    assert_eq!(
        pulled["imported_snapshot_ids"],
        json!([fixture.parent_id, fixture.child_id])
    );
    assert!(snapshot_exists_in_repo(&target_repo, &fixture.parent_id));
    assert!(snapshot_exists_in_repo(&target_repo, &fixture.child_id));
}

#[test]
fn zstd_pull_trusts_committed_boundary_without_descendant_pack_scan() {
    let fixture = create_two_snapshot_zstd_source();
    let (target_tmp, target_repo) = create_empty_fixture_repo();

    let mut parent_remote =
        remote_with_zstd_manifests(&fixture.repo, std::slice::from_ref(&fixture.parent_id));
    remote_sync::hydrate_remote_snapshot_chain_with_task_remote_and_capabilities(
        &target_repo,
        &mut parent_remote,
        "origin",
        "fixture-ait",
        &fixture.parent_id,
        &zstd_only_download_capabilities(),
    )
    .expect("seed complete local parent Snapshot");
    target_repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .lines()
        .set_line_head("main", Some(&fixture.parent_id), "2026-08-13T00:00:00Z")
        .expect("make the parent a local advertised Snapshot");
    let parent_local_plan = remote_sync::build_zstd_bulk_local_plan(
        &target_repo,
        std::slice::from_ref(&fixture.parent_id),
        &BTreeSet::new(),
    )
    .expect("local parent pack plan");
    let parent_object_pack_path = parent_local_plan
        .object_packs
        .values()
        .next()
        .and_then(|pack| pack.metadata["pack_path"].as_str())
        .expect("parent object-pack path")
        .to_string();
    fs::remove_file(target_tmp.path().join(&parent_object_pack_path))
        .expect("remove one reachable parent object pack");

    assert!(snapshot_exists_in_repo(&target_repo, &fixture.parent_id));
    assert!(
        remote_sync::remote_sync_snapshot_content_complete_for_repo(
            &target_repo,
            &fixture.parent_id,
        )
        .expect("inspect committed parent boundary"),
        "normal pull boundary detection must not traverse a committed Snapshot's physical packs"
    );
    let maintenance = target_repo
        .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .expect("target Binary DB maintenance store");
    let exact_validation =
        ait_core::local_content_gc::validate_with_local_content_maintenance_store(&maintenance);
    assert!(
        exact_validation.is_err()
            || exact_validation
                .as_ref()
                .is_ok_and(|payload| payload["needs_attention"] == json!(true)),
        "explicit exact validation must not silently accept the missing physical pack"
    );

    let snapshot_ids = vec![fixture.parent_id.clone(), fixture.child_id.clone()];
    let mut remote = remote_with_zstd_manifests(&fixture.repo, &snapshot_ids);
    enable_bulk_pull_manifest_with_boundaries(
        &mut remote,
        std::slice::from_ref(&fixture.child_id),
        &fixture.child_id,
        vec![fixture.parent_id.clone()],
    );
    let bounded_manifest = remote
        .zstd_pull_manifest
        .take()
        .expect("child manifest bounded by the advertised parent");

    enable_bulk_pull_manifest(&mut remote, &snapshot_ids, &fixture.child_id);
    let complete_manifest = remote
        .zstd_pull_manifest
        .take()
        .expect("complete retry manifest");
    remote.zstd_pull_manifests = VecDeque::from([bounded_manifest, complete_manifest]);
    remote.lines = vec![json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": fixture.child_id,
    })];

    let pulled = remote_sync::pull_line_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        false,
        false,
        false,
        &zstd_only_download_capabilities().with_zstd_pull_manifest(),
    )
    .expect("bulk pull must accept the committed parent boundary without a descendant scan");

    assert_eq!(remote.zstd_pull_manifest_requests.len(), 1);
    assert!(remote.zstd_pull_manifest_requests[0]
        .have_snapshot_ids
        .contains(&fixture.parent_id));
    assert_eq!(pulled["imported_snapshot_ids"], json!([fixture.child_id]));
    assert!(!target_tmp.path().join(parent_object_pack_path).is_file());
    assert!(remote_sync::remote_sync_snapshot_content_complete_for_repo(
        &target_repo,
        &fixture.parent_id,
    )
    .expect("committed parent boundary"));
    assert!(remote_sync::remote_sync_snapshot_content_complete_for_repo(
        &target_repo,
        &fixture.child_id,
    )
    .expect("imported child closure"));
}

#[test]
fn zstd_pull_imports_collected_manifests_parent_before_child() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote = remote_with_zstd_manifests(
        &fixture.repo,
        &[fixture.parent_id.clone(), fixture.child_id.clone()],
    );
    remote.lines = vec![json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": fixture.child_id,
    })];

    let pulled = remote_sync::pull_line_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        false,
        false,
        false,
        &zstd_only_download_capabilities(),
    )
    .expect("pull should import parent before child");

    assert_eq!(pulled["imported_snapshots"], json!(2));
    assert_eq!(
        pulled["imported_snapshot_ids"],
        json!([fixture.parent_id, fixture.child_id])
    );
    assert_eq!(pulled["head_snapshot_id"], json!(fixture.child_id));
    assert!(snapshot_exists_in_repo(&target_repo, &fixture.parent_id));
    assert!(snapshot_exists_in_repo(&target_repo, &fixture.child_id));
}

#[test]
fn zstd_pull_imports_complete_diamond_parent_closure_before_merge() {
    let fixture = create_diamond_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let snapshot_ids = vec![
        fixture.root_id.clone(),
        fixture.left_id.clone(),
        fixture.right_id.clone(),
        fixture.merge_id.clone(),
    ];
    let mut remote = remote_with_zstd_manifests(&fixture.repo, &snapshot_ids);
    remote.lines = vec![json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": fixture.merge_id,
    })];

    let pulled = remote_sync::pull_line_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        false,
        false,
        false,
        &zstd_only_download_capabilities(),
    )
    .expect("pull complete diamond");

    assert_eq!(pulled["imported_snapshots"], 4);
    let imported_ids = pulled["imported_snapshot_ids"]
        .as_array()
        .expect("imported diamond ids")
        .iter()
        .filter_map(JsonValue::as_str)
        .collect::<Vec<_>>();
    assert_eq!(imported_ids.first(), Some(&fixture.root_id.as_str()));
    assert_eq!(imported_ids.last(), Some(&fixture.merge_id.as_str()));
    assert!(imported_ids.contains(&fixture.left_id.as_str()));
    assert!(imported_ids.contains(&fixture.right_id.as_str()));
    assert_eq!(
        remote
            .zstd_import_manifest_reads
            .iter()
            .collect::<BTreeSet<_>>(),
        snapshot_ids.iter().collect::<BTreeSet<_>>()
    );
    let merge = target_repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content()
        .snapshots()
        .snapshot_by_id(&fixture.merge_id)
        .unwrap()
        .expect("imported merge metadata");
    assert_eq!(
        merge.parent_snapshot_ids,
        vec![fixture.left_id, fixture.right_id]
    );
}

#[test]
fn zstd_import_manifest_locators_cover_requested_snapshot_closure_only() {
    let fixture = create_two_snapshot_zstd_source();
    let mut remote =
        remote_with_zstd_manifests(&fixture.repo, std::slice::from_ref(&fixture.parent_id));
    let manifest = remote
        .zstd_import_manifests
        .remove(&fixture.parent_id)
        .expect("parent manifest");
    let parent_blob_ids = snapshot_blob_ids(&fixture.repo, &fixture.parent_id);
    let child_blob_ids = snapshot_blob_ids(&fixture.repo, &fixture.child_id);
    let manifest_blob_ids = manifest
        .blob_locators
        .iter()
        .map(|row| row.blob_id.clone())
        .collect::<BTreeSet<_>>();

    assert_eq!(manifest_blob_ids, parent_blob_ids);
    assert!(
        child_blob_ids
            .difference(&manifest_blob_ids)
            .next()
            .is_some(),
        "fixture must have a child-only blob outside the parent manifest closure"
    );
}

#[test]
fn zstd_import_manifest_pack_rows_match_full_downloaded_pack_indexes() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote =
        remote_with_zstd_manifests(&fixture.repo, std::slice::from_ref(&fixture.parent_id));
    let manifest = remote
        .zstd_import_manifests
        .get(&fixture.parent_id)
        .expect("parent manifest")
        .clone();

    let hydrated = remote_sync::hydrate_remote_snapshot_chain_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        &fixture.parent_id,
        &zstd_only_download_capabilities(),
    )
    .expect("hydration should validate downloaded pack indexes");

    assert_eq!(hydrated["imported_snapshots"], json!(1));
    let local_plan = remote_sync::build_zstd_bulk_local_plan(
        &fixture.repo,
        std::slice::from_ref(&fixture.parent_id),
        &BTreeSet::new(),
    )
    .expect("zstd local plan");
    for row in &manifest.object_packs {
        let metadata = &local_plan
            .object_packs
            .get(&row.pack_id)
            .expect("source object pack")
            .metadata;
        let index = metadata.get("pack_index").expect("object pack index");
        assert_eq!(
            row.member_count,
            index.get("member_count").and_then(JsonValue::as_i64)
        );
        assert_eq!(
            row.total_bytes,
            index.get("total_bytes").and_then(JsonValue::as_i64)
        );
        assert_eq!(
            row.pack_index_entry_name.as_deref(),
            index.get("index_entry_name").and_then(JsonValue::as_str)
        );
        assert_eq!(
            row.pack_index_checksum.as_deref(),
            metadata
                .get("pack_index_checksum")
                .and_then(JsonValue::as_str)
        );
    }
    for row in &manifest.tree_packs {
        let metadata = &local_plan
            .tree_packs
            .get(&row.pack_id)
            .expect("source tree pack")
            .metadata;
        let index = metadata.get("pack_index").expect("tree pack index");
        assert_eq!(
            row.tree_count,
            index.get("tree_count").and_then(JsonValue::as_i64)
        );
        assert_eq!(
            row.total_bytes,
            index.get("total_bytes").and_then(JsonValue::as_i64)
        );
        assert_eq!(
            row.pack_index_entry_name.as_deref(),
            index.get("index_entry_name").and_then(JsonValue::as_str)
        );
        assert_eq!(
            row.pack_index_checksum.as_deref(),
            metadata
                .get("pack_index_checksum")
                .and_then(JsonValue::as_str)
        );
    }
    assert!(remote.zstd_object_pack_downloads.iter().all(|pack_id| {
        manifest
            .object_packs
            .iter()
            .any(|row| row.pack_id == *pack_id)
    }));
    assert!(remote.zstd_tree_pack_downloads.iter().all(|pack_id| {
        manifest
            .tree_packs
            .iter()
            .any(|row| row.pack_id == *pack_id)
    }));
}

#[test]
fn zstd_import_does_not_upsert_extra_pack_members_without_manifest_locators() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote =
        remote_with_zstd_manifests(&fixture.repo, std::slice::from_ref(&fixture.parent_id));
    let parent_blob_ids = snapshot_blob_ids(&fixture.repo, &fixture.parent_id);
    let child_blob_ids = snapshot_blob_ids(&fixture.repo, &fixture.child_id);

    remote_sync::hydrate_remote_snapshot_chain_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        &fixture.parent_id,
        &zstd_only_download_capabilities(),
    )
    .expect("hydrate parent only");

    for blob_id in &parent_blob_ids {
        assert!(blob_exists_in_repo(&target_repo, blob_id));
    }
    for blob_id in child_blob_ids.difference(&parent_blob_ids) {
        assert!(
            !blob_exists_in_repo(&target_repo, blob_id),
            "child-only blob {blob_id} should remain inert without manifest locator"
        );
    }
}

#[test]
fn zstd_import_reuses_same_pack_across_collected_manifests_with_matching_index() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote = remote_with_zstd_manifests(
        &fixture.repo,
        &[fixture.parent_id.clone(), fixture.child_id.clone()],
    );
    let parent_object_packs = remote
        .zstd_import_manifests
        .get(&fixture.parent_id)
        .expect("parent manifest")
        .object_packs
        .clone();
    let child_manifest = remote
        .zstd_import_manifests
        .get_mut(&fixture.child_id)
        .expect("child manifest");
    for pack in parent_object_packs {
        if !child_manifest
            .object_packs
            .iter()
            .any(|row| row.pack_id == pack.pack_id)
        {
            child_manifest.object_packs.push(pack);
        }
    }
    ZstdImportManifestJson::stateless()
        .validate_domain(child_manifest)
        .expect("child manifest with reusable pack row remains valid");
    remote.lines = vec![json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": fixture.child_id,
    })];

    let pulled = remote_sync::pull_line_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        false,
        false,
        false,
        &zstd_only_download_capabilities(),
    )
    .expect("pull should reuse downloaded packs for child manifest");

    assert_eq!(pulled["imported_snapshots"], json!(2));
    assert!(pulled["zstd_bulk"]["reused_object_packs"].as_i64().unwrap() > 0);
}

#[test]
fn converted_zstd_repository_stress_covers_push_hydrate_pull_land_materialize_maintenance_boundaries_interruption_and_idempotency(
) {
    let fixture = create_two_snapshot_zstd_source();
    assert_repo_pack_metadata_zstd_only(&fixture.repo);

    let source_maintenance = fixture
        .repo
        .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .expect("source Binary DB maintenance store");
    let validate = ait_core::local_content_gc::validate_with_local_content_maintenance_store(
        &source_maintenance,
    )
    .expect("converted source Binary DB validate");
    assert!(
        validate["state"].as_str().is_some(),
        "validate payload should include a storage state"
    );
    assert_repo_pack_metadata_zstd_only(&fixture.repo);

    let interrupted_plan = remote_sync::build_zstd_bulk_local_plan(
        &fixture.repo,
        &[fixture.parent_id.clone(), fixture.child_id.clone()],
        &BTreeSet::new(),
    )
    .expect("build interruption fixture Binary DB plan");
    let interrupted_pack_id = interrupted_plan
        .object_packs
        .keys()
        .next()
        .expect("interruption fixture object pack")
        .clone();
    let mut interrupted_remote = FakeLineSnapshotRemote {
        fail_zstd_object_pack_upload_for: Some(interrupted_pack_id),
        ..FakeLineSnapshotRemote::default()
    };
    let interrupted = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &fixture.repo,
        &mut interrupted_remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect_err("injected zstd upload interruption should abort before commit");
    assert!(interrupted.contains("injected upload failure"));
    assert_eq!(interrupted_remote.zstd_commit_requests.len(), 0);
    assert!(interrupted_remote.lines.is_empty());

    let mut remote = FakeLineSnapshotRemote {
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "fixture-ait",
                "namespace": "",
                "tombstoned": false
            },
            "ci_capabilities": {
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true,
                    "zstd_pack_bulk_download": true
                }
            }
        })),
        ..FakeLineSnapshotRemote::default()
    };
    let pushed = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &fixture.repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect("converted zstd push");
    assert_eq!(
        pushed["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(pushed["head_snapshot_id"], json!(fixture.child_id));
    assert_eq!(pushed["uploaded_snapshots"], json!(2));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert!(!remote.uploaded_zstd_object_packs.is_empty());
    assert!(!remote.uploaded_zstd_tree_packs.is_empty());

    let idempotent_push = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &fixture.repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect("converted zstd push should be idempotent");
    assert_eq!(idempotent_push["checked_snapshots"], json!(0));
    assert_eq!(idempotent_push["uploaded_snapshots"], json!(0));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);

    seed_fake_zstd_download_remote_from_repo(
        &mut remote,
        &fixture.repo,
        "fixture-ait",
        &[fixture.parent_id.clone(), fixture.child_id.clone()],
    );

    let (_hydration_tmp, hydration_repo) = create_empty_fixture_repo();
    let hydrated = remote_sync::hydrate_remote_snapshot_chain_with_task_remote_and_capabilities(
        &hydration_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        &fixture.child_id,
        &zstd_only_download_capabilities(),
    )
    .expect("converted zstd hydration");
    assert_eq!(
        hydrated["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert!(snapshot_exists_in_repo(&hydration_repo, &fixture.child_id));
    assert!(!remote.zstd_object_pack_downloads.is_empty());
    assert!(!remote.zstd_tree_pack_downloads.is_empty());
    assert_repo_pack_metadata_zstd_only(&hydration_repo);

    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let pulled = remote_sync::pull_line_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        false,
        true,
        true,
        &zstd_only_download_capabilities(),
    )
    .expect("converted zstd pull with materialization");
    assert_eq!(pulled["head_snapshot_id"], json!(fixture.child_id));
    assert!(snapshot_exists_in_repo(&target_repo, &fixture.child_id));
    assert_eq!(
        fs::read_to_string(target_repo.root.join("src.txt")).expect("materialized file"),
        "zstd child"
    );
    assert_repo_pack_metadata_zstd_only(&target_repo);

    let target_maintenance = target_repo
        .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .expect("target Binary DB maintenance store");
    let target_validate =
        ait_core::local_content_gc::validate_with_local_content_maintenance_store(
            &target_maintenance,
        )
        .expect("converted target Binary DB validate");
    assert!(
        target_validate["state"].as_str().is_some(),
        "target validate payload should include a storage state"
    );
    assert_repo_pack_metadata_zstd_only(&target_repo);

    let manifest_reads_before_idempotent_pull = remote.zstd_import_manifest_reads.len();
    let idempotent_pull = remote_sync::pull_line_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        false,
        true,
        false,
        &zstd_only_download_capabilities(),
    )
    .expect("already-current converted zstd pull should be idempotent");
    assert_eq!(idempotent_pull["imported_snapshots"], json!(0));
    assert_eq!(
        remote.zstd_import_manifest_reads.len(),
        manifest_reads_before_idempotent_pull
    );

    remote_sync::set_or_create_local_line_head(
        &fixture.repo,
        "feature/stress-land",
        Some(&fixture.child_id),
    )
    .expect("create land-bound feature line");
    let commit_count_before_land_sync = remote.zstd_commit_requests.len();
    let land_sync = remote_sync::sync_patchset_revision_snapshot_with_task_remote(
        &fixture.repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "feature/stress-land",
        &fixture.child_id,
        "main",
    )
    .expect("land-bound zstd patchset snapshot sync");
    assert_eq!(land_sync["line_updated"], json!(true));
    assert_eq!(
        land_sync["sync_reason"],
        json!("remote_line_missing_head_snapshot_present")
    );
    assert_eq!(land_sync["checked_snapshots"], json!(1));
    assert_eq!(land_sync["uploaded_snapshots"], json!(0));
    assert_eq!(land_sync["skipped_snapshots"], json!(1));
    assert!(land_sync["remote_sync_backend"].is_null());
    assert_eq!(
        remote.zstd_commit_requests.len(),
        commit_count_before_land_sync
    );
    assert!(remote.lines.iter().any(|line| {
        line.get("line_name").and_then(JsonValue::as_str) == Some("feature/stress-land")
            && line.get("head_snapshot_id").and_then(JsonValue::as_str)
                == Some(fixture.child_id.as_str())
    }));
}

#[test]
fn remote_head_boundary_imports_verified_child_closure_without_parent_manifest() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote =
        remote_with_zstd_manifests(&fixture.repo, std::slice::from_ref(&fixture.child_id));
    let manifest = remote
        .zstd_import_manifests
        .remove(&fixture.child_id)
        .expect("child manifest");
    let store = target_repo
        .remote_sync_local_store::<REMOTE_SYNC_BINARY_DB_WRITE_LAYOUT>()
        .expect("Binary DB remote import store");
    let context = RemoteSyncLocalStoreContext::new(&target_repo.root);
    let plan = store
        .zstd_import_download_plan(&context, &manifest)
        .expect("remote-head download plan");

    let imported = store
        .import_zstd_manifest(
            &context,
            &manifest,
            ZstdImportHistoryMode::RemoteHeadBoundary,
            &plan,
            &remote.zstd_object_packs,
            &remote.zstd_tree_packs,
        )
        .expect("remote head closure import");

    assert_eq!(imported.snapshot_id, fixture.child_id);
    assert!(!snapshot_exists_in_repo(&target_repo, &fixture.parent_id));
    let content = target_repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content();
    let read = content.snapshots().begin_read_txn();
    let head = content
        .snapshots()
        .get_snapshot_view(&read, &fixture.child_id)
        .expect("read imported remote head")
        .expect("imported remote head exists");
    assert_eq!(head.parent_snapshot_id, None);
    assert!(head.record.is_remote_head_history_boundary());
}

#[test]
fn external_hydration_imports_one_verified_remote_head_boundary() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote =
        remote_with_zstd_manifests(&fixture.repo, std::slice::from_ref(&fixture.child_id));

    let hydrated = remote_sync::hydrate_remote_snapshot_boundary_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        &fixture.child_id,
        &zstd_only_download_capabilities(),
    )
    .expect("external hydration should not request the parent manifest");

    assert_eq!(
        remote.zstd_import_manifest_reads,
        vec![fixture.child_id.clone()]
    );
    assert_eq!(hydrated["imported_snapshots"], json!(1));
    assert_eq!(
        hydrated["imported_snapshot_ids"],
        json!([fixture.child_id.clone()])
    );
    assert_eq!(
        hydrated["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert!(snapshot_exists_in_repo(&target_repo, &fixture.child_id));
    assert!(!snapshot_exists_in_repo(&target_repo, &fixture.parent_id));
    let content = target_repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content();
    let read = content.snapshots().begin_read_txn();
    let head = content
        .snapshots()
        .get_snapshot_view(&read, &fixture.child_id)
        .expect("read imported external head")
        .expect("imported external head exists");
    assert_eq!(head.parent_snapshot_id, None);
    assert!(head.record.is_remote_head_history_boundary());
}

#[test]
fn external_hydration_accepts_an_empty_root_tree_boundary() {
    let source_tmp = tempdir().expect("source repo tempdir");
    let source_root = source_tmp.path();
    init_fixture_repo(source_root);
    let snapshot = create_local_snapshot(
        source_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("empty root boundary"),
        false,
    )
    .expect("empty root snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    set_repo_pack_format_kinds(source_root, 1);
    let source_repo = RepoRuntime::discover_from_path(source_root).expect("source runtime");
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote = remote_with_zstd_manifests(&source_repo, std::slice::from_ref(&snapshot_id));

    remote_sync::hydrate_remote_snapshot_boundary_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        &snapshot_id,
        &zstd_only_download_capabilities(),
    )
    .expect("empty root boundary hydration");

    assert!(snapshot_exists_in_repo(&target_repo, &snapshot_id));
    assert!(remote_sync::remote_sync_snapshot_content_complete_for_repo(
        &target_repo,
        &snapshot_id
    )
    .expect("empty root boundary completeness"));
}

#[test]
fn external_hydration_reuses_cross_pack_tree_collision_and_remains_materializable() {
    let source_tmp = tempdir().expect("source repo tempdir");
    let source_root = source_tmp.path();
    init_fixture_repo(source_root);
    fs::create_dir_all(source_root.join("shared")).expect("source shared dir");
    fs::create_dir_all(source_root.join("source-only")).expect("source-only dir");
    fs::write(source_root.join("shared/common.txt"), "shared bytes\n").expect("shared file");
    fs::write(source_root.join("source-only/value.txt"), "parent\n").expect("parent file");
    let parent = create_local_snapshot(
        source_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("collision parent"),
        false,
    )
    .expect("source parent snapshot");
    let parent_id = required_string_field(&parent, "snapshot_id").expect("parent id");
    fs::write(source_root.join("source-only/value.txt"), "child\n").expect("child file");
    let child = create_local_snapshot(
        source_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("collision child"),
        false,
    )
    .expect("source child snapshot");
    let child_id = required_string_field(&child, "snapshot_id").expect("child id");
    set_repo_pack_format_kinds(source_root, 1);
    let source_repo = RepoRuntime::discover_from_path(source_root).expect("source runtime");

    let target_tmp = tempdir().expect("target repo tempdir");
    let target_root = target_tmp.path();
    init_fixture_repo(target_root);
    fs::create_dir_all(target_root.join("shared")).expect("target shared dir");
    fs::create_dir_all(target_root.join("target-only")).expect("target-only dir");
    fs::write(target_root.join("shared/common.txt"), "shared bytes\n").expect("shared file");
    fs::write(target_root.join("target-only/value.txt"), "local\n").expect("target file");
    create_local_snapshot(
        target_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("collision target"),
        false,
    )
    .expect("target seed snapshot");
    set_repo_pack_format_kinds(target_root, 1);
    let target_repo = RepoRuntime::discover_from_path(target_root).expect("target runtime");

    let source_owners = tree_pack_owner_map(&source_repo);
    let target_owners = tree_pack_owner_map(&target_repo);
    assert!(source_owners.iter().any(|(tree_id, source_pack)| {
        target_owners
            .get(tree_id)
            .is_some_and(|target_pack| target_pack != source_pack)
    }));

    let mut remote = remote_with_zstd_manifests(&source_repo, std::slice::from_ref(&child_id));
    remote_sync::hydrate_remote_snapshot_boundary_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        &child_id,
        &zstd_only_download_capabilities(),
    )
    .expect("cross-pack collision boundary import");

    assert!(!snapshot_exists_in_repo(&target_repo, &parent_id));
    assert!(snapshot_exists_in_repo(&target_repo, &child_id));
    assert!(
        remote_sync::remote_sync_snapshot_content_complete_for_repo(&target_repo, &child_id,)
            .expect("boundary content completeness")
    );
}

#[test]
fn zstd_import_manifest_requires_pack_and_locator_resolution() {
    let fixture = create_two_snapshot_zstd_source();
    let (_target_tmp, target_repo) = create_empty_fixture_repo();
    let mut remote =
        remote_with_zstd_manifests(&fixture.repo, std::slice::from_ref(&fixture.parent_id));
    let manifest = remote
        .zstd_import_manifests
        .get_mut(&fixture.parent_id)
        .expect("parent manifest");
    manifest.blob_locators[0].pack_entry_name = Some("blobs/missing-entry".to_string());

    let err = remote_sync::hydrate_remote_snapshot_chain_with_task_remote_and_capabilities(
        &target_repo,
        &mut remote,
        "origin",
        "fixture-ait",
        &fixture.parent_id,
        &zstd_only_download_capabilities(),
    )
    .expect_err("bad locator must fail import");

    assert!(err.contains("references missing pack entry"));
    assert!(!snapshot_exists_in_repo(&target_repo, &fixture.parent_id));
}

#[test]
fn remote_sync_inventory_uses_pack_metadata_without_expanding_tree_payloads() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "metadata only inventory").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("metadata only inventory fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(repo_root, &snapshot_id);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let snapshots = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content()
        .snapshots()
        .clone();
    let read = snapshots.begin_read_txn();
    let tree_pack_rel_path = snapshots
        .get_snapshot_view(&read, &snapshot_id)
        .expect("read Binary DB snapshot")
        .and_then(|snapshot| snapshot.root_tree_pack_path)
        .expect("read Binary DB tree pack path");
    corrupt_first_zstd_data_chunk_preserving_index(&repo_root.join(tree_pack_rel_path));

    let inventory = remote_sync::local_remote_sync_inventory_for_snapshots(
        &repo,
        std::slice::from_ref(&snapshot_id),
    )
    .expect("inventory should use metadata only");

    assert_eq!(inventory.snapshot_ids, vec![snapshot_id]);
    assert!(inventory
        .object_pack_formats
        .contains(ait_core::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1));
    assert!(inventory
        .tree_pack_formats
        .contains(ait_core::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1));
}

#[test]
fn remote_sync_push_line_selects_zstd_backend_when_capability_is_advertised() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "zstd selected").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("zstd selected fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(repo_root, &snapshot_id);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote::default();

    let pushed = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect("zstd backend should bulk upload raw packs");

    assert_eq!(
        pushed["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(pushed["head_snapshot_id"], json!(snapshot_id));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 1);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 1);
    assert_eq!(remote.zstd_pack_parallelism_requests, vec![2]);
    assert_eq!(pushed["remote_sync_metrics"]["pack_parallelism"], json!(2));
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert_eq!(remote.line_update_calls, 0);
    assert_eq!(remote.lines[0]["head_snapshot_id"], json!(snapshot_id));
    assert!(remote.uploaded_zstd_object_packs[0].1.len() > 32);
    assert!(remote.uploaded_zstd_tree_packs[0].1.len() > 32);
}

#[test]
fn remote_sync_zstd_bulk_push_plans_only_requested_lineage() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "main base").expect("fixture file");
    let main_base = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("main base"),
        false,
    )
    .expect("create main base snapshot");
    let main_base_id = required_string_field(&main_base, "snapshot_id").expect("snapshot id");
    fs::write(repo_root.join("src.txt"), "main head").expect("fixture file");
    let main_head = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("main head"),
        false,
    )
    .expect("create main head snapshot");
    let main_head_id = required_string_field(&main_head, "snapshot_id").expect("snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    remote_sync::set_or_create_local_line_head(&repo, "feature/zstd", Some(&main_base_id))
        .expect("create feature line from main base");
    fs::write(repo_root.join("src.txt"), "feature head").expect("fixture file");
    let feature_head = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "feature/zstd",
        Some("feature head"),
        false,
    )
    .expect("create feature head snapshot");
    let feature_head_id = required_string_field(&feature_head, "snapshot_id").expect("snapshot id");
    let feature_blob_ids = snapshot_blob_ids(&repo, &feature_head_id);
    let main_blob_ids = snapshot_blob_ids(&repo, &main_base_id)
        .union(&snapshot_blob_ids(&repo, &main_head_id))
        .cloned()
        .collect::<BTreeSet<_>>();
    let feature_only_blob_ids = feature_blob_ids
        .difference(&main_blob_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(!feature_only_blob_ids.is_empty());
    repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content()
        .set_snapshot_kind(&feature_head_id, "stash")
        .expect("mark feature snapshot as non-line snapshot");
    for snapshot_id in [&main_base_id, &main_head_id, &feature_head_id] {
        rewrite_repo_packs_as_zstd(repo_root, snapshot_id);
    }
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote::default();

    let pushed = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect("zstd backend should push only the requested line snapshot chain");

    assert_eq!(
        pushed["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(pushed["checked_snapshots"], json!(2));
    assert_eq!(pushed["uploaded_snapshots"], json!(2));
    assert_eq!(pushed["head_snapshot_id"], json!(main_head_id));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    let planned_snapshot_ids = remote.zstd_plan_requests[0]["snapshot_ids"]
        .as_array()
        .expect("planned snapshots")
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(planned_snapshot_ids.len(), 2);
    assert_eq!(planned_snapshot_ids[0], main_base_id);
    assert!(planned_snapshot_ids.contains(&main_head_id));
    assert!(!planned_snapshot_ids.contains(&feature_head_id));
    assert_eq!(
        remote.zstd_commit_requests[0]["snapshots"]
            .as_array()
            .expect("committed snapshots")
            .len(),
        2
    );
    let committed_snapshot_ids = remote.zstd_commit_requests[0]["snapshots"]
        .as_array()
        .expect("committed snapshots")
        .iter()
        .map(|value| value["snapshot_id"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    assert!(!committed_snapshot_ids.contains(&feature_head_id));
    let committed_blob_ids = remote.zstd_commit_requests[0]["blob_locators"]
        .as_array()
        .expect("committed blob locators")
        .iter()
        .map(|value| value["blob_id"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    assert!(feature_only_blob_ids.is_disjoint(&committed_blob_ids));
    assert_eq!(remote.lines[0]["head_snapshot_id"], json!(main_head_id));
}

#[test]
fn remote_sync_zstd_bulk_push_excludes_bounded_remote_head_ancestors_from_plan() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let config_path = repo_root.join(".ait/config.json");
    let mut config = fs::read_to_string(&config_path)
        .expect("read fixture config")
        .parse::<JsonValue>()
        .expect("parse fixture config");
    let config_object = config.as_object_mut().expect("fixture config object");
    config_object.insert("workflow_mode".to_string(), json!("solo_remote"));
    config_object.insert("workflow_default_scope".to_string(), json!("remote"));
    config_object.insert("task_default_scope".to_string(), json!("remote"));
    config_object.insert("change_default_scope".to_string(), json!("remote"));
    fs::write(config_path, config.to_string()).expect("write fixture config");
    fs::write(repo_root.join("src.txt"), "main base").expect("fixture file");
    let main_base = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("main base"),
        false,
    )
    .expect("create main base snapshot");
    let main_base_id = required_string_field(&main_base, "snapshot_id").expect("snapshot id");
    fs::write(repo_root.join("src.txt"), "main head").expect("fixture file");
    let main_head = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("main head"),
        false,
    )
    .expect("create main head snapshot");
    let main_head_id = required_string_field(&main_head, "snapshot_id").expect("snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    assert_eq!(repo.effective_workflow_mode(), "solo_remote");
    remote_sync::set_or_create_local_line_head(&repo, "feature/zstd", Some(&main_base_id))
        .expect("create feature line from main base");
    fs::write(repo_root.join("src.txt"), "feature head").expect("fixture file");
    let feature_head = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "feature/zstd",
        Some("feature head"),
        false,
    )
    .expect("create feature head snapshot");
    let feature_head_id = required_string_field(&feature_head, "snapshot_id").expect("snapshot id");
    for snapshot_id in [&main_base_id, &main_head_id, &feature_head_id] {
        rewrite_repo_packs_as_zstd(repo_root, snapshot_id);
    }
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let boundary_blob_ids = snapshot_blob_ids(&repo, &main_base_id);
    let suffix_blob_ids = snapshot_blob_ids(&repo, &main_head_id)
        .union(&snapshot_blob_ids(&repo, &feature_head_id))
        .cloned()
        .collect::<BTreeSet<_>>();
    let boundary_only_blob_ids = boundary_blob_ids
        .difference(&suffix_blob_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(!boundary_only_blob_ids.is_empty());
    let mut remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": main_base_id,
        })],
        remote_snapshots: BTreeMap::from([(
            main_base_id.clone(),
            json!({
                "repo_name": "fixture-ait",
                "snapshot_id": main_base_id,
            }),
        )]),
        ..Default::default()
    };

    let pushed = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect("zstd backend should skip already bounded remote ancestors");

    assert_eq!(pushed["checked_snapshots"], json!(1));
    assert_eq!(pushed["uploaded_snapshots"], json!(1));
    assert_eq!(pushed["bounded_by_snapshot_id"], json!(main_base_id));
    let planned_snapshot_ids = remote.zstd_plan_requests[0]["snapshot_ids"]
        .as_array()
        .expect("planned snapshots")
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(!planned_snapshot_ids.contains(&main_base_id));
    assert!(planned_snapshot_ids.contains(&main_head_id));
    assert!(!planned_snapshot_ids.contains(&feature_head_id));
    let committed_snapshot_ids = remote.zstd_commit_requests[0]["snapshots"]
        .as_array()
        .expect("committed snapshots")
        .iter()
        .map(|value| value["snapshot_id"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert!(!committed_snapshot_ids.contains(&main_base_id));
    assert!(committed_snapshot_ids.contains(&main_head_id));
    assert!(!committed_snapshot_ids.contains(&feature_head_id));
    let committed_blob_ids = remote.zstd_commit_requests[0]["blob_locators"]
        .as_array()
        .expect("committed blob locators")
        .iter()
        .map(|value| value["blob_id"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    assert!(
        boundary_only_blob_ids.is_disjoint(&committed_blob_ids),
        "bounded ancestor-only blobs {boundary_only_blob_ids:?} leaked into committed blobs {committed_blob_ids:?}"
    );
    assert_eq!(remote.lines[0]["head_snapshot_id"], json!(main_head_id));
}

#[test]
fn remote_sync_push_requires_current_zstd_capability() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "zstd capability").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("zstd capability fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(repo_root, &snapshot_id);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote::default();

    let err = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &unavailable_remote_sync_capabilities(),
    )
    .expect_err("push requires the current upload capability");

    assert!(err.contains("Remote sync requires capability"));
    assert!(err.contains(ait_core::remote_sync_backend::REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK));
    assert_eq!(remote.zstd_plan_requests.len(), 0);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 0);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 0);
    assert_eq!(remote.zstd_commit_requests.len(), 0);
    assert_eq!(remote.line_update_calls, 0);
}

#[test]
fn remote_sync_upload_chain_requires_current_zstd_capability() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "zstd upload capability").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("zstd upload capability fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(repo_root, &snapshot_id);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote::default();

    let err = remote_sync::upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "fixture-ait",
        &snapshot_id,
        None,
        &unavailable_remote_sync_capabilities(),
    )
    .expect_err("snapshot chain upload requires the current capability");

    assert!(err.contains("Remote sync requires capability"));
    assert!(err.contains(ait_core::remote_sync_backend::REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK));
    assert_eq!(remote.zstd_plan_requests.len(), 0);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 0);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 0);
    assert_eq!(remote.zstd_commit_requests.len(), 0);
    assert_eq!(remote.line_update_calls, 0);
}

#[test]
fn remote_sync_zstd_skips_present_packs_and_commits_missing_snapshot() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "zstd resume").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("zstd resume fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(repo_root, &snapshot_id);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let content = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content();
    let object_packs = content.object_packs();
    let read = object_packs.begin_read_txn();
    let object_pack_ids = object_packs
        .list_object_pack_views(&read)
        .expect("list Binary DB object packs")
        .into_iter()
        .map(|pack| pack.pack_id)
        .collect::<BTreeSet<_>>();
    let tree_pack_ids = content
        .tree_packs()
        .list_tree_pack_views(&read)
        .expect("list Binary DB tree packs")
        .into_iter()
        .map(|pack| pack.pack_id)
        .collect::<BTreeSet<_>>();
    assert!(!object_pack_ids.is_empty());
    assert!(!tree_pack_ids.is_empty());
    let mut remote = FakeLineSnapshotRemote {
        present_zstd_object_pack_ids: object_pack_ids,
        present_zstd_tree_pack_ids: tree_pack_ids,
        ..FakeLineSnapshotRemote::default()
    };

    let pushed = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect("resume should skip raw pack upload and commit missing snapshot metadata");

    assert_eq!(
        pushed["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(pushed["head_snapshot_id"], json!(snapshot_id));
    assert_eq!(pushed["zstd_bulk"]["uploaded_object_packs"], json!(0));
    assert_eq!(pushed["zstd_bulk"]["uploaded_tree_packs"], json!(0));
    assert_eq!(pushed["zstd_bulk"]["skipped_object_packs"], json!(1));
    assert_eq!(pushed["zstd_bulk"]["skipped_tree_packs"], json!(1));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 0);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 0);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert_eq!(
        remote.zstd_commit_requests[0]["snapshots"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    for field in [
        "object_packs",
        "tree_packs",
        "blob_locators",
        "tree_locators",
    ] {
        assert!(
            remote.zstd_commit_requests[0][field]
                .as_array()
                .expect("zstd bulk commit collection")
                .is_empty(),
            "remote-present pack metadata must not be resubmitted in {field}"
        );
    }
    assert_eq!(remote.lines[0]["head_snapshot_id"], json!(snapshot_id));
}

#[test]
fn remote_sync_zstd_backend_rejects_wrong_format_pack_before_upload() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "wrong format").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("wrong format fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(repo_root, &snapshot_id);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let local_plan = remote_sync::build_zstd_bulk_local_plan(
        &repo,
        std::slice::from_ref(&snapshot_id),
        &BTreeSet::new(),
    )
    .expect("snapshot-scoped zstd local plan");
    let object_pack_rel_path = local_plan
        .object_packs
        .values()
        .next()
        .and_then(|pack| pack.metadata["pack_path"].as_str())
        .expect("snapshot object pack path")
        .to_string();
    corrupt_first_zstd_data_chunk_preserving_index(&repo_root.join(object_pack_rel_path));
    let mut remote = FakeLineSnapshotRemote::default();

    let err = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect_err("wrong-format zstd pack should be rejected before upload");

    assert!(
        err.contains("failed zstd validation") || err.contains("Invalid zstd chunked pack"),
        "{err}"
    );
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 0);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 0);
    assert_eq!(remote.zstd_commit_requests.len(), 0);
    assert_eq!(remote.line_update_calls, 0);
}

#[test]
fn remote_sync_zstd_partial_pack_failure_does_not_commit_line_update() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "zstd partial failure").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("zstd partial failure fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(repo_root, &snapshot_id);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let local_plan = remote_sync::build_zstd_bulk_local_plan(
        &repo,
        std::slice::from_ref(&snapshot_id),
        &BTreeSet::new(),
    )
    .expect("snapshot-scoped zstd local plan");
    let pack_id = local_plan
        .object_packs
        .keys()
        .next()
        .expect("snapshot object pack id")
        .clone();
    let mut remote = FakeLineSnapshotRemote {
        fail_zstd_object_pack_upload_for: Some(pack_id),
        ..FakeLineSnapshotRemote::default()
    };

    let err = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &zstd_only_capabilities(),
    )
    .expect_err("injected zstd upload failure should abort before commit");

    assert!(err.contains("injected upload failure"));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 0);
    assert!(remote.lines.is_empty());
    assert_eq!(remote.line_update_calls, 0);
}

#[test]
fn remote_sync_push_rejects_missing_zstd_capability_before_line_update() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "upload failure").expect("fixture file");
    create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("upload failure fixture"),
        false,
    )
    .expect("create snapshot");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote::default();

    let err = remote_sync::push_line_to_remote_with_task_remote_and_capabilities(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &unavailable_remote_sync_capabilities(),
    )
    .expect_err("push must require the current capability before transport");

    assert!(err.contains("Remote sync requires capability"));
    assert!(err.contains(ait_core::remote_sync_backend::REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK));
    assert_eq!(remote.line_update_calls, 0);
    assert!(remote.lines.is_empty());
}

#[test]
fn remote_sync_patchset_revision_snapshot_accepts_line_and_snapshot_remote_traits() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "patchset revision sync").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("patchset revision sync fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote {
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "fixture-ait",
                "namespace": "",
                "tombstoned": false
            },
            "ci_capabilities": {
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true
                }
            }
        })),
        ..FakeLineSnapshotRemote::default()
    };

    let base_sync = remote_sync::sync_patchset_revision_snapshot_with_task_remote(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &snapshot_id,
        "main",
    )
    .expect("base line snapshot sync should reuse line and snapshot remote");

    assert_eq!(base_sync["repo_name"], json!("fixture-ait"));
    assert_eq!(base_sync["line"], json!("main"));
    assert_eq!(base_sync["line_updated"], json!(false));
    assert_eq!(
        base_sync["line_update_skipped_reason"],
        json!("current line is the change base line")
    );
    assert_eq!(base_sync["head_snapshot_id"], json!(snapshot_id));
    assert_eq!(base_sync["checked_snapshots"], json!(1));
    assert_eq!(base_sync["uploaded_snapshots"], json!(1));
    assert_eq!(base_sync["skipped_snapshots"], json!(0));
    assert!(base_sync.get("remote").is_none());
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 1);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 1);

    remote.remote_snapshots.insert(
        snapshot_id.clone(),
        json!({
            "repo_name": "fixture-ait",
            "snapshot_id": snapshot_id,
        }),
    );
    remote.lines.push(json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": snapshot_id,
    }));
    let default_line_sync = remote_sync::sync_patchset_revision_snapshot_with_task_remote(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &snapshot_id,
        "release",
    )
    .expect("default line snapshot sync should reuse line and snapshot remote");

    assert_eq!(default_line_sync["line"], json!("main"));
    assert_eq!(default_line_sync["line_updated"], json!(false));
    assert_eq!(
        default_line_sync["line_update_skipped_reason"],
        json!("current line is the default integration line")
    );
    assert_eq!(default_line_sync["checked_snapshots"], json!(0));
    assert_eq!(default_line_sync["uploaded_snapshots"], json!(0));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);

    remote_sync::set_or_create_local_line_head(&repo, "feature/sync", Some(&snapshot_id))
        .expect("create feature line");
    let feature_sync = remote_sync::sync_patchset_revision_snapshot_with_task_remote(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "feature/sync",
        &snapshot_id,
        "main",
    )
    .expect("feature line snapshot sync should reuse push line flow");

    assert_eq!(feature_sync["remote"], json!("origin"));
    assert_eq!(feature_sync["line"], json!("feature/sync"));
    assert_eq!(feature_sync["line_updated"], json!(true));
    assert!(feature_sync["line_update_skipped_reason"].is_null());
    assert_eq!(feature_sync["head_snapshot_id"], json!(snapshot_id));
    assert_eq!(feature_sync["checked_snapshots"], json!(1));
    assert_eq!(feature_sync["uploaded_snapshots"], json!(0));
    assert_eq!(feature_sync["skipped_snapshots"], json!(1));
    assert_eq!(
        feature_sync["sync_reason"],
        json!("remote_line_missing_head_snapshot_present")
    );
    assert_eq!(
        feature_sync["remote_line"]["head_snapshot_id"],
        json!(snapshot_id)
    );
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
}

#[test]
fn remote_sync_patchset_revision_snapshot_uses_remote_zstd_capabilities() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "patchset zstd sync").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("patchset zstd sync fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    rewrite_repo_packs_as_zstd(repo_root, &snapshot_id);
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeLineSnapshotRemote {
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "fixture-ait",
                "namespace": "",
                "tombstoned": false
            },
            "ci_capabilities": {
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true
                }
            }
        })),
        ..FakeLineSnapshotRemote::default()
    };

    let base_sync = remote_sync::sync_patchset_revision_snapshot_with_task_remote(
        &repo,
        &mut remote,
        "origin",
        "fixture-ait",
        "main",
        &snapshot_id,
        "main",
    )
    .expect("zstd-capable patchset sync should use bulk pack transport");

    assert_eq!(
        base_sync["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(base_sync["line_updated"], json!(false));
    assert_eq!(
        base_sync["line_update_skipped_reason"],
        json!("current line is the change base line")
    );
    assert_eq!(base_sync["head_snapshot_id"], json!(snapshot_id));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert_eq!(remote.uploaded_zstd_object_packs.len(), 1);
    assert_eq!(remote.uploaded_zstd_tree_packs.len(), 1);
}

#[test]
fn remote_sync_line_read_accepts_line_remote_trait() {
    let mut remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "feature/demo",
            "status": "active",
            "head_snapshot_id": "SNP-REMOTE"
        })],
        ..Default::default()
    };

    let remote_line = remote_sync::remote_sync_line_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/demo",
    )
    .expect("remote line read");
    assert_eq!(remote_line["head_snapshot_id"], json!("SNP-REMOTE"));

    let err = remote_sync::remote_sync_line_read_with_task_remote(
        &mut remote,
        "other-repo",
        "feature/demo",
    )
    .expect_err("repo mismatch should fail");
    assert!(err.contains("unexpected repository"));
}

#[test]
fn remote_sync_line_head_read_accepts_line_remote_trait() {
    let mut remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "feature/demo",
            "status": "active",
            "head_snapshot_id": "SNP-REMOTE"
        })],
        ..Default::default()
    };

    let existing = remote_sync::remote_sync_line_head_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/demo",
    )
    .expect("read existing remote line head");
    assert_eq!(existing.as_deref(), Some("SNP-REMOTE"));

    let missing = remote_sync::remote_sync_line_head_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/missing",
    )
    .expect("missing remote line is allowed");
    assert_eq!(missing, None);

    let mut legacy_remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "line_name": "feature/legacy",
            "head_snapshot_id": "SNP-LEGACY"
        })],
        ..Default::default()
    };
    let legacy = remote_sync::remote_sync_line_head_with_task_remote(
        &mut legacy_remote,
        "fixture-ait",
        "feature/legacy",
    )
    .expect("legacy remote line head read");
    assert_eq!(legacy.as_deref(), Some("SNP-LEGACY"));

    let err = remote_sync::remote_sync_line_head_with_task_remote(
        &mut remote,
        "other-repo",
        "feature/demo",
    )
    .expect_err("repo mismatch should fail");
    assert!(err.contains("unexpected repository"));
}

#[test]
fn remote_sync_line_helpers_accept_single_capability_ports() {
    let mut line_reader = FakeLineReader;
    let remote_line = remote_sync::remote_sync_line_read_with_task_remote(
        &mut line_reader,
        "fixture-ait",
        "feature/demo",
    )
    .expect("line read through single-capability port");
    assert_eq!(remote_line["head_snapshot_id"], json!("SNP-REMOTE"));

    let mut head_reader = FakeLineReader;
    let head = remote_sync::remote_sync_line_head_with_task_remote(
        &mut head_reader,
        "fixture-ait",
        "feature/demo",
    )
    .expect("line head read through single-capability port");
    assert_eq!(head.as_deref(), Some("SNP-REMOTE"));

    let mut head_updater = FakeLineHeadUpdater;
    let updated = remote_sync::remote_sync_line_update_with_task_remote(
        &mut head_updater,
        "fixture-ait",
        "feature/demo",
        Some("SNP-NEW"),
        Some("SNP-OLD"),
    )
    .expect("line update through single-capability port");
    assert_eq!(updated["expected_head_snapshot_id"], json!("SNP-OLD"));
}

#[test]
fn remote_sync_snapshot_metadata_read_accepts_metadata_reader_trait() {
    let mut remote = FakeLineSnapshotRemote {
        remote_snapshots: BTreeMap::from([
            (
                "SNP-REMOTE".to_string(),
                json!({
                    "repo_name": "fixture-ait",
                    "snapshot_id": "SNP-REMOTE",
                    "parent_snapshot_id": "SNP-PARENT"
                }),
            ),
            (
                "SNP-WRONG-REPO".to_string(),
                json!({
                    "repo_name": "other-ait",
                    "snapshot_id": "SNP-WRONG-REPO"
                }),
            ),
        ]),
        ..Default::default()
    };

    let snapshot = remote_sync::remote_sync_snapshot_metadata_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "SNP-REMOTE",
    )
    .expect("remote snapshot metadata read");
    assert_eq!(snapshot["parent_snapshot_id"], json!("SNP-PARENT"));

    let err = remote_sync::remote_sync_snapshot_metadata_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "SNP-WRONG-REPO",
    )
    .expect_err("repo mismatch should fail");
    assert!(err.contains("unexpected repository"));

    let err = remote_sync::remote_sync_snapshot_metadata_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "SNP-MISSING",
    )
    .expect_err("missing snapshot should fail");
    assert!(err.contains("Unknown snapshot"));
}

#[test]
fn remote_sync_present_snapshot_ids_accepts_existence_reader_trait() {
    let mut remote = FakeLineSnapshotRemote {
        remote_snapshots: BTreeMap::from([
            (
                "SNP-A".to_string(),
                json!({
                    "snapshot_id": "SNP-A"
                }),
            ),
            (
                "SNP-C".to_string(),
                json!({
                    "snapshot_id": "SNP-C"
                }),
            ),
        ]),
        ..Default::default()
    };
    let snapshot_ids = vec![
        "SNP-A".to_string(),
        "SNP-B".to_string(),
        "SNP-C".to_string(),
    ];

    let present = remote_sync::remote_sync_present_snapshot_ids_with_task_remote(
        &mut remote,
        "fixture-ait",
        &snapshot_ids,
    )
    .expect("read present remote snapshot ids");
    assert_eq!(
        present,
        std::collections::BTreeSet::from(["SNP-A".to_string(), "SNP-C".to_string()])
    );

    let empty_snapshot_ids: Vec<String> = Vec::new();
    let empty = remote_sync::remote_sync_present_snapshot_ids_with_task_remote(
        &mut remote,
        "fixture-ait",
        &empty_snapshot_ids,
    )
    .expect("empty snapshot check is allowed");
    assert!(empty.is_empty());
}

#[test]
fn snapshot_read_helpers_accept_single_capability_ports() {
    struct FakeSnapshotMetadataReader {
        snapshots: BTreeMap<String, JsonValue>,
    }

    impl TaskWorkflowSnapshotMetadataReader for FakeSnapshotMetadataReader {
        fn get_remote_snapshot(
            &mut self,
            repo_name: &str,
            snapshot_id: &str,
            _include_content: bool,
            _path: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            self.snapshots
                .get(snapshot_id)
                .cloned()
                .map(|mut snapshot| {
                    if let Some(obj) = snapshot.as_object_mut() {
                        obj.insert(
                            "repo_name".to_string(),
                            JsonValue::String(repo_name.to_string()),
                        );
                    }
                    snapshot
                })
                .ok_or_else(|| {
                    TaskWorkflowHttpClientError::Remote(format!(
                        "GET snapshot {snapshot_id} failed: 404 Unknown snapshot"
                    ))
                })
        }
    }

    struct FakeSnapshotExistenceReader;

    impl TaskWorkflowSnapshotExistenceReader for FakeSnapshotExistenceReader {
        fn get_remote_snapshots_existence(
            &mut self,
            repo_name: &str,
            snapshot_ids: &[String],
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            Ok(json!({
                "repo_name": repo_name,
                "present": snapshot_ids.iter().filter(|id| id.as_str() != "SNP-MISSING").cloned().collect::<Vec<_>>(),
            }))
        }
    }

    let mut metadata_reader = FakeSnapshotMetadataReader {
        snapshots: BTreeMap::from([
            (
                "SNP-CHILD".to_string(),
                json!({
                    "snapshot_id": "SNP-CHILD",
                    "parent_snapshot_id": "SNP-PARENT",
                }),
            ),
            (
                "SNP-PARENT".to_string(),
                json!({
                    "snapshot_id": "SNP-PARENT",
                }),
            ),
        ]),
    };
    let metadata = remote_sync::remote_sync_snapshot_metadata_read_with_task_remote(
        &mut metadata_reader,
        "fixture-ait",
        "SNP-CHILD",
    )
    .expect("remote sync metadata read through single-capability port");
    assert_eq!(metadata["parent_snapshot_id"], json!("SNP-PARENT"));

    let snapshot_ids = vec!["SNP-CHILD".to_string(), "SNP-MISSING".to_string()];
    let present = remote_sync::remote_sync_present_snapshot_ids_with_task_remote(
        &mut FakeSnapshotExistenceReader,
        "fixture-ait",
        &snapshot_ids,
    )
    .expect("existence read through single-capability port");
    assert_eq!(present, BTreeSet::from(["SNP-CHILD".to_string()]));
}

#[test]
fn remote_sync_line_update_accepts_line_remote_trait() {
    let mut remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "feature/demo",
            "status": "active",
            "head_snapshot_id": "SNP-OLD"
        })],
        ..Default::default()
    };

    let updated = remote_sync::remote_sync_line_update_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/demo",
        Some("SNP-NEW"),
        Some("SNP-OLD"),
    )
    .expect("update remote line head");
    assert_eq!(updated["head_snapshot_id"], json!("SNP-NEW"));

    let cleared = remote_sync::remote_sync_line_update_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/demo",
        None,
        Some("SNP-NEW"),
    )
    .expect("clear remote line head");
    assert!(cleared["head_snapshot_id"].is_null());

    let err = remote_sync::remote_sync_line_update_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/demo",
        Some("SNP-LATER"),
        Some("SNP-MISMATCH"),
    )
    .expect_err("expected head mismatch should fail");
    assert!(err.contains("expected head"));
}
