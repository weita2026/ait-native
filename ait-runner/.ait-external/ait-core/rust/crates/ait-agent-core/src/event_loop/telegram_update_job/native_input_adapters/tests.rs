use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use ait_core::json_support::{json, JsonValue};
use tempfile::{tempdir, TempDir};

use super::*;
use crate::event_loop::telegram_stt_execution::TelegramSttExecutionError;
use crate::transport_config::{
    resolve_agent_worker_config, AgentWorkerConfigInput, AgentWorkerRuntimeConfig,
};

enum FileOutcome {
    Success(Vec<u8>),
    Error(TelegramFileDownloadExecutionErrorKind),
    Malformed,
    UnsafePath,
}

struct FileState {
    outcomes: Mutex<VecDeque<FileOutcome>>,
    requests: Mutex<Vec<JsonValue>>,
}

#[derive(Clone)]
struct StubFileExecutor {
    state: Arc<FileState>,
}

impl StubFileExecutor {
    fn new(outcomes: Vec<FileOutcome>) -> Self {
        Self {
            state: Arc::new(FileState {
                outcomes: Mutex::new(outcomes.into()),
                requests: Mutex::new(Vec::new()),
            }),
        }
    }

    fn requests(&self) -> Vec<JsonValue> {
        lock(&self.state.requests).clone()
    }
}

impl TelegramUpdateFileDownloadExecutor for StubFileExecutor {
    fn execute_file_download(
        &self,
        request: &JsonValue,
    ) -> Result<JsonValue, TelegramFileDownloadExecutionError> {
        lock(&self.state.requests).push(request.clone());
        let outcome = lock(&self.state.outcomes)
            .pop_front()
            .unwrap_or(FileOutcome::Malformed);
        match outcome {
            FileOutcome::Success(payload) => Ok(file_execution(request, &payload, false)),
            FileOutcome::Error(kind) => Err(TelegramFileDownloadExecutionError::new(kind)),
            FileOutcome::Malformed => Ok(json!({"ok": true})),
            FileOutcome::UnsafePath => Ok(file_execution(request, b"unsafe", true)),
        }
    }
}

enum SttOutcome {
    Success(String),
    Error(TelegramSttExecutionErrorKind),
    Malformed,
}

struct SttState {
    outcomes: Mutex<VecDeque<SttOutcome>>,
    requests: Mutex<Vec<JsonValue>>,
}

#[derive(Clone)]
struct StubSttExecutor {
    state: Arc<SttState>,
}

impl StubSttExecutor {
    fn new(outcomes: Vec<SttOutcome>) -> Self {
        Self {
            state: Arc::new(SttState {
                outcomes: Mutex::new(outcomes.into()),
                requests: Mutex::new(Vec::new()),
            }),
        }
    }

    fn requests(&self) -> Vec<JsonValue> {
        lock(&self.state.requests).clone()
    }
}

impl TelegramSttExecutor for StubSttExecutor {
    fn execute_stt(&self, request: &JsonValue) -> Result<JsonValue, TelegramSttExecutionError> {
        lock(&self.state.requests).push(request.clone());
        match lock(&self.state.outcomes)
            .pop_front()
            .unwrap_or(SttOutcome::Malformed)
        {
            SttOutcome::Success(transcript) => Ok(stt_execution(&transcript)),
            SttOutcome::Error(kind) => Err(TelegramSttExecutionError::new(kind)),
            SttOutcome::Malformed => Ok(json!({"ok": true})),
        }
    }
}

fn config(local_stt: bool) -> (TempDir, TelegramWorkerConfig) {
    let temp = tempdir().expect("tempdir");
    fs::create_dir(temp.path().join(".ait")).expect("ait dir");
    fs::write(
        temp.path().join(".ait/config.json"),
        json!({"repo_name": "input-fixture", "workflow_mode": "solo_local"}).to_string(),
    )
    .expect("repo config");
    let mut worker = json!({
        "kind": "telegram",
        "name": "main",
        "token": "123:private-telegram-token",
        "sync_state_path": temp.path().join("private-state/telegram.json"),
    });
    if local_stt {
        for (field, value) in [
            ("stt_mode", "local-stt"),
            ("stt_model", "fixture-model"),
            ("stt_device", "cpu"),
            ("stt_compute_type", "float32"),
            ("stt_language", "zh"),
        ] {
            worker[field] = JsonValue::String(value.to_string());
        }
    }
    let runtime = resolve_agent_worker_config(AgentWorkerConfigInput {
        repo_root: temp.path().to_path_buf(),
        worker_key: "telegram/main".to_string(),
        worker,
        process_env: BTreeMap::new(),
    })
    .expect("Telegram config");
    let AgentWorkerRuntimeConfig::Telegram(config) = runtime else {
        panic!("Telegram config variant");
    };
    (temp, config)
}

fn attachment(kind: &str, file_id: Option<&str>) -> JsonValue {
    json!({
        "kind": kind,
        "media_kind": if kind == "voice" { "speech" } else { "file" },
        "telegram_file_id": file_id,
        "telegram_file_unique_id": format!("unique-{}", file_id.unwrap_or("missing")),
        "mime_type": if kind == "voice" { "audio/ogg" } else { "application/octet-stream" },
    })
}

fn request(
    mode: TelegramUpdateInputMode,
    caption: Option<&str>,
    raw_text: Option<&str>,
    attachments: Vec<JsonValue>,
) -> TelegramUpdateInputRequest {
    TelegramUpdateInputRequest::new(
        mode,
        json!({
            "message_id": 7,
            "chat": {"id": 42},
            "caption": caption,
        }),
        raw_text.map(str::to_string),
        attachments,
    )
}

fn file_execution(request: &JsonValue, payload: &[u8], unsafe_path: bool) -> JsonValue {
    let cache_root = PathBuf::from(request["cache_root"].as_str().expect("cache root"));
    let file_id = request["attachment"]["telegram_file_id"]
        .as_str()
        .expect("file id");
    let local_path = if unsafe_path {
        cache_root
            .parent()
            .expect("cache parent")
            .join("unsafe-outside.bin")
    } else {
        cache_root.join(format!("{file_id}.bin"))
    };
    fs::create_dir_all(local_path.parent().expect("local parent")).expect("cache dir");
    fs::write(&local_path, payload).expect("cache payload");
    let mut resolved = request["attachment"].clone();
    resolved["telegram_file_path"] = json!(format!("fixture/{file_id}.bin"));
    resolved["local_path"] = json!(local_path.to_string_lossy());
    json!({
        "contract": FILE_CONTRACT,
        "migration_stage": FILE_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "file_download_state": "completed",
        "ok": true,
        "completed": true,
        "cache_hit": false,
        "downloaded": true,
        "byte_count": payload.len(),
        "attachment": resolved,
        "local_path": local_path.to_string_lossy(),
        "python_file_download_allowed": false,
        "python_cache_io_allowed": false,
        "python_telegram_api_allowed": false,
        "downloaded_bytes_exposed": false,
        "diagnostic_local_path_exposed": false,
        "diagnostic_telegram_path_exposed": false,
    })
}

fn stt_execution(transcript: &str) -> JsonValue {
    json!({
        "contract": TELEGRAM_STT_EXECUTION_CONTRACT,
        "migration_stage": STT_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "stt_state": "completed",
        "ok": true,
        "completed": true,
        "transcript": transcript,
        "python_stt_allowed": false,
        "python_runtime_allowed": false,
        "shell_execution_allowed": false,
        "inherited_environment_allowed": false,
        "request_payload_exposed": false,
        "response_payload_exposed": false,
        "audio_path_exposed": false,
        "program_path_exposed": false,
        "stderr_exposed": false,
        "downstream_error_exposed": false,
    })
}

#[test]
fn file_mode_downloads_every_attachment_and_preserves_candidate_text() {
    let (_temp, config) = config(false);
    let files = StubFileExecutor::new(vec![
        FileOutcome::Success(vec![1, 2, 3]),
        FileOutcome::Success(vec![4, 5]),
    ]);
    let port = NativeTelegramUpdateInputPort::with_executors(
        &config,
        files.clone(),
        None::<StubSttExecutor>,
    )
    .expect("input port");

    let prepared = port
        .prepare_input(&request(
            TelegramUpdateInputMode::DownloadAttachments,
            None,
            Some("candidate text"),
            vec![
                attachment("document", Some("doc-1")),
                attachment("photo", Some("photo-1")),
            ],
        ))
        .expect("prepared files");

    assert_eq!(prepared.raw_text(), Some("candidate text"));
    assert_eq!(prepared.attachments().len(), 2);
    for attachment in prepared.attachments() {
        let path = Path::new(attachment["local_path"].as_str().expect("local path"));
        assert!(path.is_file());
    }
    let calls = files.requests();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["bot_token"], "123:private-telegram-token");
    assert!(calls[0]["cache_root"]
        .as_str()
        .expect("cache root")
        .ends_with("private-state/telegram-downloads"));
}

#[test]
fn file_input_failures_keep_specific_safe_dispositions() {
    let (_temp, config) = config(false);
    for (outcome, expected) in [
        (
            FileOutcome::Error(TelegramFileDownloadExecutionErrorKind::MissingFilePath),
            TelegramUpdateInputErrorKind::AttachmentPathMissing,
        ),
        (
            FileOutcome::Error(TelegramFileDownloadExecutionErrorKind::CachePath),
            TelegramUpdateInputErrorKind::AttachmentHostUnsupported,
        ),
        (
            FileOutcome::Error(TelegramFileDownloadExecutionErrorKind::PayloadSize),
            TelegramUpdateInputErrorKind::AttachmentDownloadFailed,
        ),
        (
            FileOutcome::Malformed,
            TelegramUpdateInputErrorKind::AttachmentDownloadFailed,
        ),
        (
            FileOutcome::UnsafePath,
            TelegramUpdateInputErrorKind::AttachmentHostUnsupported,
        ),
    ] {
        let files = StubFileExecutor::new(vec![outcome]);
        let port =
            NativeTelegramUpdateInputPort::with_executors(&config, files, None::<StubSttExecutor>)
                .unwrap();
        let failure = port
            .prepare_input(&request(
                TelegramUpdateInputMode::DownloadAttachments,
                None,
                None,
                vec![attachment("document", Some("doc-1"))],
            ))
            .unwrap_err();
        assert_eq!(failure.kind(), expected);
        assert!(!format!("{failure:?} {failure}").contains("private"));
    }

    let files = StubFileExecutor::new(Vec::new());
    let port = NativeTelegramUpdateInputPort::with_executors(
        &config,
        files.clone(),
        None::<StubSttExecutor>,
    )
    .unwrap();
    assert_eq!(
        port.prepare_input(&request(
            TelegramUpdateInputMode::DownloadAttachments,
            None,
            None,
            vec![attachment("document", None)],
        ))
        .unwrap_err()
        .kind(),
        TelegramUpdateInputErrorKind::AttachmentFileIdMissing
    );
    assert!(files.requests().is_empty());
}

#[test]
fn speech_preconditions_fail_closed_before_external_execution() {
    let (_off_temp, off_config) = config(false);
    let off_files = StubFileExecutor::new(Vec::new());
    let off_port = NativeTelegramUpdateInputPort::with_executors(
        &off_config,
        off_files.clone(),
        None::<StubSttExecutor>,
    )
    .unwrap();
    assert_eq!(
        off_port
            .prepare_input(&request(
                TelegramUpdateInputMode::SpeechToText,
                None,
                None,
                vec![attachment("voice", Some("voice-1"))],
            ))
            .unwrap_err()
            .kind(),
        TelegramUpdateInputErrorKind::SpeechToTextNotEnabled
    );
    assert!(off_files.requests().is_empty());

    let (_on_temp, on_config) = config(true);
    let missing_port = NativeTelegramUpdateInputPort::with_executors(
        &on_config,
        StubFileExecutor::new(Vec::new()),
        None::<StubSttExecutor>,
    )
    .unwrap();
    assert_eq!(
        missing_port
            .prepare_input(&request(
                TelegramUpdateInputMode::SpeechToText,
                None,
                None,
                Vec::new(),
            ))
            .unwrap_err()
            .kind(),
        TelegramUpdateInputErrorKind::SpeechAttachmentMissing
    );
    assert_eq!(
        missing_port
            .prepare_input(&request(
                TelegramUpdateInputMode::SpeechToText,
                None,
                None,
                vec![attachment("voice", None)],
            ))
            .unwrap_err()
            .kind(),
        TelegramUpdateInputErrorKind::SpeechFileIdMissing
    );

    let unavailable_files = StubFileExecutor::new(vec![FileOutcome::Success(vec![1, 2])]);
    let unavailable_port = NativeTelegramUpdateInputPort::with_executors(
        &on_config,
        unavailable_files.clone(),
        None::<StubSttExecutor>,
    )
    .unwrap();
    assert_eq!(
        unavailable_port
            .prepare_input(&request(
                TelegramUpdateInputMode::SpeechToText,
                None,
                None,
                vec![attachment("voice", Some("voice-1"))],
            ))
            .unwrap_err()
            .kind(),
        TelegramUpdateInputErrorKind::SpeechBackendUnavailable
    );
    assert_eq!(unavailable_files.requests().len(), 1);
}

#[test]
fn speech_success_uses_typed_config_and_canonical_turn_input_planner() {
    let (_temp, config) = config(true);
    let files = StubFileExecutor::new(vec![FileOutcome::Success(vec![9, 8, 7])]);
    let stt = StubSttExecutor::new(vec![SttOutcome::Success("native transcript".to_string())]);
    let port =
        NativeTelegramUpdateInputPort::with_executors(&config, files, Some(stt.clone())).unwrap();
    let trailing = attachment("document", Some("doc-2"));

    let prepared = port
        .prepare_input(&request(
            TelegramUpdateInputMode::SpeechToText,
            Some("caption"),
            Some("ignored candidate"),
            vec![attachment("voice", Some("voice-1")), trailing.clone()],
        ))
        .expect("speech input");

    assert_eq!(
        prepared.raw_text(),
        Some("caption\n\n[local speech transcript]\nnative transcript")
    );
    assert!(prepared.attachments()[0]["local_path"].is_string());
    assert_eq!(prepared.attachments()[1], trailing);
    let calls = stt.requests();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["operation"], "transcribe");
    assert_eq!(calls[0]["model"], "fixture-model");
    assert_eq!(calls[0]["device"], "cpu");
    assert_eq!(calls[0]["compute_type"], "float32");
    assert_eq!(calls[0]["language"], "zh");
    assert!(Path::new(calls[0]["local_path"].as_str().unwrap()).is_file());
}

#[test]
fn speech_executor_failures_and_contract_failures_are_typed() {
    let (_temp, config) = config(true);
    for (outcome, expected) in [
        (
            SttOutcome::Error(TelegramSttExecutionErrorKind::Unavailable),
            TelegramUpdateInputErrorKind::SpeechBackendUnavailable,
        ),
        (
            SttOutcome::Error(TelegramSttExecutionErrorKind::Timeout),
            TelegramUpdateInputErrorKind::SpeechTimeout,
        ),
        (
            SttOutcome::Error(TelegramSttExecutionErrorKind::Exit),
            TelegramUpdateInputErrorKind::SpeechTranscriptionFailed,
        ),
        (
            SttOutcome::Error(TelegramSttExecutionErrorKind::Empty),
            TelegramUpdateInputErrorKind::SpeechEmpty,
        ),
        (
            SttOutcome::Success(String::new()),
            TelegramUpdateInputErrorKind::SpeechEmpty,
        ),
        (
            SttOutcome::Malformed,
            TelegramUpdateInputErrorKind::SpeechTranscriptionFailed,
        ),
    ] {
        let files = StubFileExecutor::new(vec![FileOutcome::Success(vec![1])]);
        let stt = StubSttExecutor::new(vec![outcome]);
        let port =
            NativeTelegramUpdateInputPort::with_executors(&config, files, Some(stt)).unwrap();
        assert_eq!(
            port.prepare_input(&request(
                TelegramUpdateInputMode::SpeechToText,
                None,
                None,
                vec![attachment("voice", Some("voice-1"))],
            ))
            .unwrap_err()
            .kind(),
            expected
        );
    }
}

#[test]
fn input_port_debug_redacts_credentials_paths_and_model_details() {
    let (temp, config) = config(true);
    let port = NativeTelegramUpdateInputPort::with_executors(
        &config,
        StubFileExecutor::new(Vec::new()),
        Some(StubSttExecutor::new(Vec::new())),
    )
    .unwrap();
    let debug = format!("{port:?}");
    assert!(!debug.contains("private-telegram-token"));
    assert!(!debug.contains("fixture-model"));
    assert!(!debug.contains(temp.path().to_string_lossy().as_ref()));
    assert!(debug.contains("bot_token_exposed: false"));
}

#[test]
fn default_file_executor_flows_real_loopback_download_metadata_through_input_port() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let address = listener.local_addr().expect("loopback address");
    let server = thread::spawn(move || {
        for (index, response) in [
            br#"{"ok":true,"result":{"file_id":"voice-1","file_unique_id":"unique-voice-1","file_size":4,"file_path":"voice/native.ogg"}}"#.to_vec(),
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
    let (_temp, config) = config(false);
    let mut port = NativeTelegramUpdateInputPort::from_config(&config).expect("native input port");
    port.config.api_base_url = Some(format!("http://{address}"));
    port.config.file_base_url = Some(format!("http://{address}"));

    let prepared = port
        .prepare_input(&request(
            TelegramUpdateInputMode::DownloadAttachments,
            None,
            Some("native"),
            vec![attachment("voice", Some("voice-1"))],
        ))
        .expect("native download");
    let local_path = PathBuf::from(prepared.attachments()[0]["local_path"].as_str().unwrap());
    assert_eq!(fs::read(local_path).expect("cached bytes"), [0, 255, 1, 2]);
    assert_eq!(
        prepared.attachments()[0]["telegram_file_path"],
        "voice/native.ogg"
    );
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
