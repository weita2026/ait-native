from __future__ import annotations

import json
from pathlib import Path

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _repo_root() -> Path:
    worktree_config = WORKSPACE_ROOT / ".ait-worktree.json"
    if not worktree_config.exists():
        return WORKSPACE_ROOT
    payload = json.loads(_read_text(worktree_config))
    repo_root = str(payload.get("repo_root") or "").strip()
    return Path(repo_root) if repo_root else WORKSPACE_ROOT


def _artifact_path(relative: str) -> Path:
    workspace_path = WORKSPACE_ROOT / relative
    if workspace_path.exists():
        return workspace_path
    return _repo_root() / relative


def test_wave_11_task_graph_fixture_matches_internal_hotspot_shape() -> None:
    task_graph = json.loads(
        _read_text(_artifact_path("docs/sprints/directory_decoupling_compact_dag_wave_11.task_graph.json"))
    )

    assert task_graph["graph_id"] == "directory-decoupling-wave-11/internal-hotspot-split-dispatch"
    assert task_graph["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert task_graph["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert [node["node_id"] for node in task_graph["nodes"]] == ["A", "B", "C", "D", "E", "F", "L"]
    assert task_graph["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_11.md"


def test_wave_11_hotspot_facades_import_extracted_modules() -> None:
    cli_app_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/app.py")
    telegram_app_text = _read_text(WORKSPACE_ROOT / "src/ait_agent/telegram/app.py")
    server_app_text = _read_text(WORKSPACE_ROOT / "src/ait_server/app.py")
    server_store_text = _read_text(WORKSPACE_ROOT / "src/ait_server/server_store.py")
    store_worktrees_text = _read_text(WORKSPACE_ROOT / "src/ait/store_worktrees.py")

    assert "from .commands.bootstrap import bootstrap_cli_commands" in cli_app_text
    assert "from .commands import (" in telegram_app_text
    assert "self._command_runtime = TelegramCommandRuntime(" in telegram_app_text
    assert "from .release_route_helpers import (" in server_app_text
    assert "from .store.lands import (" in server_store_text
    assert "from .store_worktree_views import (" in store_worktrees_text


def test_wave_11_supporting_artifacts_exist() -> None:
    required_workspace_paths = (
        "src/ait/cli/commands/bootstrap.py",
        "src/ait_agent/telegram/commands.py",
        "src/ait_server/release_route_helpers.py",
        "src/ait_server/store/land_validation.py",
        "src/ait/store_worktree_views.py",
        "tests/test_directory_decoupling_wave_11_validation.py",
        "docs/sprints/directory_decoupling_compact_dag_wave_11.task_graph.json",
    )
    for relative in required_workspace_paths:
        assert _artifact_path(relative).exists(), relative
