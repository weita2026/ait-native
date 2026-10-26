use super::*;
use ait_server_core::foundation::remote_binary_db::{BinaryDbReadTxn, ServerRemoteBinaryDb};
use ait_server_core::foundation::server_workflow_store::{
    ServerWorkflowLandStore, ServerWorkflowPolicyStore, ServerWorkflowReviewStore,
};
use std::sync::atomic::AtomicUsize;

fn patchset_count(db: &FilesystemServerRemoteBinaryDb) -> u32 {
    BinaryDbReadTxn::new(db)
        .record_count(WorkflowBinaryV0Codec::patchset_file())
        .unwrap()
}

fn fixture_gates(
    store: &BinaryDbServerWorkflowV0Store<FilesystemServerRemoteBinaryDb>,
    task: &str,
    id: &str,
) {
    store
        .record_review(
            task,
            &json!({"patchset_id":id,"reviewer":"owner","action":"approve"}),
        )
        .unwrap();
    let run = store
        .run_patchset_ci(id, &json!({"trigger":"workflow_ready"}))
        .unwrap();
    store
        .complete_patchset_ci(
            id,
            &json!({"patchset_id":id,"ci_run_seq":run["ci_run_seq"],
        "selected_suite_count":1,"suite_result_count":1,"blocking_failure_count":0,
        "overall_status":"pass","tests_status":"pass","lint_status":"pass"}),
        )
        .unwrap();
    store
        .put_attestation(
            id,
            &json!({"verification_state":"pass","require_tests_pass":true,
        "require_human_review":true,"require_lint_pass":true}),
        )
        .unwrap();
    assert_eq!(store.evaluate_policy(id).unwrap()["decision"], "pass");
}

fn pending_ci(
    output: &std::process::Output,
    timed_out: bool,
    db: &FilesystemServerRemoteBinaryDb,
    store: &BinaryDbServerWorkflowV0Store<FilesystemServerRemoteBinaryDb>,
    task: &str,
    id: &str,
) -> (u64, u64, u64) {
    if !timed_out {
        assert_ci_blocked(output);
    }
    let patchset = store.get_patchset(None, id).unwrap();
    let task = store.get_task(None, task).unwrap();
    assert_eq!(task["current_patchset_id"], patchset["patchset_id"]);
    assert_eq!(task["status"], "active");
    assert!(task["successful_land"].is_null());
    assert_eq!(patchset["ci_run_seq"], 1);
    assert_eq!(patchset["ci_completed_at_s"], 0);
    assert_ne!(patchset["ci"]["overall_status"], "pass");
    if let Ok(statement) = store.get_attestation(id) {
        assert_ne!(statement["verification_state"], "pass");
        assert_eq!(statement["ci_backed"], false);
    }
    assert_eq!(
        BinaryDbReadTxn::new(db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        0
    );
    let root = db.authority_root();
    let statements = (fs::metadata(root.as_path().join("attest.bin"))
        .unwrap()
        .len()
        - 4)
        / u64::from(ATTEST_RECORD_SIZE);
    let jobs = (fs::metadata(root.as_path().join("worker_job.bin"))
        .unwrap()
        .len()
        - 4)
        / u64::from(
            ait_server_core::foundation::operational_binary_v0::SERVER_WORKER_JOB_RECORD_SIZE,
        );
    assert!(jobs > 0);
    (statements, jobs, patchset["ci_run_seq"].as_u64().unwrap())
}

fn quote_source_era_summaries(
    db: &FilesystemServerRemoteBinaryDb,
    store: &BinaryDbServerWorkflowV0Store<FilesystemServerRemoteBinaryDb>,
) -> Vec<u8> {
    let mut tx = BinaryDbWriteTxn::begin_serving(db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    for index in 0..7 {
        let mut p = WorkflowBinaryV0Codec::decode_frozen_patchset(
            &tx.read_record(WorkflowBinaryV0Codec::patchset_file(), index)
                .unwrap(),
        )
        .unwrap();
        let raw = tx
            .read_payload(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                p.summary_offset,
                u32::from(p.summary_len),
            )
            .unwrap();
        let text = String::from_utf8(raw).unwrap();
        let (prefix, body) = text.split_once(' ').unwrap();
        let mut value: JsonValue = serde_json::from_str(body).unwrap();
        if index < 6 {
            value["contract"] = json!("ait-local-land-receipt/v1");
            value["change_ref"] = json!(format!("{}/C-01", value["task_id"].as_str().unwrap()));
            value["local_change_id"] = json!("C-01");
            value["local_change_ref"] =
                json!(format!("{}/C-01", value["local_task_id"].as_str().unwrap()));
        } else {
            value["contract"] = json!("ait-history-promotion/v1");
            value["idempotency_key"] = json!("source-era-operation");
            value["request_sha256"] = json!("0".repeat(64));
            value["aggregate"]["change_index"] = json!(5);
            value["aggregate"]["change_ref"] = json!(format!(
                "{}/C-01",
                value["aggregate"]["task_id"].as_str().unwrap()
            ));
            value["aggregate"]["patchset_id"] = json!(format!(
                "{}/C-01/P-02",
                value["aggregate"]["task_id"].as_str().unwrap()
            ));
            for (n, e) in value["entries"]
                .as_array_mut()
                .unwrap()
                .iter_mut()
                .enumerate()
            {
                e["change_index"] = json!(n);
                e["change_ref"] = json!(format!("{}/C-01", e["task_id"].as_str().unwrap()));
                e["local_change_id"] = json!("C-01");
                e["local_change_ref"] =
                    json!(format!("{}/C-01", e["local_task_id"].as_str().unwrap()));
                e["receipt_patchset_id"] =
                    json!(format!("{}/C-01/P-01", e["task_id"].as_str().unwrap()));
            }
        }
        let summary = format!("{prefix} {value}");
        let range = tx
            .append_payload(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                summary.as_bytes(),
            )
            .unwrap();
        p.summary_offset = range.payload_offset;
        p.summary_len = range.payload_len.try_into().unwrap();
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            index,
            &WorkflowBinaryV0Codec::encode_frozen_patchset(p).unwrap(),
        )
        .unwrap();
    }
    tx.commit().unwrap();
    drop(tx);
    ait_server_core::foundation::workflow_binary_v0_adapter::validate_frozen_server_workflow_v0(db)
        .unwrap();
    assert_eq!(patchset_count(db), 7);
    assert_eq!(
        store.get_task(None, "RLHT-0006").unwrap()["current_patchset_id"],
        "RLHT-0006/P-02"
    );
    fs::read(
        db.authority_root()
            .as_path()
            .join("patchset_summary_payload.bin"),
    )
    .unwrap()
}

/// Actual companion CLI, HTTP, immutable provenance and guarded acceptance.
/// The v1 rewrite is synthetic fixture setup, not a real-source conversion.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "run explicitly with AIT_CAPTURE_TEST_CLI pointing to the exact companion CLI build"]
async fn actual_cli_continues_six_legacy_receipts_and_corrects_the_fresh_v3_aggregate() {
    let directory = TestDirectory::new("legacy-six-paired-cli");
    let root = directory.path().canonicalize().unwrap();
    let config =
        ServerBinaryLifecycleConfig::from_server_data_root(root.join("server-data")).unwrap();
    initialize_installed_runtime(&config).unwrap();
    let operational = Arc::new(OperationalBinaryRuntime::ensure_from_config(&config).unwrap());
    let seed_optional = Arc::new(AtomicBool::new(true));
    let preparations = Arc::new(AtomicUsize::new(0));
    let seed_in = seed_optional.clone();
    let runtime_in = operational.clone();
    let count_in = preparations.clone();
    let router=build_router_with_state(state_from_operational(operational.clone())).layer(axum::middleware::from_fn(move |request:Request<Body>,next:axum::middleware::Next<Body>|{
        let seed=seed_in.clone();let runtime=runtime_in.clone();let count=count_in.clone();
        async move {
            let prepare=request.uri().path().ends_with("/history-promotion:prepare");
            let response=next.run(request).await;
            if prepare&&response.status().is_success() {
                count.fetch_add(1,Ordering::SeqCst);
                if seed.swap(false,Ordering::SeqCst) {
                    let (parts,body)=response.into_parts();let bytes=to_bytes(body).await.unwrap();
                    let value:JsonValue=serde_json::from_slice(&bytes).unwrap();
                    let (_,db)=runtime.repository_db(4).unwrap();
                    let store=BinaryDbServerWorkflowV0Store::new_remote_frozen(db,"LH").unwrap().with_repository_index(4);
                    // An explicit fixture statement releases only the initial
                    // aggregate. Replacement candidates receive no such gates.
                    store.put_attestation(value["aggregate"]["patchset_id"].as_str().unwrap(),&json!({"verification_state":"pass","require_tests_pass":false,"require_human_review":false,"require_lint_pass":false})).unwrap();
                    return axum::response::Response::from_parts(parts,axum::body::boxed(Body::from(bytes)));
                }
            }
            response
        }
    }));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::Server::from_tcp(listener)
            .unwrap()
            .serve(router.into_make_service())
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let cli = super::super::snapshot_capture::CaptureCli {
        executable: PathBuf::from(
            env::var_os("AIT_CAPTURE_TEST_CLI").expect("exact companion CLI"),
        ),
        native_home: root.join("native-home"),
    };
    fs::create_dir_all(&cli.native_home).unwrap();
    let client = root.join("legacy-client");
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
            "LH",
            "--user-name",
            "legacy-test",
            "--json",
        ],
    );
    cli.json(
        &client,
        &["remote", "add", "origin", &url, "--default", "--json"],
    );
    let mut local_task = String::new();
    for n in 1..=6 {
        let edit = root.join(format!("local-edit-{n}"));
        let started = cli.json(
            &client,
            &[
                "task",
                "start",
                "--title",
                &format!("Source Task {n}"),
                "--intent",
                "Preserve actual local history",
                "--local",
                "--edit-root",
                edit.to_str().unwrap(),
                "--json",
                "--full",
            ],
        );
        local_task = started["task_id"].as_str().unwrap().to_owned();
        if n == 1 {
            // Pin the remote to the first receipt boundary. Null-head
            // initialization at Task 6 would intentionally promote only Task 6.
            cli.json(
                &client,
                &["push", "--line", "main", "--remote", "origin", "--json"],
            );
        }
        fs::write(edit.join("revision.txt"), format!("local revision {n}\n")).unwrap();
        cli.json(
            &edit,
            &[
                "task",
                "finish",
                &local_task,
                "--message",
                "Local receipt only",
                "--local",
                "--json",
            ],
        );
    }
    cli.json(
        &client,
        &[
            "workflow",
            "ready",
            &local_task,
            "--remote",
            "origin",
            "--apply",
            "--json",
        ],
    );
    assert_eq!(preparations.load(Ordering::SeqCst), 1);
    let (_, db) = operational.repository_db(4).unwrap();
    let store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "LH")
        .unwrap()
        .with_repository_index(4);
    let remote = "RLHT-0006";
    let old_bytes = quote_source_era_summaries(&db, &store);
    let quoted = cli.json(
        &client,
        &[
            "patchset",
            "show",
            &format!("{remote}/P-02"),
            "--remote",
            "origin",
            "--json",
        ],
    );
    let quoted: JsonValue = serde_json::from_str(
        quoted["summary"]
            .as_str()
            .unwrap()
            .strip_prefix("ait-history-promotion/v1 ")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(quoted["contract"], "ait-history-promotion/v1");
    let (output, timed_out) = cli.run_bounded(
        &client,
        &[
            "workflow",
            "ready",
            &local_task,
            "--remote",
            "origin",
            "--apply",
            "--json",
        ],
    );
    assert_eq!(
        preparations.load(Ordering::SeqCst),
        2,
        "published legacy aggregate must be prepared before ready: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let pending = pending_ci(
        &output,
        timed_out,
        &db,
        &store,
        remote,
        &format!("{remote}/P-03"),
    );
    let fresh = store.get_patchset(None, &format!("{remote}/P-03")).unwrap();
    assert_eq!(fresh["source_kind"], "history_promotion_aggregate");
    assert!(fresh["change_ref"].is_null());
    assert_eq!(patchset_count(&db), 8);
    assert!(store.get_task(None, remote).unwrap()["successful_land"].is_null());
    let raw = fresh["summary"]
        .as_str()
        .unwrap()
        .strip_prefix("ait-history-promotion/v1 ")
        .unwrap();
    let manifest: JsonValue = serde_json::from_str(raw).unwrap();
    assert_eq!(manifest["contract"], "ait-history-promotion/v3");
    assert!(manifest["idempotency_key"]
        .as_str()
        .unwrap()
        .starts_with("history-promotion-v3:"));
    let (retry, timed_out) = cli.run_bounded(
        &client,
        &[
            "workflow",
            "ready",
            &local_task,
            "--remote",
            "origin",
            "--apply",
            "--json",
        ],
    );
    assert_eq!(
        pending_ci(
            &retry,
            timed_out,
            &db,
            &store,
            remote,
            &format!("{remote}/P-03")
        ),
        pending
    );
    assert_eq!(preparations.load(Ordering::SeqCst), 2);
    assert_eq!(patchset_count(&db), 8);
    fixture_gates(&store, remote, &format!("{remote}/P-03"));
    cli.json(
        &client,
        &[
            "workflow",
            "ready",
            &local_task,
            "--remote",
            "origin",
            "--apply",
            "--json",
        ],
    );
    let recovered = cli.json(
        &client,
        &[
            "worktree",
            "recover-task",
            remote,
            "--remote",
            "origin",
            "--json",
        ],
    );
    let edit = PathBuf::from(recovered["worktree"]["path"].as_str().unwrap());
    cli.json(
        &edit,
        &[
            "change",
            "create",
            remote,
            "--title",
            "Real correction",
            "--fork-snapshot",
            fresh["revision_snapshot_id"].as_str().unwrap(),
            "--remote",
            "origin",
            "--json",
        ],
    );
    fs::write(
        edit.join("correction.txt"),
        "correct the reviewed aggregate\n",
    )
    .unwrap();
    cli.json(
        &edit,
        &[
            "snapshot",
            "create",
            &format!("{remote}/C-01"),
            "--message",
            "Capture real correction",
            "--json",
        ],
    );
    let (correcting, timed_out) = cli.run_bounded(
        &edit,
        &[
            "workflow", "ready", remote, "--remote", "origin", "--apply", "--json",
        ],
    );
    pending_ci(
        &correcting,
        timed_out,
        &db,
        &store,
        remote,
        &format!("{remote}/P-04"),
    );
    assert_eq!(preparations.load(Ordering::SeqCst), 3);
    assert_eq!(patchset_count(&db), 9);
    let corrected = store.get_patchset(None, &format!("{remote}/P-04")).unwrap();
    assert_eq!(corrected["change_ref"], format!("{remote}/C-01"));
    let corrected_manifest: JsonValue = serde_json::from_str(
        corrected["summary"]
            .as_str()
            .unwrap()
            .strip_prefix("ait-history-promotion/v1 ")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        corrected_manifest["contract"],
        "ait-history-promotion-correction/v2"
    );
    assert_eq!(corrected_manifest["entries"], manifest["entries"]);
    fixture_gates(&store, remote, &format!("{remote}/P-04"));
    cli.json(
        &edit,
        &[
            "workflow", "ready", remote, "--remote", "origin", "--apply", "--json",
        ],
    );
    cli.json(
        &edit,
        &[
            "workflow", "finish", remote, "--remote", "origin", "--apply", "--json",
            "--reviewer","paired-fixture-reviewer","--review-message",
            "Reviewed files\n\ncorrection.txt\n\nFindings\n\nThe correction preserves all six imported receipts.\n\nRisks\n\nThis acceptance uses isolated fixture data only.\n\nTests\n\nRequired CI stayed pending without a runner; exact candidate gates were then supplied by the fixture.\n\nRecommendation\n\nAccept this isolated corrected candidate.",
        ],
    );
    let accepted = store.get_task(None, remote).unwrap()["successful_land"].clone();
    assert_eq!(accepted["patchset_id"], format!("{remote}/C-01/P-04"));
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        1
    );
    assert!(fs::read(
        db.authority_root()
            .as_path()
            .join("patchset_summary_payload.bin")
    )
    .unwrap()
    .starts_with(&old_bytes));
    for n in 1..=5 {
        let task = store.get_task(None, &format!("RLHT-{n:04}")).unwrap();
        assert_eq!(task["status"], "active");
        assert!(task["successful_land"].is_null());
    }
    assert_eq!(
        store.get_land(None, &format!("{remote}/L-01")).unwrap()["patchset_id"],
        accepted["patchset_id"]
    );
    stop.send(()).unwrap();
    server.await.unwrap();
    drop(store);
    drop(db);
    drop(operational);
    let reopened = OperationalBinaryRuntime::ensure_from_config(&config).unwrap();
    let (_, db) = reopened.repository_db(4).unwrap();
    let reopened_store = BinaryDbServerWorkflowV0Store::new_remote_frozen(db, "LH")
        .unwrap()
        .with_repository_index(4);
    assert_eq!(
        reopened_store.get_task(None, remote).unwrap()["successful_land"],
        accepted
    );
    println!("paired legacy six: fresh v3 P03, stable pending CI, real C01 correction P04, explicit fixture gates, one guarded acceptance and normal reopen passed");
}
