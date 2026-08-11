use super::*;
use crate::foundation::remote_binary_db::{
    binary_db_runtime_error_kind, BinaryDbCommandScope, BinaryDbErrorKind, BinaryDbWriteTxn,
    FilesystemServerRemoteBinaryDb, RepoId, RepoName, StoreGeneration, StorePath,
};
use crate::foundation::server_binary_db_schema_registry::{
    SERVER_BINARY_DB_BIN_SCHEMAS, SERVER_BINARY_DB_INDEX_SCHEMAS, SERVER_BINARY_DB_LAYOUT_ID,
};
use crate::foundation::server_content_binary_db::{
    ServerBinaryDbLineStore, SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use crate::foundation::server_workflow_store::{
    ServerWorkflowChangeStore, ServerWorkflowTaskStore,
};
use crate::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store;
use serde_json::json;
use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

fn make_temporary_root(label: &str) -> StorePath {
    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
    StorePath::new(std::env::temp_dir().join(format!(
        "ait-server-queue-binary-{label}-{}-{sequence}",
        std::process::id()
    )))
}

fn make_db(label: &str) -> FilesystemServerRemoteBinaryDb {
    let root = make_temporary_root(label);
    fs::create_dir_all(root.as_path()).expect("create current Binary DB fixture root");
    for path in SERVER_BINARY_DB_BIN_SCHEMAS
        .iter()
        .map(|schema| schema.path)
        .chain(
            SERVER_BINARY_DB_INDEX_SCHEMAS
                .iter()
                .map(|schema| schema.path),
        )
    {
        fs::write(
            root.as_path().join(path),
            SERVER_BINARY_DB_LAYOUT_ID.to_le_bytes(),
        )
        .unwrap_or_else(|error| panic!("initialize {path}: {error}"));
    }
    FilesystemServerRemoteBinaryDb::test_fixture(
        RepoId::new("repo-id"),
        RepoName::new("repo"),
        root,
        StoreGeneration::new(1),
    )
}

fn wait_for_queue_summary(
    service: &BinaryDbServerWorkflowReadModelService<FilesystemServerRemoteBinaryDb>,
) -> JsonValue {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match service.read_queue_summary(Some("repo"), Some("active"), true) {
            Ok(summary) => return summary,
            Err(error)
                if binary_db_runtime_error_kind(&error)
                    == Some(BinaryDbErrorKind::RetryableBusy)
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("queue projection did not warm: {error}"),
        }
    }
}

fn wait_for_refresh_attempt_count(
    service: &BinaryDbServerWorkflowReadModelService<FilesystemServerRemoteBinaryDb>,
    expected: u64,
) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.queue_projection_refresh_attempt_count() < expected {
        assert!(
            Instant::now() < deadline,
            "queue projection refresh attempt count did not reach {expected}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn repeated_cold_read_refresh_requests_do_not_queue_a_second_full_scan() {
    let db = make_db("workflow-cold-refresh-coalescing");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let service = BinaryDbServerWorkflowReadModelService::new(db, store.into_arc());
    service.set_queue_projection_refresh_in_flight_for_test(true);

    for _ in 0..32 {
        service.request_queue_projection_refresh();
    }

    assert_eq!(service.queue_projection_refresh_request_generation(), 0);
    assert!(!service.queue_projection_refresh_pending());
    assert!(!service.queue_projection_refresh_immediate());
    service.set_queue_projection_refresh_in_flight_for_test(false);
}

#[test]
fn explicit_read_refresh_overrides_only_a_pending_mutation_debounce() {
    let db = make_db("workflow-explicit-read-refresh");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    let service = BinaryDbServerWorkflowReadModelService::new(db, store.into_arc());
    service.set_queue_projection_refresh_in_flight_for_test(true);

    service.request_queue_projection_refresh_after_mutation();
    assert!(service.queue_projection_refresh_pending());
    assert!(!service.queue_projection_refresh_immediate());

    service.request_queue_projection_refresh();
    assert!(service.queue_projection_refresh_immediate());
}

#[test]
fn warm_queue_reads_do_not_rebuild_without_a_mutation_or_refresh_error() {
    let db = make_db("workflow-warm-read-stability");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .create_task(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Stable warm queue",
                "intent": "Do not rescan unchanged workflow rows"
            }),
        )
        .expect("create stable queue task");
    let service = BinaryDbServerWorkflowReadModelService::new(db, store.into_arc());
    service
        .read_queue_summary(Some("repo"), Some("active"), true)
        .expect_err("cold queue read should warm in the background");
    let expected = wait_for_queue_summary(&service);
    let attempts = service.queue_projection_refresh_attempt_count();

    std::thread::sleep(Duration::from_millis(300));
    for _ in 0..16 {
        assert_eq!(
            service
                .read_queue_summary(Some("repo"), Some("active"), true)
                .expect("warm queue summary"),
            expected
        );
    }
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(service.queue_projection_refresh_attempt_count(), attempts);
}

#[test]
fn server_workflow_binary_db_projects_queue_rows_without_worker_storage() {
    let db = make_db("workflow");
    ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone())
        .create_line("main", 0, 1)
        .expect("create canonical main Line");
    let store = BinaryDbServerWorkflowV0Store::new(db);
    store
        .create_task(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Queue projection",
                "intent": "Read workflow rows through Binary DB"
            }),
        )
        .expect("create task");
    store
        .create_change(
            "repo",
            &json!({
                "change_id": "C-01",
                "task_id": "T-0001",
                "title": "Queue change",
                "base_line": "main"
            }),
        )
        .expect("create change");
    let workflow = store.into_arc();
    let authority_root = make_temporary_root("queue-service");
    let root = authority_root.as_path().to_path_buf();
    let service = BinaryDbServerWorkflowReadModelService::new(
        FilesystemServerRemoteBinaryDb::test_fixture(
            RepoId::new("repo-id"),
            RepoName::new("repo"),
            authority_root,
            StoreGeneration::new(1),
        ),
        workflow,
    );
    assert!(
        !service.queue_projection_refresh_attempted(),
        "constructing a workflow read model for an unrelated Plan request must not scan workflow rows"
    );

    let started = Instant::now();
    let warming = service
        .read_queue_summary(Some("repo"), Some("active"), true)
        .expect_err("a cold queue read must schedule work instead of scanning request-bound");
    assert_eq!(
        binary_db_runtime_error_kind(&warming),
        Some(BinaryDbErrorKind::RetryableBusy)
    );
    assert!(
        started.elapsed() < Duration::from_millis(100),
        "cold queue read must return structured warming state promptly"
    );
    let summary = wait_for_queue_summary(&service);
    assert!(service.queue_projection_refresh_attempted());
    assert!(
        service
            .queue_projection_inputs_share_row_storage()
            .expect("cached projection storage check"),
        "status and inventory variants must share immutable cached JSON rows"
    );
    assert_eq!(summary["task_queue"]["count"], json!(1));
    assert_eq!(
        summary["task_queue"]["items"][0]["task"]["task_id"],
        json!("T-0001")
    );
    assert_eq!(
        summary["query_plan"]["input_counts"]["review_requests"],
        json!(0)
    );
    assert_eq!(
        summary["query_plan"]["input_counts"]["attestations"],
        json!(0)
    );
    assert!(!summary
        .to_string()
        .contains("queue-detail-must-not-materialize"));
    let task_queue = service
        .read_task_queue(Some("repo"), Some("active"))
        .expect("task queue");
    assert_eq!(task_queue["count"], json!(1));
    let reviewer_inbox = service
        .read_reviewer_inbox(Some("repo"))
        .expect("reviewer inbox");
    assert_eq!(reviewer_inbox["filters"]["repo_name"], json!("repo"));
    assert!(!root.join("server_worker_job.bin").exists());
    assert!(!root.join("server_worker_job_payload.bin").exists());
}

#[test]
fn warm_queue_projection_does_not_wait_for_an_active_workflow_writer() {
    let db = make_db("workflow-cache-writer");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .create_task(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Cached queue projection",
                "intent": "Serve the last complete projection while a writer is active"
            }),
        )
        .expect("create cached task");
    let workflow = store.into_arc();
    let service = BinaryDbServerWorkflowReadModelService::new(db.clone(), workflow);
    let warming = service
        .read_queue_summary(Some("repo"), Some("active"), true)
        .expect_err("cold queue projection should warm in the background");
    assert_eq!(
        binary_db_runtime_error_kind(&warming),
        Some(BinaryDbErrorKind::RetryableBusy)
    );
    let initial = wait_for_queue_summary(&service);
    assert_eq!(initial["task_queue"]["count"], json!(1));

    let mut writer = BinaryDbWriteTxn::begin(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("workflow writer should acquire the scope");
    let refresh_started = Instant::now();
    service.request_queue_projection_refresh();
    assert!(
        refresh_started.elapsed() < Duration::from_millis(100),
        "mutation refresh request must not wait for the active workflow writer"
    );
    let read_started = Instant::now();
    let cached = service
        .read_queue_summary(Some("repo"), Some("active"), true)
        .expect("warm queue projection must not wait for the active writer");
    assert!(
        read_started.elapsed() < Duration::from_millis(100),
        "warm queue read must return the last complete projection promptly"
    );
    assert_eq!(cached, initial);
    writer.abort().expect("abort empty writer");
}

#[test]
fn mutation_projection_refresh_waits_for_a_quiet_burst_and_yields_to_writers() {
    let db = make_db("workflow-mutation-debounce");
    let store = BinaryDbServerWorkflowV0Store::new(db.clone());
    store
        .create_task(
            "repo",
            &json!({
                "task_id": "T-0001",
                "title": "Mutation debounce",
                "intent": "Keep projection readers out of a closeout writer burst"
            }),
        )
        .expect("create debounce task");
    let service = BinaryDbServerWorkflowReadModelService::new(db.clone(), store.into_arc());
    service
        .read_queue_summary(Some("repo"), Some("active"), true)
        .expect_err("cold queue projection should warm in the background");
    let initial = wait_for_queue_summary(&service);
    let initial_attempts = service.queue_projection_refresh_attempt_count();

    service.request_queue_projection_refresh_after_mutation();
    std::thread::sleep(QUEUE_PROJECTION_MUTATION_QUIET_PERIOD / 2 + Duration::from_millis(20));
    service.request_queue_projection_refresh_after_mutation();
    std::thread::sleep(QUEUE_PROJECTION_MUTATION_QUIET_PERIOD / 2 + Duration::from_millis(20));
    assert_eq!(
        service.queue_projection_refresh_attempt_count(),
        initial_attempts,
        "a later mutation must reset the quiet period instead of starting a stale full scan"
    );

    let writer_started = Instant::now();
    let mut writer = BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow)
        .expect("closeout writer should acquire while the projection refresh is debounced");
    assert!(
        writer_started.elapsed() < Duration::from_millis(100),
        "a pending mutation refresh must not hold the WORKFLOW read scope"
    );
    writer.abort().expect("abort empty closeout writer");

    let read_started = Instant::now();
    let cached = service
        .read_queue_summary(Some("repo"), Some("active"), true)
        .expect("an explicit warm queue read should return cached rows and override debounce");
    assert!(read_started.elapsed() < Duration::from_millis(100));
    assert_eq!(cached, initial);
    wait_for_refresh_attempt_count(&service, initial_attempts + 1);
}
