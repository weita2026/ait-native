//! Stable typed outcomes and file-I/O error normalization.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinaryDbErrorKind {
    RetryableBusy,
    Corruption,
    LayoutMismatch,
    MissingData,
    InvalidDomainData,
    Io,
    Unsupported,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbError {
    kind: BinaryDbErrorKind,
    message: String,
}

impl BinaryDbError {
    pub fn new(kind: BinaryDbErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> BinaryDbErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn contains(&self, pattern: &str) -> bool {
        self.message.contains(pattern)
    }

    pub fn is_retryable_busy(&self) -> bool {
        self.kind == BinaryDbErrorKind::RetryableBusy
    }

    pub fn retryable_busy(message: impl Into<String>) -> Self {
        Self::new(BinaryDbErrorKind::RetryableBusy, message)
    }

    pub fn corruption(message: impl Into<String>) -> Self {
        Self::new(BinaryDbErrorKind::Corruption, message)
    }

    pub fn layout_mismatch(message: impl Into<String>) -> Self {
        Self::new(BinaryDbErrorKind::LayoutMismatch, message)
    }

    pub fn missing_data(message: impl Into<String>) -> Self {
        Self::new(BinaryDbErrorKind::MissingData, message)
    }

    pub fn invalid_domain_data(message: impl Into<String>) -> Self {
        Self::new(BinaryDbErrorKind::InvalidDomainData, message)
    }

    pub fn io(action: impl fmt::Display, err: io::Error) -> Self {
        Self::new(BinaryDbErrorKind::Io, format!("{action}: {err}"))
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(BinaryDbErrorKind::Unsupported, message)
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self::new(BinaryDbErrorKind::Other, message)
    }
}

impl fmt::Display for BinaryDbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BinaryDbError {}

impl From<String> for BinaryDbError {
    fn from(message: String) -> Self {
        Self::other(message)
    }
}

impl From<&str> for BinaryDbError {
    fn from(message: &str) -> Self {
        Self::other(message)
    }
}

impl From<BinaryDbError> for String {
    fn from(error: BinaryDbError) -> Self {
        error.to_string()
    }
}

pub type StoreResult<T> = Result<T, BinaryDbError>;

pub(crate) fn file_io_error_to_binary(
    action: impl fmt::Display,
    err: FileIoError,
) -> BinaryDbError {
    let message = format!("{action}: {err}");
    match err.kind() {
        FileIoErrorKind::Unsupported => BinaryDbError::unsupported(message),
        FileIoErrorKind::InvalidData => BinaryDbError::corruption(message),
        FileIoErrorKind::WouldBlock => BinaryDbError::retryable_busy(message),
        _ => BinaryDbError::new(BinaryDbErrorKind::Io, message),
    }
}
