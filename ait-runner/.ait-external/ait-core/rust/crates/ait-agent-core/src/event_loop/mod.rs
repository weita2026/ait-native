mod backend;
mod capacity;
mod discord_gateway_runtime;
mod discord_ingress_runtime;
mod discord_interaction_job;
mod discord_reply_delivery;
mod discord_rest_delivery;
mod driver;
mod line_event_job;
mod line_http_transaction;
mod line_reply_delivery;
mod line_webhook_ingress;
mod reactor;
mod runtime_admission;
mod slack_command_job;
mod slack_ingress_runtime;
mod slack_reply_delivery;
mod slack_socket_mode_runtime;
mod telegram_api_method;
mod telegram_background_sync;
mod telegram_background_sync_state;
mod telegram_command_runtime;
mod telegram_command_trigger;
mod telegram_dispatch_runtime;
mod telegram_event_triggers;
mod telegram_file_download;
mod telegram_logical_turn_runtime;
mod telegram_message_formatting;
mod telegram_operational_trigger;
mod telegram_owner_bootstrap;
mod telegram_polling;
mod telegram_reply_delivery;
mod telegram_reply_spool;
mod telegram_service_cycle;
mod telegram_service_run;
mod telegram_stt_execution;
mod telegram_submission_dispatch;
mod telegram_submission_runtime;
mod telegram_turn_inputs;
mod telegram_update_job;
mod telegram_webhook_ingress;
mod telegram_webhook_transaction;
mod telegram_workflow_notifications;
mod telegram_workflow_query;
mod websocket_lifecycle;
mod websocket_reactor_run;
mod websocket_reactor_tick;
mod websocket_registration;
mod websocket_runtime_orchestration;
mod websocket_runtime_timers;
mod websocket_shard;
mod websocket_turn;

pub use backend::AgentEventLoopBackend;
pub use capacity::{
    AgentEventLoopConfig, AgentRuntimeCapacity, DEFAULT_WORKERS_PER_EPOLL_SHARD,
    DEFAULT_WORKERS_PER_POLL_SHARD,
};
pub use discord_gateway_runtime::{
    agent_discord_gateway_runtime_plan_json, plan_with_discord_gateway_runtime_planner,
    DefaultDiscordGatewayRuntimePlanner, DiscordGatewayRuntimePlanner,
};
pub use discord_ingress_runtime::{
    agent_discord_ingress_runtime_plan_json, plan_with_discord_ingress_runtime_planner,
    DefaultDiscordIngressRuntimePlanner, DiscordIngressRuntimePlanner,
};
pub use discord_interaction_job::{
    agent_discord_interaction_job_execute_json, execute_with_discord_interaction_job_ports,
    DefaultDiscordInteractionJobBackendPort, DefaultDiscordInteractionJobStatePort,
    DiscordInteractionJobBackendPort, DiscordInteractionJobStatePort,
};
pub use discord_reply_delivery::{
    agent_discord_reply_delivery_callback_plan_json,
    agent_discord_reply_delivery_execution_plan_json,
};
pub use discord_rest_delivery::{
    agent_discord_rest_delivery_execute_json, execute_with_discord_rest_delivery_executor,
    DefaultDiscordRestDeliveryExecutor, DiscordRestDeliveryExecutor,
};
pub use driver::{
    agent_event_loop_backend, poll_agent_event_loop, register_agent_event_loop_read_write,
    register_agent_event_loop_readable, unregister_agent_event_loop, AgentEvent, AgentEventLoop,
    AgentEventLoopBackendPort, AgentEventLoopDriver, AgentEventLoopPollPort,
    AgentEventLoopReadWriteRegistrationPort, AgentEventLoopReadableRegistrationPort,
    AgentEventLoopRegistrationPort, AgentEventLoopUnregistrationPort,
};
pub use line_event_job::{
    agent_line_event_job_execute_json, execute_with_line_event_job_ports,
    DefaultLineEventJobBackendPort, DefaultLineEventJobDeliveryPort, DefaultLineEventJobStatePort,
    LineEventJobBackendPort, LineEventJobDeliveryPort, LineEventJobStatePort,
};
pub use line_http_transaction::{
    agent_line_http_transaction_execute_json, execute_with_line_http_transaction_ports,
    DefaultLineHttpTransactionEventJobPort, DefaultLineHttpTransactionIngressPort,
    LineHttpTransactionEventJobPort, LineHttpTransactionIngressPort,
};
pub use line_reply_delivery::{
    agent_line_reply_delivery_execute_json, execute_with_line_reply_delivery_executor,
    DefaultLineReplyDeliveryExecutor, LineReplyDeliveryExecutor,
};
pub use line_webhook_ingress::{
    agent_line_webhook_ingress_plan_json, plan_with_line_webhook_ingress_planner,
    DefaultLineWebhookIngressPlanner, LineWebhookIngressPlanner,
};
pub use reactor::{
    agent_event_loop_reactor_plan_json, plan_agent_event_loop_reactor, AgentReactorPlan,
    AgentReactorPlanInput, AgentReactorShardPlan, AgentReactorWorkerSlot,
};
pub use runtime_admission::{
    agent_runtime_admission_plan_json, plan_agent_runtime_admission, AgentRuntimeAdmissionInput,
    AgentRuntimeAdmissionPlan, AgentRuntimeShardAdmission, AgentRuntimeWorkerLease,
};
pub use slack_command_job::{
    agent_slack_command_job_execute_json, execute_with_slack_command_job_ports,
    DefaultSlackCommandJobBackendPort, DefaultSlackCommandJobDeliveryPort,
    DefaultSlackCommandJobStatePort, SlackCommandJobBackendPort, SlackCommandJobDeliveryPort,
    SlackCommandJobStatePort,
};
pub use slack_ingress_runtime::{
    agent_slack_command_http_ingress_plan_json, agent_slack_command_http_transaction_plan_json,
    agent_slack_ingress_runtime_plan_json, agent_slack_socket_mode_transaction_plan_json,
    plan_with_slack_command_http_ingress_planner, plan_with_slack_command_http_transaction_planner,
    plan_with_slack_ingress_runtime_planner, plan_with_slack_socket_mode_transaction_planner,
    DefaultSlackCommandHttpIngressPlanner, DefaultSlackCommandHttpTransactionPlanner,
    DefaultSlackIngressRuntimePlanner, DefaultSlackSocketModeTransactionPlanner,
    SlackCommandHttpIngressPlanner, SlackCommandHttpTransactionPlanner, SlackIngressRuntimePlanner,
    SlackSocketModeTransactionPlanner,
};
pub use slack_reply_delivery::{
    agent_slack_background_reply_transaction_execute_json, agent_slack_reply_delivery_plan_json,
    agent_slack_response_url_delivery_execute_json,
    execute_with_slack_background_reply_transaction,
    execute_with_slack_response_url_delivery_executor, plan_with_slack_reply_delivery_planner,
    DefaultSlackReplyDeliveryPlanner, DefaultSlackResponseUrlDeliveryExecutor,
    SlackReplyDeliveryPlanner, SlackResponseUrlDeliveryExecutor,
};
pub use slack_socket_mode_runtime::{
    agent_slack_socket_mode_runtime_plan_json, plan_with_slack_socket_mode_runtime_planner,
    DefaultSlackSocketModeRuntimePlanner, SlackSocketModeRuntimePlanner,
};
pub use telegram_api_method::{
    agent_telegram_api_execute, agent_telegram_api_json_execute_json,
    agent_telegram_api_method_execution_plan_json, execute_with_telegram_api_json_ports,
    execute_with_telegram_api_transport_ports, plan_with_telegram_api_method_planner,
    DefaultTelegramApiMethodPlanner, NativeTelegramApiJsonHttpExecutor,
    NativeTelegramApiTransportExecutor, TelegramApiJsonHttpExecutor, TelegramApiMethodPlanner,
    TelegramApiRetrySleeper, TelegramApiTransportExecution, TelegramApiTransportExecutor,
    ThreadTelegramApiRetrySleeper,
};
pub use telegram_background_sync::{
    execute_with_telegram_background_sync_ports, NativeTelegramBackgroundSyncChildExecutionPort,
    NativeTelegramBackgroundSyncOperationPort, NativeTelegramBackgroundSyncServicePort,
    RuntimeBindingTelegramBackgroundSyncReadPort, SystemTelegramBackgroundSyncClockPort,
    TelegramBackgroundSyncBindingReadPort, TelegramBackgroundSyncChildExecutionPort,
    TelegramBackgroundSyncClockPort, TelegramBackgroundSyncExecution,
    TelegramBackgroundSyncExecutionError, TelegramBackgroundSyncExecutionErrorKind,
    TelegramBackgroundSyncOperationPort, TelegramBackgroundSyncSubmissionPort,
};
pub use telegram_background_sync_state::{
    agent_telegram_background_sync_state_plan_json,
    plan_with_telegram_background_sync_state_planner, DefaultTelegramBackgroundSyncStatePlanner,
    TelegramBackgroundSyncStatePlanner,
};
pub use telegram_command_runtime::{
    agent_telegram_command_runtime_plan_json, execute_with_telegram_command_runtime_ports,
    plan_with_telegram_command_runtime_planner, DefaultTelegramCommandRuntimePlanner,
    RuntimeBindingTelegramCommandRuntimeStatePort, SelectedBackendTelegramCommandRuntimeReadPort,
    SystemTelegramCommandRuntimeClockPort, TelegramCommandRuntimeClockPort,
    TelegramCommandRuntimeDeliveryPort, TelegramCommandRuntimeExecutionError,
    TelegramCommandRuntimeExecutionErrorKind, TelegramCommandRuntimePlanner,
    TelegramCommandRuntimeReadPort, TelegramCommandRuntimeStatePort,
};
pub use telegram_command_trigger::{
    agent_telegram_command_trigger_execute_operation_json,
    execute_with_telegram_command_trigger_operation_executor,
    DefaultTelegramCommandTriggerOperationExecutor, TelegramCommandTriggerOperationExecutor,
};
pub use telegram_dispatch_runtime::{
    agent_telegram_dispatch_runtime_plan_json, plan_with_telegram_dispatch_runtime_planner,
    DefaultTelegramDispatchRuntimePlanner, TelegramDispatchRuntimePlanner,
    TelegramKeyedDispatchError, TelegramKeyedDispatchErrorKind, TelegramKeyedDispatchFuture,
    TelegramKeyedDispatchJobExecutor, TelegramKeyedDispatchRuntime,
};
pub use telegram_event_triggers::{
    agent_telegram_event_trigger_plan_json, plan_with_telegram_event_trigger_planner,
    DefaultTelegramEventTriggerPlanner, NativeTelegramEventTriggerRegistryLoader,
    TelegramEventTriggerPlanner,
};
pub use telegram_file_download::{
    agent_telegram_file_download_execute, agent_telegram_file_download_execution_plan_json,
    execute_with_telegram_file_download_ports, plan_with_telegram_file_download_planner,
    DefaultTelegramFileDownloadPlanner, NativeTelegramFileDownloadApiPort,
    NativeTelegramFileDownloadStorePort, TelegramFileCacheState, TelegramFileDownloadApiExecution,
    TelegramFileDownloadApiPort, TelegramFileDownloadExecution, TelegramFileDownloadExecutionError,
    TelegramFileDownloadExecutionErrorKind, TelegramFileDownloadPlanner,
    TelegramFileDownloadStorePort,
};
pub use telegram_logical_turn_runtime::{
    agent_telegram_logical_turn_runtime_plan_json, plan_with_telegram_logical_turn_runtime_planner,
    DefaultTelegramLogicalTurnRuntimePlanner, MonotonicTelegramLogicalTurnClock,
    TelegramLogicalTurn, TelegramLogicalTurnBufferOutcome, TelegramLogicalTurnClockPort,
    TelegramLogicalTurnError, TelegramLogicalTurnErrorKind, TelegramLogicalTurnRuntime,
    TelegramLogicalTurnRuntimePlanner, TelegramLogicalTurnSleepPort, TelegramLogicalTurnStep,
    ThreadTelegramLogicalTurnSleeper,
};
pub use telegram_message_formatting::{
    agent_telegram_message_delivery_execute_json, agent_telegram_message_formatting_plan_json,
    execute_with_telegram_message_delivery_ports, plan_with_telegram_message_formatting_planner,
    DefaultTelegramMessageFormattingPlanner, NativeTelegramMessageDeliveryApiPort,
    TelegramMessageDeliveryApiPort, TelegramMessageFormattingPlanner,
};
pub use telegram_operational_trigger::{
    execute_with_telegram_operational_trigger_ports,
    DefaultTelegramOperationalTriggerCallbackPlanner,
    RuntimeBindingTelegramOperationalTriggerStatePort, TelegramOperationalTriggerCallbackPlanner,
    TelegramOperationalTriggerDeliveryPort, TelegramOperationalTriggerDiagnosticsPort,
    TelegramOperationalTriggerExecutionConfig, TelegramOperationalTriggerExecutionError,
    TelegramOperationalTriggerExecutionErrorKind, TelegramOperationalTriggerPorts,
    TelegramOperationalTriggerStatePort,
};
pub use telegram_owner_bootstrap::{
    agent_telegram_owner_bootstrap_plan_json, execute_with_telegram_owner_bootstrap_ports,
    plan_with_telegram_owner_bootstrap_planner, DefaultTelegramOwnerBootstrapPlanner,
    RuntimeBindingTelegramOwnerBootstrapStatePort, SystemTelegramOwnerBootstrapClockPort,
    TelegramOwnerBootstrapClockPort, TelegramOwnerBootstrapExecutionError,
    TelegramOwnerBootstrapExecutionErrorKind, TelegramOwnerBootstrapMessagePort,
    TelegramOwnerBootstrapPlanner, TelegramOwnerBootstrapStatePort,
};
pub use telegram_polling::{
    agent_telegram_callback_action_boundary_plan_json, agent_telegram_callback_execution_plan_json,
    agent_telegram_callback_side_effect_adapter_plan_json,
    agent_telegram_command_trigger_execution_plan_json,
    agent_telegram_live_reply_delivery_callback_plan_json,
    agent_telegram_operational_trigger_callback_plan_json, agent_telegram_polling_cycle_plan_json,
    agent_telegram_reply_delivery_execution_plan_json,
    agent_telegram_reply_turn_delivery_callback_plan_json,
    agent_telegram_service_runtime_shell_plan_json,
    agent_telegram_service_shell_callback_plan_json,
    agent_telegram_update_batch_dispatch_plan_json, agent_telegram_update_dispatch_plan_json,
};
pub use telegram_reply_delivery::{
    agent_telegram_reply_delivery_execute_json, execute_with_telegram_reply_delivery_ports,
    DefaultTelegramReplyDeliveryPlanner, NativeTelegramReplyDeliveryPort,
    TelegramReplyDeliveryExecutionError, TelegramReplyDeliveryExecutionErrorKind,
    TelegramReplyDeliveryOperationKind, TelegramReplyDeliveryPlanner, TelegramReplyDeliveryPort,
};
pub use telegram_reply_spool::{
    agent_telegram_reply_spool_execution_plan_json, execute_with_telegram_reply_spool_ports,
    load_telegram_reply_spool_entries, plan_with_telegram_reply_spool_planner,
    DefaultTelegramReplySpoolPlanner, RuntimeBindingTelegramReplySpoolStatePort,
    SystemTelegramReplySpoolClockPort, TelegramReplySpoolClockPort, TelegramReplySpoolEntries,
    TelegramReplySpoolExecutionError, TelegramReplySpoolExecutionErrorKind,
    TelegramReplySpoolMutation, TelegramReplySpoolPlanner, TelegramReplySpoolStatePort,
};
pub use telegram_service_cycle::{
    execute_with_telegram_service_cycle_ports, DefaultTelegramServiceCycleStatePort,
    TelegramServiceCycleBackgroundSyncPort, TelegramServiceCycleDispatchPort,
    TelegramServiceCyclePollPort, TelegramServiceCycleStatePort,
};
pub use telegram_service_run::{
    execute_with_telegram_service_run_ports, TelegramServiceRunClockPort,
    TelegramServiceRunCycleExecutor, TelegramServiceRunCyclePort, TelegramServiceRunSleepPort,
    TelegramServiceRunStopPort,
};
pub use telegram_stt_execution::{
    ExternalProgramTelegramSttExecutor, TelegramSttExecutionError, TelegramSttExecutionErrorKind,
    TelegramSttExecutor, TELEGRAM_STT_EXECUTION_CONTRACT, TELEGRAM_STT_REQUEST_CONTRACT,
    TELEGRAM_STT_RESPONSE_CONTRACT,
};
pub use telegram_submission_dispatch::TelegramSubmissionDispatchPort;
pub use telegram_submission_runtime::{
    agent_telegram_submission_runtime_plan_json, plan_with_telegram_submission_runtime_planner,
    DefaultTelegramSubmissionRuntimePlanner, TelegramSubmissionExecutionError,
    TelegramSubmissionExecutionErrorKind, TelegramSubmissionExecutionPort,
    TelegramSubmissionFuture, TelegramSubmissionRuntime, TelegramSubmissionRuntimePlanner,
};
pub use telegram_turn_inputs::{
    agent_telegram_turn_input_plan_json, plan_with_telegram_turn_input_planner,
    DefaultTelegramTurnInputPlanner, TelegramTurnInputPlanner,
};
pub use telegram_update_job::{
    execute_with_telegram_update_lifecycle_runtime, DefaultTelegramUpdateAssistantReplyExecutor,
    DefaultTelegramUpdateFileDownloadExecutor, DefaultTelegramUpdateLifecycleExecutor,
    DefaultTelegramUpdateMessageExecutor, NativeTelegramOperationalTriggerDeliveryPort,
    NativeTelegramUpdateBootstrapPort, NativeTelegramUpdateCommandPort,
    NativeTelegramUpdateCommandRuntimeExecutor, NativeTelegramUpdateInputPort,
    NativeTelegramUpdateLifecyclePort, NativeTelegramUpdateLifecycleRuntime,
    NativeTelegramUpdateMessagePort, NativeTelegramUpdateOperationalExecutor,
    NativeTelegramUpdateOperationalPort, NativeTelegramUpdateOwnerBootstrapExecutor,
    SystemTelegramOperationalTriggerDiagnosticsPort, SystemTelegramUpdateDiagnosticsPort,
    TelegramPreparedUpdateInput, TelegramUpdateAssistantReplyExecutor, TelegramUpdateBootstrapPort,
    TelegramUpdateBootstrapRequest, TelegramUpdateCommandExecutor, TelegramUpdateCommandPort,
    TelegramUpdateCommandRequest, TelegramUpdateDeliveryPort, TelegramUpdateDiagnosticsPort,
    TelegramUpdateFileDownloadExecutor, TelegramUpdateInputError, TelegramUpdateInputErrorKind,
    TelegramUpdateInputMode, TelegramUpdateInputPort, TelegramUpdateInputRequest,
    TelegramUpdateJob, TelegramUpdateJobConfig, TelegramUpdateJobError, TelegramUpdateJobErrorKind,
    TelegramUpdateJobPorts, TelegramUpdateLifecycleExecutor, TelegramUpdateLifecyclePort,
    TelegramUpdateLifecycleRuntimePort, TelegramUpdateMessageExecutor,
    TelegramUpdateNormalTurnRequest, TelegramUpdateOperationalExecutor,
    TelegramUpdateOperationalMessagePort, TelegramUpdateOperationalPort,
    TelegramUpdateOperationalRequest, TelegramUpdateOwnerBootstrapExecutor,
    TelegramUpdatePortError,
};
pub use telegram_webhook_ingress::{
    agent_telegram_webhook_ingress_plan_json, plan_with_telegram_webhook_ingress_planner,
    DefaultTelegramWebhookIngressPlanner, TelegramWebhookIngressPlanner,
};
pub use telegram_webhook_transaction::{
    execute_with_telegram_webhook_transaction_ports, DefaultTelegramWebhookTransactionIngressPort,
    TelegramWebhookTransactionDispatchPort, TelegramWebhookTransactionIngressPort,
};
pub use telegram_workflow_notifications::{
    agent_telegram_workflow_notification_format_json,
    execute_with_telegram_workflow_notification_ports,
    format_with_telegram_workflow_notification_formatter,
    DefaultTelegramWorkflowNotificationFormatter, NativeTelegramWorkflowNotificationMessagePort,
    TelegramWorkflowNotificationExecution, TelegramWorkflowNotificationExecutionError,
    TelegramWorkflowNotificationExecutionErrorKind, TelegramWorkflowNotificationFormatter,
    TelegramWorkflowNotificationMessagePort,
};
pub use telegram_workflow_query::{
    agent_telegram_workflow_query_plan_json, plan_with_telegram_workflow_query_planner,
    DefaultTelegramWorkflowQueryPlanner, TelegramWorkflowQueryPlanner,
};
pub use websocket_lifecycle::{
    execute_agent_websocket_lifecycle_actions, execute_with_websocket_lifecycle_executor,
    DefaultWebSocketLifecycleExecutor, WebSocketLifecycleExecutor,
};
pub use websocket_reactor_run::execute_agent_websocket_reactor_run;
pub use websocket_reactor_tick::execute_agent_websocket_reactor_tick;
pub use websocket_registration::{
    agent_websocket_registration_action_plan_json, execute_agent_websocket_registration_actions,
};
pub use websocket_runtime_orchestration::{
    agent_websocket_runtime_orchestration_plan_json,
    plan_with_websocket_runtime_orchestration_planner, DefaultWebSocketRuntimeOrchestrationPlanner,
    WebSocketRuntimeOrchestrationPlanner,
};
pub use websocket_runtime_timers::{
    agent_websocket_runtime_timer_scheduler_plan_json, plan_with_websocket_runtime_timer_scheduler,
    DefaultWebSocketRuntimeTimerScheduler, WebSocketRuntimeTimerScheduler,
};
pub use websocket_shard::agent_websocket_shard_event_batch_plan_json;
pub use websocket_turn::agent_websocket_event_loop_turn_plan_json;
