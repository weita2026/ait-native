use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ait_agent_core::{AgentEvent, NativeSocket};
use ait_agent_worker::{
    prepare_worker_run_with_env, AgentEventLoopHostWait, WorkerDiagnostic, WorkerHostEventLoop,
    WorkerHostRuntime, WorkerHttpCompletion, WorkerHttpDispatch, WorkerHttpHandler,
    WorkerHttpHostConfig, WorkerHttpHostRuntime, WorkerHttpRequest, WorkerHttpResponse,
    WorkerPathInputs, WorkerRunContext, WorkerRunRequest, EXIT_RUNTIME_UNAVAILABLE,
};
use tempfile::tempdir;

#[derive(Clone)]
struct TestHandler {
    requests: Rc<RefCell<Vec<WorkerHttpRequest>>>,
    fail: bool,
    response: WorkerHttpResponse,
}

impl TestHandler {
    fn responding(response: WorkerHttpResponse) -> Self {
        Self {
            requests: Rc::default(),
            fail: false,
            response,
        }
    }

    fn failing() -> Self {
        Self {
            requests: Rc::default(),
            fail: true,
            response: WorkerHttpResponse::new(200, Vec::new()),
        }
    }
}

impl WorkerHttpHandler for TestHandler {
    fn handle(
        &mut self,
        request: WorkerHttpRequest,
    ) -> Result<WorkerHttpDispatch, WorkerDiagnostic> {
        self.requests.borrow_mut().push(request);
        if self.fail {
            return Err(WorkerDiagnostic::new(
                "test_handler_secret_failure",
                "secret handler detail must not cross HTTP",
                EXIT_RUNTIME_UNAVAILABLE,
            ));
        }
        Ok(WorkerHttpDispatch::Immediate(self.response.clone()))
    }
}

#[derive(Clone, Default)]
struct DeferredHandlerProbe {
    job_ids: Rc<RefCell<Vec<u64>>>,
    completions: Rc<RefCell<VecDeque<WorkerHttpCompletion>>>,
    inflight: Rc<Cell<usize>>,
    admission_closed: Rc<Cell<bool>>,
    force_detached: Rc<Cell<bool>>,
}

impl DeferredHandlerProbe {
    fn complete(&self, job_id: u64, response: WorkerHttpResponse) {
        self.completions
            .borrow_mut()
            .push_back(WorkerHttpCompletion {
                job_id,
                result: Ok(response),
            });
    }
}

impl WorkerHttpHandler for DeferredHandlerProbe {
    fn handle(
        &mut self,
        _request: WorkerHttpRequest,
    ) -> Result<WorkerHttpDispatch, WorkerDiagnostic> {
        if self.admission_closed.get() {
            return Ok(WorkerHttpDispatch::Immediate(WorkerHttpResponse::text(
                503,
                b"closed".to_vec(),
            )));
        }
        let job_id = u64::try_from(self.job_ids.borrow().len()).unwrap() + 1;
        self.job_ids.borrow_mut().push(job_id);
        self.inflight.set(self.inflight.get() + 1);
        Ok(WorkerHttpDispatch::Deferred { job_id })
    }

    fn poll_completed(&mut self) -> Vec<WorkerHttpCompletion> {
        let completions: Vec<_> = self.completions.borrow_mut().drain(..).collect();
        self.inflight
            .set(self.inflight.get().saturating_sub(completions.len()));
        completions
    }

    fn close_admission(&mut self) {
        self.admission_closed.set(true);
    }

    fn inflight_work_count(&self) -> usize {
        self.inflight.get()
    }

    fn force_shutdown(&mut self) -> Result<(), WorkerDiagnostic> {
        self.admission_closed.set(true);
        self.force_detached.set(true);
        self.inflight.set(0);
        self.completions.borrow_mut().clear();
        Ok(())
    }
}

fn fixture_context() -> (tempfile::TempDir, WorkerRunContext) {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        r#"{"version":1,"workers":{"line/main":{"kind":"line","name":"main","secret":"secret","token":"token"}}}"#,
    )
    .expect("manifest");
    let context = prepare_worker_run_with_env(
        &WorkerRunRequest {
            transport: "line".to_string(),
            worker: "main".to_string(),
            event_loop_backend: "portable_poll".to_string(),
            shard: "0".to_string(),
        },
        &WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: Some(temp.path().to_path_buf()),
            manifest_path_override: None,
        },
        BTreeMap::new(),
    )
    .expect("worker context");
    (temp, context)
}

fn host_config() -> WorkerHttpHostConfig {
    WorkerHttpHostConfig {
        expected_path: "/hook".to_string(),
        listener_token: 41_000,
        connection_token_start: 41_001,
        request_timeout: Duration::from_secs(1),
        ..WorkerHttpHostConfig::default()
    }
}

fn http_response_is_complete(response: &[u8]) -> bool {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let Ok(headers) = std::str::from_utf8(&response[..header_end]) else {
        return false;
    };
    let Some(content_length) = headers.split("\r\n").find_map(|line| {
        line.strip_prefix("Content-Length: ")
            .and_then(|value| value.parse::<usize>().ok())
    }) else {
        return false;
    };
    response.len() >= header_end + 4 + content_length
}

fn drive_client_until_closed<H>(
    context: &WorkerRunContext,
    runtime: &mut WorkerHttpHostRuntime<H>,
    event_loop: &mut dyn WorkerHostEventLoop,
    client: &mut TcpStream,
) -> Vec<u8>
where
    H: WorkerHttpHandler,
{
    client.set_nonblocking(true).expect("nonblocking client");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut response = Vec::new();
    let mut chunk = [0u8; 4_096];
    loop {
        let events = event_loop.wait(Duration::from_millis(5)).expect("poll");
        runtime
            .tick(context, event_loop, &events)
            .expect("HTTP host tick");
        loop {
            match client.read(&mut chunk) {
                Ok(0) => return response,
                Ok(read) => response.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if error.kind() == io::ErrorKind::ConnectionReset
                        && http_response_is_complete(&response) =>
                {
                    return response;
                }
                Err(error) => panic!("client read failed: {error}"),
            }
        }
        assert!(Instant::now() < deadline, "HTTP response timed out");
    }
}

fn exchange(
    config: WorkerHttpHostConfig,
    request: &[u8],
    handler: TestHandler,
) -> (Vec<u8>, Vec<WorkerHttpRequest>) {
    let (_temp, context) = fixture_context();
    let observed = handler.requests.clone();
    let mut runtime = WorkerHttpHostRuntime::new(config, handler);
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let mut client = TcpStream::connect(runtime.local_addr().expect("listener address"))
        .expect("connect client");
    client.write_all(request).expect("write request");

    let response = drive_client_until_closed(&context, &mut runtime, &mut event_loop, &mut client);
    assert_eq!(runtime.inflight_work_count(), 0);
    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop listener");
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish HTTP host");
    let requests = observed.borrow().clone();
    (response, requests)
}

fn assert_status(response: &[u8], status: u16) {
    let prefix = format!("HTTP/1.1 {status} ");
    assert!(response.starts_with(prefix.as_bytes()), "{response:?}");
    assert!(
        response
            .windows(b"Connection: close".len())
            .any(|window| window == b"Connection: close"),
        "{response:?}"
    );
}

#[test]
fn real_loopback_parses_lowercase_headers_and_binary_body_and_serializes_response() {
    let handler = TestHandler::responding(
        WorkerHttpResponse::new(202, vec![0, 1, 2]).with_header("X-Worker-Result", "accepted"),
    );
    let mut request =
        b"POST /hook HTTP/1.1\r\nHost: localhost\r\nX-LINE-Signature: signed\r\nContent-Length: 3\r\n\r\n"
            .to_vec();
    request.extend_from_slice(&[0, 255, 10]);

    let (response, requests) = exchange(host_config(), &request, handler);

    assert_status(&response, 202);
    assert!(response
        .windows(b"Content-Length: 3".len())
        .any(|window| window == b"Content-Length: 3"));
    assert!(response.ends_with(&[0, 1, 2]));
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/hook");
    assert_eq!(requests[0].version, "HTTP/1.1");
    assert_eq!(requests[0].headers["x-line-signature"], "signed");
    assert!(!requests[0].headers.contains_key("X-LINE-Signature"));
    assert_eq!(requests[0].body, [0, 255, 10]);
}

#[test]
fn real_loopback_returns_stable_gate_parse_limit_and_handler_errors() {
    struct Case {
        expected_status: u16,
        config: WorkerHttpHostConfig,
        request: Vec<u8>,
        handler: TestHandler,
        expected_handler_calls: usize,
    }

    let mut small_header_config = host_config();
    small_header_config.max_header_bytes = 48;
    let mut small_body_config = host_config();
    small_body_config.max_body_bytes = 2;
    let cases = [
        Case {
            expected_status: 404,
            config: host_config(),
            request: b"POST /wrong HTTP/1.1\r\nContent-Length: 0\r\n\r\n".to_vec(),
            handler: TestHandler::responding(WorkerHttpResponse::new(200, Vec::new())),
            expected_handler_calls: 0,
        },
        Case {
            expected_status: 405,
            config: host_config(),
            request: b"GET /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n".to_vec(),
            handler: TestHandler::responding(WorkerHttpResponse::new(200, Vec::new())),
            expected_handler_calls: 0,
        },
        Case {
            expected_status: 400,
            config: host_config(),
            request: b"POST /hook HTTP/1.1\r\nMalformed Header\r\n\r\n".to_vec(),
            handler: TestHandler::responding(WorkerHttpResponse::new(200, Vec::new())),
            expected_handler_calls: 0,
        },
        Case {
            expected_status: 431,
            config: small_header_config,
            request: format!(
                "POST /hook HTTP/1.1\r\nX-Oversized: {}\r\nContent-Length: 0\r\n\r\n",
                "x".repeat(80)
            )
            .into_bytes(),
            handler: TestHandler::responding(WorkerHttpResponse::new(200, Vec::new())),
            expected_handler_calls: 0,
        },
        Case {
            expected_status: 413,
            config: small_body_config,
            request: b"POST /hook HTTP/1.1\r\nContent-Length: 3\r\n\r\nabc".to_vec(),
            handler: TestHandler::responding(WorkerHttpResponse::new(200, Vec::new())),
            expected_handler_calls: 0,
        },
        Case {
            expected_status: 500,
            config: host_config(),
            request: b"POST /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n".to_vec(),
            handler: TestHandler::failing(),
            expected_handler_calls: 1,
        },
    ];

    for case in cases {
        let (response, requests) = exchange(case.config, &case.request, case.handler);
        assert_status(&response, case.expected_status);
        assert_eq!(requests.len(), case.expected_handler_calls);
        assert!(!String::from_utf8_lossy(&response).contains("secret handler detail"));
        if case.expected_status == 405 {
            assert!(response
                .windows(b"Allow: POST".len())
                .any(|window| window == b"Allow: POST"));
        }
    }
}

#[test]
fn real_loopback_times_out_an_incomplete_request() {
    let (_temp, context) = fixture_context();
    let mut config = host_config();
    config.request_timeout = Duration::from_millis(25);
    let handler = TestHandler::responding(WorkerHttpResponse::new(200, Vec::new()));
    let observed = handler.requests.clone();
    let mut runtime = WorkerHttpHostRuntime::new(config, handler);
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let mut client = TcpStream::connect(runtime.local_addr().expect("listener address"))
        .expect("connect client");
    client
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 3\r\n")
        .expect("write partial request");

    let response = drive_client_until_closed(&context, &mut runtime, &mut event_loop, &mut client);

    assert_status(&response, 408);
    assert!(observed.borrow().is_empty());
    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop listener");
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish HTTP host");
}

#[test]
fn real_loopback_applies_connection_backpressure_without_admitting_extra_work() {
    let (_temp, context) = fixture_context();
    let mut config = host_config();
    config.max_connections = 1;
    let handler = TestHandler::responding(WorkerHttpResponse::new(200, Vec::new()));
    let mut runtime = WorkerHttpHostRuntime::new(config, handler);
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let address = runtime.local_addr().expect("listener address");

    let mut first = TcpStream::connect(address).expect("first client");
    first
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 3\r\n")
        .expect("write partial first request");
    let events = event_loop
        .wait(Duration::from_millis(50))
        .expect("listener event");
    runtime
        .tick(&context, &mut event_loop, &events)
        .expect("accept first client");
    assert_eq!(runtime.inflight_work_count(), 1);

    let mut second = TcpStream::connect(address).expect("second client");
    second
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("write second request");
    let response = drive_client_until_closed(&context, &mut runtime, &mut event_loop, &mut second);

    assert_status(&response, 503);
    assert!(response.ends_with(b"Service Unavailable\n"));
    assert_eq!(runtime.inflight_work_count(), 1);
    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop listener");
    assert!(!runtime.is_accepting());
    assert!(TcpStream::connect(address).is_err());
    runtime
        .force_shutdown(&context, &mut event_loop)
        .expect("force cleanup");
    assert_eq!(runtime.inflight_work_count(), 0);
}

fn drive_until_deferred_jobs(
    context: &WorkerRunContext,
    runtime: &mut WorkerHttpHostRuntime<DeferredHandlerProbe>,
    event_loop: &mut dyn WorkerHostEventLoop,
    probe: &DeferredHandlerProbe,
    expected: usize,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while probe.job_ids.borrow().len() < expected {
        let events = event_loop.wait(Duration::from_millis(10)).expect("poll");
        runtime
            .tick(context, event_loop, &events)
            .expect("deferred HTTP tick");
        assert!(Instant::now() < deadline, "deferred request timed out");
    }
}

#[test]
fn deferred_response_correlates_one_job_and_drains_after_shutdown() {
    let (_temp, context) = fixture_context();
    let probe = DeferredHandlerProbe::default();
    let mut runtime = WorkerHttpHostRuntime::new(host_config(), probe.clone());
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let address = runtime.local_addr().expect("listener address");
    let mut client = TcpStream::connect(address).expect("client");
    client
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("request");
    drive_until_deferred_jobs(&context, &mut runtime, &mut event_loop, &probe, 1);
    assert_eq!(runtime.inflight_work_count(), 1);

    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop admission");
    assert!(probe.admission_closed.get());
    assert!(TcpStream::connect(address).is_err());
    probe.complete(1, WorkerHttpResponse::json(202, br#"{"ok":true}"#.to_vec()));

    let response = drive_client_until_closed(&context, &mut runtime, &mut event_loop, &mut client);

    assert_status(&response, 202);
    assert!(response.ends_with(br#"{"ok":true}"#));
    assert_eq!(runtime.inflight_work_count(), 0);
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish shutdown");
}

#[test]
fn deferred_responses_keep_connection_correlation_when_jobs_finish_out_of_order() {
    let (_temp, context) = fixture_context();
    let probe = DeferredHandlerProbe::default();
    let mut runtime = WorkerHttpHostRuntime::new(host_config(), probe.clone());
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let address = runtime.local_addr().expect("listener address");
    let mut first = TcpStream::connect(address).expect("first client");
    first
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("first request");
    drive_until_deferred_jobs(&context, &mut runtime, &mut event_loop, &probe, 1);
    let mut second = TcpStream::connect(address).expect("second client");
    second
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("second request");
    drive_until_deferred_jobs(&context, &mut runtime, &mut event_loop, &probe, 2);

    probe.complete(2, WorkerHttpResponse::text(202, b"second".to_vec()));
    probe.complete(1, WorkerHttpResponse::text(201, b"first".to_vec()));
    let first_response =
        drive_client_until_closed(&context, &mut runtime, &mut event_loop, &mut first);
    let second_response =
        drive_client_until_closed(&context, &mut runtime, &mut event_loop, &mut second);

    assert_status(&first_response, 201);
    assert!(first_response.ends_with(b"first"));
    assert_status(&second_response, 202);
    assert!(second_response.ends_with(b"second"));
    assert_eq!(runtime.inflight_work_count(), 0);
    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop admission");
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish shutdown");
}

#[test]
fn deferred_deadline_discards_late_completion_without_resurrection() {
    let (_temp, context) = fixture_context();
    let mut config = host_config();
    config.request_timeout = Duration::from_millis(25);
    let probe = DeferredHandlerProbe::default();
    let mut runtime = WorkerHttpHostRuntime::new(config, probe.clone());
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let mut client =
        TcpStream::connect(runtime.local_addr().expect("listener address")).expect("client");
    client
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("request");
    drive_until_deferred_jobs(&context, &mut runtime, &mut event_loop, &probe, 1);

    let response = drive_client_until_closed(&context, &mut runtime, &mut event_loop, &mut client);

    assert_status(&response, 408);
    assert_eq!(runtime.inflight_work_count(), 1);
    probe.complete(1, WorkerHttpResponse::new(200, b"late".to_vec()));
    runtime
        .tick(&context, &mut event_loop, &[])
        .expect("poll late completion");
    assert_eq!(runtime.inflight_work_count(), 0);
    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop admission");
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish shutdown");
}

#[test]
fn deferred_force_shutdown_detaches_handler_and_connection() {
    let (_temp, context) = fixture_context();
    let probe = DeferredHandlerProbe::default();
    let mut runtime = WorkerHttpHostRuntime::new(host_config(), probe.clone());
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let mut client =
        TcpStream::connect(runtime.local_addr().expect("listener address")).expect("client");
    client
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("request");
    drive_until_deferred_jobs(&context, &mut runtime, &mut event_loop, &probe, 1);

    runtime
        .force_shutdown(&context, &mut event_loop)
        .expect("force shutdown");

    assert!(probe.force_detached.get());
    assert_eq!(runtime.inflight_work_count(), 0);
}

struct RecordingEventLoop {
    inner: AgentEventLoopHostWait,
    registrations: BTreeMap<u64, (NativeSocket, bool)>,
    unregistered: BTreeSet<u64>,
}

impl RecordingEventLoop {
    fn new(context: &WorkerRunContext) -> Self {
        Self {
            inner: AgentEventLoopHostWait::new(context).expect("recording event loop"),
            registrations: BTreeMap::new(),
            unregistered: BTreeSet::new(),
        }
    }
}

impl WorkerHostEventLoop for RecordingEventLoop {
    fn register_readable(&mut self, token: u64, fd: NativeSocket) -> Result<(), WorkerDiagnostic> {
        self.inner.register_readable(token, fd)?;
        self.registrations.insert(token, (fd, false));
        Ok(())
    }

    fn register_read_write(
        &mut self,
        token: u64,
        fd: NativeSocket,
    ) -> Result<(), WorkerDiagnostic> {
        self.inner.register_read_write(token, fd)?;
        self.registrations.insert(token, (fd, true));
        Ok(())
    }

    fn unregister(&mut self, token: u64) -> Result<(), WorkerDiagnostic> {
        self.inner.unregister(token)?;
        self.registrations.remove(&token);
        self.unregistered.insert(token);
        Ok(())
    }

    fn wait(&mut self, timeout: Duration) -> Result<Vec<AgentEvent>, WorkerDiagnostic> {
        self.inner.wait(timeout)
    }
}

#[test]
fn lifecycle_stops_admission_drains_and_unregisters_every_descriptor() {
    let (_temp, context) = fixture_context();
    let mut config = host_config();
    config.listener_token = 51_000;
    config.connection_token_start = 51_001;
    let handler = TestHandler::responding(WorkerHttpResponse::text(200, b"ok".to_vec()));
    let mut runtime = WorkerHttpHostRuntime::new(config, handler);
    let mut event_loop = RecordingEventLoop::new(&context);
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let address = runtime.local_addr().expect("listener address");
    assert_eq!(
        event_loop.registrations.keys().copied().collect::<Vec<_>>(),
        [51_000]
    );

    let mut client = TcpStream::connect(address).expect("client");
    client
        .write_all(b"POST /hook HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("request");
    let events = event_loop
        .wait(Duration::from_millis(50))
        .expect("listener readiness");
    runtime
        .tick(&context, &mut event_loop, &events)
        .expect("accept client");
    assert_eq!(runtime.inflight_work_count(), 1);
    assert_eq!(
        event_loop.registrations.keys().copied().collect::<Vec<_>>(),
        [51_000, 51_001]
    );

    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop admission");
    assert!(!runtime.is_accepting());
    assert!(!event_loop.registrations.contains_key(&51_000));
    assert!(TcpStream::connect(address).is_err());
    let events = event_loop
        .wait(Duration::from_millis(50))
        .expect("connection readiness");
    runtime
        .tick(&context, &mut event_loop, &events)
        .expect("drain client");
    assert_eq!(runtime.inflight_work_count(), 0);
    assert!(event_loop.registrations.is_empty());
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("graceful cleanup");
    assert!(event_loop.unregistered.contains(&51_000));
    assert!(event_loop.unregistered.contains(&51_001));

    let mut response = Vec::new();
    client.read_to_end(&mut response).expect("response");
    assert_status(&response, 200);
}

#[test]
fn lifecycle_force_cleanup_unregisters_listener_and_inflight_connection() {
    let (_temp, context) = fixture_context();
    let mut config = host_config();
    config.listener_token = 52_000;
    config.connection_token_start = 52_001;
    let handler = TestHandler::responding(WorkerHttpResponse::new(200, Vec::new()));
    let mut runtime = WorkerHttpHostRuntime::new(config, handler);
    let mut event_loop = RecordingEventLoop::new(&context);
    runtime
        .start(&context, &mut event_loop)
        .expect("start HTTP host");
    let _client =
        TcpStream::connect(runtime.local_addr().expect("listener address")).expect("client");
    let events = event_loop
        .wait(Duration::from_millis(50))
        .expect("listener readiness");
    runtime
        .tick(&context, &mut event_loop, &events)
        .expect("accept client");
    assert_eq!(runtime.inflight_work_count(), 1);

    runtime
        .force_shutdown(&context, &mut event_loop)
        .expect("force cleanup");

    assert_eq!(runtime.inflight_work_count(), 0);
    assert!(event_loop.registrations.is_empty());
    assert_eq!(event_loop.unregistered, BTreeSet::from([52_000, 52_001]));
}
