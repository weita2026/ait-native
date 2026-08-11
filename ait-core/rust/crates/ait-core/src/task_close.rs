use std::fmt;

pub const TASK_STATUS_COMPLETED: &str = "completed";
pub const TASK_STATUS_ABANDONED: &str = "abandoned";
pub const TASK_STATUS_LATER_PROMOTION_EXCLUDED: &str = "later_promotion_excluded";
pub const TASK_STATUS_LEGACY_CANCELED: &str = "canceled";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskCloseScope {
    Local,
    Remote,
}

impl TaskCloseScope {
    pub fn parse(value: Option<&str>) -> Result<Self, TaskCloseError> {
        match normalize_optional_text(value) {
            Some("local") => Ok(Self::Local),
            Some("remote") => Ok(Self::Remote),
            _ => Err(TaskCloseError::InvalidScope),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }

    fn supports_status(&self, status: &str) -> bool {
        match self {
            Self::Local => matches!(
                status,
                TASK_STATUS_COMPLETED
                    | TASK_STATUS_ABANDONED
                    | TASK_STATUS_LATER_PROMOTION_EXCLUDED
                    | TASK_STATUS_LEGACY_CANCELED
            ),
            Self::Remote => matches!(
                status,
                TASK_STATUS_COMPLETED | TASK_STATUS_ABANDONED | TASK_STATUS_LEGACY_CANCELED
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskCloseRequest {
    pub requested_status: Option<String>,
    pub abandoned: bool,
    pub exclude_later_promotion: bool,
    pub scope: TaskCloseScope,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskCloseResult {
    status: String,
    scope: TaskCloseScope,
}

impl TaskCloseResult {
    pub fn new(status: String, scope: TaskCloseScope) -> Self {
        Self { status, scope }
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn scope(&self) -> &TaskCloseScope {
        &self.scope
    }

    pub fn display_label(&self) -> &'static str {
        match self.status.as_str() {
            TASK_STATUS_COMPLETED => "completed",
            TASK_STATUS_LATER_PROMOTION_EXCLUDED => "later-promotion-excluded",
            TASK_STATUS_ABANDONED | TASK_STATUS_LEGACY_CANCELED => "abandoned",
            _ => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskCloseError {
    InvalidScope,
    ConflictingImplicitFlags,
    LaterPromotionExcludedRequiresLocal,
    ExplicitStatusWithImplicitFlags,
    UnsupportedStatus {
        scope: TaskCloseScope,
        status: String,
    },
}

impl fmt::Display for TaskCloseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScope => write!(f, "Task close scope must be `local` or `remote`."),
            Self::ConflictingImplicitFlags => {
                write!(f, "Use either --abandoned or --exclude-later-promotion, not both.")
            }
            Self::LaterPromotionExcludedRequiresLocal => write!(
                f,
                "--exclude-later-promotion only applies to local unpublished task lineage."
            ),
            Self::ExplicitStatusWithImplicitFlags => write!(
                f,
                "Explicit task close status cannot be combined with `abandoned` or `exclude_later_promotion`."
            ),
            Self::UnsupportedStatus { scope, status } => {
                write!(f, "Unsupported {} task close status: {}", scope.as_str(), status)
            }
        }
    }
}

impl std::error::Error for TaskCloseError {}

pub fn resolve_task_close(request: TaskCloseRequest) -> Result<TaskCloseResult, TaskCloseError> {
    let status = match normalize_optional_text(request.requested_status.as_deref()) {
        Some(status) => {
            if request.abandoned || request.exclude_later_promotion {
                return Err(TaskCloseError::ExplicitStatusWithImplicitFlags);
            }
            status.to_string()
        }
        None => {
            if request.abandoned && request.exclude_later_promotion {
                return Err(TaskCloseError::ConflictingImplicitFlags);
            }
            if request.exclude_later_promotion {
                if request.scope != TaskCloseScope::Local {
                    return Err(TaskCloseError::LaterPromotionExcludedRequiresLocal);
                }
                TASK_STATUS_LATER_PROMOTION_EXCLUDED.to_string()
            } else {
                TASK_STATUS_ABANDONED.to_string()
            }
        }
    };

    if !request.scope.supports_status(&status) {
        return Err(TaskCloseError::UnsupportedStatus {
            scope: request.scope,
            status,
        });
    }

    Ok(TaskCloseResult::new(status, request.scope))
}

fn normalize_optional_text(value: Option<&str>) -> Option<&str> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

#[cfg(test)]
mod tests;
