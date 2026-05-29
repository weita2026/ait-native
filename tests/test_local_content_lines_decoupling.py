from __future__ import annotations

from pathlib import Path

import pytest

from ait import local_content
from ait import local_content_lines
from ait import store


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

LOCAL_CONTENT_LINE_EXPORTS = (
    "_ref_path",
    "read_ref",
    "write_ref",
    "get_line",
    "list_lines",
    "create_line",
    "set_line_head",
    "archive_line",
)


def test_local_content_line_helpers_match_facade() -> None:
    for name in LOCAL_CONTENT_LINE_EXPORTS:
        assert getattr(local_content_lines, name) is getattr(local_content, name), name


def test_local_content_line_helpers_are_extracted_from_facade() -> None:
    local_content_text = (WORKSPACE_ROOT / "src/ait/local_content.py").read_text(encoding="utf-8")
    helper_text = (WORKSPACE_ROOT / "src/ait/local_content_lines.py").read_text(encoding="utf-8")

    assert "from .local_content_lines import (" in local_content_text
    assert "def _ref_path(" not in local_content_text
    assert "def read_ref(" not in local_content_text
    assert "def write_ref(" not in local_content_text
    assert "def get_line(" not in local_content_text
    assert "def list_lines(" not in local_content_text
    assert "def create_line(" not in local_content_text
    assert "def set_line_head(" not in local_content_text
    assert "def archive_line(" not in local_content_text
    assert "from .local_content import (" not in helper_text


def test_local_content_line_helpers_round_trip_line_lifecycle(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    (repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "seed")
    created = local_content_lines.create_line(ctx, "feature/demo", snapshot["snapshot_id"])

    assert created["head_snapshot_id"] == snapshot["snapshot_id"]
    assert local_content_lines.read_ref(ctx, "feature/demo") == snapshot["snapshot_id"]
    moved = local_content_lines.set_line_head(ctx, "feature/demo", None)
    assert moved["head_snapshot_id"] is None
    assert local_content.get_line(ctx, "feature/demo")["head_snapshot_id"] is None
    archived = local_content_lines.archive_line(ctx, "feature/demo")
    assert archived["status"] == "archived"
    assert local_content.list_lines(ctx)[1]["line_name"] == "main"
    with pytest.raises(ValueError):
        local_content_lines.set_line_head(ctx, "feature/demo", snapshot["snapshot_id"])
