use super::*;
use ait_core::binary_db::BinaryDbCommandScope;
use ait_core::content_binary_db::{
    snapshot_id_from_hash48, BinaryDbContentWriteCoordinator, BinaryDbSnapshotWriteInput,
};
use ait_core::line_store::LineStore;
use ait_core::local_content_gc::{LocalContentStatsOptions, LocalContentStatsStore};
use ait_core::local_snapshot::{LocalSnapshotBlobReadStore, LocalSnapshotTreeReadStore};
use ait_core::snapshot_store::SnapshotStore;

#[derive(Debug, Default)]
struct FakeLineChangeUsageStore {
    changes: Vec<JsonValue>,
}

impl ait_core::change_store::ChangeStore for FakeLineChangeUsageStore {
    fn list_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
        Ok(self.changes.clone())
    }

    fn get_change(&self, change_id: &str) -> PlanStoreResult<JsonValue> {
        self.changes
            .iter()
            .find(|row| row["change_id"].as_str() == Some(change_id))
            .cloned()
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown change: {change_id}")))
    }

    fn allocate_change_identity(
        &self,
        _repo_name: &str,
        _namespace_prefix: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "line usage fake store does not allocate identities".to_string(),
        ))
    }

    fn create_change(
        &self,
        _task_id: &str,
        _repo_name: &str,
        _title: &str,
        _base_line: &str,
        _namespace_prefix: Option<&str>,
        _fork_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "line usage fake store does not create changes".to_string(),
        ))
    }

    fn create_change_explicit(
        &self,
        _change_id: &str,
        _task_id: &str,
        _repo_name: &str,
        _title: &str,
        _base_line: &str,
        _change_seq: Option<i64>,
        _identity_source: Option<&str>,
        _fork_snapshot_id: Option<&str>,
        _forked_from_line: Option<&str>,
        _status: Option<&str>,
        _publication_state: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "line usage fake store does not create explicit changes".to_string(),
        ))
    }

    fn close_change(&self, _change_id: &str, _status: &str) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "line usage fake store does not close changes".to_string(),
        ))
    }

    fn land_change(
        &self,
        _change_id: &str,
        _target_line: &str,
        _landed_snapshot_id: &str,
        _pre_land_target_snapshot_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "line usage fake store does not land changes".to_string(),
        ))
    }

    fn mark_change_published(
        &self,
        _change_id: &str,
        _remote_name: Option<&str>,
        _published_change_id: Option<&str>,
        _allow_landed: bool,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "line usage fake store does not publish changes".to_string(),
        ))
    }
}

#[test]
fn remote_line_helpers_accept_task_remote_trait() {
    let mut remote = FakeWorkspaceTaskRemote {
        lines: vec![
            json!({
                "line_name": "main",
                "status": "active",
                "head_snapshot_id": "SNP-1"
            }),
            json!({
                "line_name": "feature/demo",
                "status": "active",
                "head_snapshot_id": "SNP-2"
            }),
        ],
        ..Default::default()
    };

    let lines =
        remote_line_list_with_task_remote(&mut remote, "fixture-ait").expect("remote line list");
    assert_eq!(
        lines
            .as_array()
            .expect("line list array")
            .iter()
            .filter_map(|row| string_field(row, "line_name"))
            .collect::<Vec<_>>(),
        vec!["main".to_string(), "feature/demo".to_string()]
    );

    let archived = remote_line_archive_with_task_remote(&mut remote, "fixture-ait", "feature/demo")
        .expect("remote line archive");
    assert_eq!(archived["status"], json!("archived"));
    assert_eq!(
        remote.lines[1].get("status"),
        Some(&JsonValue::String("archived".to_string()))
    );
}

#[test]
fn remote_line_helpers_accept_single_capability_ports() {
    let mut line_lister = FakeLineLister;
    let lines =
        remote_line_list_with_task_remote(&mut line_lister, "fixture-ait").expect("line list");
    assert_eq!(
        lines.as_array().expect("line array")[0]["line_name"],
        json!("main")
    );

    let mut line_closer = FakeLineCloser;
    let archived =
        remote_line_archive_with_task_remote(&mut line_closer, "fixture-ait", "feature/demo")
            .expect("line archive");
    assert_eq!(archived["status"], json!("archived"));
}

#[test]
fn repo_status_snapshot_count_accepts_repo_status_store_trait() {
    let store = FakeRepoStatusStore {
        storage_counts: RepoStatusStorageCounts {
            snapshot_count: 13,
            pack_count: 5,
            packed_blob_count: 21,
        },
    };

    assert_eq!(
        repo_status_snapshot_count_with_store(&store).expect("snapshot count"),
        13
    );
}

#[test]
fn line_show_helper_accepts_line_store_trait() {
    let store = FakeLocalLineStore::default();
    create_local_line_with_line_store(
        &store,
        "feature/demo",
        Some("SNP-DEMO"),
        "2026-07-04T02:00:00Z",
    )
    .expect("create line through line store");

    let shown =
        line_show_with_line_store(&store, "feature/demo").expect("show line through line store");
    assert_eq!(shown["line_name"], json!("feature/demo"));
    assert_eq!(shown["status"], json!("active"));
    assert_eq!(shown["head_snapshot_id"], json!("SNP-DEMO"));

    assert_eq!(
        line_show_with_line_store(&store, "missing").unwrap_err(),
        "Unknown line: missing"
    );
}

#[test]
fn worktree_line_mutation_helpers_accept_line_store_trait() {
    let store = FakeLocalLineStore::default();

    let created = create_local_line_with_line_store(
        &store,
        "feature/demo",
        Some("SNP-DEMO"),
        "2026-07-04T02:00:00Z",
    )
    .expect("create line through line store");
    assert_eq!(created["line_name"], json!("feature/demo"));
    assert_eq!(created["status"], json!("active"));
    assert_eq!(created["archived_at"], JsonValue::Null);
    assert_eq!(created["created_at"], json!("2026-07-04T02:00:00Z"));
    assert_eq!(created["updated_at"], json!("2026-07-04T02:00:00Z"));
    assert_eq!(created["head_snapshot_id"], json!("SNP-DEMO"));

    let archived =
        archive_local_line_with_line_store(&store, "feature/demo", "2026-07-04T02:05:00Z")
            .expect("archive line through line store");
    assert_eq!(archived["status"], json!("archived"));
    assert_eq!(archived["archived_at"], json!("2026-07-04T02:05:00Z"));
    assert_eq!(archived["updated_at"], json!("2026-07-04T02:05:00Z"));
    assert_eq!(archived["head_snapshot_id"], json!("SNP-DEMO"));

    let archived_again =
        archive_local_line_with_line_store(&store, "feature/demo", "2026-07-04T02:10:00Z")
            .expect("archive line idempotently");
    assert_eq!(archived_again["archived_at"], json!("2026-07-04T02:05:00Z"));
    assert_eq!(archived_again["updated_at"], json!("2026-07-04T02:05:00Z"));

    let moved = set_local_line_head_with_line_store(
        &store,
        "feature/demo",
        Some("SNP-MOVED"),
        "2026-07-04T02:15:00Z",
    )
    .expect("set line head through line store");
    assert_eq!(moved["head_snapshot_id"], json!("SNP-MOVED"));
    assert_eq!(moved["updated_at"], json!("2026-07-04T02:15:00Z"));

    let cleared =
        set_local_line_head_with_line_store(&store, "feature/demo", None, "2026-07-04T02:20:00Z")
            .expect("clear line head through line store");
    assert_eq!(cleared["head_snapshot_id"], JsonValue::Null);
    assert_eq!(cleared["updated_at"], json!("2026-07-04T02:20:00Z"));
}

#[test]
fn queue_remote_reads_accept_queue_remote_trait() {
    let mut remote = FakeQueueRemote {
        queue_summary_error: Some(
            "GET /v1/native/repository-authorities/7/read/queue-summary?status=active failed with 404"
                .to_string(),
        ),
        task_queue: Some(json!({
            "items": [
                {
                    "task_id": "RCT-1",
                    "focus_change": {
                        "change_id": "RCC-1",
                        "reason": "Focus from task queue."
                    }
                }
            ],
            "summary": {
                "ready_to_land": 1
            }
        })),
        reviewer_inbox: Some(json!({
            "items": [
                {
                    "change_id": "RCC-1",
                    "review_state": {"blocking": 0},
                    "freshness": {"base_is_fresh": true},
                    "policy_state": {"decision": "pass"},
                    "attestation": {"completeness": "present"}
                }
            ],
            "count": 1
        })),
        ..Default::default()
    };
    let section = json!({
        "task_queue": JsonValue::Null,
        "reviewer_inbox": JsonValue::Null,
        "error": JsonValue::Null,
    });

    let result = queue_remote_reads_with_task_remote(&mut remote, section, "fixture-ait")
        .expect("queue remote reads");

    assert_eq!(result["error"], JsonValue::Null);
    assert_eq!(result["task_queue"]["items"][0]["task_id"], json!("RCT-1"));
    assert_eq!(result["reviewer_inbox"]["count"], json!(1));
    assert!(result.get("changes").is_none());

    let bundle_err = queue_remote_summary_bundle_with_task_remote(&mut remote, "fixture-ait")
        .expect_err("queue summary bundle is configured as missing");
    assert!(bundle_err.contains("/v1/native/repository-authorities/7/read/queue-summary"));

    let task_queue = queue_remote_task_queue_with_task_remote(&mut remote, "fixture-ait")
        .expect("read fallback task queue");
    assert_eq!(task_queue["items"][0]["task_id"], json!("RCT-1"));

    let reviewer_inbox = queue_remote_reviewer_inbox_with_task_remote(&mut remote, "fixture-ait")
        .expect("read fallback reviewer inbox");
    assert_eq!(reviewer_inbox["count"], json!(1));
}

#[test]
fn queue_remote_helpers_accept_single_capability_ports() {
    let mut task_queue_reader = FakeTaskQueueReader;
    let task_queue =
        queue_remote_task_queue_with_task_remote(&mut task_queue_reader, "fixture-ait")
            .expect("read task queue through single-capability port");
    assert_eq!(task_queue["status"], json!("active"));

    let mut reviewer_inbox_reader = FakeReviewerInboxReader;
    let reviewer_inbox =
        queue_remote_reviewer_inbox_with_task_remote(&mut reviewer_inbox_reader, "fixture-ait")
            .expect("read reviewer inbox through single-capability port");
    assert_eq!(reviewer_inbox["reviewer_inbox"], json!(true));

    let mut queue_summary_reader = FakeQueueSummaryBundleReader;
    let queue_summary =
        queue_remote_summary_bundle_with_task_remote(&mut queue_summary_reader, "fixture-ait")
            .expect("read queue summary through single-capability port");
    assert_eq!(queue_summary["summary"], json!(true));
}

#[test]
fn task_start_remote_preflight_accepts_task_remote_trait() {
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
    fs::write(repo_root.join("src.txt"), "task start preflight").expect("fixture file");
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("task start preflight fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let remote_row = RemoteRow {
        name: "origin".to_string(),
        url: "https://ait.example".to_string(),
        repo_name: Some("fixture-ait".to_string()),
    };
    let mut remote = FakeWorkspaceTaskRemote {
        lines: vec![json!({
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": snapshot_id
        })],
        ..Default::default()
    };

    let line_row = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("remote preflight");

    assert_eq!(line_row["head_snapshot_id"], json!(snapshot_id));
}

#[test]
fn task_start_remote_preflight_uses_an_available_remote_head_independently_of_local_main() {
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
    fs::write(repo_root.join("src.txt"), "remote base\n").expect("base fixture");
    let base = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("remote base fixture"),
        false,
    )
    .expect("create base snapshot");
    let base_id = required_string_field(&base, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "local ahead\n").expect("local fixture");
    let local = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local ahead fixture"),
        false,
    )
    .expect("create local head snapshot");
    let local_id = required_string_field(&local, "snapshot_id").expect("local snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let remote_row = RemoteRow {
        name: "origin".to_string(),
        url: "https://ait.example".to_string(),
        repo_name: Some("fixture-ait".to_string()),
    };
    let mut remote = FakeWorkspaceTaskRemote {
        lines: vec![json!({
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": base_id,
        })],
        ..Default::default()
    };

    let line_row = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("local-ahead authority should accept its imported Remote head");
    assert_eq!(line_row["head_snapshot_id"], json!(base_id));

    let feature = ensure_task_feature_line(&repo, "RCT-0099", "remote-only", Some(&base_id), false)
        .expect("feature Line should use the Remote Change fork without a local base Line");
    assert_eq!(feature["head_snapshot_id"], json!(base_id));
    assert_ne!(feature["head_snapshot_id"], json!(local_id));
    let empty_feature = ensure_task_feature_line(&repo, "RCT-0100", "remote-empty", None, false)
        .expect("an empty Remote base should not require a local Line");
    assert!(empty_feature["head_snapshot_id"].is_null());

    repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .lines()
        .set_line_head("main", Some(&base_id), "2026-08-14T00:00:01Z")
        .expect("move local main behind the imported Remote head");
    remote.lines[0]["head_snapshot_id"] = json!(local_id);
    let remote_ahead = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("remote-ahead authority should accept its imported Remote head");
    assert_eq!(remote_ahead["head_snapshot_id"], json!(local_id));

    fs::write(repo_root.join("src.txt"), "local divergent\n").expect("divergent fixture");
    let divergent = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local divergent fixture"),
        false,
    )
    .expect("create divergent local head");
    let divergent_id =
        required_string_field(&divergent, "snapshot_id").expect("divergent snapshot id");
    assert_ne!(divergent_id, local_id);
    let remote_diverged = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("divergent local Line should accept its imported Remote head");
    assert_eq!(remote_diverged["head_snapshot_id"], json!(local_id));

    remote.lines[0]["head_snapshot_id"] = JsonValue::Null;
    let empty_remote = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("an empty Remote head must remain authoritative");
    assert!(empty_remote["head_snapshot_id"].is_null());
}

#[test]
fn completed_local_null_remote_base_seed_is_atomic_bounded_and_race_safe() {
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
    fs::write(repo_root.join("src.txt"), "ancestry root\n").expect("root fixture");
    let root_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("ancestry root"),
        false,
    )
    .expect("root snapshot");
    let root_snapshot_id =
        required_string_field(&root_snapshot, "snapshot_id").expect("root snapshot id");
    fs::write(repo_root.join("src.txt"), "pre-land base\n").expect("base fixture");
    let base_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("pre-land base"),
        false,
    )
    .expect("pre-land base snapshot");
    let base_snapshot_id =
        required_string_field(&base_snapshot, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "final local result\n").expect("final fixture");
    let final_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("final local result"),
        false,
    )
    .expect("final snapshot");
    let final_snapshot_id =
        required_string_field(&final_snapshot, "snapshot_id").expect("final snapshot id");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let remote_row = RemoteRow {
        name: "origin".to_string(),
        url: "https://ait.example".to_string(),
        repo_name: Some("fixture-ait".to_string()),
    };
    let remote_repository = json!({
        "repository": {
            "repository_index": 7,
            "repository_name": "fixture-ait",
            "namespace": "",
            "tombstoned": false,
        },
        "ci_capabilities": {
            "remote_sync_capabilities": {
                "zstd_pack_bulk": true,
            }
        }
    });
    let null_line = json!({
        "repo_name": "fixture-ait",
        "line_name": "main",
        "status": "active",
        "head_snapshot_id": null,
    });

    let mut remote = FakeChangeRemote {
        lines: vec![null_line.clone()],
        repository: Some(remote_repository.clone()),
        ..Default::default()
    };
    let initialized = workflow::workflow_initialize_null_remote_base_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
        &base_snapshot_id,
    )
    .expect("initialize null remote at exact pre-land base");

    assert_eq!(initialized["status"], json!("initialized"));
    assert_eq!(
        initialized["reason"],
        json!("remote_null_head_seeded_from_completed_local_pre_land_base")
    );
    assert_eq!(initialized["head_snapshot_id"], json!(base_snapshot_id));
    assert_eq!(remote.lines[0]["head_snapshot_id"], json!(base_snapshot_id));
    assert!(remote.remote_snapshots.contains_key(&root_snapshot_id));
    assert!(remote.remote_snapshots.contains_key(&base_snapshot_id));
    assert!(!remote.remote_snapshots.contains_key(&final_snapshot_id));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert!(remote.zstd_commit_requests[0]["line_update"]["expected_head_snapshot_id"].is_null());
    assert_eq!(
        remote.zstd_commit_requests[0]["line_update"]["head_snapshot_id"],
        json!(base_snapshot_id)
    );
    assert_eq!(
        remote.zstd_commit_requests[0]["snapshots"]
            .as_array()
            .expect("committed snapshot rows")
            .iter()
            .filter_map(|row| string_field(row, "snapshot_id"))
            .collect::<Vec<_>>(),
        vec![root_snapshot_id, base_snapshot_id.clone()]
    );
    assert_eq!(
        line_show(&repo, Some("main")).expect("local main")["head_snapshot_id"],
        json!(final_snapshot_id)
    );

    let mut same_winner_remote = FakeChangeRemote {
        lines: vec![null_line.clone()],
        repository: Some(remote_repository.clone()),
        zstd_commit_peer_head_once: Some(base_snapshot_id.clone()),
        ..Default::default()
    };
    let same_winner = workflow::workflow_initialize_null_remote_base_with_task_remote(
        &repo,
        &remote_row,
        &mut same_winner_remote,
        "fixture-ait",
        "main",
        &base_snapshot_id,
    )
    .expect("same concurrent pre-land base must be idempotent");
    assert_eq!(
        same_winner["reason"],
        json!(
            "remote_null_head_seeded_from_completed_local_pre_land_base_after_uncertain_response"
        )
    );
    assert_eq!(same_winner["head_snapshot_id"], json!(base_snapshot_id));

    let mut different_winner_remote = FakeChangeRemote {
        lines: vec![null_line],
        repository: Some(remote_repository),
        zstd_commit_peer_head_once: Some(final_snapshot_id.clone()),
        ..Default::default()
    };
    let error = workflow::workflow_initialize_null_remote_base_with_task_remote(
        &repo,
        &remote_row,
        &mut different_winner_remote,
        "fixture-ait",
        "main",
        &base_snapshot_id,
    )
    .expect_err("different concurrent winner must fail before publication");
    assert!(error.contains("was initialized concurrently"));
    assert!(
        error.contains("Refusing to create Remote Task, Change, Patchset, or publication mappings")
    );
    assert!(error.contains(&final_snapshot_id));
    assert!(error.contains(&base_snapshot_id));
    assert_eq!(
        different_winner_remote.lines[0]["head_snapshot_id"],
        json!(final_snapshot_id)
    );
}

#[test]
fn task_start_remote_null_head_seeds_complete_local_main_atomically_and_idempotently() {
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
    fs::write(repo_root.join("src.txt"), "local base\n").expect("local base fixture");
    let base_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local base"),
        false,
    )
    .expect("local base snapshot");
    let base_snapshot_id =
        required_string_field(&base_snapshot, "snapshot_id").expect("local base snapshot id");
    fs::write(repo_root.join("src.txt"), "local head\n").expect("local head fixture");
    let local_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local head"),
        false,
    )
    .expect("local main snapshot");
    let local_snapshot_id =
        required_string_field(&local_snapshot, "snapshot_id").expect("local snapshot id");
    let local_bytes = fs::read(repo_root.join("src.txt")).expect("local fixture bytes");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let snapshot_count_before = snapshot_list(&repo)
        .expect("snapshot list before seed")
        .as_array()
        .expect("snapshot list array")
        .len();
    let remote_row = RemoteRow {
        name: "origin".to_string(),
        url: "https://ait.example".to_string(),
        repo_name: Some("fixture-ait".to_string()),
    };
    let mut remote = FakeChangeRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": null,
        })],
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "fixture-ait",
                "namespace": "",
                "tombstoned": false,
            },
            "ci_capabilities": {
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true,
                }
            }
        })),
        ..Default::default()
    };

    let empty_line = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("empty remote preflight");
    let initialized = ensure_remote_base_line_snapshot_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
        &empty_line,
    )
    .expect("seed remote base from local main");
    let seeded_snapshot_id =
        required_string_field(&initialized, "head_snapshot_id").expect("seed snapshot id");

    assert_eq!(initialized["initialized"], json!(true));
    assert_eq!(
        initialized["reason"],
        json!("remote_null_head_seeded_from_local_main")
    );
    assert_eq!(initialized["seed_source"], json!("local_main"));
    assert_eq!(
        initialized["local_seed_snapshot_id"],
        json!(local_snapshot_id)
    );
    assert!(initialized["snapshot"].is_null());
    assert_eq!(seeded_snapshot_id, local_snapshot_id);
    assert_eq!(
        remote.lines[0]["head_snapshot_id"],
        json!(local_snapshot_id)
    );
    assert!(remote.remote_snapshots.contains_key(&base_snapshot_id));
    assert!(remote.remote_snapshots.contains_key(&local_snapshot_id));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    assert_eq!(initialized["snapshot_sync"]["checked_snapshots"], json!(2));
    assert_eq!(
        remote.zstd_commit_requests[0]["line_update"]["expected_head_snapshot_id"],
        JsonValue::Null
    );
    assert_eq!(
        remote.zstd_commit_requests[0]["line_update"]["head_snapshot_id"],
        json!(local_snapshot_id)
    );
    assert_eq!(
        remote.zstd_commit_requests[0]["snapshots"]
            .as_array()
            .expect("committed snapshot rows")
            .iter()
            .filter_map(|row| string_field(row, "snapshot_id"))
            .collect::<Vec<_>>(),
        vec![base_snapshot_id.clone(), local_snapshot_id.clone()]
    );
    assert_eq!(
        snapshot_list(&repo)
            .expect("snapshot list after seed")
            .as_array()
            .expect("snapshot list array")
            .len(),
        snapshot_count_before,
        "local-main seeding must not author a detached Snapshot",
    );
    assert_eq!(
        line_show(&repo, Some("main")).expect("local main")["head_snapshot_id"],
        json!(local_snapshot_id)
    );
    assert_eq!(
        fs::read(repo_root.join("src.txt")).expect("local fixture bytes after bootstrap"),
        local_bytes
    );

    let initialized_line = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("initialized remote preflight");
    let reused = ensure_remote_base_line_snapshot_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
        &initialized_line,
    )
    .expect("reuse initialized remote base");
    assert_eq!(reused["initialized"], json!(false));
    assert_eq!(reused["head_snapshot_id"], json!(local_snapshot_id));
    assert_eq!(reused["reason"], json!("remote_base_already_initialized"));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);

    let feature = ensure_task_feature_line(
        &repo,
        "RCT-LOCAL-SEED",
        "main",
        Some(&local_snapshot_id),
        false,
    )
    .expect("feature Line from local seed");
    assert_eq!(feature["head_snapshot_id"], json!(local_snapshot_id));
}

#[test]
fn task_start_remote_null_head_still_bootstraps_empty_when_local_head_is_null() {
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
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let remote_row = RemoteRow {
        name: "origin".to_string(),
        url: "https://ait.example".to_string(),
        repo_name: Some("fixture-ait".to_string()),
    };
    let mut remote = FakeChangeRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": null,
        })],
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "fixture-ait",
                "namespace": "",
                "tombstoned": false,
            },
            "ci_capabilities": {
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true,
                }
            }
        })),
        ..Default::default()
    };

    let empty_line = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("empty remote preflight");
    let selected = ensure_remote_base_line_snapshot_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
        &empty_line,
    )
    .expect("initialize a truly empty remote base");
    let anchor_snapshot_id =
        required_string_field(&selected, "head_snapshot_id").expect("empty anchor snapshot id");

    assert_eq!(selected["initialized"], json!(true));
    assert_eq!(selected["reason"], json!("remote_null_head_initialized"));
    assert_eq!(selected["seed_source"], json!("detached_empty"));
    assert!(selected["local_seed_snapshot_id"].is_null());
    assert_eq!(
        remote.lines[0]["head_snapshot_id"],
        json!(anchor_snapshot_id)
    );
    assert!(remote.remote_snapshots.contains_key(&anchor_snapshot_id));
    assert_eq!(remote.zstd_plan_requests.len(), 1);
    assert_eq!(remote.zstd_commit_requests.len(), 1);
    let anchor = snapshot_show(&repo, &anchor_snapshot_id).expect("empty anchor readback");
    assert_eq!(anchor["file_count"], json!(0));
    assert_eq!(anchor["total_bytes"], json!(0));
    assert_eq!(anchor["parent_snapshot_ids"], json!([]));
    assert_eq!(
        line_show(&repo, Some("main")).expect("local main")["head_snapshot_id"],
        JsonValue::Null
    );
}

#[test]
fn task_start_remote_null_head_accepts_same_concurrent_local_seed() {
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
    fs::write(repo_root.join("same.txt"), "same seed\n").expect("same seed fixture");
    let local_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("same seed"),
        false,
    )
    .expect("same local seed snapshot");
    let local_snapshot_id =
        required_string_field(&local_snapshot, "snapshot_id").expect("local snapshot id");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let remote_row = RemoteRow {
        name: "origin".to_string(),
        url: "https://ait.example".to_string(),
        repo_name: Some("fixture-ait".to_string()),
    };
    let mut remote = FakeChangeRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": null,
        })],
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "fixture-ait",
                "namespace": "",
                "tombstoned": false,
            },
            "ci_capabilities": {
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true,
                }
            }
        })),
        zstd_commit_peer_head_once: Some(local_snapshot_id.clone()),
        ..Default::default()
    };
    let empty_line = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("empty remote preflight");

    let selected = ensure_remote_base_line_snapshot_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
        &empty_line,
    )
    .expect("same concurrent seed should be idempotent");

    assert_eq!(selected["initialized"], json!(true));
    assert_eq!(
        selected["reason"],
        json!("remote_null_head_seeded_from_local_main_after_uncertain_response")
    );
    assert_eq!(selected["head_snapshot_id"], json!(local_snapshot_id));
    assert_eq!(
        remote.lines[0]["head_snapshot_id"],
        json!(local_snapshot_id)
    );
    assert!(remote.remote_snapshots.contains_key(&local_snapshot_id));
    assert_eq!(remote.zstd_commit_requests.len(), 1);
}

#[test]
fn task_start_remote_null_head_rejects_a_different_concurrent_winner() {
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
    fs::write(repo_root.join("src.txt"), "peer base\n").expect("peer base fixture");
    let peer_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("peer base"),
        false,
    )
    .expect("peer base snapshot");
    let peer_snapshot_id =
        required_string_field(&peer_snapshot, "snapshot_id").expect("peer snapshot id");
    fs::write(repo_root.join("src.txt"), "selected local head\n")
        .expect("selected local head fixture");
    let local_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("selected local head"),
        false,
    )
    .expect("selected local head snapshot");
    let local_snapshot_id =
        required_string_field(&local_snapshot, "snapshot_id").expect("local snapshot id");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let remote_row = RemoteRow {
        name: "origin".to_string(),
        url: "https://ait.example".to_string(),
        repo_name: Some("fixture-ait".to_string()),
    };
    let mut remote = FakeChangeRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": null,
        })],
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "fixture-ait",
                "namespace": "",
                "tombstoned": false,
            },
            "ci_capabilities": {
                "remote_sync_capabilities": {
                    "zstd_pack_bulk": true,
                }
            }
        })),
        remote_snapshots: BTreeMap::from([(
            peer_snapshot_id.clone(),
            json!({"snapshot_id": peer_snapshot_id}),
        )]),
        zstd_commit_peer_head_once: Some(peer_snapshot_id.clone()),
        ..Default::default()
    };
    let empty_line = task_start_remote_base_line_preflight_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("empty remote preflight");

    let error = ensure_remote_base_line_snapshot_with_task_remote(
        &repo,
        &remote_row,
        &mut remote,
        "fixture-ait",
        "main",
        &empty_line,
    )
    .expect_err("a different concurrent head must fail closed");

    assert!(error.contains("Refusing to create a Task or Change from a different ancestry"));
    assert!(error.contains(&peer_snapshot_id));
    assert!(error.contains(&local_snapshot_id));
    assert_eq!(remote.lines[0]["head_snapshot_id"], json!(peer_snapshot_id));
    assert!(!remote.remote_snapshots.contains_key(&local_snapshot_id));
    assert_eq!(
        line_show(&repo, Some("main")).expect("local main")["head_snapshot_id"],
        json!(local_snapshot_id)
    );
}

#[test]
fn worktree_remote_change_read_accepts_change_remote_trait() {
    let mut remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-WORKTREE",
            "selected_patchset_id": "RCP-WORKTREE-1"
        })],
        ..Default::default()
    };

    let change =
        worktree_remote_change_read_with_task_remote(&mut remote, "fixture-ait", "RCC-WORKTREE")
            .expect("read remote worktree change");
    assert_eq!(change["selected_patchset_id"], json!("RCP-WORKTREE-1"));

    let err =
        worktree_remote_change_read_with_task_remote(&mut remote, "fixture-ait", "RCC-MISSING")
            .expect_err("missing change should fail");
    assert!(err.contains("Unknown change"));
}

#[test]
fn worktree_remote_patchset_read_accepts_closeout_remote_trait() {
    let mut remote = FakeWorkspaceCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-WORKTREE-1".to_string(),
            json!({
                "patchset_id": "RCP-WORKTREE-1",
                "change_id": "RCC-WORKTREE",
                "revision_snapshot_id": "SNP-WORKTREE"
            }),
        )]),
        ..Default::default()
    };

    let patchset = worktree_remote_patchset_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        "RCP-WORKTREE-1",
        "RCC-WORKTREE",
    )
    .expect("read remote worktree patchset");
    assert_eq!(patchset["revision_snapshot_id"], json!("SNP-WORKTREE"));

    let err = worktree_remote_patchset_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        "RCP-MISSING",
        "RCC-WORKTREE",
    )
    .expect_err("missing patchset should fail");
    assert!(err.contains("Unknown patchset"));
}

#[test]
fn worktree_remote_patchset_revision_candidate_accepts_remote_traits() {
    let tmp = tempdir().expect("tempdir");
    let repo_root = tmp.path();
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
    let snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("worktree recreate candidate fixture"),
        false,
    )
    .expect("create snapshot");
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut task_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-WORKTREE",
            "current_patchset_id": "RCP-WORKTREE-1",
        })],
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkspaceCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-WORKTREE-1".to_string(),
            json!({
                "patchset_id": "RCP-WORKTREE-1",
                "change_id": "RCC-WORKTREE",
                "revision_snapshot_id": snapshot_id,
            }),
        )]),
        ..Default::default()
    };

    let candidate = worktree_remote_patchset_revision_candidate_with_remotes(
        &repo,
        &mut task_remote,
        &mut closeout_remote,
        "fixture-ait",
        "RCC-WORKTREE",
    )
    .expect("derive remote patchset revision candidate")
    .expect("candidate should be locally available");

    assert_eq!(candidate["source"], json!("remote_patchset_revision"));
    assert_eq!(candidate["snapshot_id"], json!(snapshot_id));
    assert_eq!(candidate["change_id"], json!("RCC-WORKTREE"));
    assert_eq!(candidate["patchset_id"], json!("RCP-WORKTREE-1"));
    assert_eq!(closeout_remote.patchset_reads, vec!["RCP-WORKTREE-1"]);

    let mut missing_snapshot_task_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-MISSING-SNAPSHOT",
            "selected_patchset_id": "RCP-MISSING-SNAPSHOT-1",
        })],
        ..Default::default()
    };
    let mut missing_snapshot_closeout_remote = FakeWorkspaceCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-MISSING-SNAPSHOT-1".to_string(),
            json!({
                "patchset_id": "RCP-MISSING-SNAPSHOT-1",
                "change_id": "RCC-MISSING-SNAPSHOT",
                "revision_snapshot_id": "SNP-00000000DEAD",
            }),
        )]),
        ..Default::default()
    };
    let missing_snapshot = worktree_remote_patchset_revision_candidate_with_remotes(
        &repo,
        &mut missing_snapshot_task_remote,
        &mut missing_snapshot_closeout_remote,
        "fixture-ait",
        "RCC-MISSING-SNAPSHOT",
    )
    .expect("missing local snapshot should not fail candidate lookup");
    assert!(missing_snapshot.is_none());
}

#[test]
fn worktree_local_binding_helpers_accept_local_store_traits() {
    let task_store = FakeTaskStore::default();
    let change_store = FakeChangeStore::default();
    task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Worktree bound task",
        "Exercise worktree local task lookup",
        Some("LCT"),
        None,
        None,
        None,
    )
    .expect("create worktree local task");
    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-1",
        "Worktree bound change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create worktree local change");

    let task = worktree_local_task_for_worktree_with_task_store(&task_store, Some(" LCT-1 "))
        .expect("task lookup")
        .expect("task present");
    assert_eq!(task["title"], json!("Worktree bound task"));

    let change = worktree_local_change_for_worktree_with_change_store(&change_store, Some("LCC-1"))
        .expect("change lookup")
        .expect("change present");
    assert_eq!(change["fork_snapshot_id"], json!("SNP-BASE"));

    let missing_task =
        worktree_local_task_for_worktree_with_task_store(&task_store, Some("LCT-MISSING"))
            .expect("missing task lookup");
    assert!(missing_task.is_none());

    let empty_change =
        worktree_local_change_for_worktree_with_change_store(&change_store, Some(" "))
            .expect("empty change lookup");
    assert!(empty_change.is_none());
}

#[test]
fn line_change_usage_index_accepts_change_store_trait() {
    let store = FakeLineChangeUsageStore {
        changes: vec![
            json!({
                "change_id": "LCC-2",
                "base_line": " feature/demo ",
                "status": "draft",
            }),
            json!({
                "change_id": "LCC-1",
                "base_line": "feature/demo",
                "status": "active",
            }),
            json!({
                "change_id": "LCC-LANDED",
                "base_line": "feature/demo",
                "status": "landed",
            }),
            json!({
                "change_id": "LCC-ARCHIVED",
                "base_line": "main",
                "status": "archived",
            }),
            json!({
                "change_id": " ",
                "base_line": "main",
                "status": "draft",
            }),
            json!({
                "change_id": "LCC-MAIN",
                "base_line": "main",
                "status": "review",
            }),
        ],
    };

    let index = line_change_usage_index_with_change_store(&store).expect("line change usage index");
    assert_eq!(
        index.get("feature/demo"),
        Some(&vec!["LCC-1".to_string(), "LCC-2".to_string()])
    );
    assert_eq!(index.get("main"), Some(&vec!["LCC-MAIN".to_string()]));
}

#[test]
fn worktree_snapshot_distance_accepts_snapshot_store_trait() {
    let store = FakeSnapshotChainStore {
        chains: BTreeMap::from([(
            "SNP-C".to_string(),
            vec![
                "SNP-A".to_string(),
                "SNP-B".to_string(),
                "SNP-C".to_string(),
            ],
        )]),
        ..Default::default()
    };

    assert_eq!(
        snapshot_distance_if_ancestor_with_snapshot_store(&store, Some(" SNP-A "), Some("SNP-C"))
            .expect("distance through snapshot store"),
        Some(2)
    );
    assert_eq!(
        snapshot_distance_if_ancestor_with_snapshot_store(&store, Some("SNP-B"), Some("SNP-C"))
            .expect("intermediate distance"),
        Some(1)
    );
    assert_eq!(
        snapshot_distance_if_ancestor_with_snapshot_store(&store, Some("SNP-X"), Some("SNP-C"))
            .expect("unrelated ancestor"),
        None
    );
    assert_eq!(
        snapshot_distance_if_ancestor_with_snapshot_store(&store, Some("SNP-A"), Some(" "))
            .expect("empty snapshot id"),
        None
    );
}

#[test]
fn worktree_snapshot_distance_follows_alternate_merge_parent() {
    let store = FakeSnapshotChainStore {
        parents: BTreeMap::from([
            ("SNP-ROOT".to_string(), vec![]),
            ("SNP-LEFT".to_string(), vec!["SNP-ROOT".to_string()]),
            ("SNP-RIGHT".to_string(), vec!["SNP-ROOT".to_string()]),
            (
                "SNP-MERGE".to_string(),
                vec!["SNP-LEFT".to_string(), "SNP-RIGHT".to_string()],
            ),
        ]),
        ..Default::default()
    };

    assert_eq!(
        snapshot_distance_if_ancestor_with_snapshot_store(
            &store,
            Some("SNP-RIGHT"),
            Some("SNP-MERGE")
        )
        .expect("alternate parent distance"),
        Some(1)
    );
}

#[test]
fn worktree_snapshot_distance_short_circuits_and_reuses_parent_reads() {
    let store = FakeSnapshotChainStore {
        chains: BTreeMap::from([(
            "SNP-HEAD".to_string(),
            (0..=1_000)
                .map(|ordinal| format!("SNP-{ordinal:04}"))
                .chain(std::iter::once("SNP-HEAD".to_string()))
                .collect(),
        )]),
        ..Default::default()
    };

    assert_eq!(
        snapshot_distance_if_ancestor_with_snapshot_store(
            &store,
            Some("SNP-HEAD"),
            Some("SNP-HEAD")
        )
        .expect("equal snapshot distance"),
        Some(0)
    );
    assert_eq!(
        store.parent_link_reads.get(),
        0,
        "equal Snapshot IDs must not read history"
    );

    let mut cache = SnapshotAncestorDistanceCache::default();
    assert_eq!(
        snapshot_distance_if_ancestor_with_snapshot_store_and_cache(
            &store,
            Some("SNP-0998"),
            Some("SNP-HEAD"),
            &mut cache,
        )
        .expect("bounded near-head distance"),
        Some(3)
    );
    let reads_after_first_query = store.parent_link_reads.get();
    assert_eq!(reads_after_first_query, 4);

    assert_eq!(
        snapshot_distance_if_ancestor_with_snapshot_store_and_cache(
            &store,
            Some("SNP-0999"),
            Some("SNP-HEAD"),
            &mut cache,
        )
        .expect("cached near-head distance"),
        Some(2)
    );
    assert_eq!(
        store.parent_link_reads.get(),
        reads_after_first_query,
        "overlapping distance queries must reuse already-read parent links"
    );
}

#[test]
fn remote_zstd_snapshot_upload_includes_both_parents_before_merge_child() {
    let source_tmp = tempdir().expect("source repo tempdir");
    let source_root = source_tmp.path();
    init_repo(&InitRequest {
        root: source_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init source repo");
    let source_repo = RepoRuntime::discover_from_path(source_root).expect("source runtime");
    fs::write(source_root.join("src.txt"), "root\n").unwrap();
    let root = create_local_snapshot(
        source_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("root"),
        false,
    )
    .unwrap();
    let root_id = required_string_field(&root, "snapshot_id").unwrap();

    fs::write(source_root.join("src.txt"), "left\n").unwrap();
    let left = create_local_snapshot(
        source_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("left"),
        false,
    )
    .unwrap();
    let left_id = required_string_field(&left, "snapshot_id").unwrap();

    source_repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .lines()
        .set_line_head("main", Some(&root_id), "2026-07-19T00:00:01Z")
        .unwrap();
    fs::write(source_root.join("src.txt"), "right\n").unwrap();
    let right = create_local_snapshot(
        source_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("right"),
        false,
    )
    .unwrap();
    let right_id = required_string_field(&right, "snapshot_id").unwrap();

    let content = source_repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content();
    let left_record = content
        .snapshots()
        .snapshot_by_id(&left_id)
        .unwrap()
        .expect("left metadata");
    let left_root = content.snapshot_tree_root_locator(&left_id).unwrap();
    let merge_id = snapshot_id_from_hash48(0x0B0C_0D0E_0F10);
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
            manifest_hash: "cd".repeat(32),
            message: Some("merge".to_string()),
            line_name: "main".to_string(),
            snapshot_kind: "line".to_string(),
            file_count: left_record.file_count,
            total_bytes: left_record.total_bytes,
            created_at: "2026-07-19T00:00:02Z".to_string(),
        },
    )
    .expect("record merge");
    let deep_stats = content
        .storage_stats_with_options(LocalContentStatsOptions {
            compute_reachability: true,
        })
        .expect("deep GC reachability across merge DAG");
    assert!(deep_stats["reachable_blob_count"].as_i64().unwrap() >= 3);
    assert!(deep_stats["metadata_summary"]["tree_reachability_error"].is_null());

    let mut upload_remote = FakeLineSnapshotRemote::default();
    let uploaded = remote_sync::upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
        &source_repo,
        &mut upload_remote,
        "fixture-ait",
        &merge_id,
        None,
        &RemoteSyncCapabilities::with_zstd_pack_bulk().with_snapshot_dag_v2(),
    )
    .expect("upload complete diamond");
    assert_eq!(uploaded["checked_snapshots"], 4);
    assert_eq!(uploaded["uploaded_snapshots"], 4);
    let planned_snapshot_ids = upload_remote.zstd_plan_requests[0]["snapshot_ids"]
        .as_array()
        .expect("planned diamond snapshots")
        .iter()
        .filter_map(JsonValue::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        planned_snapshot_ids,
        BTreeSet::from([
            root_id.as_str(),
            left_id.as_str(),
            right_id.as_str(),
            merge_id.as_str(),
        ])
    );
    let mut bounded_remote = FakeLineSnapshotRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": right_id,
        })],
        remote_snapshots: BTreeMap::from([
            (
                root_id.clone(),
                json!({"repo_name": "fixture-ait", "snapshot_id": root_id}),
            ),
            (
                right_id.clone(),
                json!({"repo_name": "fixture-ait", "snapshot_id": right_id}),
            ),
        ]),
        ..Default::default()
    };
    let bounded = remote_sync::upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
        &source_repo,
        &mut bounded_remote,
        "fixture-ait",
        &merge_id,
        Some("main"),
        &RemoteSyncCapabilities::with_zstd_pack_bulk().with_snapshot_dag_v2(),
    )
    .expect("bounded upload subtracts complete alternate-parent closure");
    assert_eq!(bounded["checked_snapshots"], 2);
    assert_eq!(bounded["bounded_by_snapshot_id"], right_id);
    let bounded_ids = bounded_remote.zstd_plan_requests[0]["snapshot_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(JsonValue::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        bounded_ids,
        BTreeSet::from([left_id.as_str(), merge_id.as_str()])
    );
}

#[test]
fn snapshot_binary_db_selected_blob_ensure_writes_without_retired_backend_fallback() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    fs::create_dir_all(repo_root.join(".ait")).expect("repo .ait");
    fs::write(
        repo_root.join(".ait/config.json"),
        r#"{"repo_name":"fixture-ait","snapshot_binary_db_storage":"binary"}"#,
    )
    .expect("repo config");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");

    let blob_id = blob_ensure_bytes(&repo, b"ensured blob bytes\n", Some("ensured.txt"))
        .expect("selected Binary DB blob ensure");

    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(repo_root)
        .expect("selected Binary DB snapshot store");
    let bytes = store
        .read_blob_bytes(&blob_id)
        .expect("read ensured Binary DB blob");
    assert_eq!(bytes, b"ensured blob bytes\n");

    let duplicate = blob_ensure_bytes(&repo, b"ensured blob bytes\n", Some("ensured.txt"))
        .expect("idempotent selected Binary DB blob ensure");
    assert_eq!(duplicate, blob_id);
}
