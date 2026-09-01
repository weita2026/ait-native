use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::*;
use crate::json_support::{json, JsonValue};

fn config() -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
        headers: BTreeMap::from([(String::from("X-Test"), String::from("yes"))]),
        default_timeout_ms: 12_345,
        retry_attempts: 2,
        retry_backoff_ms: 7,
        pool_max_idle_per_host: 3,
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn spawn_scripted_http_server(
    responses: Vec<(u16, &'static str)>,
) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind scripted HTTP server");
    let address = listener.local_addr().expect("scripted HTTP address");
    let handle = thread::spawn(move || {
        let mut methods = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().expect("accept scripted request");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set scripted request timeout");
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let read = stream.read(&mut buffer).expect("read scripted request");
                assert!(read > 0, "scripted request ended before headers");
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = find_header_end(&request) else {
                    continue;
                };
                let header_text = String::from_utf8_lossy(&request[..header_end]);
                let content_length = header_text
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("content length"))
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + content_length {
                    break;
                }
            }
            let request_text = String::from_utf8_lossy(&request);
            methods.push(
                request_text
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            );
            let reason = match status {
                200 => "OK",
                503 => "Service Unavailable",
                _ => "Response",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write scripted response");
        }
        methods
    });
    (format!("http://{address}"), handle)
}

fn spawn_split_body_http_server(
    advertised_length: usize,
    body: &'static [u8],
    body_delay: Duration,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind split-body HTTP server");
    let address = listener.local_addr().expect("split-body HTTP address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept split-body request");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set split-body request timeout");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        while find_header_end(&request).is_none() {
            let read = stream.read(&mut buffer).expect("read split-body request");
            assert!(read > 0, "split-body request ended before headers");
            request.extend_from_slice(&buffer[..read]);
        }
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {advertised_length}\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(headers.as_bytes())
            .expect("write split-body response headers");
        stream.flush().expect("flush split-body response headers");
        thread::sleep(body_delay);
        let _ = stream.write_all(body);
    });
    (format!("http://{address}"), handle)
}

fn spawn_concurrent_bytes_server(
    responses: BTreeMap<String, (u16, String)>,
) -> (String, Arc<AtomicUsize>, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind concurrent HTTP server");
    let address = listener.local_addr().expect("concurrent HTTP address");
    let maximum_active = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let maximum_active_for_server = Arc::clone(&maximum_active);
    let handle = thread::spawn(move || {
        let responses = Arc::new(responses);
        let mut handlers = Vec::with_capacity(responses.len());
        for _ in 0..responses.len() {
            let (mut stream, _) = listener.accept().expect("accept concurrent request");
            let active = Arc::clone(&active);
            let maximum_active = Arc::clone(&maximum_active_for_server);
            let responses = Arc::clone(&responses);
            handlers.push(thread::spawn(move || {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum_active.fetch_max(current, Ordering::SeqCst);
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("set concurrent request timeout");
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let read = stream.read(&mut buffer).expect("read concurrent request");
                    assert!(read > 0, "concurrent request ended before headers");
                    request.extend_from_slice(&buffer[..read]);
                    if find_header_end(&request).is_some() {
                        break;
                    }
                }
                let request_text = String::from_utf8_lossy(&request);
                let path = request_text
                    .split_whitespace()
                    .nth(1)
                    .expect("concurrent request path")
                    .to_string();
                thread::sleep(Duration::from_millis(100));
                let (status, body) = responses.get(&path).expect("scripted concurrent path");
                let reason = if *status == 200 { "OK" } else { "Response" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write concurrent response");
                active.fetch_sub(1, Ordering::SeqCst);
                path
            }));
        }
        handlers
            .into_iter()
            .map(|handler| handler.join().expect("join concurrent handler"))
            .collect()
    });
    (format!("http://{address}"), maximum_active, handle)
}

fn local_http_config(base_url: String) -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url,
        repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
        default_timeout_ms: 5_000,
        ..PlanHttpClientConfig::default()
    }
}

fn retryable_busy_error(method: &str, status: u16, detail: &str) -> PlanHttpClientError {
    PlanHttpClientError::RemoteResponse {
        method: method.to_string(),
        url: "http://127.0.0.1/test".to_string(),
        status,
        detail: detail.to_string(),
    }
}

#[test]
fn remote_response_preserves_status_detail_and_exact_retryable_busy_classification() {
    let busy = retryable_busy_error(
        "GET",
        503,
        "ait.binary-db.error.v1|retryable_busy|writer_scope=ServerLand",
    );
    assert_eq!(busy.remote_status(), Some(503));
    assert_eq!(
        busy.remote_detail(),
        Some("ait.binary-db.error.v1|retryable_busy|writer_scope=ServerLand")
    );
    assert!(busy.is_retryable_busy());
    assert_eq!(
        busy.to_string(),
        "GET http://127.0.0.1/test failed: 503 ait.binary-db.error.v1|retryable_busy|writer_scope=ServerLand"
    );

    assert!(
        !retryable_busy_error("GET", 409, "ait.binary-db.error.v1|retryable_busy|writer")
            .is_retryable_busy()
    );
    assert!(
        !retryable_busy_error("GET", 503, "ait.binary-db.error.v1|integrity|broken")
            .is_retryable_busy()
    );
    assert!(!PlanHttpClientError::Remote(
        "503 ait.binary-db.error.v1|retryable_busy|flattened".to_string()
    )
    .is_retryable_busy());

    assert!(transport::retryable_busy_read_delay("GET", &busy, 0).is_some());
    assert!(transport::retryable_busy_read_delay(
        "GET",
        &busy,
        transport::RETRYABLE_BUSY_READ_MAX_RETRIES - 1
    )
    .is_some());
    assert!(transport::retryable_busy_read_delay(
        "GET",
        &busy,
        transport::RETRYABLE_BUSY_READ_MAX_RETRIES
    )
    .is_none());
    assert!(transport::retryable_busy_read_delay("POST", &busy, 0).is_none());
}

#[test]
fn get_retries_structured_retryable_busy_until_the_read_converges() {
    let busy_body = r#"{"detail":"ait.binary-db.error.v1|retryable_busy|writer_scope=ServerLand"}"#;
    let (base_url, server) = spawn_scripted_http_server(vec![
        (503, busy_body),
        (503, busy_body),
        (200, r#"{"task_id":"RCT-1","status":"completed"}"#),
    ]);
    let mut manager = PlanHttpClientManager::new(local_http_config(base_url)).unwrap();

    let task = manager
        .get_task("RCT-1", Some("ait-core"))
        .expect("retryable-busy GET should converge");

    assert_eq!(task["task_id"], json!("RCT-1"));
    assert_eq!(task["status"], json!("completed"));
    assert_eq!(manager.inspect().request_count, 3);
    assert_eq!(manager.inspect().retry_count, 2);
    assert_eq!(server.join().expect("join scripted GET server"), ["GET"; 3]);
}

#[test]
fn mutation_never_retries_a_structured_retryable_busy_response() {
    let busy_body =
        r#"{"detail":"ait.binary-db.error.v1|retryable_busy|writer_scope=ServerWorkflow"}"#;
    let (base_url, server) = spawn_scripted_http_server(vec![(503, busy_body)]);
    let mut manager = PlanHttpClientManager::new(local_http_config(base_url)).unwrap();

    let err = manager
        .create_task("ait-core", "title", "intent", None, None, None, None)
        .expect_err("retryable-busy POST must not be replayed");

    assert!(err.is_retryable_busy());
    assert_eq!(manager.inspect().request_count, 1);
    assert_eq!(manager.inspect().retry_count, 0);
    assert_eq!(server.join().expect("join scripted POST server"), ["POST"]);
}

#[test]
fn response_body_timeout_preserves_canonical_transport_timeout_classification() {
    let body = br#"{"status":"completed"}"#;
    let (base_url, server) =
        spawn_split_body_http_server(body.len(), body, Duration::from_millis(750));
    let url = format!("{base_url}/body-timeout");
    let mut manager = PlanHttpClientManager::new(local_http_config(base_url)).unwrap();

    let error = manager
        .execute_json(PlanHttpRequestSpec {
            method: "POST".to_string(),
            path: "/body-timeout".to_string(),
            url: url.clone(),
            query_pairs: Vec::new(),
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: 250,
        })
        .expect_err("response body must exceed the request deadline");

    assert!(error.is_transport_timeout());
    assert_eq!(
        error,
        PlanHttpClientError::Transport(format!("POST {url} failed: timed out"))
    );
    server.join().expect("join split-body timeout server");
}

#[test]
fn truncated_response_body_retains_context_without_timeout_classification() {
    let (base_url, server) = spawn_split_body_http_server(2, b"{", Duration::ZERO);
    let url = format!("{base_url}/truncated-body");
    let mut manager = PlanHttpClientManager::new(local_http_config(base_url)).unwrap();

    let error = manager
        .execute_json(PlanHttpRequestSpec {
            method: "POST".to_string(),
            path: "/truncated-body".to_string(),
            url: url.clone(),
            query_pairs: Vec::new(),
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: 5_000,
        })
        .expect_err("truncated response body must fail");

    let message = error.to_string();
    assert!(!error.is_transport_timeout());
    assert!(matches!(error, PlanHttpClientError::Transport(_)));
    assert!(message.starts_with(&format!("POST {url} failed: ")));
    assert!(!message.ends_with("timed out"));
    server.join().expect("join truncated-body server");
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SubstitutePlanHttpStats {
    request_count: usize,
    closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SubstitutePlanHttpError {
    Rejected,
}

#[derive(Default)]
struct SubstitutePlanHttpTransport {
    request_count: usize,
    closed: bool,
    json_paths: Vec<String>,
    bytes_paths: Vec<String>,
}

impl PlanHttpClientLifecycle for SubstitutePlanHttpTransport {
    type Stats = SubstitutePlanHttpStats;

    fn inspect(&self) -> Self::Stats {
        SubstitutePlanHttpStats {
            request_count: self.request_count,
            closed: self.closed,
        }
    }

    fn close(&mut self) -> Self::Stats {
        self.closed = true;
        self.inspect()
    }
}

impl PlanHttpTransport for SubstitutePlanHttpTransport {
    type Error = SubstitutePlanHttpError;

    fn execute_json(
        &mut self,
        spec: PlanHttpRequestSpec,
    ) -> Result<Option<JsonValue>, Self::Error> {
        if spec.path == "/reject" {
            return Err(SubstitutePlanHttpError::Rejected);
        }
        self.request_count += 1;
        self.json_paths.push(spec.path.clone());
        Ok(Some(json!({
            "transport": "substitute",
            "method": spec.method,
            "path": spec.path,
            "body": spec.body,
        })))
    }

    fn execute_bytes(&mut self, spec: PlanHttpBytesRequestSpec) -> Result<Vec<u8>, Self::Error> {
        if spec.path == "/reject" {
            return Err(SubstitutePlanHttpError::Rejected);
        }
        self.request_count += 1;
        self.bytes_paths.push(spec.path.clone());
        let body = spec.body.unwrap_or_default();
        Ok(format!("bytes:{}:", spec.path)
            .into_bytes()
            .into_iter()
            .chain(body)
            .collect())
    }
}

fn test_json_request_spec(path: &str) -> PlanHttpRequestSpec {
    PlanHttpRequestSpec {
        method: "POST".to_string(),
        path: path.to_string(),
        url: format!("https://example.test{path}"),
        query_pairs: Vec::new(),
        headers: BTreeMap::new(),
        body: Some(json!({"hello": "world"})),
        timeout_ms: 10,
    }
}

fn test_bytes_request_spec(path: &str) -> PlanHttpBytesRequestSpec {
    PlanHttpBytesRequestSpec {
        method: "PUT".to_string(),
        path: path.to_string(),
        url: format!("https://example.test{path}"),
        query_pairs: Vec::new(),
        headers: BTreeMap::new(),
        body: Some(b"payload".to_vec()),
        timeout_ms: 20,
    }
}

#[test]
fn plan_http_transport_bound_helpers_accept_substitute_transport() {
    let mut transport = SubstitutePlanHttpTransport::default();
    let lifecycle: &dyn PlanHttpClientLifecycle<Stats = SubstitutePlanHttpStats> = &transport;
    assert_eq!(
        inspect_with_plan_http_client_lifecycle(lifecycle),
        SubstitutePlanHttpStats {
            request_count: 0,
            closed: false,
        }
    );

    let transport_port: &mut dyn PlanHttpTransport<Error = SubstitutePlanHttpError> =
        &mut transport;
    let payload =
        execute_json_with_plan_http_transport(transport_port, test_json_request_spec("/json"))
            .expect("json response")
            .expect("json payload");
    assert_eq!(payload["transport"], "substitute");
    assert_eq!(payload["method"], "POST");
    assert_eq!(payload["path"], "/json");
    assert_eq!(payload["body"]["hello"], "world");

    let bytes =
        execute_bytes_with_plan_http_transport(transport_port, test_bytes_request_spec("/bytes"))
            .expect("bytes response");
    assert_eq!(bytes, b"bytes:/bytes:payload");

    assert_eq!(
        inspect_with_plan_http_client_lifecycle(&transport),
        SubstitutePlanHttpStats {
            request_count: 2,
            closed: false,
        }
    );
    assert_eq!(transport.json_paths, vec!["/json".to_string()]);
    assert_eq!(transport.bytes_paths, vec!["/bytes".to_string()]);

    let lifecycle: &mut dyn PlanHttpClientLifecycle<Stats = SubstitutePlanHttpStats> =
        &mut transport;
    assert_eq!(
        close_with_plan_http_client_lifecycle(lifecycle),
        SubstitutePlanHttpStats {
            request_count: 2,
            closed: true,
        }
    );
}

#[test]
fn repository_registration_uses_only_frozen_fixed_fields() {
    let request =
        build_ensure_repository_request_spec(&config(), "ait-runner", "main", None, Some("R"))
            .expect("build numeric Repository registration");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/native/repository-authorities");
    assert_eq!(
        request.body,
        Some(json!({
            "repository_name": "ait-runner",
            "namespace": "R",
            "policy_flags": 0b1000_0011,
        }))
    );

    let custom = build_ensure_repository_request_spec(
        &config(),
        "duplicate-name",
        "main",
        Some(&json!({
            "policy_id": "prototype",
            "version": 1,
            "defaults": {
                "require_attestation": false,
                "require_tests": true,
                "require_lint": true,
            },
            "class_overrides": [],
        })),
        Some("R2"),
    )
    .expect("build exact custom prototype policy registration");
    assert_eq!(custom.body.as_ref().unwrap()["policy_flags"], 0b0000_0110);

    assert!(build_ensure_repository_request_spec(
        &config(),
        "ait-runner",
        "trunk",
        None,
        Some("R")
    )
    .is_err());
    assert!(build_ensure_repository_request_spec(
        &config(),
        "ait-runner",
        "main",
        None,
        Some("RUN")
    )
    .is_err());
    assert!(build_ensure_repository_request_spec(
        &config(),
        "ait-runner",
        "main",
        Some(&json!({
            "policy_id": "prototype",
            "defaults": {"unknown": true},
        })),
        Some("")
    )
    .is_err());
}

#[test]
fn line_lifecycle_request_specs_freeze_cas_and_idempotency_contract() {
    let rename = build_rename_remote_line_request_spec(
        &config(),
        "repo name",
        "topic/old",
        "topic/new",
        "LNE-0000002A",
        Some("SNP-HEAD"),
        "idem-rename-1",
    )
    .expect("rename request spec");
    assert_eq!(rename.method, "POST");
    assert_eq!(
        rename.path,
        "/v1/native/repository-authorities/7/lines/topic%2Fold:rename"
    );
    assert_eq!(
        rename.body.as_ref().unwrap()["contract"],
        "line-lifecycle/v1"
    );
    assert_eq!(rename.body.as_ref().unwrap()["new_line_name"], "topic/new");
    assert_eq!(
        rename.body.as_ref().unwrap()["expected_line_id"],
        "LNE-0000002A"
    );
    assert_eq!(
        rename.body.as_ref().unwrap()["expected_head_snapshot_id"],
        "SNP-HEAD"
    );
    assert_eq!(
        rename.body.as_ref().unwrap()["idempotency_key"],
        "idem-rename-1"
    );

    let delete = build_delete_remote_line_request_spec(
        &config(),
        "repo",
        "topic/dead",
        "LNE-0000002A",
        None,
        "idem-delete-1",
    )
    .expect("delete request spec");
    assert_eq!(delete.method, "POST");
    assert_eq!(
        delete.path,
        "/v1/native/repository-authorities/7/lines/topic%2Fdead:delete"
    );
    assert!(delete.body.as_ref().unwrap()["expected_head_snapshot_id"].is_null());
    assert_eq!(
        delete.body.as_ref().unwrap()["idempotency_key"],
        "idem-delete-1"
    );

    assert!(build_rename_remote_line_request_spec(
        &config(),
        "repo",
        "old",
        "new",
        " ",
        None,
        "idem"
    )
    .unwrap_err()
    .to_string()
    .contains("expected_line_id"));
    assert!(
        build_delete_remote_line_request_spec(&config(), "repo", "line", "LNE-1", None, " ")
            .unwrap_err()
            .to_string()
            .contains("idempotency_key")
    );
}

#[test]
fn list_plans_request_spec_normalizes_optional_query() {
    let spec =
        build_list_plans_request_spec(&config(), "housekeeper", Some("  docs/sprints/demo.md  "))
            .unwrap();
    assert_eq!(spec.method, "GET");
    assert_eq!(spec.path, "/v1/native/repository-authorities/7/sprints");
    assert_eq!(
            spec.url,
            "https://example.test/v1/native/repository-authorities/7/sprints?artifact_path=docs%2Fsprints%2Fdemo.md"
        );
    assert_eq!(
        spec.query_pairs,
        vec![(
            "artifact_path".to_string(),
            "docs/sprints/demo.md".to_string()
        )]
    );
    assert_eq!(spec.timeout_ms, 12_345);
    assert_eq!(
        spec.headers.get("Accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(spec.headers.get("X-Test").map(String::as_str), Some("yes"));
    assert!(spec.body.is_none());
}

#[test]
fn atomic_task_start_request_spec_preserves_one_repository_scoped_payload() {
    let payload = json!({
        "contract": "task-start-atomic/v1",
        "idempotency_key": "task-start-1",
        "plan_item_ref": "card/item",
        "plan": {"action": "existing", "plan_id": "PR-1", "plan_revision_id": "plan-revision:2"},
        "task": {"title": "Atomic task", "intent": "Use one mutation"},
        "change": {"title": "Atomic change", "base_line": "main"}
    });
    let spec = build_start_plan_bound_task_request_spec(&config(), " repo/a ", &payload).unwrap();

    assert_eq!(spec.method, "POST");
    assert_eq!(spec.path, "/v1/native/repository-authorities/7/task-start");
    assert_eq!(spec.body.as_ref(), Some(&payload));
    assert!(build_start_plan_bound_task_request_spec(
        &config(),
        "repo",
        &json!({"contract": "task-start-atomic/v0"})
    )
    .unwrap_err()
    .to_string()
    .contains("task-start-atomic/v1"));
    assert!(build_start_plan_bound_task_request_spec(
        &config(),
        "repo",
        &JsonValue::Array(Vec::new())
    )
    .unwrap_err()
    .to_string()
    .contains("must be an object"));
}

#[test]
fn zstd_download_request_specs_use_download_routes_and_media_types() {
    let object_spec =
        build_get_remote_zstd_object_pack_request_spec(&config(), " repo-a ", " OPK-1 ").unwrap();
    assert_eq!(object_spec.method, "GET");
    assert_eq!(
        object_spec.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/object-packs/OPK-1"
    );
    assert_eq!(
        object_spec.headers.get("Accept").map(String::as_str),
        Some(ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE)
    );
    assert_eq!(object_spec.timeout_ms, 900_000);
    assert!(!object_spec.headers.contains_key("Content-Type"));
    assert!(object_spec.body.is_none());

    let tree_spec =
        build_get_remote_zstd_tree_pack_request_spec(&config(), " repo-a ", " TPK-1 ").unwrap();
    assert_eq!(tree_spec.method, "GET");
    assert_eq!(
        tree_spec.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/tree-packs/TPK-1"
    );
    assert_eq!(
        tree_spec.headers.get("Accept").map(String::as_str),
        Some(ZSTD_BULK_TREE_PACK_MEDIA_TYPE)
    );
    assert_eq!(tree_spec.timeout_ms, 900_000);
    assert!(!tree_spec.headers.contains_key("Content-Type"));
    assert!(tree_spec.body.is_none());

    let manifest_spec =
        build_get_remote_zstd_import_manifest_request_spec(&config(), " repo-a ", " SNP-1 ")
            .unwrap();
    assert_eq!(manifest_spec.method, "GET");
    assert_eq!(
        manifest_spec.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/import-manifests/SNP-1"
    );
    assert_eq!(
        manifest_spec.headers.get("Accept").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(manifest_spec.timeout_ms, 900_000);
    assert!(manifest_spec.body.is_none());

    let pull_manifest_spec = build_get_remote_zstd_pull_manifest_request_spec(
        &config(),
        " repo-a ",
        &ZstdPullManifestRequest {
            contract: ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME.to_string(),
            head_snapshot_id: "SNP-2".to_string(),
            have_snapshot_ids: vec!["SNP-1".to_string()],
        },
    )
    .unwrap();
    assert_eq!(pull_manifest_spec.method, "POST");
    assert_eq!(
        pull_manifest_spec.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/pull-manifests"
    );
    assert_eq!(
        pull_manifest_spec
            .body
            .as_ref()
            .and_then(|body| body["head_snapshot_id"].as_str()),
        Some("SNP-2")
    );
}

#[test]
fn zstd_bulk_upload_and_commit_specs_use_extended_timeout() {
    let plan = build_plan_remote_zstd_bulk_request_spec(
        &config(),
        " repo-a ",
        &json!({
            "snapshot_ids": ["SNP-1"],
            "object_packs": [],
            "tree_packs": []
        }),
    )
    .unwrap();
    assert_eq!(plan.method, "POST");
    assert_eq!(
        plan.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/plan"
    );
    assert_eq!(plan.timeout_ms, 900_000);

    let object_upload = build_put_remote_zstd_object_pack_request_spec(
        &config(),
        " repo-a ",
        " OPK-1 ",
        b"object pack",
    )
    .unwrap();
    assert_eq!(object_upload.method, "PUT");
    assert_eq!(
        object_upload.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/object-packs/OPK-1"
    );
    assert_eq!(object_upload.timeout_ms, 900_000);

    let tree_upload = build_put_remote_zstd_tree_pack_request_spec(
        &config(),
        " repo-a ",
        " TPK-1 ",
        b"tree pack",
    )
    .unwrap();
    assert_eq!(tree_upload.method, "PUT");
    assert_eq!(
        tree_upload.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/tree-packs/TPK-1"
    );
    assert_eq!(tree_upload.timeout_ms, 900_000);

    let commit = build_commit_remote_zstd_bulk_request_spec(
        &config(),
        " repo-a ",
        &json!({
            "contract": "ait.remote_sync.zstd_bulk.commit.v1",
            "object_packs": [],
            "tree_packs": [],
            "blob_locators": [],
            "tree_locators": [],
            "snapshots": []
        }),
    )
    .unwrap();
    assert_eq!(commit.method, "POST");
    assert_eq!(
        commit.path,
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
    );
    assert_eq!(commit.timeout_ms, 900_000);
}

#[test]
fn auth_request_specs_match_native_auth_contract() {
    let whoami = build_auth_whoami_request_spec(&config(), Some(" housekeeper ")).unwrap();
    assert_eq!(whoami.method, "GET");
    assert_eq!(whoami.path, "/v1/native/auth/whoami");
    assert_eq!(
        whoami.url,
        "https://example.test/v1/native/auth/whoami?repo_name=housekeeper"
    );
    assert!(whoami.body.is_none());

    let roles = vec![
        " repo_owner ".to_string(),
        "".to_string(),
        "repo_reviewer".to_string(),
    ];
    let grant = build_grant_role_bindings_request_spec(
        &config(),
        "house/keeper",
        "  dev@example.com  ",
        &roles,
    )
    .unwrap();
    assert_eq!(grant.method, "POST");
    assert_eq!(
        grant.path,
        "/v1/native/admin/repositories/house%2Fkeeper/bindings"
    );
    let body = grant.body.unwrap();
    assert_eq!(body["actor_identity"], "dev@example.com");
    assert_eq!(
        body["roles"],
        JsonValue::Array(vec![
            JsonValue::String("repo_owner".to_string()),
            JsonValue::String("repo_reviewer".to_string()),
        ])
    );

    let list = build_list_role_bindings_request_spec(&config(), "house keeper").unwrap();
    assert_eq!(list.method, "GET");
    assert_eq!(
        list.path,
        "/v1/native/admin/repositories/house%20keeper/bindings"
    );
    assert!(list.body.is_none());
}

#[test]
fn repo_operational_specs_use_numeric_authority() {
    let spec = build_get_repository_storage_request_spec(&config(), "repo with/slash").unwrap();
    assert_eq!(spec.method, "GET");
    assert_eq!(
        spec.path,
        "/v1/native/admin/repositories/repo%20with%2Fslash/storage"
    );
    assert_eq!(
        spec.url,
        "https://example.test/v1/native/admin/repositories/repo%20with%2Fslash/storage"
    );
    assert!(spec.body.is_none());

    let spec = build_get_server_metrics_request_spec(&config(), 10, 300).unwrap();
    assert_eq!(spec.method, "GET");
    assert_eq!(spec.path, "/v1/native/admin/metrics");
    assert_eq!(
        spec.url,
        "https://example.test/v1/native/admin/metrics?recent_jobs_limit=10&stale_after_seconds=300"
    );
    assert!(spec.body.is_none());

    let spec = build_get_server_readiness_request_spec(&config(), 10, 300).unwrap();
    assert_eq!(spec.method, "GET");
    assert_eq!(spec.path, "/v1/native/admin/readiness");
    assert_eq!(
            spec.url,
            "https://example.test/v1/native/admin/readiness?recent_jobs_limit=10&stale_after_seconds=300"
        );
    assert!(spec.body.is_none());
}

#[test]
fn worker_job_list_request_spec_enforces_the_server_limit_contract() {
    let repository_index = crate::server_operational::RepositoryIndex::new(7);
    let minimum = build_list_worker_jobs_request_spec(
        &config(),
        repository_index,
        Some(4),
        crate::server_operational::WORKER_JOB_LIST_LIMIT_MIN,
    )
    .expect("minimum Worker Job list limit");
    assert_eq!(
        minimum.url,
        "https://example.test/v1/native/repository-authorities/7/worker-jobs?limit=1&state_kind=4"
    );

    let maximum = build_list_worker_jobs_request_spec(
        &config(),
        repository_index,
        None,
        crate::server_operational::WORKER_JOB_LIST_LIMIT_MAX,
    )
    .expect("maximum Worker Job list limit");
    assert_eq!(
        maximum.url,
        "https://example.test/v1/native/repository-authorities/7/worker-jobs?limit=1000"
    );

    for invalid in [0, crate::server_operational::WORKER_JOB_LIST_LIMIT_MAX + 1] {
        let error = build_list_worker_jobs_request_spec(&config(), repository_index, None, invalid)
            .expect_err("out-of-range Worker Job list limit must fail before transport");
        assert!(
            error
                .to_string()
                .contains("Worker Job list limit must be between 1 and 1000"),
            "{error}"
        );
    }
}

#[test]
fn patchset_run_ci_request_spec_carries_optional_execution_profile() {
    let spec = build_run_patchset_ci_request_spec(
        &config(),
        " RP-1 ",
        " workflow_ready_apply ",
        Some(" workflow_ready_foreground "),
    )
    .unwrap();
    assert_eq!(spec.method, "POST");
    assert_eq!(
        spec.path,
        "/v1/native/repository-authorities/7/patchsets/RP-1:runCi"
    );
    let body = spec.body.expect("request body");
    assert_eq!(body["trigger"], "workflow_ready_apply");
    assert_eq!(body["execution_profile"], "workflow_ready_foreground");

    let spec =
        build_run_patchset_ci_request_spec(&config(), "RP-1", "manual_rerun", Some(" ")).unwrap();
    let body = spec.body.expect("request body");
    assert_eq!(body["trigger"], "manual_rerun");
    assert!(body.get("execution_profile").is_none());
}

#[test]
fn patchset_ci_request_specs_use_the_configured_repository_authority_id() {
    let mut scoped = config();
    scoped.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));

    let run = build_run_patchset_ci_request_spec(
        &scoped,
        "P-RCT-1/C-01-1",
        "workflow_ready_apply",
        Some("workflow_ready_foreground"),
    )
    .unwrap();
    assert_eq!(
        run.path,
        "/v1/native/repository-authorities/9/patchsets/P-RCT-1%2FC-01-1:runCi"
    );

    let status = build_read_patchset_ci_status_request_spec(&scoped, "P-RCT-1/C-01-1", 10).unwrap();
    assert_eq!(
        status.path,
        "/v1/native/repository-authorities/9/read/patchsets/P-RCT-1%2FC-01-1/ci-status"
    );

    let readiness =
        build_read_patchset_ci_readiness_request_spec(&scoped, "P-RCT-1/C-01-1", 200).unwrap();
    assert_eq!(readiness.path, status.path);
    assert_eq!(
        readiness.query_pairs,
        vec![
            ("recent_limit".to_string(), "20".to_string()),
            ("projection".to_string(), "readiness".to_string()),
        ]
    );
}

#[test]
fn workflow_short_ids_are_encoded_as_single_http_path_segments() {
    let config = config();
    let task_id = "remote/RCT-1000";
    let change_id = "RCT-1000/C-01";
    let patchset_id = "P-RCT-1000/C-01-1";
    let submission_id = "L-RCT-1000/C-01-1";

    assert_eq!(
        build_get_task_request_spec(&config, task_id, None)
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/tasks/remote%2FRCT-1000"
    );
    assert_eq!(
        build_read_task_audit_request_spec(&config, "ait-core", task_id, "main")
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/read/tasks/remote%2FRCT-1000/audit"
    );
    assert_eq!(
        build_get_change_detail_request_spec(&config, change_id, None)
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01"
    );
    assert_eq!(
        build_close_change_request_spec(&config, change_id, "archived")
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01:close"
    );
    assert_eq!(
        build_publish_patchset_request_spec(
            &config,
            change_id,
            "SNP-BASE",
            "SNP-REVISION",
            "summary",
            "agent",
        )
        .unwrap()
        .path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01/patchsets"
    );
    assert_eq!(
        build_list_patchsets_request_spec(&config, change_id, Some("ait-core"))
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01/patchsets"
    );
    assert_eq!(
        build_select_patchset_request_spec(&config, change_id, patchset_id)
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01:selectPatchset"
    );
    assert_eq!(
        build_request_review_request_spec(&config, change_id, patchset_id, &[], None)
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01:requestReview"
    );
    assert_eq!(
        build_record_review_request_spec(
            &config,
            change_id,
            patchset_id,
            "reviewer",
            "approve",
            None,
            false,
        )
        .unwrap()
        .path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01/reviews"
    );
    let list_reviews = build_list_reviews_request_spec(&config, change_id).unwrap();
    assert_eq!(
        list_reviews.path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01/reviews"
    );
    assert_eq!(
        list_reviews.url,
        "https://example.test/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01/reviews"
    );

    let encoded_patchset = "P-RCT-1000%2FC-01-1";
    assert_eq!(
        build_run_patchset_ci_request_spec(&config, patchset_id, "workflow_ready", None)
            .unwrap()
            .path,
        format!("/v1/native/repository-authorities/7/patchsets/{encoded_patchset}:runCi")
    );
    assert_eq!(
        build_read_patchset_ci_status_request_spec(&config, patchset_id, 5)
            .unwrap()
            .path,
        format!("/v1/native/repository-authorities/7/read/patchsets/{encoded_patchset}/ci-status")
    );
    let readiness =
        build_read_patchset_ci_readiness_request_spec(&config, patchset_id, 200).unwrap();
    assert_eq!(
        readiness.path,
        format!("/v1/native/repository-authorities/7/read/patchsets/{encoded_patchset}/ci-status")
    );
    assert_eq!(
        readiness.query_pairs,
        vec![
            ("recent_limit".to_string(), "20".to_string()),
            ("projection".to_string(), "readiness".to_string()),
        ]
    );
    assert_eq!(
        build_get_attestation_request_spec(&config, patchset_id)
            .unwrap()
            .path,
        format!("/v1/native/repository-authorities/7/patchsets/{encoded_patchset}/attestation")
    );
    assert_eq!(
        build_get_policy_request_spec(&config, patchset_id)
            .unwrap()
            .path,
        format!("/v1/native/repository-authorities/7/patchsets/{encoded_patchset}/policy")
    );
    assert_eq!(
        build_create_waiver_request_spec(&config, patchset_id, "rule", "reason", None)
            .unwrap()
            .path,
        format!("/v1/native/repository-authorities/7/patchsets/{encoded_patchset}/waivers")
    );
    assert_eq!(
        build_submit_land_request_spec(
            &config,
            change_id,
            Some(patchset_id),
            "main",
            "merge",
            None,
        )
        .unwrap()
        .path,
        "/v1/native/repository-authorities/7/changes/RCT-1000%2FC-01:submit"
    );
    assert_eq!(
        build_get_land_request_spec(&config, submission_id, None)
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/lands/L-RCT-1000%2FC-01-1"
    );
    assert_eq!(
        build_retry_land_request_spec(&config, submission_id, None, None)
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/lands/L-RCT-1000%2FC-01-1:retry"
    );
}

#[test]
fn workflow_closeout_specs_prefer_the_configured_repository_authority() {
    let mut scoped = config();
    scoped.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));
    let change_id = "RCT-1000/C-01";
    let patchset_id = "P-RCT-1000/C-01-1";
    let authority = "/v1/native/repository-authorities/9";

    assert_eq!(
        build_get_task_request_spec(&scoped, "RCT-1000", Some("legacy-name"))
            .unwrap()
            .path,
        format!("{authority}/tasks/RCT-1000")
    );
    assert_eq!(
        build_close_task_request_spec(&scoped, "RCT-1000", "completed")
            .unwrap()
            .path,
        format!("{authority}/tasks/RCT-1000:close")
    );
    assert_eq!(
        build_list_changes_request_spec(&scoped, "legacy-name")
            .unwrap()
            .path,
        format!("{authority}/changes")
    );
    assert_eq!(
        build_get_change_request_spec(&scoped, change_id, Some("legacy-name"))
            .unwrap()
            .path,
        format!("{authority}/changes/RCT-1000%2FC-01")
    );
    assert_eq!(
        build_close_change_request_spec(&scoped, change_id, "archived")
            .unwrap()
            .path,
        format!("{authority}/changes/RCT-1000%2FC-01:close")
    );
    assert_eq!(
        build_publish_patchset_request_spec(
            &scoped, change_id, "SNP-BASE", "SNP-REV", "summary", "agent",
        )
        .unwrap()
        .path,
        format!("{authority}/changes/RCT-1000%2FC-01/patchsets")
    );
    assert_eq!(
        build_list_patchsets_request_spec(&scoped, change_id, Some("legacy-name"))
            .unwrap()
            .path,
        format!("{authority}/changes/RCT-1000%2FC-01/patchsets")
    );
    assert_eq!(
        build_get_patchset_request_spec(
            &scoped,
            patchset_id,
            Some("legacy-name"),
            Some(change_id),
        )
        .unwrap()
        .path,
        format!("{authority}/patchsets/P-RCT-1000%2FC-01-1")
    );
    assert_eq!(
        build_select_patchset_request_spec(&scoped, change_id, patchset_id)
            .unwrap()
            .path,
        format!("{authority}/changes/RCT-1000%2FC-01:selectPatchset")
    );
    assert_eq!(
        build_request_review_request_spec(&scoped, change_id, patchset_id, &[], None)
            .unwrap()
            .path,
        format!("{authority}/changes/RCT-1000%2FC-01:requestReview")
    );
    assert_eq!(
        build_record_review_request_spec(
            &scoped,
            change_id,
            patchset_id,
            "reviewer",
            "approve",
            None,
            false,
        )
        .unwrap()
        .path,
        format!("{authority}/changes/RCT-1000%2FC-01/reviews")
    );
    assert_eq!(
        build_list_reviews_request_spec(&scoped, change_id)
            .unwrap()
            .path,
        format!("{authority}/changes/RCT-1000%2FC-01/reviews")
    );
    assert_eq!(
        build_get_attestation_request_spec(&scoped, patchset_id)
            .unwrap()
            .path,
        format!("{authority}/patchsets/P-RCT-1000%2FC-01-1/attestation")
    );
    assert_eq!(
        build_evaluate_policy_request_spec(&scoped, patchset_id)
            .unwrap()
            .path,
        format!("{authority}/patchsets/P-RCT-1000%2FC-01-1:evaluatePolicy")
    );
    assert_eq!(
        build_get_policy_request_spec(&scoped, patchset_id)
            .unwrap()
            .path,
        format!("{authority}/patchsets/P-RCT-1000%2FC-01-1/policy")
    );
    assert_eq!(
        build_submit_land_request_spec(
            &scoped,
            change_id,
            Some(patchset_id),
            "main",
            "merge",
            Some("legacy-name"),
        )
        .unwrap()
        .path,
        format!("{authority}/changes/RCT-1000%2FC-01:submit")
    );
    assert_eq!(
        build_get_land_request_spec(&scoped, "LAND-1", Some("legacy-name"))
            .unwrap()
            .path,
        format!("{authority}/lands/LAND-1")
    );
}

#[test]
fn read_plan_candidate_inputs_request_spec_normalizes_contains_query() {
    let spec = build_read_plan_candidate_inputs_request_spec(
        &config(),
        "housekeeper",
        &[
            " task-review ".to_string(),
            "workflow-land".to_string(),
            "".to_string(),
        ],
    )
    .unwrap();
    assert_eq!(spec.method, "GET");
    assert_eq!(
        spec.path,
        "/v1/native/repository-authorities/7/read/plans/candidate-inputs"
    );
    assert_eq!(
            spec.url,
            "https://example.test/v1/native/repository-authorities/7/read/plans/candidate-inputs?contains=task-review%2Cworkflow-land"
        );
    assert_eq!(
        spec.query_pairs,
        vec![(
            "contains".to_string(),
            "task-review,workflow-land".to_string()
        )]
    );
    assert!(spec.body.is_none());

    let unfiltered =
        build_read_plan_candidate_inputs_request_spec(&config(), "housekeeper", &[]).unwrap();
    assert_eq!(unfiltered.method, "GET");
    assert_eq!(
        unfiltered.url,
        "https://example.test/v1/native/repository-authorities/7/read/plans/candidate-inputs"
    );
    assert!(unfiltered.query_pairs.is_empty());
    assert!(unfiltered.body.is_none());
}

#[test]
fn plan_linkage_request_specs_match_native_rust_contract() {
    let linkage_spec = build_resolve_task_plan_linkage_request_spec(
        &config(),
        " repo/one ",
        Some(" PL-1 "),
        Some(" PR-1 "),
        Some(" item-1 "),
    )
    .unwrap();
    assert_eq!(linkage_spec.method, "POST");
    assert_eq!(
        linkage_spec.path,
        "/v1/native/repository-authorities/7/sprint-task-linkage/resolve"
    );
    let body = linkage_spec.body.expect("linkage body");
    assert_eq!(body["plan_id"], "PL-1");
    assert_eq!(body["origin_plan_revision_id"], "PR-1");
    assert_eq!(body["plan_item_ref"], "item-1");

    let contains_spec = build_list_plan_ids_matching_contains_request_spec(
        &config(),
        " repo/one ",
        &[" ASBFC-08 ".to_string(), " ".to_string()],
    )
    .unwrap();
    assert_eq!(contains_spec.method, "POST");
    assert_eq!(
        contains_spec.path,
        "/v1/native/repository-authorities/7/sprint-plan-ids/by-contains"
    );
    assert_eq!(
        contains_spec.body.expect("contains body")["contains_terms"],
        json!(["ASBFC-08"])
    );
}

#[test]
fn create_plan_request_spec_omits_caller_selected_plan_identity() {
    let spec = build_create_plan_request_spec(
        &config(),
        "housekeeper",
        "Plan title",
        "docs/sprints/demo.md",
        None,
        "Demo heading",
        &[JsonValue::Object(Map::from_iter([(
            "plan_item_ref".to_string(),
            JsonValue::String("demo/ref-1".to_string()),
        )]))],
        Some("summary"),
        "draft",
        Some("PL-123"),
        "manual_edit",
        Some("# body\n"),
        None,
    )
    .unwrap();
    assert_eq!(spec.method, "POST");
    let body = spec.body.expect("body");
    assert_eq!(body["title"], "Plan title");
    assert_eq!(body["artifact_path"], "docs/sprints/demo.md");
    assert_eq!(body["artifact_selector"], JsonValue::Null);
    assert_eq!(body["artifact_heading"], "Demo heading");
    assert_eq!(body["summary"], "summary");
    assert!(body.get("plan_id").is_none());
    assert_eq!(body["source_kind"], "manual_edit");
    let legacy_identity_key = ["source", "_session", "_id"].concat();
    assert!(body.get(&legacy_identity_key).is_none());
    assert_eq!(body["artifact_body"], "# body\n");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
}

#[test]
fn repository_scoped_plan_and_remote_sync_specs_use_configured_repository_index() {
    let mut scoped = config();
    scoped.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));

    let list = build_list_plans_request_spec(&scoped, "ignored-name", None).unwrap();
    assert_eq!(list.path, "/v1/native/repository-authorities/9/sprints");
    let get = build_get_plan_request_spec(&scoped, "PR-4").unwrap();
    assert_eq!(get.path, "/v1/native/repository-authorities/9/sprints/PR-4");
    let revisions = build_list_plan_revisions_request_spec(&scoped, "PR-4").unwrap();
    assert_eq!(
        revisions.path,
        "/v1/native/repository-authorities/9/sprints/PR-4/revisions"
    );
    let revision =
        build_get_plan_revision_request_spec(&scoped, "PR-4", "plan-revision:8").unwrap();
    assert_eq!(
        revision.path,
        "/v1/native/repository-authorities/9/sprints/PR-4/revisions/plan-revision%3A8"
    );
    let create = build_create_plan_request_spec(
        &scoped,
        "ignored-name",
        "Plan",
        "docs/plan.md",
        None,
        "Plan",
        &[],
        None,
        "draft",
        Some("PR-999"),
        "manual_edit",
        None,
        None,
    )
    .unwrap();
    assert_eq!(create.path, list.path);
    assert!(create.body.unwrap().get("plan_id").is_none());

    let bulk = build_plan_remote_zstd_bulk_request_spec(
        &scoped,
        "ignored-name",
        &json!({
            "snapshot_ids": [],
            "object_packs": [],
            "tree_packs": []
        }),
    )
    .unwrap();
    assert_eq!(
        bulk.path,
        "/v1/native/repository-authorities/9/remote-sync/zstd-bulk/plan"
    );
}

#[test]
fn create_plan_request_spec_carries_packed_artifact_locator() {
    let packed_artifact = json!({
        "artifact_blob_id": "BLB-abc",
        "storage_authority": "remote_zstd_pack",
        "object_pack": {
            "pack_id": "PCK-abc",
            "pack_format": "ait-pack-v3-zstd-chunked",
        },
        "root_tree": {
            "tree_id": "TRE-abc",
            "tree_pack_id": "TPK-abc",
        },
    });
    let spec = build_create_plan_request_spec(
        &config(),
        "housekeeper",
        "Plan title",
        "docs/sprints/demo.md",
        None,
        "Demo heading",
        &[],
        None,
        "draft",
        Some("PL-123"),
        "manual_edit",
        None,
        Some(&packed_artifact),
    )
    .unwrap();
    let body = spec.body.expect("body");
    assert_eq!(body["artifact_blob_id"], "BLB-abc");
    assert_eq!(body["packed_artifact"], packed_artifact);
    assert!(body.get("artifact_body").is_none());
}

#[test]
fn revise_plan_request_spec_omits_empty_optional_fields() {
    let spec = build_revise_plan_request_spec(
        &config(),
        "PL-123",
        "docs/sprints/demo.md",
        Some("demo/root"),
        "Demo heading",
        &[],
        Some("  "),
        None,
        "manual_edit",
        None,
        None,
        None,
    )
    .unwrap();
    let body = spec.body.expect("body");
    assert!(body.get("title").is_none());
    assert!(body.get("summary").is_none());
    let legacy_identity_key = ["source", "_session", "_id"].concat();
    assert!(body.get(&legacy_identity_key).is_none());
    assert!(body.get("artifact_body").is_none());
    assert!(body.get("expected_head_revision_id").is_none());
    assert_eq!(body["artifact_selector"], "demo/root");
}

#[test]
fn update_status_and_artifact_specs_are_shaped_correctly() {
    let update_spec =
        build_update_plan_status_request_spec(&config(), "PL-123", "archived").unwrap();
    assert_eq!(update_spec.method, "PATCH");
    assert_eq!(update_spec.body.unwrap()["status"], "archived");

    let artifact_spec = build_put_plan_revision_artifacts_request_spec(
        &config(),
        "PL-123",
        "PR-2",
        &[JsonValue::Object(Map::from_iter([(
            "artifact_path".to_string(),
            JsonValue::String("docs/sprints/demo.evidence.json".to_string()),
        )]))],
    )
    .unwrap();
    assert_eq!(artifact_spec.method, "PUT");
    assert_eq!(
        artifact_spec.url,
        "https://example.test/v1/native/repository-authorities/7/sprints/PL-123/revisions/PR-2/artifacts"
    );
    assert_eq!(
        artifact_spec.body.unwrap()["artifacts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn planning_session_request_specs_match_native_contract_shape() {
    let create_spec = build_create_planning_session_request_spec(
        &config(),
        "PL-123",
        Some("Relay planning"),
        "connected_local",
        Some("codex"),
        true,
        Some("PS-123"),
    )
    .unwrap();
    assert_eq!(create_spec.method, "POST");
    assert_eq!(
        create_spec.url,
        "https://example.test/v1/native/sprints/PL-123/planning-sessions"
    );
    let create_body = create_spec.body.unwrap();
    assert_eq!(create_body["mode"], "connected_local");
    assert_eq!(create_body["resume_if_active"], true);
    assert_eq!(create_body["title"], "Relay planning");
    assert_eq!(create_body["preferred_agent"], "codex");
    assert_eq!(create_body["planning_session_id"], "PS-123");

    let list_spec =
        build_list_planning_sessions_request_spec(&config(), "PL-123", Some("closed")).unwrap();
    assert_eq!(list_spec.method, "GET");
    assert_eq!(
        list_spec.url,
        "https://example.test/v1/native/sprints/PL-123/planning-sessions?status=closed"
    );

    let get_spec = build_get_planning_session_request_spec(&config(), "PS-123").unwrap();
    assert_eq!(
        get_spec.url,
        "https://example.test/v1/native/planning-sessions/PS-123"
    );

    let append_spec = build_append_planning_session_event_request_spec(
        &config(),
        "PS-123",
        "plan.message",
        &JsonValue::Object(Map::from_iter([(
            "text".to_string(),
            JsonValue::String("hello".to_string()),
        )])),
    )
    .unwrap();
    let append_body = append_spec.body.unwrap();
    assert_eq!(append_body["event_type"], "plan.message");
    assert_eq!(append_body["payload"]["text"], "hello");

    let events_spec =
        build_list_planning_session_events_request_spec(&config(), "PS-123", 1, 50).unwrap();
    assert_eq!(
        events_spec.url,
        "https://example.test/v1/native/planning-sessions/PS-123/events?after_sequence=1&limit=50"
    );

    let join_spec = build_join_planning_session_request_spec(
        &config(),
        "PS-123",
        "editor",
        Some("Editor relay"),
        Some("gpt-5-codex"),
        true,
    )
    .unwrap();
    let join_body = join_spec.body.unwrap();
    assert_eq!(join_body["surface"], "editor");
    assert_eq!(join_body["resume_if_active"], true);
    assert_eq!(join_body["title"], "Editor relay");
    assert_eq!(join_body["model_name"], "gpt-5-codex");
    let mut join_body_keys = join_body
        .as_object()
        .expect("join body")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    join_body_keys.sort();
    assert_eq!(
        join_body_keys,
        vec!["model_name", "resume_if_active", "surface", "title"]
    );

    let promote_spec = build_promote_planning_session_request_spec(
        &config(),
        "PS-123",
        "docs/sprints/demo.md",
        "demo/root",
        "Demo heading",
        &[JsonValue::Object(Map::from_iter([(
            "plan_item_ref".to_string(),
            JsonValue::String("demo/ref-1".to_string()),
        )]))],
        Some("Promoted title"),
        Some("Promoted summary"),
        Some("# promoted body\n"),
    )
    .unwrap();
    let promote_body = promote_spec.body.unwrap();
    assert_eq!(promote_body["artifact_path"], "docs/sprints/demo.md");
    assert_eq!(promote_body["artifact_selector"], "demo/root");
    assert_eq!(promote_body["artifact_heading"], "Demo heading");
    assert_eq!(promote_body["title"], "Promoted title");
    assert_eq!(promote_body["summary"], "Promoted summary");
    assert_eq!(promote_body["artifact_body"], "# promoted body\n");
    assert_eq!(promote_body["items"].as_array().unwrap().len(), 1);

    let close_spec =
        build_close_planning_session_request_spec(&config(), "PS-123", "closed").unwrap();
    assert_eq!(close_spec.method, "POST");
    assert_eq!(
        close_spec.url,
        "https://example.test/v1/native/planning-sessions/PS-123:close"
    );
    assert_eq!(close_spec.body.unwrap()["status"], "closed");
}

#[test]
fn planning_session_request_specs_enforce_invalid_payload_boundaries() {
    let err = build_append_planning_session_event_request_spec(
        &config(),
        "PS-123",
        "plan.message",
        &JsonValue::String("not-an-object".to_string()),
    )
    .unwrap_err();
    assert!(matches!(err, PlanHttpClientError::Invalid(_)));

    let err = build_create_planning_session_request_spec(
        &config(),
        "PL-123",
        None,
        " ",
        None,
        true,
        None,
    )
    .unwrap_err();
    assert!(matches!(err, PlanHttpClientError::Invalid(_)));
}

#[test]
fn client_config_rejects_empty_base_url_and_zero_pool() {
    let err = PlanHttpClientManager::new(PlanHttpClientConfig {
        base_url: " ".to_string(),
        ..PlanHttpClientConfig::default()
    })
    .unwrap_err();
    assert!(matches!(err, PlanHttpClientError::Invalid(_)));

    let err = PlanHttpClientManager::new(PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        pool_max_idle_per_host: 0,
        ..PlanHttpClientConfig::default()
    })
    .unwrap_err();
    assert!(matches!(err, PlanHttpClientError::Invalid(_)));
}

#[test]
fn inspect_and_close_expose_lifecycle_state() {
    let mut manager = PlanHttpClientManager::new(config()).unwrap();
    let initial = manager.inspect();
    assert_eq!(initial.request_count, 0);
    assert_eq!(initial.retry_count, 0);
    assert!(!initial.closed);
    let closed = manager.close();
    assert!(closed.closed);
    let err = manager.list_plans("housekeeper", None).unwrap_err();
    assert!(matches!(err, PlanHttpClientError::Closed(_)));
}

#[test]
fn bounded_bytes_requests_run_concurrently_preserve_order_and_stats() {
    let responses = BTreeMap::from([
        ("/pack-a".to_string(), (200, "a".to_string())),
        ("/pack-b".to_string(), (200, "b".to_string())),
        ("/pack-c".to_string(), (200, "c".to_string())),
        ("/pack-d".to_string(), (200, "d".to_string())),
    ]);
    let (base_url, maximum_active, server) = spawn_concurrent_bytes_server(responses);
    let mut manager = PlanHttpClientManager::new(local_http_config(base_url.clone())).unwrap();
    let specs = ["/pack-a", "/pack-b", "/pack-c", "/pack-d"]
        .into_iter()
        .map(|path| PlanHttpBytesRequestSpec {
            method: "GET".to_string(),
            path: path.to_string(),
            url: format!("{base_url}{path}"),
            query_pairs: Vec::new(),
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: 5_000,
        })
        .collect();

    let payloads = manager
        .execute_bytes_bounded(specs, 4)
        .expect("bounded bytes requests");

    assert_eq!(
        payloads,
        vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec(), b"d".to_vec()]
    );
    assert!(maximum_active.load(Ordering::SeqCst) >= 2);
    assert_eq!(manager.inspect().request_count, 4);
    assert_eq!(manager.inspect().retry_count, 0);
    assert_eq!(server.join().expect("join concurrent server").len(), 4);
}

#[test]
fn bounded_bytes_failure_keeps_stats_for_every_started_worker() {
    let responses = BTreeMap::from([
        ("/fail".to_string(), (500, "failed".to_string())),
        ("/pack-b".to_string(), (200, "b".to_string())),
        ("/pack-c".to_string(), (200, "c".to_string())),
    ]);
    let (base_url, maximum_active, server) = spawn_concurrent_bytes_server(responses);
    let mut manager = PlanHttpClientManager::new(local_http_config(base_url.clone())).unwrap();
    let specs = ["/fail", "/pack-b", "/pack-c"]
        .into_iter()
        .map(|path| PlanHttpBytesRequestSpec {
            method: "GET".to_string(),
            path: path.to_string(),
            url: format!("{base_url}{path}"),
            query_pairs: Vec::new(),
            headers: BTreeMap::new(),
            body: None,
            timeout_ms: 5_000,
        })
        .collect();

    let error = manager
        .execute_bytes_bounded(specs, 3)
        .expect_err("one bounded request must fail");

    assert_eq!(error.remote_status(), Some(500));
    assert!(maximum_active.load(Ordering::SeqCst) >= 2);
    assert_eq!(manager.inspect().request_count, 3);
    assert_eq!(manager.inspect().retry_count, 0);
    assert_eq!(server.join().expect("join failure server").len(), 3);
}
