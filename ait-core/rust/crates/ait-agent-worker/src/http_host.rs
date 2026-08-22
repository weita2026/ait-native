use std::collections::{btree_map::Entry, BTreeMap};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use ait_agent_core::{tcp_listener_native_socket, tcp_stream_native_socket, AgentEvent};

use crate::{
    WorkerDiagnostic, WorkerHostEventLoop, WorkerHostRuntime, WorkerRunContext,
    EXIT_INVALID_CONFIGURATION, EXIT_RUNTIME_UNAVAILABLE,
};

const DEFAULT_MAX_HEADER_BYTES: usize = 32 * 1024;
const DEFAULT_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_MAX_CONNECTIONS: usize = 1_024;
const DEFAULT_MAX_ACCEPTS_PER_TICK: usize = 64;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_LISTENER_TOKEN: u64 = 0xA17_0000;
const DEFAULT_CONNECTION_TOKEN_START: u64 = DEFAULT_LISTENER_TOKEN + 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHttpHostConfig {
    pub bind_addr: SocketAddr,
    pub expected_method: String,
    pub expected_path: String,
    pub enforce_expected_path: bool,
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
    pub max_connections: usize,
    pub max_accepts_per_tick: usize,
    pub request_timeout: Duration,
    pub listener_token: u64,
    pub connection_token_start: u64,
}

impl Default for WorkerHttpHostConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            expected_method: "POST".to_string(),
            expected_path: "/".to_string(),
            enforce_expected_path: true,
            max_header_bytes: DEFAULT_MAX_HEADER_BYTES,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            max_accepts_per_tick: DEFAULT_MAX_ACCEPTS_PER_TICK,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            listener_token: DEFAULT_LISTENER_TOKEN,
            connection_token_start: DEFAULT_CONNECTION_TOKEN_START,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHttpRequest {
    pub method: String,
    pub path: String,
    pub version: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
    pub peer_addr: SocketAddr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHttpResponse {
    pub status_code: u16,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

impl WorkerHttpResponse {
    pub fn new(status_code: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status_code,
            headers: BTreeMap::new(),
            body: body.into(),
        }
    }

    pub fn text(status_code: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::new(status_code, body).with_header("Content-Type", "text/plain; charset=utf-8")
    }

    pub fn json(status_code: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::new(status_code, body).with_header("Content-Type", "application/json")
    }

    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }
}

#[derive(Debug)]
pub enum WorkerHttpDispatch {
    Immediate(WorkerHttpResponse),
    Deferred { job_id: u64 },
}

#[derive(Debug)]
pub struct WorkerHttpCompletion {
    pub job_id: u64,
    pub result: Result<WorkerHttpResponse, WorkerDiagnostic>,
}

pub trait WorkerHttpHandler {
    fn handle(
        &mut self,
        request: WorkerHttpRequest,
    ) -> Result<WorkerHttpDispatch, WorkerDiagnostic>;

    fn poll_completed(&mut self) -> Vec<WorkerHttpCompletion> {
        Vec::new()
    }

    fn close_admission(&mut self) {}

    fn inflight_work_count(&self) -> usize {
        0
    }

    fn finish_shutdown(&mut self) -> Result<(), WorkerDiagnostic> {
        Ok(())
    }

    fn force_shutdown(&mut self) -> Result<(), WorkerDiagnostic> {
        Ok(())
    }
}

impl<F> WorkerHttpHandler for F
where
    F: FnMut(WorkerHttpRequest) -> Result<WorkerHttpResponse, WorkerDiagnostic>,
{
    fn handle(
        &mut self,
        request: WorkerHttpRequest,
    ) -> Result<WorkerHttpDispatch, WorkerDiagnostic> {
        self(request).map(WorkerHttpDispatch::Immediate)
    }
}

pub struct WorkerHttpHostRuntime<H> {
    config: WorkerHttpHostConfig,
    handler: H,
    listener: Option<TcpListener>,
    bound_addr: Option<SocketAddr>,
    connections: BTreeMap<u64, HttpConnection>,
    pending_jobs: BTreeMap<u64, u64>,
    next_connection_token: u64,
    accepting: bool,
    listener_pending: bool,
    started: bool,
}

impl<H> WorkerHttpHostRuntime<H> {
    pub fn new(config: WorkerHttpHostConfig, handler: H) -> Self {
        let next_connection_token = config.connection_token_start;
        Self {
            config,
            handler,
            listener: None,
            bound_addr: None,
            connections: BTreeMap::new(),
            pending_jobs: BTreeMap::new(),
            next_connection_token,
            accepting: false,
            listener_pending: false,
            started: false,
        }
    }

    pub fn local_addr(&self) -> Option<SocketAddr> {
        self.bound_addr
    }

    pub fn is_accepting(&self) -> bool {
        self.accepting
    }

    pub fn handler(&self) -> &H {
        &self.handler
    }
}

impl<H> WorkerHttpHostRuntime<H>
where
    H: WorkerHttpHandler,
{
    fn validate_config(&self) -> Result<(), WorkerDiagnostic> {
        let invalid = |field: &'static str, message: &'static str| {
            WorkerDiagnostic::new(
                "worker_http_host_config_invalid",
                message,
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("field", field)
        };
        if !is_http_token(&self.config.expected_method) {
            return Err(invalid(
                "expected_method",
                "The worker HTTP method must be a non-empty HTTP token.",
            ));
        }
        if !valid_request_target(&self.config.expected_path) {
            return Err(invalid(
                "expected_path",
                "The worker HTTP path must be an absolute ASCII request target.",
            ));
        }
        if self.config.max_header_bytes < 4 {
            return Err(invalid(
                "max_header_bytes",
                "The worker HTTP header limit must be at least four bytes.",
            ));
        }
        if self.config.max_connections == 0 {
            return Err(invalid(
                "max_connections",
                "The worker HTTP connection limit must be greater than zero.",
            ));
        }
        if self.config.max_accepts_per_tick == 0 {
            return Err(invalid(
                "max_accepts_per_tick",
                "The worker HTTP accept limit must be greater than zero.",
            ));
        }
        if self.config.request_timeout.is_zero()
            || Instant::now()
                .checked_add(self.config.request_timeout)
                .is_none()
        {
            return Err(invalid(
                "request_timeout",
                "The worker HTTP request timeout must be positive and representable.",
            ));
        }
        if self.config.listener_token == self.config.connection_token_start {
            return Err(invalid(
                "connection_token_start",
                "The worker HTTP listener and first connection tokens must differ.",
            ));
        }
        Ok(())
    }

    fn accept_ready(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        self.listener_pending = false;
        let mut accepted = 0usize;
        while accepted < self.config.max_accepts_per_tick {
            let result = match self.listener.as_ref() {
                Some(listener) => listener.accept(),
                None => return Ok(()),
            };
            let (stream, peer_addr) = match result {
                Ok(connection) => connection,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(runtime_error(
                        "worker_http_accept_failed",
                        format!("Rust worker HTTP accept failed: {error}"),
                    ))
                }
            };
            accepted += 1;
            stream.set_nonblocking(true).map_err(|error| {
                runtime_error(
                    "worker_http_connection_setup_failed",
                    format!("Cannot make an accepted worker HTTP connection nonblocking: {error}"),
                )
            })?;
            let _ = stream.set_nodelay(true);

            if self.connections.len() >= self.config.max_connections {
                write_capacity_response(stream);
                continue;
            }

            let token = self.allocate_connection_token()?;
            let deadline = Instant::now()
                .checked_add(self.config.request_timeout)
                .ok_or_else(|| {
                    runtime_error(
                        "worker_http_deadline_unavailable",
                        "Cannot represent the configured worker HTTP request deadline.",
                    )
                })?;
            event_loop.register_readable(token, tcp_stream_native_socket(&stream))?;
            self.connections
                .insert(token, HttpConnection::new(stream, peer_addr, deadline));
        }
        self.listener_pending = true;
        Ok(())
    }

    fn allocate_connection_token(&mut self) -> Result<u64, WorkerDiagnostic> {
        let mut candidate = self.next_connection_token;
        for _ in 0..=self.connections.len() {
            if candidate != self.config.listener_token && !self.connections.contains_key(&candidate)
            {
                self.next_connection_token = candidate.wrapping_add(1);
                return Ok(candidate);
            }
            candidate = candidate.wrapping_add(1);
        }
        Err(runtime_error(
            "worker_http_connection_token_unavailable",
            "Cannot allocate a worker HTTP event-loop token.",
        ))
    }

    fn process_connection_event(
        &mut self,
        token: u64,
        event: &AgentEvent,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let Some(mut connection) = self.connections.remove(&token) else {
            return Ok(());
        };
        let mut peer_closed = false;
        if connection.response.is_none()
            && connection.pending_job_id.is_none()
            && (event.readable || event.hangup)
        {
            peer_closed = self.read_connection(token, &mut connection);
        }
        let write_state = if connection.response.is_some() {
            flush_response(&mut connection)
        } else {
            WriteState::Pending
        };
        let should_close = matches!(write_state, WriteState::Complete | WriteState::Closed)
            || (peer_closed && connection.response.is_none())
            || (event.hangup && connection.response.is_none());
        if should_close {
            self.forget_pending_job(&mut connection);
            return event_loop.unregister(token);
        }
        if connection.response.is_some() && !connection.write_interest {
            if let Err(error) =
                event_loop.register_read_write(token, tcp_stream_native_socket(&connection.stream))
            {
                let _ = event_loop.unregister(token);
                return Err(error);
            }
            connection.write_interest = true;
        }
        self.connections.insert(token, connection);
        Ok(())
    }

    fn expire_connection(
        &mut self,
        token: u64,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let Some(mut connection) = self.connections.remove(&token) else {
            return Ok(());
        };
        self.forget_pending_job(&mut connection);
        connection.queue_response(standard_error_response(408));
        match flush_response(&mut connection) {
            WriteState::Complete | WriteState::Closed => event_loop.unregister(token),
            WriteState::Pending => {
                if let Err(error) = event_loop
                    .register_read_write(token, tcp_stream_native_socket(&connection.stream))
                {
                    let _ = event_loop.unregister(token);
                    return Err(error);
                }
                connection.write_interest = true;
                self.connections.insert(token, connection);
                Ok(())
            }
        }
    }

    fn read_connection(&mut self, token: u64, connection: &mut HttpConnection) -> bool {
        let mut chunk = [0u8; 8 * 1024];
        loop {
            match connection.stream.read(&mut chunk) {
                Ok(0) => return true,
                Ok(read) => {
                    connection.read_buffer.extend_from_slice(&chunk[..read]);
                    self.prepare_response(token, connection);
                    if connection.response.is_some() || connection.pending_job_id.is_some() {
                        return false;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return false,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return true,
            }
        }
    }

    fn prepare_response(&mut self, token: u64, connection: &mut HttpConnection) {
        if connection.request_head.is_none() {
            let Some(header_end) = find_header_end(&connection.read_buffer) else {
                if connection.read_buffer.len() >= self.config.max_header_bytes {
                    connection.queue_response(standard_error_response(431));
                }
                return;
            };
            let body_offset = header_end + 4;
            if body_offset > self.config.max_header_bytes {
                connection.queue_response(standard_error_response(431));
                return;
            }
            let mut head = match parse_request_head(
                &connection.read_buffer[..header_end],
                body_offset,
                self.config.max_body_bytes,
            ) {
                Ok(head) => head,
                Err(ParseFailure::Malformed) => {
                    connection.queue_response(standard_error_response(400));
                    return;
                }
                Err(ParseFailure::BodyTooLarge) => {
                    connection.queue_response(standard_error_response(413));
                    return;
                }
            };
            if self.config.enforce_expected_path && head.path != self.config.expected_path {
                connection.queue_response(standard_error_response(404));
                return;
            }
            if head.method != self.config.expected_method {
                let response = standard_error_response(405)
                    .with_header("Allow", self.config.expected_method.clone());
                connection.queue_response(response);
                return;
            }
            head.body_offset = body_offset;
            connection.request_head = Some(head);
        }

        let Some(head) = connection.request_head.as_ref() else {
            return;
        };
        let Some(request_end) = head.body_offset.checked_add(head.content_length) else {
            connection.queue_response(standard_error_response(413));
            return;
        };
        if connection.read_buffer.len() < request_end {
            return;
        }
        let head = connection
            .request_head
            .take()
            .expect("request head was checked above");
        let request = WorkerHttpRequest {
            method: head.method,
            path: head.path,
            version: head.version,
            headers: head.headers,
            body: connection.read_buffer[head.body_offset..request_end].to_vec(),
            peer_addr: connection.peer_addr,
        };
        let dispatch = self.handler.handle(request);
        match dispatch {
            Ok(WorkerHttpDispatch::Immediate(response)) => connection.queue_response(response),
            Ok(WorkerHttpDispatch::Deferred { job_id }) => match self.pending_jobs.entry(job_id) {
                Entry::Vacant(entry) => {
                    entry.insert(token);
                    connection.pending_job_id = Some(job_id);
                }
                Entry::Occupied(_) => {
                    connection.queue_response(standard_error_response(500));
                }
            },
            Err(_) => connection.queue_response(standard_error_response(500)),
        }
    }

    fn forget_pending_job(&mut self, connection: &mut HttpConnection) {
        if let Some(job_id) = connection.pending_job_id.take() {
            self.pending_jobs.remove(&job_id);
        }
    }

    fn poll_handler_completions(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        for completion in self.handler.poll_completed() {
            let Some(token) = self.pending_jobs.remove(&completion.job_id) else {
                continue;
            };
            let Some(mut connection) = self.connections.remove(&token) else {
                continue;
            };
            if connection.pending_job_id != Some(completion.job_id) {
                self.connections.insert(token, connection);
                continue;
            }
            connection.pending_job_id = None;
            let response = completion
                .result
                .unwrap_or_else(|_| standard_error_response(500));
            connection.queue_response(response);
            match flush_response(&mut connection) {
                WriteState::Complete | WriteState::Closed => event_loop.unregister(token)?,
                WriteState::Pending => {
                    if !connection.write_interest {
                        if let Err(error) = event_loop.register_read_write(
                            token,
                            tcp_stream_native_socket(&connection.stream),
                        ) {
                            let _ = event_loop.unregister(token);
                            return Err(error);
                        }
                        connection.write_interest = true;
                    }
                    self.connections.insert(token, connection);
                }
            }
        }
        Ok(())
    }

    fn stop_listener(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        self.accepting = false;
        self.listener_pending = false;
        if self.listener.take().is_some() {
            event_loop.unregister(self.config.listener_token)
        } else {
            Ok(())
        }
    }

    fn cleanup_all(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let mut first_error = self.stop_listener(event_loop).err();
        let tokens: Vec<u64> = self.connections.keys().copied().collect();
        for token in tokens {
            if let Some(mut connection) = self.connections.remove(&token) {
                self.forget_pending_job(&mut connection);
            }
            if let Err(error) = event_loop.unregister(token) {
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        self.pending_jobs.clear();
        self.started = false;
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl<H> WorkerHostRuntime for WorkerHttpHostRuntime<H>
where
    H: WorkerHttpHandler,
{
    fn start(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        if self.started {
            return Err(runtime_error(
                "worker_http_host_already_started",
                "The Rust worker HTTP host is already started.",
            ));
        }
        self.validate_config()?;
        let listener = TcpListener::bind(self.config.bind_addr).map_err(|error| {
            runtime_error(
                "worker_http_bind_failed",
                format!(
                    "Cannot bind the Rust worker HTTP listener at {}: {error}",
                    self.config.bind_addr
                ),
            )
            .with_detail("bind_addr", self.config.bind_addr.to_string())
        })?;
        listener.set_nonblocking(true).map_err(|error| {
            runtime_error(
                "worker_http_listener_setup_failed",
                format!("Cannot make the Rust worker HTTP listener nonblocking: {error}"),
            )
        })?;
        let bound_addr = listener.local_addr().map_err(|error| {
            runtime_error(
                "worker_http_listener_setup_failed",
                format!("Cannot inspect the Rust worker HTTP listener address: {error}"),
            )
        })?;
        event_loop.register_readable(
            self.config.listener_token,
            tcp_listener_native_socket(&listener),
        )?;
        self.listener = Some(listener);
        self.bound_addr = Some(bound_addr);
        self.next_connection_token = self.config.connection_token_start;
        self.accepting = true;
        self.listener_pending = false;
        self.started = true;
        Ok(())
    }

    fn tick(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
        events: &[AgentEvent],
    ) -> Result<(), WorkerDiagnostic> {
        self.poll_handler_completions(event_loop)?;
        let listener_ready = events.iter().any(|event| {
            event.token == self.config.listener_token && (event.readable || event.hangup)
        });
        if self.accepting && (self.listener_pending || listener_ready) {
            self.accept_ready(event_loop)?;
        }

        for event in events {
            if event.token != self.config.listener_token {
                self.process_connection_event(event.token, event, event_loop)?;
            }
        }

        let now = Instant::now();
        let expired: Vec<u64> = self
            .connections
            .iter()
            .filter_map(|(token, connection)| {
                (connection.response.is_none() && now >= connection.deadline).then_some(*token)
            })
            .collect();
        for token in expired {
            self.expire_connection(token, event_loop)?;
        }
        Ok(())
    }

    fn request_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
        _signal: i32,
    ) -> Result<(), WorkerDiagnostic> {
        self.handler.close_admission();
        self.stop_listener(event_loop)
    }

    fn inflight_work_count(&self) -> usize {
        self.connections.len()
            + self
                .handler
                .inflight_work_count()
                .saturating_sub(self.pending_jobs.len())
    }

    fn finish_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let handler_error = self.handler.finish_shutdown().err();
        let cleanup_error = self.cleanup_all(event_loop).err();
        handler_error.or(cleanup_error).map_or(Ok(()), Err)
    }

    fn force_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let handler_error = self.handler.force_shutdown().err();
        let cleanup_error = self.cleanup_all(event_loop).err();
        handler_error.or(cleanup_error).map_or(Ok(()), Err)
    }
}

struct HttpConnection {
    stream: TcpStream,
    peer_addr: SocketAddr,
    deadline: Instant,
    read_buffer: Vec<u8>,
    request_head: Option<ParsedRequestHead>,
    pending_job_id: Option<u64>,
    response: Option<Vec<u8>>,
    response_offset: usize,
    write_interest: bool,
}

impl HttpConnection {
    fn new(stream: TcpStream, peer_addr: SocketAddr, deadline: Instant) -> Self {
        Self {
            stream,
            peer_addr,
            deadline,
            read_buffer: Vec::new(),
            request_head: None,
            pending_job_id: None,
            response: None,
            response_offset: 0,
            write_interest: false,
        }
    }

    fn queue_response(&mut self, response: WorkerHttpResponse) {
        self.response =
            Some(serialize_response(&response).unwrap_or_else(|_| internal_server_error_bytes()));
        self.response_offset = 0;
    }
}

struct ParsedRequestHead {
    method: String,
    path: String,
    version: String,
    headers: BTreeMap<String, String>,
    content_length: usize,
    body_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseFailure {
    Malformed,
    BodyTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteState {
    Pending,
    Complete,
    Closed,
}

fn parse_request_head(
    bytes: &[u8],
    body_offset: usize,
    max_body_bytes: usize,
) -> Result<ParsedRequestHead, ParseFailure> {
    let text = std::str::from_utf8(bytes).map_err(|_| ParseFailure::Malformed)?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(ParseFailure::Malformed)?;
    let parts: Vec<&str> = request_line.split(' ').collect();
    if parts.len() != 3
        || parts.iter().any(|part| part.is_empty())
        || !is_http_token(parts[0])
        || !valid_request_target(parts[1])
        || parts[2] != "HTTP/1.1"
    {
        return Err(ParseFailure::Malformed);
    }

    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        if line.is_empty() || line.starts_with([' ', '\t']) {
            return Err(ParseFailure::Malformed);
        }
        let (name, raw_value) = line.split_once(':').ok_or(ParseFailure::Malformed)?;
        if !is_http_token(name) {
            return Err(ParseFailure::Malformed);
        }
        let value = raw_value.trim_matches([' ', '\t']);
        if !valid_header_value(value) {
            return Err(ParseFailure::Malformed);
        }
        let name = name.to_ascii_lowercase();
        if let Some(existing) = headers.get_mut(&name) {
            if name == "content-length" {
                if existing != value {
                    return Err(ParseFailure::Malformed);
                }
            } else {
                existing.push_str(", ");
                existing.push_str(value);
            }
        } else {
            headers.insert(name, value.to_string());
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err(ParseFailure::Malformed);
    }
    let content_length = match headers.get("content-length") {
        Some(value) => parse_content_length(value, max_body_bytes)?,
        None => 0,
    };
    Ok(ParsedRequestHead {
        method: parts[0].to_string(),
        path: parts[1].to_string(),
        version: parts[2].to_string(),
        headers,
        content_length,
        body_offset,
    })
}

fn parse_content_length(value: &str, max_body_bytes: usize) -> Result<usize, ParseFailure> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseFailure::Malformed);
    }
    let mut length = 0usize;
    for digit in value.bytes() {
        length = length
            .checked_mul(10)
            .and_then(|current| current.checked_add(usize::from(digit - b'0')))
            .ok_or(ParseFailure::BodyTooLarge)?;
        if length > max_body_bytes {
            return Err(ParseFailure::BodyTooLarge);
        }
    }
    Ok(length)
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn is_http_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn valid_request_target(value: &str) -> bool {
    value.starts_with('/')
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| !byte.is_ascii_control() && byte != b' ')
}

fn valid_header_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte == b'\t' || (!byte.is_ascii_control() && byte != 0x7f))
}

fn standard_error_response(status_code: u16) -> WorkerHttpResponse {
    let body = match status_code {
        400 => "Bad Request\n",
        404 => "Not Found\n",
        405 => "Method Not Allowed\n",
        408 => "Request Timeout\n",
        413 => "Payload Too Large\n",
        431 => "Request Header Fields Too Large\n",
        500 => "Internal Server Error\n",
        503 => "Service Unavailable\n",
        _ => "HTTP Error\n",
    };
    WorkerHttpResponse::text(status_code, body.as_bytes().to_vec())
}

fn serialize_response(response: &WorkerHttpResponse) -> Result<Vec<u8>, ()> {
    if !(100..=599).contains(&response.status_code) {
        return Err(());
    }
    let mut output = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status_code,
        reason_phrase(response.status_code),
        response.body.len()
    )
    .into_bytes();
    for (name, value) in &response.headers {
        if name.eq_ignore_ascii_case("content-length") || name.eq_ignore_ascii_case("connection") {
            continue;
        }
        if !is_http_token(name) || !valid_header_value(value) {
            return Err(());
        }
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(b": ");
        output.extend_from_slice(value.as_bytes());
        output.extend_from_slice(b"\r\n");
    }
    output.extend_from_slice(b"\r\n");
    output.extend_from_slice(&response.body);
    Ok(output)
}

fn reason_phrase(status_code: u16) -> &'static str {
    match status_code {
        100 => "Continue",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

fn flush_response(connection: &mut HttpConnection) -> WriteState {
    let Some(response) = connection.response.as_ref() else {
        return WriteState::Pending;
    };
    while connection.response_offset < response.len() {
        match connection
            .stream
            .write(&response[connection.response_offset..])
        {
            Ok(0) => return WriteState::Closed,
            Ok(written) => connection.response_offset += written,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return WriteState::Pending,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return WriteState::Closed,
        }
    }
    let _ = connection.stream.shutdown(Shutdown::Write);
    WriteState::Complete
}

fn write_capacity_response(mut stream: TcpStream) {
    let response = serialize_response(&standard_error_response(503))
        .unwrap_or_else(|_| internal_server_error_bytes());
    let mut offset = 0usize;
    while offset < response.len() {
        match stream.write(&response[offset..]) {
            Ok(0) => break,
            Ok(written) => offset += written,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

fn internal_server_error_bytes() -> Vec<u8> {
    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 22\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nInternal Server Error\n".to_vec()
}

fn runtime_error(code: &'static str, message: impl Into<String>) -> WorkerDiagnostic {
    WorkerDiagnostic::new(code, message, EXIT_RUNTIME_UNAVAILABLE)
}
