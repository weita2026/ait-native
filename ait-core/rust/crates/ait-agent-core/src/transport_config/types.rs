use std::fmt;
use std::path::PathBuf;

use ait_core::json_support::{json, JsonValue};

use crate::supervisor::AgentWorkerRuntimePaths;
use crate::transport::TransportKind;

#[derive(Clone, PartialEq, Eq)]
pub struct AgentSecret(String);

impl AgentSecret {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AgentSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AgentSecret(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentWorkflowMode {
    SoloLocal,
    SoloRemote,
    TeamRemote,
}

impl AgentWorkflowMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SoloLocal => "solo_local",
            Self::SoloRemote => "solo_remote",
            Self::TeamRemote => "team_remote",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRuntimeMode {
    Local,
    Remote,
}

impl AgentRuntimeMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRuntimeTarget {
    pub mode: AgentRuntimeMode,
    pub workflow_mode: AgentWorkflowMode,
    pub repo_name: String,
    pub repo_root: PathBuf,
    pub remote_name: Option<String>,
    pub server_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentSharedWorkerConfig {
    pub worker_key: String,
    pub worker_name: String,
    pub transport: TransportKind,
    pub runtime_target: AgentRuntimeTarget,
    pub paths: AgentWorkerRuntimePaths,
    pub ait_web_url: Option<String>,
    pub request_timeout_seconds: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramSttMode {
    Off,
    LocalStt,
}

impl TelegramSttMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::LocalStt => "local-stt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramWorkerMode {
    Poll,
    Webhook,
}

impl TelegramWorkerMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Poll => "poll",
            Self::Webhook => "webhook",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TelegramWorkerConfig {
    pub shared: AgentSharedWorkerConfig,
    pub token: AgentSecret,
    pub username: String,
    pub service_mode: TelegramWorkerMode,
    pub bind_host: String,
    pub bind_port: i64,
    pub webhook_path: String,
    pub webhook_secret: Option<AgentSecret>,
    pub poll_timeout_seconds: u64,
    pub background_sync_enabled: bool,
    pub background_sync_interval_seconds: f64,
    pub openai_api_key: Option<AgentSecret>,
    pub openai_base_url: String,
    pub openai_model: String,
    pub openai_reasoning_effort: Option<String>,
    pub openai_timeout_seconds: Option<f64>,
    pub openai_max_output_tokens: u64,
    pub turn_merge_window_seconds: f64,
    pub turn_merge_max_messages: u64,
    pub decoupled_reply_enabled: bool,
    pub reply_markdown_enabled: bool,
    pub owner_bootstrap_enabled: bool,
    pub stt_mode: TelegramSttMode,
    pub stt_model: String,
    pub stt_device: String,
    pub stt_compute_type: Option<String>,
    pub stt_language: Option<String>,
    pub stt_include_audio_uploads: bool,
    pub stt_program: Option<PathBuf>,
    pub stt_timeout_seconds: f64,
    pub expected_concurrent_workers: Option<usize>,
    pub event_loop_backend: Option<String>,
    pub workers_per_shard: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineWorkerConfig {
    pub shared: AgentSharedWorkerConfig,
    pub channel_access_token: AgentSecret,
    pub channel_secret: AgentSecret,
    pub api_base_url: String,
    pub bind_host: String,
    pub bind_port: i64,
    pub webhook_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscordWorkerConfig {
    pub shared: AgentSharedWorkerConfig,
    pub application_id: AgentSecret,
    pub public_key: Option<AgentSecret>,
    pub bot_token: Option<AgentSecret>,
    pub turn_timeout_seconds: Option<f64>,
    pub api_base_url: String,
    pub http_user_agent: String,
    pub bind_host: String,
    pub bind_port: i64,
    pub interaction_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlackWorkerConfig {
    pub shared: AgentSharedWorkerConfig,
    pub app_token: Option<AgentSecret>,
    pub signing_secret: Option<AgentSecret>,
    pub api_base_url: String,
    pub http_user_agent: String,
    pub bind_host: String,
    pub bind_port: i64,
    pub command_path: String,
    pub ack_text: String,
    pub response_type: String,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum AgentWorkerRuntimeConfig {
    Telegram(TelegramWorkerConfig),
    Line(LineWorkerConfig),
    Discord(DiscordWorkerConfig),
    Slack(SlackWorkerConfig),
}

impl AgentWorkerRuntimeConfig {
    pub fn transport(&self) -> TransportKind {
        self.shared().transport
    }

    pub fn shared(&self) -> &AgentSharedWorkerConfig {
        match self {
            Self::Telegram(config) => &config.shared,
            Self::Line(config) => &config.shared,
            Self::Discord(config) => &config.shared,
            Self::Slack(config) => &config.shared,
        }
    }

    pub fn redacted_json(&self) -> JsonValue {
        let shared = self.shared();
        let common = json!({
            "worker_key": shared.worker_key,
            "worker_name": shared.worker_name,
            "transport": shared.transport,
            "runtime": {
                "mode": shared.runtime_target.mode.as_str(),
                "workflow_mode": shared.runtime_target.workflow_mode.as_str(),
                "repo_name": shared.runtime_target.repo_name,
                "remote_name": shared.runtime_target.remote_name,
                "server_url_set": shared.runtime_target.server_url.is_some(),
            },
            "paths": shared.paths,
            "ait_web_url_set": shared.ait_web_url.is_some(),
            "request_timeout_seconds": shared.request_timeout_seconds,
        });
        match self {
            Self::Telegram(config) => json!({
                "common": common,
                "credentials": {
                    "token_set": true,
                    "openai_api_key_set": config.openai_api_key.is_some(),
                    "webhook_secret_set": config.webhook_secret.is_some(),
                },
                "telegram": {
                    "username": config.username,
                    "service_mode": config.service_mode.as_str(),
                    "bind_host": config.bind_host,
                    "bind_port": config.bind_port,
                    "webhook_path": config.webhook_path,
                    "poll_timeout_seconds": config.poll_timeout_seconds,
                    "background_sync_enabled": config.background_sync_enabled,
                    "background_sync_interval_seconds": config.background_sync_interval_seconds,
                    "openai_base_url_set": !config.openai_base_url.is_empty(),
                    "openai_model": config.openai_model,
                    "openai_reasoning_effort": config.openai_reasoning_effort,
                    "openai_timeout_seconds": config.openai_timeout_seconds,
                    "openai_max_output_tokens": config.openai_max_output_tokens,
                    "turn_merge_window_seconds": config.turn_merge_window_seconds,
                    "turn_merge_max_messages": config.turn_merge_max_messages,
                    "decoupled_reply_enabled": config.decoupled_reply_enabled,
                    "reply_markdown_enabled": config.reply_markdown_enabled,
                    "owner_bootstrap_enabled": config.owner_bootstrap_enabled,
                    "stt_mode": config.stt_mode.as_str(),
                    "stt_model": config.stt_model,
                    "stt_device": config.stt_device,
                    "stt_compute_type": config.stt_compute_type,
                    "stt_language": config.stt_language,
                    "stt_include_audio_uploads": config.stt_include_audio_uploads,
                    "stt_program_set": config.stt_program.is_some(),
                    "stt_timeout_seconds": config.stt_timeout_seconds,
                    "expected_concurrent_workers": config.expected_concurrent_workers,
                    "event_loop_backend": config.event_loop_backend,
                    "workers_per_shard": config.workers_per_shard,
                },
            }),
            Self::Line(config) => json!({
                "common": common,
                "credentials": {
                    "channel_access_token_set": true,
                    "channel_secret_set": true,
                },
                "line": {
                    "api_base_url_set": !config.api_base_url.is_empty(),
                    "bind_host": config.bind_host,
                    "bind_port": config.bind_port,
                    "webhook_path": config.webhook_path,
                },
            }),
            Self::Discord(config) => json!({
                "common": common,
                "credentials": {
                    "application_id_set": true,
                    "public_key_set": config.public_key.is_some(),
                    "bot_token_set": config.bot_token.is_some(),
                },
                "discord": {
                    "turn_timeout_seconds": config.turn_timeout_seconds,
                    "api_base_url_set": !config.api_base_url.is_empty(),
                    "http_user_agent": config.http_user_agent,
                    "bind_host": config.bind_host,
                    "bind_port": config.bind_port,
                    "interaction_path": config.interaction_path,
                },
            }),
            Self::Slack(config) => json!({
                "common": common,
                "credentials": {
                    "app_token_set": config.app_token.is_some(),
                    "signing_secret_set": config.signing_secret.is_some(),
                },
                "slack": {
                    "api_base_url_set": !config.api_base_url.is_empty(),
                    "http_user_agent": config.http_user_agent,
                    "bind_host": config.bind_host,
                    "bind_port": config.bind_port,
                    "command_path": config.command_path,
                    "ack_text": config.ack_text,
                    "response_type": config.response_type,
                },
            }),
        }
    }
}
