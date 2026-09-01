use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::thread;
use std::thread::sleep;
use std::time::Duration;

use crate::json_support::JsonValue as Value;
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{Method, Url};

use crate::json_support::JsonCodec;
use crate::server_operational::RepositoryIndex;

pub use super::transport_ports::{
    close_with_plan_http_client_lifecycle, execute_bytes_with_plan_http_transport,
    execute_json_with_plan_http_transport, inspect_with_plan_http_client_lifecycle,
    PlanHttpBytesRequestSpec, PlanHttpClientLifecycle, PlanHttpRequestSpec, PlanHttpTransport,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanHttpClientError {
    Invalid(String),
    Remote(String),
    RemoteResponse {
        method: String,
        url: String,
        status: u16,
        detail: String,
    },
    Transport(String),
    Closed(String),
}

impl Display for PlanHttpClientError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message)
            | Self::Remote(message)
            | Self::Transport(message)
            | Self::Closed(message) => f.write_str(message),
            Self::RemoteResponse {
                method,
                url,
                status,
                detail,
            } => write!(f, "{method} {url} failed: {status} {detail}"),
        }
    }
}

impl std::error::Error for PlanHttpClientError {}

impl PlanHttpClientError {
    pub fn remote_status(&self) -> Option<u16> {
        match self {
            Self::RemoteResponse { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn remote_detail(&self) -> Option<&str> {
        match self {
            Self::RemoteResponse { detail, .. } => Some(detail.as_str()),
            _ => None,
        }
    }

    pub fn is_retryable_busy(&self) -> bool {
        self.remote_status() == Some(503)
            && self
                .remote_detail()
                .is_some_and(|detail| detail.starts_with(RETRYABLE_BUSY_ERROR_PREFIX))
    }
}

const RETRYABLE_BUSY_ERROR_PREFIX: &str = "ait.binary-db.error.v1|retryable_busy|";
pub(super) const RETRYABLE_BUSY_READ_MAX_RETRIES: usize = 120;
const RETRYABLE_BUSY_READ_BACKOFF: Duration = Duration::from_millis(250);

pub type PlanHttpClientResult<T> = Result<T, PlanHttpClientError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanHttpClientConfig {
    pub base_url: String,
    /// Repository authority selected by `.ait/config.json` for repository-local
    /// Binary DB routes. This is HTTP routing state, not Plan identity.
    pub repository_index: Option<RepositoryIndex>,
    pub headers: BTreeMap<String, String>,
    pub default_timeout_ms: u64,
    pub retry_attempts: usize,
    pub retry_backoff_ms: u64,
    pub pool_max_idle_per_host: usize,
}

impl Default for PlanHttpClientConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            repository_index: None,
            headers: BTreeMap::new(),
            default_timeout_ms: 30_000,
            retry_attempts: 0,
            retry_backoff_ms: 0,
            pool_max_idle_per_host: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanHttpClientStats {
    pub base_url: String,
    pub default_timeout_ms: u64,
    pub retry_attempts: usize,
    pub retry_backoff_ms: u64,
    pub pool_max_idle_per_host: usize,
    pub request_count: usize,
    pub retry_count: usize,
    pub closed: bool,
}

#[derive(Debug)]
pub struct PlanHttpClientManager {
    pub(crate) config: PlanHttpClientConfig,
    pub(crate) client: Option<Client>,
    pub(crate) request_count: usize,
    pub(crate) retry_count: usize,
    pub(crate) closed: bool,
}

impl PlanHttpClientManager {
    pub fn new(config: PlanHttpClientConfig) -> PlanHttpClientResult<Self> {
        let normalized_config = normalize_client_config(config)?;
        let client = build_client(&normalized_config)?;
        Ok(Self {
            config: normalized_config,
            client: Some(client),
            request_count: 0,
            retry_count: 0,
            closed: false,
        })
    }

    pub fn inspect(&self) -> PlanHttpClientStats {
        PlanHttpClientStats {
            base_url: self.config.base_url.clone(),
            default_timeout_ms: self.config.default_timeout_ms,
            retry_attempts: self.config.retry_attempts,
            retry_backoff_ms: self.config.retry_backoff_ms,
            pool_max_idle_per_host: self.config.pool_max_idle_per_host,
            request_count: self.request_count,
            retry_count: self.retry_count,
            closed: self.closed,
        }
    }

    pub fn close(&mut self) -> PlanHttpClientStats {
        self.client = None;
        self.closed = true;
        self.inspect()
    }

    pub(super) fn execute_json(
        &mut self,
        spec: PlanHttpRequestSpec,
    ) -> PlanHttpClientResult<Option<Value>> {
        if self.closed {
            return Err(PlanHttpClientError::Closed(
                "Rust plan HTTP client manager is already closed.".to_string(),
            ));
        }
        let Some(client) = self.client.as_ref() else {
            return Err(PlanHttpClientError::Closed(
                "Rust plan HTTP client manager is not available.".to_string(),
            ));
        };
        let mut transport_attempt = 0usize;
        let mut busy_read_retry_count = 0usize;
        loop {
            self.request_count += 1;
            match perform_request(client, &spec) {
                Ok(response) => match parse_response_payload(&spec.method, &spec.url, response) {
                    Ok(payload) => return Ok(payload),
                    Err(err) => {
                        let Some(delay) =
                            retryable_busy_read_delay(&spec.method, &err, busy_read_retry_count)
                        else {
                            return Err(err);
                        };
                        self.retry_count += 1;
                        busy_read_retry_count += 1;
                        sleep(delay);
                    }
                },
                Err(err)
                    if should_retry_transport_error(&err)
                        && transport_attempt < self.config.retry_attempts =>
                {
                    self.retry_count += 1;
                    transport_attempt += 1;
                    if self.config.retry_backoff_ms > 0 {
                        sleep(Duration::from_millis(self.config.retry_backoff_ms));
                    }
                }
                Err(err) => return Err(map_transport_error(&spec.method, &spec.url, err)),
            }
        }
    }

    pub(super) fn execute_bytes(
        &mut self,
        spec: PlanHttpBytesRequestSpec,
    ) -> PlanHttpClientResult<Vec<u8>> {
        if self.closed {
            return Err(PlanHttpClientError::Closed(
                "Rust plan HTTP client manager is already closed.".to_string(),
            ));
        }
        let Some(client) = self.client.as_ref() else {
            return Err(PlanHttpClientError::Closed(
                "Rust plan HTTP client manager is not available.".to_string(),
            ));
        };
        let mut transport_attempt = 0usize;
        let mut busy_read_retry_count = 0usize;
        loop {
            self.request_count += 1;
            match perform_bytes_request(client, &spec) {
                Ok(response) => {
                    match parse_bytes_response_payload(&spec.method, &spec.url, response) {
                        Ok(payload) => return Ok(payload),
                        Err(err) => {
                            let Some(delay) = retryable_busy_read_delay(
                                &spec.method,
                                &err,
                                busy_read_retry_count,
                            ) else {
                                return Err(err);
                            };
                            self.retry_count += 1;
                            busy_read_retry_count += 1;
                            sleep(delay);
                        }
                    }
                }
                Err(err)
                    if should_retry_transport_error(&err)
                        && transport_attempt < self.config.retry_attempts =>
                {
                    self.retry_count += 1;
                    transport_attempt += 1;
                    if self.config.retry_backoff_ms > 0 {
                        sleep(Duration::from_millis(self.config.retry_backoff_ms));
                    }
                }
                Err(err) => return Err(map_transport_error(&spec.method, &spec.url, err)),
            }
        }
    }

    pub(crate) fn execute_bytes_bounded(
        &mut self,
        specs: Vec<PlanHttpBytesRequestSpec>,
        max_parallelism: usize,
    ) -> PlanHttpClientResult<Vec<Vec<u8>>> {
        if specs.is_empty() {
            return Ok(Vec::new());
        }
        if self.closed {
            return Err(PlanHttpClientError::Closed(
                "Rust plan HTTP client manager is already closed.".to_string(),
            ));
        }
        let Some(shared_client) = self.client.as_ref().cloned() else {
            return Err(PlanHttpClientError::Closed(
                "Rust plan HTTP client manager is not available.".to_string(),
            ));
        };
        let parallelism = max_parallelism.clamp(1, 16).min(specs.len());
        if parallelism == 1 {
            return specs
                .into_iter()
                .map(|spec| self.execute_bytes(spec))
                .collect();
        }

        let mut output = Vec::with_capacity(specs.len());
        for chunk in specs.chunks(parallelism) {
            let config = self.config.clone();
            let chunk_results = thread::scope(|scope| {
                let handles = chunk
                    .iter()
                    .cloned()
                    .map(|spec| {
                        let client = shared_client.clone();
                        let worker_config = config.clone();
                        scope.spawn(move || {
                            let mut worker = PlanHttpClientManager {
                                config: worker_config,
                                client: Some(client),
                                request_count: 0,
                                retry_count: 0,
                                closed: false,
                            };
                            let result = worker.execute_bytes(spec);
                            (result, worker.request_count, worker.retry_count)
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle.join().map_err(|_| {
                            PlanHttpClientError::Transport(
                                "bounded HTTP transfer worker panicked".to_string(),
                            )
                        })
                    })
                    .collect::<PlanHttpClientResult<Vec<_>>>()
            })?;
            let mut first_error = None;
            for (result, request_count, retry_count) in chunk_results {
                self.request_count = self.request_count.saturating_add(request_count);
                self.retry_count = self.retry_count.saturating_add(retry_count);
                match result {
                    Ok(bytes) => output.push(bytes),
                    Err(error) if first_error.is_none() => first_error = Some(error),
                    Err(_) => {}
                }
            }
            if let Some(error) = first_error {
                return Err(error);
            }
        }
        Ok(output)
    }
}

impl PlanHttpClientLifecycle for PlanHttpClientManager {
    type Stats = PlanHttpClientStats;

    fn inspect(&self) -> Self::Stats {
        PlanHttpClientManager::inspect(self)
    }

    fn close(&mut self) -> Self::Stats {
        PlanHttpClientManager::close(self)
    }
}

impl PlanHttpTransport for PlanHttpClientManager {
    type Error = PlanHttpClientError;

    fn execute_json(&mut self, spec: PlanHttpRequestSpec) -> Result<Option<Value>, Self::Error> {
        PlanHttpClientManager::execute_json(self, spec)
    }

    fn execute_bytes(&mut self, spec: PlanHttpBytesRequestSpec) -> Result<Vec<u8>, Self::Error> {
        PlanHttpClientManager::execute_bytes(self, spec)
    }
}

pub(crate) fn build_request_spec(
    config: &PlanHttpClientConfig,
    method: Method,
    path: &str,
    query_pairs: Vec<(String, String)>,
    body: Option<Value>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let mut url = join_url(&config.base_url, path)?;
    if !query_pairs.is_empty() {
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &query_pairs {
                pairs.append_pair(key, value);
            }
        }
    }
    let mut headers = BTreeMap::new();
    headers.insert("Accept".to_string(), "application/json".to_string());
    for (key, value) in &config.headers {
        headers.insert(key.clone(), value.clone());
    }
    if body.is_some() {
        headers.insert("Content-Type".to_string(), "application/json".to_string());
    }
    Ok(PlanHttpRequestSpec {
        method: method.as_str().to_string(),
        path: path.to_string(),
        url: url.to_string(),
        query_pairs,
        headers,
        body,
        timeout_ms: config.default_timeout_ms,
    })
}

pub(crate) fn build_bytes_request_spec(
    config: &PlanHttpClientConfig,
    method: Method,
    path: &str,
    query_pairs: Vec<(String, String)>,
    body: Option<Vec<u8>>,
    accept: &str,
    content_type: Option<&str>,
) -> PlanHttpClientResult<PlanHttpBytesRequestSpec> {
    let mut url = join_url(&config.base_url, path)?;
    if !query_pairs.is_empty() {
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in &query_pairs {
                pairs.append_pair(key, value);
            }
        }
    }
    let mut headers = BTreeMap::new();
    headers.insert("Accept".to_string(), accept.to_string());
    for (key, value) in &config.headers {
        headers.insert(key.clone(), value.clone());
    }
    if body.is_some() {
        if let Some(content_type) = content_type {
            headers.insert("Content-Type".to_string(), content_type.to_string());
        }
    }
    Ok(PlanHttpBytesRequestSpec {
        method: method.as_str().to_string(),
        path: path.to_string(),
        url: url.to_string(),
        query_pairs,
        headers,
        body,
        timeout_ms: config.default_timeout_ms,
    })
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    let text = value?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn normalize_client_config(
    config: PlanHttpClientConfig,
) -> PlanHttpClientResult<PlanHttpClientConfig> {
    let base_url = normalize_optional_text(Some(config.base_url.as_str())).ok_or_else(|| {
        PlanHttpClientError::Invalid("Plan HTTP base_url must not be empty.".to_string())
    })?;
    if config.pool_max_idle_per_host == 0 {
        return Err(PlanHttpClientError::Invalid(
            "Plan HTTP pool_max_idle_per_host must be >= 1.".to_string(),
        ));
    }
    let base_url = normalize_base_url(&base_url)?;
    let mut normalized_headers = BTreeMap::new();
    for (key, value) in config.headers {
        let Some(key_text) = normalize_optional_text(Some(key.as_str())) else {
            return Err(PlanHttpClientError::Invalid(
                "Plan HTTP header names must be non-empty strings.".to_string(),
            ));
        };
        let Some(value_text) = normalize_optional_text(Some(value.as_str())) else {
            return Err(PlanHttpClientError::Invalid(format!(
                "Plan HTTP header `{key_text}` must carry a non-empty string value."
            )));
        };
        normalized_headers.insert(key_text, value_text);
    }
    Ok(PlanHttpClientConfig {
        base_url,
        headers: normalized_headers,
        ..config
    })
}

fn normalize_base_url(base_url: &str) -> PlanHttpClientResult<String> {
    let mut normalized = base_url.trim().to_string();
    if !normalized.ends_with('/') {
        normalized.push('/');
    }
    Url::parse(&normalized).map_err(|err| {
        PlanHttpClientError::Invalid(format!("Invalid plan HTTP base_url `{base_url}`: {err}"))
    })?;
    Ok(normalized)
}

fn build_client(config: &PlanHttpClientConfig) -> PlanHttpClientResult<Client> {
    let mut builder = Client::builder().pool_max_idle_per_host(config.pool_max_idle_per_host);
    if config.default_timeout_ms > 0 {
        builder = builder.timeout(Duration::from_millis(config.default_timeout_ms));
    }
    builder.build().map_err(|err| {
        PlanHttpClientError::Transport(format!(
            "Failed to build Rust plan HTTP client for {}: {}",
            config.base_url, err
        ))
    })
}

fn perform_request(
    client: &Client,
    spec: &PlanHttpRequestSpec,
) -> Result<Response, reqwest::Error> {
    let method = Method::from_bytes(spec.method.as_bytes()).unwrap_or(Method::GET);
    let mut request = client.request(method, &spec.url);
    request = request.timeout(Duration::from_millis(spec.timeout_ms));
    let mut header_map = HeaderMap::new();
    for (key, value) in &spec.headers {
        if let (Ok(name), Ok(header_value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            header_map.insert(name, header_value);
        }
    }
    if !header_map.contains_key(ACCEPT) {
        header_map.insert(ACCEPT, HeaderValue::from_static("application/json"));
    }
    request = request.headers(header_map);
    if let Some(body) = &spec.body {
        request = request
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string());
    }
    request.send()
}

fn perform_bytes_request(
    client: &Client,
    spec: &PlanHttpBytesRequestSpec,
) -> Result<Response, reqwest::Error> {
    let method = Method::from_bytes(spec.method.as_bytes()).unwrap_or(Method::GET);
    let mut request = client.request(method, &spec.url);
    request = request.timeout(Duration::from_millis(spec.timeout_ms));
    let mut header_map = HeaderMap::new();
    for (key, value) in &spec.headers {
        if let (Ok(name), Ok(header_value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            header_map.insert(name, header_value);
        }
    }
    if !header_map.contains_key(ACCEPT) {
        header_map.insert(ACCEPT, HeaderValue::from_static("*/*"));
    }
    request = request.headers(header_map);
    if let Some(body) = &spec.body {
        request = request.body(body.clone());
    }
    request.send()
}

fn parse_response_payload(
    method: &str,
    url: &str,
    response: Response,
) -> PlanHttpClientResult<Option<Value>> {
    let status = response.status();
    let body_text = response
        .text()
        .map_err(|err| map_transport_error(method, url, err))?;
    if status.is_success() {
        if body_text.trim().is_empty() {
            return Ok(None);
        }
        return JsonCodec::parse_value_with_error_prefix(&body_text, "invalid JSON response")
            .map(Some)
            .map_err(|err| PlanHttpClientError::Remote(format!("{method} {url} failed: {err}")));
    }
    let message = normalize_error_message(&body_text);
    Err(PlanHttpClientError::RemoteResponse {
        method: method.to_string(),
        url: url.to_string(),
        status: status.as_u16(),
        detail: message,
    })
}

fn parse_bytes_response_payload(
    method: &str,
    url: &str,
    response: Response,
) -> PlanHttpClientResult<Vec<u8>> {
    let status = response.status();
    let bytes = response
        .bytes()
        .map_err(|err| map_transport_error(method, url, err))?;
    if status.is_success() {
        return Ok(bytes.to_vec());
    }
    let body_text = String::from_utf8_lossy(&bytes);
    let message = normalize_error_message(&body_text);
    Err(PlanHttpClientError::RemoteResponse {
        method: method.to_string(),
        url: url.to_string(),
        status: status.as_u16(),
        detail: message,
    })
}

fn normalize_error_message(body_text: &str) -> String {
    if body_text.trim().is_empty() {
        return "unknown error".to_string();
    }
    match JsonCodec::parse_value_with_error_prefix(body_text, "invalid JSON error response") {
        Ok(Value::Object(map)) => {
            if let Some(Value::String(detail)) = map.get("detail") {
                return detail.clone();
            }
            if let Some(Value::String(detail)) = map.get("error") {
                return detail.clone();
            }
            Value::Object(map).to_string()
        }
        Ok(other) => other.to_string(),
        Err(_) => body_text.to_string(),
    }
}

fn should_retry_transport_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

pub(super) fn retryable_busy_read_delay(
    method: &str,
    err: &PlanHttpClientError,
    retry_count: usize,
) -> Option<Duration> {
    if method.eq_ignore_ascii_case("GET")
        && err.is_retryable_busy()
        && retry_count < RETRYABLE_BUSY_READ_MAX_RETRIES
    {
        Some(RETRYABLE_BUSY_READ_BACKOFF)
    } else {
        None
    }
}

fn map_transport_error(method: &str, url: &str, err: reqwest::Error) -> PlanHttpClientError {
    if err.is_timeout() {
        return PlanHttpClientError::Transport(format!("{method} {url} failed: timed out"));
    }
    PlanHttpClientError::Transport(format!("{method} {url} failed: {err}"))
}

fn join_url(base_url: &str, path: &str) -> PlanHttpClientResult<Url> {
    let parsed_base = Url::parse(base_url).map_err(|err| {
        PlanHttpClientError::Invalid(format!("Invalid plan HTTP base_url `{base_url}`: {err}"))
    })?;
    parsed_base
        .join(path.trim_start_matches('/'))
        .map_err(|err| {
            PlanHttpClientError::Invalid(format!("Invalid plan HTTP path `{path}`: {err}"))
        })
}
