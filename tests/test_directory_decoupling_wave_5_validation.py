from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_5_docs_route_to_shipped_hotspot_wave() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)

    assert "shipped wave-5 hotspot seams" in plan_text
    assert "src/ait_server/store/task_tracking.py" in plan_text
    assert "src/ait/store_repo_config.py" in plan_text
    assert "src/ait/store_worktree_layout.py" in plan_text
    assert "tests/test_directory_decoupling_wave_5_validation.py" in plan_text

    assert "src/ait_server/store/task_tracking.py" in ownership_text
    assert "src/ait/store_repo_config.py" in ownership_text
    assert "src/ait/store_worktree_layout.py" in ownership_text


def test_wave_5_hotspot_facades_import_extracted_modules() -> None:
    server_store_text = _read_text(WORKSPACE_ROOT / "src/ait_server/server_store.py")
    store_text = _read_text(WORKSPACE_ROOT / "src/ait/store.py")
    store_local_workflow_text = _read_text(WORKSPACE_ROOT / "src/ait/store_local_workflow.py")
    store_worktree_metadata_text = _read_text(WORKSPACE_ROOT / "src/ait/store_worktree_metadata.py")
    store_worktrees_text = _read_text(WORKSPACE_ROOT / "src/ait/store_worktrees.py")

    assert "from .store.task_tracking import (" in server_store_text
    assert "from .store_repo_config import (" in store_text
    assert "from .store_repo_config import (" in store_local_workflow_text
    assert "from .store_repo_config import load_config" in store_worktree_metadata_text
    assert "from .store_worktree_layout import (" in store_worktrees_text


def test_wave_5_supporting_modules_exist() -> None:
    required_paths = (
        "src/ait_server/store/task_tracking.py",
        "src/ait/store_repo_config.py",
        "src/ait/store_worktree_layout.py",
        "tests/test_directory_decoupling_wave_5_validation.py",
        "tests/test_directory_split_packages.py",
    )
    for relative in required_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative
