from __future__ import annotations

from pathlib import Path

from ait.repo_paths import RepoContext


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]
AUTHORED_ROOT = RepoContext.discover(WORKSPACE_ROOT).repo_root
DECOUPLING_PLAN = AUTHORED_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
OWNERSHIP_MAP = AUTHORED_ROOT / "docs/ait_module_ownership_map.md"


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_wave_2_contract_docs_classify_reply_runtime_boundary() -> None:
    plan_text = _read_text(DECOUPLING_PLAN)
    ownership_text = _read_text(OWNERSHIP_MAP)

    assert "./sprints/directory_decoupling_hotspot_wave_2.md" in plan_text
    assert "ait -> ait_chat" in plan_text
    assert "src/ait/cli/reply_runtime_seam.py" in plan_text
    assert "ait_agent -> ait_chat" in plan_text
    assert "src/ait_agent/runtime_backend.py" in plan_text
    assert "src/ait_agent/telegram/app.py" in plan_text
    assert "ait_server -> ait_chat" in plan_text
    assert "src/ait_server/app.py" in plan_text
    assert "ait_server -> ait_agent" in plan_text
    assert "src/ait_server/admin_cache.py" in plan_text
    assert "dedicated shared reply-runtime seam" in plan_text
    assert "src/ait_chat/runtime_config.py" in plan_text
    assert "src/ait_chat/session_reply.py" in plan_text
    assert "tests/test_cross_root_decoupling_contract.py" in plan_text
    assert "tests/test_reply_runtime_seam.py" in plan_text
    assert "tests/test_directory_decoupling_wave_2_contracts.py" in plan_text

    assert "src/ait_chat/runtime_config.py" in ownership_text
    assert "src/ait_chat/session_reply.py" in ownership_text
    assert "shared reply-runtime seam" in ownership_text
    assert "transport-agnostic" in ownership_text
