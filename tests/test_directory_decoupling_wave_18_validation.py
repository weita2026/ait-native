from __future__ import annotations

from pathlib import Path

from ait.plan_graph import load_task_graph
from ait.repo_paths import RepoContext


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
WAVE_18_MARKDOWN = AUTHORED_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_18.md"
WAVE_18_TASK_GRAPH = AUTHORED_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_18.task_graph.json"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_18_docs_route_to_latest_shipped_boundary_batch() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    wave_text = _read_text(WAVE_18_MARKDOWN)

    assert (
        "- For the shipped residual boundary batch after the shipped wave-17 follow-up,"
    ) in plan_text
    assert (
        "[Directory Decoupling Compact DAG Wave 18]"
        "(./sprints/directory_decoupling_compact_dag_wave_18.md)"
    ) in plan_text
    assert "./sprints/directory_decoupling_compact_dag_wave_18.md" in plan_text
    assert (
        "| Latest shipped internal hotspot wave | "
        "[Directory Decoupling Compact DAG Wave 22]"
        "(./sprints/directory_decoupling_compact_dag_wave_22.md) |"
    ) in plan_text
    assert (
        "| Latest internal hotspot DAG artifact | "
        "[Directory Decoupling Compact DAG Wave 23]"
        "(./sprints/directory_decoupling_compact_dag_wave_23.md) |"
    ) in plan_text
    assert "Status: completed sprint artifact" in wave_text
    assert "LT-1283 /" in wave_text
    assert "LC-1091" in wave_text

    for needle in (
        "src/ait_server/server_store.py",
        "src/ait_server/store/*",
        "src/ait_web/agent_transport_runtime.py",
        "ait_agent.telegram.app",
        "src/ait_server/app.py",
        "src/ait_agent/telegram/app.py",
        "src/ait_agent/discord/app.py",
        "src/ait/cli/task_dag_telegram_watch.py",
        "local_first_final_local_land",
        "explicit local land gate",
    ):
        assert needle in wave_text


def test_wave_18_task_graph_stays_local_final_compact_packet_dag() -> None:
    graph = load_task_graph(WAVE_18_TASK_GRAPH)

    assert graph["graph_id"] == "directory-decoupling-wave-18/residual-boundary-dispatch"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_18.md"
    assert graph["source_plan"]["plan_ref"] == "directory-decoupling-wave-18/root"
    assert graph["source_plan"]["plan_id"] == "PL-01KSCERN3K05ZB9WWHF0C7D0A2"
    assert graph["source_plan"]["plan_revision_id"].startswith("PR-")
    assert graph["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert graph["execution_policy"]["dispatch_model"] == "compact_packet"
    assert graph["execution_policy"]["change_strategy"] == "local_first_final_local_land"
    assert graph["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert graph["execution_policy"]["local_first_final_land"] is True

    node_ids = [node["node_id"] for node in graph["nodes"]]
    assert node_ids == ["A", "B", "C", "D", "E", "F", "L"]

    expected_refs = {
        "directory-decoupling-wave-18/server-store-boundary",
        "directory-decoupling-wave-18/web-agent-seam",
        "directory-decoupling-wave-18/server-app-route-split",
        "directory-decoupling-wave-18/agent-transport-hotspot",
        "directory-decoupling-wave-18/local-cli-telegram-watch-seam",
        "directory-decoupling-wave-18/verification-land",
    }
    assert {node.get("plan_item_ref") for node in graph["nodes"] if node["node_kind"] == "task"} == expected_refs

    converged = next(node for node in graph["nodes"] if node["node_id"] == "F")
    assert converged["workflow_boundary"] == "reviewable_output"
    assert converged["converged_output"] is True
    assert converged["depends_on"] == ["A", "B", "C", "D", "E"]
    assert "local-land" in converged["title"].lower()

    final_gate = next(node for node in graph["nodes"] if node["node_id"] == "L")
    assert final_gate["node_kind"] == "land_gate"
    assert final_gate["depends_on"] == ["F"]
    assert "local land" in final_gate["title"].lower()


def test_wave_18_supporting_artifacts_exist() -> None:
    for path in (
        WAVE_18_MARKDOWN,
        WAVE_18_TASK_GRAPH,
        WORKSPACE_ROOT / "tests/test_directory_decoupling_wave_18_validation.py",
    ):
        assert path.exists(), str(path)
