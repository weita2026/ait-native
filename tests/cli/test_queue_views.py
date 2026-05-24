from __future__ import annotations

import importlib

from ait.cli import queue_views

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_queue_view_helpers() -> None:
    helper_names = [
        "_queue_actionable_local_tasks",
        "_queue_actionable_local_changes",
        "_queue_local_summary",
        "_queue_focus_change_reasons",
        "_queue_change_reason",
        "_queue_change_ready_to_land",
        "_queue_change_inventory",
        "_render_queue_summary",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(queue_views, name)


def test_queue_helper_inventory_contract_preserved() -> None:
    local_summary = queue_views._queue_local_summary(
        [
            {"task_id": "T-1", "status": "active", "publication_state": "draft"},
            {"task_id": "T-2", "status": "completed", "publication_state": "published"},
        ],
        [
            {"change_id": "C-1", "status": "draft", "publication_state": "draft"},
            {"change_id": "C-2", "status": "landed", "publication_state": "published"},
        ],
    )
    assert local_summary["draft_task_count"] == 1
    assert local_summary["draft_change_count"] == 1
    assert local_summary["published_task_count"] == 1
    assert local_summary["published_change_count"] == 1

    inventory = queue_views._queue_change_inventory(
        [
            {
                "change_id": "RC-1",
                "title": "Ready change",
                "status": "draft",
                "base_line": "main",
                "current_patchset_number": 1,
            },
            {
                "change_id": "RC-2",
                "title": "Needs first patchset",
                "status": "draft",
                "base_line": "main",
                "current_patchset_number": 0,
            },
            {
                "change_id": "RC-3",
                "title": "Already landed",
                "status": "landed",
                "base_line": "main",
                "current_patchset_number": 1,
            },
        ],
        [
            {
                "focus_change": {
                    "change_id": "RC-2",
                    "reason": "Create the first patchset before asking for review.",
                }
            }
        ],
        [
            {
                "change_id": "RC-1",
                "review_state": {"blocking": 0},
                "freshness": {"base_is_fresh": True},
                "attestation": {"tests": "pass"},
                "policy_state": {"decision": "pass", "missing_requirements": []},
            }
        ],
    )

    assert [row["change_id"] for row in inventory] == ["RC-1", "RC-2"]
    assert inventory[0]["ready_to_land"] is True
    assert inventory[0]["reason"] == "Ready to land."
    assert inventory[1]["ready_to_land"] is False
    assert inventory[1]["reason"] == "Create the first patchset before asking for review."


def test_render_queue_summary_populated_smoke(capsys) -> None:
    queue_views._render_queue_summary(
        {
            "repo_name": "ait",
            "query": {"all_changes": True, "status": "active"},
            "remote": {
                "configured": True,
                "remote_name": "origin",
                "repo_name": "ait",
                "task_queue": {
                    "items": [
                        {
                            "task": {"task_id": "RT-1150", "title": "Extract queue views"},
                            "workflow": {"state": "in_progress"},
                            "next_action": {"code": "publish_patchset"},
                        }
                    ]
                },
                "reviewer_inbox": {
                    "items": [
                        {
                            "change_id": "RC-0990",
                            "title": "Extract queue views",
                            "policy_state": {"decision": "pending"},
                            "attestation": {"tests": "pending"},
                        }
                    ]
                },
                "changes": [
                    {
                        "change_id": "RC-0990",
                        "title": "Extract queue views",
                        "status": "draft",
                        "base_line": "main",
                        "current_patchset_number": 1,
                        "reason": "Tests are still pending for the current patchset.",
                        "ready_to_land": False,
                    }
                ],
            },
            "local": {
                "tasks": [{"task_id": "T-1", "title": "Local draft", "status": "active", "publication_state": "draft"}],
                "changes": [{"change_id": "C-1", "title": "Local draft change", "status": "draft", "base_line": "main", "publication_state": "draft"}],
                "summary": {"draft_task_count": 1, "draft_change_count": 1},
            },
            "workspace": {
                "status": {"clean": False, "modified_paths": ["src/ait/cli/app.py"], "missing_paths": [], "untracked_paths": []},
                "worktrees": {"dirty_count": 1, "stale_count": 1},
            },
            "summary": {
                "shared_task_count": 1,
                "attention_required_count": 1,
                "ready_to_land_count": 0,
                "ready_to_complete_count": 0,
                "open_shared_change_count": 1,
                "reviewer_inbox_count": 1,
                "local_draft_task_count": 1,
                "local_draft_change_count": 1,
                "workspace_dirty": True,
                "workspace_changed_count": 1,
                "dirty_worktree_count": 1,
                "stale_worktree_count": 1,
            },
        }
    )
    captured = capsys.readouterr().out
    assert "ait queue summary" in captured
    assert "shared queue" in captured
    assert "review inbox" in captured
    assert "shared changes" in captured
    assert "local draft tasks" in captured
    assert "local draft changes" in captured
    assert "Workspace dirty:" in captured
    assert "Worktree attention:" in captured
    assert "publish_patchset" in captured


def test_render_queue_summary_empty_state_smoke(capsys) -> None:
    queue_views._render_queue_summary(
        {
            "repo_name": "ait",
            "query": {"all_changes": True, "status": "active"},
            "remote": {
                "configured": True,
                "remote_name": "origin",
                "repo_name": "ait",
                "task_queue": {"items": []},
                "reviewer_inbox": {"items": []},
                "changes": [],
            },
            "local": {"tasks": [], "changes": [], "summary": {"draft_task_count": 0, "draft_change_count": 0}},
            "workspace": {"status": {"clean": True}, "worktrees": {"dirty_count": 0, "stale_count": 0}},
            "summary": {
                "shared_task_count": 0,
                "attention_required_count": 0,
                "ready_to_land_count": 0,
                "ready_to_complete_count": 0,
                "open_shared_change_count": 0,
                "reviewer_inbox_count": 0,
                "local_draft_task_count": 0,
                "local_draft_change_count": 0,
                "workspace_dirty": False,
                "workspace_changed_count": 0,
                "dirty_worktree_count": 0,
                "stale_worktree_count": 0,
            },
        }
    )
    captured = capsys.readouterr().out
    assert "No open shared changes detected on the remote." in captured
    assert "No active shared tasks, local drafts, or workspace changes detected." in captured
