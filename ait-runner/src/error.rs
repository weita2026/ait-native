use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("invalid native job request: {0}")]
    InvalidRequest(String),
    #[error("filesystem operation `{operation}` failed for `{path}`: {source}")]
    FileSystem {
        operation: &'static str,
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("native process execution failed: {0}")]
    Process(String),
    #[error("ait-server temporarily unavailable: {0}")]
    ServerUnavailable(String),
    #[error("ait-server protocol failed: {0}")]
    Server(String),
    #[error("attempt cleanup failed: {0}")]
    Cleanup(String),
}

impl RunnerError {
    pub fn fs(operation: &'static str, path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::FileSystem {
            operation,
            path: path.as_ref().display().to_string(),
            source,
        }
    }

    pub(crate) fn bounded_message(&self, max_chars: usize) -> String {
        let message = self.to_string();
        if message.chars().count() <= max_chars {
            return message;
        }
        let mut bounded = message
            .chars()
            .take(max_chars.saturating_sub(1))
            .collect::<String>();
        bounded.push('…');
        bounded
    }

    pub(crate) fn is_server_unavailable(&self) -> bool {
        matches!(self, Self::ServerUnavailable(_))
    }
}
