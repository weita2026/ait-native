from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_3_docs_route_to_shipped_hotspot_wave() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)

    assert "./sprints/directory_decoupling_compact_dag_wave_3.md" in plan_text
    assert "shipped wave-3 hotspot seams" in plan_text
    assert "src/ait/cli/workflow_land_sync.py" in plan_text
    assert "src/ait_agent/telegram/graph_watches.py" in plan_text
    assert "src/ait_agent/telegram/session_views.py" in plan_text
    assert "src/ait/store_local_views.py" in plan_text
    assert "src/ait/store_worktree_metadata.py" in plan_text
    assert "tests/test_directory_decoupling_wave_3_validation.py" in plan_text

    assert "src/ait/store_local_views.py" in ownership_text
    assert "src/ait/store_worktree_metadata.py" in ownership_text
    assert "src/ait/cli/workflow_land_sync.py" in ownership_text
    assert "src/ait_agent/telegram/session_views.py" in ownership_text
    assert "src/ait_agent/telegram/graph_watches.py" in ownership_text


def test_wave_3_hotspot_facades_import_extracted_modules() -> None:
    cli_app_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/app.py")
    telegram_app_text = _read_text(WORKSPACE_ROOT / "src/ait_agent/telegram/app.py")
    store_text = _read_text(WORKSPACE_ROOT / "src/ait/store.py")
    store_local_workflow_text = _read_text(WORKSPACE_ROOT / "src/ait/store_local_workflow.py")
    store_worktrees_text = _read_text(WORKSPACE_ROOT / "src/ait/store_worktrees.py")
    store_worktree_cleanup_text = _read_text(WORKSPACE_ROOT / "src/ait/store_worktree_cleanup.py")

    assert "from .workflow_land_sync import (" in cli_app_text
    assert "from .graph_watches import (" in telegram_app_text
    assert "from .session_views import (" in telegram_app_text
    assert "from .store_local_views import (" in store_text
    assert "from .store_local_views import (" in store_local_workflow_text
    assert "from .store_worktree_cleanup import (" in store_worktrees_text
    assert "from .store_worktree_metadata import (" in store_worktree_cleanup_text


def test_wave_3_supporting_modules_exist() -> None:
    required_paths = (
        "src/ait/cli/workflow_land_sync.py",
        "src/ait_agent/telegram/graph_watches.py",
        "src/ait_agent/telegram/session_views.py",
        "src/ait/store_local_views.py",
        "src/ait/store_worktree_metadata.py",
        "tests/cli/test_workflow_land_sync.py",
        "tests/test_directory_decoupling_wave_3_validation.py",
        "tests/test_directory_split_packages.py",
        "tests/test_task_dag_plan_graph.py",
    )
    for relative in required_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative
