from __future__ import annotations

from pathlib import Path

import ait_server.server_content as server_content
import ait_server.server_content_repo_lines as server_content_repo_lines


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

SERVER_CONTENT_REPO_LINE_EXPORTS = (
    "read_ref",
    "write_ref",
    "ensure_repository",
    "repository_exists",
    "get_repository",
    "set_repository_lifecycle_state",
    "list_lines",
    "get_line",
    "list_lines_by_head_snapshot_ids",
    "update_line",
    "archive_line",
)

SERVER_CONTENT_REPO_LINE_CALLERS = (
    "src/ait_server/authority_store.py",
    "src/ait_server/read_models.py",
    "src/ait_server/read_models_domains/workflow_detail.py",
    "src/ait_server/repo_ci.py",
    "src/ait_server/server_auth.py",
    "src/ait_server/server_control.py",
    "src/ait_server/server_store.py",
    "src/ait_server/store/lands.py",
    "src/ait_server/store/plans.py",
    "src/ait_server/store/releases.py",
    "src/ait_server/store/repo_ops.py",
    "src/ait_server/store/repo_retire.py",
    "src/ait_server/store/sessions.py",
    "src/ait_server/store/stacks.py",
    "src/ait_server/store/task_tracking.py",
)


def test_server_content_repo_line_helpers_match_facade() -> None:
    for name in SERVER_CONTENT_REPO_LINE_EXPORTS:
        assert getattr(server_content_repo_lines, name) is getattr(server_content, name), name


def test_server_content_repo_line_helpers_are_extracted_from_facade() -> None:
    server_content_text = (WORKSPACE_ROOT / "src/ait_server/server_content.py").read_text(encoding="utf-8")
    repo_line_text = (WORKSPACE_ROOT / "src/ait_server/server_content_repo_lines.py").read_text(encoding="utf-8")

    assert "from .server_content_repo_lines import (" in server_content_text
    assert "def read_ref(" not in server_content_text
    assert "def write_ref(" not in server_content_text
    assert "def ensure_repository(" not in server_content_text
    assert "def repository_exists(" not in server_content_text
    assert "def get_repository(" not in server_content_text
    assert "def set_repository_lifecycle_state(" not in server_content_text
    assert "def list_lines(" not in server_content_text
    assert "def get_line(" not in server_content_text
    assert "def list_lines_by_head_snapshot_ids(" not in server_content_text
    assert "def update_line(" not in server_content_text
    assert "def archive_line(" not in server_content_text
    assert "from .server_content import (" not in repo_line_text


def test_server_content_repo_line_callers_use_narrow_seam() -> None:
    for relative_path in SERVER_CONTENT_REPO_LINE_CALLERS:
        text = (WORKSPACE_ROOT / relative_path).read_text(encoding="utf-8")
        assert "server_content_repo_lines import" in text, relative_path
