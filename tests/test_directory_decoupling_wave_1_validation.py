from __future__ import annotations

import re
from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"
SPRINT_CARD = AUTHORED_ROOT / "docs/sprints/directory_decoupling_parallel_wave_1.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_1_docs_route_to_active_execution_artifact() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)
    sprint_text = _read_text(SPRINT_CARD)

    assert "./sprints/ait_agent_server_runtime_seam_execution.md" in plan_text
    assert "./ait_module_ownership_map.md" in plan_text
    assert "src/ait/cli/" in ownership_text
    assert "src/ait_agent/telegram/" in ownership_text
    assert "src/ait_server/app.py" in ownership_text
    assert "src/ait_server/store/" in ownership_text
    assert "src/ait_agent/telegram/workflow_notifications.py" in sprint_text
    assert "src/ait/store_worktree_state.py" in sprint_text
    assert "src/ait_server/admin_cache.py" in sprint_text
    assert "src/ait_server/workflow_async_jobs.py" in sprint_text
    assert "tests/test_directory_decoupling_wave_1_validation.py" in sprint_text


def test_wave_1_sprint_card_marks_all_parallel_items_complete() -> None:
    sprint_text = _read_text(SPRINT_CARD)
    refs = (
        "directory-decoupling-parallel-wave-1/cli-split",
        "directory-decoupling-parallel-wave-1/telegram-split",
        "directory-decoupling-parallel-wave-1/server-store-split",
        "directory-decoupling-parallel-wave-1/local-core-split",
        "directory-decoupling-parallel-wave-1/server-app-split",
        "directory-decoupling-parallel-wave-1/validation-land",
    )
    for ref in refs:
        assert re.search(rf"^- \[x\] .*\[ref: {re.escape(ref)}\]$", sprint_text, flags=re.MULTILINE), ref


def test_wave_1_hotspot_facades_import_extracted_modules() -> None:
    store_worktrees_text = _read_text(WORKSPACE_ROOT / "src/ait/store_worktrees.py")
    telegram_app_text = _read_text(WORKSPACE_ROOT / "src/ait_agent/telegram/app.py")
    server_app_text = _read_text(WORKSPACE_ROOT / "src/ait_server/app.py")

    assert "from .store_worktree_state import (" in store_worktrees_text
    assert "from .workflow_notifications import (" in telegram_app_text
    assert "from .admin_cache import (" in server_app_text
    assert "from .workflow_async_jobs import (" in server_app_text


def test_wave_1_supporting_modules_exist() -> None:
    required_paths = (
        "src/ait/cli/shared.py",
        "src/ait_agent/telegram/workflow_notifications.py",
        "src/ait/store_worktree_state.py",
        "src/ait_server/admin_cache.py",
        "src/ait_server/workflow_async_jobs.py",
        "src/ait_server/store/plans.py",
        "src/ait_server/store/repo_ops.py",
        "src/ait_server/store/releases.py",
        "src/ait_server/store/sessions.py",
        "src/ait_server/store/stacks.py",
        "src/ait_server/store/workflow_artifacts.py",
    )
    for relative in required_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative

    assert any((WORKSPACE_ROOT / "src/ait/cli/commands").glob("*.py"))
