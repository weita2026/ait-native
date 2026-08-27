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
fn patchset_publish_full_flow_accepts_change_line_snapshot_and_closeout_remote_traits() {
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
    let revision_snapshot_id =
        required_string_field(&revision_snapshot, "snapshot_id").expect("revision snapshot id");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut task_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-PATCHSET-FLOW",
            "task_id": "RCT-PATCHSET-FLOW",
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

    let payload = patchset_publish_flow_with_task_and_closeout_remotes(
        &repo,
        &mut task_remote,
        &mut closeout_remote,
        "origin",
        "fixture-ait",
        "RCC-PATCHSET-FLOW",
        "trait full patchset publish",
        "ai_with_human_review",
    )
    .expect("publish patchset through trait remotes");

    assert_eq!(payload["change_id"], json!("RCC-PATCHSET-FLOW"));
    assert_eq!(payload["base_snapshot_id"], json!(base_snapshot_id));
    assert_eq!(payload["revision_snapshot_id"], json!(revision_snapshot_id));
    assert_eq!(payload["current_line"], json!("main"));
    assert_eq!(
        payload["patchset"]["summary"],
        json!("trait full patchset publish")
    );
    assert_eq!(payload["snapshot_sync"]["uploaded_snapshots"], json!(1));
    assert_eq!(payload["snapshot_sync"]["line_updated"], json!(false));
    assert_eq!(
        payload["snapshot_sync"]["line_update_skipped_reason"],
        json!("current line is the change base line")
    );
    assert_eq!(
        payload["snapshot_sync"]["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(task_remote.zstd_plan_requests.len(), 1);
    assert_eq!(task_remote.zstd_commit_requests.len(), 1);
    let committed_snapshot_ids = task_remote.zstd_commit_requests[0]["snapshots"]
        .as_array()
        .expect("zstd commit snapshots")
        .iter()
        .filter_map(|snapshot| string_field(snapshot, "snapshot_id"))
        .collect::<Vec<_>>();
    assert!(committed_snapshot_ids.contains(&revision_snapshot_id));
    assert_eq!(closeout_remote.patchsets.len(), 1);
}

#[test]
fn workflow_publish_patchset_action_accepts_change_line_snapshot_and_closeout_remote_traits() {
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
    let revision_snapshot_id =
        required_string_field(&revision_snapshot, "snapshot_id").expect("revision snapshot id");
    set_runtime_repository_index(repo_root, 7);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut task_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-WORKFLOW-PUBLISH",
            "task_id": "RCT-WORKFLOW-PUBLISH",
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

    let payload = workflow_publish_patchset_action_with_task_and_closeout_remotes(
        &repo,
        &mut task_remote,
        &mut closeout_remote,
        "origin",
        "fixture-ait",
        "RCC-WORKFLOW-PUBLISH",
        "trait workflow patchset publish",
        "ai_with_human_review",
        Some(auto_rebase.clone()),
        "ready",
    )
    .expect("publish and select patchset through workflow trait helper");

    assert_eq!(payload["patchset_id"], json!("RCP-RCC-WORKFLOW-PUBLISH-1"));
    assert_eq!(
        payload["result"]["patchset"]["summary"],
        json!("trait workflow patchset publish")
    );
    assert_eq!(
        payload["result"]["revision_snapshot_id"],
        json!(revision_snapshot_id)
    );
    assert_eq!(payload["auto_rebase"], auto_rebase);
    assert_eq!(payload["result"]["auto_rebase"], auto_rebase);
    assert_eq!(payload["selection"]["selected"], json!(true));
    assert_eq!(
        payload["selection"]["patchset_id"],
        json!("RCP-RCC-WORKFLOW-PUBLISH-1")
    );
    assert_eq!(
        payload["result"]["snapshot_sync"]["remote_sync_backend"]["backend"],
        json!("zstd_pack_bulk")
    );
    assert_eq!(task_remote.zstd_plan_requests.len(), 1);
    assert_eq!(task_remote.zstd_commit_requests.len(), 1);
    let committed_snapshot_ids = task_remote.zstd_commit_requests[0]["snapshots"]
        .as_array()
        .expect("zstd commit snapshots")
        .iter()
        .filter_map(|snapshot| string_field(snapshot, "snapshot_id"))
        .collect::<Vec<_>>();
    assert!(committed_snapshot_ids.contains(&revision_snapshot_id));
    assert_eq!(closeout_remote.patchsets.len(), 1);
    assert_eq!(closeout_remote.selected_patchsets.len(), 1);
}

#[test]
fn patchset_argument_resolution_accepts_change_and_closeout_remote_traits() {
    let mut change_remote = FakeChangeRemote {
        changes: vec![
            json!({
                "repo_name": "fixture-ait",
                "change_id": "RCC-SELECTED",
                "selected_patchset_id": "RCP-SELECTED-2"
            }),
            json!({
                "repo_name": "fixture-ait",
                "change_id": "RCC-LATEST"
            }),
        ],
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkspaceCloseoutRemote {
        patchsets: BTreeMap::from([
            (
                "RCP-DIRECT-1".to_string(),
                json!({
                    "patchset_id": "RCP-DIRECT-1",
                    "change_id": "RCC-DIRECT",
                    "patchset_number": 1
                }),
            ),
            (
                "RCP-LATEST-1".to_string(),
                json!({
                    "patchset_id": "RCP-LATEST-1",
                    "change_id": "RCC-LATEST",
                    "patchset_number": 1
                }),
            ),
            (
                "RCP-LATEST-3".to_string(),
                json!({
                    "patchset_id": "RCP-LATEST-3",
                    "change_id": "RCC-LATEST",
                    "patchset_number": 3
                }),
            ),
        ]),
        ..Default::default()
    };

    let direct = resolve_patchset_argument_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        Some("RCP-DIRECT-1"),
        None,
        Some("fixture-ait"),
    )
    .expect("resolve direct patchset id");
    assert_eq!(direct, "RCP-DIRECT-1");

    let selected = resolve_patchset_argument_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        None,
        Some("RCC-SELECTED"),
        Some("fixture-ait"),
    )
    .expect("resolve selected patchset from change identity");
    assert_eq!(selected, "RCP-SELECTED-2");

    let latest = resolve_patchset_argument_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        None,
        Some("RCC-LATEST"),
        Some("fixture-ait"),
    )
    .expect("resolve latest patchset from closeout listing");
    assert_eq!(latest, "RCP-LATEST-3");

    let err = resolve_patchset_argument_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        None,
        None,
        Some("fixture-ait"),
    )
    .expect_err("missing patchset and change should fail");
    assert!(err.contains("Provide PATCHSET_ID or --change"));
}

#[test]
fn attestation_put_flow_accepts_change_and_closeout_remote_traits() {
    let mut change_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-ATTEST",
            "selected_patchset_id": "RCP-ATTEST-2"
        })],
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();

    let attestation = attestation_put_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        None,
        Some("RCC-ATTEST"),
        Some("pass"),
        Some("warn"),
        Some("pass"),
        None,
        "ai_with_human_review",
        Some("gpt-5".to_string()),
        "fixture-ait",
    )
    .expect("put attestation through reusable flow");

    assert_eq!(attestation["patchset_id"], json!("RCP-ATTEST-2"));
    assert_eq!(attestation["evaluation_summary"]["tests"], json!("pass"));
    assert_eq!(attestation["evaluation_summary"]["lint"], json!("warn"));
    assert_eq!(
        attestation["evaluation_summary"]["security_scan"],
        json!("pass")
    );
    assert!(attestation["evaluation_summary"]
        .get("license_scan")
        .is_none());
    assert_eq!(
        attestation["provenance_summary"]["model_name"],
        json!("gpt-5")
    );
    assert_eq!(
        attestation["detail"]["minimum_evidence"]["policy_readable"],
        json!(true)
    );

    let stored = closeout_remote
        .attestations
        .get("RCP-ATTEST-2")
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
    assert!(err.contains("Provide PATCHSET_ID or --change"));
}

#[test]
fn review_record_flow_accepts_change_and_closeout_remote_traits() {
    let mut change_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-REVIEW",
            "selected_patchset_id": "RCP-REVIEW-2"
        })],
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();

    let recorded = review_record_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "RCC-REVIEW",
        None,
        "alice@example.com",
        "approve",
        Some("looks good"),
        false,
        "fixture-ait",
    )
    .expect("record review through reusable flow");

    assert_eq!(recorded["change_id"], json!("RCC-REVIEW"));
    assert_eq!(recorded["patchset_id"], json!("RCP-REVIEW-2"));
    assert_eq!(recorded["reviewer"], json!("alice@example.com"));
    assert_eq!(recorded["action"], json!("approve"));
    assert_eq!(recorded["comment"], json!("looks good"));
    assert_eq!(recorded["blocking"], json!(false));

    let reviews = closeout_remote
        .reviews
        .get("RCC-REVIEW")
        .expect("stored reviews");
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0]["patchset_id"], json!("RCP-REVIEW-2"));

    let err = review_record_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "RCC-REVIEW",
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
    let mut change_remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-REQUEST",
            "selected_patchset_id": "RCP-REQUEST-2"
        })],
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkspaceCloseoutRemote::default();
    let reviewer_groups = vec!["core".to_string(), "release".to_string()];

    let requested = review_request_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "RCC-REQUEST",
        None,
        &reviewer_groups,
        Some("please review"),
        "fixture-ait",
    )
    .expect("request review through reusable flow");

    assert_eq!(requested["change_id"], json!("RCC-REQUEST"));
    assert_eq!(requested["patchset_id"], json!("RCP-REQUEST-2"));
    assert_eq!(requested["reviewer_groups"], json!(["core", "release"]));
    assert_eq!(requested["note"], json!("please review"));

    let requests = closeout_remote
        .review_requests
        .get("RCC-REQUEST")
        .expect("stored review requests");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["patchset_id"], json!("RCP-REQUEST-2"));

    let err = review_request_flow_with_task_and_closeout_remotes(
        &mut change_remote,
        &mut closeout_remote,
        "RCC-REQUEST",
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
    let mut remote = FakeChangeRemote {
        changes: vec![json!({
            "repo_name": "fixture-ait",
            "change_id": "RCC-IDENTITY",
            "selected_patchset_id": "RCP-IDENTITY-1"
        })],
        ..Default::default()
    };

    let identity = super::change_flow::change_identity_with_task_remote(
        &mut remote,
        "RCC-IDENTITY",
        Some("fixture-ait"),
    )
    .expect("resolve change identity");
    assert_eq!(identity.0, "RCC-IDENTITY");
    assert_eq!(identity.1.as_deref(), Some("RCP-IDENTITY-1"));

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
