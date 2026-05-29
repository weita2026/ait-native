from __future__ import annotations

import json
from pathlib import Path
from types import SimpleNamespace

from ait import local_control
from ait.repo_paths import RepoContext
from ait_server.read_models_domains import task_dag as task_dag_read_models
from ait_protocol.common import connect_sqlite


CONTRACT_PATH = Path("docs/snapshot_lineage_boundary_contract.json")


def _repo_context(tmp_path: Path) -> RepoContext:
    ait_dir = tmp_path / ".ait"
    ait_dir.mkdir()
    return RepoContext(
        root=tmp_path,
        ait_dir=ait_dir,
        content_db_path=ait_dir / "content.db",
        control_db_path=ait_dir / "control.db",
        config_path=ait_dir / "config.json",
    )


def _table_columns(conn, table_name: str) -> set[str]:
    rows = conn.execute(f"pragma table_info({table_name})").fetchall()
    return {str(row["name"]) for row in rows if row["name"] is not None}


def test_snapshot_lineage_boundary_contract_shape() -> None:
    payload = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))

    assert payload["schema_version"] == 1
    assert payload["workflow_change_lineage"]["storage"] == "workflow_changes"
    assert payload["dag_execution_evidence"]["event_types"] == [
        "task_graph.node_local_progress",
        "task_graph.node_completed",
    ]

    workflow_fields = set(payload["workflow_change_lineage"]["durable_snapshot_fields"])
    dag_fields = set(payload["dag_execution_evidence"]["snapshot_fields"])

    assert {"fork_snapshot_id", "pre_land_target_snapshot_id", "landed_snapshot_id"} <= workflow_fields
    assert {"completion_snapshot_id", "completion_fork_snapshot_id"} <= dag_fields
    assert workflow_fields.isdisjoint(dag_fields)


def test_snapshot_lineage_boundary_contract_matches_workflow_change_schema(tmp_path: Path) -> None:
    payload = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    ctx = _repo_context(tmp_path)
    local_control.initialize(ctx, repo_name="repo-a", default_line="main")

    conn = connect_sqlite(ctx.control_db_path)
    try:
        workflow_change_columns = _table_columns(conn, "workflow_changes")
    finally:
        conn.close()

    workflow_fields = set(payload["workflow_change_lineage"]["durable_snapshot_fields"])
    forbidden_fields = set(payload["workflow_change_lineage"]["forbidden_dag_completion_fields"])

    assert workflow_fields <= workflow_change_columns
    assert forbidden_fields.isdisjoint(workflow_change_columns)


def test_snapshot_lineage_boundary_contract_matches_task_dag_node_state_projection(
    monkeypatch,
) -> None:
    payload = json.loads(CONTRACT_PATH.read_text(encoding="utf-8"))
    sessions = [
        {
            "session_id": "S-GRAPH",
            "session_kind": "task_graph_run",
            "metadata": {"graph_run_id": "RUN-1"},
        }
    ]
    events = {
        "S-GRAPH": [
            {
                "sequence": 1,
                "event_type": "task_graph.node_completed",
                "payload": {
                    "node_id": "A",
                    "task_id": "RT-100",
                    "change_id": "RC-100",
                    "summary": "Node A completed.",
                    "completion_snapshot_id": "SNP-A",
                    "completion_fork_snapshot_id": "SNP-base-A",
                    "completion_line_name": "feature/rt-100",
                    "completion_worktree_name": "rt-100",
                },
                "created_at": "2026-01-01T00:00:01Z",
            }
        ]
    }
    monkeypatch.setattr(task_dag_read_models, "list_session_events", lambda _ctx, session_id: events[session_id])

    rows = task_dag_read_models._task_dag_node_state_rows_from_sessions(SimpleNamespace(), sessions)

    assert len(rows) == 1
    row = rows[0]
    for field in payload["dag_execution_evidence"]["snapshot_fields"]:
        assert row[field]
    for field in payload["dag_execution_evidence"]["context_fields"]:
        assert row[field]
    assert row["completion_snapshot_id"] == "SNP-A"
    assert row["completion_fork_snapshot_id"] == "SNP-base-A"
