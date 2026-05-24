from __future__ import annotations

import copy
import json
from pathlib import Path

import pytest

from ait.plan_graph import (
    TaskGraphValidationError,
    build_task_graph_progress,
    load_task_graph,
    topological_node_order,
    validate_task_graph,
)


REPO_ROOT = Path(__file__).resolve().parents[1]
CANONICAL_GRAPH_PATH = REPO_ROOT / "docs/sprints/task_dag_scheduler.task_graph.example.json"
MVP_GRAPH_PATH = REPO_ROOT / "docs/sprints/task_dag_scheduler_parallel_execution.task_graph.json"


def _resolve_in_repo_tree(relative: str) -> Path | None:
    for root in [REPO_ROOT, *REPO_ROOT.parents]:
        candidate = root / relative
        if candidate.exists():
            return candidate
    return None


RESIDUAL_CROSS_ROOT_GRAPH_PATH = _resolve_in_repo_tree(
    "docs/sprints/residual_cross_root_seam_compact_dag_execution.task_graph.json"
)
DIRECTORY_DECOUPLING_WAVE_3_GRAPH_PATH = _resolve_in_repo_tree(
    "docs/sprints/directory_decoupling_compact_dag_wave_3.task_graph.json"
)
DIRECTORY_DECOUPLING_WAVE_8_GRAPH_PATH = _resolve_in_repo_tree(
    "docs/sprints/directory_decoupling_compact_dag_wave_8.task_graph.json"
)
DIRECTORY_DECOUPLING_WAVE_10_GRAPH_PATH = _resolve_in_repo_tree(
    "docs/sprints/directory_decoupling_compact_dag_wave_10.task_graph.json"
)
DIRECTORY_DECOUPLING_WAVE_14_GRAPH_PATH = _resolve_in_repo_tree(
    "docs/sprints/directory_decoupling_compact_dag_wave_14.task_graph.json"
)


def _load_raw_graph(path: Path = MVP_GRAPH_PATH) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def test_load_task_graph_fixtures_and_topological_order():
    expected_orders = {
        CANONICAL_GRAPH_PATH: ["A", "B", "C", "D", "L"],
        MVP_GRAPH_PATH: ["A", "B", "C", "D", "E", "F", "G"],
    }

    for path, expected_order in expected_orders.items():
        data = load_task_graph(path)

        assert topological_node_order(data) == expected_order


def test_validate_task_graph_rejects_duplicate_node_ids():
    data = _load_raw_graph()
    duplicate = copy.deepcopy(data["nodes"][0])
    data["nodes"].append(duplicate)

    with pytest.raises(TaskGraphValidationError, match="Duplicate task graph node_id"):
        validate_task_graph(data)


def test_validate_task_graph_rejects_unknown_edge_endpoints():
    data = _load_raw_graph()
    data["edges"].append({"from": "A", "to": "missing-node", "edge_kind": "depends_on"})

    with pytest.raises(TaskGraphValidationError, match="unknown target node: missing-node"):
        validate_task_graph(data)


def test_validate_task_graph_rejects_cycles():
    data = _load_raw_graph()
    data["edges"].append({"from": "G", "to": "A", "edge_kind": "depends_on"})

    with pytest.raises(TaskGraphValidationError, match="cycle"):
        validate_task_graph(data)


def test_validate_task_graph_rejects_parallel_lock_conflicts():
    data = _load_raw_graph()
    nodes = {node["node_id"]: node for node in data["nodes"]}
    nodes["C"]["lock_keys"] = ["module:ait_server.read_models"]
    nodes["D"]["lock_keys"] = ["module:ait_server.read_models"]

    with pytest.raises(TaskGraphValidationError, match="lock conflict.*module:ait_server.read_models.*C.*D"):
        validate_task_graph(data)

    nodes["D"]["depends_on"].append("C")
    data["edges"].append({"from": "C", "to": "D", "edge_kind": "depends_on"})
    validate_task_graph(data)


def test_validate_task_graph_rejects_task_node_missing_template():
    data = _load_raw_graph()
    task_node = next(node for node in data["nodes"] if node["node_kind"] == "task")
    task_node.pop("task_template")

    with pytest.raises(TaskGraphValidationError, match="must include task_template"):
        validate_task_graph(data)


def test_validate_task_graph_rejects_unknown_dependency_and_bad_node_metadata():
    data = _load_raw_graph()
    data["nodes"][0]["depends_on"] = ["missing"]

    with pytest.raises(TaskGraphValidationError, match="depends on unknown node"):
        validate_task_graph(data)

    data = _load_raw_graph()
    data["nodes"][0]["node_kind"] = "window"
    with pytest.raises(TaskGraphValidationError, match="unsupported node_kind"):
        validate_task_graph(data)

    data = _load_raw_graph()
    data["nodes"][0]["hotspot_keys"] = ["unknown-lock"]
    with pytest.raises(TaskGraphValidationError, match="hotspot key must include a prefix"):
        validate_task_graph(data)

    data = _load_raw_graph()
    data["nodes"][0]["task_template"]["title"] = ""
    with pytest.raises(TaskGraphValidationError, match="task_template must include non-empty title"):
        validate_task_graph(data)

    data = _load_raw_graph()
    data["nodes"][0]["workflow_boundary"] = "mystery"
    with pytest.raises(TaskGraphValidationError, match="workflow_boundary must be one of"):
        validate_task_graph(data)

    data = _load_raw_graph()
    data["nodes"][0]["converged_output"] = "yes"
    with pytest.raises(TaskGraphValidationError, match="converged_output must be a boolean"):
        validate_task_graph(data)

    data = _load_raw_graph()
    data["execution_policy"]["solo_gate_strategy"] = "after_party"
    with pytest.raises(TaskGraphValidationError, match="solo_gate_strategy must be one of"):
        validate_task_graph(data)

    data = _load_raw_graph()
    data["execution_policy"]["final_gate_bundle"] = ["review", "celebrate"]
    with pytest.raises(TaskGraphValidationError, match="final_gate_bundle entries must be one of"):
        validate_task_graph(data)

    data = _load_raw_graph()
    data["execution_policy"]["mode"] = "advisory_first"
    with pytest.raises(TaskGraphValidationError, match="execution_policy.mode must be one of"):
        validate_task_graph(data)


def test_validate_task_graph_rejects_non_final_reviewable_nodes():
    data = _load_raw_graph()
    for node in data["nodes"]:
        node.pop("converged_output", None)
    data["nodes"][0]["workflow_boundary"] = "reviewable_output"
    data["nodes"][-1]["workflow_boundary"] = "reviewable_output"
    data["nodes"][-1]["converged_output"] = True

    with pytest.raises(TaskGraphValidationError, match="is not the converged output and cannot declare workflow_boundary=reviewable_output"):
        validate_task_graph(data)

    data = _load_raw_graph()
    for node in data["nodes"]:
        node.pop("converged_output", None)
    data["edges"] = data["edges"][:-1]

    with pytest.raises(TaskGraphValidationError, match="one terminal task node"):
        validate_task_graph(data)


def test_validate_task_graph_accepts_guarded_full_dag_metadata():
    data = _load_raw_graph()
    data["nodes"][0]["workflow_boundary"] = "execution_only"
    data["nodes"][-1]["workflow_boundary"] = "reviewable_output"
    data["nodes"][-1]["converged_output"] = True
    data["nodes"][-1]["safety_boundary"] = False
    data["execution_policy"]["solo_gate_strategy"] = "end_of_dag_gate_concentration"
    data["execution_policy"]["final_gate_bundle"] = ["review", "attestation", "policy", "land"]

    validate_task_graph(data)


def test_validate_task_graph_accepts_worker_only_compact_packet_metadata():
    data = _load_raw_graph()
    assert data["execution_policy"]["mode"] == "guarded_full_dag_convergence"

    validate_task_graph(data)


def test_validate_task_graph_accepts_local_final_land_change_strategy():
    data = _load_raw_graph()
    data["execution_policy"]["change_strategy"] = "local_first_final_local_land"

    validate_task_graph(data)


@pytest.mark.parametrize(
    ("mutator", "message"),
    [
        (
            lambda data: data["execution_policy"].pop("worker_execution_mode"),
            "worker_execution_mode is required when execution_policy.mode is guarded_full_dag_convergence",
        ),
    ],
)
def test_validate_task_graph_requires_explicit_guarded_full_dag_worker_execution_mode(mutator, message):
    data = _load_raw_graph()
    mutator(data)

    with pytest.raises(TaskGraphValidationError, match=message):
        validate_task_graph(data)


def test_cross_root_graph_keeps_autonomy_ready_converged_output_contract():
    if RESIDUAL_CROSS_ROOT_GRAPH_PATH is None:
        pytest.skip("docs/sprints task-graph fixtures are not materialized in execution snapshots")
    data = load_task_graph(RESIDUAL_CROSS_ROOT_GRAPH_PATH)
    node_map = {node["node_id"]: node for node in data["nodes"]}

    assert data["source_plan"]["artifact_path"] == "docs/sprints/residual_cross_root_seam_compact_dag_execution.md"
    assert topological_node_order(data) == ["A", "B", "C", "D", "E", "F", "G", "H", "L"]
    assert data["execution_policy"]["solo_gate_strategy"] == "end_of_dag_gate_concentration"
    assert data["execution_policy"]["final_gate_bundle"] == [
        "review",
        "attestation",
        "policy",
        "land",
    ]
    assert data["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert data["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert {node_id for node_id, node in node_map.items() if node.get("workflow_boundary") == "execution_only"} == {
        "A",
        "B",
        "C",
        "D",
        "E",
        "F",
        "G",
    }
    assert node_map["H"]["workflow_boundary"] == "reviewable_output"
    assert node_map["H"]["converged_output"] is True
    assert node_map["L"]["node_kind"] == "land_gate"
    assert "Explicit remote land" in node_map["L"]["safety_boundary_reason"]


def test_directory_decoupling_wave_3_graph_keeps_worker_only_converged_output_contract():
    if DIRECTORY_DECOUPLING_WAVE_3_GRAPH_PATH is None:
        pytest.skip("docs/sprints task-graph fixtures are not materialized in execution snapshots")
    data = load_task_graph(DIRECTORY_DECOUPLING_WAVE_3_GRAPH_PATH)
    node_map = {node["node_id"]: node for node in data["nodes"]}

    assert data["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_3.md"
    assert topological_node_order(data) == ["A", "B", "C", "D", "L"]
    assert data["execution_policy"]["solo_gate_strategy"] == "end_of_dag_gate_concentration"
    assert data["execution_policy"]["final_gate_bundle"] == [
        "review",
        "attestation",
        "policy",
        "land",
    ]
    assert data["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert data["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert {node_id for node_id, node in node_map.items() if node.get("workflow_boundary") == "execution_only"} == {
        "A",
        "B",
        "C",
    }
    assert node_map["D"]["workflow_boundary"] == "reviewable_output"
    assert node_map["D"]["converged_output"] is True
    assert node_map["L"]["node_kind"] == "land_gate"
    assert "Explicit remote land" in node_map["L"]["safety_boundary_reason"]


def test_directory_decoupling_wave_8_graph_keeps_worker_only_converged_output_contract():
    if DIRECTORY_DECOUPLING_WAVE_8_GRAPH_PATH is None:
        pytest.skip("docs/sprints task-graph fixtures are not materialized in execution snapshots")
    data = load_task_graph(DIRECTORY_DECOUPLING_WAVE_8_GRAPH_PATH)
    node_map = {node["node_id"]: node for node in data["nodes"]}

    assert data["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_8.md"
    assert topological_node_order(data) == ["A", "B", "C", "D", "E", "L"]
    assert data["execution_policy"]["solo_gate_strategy"] == "end_of_dag_gate_concentration"
    assert data["execution_policy"]["final_gate_bundle"] == [
        "review",
        "attestation",
        "policy",
        "land",
    ]
    assert data["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert data["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert {node_id for node_id, node in node_map.items() if node.get("workflow_boundary") == "execution_only"} == {
        "A",
        "B",
        "C",
        "D",
    }
    assert node_map["E"]["workflow_boundary"] == "reviewable_output"
    assert node_map["E"]["converged_output"] is True
    assert node_map["L"]["node_kind"] == "land_gate"
    assert "Explicit remote land" in node_map["L"]["safety_boundary_reason"]


def test_directory_decoupling_wave_10_graph_keeps_worker_only_compact_follow_up_contract():
    if DIRECTORY_DECOUPLING_WAVE_10_GRAPH_PATH is None:
        pytest.skip("docs/sprints task-graph fixtures are not materialized in execution snapshots")
    data = load_task_graph(DIRECTORY_DECOUPLING_WAVE_10_GRAPH_PATH)
    node_map = {node["node_id"]: node for node in data["nodes"]}

    assert data["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_10.md"
    assert topological_node_order(data) == ["A", "B", "C", "D", "E", "F", "L"]
    assert data["execution_policy"]["gate_strategy"] == "end_of_dag_gate_concentration"
    assert data["execution_policy"]["change_strategy"] == "local_first_final_remote_land"
    assert data["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert data["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert {node_id for node_id, node in node_map.items() if node.get("workflow_boundary") == "execution_only"} == {
        "A",
        "B",
        "C",
        "D",
        "E",
    }
    assert node_map["F"]["workflow_boundary"] == "reviewable_output"
    assert node_map["F"]["converged_output"] is True
    assert node_map["L"]["node_kind"] == "land_gate"
    assert "Explicit remote land" in node_map["L"]["safety_boundary_reason"]


def test_directory_decoupling_wave_14_graph_keeps_worker_only_local_land_contract():
    if DIRECTORY_DECOUPLING_WAVE_14_GRAPH_PATH is None:
        pytest.skip("docs/sprints task-graph fixtures are not materialized in execution snapshots")
    data = load_task_graph(DIRECTORY_DECOUPLING_WAVE_14_GRAPH_PATH)
    node_map = {node["node_id"]: node for node in data["nodes"]}

    assert data["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_14.md"
    assert topological_node_order(data) == ["A", "B", "C", "D", "E", "F", "L"]
    assert data["execution_policy"]["change_strategy"] == "local_first_final_local_land"
    assert data["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert data["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert node_map["F"]["workflow_boundary"] == "reviewable_output"
    assert node_map["F"]["converged_output"] is True
    assert node_map["L"]["node_kind"] == "land_gate"
    assert "local land" in str(node_map["L"]["title"]).lower()
    assert "local land" in str(node_map["L"]["completion_rule"]).lower()


@pytest.mark.parametrize(
    ("mutator", "message"),
    [
        (
            lambda data: data["execution_policy"].__setitem__("worker_execution_mode", "legacy_batch"),
            "execution_policy.worker_execution_mode must be one of",
        ),
        (
            lambda data: data["execution_policy"].__setitem__("session_topology_default", "legacy_batch"),
            "execution_policy.session_topology_default is retired",
        ),
        (
            lambda data: data["execution_policy"].__setitem__("coordinator_token_budget", 200),
            "execution_policy.coordinator_token_budget is retired",
        ),
    ],
)
def test_validate_task_graph_rejects_retired_or_unknown_worker_execution_metadata(mutator, message):
    data = _load_raw_graph()
    mutator(data)

    with pytest.raises(TaskGraphValidationError, match=message):
        validate_task_graph(data)


@pytest.mark.parametrize(
    "field",
    ["schema_version", "graph_id", "source_plan", "nodes", "edges", "execution_policy"],
)
def test_validate_task_graph_rejects_missing_required_top_level_field(field: str):
    data = _load_raw_graph()
    data.pop(field)

    with pytest.raises(TaskGraphValidationError, match=f"missing required field.*{field}"):
        validate_task_graph(data)


def test_build_task_graph_progress_uses_completed_weight_for_stable_percent():
    data = load_task_graph(MVP_GRAPH_PATH)

    progress = build_task_graph_progress(
        data,
        {
            "A": "completed",
            "B": "landed",
            "C": "running",
            "D": "review",
            "E": "landable",
            "F": "ready",
            "G": {"state": "blocked", "reason": "dependency C is not landed"},
        },
    )

    assert progress["completed_percent"] == 28
    assert progress["estimated_percent"] == 55
    assert progress["completed_nodes"] == 2
    assert progress["running_nodes"] == 3
    assert progress["ready_nodes"] == 1
    assert progress["blocked_nodes"] == 1
    assert progress["total_nodes"] == 7
    assert progress["total_weight"] == 7
    assert progress["next_action"] == "start F"
    assert progress["node_states"]["D"]["estimated_fraction"] == 0.65


def test_build_task_graph_progress_keeps_zero_weight_gate_nodes_out_of_percent_denominator():
    data = load_task_graph(CANONICAL_GRAPH_PATH)

    progress = build_task_graph_progress(
        data,
        {
            "A": "completed",
            "B": "completed",
            "C": "ready",
            "D": "blocked",
            "L": "landed",
        },
    )

    assert progress["completed_percent"] == 33
    assert progress["estimated_percent"] is None
    assert progress["total_weight"] == 3
    assert progress["completed_weight"] == 1
    assert progress["total_nodes"] == 5
    assert progress["node_states"]["A"]["progress_weight"] == 0
    assert progress["node_states"]["L"]["progress_weight"] == 0


def test_build_task_graph_progress_does_not_count_open_session_as_complete():
    data = load_task_graph(MVP_GRAPH_PATH)

    progress = build_task_graph_progress(data, {"A": {"session_id": "S-1", "session_state": "open"}})

    assert progress["completed_percent"] == 0
    assert progress["estimated_percent"] == 5
    assert progress["running_nodes"] == 1
    assert progress["node_states"]["A"]["state"] == "running"


def test_build_task_graph_progress_requires_supersession_before_canceled_node_is_terminal():
    data = load_task_graph(MVP_GRAPH_PATH)

    blocked = build_task_graph_progress(data, {"A": {"state": "canceled"}})
    superseded = build_task_graph_progress(data, {"A": {"state": "canceled", "superseded_by_node_id": "B"}})

    assert blocked["completed_percent"] == 0
    assert blocked["blocked_nodes"] == 7
    assert blocked["node_states"]["A"]["state"] == "blocked"
    assert blocked["node_states"]["A"]["reason"] == "canceled_without_supersession"
    assert superseded["completed_percent"] == 14
    assert superseded["completed_nodes"] == 1
    assert superseded["node_states"]["A"]["state"] == "superseded"


def test_build_task_graph_progress_rejects_unsupported_states_and_bad_weights():
    data = _load_raw_graph()

    with pytest.raises(TaskGraphValidationError, match="unsupported progress state"):
        build_task_graph_progress(data, {"A": "session_open"})

    data["nodes"][0]["progress_weight"] = -1
    with pytest.raises(TaskGraphValidationError, match="progress_weight must be non-negative"):
        build_task_graph_progress(data, {"A": "completed"})
