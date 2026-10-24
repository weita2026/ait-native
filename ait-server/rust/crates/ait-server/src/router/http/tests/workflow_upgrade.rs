use super::*;
use crate::operational_binary_runtime::initialize_installed_runtime;
use ait_server_core::foundation::remote_binary_db::{
    BinaryDbCommandScope, BinaryDbWriteTxn, FilesystemServerRemoteBinaryDb, RepoId, RepoName,
    StoreGeneration, StorePath,
};
use ait_server_core::foundation::server_binary_db_schema_registry::SERVER_BINARY_DB_BIN_SCHEMAS;
use ait_server_core::foundation::server_content_binary_db::{
    server_snapshot_hash48_from_id, ServerBinaryDbLineStore, ServerBinaryDbSnapshotStore,
    ServerBinarySnapshotPayload, ServerBinarySnapshotRecord,
};
use ait_server_core::foundation::server_workflow_store::{
    ServerWorkflowAttestationStore, ServerWorkflowChangeStore, ServerWorkflowPatchsetStore,
    ServerWorkflowTaskStore,
};
use ait_server_core::foundation::server_workflow_upgrade::{
    workflow_generation_fingerprint, WorkflowGenerationAdmission,
};
use ait_server_core::foundation::workflow_binary_v0::*;
use ait_server_core::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store;
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;

#[path = "legacy_history.rs"]
mod legacy_history;

#[tokio::test]
async fn production_router_upgrades_metadata_and_reopens_without_manual_admission() {
    let root = TestDirectory::new("production-metadata-upgrade");
    let config =
        ServerBinaryLifecycleConfig::from_server_data_root(root.0.join("server-data")).unwrap();
    initialize_installed_runtime(&config).unwrap();
    let source = config.activation().unwrap().unwrap();
    let pointer = fs::read(&config.activation_pointer).unwrap();
    let shared_root = source.generation_root.join("repositories/1");
    let db = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new("1"),
        RepoName::new("ait-server"),
        StorePath::new(shared_root.clone()),
        StoreGeneration::new(1),
    );
    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "SE")
        .unwrap()
        .with_repository_index(1);
    workflow
        .resolve_user_actor(
            &ait_server_core::foundation::workflow_binary_v0_adapter::UserActorIdentity::new(
                "fixture",
                "fixture@example.test",
            )
            .unwrap(),
        )
        .unwrap();
    let main = ServerBinaryDbLineStore::<_, 1>::new(db.clone())
        .create_line("main", 0, 100)
        .unwrap();
    let mut historical_tasks = Vec::new();
    for title in [
        "historical first association",
        "historical second association",
    ] {
        let task = workflow
            .create_task(
                "ait-server",
                &json!({"actor_key_index":0,"title":title,"intent":"Preserve shared immutable history"}),
            )
            .unwrap();
        workflow.create_change("ait-server", &json!({"actor_key_index":0,"task_id":task["task_id"],"title":"A different first old Change title","base_line":"main"})).unwrap();
        historical_tasks.push(task);
    }
    let shared_id = "SNP-0000000000D7";
    let shared = ServerBinaryDbSnapshotStore::<_, 1>::new(db)
        .append_snapshot(
            shared_id,
            ServerBinarySnapshotRecord {
                actor_index_plus1: 0,
                snapshot_meta: 0,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: server_snapshot_hash48_from_id(shared_id).unwrap(),
                parent_snapshot_index_plus1: 0,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: main + 1,
                manifest_hash: [0; 32],
                file_count: 0,
                total_bytes: 0,
                created_at_s: 123,
            },
            &ServerBinarySnapshotPayload {
                line_name: "main".into(),
                message: None,
            },
        )
        .unwrap();
    for index in 0..4 {
        let directory = source
            .generation_root
            .join("repositories")
            .join(index.to_string());
        for schema in SERVER_BINARY_DB_BIN_SCHEMAS
            .iter()
            .filter(|schema| schema.layout_id == 2)
        {
            let path = directory.join(schema.path);
            let mut bytes = fs::read(&path).unwrap();
            let widths = match schema.path {
                "task.bin" => Some((64, 60)),
                "change.bin" => Some((72, 68)),
                "snapshot.bin" => Some((92, 88)),
                "land.bin" => Some((52, 48)),
                "plan.bin" => Some((52, 48)),
                "plan_revision.bin" => Some((60, 56)),
                _ => None,
            };
            if let Some((width, prefix)) = widths {
                bytes = [
                    bytes[..4].to_vec(),
                    bytes[4..]
                        .chunks_exact(width)
                        .flat_map(|row| row[..prefix].iter().copied())
                        .collect(),
                ]
                .concat();
            }
            bytes[..4].copy_from_slice(&1u32.to_le_bytes());
            fs::write(path, bytes).unwrap();
        }
        fs::write(
            directory.join("snapshot_link_payload.bin"),
            1u32.to_le_bytes(),
        )
        .unwrap();
    }
    let mut links = 1u32.to_le_bytes().to_vec();
    let mut retired = 1u32.to_le_bytes().to_vec();
    let mut owners = 1u32.to_le_bytes().to_vec();
    for task in 0u32..2 {
        let mut row = vec![0; 40];
        row[2..4].copy_from_slice(&10u16.to_le_bytes());
        row[4..12].copy_from_slice(&(retired.len() as u64).to_le_bytes());
        row[12..16].copy_from_slice(&task.to_le_bytes());
        row[20..24].copy_from_slice(&shared.to_le_bytes());
        row[32..40].copy_from_slice(&123u64.to_le_bytes());
        links.extend_from_slice(&row);
        retired.extend_from_slice(&[0; 10]);
        owners.extend_from_slice(
            &WorkflowBinaryV0Codec::encode_task_snapshot_index(V0OrdinalIndexRecord {
                latest_index_plus1: task + 1,
                count: 1,
                next_ordinal: 1,
                reserved0: 0,
            })
            .unwrap(),
        );
    }
    fs::write(shared_root.join("snapshot_link.bin"), links).unwrap();
    fs::write(shared_root.join("snapshot_link_payload.bin"), retired).unwrap();
    fs::write(shared_root.join("task_snapshot_index.bin"), owners).unwrap();
    let old = workflow_generation_fingerprint(&source.generation_root).unwrap();
    let app = build_router(config.clone());
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let current = workflow_generation_fingerprint(&source.generation_root).unwrap();
    assert_ne!(current, old);
    assert_eq!(fs::read(&config.activation_pointer).unwrap(), pointer);
    assert_eq!(fs::read(shared_root.join("change.bin")).unwrap().len(), 4);
    let links = fs::read(shared_root.join("snapshot_link.bin")).unwrap();
    assert_eq!(links.len(), 4 + 2 * 22);
    for (task, raw) in links[4..].chunks_exact(22).enumerate() {
        let link = WorkflowBinaryV0Codec::decode_snapshot_link(raw).unwrap();
        assert_eq!(link.task_index, task as u32);
        assert_eq!(link.content_snapshot_index, shared);
        assert_eq!(link.created_at_s, 123);
        assert_eq!(link.snapshot_ordinal, 0);
        assert_eq!(link.change_index_plus1, 0);
    }
    for index in 0..4 {
        let directory = source
            .generation_root
            .join("repositories")
            .join(index.to_string());
        assert!(!directory.join("snapshot_link_payload.bin").exists());
        for schema in SERVER_BINARY_DB_BIN_SCHEMAS {
            assert_eq!(
                &fs::read(directory.join(schema.path)).unwrap()[..4],
                schema.layout_id.to_le_bytes()
            );
        }
    }
    let real = workflow.create_change("ait-server", &json!({"actor_key_index":0,"task_id":historical_tasks[0]["task_id"],"title":"A real target-layout revision","base_line":"main"})).unwrap();
    assert_eq!(real["change_id"], "C-01");
    let current = workflow_generation_fingerprint(&source.generation_root).unwrap();
    drop(app);
    let reopened = build_router(config.clone());
    assert_eq!(
        workflow_generation_fingerprint(&source.generation_root).unwrap(),
        current
    );
    assert_eq!(config.activation().unwrap().unwrap(), source);
    assert_eq!(
        fs::read(shared_root.join("change.bin")).unwrap().len(),
        4 + 72
    );
    drop(reopened);
}

fn admitted_retained_fixture(
    root: &FsPath,
) -> (ServerBinaryLifecycleConfig, Arc<OperationalBinaryRuntime>) {
    let config =
        ServerBinaryLifecycleConfig::from_server_data_root(root.join("server-data")).unwrap();
    initialize_installed_runtime(&config).unwrap();
    let source = config.activation().unwrap().unwrap();
    let db = FilesystemServerRemoteBinaryDb::serving_authority(
        RepoId::new("1"),
        RepoName::new("ait-server"),
        StorePath::new(source.generation_root.join("repositories/1")),
        StoreGeneration::new(1),
    );
    let lines = ServerBinaryDbLineStore::<_, 1>::new(db.clone());
    let line = lines.create_line("main", 0, 1).unwrap();
    let snapshots = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
    for (id, parent, time) in [("SNP-0000000000A1", 0, 1), ("SNP-0000000000A2", 1, 2)] {
        snapshots
            .append_snapshot(
                id,
                ServerBinarySnapshotRecord {
                    actor_index_plus1: 0,
                    snapshot_meta: 0,
                    history_flags: 0,
                    payload_len: 0,
                    payload_offset: 0,
                    snapshot_hash48: server_snapshot_hash48_from_id(id).unwrap(),
                    parent_snapshot_index_plus1: parent,
                    root_tree_pack_index_plus1: 0,
                    root_entry_ordinal: 0,
                    line_index_plus1: line + 1,
                    manifest_hash: [0; 32],
                    file_count: 0,
                    total_bytes: 0,
                    created_at_s: time,
                },
                &ServerBinarySnapshotPayload {
                    line_name: "main".into(),
                    message: None,
                },
            )
            .unwrap();
    }
    lines.set_line_head("main", 1, 3).unwrap();
    let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "SE").unwrap();
    store
        .resolve_user_actor(
            &ait_server_core::foundation::workflow_binary_v0_adapter::UserActorIdentity::new(
                "fixture",
                "fixture@example.test",
            )
            .unwrap(),
        )
        .unwrap();
    store
        .create_task(
            "ait-server",
            &json!({"actor_key_index":0,"title":"Task primary identity","intent":"Retain the real revision"}),
        )
        .unwrap();
    store
        .create_change(
            "ait-server",
            &json!({"actor_key_index":0,"task_id":"RSET-0001","title":"Deleted initial content","base_line":"main"}),
        )
        .unwrap();
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    let mut initial = WorkflowBinaryV0Codec::decode_change(
        &tx.read_record(WorkflowBinaryV0Codec::change_file(), 0)
            .unwrap(),
    )
    .unwrap();
    let title =
        WorkflowBinaryV0Codec::encode_single_text_payload("Retained correction", "Change title")
            .unwrap();
    let range = tx
        .append_payload(WorkflowBinaryV0Codec::change_payload_file(), &title)
        .unwrap();
    let retained = V0ChangeRecord {
        change_meta: CHANGE_LIFECYCLE_ARCHIVED,
        change_state: CHANGE_STATE_CANCELED,
        change_ordinal: 1,
        previous_change_index_plus1: 1,
        fork_snapshot_index_plus1: 2,
        payload_offset: range.payload_offset,
        payload_len: range.payload_len as u16,
        archived_at_s: initial.updated_at_s,
        ..initial
    };
    initial.change_meta = CHANGE_LIFECYCLE_ARCHIVED | CHANGE_META_SUPERSEDED;
    tx.overwrite_record(
        WorkflowBinaryV0Codec::change_file(),
        0,
        &WorkflowBinaryV0Codec::encode_change(initial).unwrap(),
    )
    .unwrap();
    tx.append_record(
        WorkflowBinaryV0Codec::change_file(),
        &WorkflowBinaryV0Codec::encode_change(retained).unwrap(),
    )
    .unwrap();
    for name in [
        "change_patchset_index.bin",
        "change_land_index.bin",
        "change_snapshot_index.bin",
    ] {
        tx.append_record(
            WorkflowBinaryV0Codec::chain_index_file(name),
            &WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord::default())
                .unwrap(),
        )
        .unwrap();
    }
    tx.overwrite_record(
        WorkflowBinaryV0Codec::chain_index_file("task_change_index.bin"),
        0,
        &WorkflowBinaryV0Codec::encode_ordinal_index(V0OrdinalIndexRecord {
            latest_index_plus1: 2,
            count: 2,
            next_ordinal: 2,
            reserved0: 0,
        })
        .unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(tx);
    store
        .create_task(
            "ait-server",
            &json!({"actor_key_index":0,"title":"No Change","intent":"Exact zero-Change query"}),
        )
        .unwrap();
    store
        .create_task(
            "ait-server",
            &json!({"actor_key_index":0,"title":"Multiple real Changes","intent":"Do not select only open revisions"}),
        )
        .unwrap();
    store
        .create_change(
            "ait-server",
            &json!({"actor_key_index":0,"task_id":"RSET-0003","title":"First real revision","base_line":"main"}),
        )
        .unwrap();
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    let first = WorkflowBinaryV0Codec::decode_change(
        &tx.read_record(WorkflowBinaryV0Codec::change_file(), 2)
            .unwrap(),
    )
    .unwrap();
    let title = WorkflowBinaryV0Codec::encode_single_text_payload(
        "Other terminal revision",
        "Change title",
    )
    .unwrap();
    let range = tx
        .append_payload(WorkflowBinaryV0Codec::change_payload_file(), &title)
        .unwrap();
    let second = V0ChangeRecord {
        change_meta: CHANGE_LIFECYCLE_ARCHIVED,
        change_state: CHANGE_STATE_CANCELED,
        change_ordinal: 1,
        previous_change_index_plus1: 3,
        fork_snapshot_index_plus1: 2,
        payload_offset: range.payload_offset,
        payload_len: range.payload_len as u16,
        archived_at_s: first.updated_at_s,
        ..first
    };
    tx.append_record(
        WorkflowBinaryV0Codec::change_file(),
        &WorkflowBinaryV0Codec::encode_change(second).unwrap(),
    )
    .unwrap();
    for name in [
        "change_patchset_index.bin",
        "change_land_index.bin",
        "change_snapshot_index.bin",
    ] {
        tx.append_record(
            WorkflowBinaryV0Codec::chain_index_file(name),
            &WorkflowBinaryV0Codec::encode_inventory_index(V0InventoryIndexRecord::default())
                .unwrap(),
        )
        .unwrap();
    }
    tx.overwrite_record(
        WorkflowBinaryV0Codec::chain_index_file("task_change_index.bin"),
        2,
        &WorkflowBinaryV0Codec::encode_ordinal_index(V0OrdinalIndexRecord {
            latest_index_plus1: 4,
            count: 2,
            next_ordinal: 2,
            reserved0: 0,
        })
        .unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(tx);
    for index in 0..4 {
        let root = source
            .generation_root
            .join("repositories")
            .join(index.to_string());
        for schema in SERVER_BINARY_DB_BIN_SCHEMAS
            .iter()
            .filter(|schema| schema.layout_id == 2)
        {
            let path = root.join(schema.path);
            let mut bytes = fs::read(&path).unwrap();
            let widths = match schema.path {
                "task.bin" => Some((64, 60)),
                "change.bin" => Some((72, 68)),
                "snapshot.bin" => Some((92, 88)),
                "land.bin" => Some((52, 48)),
                "plan.bin" => Some((52, 48)),
                "plan_revision.bin" => Some((60, 56)),
                _ => None,
            };
            if let Some((width, prefix)) = widths {
                bytes = [
                    bytes[..4].to_vec(),
                    bytes[4..]
                        .chunks_exact(width)
                        .flat_map(|row| row[..prefix].iter().copied())
                        .collect(),
                ]
                .concat();
            }
            bytes[..4].copy_from_slice(&1u32.to_le_bytes());
            fs::write(path, bytes).unwrap();
        }
        fs::write(root.join("snapshot_link_payload.bin"), 1u32.to_le_bytes()).unwrap();
    }
    let fingerprint = workflow_generation_fingerprint(&source.generation_root).unwrap();
    let admission = WorkflowGenerationAdmission {
        source_fingerprint: fingerprint.clone(),
        initial_changes_by_repository: (0..4)
            .map(|index| {
                (
                    index,
                    if index == 1 {
                        BTreeSet::from([0])
                    } else {
                        BTreeSet::new()
                    },
                )
            })
            .collect(),
        incoming_changes_by_repository: (0..4).map(|index| (index, BTreeSet::new())).collect(),
        historical_dispositions_by_repository: (0..4)
            .map(|index| (index, Default::default()))
            .collect(),
    };
    let converted =
        OperationalBinaryRuntime::ensure_from_config_with_disposition(&config, Some(&admission))
            .unwrap();
    assert_eq!(config.activation().unwrap().unwrap(), source);
    assert_ne!(
        workflow_generation_fingerprint(&source.generation_root).unwrap(),
        fingerprint
    );
    drop(converted);
    let reopened = Arc::new(OperationalBinaryRuntime::ensure_from_config(&config).unwrap());
    (config, reopened)
}

async fn exact_retained_change(router: &Router) -> JsonValue {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/1/changes/RSET-0001%2FC-01")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

fn finished_local_task(
    cli: &super::snapshot_capture::CaptureCli,
    root: &FsPath,
    name: &str,
    namespace: &str,
    url: &str,
) -> (PathBuf, String) {
    let client = root.join(name);
    fs::create_dir(&client).unwrap();
    cli.json(&client, &["init", "--json"]);
    cli.json(
        &client,
        &[
            "config",
            "set",
            "--sprint",
            "off",
            "--agent-teams",
            "disabled",
            "--id-namespace-prefix",
            namespace,
            "--user-name",
            "query-test",
            "--json",
        ],
    );
    cli.json(
        &client,
        &["remote", "add", "origin", url, "--default", "--json"],
    );
    let edit = root.join(format!("{name}-edit"));
    let started = cli.json(
        &client,
        &[
            "task",
            "start",
            "--title",
            "Published local Task",
            "--intent",
            "Inspect through published Task identity",
            "--local",
            "--edit-root",
            edit.to_str().unwrap(),
            "--json",
            "--full",
        ],
    );
    let task = started["task_id"].as_str().unwrap().to_string();
    fs::write(edit.join("revision.txt"), "local accepted revision\n").unwrap();
    cli.json(
        &edit,
        &[
            "task",
            "finish",
            &task,
            "--message",
            "Local receipt only",
            "--local",
            "--json",
        ],
    );
    (client, task)
}

fn assert_ci_blocked(output: &std::process::Output) {
    let text = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_ascii_lowercase();
    assert!(text.contains("ci") && !text.contains("exhausted"), "{text}");
    if output.status.success() {
        let result: JsonValue = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            matches!(
                result["next_action"]["code"].as_str(),
                Some("run_patchset_ci" | "waiting_for_ci")
            ),
            "{result}"
        );
    }
}

fn attestation_count(root: &FsPath, index: u32) -> usize {
    (fs::metadata(
        root.join("repositories")
            .join(index.to_string())
            .join("attest.bin"),
    )
    .unwrap()
    .len() as usize
        - 4)
        / ATTEST_RECORD_SIZE as usize
}

fn reject_without_remote_effects(
    cli: &super::snapshot_capture::CaptureCli,
    client: &FsPath,
    args: &[&str],
    reason: &str,
    generation: &FsPath,
) {
    let before = workflow_generation_fingerprint(generation).unwrap();
    cli.rejects(client, args, reason);
    assert_eq!(workflow_generation_fingerprint(generation).unwrap(), before);
}

#[tokio::test]
async fn startup_upgrade_get_change_returns_retained_c01_after_reopen_without_query_writes() {
    let directory = TestDirectory::new("retained-c01-http");
    let (config, reopened) = admitted_retained_fixture(directory.path());
    let active = config.activation().unwrap().unwrap();
    let before = workflow_generation_fingerprint(&active.generation_root).unwrap();
    let router = build_router_with_state(state_from_operational(reopened));
    let change = exact_retained_change(&router).await;
    assert_eq!(change["task_id"], "RSET-0001");
    assert_eq!(change["change_ref"], "RSET-0001/C-01");
    assert_eq!(change["title"], "Retained correction");
    assert_eq!(change["status"], "abandoned");
    assert_eq!(change["fork_snapshot_id"], "SNP-0000000000A2");
    let stale = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/1/changes/RSET-0001%2FC-02")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(!stale.status().is_success());
    let bare = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/1/changes/RSET-0001")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bare.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        workflow_generation_fingerprint(&active.generation_root).unwrap(),
        before
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run explicitly with AIT_CAPTURE_TEST_CLI pointing to the exact companion CLI build"]
async fn actual_cli_inspects_converted_terminal_c01_by_exact_remote_task_identity() {
    let directory = TestDirectory::new("retained-c01-paired-cli");
    let fixture_root = directory.path().canonicalize().unwrap();
    let (config, reopened) = admitted_retained_fixture(&fixture_root);
    let operational = reopened.clone();
    let router = build_router_with_state(state_from_operational(reopened));
    let expected = exact_retained_change(&router).await;
    let seed_optional = Arc::new(AtomicBool::new(false));
    let seed_in = seed_optional.clone();
    let incompatible = Arc::new(AtomicBool::new(false));
    let incompatible_in = incompatible.clone();
    let runtime_in = operational.clone();
    let router = router.layer(axum::middleware::from_fn(move |request: Request<Body>, next: axum::middleware::Next<Body>| {
        let seed = seed_in.clone();
        let incompatible = incompatible_in.clone();
        let runtime = runtime_in.clone();
        async move {
            let prepare = request.uri().path().ends_with("/history-promotion:prepare");
            let statement_read = request.method() == "GET" && request.uri().path().ends_with("/attestation");
            let response = next.run(request).await;
            if statement_read && response.status().is_success() && incompatible.load(Ordering::SeqCst) {
                let (mut parts, body) = response.into_parts();
                let mut value: JsonValue = serde_json::from_slice(&to_bytes(body).await.unwrap()).unwrap();
                value.as_object_mut().unwrap().remove("verification_state");
                parts.headers.remove("content-length");
                return axum::response::Response::from_parts(parts, axum::body::boxed(Body::from(serde_json::to_vec(&value).unwrap())));
            }
            if prepare && response.status().is_success() && seed.swap(false, Ordering::SeqCst) {
                let (parts, body) = response.into_parts();
                let bytes = to_bytes(body).await.unwrap();
                let value: JsonValue = serde_json::from_slice(&bytes).unwrap();
                let id = value["aggregate"]["patchset_id"].as_str().unwrap();
                let (_, db) = runtime.repository_db(4).unwrap();
                let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db, "LP").unwrap().with_repository_index(4);
                assert!(store.get_attestation(id).is_err());
                // An explicitly established optional-CI statement is fixture
                // authority. The client cannot invent an opt-out policy.
                store.put_attestation(id, &json!({"verification_state":"pass","require_tests_pass":false,"require_human_review":false,"require_lint_pass":false})).unwrap();
                return axum::response::Response::from_parts(parts, axum::body::boxed(Body::from(bytes)));
            }
            response
        }
    }));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::Server::from_tcp(listener)
            .unwrap()
            .serve(router.into_make_service())
            .await
            .unwrap();
    });
    let cli = super::snapshot_capture::CaptureCli {
        executable: PathBuf::from(
            env::var_os("AIT_CAPTURE_TEST_CLI").expect("set exact companion CLI executable"),
        ),
        native_home: fixture_root.join("native-home"),
    };
    fs::create_dir_all(&cli.native_home).unwrap();
    let client = fixture_root.join("ait-server");
    fs::create_dir(&client).unwrap();
    cli.json(&client, &["init", "--json"]);
    cli.json(
        &client,
        &[
            "config",
            "set",
            "--sprint",
            "off",
            "--agent-teams",
            "disabled",
            "--id-namespace-prefix",
            "SE",
            "--user-name",
            "query-test",
            "--json",
        ],
    );
    cli.json(
        &client,
        &["remote", "add", "origin", &url, "--default", "--json"],
    );
    let active = config.activation().unwrap().unwrap();
    let before = workflow_generation_fingerprint(&active.generation_root).unwrap();
    for reference in ["RSET-0001/C-01", "RSET-0001"] {
        let actual = cli.json(
            &client,
            &["change", "show", reference, "--remote", "origin", "--json"],
        );
        for field in [
            "task_id",
            "change_ref",
            "title",
            "status",
            "fork_snapshot_id",
        ] {
            assert_eq!(actual[field], expected[field], "{reference}: {field}");
        }
    }
    cli.rejects(
        &client,
        &[
            "change",
            "show",
            "RSET-0002",
            "--remote",
            "origin",
            "--json",
        ],
        "has no Change",
    );
    cli.rejects(
        &client,
        &[
            "change",
            "show",
            "RSET-0003",
            "--remote",
            "origin",
            "--json",
        ],
        "has multiple Changes",
    );
    let terminal = cli.json(
        &client,
        &[
            "change",
            "show",
            "RSET-0003/C-02",
            "--remote",
            "origin",
            "--json",
        ],
    );
    assert_eq!(terminal["title"], "Other terminal revision");
    assert_eq!(terminal["status"], "abandoned");
    assert_eq!(
        workflow_generation_fingerprint(&active.generation_root).unwrap(),
        before
    );
    // The converted query fixture has no object packs. Use a fresh complete
    // authority for real local-history promotion after conversion/reopen.
    let (published_client, local_task) =
        finished_local_task(&cli, &fixture_root, "published-client", "LP", &url);
    let local_task = local_task.as_str();
    for (flag, value, reason) in [
        ("--lint", "pass", "--lint"),
        ("--security", "pass", "--security"),
        ("--license", "pass", "--license"),
        ("--model", "explicit-model", "--model"),
        ("--tests", "not_required", "cannot represent"),
    ] {
        reject_without_remote_effects(
            &cli,
            &published_client,
            &[
                "workflow", "ready", local_task, "--remote", "origin", "--apply", "--json", flag,
                value,
            ],
            reason,
            &active.generation_root,
        );
    }
    assert!(cli.json(
        &published_client,
        &["task", "show", local_task, "--local", "--json"]
    )["published_task_id"]
        .is_null());
    seed_optional.store(true, Ordering::SeqCst);
    cli.json(
        &published_client,
        &[
            "workflow", "ready", local_task, "--remote", "origin", "--apply", "--json",
        ],
    );
    assert!(!seed_optional.load(Ordering::SeqCst));
    let local = cli.json(
        &published_client,
        &["task", "show", local_task, "--local", "--json"],
    );
    let remote_task = local["published_task_id"].as_str().unwrap();
    assert_ne!(remote_task, local_task);
    assert_eq!(operational.repository_indexes(), vec![0, 1, 2, 3, 4]);
    let (_, db) = operational.repository_db(4).unwrap();
    let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db, "LP")
        .unwrap()
        .with_repository_index(4);
    let task = store
        .get_task(Some("published-client"), remote_task)
        .unwrap();
    assert!(
        task["successful_land"].is_null(),
        "history import is not remote acceptance"
    );
    let candidate = store
        .get_patchset(
            Some("published-client"),
            task["current_patchset_id"].as_str().unwrap(),
        )
        .unwrap();
    let revision = candidate["revision_snapshot_id"].as_str().unwrap();
    let id = candidate["patchset_id"].as_str().unwrap();
    assert!(candidate["change_ref"].is_null());
    let statement = store.get_attestation(id).unwrap();
    assert_eq!(statement["verification_state"], "pass");
    assert_eq!(statement["ci_backed"], false);
    assert_eq!(statement["evaluation_summary"]["tests"], "none");
    let count = attestation_count(&active.generation_root, 4);
    let original_optional_count = count;
    cli.json(
        &published_client,
        &[
            "workflow", "ready", local_task, "--remote", "origin", "--apply", "--json",
        ],
    );
    assert_eq!(attestation_count(&active.generation_root, 4), count);
    assert_eq!(
        store
            .get_task(Some("published-client"), remote_task)
            .unwrap()["current_patchset_id"],
        id
    );
    cli.json(
        &published_client,
        &[
            "config",
            "set",
            "--default-model",
            "automatic-default",
            "--json",
        ],
    );
    cli.json(
        &published_client,
        &[
            "workflow", "ready", local_task, "--remote", "origin", "--apply", "--json",
        ],
    );
    cli.json(
        &published_client,
        &["config", "unset", "default-model", "--json"],
    );
    cli.json(
        &published_client,
        &[
            "workflow", "ready", local_task, "--remote", "origin", "--apply", "--json",
        ],
    );
    assert_eq!(attestation_count(&active.generation_root, 4), count);
    for (flag, value, reason) in [
        ("--lint", "pass", "--lint"),
        ("--security", "pass", "--security"),
        ("--license", "pass", "--license"),
        ("--model", "explicit-model", "--model"),
        ("--tests", "not_required", "cannot represent"),
    ] {
        reject_without_remote_effects(
            &cli,
            &published_client,
            &[
                "attest", "put", id, "--remote", "origin", "--json", flag, value,
            ],
            reason,
            &active.generation_root,
        );
    }
    cli.json(
        &published_client,
        &["attest", "put", id, "--remote", "origin", "--json"],
    );
    assert_eq!(
        store.get_attestation(id).unwrap()["verification_state"],
        "unknown"
    );
    for verification in ["unknown", "pending", "fail", "pass"] {
        cli.json(
            &published_client,
            &[
                "attest",
                "put",
                id,
                "--tests",
                verification,
                "--remote",
                "origin",
                "--json",
            ],
        );
        let statement = store.get_attestation(id).unwrap();
        assert_eq!(statement["verification_state"], verification);
        assert_eq!(statement["require_tests_pass"], false);
        assert_eq!(statement["ci_backed"], false);
        assert_eq!(statement["evaluation_summary"]["tests"], "none");
        let before = attestation_count(&active.generation_root, 4);
        if verification != "pass" {
            let (output, timed_out) = cli.run_bounded(
                &published_client,
                &[
                    "workflow", "ready", local_task, "--remote", "origin", "--apply", "--json",
                ],
            );
            assert!(
                !timed_out && !output.status.success(),
                "verification {verification} must stop actionably"
            );
        }
        assert_eq!(attestation_count(&active.generation_root, 4), before);
    }
    store
        .put_attestation(
            id,
            &json!({"verification_state":"pass","revoked":true,"require_tests_pass":false}),
        )
        .unwrap();
    let before = attestation_count(&active.generation_root, 4);
    let (revoked, timed_out) = cli.run_bounded(
        &published_client,
        &[
            "workflow", "ready", local_task, "--remote", "origin", "--apply", "--json",
        ],
    );
    assert!(!timed_out && !revoked.status.success());
    assert_eq!(attestation_count(&active.generation_root, 4), before);
    assert_eq!(
        (fs::metadata(active.generation_root.join("repositories/4/worker_job.bin"))
            .unwrap()
            .len()
            - 4)
            / 52,
        0
    );
    incompatible.store(true, Ordering::SeqCst);
    reject_without_remote_effects(
        &cli,
        &published_client,
        &[
            "attest", "put", id, "--tests", "pass", "--remote", "origin", "--json",
        ],
        "Incompatible",
        &active.generation_root,
    );
    incompatible.store(false, Ordering::SeqCst);
    assert_eq!(attestation_count(&active.generation_root, 4), before);
    assert_eq!(
        store
            .get_task(Some("published-client"), remote_task)
            .unwrap()["current_patchset_id"],
        id
    );
    store.create_change("published-client", &json!({"actor_key_index":0,"task_id":remote_task,"title":"Revision found through published Task","base_line":"main","fork_snapshot_id":revision})).unwrap();
    let before = workflow_generation_fingerprint(&active.generation_root).unwrap();
    let actual = cli.json(
        &published_client,
        &["change", "show", local_task, "--remote", "origin", "--json"],
    );
    assert_eq!(actual["task_id"], remote_task);
    assert_eq!(actual["change_ref"], format!("{remote_task}/C-01"));
    assert_eq!(actual["title"], "Revision found through published Task");
    assert_eq!(
        workflow_generation_fingerprint(&active.generation_root).unwrap(),
        before
    );
    let (required_client, required_task) =
        finished_local_task(&cli, &fixture_root, "required-client", "RQ", &url);
    let args = [
        "workflow",
        "ready",
        &required_task,
        "--remote",
        "origin",
        "--apply",
        "--json",
    ];
    let (first, first_waiting) = cli.run_bounded(&required_client, &args);
    if !first_waiting {
        assert_ci_blocked(&first);
    }
    let count = attestation_count(&active.generation_root, 5);
    assert!(count <= 1, "a first statement cannot loop: {count}");
    let (retry, retry_waiting) = cli.run_bounded(&required_client, &args);
    if !retry_waiting {
        assert_ci_blocked(&retry);
    }
    assert_eq!(attestation_count(&active.generation_root, 5), count);
    let local = cli.json(
        &required_client,
        &["task", "show", &required_task, "--local", "--json"],
    );
    let (_, db) = operational.repository_db(5).unwrap();
    let required = BinaryDbServerWorkflowV0Store::new_remote_frozen(db, "RQ")
        .unwrap()
        .with_repository_index(5);
    let task = required
        .get_task(
            Some("required-client"),
            local["published_task_id"].as_str().unwrap(),
        )
        .unwrap();
    assert!(task["successful_land"].is_null());
    let candidate = required
        .get_patchset(
            Some("required-client"),
            task["current_patchset_id"].as_str().unwrap(),
        )
        .unwrap();
    assert_eq!(candidate["ci_completed_at_s"], 0);
    if first_waiting || retry_waiting {
        assert_eq!(
            (fs::metadata(active.generation_root.join("repositories/5/worker_job.bin"))
                .unwrap()
                .len()
                - 4)
                / 52,
            1
        );
    }
    if let Ok(statement) = required.get_attestation(task["current_patchset_id"].as_str().unwrap()) {
        assert_eq!(statement["require_tests_pass"], true);
        assert_ne!(statement["verification_state"], "pass");
        assert_eq!(statement["ci_backed"], false);
    }
    println!("paired retained C01, Task lookup, compact verification and bounded CI passed; original optional statement count={original_optional_count}, fresh statement count={}", attestation_count(&active.generation_root, 5));
    server.abort();
    let _ = server.await;
}
