use super::{
    action_mutation_receipts_with_task_workflow_closeout_remote,
    change_lineage_payload_with_task_workflow_task_remote,
    close_change_with_task_workflow_task_remote, close_client_with_task_workflow_closeout_remote,
    close_client_with_task_workflow_task_remote, close_line_with_task_workflow_task_remote,
    close_task_with_task_workflow_closeout_remote, create_change_with_task_workflow_task_remote,
    create_task_with_task_workflow_task_remote, create_waiver_with_task_workflow_closeout_remote,
    ensure_repository_with_task_workflow_task_remote,
    evaluate_policy_with_task_workflow_closeout_remote,
    get_attestation_with_task_workflow_closeout_remote,
    get_change_detail_with_task_workflow_task_remote, get_change_with_task_workflow_task_remote,
    get_land_with_task_workflow_closeout_remote, get_line_with_task_workflow_task_remote,
    get_patchset_with_task_workflow_closeout_remote, get_policy_with_task_workflow_closeout_remote,
    get_remote_snapshot_with_task_workflow_task_remote,
    get_remote_snapshots_existence_with_task_workflow_task_remote,
    get_remote_zstd_import_manifest_with_task_workflow_task_remote,
    get_remote_zstd_object_pack_with_task_workflow_task_remote,
    get_remote_zstd_tree_pack_with_task_workflow_task_remote,
    get_repository_with_task_workflow_task_remote, get_task_with_task_workflow_task_remote,
    inspect_client_with_task_workflow_closeout_remote,
    inspect_client_with_task_workflow_task_remote, list_changes_with_task_workflow_task_remote,
    list_lines_with_task_workflow_task_remote, list_patchsets_with_task_workflow_closeout_remote,
    list_repo_jobs_with_task_workflow_closeout_remote,
    list_reviews_with_task_workflow_closeout_remote, list_tasks_with_task_workflow_task_remote,
    mutation_receipt_with_task_workflow_closeout_remote,
    normalize_task_workflow_http_compatibility_payload_json,
    normalize_task_workflow_http_readiness_payload_json,
    publish_patchset_with_task_workflow_closeout_remote,
    put_attestation_with_task_workflow_closeout_remote,
    read_patchset_ci_readiness_with_task_workflow_closeout_remote,
    read_patchset_ci_status_with_task_workflow_closeout_remote,
    read_queue_summary_bundle_with_task_workflow_task_remote,
    read_reviewer_inbox_with_task_workflow_task_remote,
    read_task_audit_with_task_workflow_task_remote, read_task_queue_with_task_workflow_task_remote,
    record_review_with_task_workflow_closeout_remote,
    request_review_with_task_workflow_closeout_remote,
    retry_land_with_task_workflow_closeout_remote,
    run_patchset_ci_with_task_workflow_closeout_remote,
    select_patchset_with_task_workflow_closeout_remote,
    submit_land_with_task_workflow_closeout_remote,
    submit_task_land_with_task_workflow_closeout_remote,
    update_remote_line_with_task_workflow_task_remote, TaskWorkflowActionMutationReceiptsBuilder,
    TaskWorkflowAttestationReader, TaskWorkflowAttestationRemote, TaskWorkflowAttestationWriter,
    TaskWorkflowChangeRemote, TaskWorkflowCloseoutRemote, TaskWorkflowHttpClientCloser,
    TaskWorkflowHttpClientConfig, TaskWorkflowHttpClientInspector, TaskWorkflowHttpClientRemote,
    TaskWorkflowHttpClientResult, TaskWorkflowHttpClientStats, TaskWorkflowLandReader,
    TaskWorkflowLandRemote, TaskWorkflowLandRetryer, TaskWorkflowLandSubmitter,
    TaskWorkflowLineCloser, TaskWorkflowLineHeadUpdater, TaskWorkflowLineLister,
    TaskWorkflowLineReader, TaskWorkflowLineRemote, TaskWorkflowLineagePayloadBuilder,
    TaskWorkflowMutationReceiptBuilder, TaskWorkflowMutationReceiptRemote,
    TaskWorkflowPatchsetCiRemote, TaskWorkflowPatchsetCiRunner, TaskWorkflowPatchsetCiStatusReader,
    TaskWorkflowPatchsetLister, TaskWorkflowPatchsetPublisher, TaskWorkflowPatchsetReader,
    TaskWorkflowPatchsetRemote, TaskWorkflowPatchsetSelector, TaskWorkflowPolicyEvaluator,
    TaskWorkflowPolicyReader, TaskWorkflowPolicyRemote, TaskWorkflowPolicyWaiverCreator,
    TaskWorkflowQueueRemote, TaskWorkflowQueueSummaryBundleReader, TaskWorkflowRemoteChangeCloser,
    TaskWorkflowRemoteChangeCreator, TaskWorkflowRemoteChangeDetailReader,
    TaskWorkflowRemoteChangeLister, TaskWorkflowRemoteChangeReader,
    TaskWorkflowRemoteTaskAuditReader, TaskWorkflowRemoteTaskCloser, TaskWorkflowRemoteTaskCreator,
    TaskWorkflowRemoteTaskLister, TaskWorkflowRemoteTaskReader, TaskWorkflowRepoJobLister,
    TaskWorkflowRepositoryEnsurer, TaskWorkflowRepositoryReader, TaskWorkflowRepositoryRemote,
    TaskWorkflowReviewLister, TaskWorkflowReviewRecorder, TaskWorkflowReviewRemote,
    TaskWorkflowReviewRequester, TaskWorkflowReviewerInboxReader,
    TaskWorkflowSnapshotExistenceReader, TaskWorkflowSnapshotMetadataReader,
    TaskWorkflowSnapshotRemote, TaskWorkflowTaskQueueReader, TaskWorkflowTaskRecordRemote,
    TaskWorkflowTaskRemote, TaskWorkflowZstdPackReader, TaskWorkflowZstdPackUploader,
};
use crate::json_support::{json, JsonCodec, JsonValue};
use crate::plan_http_client::PlanHttpClientError;
use crate::repository_pack_json::ZstdImportManifestPayload;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug)]
struct RecordedHttpRequest {
    method: String,
    target: String,
    body: Option<JsonValue>,
}

fn serve_task_workflow_json_once(
    response: JsonValue,
) -> (
    TaskWorkflowHttpClientConfig,
    JoinHandle<RecordedHttpRequest>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
    let addr = listener.local_addr().expect("fixture server addr");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fixture request");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set fixture read timeout");
        let request = read_recorded_http_request(&mut stream);
        let response_text = response.to_string();
        let http_response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_text.len(),
            response_text
        );
        stream
            .write_all(http_response.as_bytes())
            .expect("write fixture response");
        request
    });
    (
        TaskWorkflowHttpClientConfig {
            base_url: format!("http://{addr}/"),
            repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
            headers: BTreeMap::new(),
            default_timeout_ms: 5_000,
            retry_attempts: 0,
            retry_backoff_ms: 0,
            pool_max_idle_per_host: 1,
        },
        handle,
    )
}

fn serve_task_workflow_timeout_then_json(
    response: JsonValue,
) -> (
    TaskWorkflowHttpClientConfig,
    JoinHandle<Vec<RecordedHttpRequest>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind timeout fixture server");
    let addr = listener.local_addr().expect("timeout fixture server addr");
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        let mut delayed_first_response = None;
        for attempt in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept timeout fixture request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set timeout fixture read timeout");
            requests.push(read_recorded_http_request(&mut stream));
            let response_text = response.to_string();
            if attempt == 0 {
                delayed_first_response = Some(thread::spawn(move || {
                    thread::sleep(Duration::from_millis(30));
                    let http_response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_text.len(),
                        response_text
                    );
                    let _ = stream.write_all(http_response.as_bytes());
                }));
                continue;
            }
            let http_response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_text.len(),
                response_text
            );
            let _ = stream.write_all(http_response.as_bytes());
        }
        if let Some(delayed_first_response) = delayed_first_response {
            delayed_first_response
                .join()
                .expect("join delayed timeout fixture response");
        }
        requests
    });
    (
        TaskWorkflowHttpClientConfig {
            base_url: format!("http://{addr}/"),
            repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
            headers: BTreeMap::new(),
            default_timeout_ms: 20,
            retry_attempts: 0,
            retry_backoff_ms: 0,
            pool_max_idle_per_host: 1,
        },
        handle,
    )
}

fn serve_task_workflow_repeated_timeouts_then_json(
    response: JsonValue,
) -> (
    TaskWorkflowHttpClientConfig,
    JoinHandle<Vec<RecordedHttpRequest>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind repeated-timeout server");
    let addr = listener
        .local_addr()
        .expect("repeated-timeout fixture server addr");
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        let mut delayed_responses = Vec::new();
        for attempt in 0..3 {
            let (mut stream, _) = listener
                .accept()
                .expect("accept repeated-timeout fixture request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set repeated-timeout fixture read timeout");
            requests.push(read_recorded_http_request(&mut stream));
            let response_text = response.to_string();
            if attempt < 2 {
                delayed_responses.push(thread::spawn(move || {
                    thread::sleep(Duration::from_millis(40));
                    let http_response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response_text.len(),
                        response_text
                    );
                    let _ = stream.write_all(http_response.as_bytes());
                }));
                continue;
            }
            let http_response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_text.len(),
                response_text
            );
            let _ = stream.write_all(http_response.as_bytes());
        }
        for delayed_response in delayed_responses {
            delayed_response
                .join()
                .expect("join delayed repeated-timeout fixture response");
        }
        requests
    });
    (
        TaskWorkflowHttpClientConfig {
            base_url: format!("http://{addr}/"),
            repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
            headers: BTreeMap::new(),
            default_timeout_ms: 20,
            retry_attempts: 0,
            retry_backoff_ms: 0,
            pool_max_idle_per_host: 1,
        },
        handle,
    )
}

fn read_recorded_http_request(stream: &mut impl Read) -> RecordedHttpRequest {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).expect("read fixture request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some((header_end, content_length)) = http_request_header_end_and_length(&buffer) {
            let body_start = header_end + 4;
            if buffer.len() >= body_start + content_length {
                break;
            }
        }
    }

    let (header_end, content_length) =
        http_request_header_end_and_length(&buffer).expect("fixture request headers");
    let header_text = String::from_utf8_lossy(&buffer[..header_end]);
    let request_line = header_text.lines().next().expect("fixture request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().expect("fixture request method").to_string();
    let target = parts.next().expect("fixture request target").to_string();
    let body_start = header_end + 4;
    let body_end = body_start + content_length;
    let body = if content_length == 0 {
        None
    } else {
        Some(
            JsonCodec::parse_slice_with_error_prefix(
                &buffer[body_start..body_end],
                "Failed to parse fixture request JSON body",
            )
            .expect("fixture request JSON body"),
        )
    };
    RecordedHttpRequest {
        method,
        target,
        body,
    }
}

fn http_request_header_end_and_length(buffer: &[u8]) -> Option<(usize, usize)> {
    let header_end = buffer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let header_text = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = header_text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.trim().eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    Some((header_end, content_length))
}

#[test]
fn task_workflow_http_json_normalizers_parse_at_task_boundary() {
    let payload = normalize_task_workflow_http_compatibility_payload_json(
        r#"{"status":"ok","transport":"http"}"#,
    )
    .unwrap();
    assert_eq!(payload["status"], "ok");

    let readiness = normalize_task_workflow_http_readiness_payload_json(r#"{"ready":true}"#)
        .expect("readiness payload");
    assert_eq!(readiness["ready"], true);

    let invalid_json = normalize_task_workflow_http_compatibility_payload_json("{").unwrap_err();
    assert!(invalid_json
        .to_string()
        .contains("task workflow HTTP compatibility payload invalid JSON"));

    let non_object = normalize_task_workflow_http_readiness_payload_json("[]").unwrap_err();
    assert_eq!(
        non_object.to_string(),
        "task workflow HTTP readiness payload must be an object"
    );
}

#[test]
fn change_listing_recovery_does_not_restart_an_exhausted_busy_read_budget() {
    let busy = PlanHttpClientError::RemoteResponse {
        method: "GET".to_string(),
        url: "http://127.0.0.1/v1/native/changes/RCT-1%2FC-01".to_string(),
        status: 503,
        detail: "ait.binary-db.error.v1|retryable_busy|writer_scope=ServerLand".to_string(),
    };
    assert!(!super::helpers::change_read_error_allows_listing_recovery(
        &busy
    ));
    assert!(super::helpers::change_read_error_allows_listing_recovery(
        &PlanHttpClientError::Remote("GET failed: 404 unknown change".to_string())
    ));
}

#[derive(Debug, Default)]
struct FakeTaskRemote {
    closed: bool,
}

#[derive(Debug, Default)]
struct FakeClientLifecycleRemote {
    closed: bool,
}

#[derive(Debug, Default)]
struct FakeClientInspectorPort;

#[derive(Debug, Default)]
struct FakeClientCloserPort {
    closed: bool,
}

#[derive(Debug, Default)]
struct FakeTaskRecordRemote;

#[derive(Debug, Default)]
struct FakeRemoteTaskReaderPort;

#[derive(Debug, Default)]
struct FakeRemoteTaskListerPort;

#[derive(Debug, Default)]
struct FakeRemoteTaskAuditReaderPort;

#[derive(Debug, Default)]
struct FakeRemoteTaskCreatorPort;

fn fake_stats(closed: bool) -> TaskWorkflowHttpClientStats {
    TaskWorkflowHttpClientStats {
        base_url: "https://ait.example".to_string(),
        default_timeout_ms: 30_000,
        retry_attempts: 1,
        retry_backoff_ms: 10,
        pool_max_idle_per_host: 2,
        request_count: 3,
        retry_count: 0,
        closed,
    }
}

impl TaskWorkflowHttpClientInspector for FakeTaskRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        fake_stats(self.closed)
    }
}

impl TaskWorkflowHttpClientCloser for FakeTaskRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        self.closed = true;
        fake_stats(true)
    }
}

impl TaskWorkflowHttpClientInspector for FakeClientLifecycleRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        fake_stats(self.closed)
    }
}

impl TaskWorkflowHttpClientCloser for FakeClientLifecycleRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        self.closed = true;
        fake_stats(true)
    }
}

impl TaskWorkflowHttpClientInspector for FakeClientInspectorPort {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        fake_stats(false)
    }
}

impl TaskWorkflowHttpClientCloser for FakeClientCloserPort {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        self.closed = true;
        fake_stats(true)
    }
}

impl TaskWorkflowRepositoryEnsurer for FakeTaskRemote {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy.cloned().unwrap_or(JsonValue::Null),
            "id_namespace_prefix": id_namespace_prefix,
        }))
    }
}

impl TaskWorkflowRepositoryReader for FakeTaskRemote {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"repo_name": repo_name}))
    }
}

impl TaskWorkflowLineagePayloadBuilder for FakeTaskRemote {
    fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "base_line": base_line,
            "fork_snapshot_id": line_row
                .and_then(|row| row.get("head_snapshot_id"))
                .cloned()
                .unwrap_or(JsonValue::Null),
        }))
    }
}

impl TaskWorkflowLineReader for FakeTaskRemote {
    fn get_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"repo_name": repo_name, "line_name": line_name}))
    }
}

impl TaskWorkflowLineLister for FakeTaskRemote {
    fn list_lines(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({"repo_name": repo_name, "line_name": "main"})])
    }
}

impl TaskWorkflowRemoteTaskReader for FakeTaskRemote {
    fn get_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"task_id": task_id, "repo_name": repo_name}))
    }
}

impl TaskWorkflowRemoteTaskLister for FakeTaskRemote {
    fn list_tasks(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({"repo_name": repo_name, "task_id": "T-1"})])
    }
}

impl TaskWorkflowRemoteTaskAuditReader for FakeTaskRemote {
    fn read_task_audit(
        &mut self,
        repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id,
            "target_line": target_line,
        }))
    }
}

impl TaskWorkflowTaskQueueReader for FakeTaskRemote {
    fn read_task_queue(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"repo_name": repo_name, "status": status}))
    }
}

impl TaskWorkflowReviewerInboxReader for FakeTaskRemote {
    fn read_reviewer_inbox(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"repo_name": repo_name, "reviewer_inbox": true}))
    }
}

impl TaskWorkflowQueueSummaryBundleReader for FakeTaskRemote {
    fn read_queue_summary_bundle(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"repo_name": repo_name, "status": status, "summary": true}))
    }
}

impl TaskWorkflowRemoteTaskCreator for FakeTaskRemote {
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "title": title,
            "intent": intent,
            "task_id": task_id,
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        }))
    }
}

impl TaskWorkflowRemoteChangeCreator for FakeTaskRemote {
    fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id,
            "title": title,
            "base_line": base_line,
            "change_id": change_id,
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": forked_from_line,
        }))
    }
}

impl TaskWorkflowRemoteChangeLister for FakeTaskRemote {
    fn list_changes(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({"repo_name": repo_name, "change_id": "C-1"})])
    }
}

impl TaskWorkflowRemoteChangeDetailReader for FakeTaskRemote {
    fn get_change_detail(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"change_id": change_ref, "repo_name": repo_name, "detail": true}))
    }
}

impl TaskWorkflowRemoteChangeReader for FakeTaskRemote {
    fn get_change(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"change_id": change_ref, "repo_name": repo_name}))
    }
}

impl TaskWorkflowRemoteChangeCloser for FakeTaskRemote {
    fn close_change(
        &mut self,
        change_ref: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"change_id": change_ref, "status": status, "repo_name": repo_name}))
    }
}

impl TaskWorkflowLineHeadUpdater for FakeTaskRemote {
    fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "head_snapshot_id": head_snapshot_id,
            "expected_head_snapshot_id": expected_head_snapshot_id,
        }))
    }
}

impl TaskWorkflowLineCloser for FakeTaskRemote {
    fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"repo_name": repo_name, "line_name": line_name, "status": status}))
    }
}

impl TaskWorkflowZstdPackUploader for FakeTaskRemote {}

impl TaskWorkflowSnapshotMetadataReader for FakeTaskRemote {
    fn get_remote_snapshot(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "snapshot_id": snapshot_id,
            "include_content": include_content,
            "path": path,
        }))
    }
}

impl TaskWorkflowZstdPackReader for FakeTaskRemote {}

impl TaskWorkflowSnapshotExistenceReader for FakeTaskRemote {
    fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"repo_name": repo_name, "snapshot_ids": snapshot_ids}))
    }
}

impl TaskWorkflowRemoteTaskReader for FakeTaskRecordRemote {
    fn get_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "task_id": task_id,
            "repo_name": repo_name,
        }))
    }
}

impl TaskWorkflowRemoteTaskLister for FakeTaskRecordRemote {
    fn list_tasks(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "task_id": "T-1",
            "repo_name": repo_name,
        })])
    }
}

impl TaskWorkflowRemoteTaskAuditReader for FakeTaskRecordRemote {
    fn read_task_audit(
        &mut self,
        repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id,
            "target_line": target_line,
        }))
    }
}

impl TaskWorkflowRemoteTaskCreator for FakeTaskRecordRemote {
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id.unwrap_or("T-NEW"),
            "title": title,
            "intent": intent,
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        }))
    }
}

impl TaskWorkflowRemoteTaskReader for FakeRemoteTaskReaderPort {
    fn get_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "task_id": task_id,
            "repo_name": repo_name,
        }))
    }
}

impl TaskWorkflowRemoteTaskLister for FakeRemoteTaskListerPort {
    fn list_tasks(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "task_id": "T-LIST",
            "repo_name": repo_name,
        })])
    }
}

impl TaskWorkflowRemoteTaskAuditReader for FakeRemoteTaskAuditReaderPort {
    fn read_task_audit(
        &mut self,
        repo_name: &str,
        task_id: &str,
        target_line: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id,
            "target_line": target_line,
        }))
    }
}

impl TaskWorkflowRemoteTaskCreator for FakeRemoteTaskCreatorPort {
    fn create_task(
        &mut self,
        repo_name: &str,
        title: &str,
        intent: &str,
        task_id: Option<&str>,
        plan_id: Option<&str>,
        origin_plan_revision_id: Option<&str>,
        plan_item_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id.unwrap_or("T-CREATE"),
            "title": title,
            "intent": intent,
            "plan_id": plan_id,
            "origin_plan_revision_id": origin_plan_revision_id,
            "plan_item_ref": plan_item_ref,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeRepositoryRemote;

#[derive(Debug, Default)]
struct FakeRepositoryEnsurerPort;

#[derive(Debug, Default)]
struct FakeRepositoryReaderPort;

impl TaskWorkflowRepositoryEnsurer for FakeRepositoryRemote {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy.cloned().unwrap_or(JsonValue::Null),
            "id_namespace_prefix": id_namespace_prefix,
        }))
    }
}

impl TaskWorkflowRepositoryReader for FakeRepositoryRemote {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "default_line": "main",
        }))
    }
}

impl TaskWorkflowRepositoryEnsurer for FakeRepositoryEnsurerPort {
    fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&JsonValue>,
        id_namespace_prefix: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy.cloned().unwrap_or(JsonValue::Null),
            "id_namespace_prefix": id_namespace_prefix,
        }))
    }
}

impl TaskWorkflowRepositoryReader for FakeRepositoryReaderPort {
    fn get_repository(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "default_line": "reader-only",
        }))
    }
}

#[derive(Debug, Default)]
struct FakeLineRemote;

#[derive(Debug, Default)]
struct FakeLineagePayloadBuilder;

#[derive(Debug, Default)]
struct FakeLineReader;

#[derive(Debug, Default)]
struct FakeLineLister;

#[derive(Debug, Default)]
struct FakeLineHeadUpdater;

#[derive(Debug, Default)]
struct FakeLineCloser;

impl TaskWorkflowLineagePayloadBuilder for FakeLineRemote {
    fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "base_line": base_line,
            "fork_snapshot_id": line_row
                .and_then(|row| row.get("head_snapshot_id"))
                .cloned()
                .unwrap_or(JsonValue::Null),
        }))
    }
}

impl TaskWorkflowLineagePayloadBuilder for FakeLineagePayloadBuilder {
    fn change_lineage_payload(
        &self,
        base_line: &str,
        line_row: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "base_line": base_line,
            "fork_snapshot_id": line_row
                .and_then(|row| row.get("head_snapshot_id"))
                .cloned()
                .unwrap_or(JsonValue::Null),
        }))
    }
}

impl TaskWorkflowLineReader for FakeLineRemote {
    fn get_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "head_snapshot_id": "SNP-LINE",
        }))
    }
}

impl TaskWorkflowLineReader for FakeLineReader {
    fn get_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "head_snapshot_id": "SNP-LINE",
        }))
    }
}

impl TaskWorkflowLineLister for FakeLineRemote {
    fn list_lines(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({"repo_name": repo_name, "line_name": "main"})])
    }
}

impl TaskWorkflowLineLister for FakeLineLister {
    fn list_lines(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({"repo_name": repo_name, "line_name": "main"})])
    }
}

impl TaskWorkflowLineHeadUpdater for FakeLineRemote {
    fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "head_snapshot_id": head_snapshot_id,
            "expected_head_snapshot_id": expected_head_snapshot_id,
        }))
    }
}

impl TaskWorkflowLineHeadUpdater for FakeLineHeadUpdater {
    fn update_remote_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        expected_head_snapshot_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "head_snapshot_id": head_snapshot_id,
            "expected_head_snapshot_id": expected_head_snapshot_id,
        }))
    }
}

impl TaskWorkflowLineCloser for FakeLineRemote {
    fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "status": status,
        }))
    }
}

impl TaskWorkflowLineCloser for FakeLineCloser {
    fn close_line(
        &mut self,
        repo_name: &str,
        line_name: &str,
        status: &str,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "line_name": line_name,
            "status": status,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeQueueRemote;

#[derive(Debug, Default)]
struct FakeTaskQueueReader;

#[derive(Debug, Default)]
struct FakeReviewerInboxReader;

#[derive(Debug, Default)]
struct FakeQueueSummaryBundleReader;

impl TaskWorkflowTaskQueueReader for FakeQueueRemote {
    fn read_task_queue(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "status": status,
            "items": [],
        }))
    }
}

impl TaskWorkflowTaskQueueReader for FakeTaskQueueReader {
    fn read_task_queue(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "status": status,
        }))
    }
}

impl TaskWorkflowReviewerInboxReader for FakeReviewerInboxReader {
    fn read_reviewer_inbox(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "reviewer_inbox": true,
        }))
    }
}

impl TaskWorkflowQueueSummaryBundleReader for FakeQueueSummaryBundleReader {
    fn read_queue_summary_bundle(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "status": status,
            "summary": true,
        }))
    }
}

impl TaskWorkflowReviewerInboxReader for FakeQueueRemote {
    fn read_reviewer_inbox(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "reviewer_inbox": true,
        }))
    }
}

impl TaskWorkflowQueueSummaryBundleReader for FakeQueueRemote {
    fn read_queue_summary_bundle(
        &mut self,
        repo_name: &str,
        status: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "status": status,
            "summary": true,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeChangeRemote;

impl TaskWorkflowRemoteChangeCreator for FakeChangeRemote {
    fn create_change(
        &mut self,
        repo_name: &str,
        task_id: &str,
        title: &str,
        base_line: &str,
        change_id: Option<&str>,
        fork_snapshot_id: Option<&str>,
        forked_from_line: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "task_id": task_id,
            "title": title,
            "base_line": base_line,
            "change_id": change_id,
            "fork_snapshot_id": fork_snapshot_id,
            "forked_from_line": forked_from_line,
        }))
    }
}

impl TaskWorkflowRemoteChangeLister for FakeChangeRemote {
    fn list_changes(&mut self, repo_name: &str) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "repo_name": repo_name,
            "change_id": "C-1",
        })])
    }
}

impl TaskWorkflowRemoteChangeDetailReader for FakeChangeRemote {
    fn get_change_detail(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "change_id": change_ref,
            "detail": true,
        }))
    }
}

impl TaskWorkflowRemoteChangeReader for FakeChangeRemote {
    fn get_change(
        &mut self,
        change_ref: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "change_id": change_ref,
        }))
    }
}

impl TaskWorkflowRemoteChangeCloser for FakeChangeRemote {
    fn close_change(
        &mut self,
        change_ref: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "change_id": change_ref,
            "status": status,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeSnapshotRemote;

impl TaskWorkflowZstdPackUploader for FakeSnapshotRemote {}

impl TaskWorkflowSnapshotMetadataReader for FakeSnapshotRemote {
    fn get_remote_snapshot(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
        include_content: bool,
        path: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "snapshot_id": snapshot_id,
            "include_content": include_content,
            "path": path,
        }))
    }
}

impl TaskWorkflowZstdPackReader for FakeSnapshotRemote {
    fn get_remote_zstd_import_manifest(
        &mut self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> TaskWorkflowHttpClientResult<ZstdImportManifestPayload> {
        Ok(ZstdImportManifestPayload {
            contract: "ait.remote_sync.zstd_bulk.import_manifest.v1".to_string(),
            repo_name: repo_name.to_string(),
            snapshot_id: snapshot_id.to_string(),
            snapshots: Vec::new(),
            object_packs: Vec::new(),
            tree_packs: Vec::new(),
            blob_locators: Vec::new(),
            tree_locators: Vec::new(),
            line_update: None,
        })
    }

    fn get_remote_zstd_object_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        Ok(format!("{repo_name}:object:{pack_id}").into_bytes())
    }

    fn get_remote_zstd_tree_pack(
        &mut self,
        repo_name: &str,
        pack_id: &str,
    ) -> TaskWorkflowHttpClientResult<Vec<u8>> {
        Ok(format!("{repo_name}:tree:{pack_id}").into_bytes())
    }
}

impl TaskWorkflowSnapshotExistenceReader for FakeSnapshotRemote {
    fn get_remote_snapshots_existence(
        &mut self,
        repo_name: &str,
        snapshot_ids: &[String],
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({"repo_name": repo_name, "snapshot_ids": snapshot_ids}))
    }
}

#[derive(Debug, Default)]
struct FakeCloseoutRemote {
    closed: bool,
}

#[derive(Debug, Default)]
struct FakeMutationReceiptRemote;

#[derive(Debug, Default)]
struct FakeMutationReceiptBuilderPort;

#[derive(Debug, Default)]
struct FakeActionMutationReceiptsBuilderPort;

#[derive(Debug, Default)]
struct FakePatchsetRemote;

#[derive(Debug, Default)]
struct FakePatchsetListerPort;

#[derive(Debug, Default)]
struct FakePatchsetReaderPort;

#[derive(Debug, Default)]
struct FakePatchsetPublisherPort;

#[derive(Debug, Default)]
struct FakePatchsetSelectorPort;

#[derive(Debug, Default)]
struct FakePatchsetCiRunnerPort;

#[derive(Debug, Default)]
struct FakePatchsetCiStatusReaderPort;

#[derive(Debug, Default)]
struct FakeRepoJobListerPort;

#[derive(Debug, Default)]
struct FakePatchsetCiRemote;

#[derive(Debug, Default)]
struct FakePolicyRemote;

#[derive(Debug, Default)]
struct FakeAttestationRemote;

#[derive(Debug, Default)]
struct FakeLandRemote;

#[derive(Debug, Default)]
struct FakeRemoteTaskCloserPort;

#[derive(Debug, Default)]
struct FakeReviewRemote;

impl TaskWorkflowHttpClientInspector for FakeCloseoutRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        fake_stats(self.closed)
    }
}

impl TaskWorkflowHttpClientCloser for FakeCloseoutRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        self.closed = true;
        fake_stats(true)
    }
}

impl TaskWorkflowMutationReceiptBuilder for FakeCloseoutRemote {
    fn mutation_receipt(
        &self,
        action: &str,
        source_action: &str,
        delivery: &str,
        response_recovery: Option<&JsonValue>,
        result: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "action": action,
            "source_action": source_action,
            "delivery": delivery,
            "response_recovery": response_recovery.cloned().unwrap_or(JsonValue::Null),
            "result": result.cloned().unwrap_or(JsonValue::Null),
        }))
    }
}

impl TaskWorkflowActionMutationReceiptsBuilder for FakeCloseoutRemote {
    fn action_mutation_receipts(
        &self,
        code: &str,
        result: &JsonValue,
    ) -> Result<JsonValue, String> {
        Ok(json!({"code": code, "result": result}))
    }
}

impl TaskWorkflowPatchsetLister for FakeCloseoutRemote {
    fn list_patchsets(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "change_id": change_id,
            "repo_name": repo_name,
            "patchset_id": "RCP-1",
        })])
    }
}

impl TaskWorkflowPatchsetReader for FakeCloseoutRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "change_ref": change_ref,
        }))
    }
}

impl TaskWorkflowPatchsetPublisher for FakeCloseoutRemote {
    fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "author_mode": author_mode,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowPatchsetSelector for FakeCloseoutRemote {
    fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowPatchsetCiRunner for FakeCloseoutRemote {
    fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "trigger": trigger,
            "execution_profile": execution_profile,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowReviewRequester for FakeCloseoutRemote {
    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer_groups": reviewer_groups,
            "note": note,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakeCloseoutRemote {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "recent_limit": recent_limit,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowRepoJobLister for FakeCloseoutRemote {
    fn list_repo_jobs(
        &mut self,
        repo_name: &str,
        state: Option<&str>,
        limit: i64,
        diagnostics: bool,
        stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "state": state,
            "limit": limit,
            "diagnostics": diagnostics,
            "stale_after_seconds": stale_after_seconds,
        }))
    }
}

impl TaskWorkflowReviewRecorder for FakeCloseoutRemote {
    fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "action": action,
            "comment": comment,
            "blocking": blocking,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowReviewLister for FakeCloseoutRemote {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "reviews": [],
        }))
    }
}

impl TaskWorkflowAttestationWriter for FakeCloseoutRemote {
    fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &JsonValue,
        provenance_summary: &JsonValue,
        detail: &JsonValue,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "author_mode": author_mode,
            "evaluation_summary": evaluation_summary,
            "provenance_summary": provenance_summary,
            "detail": detail,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowAttestationReader for FakeCloseoutRemote {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "attestation": true,
        }))
    }
}

impl TaskWorkflowPolicyEvaluator for FakeCloseoutRemote {
    fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "evaluation_state": "pass",
        }))
    }
}

impl TaskWorkflowPolicyReader for FakeCloseoutRemote {
    fn get_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "policy": true,
        }))
    }
}

impl TaskWorkflowPolicyWaiverCreator for FakeCloseoutRemote {
    fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "rule_name": rule_name,
            "reason": reason,
            "expires_at": expires_at,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowLandSubmitter for FakeCloseoutRemote {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "target_line": target_line,
            "mode": mode,
            "repo_name": repo_name,
            "submission_id": "LAND-1",
        }))
    }
}

impl TaskWorkflowLandReader for FakeCloseoutRemote {
    fn get_land(
        &mut self,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": submission_id,
            "repo_name": repo_name,
            "status": "queued",
        }))
    }
}

impl TaskWorkflowLandRetryer for FakeCloseoutRemote {
    fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": submission_id,
            "reason": reason,
            "repo_name": repo_name,
            "retried": true,
        }))
    }
}

impl TaskWorkflowRemoteTaskCloser for FakeCloseoutRemote {
    fn close_task(
        &mut self,
        task_id: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "task_id": task_id,
            "status": status,
            "repo_name": repo_name,
        }))
    }
}

impl TaskWorkflowMutationReceiptBuilder for FakeMutationReceiptRemote {
    fn mutation_receipt(
        &self,
        action: &str,
        source_action: &str,
        delivery: &str,
        response_recovery: Option<&JsonValue>,
        result: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "action": action,
            "source_action": source_action,
            "delivery": delivery,
            "response_recovery": response_recovery.cloned().unwrap_or(JsonValue::Null),
            "result": result.cloned().unwrap_or(JsonValue::Null),
        }))
    }
}

impl TaskWorkflowActionMutationReceiptsBuilder for FakeMutationReceiptRemote {
    fn action_mutation_receipts(
        &self,
        code: &str,
        result: &JsonValue,
    ) -> Result<JsonValue, String> {
        Ok(json!({"code": code, "result": result}))
    }
}

impl TaskWorkflowMutationReceiptBuilder for FakeMutationReceiptBuilderPort {
    fn mutation_receipt(
        &self,
        action: &str,
        source_action: &str,
        delivery: &str,
        response_recovery: Option<&JsonValue>,
        result: Option<&JsonValue>,
    ) -> Result<JsonValue, String> {
        Ok(json!({
            "action": action,
            "source_action": source_action,
            "delivery": delivery,
            "response_recovery": response_recovery.cloned().unwrap_or(JsonValue::Null),
            "result": result.cloned().unwrap_or(JsonValue::Null),
        }))
    }
}

impl TaskWorkflowActionMutationReceiptsBuilder for FakeActionMutationReceiptsBuilderPort {
    fn action_mutation_receipts(
        &self,
        code: &str,
        result: &JsonValue,
    ) -> Result<JsonValue, String> {
        Ok(json!({"code": code, "result": result}))
    }
}

impl TaskWorkflowPatchsetLister for FakePatchsetRemote {
    fn list_patchsets(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "change_id": change_id,
            "repo_name": repo_name,
            "patchset_id": "P-C-1-1",
        })])
    }
}

impl TaskWorkflowPatchsetReader for FakePatchsetRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "change_ref": change_ref,
        }))
    }
}

impl TaskWorkflowPatchsetPublisher for FakePatchsetRemote {
    fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": format!("P-{change_id}-1"),
            "change_id": change_id,
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "author_mode": author_mode,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowPatchsetSelector for FakePatchsetRemote {
    fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "selected": true,
        }))
    }
}

impl TaskWorkflowPatchsetCiRunner for FakePatchsetRemote {
    fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "trigger": trigger,
            "execution_profile": execution_profile,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "queued": true,
        }))
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakePatchsetRemote {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "recent_limit": recent_limit,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "tests_status": "pass",
        }))
    }
}

impl TaskWorkflowPatchsetLister for FakePatchsetListerPort {
    fn list_patchsets(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
        Ok(vec![json!({
            "change_id": change_id,
            "repo_name": repo_name,
            "patchset_id": "P-C-1-1",
        })])
    }
}

impl TaskWorkflowPatchsetReader for FakePatchsetReaderPort {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "change_ref": change_ref,
        }))
    }
}

impl TaskWorkflowPatchsetPublisher for FakePatchsetPublisherPort {
    fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": "P-C-1-1",
            "change_id": change_id,
            "base_snapshot_id": base_snapshot_id,
            "revision_snapshot_id": revision_snapshot_id,
            "summary": summary,
            "author_mode": author_mode,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowPatchsetSelector for FakePatchsetSelectorPort {
    fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "selected": true,
        }))
    }
}

impl TaskWorkflowPatchsetCiRunner for FakePatchsetCiRunnerPort {
    fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "trigger": trigger,
            "execution_profile": execution_profile,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "queued": true,
        }))
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakePatchsetCiStatusReaderPort {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "recent_limit": recent_limit,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "tests_status": "pass",
        }))
    }
}

impl TaskWorkflowPatchsetReader for FakePatchsetCiRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "change_ref": change_ref,
        }))
    }
}

impl TaskWorkflowPatchsetCiStatusReader for FakePatchsetCiRemote {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "recent_limit": recent_limit,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "tests_status": "pass",
        }))
    }
}

impl TaskWorkflowRepoJobLister for FakePatchsetCiRemote {
    fn list_repo_jobs(
        &mut self,
        repo_name: &str,
        state: Option<&str>,
        limit: i64,
        diagnostics: bool,
        stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "state": state,
            "limit": limit,
            "diagnostics": diagnostics,
            "stale_after_seconds": stale_after_seconds,
            "jobs": [],
        }))
    }
}

impl TaskWorkflowRepoJobLister for FakeRepoJobListerPort {
    fn list_repo_jobs(
        &mut self,
        repo_name: &str,
        state: Option<&str>,
        limit: i64,
        diagnostics: bool,
        stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "repo_name": repo_name,
            "state": state,
            "limit": limit,
            "diagnostics": diagnostics,
            "stale_after_seconds": stale_after_seconds,
            "jobs": [],
        }))
    }
}

impl TaskWorkflowPolicyEvaluator for FakePolicyRemote {
    fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "evaluation_state": "pass",
        }))
    }
}

impl TaskWorkflowPolicyReader for FakePolicyRemote {
    fn get_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "policy": true,
        }))
    }
}

impl TaskWorkflowPolicyWaiverCreator for FakePolicyRemote {
    fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "rule_name": rule_name,
            "reason": reason,
            "expires_at": expires_at,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "waived": true,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeAttestationWriterPort;

#[derive(Debug, Default)]
struct FakeAttestationReaderPort;

#[derive(Debug, Default)]
struct FakePolicyEvaluatorPort;

#[derive(Debug, Default)]
struct FakePolicyReaderPort;

#[derive(Debug, Default)]
struct FakePolicyWaiverCreatorPort;

#[derive(Debug, Default)]
struct FakeLandSubmitterPort;

#[derive(Debug, Default)]
struct FakeLandReaderPort;

#[derive(Debug, Default)]
struct FakeLandRetryerPort;

impl TaskWorkflowAttestationWriter for FakeAttestationRemote {
    fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &JsonValue,
        provenance_summary: &JsonValue,
        detail: &JsonValue,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "author_mode": author_mode,
            "evaluation_summary": evaluation_summary,
            "provenance_summary": provenance_summary,
            "detail": detail,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowAttestationReader for FakeAttestationRemote {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "attestation": true,
        }))
    }
}

impl TaskWorkflowAttestationWriter for FakeAttestationWriterPort {
    fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &JsonValue,
        provenance_summary: &JsonValue,
        detail: &JsonValue,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "author_mode": author_mode,
            "evaluation_summary": evaluation_summary,
            "provenance_summary": provenance_summary,
            "detail": detail,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowAttestationReader for FakeAttestationReaderPort {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "attestation": true,
        }))
    }
}

impl TaskWorkflowPolicyEvaluator for FakePolicyEvaluatorPort {
    fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "evaluation_state": "pass",
        }))
    }
}

impl TaskWorkflowPolicyReader for FakePolicyReaderPort {
    fn get_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "policy": true,
        }))
    }
}

impl TaskWorkflowPolicyWaiverCreator for FakePolicyWaiverCreatorPort {
    fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "patchset_id": patchset_id,
            "rule_name": rule_name,
            "reason": reason,
            "expires_at": expires_at,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "waived": true,
        }))
    }
}

impl TaskWorkflowLandSubmitter for FakeLandRemote {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "target_line": target_line,
            "mode": mode,
            "repo_name": repo_name,
            "submission_id": "LAND-1",
        }))
    }
}

impl TaskWorkflowLandReader for FakeLandRemote {
    fn get_land(
        &mut self,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": submission_id,
            "repo_name": repo_name,
            "status": "queued",
        }))
    }
}

impl TaskWorkflowLandRetryer for FakeLandRemote {
    fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": submission_id,
            "reason": reason,
            "repo_name": repo_name,
            "retried": true,
        }))
    }
}

impl TaskWorkflowLandSubmitter for FakeLandSubmitterPort {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "target_line": target_line,
            "mode": mode,
            "repo_name": repo_name,
            "submission_id": "LAND-1",
        }))
    }
}

impl TaskWorkflowLandReader for FakeLandReaderPort {
    fn get_land(
        &mut self,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": submission_id,
            "repo_name": repo_name,
            "status": "queued",
        }))
    }
}

impl TaskWorkflowLandRetryer for FakeLandRetryerPort {
    fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "submission_id": submission_id,
            "reason": reason,
            "repo_name": repo_name,
            "retried": true,
        }))
    }
}

impl TaskWorkflowRemoteTaskCloser for FakeRemoteTaskCloserPort {
    fn close_task(
        &mut self,
        task_id: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "task_id": task_id,
            "status": status,
            "repo_name": repo_name,
        }))
    }
}

#[derive(Debug, Default)]
struct FakeReviewRequesterPort;

#[derive(Debug, Default)]
struct FakeReviewRecorderPort;

#[derive(Debug, Default)]
struct FakeReviewListerPort;

impl TaskWorkflowReviewRequester for FakeReviewRemote {
    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer_groups": reviewer_groups,
            "note": note,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "requested": true,
        }))
    }
}

impl TaskWorkflowReviewRecorder for FakeReviewRemote {
    fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "action": action,
            "comment": comment,
            "blocking": blocking,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowReviewLister for FakeReviewRemote {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "reviews": [{"action": "approve"}],
        }))
    }
}

impl TaskWorkflowReviewRequester for FakeReviewRequesterPort {
    fn request_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer_groups: &[String],
        note: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer_groups": reviewer_groups,
            "note": note,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "requested": true,
        }))
    }
}

impl TaskWorkflowReviewRecorder for FakeReviewRecorderPort {
    fn record_review(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        reviewer: &str,
        action: &str,
        comment: Option<&str>,
        blocking: bool,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "patchset_id": patchset_id,
            "reviewer": reviewer,
            "action": action,
            "comment": comment,
            "blocking": blocking,
            "repo_name": repo_name,
            "exact_id": exact_id,
        }))
    }
}

impl TaskWorkflowReviewLister for FakeReviewListerPort {
    fn list_reviews(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<JsonValue> {
        Ok(json!({
            "change_id": change_id,
            "repo_name": repo_name,
            "exact_id": exact_id,
            "reviews": [{"action": "approve"}],
        }))
    }
}

mod attestation;
mod change;
mod client;
mod closeout;
mod history_promotion;
mod land;
mod line;
mod mutation_receipts;
mod patchset;
mod policy;
mod queue;
mod repository;
mod review;
mod snapshot;
mod task;
