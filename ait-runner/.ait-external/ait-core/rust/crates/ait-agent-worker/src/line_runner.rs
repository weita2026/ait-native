use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

use ait_agent_core::{
    agent_line_http_transaction_execute_json, AgentWorkerRuntimeConfig, LineWorkerConfig,
};
use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};

use crate::{
    run_worker_host, BoundedWorkerJobExecutor, WorkerDiagnostic, WorkerHttpCompletion,
    WorkerHttpDispatch, WorkerHttpHandler, WorkerHttpHostConfig, WorkerHttpHostRuntime,
    WorkerHttpRequest, WorkerHttpResponse, WorkerJobExecutorConfig, WorkerRunContext,
    EXIT_INVALID_CONFIGURATION, EXIT_RUNTIME_UNAVAILABLE,
};

const DEFAULT_LINE_MAX_INFLIGHT_JOBS: usize = 4;
const LINE_HTTP_REQUEST_DEADLINE: Duration = Duration::from_secs(120);
const LINE_HTTP_JOB_KIND: &str = "line.http_transaction";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

pub trait LineHttpTransactionExecutor: Clone + Send + Sync + 'static {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultLineHttpTransactionExecutor;

impl LineHttpTransactionExecutor for DefaultLineHttpTransactionExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_line_http_transaction_execute_json(request)
    }
}

#[derive(Clone)]
struct LineTransactionRequestConfig {
    channel_access_token: String,
    channel_secret: String,
    api_base_url: String,
    webhook_path: String,
    state_path: String,
    runtime_target: JsonValue,
    timeout_seconds: Option<f64>,
    local_reply: Option<JsonValue>,
}

pub struct LineWorkerHttpHandler<E = DefaultLineHttpTransactionExecutor> {
    request_config: LineTransactionRequestConfig,
    transaction_executor: E,
    jobs: BoundedWorkerJobExecutor<WorkerHttpResponse>,
}

impl<E> LineWorkerHttpHandler<E>
where
    E: LineHttpTransactionExecutor,
{
    pub fn new(
        config: &LineWorkerConfig,
        transaction_executor: E,
        max_inflight_jobs: usize,
    ) -> Result<Self, WorkerDiagnostic> {
        let shared = &config.shared;
        let target = &shared.runtime_target;
        let request_config = LineTransactionRequestConfig {
            channel_access_token: config.channel_access_token.expose().to_string(),
            channel_secret: config.channel_secret.expose().to_string(),
            api_base_url: config.api_base_url.clone(),
            webhook_path: config.webhook_path.clone(),
            state_path: shared.paths.sync_state_path.clone(),
            runtime_target: json!({
                "mode": target.mode.as_str(),
                "workflow_mode": target.workflow_mode.as_str(),
                "repo_name": target.repo_name,
                "repo_root": target.repo_root.to_string_lossy().to_string(),
                "remote_name": target.remote_name,
                "server_url": target.server_url,
            }),
            timeout_seconds: shared.request_timeout_seconds,
            local_reply: shared.local_reply.clone(),
        };
        let jobs = BoundedWorkerJobExecutor::new(WorkerJobExecutorConfig {
            max_inflight: max_inflight_jobs,
        })?;
        Ok(Self {
            request_config,
            transaction_executor,
            jobs,
        })
    }

    fn transaction_request(
        &self,
        request: WorkerHttpRequest,
    ) -> Result<JsonValue, WorkerHttpResponse> {
        let raw_payload = String::from_utf8(request.body)
            .map_err(|_| line_public_error(400, "LINE webhook payload must be UTF-8."))?;
        let signature = request.headers.get("x-line-signature").cloned();
        Ok(json!({
            "raw_payload": raw_payload,
            "signature": signature,
            "channel_secret": self.request_config.channel_secret,
            "request_path": request.path,
            "webhook_path": self.request_config.webhook_path,
            "state_path": self.request_config.state_path,
            "runtime_target": self.request_config.runtime_target,
            "channel_access_token": self.request_config.channel_access_token,
            "api_base_url": self.request_config.api_base_url,
            "timeout_seconds": self.request_config.timeout_seconds,
            "local_reply": self.request_config.local_reply,
        }))
    }
}

impl<E> WorkerHttpHandler for LineWorkerHttpHandler<E>
where
    E: LineHttpTransactionExecutor,
{
    fn handle(
        &mut self,
        request: WorkerHttpRequest,
    ) -> Result<WorkerHttpDispatch, WorkerDiagnostic> {
        let request = match self.transaction_request(request) {
            Ok(request) => request,
            Err(response) => return Ok(WorkerHttpDispatch::Immediate(response)),
        };
        let executor = self.transaction_executor.clone();
        match self.jobs.submit(LINE_HTTP_JOB_KIND, move || {
            let outcome = executor
                .execute(&request)
                .map_err(|_| line_transaction_failure())?;
            line_transaction_response(&outcome)
        }) {
            Ok(job_id) => Ok(WorkerHttpDispatch::Deferred { job_id }),
            Err(error)
                if matches!(
                    error.code,
                    "worker_job_capacity_exhausted" | "worker_job_executor_closed"
                ) =>
            {
                Ok(WorkerHttpDispatch::Immediate(line_public_error(
                    503,
                    "LINE webhook worker is busy.",
                )))
            }
            Err(error) => Err(error),
        }
    }

    fn poll_completed(&mut self) -> Vec<WorkerHttpCompletion> {
        self.jobs
            .poll_completed()
            .into_iter()
            .map(|completion| WorkerHttpCompletion {
                job_id: completion.job_id,
                result: completion.result,
            })
            .collect()
    }

    fn close_admission(&mut self) {
        self.jobs.close_admission();
    }

    fn inflight_work_count(&self) -> usize {
        self.jobs.inflight_count()
    }

    fn finish_shutdown(&mut self) -> Result<(), WorkerDiagnostic> {
        if self.jobs.inflight_count() == 0 {
            Ok(())
        } else {
            Err(WorkerDiagnostic::new(
                "line_worker_jobs_still_inflight",
                "Rust LINE worker jobs remain in flight during graceful shutdown.",
                EXIT_RUNTIME_UNAVAILABLE,
            ))
        }
    }

    fn force_shutdown(&mut self) -> Result<(), WorkerDiagnostic> {
        self.jobs.close_admission();
        self.jobs.force_detach();
        Ok(())
    }
}

pub fn run_line_transport(context: &WorkerRunContext) -> Result<(), WorkerDiagnostic> {
    let AgentWorkerRuntimeConfig::Line(config) = &context.config else {
        return Err(WorkerDiagnostic::new(
            "line_worker_config_mismatch",
            "The Rust LINE runner received a non-LINE worker configuration.",
            EXIT_INVALID_CONFIGURATION,
        ));
    };
    let bind_addr = resolve_line_bind_addr(config)?;
    let handler = LineWorkerHttpHandler::new(
        config,
        DefaultLineHttpTransactionExecutor,
        DEFAULT_LINE_MAX_INFLIGHT_JOBS,
    )?;
    let mut runtime = WorkerHttpHostRuntime::new(
        WorkerHttpHostConfig {
            bind_addr,
            expected_method: "POST".to_string(),
            expected_path: config.webhook_path.clone(),
            enforce_expected_path: false,
            request_timeout: LINE_HTTP_REQUEST_DEADLINE,
            ..WorkerHttpHostConfig::default()
        },
        handler,
    );
    run_worker_host(context, &mut runtime)
}

fn resolve_line_bind_addr(config: &LineWorkerConfig) -> Result<SocketAddr, WorkerDiagnostic> {
    let port = u16::try_from(config.bind_port)
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "line_worker_bind_port_invalid",
                "The Rust LINE worker bind port must be between 1 and 65535.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_port", config.bind_port)
        })?;
    (config.bind_host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| {
            WorkerDiagnostic::new(
                "line_worker_bind_address_invalid",
                format!(
                    "Cannot resolve the Rust LINE worker bind host `{}`: {error}",
                    config.bind_host
                ),
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_host", config.bind_host.clone())
        })?
        .next()
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "line_worker_bind_address_invalid",
                "The Rust LINE worker bind host did not resolve to an address.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_host", config.bind_host.clone())
        })
}

fn line_transaction_response(outcome: &JsonValue) -> Result<WorkerHttpResponse, WorkerDiagnostic> {
    let object = outcome
        .as_object()
        .ok_or_else(line_transaction_contract_failure)?;
    let status_code = object
        .get("http_status")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| (100..=599).contains(value))
        .ok_or_else(line_transaction_contract_failure)?;
    let write_json_response = object
        .get("write_json_response")
        .and_then(JsonValue::as_bool)
        .ok_or_else(line_transaction_contract_failure)?;
    if !write_json_response {
        return Ok(WorkerHttpResponse::new(status_code, Vec::new()));
    }
    let response = object
        .get("response")
        .ok_or_else(line_transaction_contract_failure)?;
    let body = JsonCodec::encode_value_to_vec_with_error_prefix(
        response,
        JsonEncodeOptions::compact(),
        "Failed to encode Rust LINE HTTP response",
    )
    .map_err(|_| line_transaction_contract_failure())?;
    Ok(WorkerHttpResponse::new(status_code, body).with_header("Content-Type", JSON_CONTENT_TYPE))
}

fn line_public_error(status_code: u16, message: &str) -> WorkerHttpResponse {
    let body = JsonCodec::encode_value_to_vec(
        &json!({"ok": false, "error": message}),
        JsonEncodeOptions::compact(),
    )
    .unwrap_or_else(|_| b"{\"ok\":false,\"error\":\"LINE webhook failed.\"}".to_vec());
    WorkerHttpResponse::new(status_code, body).with_header("Content-Type", JSON_CONTENT_TYPE)
}

fn line_transaction_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "line_http_transaction_failed",
        "The Rust LINE HTTP transaction failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn line_transaction_contract_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "line_http_transaction_contract_invalid",
        "The Rust LINE HTTP transaction returned an invalid response contract.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::net::SocketAddr;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use ait_agent_core::{
        resolve_agent_worker_config, AgentWorkerConfigInput, AgentWorkerRuntimeConfig,
    };
    use ait_core::json_support::{json, JsonCodec};
    use tempfile::tempdir;

    use super::*;

    #[derive(Clone)]
    struct StubExecutor {
        calls: Arc<Mutex<Vec<JsonValue>>>,
        results: Arc<Mutex<VecDeque<Result<JsonValue, String>>>>,
        gate: Option<Arc<(Mutex<bool>, Condvar)>>,
    }

    impl StubExecutor {
        fn new(results: Vec<Result<JsonValue, String>>) -> Self {
            Self {
                calls: Arc::default(),
                results: Arc::new(Mutex::new(results.into())),
                gate: None,
            }
        }

        fn blocked(result: Result<JsonValue, String>) -> (Self, Arc<(Mutex<bool>, Condvar)>) {
            let gate = Arc::new((Mutex::new(false), Condvar::new()));
            (
                Self {
                    calls: Arc::default(),
                    results: Arc::new(Mutex::new(VecDeque::from([result]))),
                    gate: Some(gate.clone()),
                },
                gate,
            )
        }
    }

    impl LineHttpTransactionExecutor for StubExecutor {
        fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
            self.calls.lock().expect("calls").push(request.clone());
            if let Some(gate) = &self.gate {
                let (lock, ready) = &**gate;
                let released = lock.lock().expect("gate lock");
                drop(
                    ready
                        .wait_while(released, |released| !*released)
                        .expect("gate wait"),
                );
            }
            self.results
                .lock()
                .expect("results")
                .pop_front()
                .expect("stub result")
        }
    }

    fn line_config() -> AgentWorkerRuntimeConfig {
        let temp = tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
        std::fs::write(
            temp.path().join(".ait/config.json"),
            r#"{"repo_name":"fixture","workflow_mode":"solo_remote","default_remote":"origin","remotes":{"origin":{"url":"http://127.0.0.1:8088"}}}"#,
        )
        .expect("repo config");
        resolve_agent_worker_config(AgentWorkerConfigInput {
            repo_root: temp.path().to_path_buf(),
            worker_key: "line/main".to_string(),
            worker: json!({
                "kind": "line",
                "name": "main",
                "token": "line-access-secret",
                "secret": "line-channel-secret",
                "webhook_path": "/callback",
                "api_base_url": "https://api.line.example",
                "request_timeout_seconds": 15,
                "local_reply": {"model": "fixture-model"},
            }),
            process_env: BTreeMap::new(),
        })
        .expect("line config")
    }

    fn handler<E: LineHttpTransactionExecutor>(
        executor: E,
        max_inflight: usize,
    ) -> LineWorkerHttpHandler<E> {
        let config = line_config();
        let AgentWorkerRuntimeConfig::Line(config) = config else {
            panic!("line config");
        };
        LineWorkerHttpHandler::new(&config, executor, max_inflight).expect("LINE handler")
    }

    fn request(body: Vec<u8>) -> WorkerHttpRequest {
        WorkerHttpRequest {
            method: "POST".to_string(),
            path: "/callback".to_string(),
            version: "HTTP/1.1".to_string(),
            headers: BTreeMap::from([(
                "x-line-signature".to_string(),
                "line-signature-secret".to_string(),
            )]),
            body,
            peer_addr: SocketAddr::from(([127, 0, 0, 1], 40000)),
        }
    }

    fn wait_completion<E: LineHttpTransactionExecutor>(
        handler: &mut LineWorkerHttpHandler<E>,
    ) -> WorkerHttpCompletion {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(completion) = handler.poll_completed().into_iter().next() {
                return completion;
            }
            assert!(Instant::now() < deadline, "LINE job completion timed out");
            thread::yield_now();
        }
    }

    #[test]
    fn typed_config_and_lowercase_signature_feed_deferred_transaction_response() {
        let executor = StubExecutor::new(vec![Ok(json!({
            "http_status": 201,
            "write_json_response": true,
            "response": {"ok": true, "processed_events": 2},
        }))]);
        let calls = executor.calls.clone();
        let mut handler = handler(executor, 2);

        let dispatch = handler
            .handle(request(br#"{"events":[]}"#.to_vec()))
            .expect("dispatch");
        let WorkerHttpDispatch::Deferred { job_id } = dispatch else {
            panic!("deferred dispatch");
        };
        let completion = wait_completion(&mut handler);

        assert_eq!(completion.job_id, job_id);
        let response = completion.result.expect("HTTP response");
        assert_eq!(response.status_code, 201);
        assert_eq!(response.headers["Content-Type"], JSON_CONTENT_TYPE);
        assert_eq!(
            JsonCodec::parse_slice_with_error_prefix(&response.body, "response JSON")
                .expect("response JSON"),
            json!({"ok": true, "processed_events": 2})
        );
        let calls = calls.lock().expect("calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["signature"], "line-signature-secret");
        assert_eq!(calls[0]["channel_secret"], "line-channel-secret");
        assert_eq!(calls[0]["channel_access_token"], "line-access-secret");
        assert_eq!(calls[0]["webhook_path"], "/callback");
        assert_eq!(calls[0]["runtime_target"]["mode"], "remote");
        assert_eq!(calls[0]["runtime_target"]["repo_name"], "fixture");
        assert_eq!(calls[0]["timeout_seconds"], 15.0);
        assert_eq!(calls[0]["local_reply"]["model"], "fixture-model");
        assert!(calls[0].get("peer_addr").is_none());
    }

    #[test]
    fn invalid_utf8_is_immediate_and_never_reaches_transaction() {
        let executor = StubExecutor::new(Vec::new());
        let calls = executor.calls.clone();
        let mut handler = handler(executor, 1);

        let dispatch = handler.handle(request(vec![0xff])).expect("dispatch");
        let WorkerHttpDispatch::Immediate(response) = dispatch else {
            panic!("immediate response");
        };

        assert_eq!(response.status_code, 400);
        assert!(calls.lock().expect("calls").is_empty());
        assert!(!String::from_utf8_lossy(&response.body).contains("line-access-secret"));
    }

    #[test]
    fn bounded_capacity_returns_public_503_and_recovers_after_reap() {
        let outcome = Ok(json!({
            "http_status": 200,
            "write_json_response": true,
            "response": {"ok": true, "processed_events": 0},
        }));
        let (executor, gate) = StubExecutor::blocked(outcome);
        let mut handler = handler(executor, 1);

        assert!(matches!(
            handler.handle(request(b"{}".to_vec())).expect("first"),
            WorkerHttpDispatch::Deferred { .. }
        ));
        let deadline = Instant::now() + Duration::from_secs(3);
        while handler.inflight_work_count() != 1 {
            assert!(Instant::now() < deadline, "first job did not start");
            thread::yield_now();
        }
        let second = handler.handle(request(b"{}".to_vec())).expect("second");
        let WorkerHttpDispatch::Immediate(second) = second else {
            panic!("capacity response");
        };
        assert_eq!(second.status_code, 503);

        let (lock, ready) = &*gate;
        *lock.lock().expect("gate lock") = true;
        ready.notify_all();
        assert!(wait_completion(&mut handler).result.is_ok());
        assert_eq!(handler.inflight_work_count(), 0);
    }

    #[test]
    fn transaction_failures_and_invalid_contracts_are_sanitized() {
        let cases = [
            Err("line-access-secret backend failure".to_string()),
            Ok(json!({"http_status": "invalid", "write_json_response": true})),
        ];
        for outcome in cases {
            let mut handler = handler(StubExecutor::new(vec![outcome]), 1);
            assert!(matches!(
                handler.handle(request(b"{}".to_vec())).expect("dispatch"),
                WorkerHttpDispatch::Deferred { .. }
            ));
            let error = wait_completion(&mut handler)
                .result
                .expect_err("transaction error");
            assert!(matches!(
                error.code,
                "line_http_transaction_failed" | "line_http_transaction_contract_invalid"
            ));
            assert!(!error.render_json().contains("line-access-secret"));
        }
    }

    #[test]
    fn no_body_transaction_response_preserves_status() {
        let mut handler = handler(
            StubExecutor::new(vec![Ok(json!({
                "http_status": 404,
                "write_json_response": false,
                "response": null,
            }))]),
            1,
        );
        let _ = handler.handle(request(b"{}".to_vec())).expect("dispatch");

        let response = wait_completion(&mut handler).result.expect("response");

        assert_eq!(response.status_code, 404);
        assert!(response.body.is_empty());
        assert!(response.headers.is_empty());
    }
}
