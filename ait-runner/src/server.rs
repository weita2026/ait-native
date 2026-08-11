use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ait_core::server_operational::{
    NATIVE_JOB_V3_CONTRACT, RepositoryIndex, ServerOperationalCapabilities, WorkerJobIndex,
    WorkerJobKey, WorkerLeaseProof, claim_next_worker_job_path, worker_job_operation_path,
};
use reqwest::blocking::{Client, RequestBuilder, Response};
use serde_json::{Value as JsonValue, json};

use crate::protocol::{
    DELIVERY_CONTRACT, LEGACY_NATIVE_JOB_CONTRACT, MAX_REQUEST_BYTES, NATIVE_JOB_CONTRACT,
    NativeJobRequest, NativeResult, SourceSpec,
};
use crate::{NativeExecutor, RunnerError};
use crate::{RemotePackKind, RemoteSnapshotProvider, RemoteSnapshotReference};

const BINARY_QUEUE_SERVICE_CONTRACT: &str = "ait.server.worker-job.service.v1";
const LEGACY_QUEUE_SERVICE_CONTRACT: &str = "ait.server.worker_queue.service.v1";
const DOCTOR_CONTRACT: &str = "ait.runner.doctor.v1";
const SERVICE_CONTRACT: &str = "ait.runner.service.v1";
const MAX_SERVER_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_FAILURE_MESSAGE_CHARS: usize = 4096;
const DEFAULT_HTTP_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(30);
const OBJECT_PACK_MEDIA_TYPE: &str = "application/vnd.ait.remote-sync.object-pack+zstd";
const TREE_PACK_MEDIA_TYPE: &str = "application/vnd.ait.remote-sync.tree-pack+zstd";
const ACCEPTED_BINARY_JOB_KINDS: [u8; 2] = [7, 11];

#[derive(Clone, Debug)]
pub struct RunJobOptions {
    pub key: WorkerJobKey,
}

#[derive(Clone, Debug)]
pub struct ServeOptions {
    pub worker_id: String,
    pub repository_indexes: Vec<RepositoryIndex>,
    pub once: bool,
    pub poll_interval: Duration,
    pub heartbeat_interval: Duration,
}

#[derive(Clone)]
pub struct ServerClient {
    base_url: String,
    client: Client,
    bearer_token: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QueueProtocol {
    BinaryV3,
    LegacyV1,
}

#[derive(Clone, Debug)]
struct BinaryClaim {
    proof: WorkerLeaseProof,
    lease_expires_at_s: u32,
    job_kind: u8,
    request: NativeJobRequest,
}

#[derive(Clone, Debug)]
struct LegacyQueueJob {
    job_id: i64,
    repo_name: String,
    job_type: String,
    state: String,
    payload: JsonValue,
}

#[derive(Clone, Debug)]
enum ClaimedJob {
    Binary(BinaryClaim),
    Legacy(LegacyQueueJob),
}

struct HeartbeatLease {
    stop: mpsc::Sender<()>,
    thread: thread::JoinHandle<()>,
    error: Arc<Mutex<Option<String>>>,
}

impl ServerClient {
    pub fn new(base_url: &str, bearer_token: Option<String>) -> Result<Self, RunnerError> {
        let base_url = normalize_base_url(base_url)?;
        if bearer_token
            .as_deref()
            .is_some_and(|token| token.trim().is_empty() || token.contains(['\r', '\n']))
        {
            return Err(RunnerError::Server(
                "bearer token must be non-empty and contain no newline".to_string(),
            ));
        }
        let client = Client::builder()
            .timeout(DEFAULT_HTTP_TIMEOUT)
            .build()
            .map_err(|error| {
                RunnerError::Server(format!("could not create HTTP client: {error}"))
            })?;
        Ok(Self {
            base_url,
            client,
            bearer_token,
        })
    }

    pub fn doctor(&self) -> Result<JsonValue, RunnerError> {
        let health = self.health_payload()?;
        let protocol = negotiate_protocol(&health)?;
        Ok(json!({
            "contract": DOCTOR_CONTRACT,
            "status": "ready",
            "ready": true,
            "server_url": self.base_url,
            "selected_runner_contract": protocol.contract(),
            "operational_capabilities": health
                .get("operational_capabilities")
                .cloned()
                .unwrap_or(JsonValue::Null),
            "ci_capabilities": health
                .get("ci_capabilities")
                .cloned()
                .unwrap_or(JsonValue::Null),
        }))
    }

    pub fn run_job(
        &self,
        executor: &NativeExecutor,
        options: &RunJobOptions,
    ) -> Result<JsonValue, RunnerError> {
        validate_run_job_options(options)?;
        let protocol = negotiate_protocol(&self.health_payload()?)?;
        if protocol != QueueProtocol::BinaryV3 {
            return Err(RunnerError::Server(format!(
                "explicit pair-key run requires `{NATIVE_JOB_V3_CONTRACT}`; the server advertises only `{LEGACY_NATIVE_JOB_CONTRACT}`"
            )));
        }
        let claimed = self.claim_binary_job(options.key)?;
        executor.preflight(&claimed.request).or_else(|error| {
            self.fail_binary_job(
                &claimed.proof,
                &error.bounded_message(MAX_FAILURE_MESSAGE_CHARS),
                false,
            )
            .and(Err(error))
        })?;
        self.execute_binary_claimed(executor, &claimed, Duration::from_secs(30), None)
    }

    pub fn serve(
        &self,
        executor: &NativeExecutor,
        options: &ServeOptions,
    ) -> Result<JsonValue, RunnerError> {
        validate_serve_options(options)?;
        let _attempt_root_lease = executor.acquire_attempt_root_lease()?;
        let mut protocol = negotiate_protocol(&self.health_payload()?)?;
        if protocol == QueueProtocol::LegacyV1 && !options.repository_indexes.is_empty() {
            return Err(RunnerError::Server(
                "numeric Repository filters require native-job.v3; they are never translated to legacy repo_name routing"
                    .to_string(),
            ));
        }
        let mut binary_selected = protocol == QueueProtocol::BinaryV3;
        let mut consecutive_claim_failures = 0_u32;

        loop {
            let claim_result = match protocol {
                QueueProtocol::BinaryV3 => self
                    .claim_next_binary_job(&options.repository_indexes)
                    .map(|claim| claim.map(ClaimedJob::Binary)),
                QueueProtocol::LegacyV1 => self
                    .claim_next_legacy_job(&options.worker_id)
                    .map(|claim| claim.map(ClaimedJob::Legacy)),
            };
            let claimed = match claim_result {
                Ok(claimed) => {
                    consecutive_claim_failures = 0;
                    claimed
                }
                Err(error) if !options.once && error.is_server_unavailable() => {
                    thread::sleep(reconnect_delay(
                        options.poll_interval,
                        consecutive_claim_failures,
                    ));
                    consecutive_claim_failures = consecutive_claim_failures.saturating_add(1);
                    match self
                        .health_payload()
                        .and_then(|health| negotiate_protocol(&health))
                    {
                        Ok(next) => {
                            if binary_selected && next == QueueProtocol::LegacyV1 {
                                return Err(binary_downgrade_error());
                            }
                            if next == QueueProtocol::BinaryV3 {
                                binary_selected = true;
                            }
                            protocol = next;
                        }
                        Err(health_error) if health_error.is_server_unavailable() => {}
                        Err(health_error) => return Err(health_error),
                    }
                    continue;
                }
                Err(error) if protocol == QueueProtocol::LegacyV1 => {
                    match self
                        .health_payload()
                        .and_then(|health| negotiate_protocol(&health))
                    {
                        Ok(QueueProtocol::BinaryV3) => {
                            protocol = QueueProtocol::BinaryV3;
                            binary_selected = true;
                            continue;
                        }
                        Ok(QueueProtocol::LegacyV1) | Err(_) => return Err(error),
                    }
                }
                Err(error) => return Err(error),
            };

            let Some(claimed) = claimed else {
                if options.once {
                    return Ok(json!({
                        "contract": SERVICE_CONTRACT,
                        "status": "idle",
                        "worker_id": options.worker_id,
                        "claimed": false,
                        "selected_runner_contract": protocol.contract(),
                    }));
                }
                thread::sleep(options.poll_interval);
                continue;
            };

            match self.execute_claimed_in_serve(executor, options, &claimed) {
                Ok(delivered) if options.once => return Ok(delivered),
                Ok(_) => {}
                Err(error) if options.once => return Err(error),
                Err(error) => {
                    eprintln!("{}", serve_job_error_event(&claimed, &error));
                    thread::sleep(options.poll_interval);
                }
            }
        }
    }

    fn execute_claimed_in_serve(
        &self,
        executor: &NativeExecutor,
        options: &ServeOptions,
        claimed: &ClaimedJob,
    ) -> Result<JsonValue, RunnerError> {
        let request = match claimed {
            ClaimedJob::Binary(claim) => claim.request.clone(),
            ClaimedJob::Legacy(claim) => compatible_legacy_native_request(claim)?,
        };
        executor.preflight(&request).or_else(|error| {
            self.fail_claimed(
                claimed,
                &options.worker_id,
                &error.bounded_message(MAX_FAILURE_MESSAGE_CHARS),
                false,
            )
            .and(Err(error))
        })?;
        match claimed {
            ClaimedJob::Binary(claim) => self.execute_binary_claimed(
                executor,
                claim,
                options.heartbeat_interval,
                Some(&options.worker_id),
            ),
            ClaimedJob::Legacy(claim) => self.execute_legacy_claimed(
                executor,
                claim,
                &options.worker_id,
                options.heartbeat_interval,
            ),
        }
    }

    fn execute_binary_claimed(
        &self,
        executor: &NativeExecutor,
        claimed: &BinaryClaim,
        heartbeat_interval: Duration,
        worker_id: Option<&str>,
    ) -> Result<JsonValue, RunnerError> {
        let heartbeat_interval = bounded_binary_heartbeat_interval(
            claimed.lease_expires_at_s,
            unix_time_s()?,
            heartbeat_interval,
        )?;
        let heartbeat =
            HeartbeatLease::start_binary(self.clone(), claimed.proof.clone(), heartbeat_interval);
        let execution = executor.execute_with_provider(&claimed.request, Some(self));
        if let Err(error) = heartbeat.finish() {
            return self.fail_binary_after_claim(claimed, error);
        }
        let result = match execution {
            Ok(result) => result,
            Err(error) => return self.fail_binary_after_claim(claimed, error),
        };
        if let Err(error) = result.validate_bound() {
            return self.fail_binary_after_claim(claimed, error);
        }
        let terminal_state = self.complete_binary_job(&claimed.proof, &result, claimed.job_kind)?;
        Ok(json!({
            "contract": DELIVERY_CONTRACT,
            "status": "delivered",
            "repository_index": claimed.proof.repository_index,
            "worker_job_index": claimed.proof.worker_job_index,
            "attempt_count": claimed.proof.attempt_count,
            "worker_id": worker_id,
            "terminal_operation": "complete",
            "terminal_state_kind": terminal_state,
            "result": result,
        }))
    }

    fn execute_legacy_claimed(
        &self,
        executor: &NativeExecutor,
        claimed: &LegacyQueueJob,
        worker_id: &str,
        heartbeat_interval: Duration,
    ) -> Result<JsonValue, RunnerError> {
        let request = compatible_legacy_native_request(claimed)?;
        let heartbeat = HeartbeatLease::start_legacy(
            self.clone(),
            claimed.job_id,
            worker_id.to_string(),
            heartbeat_interval,
        );
        let execution = executor.execute_with_provider(&request, Some(self));
        if let Err(error) = heartbeat.finish() {
            return self.fail_legacy_after_claim(claimed.job_id, worker_id, error);
        }
        let result = match execution {
            Ok(result) => result,
            Err(error) => {
                return self.fail_legacy_after_claim(claimed.job_id, worker_id, error);
            }
        };
        if let Err(error) = result.validate_bound() {
            return self.fail_legacy_after_claim(claimed.job_id, worker_id, error);
        }
        let terminal_job = self.complete_legacy_job(claimed.job_id, worker_id, &result)?;
        Ok(json!({
            "contract": DELIVERY_CONTRACT,
            "status": "delivered",
            "legacy_job_id": claimed.job_id,
            "legacy_repo_name": claimed.repo_name,
            "worker_id": worker_id,
            "selected_runner_contract": LEGACY_NATIVE_JOB_CONTRACT,
            "terminal_operation": "complete-job",
            "terminal_state": terminal_job.state,
            "result": result,
        }))
    }

    fn fail_claimed(
        &self,
        claimed: &ClaimedJob,
        worker_id: &str,
        error: &str,
        retryable: bool,
    ) -> Result<(), RunnerError> {
        match claimed {
            ClaimedJob::Binary(claim) => {
                self.fail_binary_job(&claim.proof, error, retryable)?;
            }
            ClaimedJob::Legacy(claim) => {
                self.fail_legacy_job(claim.job_id, worker_id, error, retryable)?;
            }
        }
        Ok(())
    }

    fn fail_binary_after_claim(
        &self,
        claimed: &BinaryClaim,
        error: RunnerError,
    ) -> Result<JsonValue, RunnerError> {
        let key = claimed.proof.key();
        let message = error.bounded_message(MAX_FAILURE_MESSAGE_CHARS);
        match self.fail_binary_job(&claimed.proof, &message, true) {
            Ok(_) => Err(RunnerError::Process(format!(
                "Worker Job {key} attempt {} failed and ait-server acknowledged fail: {message}",
                claimed.proof.attempt_count
            ))),
            Err(server_error) => Err(RunnerError::Server(format!(
                "Worker Job {key} attempt {} failed: {message}; failure delivery also failed: {server_error}",
                claimed.proof.attempt_count
            ))),
        }
    }

    fn fail_legacy_after_claim(
        &self,
        job_id: i64,
        worker_id: &str,
        error: RunnerError,
    ) -> Result<JsonValue, RunnerError> {
        let message = error.bounded_message(MAX_FAILURE_MESSAGE_CHARS);
        match self.fail_legacy_job(job_id, worker_id, &message, true) {
            Ok(_) => Err(RunnerError::Process(format!(
                "legacy job {job_id} failed and ait-server acknowledged fail-job: {message}"
            ))),
            Err(server_error) => Err(RunnerError::Server(format!(
                "legacy job {job_id} failed: {message}; fail-job delivery also failed: {server_error}"
            ))),
        }
    }

    fn health_payload(&self) -> Result<JsonValue, RunnerError> {
        let request = self.auth(self.client.get(format!("{}/healthz", self.base_url)));
        let payload = decode_response(request.send().map_err(|error| {
            RunnerError::ServerUnavailable(format!("GET /healthz could not connect: {error}"))
        })?)?;
        if payload.get("ready").and_then(JsonValue::as_bool) != Some(true) {
            return Err(RunnerError::Server(
                "GET /healthz did not report `ready: true`".to_string(),
            ));
        }
        Ok(payload)
    }

    fn claim_binary_job(&self, key: WorkerJobKey) -> Result<BinaryClaim, RunnerError> {
        let path = worker_job_operation_path(key, "claim").map_err(RunnerError::Server)?;
        let payload = self.binary_operation(
            &path,
            "claim",
            json!({"accepted_runtime_contracts": [NATIVE_JOB_CONTRACT]}),
            None,
        )?;
        let claim =
            parse_binary_claim_field(&payload, "claimed_job", "claim")?.ok_or_else(|| {
                RunnerError::Server("claim response requires `claimed_job`".to_string())
            })?;
        if claim.proof.key() != key {
            return Err(RunnerError::Server(format!(
                "claim returned Worker Job {}, expected {key}",
                claim.proof.key()
            )));
        }
        Ok(claim)
    }

    fn claim_next_binary_job(
        &self,
        repository_indexes: &[RepositoryIndex],
    ) -> Result<Option<BinaryClaim>, RunnerError> {
        let payload = self.binary_operation(
            claim_next_worker_job_path(),
            "claim",
            json!({
                "accepted_job_kinds": ACCEPTED_BINARY_JOB_KINDS,
                "repository_indexes": repository_indexes,
                "accepted_runtime_contracts": [NATIVE_JOB_CONTRACT],
            }),
            None,
        )?;
        parse_binary_claim_field(&payload, "claimed_job", "claim")
    }

    fn heartbeat_binary_job(
        &self,
        proof: &WorkerLeaseProof,
        request_timeout: Duration,
    ) -> Result<u8, RunnerError> {
        let payload =
            self.binary_lease_operation("heartbeat", proof, None, Some(request_timeout))?;
        parse_binary_job_state(&payload, proof, "heartbeat", &[2])
    }

    fn complete_binary_job(
        &self,
        proof: &WorkerLeaseProof,
        result: &NativeResult,
        job_kind: u8,
    ) -> Result<u8, RunnerError> {
        result.validate_bound()?;
        let result = serde_json::to_value(result).map_err(|error| {
            RunnerError::Server(format!("could not encode complete result: {error}"))
        })?;
        let detail = json!({
            "job_kind": job_kind,
            "result": result,
        });
        let payload = self.binary_lease_operation("complete", proof, Some(detail), None)?;
        parse_binary_job_state(&payload, proof, "complete", &[3])
    }

    fn fail_binary_job(
        &self,
        proof: &WorkerLeaseProof,
        error: &str,
        retryable: bool,
    ) -> Result<u8, RunnerError> {
        let detail = json!({
            "error": bounded_text(error, MAX_FAILURE_MESSAGE_CHARS),
            "retryable": retryable,
        });
        let payload = self.binary_lease_operation("fail", proof, Some(detail), None)?;
        parse_binary_job_state(&payload, proof, "fail", &[1, 4])
    }

    fn binary_lease_operation(
        &self,
        operation: &str,
        proof: &WorkerLeaseProof,
        detail: Option<JsonValue>,
        request_timeout: Option<Duration>,
    ) -> Result<JsonValue, RunnerError> {
        proof.validate().map_err(RunnerError::InvalidRequest)?;
        let path =
            worker_job_operation_path(proof.key(), operation).map_err(RunnerError::Server)?;
        let mut body = json!({
            "attempt_count": proof.attempt_count,
            "lease_token": proof.lease_token,
        });
        if let Some(detail) = detail {
            body["detail"] = detail;
        }
        self.binary_operation(&path, operation, body, request_timeout)
    }

    fn binary_operation(
        &self,
        path: &str,
        operation: &str,
        body: JsonValue,
        request_timeout: Option<Duration>,
    ) -> Result<JsonValue, RunnerError> {
        let mut request = self.auth(
            self.client
                .post(format!("{}{}", self.base_url, path))
                .json(&body),
        );
        if let Some(request_timeout) = request_timeout {
            request = request.timeout(request_timeout);
        }
        let payload = decode_response(request.send().map_err(|error| {
            RunnerError::ServerUnavailable(format!(
                "Binary Worker Job {operation} request failed: {error}"
            ))
        })?)?;
        if payload.get("contract").and_then(JsonValue::as_str)
            != Some(BINARY_QUEUE_SERVICE_CONTRACT)
        {
            return Err(RunnerError::Server(format!(
                "Binary Worker Job {operation} returned an unsupported contract"
            )));
        }
        if payload.get("operation").and_then(JsonValue::as_str) != Some(operation) {
            return Err(RunnerError::Server(format!(
                "Binary Worker Job response operation does not match `{operation}`"
            )));
        }
        Ok(payload)
    }

    fn claim_next_legacy_job(
        &self,
        worker_id: &str,
    ) -> Result<Option<LegacyQueueJob>, RunnerError> {
        let payload = self.legacy_queue_operation(
            "claim-next-job",
            json!({
                "worker_id": worker_id,
                "accepted_job_types": ["patchset.ci", "repo.ci"],
                "accepted_runtime_contracts": [LEGACY_NATIVE_JOB_CONTRACT],
                "excluded_runtime_contracts": [],
            }),
        )?;
        let Some(job) = parse_optional_legacy_job_field(&payload, "claimed_job", "claim-next-job")?
        else {
            return Ok(None);
        };
        if job.state != "running" {
            return Err(RunnerError::Server(format!(
                "legacy claim-next-job returned state `{}` instead of `running`",
                job.state
            )));
        }
        Ok(Some(job))
    }

    fn heartbeat_legacy_job(
        &self,
        job_id: i64,
        worker_id: &str,
    ) -> Result<LegacyQueueJob, RunnerError> {
        let payload = self.legacy_queue_operation(
            "heartbeat-job",
            json!({
                "job_id": job_id,
                "worker_id": worker_id,
            }),
        )?;
        let job = parse_legacy_job_field(&payload, "job", "heartbeat-job")?;
        if job.state != "running" {
            return Err(RunnerError::Server(format!(
                "legacy heartbeat-job returned state `{}` instead of `running`",
                job.state
            )));
        }
        Ok(job)
    }

    fn complete_legacy_job(
        &self,
        job_id: i64,
        worker_id: &str,
        result: &NativeResult,
    ) -> Result<LegacyQueueJob, RunnerError> {
        result.validate_bound()?;
        let result = serde_json::to_value(result).map_err(|error| {
            RunnerError::Server(format!(
                "could not encode legacy complete-job result: {error}"
            ))
        })?;
        let payload = self.legacy_queue_operation(
            "complete-job",
            json!({
                "job_id": job_id,
                "worker_id": worker_id,
                "result": result,
            }),
        )?;
        let job = parse_legacy_job_field(&payload, "job", "complete-job")?;
        if job.state != "succeeded" {
            return Err(RunnerError::Server(format!(
                "legacy complete-job returned state `{}` instead of `succeeded`",
                job.state
            )));
        }
        Ok(job)
    }

    fn fail_legacy_job(
        &self,
        job_id: i64,
        worker_id: &str,
        error: &str,
        retryable: bool,
    ) -> Result<LegacyQueueJob, RunnerError> {
        let payload = self.legacy_queue_operation(
            "fail-job",
            json!({
                "job_id": job_id,
                "worker_id": worker_id,
                "error": bounded_text(error, MAX_FAILURE_MESSAGE_CHARS),
                "retryable": retryable,
            }),
        )?;
        let job = parse_legacy_job_field(&payload, "job", "fail-job")?;
        if !matches!(job.state.as_str(), "queued" | "failed") {
            return Err(RunnerError::Server(format!(
                "legacy fail-job returned unexpected state `{}`",
                job.state
            )));
        }
        Ok(job)
    }

    fn legacy_queue_operation(
        &self,
        operation: &str,
        mut body: JsonValue,
    ) -> Result<JsonValue, RunnerError> {
        body["operation"] = JsonValue::String(operation.to_string());
        let request = self.auth(
            self.client
                .post(format!(
                    "{}/v1/worker-queue/operations/{operation}",
                    self.base_url
                ))
                .json(&body),
        );
        let payload = decode_response(request.send().map_err(|error| {
            RunnerError::ServerUnavailable(format!(
                "legacy worker queue {operation} request failed: {error}"
            ))
        })?)?;
        if payload.get("contract").and_then(JsonValue::as_str)
            != Some(LEGACY_QUEUE_SERVICE_CONTRACT)
        {
            return Err(RunnerError::Server(format!(
                "legacy worker queue {operation} returned an unsupported contract"
            )));
        }
        if payload.get("operation").and_then(JsonValue::as_str) != Some(operation) {
            return Err(RunnerError::Server(format!(
                "legacy worker queue response operation does not match `{operation}`"
            )));
        }
        Ok(payload)
    }

    fn auth(&self, request: RequestBuilder) -> RequestBuilder {
        match self.bearer_token.as_deref() {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }

    fn repository_pack_url(
        &self,
        source: &RemoteSnapshotReference,
        resource: &str,
        identity: &str,
    ) -> Result<reqwest::Url, RunnerError> {
        let mut url = reqwest::Url::parse(&self.base_url)
            .map_err(|error| RunnerError::Server(format!("invalid server base URL: {error}")))?;
        let mut segments = url.path_segments_mut().map_err(|_| {
            RunnerError::Server("ait-server base URL cannot carry path segments".to_string())
        })?;
        segments.pop_if_empty();
        segments.extend(["v1", "native"]);
        match source.repository_index {
            Some(repository_index) => {
                let repository_index = repository_index.to_string();
                segments.extend(["repository-authorities", repository_index.as_str()]);
            }
            None => match source.legacy_repo_id.as_deref() {
                Some(repo_id) => {
                    segments.extend(["repository-authorities", repo_id]);
                }
                None => {
                    segments.extend(["repositories", source.repository_name.as_str()]);
                }
            },
        }
        segments.extend(["remote-sync", "zstd-bulk", resource, identity]);
        drop(segments);
        Ok(url)
    }
}

impl QueueProtocol {
    const fn contract(self) -> &'static str {
        match self {
            Self::BinaryV3 => NATIVE_JOB_CONTRACT,
            Self::LegacyV1 => LEGACY_NATIVE_JOB_CONTRACT,
        }
    }
}

impl HeartbeatLease {
    fn start_binary(client: ServerClient, proof: WorkerLeaseProof, interval: Duration) -> Self {
        let request_timeout = heartbeat_request_timeout(interval);
        Self::start(interval, move || {
            client
                .heartbeat_binary_job(&proof, request_timeout)
                .map(|_| ())
        })
    }

    fn start_legacy(
        client: ServerClient,
        job_id: i64,
        worker_id: String,
        interval: Duration,
    ) -> Self {
        Self::start(interval, move || {
            client.heartbeat_legacy_job(job_id, &worker_id).map(|_| ())
        })
    }

    fn start<F>(interval: Duration, mut heartbeat: F) -> Self
    where
        F: FnMut() -> Result<(), RunnerError> + Send + 'static,
    {
        let (stop, stopped) = mpsc::channel();
        let error = Arc::new(Mutex::new(None));
        let thread_error = Arc::clone(&error);
        let thread = thread::spawn(move || {
            loop {
                match stopped.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if let Err(heartbeat_error) = heartbeat() {
                    if let Ok(mut slot) = thread_error.lock() {
                        *slot = Some(heartbeat_error.to_string());
                    }
                    break;
                }
            }
        });
        Self {
            stop,
            thread,
            error,
        }
    }

    fn finish(self) -> Result<(), RunnerError> {
        let _ = self.stop.send(());
        self.thread
            .join()
            .map_err(|_| RunnerError::Server("heartbeat thread panicked".to_string()))?;
        let error = self
            .error
            .lock()
            .map_err(|_| RunnerError::Server("heartbeat error lock poisoned".to_string()))?
            .clone();
        match error {
            Some(error) => Err(RunnerError::Server(format!(
                "runner lost the queue lease heartbeat: {error}"
            ))),
            None => Ok(()),
        }
    }
}

impl RemoteSnapshotProvider for ServerClient {
    fn fetch_import_manifest(
        &self,
        source: &RemoteSnapshotReference,
    ) -> Result<Vec<u8>, RunnerError> {
        let url = self.repository_pack_url(source, "import-manifests", &source.snapshot_id)?;
        let response = self.auth(self.client.get(url)).send().map_err(|error| {
            RunnerError::ServerUnavailable(format!(
                "remote Snapshot import manifest request failed: {error}"
            ))
        })?;
        decode_bounded_bytes(
            response,
            crate::materialize::MAX_IMPORT_MANIFEST_BYTES,
            "remote Snapshot import manifest",
        )
    }

    fn download_pack(
        &self,
        source: &RemoteSnapshotReference,
        kind: RemotePackKind,
        pack_id: &str,
        destination: &Path,
        maximum_bytes: u64,
    ) -> Result<u64, RunnerError> {
        let (resource, expected_media_type) = match kind {
            RemotePackKind::Object => ("object-packs", OBJECT_PACK_MEDIA_TYPE),
            RemotePackKind::Tree => ("tree-packs", TREE_PACK_MEDIA_TYPE),
        };
        let url = self.repository_pack_url(source, resource, pack_id)?;
        let mut response = self.auth(self.client.get(url)).send().map_err(|error| {
            RunnerError::ServerUnavailable(format!(
                "remote pack `{pack_id}` request failed: {error}"
            ))
        })?;
        if !response.status().is_success() {
            return Err(decode_error_response(
                response,
                &format!("remote pack `{pack_id}`"),
            ));
        }
        let media_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim);
        if media_type != Some(expected_media_type) {
            return Err(RunnerError::Server(format!(
                "remote pack `{pack_id}` returned unsupported content type `{}`; expected `{expected_media_type}`",
                media_type.unwrap_or("<missing>")
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > maximum_bytes)
        {
            return Err(RunnerError::Server(format!(
                "remote pack `{pack_id}` exceeds download bound {maximum_bytes} bytes"
            )));
        }
        let parent = destination.parent().ok_or_else(|| {
            RunnerError::Server(format!(
                "remote pack destination `{}` has no parent",
                destination.display()
            ))
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| RunnerError::fs("create remote pack directory", parent, error))?;
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .map_err(|error| RunnerError::fs("create remote pack file", destination, error))?;
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = response.read(&mut buffer).map_err(|error| {
                RunnerError::fs("read remote pack response", destination, error)
            })?;
            if read == 0 {
                break;
            }
            total = total.checked_add(read as u64).ok_or_else(|| {
                RunnerError::Server(format!("remote pack `{pack_id}` size overflowed u64"))
            })?;
            if total > maximum_bytes {
                return Err(RunnerError::Server(format!(
                    "remote pack `{pack_id}` exceeds download bound {maximum_bytes} bytes"
                )));
            }
            output
                .write_all(&buffer[..read])
                .map_err(|error| RunnerError::fs("write remote pack file", destination, error))?;
        }
        output
            .sync_all()
            .map_err(|error| RunnerError::fs("flush remote pack file", destination, error))?;
        Ok(total)
    }
}

fn validate_run_job_options(options: &RunJobOptions) -> Result<(), RunnerError> {
    let encoded = options.key.to_string();
    encoded.parse::<WorkerJobKey>().map_err(|error| {
        RunnerError::InvalidRequest(format!("invalid Worker Job pair key: {error}"))
    })?;
    Ok(())
}

fn validate_serve_options(options: &ServeOptions) -> Result<(), RunnerError> {
    validate_worker_id(&options.worker_id)?;
    if options.poll_interval.is_zero() || options.heartbeat_interval.is_zero() {
        return Err(RunnerError::InvalidRequest(
            "serve poll and heartbeat intervals must be greater than zero".to_string(),
        ));
    }
    if options
        .repository_indexes
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(RunnerError::InvalidRequest(
            "serve Repository indexes must be strictly increasing without duplicates".to_string(),
        ));
    }
    Ok(())
}

fn validate_worker_id(worker_id: &str) -> Result<(), RunnerError> {
    if worker_id.trim().is_empty() || worker_id.len() > 256 || worker_id.contains('\0') {
        return Err(RunnerError::InvalidRequest(
            "worker_id must be non-empty, at most 256 bytes, and contain no NUL byte".to_string(),
        ));
    }
    Ok(())
}

fn negotiate_protocol(health: &JsonValue) -> Result<QueueProtocol, RunnerError> {
    let capabilities = ServerOperationalCapabilities::from_server_payload(Some(health));
    if capabilities.require_binary_runtime().is_ok() {
        return Ok(QueueProtocol::BinaryV3);
    }
    let native_runner = health
        .get("ci_capabilities")
        .and_then(|value| value.get("native_runner"));
    let legacy_contract = native_runner
        .and_then(|value| value.get("contract").and_then(JsonValue::as_str))
        == Some(LEGACY_NATIVE_JOB_CONTRACT);
    let legacy_queue = native_runner
        .and_then(|value| value.get("queue_contract").and_then(JsonValue::as_str))
        == Some(LEGACY_QUEUE_SERVICE_CONTRACT);
    if legacy_contract && legacy_queue {
        return Ok(QueueProtocol::LegacyV1);
    }
    Err(RunnerError::Server(format!(
        "ait-server advertises neither complete `{NATIVE_JOB_V3_CONTRACT}` Binary capabilities nor the exact transitional `{LEGACY_NATIVE_JOB_CONTRACT}` queue contract"
    )))
}

fn binary_downgrade_error() -> RunnerError {
    RunnerError::Server(
        "ait-server attempted to downgrade a selected native-job.v3 session to legacy v1"
            .to_string(),
    )
}

fn reconnect_delay(poll_interval: Duration, consecutive_failures: u32) -> Duration {
    let cap = MAX_RECONNECT_BACKOFF.max(poll_interval);
    let mut delay = poll_interval;
    for _ in 0..consecutive_failures.min(16) {
        delay = delay.saturating_add(delay);
        if delay >= cap {
            return cap;
        }
    }
    delay.min(cap)
}

fn serve_job_error_event(claimed: &ClaimedJob, error: &RunnerError) -> JsonValue {
    let (repository_index, worker_job_index, attempt_count, legacy_job_id) = match claimed {
        ClaimedJob::Binary(claim) => (
            Some(claim.proof.repository_index.get()),
            Some(claim.proof.worker_job_index.get()),
            Some(claim.proof.attempt_count),
            None,
        ),
        ClaimedJob::Legacy(claim) => (None, None, None, Some(claim.job_id)),
    };
    json!({
        "contract": "ait.runner.serve-event.v1",
        "event": "job_failed",
        "status": "continuing",
        "repository_index": repository_index,
        "worker_job_index": worker_job_index,
        "attempt_count": attempt_count,
        "legacy_job_id": legacy_job_id,
        "error": error.bounded_message(MAX_FAILURE_MESSAGE_CHARS),
    })
}

fn unix_time_s() -> Result<u32, RunnerError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| RunnerError::Server(format!("system time precedes Unix epoch: {error}")))?
        .as_secs();
    u32::try_from(seconds)
        .map_err(|_| RunnerError::Server("current Unix time exceeds u32".to_string()))
}

fn bounded_binary_heartbeat_interval(
    lease_expires_at_s: u32,
    now_s: u32,
    configured: Duration,
) -> Result<Duration, RunnerError> {
    if configured.is_zero() {
        return Err(RunnerError::InvalidRequest(
            "binary heartbeat interval must be greater than zero".to_string(),
        ));
    }
    let remaining_s = lease_expires_at_s.checked_sub(now_s).ok_or_else(|| {
        RunnerError::Server("claimed Binary Worker Job lease is already expired".to_string())
    })?;
    if remaining_s < 4 {
        return Err(RunnerError::Server(
            "claimed Binary Worker Job lease leaves no safe heartbeat window".to_string(),
        ));
    }
    let safe = Duration::from_secs(u64::from((remaining_s / 4).max(1)));
    Ok(configured.min(safe))
}

fn heartbeat_request_timeout(interval: Duration) -> Duration {
    interval
        .saturating_mul(2)
        .clamp(Duration::from_secs(1), Duration::from_secs(30))
}

fn parse_binary_claim_field(
    payload: &JsonValue,
    field: &str,
    operation: &str,
) -> Result<Option<BinaryClaim>, RunnerError> {
    let Some(value) = payload.get(field) else {
        return Err(RunnerError::Server(format!(
            "Binary Worker Job {operation} response requires `{field}`"
        )));
    };
    if value.is_null() {
        return Ok(None);
    }
    let job = value.as_object().ok_or_else(|| {
        RunnerError::Server(format!(
            "Binary Worker Job {operation} response `{field}` must be an object or null"
        ))
    })?;
    for forbidden in ["job_id", "repo_id", "repo_name"] {
        if job.contains_key(forbidden) {
            return Err(RunnerError::Server(format!(
                "native-job.v3 claim must not synthesize legacy field `{forbidden}`"
            )));
        }
    }
    let key = parse_worker_job_key(job, operation)?;
    let attempt_count = parse_u16(job, "attempt_count", operation)?;
    if attempt_count == 0 {
        return Err(RunnerError::Server(format!(
            "Binary Worker Job {operation} attempt_count must be non-zero"
        )));
    }
    let lease_token = required_text(job, "lease_token", operation)?;
    let proof = WorkerLeaseProof::new(key, attempt_count, lease_token)
        .map_err(|error| RunnerError::Server(format!("{operation} lease proof: {error}")))?;
    let lease_expires_at_s = parse_u32(job, "lease_expires_at_s", operation)?;
    let state_kind = parse_u8(job, "state_kind", operation)?;
    if state_kind != 2 {
        return Err(RunnerError::Server(format!(
            "Binary Worker Job {operation} returned state_kind {state_kind}, expected running (2)"
        )));
    }
    let job_kind = parse_u8(job, "job_kind", operation)?;
    let expected_type = match job_kind {
        7 => "patchset.ci",
        11 => "repo.ci",
        other => {
            return Err(RunnerError::Server(format!(
                "native runner cannot execute Binary job_kind {other}"
            )));
        }
    };
    if required_text(job, "job_type", operation)? != expected_type {
        return Err(RunnerError::Server(format!(
            "Binary Worker Job {operation} job_type does not match job_kind {job_kind}"
        )));
    }
    let runtime_request = job
        .get("runtime_request")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            RunnerError::Server(format!(
                "Binary Worker Job {operation} requires object `runtime_request`"
            ))
        })?;
    let encoded = serde_json::to_vec(runtime_request).map_err(|error| {
        RunnerError::Server(format!(
            "{operation} runtime_request could not be encoded: {error}"
        ))
    })?;
    let request = NativeJobRequest::parse_bounded(&encoded)?;
    match &request.source {
        SourceSpec::RemoteSnapshot {
            repository_index, ..
        } if *repository_index == key.repository_index => {}
        SourceSpec::RemoteSnapshot {
            repository_index, ..
        } => {
            return Err(RunnerError::Server(format!(
                "runtime_request Repository index {repository_index} does not match claimed {}",
                key.repository_index
            )));
        }
        _ => {
            return Err(RunnerError::Server(
                "claimed native-job.v3 requires numeric remote_snapshot source routing".to_string(),
            ));
        }
    }
    Ok(Some(BinaryClaim {
        proof,
        lease_expires_at_s,
        job_kind,
        request,
    }))
}

fn parse_binary_job_state(
    payload: &JsonValue,
    proof: &WorkerLeaseProof,
    operation: &str,
    accepted_states: &[u8],
) -> Result<u8, RunnerError> {
    let job = payload
        .get("job")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            RunnerError::Server(format!(
                "Binary Worker Job {operation} response requires object `job`"
            ))
        })?;
    let key = parse_worker_job_key(job, operation)?;
    let attempt_count = parse_u16(job, "attempt_count", operation)?;
    if key != proof.key() || attempt_count != proof.attempt_count {
        return Err(RunnerError::Server(format!(
            "Binary Worker Job {operation} response changed pair identity or attempt"
        )));
    }
    let state_kind = parse_u8(job, "state_kind", operation)?;
    if !accepted_states.contains(&state_kind) {
        return Err(RunnerError::Server(format!(
            "Binary Worker Job {operation} returned unexpected state_kind {state_kind}"
        )));
    }
    Ok(state_kind)
}

fn parse_worker_job_key(
    object: &serde_json::Map<String, JsonValue>,
    operation: &str,
) -> Result<WorkerJobKey, RunnerError> {
    Ok(WorkerJobKey::new(
        RepositoryIndex::new(parse_u32(object, "repository_index", operation)?),
        WorkerJobIndex::new(parse_u32(object, "worker_job_index", operation)?),
    ))
}

fn parse_u8(
    object: &serde_json::Map<String, JsonValue>,
    field: &str,
    operation: &str,
) -> Result<u8, RunnerError> {
    let value = object
        .get(field)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| {
            RunnerError::Server(format!(
                "Binary Worker Job {operation} requires unsigned integer `{field}`"
            ))
        })?;
    u8::try_from(value).map_err(|_| {
        RunnerError::Server(format!(
            "Binary Worker Job {operation} `{field}` does not fit u8"
        ))
    })
}

fn parse_u16(
    object: &serde_json::Map<String, JsonValue>,
    field: &str,
    operation: &str,
) -> Result<u16, RunnerError> {
    let value = object
        .get(field)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| {
            RunnerError::Server(format!(
                "Binary Worker Job {operation} requires unsigned integer `{field}`"
            ))
        })?;
    u16::try_from(value).map_err(|_| {
        RunnerError::Server(format!(
            "Binary Worker Job {operation} `{field}` does not fit u16"
        ))
    })
}

fn parse_u32(
    object: &serde_json::Map<String, JsonValue>,
    field: &str,
    operation: &str,
) -> Result<u32, RunnerError> {
    let value = object
        .get(field)
        .and_then(JsonValue::as_u64)
        .ok_or_else(|| {
            RunnerError::Server(format!(
                "Binary Worker Job {operation} requires unsigned integer `{field}`"
            ))
        })?;
    u32::try_from(value).map_err(|_| {
        RunnerError::Server(format!(
            "Binary Worker Job {operation} `{field}` does not fit u32"
        ))
    })
}

fn required_text(
    object: &serde_json::Map<String, JsonValue>,
    field: &str,
    operation: &str,
) -> Result<String, RunnerError> {
    object
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            RunnerError::Server(format!(
                "Binary Worker Job {operation} requires non-empty `{field}`"
            ))
        })
}

fn compatible_legacy_native_request(job: &LegacyQueueJob) -> Result<NativeJobRequest, RunnerError> {
    if job.state != "running" {
        return Err(RunnerError::InvalidRequest(format!(
            "legacy job {} must be running after claim; state is `{}`",
            job.job_id, job.state
        )));
    }
    if !matches!(job.job_type.as_str(), "repo.ci" | "patchset.ci") {
        return Err(RunnerError::InvalidRequest(format!(
            "legacy job {} has unsupported job_type `{}`",
            job.job_id, job.job_type
        )));
    }
    let runtime_payload = job
        .payload
        .get("runtime_payload")
        .filter(|value| !value.is_null())
        .ok_or_else(|| {
            RunnerError::InvalidRequest(format!(
                "legacy job {} has no payload.runtime_payload",
                job.job_id
            ))
        })?;
    if runtime_payload.get("contract").and_then(JsonValue::as_str)
        != Some(LEGACY_NATIVE_JOB_CONTRACT)
    {
        return Err(RunnerError::InvalidRequest(format!(
            "legacy job {} runtime payload contract must be `{LEGACY_NATIVE_JOB_CONTRACT}`",
            job.job_id
        )));
    }
    let encoded = serde_json::to_vec(runtime_payload).map_err(|error| {
        RunnerError::InvalidRequest(format!(
            "legacy job {} runtime payload could not be encoded: {error}",
            job.job_id
        ))
    })?;
    if encoded.len() > MAX_REQUEST_BYTES {
        return Err(RunnerError::InvalidRequest(format!(
            "legacy job {} runtime payload exceeds {MAX_REQUEST_BYTES} bytes",
            job.job_id
        )));
    }
    NativeJobRequest::parse_legacy_bounded(&encoded)
}

fn parse_legacy_job_field(
    payload: &JsonValue,
    field: &str,
    operation: &str,
) -> Result<LegacyQueueJob, RunnerError> {
    let job = payload
        .get(field)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            RunnerError::Server(format!(
                "legacy worker queue {operation} response requires object field `{field}`"
            ))
        })?;
    let job_id = job
        .get("job_id")
        .and_then(JsonValue::as_i64)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            RunnerError::Server(format!(
                "legacy worker queue {operation} job requires positive integer job_id"
            ))
        })?;
    let text = |name: &str| {
        job.get(name)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| RunnerError::Server(format!("{operation} job requires `{name}`")))
    };
    let payload = job.get("payload").cloned().ok_or_else(|| {
        RunnerError::Server(format!("{operation} job requires decoded `payload`"))
    })?;
    if !payload.is_object() {
        return Err(RunnerError::Server(format!(
            "{operation} job payload must be an object"
        )));
    }
    Ok(LegacyQueueJob {
        job_id,
        repo_name: text("repo_name")?,
        job_type: text("job_type")?,
        state: text("state")?,
        payload,
    })
}

fn parse_optional_legacy_job_field(
    payload: &JsonValue,
    field: &str,
    operation: &str,
) -> Result<Option<LegacyQueueJob>, RunnerError> {
    match payload.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(_)) => parse_legacy_job_field(payload, field, operation).map(Some),
        Some(_) => Err(RunnerError::Server(format!(
            "legacy worker queue {operation} response field `{field}` must be an object or null"
        ))),
    }
}

fn normalize_base_url(raw: &str) -> Result<String, RunnerError> {
    let normalized = raw.trim().trim_end_matches('/');
    if normalized.is_empty() {
        return Err(RunnerError::Server(
            "ait-server base URL is required".to_string(),
        ));
    }
    let parsed = reqwest::Url::parse(normalized)
        .map_err(|error| RunnerError::Server(format!("invalid ait-server URL: {error}")))?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
    {
        return Err(RunnerError::Server(
            "ait-server URL must be an http(s) origin without path, query, or fragment".to_string(),
        ));
    }
    Ok(normalized.to_string())
}

fn decode_response(response: Response) -> Result<JsonValue, RunnerError> {
    let status = response.status();
    let bytes = decode_bounded_bytes(response, MAX_SERVER_RESPONSE_BYTES, "ait-server response")?;
    serde_json::from_slice(&bytes).map_err(|error| {
        RunnerError::Server(format!(
            "ait-server returned non-JSON status {}: {error}",
            status.as_u16()
        ))
    })
}

fn decode_bounded_bytes(
    mut response: Response,
    maximum_bytes: usize,
    label: &str,
) -> Result<Vec<u8>, RunnerError> {
    let status = response.status();
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes as u64)
    {
        return Err(RunnerError::Server(format!(
            "{label} exceeds {maximum_bytes} bytes"
        )));
    }
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take((maximum_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            RunnerError::ServerUnavailable(format!("could not read {label}: {error}"))
        })?;
    if bytes.len() > maximum_bytes {
        return Err(RunnerError::Server(format!(
            "{label} exceeds {maximum_bytes} bytes"
        )));
    }
    if !status.is_success() {
        let payload = serde_json::from_slice::<JsonValue>(&bytes).unwrap_or(JsonValue::Null);
        let detail = payload
            .get("error")
            .and_then(JsonValue::as_str)
            .unwrap_or("request rejected");
        let message = format!(
            "ait-server returned HTTP {}: {}",
            status.as_u16(),
            bounded_text(detail, MAX_FAILURE_MESSAGE_CHARS)
        );
        return Err(if status.as_u16() == 429 || status.is_server_error() {
            RunnerError::ServerUnavailable(message)
        } else {
            RunnerError::Server(message)
        });
    }
    Ok(bytes)
}

fn decode_error_response(response: Response, label: &str) -> RunnerError {
    match decode_bounded_bytes(response, MAX_SERVER_RESPONSE_BYTES, label) {
        Ok(_) => RunnerError::Server(format!("{label} returned an unexpected error response")),
        Err(error) => error,
    }
}

fn bounded_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut bounded = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    bounded.push('…');
    bounded
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::thread;
    use std::time::Instant;

    use tempfile::TempDir;

    use super::*;
    use crate::protocol::{CleanupEvidence, NativeResult, TerminalStatus, TestStatus};
    use crate::{ExecutorConfig, NATIVE_RESULT_CONTRACT};

    #[derive(Clone, Debug)]
    struct RecordedRequest {
        path: String,
        body: JsonValue,
    }

    struct MockServer {
        url: String,
        requests: Arc<Mutex<Vec<RecordedRequest>>>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl MockServer {
        fn start(responses: Vec<(u16, JsonValue)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let address = listener.local_addr().expect("mock address");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&requests);
            let thread = thread::spawn(move || {
                for (status, response) in responses {
                    let (mut stream, _) = listener.accept().expect("accept request");
                    let (path, body) = read_request(&mut stream);
                    captured
                        .lock()
                        .expect("request lock")
                        .push(RecordedRequest { path, body });
                    let encoded = serde_json::to_vec(&response).expect("encode response");
                    let reason = if status < 300 { "OK" } else { "Error" };
                    write!(
                        stream,
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        encoded.len()
                    )
                    .expect("response headers");
                    stream.write_all(&encoded).expect("response body");
                }
            });
            Self {
                url: format!("http://{address}"),
                requests,
                thread: Some(thread),
            }
        }

        fn finish(mut self) -> Vec<RecordedRequest> {
            self.thread
                .take()
                .expect("thread")
                .join()
                .expect("mock join");
            Arc::try_unwrap(self.requests)
                .expect("request owners")
                .into_inner()
                .expect("request mutex")
        }

        fn wait_for_requests(&self, expected: usize, timeout: Duration) {
            let deadline = Instant::now() + timeout;
            loop {
                let observed = self.requests.lock().expect("request lock").len();
                if observed >= expected {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {expected} mock requests; observed {observed}"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> (String, JsonValue) {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end = loop {
            let read = stream.read(&mut buffer).expect("request read");
            assert!(read > 0, "request ended before headers");
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8(bytes[..header_end].to_vec()).expect("utf8 headers");
        let path = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .expect("request path")
            .to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .unwrap_or(0);
        while bytes.len() - header_end < content_length {
            let read = stream.read(&mut buffer).expect("body read");
            assert!(read > 0, "request ended before body");
            bytes.extend_from_slice(&buffer[..read]);
        }
        let body = if content_length == 0 {
            json!({})
        } else {
            serde_json::from_slice(&bytes[header_end..header_end + content_length])
                .expect("request JSON")
        };
        (path, body)
    }

    fn binary_health() -> JsonValue {
        json!({
            "ready": true,
            "operational_capabilities": {
                "contract": "ait.server.operational-capabilities.v1",
                "repository_identity": "binary-repository-index.v0",
                "worker_job_identity": "binary-worker-job-key.v0",
                "runner_contracts": [NATIVE_JOB_CONTRACT],
            }
        })
    }

    fn legacy_health() -> JsonValue {
        json!({
            "ready": true,
            "ci_capabilities": {
                "native_runner": {
                    "contract": LEGACY_NATIVE_JOB_CONTRACT,
                    "queue_contract": LEGACY_QUEUE_SERVICE_CONTRACT,
                }
            }
        })
    }

    fn binary_service(operation: &str, field: &str, value: JsonValue) -> JsonValue {
        json!({
            "contract": BINARY_QUEUE_SERVICE_CONTRACT,
            "operation": operation,
            field: value,
        })
    }

    fn binary_job_state(key: WorkerJobKey, attempt_count: u16, state_kind: u8) -> JsonValue {
        json!({
            "repository_index": key.repository_index,
            "worker_job_index": key.worker_job_index,
            "attempt_count": attempt_count,
            "state_kind": state_kind,
        })
    }

    fn v3_runtime_request(repository_index: RepositoryIndex) -> JsonValue {
        json!({
            "contract": NATIVE_JOB_CONTRACT,
            "source": {
                "kind": "remote_snapshot",
                "repository_index": repository_index,
                "repository_name": "duplicate-name-is-display-only",
                "snapshot_id": "SNP-ABC",
                "external_repository_indexes": {},
            },
            "command": {"argv": ["./ci/run", "patchset"]},
            "timeout_ms": 5000,
        })
    }

    fn binary_claim(key: WorkerJobKey, attempt_count: u16, token: &str, job_kind: u8) -> JsonValue {
        json!({
            "repository_index": key.repository_index,
            "worker_job_index": key.worker_job_index,
            "attempt_count": attempt_count,
            "lease_token": token,
            "lease_expires_at_s": u32::MAX,
            "job_kind": job_kind,
            "job_type": if job_kind == 7 { "patchset.ci" } else { "repo.ci" },
            "state_kind": 2,
            "runtime_request": v3_runtime_request(key.repository_index),
        })
    }

    fn legacy_runtime_request() -> JsonValue {
        json!({
            "contract": LEGACY_NATIVE_JOB_CONTRACT,
            "source": {"kind": "local_directory", "path": "."},
            "command": {"argv": ["./ci/run.sh", "patchset"]},
            "timeout_ms": 5000,
        })
    }

    fn legacy_job(state: &str, runtime_payload: JsonValue) -> JsonValue {
        json!({
            "job_id": 41,
            "repo_name": "sample",
            "job_type": "repo.ci",
            "state": state,
            "payload": {
                "repo_name": "sample",
                "runtime_payload": runtime_payload,
            }
        })
    }

    fn legacy_service(operation: &str, field: &str, job: JsonValue) -> JsonValue {
        json!({
            "contract": LEGACY_QUEUE_SERVICE_CONTRACT,
            "operation": operation,
            field: job,
        })
    }

    fn source(script: &str) -> TempDir {
        let source = tempfile::tempdir().expect("source");
        fs::create_dir(source.path().join("ci")).expect("ci dir");
        let run = source.path().join("ci/run.sh");
        fs::write(&run, script).expect("script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        source
    }

    fn executor(source: &TempDir, attempts: &TempDir) -> NativeExecutor {
        NativeExecutor::new(ExecutorConfig {
            source_root: source.path().to_path_buf(),
            attempt_root: attempts.path().to_path_buf(),
        })
    }

    fn terminal_result() -> NativeResult {
        NativeResult {
            contract: NATIVE_RESULT_CONTRACT,
            status: TerminalStatus::Succeeded,
            tests_status: TestStatus::Pass,
            suite_result_count: 0,
            suite_results: Vec::new(),
            cleanup: CleanupEvidence {
                attempt_root_removed: true,
                remaining_owned_paths: 0,
            },
        }
    }

    #[test]
    fn doctor_prefers_complete_binary_v3_and_admits_exact_legacy_transition() {
        let binary = MockServer::start(vec![(200, binary_health())]);
        let report = ServerClient::new(&binary.url, None)
            .unwrap()
            .doctor()
            .expect("binary doctor");
        assert_eq!(report["selected_runner_contract"], NATIVE_JOB_CONTRACT);
        binary.finish();

        let legacy = MockServer::start(vec![(200, legacy_health())]);
        let report = ServerClient::new(&legacy.url, None)
            .unwrap()
            .doctor()
            .expect("legacy doctor");
        assert_eq!(
            report["selected_runner_contract"],
            LEGACY_NATIVE_JOB_CONTRACT
        );
        legacy.finish();

        let incomplete = MockServer::start(vec![(200, json!({"ready": true}))]);
        let error = ServerClient::new(&incomplete.url, None)
            .unwrap()
            .doctor()
            .expect_err("incomplete capabilities");
        assert!(error.to_string().contains("neither complete"));
        incomplete.finish();

        let previous_binary = MockServer::start(vec![(
            200,
            json!({
                "ready": true,
                "operational_capabilities": {
                    "contract": "ait.server.operational-capabilities.v1",
                    "repository_identity": "binary-repository-index.v0",
                    "worker_job_identity": "binary-worker-job-key.v0",
                    "runner_contracts": ["ait.runner.native-job.v2"],
                }
            }),
        )]);
        let error = ServerClient::new(&previous_binary.url, None)
            .unwrap()
            .doctor()
            .expect_err("v2 must not be reinterpreted as v3");
        assert!(error.to_string().contains(NATIVE_JOB_V3_CONTRACT));
        previous_binary.finish();
    }

    #[test]
    fn claim_next_v3_sends_only_numeric_filters_and_pair_contract() {
        let server = MockServer::start(vec![
            (200, binary_health()),
            (200, binary_service("claim", "claimed_job", JsonValue::Null)),
        ]);
        let source = source("#!/bin/sh\nexit 0\n");
        let attempts = tempfile::tempdir().expect("attempts");
        let report = ServerClient::new(&server.url, None)
            .unwrap()
            .serve(
                &executor(&source, &attempts),
                &ServeOptions {
                    worker_id: "diagnostic-runner".to_string(),
                    repository_indexes: vec![RepositoryIndex::new(1), RepositoryIndex::new(4)],
                    once: true,
                    poll_interval: Duration::from_millis(5),
                    heartbeat_interval: Duration::from_secs(60),
                },
            )
            .expect("idle v3");
        assert_eq!(report["status"], "idle");
        let requests = server.finish();
        assert_eq!(requests[1].path, "/v1/native/worker-jobs:claim");
        assert_eq!(requests[1].body["accepted_job_kinds"], json!([7, 11]));
        assert_eq!(requests[1].body["repository_indexes"], json!([1, 4]));
        assert_eq!(
            requests[1].body["accepted_runtime_contracts"],
            json!([NATIVE_JOB_CONTRACT])
        );
        let encoded = requests[1].body.to_string();
        assert!(!encoded.contains("job_id"));
        assert!(!encoded.contains("repo_name"));
        assert!(!encoded.contains("worker_id"));
    }

    #[test]
    fn exact_pair_claim_rejects_legacy_server_without_synthesizing_job_id() {
        let server = MockServer::start(vec![(200, legacy_health())]);
        let source = source("#!/bin/sh\nexit 0\n");
        let attempts = tempfile::tempdir().expect("attempts");
        let error = ServerClient::new(&server.url, None)
            .unwrap()
            .run_job(
                &executor(&source, &attempts),
                &RunJobOptions {
                    key: WorkerJobKey::new(RepositoryIndex::new(1), WorkerJobIndex::new(2)),
                },
            )
            .expect_err("legacy server cannot provide pair identity");
        assert!(error.to_string().contains("explicit pair-key run"));
        assert_eq!(server.finish().len(), 1);
    }

    #[test]
    fn every_v3_terminal_request_carries_exact_pair_attempt_and_token() {
        let key = WorkerJobKey::new(RepositoryIndex::new(4), WorkerJobIndex::new(9));
        let proof =
            WorkerLeaseProof::new(key, 3, "00112233445566778899aabbccddeeff").expect("proof");
        let server = MockServer::start(vec![
            (
                200,
                binary_service("heartbeat", "job", binary_job_state(key, 3, 2)),
            ),
            (
                200,
                binary_service("complete", "job", binary_job_state(key, 3, 3)),
            ),
            (
                200,
                binary_service("fail", "job", binary_job_state(key, 3, 1)),
            ),
        ]);
        let client = ServerClient::new(&server.url, None).unwrap();
        assert_eq!(
            client
                .heartbeat_binary_job(&proof, Duration::from_secs(5))
                .unwrap(),
            2
        );
        assert_eq!(
            client
                .complete_binary_job(&proof, &terminal_result(), 7)
                .unwrap(),
            3
        );
        assert_eq!(
            client.fail_binary_job(&proof, "retry this", true).unwrap(),
            1
        );
        let requests = server.finish();
        for (request, operation) in requests.iter().zip(["heartbeat", "complete", "fail"]) {
            assert_eq!(
                request.path,
                format!("/v1/native/repository-authorities/4/worker-jobs/9:{operation}")
            );
            assert_eq!(request.body["attempt_count"], 3);
            assert_eq!(
                request.body["lease_token"],
                "00112233445566778899aabbccddeeff"
            );
            assert!(request.body.get("job_id").is_none());
            assert!(request.body.get("worker_id").is_none());
        }
    }

    #[test]
    fn v3_claim_parser_validates_pair_attempt_token_and_numeric_source_route() {
        let key = WorkerJobKey::new(RepositoryIndex::new(7), WorkerJobIndex::new(12));
        let payload = binary_service(
            "claim",
            "claimed_job",
            binary_claim(key, 2, "00112233445566778899aabbccddeeff", 7),
        );
        let claim = parse_binary_claim_field(&payload, "claimed_job", "claim")
            .expect("parse")
            .expect("claim");
        assert_eq!(claim.proof.key(), key);
        assert_eq!(claim.proof.attempt_count, 2);
        assert_eq!(claim.lease_expires_at_s, u32::MAX);

        let mut legacy_field = binary_claim(key, 2, "00112233445566778899aabbccddeeff", 7);
        legacy_field["job_id"] = json!(99);
        let payload = binary_service("claim", "claimed_job", legacy_field);
        assert!(parse_binary_claim_field(&payload, "claimed_job", "claim").is_err());

        let mut wrong_route = binary_claim(key, 2, "00112233445566778899aabbccddeeff", 7);
        wrong_route["runtime_request"]["source"]["repository_index"] = json!(8);
        let payload = binary_service("claim", "claimed_job", wrong_route);
        assert!(parse_binary_claim_field(&payload, "claimed_job", "claim").is_err());
    }

    #[test]
    fn numeric_remote_pack_route_never_uses_duplicate_repository_name() {
        let client = ServerClient::new("https://example.test", None).unwrap();
        let source = RemoteSnapshotReference {
            repository_index: Some(RepositoryIndex::new(4)),
            repository_name: "duplicate".to_string(),
            legacy_repo_id: None,
            snapshot_id: "SNP-ABC".to_string(),
            external_repository_indexes: BTreeMap::new(),
        };
        let url = client
            .repository_pack_url(&source, "import-manifests", "SNP-ABC")
            .expect("numeric URL");
        assert_eq!(
            url.path(),
            "/v1/native/repository-authorities/4/remote-sync/zstd-bulk/import-manifests/SNP-ABC"
        );
        assert!(!url.as_str().contains("duplicate"));
    }

    #[test]
    fn legacy_transition_executes_and_preserves_attempt_cleanup() {
        let runtime = legacy_runtime_request();
        let server = MockServer::start(vec![
            (200, legacy_health()),
            (
                200,
                legacy_service(
                    "claim-next-job",
                    "claimed_job",
                    legacy_job("running", runtime),
                ),
            ),
            (
                200,
                legacy_service(
                    "complete-job",
                    "job",
                    legacy_job("succeeded", JsonValue::Null),
                ),
            ),
        ]);
        let source = source("#!/bin/sh\nset -eu\nprintf 'legacy-ok'\n");
        let attempts = tempfile::tempdir().expect("attempts");
        let report = ServerClient::new(&server.url, None)
            .unwrap()
            .serve(
                &executor(&source, &attempts),
                &ServeOptions {
                    worker_id: "legacy-runner".to_string(),
                    repository_indexes: Vec::new(),
                    once: true,
                    poll_interval: Duration::from_millis(5),
                    heartbeat_interval: Duration::from_secs(60),
                },
            )
            .expect("legacy delivery");
        assert_eq!(report["status"], "delivered");
        assert!(
            fs::read_dir(attempts.path())
                .expect("attempt root")
                .all(|entry| {
                    entry.expect("attempt entry").file_name().to_string_lossy()
                        == ".ait-runner-attempt-root.lock"
                }),
            "serve may retain only its root lock, never an attempt directory"
        );
        let requests = server.finish();
        assert_eq!(
            requests[1].path,
            "/v1/worker-queue/operations/claim-next-job"
        );
        assert_eq!(
            requests[1].body["accepted_runtime_contracts"],
            json!([LEGACY_NATIVE_JOB_CONTRACT])
        );
        assert_eq!(requests[2].path, "/v1/worker-queue/operations/complete-job");
    }

    #[test]
    fn legacy_transition_refuses_numeric_repository_filter() {
        let server = MockServer::start(vec![(200, legacy_health())]);
        let source = source("#!/bin/sh\nexit 0\n");
        let attempts = tempfile::tempdir().expect("attempts");
        let error = ServerClient::new(&server.url, None)
            .unwrap()
            .serve(
                &executor(&source, &attempts),
                &ServeOptions {
                    worker_id: "legacy-runner".to_string(),
                    repository_indexes: vec![RepositoryIndex::new(1)],
                    once: true,
                    poll_interval: Duration::from_millis(5),
                    heartbeat_interval: Duration::from_secs(60),
                },
            )
            .expect_err("numeric filter cannot become a name");
        assert!(error.to_string().contains("never translated"));
        server.finish();
    }

    #[test]
    fn binary_heartbeat_is_attempt_and_token_bound_and_stops_cleanly() {
        let key = WorkerJobKey::new(RepositoryIndex::new(2), WorkerJobIndex::new(3));
        let proof =
            WorkerLeaseProof::new(key, 4, "ffeeddccbbaa99887766554433221100").expect("proof");
        let server = MockServer::start(vec![(
            200,
            binary_service("heartbeat", "job", binary_job_state(key, 4, 2)),
        )]);
        let heartbeat = HeartbeatLease::start_binary(
            ServerClient::new(&server.url, None).unwrap(),
            proof,
            Duration::from_millis(10),
        );
        server.wait_for_requests(1, Duration::from_secs(2));
        heartbeat.finish().expect("heartbeat");
        let request = &server.finish()[0];
        assert_eq!(request.body["attempt_count"], 4);
        assert_eq!(
            request.body["lease_token"],
            "ffeeddccbbaa99887766554433221100"
        );
    }

    #[test]
    fn ambiguous_binary_heartbeat_error_is_not_retried() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&attempts);
        let heartbeat = HeartbeatLease::start(Duration::from_millis(10), move || {
            observed.fetch_add(1, Ordering::SeqCst);
            Err(RunnerError::ServerUnavailable(
                "heartbeat response timed out after server admission".to_string(),
            ))
        });
        let deadline = Instant::now() + Duration::from_secs(2);
        while attempts.load(Ordering::SeqCst) == 0 {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the first heartbeat attempt"
            );
            thread::sleep(Duration::from_millis(5));
        }
        thread::sleep(Duration::from_millis(40));
        let error = heartbeat
            .finish()
            .expect_err("ambiguous heartbeat must fail the Job");
        assert!(error.to_string().contains("heartbeat response timed out"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn binary_heartbeat_schedule_stays_inside_the_claimed_lease_window() {
        assert_eq!(
            bounded_binary_heartbeat_interval(160, 100, Duration::from_secs(30)).unwrap(),
            Duration::from_secs(15)
        );
        assert_eq!(
            bounded_binary_heartbeat_interval(160, 100, Duration::from_secs(10)).unwrap(),
            Duration::from_secs(10)
        );
        assert!(bounded_binary_heartbeat_interval(103, 100, Duration::from_secs(1)).is_err());
        assert_eq!(
            heartbeat_request_timeout(Duration::from_secs(15)),
            Duration::from_secs(30)
        );
        assert_eq!(
            heartbeat_request_timeout(Duration::from_secs(10)),
            Duration::from_secs(20)
        );
        assert_eq!(
            heartbeat_request_timeout(Duration::from_millis(100)),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn persistent_serve_job_error_event_is_bounded_and_keeps_numeric_identity() {
        let key = WorkerJobKey::new(RepositoryIndex::new(4), WorkerJobIndex::new(9));
        let claim = parse_binary_claim_field(
            &binary_service(
                "claim",
                "claimed_job",
                binary_claim(key, 3, "00112233445566778899aabbccddeeff", 7),
            ),
            "claimed_job",
            "claim",
        )
        .unwrap()
        .unwrap();
        let event = serve_job_error_event(
            &ClaimedJob::Binary(claim),
            &RunnerError::Server("x".repeat(MAX_FAILURE_MESSAGE_CHARS + 10)),
        );

        assert_eq!(event["contract"], "ait.runner.serve-event.v1");
        assert_eq!(event["status"], "continuing");
        assert_eq!(event["repository_index"], 4);
        assert_eq!(event["worker_job_index"], 9);
        assert_eq!(event["attempt_count"], 3);
        assert!(event["error"].as_str().unwrap().chars().count() <= MAX_FAILURE_MESSAGE_CHARS);
    }

    #[test]
    fn reconnect_delay_is_exponential_and_bounded() {
        let base = Duration::from_millis(250);
        assert_eq!(reconnect_delay(base, 0), base);
        assert_eq!(reconnect_delay(base, 1), Duration::from_millis(500));
        assert_eq!(reconnect_delay(base, 3), Duration::from_secs(2));
        assert_eq!(reconnect_delay(base, 20), MAX_RECONNECT_BACKOFF);
    }
}
