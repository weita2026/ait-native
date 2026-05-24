from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_4_docs_route_to_shipped_hotspot_wave() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)

    assert "./sprints/directory_decoupling_compact_dag_wave_4.md" in plan_text
    assert "shipped wave-4 hotspot seams" in plan_text
    assert "src/ait/cli/bootstrap_views.py" in plan_text
    assert "src/ait_agent/telegram/config.py" in plan_text
    assert "src/ait_server/task_dag_route_helpers.py" in plan_text
    assert "tests/test_directory_decoupling_wave_4_validation.py" in plan_text

    assert "src/ait/cli/bootstrap_views.py" in ownership_text
    assert "src/ait_agent/telegram/config.py" in ownership_text
    assert "src/ait_server/task_dag_route_helpers.py" in ownership_text


def test_wave_4_hotspot_facades_import_extracted_modules() -> None:
    cli_app_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/app.py")
    telegram_app_text = _read_text(WORKSPACE_ROOT / "src/ait_agent/telegram/app.py")
    server_app_text = _read_text(WORKSPACE_ROOT / "src/ait_server/app.py")

    assert "from .bootstrap_views import (" in cli_app_text
    assert "from .config import (" in telegram_app_text
    assert "from .task_dag_route_helpers import (" in server_app_text


def test_wave_4_supporting_modules_exist() -> None:
    workspace_paths = (
        "src/ait/cli/bootstrap_views.py",
        "src/ait_agent/telegram/config.py",
        "src/ait_server/task_dag_route_helpers.py",
        "tests/test_directory_decoupling_wave_4_validation.py",
        "tests/test_directory_split_packages.py",
    )
    for relative in workspace_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative
    assert (AUTHORED_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_4.md").exists()
