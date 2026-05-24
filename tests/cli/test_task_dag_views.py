from __future__ import annotations

import importlib

from ait.cli import task_dag_views

cli_app_module = importlib.import_module("ait.cli.app")


def test_cli_app_reexports_extracted_task_dag_view_helpers() -> None:
    helper_names = [
        "_render_task_dag_graph",
        "_render_task_dag_schedule",
        "_render_task_dag_progress",
        "_render_task_dag_dispatch",
        "_render_task_dag_execute",
    ]

    for name in helper_names:
        assert getattr(cli_app_module, name) is getattr(task_dag_views, name)


def test_render_task_dag_graph_and_schedule_smoke(capsys) -> None:
    payload = {
        "graph_id": "demo/task-dag",
        "plan_id": "PL-demo",
        "workflow_summary": {
            "ready_nodes": 1,
            "dispatched_nodes": 1,
            "running_nodes": 1,
            "blocked_nodes": 0,
            "completed_nodes": 1,
            "next_action": "dispatch_ready",
        },
        "readiness_summary": {"stale_source_plan": True},
        "execution_strategy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "dispatch_model": "compact_packet",
            "recommended_worker_sessions": 1,
        },
        "nodes": [
            {
                "node_id": "A",
                "state": "completed",
                "workflow_state": "completed",
                "depends_on": [],
                "task_id": "RT-1",
                "change_id": "RC-1",
                "title": "Finished node",
            }
        ],
        "ready": [
            {
                "node_id": "B",
                "state": "ready",
                "workflow_state": "ready",
                "plan_item_ref": "demo/b",
                "session_recommendation": {"action": "reuse_or_create"},
                "title": "Ready node",
            }
        ],
        "dispatched": [
            {
                "node_id": "C",
                "state": "dispatched",
                "workflow_state": "planning",
                "task_id": "RT-2",
                "title": "Dispatched node",
            }
        ],
        "running": [
            {
                "node_id": "D",
                "task_id": "RT-3",
                "change_id": "RC-3",
                "session_id": "S-123",
                "action": "continue",
            }
        ],
        "telegram_graph_watch": {
            "registered": True,
            "already_registered": False,
            "chat_id": "7661100833",
            "resolution_mode": "linked_session",
        },
    }

    task_dag_views._render_task_dag_graph(payload)
    task_dag_views._render_task_dag_schedule(payload)
    captured = capsys.readouterr().out
    assert "demo/task-dag" in captured
    assert "task DAG graph for PL-demo" in captured
    assert "telegram graph watch:" in captured
    assert "auto-registered" in captured
    assert "ready work for PL-demo" in captured
    assert "dispatched planning" in captured
    assert "running workflow evidence" in captured
    assert "source plan revision is stale" in captured


def test_render_task_dag_progress_dispatch_and_execute_smoke(capsys) -> None:
    progress_payload = {
        "workflow_summary": {"dispatched_nodes": 1},
        "readiness_summary": {},
        "progress": {
            "completed_percent": 50,
            "estimated_percent": 75,
            "completed_nodes": 2,
            "total_nodes": 4,
            "running_nodes": 1,
            "ready_nodes": 1,
            "blocked_nodes": 1,
            "next_action": "finish_review",
        },
        "blockers": [{"node_id": "D", "reason": "waiting for review"}],
    }
    dispatch_payload = {
        "mode": "create_tasks",
        "dispatch_scope": "ready_nodes",
        "workflow_summary": {
            "ready_nodes": 1,
            "dispatched_nodes": 1,
            "running_nodes": 0,
            "blocked_nodes": 1,
            "next_action": "dispatch_ready",
        },
        "readiness_summary": {},
        "execution_strategy": {
            "default_mode": "local_execution_dag_with_selective_promotion",
            "dispatch_model": "compact_packet",
            "recommended_worker_sessions": 1,
        },
        "created_tasks": [{"node_id": "B", "state": "created", "task_id": "RT-2", "title": "Ready node"}],
        "created_batch_sessions": [{"batch_id": "B1", "session_id": "S-1", "node_ids": ["B"], "title": "Batch 1"}],
        "dispatched": [{"node_id": "C", "state": "dispatched", "workflow_state": "planning", "task_id": "RT-3", "title": "Dispatched node"}],
        "skipped": [{"node_id": "D", "reason": "already linked"}],
    }
    execute_payload = {
        "mode": "record_run",
        "workflow_summary": {
            "ready_nodes": 1,
            "dispatched_nodes": 1,
            "running_nodes": 1,
            "blocked_nodes": 0,
            "completed_nodes": 2,
            "next_action": "resume_run",
        },
        "readiness_summary": {},
        "execute_run_contract": {
            "capability_stage": "scaffold_only",
            "auto_continue_supported": False,
            "final_remote_disposition_default": True,
            "current_boundary": "reviewable_output",
            "next_focus_node_id": "C",
            "next_focus_change_id": "RC-3",
        },
    }

    task_dag_views._render_task_dag_progress(progress_payload)
    task_dag_views._render_task_dag_dispatch(dispatch_payload)
    task_dag_views._render_task_dag_execute(execute_payload)
    captured = capsys.readouterr().out
    assert "DAG 50% complete" in captured
    assert "blockers" in captured
    assert "task DAG dispatch" in captured
    assert "compact-worker sessions" in captured
    assert "skipped" in captured
    assert "execute mode:" in captured
    assert "capability:" in captured
    assert "boundary:" in captured
    assert "focus cut:" in captured
    assert "RC-3" in captured
