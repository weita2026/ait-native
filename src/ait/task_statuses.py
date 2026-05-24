from __future__ import annotations

from ait_protocol.task_statuses import (
    TASK_ABANDONED_STATUSES,
    TASK_CLOSED_STATUSES,
    TASK_LOCAL_CLOSE_TARGET_STATUSES,
    TASK_REMOTE_CLOSE_TARGET_STATUSES,
    TASK_STATUS_ABANDONED,
    TASK_STATUS_ACTIVE,
    TASK_STATUS_COMPLETED,
    TASK_STATUS_LATER_PROMOTION_EXCLUDED,
    TASK_STATUS_LEGACY_CANCELED,
    is_task_abandoned_status,
    is_task_closed_status,
    is_task_completed_status,
    is_task_later_promotion_excluded_status,
    normalize_task_status,
    task_close_session_status,
    task_status_display_label,
    task_status_matches_filter,
)
