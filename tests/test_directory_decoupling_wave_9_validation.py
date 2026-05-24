from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_9_docs_route_to_physical_fanout_wave() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)

    assert "./sprints/directory_decoupling_compact_dag_wave_9.md" in plan_text
    assert "src/ait/cli/init_command.py" in plan_text
    assert "src/ait_agent/telegram/workflow_queries.py" in plan_text
    assert "src/ait_server/server_process_runtime.py" in plan_text
    assert "src/ait_server/store/land_request_payloads.py" in plan_text
    assert "src/ait/store_worktree_bindings.py" in plan_text
    assert "tests/test_directory_decoupling_wave_9_validation.py" in plan_text

    assert "src/ait/cli/init_command.py" in ownership_text
    assert "src/ait_agent/telegram/workflow_queries.py" in ownership_text
    assert "src/ait_server/server_process_runtime.py" in ownership_text
    assert "src/ait_server/store/land_request_payloads.py" in ownership_text
    assert "src/ait/store_worktree_bindings.py" in ownership_text


def test_wave_9_supporting_artifacts_exist() -> None:
    required_workspace_paths = (
        "src/ait/cli/init_command.py",
        "src/ait_agent/telegram/workflow_queries.py",
        "src/ait_server/server_process_runtime.py",
        "src/ait_server/store/land_request_payloads.py",
        "src/ait/store_worktree_bindings.py",
        "tests/test_directory_split_packages.py",
        "tests/test_directory_decoupling_wave_9_validation.py",
    )
    for relative in required_workspace_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative

    required_authored_paths = (
        "docs/ait_directory_structure_decoupling_plan.md",
        "docs/ait_module_ownership_map.md",
        "docs/sprints/directory_decoupling_compact_dag_wave_9.md",
        "docs/sprints/directory_decoupling_compact_dag_wave_9.task_graph.json",
    )
    for relative in required_authored_paths:
        assert (AUTHORED_ROOT / relative).exists(), relative
