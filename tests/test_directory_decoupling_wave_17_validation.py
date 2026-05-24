from __future__ import annotations

from pathlib import Path

from ait.plan_graph import load_task_graph
from ait.repo_paths import RepoContext


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
WAVE_17_MARKDOWN = AUTHORED_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_17.md"
WAVE_17_TASK_GRAPH = AUTHORED_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_17.task_graph.json"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_17_docs_remain_routable_after_wave_18_ship() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    wave_text = _read_text(WAVE_17_MARKDOWN)

    assert "./sprints/directory_decoupling_compact_dag_wave_17.md" in plan_text
    assert "./sprints/directory_decoupling_compact_dag_wave_18.md" in plan_text

    for needle in (
        "src/ait_agent/telegram/app.py",
        "src/ait/cli/commands/queue_workflow_land.py",
        "src/ait_server/read_models.py",
        "src/ait_server/server_content.py",
        "src/ait_web/rendering/theme.py",
        "local_first_final_local_land",
        "local-final DAG",
        "explicit local land gate",
    ):
        assert needle in wave_text


def test_wave_17_task_graph_stays_local_final_compact_packet_dag() -> None:
    graph = load_task_graph(WAVE_17_TASK_GRAPH)

    assert graph["graph_id"] == "directory-decoupling-wave-17/residual-hotspot-dispatch"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_17.md"
    assert graph["source_plan"]["plan_ref"] == "directory-decoupling-wave-17/root"
    assert graph["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert graph["execution_policy"]["dispatch_model"] == "compact_packet"
    assert graph["execution_policy"]["change_strategy"] == "local_first_final_local_land"
    assert graph["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert graph["execution_policy"]["local_first_final_land"] is True

    node_ids = [node["node_id"] for node in graph["nodes"]]
    assert node_ids == ["A", "B", "C", "D", "E", "F", "L"]

    converged = next(node for node in graph["nodes"] if node["node_id"] == "F")
    assert converged["workflow_boundary"] == "reviewable_output"
    assert converged["converged_output"] is True
    assert converged["depends_on"] == ["A", "B", "C", "D", "E"]
    assert "local-land" in converged["title"].lower()

    final_gate = next(node for node in graph["nodes"] if node["node_id"] == "L")
    assert final_gate["node_kind"] == "land_gate"
    assert final_gate["depends_on"] == ["F"]
    assert "local land" in final_gate["title"].lower()


def test_wave_17_supporting_artifacts_exist() -> None:
    for path in (
        WAVE_17_MARKDOWN,
        WAVE_17_TASK_GRAPH,
        WORKSPACE_ROOT / "tests/test_directory_decoupling_wave_17_validation.py",
    ):
        assert path.exists(), str(path)
