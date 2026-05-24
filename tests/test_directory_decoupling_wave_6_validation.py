from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"



def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")



def test_wave_6_docs_route_to_shipped_hotspot_wave() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)

    assert "./sprints/directory_decoupling_compact_dag_wave_6.md" in plan_text
    assert "shipped wave-6 hotspot seams" in plan_text
    assert "src/ait/cli/app_surfaces.py" in plan_text
    assert "src/ait_agent/telegram/transport_io.py" in plan_text
    assert "src/ait_server/session_route_helpers.py" in plan_text
    assert "tests/test_directory_decoupling_wave_6_validation.py" in plan_text

    assert "src/ait/cli/app_surfaces.py" in ownership_text
    assert "src/ait_agent/telegram/transport_io.py" in ownership_text
    assert "src/ait_server/session_route_helpers.py" in ownership_text



def test_wave_6_hotspot_facades_import_extracted_modules() -> None:
    cli_app_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/app.py")
    telegram_app_text = _read_text(WORKSPACE_ROOT / "src/ait_agent/telegram/app.py")
    server_app_text = _read_text(WORKSPACE_ROOT / "src/ait_server/app.py")

    assert "from .app_surfaces import (" in cli_app_text
    assert "from .transport_io import (" in telegram_app_text
    assert "from .session_route_helpers import (" in server_app_text



def test_wave_6_supporting_modules_exist() -> None:
    workspace_paths = (
        "src/ait/cli/app_surfaces.py",
        "src/ait_agent/telegram/transport_io.py",
        "src/ait_server/session_route_helpers.py",
        "tests/test_directory_decoupling_wave_6_validation.py",
        "tests/test_directory_split_packages.py",
    )
    for relative in workspace_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative
    assert (AUTHORED_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_6.md").exists()
