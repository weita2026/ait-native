use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

use ait_agent_core::{
    agent_slack_command_http_transaction_plan_json, agent_slack_command_job_execute_json,
    AgentWorkerRuntimeConfig, SlackWorkerConfig,
};
use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};

use crate::slack_command_once::{command_job_request, validate_command_job_contract};
use crate::{
    run_slack_socket_mode_transport, run_worker_host, BoundedWorkerJobExecutor, WorkerDiagnostic,
    WorkerHttpCompletion, WorkerHttpDispatch, WorkerHttpHandler, WorkerHttpHostConfig,
    WorkerHttpHostRuntime, WorkerHttpRequest, WorkerHttpResponse, WorkerJobExecutorConfig,
    WorkerRunContext, EXIT_INVALID_CONFIGURATION, EXIT_RUNTIME_UNAVAILABLE,
};

const DEFAULT_SLACK_MAX_INFLIGHT_JOBS: usize = 4;
const SLACK_HTTP_REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const SLACK_COMMAND_JOB_KIND: &str = "slack.command_job";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";

pub trait SlackHttpCommandJobExecutor: Clone + Send + Sync + 'static {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackHttpCommandJobExecutor;

impl SlackHttpCommandJobExecutor for DefaultSlackHttpCommandJobExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_slack_command_job_execute_json(request)
    }
}

pub struct SlackWorkerHttpHandler<E = DefaultSlackHttpCommandJobExecutor> {
    config: SlackWorkerConfig,
    signing_secret: String,
    command_executor: E,
    jobs: BoundedWorkerJobExecutor<WorkerHttpResponse>,
}

impl<E> SlackWorkerHttpHandler<E>
where
    E: SlackHttpCommandJobExecutor,
{
    pub fn new(
        config: &SlackWorkerConfig,
        command_executor: E,
        max_inflight_jobs: usize,
    ) -> Result<Self, WorkerDiagnostic> {
        let signing_secret = config
            .signing_secret
            .as_ref()
            .map(|secret| secret.expose().trim().to_string())
            .filter(|secret| !secret.is_empty())
            .ok_or_else(slack_signing_secret_missing)?;
        let jobs = BoundedWorkerJobExecutor::new(WorkerJobExecutorConfig {
            max_inflight: max_inflight_jobs,
        })?;
        Ok(Self {
            config: config.clone(),
            signing_secret,
            command_executor,
            jobs,
        })
    }

    fn transaction_request(
        &self,
        request: WorkerHttpRequest,
    ) -> Result<JsonValue, WorkerHttpResponse> {
        let raw_payload = String::from_utf8(request.body)
            .map_err(|_| slack_public_error(400, "Slack command payload must be UTF-8."))?;
        Ok(json!({
            "request_path": request.path,
            "command_path": self.config.command_path,
            "raw_payload": raw_payload,
            "signature": optional_string_json(request.headers.get("x-slack-signature")),
            "signature_timestamp": optional_string_json(
                request.headers.get("x-slack-request-timestamp")
            ),
            "signing_secret": self.signing_secret,
            "repo_name": self.config.shared.runtime_target.repo_name,
            "defer_replies": true,
            "ack_text": self.config.ack_text,
            "response_type": self.config.response_type,
        }))
    }
}

impl<E> WorkerHttpHandler for SlackWorkerHttpHandler<E>
where
    E: SlackHttpCommandJobExecutor,
{
    fn handle(
        &mut self,
        request: WorkerHttpRequest,
    ) -> Result<WorkerHttpDispatch, WorkerDiagnostic> {
        let request = match self.transaction_request(request) {
            Ok(request) => request,
            Err(response) => return Ok(WorkerHttpDispatch::Immediate(response)),
        };
        let transaction = agent_slack_command_http_transaction_plan_json(&request)
            .map_err(|_| slack_transaction_failure())?;
        let response = slack_transaction_response(&transaction)?;
        if transaction
            .get("should_submit_turn")
            .and_then(JsonValue::as_bool)
            != Some(true)
        {
            return Ok(WorkerHttpDispatch::Immediate(response));
        }

        let command_payload = transaction
            .get("http_ingress_plan")
            .and_then(|plan| plan.get("command_payload"))
            .filter(|payload| payload.is_object())
            .cloned()
            .ok_or_else(slack_transaction_contract_failure)?;
        let job_request = command_job_request(&self.config, command_payload);
        let executor = self.command_executor.clone();
        match self.jobs.submit(SLACK_COMMAND_JOB_KIND, move || {
            let outcome = executor
                .execute(&job_request)
                .map_err(|_| slack_command_job_failure())?;
            validate_command_job_contract(&outcome)?;
            Ok(WorkerHttpResponse::new(204, Vec::new()))
        }) {
            Ok(_) => Ok(WorkerHttpDispatch::Immediate(response)),
            Err(error)
                if matches!(
                    error.code,
                    "worker_job_capacity_exhausted" | "worker_job_executor_closed"
                ) =>
            {
                Ok(WorkerHttpDispatch::Immediate(slack_public_error(
                    503,
                    "Slack command worker is busy.",
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
                "slack_worker_jobs_still_inflight",
                "Rust Slack command jobs remain in flight during graceful shutdown.",
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

pub fn run_slack_transport(context: &WorkerRunContext) -> Result<(), WorkerDiagnostic> {
    let AgentWorkerRuntimeConfig::Slack(config) = &context.config else {
        return Err(WorkerDiagnostic::new(
            "slack_worker_config_mismatch",
            "The Rust Slack runner received a non-Slack worker configuration.",
            EXIT_INVALID_CONFIGURATION,
        ));
    };
    if config.app_token.is_some() {
        return run_slack_socket_mode_transport(context, config);
    }
    run_slack_http_transport(context, config)
}

fn run_slack_http_transport(
    context: &WorkerRunContext,
    config: &SlackWorkerConfig,
) -> Result<(), WorkerDiagnostic> {
    let bind_addr = resolve_slack_bind_addr(config)?;
    let handler = SlackWorkerHttpHandler::new(
        config,
        DefaultSlackHttpCommandJobExecutor,
        DEFAULT_SLACK_MAX_INFLIGHT_JOBS,
    )?;
    let mut runtime = WorkerHttpHostRuntime::new(
        WorkerHttpHostConfig {
            bind_addr,
            expected_method: "POST".to_string(),
            expected_path: config.command_path.clone(),
            enforce_expected_path: false,
            request_timeout: SLACK_HTTP_REQUEST_DEADLINE,
            ..WorkerHttpHostConfig::default()
        },
        handler,
    );
    run_worker_host(context, &mut runtime)
}

fn resolve_slack_bind_addr(config: &SlackWorkerConfig) -> Result<SocketAddr, WorkerDiagnostic> {
    let port = u16::try_from(config.bind_port)
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "slack_worker_bind_port_invalid",
                "The Rust Slack worker bind port must be between 1 and 65535.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_port", config.bind_port)
        })?;
    (config.bind_host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| {
            WorkerDiagnostic::new(
                "slack_worker_bind_address_invalid",
                format!(
                    "Cannot resolve the Rust Slack worker bind host `{}`: {error}",
                    config.bind_host
                ),
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_host", config.bind_host.clone())
        })?
        .next()
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "slack_worker_bind_address_invalid",
                "The Rust Slack worker bind host did not resolve to an address.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_host", config.bind_host.clone())
        })
}

fn slack_transaction_response(
    transaction: &JsonValue,
) -> Result<WorkerHttpResponse, WorkerDiagnostic> {
    let object = transaction
        .as_object()
        .ok_or_else(slack_transaction_contract_failure)?;
    let status_code = object
        .get("http_status")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| (100..=599).contains(value))
        .ok_or_else(slack_transaction_contract_failure)?;
    let write_json_response = object
        .get("write_json_response")
        .and_then(JsonValue::as_bool)
        .ok_or_else(slack_transaction_contract_failure)?;
    if !write_json_response {
        return Ok(WorkerHttpResponse::new(status_code, Vec::new()));
    }
    let response = object
        .get("response")
        .ok_or_else(slack_transaction_contract_failure)?;
    let body = JsonCodec::encode_value_to_vec_with_error_prefix(
        response,
        JsonEncodeOptions::compact(),
        "Failed to encode Rust Slack HTTP response",
    )
    .map_err(|_| slack_transaction_contract_failure())?;
    Ok(WorkerHttpResponse::new(status_code, body).with_header("Content-Type", JSON_CONTENT_TYPE))
}

fn slack_public_error(status_code: u16, message: &str) -> WorkerHttpResponse {
    let body = JsonCodec::encode_value_to_vec(
        &json!({"ok": false, "error": message}),
        JsonEncodeOptions::compact(),
    )
    .unwrap_or_else(|_| b"{\"ok\":false,\"error\":\"Slack command failed.\"}".to_vec());
    WorkerHttpResponse::new(status_code, body).with_header("Content-Type", JSON_CONTENT_TYPE)
}

fn optional_string_json(value: Option<&String>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.clone()))
        .unwrap_or(JsonValue::Null)
}

fn slack_signing_secret_missing() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_signing_secret_missing",
        "The selected Slack command worker requires a signing secret.",
        EXIT_INVALID_CONFIGURATION,
    )
}

fn slack_transaction_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_command_http_transaction_failed",
        "The Rust Slack command HTTP transaction failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_transaction_contract_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_command_http_transaction_contract_invalid",
        "The Rust Slack command HTTP transaction returned an invalid response contract.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_command_job_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_command_job_failed",
        "The Rust Slack command job failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, VecDeque};
    use std::net::SocketAddr;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use ait_agent_core::{
        resolve_agent_worker_config, AgentWorkerConfigInput, AgentWorkerRuntimeConfig,
    };
    use ait_core::json_support::{json, JsonCodec};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use tempfile::tempdir;

    use super::*;

    const SIGNING_SECRET: &str = "slack-product-runner-secret";
    const RAW_COMMAND: &str = "team_id=T1&channel_id=C1&channel_name=ops&user_id=U1&user_name=alice&command=%2Fait&text=hello+world&response_url=https%3A%2F%2Fhooks.slack.test%2Fsecret-response&trigger_id=trig-product";

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

    impl SlackHttpCommandJobExecutor for StubExecutor {
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

    fn slack_config(
        signing_secret: Option<&str>,
        environment: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> AgentWorkerRuntimeConfig {
        let temp = tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
        std::fs::write(
            temp.path().join(".ait/config.json"),
            r#"{"repo_name":"fixture","workflow_mode":"solo_remote","default_remote":"origin","remotes":{"origin":{"url":"http://127.0.0.1:8088"}}}"#,
        )
        .expect("repo config");
        let mut worker = json!({
            "kind": "slack",
            "name": "main",
        });
        if let Some(signing_secret) = signing_secret {
            worker["signing_secret"] = JsonValue::String(signing_secret.to_string());
        }
        let mut process_env = BTreeMap::from([
            ("AIT_SLACK_COMMAND_PATH".to_string(), "/command".to_string()),
            (
                "AIT_SLACK_ACK_TEXT".to_string(),
                "queued by rust".to_string(),
            ),
            (
                "AIT_SLACK_RESPONSE_TYPE".to_string(),
                "in_channel".to_string(),
            ),
        ]);
        process_env.extend(
            environment
                .into_iter()
                .map(|(key, value)| (key.to_string(), value.to_string())),
        );
        resolve_agent_worker_config(AgentWorkerConfigInput {
            repo_root: temp.path().to_path_buf(),
            worker_key: "slack/main".to_string(),
            worker,
            process_env,
        })
        .expect("Slack config")
    }

    fn handler<E: SlackHttpCommandJobExecutor>(
        executor: E,
        max_inflight: usize,
    ) -> SlackWorkerHttpHandler<E> {
        let config = slack_config(Some(SIGNING_SECRET), []);
        let AgentWorkerRuntimeConfig::Slack(config) = config else {
            panic!("Slack config");
        };
        SlackWorkerHttpHandler::new(&config, executor, max_inflight).expect("Slack handler")
    }

    fn signed_request(body: Vec<u8>) -> WorkerHttpRequest {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_secs()
            .to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(SIGNING_SECRET.as_bytes()).expect("HMAC");
        mac.update(format!("v0:{timestamp}:{}", String::from_utf8_lossy(&body)).as_bytes());
        let signature = format!(
            "v0={}",
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        WorkerHttpRequest {
            method: "POST".to_string(),
            path: "/command".to_string(),
            version: "HTTP/1.1".to_string(),
            headers: BTreeMap::from([
                ("x-slack-signature".to_string(), signature),
                ("x-slack-request-timestamp".to_string(), timestamp),
            ]),
            body,
            peer_addr: SocketAddr::from(([127, 0, 0, 1], 40000)),
        }
    }

    fn processed_job() -> JsonValue {
        json!({
            "contract": "ait_agent_core.event_loop.SlackCommandJob.v1",
            "migration_stage": "rust_agent_slack_command_job_transaction",
            "command_job_state": "processed",
            "ok": true,
            "processed": true,
            "duplicate": false,
            "conversation_key": "slack:C123:root",
            "binding_created": false,
            "turn_ok": true,
            "delivery_attempted": true,
            "delivered": true,
            "recorded": true,
            "sequence": 7,
            "error_kind": null,
        })
    }

    fn wait_completion<E: SlackHttpCommandJobExecutor>(
        handler: &mut SlackWorkerHttpHandler<E>,
    ) -> WorkerHttpCompletion {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(completion) = handler.poll_completed().into_iter().next() {
                return completion;
            }
            assert!(Instant::now() < deadline, "Slack job completion timed out");
            thread::yield_now();
        }
    }

    #[test]
    fn valid_command_is_acknowledged_before_bounded_job_completion() {
        let executor = StubExecutor::new(vec![Ok(processed_job())]);
        let calls = executor.calls.clone();
        let mut handler = handler(executor, 2);

        let dispatch = handler
            .handle(signed_request(RAW_COMMAND.as_bytes().to_vec()))
            .expect("dispatch");
        let WorkerHttpDispatch::Immediate(response) = dispatch else {
            panic!("immediate Slack acknowledgement");
        };

        assert_eq!(response.status_code, 200);
        assert_eq!(response.headers["Content-Type"], JSON_CONTENT_TYPE);
        assert_eq!(
            JsonCodec::parse_slice_with_error_prefix(&response.body, "Slack acknowledgement")
                .expect("Slack acknowledgement"),
            json!({"response_type": "ephemeral", "text": "queued by rust"})
        );
        assert_eq!(handler.inflight_work_count(), 1);
        let completion = wait_completion(&mut handler);
        assert_eq!(completion.result.expect("job completion").status_code, 204);
        assert_eq!(handler.inflight_work_count(), 0);
        let calls = calls.lock().expect("calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["command_payload"]["text"], "hello world");
        assert_eq!(calls[0]["runtime_target"]["mode"], "remote");
        assert_eq!(calls[0]["runtime_target"]["repo_name"], "fixture");
        assert!(calls[0]["state_path"].as_str().is_some());
        assert!(!calls[0].to_string().contains(SIGNING_SECRET));
    }

    #[test]
    fn request_shell_errors_never_start_background_jobs() {
        let executor = StubExecutor::new(Vec::new());
        let calls = executor.calls.clone();
        let mut handler = handler(executor, 1);

        let mut invalid_signature = signed_request(RAW_COMMAND.as_bytes().to_vec());
        invalid_signature
            .headers
            .insert("x-slack-signature".to_string(), "v0=invalid".to_string());
        let WorkerHttpDispatch::Immediate(invalid_signature) = handler
            .handle(invalid_signature)
            .expect("invalid signature response")
        else {
            panic!("immediate invalid signature response");
        };
        assert_eq!(invalid_signature.status_code, 401);
        assert!(String::from_utf8_lossy(&invalid_signature.body).contains("Invalid Slack"));

        let mut bad_path = signed_request(RAW_COMMAND.as_bytes().to_vec());
        bad_path.path = "/other".to_string();
        let WorkerHttpDispatch::Immediate(bad_path) =
            handler.handle(bad_path).expect("bad path response")
        else {
            panic!("immediate bad path response");
        };
        assert_eq!(bad_path.status_code, 404);
        assert!(bad_path.body.is_empty());

        let WorkerHttpDispatch::Immediate(invalid_utf8) = handler
            .handle(signed_request(vec![0xff]))
            .expect("invalid UTF-8 response")
        else {
            panic!("immediate invalid UTF-8 response");
        };
        assert_eq!(invalid_utf8.status_code, 400);
        assert!(calls.lock().expect("calls").is_empty());
    }

    #[test]
    fn bounded_capacity_returns_503_and_recovers_after_reap() {
        let (executor, gate) = StubExecutor::blocked(Ok(processed_job()));
        let mut handler = handler(executor, 1);

        assert!(matches!(
            handler
                .handle(signed_request(RAW_COMMAND.as_bytes().to_vec()))
                .expect("first command"),
            WorkerHttpDispatch::Immediate(response) if response.status_code == 200
        ));
        let deadline = Instant::now() + Duration::from_secs(3);
        while handler.inflight_work_count() != 1 {
            assert!(Instant::now() < deadline, "Slack job did not start");
            thread::yield_now();
        }
        let WorkerHttpDispatch::Immediate(busy) = handler
            .handle(signed_request(RAW_COMMAND.as_bytes().to_vec()))
            .expect("busy response")
        else {
            panic!("immediate capacity response");
        };
        assert_eq!(busy.status_code, 503);

        let (lock, ready) = &*gate;
        *lock.lock().expect("gate lock") = true;
        ready.notify_all();
        assert!(wait_completion(&mut handler).result.is_ok());
        assert_eq!(handler.inflight_work_count(), 0);
    }

    #[test]
    fn job_failures_and_shutdown_are_sanitized_and_bounded() {
        let mut handler = handler(
            StubExecutor::new(vec![Err(format!("{SIGNING_SECRET} backend failure"))]),
            1,
        );
        let _ = handler
            .handle(signed_request(RAW_COMMAND.as_bytes().to_vec()))
            .expect("dispatch");
        let error = wait_completion(&mut handler)
            .result
            .expect_err("job failure");
        assert_eq!(error.code, "slack_command_job_failed");
        assert!(!error.render_json().contains(SIGNING_SECRET));

        handler.close_admission();
        let WorkerHttpDispatch::Immediate(closed) = handler
            .handle(signed_request(RAW_COMMAND.as_bytes().to_vec()))
            .expect("closed response")
        else {
            panic!("immediate closed response");
        };
        assert_eq!(closed.status_code, 503);
        assert!(handler.finish_shutdown().is_ok());
        handler.force_shutdown().expect("forced shutdown");
        assert_eq!(handler.inflight_work_count(), 0);
    }

    #[test]
    fn typed_configuration_rejects_missing_secret_and_invalid_bind_address() {
        let missing = slack_config(None, []);
        let AgentWorkerRuntimeConfig::Slack(missing) = missing else {
            panic!("Slack config");
        };
        let error = SlackWorkerHttpHandler::new(&missing, StubExecutor::new(Vec::new()), 1)
            .err()
            .expect("missing signing secret");
        assert_eq!(error.code, "slack_signing_secret_missing");

        let invalid_port = slack_config(Some(SIGNING_SECRET), [("AIT_SLACK_BIND_PORT", "70000")]);
        let AgentWorkerRuntimeConfig::Slack(invalid_port) = invalid_port else {
            panic!("Slack config");
        };
        let error = resolve_slack_bind_addr(&invalid_port).expect_err("invalid port");
        assert_eq!(error.code, "slack_worker_bind_port_invalid");
        assert!(!error.render_json().contains(SIGNING_SECRET));
    }
}
