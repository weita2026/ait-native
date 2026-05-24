from __future__ import annotations

import re
from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"
SPRINT_CARD = AUTHORED_ROOT / "docs/sprints/directory_decoupling_hotspot_wave_2.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_2_docs_route_to_shipped_hotspot_wave() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)
    sprint_text = _read_text(SPRINT_CARD)

    assert "./sprints/directory_decoupling_hotspot_wave_2.md" in plan_text
    assert "shipped wave-2 hotspot seams" in plan_text
    assert "src/ait/cli/task_dag_telegram_watch.py" in plan_text
    assert "src/ait_agent/telegram/worker_config.py" in plan_text
    assert "src/ait/store_worktree_filesystem.py" in plan_text
    assert "src/ait_server/admin_cache.py" in plan_text
    assert "src/ait_server/app.py" in plan_text

    assert "src/ait/store.py" in ownership_text
    assert "src/ait/store_worktrees.py" in ownership_text
    assert "src/ait_server/server_store.py" in ownership_text
    assert "shared reply-runtime seam" in ownership_text

    assert "Status: completed execution artifact" in sprint_text
    assert "src/ait/cli/task_dag_telegram_watch.py" in sprint_text
    assert "src/ait_agent/telegram/worker_config.py" in sprint_text
    assert "src/ait/store_worktree_filesystem.py" in sprint_text
    assert "tests/test_directory_decoupling_wave_2_validation.py" in sprint_text


def test_wave_2_sprint_card_marks_all_parallel_items_complete() -> None:
    sprint_text = _read_text(SPRINT_CARD)
    refs = (
        "directory-decoupling-hotspot-wave-2/contracts",
        "directory-decoupling-hotspot-wave-2/reply-runtime",
        "directory-decoupling-hotspot-wave-2/cli-task-dag",
        "directory-decoupling-hotspot-wave-2/telegram-runtime",
        "directory-decoupling-hotspot-wave-2/server-store",
        "directory-decoupling-hotspot-wave-2/server-app",
        "directory-decoupling-hotspot-wave-2/local-core",
        "directory-decoupling-hotspot-wave-2/validation-land",
    )
    for ref in refs:
        assert re.search(rf"^- \[x\] .*\[ref: {re.escape(ref)}\]$", sprint_text, flags=re.MULTILINE), ref


def test_wave_2_hotspot_facades_import_extracted_modules() -> None:
    cli_app_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/app.py")
    store_text = _read_text(WORKSPACE_ROOT / "src/ait/store.py")
    store_worktrees_text = _read_text(WORKSPACE_ROOT / "src/ait/store_worktrees.py")
    server_store_text = _read_text(WORKSPACE_ROOT / "src/ait_server/server_store.py")

    assert "from .task_dag_telegram_watch import (" in cli_app_text
    assert "from . import store_worktrees as _store_worktrees" in store_text
    assert "from .store_worktree_filesystem import (" in store_worktrees_text
    assert "from .store.plans import (" in server_store_text
    assert "from .store.releases import (" in server_store_text
    assert "from .store.sessions import (" in server_store_text
    assert "from .store.stacks import (" in server_store_text


def test_wave_2_supporting_modules_exist() -> None:
    required_paths = (
        "src/ait/cli/task_dag_telegram_watch.py",
        "src/ait_agent/telegram/worker_config.py",
        "src/ait/store_worktree_filesystem.py",
        "src/ait_agent/runtime_bindings.py",
        "src/ait_server/admin_cache.py",
        "src/ait_server/store/plans.py",
        "src/ait_server/store/releases.py",
        "src/ait_server/store/sessions.py",
        "src/ait_server/store/stacks.py",
        "tests/cli/test_task_dag_telegram_watch.py",
        "tests/test_directory_decoupling_wave_2_validation.py",
        "tests/test_directory_split_packages.py",
        "tests/test_store_worktrees_decoupling.py",
    )
    for relative in required_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative
