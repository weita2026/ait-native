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

impl BinaryDbErrorKind {
    pub const fn transport_tag(self) -> &'static str {
        match self {
            Self::RetryableBusy => "retryable_busy",
            Self::Corruption => "corruption",
            Self::LayoutMismatch => "layout_mismatch",
            Self::MissingData => "missing_data",
            Self::InvalidDomainData => "invalid_domain_data",
            Self::Io => "io",
            Self::Unsupported => "unsupported",
            Self::Other => "other",
        }
    }

    pub fn from_transport_tag(tag: &str) -> Option<Self> {
        match tag {
            "retryable_busy" => Some(Self::RetryableBusy),
            "corruption" => Some(Self::Corruption),
            "layout_mismatch" => Some(Self::LayoutMismatch),
            "missing_data" => Some(Self::MissingData),
            "invalid_domain_data" => Some(Self::InvalidDomainData),
            "io" => Some(Self::Io),
            "unsupported" => Some(Self::Unsupported),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

const BINARY_DB_RUNTIME_ERROR_PREFIX: &str = "ait.binary-db.error.v1|";

pub fn binary_db_runtime_error(context: &str, error: BinaryDbError) -> String {
    if error.is_retryable_busy() {
        return format!(
            "{BINARY_DB_RUNTIME_ERROR_PREFIX}{}|{context}: {}",
            error.kind().transport_tag(),
            error.message()
        );
    }

    format!("{context}: {}", error.message())
}

pub fn binary_db_runtime_error_kind(message: &str) -> Option<BinaryDbErrorKind> {
    let tagged = message.strip_prefix(BINARY_DB_RUNTIME_ERROR_PREFIX)?;
    let (tag, _) = tagged.split_once('|')?;
    BinaryDbErrorKind::from_transport_tag(tag)
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

    pub fn io(action: impl fmt::Display, err: std::io::Error) -> Self {
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
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
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
