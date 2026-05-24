from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_8_docs_route_to_active_residual_seam_wave() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)

    assert "./sprints/directory_decoupling_compact_dag_wave_8.md" in plan_text
    assert "directory-structure-decoupling/wave-8-residual-seam-dispatch" in plan_text
    assert "src/ait_web/server_data_runtime.py" in plan_text
    assert "src/ait_web/server_read_runtime.py" in plan_text
    assert "src/ait_web/telegram_workflow_runtime.py" in plan_text
    assert "src/ait_server/app.py" in plan_text
    assert "src/ait_server/session_transport_payloads.py" in plan_text
    assert "src/ait/cli/server_runtime_helpers.py" in plan_text
    assert "src/ait_chat/session_reply.py" in plan_text
    assert "src/ait_chat/reply_context.py" in plan_text

    assert "src/ait_web/server_data_runtime.py" in ownership_text
    assert "src/ait_web/agent_transport_runtime.py" in ownership_text
    assert "src/ait/server_runtime_seam.py" in ownership_text
    assert "src/ait/cli/server_runtime_helpers.py" in ownership_text
    assert "src/ait_server/app.py" in ownership_text
    assert "src/ait_server/session_transport_payloads.py" in ownership_text
    assert "src/ait_chat/session_reply.py" in ownership_text
    assert "src/ait_chat/reply_context.py" in ownership_text


def test_wave_8_supporting_artifacts_exist() -> None:
    required_workspace_paths = (
        "src/ait_web/server_data_runtime.py",
        "src/ait_web/server_auth_runtime.py",
        "src/ait_web/server_context_runtime.py",
        "src/ait_web/server_read_runtime.py",
        "src/ait_web/server_write_runtime.py",
        "src/ait_web/agent_transport_runtime.py",
        "src/ait_web/transport_binding_runtime.py",
        "src/ait_web/telegram_transport_runtime.py",
        "src/ait_web/telegram_workflow_runtime.py",
        "src/ait/server_runtime_seam.py",
        "src/ait/cli/server_runtime_helpers.py",
        "src/ait_server/app.py",
        "src/ait_server/session_transport_payloads.py",
        "src/ait_chat/session_reply.py",
        "src/ait_chat/reply_context.py",
        "src/ait_chat/reply_attachments.py",
        "src/ait_chat/reply_http.py",
        "tests/test_directory_decoupling_wave_8_validation.py",
        "docs/sprints/directory_decoupling_compact_dag_wave_8.task_graph.json",
    )
    for relative in required_workspace_paths:
        assert (WORKSPACE_ROOT / relative).exists(), relative

    required_authored_paths = (
        "docs/ait_directory_structure_decoupling_plan.md",
        "docs/ait_module_ownership_map.md",
        "docs/sprints/directory_decoupling_compact_dag_wave_8.md",
    )
    for relative in required_authored_paths:
        assert (AUTHORED_ROOT / relative).exists(), relative
