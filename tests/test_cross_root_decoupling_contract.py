from __future__ import annotations

from pathlib import Path
from typing import Iterable

import pytest


REPO_ROOT = Path(__file__).resolve().parents[1]
DECOUPLING_PLAN_PATH = REPO_ROOT / "docs/ait_directory_structure_decoupling_plan.md"
EXECUTION_ARTIFACT_REL_PATH = "docs/sprints/ait_agent_server_runtime_seam_execution.md"


def _resolve_in_repo_tree(relative: str) -> Path:
    candidates: Iterable[Path] = [REPO_ROOT, *REPO_ROOT.parents]
    for root in candidates:
        candidate = root / relative
        if candidate.exists():
            return candidate
    msg = f"Could not resolve required file: {relative}"
    raise FileNotFoundError(msg)


def _python_files_with_direct_import(root: Path, prefix: str) -> set[str]:
    matches: set[str] = set()
    for path in root.rglob("*.py"):
        text = path.read_text(encoding="utf-8")
        for line in text.splitlines():
            stripped = line.strip()
            if stripped.startswith(f"from {prefix}") or stripped.startswith(f"import {prefix}"):
                matches.add(path.relative_to(REPO_ROOT).as_posix())
                break
    return matches


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def test_cross_root_contract_freezes_known_ait_web_to_ait_server_imports():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web", "ait_server") == {
        "src/ait_web/server_entry_runtime.py",
        "src/ait_web/server_data_runtime.py",
    }


def test_cross_root_contract_freezes_known_ait_web_to_ait_seam():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web", "ait.") == {
        "src/ait_web/local_repo_runtime.py",
    }


def test_cross_root_contract_freezes_known_ait_web_to_ait_agent_seam():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web", "ait_agent") == {
        "src/ait_web/agent_transport_runtime.py",
    }


def test_cross_root_contract_keeps_ait_web_pages_direct_import_free():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web/pages", "ait_server") == set()
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web/pages", "ait.") == set()
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web/pages", "ait_agent") == set()


def test_cross_root_contract_keeps_key_ait_web_routes_direct_import_free():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web/routes", "ait_server") == set()
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web/routes", "ait.") == set()
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_web/routes", "ait_agent") == set()


def test_cross_root_contract_keeps_known_ait_agent_direct_import_surface_small():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_agent", "ait_server") == set()


def test_cross_root_contract_freezes_known_ait_server_to_ait_seam():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_server", "ait.") == {
        "src/ait_server/local_repo_seams.py",
        "src/ait_server/session_route_helpers.py",
        "src/ait_server/task_dag_seams.py",
    }


def test_cross_root_contract_freezes_known_ait_to_ait_server_exceptions():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait", "ait_server") == {
        "src/ait/server_runtime_seam.py",
    }


def test_cross_root_contract_freezes_known_ait_to_ait_chat_reply_runtime_seam():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait", "ait_chat") == {
        "src/ait/cli/reply_runtime_seam.py",
    }


def test_cross_root_contract_freezes_known_ait_agent_to_ait_chat_reply_runtime_seam():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_agent", "ait_chat") == {
        "src/ait_agent/runtime_backend.py",
        "src/ait_agent/telegram/app.py",
    }


def test_cross_root_contract_freezes_known_ait_server_to_ait_chat_reply_runtime_seam():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_server", "ait_chat") == {
        "src/ait_server/app.py",
        "src/ait_server/session_route_helpers.py",
    }


def test_cross_root_contract_freezes_known_ait_server_to_ait_agent_runtime_exceptions():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_server", "ait_agent") == {
        "src/ait_server/agent_transport_runtime.py",
    }


def test_cross_root_contract_keeps_ait_server_free_of_direct_telegram_runtime_imports():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_server", "ait_agent.telegram.runtime") == set()


def test_cross_root_contract_keeps_ait_free_of_direct_telegram_app_imports():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait", "ait_agent.telegram.app") == set()


def test_cross_root_contract_keeps_ait_server_free_of_direct_telegram_app_imports():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_server", "ait_agent.telegram.app") == set()


def test_cross_root_contract_keeps_ait_chat_direct_import_free_from_product_roots():
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_chat", "ait.") == set()
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_chat", "ait_server") == set()
    assert _python_files_with_direct_import(REPO_ROOT / "src/ait_chat", "ait_agent") == set()


def test_cross_root_decoupling_docs_route_to_active_execution_artifact():
    if not DECOUPLING_PLAN_PATH.exists():
        pytest.skip("Decoupling plan Markdown is lineage-only and may be absent from materialized line snapshots.")
    plan_text = _read_text(DECOUPLING_PLAN_PATH)
    try:
        execution_artifact_path = _resolve_in_repo_tree(EXECUTION_ARTIFACT_REL_PATH)
    except FileNotFoundError:
        pytest.skip("Execution sprint Markdown is lineage-only and may be absent from materialized line snapshots.")
    artifact_text = _read_text(execution_artifact_path)

    assert (
        "./sprints/ait_agent_server_runtime_seam_execution.md" in plan_text
        or "./execution_plans/ait_agent_server_runtime_seam_execution.md" in plan_text
    )
    assert "tests/test_cross_root_decoupling_contract.py" in plan_text
    assert "src/ait_agent/server_runtime_seam.py" in plan_text
    assert "src/ait_agent/local_runtime_seam.py" in plan_text
    assert "Collapse ait_agent server runtime seam" in artifact_text
    assert "tests/test_cross_root_decoupling_contract.py" in artifact_text
    assert "remote-land the slice" in artifact_text
