from __future__ import annotations

from pathlib import Path

from ait.plan_graph import load_task_graph
from ait.repo_paths import RepoContext


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"
WAVE_23_MARKDOWN = AUTHORED_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_23.md"
WAVE_23_TASK_GRAPH = AUTHORED_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_23.task_graph.json"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_23_docs_route_to_converged_cli_server_content_batch() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)
    wave_text = _read_text(WAVE_23_MARKDOWN)

    assert (
        "| Latest internal hotspot DAG artifact | "
        "[Directory Decoupling Compact DAG Wave 23]"
        "(./sprints/directory_decoupling_compact_dag_wave_23.md) |"
    ) in plan_text
    assert "./sprints/directory_decoupling_compact_dag_wave_23.md" in plan_text

    for needle in (
        "src/ait/cli/app_surfaces.py",
        "src/ait/cli/commands/bootstrap.py",
        "src/ait_server/task_dag_route_helpers.py",
        "src/ait_server/session_route_helpers.py",
        "src/ait_server/server_content_groups.py",
    ):
        assert needle in ownership_text

    for needle in (
        "Converge docs, guard tests, cross-root seam checks, and remote land for the batch.",
        "src/ait/cli/app.py",
        "src/ait_server/app.py",
        "src/ait_server/server_content.py",
        "review, attestation, policy, and remote",
    ):
        assert needle in wave_text


def test_wave_23_task_graph_stays_reviewable_output_remote_land_dag() -> None:
    graph = load_task_graph(WAVE_23_TASK_GRAPH)

    assert graph["graph_id"] == "directory-decoupling-wave-23/cli-server-app-content-dispatch"
    assert graph["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_23.md"
    assert graph["source_plan"]["plan_ref"] == "directory-decoupling-wave-23/root"
    assert graph["source_plan"]["plan_id"] == "PL-01KSM5P6W69BYZ4NBBNYY5CR02"
    assert graph["source_plan"]["plan_revision_id"] == "PR-01KSM5P6W6M31CV53SFPP9Q3V3"
    assert graph["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert graph["execution_policy"]["dispatch_model"] == "compact_packet"
    assert graph["execution_policy"]["change_strategy"] == "local_first_final_remote_land"
    assert graph["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"

    assert [node["node_id"] for node in graph["nodes"]] == ["A", "B", "C", "E", "L"]
    assert {
        node.get("plan_item_ref")
        for node in graph["nodes"]
        if node["node_kind"] == "task"
    } == {
        "directory-decoupling-wave-23/cli-app-follow-up",
        "directory-decoupling-wave-23/server-app-follow-up",
        "directory-decoupling-wave-23/server-content-follow-up",
        "directory-decoupling-wave-23/verification-land",
    }

    converged = next(node for node in graph["nodes"] if node["node_id"] == "E")
    assert converged["workflow_boundary"] == "reviewable_output"
    assert converged["converged_output"] is True
    assert converged["depends_on"] == ["A", "B", "C"]
    assert converged["hotspot_keys"] == ["dir:docs", "dir:tests", "contract:directory-decoupling-wave-23"]

    final_gate = next(node for node in graph["nodes"] if node["node_id"] == "L")
    assert final_gate["node_kind"] == "land_gate"
    assert final_gate["depends_on"] == ["E"]
    assert "remote land" in final_gate["title"].lower()


def test_wave_23_hotspot_facades_keep_extracted_boundaries() -> None:
    cli_app_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/app.py")
    cli_app_surfaces_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/app_surfaces.py")
    line_transport_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/line_transport_helpers.py")
    server_app_text = _read_text(WORKSPACE_ROOT / "src/ait_server/app.py")
    task_dag_helper_text = _read_text(WORKSPACE_ROOT / "src/ait_server/task_dag_route_helpers.py")
    session_helper_text = _read_text(WORKSPACE_ROOT / "src/ait_server/session_route_helpers.py")
    server_content_text = _read_text(WORKSPACE_ROOT / "src/ait_server/server_content.py")
    server_content_groups_text = _read_text(WORKSPACE_ROOT / "src/ait_server/server_content_groups.py")

    assert "from .commands.bootstrap import bootstrap_cli_commands" in cli_app_text
    assert "from .app_surfaces import (" in cli_app_text
    assert "from .line_transport_helpers import _pull_line" in cli_app_text
    assert "def register_cli_subapps(" not in cli_app_text
    assert "def _ctx(" not in cli_app_text
    assert "def _pull_line(" not in cli_app_text
    assert "def register_cli_subapps(" in cli_app_surfaces_text
    assert "def _ctx(" in cli_app_surfaces_text
    assert "def _pull_line(" in line_transport_text

    assert "from .task_dag_route_helpers import (" in server_app_text
    assert "from .session_route_helpers import (" in server_app_text
    assert "def _schedule_task_dag_notification(" not in server_app_text
    assert "def _task_dag_progress_summary_for_turn_impl(" not in server_app_text
    assert "def _session_with_telegram_runtime_state(" not in server_app_text
    assert "def _reply_generation_config(" not in server_app_text
    assert "def _schedule_task_dag_notification(" in task_dag_helper_text
    assert "def _task_dag_progress_summary_for_turn_impl(" in task_dag_helper_text
    assert "def _session_with_telegram_runtime_state(" in session_helper_text
    assert "def _reply_generation_config(" in session_helper_text

    assert "from .server_content_groups import (" in server_content_text
    assert "def list_repository_groups(" not in server_content_text
    assert "def create_repository_group(" not in server_content_text
    assert "def replace_repository_group_layout(" not in server_content_text
    assert "def list_repository_groups(" in server_content_groups_text
    assert "def create_repository_group(" in server_content_groups_text
    assert "def replace_repository_group_layout(" in server_content_groups_text


def test_wave_23_supporting_artifacts_exist() -> None:
    required_paths = (
        DECOUPLING_PLAN,
        OWNERSHIP_MAP,
        WAVE_23_MARKDOWN,
        WAVE_23_TASK_GRAPH,
        WORKSPACE_ROOT / "src/ait/cli/app_surfaces.py",
        WORKSPACE_ROOT / "src/ait/cli/commands/bootstrap.py",
        WORKSPACE_ROOT / "src/ait/cli/line_transport_helpers.py",
        WORKSPACE_ROOT / "src/ait_server/task_dag_route_helpers.py",
        WORKSPACE_ROOT / "src/ait_server/session_route_helpers.py",
        WORKSPACE_ROOT / "src/ait_server/server_content_groups.py",
        WORKSPACE_ROOT / "tests/test_cross_root_decoupling_contract.py",
        WORKSPACE_ROOT / "tests/test_directory_split_packages.py",
        WORKSPACE_ROOT / "tests/test_directory_decoupling_wave_23_validation.py",
    )
    for path in required_paths:
        assert path.exists(), str(path)
