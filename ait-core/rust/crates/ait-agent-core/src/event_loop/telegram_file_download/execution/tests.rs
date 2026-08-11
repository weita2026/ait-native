use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use ait_core::json_support::{json, JsonValue};

use super::*;

#[derive(Default)]
struct StubApi {
    results: Mutex<VecDeque<Result<TelegramFileDownloadApiExecution, String>>>,
    requests: Mutex<Vec<JsonValue>>,
}

impl StubApi {
    fn with_results(results: Vec<TelegramFileDownloadApiExecution>) -> Self {
        Self {
            results: Mutex::new(results.into_iter().map(Ok).collect()),
            requests: Mutex::new(Vec::new()),
        }
    }
}

impl TelegramFileDownloadApiPort for StubApi {
    fn execute_api(&self, request: &JsonValue) -> Result<TelegramFileDownloadApiExecution, String> {
        lock_unpoisoned(&self.requests).push(request.clone());
        lock_unpoisoned(&self.results)
            .pop_front()
            .unwrap_or_else(|| Err("private-api-secret".to_string()))
    }
}

struct StubStore {
    state: Mutex<TelegramFileCacheState>,
    publishes: Mutex<Vec<Vec<u8>>>,
    inspect_failure: bool,
    publish_failure: bool,
    publish_count_delta: isize,
}

impl StubStore {
    fn new(state: TelegramFileCacheState) -> Self {
        Self {
            state: Mutex::new(state),
            publishes: Mutex::new(Vec::new()),
            inspect_failure: false,
            publish_failure: false,
            publish_count_delta: 0,
        }
    }
}

impl TelegramFileDownloadStorePort for StubStore {
    fn inspect(
        &self,
        _cache_root: &Path,
        _local_path: &Path,
    ) -> Result<TelegramFileCacheState, String> {
        if self.inspect_failure {
            return Err("private-inspect-secret".to_string());
        }
        Ok(*lock_unpoisoned(&self.state))
    }

    fn publish(
        &self,
        _cache_root: &Path,
        _local_path: &Path,
        payload: &[u8],
    ) -> Result<usize, String> {
        if self.publish_failure {
            return Err("private-publish-secret".to_string());
        }
        lock_unpoisoned(&self.publishes).push(payload.to_vec());
        *lock_unpoisoned(&self.state) = TelegramFileCacheState::Hit {
            byte_count: payload.len(),
        };
        Ok(payload
            .len()
            .saturating_add_signed(self.publish_count_delta))
    }
}

fn request(cache_root: &Path) -> JsonValue {
    json!({
        "message": {"message_id": 7, "chat": {"id": 42}},
        "attachment": {
            "kind": "voice",
            "media_kind": "speech",
            "telegram_file_id": "tg-file-1",
            "telegram_file_unique_id": "unique-1",
            "mime_type": "audio/ogg"
        },
        "cache_root": cache_root.to_string_lossy(),
        "bot_token": "123:private-token",
        "request_timeout_seconds": 3.0
    })
}

fn api_metadata(operation: &str, transport: &str, downloaded: bool) -> JsonValue {
    json!({
        "contract": API_CONTRACT,
        "migration_stage": API_MIGRATION_STAGE,
        "stage": "execute",
        "telegram_api_state": "completed",
        "operation": operation,
        "telegram_method": if operation == "get_file" { "getFile" } else { "downloadFile" },
        "transport": transport,
        "retry_family": "delivery",
        "max_attempts": 3,
        "attempts": 1,
        "retry_count": 0,
        "retry_delays_seconds": [],
        "retry_exhausted": false,
        "http_status_code": 200,
        "ok": true,
        "completed": true,
        "downloaded": downloaded,
        "byte_count": if downloaded { json!(4) } else { JsonValue::Null },
        "sent": false,
        "error_kind": JsonValue::Null,
        "error": JsonValue::Null,
        "python_telegram_api_allowed": false,
        "python_http_execution_allowed": false,
        "python_retry_allowed": false,
        "raw_telegram_payload_exposed": false,
        "token_bearing_url_exposed": false,
        "downloaded_bytes_exposed": false,
        "local_path_exposed": false,
        "multipart_fields_exposed": false,
        "file_name_exposed": false,
    })
}

fn get_file_execution(path: Option<&str>) -> TelegramFileDownloadApiExecution {
    let mut metadata = api_metadata("get_file", "json", false);
    metadata["file_info"] = match path {
        Some(path) => json!({
            "file_id": "tg-file-1",
            "file_unique_id": "unique-1",
            "file_size": 4,
            "file_path": path,
        }),
        None => json!({"file_id": "tg-file-1"}),
    };
    TelegramFileDownloadApiExecution::new(metadata, None)
}

fn download_execution(payload: &[u8]) -> TelegramFileDownloadApiExecution {
    let mut metadata = api_metadata("download_file", "bytes", true);
    metadata["byte_count"] = json!(payload.len());
    TelegramFileDownloadApiExecution::new(metadata, Some(payload.to_vec()))
}

#[test]
fn cache_miss_downloads_exact_bytes_and_preserves_attachment_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cache_root = temp.path().join("telegram-downloads");
    let api = StubApi::with_results(vec![
        get_file_execution(Some("voice/private.ogg")),
        download_execution(&[0, 255, 1, 2]),
    ]);
    let store = StubStore::new(TelegramFileCacheState::Missing);

    let execution = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &api,
        &store,
        &request(&cache_root),
    )
    .expect("file download");

    let outcome = execution.metadata();
    assert_eq!(outcome["contract"], CONTRACT);
    assert_eq!(outcome["file_download_state"], "completed");
    assert_eq!(outcome["cache_hit"], false);
    assert_eq!(outcome["downloaded"], true);
    assert_eq!(outcome["byte_count"], 4);
    assert_eq!(outcome["attachment"]["kind"], "voice");
    assert_eq!(outcome["attachment"]["media_kind"], "speech");
    assert_eq!(outcome["attachment"]["mime_type"], "audio/ogg");
    assert_eq!(
        outcome["attachment"]["telegram_file_path"],
        "voice/private.ogg"
    );
    assert_eq!(outcome["attachment"]["local_path"], outcome["local_path"]);
    assert_eq!(&*lock_unpoisoned(&store.publishes), &[vec![0, 255, 1, 2]]);
    let calls = lock_unpoisoned(&api.requests);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["operation"], "get_file");
    assert_eq!(calls[0]["file_id"], "tg-file-1");
    assert_eq!(calls[1]["operation"], "download_file");
    assert_eq!(calls[1]["file_path"], "voice/private.ogg");
    assert_eq!(calls[1]["bot_token"], "123:private-token");
    let debug = format!("{execution:?}");
    assert!(!debug.contains("private"));
    assert!(!debug.contains("telegram-downloads"));
}

#[test]
fn cache_hit_skips_byte_download_and_publication() {
    let temp = tempfile::tempdir().expect("tempdir");
    let api = StubApi::with_results(vec![get_file_execution(Some("docs/file.bin"))]);
    let store = StubStore::new(TelegramFileCacheState::Hit { byte_count: 19 });

    let execution = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &api,
        &store,
        &request(&temp.path().join("cache")),
    )
    .expect("cache hit");

    assert_eq!(execution.metadata()["cache_hit"], true);
    assert_eq!(execution.metadata()["downloaded"], false);
    assert_eq!(execution.metadata()["byte_count"], 19);
    assert_eq!(lock_unpoisoned(&api.requests).len(), 1);
    assert!(lock_unpoisoned(&store.publishes).is_empty());
}

#[test]
fn audio_photo_and_document_metadata_survive_cache_resolution() {
    let temp = tempfile::tempdir().expect("tempdir");
    for (kind, media_kind, file_name, mime_type, telegram_path) in [
        (
            "audio",
            "music",
            Some("track.mp3"),
            Some("audio/mpeg"),
            "music/track.mp3",
        ),
        (
            "photo",
            "image",
            None,
            Some("image/jpeg"),
            "photos/image.jpg",
        ),
        (
            "document",
            "file",
            Some("report.pdf"),
            Some("application/pdf"),
            "docs/report.pdf",
        ),
    ] {
        let mut input = request(&temp.path().join(format!("cache-{kind}")));
        input["attachment"] = json!({
            "kind": kind,
            "media_kind": media_kind,
            "telegram_file_id": "tg-file-1",
            "telegram_file_unique_id": format!("unique-{kind}"),
            "file_name": file_name,
            "mime_type": mime_type,
        });
        let api = StubApi::with_results(vec![get_file_execution(Some(telegram_path))]);
        let store = StubStore::new(TelegramFileCacheState::Hit { byte_count: 7 });
        let outcome = execute_with_telegram_file_download_ports(
            &DefaultTelegramFileDownloadPlanner,
            &api,
            &store,
            &input,
        )
        .expect("attachment kind")
        .into_metadata();
        assert_eq!(outcome["attachment"]["kind"], kind);
        assert_eq!(outcome["attachment"]["media_kind"], media_kind);
        assert_eq!(outcome["attachment"]["telegram_file_path"], telegram_path);
        if let Some(file_name) = file_name {
            assert_eq!(outcome["attachment"]["file_name"], file_name);
        }
        if let Some(mime_type) = mime_type {
            assert_eq!(outcome["attachment"]["mime_type"], mime_type);
        }
    }
}

#[test]
fn missing_id_and_missing_path_use_typed_fail_closed_errors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut missing_id = request(&temp.path().join("cache"));
    missing_id["attachment"]
        .as_object_mut()
        .expect("attachment")
        .remove("telegram_file_id");
    let api = StubApi::default();
    let store = StubStore::new(TelegramFileCacheState::Missing);
    let error = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &api,
        &store,
        &missing_id,
    )
    .expect_err("missing id");
    assert_eq!(
        error.kind(),
        TelegramFileDownloadExecutionErrorKind::MissingFileId
    );
    assert!(lock_unpoisoned(&api.requests).is_empty());

    let api = StubApi::with_results(vec![get_file_execution(None)]);
    let error = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &api,
        &store,
        &request(&temp.path().join("cache")),
    )
    .expect_err("missing path");
    assert_eq!(
        error.kind(),
        TelegramFileDownloadExecutionErrorKind::MissingFilePath
    );
}

struct EscapingPlanner;

impl TelegramFileDownloadPlanner for EscapingPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let mut planned = DefaultTelegramFileDownloadPlanner.plan_json(request)?;
        if request.get("stage").and_then(JsonValue::as_str) == Some("file_info") {
            let escaped = request
                .get("execution_request")
                .and_then(|value| value.get("cache_root"))
                .and_then(JsonValue::as_str)
                .map(PathBuf::from)
                .and_then(|root| root.parent().map(|parent| parent.join("escaped.bin")))
                .expect("escaped path");
            planned["request"]["local_path"] = json!(escaped.to_string_lossy());
            planned["request"]["operations"][0]["local_path"] = json!(escaped.to_string_lossy());
        }
        Ok(planned)
    }
}

#[test]
fn escaping_planner_path_is_rejected_before_store_side_effects() {
    let temp = tempfile::tempdir().expect("tempdir");
    let api = StubApi::with_results(vec![get_file_execution(Some("docs/file.bin"))]);
    let store = StubStore::new(TelegramFileCacheState::Missing);
    let error = execute_with_telegram_file_download_ports(
        &EscapingPlanner,
        &api,
        &store,
        &request(&temp.path().join("cache")),
    )
    .expect_err("escaping path");
    assert_eq!(
        error.kind(),
        TelegramFileDownloadExecutionErrorKind::CachePath
    );
    assert!(lock_unpoisoned(&store.publishes).is_empty());
}

struct CorruptPlanner;

impl TelegramFileDownloadPlanner for CorruptPlanner {
    fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Ok(json!({"private": "planner-secret"}))
    }
}

#[test]
fn corrupt_planner_telegram_failure_and_oversize_contracts_fail_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cache_root = temp.path().join("cache");
    let store = StubStore::new(TelegramFileCacheState::Missing);
    let corrupt = execute_with_telegram_file_download_ports(
        &CorruptPlanner,
        &StubApi::default(),
        &store,
        &request(&cache_root),
    )
    .expect_err("corrupt planner");
    assert_eq!(
        corrupt.kind(),
        TelegramFileDownloadExecutionErrorKind::PlannerContract
    );
    assert!(!corrupt.to_string().contains("planner-secret"));

    let mut failed_metadata = api_metadata("get_file", "json", false);
    failed_metadata["telegram_api_state"] = json!("telegram_api_failed");
    failed_metadata["ok"] = json!(false);
    failed_metadata["completed"] = json!(false);
    failed_metadata["error_kind"] = json!("telegram");
    failed_metadata["error"] = json!("private-telegram-secret");
    let api = StubApi::with_results(vec![TelegramFileDownloadApiExecution::new(
        failed_metadata,
        None,
    )]);
    let failed = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &api,
        &store,
        &request(&cache_root),
    )
    .expect_err("Telegram failure");
    assert_eq!(
        failed.kind(),
        TelegramFileDownloadExecutionErrorKind::TelegramFileInfo
    );
    assert!(!failed.to_string().contains("private-telegram-secret"));

    let mut oversized = request(&cache_root);
    oversized["attachment"]["file_size_bytes"] = json!((MAX_ATTACHMENT_BYTES as u64) + 1);
    let oversized = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &StubApi::default(),
        &store,
        &oversized,
    )
    .expect_err("oversized attachment");
    assert_eq!(
        oversized.kind(),
        TelegramFileDownloadExecutionErrorKind::PayloadSize
    );
}

#[test]
fn api_payload_and_store_failures_are_typed_and_secret_safe() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cache_root = temp.path().join("cache");
    let store = StubStore::new(TelegramFileCacheState::Missing);
    let api = StubApi::with_results(vec![
        get_file_execution(Some("docs/file.bin")),
        download_execution(&[]),
    ]);
    let empty = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &api,
        &store,
        &request(&cache_root),
    )
    .expect_err("empty payload");
    assert_eq!(
        empty.kind(),
        TelegramFileDownloadExecutionErrorKind::PayloadSize
    );

    let mut failed_store = StubStore::new(TelegramFileCacheState::Missing);
    failed_store.publish_failure = true;
    let api = StubApi::with_results(vec![
        get_file_execution(Some("docs/file.bin")),
        download_execution(&[1, 2, 3, 4]),
    ]);
    let publish = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &api,
        &failed_store,
        &request(&cache_root),
    )
    .expect_err("publish failure");
    assert_eq!(
        publish.kind(),
        TelegramFileDownloadExecutionErrorKind::Publish
    );
    assert_eq!(
        publish.to_string(),
        "Telegram file download execution failed."
    );

    let mut mismatch = StubStore::new(TelegramFileCacheState::Missing);
    mismatch.publish_count_delta = 1;
    let api = StubApi::with_results(vec![
        get_file_execution(Some("docs/file.bin")),
        download_execution(&[1, 2, 3, 4]),
    ]);
    let mismatch = execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &api,
        &mismatch,
        &request(&cache_root),
    )
    .expect_err("publish mismatch");
    assert_eq!(
        mismatch.kind(),
        TelegramFileDownloadExecutionErrorKind::Publish
    );
}

struct CountingApi {
    download_count: Mutex<usize>,
}

impl TelegramFileDownloadApiPort for CountingApi {
    fn execute_api(&self, request: &JsonValue) -> Result<TelegramFileDownloadApiExecution, String> {
        match request.get("operation").and_then(JsonValue::as_str) {
            Some("get_file") => Ok(get_file_execution(Some("voice/shared.ogg"))),
            Some("download_file") => {
                *lock_unpoisoned(&self.download_count) += 1;
                Ok(download_execution(&[1, 2, 3, 4]))
            }
            _ => Err("unexpected".to_string()),
        }
    }
}

#[test]
fn concurrent_same_path_requests_publish_only_once() {
    let temp = tempfile::tempdir().expect("tempdir");
    let request = Arc::new(request(&temp.path().join("cache")));
    let api = Arc::new(CountingApi {
        download_count: Mutex::new(0),
    });
    let store = Arc::new(StubStore::new(TelegramFileCacheState::Missing));
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let request = Arc::clone(&request);
        let api = Arc::clone(&api);
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            execute_with_telegram_file_download_ports(
                &DefaultTelegramFileDownloadPlanner,
                api.as_ref(),
                store.as_ref(),
                request.as_ref(),
            )
            .expect("concurrent execution")
            .into_metadata()
        }));
    }
    barrier.wait();
    let outcomes = handles
        .into_iter()
        .map(|handle| handle.join().expect("download thread"))
        .collect::<Vec<_>>();
    assert_eq!(*lock_unpoisoned(&api.download_count), 1);
    assert_eq!(lock_unpoisoned(&store.publishes).len(), 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["downloaded"] == true)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| outcome["cache_hit"] == true)
            .count(),
        1
    );
}

#[cfg(unix)]
#[test]
fn native_store_rejects_symlinked_cache_components_and_directory_targets() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let cache_root = temp.path().join("cache");
    fs::create_dir_all(&cache_root).expect("cache root");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).expect("outside");
    symlink(&outside, cache_root.join("linked")).expect("cache symlink");
    let linked_target = cache_root.join("linked/file.bin");
    assert!(NativeTelegramFileDownloadStorePort
        .inspect(&cache_root, &linked_target)
        .is_err());

    let directory_target = cache_root.join("directory.bin");
    fs::create_dir(&directory_target).expect("directory target");
    assert!(NativeTelegramFileDownloadStorePort
        .inspect(&cache_root, &directory_target)
        .is_err());
}

#[test]
fn native_executor_runs_get_file_then_binary_download_and_atomic_publish() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        for (index, response) in [
            br#"{"ok":true,"result":{"file_id":"tg-file-1","file_unique_id":"unique-1","file_size":4,"file_path":"voice/native.ogg"}}"#.to_vec(),
            vec![0, 255, 1, 2],
        ]
        .into_iter()
        .enumerate()
        {
            let (mut stream, _) = listener.accept().expect("accept loopback request");
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .expect("read timeout");
            let request = read_http_request(&mut stream);
            if index == 0 {
                assert!(String::from_utf8_lossy(&request).starts_with("POST /getFile HTTP/1.1"));
            } else {
                assert!(String::from_utf8_lossy(&request)
                    .starts_with("GET /voice/native.ogg HTTP/1.1"));
            }
            let content_type = if index == 0 {
                "application/json"
            } else {
                "application/octet-stream"
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .expect("response headers");
            stream.write_all(&response).expect("response body");
        }
    });
    let temp = tempfile::tempdir().expect("tempdir");
    let cache_root = temp.path().join("cache");
    let mut native_request = request(&cache_root);
    native_request
        .as_object_mut()
        .expect("request")
        .remove("bot_token");
    native_request["base_url"] = json!(format!("http://{address}"));
    native_request["file_base_url"] = json!(format!("http://{address}"));

    let execution = agent_telegram_file_download_execute(&native_request).expect("native download");
    let local_path = PathBuf::from(
        execution.metadata()["local_path"]
            .as_str()
            .expect("local path"),
    );
    assert_eq!(fs::read(&local_path).expect("cached bytes"), [0, 255, 1, 2]);
    assert_eq!(execution.metadata()["downloaded"], true);
    server.join().expect("loopback server");
}

fn read_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 4_096];
    loop {
        let read = stream.read(&mut chunk).expect("read request");
        assert!(read > 0, "request ended before headers");
        raw.extend_from_slice(&chunk[..read]);
        let Some(header_index) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let header_end = header_index + 4;
        let headers = String::from_utf8_lossy(&raw[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        if raw.len() >= header_end + content_length {
            return raw;
        }
    }
}
