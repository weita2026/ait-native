use super::*;

struct FakeWorkflowContextTaskStore {
    tasks: Vec<JsonValue>,
}

impl ait_core::task_store::TaskStore for FakeWorkflowContextTaskStore {
    fn list_tasks(&self) -> PlanStoreResult<Vec<JsonValue>> {
        Ok(self.tasks.clone())
    }

    fn list_completed_tasks_with_landed_changes(&self) -> PlanStoreResult<Vec<JsonValue>> {
        Ok(self
            .tasks
            .iter()
            .filter(|task| task["status"].as_str() == Some("completed"))
            .cloned()
            .collect())
    }

    fn get_task(&self, task_id: &str) -> PlanStoreResult<JsonValue> {
        self.tasks
            .iter()
            .find(|task| task["task_id"].as_str() == Some(task_id))
            .cloned()
            .ok_or_else(|| PlanStoreError::NotFound(format!("Unknown task: {task_id}")))
    }

    fn allocate_task_identity(
        &self,
        _repo_name: &str,
        _namespace_prefix: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "workflow context fake does not allocate task identities".to_string(),
        ))
    }

    fn sequence_floor(&self, _repo_name: &str, _family: &str) -> PlanStoreResult<i64> {
        Ok(0)
    }

    fn create_task(
        &self,
        _repo_name: &str,
        _title: &str,
        _intent: &str,
        _namespace_prefix: Option<&str>,
        _plan_id: Option<&str>,
        _origin_plan_revision_id: Option<&str>,
        _plan_item_ref: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "workflow context fake does not create tasks".to_string(),
        ))
    }

    fn create_task_explicit(
        &self,
        _task_id: &str,
        _repo_name: &str,
        _title: &str,
        _intent: &str,
        _task_seq: Option<i64>,
        _identity_source: Option<&str>,
        _planning_state: Option<&str>,
        _plan_id: Option<&str>,
        _origin_plan_revision_id: Option<&str>,
        _plan_item_ref: Option<&str>,
        _plan_linked_at: Option<&str>,
        _status: Option<&str>,
        _publication_state: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "workflow context fake does not create explicit tasks".to_string(),
        ))
    }

    fn close_task(&self, _task_id: &str, _status: &str) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "workflow context fake does not close tasks".to_string(),
        ))
    }

    fn mark_task_published(
        &self,
        _task_id: &str,
        _remote_name: Option<&str>,
        _published_task_id: Option<&str>,
    ) -> PlanStoreResult<JsonValue> {
        Err(PlanStoreError::Invalid(
            "workflow context fake does not publish tasks".to_string(),
        ))
    }
}

#[test]
fn task_remote_read_accepts_task_record_remote_trait() {
    let mut remote = FakeTaskRecordRemote {
        tasks: BTreeMap::from([(
            "RCT-READ".to_string(),
            json!({
                "repo_name": "fixture-ait",
                "task_id": "RCT-READ",
                "title": "Read remote task"
            }),
        )]),
        ..Default::default()
    };

    let remote_port: &mut dyn TaskWorkflowTaskRecordRemote = &mut remote;
    let task = task_remote_read_with_task_remote(remote_port, "fixture-ait", "RCT-READ")
        .expect("read remote task");
    assert_eq!(task["task_id"], json!("RCT-READ"));

    let err = task_remote_read_with_task_remote(remote_port, "fixture-ait", "RCT-MISSING")
        .expect_err("missing remote task should fail");
    assert!(err.contains("Unknown task"));
}

#[test]
fn task_remote_create_list_accepts_task_record_remote_trait() {
    let mut remote = FakeTaskRecordRemote::default();
    let remote_port: &mut dyn TaskWorkflowTaskRecordRemote = &mut remote;

    let created = task_remote_create_with_task_remote(
        remote_port,
        "fixture-ait",
        "Create remote task",
        "Exercise trait helper",
        Some("RCT-CREATE"),
        Some("PL-1"),
        Some("PR-1"),
        Some("item-1"),
    )
    .expect("create remote task");
    assert_eq!(created["task_id"], json!("RCT-CREATE"));
    assert_eq!(created["repo_name"], json!("fixture-ait"));
    assert_eq!(created["plan_id"], json!("PL-1"));

    let tasks =
        task_remote_list_with_task_remote(remote_port, "fixture-ait").expect("list remote tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["task_id"], json!("RCT-CREATE"));
}

#[test]
fn task_remote_helpers_accept_single_capability_ports() {
    let mut reader = FakeRemoteTaskReaderOnly;
    let task = task_remote_read_with_task_remote(&mut reader, "fixture-ait", "RCT-READ").unwrap();
    assert_eq!(task["task_id"], json!("RCT-READ"));

    let land_task =
        task_land_remote_task_read_with_task_remote(&mut reader, "fixture-ait", "RCT-LAND")
            .unwrap();
    assert_eq!(land_task["task_id"], json!("RCT-LAND"));

    let mut lister = FakeRemoteTaskListerOnly;
    let tasks = task_remote_list_with_task_remote(&mut lister, "fixture-ait").unwrap();
    assert_eq!(tasks[0]["task_id"], json!("T-LIST"));

    let mut audit_reader = FakeRemoteTaskAuditReaderOnly;
    let audit = task_remote_audit_read_with_task_remote(
        &mut audit_reader,
        "fixture-ait",
        "RCT-AUDIT",
        "main",
    )
    .unwrap();
    assert_eq!(audit["target_line"], json!("main"));

    let mut creator = FakeRemoteTaskCreatorOnly;
    let created = task_remote_create_with_task_remote(
        &mut creator,
        "fixture-ait",
        "Create remote task",
        "Exercise single trait helper",
        Some("RCT-CREATE"),
        Some("PL-1"),
        Some("PR-1"),
        Some("item-1"),
    )
    .unwrap();
    assert_eq!(created["task_id"], json!("RCT-CREATE"));
}

#[test]
fn task_remote_create_flow_accepts_task_record_and_repository_remote_traits() {
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
    let mut remote = FakeTaskRecordRemote {
        repository: Some(json!({
            "repository": {
                "repository_index": 7,
                "repository_name": "fixture-ait",
                "namespace": "",
                "tombstoned": false
            }
        })),
        ..Default::default()
    };

    let created = task_remote_create_flow_with_task_remote(
        &repo,
        &mut remote,
        "fixture-ait",
        "Create remote task flow",
        "Exercise reusable remote task create flow",
        Some("RCT-FLOW"),
        Some("PL-1"),
        Some("PR-1"),
        Some("item-1"),
    )
    .expect("create remote task through reusable flow");

    assert_eq!(created["repo_name"], json!("fixture-ait"));
    assert_eq!(created["task_id"], json!("RCT-FLOW"));
    assert_eq!(created["title"], json!("Create remote task flow"));
    assert_eq!(created["plan_id"], json!("PL-1"));
    assert!(remote.ensured_repositories.is_empty());
    assert_eq!(
        remote
            .repository
            .as_ref()
            .and_then(|row| row.get("repository"))
            .and_then(|row| row.get("repository_index")),
        Some(&json!(7))
    );
    assert_eq!(
        remote
            .repository
            .as_ref()
            .and_then(|row| row.get("repository"))
            .and_then(|row| row.get("repository_name")),
        Some(&json!("fixture-ait"))
    );
    assert_eq!(remote.task_create_repository_present, vec![true]);

    let second = task_remote_create_flow_with_task_remote(
        &repo,
        &mut remote,
        "fixture-ait",
        "Create remote task flow again",
        "Exercise existing remote repository path",
        Some("RCT-FLOW-2"),
        None,
        None,
        None,
    )
    .expect("create remote task through existing repository");
    assert_eq!(second["task_id"], json!("RCT-FLOW-2"));
    assert!(remote.ensured_repositories.is_empty());
    assert_eq!(remote.task_create_repository_present, vec![true, true]);
}

#[test]
fn task_local_store_helpers_accept_task_store_trait() {
    let store = FakeTaskStore::default();

    let created = task_local_create_with_task_store(
        &store,
        "fixture-ait",
        "Create local task",
        "Exercise local store trait helper",
        Some("LCT"),
        Some("PL-1"),
        Some("PR-1"),
        Some("item-1"),
    )
    .expect("create local task");
    assert_eq!(created["repo_name"], json!("fixture-ait"));
    assert_eq!(created["task_id"], json!("LCT-1"));
    assert_eq!(created["plan_id"], json!("PL-1"));

    let tasks = task_local_list_with_task_store(&store).expect("list local tasks");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["task_id"], json!("LCT-1"));

    let task = task_local_read_with_task_store(&store, "LCT-1").expect("read local task");
    assert_eq!(task["title"], json!("Create local task"));

    let err = task_local_read_with_task_store(&store, "LCT-MISSING")
        .expect_err("missing local task should fail");
    assert!(err.contains("Unknown task"));

    let closed =
        task_local_close_with_task_store(&store, "LCT-1", "completed").expect("close local task");
    assert_eq!(closed["status"], json!("completed"));

    let published =
        task_local_mark_published_with_task_store(&store, "LCT-1", Some("origin"), Some("RCT-1"))
            .expect("mark local task published");
    assert_eq!(published["publication_state"], json!("published"));
    assert_eq!(published["published_remote_name"], json!("origin"));
}

#[test]
fn change_local_store_helpers_accept_change_store_trait() {
    let store = FakeChangeStore::default();

    let created = change_local_create_with_change_store(
        &store,
        "fixture-ait",
        "LCT-1",
        "Create local change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create local change");
    assert_eq!(created["change_id"], json!("LCC-1"));
    assert_eq!(created["fork_snapshot_id"], json!("SNP-BASE"));

    let changes = change_local_list_with_change_store(&store).expect("list local changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["change_id"], json!("LCC-1"));

    let change = change_local_read_with_change_store(&store, "LCC-1").expect("read local change");
    assert_eq!(change["title"], json!("Create local change"));

    let closed = change_local_close_with_change_store(&store, "LCC-1", "archived")
        .expect("close local change");
    assert_eq!(closed["status"], json!("archived"));

    let published = change_local_mark_published_with_change_store(
        &store,
        "LCC-1",
        Some("origin"),
        Some("RCC-1"),
        false,
    )
    .expect("mark local change published");
    assert_eq!(published["publication_state"], json!("published"));
    assert_eq!(published["published_change_id"], json!("RCC-1"));

    let err = change_local_read_with_change_store(&store, "LCC-MISSING")
        .expect_err("missing local change should fail");
    assert!(err.contains("Unknown change"));
}

#[test]
fn workflow_local_store_helpers_accept_task_and_change_store_traits() {
    let task_store = FakeTaskStore::default();
    let change_store = FakeChangeStore::default();
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Workflow local task",
        "Exercise workflow local store helpers",
        Some("LCT"),
        None,
        None,
        None,
    )
    .expect("create local task");
    let change = change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-1",
        "Workflow local change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create local change");
    assert_eq!(task["task_id"], json!("LCT-1"));
    assert_eq!(change["change_id"], json!("LCC-1"));

    let read_task =
        workflow_local_task_read_with_task_store(&task_store, "LCT-1").expect("read local task");
    assert_eq!(read_task["title"], json!("Workflow local task"));

    let read_change = workflow_local_change_read_with_change_store(&change_store, "LCC-1")
        .expect("read local change");
    assert_eq!(read_change["base_line"], json!("main"));

    let changes =
        workflow_local_change_rows_with_change_store(&change_store).expect("list local changes");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["change_id"], json!("LCC-1"));

    let landed = workflow_local_change_land_with_change_store(
        &change_store,
        "LCC-1",
        "main",
        "SNP-LANDED",
        Some("SNP-BASE"),
    )
    .expect("land local change");
    assert_eq!(landed["status"], json!("landed"));
    assert_eq!(landed["landed_snapshot_id"], json!("SNP-LANDED"));

    let closed = workflow_local_task_close_with_task_store(&task_store, "LCT-1", "completed")
        .expect("close local task");
    assert_eq!(closed["status"], json!("completed"));

    let err = workflow_local_change_read_with_change_store(&change_store, "LCC-MISSING")
        .expect_err("missing workflow local change should fail");
    assert!(err.contains("Unknown change"));
}

#[test]
fn task_land_local_change_id_accepts_change_store_trait() {
    let change_store = FakeChangeStore::default();
    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-1",
        "Direct local change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create direct local change");
    assert_eq!(
        task_land_local_change_id_with_change_store(&change_store, "LCC-1")
            .expect("direct local change"),
        Some("LCC-1".to_string())
    );
    assert_eq!(
        task_land_local_change_id_with_change_store(&change_store, "LCT-1")
            .expect("task local change"),
        Some("LCC-1".to_string())
    );
    assert_eq!(
        task_land_local_change_id_with_change_store(&change_store, "LCC-MISSING")
            .expect("missing explicit change"),
        None
    );

    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-2",
        "First task change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create first task change");
    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-2",
        "Second task change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create second task change");
    let err = task_land_local_change_id_with_change_store(&change_store, "LCT-2")
        .expect_err("multiple local changes should fail");
    assert!(err.contains("multiple finishable changes"));
    assert!(err.contains("LCC-2"));
    assert!(err.contains("LCC-3"));

    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-3",
        "Active task change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create active task change");
    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-3",
        "Archived task change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create archived task change");
    change_local_close_with_change_store(&change_store, "LCC-5", "archived")
        .expect("archive task change");
    assert_eq!(
        task_land_local_change_id_with_change_store(&change_store, "LCT-3")
            .expect("archived rows ignored"),
        Some("LCC-4".to_string())
    );
}

#[test]
fn workspace_local_store_helpers_accept_task_and_change_store_traits() {
    let task_store = FakeTaskStore::default();
    let change_store = FakeChangeStore::default();
    task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Workspace local task",
        "Exercise workspace guard helper",
        Some("LCT"),
        None,
        None,
        None,
    )
    .expect("create local task");
    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-1",
        "Workspace local change",
        "main",
        Some("LCC"),
        None,
    )
    .expect("create local change");

    let task = workspace_local_task_read_with_task_store(&task_store, "LCT-1")
        .expect("read workspace local task");
    assert_eq!(task["title"], json!("Workspace local task"));

    let change = workspace_local_change_read_with_change_store(&change_store, "LCC-1")
        .expect("read workspace local change");
    assert_eq!(change["title"], json!("Workspace local change"));

    let err = workspace_local_task_read_with_task_store(&task_store, "LCT-MISSING")
        .expect_err("missing workspace local task should fail");
    assert!(err.contains("Unknown task"));
}

#[test]
fn repository_task_workflow_context_accepts_task_store_trait() {
    let empty = FakeWorkflowContextTaskStore { tasks: vec![] };
    assert!(
        !repository_has_task_workflow_context_with_task_store(&empty)
            .expect("empty workflow context")
    );

    let populated = FakeWorkflowContextTaskStore {
        tasks: vec![json!({"task_id": "LCT-1"})],
    };
    assert!(
        repository_has_task_workflow_context_with_task_store(&populated)
            .expect("populated workflow context")
    );
}

#[test]
fn workspace_identity_alias_helpers_accept_local_store_traits() {
    let task_store = FakeTaskStore::default();
    let change_store = FakeChangeStore::default();
    task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Workspace alias task",
        "Exercise workspace alias helper",
        Some("LCT"),
        None,
        None,
        None,
    )
    .expect("create alias task");
    task_local_mark_published_with_task_store(&task_store, "LCT-1", Some("origin"), Some("RCT-1"))
        .expect("mark alias task published");
    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-1",
        "Workspace alias change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create alias change");
    change_local_mark_published_with_change_store(
        &change_store,
        "LCC-1",
        Some("origin"),
        Some("RCC-1"),
        false,
    )
    .expect("mark alias change published");

    let task_aliases = workspace_task_identity_aliases_with_task_store(&task_store, Some("LCT-1"))
        .expect("task aliases");
    assert!(task_aliases.contains("LCT-1"));
    assert!(task_aliases.contains("RCT-1"));

    let change_aliases =
        workspace_change_identity_aliases_with_change_store(&change_store, Some("LCC-1"))
            .expect("change aliases");
    assert!(change_aliases.contains("LCC-1"));
    assert!(change_aliases.contains("RCC-1"));

    let missing_task_aliases =
        workspace_task_identity_aliases_with_task_store(&task_store, Some("LCT-MISSING"))
            .expect("missing task aliases");
    assert_eq!(
        missing_task_aliases,
        BTreeSet::from(["LCT-MISSING".to_string()])
    );
    let empty_change_aliases =
        workspace_change_identity_aliases_with_change_store(&change_store, None)
            .expect("empty change aliases");
    assert!(empty_change_aliases.is_empty());
}

#[test]
fn remote_task_create_response_rejects_binary_allocator_collision() {
    let err = validate_remote_task_create_response(
        &json!({
            "task_id": "RT-2698",
            "repo_name": "ait",
            "title": "Remove ait_server native planning compatibility",
            "intent": "Historical unrelated task",
            "plan_id": "PR-OLD",
            "origin_plan_revision_id": "plan-revision:old",
            "plan_item_ref": "old/ref",
        }),
        "ait",
        "Native Sprint authoring and legacy storage retirement",
        "Retire production fallback storage without local workflow sidecars",
        Some("PR-1597"),
        Some("plan-revision:6488"),
        Some("sprint-new-page-native-test-output/repair"),
    )
    .expect_err("unrelated allocated task row must fail closed");

    assert!(err.contains("RT-2698"));
    assert!(err.contains("existing or unrelated task"));
    assert!(err.contains("repair the server Binary task-id allocator"));
}

#[test]
fn task_audit_local_change_rows_accept_change_store_trait() {
    let store = FakeChangeStore::default();
    change_local_create_with_change_store(
        &store,
        "fixture-ait",
        "LCT-1",
        "Task audit change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create first local change");
    change_local_create_with_change_store(
        &store,
        "fixture-ait",
        "LCT-2",
        "Other task change",
        "main",
        Some("LCC"),
        None,
    )
    .expect("create second local change");

    let rows = task_audit_local_change_rows_with_change_store(&store)
        .expect("list task audit local changes");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["change_id"], json!("LCC-1"));
    assert_eq!(rows[1]["task_id"], json!("LCT-2"));
}

#[test]
fn task_audit_local_lines_accept_line_store_trait() {
    let store = FakeLocalLineStore::default();
    store.lines.borrow_mut().insert(
        "main".to_string(),
        LineRecord {
            line_id: "LNE-FAKE-MAIN".to_string(),
            line_name: "main".to_string(),
            status: "active".to_string(),
            archived_at: None,
            created_at: Some("2026-06-20T00:00:00Z".to_string()),
            updated_at: Some("2026-06-20T00:00:01Z".to_string()),
            head_snapshot_id: Some("SNP-MAIN".to_string()),
        },
    );

    let rows = local_task_audit_lines_with_line_store(&store).expect("list task audit lines");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["line_name"], json!("main"));
    assert_eq!(rows[0]["status"], json!("active"));
    assert_eq!(rows[0]["head_snapshot_id"], json!("SNP-MAIN"));
    assert_eq!(rows[0]["created_at"], json!("2026-06-20T00:00:00Z"));
    assert_eq!(rows[0]["updated_at"], json!("2026-06-20T00:00:01Z"));
}

#[test]
fn task_audit_target_info_accepts_line_and_snapshot_store_traits() {
    let line_store = FakeLocalLineStore::default();
    line_store.lines.borrow_mut().extend([
        (
            "main".to_string(),
            LineRecord {
                line_id: "LNE-FAKE-MAIN".to_string(),
                line_name: "main".to_string(),
                status: "active".to_string(),
                archived_at: None,
                created_at: Some("2026-06-20T00:00:00Z".to_string()),
                updated_at: Some("2026-06-20T00:00:01Z".to_string()),
                head_snapshot_id: Some("SNP-HEAD".to_string()),
            },
        ),
        (
            "empty".to_string(),
            LineRecord {
                line_id: "LNE-FAKE-EMPTY".to_string(),
                line_name: "empty".to_string(),
                status: "active".to_string(),
                archived_at: None,
                created_at: Some("2026-06-20T00:00:02Z".to_string()),
                updated_at: Some("2026-06-20T00:00:03Z".to_string()),
                head_snapshot_id: None,
            },
        ),
    ]);
    let snapshot_store = FakeSnapshotChainStore {
        chains: BTreeMap::from([(
            "SNP-HEAD".to_string(),
            vec![
                "SNP-BASE".to_string(),
                "SNP-MID".to_string(),
                "SNP-HEAD".to_string(),
            ],
        )]),
        ..Default::default()
    };

    let target = local_task_audit_target_info_with_stores(&line_store, &snapshot_store, "main")
        .expect("target info");
    assert_eq!(target["line_name"], json!("main"));
    assert_eq!(target["head_snapshot_id"], json!("SNP-HEAD"));
    assert_eq!(target["ancestor_snapshot_count"], json!(3));
    assert_eq!(target["source"], json!("local"));
    assert_eq!(
        target["ancestry"],
        json!(["SNP-BASE", "SNP-MID", "SNP-HEAD"])
    );

    let empty = local_task_audit_target_info_with_stores(&line_store, &snapshot_store, "empty")
        .expect("empty target info");
    assert_eq!(empty["head_snapshot_id"], JsonValue::Null);
    assert_eq!(empty["ancestor_snapshot_count"], json!(0));
    assert_eq!(empty["ancestry"], json!([]));

    let err =
        local_task_audit_target_info_with_stores(&line_store, &snapshot_store, "feature/missing")
            .expect_err("missing line should fail");
    assert!(err.contains("Unknown line: feature/missing"));
}

#[test]
fn task_audit_local_projection_uses_only_local_authority() {
    let tmp = tempdir().expect("task audit tempdir");
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
    fs::write(repo_root.join("task-audit.txt"), "task audit fixture").expect("fixture file");
    create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("task audit fixture"),
        false,
    )
    .expect("create local target snapshot");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let task_store = FakeTaskStore::default();
    let change_store = FakeChangeStore::default();
    task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Local draft audit task",
        "Exercise local draft audit",
        Some("LCT"),
        None,
        None,
        None,
    )
    .expect("create local draft audit task");
    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-1",
        "Local draft audit change",
        "main",
        Some("LCC"),
        None,
    )
    .expect("create local draft audit change");

    let task = task_local_read_with_task_store(&task_store, "LCT-1").expect("local task");
    let target = local_task_audit_target_info(&repo, "main").expect("local main target");
    let local_draft_audit = infer_local_task_audit_with_change_store(
        &repo,
        &change_store,
        &task,
        "LCT-1",
        "main",
        &target,
    )
    .expect("local draft audit");
    assert_eq!(local_draft_audit["audit_source"]["mode"], json!("local"));
    assert_eq!(
        local_draft_audit["audit_source"]["remote_task_missing"],
        json!(false)
    );
    assert_eq!(local_draft_audit["workflow"]["state"], json!("in_progress"));
    assert_eq!(
        local_draft_audit["changes"][0]["change"]["title"],
        json!("Local draft audit change")
    );
    assert_eq!(
        local_draft_audit["task_land_closeout"]["plan_closeout_policy"],
        json!("automatic_exact_local_when_final_task_completed")
    );
}

#[test]
fn completed_local_closeout_evidence_fails_safe_for_each_incomplete_phase() {
    let target = json!({"ancestry": ["SNP-BASE", "SNP-LANDED"]});
    let change_rows = vec![json!({
        "change": {
            "task_id": "LCT-1",
            "change_id": "C-01",
            "change_ref": "LCT-1/C-01",
            "status": "landed",
            "landed_snapshot_id": "SNP-LANDED",
        },
        "candidate_lines": [{
            "line_name": "feature/lct-1",
            "status": "archived",
        }],
    })];
    let done_plan = json!({"status": "done", "scope": "local"});
    let absent_worktree = json!({"status": "absent"});

    let complete = local_task_closeout_evidence_from_parts(
        done_plan.clone(),
        absent_worktree.clone(),
        &change_rows,
        &target,
    );
    assert_eq!(complete["status"], "done");
    assert_eq!(complete["feature_lines"]["active_count"], 0);
    assert_eq!(complete["changes"]["incomplete_count"], 0);

    let open_plan = local_task_closeout_evidence_from_parts(
        json!({"status": "pending", "scope": "local"}),
        absent_worktree.clone(),
        &change_rows,
        &target,
    );
    assert_eq!(open_plan["status"], "incomplete");

    let retained_worktree = local_task_closeout_evidence_from_parts(
        done_plan.clone(),
        json!({"status": "present", "name": "lct-1"}),
        &change_rows,
        &target,
    );
    assert_eq!(retained_worktree["status"], "incomplete");

    let mut active_line_rows = change_rows.clone();
    active_line_rows[0]["candidate_lines"][0]["status"] = json!("active");
    let active_line = local_task_closeout_evidence_from_parts(
        done_plan.clone(),
        absent_worktree.clone(),
        &active_line_rows,
        &target,
    );
    assert_eq!(active_line["status"], "incomplete");
    assert_eq!(active_line["feature_lines"]["active_count"], 1);

    let divergent_target = local_task_closeout_evidence_from_parts(
        done_plan,
        absent_worktree,
        &change_rows,
        &json!({"ancestry": ["SNP-OTHER"]}),
    );
    assert_eq!(divergent_target["status"], "incomplete");
    assert_eq!(divergent_target["changes"]["incomplete_count"], 1);
}

#[test]
fn task_remote_audit_read_accepts_task_record_remote_trait() {
    let mut remote = FakeTaskRecordRemote {
        task_audit: Some(json!({
            "task": {
                "task_id": "RCT-AUDIT",
                "repo_name": "fixture-ait"
            },
            "target_line": "main",
            "target_line_head": "SNP-MAIN",
            "summary": {
                "verdict": "in_progress",
                "open_changes": 1
            },
            "verdict": {
                "status": "in_progress"
            },
            "changes": []
        })),
        ..Default::default()
    };

    let remote_port: &mut dyn TaskWorkflowTaskRecordRemote = &mut remote;
    let audit =
        task_remote_audit_read_with_task_remote(remote_port, "fixture-ait", "RCT-AUDIT", "main")
            .expect("read remote task audit");
    assert_eq!(audit["workflow"]["state"], json!("in_progress"));
    assert_eq!(audit["queue_workflow"], audit["workflow"]);
    assert_eq!(audit["target"]["line_name"], json!("main"));
    assert_eq!(audit["target"]["head_snapshot_id"], json!("SNP-MAIN"));

    let mut missing = FakeTaskRecordRemote::default();
    let missing_port: &mut dyn TaskWorkflowTaskRecordRemote = &mut missing;
    let err =
        task_remote_audit_read_with_task_remote(missing_port, "fixture-ait", "RCT-AUDIT", "main")
            .expect_err("missing audit should fail");
    assert!(err.contains("missing task audit"));
}
