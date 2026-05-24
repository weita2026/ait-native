from __future__ import annotations


TASK_STATUS_ACTIVE = "active"
TASK_STATUS_COMPLETED = "completed"
TASK_STATUS_ABANDONED = "abandoned"
TASK_STATUS_LATER_PROMOTION_EXCLUDED = "later_promotion_excluded"
TASK_STATUS_LEGACY_CANCELED = "canceled"

TASK_CLOSED_STATUSES = frozenset(
    {
        TASK_STATUS_COMPLETED,
        TASK_STATUS_ABANDONED,
        TASK_STATUS_LATER_PROMOTION_EXCLUDED,
        TASK_STATUS_LEGACY_CANCELED,
    }
)
TASK_ABANDONED_STATUSES = frozenset({TASK_STATUS_ABANDONED, TASK_STATUS_LEGACY_CANCELED})
TASK_LOCAL_CLOSE_TARGET_STATUSES = frozenset(
    {
        TASK_STATUS_COMPLETED,
        TASK_STATUS_ABANDONED,
        TASK_STATUS_LATER_PROMOTION_EXCLUDED,
        TASK_STATUS_LEGACY_CANCELED,
    }
)
TASK_REMOTE_CLOSE_TARGET_STATUSES = frozenset(
    {
        TASK_STATUS_COMPLETED,
        TASK_STATUS_ABANDONED,
        TASK_STATUS_LEGACY_CANCELED,
    }
)


def normalize_task_status(value: str | None) -> str | None:
    text = str(value or "").strip().lower()
    return text or None


def is_task_completed_status(value: str | None) -> bool:
    return normalize_task_status(value) == TASK_STATUS_COMPLETED


def is_task_abandoned_status(value: str | None) -> bool:
    return normalize_task_status(value) in TASK_ABANDONED_STATUSES


def is_task_later_promotion_excluded_status(value: str | None) -> bool:
    return normalize_task_status(value) == TASK_STATUS_LATER_PROMOTION_EXCLUDED


def is_task_closed_status(value: str | None) -> bool:
    return normalize_task_status(value) in TASK_CLOSED_STATUSES


def task_close_session_status(value: str | None) -> str | None:
    normalized = normalize_task_status(value)
    if normalized == TASK_STATUS_COMPLETED:
        return "completed"
    if normalized in TASK_CLOSED_STATUSES:
        return "canceled"
    return None


def task_status_display_label(value: str | None) -> str:
    normalized = normalize_task_status(value)
    if normalized == TASK_STATUS_COMPLETED:
        return "completed"
    if normalized == TASK_STATUS_LATER_PROMOTION_EXCLUDED:
        return "later-promotion-excluded"
    if normalized in TASK_ABANDONED_STATUSES:
        return "abandoned"
    return normalized or "unknown"


def task_status_matches_filter(task_status: str | None, requested_status: str | None) -> bool:
    normalized_filter = normalize_task_status(requested_status)
    normalized_task = normalize_task_status(task_status)
    if normalized_filter in {None, "all"}:
        return True
    if normalized_filter == TASK_STATUS_LEGACY_CANCELED:
        return normalized_task in TASK_ABANDONED_STATUSES
    return normalized_task == normalized_filter
