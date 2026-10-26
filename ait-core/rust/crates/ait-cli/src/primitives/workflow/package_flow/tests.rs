use super::*;
use crate::init_surface::{init_repo, InitRequest};
use ait_core::line_store::LineStore;
use ait_core::local_snapshot::SnapshotAuthoringOptions;
use ait_core::task_store::TaskStore;
use ait_core::task_workflow_http_adapter::{
    TaskWorkflowHttpClientResult, TaskWorkflowSnapshotMetadataReader,
};
use ait_core::workflow_package::PackageCompletion;

mod authority;
mod finish_cleanup;

struct AncestryRemote {
    rows: BTreeMap<String, JsonValue>,
    reads: Vec<String>,
}

impl TaskWorkflowSnapshotMetadataReader for AncestryRemote {
    fn get_remote_snapshot(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        assert_eq!(repo_name, "fixture");
        assert!(!include_content && path.is_none());
        self.reads.push(snapshot_id.into());
        Ok(self.rows[snapshot_id].clone())
    }
}

fn remote_boundary(fixture: &Fixture, content_id: &str, id: &str) -> JsonValue {
    use ait_core::binary_db::BinaryDbCommandScope;
    use ait_core::content_binary_db::{
        BinaryDbContentWriteCoordinator, BinaryDbSnapshotWriteInput,
    };
    let stores = fixture
        .repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>();
    let content = stores.content();
    let source = content
        .snapshots()
        .snapshot_by_id(content_id)
        .unwrap()
        .unwrap();
    let root = content.snapshot_tree_root_locator(content_id).unwrap();
    let input = BinaryDbSnapshotWriteInput {
        snapshot_id: id.into(),
        parent_snapshot_ids: vec![fixture.base.clone()],
        root_tree_pack_id: root.root_tree_pack_id,
        root_entry_ordinal: root.root_entry_ordinal,
        manifest_hash: source.manifest_hash,
        message: source.message,
        line_name: "main".into(),
        snapshot_kind: "line".into(),
        file_count: source.file_count,
        total_bytes: source.total_bytes,
        created_at: source.created_at,
    };
    BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    )
    .record_snapshot_at_remote_head_history_boundary(
        BinaryDbCommandScope::RemoteSyncLocalImport,
        &input,
    )
    .unwrap();
    json!({
        "snapshot_id":id,"repository_index":0,"repo_name":"fixture",
        "remote_head_history_boundary":false,"parent_snapshot_ids":input.parent_snapshot_ids,
        "parent_snapshot_id":fixture.base,"primary_parent_snapshot_id":fixture.base,
        "root_tree_pack_id":input.root_tree_pack_id,"root_entry_ordinal":input.root_entry_ordinal,
        "manifest_hash":input.manifest_hash,"message":input.message,"line_name":input.line_name,
        "snapshot_kind":input.snapshot_kind,"file_count":input.file_count,"total_bytes":input.total_bytes,
        "created_at":input.created_at,
    })
}

fn assert_boundary_unchanged(fixture: &Fixture, id: &str) {
    let stores = fixture
        .repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>();
    let content = stores.content();
    let snapshots = content.snapshots();
    let read = snapshots.begin_read_txn();
    let row = snapshots.get_snapshot_view(&read, id).unwrap().unwrap();
    assert!(row.record.is_remote_head_history_boundary());
    assert!(row.parent_snapshot_ids.is_empty());
}

struct Fixture {
    temp: tempfile::TempDir,
    repo: RepoRuntime,
    base: String,
    original: String,
    target: String,
    id: String,
    remote: RemoteRow,
    entry: JsonValue,
}

fn fixture() -> Fixture {
    fixture_with_receipt_boundary(false)
}

#[test]
fn no_op_final_task_is_not_omitted_or_rebound_to_the_previous_snapshot() {
    let fixture = fixture();
    let tasks = fixture.repo.task_store().unwrap();
    let task = TaskStore::create_task(
        &tasks,
        "fixture",
        "No-op source",
        "Do not invent ownership",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let id = required_string_field(&task, "task_id").unwrap();
    tasks
        .finish_local_task(
            &id,
            &fixture.original,
            "main",
            Some(&fixture.original),
            None,
            None,
        )
        .unwrap();
    let entry = workflow_completed_local_entry_if_present(&fixture.repo, &id, Some("origin"))
        .unwrap()
        .unwrap();
    let before = tasks
        .snapshot_ownership_rows(std::slice::from_ref(&fixture.original))
        .unwrap();
    let error = prepare_source_history_at_base(
        &fixture.repo,
        &fixture.remote,
        &entry,
        &fixture.base,
        None,
        None,
    )
    .err()
    .unwrap();
    assert!(
        error.contains("ambiguous") || error.contains("no exactly owned"),
        "{error}"
    );
    assert_eq!(
        tasks
            .snapshot_ownership_rows(std::slice::from_ref(&fixture.original))
            .unwrap(),
        before
    );
    assert_eq!(before[0]["task_id"], fixture.id);
    assert_eq!(
        TaskStore::get_task(&tasks, &id).unwrap()["status"],
        "completed"
    );
}

#[test]
fn incomparable_merge_bases_and_owned_merge_replay_stop_before_capture() {
    let fixture = fixture();
    let content = fixture
        .repo
        .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .unwrap();
    let tasks = fixture.repo.task_store().unwrap();
    let task = TaskStore::create_task(
        &tasks,
        "fixture",
        "Merge source",
        "Keep original DAG",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    fixture
        .repo
        .line_store()
        .unwrap()
        .create_line(
            "merge-source",
            Some(&fixture.original),
            "2026-10-21T00:00:00Z",
        )
        .unwrap();
    let owner = SnapshotWorkflowBinding {
        task_id: task_id.clone(),
        change_id: String::new(),
        line_name: "merge-source".into(),
    };
    let merged = content
        .create_snapshot_content_with_workflow_binding(
            "fixture",
            "merge-source",
            &[fixture.original.clone(), fixture.target.clone()],
            Some("Original ordered merge"),
            true,
            SnapshotAuthoringOptions::default(),
            Some((&owner, "")),
        )
        .unwrap();
    let merged = required_string_field(&merged, "snapshot_id").unwrap();
    tasks
        .finish_local_task(
            &task_id,
            &merged,
            "main",
            Some(&fixture.original),
            None,
            None,
        )
        .unwrap();
    let remote_id = "SNP-ABCDEF000009";
    let mut remote_row = remote_boundary(&fixture, &fixture.target, remote_id);
    let before = content.snapshots().list_line_snapshots().unwrap();
    remote_row["parent_snapshot_ids"] = json!([fixture.original, fixture.target]);
    remote_row["primary_parent_snapshot_id"] = json!(fixture.original);
    remote_row["parent_snapshot_id"] = json!(fixture.original);
    let mut remote = AncestryRemote {
        rows: BTreeMap::from([(remote_id.into(), remote_row)]),
        reads: vec![],
    };
    let error = PackageAncestry::load(&fixture.repo, &mut remote, "fixture", &merged, remote_id)
        .err()
        .unwrap();
    assert!(error.contains("multiple incomparable"), "{error}");
    let entry = workflow_completed_local_entry_if_present(&fixture.repo, &task_id, Some("origin"))
        .unwrap()
        .unwrap();
    let error = prepare_source_history_at_base(
        &fixture.repo,
        &fixture.remote,
        &entry,
        &fixture.base,
        None,
        None,
    )
    .err()
    .unwrap();
    assert!(error.contains("owned merge Snapshot"), "{error}");
    assert_eq!(content.snapshots().list_line_snapshots().unwrap(), before);
}

#[test]
fn projected_line_utf8_expansion_and_terminal_whole_batch_reject_before_replay() {
    use ait_core::change_store::ChangeStore;
    let fixture = fixture();
    let long_base = "界".repeat(11000);
    let tasks = fixture.repo.task_store().unwrap();
    let task = TaskStore::create_task(
        &tasks,
        "fixture",
        "UTF-8 source",
        "Keep source Line bytes",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    fixture
        .repo
        .line_store()
        .unwrap()
        .create_line(&long_base, Some(&fixture.base), "2026-10-21T00:00:00Z")
        .unwrap();
    ChangeStore::create_change(
        &fixture.repo.change_store().unwrap(),
        &task_id,
        "fixture",
        "Historical revision",
        &long_base,
        None,
        Some(&fixture.base),
    )
    .unwrap();
    let owner = SnapshotWorkflowBinding {
        task_id: task_id.clone(),
        change_id: "C-01".into(),
        line_name: long_base.clone(),
    };
    let snapshot = fixture
        .repo
        .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .unwrap()
        .create_snapshot_content_with_workflow_binding(
            "fixture",
            &long_base,
            std::slice::from_ref(&fixture.original),
            Some("UTF-8 original"),
            true,
            SnapshotAuthoringOptions::default(),
            Some((&owner, "")),
        )
        .unwrap();
    let snapshot = required_string_field(&snapshot, "snapshot_id").unwrap();
    tasks
        .finish_local_task(
            &task_id,
            &snapshot,
            "main",
            Some(&fixture.original),
            Some(&format!("{task_id}/C-01")),
            None,
        )
        .unwrap();
    let completed =
        workflow_completed_local_entry_if_present(&fixture.repo, &task_id, Some("origin"))
            .unwrap()
            .unwrap();
    let mut entry = completed["state"].clone();
    entry["local_task_id"] = json!(task_id);
    assert!(preflight_source_entries(&fixture.repo, &[entry.clone()])
        .unwrap_err()
        .contains("projected Line bounds"));
    entry["task"]["published_task_id"] = json!("RT-1001");
    assert!(preflight_source_entries(&fixture.repo, &[entry])
        .unwrap_err()
        .contains("already published whole batch"));
}

#[test]
fn resumed_ready_requires_verified_mode_even_with_an_explicit_override() {
    for explicit in [None, Some("human_only")] {
        let mut fixture = fixture();
        let identity = pinned(&fixture);
        fixture
            .repo
            .config
            .insert("default_author_mode".into(), json!("ai_only_experimental"));
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        configure_remote(
            &mut fixture.repo,
            &format!("http://{}", server.server_addr()),
        );
        let expected = identity.clone();
        let thread = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap();
            assert_eq!(request.method().as_str(), "GET");
            assert!(request.url().ends_with("/worker-jobs/0"));
            let response = json!({"job":{"repository_index":0,"worker_job_index":0,"patchset_index":null,"tombstoned":false},"package_execution":{"expected_identity":expected,"ci_state":"passed","availability":"available","recoverable":true}});
            request
                .respond(tiny_http::Response::from_string(response.to_string()))
                .unwrap();
            assert!(server
                .recv_timeout(Duration::from_millis(100))
                .unwrap()
                .is_none());
        });
        let result = workflow_package_ready(
            &fixture.repo,
            &fixture.id,
            Some("origin"),
            Some(&identity),
            true,
            PackageReadyOptions {
                author_mode: explicit,
                ..Default::default()
            },
            None::<fn(&mut JsonValue) -> Result<bool, String>>,
        )
        .unwrap();
        thread.join().unwrap();
        assert_eq!(result["next_action"]["code"], "author_mode_required_action");
        assert_eq!(result["expected_identity"], json!(identity));
    }
}

#[test]
fn package_alias_and_conflicting_origin_owners_stop_before_plan_sync_or_capture() {
    let fixture = fixture();
    let mut alias = fixture.remote.clone();
    alias.name = "another-server".into();
    assert!(package_client(&fixture.repo, &alias, "fixture")
        .err()
        .unwrap()
        .contains("contextual origin"));
    let originals = fixture
        .repo
        .task_store()
        .unwrap()
        .completed_task_source_snapshots(&fixture.id)
        .unwrap();
    let mut entry = fixture.entry["state"].clone();
    entry["local_task_id"] = json!(fixture.id);
    entry["task"]["published_task_id"] = json!("RT-1001");
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let remote = RemoteRow {
        url: format!("http://{}", server.server_addr()),
        ..fixture.remote.clone()
    };
    let expected_id = originals[0].0.clone();
    let handle = std::thread::spawn(move || {
        loop {
            let request = server
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap();
            assert_eq!(request.method().as_str(), "GET");
            let snapshot = request.url().contains("/snapshots/");
            let response = if snapshot {
                json!({"repository_index":0,"snapshot_id":expected_id,"workflow_owner":{"task_id":"RT-1002","change_ref":null}})
            } else if request.url().ends_with("/patchsets") {
                json!([])
            } else {
                json!({"repository_index":0,"task_id":"RT-1001"})
            };
            request
                .respond(tiny_http::Response::from_string(response.to_string()))
                .unwrap();
            if snapshot {
                break;
            }
        }
        assert!(server
            .recv_timeout(Duration::from_millis(100))
            .unwrap()
            .is_none());
    });
    let inventory = BTreeMap::from([(fixture.id.clone(), originals)]);
    let before = TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap();
    let result = verify_published_source_authority(&fixture.repo, &remote, &[entry], &inventory);
    assert!(result
        .unwrap_err()
        .contains("conflicts with the original publication authority"));
    handle.join().unwrap();
    assert_eq!(
        TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap(),
        before
    );
    for field in [
        None,
        Some("manifest_hash"),
        Some("line_name"),
        Some("parent_snapshot_ids"),
        Some("created_at"),
    ] {
        let originals = fixture
            .repo
            .task_store()
            .unwrap()
            .completed_task_source_snapshots(&fixture.id)
            .unwrap();
        let mut row = snapshot_show(&fixture.repo, &originals[0].0).unwrap();
        row["repository_index"] = json!(0);
        row["workflow_owner"] = json!({"task_id":"RT-1001","change_ref":null});
        if let Some(field) = field {
            row[field] = match field {
                "parent_snapshot_ids" => json!([]),
                "created_at" => json!("1"),
                _ => json!("different immutable value"),
            };
        }
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let remote = RemoteRow {
            url: format!("http://{}", server.server_addr()),
            ..fixture.remote.clone()
        };
        let handle = std::thread::spawn(move || loop {
            let request = server
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap();
            assert_eq!(request.method().as_str(), "GET");
            let snapshot = request.url().contains("/snapshots/");
            let response = if snapshot {
                row.clone()
            } else if request.url().ends_with("/patchsets") {
                json!([])
            } else {
                json!({"repository_index":0,"task_id":"RT-1001"})
            };
            request
                .respond(tiny_http::Response::from_string(response.to_string()))
                .unwrap();
            if snapshot {
                break;
            }
        });
        let mut entry = fixture.entry["state"].clone();
        entry["local_task_id"] = json!(fixture.id);
        entry["task"]["published_task_id"] = json!("RT-1001");
        let result = verify_published_source_authority(
            &fixture.repo,
            &remote,
            &[entry],
            &BTreeMap::from([(fixture.id.clone(), originals)]),
        );
        handle.join().unwrap();
        assert_eq!(result.is_ok(), field.is_none(), "{field:?}: {result:?}");
    }
}

fn fixture_with_receipt_boundary(self_boundary: bool) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: temp.path().into(),
        name: Some("fixture".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let mut repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    repo.config.insert("repository_index".into(), json!(0));
    let content = repo
        .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .unwrap();
    let capture = |line: &str,
                   parents: &[String],
                   message: &str,
                   binding: Option<&SnapshotWorkflowBinding>| {
        required_string_field(
            &content
                .create_snapshot_content_with_workflow_binding(
                    "fixture",
                    line,
                    parents,
                    Some(message),
                    binding.is_some(),
                    SnapshotAuthoringOptions::default(),
                    binding.map(|owner| (owner, "")),
                )
                .unwrap(),
            "snapshot_id",
        )
        .unwrap()
    };
    fs::write(temp.path().join("base.txt"), "base").unwrap();
    let base = capture("main", &[], "base", None);
    set_local_line_head(&repo, "main", Some(&base)).unwrap();
    let lines = repo.line_store().unwrap();
    lines
        .create_line("feature", Some(&base), "2026-10-20T00:00:00Z")
        .unwrap();
    let tasks = repo.task_store().unwrap();
    let task = TaskStore::create_task(
        &tasks,
        "fixture",
        "Feature",
        "Preserve source receipt",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let id = required_string_field(&task, "task_id").unwrap();
    fs::write(temp.path().join("feature.txt"), "feature").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            temp.path().join("feature.txt"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        std::os::unix::fs::symlink("feature.txt", temp.path().join("feature-link")).unwrap();
    }
    let owner = SnapshotWorkflowBinding {
        task_id: id.clone(),
        change_id: String::new(),
        line_name: "feature".into(),
    };
    let original = capture(
        "feature",
        std::slice::from_ref(&base),
        "feature",
        Some(&owner),
    );
    if self_boundary {
        set_local_line_head(&repo, "main", Some(&original)).unwrap();
    }
    tasks
        .finish_local_task(
            &id,
            &original,
            "main",
            Some(if self_boundary { &original } else { &base }),
            None,
            None,
        )
        .unwrap();
    fs::remove_file(temp.path().join("feature.txt")).unwrap();
    #[cfg(unix)]
    fs::remove_file(temp.path().join("feature-link")).unwrap();
    fs::write(temp.path().join("remote.txt"), "remote").unwrap();
    let target = capture("main", std::slice::from_ref(&base), "remote", None);
    let remote = RemoteRow {
        name: "origin".into(),
        url: "http://127.0.0.1:1".into(),
        repo_name: Some("fixture".into()),
    };
    let entry = workflow_completed_local_entry_if_present(&repo, &id, Some("origin"))
        .unwrap()
        .unwrap();
    Fixture {
        temp,
        repo,
        base,
        original,
        target,
        id,
        remote,
        entry,
    }
}

#[test]
fn null_remote_package_boundary_preserves_receipt_and_recovers_only_earlier_ownership() {
    for self_boundary in [false, true] {
        let fixture = fixture_with_receipt_boundary(self_boundary);
        let before = TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap();
        let (base, recovered) =
            workflow_null_remote_base_from_entry(&fixture.repo, &fixture.entry).unwrap();
        assert_eq!(base, fixture.base);
        assert_ne!(base, fixture.original);
        assert_eq!(recovered, self_boundary);
        assert_eq!(
            TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap(),
            before
        );
    }
}

#[test]
fn null_remote_package_rejects_final_snapshot_without_an_earlier_boundary() {
    let fixture = fixture();
    let tasks = fixture.repo.task_store().unwrap();
    let task = TaskStore::create_task(
        &tasks,
        "fixture",
        "Root-only Task",
        "No earlier boundary",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let id = required_string_field(&task, "task_id").unwrap();
    let owner = SnapshotWorkflowBinding {
        task_id: id.clone(),
        change_id: String::new(),
        line_name: "feature".into(),
    };
    let root = fixture
        .repo
        .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .unwrap()
        .create_snapshot_content_with_workflow_binding(
            "fixture",
            "feature",
            &[],
            Some("No earlier boundary"),
            true,
            SnapshotAuthoringOptions::default(),
            Some((&owner, "")),
        )
        .unwrap();
    let root = required_string_field(&root, "snapshot_id").unwrap();
    set_local_line_head(&fixture.repo, "main", Some(&root)).unwrap();
    tasks
        .finish_local_task(&id, &root, "main", Some(&root), None, None)
        .unwrap();
    let mut entry = workflow_completed_local_entry_if_present(&fixture.repo, &id, Some("origin"))
        .unwrap()
        .unwrap();
    assert!(workflow_null_remote_base_from_entry(&fixture.repo, &entry)
        .unwrap_err()
        .contains("one exact Snapshot parent"));
    entry["state"]["local_finish"]["target_line"] = json!("feature");
    assert!(workflow_null_remote_base_from_entry(&fixture.repo, &entry)
        .unwrap_err()
        .contains("configured default Line"));
}

#[test]
fn null_remote_package_requires_explicit_well_formed_target_head() {
    assert_eq!(
        workflow_remote_line_head(&json!({"line_name":"main","head_snapshot_id":null}), "main")
            .unwrap(),
        None
    );
    assert_eq!(
        workflow_remote_line_head(
            &json!({"line_name":"main","head_snapshot_id":"SNP-BASE"}),
            "main"
        )
        .unwrap()
        .as_deref(),
        Some("SNP-BASE")
    );
    for line in [
        json!({"line_name":"main"}),
        json!({"line_name":"main","head_snapshot_id":""}),
        json!({"line_name":"main","head_snapshot_id":" SNP-BASE"}),
        json!({"line_name":"main","head_snapshot_id":0}),
        json!({"line_name":"other","head_snapshot_id":null}),
        JsonValue::Null,
    ] {
        assert!(workflow_remote_line_head(&line, "main").is_err());
    }
}

#[test]
fn recovery_retains_existing_exact_owner_repair_for_a_self_boundary_receipt() {
    let fixture = fixture_with_receipt_boundary(true);
    let identity = pinned(&fixture);
    let before = TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap();
    let (_, recovered) =
        recover_source_history(&fixture.repo, &fixture.id, "origin", &identity).unwrap();
    assert_eq!(recovered.len(), 1);
    assert_eq!(recovered[0].local_task_id, fixture.id);
    assert!(recovered[0]
        .snapshots
        .iter()
        .any(|row| row.snapshot_id == fixture.original));
    assert_eq!(
        TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap(),
        before
    );
}

#[test]
fn foreign_boundary_rejects_automatic_rebase_without_rewriting_local_history() {
    let fixture = fixture();
    let repo = &fixture.repo;
    let mut remote = AncestryRemote {
        rows: BTreeMap::from([(
            "SNP-ABCDEF000002".into(),
            remote_boundary(&fixture, &fixture.target, "SNP-ABCDEF000002"),
        )]),
        reads: Vec::new(),
    };
    let tasks = repo.task_store().unwrap();
    let before_task = TaskStore::get_task(&tasks, &fixture.id).unwrap();
    let before_owners = tasks.completed_task_source_snapshots(&fixture.id).unwrap();
    let snapshots = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&repo.workspace_root())
        .unwrap();
    let before = snapshots.list_line_snapshots().unwrap();
    let ancestry = PackageAncestry::load(
        repo,
        &mut remote,
        "fixture",
        &fixture.original,
        "SNP-ABCDEF000002",
    )
    .unwrap();
    let error = prepare_source_history_at_base_with_ancestry(
        repo,
        &fixture.remote,
        &fixture.entry,
        "SNP-ABCDEF000002",
        None,
        None,
        Some(&ancestry),
    )
    .err()
    .unwrap();
    assert!(
        error.contains("BASE_STALE") && error.contains("Reconcile Local history explicitly"),
        "{error}"
    );
    assert_eq!(snapshots.list_line_snapshots().unwrap(), before);
    assert_eq!(
        TaskStore::get_task(&tasks, &fixture.id).unwrap(),
        before_task
    );
    assert_eq!(
        tasks.completed_task_source_snapshots(&fixture.id).unwrap(),
        before_owners
    );
    assert_boundary_unchanged(&fixture, "SNP-ABCDEF000002");
}

#[test]
fn foreign_boundary_rejects_wrong_authority_unknown_history_and_cycles_before_capture() {
    let fixture = fixture();
    let id = "SNP-ABCDEF000003";
    let row = remote_boundary(&fixture, &fixture.target, id);
    let content = fixture
        .repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
            &fixture.repo.workspace_root(),
        )
        .unwrap();
    let before = content.list_line_snapshots().unwrap();
    for (field, value) in [
        ("repository_index", json!(1)),
        ("repo_name", json!("another")),
        ("snapshot_id", json!(fixture.base)),
        ("remote_head_history_boundary", json!(true)),
        ("remote_head_history_boundary", JsonValue::Null),
        ("parent_snapshot_ids", JsonValue::Null),
        ("manifest_hash", json!("ff".repeat(32))),
    ] {
        let mut invalid = row.clone();
        invalid[field] = value;
        let mut remote = AncestryRemote {
            rows: BTreeMap::from([(id.into(), invalid)]),
            reads: Vec::new(),
        };
        assert!(PackageAncestry::load(
            &fixture.repo,
            &mut remote,
            "fixture",
            &fixture.original,
            id
        )
        .is_err());
    }
    let missing = "SNP-ABCDEF000004";
    let mut first = row.clone();
    first["parent_snapshot_ids"] = json!([missing]);
    first["parent_snapshot_id"] = json!(missing);
    first["primary_parent_snapshot_id"] = json!(missing);
    let mut second = row;
    second["snapshot_id"] = json!(missing);
    second["parent_snapshot_ids"] = json!([id]);
    second["parent_snapshot_id"] = json!(id);
    second["primary_parent_snapshot_id"] = json!(id);
    let mut remote = AncestryRemote {
        rows: BTreeMap::from([(id.into(), first), (missing.into(), second)]),
        reads: Vec::new(),
    };
    let error = PackageAncestry::load(&fixture.repo, &mut remote, "fixture", &fixture.original, id)
        .err()
        .unwrap();
    assert!(error.contains("Cycle detected"), "{error}");
    assert_eq!(content.list_line_snapshots().unwrap(), before);
    assert_boundary_unchanged(&fixture, id);
}

#[test]
fn package_selects_existing_snapshots_without_replaying_or_rewriting_receipts() {
    let fixture = fixture();
    let original_files = source_files(&fixture.repo, &fixture.original).unwrap();
    let tasks = fixture.repo.task_store().unwrap();
    let before_task = TaskStore::get_task(&tasks, &fixture.id).unwrap();
    let snapshots = fixture
        .repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
            &fixture.repo.workspace_root(),
        )
        .unwrap();
    let before = snapshots.list_line_snapshots().unwrap();
    let prepared = prepare_source_history_at_base(
        &fixture.repo,
        &fixture.remote,
        &fixture.entry,
        &fixture.base,
        None,
        Some(" Original summary "),
    )
    .unwrap();
    let value = prepared.manifest.to_value().unwrap();
    assert_eq!(value["base_snapshot_id"], fixture.base);
    assert_eq!(value["revision_snapshot_id"], fixture.original);
    assert_eq!(
        value["history_entries"][0]["landed_snapshot_id"],
        fixture.original
    );
    assert_eq!(value["summary"], " Original summary ");
    assert_eq!(prepared.mappings[0].snapshots.len(), 1);
    assert_eq!(
        prepared.mappings[0].snapshots[0].snapshot_id,
        fixture.original
    );
    let identity = PackageWorkflowExpectedIdentity {
        repository_index: 0,
        worker_job_index: 0,
        package_hash: prepared.package.package_hash.clone(),
        base_snapshot_id: fixture.base.clone(),
        revision_snapshot_id: fixture.original.clone(),
    };
    let (_, recovered) =
        recover_source_history(&fixture.repo, &fixture.id, "origin", &identity).unwrap();
    assert_eq!(recovered, prepared.mappings);
    let repeated = prepare_source_history_at_base(
        &fixture.repo,
        &fixture.remote,
        &fixture.entry,
        &fixture.base,
        None,
        Some(" Original summary "),
    )
    .unwrap();
    assert_eq!(repeated.package, prepared.package);
    assert_eq!(snapshots.list_line_snapshots().unwrap(), before);
    assert_eq!(
        TaskStore::get_task(&tasks, &fixture.id).unwrap(),
        before_task
    );
    assert_eq!(
        local_line_head_snapshot_id(&fixture.repo, "main")
            .unwrap()
            .as_deref(),
        Some(fixture.original.as_str())
    );
    let files = source_files(&fixture.repo, &fixture.original).unwrap();
    assert_eq!(files, original_files);
    assert!(files.contains_key("feature.txt") && !files.contains_key("remote.txt"));
    #[cfg(unix)]
    {
        assert!(files.contains_key("feature-link"));
    }
}

fn configure_remote(repo: &mut RepoRuntime, url: &str) {
    use ait_core::remote_store::{RemoteAddRecord, RemoteStore};
    repo.remote_store()
        .unwrap()
        .add_remote(&RemoteAddRecord {
            name: "origin".into(),
            url: url.into(),
            repo_name: Some("fixture".into()),
            make_default: true,
            created_at: "2026-10-20T00:00:00Z".into(),
        })
        .unwrap();
    repo.config
        .insert("user_name".into(), json!("Configured user"));
    repo.config
        .insert("user_email".into(), json!("fixture@example.test"));
}

fn pinned(fixture: &Fixture) -> PackageWorkflowExpectedIdentity {
    let prepared = prepare_source_history_at_base(
        &fixture.repo,
        &fixture.remote,
        &fixture.entry,
        &fixture.base,
        None,
        None,
    )
    .unwrap();
    PackageWorkflowExpectedIdentity {
        repository_index: 0,
        worker_job_index: 0,
        package_hash: prepared.package.package_hash,
        base_snapshot_id: fixture.base.clone(),
        revision_snapshot_id: prepared.manifest.revision_snapshot_id,
    }
}

#[test]
fn completed_tasks_with_mixed_changes_keep_existing_snapshots_with_scoped_64_task_limit() {
    use ait_core::change_store::ChangeStore;
    for (archived_branch, task_count) in [(false, 5), (true, 5), (false, 64), (false, 65)] {
        let mut fixture = fixture();
        fs::remove_file(fixture.temp.path().join("remote.txt")).unwrap();
        fs::write(fixture.temp.path().join("feature.txt"), "feature").unwrap();
        let content = fixture
            .repo
            .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .unwrap();
        let tasks = fixture.repo.task_store().unwrap();
        let changes = fixture.repo.change_store().unwrap();
        let mut previous = fixture.original.clone();
        let mut task_ids = vec![fixture.id.clone()];
        let mut receipts =
            vec![TaskStore::get_task(&tasks, &fixture.id).unwrap()["local_finish"].clone()];
        for number in 2..=task_count {
            let task = TaskStore::create_task(
                &tasks,
                "fixture",
                &format!("Task {number}"),
                "Preserve exact batch owners",
                None,
                None,
                None,
                None,
            )
            .unwrap();
            let id = required_string_field(&task, "task_id").unwrap();
            let line = format!("feature-{number}");
            fixture
                .repo
                .line_store()
                .unwrap()
                .create_line(&line, Some(&previous), "2026-10-20T00:00:00Z")
                .unwrap();
            let mut parent = previous.clone();
            let mut selected = None;
            for revision in 1..=if number == 2 { 2 } else { 1 } {
                let change_id = if number == 2 {
                    let change = ChangeStore::create_change(
                        &changes,
                        &id,
                        "fixture",
                        &format!("Revision {revision}"),
                        &line,
                        None,
                        Some(&parent),
                    )
                    .unwrap();
                    let child = required_string_field(&change, "change_id").unwrap();
                    selected = Some(format!("{id}/{child}"));
                    child
                } else {
                    String::new()
                };
                fs::write(
                    fixture
                        .temp
                        .path()
                        .join(format!("task-{number}-{revision}.txt")),
                    format!("Task {number} revision {revision}"),
                )
                .unwrap();
                let owner = SnapshotWorkflowBinding {
                    task_id: id.clone(),
                    change_id,
                    line_name: line.clone(),
                };
                let capture_parent = if number == 2 && revision == 2 && archived_branch {
                    previous.clone()
                } else {
                    parent
                };
                let snapshot = content
                    .create_snapshot_content_with_workflow_binding(
                        "fixture",
                        &line,
                        &[capture_parent],
                        Some("batch source"),
                        true,
                        SnapshotAuthoringOptions::default(),
                        Some((&owner, "")),
                    )
                    .unwrap();
                parent = required_string_field(&snapshot, "snapshot_id").unwrap();
                if number == 2 && revision == 1 && archived_branch {
                    ChangeStore::close_change(&changes, selected.as_deref().unwrap(), "archived")
                        .unwrap();
                }
            }
            let receipt = tasks
                .finish_local_task(
                    &id,
                    &parent,
                    "main",
                    Some(&previous),
                    selected.as_deref(),
                    None,
                )
                .unwrap();
            receipts.push(receipt);
            previous = parent;
            task_ids.push(id);
        }
        fixture.id = task_ids.last().unwrap().clone();
        fixture.entry =
            workflow_completed_local_entry_if_present(&fixture.repo, &fixture.id, Some("origin"))
                .unwrap()
                .unwrap();
        let before = content.snapshots().list_line_snapshots().unwrap();
        let prepared = prepare_source_history_at_base(
            &fixture.repo,
            &fixture.remote,
            &fixture.entry,
            &fixture.base,
            None,
            None,
        );
        if task_count == 65 {
            assert!(prepared.err().unwrap().contains("1..64 source Tasks"));
            assert_eq!(content.snapshots().list_line_snapshots().unwrap(), before);
            continue;
        }
        let prepared = prepared.unwrap();
        assert_eq!(content.snapshots().list_line_snapshots().unwrap(), before);
        let value = prepared.manifest.to_value().unwrap();
        let entries = value["history_entries"].as_array().unwrap();
        assert_eq!(entries.len(), task_count);
        assert_eq!(entries[1]["changes"].as_array().unwrap().len(), 2);
        assert_eq!(entries[1]["snapshots"].as_array().unwrap().len(), 2);
        let rows = value["content"]["snapshots"].as_array().unwrap();
        let ids = rows
            .iter()
            .map(|row| row["snapshot_id"].as_str().unwrap())
            .collect::<BTreeSet<_>>();
        for (entry, receipt) in entries.iter().zip(&receipts) {
            for field in ["pre_land_target_snapshot_id", "landed_snapshot_id"] {
                assert_eq!(entry[field], receipt[field]);
                assert!(ids.contains(entry[field].as_str().unwrap()));
            }
            let id = entry["local_task_id"].as_str().unwrap();
            assert_eq!(
                TaskStore::get_task(&tasks, id).unwrap()["local_finish"],
                *receipt
            );
        }
        let identity = PackageWorkflowExpectedIdentity {
            repository_index: 0,
            worker_job_index: 0,
            package_hash: prepared.package.package_hash,
            base_snapshot_id: fixture.base.clone(),
            revision_snapshot_id: prepared.manifest.revision_snapshot_id,
        };
        let (_, recovered) =
            recover_source_history(&fixture.repo, &fixture.id, "origin", &identity).unwrap();
        assert_eq!(
            recovered
                .iter()
                .map(|entry| &entry.local_task_id)
                .collect::<Vec<_>>(),
            task_ids.iter().collect::<Vec<_>>()
        );
        for (wire, source) in entries.iter().zip(recovered) {
            assert_eq!(
                wire["snapshots"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| row["snapshot_id"].as_str().unwrap())
                    .collect::<BTreeSet<_>>(),
                source
                    .snapshots
                    .iter()
                    .map(|row| row.snapshot_id.as_str())
                    .collect::<BTreeSet<_>>()
            );
        }
    }
}

#[test]
fn selected_ready_recovery_reads_only_original_job_without_rebase_upload_or_replacement() {
    let mut fixture = fixture();
    let identity = pinned(&fixture);
    let before = fixture
        .repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
            &fixture.repo.workspace_root(),
        )
        .unwrap()
        .list_line_snapshots()
        .unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr());
    configure_remote(&mut fixture.repo, &url);
    let expected = identity.clone();
    let handle = std::thread::spawn(move || {
        for ci in ["queued", "running", "failed"] {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(request.method().as_str(), "GET");
            assert_eq!(
                request.url(),
                "/v1/native/repository-authorities/0/worker-jobs/0"
            );
            request.respond(tiny_http::Response::from_string(json!({"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":null},"package_execution":{"expected_identity":expected,"ci_state":ci,"availability":"available","recoverable":ci != "failed","author_mode":"ai_with_human_review"}}).to_string())).unwrap();
        }
        assert!(server
            .recv_timeout(Duration::from_millis(100))
            .unwrap()
            .is_none());
    });
    let mut waits = 0;
    for code in ["waiting_for_ci", "waiting_for_ci", "ci_failed"] {
        let result = workflow_package_ready(
            &fixture.repo,
            &fixture.id,
            Some("origin"),
            Some(&identity),
            true,
            PackageReadyOptions::default(),
            Some(|payload: &mut JsonValue| {
                waits += 1;
                assert_eq!(payload["expected_identity"], json!(identity));
                Ok(false)
            }),
        )
        .unwrap();
        assert_eq!(result["next_action"]["code"], code);
        assert_eq!(result["expected_identity"], json!(identity));
    }
    assert_eq!(waits, 2);
    assert_eq!(
        fixture
            .repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                &fixture.repo.workspace_root()
            )
            .unwrap()
            .list_line_snapshots()
            .unwrap(),
        before
    );
    handle.join().unwrap();
}

#[test]
fn selected_ready_attestation_keeps_author_mode_requirements_and_original_source_with_newer_head() {
    for (mode, requested, manual_tests) in [
        ("ai_with_human_review", None, None),
        (
            "ai_with_human_review",
            Some("ai_with_human_review"),
            Some("pending"),
        ),
        ("human_only", None, None),
        ("human_with_ai_assist", Some("human_with_ai_assist"), None),
        (
            "ai_only_experimental",
            Some("ai_only_experimental"),
            Some("fail"),
        ),
    ] {
        let mut fixture = fixture();
        fixture
            .repo
            .config
            .insert("default_author_mode".into(), json!(mode));
        let identity = pinned(&fixture);
        // The installed CLI may have completed another Local Task after this package was sealed.
        let tasks = fixture.repo.task_store().unwrap();
        let newer = TaskStore::create_task(
            &tasks,
            "fixture",
            "Later CLI correction",
            "Keep outside the selected package",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let newer_id = required_string_field(&newer, "task_id").unwrap();
        fs::write(
            fixture.temp.path().join("later.txt"),
            "Later source stays local",
        )
        .unwrap();
        let owner = SnapshotWorkflowBinding {
            task_id: newer_id.clone(),
            change_id: String::new(),
            line_name: "feature-later".into(),
        };
        fixture
            .repo
            .line_store()
            .unwrap()
            .create_line(
                "feature-later",
                Some(&fixture.original),
                "2026-10-21T00:00:00Z",
            )
            .unwrap();
        let newer_snapshot = fixture
            .repo
            .local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .unwrap()
            .create_snapshot_content_with_workflow_binding(
                "fixture",
                "feature-later",
                std::slice::from_ref(&fixture.original),
                Some("Later CLI correction"),
                true,
                SnapshotAuthoringOptions::default(),
                Some((&owner, "")),
            )
            .unwrap();
        let newer_snapshot = required_string_field(&newer_snapshot, "snapshot_id").unwrap();
        tasks
            .finish_local_task(
                &newer_id,
                &newer_snapshot,
                "main",
                Some(&fixture.original),
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            local_line_head_snapshot_id(&fixture.repo, "main")
                .unwrap()
                .as_deref(),
            Some(newer_snapshot.as_str())
        );
        let before_tasks = [
            TaskStore::get_task(&tasks, &fixture.id).unwrap(),
            TaskStore::get_task(&tasks, &newer_id).unwrap(),
        ];
        let snapshots = fixture
            .repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                &fixture.repo.workspace_root(),
            )
            .unwrap();
        let before_snapshots = snapshots.list_line_snapshots().unwrap();
        let before_owners = tasks
            .snapshot_ownership_rows(&[
                fixture.original.clone(),
                identity.revision_snapshot_id.clone(),
                newer_snapshot.clone(),
            ])
            .unwrap();
        if requested.is_some() {
            // The flag must follow the requested immutable mode, not the current default.
            fixture.repo.config.insert(
                "default_author_mode".into(),
                json!(if mode == "ai_with_human_review" {
                    "human_only"
                } else {
                    "ai_with_human_review"
                }),
            );
        }
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        configure_remote(
            &mut fixture.repo,
            &format!("http://{}", server.server_addr()),
        );
        let expected = identity.clone();
        let handle = std::thread::spawn(move || {
            let mut attestations = 0;
            while let Some(mut request) = server.recv_timeout(Duration::from_secs(5)).unwrap() {
                let path = request.url().to_string();
                let method = request.method().as_str().to_string();
                let response = match (method.as_str(), path.as_str()) {
                    ("GET", "/v1/native/repository-authorities/0/worker-jobs/0") => {
                        json!({"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":null},"package_execution":{"expected_identity":expected,"ci_state":"passed","availability":"available","recoverable":true,"author_mode":mode}})
                    }
                    ("GET", "/v1/native/repository-authorities/0") => {
                        json!({"repository_index":0,"repository_name":"fixture","namespace":"","policy_flags":0,"tombstoned":false})
                    }
                    ("POST", "/v1/native/repository-authorities") => {
                        json!({"repository_index":0,"actor_key_index":0})
                    }
                    ("POST", "/v1/native/repository-authorities/0/worker-jobs/0") => {
                        let mut body = String::new();
                        request.as_reader().read_to_string(&mut body).unwrap();
                        let body: JsonValue = serde_json::from_str(&body).unwrap();
                        assert_eq!(body["expected_identity"], json!(expected));
                        assert_eq!(body["operation"], "attestation");
                        assert_eq!(body["actor_key_index"], 0);
                        assert_eq!(
                            body["payload"],
                            json!({"author_mode":mode,"verification_state":manual_tests.unwrap_or("pass"),"require_tests_pass":true,"require_human_review":mode=="ai_with_human_review","require_lint_pass":false,"revoked":false})
                        );
                        attestations += 1;
                        json!({"expected_identity":expected,"operation":"attestation","accepted":true,"recorded_at_s":123})
                    }
                    _ => panic!("Selected ready changed the package source: {method} {path}"),
                };
                request
                    .respond(tiny_http::Response::from_string(response.to_string()))
                    .unwrap();
                if attestations == 1 {
                    break;
                }
            }
            assert_eq!(attestations, 1);
            assert!(server
                .recv_timeout(Duration::from_millis(100))
                .unwrap()
                .is_none());
        });
        let result = workflow_package_ready(
            &fixture.repo,
            &fixture.id,
            Some("origin"),
            Some(&identity),
            true,
            PackageReadyOptions {
                author_mode: requested,
                tests: manual_tests,
                ..Default::default()
            },
            Some(|_: &mut JsonValue| -> Result<bool, String> {
                panic!("Passing selected Job must not wait for replacement CI")
            }),
        )
        .unwrap();
        handle.join().unwrap();
        assert_eq!(result["next_action"]["code"], "done", "{result}");
        assert_eq!(result["expected_identity"], json!(identity));
        let (_, source) =
            recover_source_history(&fixture.repo, &fixture.id, "origin", &identity).unwrap();
        assert_eq!(
            source
                .iter()
                .map(|entry| &entry.local_task_id)
                .collect::<Vec<_>>(),
            vec![&fixture.id]
        );
        assert_eq!(
            local_line_head_snapshot_id(&fixture.repo, "main")
                .unwrap()
                .as_deref(),
            Some(newer_snapshot.as_str())
        );
        assert_eq!(snapshots.list_line_snapshots().unwrap(), before_snapshots);
        assert_eq!(
            tasks
                .snapshot_ownership_rows(&[
                    fixture.original.clone(),
                    identity.revision_snapshot_id.clone(),
                    newer_snapshot
                ])
                .unwrap(),
            before_owners
        );
        assert_eq!(
            [
                TaskStore::get_task(&tasks, &fixture.id).unwrap(),
                TaskStore::get_task(&tasks, &newer_id).unwrap()
            ],
            before_tasks
        );
        assert_eq!(
            fs::read_to_string(fixture.temp.path().join("later.txt")).unwrap(),
            "Later source stays local"
        );
    }
}

#[test]
fn fresh_ready_uploads_original_compressed_bytes_once_and_pins_returned_zero_job() {
    let mut fixture = fixture();
    let prepared = prepare_source_history_at_base(
        &fixture.repo,
        &fixture.remote,
        &fixture.entry,
        &fixture.base,
        None,
        None,
    )
    .unwrap();
    let identity = PackageWorkflowExpectedIdentity {
        repository_index: 0,
        worker_job_index: 0,
        package_hash: prepared.package.package_hash.clone(),
        base_snapshot_id: fixture.base.clone(),
        revision_snapshot_id: prepared.manifest.revision_snapshot_id,
    };
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    configure_remote(
        &mut fixture.repo,
        &format!("http://{}", server.server_addr()),
    );
    let expected = identity.clone();
    let target = fixture.base.clone();
    let handle = std::thread::spawn(move || {
        let mut uploads = 0;
        loop {
            let mut request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            let path = request.url().to_string();
            let method = request.method().as_str().to_string();
            let response = if path.ends_with("/lines/main") {
                assert_eq!(method, "GET");
                json!({"line_name":"main","head_snapshot_id":target})
            } else if path == "/v1/native/repository-authorities/0" {
                json!({"repository_index":0,"repository_name":"fixture","namespace":"","policy_flags":0,"tombstoned":false})
            } else if path == "/v1/native/repository-authorities" {
                json!({"repository_index":0,"actor_key_index":0})
            } else if path.ends_with("/remote-sync/zstd-bulk/commit") {
                assert_eq!(method, "POST");
                let header = |name: &str| {
                    request
                        .headers()
                        .iter()
                        .find(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name))
                        .map(|header| header.value.as_str())
                };
                assert_eq!(
                    header("Content-Type"),
                    Some(ait_core::workflow_package::WORKFLOW_PACKAGE_MEDIA_TYPE)
                );
                assert_eq!(
                    header("X-Ait-Package-Sha256"),
                    Some(expected.package_hash.as_str())
                );
                assert_eq!(header("X-Ait-Actor-Key-Index"), Some("0"));
                assert!(header("X-Ait-Worker-Job-Index").is_none());
                let mut bytes = Vec::new();
                request.as_reader().read_to_end(&mut bytes).unwrap();
                assert_eq!(bytes, prepared.package.bytes);
                uploads += 1;
                json!({"expected_identity":expected,"ci_state":"queued","reused":false})
            } else if path.ends_with("/worker-jobs/0") {
                json!({"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":null},"package_execution":{"expected_identity":expected,"ci_state":"queued","availability":"available","recoverable":true}})
            } else {
                panic!("Unexpected fresh preparation request {method} {path}");
            };
            request
                .respond(tiny_http::Response::from_string(response.to_string()))
                .unwrap();
            if path.ends_with("/worker-jobs/0") {
                break;
            }
        }
        assert_eq!(uploads, 1);
    });
    let result = workflow_package_ready(
        &fixture.repo,
        &fixture.id,
        Some("origin"),
        None,
        true,
        PackageReadyOptions::default(),
        Some(|_: &mut JsonValue| Ok(false)),
    )
    .unwrap();
    handle.join().unwrap();
    assert_eq!(result["expected_identity"], json!(identity));
    assert_eq!(result["next_action"]["code"], "waiting_for_ci");
}

#[test]
fn lost_finish_body_preserves_formal_acceptance_with_pending_mappings_and_original_receipt() {
    assert_finish_response_failure_never_replays_or_writes_unproven_mappings(FinishReply::Lost);
}

#[test]
fn timeout_response_preserves_formal_acceptance_without_reposting_or_unproven_writeback() {
    assert_finish_response_failure_never_replays_or_writes_unproven_mappings(FinishReply::Timeout);
}

#[test]
fn repeated_summary_intent_returns_existing_evidence_api_for_a_fresh_lane_decision() {
    assert_finish_response_failure_never_replays_or_writes_unproven_mappings(
        FinishReply::ReviewBlocked,
    );
}

#[test]
fn generic_task_lane_block_does_not_guess_a_general_lane_remediation() {
    assert_finish_response_failure_never_replays_or_writes_unproven_mappings(
        FinishReply::GenericReviewBlocked {
            lane: "task",
            reviewer: "Einstein",
            kind: "bot",
        },
    );
}

#[test]
fn generic_other_bot_and_user_blocks_do_not_impersonate_the_acting_bot() {
    for (lane, reviewer, kind) in [
        ("general", "Other reviewer", "bot"),
        ("general", "Human reviewer", "user"),
        ("task", "Human reviewer", "user"),
    ] {
        assert_finish_response_failure_never_replays_or_writes_unproven_mappings(
            FinishReply::GenericReviewBlocked {
                lane,
                reviewer,
                kind,
            },
        );
    }
}

#[test]
fn direct_exact_finish_response_confirms_and_writes_the_full_source_mapping() {
    assert_finish_response_failure_never_replays_or_writes_unproven_mappings(
        FinishReply::Confirmed,
    );
}

#[derive(Clone, Copy)]
enum FinishReply {
    Lost,
    Timeout,
    ReviewBlocked,
    GenericReviewBlocked {
        lane: &'static str,
        reviewer: &'static str,
        kind: &'static str,
    },
    Confirmed,
}

fn assert_finish_response_failure_never_replays_or_writes_unproven_mappings(reply: FinishReply) {
    let timeout_response = matches!(reply, FinishReply::Timeout);
    let generic_block = match reply {
        FinishReply::GenericReviewBlocked {
            lane,
            reviewer,
            kind,
        } => Some((lane, reviewer, kind)),
        _ => None,
    };
    let review_blocked = matches!(reply, FinishReply::ReviewBlocked) || generic_block.is_some();
    let direct_confirmed = matches!(reply, FinishReply::Confirmed);
    let mut fixture = fixture();
    let identity = pinned(&fixture);
    let before = TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap();
    let before_head = local_line_head_snapshot_id(&fixture.repo, "main").unwrap();
    let snapshots = fixture
        .repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
            &fixture.repo.workspace_root(),
        )
        .unwrap();
    let before_snapshots = snapshots.list_line_snapshots().unwrap();
    let before_content = source_files(&fixture.repo, &fixture.original).unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr());
    configure_remote(&mut fixture.repo, &url);
    let expected = identity.clone();
    let local = fixture.id.clone();
    let handle = std::thread::spawn(move || {
        let mut accepted = false;
        let mut commits = 0;
        let mut formal_reads = 0;
        let mut bot_reviews = 0;
        let mut requests = Vec::new();
        loop {
            let Some(mut request) = server.recv_timeout(Duration::from_secs(5)).unwrap() else {
                break;
            };
            let path = request.url().to_string();
            let method = request.method().as_str().to_string();
            requests.push(format!("{method} {path}"));
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let body: JsonValue = if body.is_empty() {
                JsonValue::Null
            } else {
                serde_json::from_str(&body).unwrap()
            };
            let receipt = json!({"repository_index":0,"task_id":"RT-0001","change_id":null,"change_ref":null,"submission_id":"RT-0001/L-01","patchset_id":"RT-0001/P-01","status":"succeeded","target_line":"main","target_line_index":0,"mode":"direct","base_snapshot_id":expected.base_snapshot_id,"revision_snapshot_id":expected.revision_snapshot_id,"landed_snapshot_id":expected.revision_snapshot_id,"pre_land_target_snapshot_id":expected.base_snapshot_id,"created_at":"2026-10-20T00:00:00Z","updated_at":"2026-10-20T00:00:00Z"});
            let response = if path == "/healthz" {
                assert_eq!(method, "GET");
                json!({"ready":true,"phase":null,"processed":null,"total":null,"last_progress_at":null})
            } else if path.ends_with("/worker-jobs/0") && method == "GET" {
                json!({"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":if accepted {json!(9)} else {JsonValue::Null}},"package_execution":if accepted {JsonValue::Null} else {json!({"expected_identity":expected,"ci_state":"passed","availability":"available","recoverable":true})}})
            } else if path == "/v1/native/repository-authorities/0" {
                assert_eq!(method, "GET");
                json!({"repository_index":0,"repository_name":"fixture","namespace":"","policy_flags":0,"tombstoned":false})
            } else if path == "/v1/native/repository-authorities" {
                assert_eq!(body["repository_index"], 0);
                json!({"repository_index":0,"actor_key_index":0})
            } else if path.ends_with("/worker-jobs/0") && method == "POST" {
                assert_eq!(body["expected_identity"], json!(expected));
                assert_eq!(body["payload"]["reviewer"], "Einstein");
                assert_eq!(body["payload"]["reviewer_kind"], "bot");
                if let Some((_, reviewer, kind)) = generic_block {
                    // These fixed-key retries predate the unresolved decision.
                    // Another principal also cannot resolve that decision.
                    if reviewer != "Einstein" || kind != "bot" {
                        assert_ne!(
                            (
                                body["payload"]["reviewer"].as_str(),
                                body["payload"]["reviewer_kind"].as_str()
                            ),
                            (Some(reviewer), Some(kind))
                        );
                    }
                }
                bot_reviews += 1;
                json!({"expected_identity":expected,"operation":"review","accepted":true,"recorded_at_s":123})
            } else if path.ends_with("/task-land") {
                assert_eq!(method, "POST");
                assert_eq!(body["contract"], "task-land-package-guarded/v1");
                assert_eq!(body["expected_identity"], json!(expected));
                assert_eq!(body.as_object().unwrap().len(), 6);
                assert_eq!(
                    body["idempotency_key"],
                    format!(
                        "workflow-package:{}:{}",
                        expected.package_hash, expected.worker_job_index
                    )
                );
                commits += 1;
                assert_eq!(commits, 1);
                accepted = !review_blocked;
                let response = if direct_confirmed {
                    tiny_http::Response::from_string(json!({"outcome":"confirmed","expected_identity":expected,"patchset_id":"RT-0001/P-01","submission_id":"RT-0001/L-01","entries":[{"local_task_id":local,"remote_task_id":"RT-0001","changes":[]}]}).to_string())
                } else if review_blocked {
                    tiny_http::Response::from_string(if generic_block.is_some() {
                        "PACKAGE_REVIEW_BLOCKED: unresolved review"
                    } else {
                        "PACKAGE_CODE_REVIEW_REQUIRED"
                    })
                    .with_status_code(409)
                } else if timeout_response {
                    tiny_http::Response::from_string("{\"detail\":\"Request timed out\"}")
                        .with_status_code(408)
                } else {
                    tiny_http::Response::from_string("{")
                };
                request.respond(response).unwrap();
                if direct_confirmed {
                    break;
                }
                continue;
            } else if let Some((_, id)) = path.split_once("/snapshots/") {
                let id = id.split('?').next().unwrap();
                json!({"snapshot_id":id,"workflow_owner":{"task_id":"RT-0001","change_ref":null}})
            } else if path.contains("/tasks/") {
                json!({"task_id":"RT-0001","successful_land":receipt})
            } else if path.contains("/lands/") {
                receipt
            } else if path.contains("/patchsets/") {
                formal_reads += 1;
                let manifest = json!({"idempotency_key":format!("workflow-package:{}:{}",expected.package_hash,expected.worker_job_index),"target_line":"main","base_snapshot_id":expected.base_snapshot_id,"revision_snapshot_id":expected.revision_snapshot_id,"entries":[{"local_task_id":local,"task_id":"RT-0001","local_change_ref":null,"change_ref":null}],"aggregate":{"task_id":"RT-0001","patchset_id":"RT-0001/P-01","patchset_index":9}});
                json!({"patchset_id":"RT-0001/P-01","task_id":"RT-0001","base_snapshot_id":expected.base_snapshot_id,"revision_snapshot_id":expected.revision_snapshot_id,"summary":format!("ait-history-promotion/v1 {manifest}")})
            } else {
                panic!("Unexpected request {method} {path}");
            };
            request
                .respond(tiny_http::Response::from_string(response.to_string()))
                .unwrap();
            if accepted && formal_reads == 2 {
                break;
            }
        }
        assert_eq!(commits, 1);
        assert_eq!(bot_reviews, 2);
        requests
    });
    let message="1. Reviewed files\nPackage adapter.\n2. Findings\nRecovery joins checked.\n3. Risks\nServer pairing remains required.\n4. Tests\nIsolated fixture passed.\n5. Recommendation\nPass for this fixture.";
    let result = workflow_package_finish(
        &fixture.repo,
        &fixture.id,
        Some("origin"),
        &identity,
        true,
        Some("Einstein"),
        Some(message),
    )
    .unwrap();
    if !review_blocked && !direct_confirmed {
        let repeated = workflow_package_finish(
            &fixture.repo,
            &fixture.id,
            Some("origin"),
            &identity,
            true,
            None,
            None,
        )
        .unwrap();
        assert_eq!(repeated, result);
    }
    let requests = handle.join().unwrap();
    let finish = requests
        .iter()
        .position(|request| request == "POST /v1/native/repository-authorities/0/task-land")
        .unwrap();
    assert!(requests[finish + 1..]
        .iter()
        .all(|request| request.starts_with("GET ")));
    assert_eq!(
        result["next_action"]["code"],
        if direct_confirmed {
            "done"
        } else if review_blocked {
            "finish_incomplete"
        } else {
            "restore_publication_mappings"
        },
        "{result}; requests={requests:?}"
    );
    if direct_confirmed {
        assert_eq!(result["closeout_status"], "complete");
    } else if !review_blocked {
        assert_eq!(result["apply_status"], "partial");
        assert_eq!(result["closeout_status"], "partial");
        assert_eq!(result["patchset_id"], "RT-0001/P-01");
        assert_eq!(result["submission_id"], "RT-0001/L-01");
        let completion = PackageCompletion::from_value(
            &result["completion"],
            &identity,
            &recover_source_history(&fixture.repo, &fixture.id, "origin", &identity)
                .unwrap()
                .1,
        )
        .unwrap();
        assert!(completion.is_accepted());
        assert!(!completion.has_complete_mappings());
        assert!(completion.entries.is_empty());
    } else {
        assert!(result["closeout_status"].is_null());
    }
    if let Some((lane, reviewer, kind)) = generic_block {
        assert!(result["next_action"]["request"].is_null());
        let templates = result["next_action"]["request_templates"]
            .as_array()
            .unwrap();
        let applicable = templates
            .iter()
            .filter(|template| {
                template["lane"] == lane
                    && template["request"]["body"]["payload"]["reviewer_kind"] == kind
            })
            .collect::<Vec<_>>();
        assert_eq!(applicable.len(), 1, "{lane}/{reviewer}/{kind}: {result}");
        let expected_action = if lane == "task" {
            "task_approve"
        } else {
            "code_review_summary"
        };
        assert_eq!(
            applicable[0]["request"]["body"]["payload"]["action"],
            expected_action
        );
        for template in templates {
            let request = &template["request"];
            assert_eq!(request["method"], "POST");
            assert_eq!(
                request["path"],
                "/v1/native/repository-authorities/0/worker-jobs/0"
            );
            assert_eq!(request["body"]["expected_identity"], json!(identity));
            assert_eq!(request["body"]["operation"], "review");
            assert_eq!(
                request["body"]["payload"]["reviewer"],
                "<blocked-reviewer-name>"
            );
            assert_eq!(
                request["body"]["actor_key_index"],
                "<authorized-request-Actor-key-index>"
            );
            assert_eq!(
                request["body"]["idempotency_key"],
                "<new-caller-stable-key-for-this-decision>"
            );
            assert_eq!(request["body"]["payload"]["blocking"], false);
        }
    } else if review_blocked {
        let request = &result["next_action"]["request"];
        assert_eq!(request["method"], "POST");
        assert_eq!(
            request["path"],
            "/v1/native/repository-authorities/0/worker-jobs/0"
        );
        assert_eq!(request["body"]["operation"], "review");
        assert_eq!(request["body"]["payload"]["action"], "code_review_summary");
        assert_eq!(request["body"]["actor_key_index"], 0);
        assert!(result["next_action"]["review_required_action"]
            .as_str()
            .unwrap()
            .contains("NEW caller-stable key"));
    }
    let after = TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap();
    if direct_confirmed {
        assert_eq!(after["published_task_id"], "RT-0001");
    } else {
        assert!(after["published_task_id"].is_null());
        assert_eq!(after, before);
    }
    assert_eq!(after["local_finish"], before["local_finish"]);
    assert_eq!(
        local_line_head_snapshot_id(&fixture.repo, "main").unwrap(),
        before_head
    );
    assert_eq!(snapshots.list_line_snapshots().unwrap(), before_snapshots);
    assert_eq!(
        source_files(&fixture.repo, &fixture.original).unwrap(),
        before_content
    );
}

#[test]
fn explicit_package_abandonment_retries_same_attempt_and_never_abandons_source_tasks() {
    let mut fixture = fixture();
    let identity = pinned(&fixture);
    let before = TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    configure_remote(
        &mut fixture.repo,
        &format!("http://{}", server.server_addr()),
    );
    let expected = identity.clone();
    let handle = std::thread::spawn(move || {
        let mut previous = None;
        let mut requests = 0;
        loop {
            let mut request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            let path = request.url().to_string();
            let response = if path == "/v1/native/repository-authorities/0" {
                json!({"repository_index":0,"repository_name":"fixture","namespace":"","policy_flags":0,"tombstoned":false})
            } else if path == "/v1/native/repository-authorities" {
                json!({"repository_index":0,"actor_key_index":0})
            } else if path.ends_with("/worker-jobs/0") {
                assert_eq!(request.method().as_str(), "POST");
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                let body: JsonValue = serde_json::from_str(&body).unwrap();
                assert_eq!(body.as_object().unwrap().len(), 4);
                assert_eq!(body["contract"], "workflow-package-abandon/v1");
                assert_eq!(body["expected_identity"], json!(expected));
                if let Some(previous) = &previous {
                    assert_eq!(&body, previous);
                }
                previous = Some(body);
                requests += 1;
                json!({"expected_identity":expected,"state":if requests==1 {"abandoning"} else {"abandoned"}})
            } else {
                panic!("Unexpected abandonment endpoint {path}");
            };
            let status = if response["state"] == "abandoning" {
                202
            } else {
                200
            };
            request
                .respond(
                    tiny_http::Response::from_string(response.to_string()).with_status_code(status),
                )
                .unwrap();
            if requests == 2 {
                break;
            }
        }
    });
    let first = workflow_package_abandon(&fixture.repo, "origin", &identity).unwrap();
    assert_eq!(first["state"], "abandoning");
    assert_eq!(first["complete"], false);
    let second = workflow_package_abandon(&fixture.repo, "origin", &identity).unwrap();
    assert_eq!(second["state"], "abandoned");
    assert_eq!(second["complete"], true);
    assert_eq!(
        TaskStore::get_task(&fixture.repo.task_store().unwrap(), &fixture.id).unwrap(),
        before
    );
    handle.join().unwrap();
}
