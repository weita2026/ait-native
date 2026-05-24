from __future__ import annotations

import importlib

from ait.cli import task_close_tracking


task_cli_module = importlib.import_module("ait.cli.commands.task")


def test_task_command_reexports_extracted_close_tracking_helpers() -> None:
    helper_names = [
        "_SESSION_STATUS_PRIORITY",
        "_append_task_retrospective_event",
        "_auto_track_created_task",
        "_build_task_retrospective",
        "_close_task_review_session",
        "_create_task_retrospective_checkpoint",
        "_ensure_task_review_session_active",
        "_finalize_task_close_tracking",
        "_latest_task_session_row",
        "_list_task_review_session_events",
        "_maybe_attach_task_tracking",
        "_resolve_task_review_session",
        "_task_improvement_plan",
        "_task_tracking_session_metadata",
        "_task_tracking_session_title",
        "_trim_current_task_close_event",
    ]

    for name in helper_names:
        assert getattr(task_cli_module, name) is getattr(task_close_tracking, name)
