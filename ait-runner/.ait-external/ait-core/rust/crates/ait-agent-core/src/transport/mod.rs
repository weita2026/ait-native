use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub mod config;
pub mod envelope;
pub mod http_request;
pub mod retry;
pub mod websocket_connect;
pub mod websocket_fd_io;
pub mod websocket_frame;
pub mod websocket_handshake;
pub mod websocket_stream;
pub mod websocket_tls;
pub use config::{
    agent_transport_config_clean_optional_text, agent_transport_config_normalize_base_url,
    agent_transport_config_parse_int, agent_transport_config_parse_timeout_seconds,
    agent_transport_config_split_message_chunks, agent_transport_config_split_message_chunks_json,
    AgentTransportConfigIntMode,
};
pub use envelope::{
    agent_compact_transport_event_envelope_json, agent_compact_transport_reply_envelope_json,
    agent_transport_binding_metadata_json, agent_transport_envelope_ir_version,
    agent_transport_envelope_schema_json, agent_transport_event_envelope_json,
    agent_transport_reply_envelope_json,
};
pub use http_request::{
    agent_transport_http_error_message, agent_transport_http_execute_bytes_request,
    agent_transport_http_execute_json_request_json,
    agent_transport_http_execute_multipart_json_request_json,
    agent_transport_http_execute_multipart_json_request_with_bytes,
    agent_transport_http_invalid_timeout_message, agent_transport_http_plan_json_request_json,
    agent_transport_http_plan_multipart_request_json, agent_transport_http_response_payload_json,
    agent_transport_http_timeout_message, agent_transport_http_transport_error_message,
    agent_transport_http_url_error_message, AgentTransportHttpBytesExecution,
};
pub use retry::{
    agent_transport_retry_default_errnos_json, agent_transport_retry_default_markers_json,
    agent_transport_retry_default_server_read_markers_json, agent_transport_retry_delay_seconds,
    agent_transport_retry_is_loopback_url,
    agent_transport_retry_is_retryable_server_read_error_json,
    agent_transport_retry_is_retryable_transport_error_json, agent_transport_retry_timeout_phrase,
    agent_transport_retry_timeout_value, DEFAULT_RETRYABLE_ERRNOS, DEFAULT_RETRYABLE_MARKERS,
    DEFAULT_SERVER_READ_MARKERS,
};
pub use websocket_connect::agent_transport_websocket_connect_execute_json;
pub use websocket_fd_io::agent_transport_websocket_fd_io_execute_json;
pub use websocket_frame::agent_transport_websocket_frame_plan_json;
pub use websocket_handshake::agent_transport_websocket_handshake_plan_json;
pub use websocket_stream::agent_transport_websocket_stream_plan_json;
pub use websocket_tls::agent_transport_websocket_tls_execute_json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Telegram,
    Discord,
    Slack,
    Line,
}

impl TransportKind {
    pub const ALL: [TransportKind; 4] = [
        TransportKind::Telegram,
        TransportKind::Discord,
        TransportKind::Slack,
        TransportKind::Line,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::Line => "line",
        }
    }

    pub fn actor_type(self) -> &'static str {
        match self {
            Self::Telegram => "telegram_bot",
            Self::Discord => "discord_bot",
            Self::Slack => "slack_bot",
            Self::Line => "line_bot",
        }
    }
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TransportKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "telegram" => Ok(Self::Telegram),
            "discord" => Ok(Self::Discord),
            "slack" => Ok(Self::Slack),
            "line" => Ok(Self::Line),
            other => Err(format!("unsupported ait-agent transport `{other}`")),
        }
    }
}
