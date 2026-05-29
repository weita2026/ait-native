from __future__ import annotations

from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_store_bootstrap_uses_local_content_schema_narrow_seam() -> None:
    bootstrap_text = (WORKSPACE_ROOT / "src/ait/store_bootstrap.py").read_text(encoding="utf-8")

    assert "from . import local_content_schema, local_control" in bootstrap_text
    assert "local_content_schema.initialize(ctx, resolved_default_line)" in bootstrap_text
    assert "local_content.initialize(ctx, resolved_default_line)" not in bootstrap_text
