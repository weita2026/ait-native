from __future__ import annotations

from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_plan_sync_adoption_uses_local_content_pack_runtime_blob_seam() -> None:
    adoption_text = (WORKSPACE_ROOT / "src/ait/cli/plan_sync_adoption.py").read_text(encoding="utf-8")

    assert "from ..local_content_pack_runtime import _read_blob_bytes, ensure_blob_bytes" in adoption_text
    assert "from ..local_content import _read_blob_bytes, ensure_blob_bytes" not in adoption_text


def test_plan_sync_scope_uses_local_content_pack_runtime_for_blob_materialization() -> None:
    scope_text = (WORKSPACE_ROOT / "src/ait/cli/plan_sync_scope.py").read_text(encoding="utf-8")

    assert "from ..local_content import IGNORED_DIRS, workspace_path_is_ignored" in scope_text
    assert "from ..local_content_pack_runtime import ensure_blob_bytes" in scope_text
    assert "from ..local_content import IGNORED_DIRS, ensure_blob_bytes, workspace_path_is_ignored" not in scope_text
