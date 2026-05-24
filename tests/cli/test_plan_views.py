from __future__ import annotations

import importlib

from ait.cli import plan_views

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_plan_view_helpers() -> None:
    helper_names = [
        "_render_plan_summary",
        "_render_plan_detail",
        "_render_plan_revisions",
        "_render_plan_items",
        "_render_plan_candidates",
        "_render_plan_inspect",
        "_render_plan_sync_summary",
        "_render_planning_sessions",
        "_render_planning_session_detail",
        "_render_planning_session_events",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(plan_views, name)


def test_render_plan_detail_smoke_with_items(capsys) -> None:
    plan_views._render_plan_detail(
        {
            "plan_id": "PL-123",
            "title": "Decouple CLI plan views",
            "status": "active",
            "repo_name": "ait",
            "head_revision_id": "PR-123",
            "updated_at": "2026-05-17T12:00:00Z",
            "head_revision": {
                "plan_revision_id": "PR-123",
                "revision_number": 2,
                "title_snapshot": "Decouple CLI plan views",
                "summary": "Extract plan renderers",
                "artifact_path": "docs/sprints/cli_app_plan_rendering_extraction.md",
                "artifact_selector": "cli-app-plan-rendering-extraction/module-split",
                "artifact_heading": "## Module Split",
                "source_kind": "markdown",
                "created_at": "2026-05-17T12:00:00Z",
                "items": [
                    {
                        "plan_item_ref": "cli-app-plan-rendering-extraction/module-split",
                        "checkbox_state": "open",
                        "text": "extract plan renderers",
                        "heading_path": ["CLI app plan rendering extraction", "Module Split"],
                        "line_number": 8,
                    }
                ],
            },
        }
    )
    captured = capsys.readouterr().out
    assert "ait plan PL-123" in captured
    assert "plan revision" in captured
    assert "plan items for PL-123" in captured
    assert "cli-app-plan-rendering-extraction/module-split" in captured


def test_render_plan_sync_summary_smoke_with_artifact_uploads(capsys) -> None:
    plan_views._render_plan_sync_summary(
        {
            "target": "docs/sprints/cli_app_plan_rendering_extraction.md",
            "scope": "remote",
            "summary": {
                "created_count": 1,
                "updated_count": 0,
                "unchanged_count": 0,
                "pruned_count": 0,
                "processed_count": 1,
                "published_count": 1,
                "artifact_count": 1,
            },
            "results": [
                {
                    "action": "created",
                    "artifact_path": "docs/sprints/cli_app_plan_rendering_extraction.md",
                    "plan_id": "PL-123",
                    "plan_revision_id": "PR-123",
                    "status": "active",
                }
            ],
            "artifact_results": [
                {
                    "role": "artifact_body",
                    "artifact_path": "docs/sprints/cli_app_plan_rendering_extraction.md",
                    "plan_id": "PL-123",
                    "plan_revision_id": "PR-123",
                    "blob_id": "BLB-123",
                }
            ],
        }
    )
    captured = capsys.readouterr().out
    assert "ait plan sync" in captured
    assert "plan sync results" in captured
    assert "paired artifact uploads" in captured
    assert "BLB-123" in captured


def test_render_planning_session_views_smoke(capsys) -> None:
    plan_views._render_planning_sessions(
        "PL-123",
        [
            {
                "planning_session_id": "PS-123",
                "status": "active",
                "mode": "connected_local",
                "artifact_status": "draft",
                "title": "CLI decoupling",
                "updated_at": "2026-05-17T12:00:00Z",
            }
        ],
    )
    plan_views._render_planning_session_detail(
        {
            "planning_session_id": "PS-123",
            "plan_id": "PL-123",
            "status": "active",
            "mode": "connected_local",
            "preferred_agent": "codex",
            "artifact_status": "draft",
            "derived_task_id": "RT-1147",
            "last_promoted_plan_revision_id": "PR-123",
            "last_event_sequence": 4,
            "created_by": "codex",
            "created_at": "2026-05-17T12:00:00Z",
            "updated_at": "2026-05-17T12:05:00Z",
            "title": "CLI decoupling",
        }
    )
    plan_views._render_planning_session_events(
        "PS-123",
        [
            {
                "sequence": 4,
                "event_type": "plan.message",
                "actor_identity": "codex",
                "created_at": "2026-05-17T12:06:00Z",
                "payload": {"text": "continue extraction"},
            }
        ],
    )
    captured = capsys.readouterr().out
    assert "planning sessions for PL-123" in captured
    assert "planning session PS-123" in captured
    assert "planning session events for PS-123" in captured
    assert "continue" in captured
    assert "extraction" in captured
