from __future__ import annotations

import json
from pathlib import Path

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_10_task_graph_fixture_matches_compact_follow_up_shape() -> None:
    task_graph = json.loads(
        _read_text(WORKSPACE_ROOT / "docs/sprints/directory_decoupling_compact_dag_wave_10.task_graph.json")
    )

    assert task_graph["graph_id"] == "directory-decoupling-wave-10/compact-follow-up-dispatch"
    assert task_graph["execution_policy"]["mode"] == "guarded_full_dag_convergence"
    assert task_graph["execution_policy"]["worker_execution_mode"] == "worker_only_compact_packet"
    assert [node["node_id"] for node in task_graph["nodes"]] == ["A", "B", "C", "D", "E", "F", "L"]
    assert task_graph["source_plan"]["artifact_path"] == "docs/sprints/directory_decoupling_compact_dag_wave_10.md"


def test_wave_10_hotspot_facades_import_extracted_modules() -> None:
    web_chat_route_text = _read_text(WORKSPACE_ROOT / "src/ait_web/routes/chat.py")
    reply_runtime_text = _read_text(WORKSPACE_ROOT / "src/ait_chat/session_reply.py")
    telegram_app_text = _read_text(WORKSPACE_ROOT / "src/ait_agent/telegram/app.py")
    server_app_text = _read_text(WORKSPACE_ROOT / "src/ait_server/app.py")
    cli_server_runtime_text = _read_text(WORKSPACE_ROOT / "src/ait/cli/server_runtime_helpers.py")
    store_text = _read_text(WORKSPACE_ROOT / "src/ait/store.py")

    assert "from ait_web.chat_session_runtime import (" in web_chat_route_text
    assert "from ait_chat.reply_config import ReplyGenerationConfig, load_reply_generation_config" in reply_runtime_text
    assert "from .background_sync import TelegramBackgroundSyncManager" in telegram_app_text
    assert "from .agent_transport_runtime import (" in server_app_text
    assert "from ..server_runtime_preflight import (" in cli_server_runtime_text
    assert "from .store_line_cleanup import (" in store_text


def test_wave_10_supporting_artifacts_exist() -> None:
    required_workspace_paths = (
        "src/ait_web/chat_session_runtime.py",
        "src/ait_chat/reply_config.py",
        "src/ait_agent/telegram/background_sync.py",
        "src/ait_server/agent_transport_runtime.py",
        "src/ait/server_runtime_preflight.py",
        "src/ait/store_line_cleanup.py",
        "tests/test_directory_decoupling_wave_10_validation.py",
        "docs/sprints/directory_decoupling_compact_dag_wave_10.task_graph.json",
    )
    for relative in required_workspace_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative
