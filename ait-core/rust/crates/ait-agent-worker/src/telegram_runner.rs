use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ait_agent_core::{
    agent_telegram_api_execute, execute_with_telegram_service_run_ports,
    execute_with_telegram_webhook_transaction_ports, AgentEvent, AgentWorkerRuntimeConfig,
    DefaultTelegramServiceCycleStatePort, DefaultTelegramWebhookTransactionIngressPort,
    NativeTelegramBackgroundSyncServicePort, NativeTelegramUpdateBootstrapPort,
    NativeTelegramUpdateCommandPort, NativeTelegramUpdateInputPort,
    NativeTelegramUpdateLifecyclePort, NativeTelegramUpdateMessagePort,
    NativeTelegramUpdateOperationalPort, RuntimeBindingTelegramBackgroundSyncReadPort,
    SystemTelegramUpdateDiagnosticsPort, TelegramLogicalTurnRuntime,
    TelegramServiceCycleBackgroundSyncPort, TelegramServiceCycleDispatchPort,
    TelegramServiceCyclePollPort, TelegramServiceCycleStatePort, TelegramServiceRunClockPort,
    TelegramServiceRunCycleExecutor, TelegramServiceRunSleepPort, TelegramServiceRunStopPort,
    TelegramSttMode, TelegramSubmissionDispatchPort, TelegramSubmissionExecutionPort,
    TelegramSubmissionRuntime, TelegramUpdateJob, TelegramUpdateJobConfig, TelegramUpdateJobPorts,
    TelegramWebhookTransactionDispatchPort, TelegramWorkerConfig, TelegramWorkerMode,
};
use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};

use crate::{
    run_worker_host, BoundedWorkerJobExecutor, WorkerDiagnostic, WorkerHostEventLoop,
    WorkerHostRuntime, WorkerHttpCompletion, WorkerHttpDispatch, WorkerHttpHandler,
    WorkerHttpHostConfig, WorkerHttpHostRuntime, WorkerHttpRequest, WorkerHttpResponse,
    WorkerJobExecutorConfig, WorkerRunContext, EXIT_INVALID_CONFIGURATION, EXIT_INVALID_REQUEST,
    EXIT_RUNTIME_UNAVAILABLE,
};

const TELEGRAM_API_EXECUTION_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramApiTransportExecution.v1";
const TELEGRAM_API_EXECUTION_STAGE: &str = "rust_agent_telegram_transport_execution";
const TELEGRAM_SERVICE_RUN_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramServiceRunExecution.v1";
const TELEGRAM_SERVICE_RUN_STAGE: &str = "rust_agent_telegram_service_run_execution";
const TELEGRAM_WEBHOOK_TRANSACTION_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramWebhookTransaction.v1";
const TELEGRAM_WEBHOOK_TRANSACTION_STAGE: &str = "rust_agent_telegram_webhook_dispatch_transaction";
pub const TELEGRAM_WEBHOOK_ONCE_CONTRACT: &str = "ait.agent.worker.telegram_webhook_once.v1";
const TELEGRAM_POLLING_JOB_KIND: &str = "telegram.polling_service";
const TELEGRAM_WEBHOOK_JOB_KIND: &str = "telegram.webhook_transaction";
const JSON_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const TELEGRAM_WEBHOOK_SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";
const DEFAULT_TELEGRAM_WEBHOOK_MAX_INFLIGHT_JOBS: usize = 4;
const TELEGRAM_WEBHOOK_REQUEST_DEADLINE: Duration = Duration::from_secs(120);
const TELEGRAM_PRODUCT_DRAIN_TIMEOUT: Duration = Duration::from_secs(120);
const TELEGRAM_LOGICAL_TURN_POLL_INTERVAL: Duration = Duration::from_millis(10);
const TELEGRAM_MAX_DISPATCH_WORKERS: usize = 64;
const TELEGRAM_PER_KEY_QUEUE_CAPACITY: usize = 1_024;
const TELEGRAM_MAX_PENDING_CHATS: usize = 4_096;
const TELEGRAM_MAX_PENDING_PER_CHAT: usize = 1_024;

pub trait TelegramPollingApiExecutor: Send + Sync + 'static {
    fn execute_get_updates(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramPollingApiExecutor;

impl TelegramPollingApiExecutor for DefaultTelegramPollingApiExecutor {
    fn execute_get_updates(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_api_execute(request).map(|execution| execution.metadata().clone())
    }
}

pub struct TelegramPollingApiPort<E = DefaultTelegramPollingApiExecutor> {
    bot_token: String,
    request_timeout_seconds: Option<f64>,
    executor: E,
}

impl TelegramPollingApiPort<DefaultTelegramPollingApiExecutor> {
    pub fn from_config(config: &TelegramWorkerConfig) -> Self {
        Self::new(config, DefaultTelegramPollingApiExecutor)
    }
}

impl<E> TelegramPollingApiPort<E> {
    pub fn new(config: &TelegramWorkerConfig, executor: E) -> Self {
        Self {
            bot_token: config.token.expose().to_string(),
            request_timeout_seconds: config.shared.request_timeout_seconds,
            executor,
        }
    }
}

impl<E> fmt::Debug for TelegramPollingApiPort<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramPollingApiPort")
            .field("bot_token", &"<redacted>")
            .field("request_timeout_seconds", &self.request_timeout_seconds)
            .finish_non_exhaustive()
    }
}

impl<E> TelegramServiceCyclePollPort for TelegramPollingApiPort<E>
where
    E: TelegramPollingApiExecutor,
{
    fn poll_updates(&self, request: &JsonValue) -> Result<Vec<JsonValue>, String> {
        let poll_request = request
            .get("poll_request")
            .and_then(JsonValue::as_object)
            .ok_or_else(telegram_polling_api_error)?;
        let offset =
            nonnegative_i64(poll_request.get("offset")).ok_or_else(telegram_polling_api_error)?;
        let timeout_seconds = nonnegative_i64(poll_request.get("timeout_seconds"))
            .ok_or_else(telegram_polling_api_error)?;
        let execution = self
            .executor
            .execute_get_updates(&json!({
                "operation": "get_updates",
                "bot_token": self.bot_token,
                "offset": offset,
                "timeout_seconds": timeout_seconds,
                "request_timeout_seconds": self.request_timeout_seconds,
            }))
            .map_err(|_| telegram_polling_api_error())?;
        validated_poll_updates(&execution).ok_or_else(telegram_polling_api_error)
    }
}

pub trait TelegramPollingServiceExecutor: Clone + Send + Sync + 'static {
    fn execute_service_run(&self, stop: Arc<AtomicBool>) -> Result<JsonValue, String>;
}

pub struct TelegramPollingServiceJob<S, P, D, B> {
    state: Arc<S>,
    poll: Arc<P>,
    dispatch: Arc<D>,
    background_sync: Arc<B>,
    request: JsonValue,
}

impl<S, P, D, B> TelegramPollingServiceJob<S, P, D, B> {
    pub fn new(
        config: &TelegramWorkerConfig,
        state: Arc<S>,
        poll: Arc<P>,
        dispatch: Arc<D>,
        background_sync: Arc<B>,
    ) -> Self {
        Self {
            state,
            poll,
            dispatch,
            background_sync,
            request: json!({
                "state_path": config.shared.paths.sync_state_path,
                "poll_timeout_seconds": config.poll_timeout_seconds,
                "background_sync_enabled": config.background_sync_enabled,
                "background_sync_interval_seconds": config.background_sync_interval_seconds,
                "retry_backoff_seconds": 1.0,
            }),
        }
    }
}

impl<S, P, D, B> Clone for TelegramPollingServiceJob<S, P, D, B> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            poll: Arc::clone(&self.poll),
            dispatch: Arc::clone(&self.dispatch),
            background_sync: Arc::clone(&self.background_sync),
            request: self.request.clone(),
        }
    }
}

impl<S, P, D, B> TelegramPollingServiceExecutor for TelegramPollingServiceJob<S, P, D, B>
where
    S: TelegramServiceCycleStatePort + Send + Sync + 'static,
    P: TelegramServiceCyclePollPort + Send + Sync + 'static,
    D: TelegramServiceCycleDispatchPort + Send + Sync + 'static,
    B: TelegramServiceCycleBackgroundSyncPort + Send + Sync + 'static,
{
    fn execute_service_run(&self, stop: Arc<AtomicBool>) -> Result<JsonValue, String> {
        let cycle = TelegramServiceRunCycleExecutor::new(
            self.state.as_ref(),
            self.poll.as_ref(),
            self.dispatch.as_ref(),
            self.background_sync.as_ref(),
        );
        let control = TelegramPollingJobControl::new(stop);
        execute_with_telegram_service_run_ports(&cycle, &control, &control, &control, &self.request)
    }
}

pub struct TelegramPollingWorkerRuntime<E> {
    executor: E,
    stop: Arc<AtomicBool>,
    jobs: BoundedWorkerJobExecutor<JsonValue>,
    started: bool,
    completed: bool,
}

impl<E> TelegramPollingWorkerRuntime<E>
where
    E: TelegramPollingServiceExecutor,
{
    pub fn new(executor: E) -> Result<Self, WorkerDiagnostic> {
        Ok(Self {
            executor,
            stop: Arc::new(AtomicBool::new(false)),
            jobs: BoundedWorkerJobExecutor::new(WorkerJobExecutorConfig { max_inflight: 1 })?,
            started: false,
            completed: false,
        })
    }

    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub fn completed(&self) -> bool {
        self.completed
    }

    fn poll_service_completion(&mut self) -> Result<(), WorkerDiagnostic> {
        let completions = self.jobs.poll_completed();
        if completions.len() > 1 {
            return Err(telegram_polling_service_contract_failure());
        }
        let Some(completion) = completions.into_iter().next() else {
            return Ok(());
        };
        self.completed = true;
        let outcome = completion.result?;
        validate_service_run_result(&outcome, self.stop_requested())
    }
}

impl<E> WorkerHostRuntime for TelegramPollingWorkerRuntime<E>
where
    E: TelegramPollingServiceExecutor,
{
    fn start(
        &mut self,
        context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        if !matches!(context.config, AgentWorkerRuntimeConfig::Telegram(_)) {
            return Err(WorkerDiagnostic::new(
                "telegram_polling_worker_config_mismatch",
                "The Rust Telegram polling host received a non-Telegram worker configuration.",
                EXIT_INVALID_CONFIGURATION,
            ));
        }
        if self.started {
            return Err(WorkerDiagnostic::new(
                "telegram_polling_worker_already_started",
                "The Rust Telegram polling host cannot be started more than once.",
                EXIT_RUNTIME_UNAVAILABLE,
            ));
        }
        self.stop.store(false, Ordering::Release);
        self.completed = false;
        let executor = self.executor.clone();
        let stop = Arc::clone(&self.stop);
        self.jobs.submit(TELEGRAM_POLLING_JOB_KIND, move || {
            executor
                .execute_service_run(stop)
                .map_err(|_| telegram_polling_service_execution_failure())
        })?;
        self.started = true;
        Ok(())
    }

    fn tick(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
        _events: &[AgentEvent],
    ) -> Result<(), WorkerDiagnostic> {
        self.poll_service_completion()
    }

    fn request_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
        _signal: i32,
    ) -> Result<(), WorkerDiagnostic> {
        self.stop.store(true, Ordering::Release);
        self.jobs.close_admission();
        Ok(())
    }

    fn inflight_work_count(&self) -> usize {
        self.jobs.inflight_count()
    }

    fn finish_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        self.poll_service_completion()?;
        if self.jobs.inflight_count() == 0 && self.completed && self.stop_requested() {
            Ok(())
        } else {
            Err(WorkerDiagnostic::new(
                "telegram_polling_worker_shutdown_incomplete",
                "The Rust Telegram polling host did not complete graceful shutdown.",
                EXIT_RUNTIME_UNAVAILABLE,
            ))
        }
    }

    fn force_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        self.stop.store(true, Ordering::Release);
        self.jobs.close_admission();
        self.jobs.force_detach();
        Ok(())
    }
}

pub trait TelegramWebhookJobExecutor: Clone + Send + Sync + 'static {
    fn execute_webhook(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub struct NativeTelegramWebhookJobExecutor<D> {
    dispatch: Arc<D>,
}

impl<D> NativeTelegramWebhookJobExecutor<D> {
    pub fn new(dispatch: Arc<D>) -> Self {
        Self { dispatch }
    }
}

impl<D> Clone for NativeTelegramWebhookJobExecutor<D> {
    fn clone(&self) -> Self {
        Self {
            dispatch: Arc::clone(&self.dispatch),
        }
    }
}

impl<D> TelegramWebhookJobExecutor for NativeTelegramWebhookJobExecutor<D>
where
    D: TelegramWebhookTransactionDispatchPort + Send + Sync + 'static,
{
    fn execute_webhook(&self, request: &JsonValue) -> Result<JsonValue, String> {
        execute_with_telegram_webhook_transaction_ports(
            &DefaultTelegramWebhookTransactionIngressPort,
            self.dispatch.as_ref(),
            request,
        )
    }
}

pub struct TelegramWorkerHttpHandler<E> {
    executor: E,
    webhook_secret: Option<String>,
    jobs: BoundedWorkerJobExecutor<WorkerHttpResponse>,
}

impl<E> TelegramWorkerHttpHandler<E>
where
    E: TelegramWebhookJobExecutor,
{
    pub fn new(executor: E, max_inflight_jobs: usize) -> Result<Self, WorkerDiagnostic> {
        Self::with_webhook_secret(executor, max_inflight_jobs, None)
    }

    pub fn from_config(
        config: &TelegramWorkerConfig,
        executor: E,
        max_inflight_jobs: usize,
    ) -> Result<Self, WorkerDiagnostic> {
        Self::with_webhook_secret(
            executor,
            max_inflight_jobs,
            config
                .webhook_secret
                .as_ref()
                .map(|secret| secret.expose().to_string()),
        )
    }

    fn with_webhook_secret(
        executor: E,
        max_inflight_jobs: usize,
        webhook_secret: Option<String>,
    ) -> Result<Self, WorkerDiagnostic> {
        Ok(Self {
            executor,
            webhook_secret,
            jobs: BoundedWorkerJobExecutor::new(WorkerJobExecutorConfig {
                max_inflight: max_inflight_jobs,
            })?,
        })
    }

    fn transaction_request(
        &self,
        request: WorkerHttpRequest,
    ) -> Result<JsonValue, WorkerHttpResponse> {
        if let Some(expected) = self.webhook_secret.as_deref() {
            let valid = request
                .headers
                .get(TELEGRAM_WEBHOOK_SECRET_HEADER)
                .is_some_and(|actual| constant_time_eq(expected.as_bytes(), actual.as_bytes()));
            if !valid {
                return Err(telegram_webhook_public_error(
                    401,
                    "Telegram webhook authentication failed.",
                ));
            }
        }
        let raw_payload = String::from_utf8(request.body).map_err(|_| {
            telegram_webhook_public_error(400, "Telegram webhook payload must be UTF-8.")
        })?;
        Ok(json!({"raw_payload": raw_payload}))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelegramWorkerHostPlan {
    Poll,
    Webhook { bind_addr: SocketAddr },
}

type TelegramProductBackgroundSyncPort = NativeTelegramBackgroundSyncServicePort<
    RuntimeBindingTelegramBackgroundSyncReadPort,
    TelegramSubmissionRuntime,
>;

struct NativeTelegramProductComposition {
    runtime: Arc<TelegramSubmissionRuntime>,
    dispatch: Arc<TelegramSubmissionDispatchPort>,
    background_sync: Arc<TelegramProductBackgroundSyncPort>,
}

impl NativeTelegramProductComposition {
    fn from_context(context: &WorkerRunContext) -> Result<Self, WorkerDiagnostic> {
        let AgentWorkerRuntimeConfig::Telegram(config) = &context.config else {
            return Err(telegram_product_configuration_failure(
                "telegram_product_config_mismatch",
                "The native Telegram product composition received a non-Telegram configuration.",
            ));
        };
        let inflight_limit = validated_product_admission(context)?;
        let merge_window = validated_merge_window(config.turn_merge_window_seconds)?;
        let max_messages = usize::try_from(config.turn_merge_max_messages)
            .ok()
            .filter(|value| (1..=TELEGRAM_MAX_PENDING_PER_CHAT).contains(value))
            .ok_or_else(|| {
                telegram_product_configuration_failure(
                    "telegram_logical_turn_config_invalid",
                    "The Telegram logical-turn buffer configuration is invalid.",
                )
            })?;

        let token = config.token.expose().to_string();
        let state_path = config.shared.paths.sync_state_path.clone();
        let runtime_target = config.shared.runtime_target.clone();
        let request_timeout = config.shared.request_timeout_seconds;
        let ait_web_url = config.shared.ait_web_url.clone();

        let ports = TelegramUpdateJobPorts::new(
            Arc::new(
                NativeTelegramUpdateInputPort::from_config(config).map_err(|_| {
                    telegram_product_configuration_failure(
                        "telegram_input_config_invalid",
                        "The native Telegram input configuration is invalid.",
                    )
                })?,
            ),
            Arc::new(
                NativeTelegramUpdateBootstrapPort::new(
                    runtime_target.repo_name.clone(),
                    state_path.clone(),
                    config.owner_bootstrap_enabled,
                    token.clone(),
                    request_timeout,
                    config.reply_markdown_enabled,
                )
                .map_err(|_| {
                    telegram_product_configuration_failure(
                        "telegram_bootstrap_config_invalid",
                        "The native Telegram owner-bootstrap configuration is invalid.",
                    )
                })?,
            ),
            Arc::new(
                NativeTelegramUpdateOperationalPort::new(
                    runtime_target.repo_name.clone(),
                    runtime_target.repo_root.clone(),
                    state_path.clone(),
                    token.clone(),
                    request_timeout,
                    config.reply_markdown_enabled,
                )
                .map_err(|_| {
                    telegram_product_configuration_failure(
                        "telegram_operational_config_invalid",
                        "The native Telegram operational-trigger configuration is invalid.",
                    )
                })?,
            ),
            Arc::new(
                NativeTelegramUpdateCommandPort::new(
                    state_path.clone(),
                    runtime_target.clone(),
                    request_timeout,
                    token.clone(),
                    ait_web_url.clone(),
                    config.username.clone(),
                    config.background_sync_enabled,
                    config.background_sync_interval_seconds,
                    config.reply_markdown_enabled,
                )
                .map_err(|_| {
                    telegram_product_configuration_failure(
                        "telegram_command_config_invalid",
                        "The native Telegram command configuration is invalid.",
                    )
                })?,
            ),
            Arc::new(
                NativeTelegramUpdateMessagePort::new(
                    token.clone(),
                    request_timeout,
                    config.reply_markdown_enabled,
                )
                .map_err(|_| {
                    telegram_product_configuration_failure(
                        "telegram_delivery_config_invalid",
                        "The native Telegram message-delivery configuration is invalid.",
                    )
                })?,
            ),
            Arc::new(
                NativeTelegramUpdateLifecyclePort::new(
                    state_path.clone(),
                    runtime_target,
                    request_timeout,
                    config.shared.local_reply.clone(),
                    token,
                    ait_web_url,
                    config.reply_markdown_enabled,
                )
                .map_err(|_| {
                    telegram_product_configuration_failure(
                        "telegram_lifecycle_config_invalid",
                        "The native Telegram reply-lifecycle configuration is invalid.",
                    )
                })?,
            ),
            Arc::new(SystemTelegramUpdateDiagnosticsPort),
        );
        let update_job = TelegramUpdateJob::new(
            TelegramUpdateJobConfig::new(
                config.username.clone(),
                config.stt_mode == TelegramSttMode::LocalStt,
                config.stt_include_audio_uploads,
                config.decoupled_reply_enabled,
            ),
            ports,
        )
        .map_err(|_| {
            telegram_product_configuration_failure(
                "telegram_update_job_config_invalid",
                "The native Telegram update-job configuration is invalid.",
            )
        })?;
        let logical_turn = Arc::new(
            TelegramLogicalTurnRuntime::new(
                config.username.clone(),
                merge_window,
                max_messages,
                TELEGRAM_LOGICAL_TURN_POLL_INTERVAL,
                inflight_limit.clamp(1, TELEGRAM_MAX_PENDING_CHATS),
                max_messages.min(TELEGRAM_MAX_PENDING_PER_CHAT),
            )
            .map_err(|_| {
                telegram_product_configuration_failure(
                    "telegram_logical_turn_config_invalid",
                    "The Telegram logical-turn buffer configuration is invalid.",
                )
            })?,
        );
        let execution: Arc<dyn TelegramSubmissionExecutionPort> = Arc::new(update_job);
        let runtime = Arc::new(
            TelegramSubmissionRuntime::new(
                execution,
                logical_turn,
                &context.runtime_admission_plan,
                inflight_limit.clamp(1, TELEGRAM_MAX_DISPATCH_WORKERS),
                TELEGRAM_PER_KEY_QUEUE_CAPACITY,
            )
            .map_err(|_| {
                telegram_product_configuration_failure(
                    "telegram_submission_runtime_invalid",
                    "The native Telegram submission runtime could not be configured.",
                )
            })?,
        );
        let dispatch = Arc::new(TelegramSubmissionDispatchPort::new(Arc::clone(&runtime)));
        let background_sync = Arc::new(
            NativeTelegramBackgroundSyncServicePort::new(state_path, Arc::clone(&runtime))
                .map_err(|_| {
                    telegram_product_configuration_failure(
                        "telegram_background_sync_config_invalid",
                        "The native Telegram background-sync configuration is invalid.",
                    )
                })?,
        );
        Ok(Self {
            runtime,
            dispatch,
            background_sync,
        })
    }

    fn shutdown(&self) -> Result<(), WorkerDiagnostic> {
        self.dispatch.request_stop().map_err(|_| {
            telegram_product_runtime_failure(
                "telegram_submission_stop_failed",
                "The native Telegram submission runtime could not stop.",
            )
        })?;
        let idle = self
            .dispatch
            .wait_for_idle(Some(TELEGRAM_PRODUCT_DRAIN_TIMEOUT))
            .map_err(|_| {
                telegram_product_runtime_failure(
                    "telegram_submission_drain_failed",
                    "The native Telegram submission runtime failed while draining.",
                )
            })?;
        if !idle {
            return Err(telegram_product_runtime_failure(
                "telegram_submission_drain_timeout",
                "The native Telegram submission runtime did not drain before its deadline.",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for NativeTelegramProductComposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let runtime = self.runtime.snapshot_json();
        formatter
            .debug_struct("NativeTelegramProductComposition")
            .field("native_update_job", &true)
            .field("native_submission_runtime", &true)
            .field("native_background_sync", &true)
            .field("stopped", &runtime["stopped"])
            .field("runtime_state_exposed", &false)
            .field("configuration_exposed", &false)
            .finish()
    }
}

pub fn run_telegram_transport(context: &WorkerRunContext) -> Result<(), WorkerDiagnostic> {
    let composition = NativeTelegramProductComposition::from_context(context)?;
    let run_result = run_telegram_transport_with_ports(
        context,
        Arc::clone(&composition.dispatch),
        Arc::clone(&composition.background_sync),
    );
    let shutdown_result = composition.shutdown();
    match run_result {
        Err(error) => Err(error),
        Ok(()) => shutdown_result,
    }
}

pub fn execute_telegram_webhook_once(
    context: &WorkerRunContext,
    raw_payload: &str,
) -> Result<JsonValue, WorkerDiagnostic> {
    if context.transport != ait_agent_core::TransportKind::Telegram {
        return Err(WorkerDiagnostic::new(
            "telegram_webhook_worker_mismatch",
            "The stdin webhook mode requires a Telegram worker.",
            EXIT_INVALID_REQUEST,
        ));
    }
    let composition = NativeTelegramProductComposition::from_context(context)?;
    let executor = NativeTelegramWebhookJobExecutor::new(Arc::clone(&composition.dispatch));
    let execution_result =
        execute_telegram_webhook_once_with_executor(context, raw_payload, &executor);
    let shutdown_result = composition.shutdown();
    match execution_result {
        Err(error) => Err(error),
        Ok(result) => shutdown_result.map(|()| result),
    }
}

fn execute_telegram_webhook_once_with_executor<E>(
    context: &WorkerRunContext,
    raw_payload: &str,
    executor: &E,
) -> Result<JsonValue, WorkerDiagnostic>
where
    E: TelegramWebhookJobExecutor,
{
    let outcome = executor
        .execute_webhook(&json!({"raw_payload": raw_payload}))
        .map_err(|_| telegram_webhook_execution_failure())?;
    let response = telegram_webhook_response(&outcome)?;
    let ok = outcome.get("ok").and_then(JsonValue::as_bool) == Some(true);
    if !ok {
        let (code, message, exit_code) = if response.status_code < 500 {
            (
                "telegram_webhook_input_invalid",
                "The Telegram webhook payload from stdin is invalid.",
                EXIT_INVALID_REQUEST,
            )
        } else {
            (
                "telegram_webhook_transaction_failed",
                "The Rust Telegram webhook transaction failed.",
                EXIT_RUNTIME_UNAVAILABLE,
            )
        };
        return Err(WorkerDiagnostic::new(code, message, exit_code)
            .with_detail("http_status", response.status_code));
    }
    Ok(json!({
        "contract": TELEGRAM_WEBHOOK_ONCE_CONTRACT,
        "binary": "ait-agent-worker",
        "transport": "telegram",
        "worker": context.worker_name,
        "ok": true,
        "http_status": response.status_code,
        "response": outcome["response"].clone(),
        "python_worker_execution_allowed": false,
        "python_ingress_allowed": false,
        "python_update_dispatch_allowed": false,
        "python_http_response_allowed": false,
    }))
}

fn validated_merge_window(seconds: f64) -> Result<Duration, WorkerDiagnostic> {
    if !seconds.is_finite() || !(0.0..=300.0).contains(&seconds) {
        return Err(telegram_product_configuration_failure(
            "telegram_logical_turn_config_invalid",
            "The Telegram logical-turn buffer configuration is invalid.",
        ));
    }
    Ok(Duration::from_secs_f64(seconds))
}

fn validated_product_admission(context: &WorkerRunContext) -> Result<usize, WorkerDiagnostic> {
    let object = context
        .runtime_admission_plan
        .as_object()
        .ok_or_else(telegram_runtime_admission_invalid)?;
    if object.get("migration_stage").and_then(JsonValue::as_str)
        != Some("rust_agent_high_concurrency_runtime_admission")
        || object.get("admission_contract").and_then(JsonValue::as_str)
            != Some("ait_agent_core.event_loop.AgentRuntimeAdmission.v1")
        || object.get("backend").and_then(JsonValue::as_str)
            != Some(context.event_loop_backend.label())
        || object.get("transport_runtime").and_then(JsonValue::as_str) != Some("rust")
        || object.get("admission_state").and_then(JsonValue::as_str) != Some("admitted")
        || object.get("launch_allowed").and_then(JsonValue::as_bool) != Some(true)
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_worker_execution_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object
            .get("python_fallback_requested")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(telegram_runtime_admission_invalid());
    }
    let lease = object
        .get("worker_leases")
        .and_then(JsonValue::as_array)
        .and_then(|leases| {
            leases.iter().find(|lease| {
                lease.get("worker_key").and_then(JsonValue::as_str)
                    == Some(context.worker_key.as_str())
            })
        })
        .and_then(JsonValue::as_object)
        .ok_or_else(telegram_runtime_admission_invalid)?;
    if lease.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || lease.get("backend").and_then(JsonValue::as_str)
            != Some(context.event_loop_backend.label())
        || lease.get("shard_index").and_then(JsonValue::as_u64)
            != u64::try_from(context.shard_index).ok()
        || lease
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || lease
            .get("python_fallback_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(telegram_runtime_admission_invalid());
    }
    object
        .get("shard_admissions")
        .and_then(JsonValue::as_array)
        .and_then(|shards| {
            shards.iter().find(|shard| {
                shard.get("shard_index").and_then(JsonValue::as_u64)
                    == u64::try_from(context.shard_index).ok()
            })
        })
        .and_then(|shard| shard.get("inflight_limit"))
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| (1..=65_536).contains(value))
        .ok_or_else(telegram_runtime_admission_invalid)
}

fn telegram_runtime_admission_invalid() -> WorkerDiagnostic {
    telegram_product_configuration_failure(
        "telegram_runtime_admission_invalid",
        "The native Telegram product runner received an invalid runtime-admission plan.",
    )
}

fn telegram_product_configuration_failure(
    code: &'static str,
    message: &'static str,
) -> WorkerDiagnostic {
    WorkerDiagnostic::new(code, message, EXIT_INVALID_CONFIGURATION)
}

fn telegram_product_runtime_failure(code: &'static str, message: &'static str) -> WorkerDiagnostic {
    WorkerDiagnostic::new(code, message, EXIT_RUNTIME_UNAVAILABLE)
}

pub fn run_telegram_transport_with_ports<D, B>(
    context: &WorkerRunContext,
    dispatch: Arc<D>,
    background_sync: Arc<B>,
) -> Result<(), WorkerDiagnostic>
where
    D: TelegramServiceCycleDispatchPort
        + TelegramWebhookTransactionDispatchPort
        + Send
        + Sync
        + 'static,
    B: TelegramServiceCycleBackgroundSyncPort + Send + Sync + 'static,
{
    let AgentWorkerRuntimeConfig::Telegram(config) = &context.config else {
        return Err(WorkerDiagnostic::new(
            "telegram_worker_config_mismatch",
            "The Rust Telegram runner received a non-Telegram worker configuration.",
            EXIT_INVALID_CONFIGURATION,
        ));
    };
    match plan_telegram_worker_host(config)? {
        TelegramWorkerHostPlan::Poll => {
            let job = TelegramPollingServiceJob::new(
                config,
                Arc::new(DefaultTelegramServiceCycleStatePort),
                Arc::new(TelegramPollingApiPort::from_config(config)),
                dispatch,
                background_sync,
            );
            let mut runtime = TelegramPollingWorkerRuntime::new(job)?;
            run_worker_host(context, &mut runtime)
        }
        TelegramWorkerHostPlan::Webhook { bind_addr } => {
            let executor = NativeTelegramWebhookJobExecutor::new(dispatch);
            let handler = TelegramWorkerHttpHandler::from_config(
                config,
                executor,
                DEFAULT_TELEGRAM_WEBHOOK_MAX_INFLIGHT_JOBS,
            )?;
            let mut runtime = WorkerHttpHostRuntime::new(
                telegram_webhook_host_config(config, bind_addr),
                handler,
            );
            run_worker_host(context, &mut runtime)
        }
    }
}

fn telegram_webhook_host_config(
    config: &TelegramWorkerConfig,
    bind_addr: SocketAddr,
) -> WorkerHttpHostConfig {
    WorkerHttpHostConfig {
        bind_addr,
        expected_method: "POST".to_string(),
        expected_path: config.webhook_path.clone(),
        enforce_expected_path: true,
        request_timeout: TELEGRAM_WEBHOOK_REQUEST_DEADLINE,
        ..WorkerHttpHostConfig::default()
    }
}

fn plan_telegram_worker_host(
    config: &TelegramWorkerConfig,
) -> Result<TelegramWorkerHostPlan, WorkerDiagnostic> {
    match config.service_mode {
        TelegramWorkerMode::Poll => Ok(TelegramWorkerHostPlan::Poll),
        TelegramWorkerMode::Webhook => Ok(TelegramWorkerHostPlan::Webhook {
            bind_addr: resolve_telegram_bind_addr(config)?,
        }),
    }
}

fn resolve_telegram_bind_addr(
    config: &TelegramWorkerConfig,
) -> Result<SocketAddr, WorkerDiagnostic> {
    let port = u16::try_from(config.bind_port)
        .ok()
        .filter(|port| *port > 0)
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "telegram_worker_bind_port_invalid",
                "The Rust Telegram worker bind port must be between 1 and 65535.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_port", config.bind_port)
        })?;
    (config.bind_host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| {
            WorkerDiagnostic::new(
                "telegram_worker_bind_address_invalid",
                format!(
                    "Cannot resolve the Rust Telegram worker bind host `{}`: {error}",
                    config.bind_host
                ),
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_host", config.bind_host.clone())
        })?
        .next()
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "telegram_worker_bind_address_invalid",
                "The Rust Telegram worker bind host did not resolve to an address.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("bind_host", config.bind_host.clone())
        })
}

impl<E> WorkerHttpHandler for TelegramWorkerHttpHandler<E>
where
    E: TelegramWebhookJobExecutor,
{
    fn handle(
        &mut self,
        request: WorkerHttpRequest,
    ) -> Result<WorkerHttpDispatch, WorkerDiagnostic> {
        let request = match self.transaction_request(request) {
            Ok(request) => request,
            Err(response) => return Ok(WorkerHttpDispatch::Immediate(response)),
        };
        let executor = self.executor.clone();
        match self.jobs.submit(TELEGRAM_WEBHOOK_JOB_KIND, move || {
            let outcome = executor
                .execute_webhook(&request)
                .map_err(|_| telegram_webhook_execution_failure())?;
            telegram_webhook_response(&outcome)
        }) {
            Ok(job_id) => Ok(WorkerHttpDispatch::Deferred { job_id }),
            Err(error)
                if matches!(
                    error.code,
                    "worker_job_capacity_exhausted" | "worker_job_executor_closed"
                ) =>
            {
                Ok(WorkerHttpDispatch::Immediate(
                    telegram_webhook_public_error(503, "Telegram webhook worker is busy."),
                ))
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
                "telegram_webhook_jobs_still_inflight",
                "Rust Telegram webhook jobs remain in flight during graceful shutdown.",
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

struct TelegramPollingJobControl {
    started_at: Instant,
    stop: Arc<AtomicBool>,
}

impl TelegramPollingJobControl {
    fn new(stop: Arc<AtomicBool>) -> Self {
        Self {
            started_at: Instant::now(),
            stop,
        }
    }
}

impl TelegramServiceRunClockPort for TelegramPollingJobControl {
    fn monotonic_seconds(&self) -> Result<f64, String> {
        Ok(self.started_at.elapsed().as_secs_f64())
    }
}

impl TelegramServiceRunStopPort for TelegramPollingJobControl {
    fn stop_requested(&self) -> Result<bool, String> {
        Ok(self.stop.load(Ordering::Acquire))
    }
}

impl TelegramServiceRunSleepPort for TelegramPollingJobControl {
    fn sleep_seconds(&self, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() || !(0.0..=60.0).contains(&seconds) {
            return Err("Telegram polling retry sleep is invalid.".to_string());
        }
        thread::sleep(Duration::from_secs_f64(seconds));
        Ok(())
    }
}

fn validated_poll_updates(execution: &JsonValue) -> Option<Vec<JsonValue>> {
    let object = execution.as_object()?;
    if object.get("contract").and_then(JsonValue::as_str) != Some(TELEGRAM_API_EXECUTION_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(TELEGRAM_API_EXECUTION_STAGE)
        || object.get("telegram_api_state").and_then(JsonValue::as_str) != Some("completed")
        || object.get("operation").and_then(JsonValue::as_str) != Some("get_updates")
        || object.get("transport").and_then(JsonValue::as_str) != Some("json")
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || object
            .get("python_telegram_api_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object.get("downloaded").and_then(JsonValue::as_bool) != Some(false)
    {
        return None;
    }
    object.get("updates")?.as_array().cloned()
}

fn validate_service_run_result(
    outcome: &JsonValue,
    stop_requested: bool,
) -> Result<(), WorkerDiagnostic> {
    let Some(object) = outcome.as_object() else {
        return Err(telegram_polling_service_contract_failure());
    };
    if object.get("contract").and_then(JsonValue::as_str) != Some(TELEGRAM_SERVICE_RUN_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(TELEGRAM_SERVICE_RUN_STAGE)
        || object.get("stage").and_then(JsonValue::as_str) != Some("run")
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || object
            .get("unbounded_run_requested")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || !object
            .get("configured_max_cycles")
            .is_some_and(JsonValue::is_null)
        || [
            "python_service_loop_allowed",
            "python_cycle_execution_allowed",
            "python_retry_sleep_allowed",
            "python_stop_control_allowed",
            "python_monotonic_clock_allowed",
        ]
        .into_iter()
        .any(|key| object.get(key).and_then(JsonValue::as_bool) != Some(false))
    {
        return Err(telegram_polling_service_contract_failure());
    }
    if object.get("ok").and_then(JsonValue::as_bool) != Some(true) {
        return Err(telegram_polling_service_execution_failure());
    }
    if !stop_requested
        || object.get("service_run_state").and_then(JsonValue::as_str) != Some("stopped")
        || object.get("stop_reason").and_then(JsonValue::as_str) != Some("stop_requested")
        || object.get("graceful_stop").and_then(JsonValue::as_bool) != Some(true)
        || object
            .get("production_stop_observed")
            .and_then(JsonValue::as_bool)
            != Some(true)
    {
        return Err(WorkerDiagnostic::new(
            "telegram_polling_service_stopped_unexpectedly",
            "The Rust Telegram polling service stopped without a worker-host shutdown request.",
            EXIT_RUNTIME_UNAVAILABLE,
        ));
    }
    Ok(())
}

fn telegram_webhook_response(outcome: &JsonValue) -> Result<WorkerHttpResponse, WorkerDiagnostic> {
    let object = outcome
        .as_object()
        .ok_or_else(telegram_webhook_contract_failure)?;
    if object.get("contract").and_then(JsonValue::as_str)
        != Some(TELEGRAM_WEBHOOK_TRANSACTION_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(TELEGRAM_WEBHOOK_TRANSACTION_STAGE)
        || object.get("stage").and_then(JsonValue::as_str) != Some("execute")
        || [
            "python_ingress_allowed",
            "python_service_entry_loop_allowed",
            "python_update_dispatch_allowed",
            "python_http_response_allowed",
        ]
        .into_iter()
        .any(|key| object.get(key).and_then(JsonValue::as_bool) != Some(false))
    {
        return Err(telegram_webhook_contract_failure());
    }
    let status_code = object
        .get("http_status")
        .and_then(JsonValue::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|value| (100..=599).contains(value))
        .ok_or_else(telegram_webhook_contract_failure)?;
    if object
        .get("write_json_response")
        .and_then(JsonValue::as_bool)
        != Some(true)
    {
        return Err(telegram_webhook_contract_failure());
    }
    let response = object
        .get("response")
        .filter(|value| value.is_object())
        .ok_or_else(telegram_webhook_contract_failure)?;
    if response.get("ok").and_then(JsonValue::as_bool)
        != object.get("ok").and_then(JsonValue::as_bool)
    {
        return Err(telegram_webhook_contract_failure());
    }
    let body = JsonCodec::encode_value_to_vec_with_error_prefix(
        response,
        JsonEncodeOptions::compact(),
        "Failed to encode Rust Telegram webhook response",
    )
    .map_err(|_| telegram_webhook_contract_failure())?;
    Ok(WorkerHttpResponse::new(status_code, body).with_header("Content-Type", JSON_CONTENT_TYPE))
}

fn telegram_webhook_public_error(status_code: u16, message: &str) -> WorkerHttpResponse {
    let body = JsonCodec::encode_value_to_vec(
        &json!({"ok": false, "error": message}),
        JsonEncodeOptions::compact(),
    )
    .unwrap_or_else(|_| b"{\"ok\":false,\"error\":\"Telegram webhook failed.\"}".to_vec());
    WorkerHttpResponse::new(status_code, body).with_header("Content-Type", JSON_CONTENT_TYPE)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left_byte, right_byte) in left.iter().zip(right.iter()) {
        diff |= left_byte ^ right_byte;
    }
    diff == 0
}

fn nonnegative_i64(value: Option<&JsonValue>) -> Option<i64> {
    let value = match value? {
        JsonValue::Number(value) => value.as_i64(),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }?;
    (value >= 0).then_some(value)
}

fn telegram_polling_api_error() -> String {
    "Telegram polling API execution failed.".to_string()
}

fn telegram_polling_service_execution_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "telegram_polling_service_failed",
        "The Rust Telegram polling service failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn telegram_polling_service_contract_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "telegram_polling_service_contract_invalid",
        "The Rust Telegram polling service returned an invalid run contract.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn telegram_webhook_execution_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "telegram_webhook_transaction_failed",
        "The Rust Telegram webhook transaction failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn telegram_webhook_contract_failure() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "telegram_webhook_transaction_contract_invalid",
        "The Rust Telegram webhook transaction returned an invalid response contract.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::sync::{Condvar, Mutex};

    use ait_agent_core::{
        agent_runtime_admission_plan_json, resolve_agent_worker_config, AgentEventLoopBackend,
        AgentWorkerConfigInput, NativeSocket,
    };
    use tempfile::{tempdir, TempDir};

    use super::*;
    use crate::ResolvedWorkerPaths;

    #[derive(Clone)]
    struct StubPollingApiExecutor {
        requests: Arc<Mutex<Vec<JsonValue>>>,
        result: Result<JsonValue, String>,
    }

    impl StubPollingApiExecutor {
        fn new(result: Result<JsonValue, String>) -> Self {
            Self {
                requests: Arc::new(Mutex::new(Vec::new())),
                result,
            }
        }
    }

    impl TelegramPollingApiExecutor for StubPollingApiExecutor {
        fn execute_get_updates(&self, request: &JsonValue) -> Result<JsonValue, String> {
            self.requests.lock().unwrap().push(request.clone());
            self.result.clone()
        }
    }

    #[derive(Default)]
    struct NoopState;

    impl TelegramServiceCycleStatePort for NoopState {
        fn execute_state(
            &self,
            _path: &str,
            _operation: &str,
            _request: &JsonValue,
        ) -> Result<JsonValue, String> {
            Ok(json!({"last_update_id": 0}))
        }
    }

    #[derive(Default)]
    struct EmptyPoll;

    impl TelegramServiceCyclePollPort for EmptyPoll {
        fn poll_updates(&self, _request: &JsonValue) -> Result<Vec<JsonValue>, String> {
            Ok(Vec::new())
        }
    }

    #[derive(Default)]
    struct NoopDispatch;

    impl TelegramServiceCycleDispatchPort for NoopDispatch {
        fn dispatch_update(&self, _request: &JsonValue) -> Result<(), String> {
            Ok(())
        }
    }

    impl TelegramWebhookTransactionDispatchPort for NoopDispatch {
        fn dispatch_update(&self, _request: &JsonValue) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopBackground;

    impl TelegramServiceCycleBackgroundSyncPort for NoopBackground {
        fn run_background_sync_once(&self, _request: &JsonValue) -> Result<usize, String> {
            Ok(0)
        }
    }

    #[derive(Clone)]
    struct StopAwareServiceExecutor {
        started: Arc<AtomicBool>,
    }

    impl TelegramPollingServiceExecutor for StopAwareServiceExecutor {
        fn execute_service_run(&self, stop: Arc<AtomicBool>) -> Result<JsonValue, String> {
            self.started.store(true, Ordering::Release);
            while !stop.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            Ok(graceful_service_run_outcome())
        }
    }

    #[derive(Clone)]
    struct StaticServiceExecutor {
        outcome: Result<JsonValue, String>,
    }

    impl TelegramPollingServiceExecutor for StaticServiceExecutor {
        fn execute_service_run(&self, _stop: Arc<AtomicBool>) -> Result<JsonValue, String> {
            self.outcome.clone()
        }
    }

    #[derive(Default)]
    struct RecordingWebhookDispatch {
        requests: Mutex<Vec<JsonValue>>,
        fail: AtomicBool,
    }

    impl TelegramWebhookTransactionDispatchPort for RecordingWebhookDispatch {
        fn dispatch_update(&self, request: &JsonValue) -> Result<(), String> {
            self.requests.lock().unwrap().push(request.clone());
            if self.fail.load(Ordering::Acquire) {
                Err("dispatch-secret private-update".to_string())
            } else {
                Ok(())
            }
        }
    }

    #[derive(Clone)]
    struct StaticWebhookExecutor {
        result: Result<JsonValue, String>,
    }

    impl TelegramWebhookJobExecutor for StaticWebhookExecutor {
        fn execute_webhook(&self, _request: &JsonValue) -> Result<JsonValue, String> {
            self.result.clone()
        }
    }

    #[derive(Default)]
    struct BlockingGate {
        state: Mutex<(bool, bool)>,
        changed: Condvar,
    }

    #[derive(Clone)]
    struct BlockingWebhookExecutor {
        gate: Arc<BlockingGate>,
    }

    impl TelegramWebhookJobExecutor for BlockingWebhookExecutor {
        fn execute_webhook(&self, _request: &JsonValue) -> Result<JsonValue, String> {
            let mut state = self.gate.state.lock().unwrap();
            state.0 = true;
            self.gate.changed.notify_all();
            while !state.1 {
                state = self.gate.changed.wait(state).unwrap();
            }
            Ok(successful_webhook_outcome())
        }
    }

    #[derive(Default)]
    struct NoopEventLoop;

    impl WorkerHostEventLoop for NoopEventLoop {
        fn register_readable(
            &mut self,
            _token: u64,
            _fd: NativeSocket,
        ) -> Result<(), WorkerDiagnostic> {
            Ok(())
        }

        fn register_read_write(
            &mut self,
            _token: u64,
            _fd: NativeSocket,
        ) -> Result<(), WorkerDiagnostic> {
            Ok(())
        }

        fn unregister(&mut self, _token: u64) -> Result<(), WorkerDiagnostic> {
            Ok(())
        }

        fn wait(&mut self, _timeout: Duration) -> Result<Vec<AgentEvent>, WorkerDiagnostic> {
            Ok(Vec::new())
        }
    }

    fn context() -> (TempDir, WorkerRunContext) {
        context_with_worker(json!({
            "kind": "telegram",
            "name": "main",
            "token": "123:telegram-secret",
            "username": "ait_bot",
        }))
    }

    fn context_with_worker(worker: JsonValue) -> (TempDir, WorkerRunContext) {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join(".ait")).expect("ait dir");
        fs::write(
            temp.path().join(".ait/config.json"),
            r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
        )
        .expect("repo config");
        let config = resolve_agent_worker_config(AgentWorkerConfigInput {
            repo_root: temp.path().to_path_buf(),
            worker_key: "telegram/main".to_string(),
            worker,
            process_env: BTreeMap::new(),
        })
        .expect("Telegram config");
        let context = WorkerRunContext {
            paths: ResolvedWorkerPaths {
                repo_root: temp.path().to_path_buf(),
                manifest_path: temp.path().join(".ait/agent-workers.json"),
            },
            transport: ait_agent_core::TransportKind::Telegram,
            worker_key: "telegram/main".to_string(),
            worker_name: "main".to_string(),
            event_loop_backend: AgentEventLoopBackend::PortablePoll,
            shard_index: 0,
            runtime_admission_plan: agent_runtime_admission_plan_json(&json!({
                "worker_manifest": {
                    "version": 1,
                    "workers": {"telegram/main": {"kind": "telegram", "name": "main"}}
                },
                "backend": "portable_poll",
                "transport_runtime": "rust",
                "allow_python_fallback": false,
                "requested_worker_keys": ["telegram/main"],
            }))
            .expect("runtime admission"),
            config,
        };
        (temp, context)
    }

    fn telegram_config(context: &WorkerRunContext) -> &TelegramWorkerConfig {
        let AgentWorkerRuntimeConfig::Telegram(config) = &context.config else {
            panic!("Telegram config")
        };
        config
    }

    fn telegram_config_mut(context: &mut WorkerRunContext) -> &mut TelegramWorkerConfig {
        let AgentWorkerRuntimeConfig::Telegram(config) = &mut context.config else {
            panic!("Telegram config")
        };
        config
    }

    fn valid_api_outcome(updates: JsonValue) -> JsonValue {
        json!({
            "contract": TELEGRAM_API_EXECUTION_CONTRACT,
            "migration_stage": TELEGRAM_API_EXECUTION_STAGE,
            "telegram_api_state": "completed",
            "operation": "get_updates",
            "transport": "json",
            "ok": true,
            "updates": updates,
            "downloaded": false,
            "python_telegram_api_allowed": false,
        })
    }

    fn graceful_service_run_outcome() -> JsonValue {
        json!({
            "contract": TELEGRAM_SERVICE_RUN_CONTRACT,
            "migration_stage": TELEGRAM_SERVICE_RUN_STAGE,
            "stage": "run",
            "service_run_state": "stopped",
            "stop_reason": "stop_requested",
            "ok": true,
            "completed": true,
            "graceful_stop": true,
            "production_stop_observed": true,
            "unbounded_run_requested": true,
            "configured_max_cycles": JsonValue::Null,
            "python_service_loop_allowed": false,
            "python_cycle_execution_allowed": false,
            "python_retry_sleep_allowed": false,
            "python_stop_control_allowed": false,
            "python_monotonic_clock_allowed": false,
        })
    }

    fn failed_service_run_outcome() -> JsonValue {
        let mut outcome = graceful_service_run_outcome();
        outcome["service_run_state"] = json!("failed_closed");
        outcome["stop_reason"] = json!("cycle_execution_failed");
        outcome["ok"] = json!(false);
        outcome["graceful_stop"] = json!(false);
        outcome["production_stop_observed"] = json!(false);
        outcome["private"] = json!("fatal-secret");
        outcome
    }

    fn successful_webhook_outcome() -> JsonValue {
        json!({
            "contract": TELEGRAM_WEBHOOK_TRANSACTION_CONTRACT,
            "migration_stage": TELEGRAM_WEBHOOK_TRANSACTION_STAGE,
            "stage": "execute",
            "transaction_state": "completed",
            "ok": true,
            "http_status": 200,
            "write_json_response": true,
            "response": {"ok": true, "processed_updates": 1},
            "python_ingress_allowed": false,
            "python_service_entry_loop_allowed": false,
            "python_update_dispatch_allowed": false,
            "python_http_response_allowed": false,
        })
    }

    fn webhook_request(body: Vec<u8>) -> WorkerHttpRequest {
        WorkerHttpRequest {
            method: "POST".to_string(),
            path: "/telegram".to_string(),
            version: "HTTP/1.1".to_string(),
            headers: BTreeMap::new(),
            body,
            peer_addr: "127.0.0.1:12345".parse().unwrap(),
        }
    }

    fn wait_for_completion<E>(handler: &mut TelegramWorkerHttpHandler<E>) -> WorkerHttpCompletion
    where
        E: TelegramWebhookJobExecutor,
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(completion) = handler.poll_completed().into_iter().next() {
                return completion;
            }
            assert!(Instant::now() < deadline, "Telegram webhook job timed out");
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_for_runtime<E>(
        runtime: &mut TelegramPollingWorkerRuntime<E>,
        context: &WorkerRunContext,
        event_loop: &mut NoopEventLoop,
    ) -> Result<(), WorkerDiagnostic>
    where
        E: TelegramPollingServiceExecutor,
    {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            runtime.tick(context, event_loop, &[])?;
            if runtime.completed() {
                return Ok(());
            }
            assert!(Instant::now() < deadline, "Telegram polling job timed out");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn polling_api_port_translates_exact_callback_and_redacts_debug_output() {
        let (_temp, context) = context();
        let executor = StubPollingApiExecutor::new(Ok(valid_api_outcome(json!([
            {"update_id": 7, "message": {"text": "hello"}}
        ]))));
        let requests = Arc::clone(&executor.requests);
        let port = TelegramPollingApiPort::new(telegram_config(&context), executor);

        let updates = port
            .poll_updates(&json!({
                "callback_kind": "poll_updates",
                "poll_request": {"offset": "7", "timeout_seconds": 12},
            }))
            .expect("poll updates");

        assert_eq!(updates[0]["update_id"], 7);
        let recorded = requests.lock().unwrap();
        assert_eq!(recorded[0]["operation"], "get_updates");
        assert_eq!(recorded[0]["bot_token"], "123:telegram-secret");
        assert_eq!(recorded[0]["offset"], 7);
        assert_eq!(recorded[0]["timeout_seconds"], 12);
        let debug = format!("{port:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("telegram-secret"));
    }

    #[test]
    fn polling_api_port_fails_closed_with_generic_secret_safe_errors() {
        let (_temp, context) = context();
        for result in [
            Err("executor-secret".to_string()),
            Ok(json!({"ok": false, "private": "contract-secret"})),
        ] {
            let port = TelegramPollingApiPort::new(
                telegram_config(&context),
                StubPollingApiExecutor::new(result),
            );
            let error = port
                .poll_updates(&json!({
                    "poll_request": {"offset": 0, "timeout_seconds": 5},
                }))
                .expect_err("poll must fail");
            assert_eq!(error, "Telegram polling API execution failed.");
            assert!(!error.contains("secret"));
        }
        let port = TelegramPollingApiPort::new(
            telegram_config(&context),
            StubPollingApiExecutor::new(Ok(valid_api_outcome(json!([])))),
        );
        assert!(port
            .poll_updates(&json!({
                "poll_request": {"offset": -1, "timeout_seconds": 5},
            }))
            .is_err());
    }

    #[test]
    fn polling_service_job_uses_unbounded_rust_driver_and_shared_stop() {
        let (_temp, context) = context();
        let job = TelegramPollingServiceJob::new(
            telegram_config(&context),
            Arc::new(NoopState),
            Arc::new(EmptyPoll),
            Arc::new(NoopDispatch),
            Arc::new(NoopBackground),
        );
        let stop = Arc::new(AtomicBool::new(true));

        let outcome = job
            .execute_service_run(stop)
            .expect("stopped Rust service run");

        assert_eq!(outcome["contract"], TELEGRAM_SERVICE_RUN_CONTRACT);
        assert_eq!(outcome["graceful_stop"], true);
        assert_eq!(outcome["unbounded_run_requested"], true);
        assert_eq!(outcome["configured_max_cycles"], JsonValue::Null);
        assert_eq!(outcome["python_service_loop_allowed"], false);
    }

    #[test]
    fn polling_worker_host_propagates_graceful_stop_and_drains_one_job() {
        let (_temp, context) = context();
        let started = Arc::new(AtomicBool::new(false));
        let executor = StopAwareServiceExecutor {
            started: Arc::clone(&started),
        };
        let mut runtime = TelegramPollingWorkerRuntime::new(executor).unwrap();
        let mut event_loop = NoopEventLoop;
        runtime.start(&context, &mut event_loop).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !started.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "polling service did not start");
            thread::sleep(Duration::from_millis(1));
        }

        runtime
            .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
            .unwrap();
        wait_for_runtime(&mut runtime, &context, &mut event_loop).unwrap();

        assert!(runtime.stop_requested());
        assert!(runtime.completed());
        assert_eq!(runtime.inflight_work_count(), 0);
        runtime.finish_shutdown(&context, &mut event_loop).unwrap();
    }

    #[test]
    fn polling_worker_host_rejects_unexpected_and_fatal_completion_without_secrets() {
        let (_temp, context) = context();
        for (outcome, expected_code) in [
            (
                Ok(graceful_service_run_outcome()),
                "telegram_polling_service_stopped_unexpectedly",
            ),
            (
                Ok(failed_service_run_outcome()),
                "telegram_polling_service_failed",
            ),
            (
                Err("executor-secret".to_string()),
                "telegram_polling_service_failed",
            ),
        ] {
            let executor = StaticServiceExecutor { outcome };
            let mut runtime = TelegramPollingWorkerRuntime::new(executor).unwrap();
            let mut event_loop = NoopEventLoop;
            runtime.start(&context, &mut event_loop).unwrap();
            let error = wait_for_runtime(&mut runtime, &context, &mut event_loop)
                .expect_err("completion must fail closed");
            assert_eq!(error.code, expected_code);
            assert!(!error.message.contains("secret"));
            runtime.force_shutdown(&context, &mut event_loop).unwrap();
        }
    }

    #[test]
    fn native_webhook_handler_executes_transaction_and_renders_public_json() {
        let dispatch = Arc::new(RecordingWebhookDispatch::default());
        let executor = NativeTelegramWebhookJobExecutor::new(Arc::clone(&dispatch));
        let mut handler = TelegramWorkerHttpHandler::new(executor, 1).unwrap();
        let raw = br#"{"update_id":42,"message":{"chat":{"id":9},"text":"private-update"}}"#;

        let WorkerHttpDispatch::Deferred { job_id } =
            handler.handle(webhook_request(raw.to_vec())).unwrap()
        else {
            panic!("deferred webhook")
        };
        let completion = wait_for_completion(&mut handler);
        assert_eq!(completion.job_id, job_id);
        let response = completion.result.expect("webhook response");
        assert_eq!(response.status_code, 200);
        assert_eq!(response.headers["Content-Type"], JSON_CONTENT_TYPE);
        let body = String::from_utf8(response.body).unwrap();
        assert_eq!(body, r#"{"ok":true,"processed_updates":1}"#);
        assert!(!body.contains("private-update"));
        let requests = dispatch.requests.lock().unwrap();
        assert_eq!(requests[0]["dispatch_key"], "chat-9");
        assert_eq!(requests[0]["fallback_update_key"], "webhook-42");
    }

    #[test]
    fn stdin_webhook_once_executes_native_transaction_and_emits_stable_contract() {
        let (_temp, context) = context();
        let dispatch = Arc::new(RecordingWebhookDispatch::default());
        let executor = NativeTelegramWebhookJobExecutor::new(Arc::clone(&dispatch));
        let raw = r#"{"update_id":42,"message":{"chat":{"id":9},"text":"private-update"}}"#;

        let result = execute_telegram_webhook_once_with_executor(&context, raw, &executor).unwrap();

        assert_eq!(result["contract"], TELEGRAM_WEBHOOK_ONCE_CONTRACT);
        assert_eq!(result["binary"], "ait-agent-worker");
        assert_eq!(result["transport"], "telegram");
        assert_eq!(result["worker"], "main");
        assert_eq!(result["ok"], true);
        assert_eq!(result["http_status"], 200);
        assert_eq!(result["response"]["processed_updates"], 1);
        assert_eq!(result["python_worker_execution_allowed"], false);
        assert_eq!(result["python_ingress_allowed"], false);
        assert!(!result.to_string().contains("private-update"));
        let requests = dispatch.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["fallback_update_key"], "webhook-42");
    }

    #[test]
    fn stdin_webhook_once_rejects_malformed_input_without_echoing_it() {
        let (_temp, context) = context();
        let dispatch = Arc::new(RecordingWebhookDispatch::default());
        let executor = NativeTelegramWebhookJobExecutor::new(dispatch);
        let raw = "{private-parser-secret";

        let error = execute_telegram_webhook_once_with_executor(&context, raw, &executor)
            .expect_err("malformed stdin");

        assert_eq!(error.code, "telegram_webhook_input_invalid");
        assert_eq!(error.exit_code, EXIT_INVALID_REQUEST);
        assert_eq!(error.details["http_status"], 400);
        assert!(!error.message.contains(raw));
        assert!(!error.render_json().contains(raw));
    }

    #[test]
    fn stdin_webhook_once_rejects_invalid_native_contract_secret_safely() {
        let (_temp, context) = context();
        let executor = StaticWebhookExecutor {
            result: Ok(json!({"private": "contract-secret"})),
        };

        let error = execute_telegram_webhook_once_with_executor(&context, "{}", &executor)
            .expect_err("invalid contract");

        assert_eq!(error.code, "telegram_webhook_transaction_contract_invalid");
        assert_eq!(error.exit_code, EXIT_RUNTIME_UNAVAILABLE);
        assert!(!error.render_json().contains("contract-secret"));
    }

    #[test]
    fn native_webhook_dispatch_failure_is_http_500_and_secret_safe() {
        let dispatch = Arc::new(RecordingWebhookDispatch::default());
        dispatch.fail.store(true, Ordering::Release);
        let executor = NativeTelegramWebhookJobExecutor::new(dispatch);
        let mut handler = TelegramWorkerHttpHandler::new(executor, 1).unwrap();
        let raw = br#"{"update_id":7,"message":{"text":"private-update"}}"#;

        let WorkerHttpDispatch::Deferred { .. } =
            handler.handle(webhook_request(raw.to_vec())).unwrap()
        else {
            panic!("deferred webhook")
        };
        let response = wait_for_completion(&mut handler)
            .result
            .expect("transaction failure response");
        let body = String::from_utf8(response.body).unwrap();
        assert_eq!(response.status_code, 500);
        assert!(body.contains("Telegram webhook update dispatch failed."));
        assert!(!body.contains("dispatch-secret"));
        assert!(!body.contains("private-update"));
    }

    #[test]
    fn webhook_handler_rejects_invalid_utf8_capacity_and_closed_admission() {
        let gate = Arc::new(BlockingGate::default());
        let executor = BlockingWebhookExecutor {
            gate: Arc::clone(&gate),
        };
        let mut handler = TelegramWorkerHttpHandler::new(executor, 1).unwrap();

        let WorkerHttpDispatch::Immediate(invalid) =
            handler.handle(webhook_request(vec![0xff])).unwrap()
        else {
            panic!("invalid UTF-8 must be immediate")
        };
        assert_eq!(invalid.status_code, 400);

        let WorkerHttpDispatch::Deferred { .. } = handler
            .handle(webhook_request(br#"{"update_id":1}"#.to_vec()))
            .unwrap()
        else {
            panic!("first webhook must be deferred")
        };
        {
            let mut state = gate.state.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !state.0 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "blocking job did not start");
                state = gate.changed.wait_timeout(state, remaining).unwrap().0;
            }
        }
        let WorkerHttpDispatch::Immediate(busy) = handler
            .handle(webhook_request(br#"{"update_id":2}"#.to_vec()))
            .unwrap()
        else {
            panic!("second webhook must be rejected")
        };
        assert_eq!(busy.status_code, 503);
        {
            let mut state = gate.state.lock().unwrap();
            state.1 = true;
            gate.changed.notify_all();
        }
        let completed = wait_for_completion(&mut handler);
        assert_eq!(completed.result.unwrap().status_code, 200);
        handler.close_admission();
        let WorkerHttpDispatch::Immediate(closed) = handler
            .handle(webhook_request(br#"{"update_id":3}"#.to_vec()))
            .unwrap()
        else {
            panic!("closed handler must reject")
        };
        assert_eq!(closed.status_code, 503);
        handler.finish_shutdown().unwrap();
    }

    #[test]
    fn webhook_handler_rejects_invalid_or_failed_executor_contract_without_echoing_errors() {
        for result in [
            Err("executor-secret".to_string()),
            Ok(json!({"private": "contract-secret"})),
        ] {
            let mut handler =
                TelegramWorkerHttpHandler::new(StaticWebhookExecutor { result }, 1).unwrap();
            let WorkerHttpDispatch::Deferred { .. } = handler
                .handle(webhook_request(br#"{"update_id":1}"#.to_vec()))
                .unwrap()
            else {
                panic!("deferred webhook")
            };
            let error = wait_for_completion(&mut handler)
                .result
                .expect_err("executor must fail");
            assert!(matches!(
                error.code,
                "telegram_webhook_transaction_failed"
                    | "telegram_webhook_transaction_contract_invalid"
            ));
            assert!(!error.message.contains("secret"));
        }
    }

    #[test]
    fn webhook_handler_enforces_optional_telegram_secret_without_leaking_it() {
        let (_temp, secret_context) = context_with_worker(json!({
            "kind": "telegram",
            "name": "main",
            "token": "123:telegram-secret",
            "mode": "webhook",
            "webhook_secret": "expected-webhook-secret",
        }));
        let executor = StaticWebhookExecutor {
            result: Ok(successful_webhook_outcome()),
        };
        let mut handler =
            TelegramWorkerHttpHandler::from_config(telegram_config(&secret_context), executor, 1)
                .unwrap();

        for supplied in [None, Some("wrong-webhook-secret")] {
            let mut request = webhook_request(br#"{"update_id":1}"#.to_vec());
            if let Some(supplied) = supplied {
                request.headers.insert(
                    TELEGRAM_WEBHOOK_SECRET_HEADER.to_string(),
                    supplied.to_string(),
                );
            }
            let WorkerHttpDispatch::Immediate(response) = handler.handle(request).unwrap() else {
                panic!("invalid Telegram secret must be rejected immediately")
            };
            assert_eq!(response.status_code, 401);
            let body = String::from_utf8(response.body).unwrap();
            assert!(body.contains("authentication failed"));
            assert!(!body.contains("expected-webhook-secret"));
            assert!(!body.contains("wrong-webhook-secret"));
            assert_eq!(handler.inflight_work_count(), 0);
        }

        let mut accepted = webhook_request(br#"{"update_id":2}"#.to_vec());
        accepted.headers.insert(
            TELEGRAM_WEBHOOK_SECRET_HEADER.to_string(),
            "expected-webhook-secret".to_string(),
        );
        let WorkerHttpDispatch::Deferred { .. } = handler.handle(accepted).unwrap() else {
            panic!("matching Telegram secret must be accepted")
        };
        assert_eq!(
            wait_for_completion(&mut handler)
                .result
                .unwrap()
                .status_code,
            200
        );

        let (_temp, plain_context) = context();
        let mut plain_handler = TelegramWorkerHttpHandler::from_config(
            telegram_config(&plain_context),
            StaticWebhookExecutor {
                result: Ok(successful_webhook_outcome()),
            },
            1,
        )
        .unwrap();
        let WorkerHttpDispatch::Deferred { .. } = plain_handler
            .handle(webhook_request(br#"{"update_id":3}"#.to_vec()))
            .unwrap()
        else {
            panic!("missing configured secret must preserve unauthenticated compatibility")
        };
        assert_eq!(
            wait_for_completion(&mut plain_handler)
                .result
                .unwrap()
                .status_code,
            200
        );
    }

    #[test]
    fn telegram_host_plan_selects_mode_and_validates_webhook_bind_before_dispatch() {
        let (_temp, context) = context();
        let config = telegram_config(&context);
        assert_eq!(
            plan_telegram_worker_host(config).unwrap(),
            TelegramWorkerHostPlan::Poll
        );

        let mut webhook = config.clone();
        webhook.service_mode = TelegramWorkerMode::Webhook;
        webhook.bind_host = "127.0.0.1".to_string();
        webhook.bind_port = 8181;
        assert_eq!(
            plan_telegram_worker_host(&webhook).unwrap(),
            TelegramWorkerHostPlan::Webhook {
                bind_addr: "127.0.0.1:8181".parse().unwrap(),
            }
        );

        for port in [0, 70_000] {
            webhook.bind_port = port;
            let error = plan_telegram_worker_host(&webhook).expect_err("invalid bind port");
            assert_eq!(error.code, "telegram_worker_bind_port_invalid");
            assert!(!error.message.contains("telegram-secret"));
        }
        webhook.bind_port = 8181;
        webhook.bind_host = "invalid host name !".to_string();
        let error = plan_telegram_worker_host(&webhook).expect_err("invalid bind host");
        assert_eq!(error.code, "telegram_worker_bind_address_invalid");
        assert!(!error.message.contains("telegram-secret"));
    }

    #[test]
    fn telegram_webhook_host_enforces_exact_path_before_listener_start() {
        let (_temp, context) = context();
        let mut webhook = telegram_config(&context).clone();
        webhook.service_mode = TelegramWorkerMode::Webhook;
        webhook.webhook_path = "/telegram-hook".to_string();
        let host_config =
            telegram_webhook_host_config(&webhook, "127.0.0.1:8181".parse().expect("bind address"));
        assert_eq!(host_config.expected_method, "POST");
        assert_eq!(host_config.expected_path, "/telegram-hook");
        assert!(host_config.enforce_expected_path);
        assert_eq!(
            host_config.request_timeout,
            TELEGRAM_WEBHOOK_REQUEST_DEADLINE
        );

        webhook.webhook_path = "relative path".to_string();
        let handler = TelegramWorkerHttpHandler::from_config(
            &webhook,
            StaticWebhookExecutor {
                result: Ok(successful_webhook_outcome()),
            },
            1,
        )
        .unwrap();
        let mut runtime = WorkerHttpHostRuntime::new(
            telegram_webhook_host_config(&webhook, "127.0.0.1:8181".parse().expect("bind address")),
            handler,
        );
        let error = runtime
            .start(&context, &mut NoopEventLoop)
            .expect_err("invalid webhook path must fail before listener registration");
        assert_eq!(error.code, "worker_http_host_config_invalid");
        assert_eq!(error.details["field"], "expected_path");
        assert_eq!(runtime.inflight_work_count(), 0);
    }

    #[test]
    fn telegram_composition_fails_closed_for_non_telegram_config() {
        let (temp, mut context) = context();
        context.transport = ait_agent_core::TransportKind::Line;
        context.worker_key = "line/main".to_string();
        context.config = resolve_agent_worker_config(AgentWorkerConfigInput {
            repo_root: temp.path().to_path_buf(),
            worker_key: "line/main".to_string(),
            worker: json!({
                "kind": "line",
                "name": "main",
                "token": "line-token",
                "secret": "line-secret",
            }),
            process_env: BTreeMap::new(),
        })
        .expect("LINE config");

        let error = run_telegram_transport_with_ports(
            &context,
            Arc::new(NoopDispatch),
            Arc::new(NoopBackground),
        )
        .expect_err("non-Telegram config must fail before host startup");

        assert_eq!(error.code, "telegram_worker_config_mismatch");
        assert!(!error.message.contains("line-token"));
        assert!(!error.message.contains("line-secret"));
    }

    #[test]
    fn native_telegram_product_composition_executes_ignored_update_and_drains() {
        let (_temp, context) = context();
        let composition = NativeTelegramProductComposition::from_context(&context)
            .expect("native Telegram composition");

        let rendered = format!("{composition:?}");
        assert!(rendered.contains("native_update_job: true"));
        assert!(rendered.contains("native_submission_runtime: true"));
        assert!(rendered.contains("native_background_sync: true"));
        assert!(!rendered.contains("telegram-secret"));

        let outcome = composition
            .runtime
            .submit_update(json!({"update_id": 1}))
            .expect("ignored update accepted")
            .wait(Some(Duration::from_secs(2)))
            .expect("ignored update completed");
        assert_eq!(outcome["update_state"], "ignored");
        assert_eq!(outcome["action"], "missing_chat");
        assert_eq!(outcome["handled"], false);
        assert_eq!(
            composition
                .background_sync
                .run_background_sync_once(&json!({
                    "callback_kind": "run_background_sync_once",
                    "callback_group": "background_sync",
                }))
                .expect("empty background sync"),
            0
        );

        let running = composition.runtime.snapshot_json();
        assert_eq!(running["submitted_update_count"], 1);
        assert_eq!(running["handled_update_count"], 1);
        assert_eq!(running["dispatch_failed_count"], 0);
        assert_eq!(running["python_submission_allowed"], false);
        assert_eq!(running["python_callback_execution_allowed"], false);

        composition.shutdown().expect("product shutdown");
        let stopped = composition.runtime.snapshot_json();
        assert_eq!(stopped["stopped"], true);
        assert_eq!(stopped["dispatch_inflight_count"], 0);
        assert_eq!(stopped["dispatch_queued_count"], 0);
        assert_eq!(stopped["dispatch_running_count"], 0);
        assert_eq!(stopped["idle_timeout_count"], 0);
    }

    #[test]
    fn native_telegram_product_composition_rejects_invalid_admission_before_startup() {
        let (_temp, context) = context();
        let mut cases = Vec::new();

        let mut python_fallback = context.clone();
        python_fallback.runtime_admission_plan["python_fallback_requested"] = json!(true);
        cases.push(python_fallback);

        let mut wrong_backend = context.clone();
        wrong_backend.runtime_admission_plan["backend"] = json!("linux_epoll");
        cases.push(wrong_backend);

        let mut wrong_shard = context.clone();
        wrong_shard.runtime_admission_plan["worker_leases"][0]["shard_index"] = json!(1);
        cases.push(wrong_shard);

        let mut zero_capacity = context;
        zero_capacity.runtime_admission_plan["shard_admissions"][0]["inflight_limit"] = json!(0);
        cases.push(zero_capacity);

        for invalid in cases {
            let error = NativeTelegramProductComposition::from_context(&invalid)
                .expect_err("invalid runtime admission must fail closed");
            assert_eq!(error.code, "telegram_runtime_admission_invalid");
            assert_eq!(error.exit_code, EXIT_INVALID_CONFIGURATION);
            assert!(!error.render_json().contains("telegram-secret"));
        }
    }

    #[test]
    fn native_telegram_product_composition_rejects_unbounded_turn_config() {
        let (_temp, mut merge_window) = context();
        telegram_config_mut(&mut merge_window).turn_merge_window_seconds = 301.0;
        let error = NativeTelegramProductComposition::from_context(&merge_window)
            .expect_err("unbounded merge window");
        assert_eq!(error.code, "telegram_logical_turn_config_invalid");

        let (_temp, mut max_messages) = context();
        telegram_config_mut(&mut max_messages).turn_merge_max_messages = 1_025;
        let error = NativeTelegramProductComposition::from_context(&max_messages)
            .expect_err("unbounded pending messages");
        assert_eq!(error.code, "telegram_logical_turn_config_invalid");
    }

    #[test]
    fn telegram_registry_exposes_the_native_product_runner() {
        let capability = crate::TransportRunnerRegistry::compiled()
            .capabilities()
            .into_iter()
            .find(|capability| capability.transport == ait_agent_core::TransportKind::Telegram)
            .expect("Telegram capability");
        assert!(capability.runner_available);
        assert!(capability.diagnostic.is_none());
    }
}
