mod diagnostic;
mod discord_gateway;
mod discord_interaction_once;
mod discord_runner;
mod http_host;
mod language_bindings;
mod line_runner;
mod paths;
mod registry;
mod reply_provider;
mod run;
mod slack_command_once;
mod slack_runner;
mod slack_socket_mode;
mod telegram_runner;
mod worker_host;
mod worker_jobs;

use ait_agent_core::{AgentEventLoopBackend, TransportKind};
use ait_core::json_support::{JsonCodec, JsonEncodeOptions};
use serde::Serialize;

pub use diagnostic::{
    WorkerDiagnostic, EXIT_INVALID_CONFIGURATION, EXIT_INVALID_REQUEST, EXIT_RUNTIME_UNAVAILABLE,
    WORKER_ERROR_CONTRACT,
};
pub use discord_gateway::{
    run_discord_gateway_transport, DefaultDiscordGatewayConnector,
    DefaultDiscordGatewayHttpExecutor, DiscordGatewayClock, DiscordGatewayConnection,
    DiscordGatewayConnector, DiscordGatewayEvent, DiscordGatewayHttpExecutor,
    DiscordGatewaySettings, DiscordGatewayWorkerRuntime, SystemDiscordGatewayClock,
};
pub use discord_interaction_once::{
    execute_discord_interaction_once, execute_discord_interaction_once_with_job_executor,
    DefaultDiscordInteractionOnceJobExecutor, DiscordInteractionOnceJobExecutor,
    DiscordInteractionOnceRequest, DISCORD_INTERACTION_ONCE_CONTRACT,
};
pub use discord_runner::{
    run_discord_transport, DefaultDiscordHttpInteractionJobExecutor,
    DiscordHttpInteractionJobExecutor, DiscordWorkerHttpHandler,
};
pub use http_host::{
    WorkerHttpCompletion, WorkerHttpDispatch, WorkerHttpHandler, WorkerHttpHostConfig,
    WorkerHttpHostRuntime, WorkerHttpRequest, WorkerHttpResponse,
};
pub use language_bindings::{
    agent_worker_capabilities_binding_json, agent_worker_transaction_binding_json,
};
pub use line_runner::{
    run_line_transport, DefaultLineHttpTransactionExecutor, LineHttpTransactionExecutor,
    LineWorkerHttpHandler,
};
pub use paths::{
    process_worker_path_inputs, resolve_worker_paths, ResolvedWorkerPaths, WorkerPathInputs,
};
pub use registry::{
    TransportRunner, TransportRunnerCapability, TransportRunnerRegistration,
    TransportRunnerRegistry,
};
pub use reply_provider::{configure_native_reply_provider, execute_native_reply_provider};
pub use run::{
    execute_worker_request, prepare_worker_run, prepare_worker_run_with_env, WorkerRunContext,
    WorkerRunRequest,
};
pub use slack_command_once::{
    execute_slack_command_once, execute_slack_command_once_with_job_executor,
    DefaultSlackCommandOnceJobExecutor, SlackCommandOnceJobExecutor, SlackCommandOnceRequest,
    SLACK_COMMAND_ONCE_CONTRACT,
};
pub use slack_runner::{
    run_slack_transport, DefaultSlackHttpCommandJobExecutor, SlackHttpCommandJobExecutor,
    SlackWorkerHttpHandler,
};
pub use slack_socket_mode::{
    run_slack_socket_mode_transport, DefaultSlackSocketModeConnector, SlackSocketModeClock,
    SlackSocketModeConnection, SlackSocketModeConnector, SlackSocketModeEvent,
    SlackSocketModeSettings, SlackSocketModeWorkerRuntime, SystemSlackSocketModeClock,
};
pub use telegram_runner::{
    execute_telegram_webhook_once, run_telegram_transport, run_telegram_transport_with_ports,
    DefaultTelegramPollingApiExecutor, NativeTelegramWebhookJobExecutor,
    TelegramPollingApiExecutor, TelegramPollingApiPort, TelegramPollingServiceExecutor,
    TelegramPollingServiceJob, TelegramPollingWorkerRuntime, TelegramWebhookJobExecutor,
    TelegramWorkerHttpHandler, TELEGRAM_WEBHOOK_ONCE_CONTRACT,
};
pub use worker_host::{
    run_worker_host, run_worker_host_with_ports, AgentEventLoopHostWait, ProcessShutdownSource,
    SystemWorkerHostClock, WorkerHostClock, WorkerHostEventLoop, WorkerHostHealthSnapshot,
    WorkerHostHealthState, WorkerHostRuntime, WorkerHostSettings, WorkerShutdownSource,
    WORKER_HOST_HEALTH_CONTRACT,
};
pub use worker_jobs::{
    BoundedWorkerJobExecutor, WorkerJobCompletion, WorkerJobExecutorConfig, WorkerJobExecutorState,
};

pub const WORKER_CAPABILITIES_CONTRACT: &str = "ait.agent.worker.capabilities.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkerCapabilityReport {
    pub contract: &'static str,
    pub binary: &'static str,
    pub version: &'static str,
    pub platform: &'static str,
    pub architecture: &'static str,
    pub platform_supported: bool,
    pub native_socket_backend: &'static str,
    pub process_control_backend: &'static str,
    pub supported_transports: Vec<TransportKind>,
    pub transport_capabilities: Vec<TransportRunnerCapability>,
    pub event_loop_backends: Vec<&'static str>,
    pub default_event_loop_backend: &'static str,
    pub python_worker_execution_allowed: bool,
}

pub fn compiled_worker_capabilities() -> WorkerCapabilityReport {
    worker_capabilities(&TransportRunnerRegistry::compiled())
}

pub fn worker_capabilities(registry: &TransportRunnerRegistry) -> WorkerCapabilityReport {
    let mut event_loop_backends = vec![AgentEventLoopBackend::PortablePoll.label()];
    if cfg!(target_os = "linux") {
        event_loop_backends.insert(0, AgentEventLoopBackend::LinuxEpoll.label());
    }
    WorkerCapabilityReport {
        contract: WORKER_CAPABILITIES_CONTRACT,
        binary: "ait-agent-worker",
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        platform_supported: cfg!(any(unix, windows)),
        native_socket_backend: if cfg!(windows) { "winsock" } else { "posix" },
        process_control_backend: if cfg!(windows) {
            "windows_process_api"
        } else {
            "unix_signals"
        },
        supported_transports: registry.supported_transports(),
        transport_capabilities: registry.capabilities(),
        event_loop_backends,
        default_event_loop_backend: AgentEventLoopBackend::current_platform_default().label(),
        python_worker_execution_allowed: false,
    }
}

pub fn render_capabilities_json(capabilities: &WorkerCapabilityReport) -> Result<String, String> {
    JsonCodec::encode_serializable_with_error_prefix(
        capabilities,
        JsonEncodeOptions::pretty().with_trailing_newline(),
        "Failed to serialize ait-agent-worker capabilities",
    )
    .map_err(String::from)
}

pub fn render_capabilities_text(capabilities: &WorkerCapabilityReport) -> String {
    let supported = if capabilities.supported_transports.is_empty() {
        "none".to_string()
    } else {
        capabilities
            .supported_transports
            .iter()
            .map(|transport| transport.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = vec![
        "ait-agent-worker capabilities".to_string(),
        format!(
            "platform: {}/{} ({})",
            capabilities.platform,
            capabilities.architecture,
            if capabilities.platform_supported {
                "supported"
            } else {
                "unsupported"
            }
        ),
        format!(
            "native socket backend: {}",
            capabilities.native_socket_backend
        ),
        format!(
            "process control backend: {}",
            capabilities.process_control_backend
        ),
        format!("supported transports: {supported}"),
        format!(
            "event-loop backends: {}",
            capabilities.event_loop_backends.join(", ")
        ),
        format!(
            "default event-loop backend: {}",
            capabilities.default_event_loop_backend
        ),
        "python worker execution allowed: false".to_string(),
    ];
    for capability in &capabilities.transport_capabilities {
        let state = if capability.runner_available {
            "available"
        } else {
            "unavailable"
        };
        lines.push(format!("{}: {state}", capability.transport));
    }
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_capability_report_is_machine_readable_and_enables_all_product_runners() {
        let capabilities = compiled_worker_capabilities();

        assert_eq!(capabilities.contract, WORKER_CAPABILITIES_CONTRACT);
        assert!(capabilities.platform_supported);
        assert!(!capabilities.platform.is_empty());
        assert!(!capabilities.architecture.is_empty());
        assert_eq!(
            capabilities.supported_transports,
            vec![
                TransportKind::Telegram,
                TransportKind::Discord,
                TransportKind::Slack,
                TransportKind::Line
            ]
        );
        assert_eq!(
            capabilities.transport_capabilities.len(),
            TransportKind::ALL.len()
        );
        assert!(!capabilities.python_worker_execution_allowed);
        let rendered = render_capabilities_json(&capabilities).expect("capability JSON");
        assert!(rendered.contains(
            "\"supported_transports\": [\n    \"telegram\",\n    \"discord\",\n    \"slack\",\n    \"line\"\n  ]"
        ));
    }
}
