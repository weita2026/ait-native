use super::*;

#[test]
fn configured_remote_alias_normalizes_to_contextual_publication_authority() {
    assert_eq!(
        contextual_publication_remote_name("camera-server").unwrap(),
        "origin"
    );
    let error = contextual_publication_remote_name("  ")
        .expect_err("empty Remote alias must fail before publication");
    assert!(error.contains("selected configured Remote"), "{error}");
}

#[test]
fn remote_change_task_id_accepts_change_remote_trait() {
    let mut remote = FakeChangeRemote {
        detail: None,
        changes: vec![json!({
            "change_id": "RCC-1",
            "change": {
                "task_id": "RCT-1"
            }
        })],
        ..Default::default()
    };
    let change = json!({
        "change_id": "published:RCC-1"
    });

    let task_id =
        remote_change_task_id(&mut remote, "repo", &change, "published:RCC-1", "RCC-1").unwrap();

    assert_eq!(task_id.as_deref(), Some("RCT-1"));

    let mut helper_remote = FakeChangeRemote {
        detail: Some(json!({
            "change": {
                "change_id": "RCC-DETAIL",
                "task_id": "RCT-DETAIL"
            }
        })),
        changes: remote.changes.clone(),
        ..Default::default()
    };
    let detail =
        workspace_remote_change_detail_with_task_remote(&mut helper_remote, "repo", "RCC-DETAIL")
            .expect("read workspace remote change detail");
    assert_eq!(detail["change"]["task_id"], json!("RCT-DETAIL"));

    let rows = workspace_remote_change_rows_with_task_remote(&mut helper_remote, "repo")
        .expect("read workspace remote change rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["change"]["task_id"], json!("RCT-1"));

    let mut missing = FakeChangeRemote::default();
    let err = workspace_remote_change_detail_with_task_remote(&mut missing, "repo", "RCC-MISSING")
        .expect_err("missing remote change detail should fail");
    assert!(err.contains("missing detail"));
}

#[test]
fn change_remote_read_close_helpers_accept_change_remote_trait() {
    let mut remote = FakeChangeRemote {
        changes: vec![
            json!({
                "change_id": "RCC-0087",
                "task_id": "RCT-0087",
                "title": "Trait boundary",
                "status": "draft"
            }),
            json!({
                "change_id": "RCC-0088",
                "task_id": "RCT-0088",
                "title": "Other",
                "status": "review"
            }),
        ],
        ..Default::default()
    };

    let listed = super::change_flow::change_list_with_task_remote(&mut remote, "fixture-ait")
        .expect("list changes");
    assert_eq!(listed.as_array().map(Vec::len), Some(2));

    let shown =
        super::change_flow::change_show_with_task_remote(&mut remote, "RCC-0087", "fixture-ait")
            .expect("show change");
    assert_eq!(shown["title"], json!("Trait boundary"));

    let closed =
        super::change_flow::change_close_with_task_remote(&mut remote, "RCC-0087", "fixture-ait")
            .expect("close change");
    assert_eq!(closed["change_id"], json!("RCC-0087"));
    assert_eq!(closed["status"], json!("archived"));

    let shown_after_close =
        super::change_flow::change_show_with_task_remote(&mut remote, "RCC-0087", "fixture-ait")
            .expect("show closed change");
    assert_eq!(shown_after_close["status"], json!("archived"));
}

#[test]
fn change_create_remote_helpers_accept_change_remote_trait() {
    let mut remote = FakeChangeRemote {
        lines: vec![json!({
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": "SNP-BASE"
        })],
        ..Default::default()
    };

    let (line_row, lineage_payload) =
        super::change_flow::change_create_remote_lineage_with_task_remote(
            &mut remote,
            "fixture-ait",
            "main",
        )
        .expect("remote create lineage");
    assert_eq!(line_row["head_snapshot_id"], json!("SNP-BASE"));
    assert_eq!(lineage_payload["fork_snapshot_id"], json!("SNP-BASE"));
    assert_eq!(lineage_payload["forked_from_line"], json!("main"));

    let created = super::change_flow::change_create_with_task_remote(
        &mut remote,
        "fixture-ait",
        "RCT-0090",
        "Trait create",
        "main",
        None,
        &lineage_payload,
    )
    .expect("create remote change");
    assert_eq!(created["task_id"], json!("RCT-0090"));
    assert_eq!(created["title"], json!("Trait create"));
    assert_eq!(created["base_line"], json!("main"));
    assert_eq!(created["fork_snapshot_id"], json!("SNP-BASE"));

    let listed = super::change_flow::change_list_with_task_remote(&mut remote, "fixture-ait")
        .expect("list changes");
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
}

#[test]
fn normal_remote_change_create_rejects_legacy_global_id() {
    let err = validate_short_remote_change_id(
        &json!({"change_id": "LC-1786", "task_id": "LT-1953"}),
        "LT-1953",
    )
    .expect_err("legacy global change id must be migration-only");
    assert!(err.contains("non-short change id `LC-1786`"));
    assert!(err.contains("`C-01`-style"));
}

#[test]
fn change_create_remote_flow_uses_remote_head_when_local_line_is_ahead() {
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
    fs::write(repo_root.join("src.txt"), "change create base").expect("base fixture");
    let base_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("change create base fixture"),
        false,
    )
    .expect("create base snapshot");
    let base_snapshot_id =
        required_string_field(&base_snapshot, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "change create head").expect("head fixture");
    let head_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("change create head fixture"),
        false,
    )
    .expect("create head snapshot");
    let head_snapshot_id =
        required_string_field(&head_snapshot, "snapshot_id").expect("head snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut remote = FakeChangeRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": base_snapshot_id.clone(),
        })],
        ..Default::default()
    };

    let created = super::change_flow::change_create_remote_flow_with_task_remote(
        &mut remote,
        "fixture-ait",
        "RCT-0091",
        "Trait create flow",
        "main",
        None,
    )
    .expect("remote create flow should use the Remote head independently");

    assert_eq!(created["repo_name"], json!("fixture-ait"));
    assert_eq!(created["task_id"], json!("RCT-0091"));
    assert_eq!(created["title"], json!("Trait create flow"));
    assert_eq!(created["base_line"], json!("main"));
    assert_eq!(created["fork_snapshot_id"], json!(base_snapshot_id));
    assert_eq!(created["forked_from_line"], json!("main"));
    assert_eq!(remote.changes.len(), 1);
    assert_eq!(
        local_line_head_snapshot_id(&repo, "main").expect("local main head"),
        Some(head_snapshot_id),
        "Remote Change creation must not move the local Line",
    );
}

#[test]
fn change_base_line_head_accepts_task_remote_trait() {
    let mut remote = FakeWorkspaceTaskRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": "SNP-BASE"
        })],
        ..Default::default()
    };

    let head = super::change_flow::change_base_line_head_with_task_remote(
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("read change base line head");
    assert_eq!(head, "SNP-BASE");

    let err = super::change_flow::change_base_line_head_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/missing",
    )
    .expect_err("missing base line should fail");
    assert!(err.contains("Unknown line"));
}

#[test]
fn change_base_line_read_accepts_task_remote_trait() {
    let mut remote = FakeWorkspaceTaskRemote {
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": "SNP-BASE"
        })],
        ..Default::default()
    };

    let line = super::change_flow::change_base_line_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "main",
    )
    .expect("read change base line");
    assert_eq!(line["head_snapshot_id"], json!("SNP-BASE"));

    let err = super::change_flow::change_base_line_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/missing",
    )
    .expect_err("missing base line should fail");
    assert!(err.contains("Unknown line"));
}

#[test]
fn change_snapshot_lineage_accepts_snapshot_store_trait() {
    let store = FakeSnapshotChainStore {
        chains: BTreeMap::from([
            (
                "SNP-HEAD".to_string(),
                vec![
                    "SNP-BASE".to_string(),
                    "SNP-MID".to_string(),
                    "SNP-HEAD".to_string(),
                ],
            ),
            ("SNP-BASE".to_string(), vec!["SNP-BASE".to_string()]),
            ("SNP-OTHER".to_string(), vec!["SNP-OTHER".to_string()]),
        ]),
        ..Default::default()
    };

    let chain = super::change_flow::change_snapshot_lineage_with_snapshot_store(
        &store, "LCC-1", "SNP-HEAD", "SNP-BASE",
    )
    .expect("fork should be ancestor of latest snapshot");
    assert_eq!(
        chain,
        vec![
            "SNP-BASE".to_string(),
            "SNP-MID".to_string(),
            "SNP-HEAD".to_string()
        ]
    );

    let missing_latest = super::change_flow::change_snapshot_lineage_with_snapshot_store(
        &store,
        "LCC-1",
        "SNP-MISSING",
        "SNP-BASE",
    )
    .expect_err("missing latest snapshot should fail");
    assert!(missing_latest.contains("Latest recorded change snapshot is not available locally"));

    let missing_fork = super::change_flow::change_snapshot_lineage_with_snapshot_store(
        &store,
        "LCC-1",
        "SNP-HEAD",
        "SNP-MISSING",
    )
    .expect_err("missing fork snapshot should fail");
    assert!(missing_fork.contains("Change fork snapshot is not available locally"));

    let unrelated_fork = super::change_flow::change_snapshot_lineage_with_snapshot_store(
        &store,
        "LCC-1",
        "SNP-HEAD",
        "SNP-OTHER",
    )
    .expect_err("unrelated fork should fail");
    assert!(unrelated_fork.contains("is not an ancestor of latest recorded change snapshot"));
}

#[test]
fn change_snapshot_lineage_accepts_fork_on_alternate_merge_parent() {
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

    let lineage = super::change_flow::change_snapshot_lineage_with_snapshot_store(
        &store,
        "LCC-MERGE",
        "SNP-MERGE",
        "SNP-RIGHT",
    )
    .expect("alternate merge parent is a real ancestor");

    assert_eq!(
        lineage,
        vec!["SNP-ROOT", "SNP-LEFT", "SNP-RIGHT", "SNP-MERGE"]
    );
}

#[test]
fn change_publish_flow_accepts_local_stores_and_change_remote_traits() {
    let task_store = FakeTaskStore::default();
    let change_store = FakeChangeStore::default();
    task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Published task",
        "Exercise change publish flow helper",
        Some("LCT"),
        None,
        None,
        None,
    )
    .expect("create task");
    task_local_mark_published_with_task_store(&task_store, "LCT-1", Some("origin"), Some("RCT-1"))
        .expect("mark task published");
    change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        "LCT-1",
        "Publishable change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create change");
    let local_change =
        change_local_read_with_change_store(&change_store, "LCC-1").expect("read change");
    let local_task = task_local_read_with_task_store(&task_store, "LCT-1").expect("read task");

    let mut rejected_remote = FakeChangeRemote::default();
    let error = change_publish_with_local_stores_and_task_remote(
        &change_store,
        &mut rejected_remote,
        &local_change,
        &local_task,
        "LCC-1",
        "fixture-ait",
        "  ",
        false,
    )
    .expect_err("empty Remote alias must fail before Remote publication");
    assert!(error.contains("selected configured Remote"), "{error}");
    assert!(rejected_remote.changes.is_empty());

    let mut remote = FakeChangeRemote::default();
    let published = change_publish_with_local_stores_and_task_remote(
        &change_store,
        &mut remote,
        &local_change,
        &local_task,
        "LCC-1",
        "fixture-ait",
        "camera-server",
        false,
    )
    .expect("publish change through flow helper");
    assert_eq!(published["publication_state"], json!("published"));
    assert_eq!(published["published_remote_name"], json!("origin"));
    assert_eq!(published["published_change_id"], json!("LCC-1"));
    assert_eq!(remote.changes[0]["task_id"], json!("RCT-1"));

    let unpublished_task_store = FakeTaskStore::default();
    let unpublished_change_store = FakeChangeStore::default();
    task_local_create_with_task_store(
        &unpublished_task_store,
        "fixture-ait",
        "Unpublished task",
        "Verify unpublished task gate",
        Some("LCT"),
        None,
        None,
        None,
    )
    .expect("create unpublished task");
    change_local_create_with_change_store(
        &unpublished_change_store,
        "fixture-ait",
        "LCT-1",
        "Blocked change",
        "main",
        Some("LCC"),
        Some("SNP-BASE"),
    )
    .expect("create blocked change");
    let unpublished_change =
        change_local_read_with_change_store(&unpublished_change_store, "LCC-1")
            .expect("read blocked change");
    let unpublished_task = task_local_read_with_task_store(&unpublished_task_store, "LCT-1")
        .expect("read unpublished task");
    let mut rejected_remote = FakeChangeRemote::default();
    let err = change_publish_with_local_stores_and_task_remote(
        &unpublished_change_store,
        &mut rejected_remote,
        &unpublished_change,
        &unpublished_task,
        "LCC-1",
        "fixture-ait",
        "origin",
        false,
    )
    .expect_err("unpublished local task should block change publish");
    assert!(err.contains("must be published"));
    assert!(rejected_remote.changes.is_empty());
}

#[test]
fn change_publish_remote_helper_accepts_change_remote_trait() {
    let mut remote = FakeChangeRemote::default();
    let local_change = json!({
        "change_id": "LCC-0091",
        "title": "Publish trait boundary",
        "base_line": "main",
        "fork_snapshot_id": "SNP-BASE",
        "forked_from_line": "main"
    });

    let published = super::change_flow::change_publish_with_task_remote(
        &mut remote,
        "fixture-ait",
        "RCT-0091",
        &local_change,
        "LCC-0091",
    )
    .expect("publish remote change");
    assert_eq!(published["change_id"], json!("LCC-0091"));
    assert_eq!(published["task_id"], json!("RCT-0091"));
    assert_eq!(published["title"], json!("Publish trait boundary"));
    assert_eq!(published["fork_snapshot_id"], json!("SNP-BASE"));

    let listed = super::change_flow::change_list_with_task_remote(&mut remote, "fixture-ait")
        .expect("list changes");
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
}

#[test]
fn patchset_publish_helpers_accept_change_line_and_closeout_remote_traits() {
    let mut task_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-PATCHSET",
            "task_id": "RCT-PATCHSET",
            "base_line": "main"
        })],
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": "SNP-BASE"
        })],
        ..Default::default()
    };

    let context = patchset_publish_remote_context_with_task_remote(
        &mut task_remote,
        "RCC-PATCHSET",
        "fixture-ait",
        true,
    )
    .expect("resolve patchset publish remote context");
    assert_eq!(context.resolved_change_id, "RCC-PATCHSET");
    assert_eq!(context.change_task_id.as_deref(), Some("RCT-PATCHSET"));
    assert_eq!(context.base_line, "main");
    assert_eq!(context.base_snapshot_id, "SNP-BASE");

    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();
    let payload = patchset_publish_payload_with_closeout_remote(
        &mut closeout_remote,
        &context.resolved_change_id,
        &context.base_snapshot_id,
        "SNP-REVISION",
        "trait patchset publish",
        "ai_with_human_review",
        "fixture-ait",
        Some("feature/lct-0138".to_string()),
        Some(json!({"uploaded_snapshot_id": "SNP-REVISION"})),
    )
    .expect("publish patchset payload");
    assert_eq!(payload["change_id"], json!("RCC-PATCHSET"));
    assert_eq!(payload["base_snapshot_id"], json!("SNP-BASE"));
    assert_eq!(payload["revision_snapshot_id"], json!("SNP-REVISION"));
    assert_eq!(payload["current_line"], json!("feature/lct-0138"));
    assert_eq!(
        payload["snapshot_sync"]["uploaded_snapshot_id"],
        json!("SNP-REVISION")
    );
    assert_eq!(
        payload["patchset"]["summary"],
        json!("trait patchset publish")
    );
    assert_eq!(closeout_remote.patchsets.len(), 1);

    let mut missing_line_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-MISSING-LINE",
            "task_id": "RCT-MISSING-LINE",
            "base_line": "feature/missing"
        })],
        ..Default::default()
    };
    let err = patchset_publish_remote_context_with_task_remote(
        &mut missing_line_remote,
        "RCC-MISSING-LINE",
        "fixture-ait",
        true,
    )
    .expect_err("missing base line should block patchset publish context");
    assert!(err.contains("Unknown line"));
}

#[test]
fn patchset_publish_full_flow_rejects_unbound_change_line_snapshot_and_closeout_remote_traits() {
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
    fs::write(repo_root.join("src.txt"), "base").expect("base fixture");
    let base_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("patchset publish base"),
        false,
    )
    .expect("create base snapshot");
    let base_snapshot_id =
        required_string_field(&base_snapshot, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "revision").expect("revision fixture");
    let revision_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("patchset publish revision"),
        false,
    )
    .expect("create revision snapshot");
    let _revision_snapshot_id =
        required_string_field(&revision_snapshot, "snapshot_id").expect("revision snapshot id");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut task_remote = FakeChangeRemote {
        tasks: BTreeMap::from([(
            "RCT-0083".into(),
            json!({"task_id":"RCT-0083","status":"active","successful_land":null,"current_patchset_id":null}),
        )]),
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCT-0083/C-01",
            "task_id": "RCT-0083",
            "base_line": "main"
        })],
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": base_snapshot_id
        })],
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
        remote_snapshots: BTreeMap::from([(
            base_snapshot_id.clone(),
            json!({
                "snapshot_id": base_snapshot_id
            }),
        )]),
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();

    let error = patchset_publish_flow_with_task_and_closeout_remotes(
        &repo,
        &mut task_remote,
        &mut closeout_remote,
        "origin",
        "fixture-ait",
        "RCT-0083/C-01",
        "trait full patchset publish",
        "ai_with_human_review",
    )
    .expect_err("unbound Snapshot must fail before publication");
    assert!(
        error.contains("remote") || error.contains("Remote"),
        "{error}"
    );
    assert!(closeout_remote.patchsets.is_empty());
    assert!(task_remote.zstd_plan_requests.is_empty());
}

#[test]
fn workflow_publish_patchset_action_rejects_unbound_change_line_snapshot_and_closeout_remote_traits(
) {
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
    fs::write(repo_root.join("src.txt"), "base").expect("base fixture");
    let base_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("workflow publish base"),
        false,
    )
    .expect("create base snapshot");
    let base_snapshot_id =
        required_string_field(&base_snapshot, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "revision").expect("revision fixture");
    let revision_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("workflow publish revision"),
        false,
    )
    .expect("create revision snapshot");
    let _revision_snapshot_id =
        required_string_field(&revision_snapshot, "snapshot_id").expect("revision snapshot id");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut task_remote = FakeChangeRemote {
        tasks: BTreeMap::from([(
            "RCT-0084".into(),
            json!({"task_id":"RCT-0084","status":"active","successful_land":null,"current_patchset_id":null}),
        )]),
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCT-0084/C-01",
            "task_id": "RCT-0084",
            "base_line": "main"
        })],
        lines: vec![json!({
            "repo_name": "fixture-ait",
            "line_name": "main",
            "status": "active",
            "head_snapshot_id": base_snapshot_id
        })],
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
        remote_snapshots: BTreeMap::from([(
            base_snapshot_id.clone(),
            json!({
                "snapshot_id": base_snapshot_id
            }),
        )]),
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();
    let auto_rebase = json!({
        "status": "rebased",
        "line_name": "feature/lct-0159",
    });

    let error = workflow_publish_patchset_action_with_task_and_closeout_remotes(
        &repo,
        &mut task_remote,
        &mut closeout_remote,
        "origin",
        "fixture-ait",
        "RCT-0084/C-01",
        "trait workflow patchset publish",
        "ai_with_human_review",
        Some(auto_rebase.clone()),
        "ready",
    )
    .expect_err("unbound Snapshot must fail before publication");
    assert!(
        error.contains("remote") || error.contains("Remote"),
        "{error}"
    );
    assert!(closeout_remote.patchsets.is_empty());
    assert!(task_remote.zstd_plan_requests.is_empty());
}

fn current_candidate_port_fixture(
    task: &str,
    change: Option<&str>,
    ordinal: u8,
) -> (FakeChangeRemote, FakeWorkspaceCloseoutRemote) {
    let id = format!("{task}/P-{ordinal:02}");
    let child = change.map(|reference| reference.rsplit('/').next().unwrap());
    let changes = change.map(|reference| json!({"repo_name":"fixture-ait","task_id":task,"change_id":reference,"change_ref":reference})).into_iter().collect();
    let task_remote = FakeChangeRemote {
        tasks: BTreeMap::from([(
            task.into(),
            json!({"task_id":task,"status":"active","successful_land":null,"current_patchset_id":id}),
        )]),
        changes,
        ..Default::default()
    };
    let closeout = FakeWorkspaceCloseoutRemote {
        patchsets: BTreeMap::from([(
            id.clone(),
            json!({"patchset_id":id,"task_id":task,"change_id":child,"change_ref":change,"base_snapshot_id":"B","revision_snapshot_id":"R","author_mode":"ai_with_human_review"}),
        )]),
        ..Default::default()
    };
    (task_remote, closeout)
}

fn remote_revision_fixture() -> (FakeChangeRemote, FakeWorkspaceCloseoutRemote) {
    let (mut tasks, mut closeout) = current_candidate_port_fixture("RT-0108", None, 1);
    tasks
        .lines
        .push(json!({"repo_name":"fixture-ait","line_name":"main","head_snapshot_id":"S0"}));
    let candidate = closeout.patchsets.get_mut("RT-0108/P-01").unwrap();
    candidate["base_snapshot_id"] = json!("S0");
    candidate["revision_snapshot_id"] = json!("S1");
    (tasks, closeout)
}

#[test]
fn remote_revision_creation_forks_current_reviewed_revision_instead_of_main() {
    for assertion in [None, Some("S1")] {
        let (mut tasks, mut closeout) = remote_revision_fixture();
        let created = super::change_flow::change_create_remote_revision_with_remotes(
            &mut tasks,
            &mut closeout,
            "fixture-ait",
            "RT-0108",
            "Actual correction",
            "main",
            assertion,
        )
        .unwrap();
        assert_eq!(created["change_ref"], "RT-0108/C-01");
        assert_eq!(created["fork_snapshot_id"], "S1");
        assert_eq!(tasks.changes.len(), 1);
        assert_eq!(tasks.changes[0]["fork_snapshot_id"], "S1");
        assert_eq!(closeout.patchset_reads, vec!["RT-0108/P-01"]);
    }
}

#[test]
fn remote_revision_creation_rejects_bad_assertions_or_candidate_without_effects() {
    for case in 0..6 {
        let (mut tasks, mut closeout) = remote_revision_fixture();
        let mut assertion = None;
        match case {
            0 => assertion = Some("S0"),
            1 => assertion = Some("FOREIGN"),
            2 => {
                closeout.patchsets.get_mut("RT-0108/P-01").unwrap()["invalidated"] = json!(true);
            }
            3 => {
                closeout.patchsets.get_mut("RT-0108/P-01").unwrap()["task_id"] = json!("RT-0109");
            }
            4 => {
                tasks
                    .tasks
                    .get_mut("RT-0108")
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove("current_patchset_id");
            }
            _ => {
                tasks.tasks.get_mut("RT-0108").unwrap()["current_patchset_id"] =
                    json!("RT-0108/P-02");
            }
        }
        assert!(
            super::change_flow::change_create_remote_revision_with_remotes(
                &mut tasks,
                &mut closeout,
                "fixture-ait",
                "RT-0108",
                "Actual correction",
                "main",
                assertion
            )
            .is_err(),
            "case {case}"
        );
        assert!(tasks.changes.is_empty(), "case {case}");
    }
}

#[test]
fn remote_revision_creation_rechecks_candidate_before_creation_and_keeps_no_candidate_boundary() {
    let (mut tasks, mut closeout) = remote_revision_fixture();
    let original = tasks.tasks["RT-0108"].clone();
    let mut moved = original.clone();
    moved["current_patchset_id"] = json!("RT-0108/P-02");
    tasks.task_read_sequence = VecDeque::from([original, moved]);
    let error = super::change_flow::change_create_remote_revision_with_remotes(
        &mut tasks,
        &mut closeout,
        "fixture-ait",
        "RT-0108",
        "Actual correction",
        "main",
        None,
    )
    .unwrap_err();
    assert!(error.contains("candidate changed"), "{error}");
    assert!(tasks.changes.is_empty());
    let (mut tasks, mut closeout) = remote_revision_fixture();
    tasks.tasks.get_mut("RT-0108").unwrap()["current_patchset_id"] = JsonValue::Null;
    let created = super::change_flow::change_create_remote_revision_with_remotes(
        &mut tasks,
        &mut closeout,
        "fixture-ait",
        "RT-0108",
        "Initial explicit revision",
        "main",
        None,
    )
    .unwrap();
    assert_eq!(created["fork_snapshot_id"], "S0");
    assert!(closeout.patchset_reads.is_empty());
}

#[test]
fn patchset_argument_resolution_accepts_change_and_closeout_remote_traits() {
    let (mut task_remote, mut closeout) = current_candidate_port_fixture("RT-0105", None, 2);
    let resolved = resolve_patchset_argument_with_task_and_closeout_remotes(
        &mut task_remote,
        &mut closeout,
        None,
        Some("RT-0105"),
        Some("fixture-ait"),
    )
    .unwrap();
    assert_eq!(resolved, "RT-0105/P-02");
    closeout.patchsets.insert("RT-0105/P-01".into(), json!({"patchset_id":"RT-0105/P-01","task_id":"RT-0105","change_id":null,"change_ref":null,"base_snapshot_id":"B","revision_snapshot_id":"R"}));
    closeout.patchsets.get_mut("RT-0105/P-02").unwrap()["invalidated"] = json!(true);
    assert!(resolve_patchset_argument_with_task_and_closeout_remotes(
        &mut task_remote,
        &mut closeout,
        None,
        Some("RT-0105"),
        Some("fixture-ait")
    )
    .is_err());
    let err = resolve_patchset_argument_with_task_and_closeout_remotes(
        &mut task_remote,
        &mut closeout,
        None,
        None,
        Some("fixture-ait"),
    )
    .unwrap_err();
    assert!(err.contains("Provide a Patchset ID"));
}

#[test]
fn attestation_put_flow_accepts_change_and_closeout_remote_traits() {
    let (mut change_remote, mut closeout_remote) =
        current_candidate_port_fixture("RT-0101", Some("RT-0101/C-01"), 2);

    let attestation = attestation_put_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        None,
        Some("RT-0101/C-01"),
        Some("pass"),
        None,
        None,
        None,
        "ai_with_human_review",
        Some("gpt-5".to_string()),
        "fixture-ait",
    )
    .expect("put attestation through reusable flow");

    assert_eq!(attestation["patchset_id"], json!("RT-0101/P-02"));
    assert_eq!(attestation["verification_state"], "pass");
    assert!(attestation.get("provenance_summary").is_none());
    assert_eq!(attestation["evaluation_summary"]["tests"], "none");
    let stored = closeout_remote
        .attestations
        .get("RT-0101/P-02")
        .expect("stored attestation");
    assert_eq!(stored["repo_name"], json!("fixture-ait"));

    let err = attestation_put_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        None,
        None,
        Some("pass"),
        None,
        None,
        None,
        "manual",
        None,
        "fixture-ait",
    )
    .expect_err("missing patchset and change should fail");
    assert!(err.contains("Provide a Patchset ID"));
}

#[test]
fn review_record_flow_accepts_change_and_closeout_remote_traits() {
    let (mut change_remote, mut closeout_remote) =
        current_candidate_port_fixture("RT-0102", Some("RT-0102/C-01"), 2);

    let recorded = review_record_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "RT-0102/C-01",
        None,
        "alice@example.com",
        "approve",
        Some("looks good"),
        false,
        "fixture-ait",
    )
    .expect("record review through reusable flow");

    assert_eq!(recorded["change_id"], json!("RT-0102/C-01"));
    assert_eq!(recorded["patchset_id"], json!("RT-0102/P-02"));
    assert_eq!(recorded["reviewer"], json!("alice@example.com"));
    assert_eq!(recorded["action"], json!("approve"));
    assert_eq!(recorded["comment"], json!("looks good"));
    assert_eq!(recorded["blocking"], json!(false));

    let reviews = closeout_remote
        .reviews
        .get("RT-0102/C-01")
        .expect("stored reviews");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0]["patchset_id"], json!("RT-0102/P-02"));

    let err = review_record_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "RT-0102/C-01",
        Some("RCP-MISSING"),
        "alice@example.com",
        "approve",
        None,
        false,
        "fixture-ait",
    )
    .expect_err("unknown direct patchset should fail");
    assert!(err.contains("Unknown patchset"));
}

#[test]
fn review_request_flow_accepts_change_and_closeout_remote_traits() {
    let (mut change_remote, mut closeout_remote) =
        current_candidate_port_fixture("RT-0103", Some("RT-0103/C-01"), 2);
    let reviewer_groups = vec!["core".to_string(), "release".to_string()];

    let requested = review_request_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "RT-0103/C-01",
        None,
        &reviewer_groups,
        Some("please review"),
        "fixture-ait",
    )
    .expect("request review through reusable flow");

    assert_eq!(requested["change_id"], json!("RT-0103/C-01"));
    assert_eq!(requested["patchset_id"], json!("RT-0103/P-02"));
    assert_eq!(requested["reviewer_groups"], json!(["core", "release"]));
    assert_eq!(requested["note"], json!("please review"));

    let requests = closeout_remote
        .review_requests
        .get("RT-0103/C-01")
        .expect("stored review requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["patchset_id"], json!("RT-0103/P-02"));

    let err = review_request_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "RT-0103/C-01",
        Some("RCP-MISSING"),
        &reviewer_groups,
        None,
        "fixture-ait",
    )
    .expect_err("unknown direct patchset should fail");
    assert!(err.contains("Unknown patchset"));
}

#[test]
fn land_submit_flow_accepts_change_and_closeout_remote_traits() {
    let mut change_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-LAND",
            "task_id": "RCT-LAND"
        })],
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkspaceCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-LAND-1".to_string(),
            json!({
                "patchset_id": "RCP-LAND-1",
                "change_id": "RCC-LAND"
            }),
        )]),
        ..Default::default()
    };
    let mut guard_seen = Vec::new();

    let submitted = land_submit_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "fixture-ait",
        "RCC-LAND",
        Some("RCP-LAND-1"),
        "main",
        "merge",
        true,
        |task_id, change_id| {
            guard_seen.push((task_id.map(str::to_string), change_id.to_string()));
            Ok(())
        },
    )
    .expect("submit land through reusable flow");

    assert_eq!(submitted["change_id"], json!("RCC-LAND"));
    assert_eq!(submitted["patchset_id"], json!("RCP-LAND-1"));
    assert_eq!(submitted["target_line"], json!("main"));
    assert_eq!(submitted["mode"], json!("merge"));
    assert_eq!(
        guard_seen,
        vec![(Some("RCT-LAND".to_string()), "RCC-LAND".to_string())]
    );
    assert_eq!(closeout_remote.patchset_reads, vec!["RCP-LAND-1"]);
    assert_eq!(closeout_remote.land_submissions.len(), 1);

    let mut blocked_closeout_remote = FakeWorkspaceCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-LAND-1".to_string(),
            json!({
                "patchset_id": "RCP-LAND-1",
                "change_id": "RCC-LAND"
            }),
        )]),
        ..Default::default()
    };
    let err = land_submit_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut blocked_closeout_remote,
        "fixture-ait",
        "RCC-LAND",
        Some("RCP-LAND-1"),
        "main",
        "merge",
        true,
        |_task_id, _change_id| Err("blocked before closeout".to_string()),
    )
    .expect_err("guard failure should stop before closeout side effects");
    assert!(err.contains("blocked before closeout"));
    assert!(blocked_closeout_remote.patchset_reads.is_empty());
    assert!(blocked_closeout_remote.land_submissions.is_empty());
}

#[test]
fn change_identity_accepts_change_remote_trait() {
    let (mut remote, _) = current_candidate_port_fixture("RT-0104", Some("RT-0104/C-01"), 1);

    let identity = super::change_flow::change_identity_with_task_remote(
        &mut remote,
        "RT-0104/C-01",
        Some("fixture-ait"),
    )
    .expect("resolve change identity");
    assert_eq!(identity.0, "RT-0104/C-01");
    assert_eq!(identity.1.as_deref(), Some("RT-0104/P-01"));

    let err = super::change_flow::change_identity_with_task_remote(
        &mut remote,
        "RCC-MISSING",
        Some("fixture-ait"),
    )
    .expect_err("missing change should fail");
    assert!(err.contains("Unknown change"));
}

#[test]
fn remote_change_helpers_accept_single_capability_ports() {
    let mut change_creator = FakeRemoteChangeCreator;
    let created = super::change_flow::change_create_with_task_remote(
        &mut change_creator,
        "fixture-ait",
        "RCT-1",
        "Remote change",
        "main",
        Some("RCC-1"),
        &json!({
            "fork_snapshot_id": "SNP-1",
            "forked_from_line": "main",
        }),
    )
    .expect("create change through single-capability port");
    assert_eq!(created["change_id"], json!("RCC-1"));
    assert_eq!(created["fork_snapshot_id"], json!("SNP-1"));

    let mut change_lister = FakeRemoteChangeLister;
    let listed =
        super::change_flow::change_list_with_task_remote(&mut change_lister, "fixture-ait")
            .expect("list changes through single-capability port");
    assert_eq!(
        listed.as_array().expect("change list")[0]["change_id"],
        json!("RCC-1")
    );

    let mut change_reader = FakeRemoteChangeReader;
    let read = super::change_flow::change_show_with_task_remote(
        &mut change_reader,
        "RCC-1",
        "fixture-ait",
    )
    .expect("read change through single-capability port");
    assert_eq!(read["selected_patchset_id"], json!("PS-1"));

    let mut change_detail_reader = FakeRemoteChangeDetailReader;
    let detail = super::workflow::workflow_land_change_detail_read_with_task_remote(
        &mut change_detail_reader,
        "fixture-ait",
        "RCC-1",
    );
    assert_eq!(detail["task_id"], json!("RCT-1"));

    let mut change_closer = FakeRemoteChangeCloser;
    let closed = super::change_flow::change_close_with_task_remote(
        &mut change_closer,
        "RCC-1",
        "fixture-ait",
    )
    .expect("close change through single-capability port");
    assert_eq!(closed["status"], json!("archived"));
}

#[test]
fn patchset_helpers_accept_single_capability_ports() {
    let mut publisher = FakePatchsetPublisher;
    let published = super::change_flow::patchset_publish_with_closeout_remote(
        &mut publisher,
        "RCC-1",
        "SNP-BASE",
        "SNP-REV",
        "summary",
        "codex",
        "fixture-ait",
    )
    .expect("publish patchset through single-capability port");
    assert_eq!(published["revision_snapshot_id"], json!("SNP-REV"));

    let mut lister = FakePatchsetLister;
    let listed = super::change_flow::patchset_list_with_closeout_remote(
        &mut lister,
        "RCC-1",
        Some("fixture-ait"),
    )
    .expect("list patchsets through single-capability port");
    assert_eq!(
        listed.as_array().expect("patchset list")[0]["patchset_id"],
        json!("PS-1")
    );

    let mut reader = FakePatchsetReader;
    let read = super::change_flow::patchset_show_with_closeout_remote(
        &mut reader,
        "PS-1",
        Some("fixture-ait"),
        Some("RCC-1"),
    )
    .expect("read patchset through single-capability port");
    assert_eq!(read["change_ref"], json!("RCC-1"));

    let mut selector = FakePatchsetSelector;
    let selected = super::change_flow::patchset_select_with_closeout_remote(
        &mut selector,
        "RCC-1",
        "PS-1",
        "fixture-ait",
    )
    .expect("select patchset through single-capability port");
    assert_eq!(selected["selected"], json!(true));

    let mut ci_runner = FakePatchsetCiRunner;
    let queued = super::change_flow::patchset_run_ci_with_closeout_remote(
        &mut ci_runner,
        "PS-1",
        "workflow_ready_apply",
        Some("foreground"),
        "fixture-ait",
    )
    .expect("run patchset CI through single-capability port");
    assert_eq!(queued["queued"], json!(true));
}

#[test]
fn compact_attestation_manual_states_retry_and_optional_no_ci_preserve_history() {
    use ait_core::attest_json::{RemoteAttestationStatement, RemoteVerificationState};
    let (_, mut remote) = current_candidate_port_fixture("RT-0109", None, 1);
    let id = "RT-0109/P-01";
    for (tests, state, writes) in [
        (None, "unknown", 1),
        (None, "unknown", 1),
        (Some("pass"), "pass", 2),
        (Some("pending"), "pending", 3),
        (Some("fail"), "fail", 4),
    ] {
        let value = crate::primitives::change_flow::attestation_put_payload_with_closeout_remote(
            &mut remote,
            id,
            tests,
            None,
            None,
            None,
            "ai_with_human_review",
            Some("automatic-model".into()),
            "fixture-ait",
        )
        .unwrap();
        assert_eq!(value["verification_state"], state);
        assert_eq!(value["require_tests_pass"], true);
        assert_eq!(value["evaluation_summary"]["tests"], "none");
        assert_eq!(remote.attestation_writes, writes);
    }
    let mut value = remote.attestations[id].clone();
    value["verification_state"] = json!("pass");
    value["require_tests_pass"] = json!(false);
    value["ci_backed"] = json!(false);
    remote.attestations.insert(id.into(), value);
    let patchset = remote.patchsets[id].clone();
    let before = remote.attestation_writes;
    for _ in 0..2 {
        let result = workflow_ready_record_attestation_with_closeout_remote(
            &mut remote,
            &patchset,
            None,
            None,
            None,
            None,
            "ai_with_human_review",
            None,
            "fixture-ait",
        )
        .unwrap();
        assert_eq!(result["result"]["require_tests_pass"], false);
        assert_eq!(result["result"]["evaluation_summary"]["tests"], "none");
        assert_eq!(result["result"]["ci_backed"], false);
    }
    assert_eq!(remote.attestation_writes, before);
    let mut required = RemoteAttestationStatement {
        verification_state: RemoteVerificationState::Pass,
        require_tests_pass: false,
        ..Default::default()
    };
    required.retain_requirements_from(&RemoteAttestationStatement::default());
    assert!(required.require_tests_pass);
}

#[test]
fn compact_attestation_nonpassing_or_incompatible_auto_readback_stops_without_append() {
    let (_, mut remote) = current_candidate_port_fixture("RT-0110", None, 1);
    let id = "RT-0110/P-01";
    let patchset = remote.patchsets[id].clone();
    for state in ["unknown", "pending", "fail", "revoked", "incompatible"] {
        let mut value = json!({"task_id":"RT-0110","change_id":null,"change_ref":null,"patchset_id":id,"author_mode":"ai_with_human_review","verification_state":if state=="revoked" {"pass"}else{state},"require_tests_pass":false,"require_lint_pass":false,"require_human_review":false,"revoked":state=="revoked","ci_backed":false});
        if state == "incompatible" {
            value = json!({"evaluation_summary":{"tests":"pass"}});
        }
        remote.attestations.insert(id.into(), value);
        for _ in 0..2 {
            assert!(workflow_ready_record_attestation_with_closeout_remote(
                &mut remote,
                &patchset,
                None,
                None,
                None,
                None,
                "ai_with_human_review",
                None,
                "fixture-ait"
            )
            .is_err());
        }
        assert_eq!(remote.attestation_writes, 0);
    }
    remote.attestations.clear();
    remote.patchsets.get_mut(id).unwrap()["revision_snapshot_id"] = json!("CHANGED");
    assert!(workflow_ready_record_attestation_with_closeout_remote(
        &mut remote,
        &patchset,
        Some("pass"),
        None,
        None,
        None,
        "ai_with_human_review",
        None,
        "fixture-ait"
    )
    .unwrap_err()
    .contains("candidate changed"));
    assert_eq!(remote.attestation_writes, 0);
}
