from __future__ import annotations

from pathlib import Path


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_local_content_storage_is_pack_runtime_compatibility_facade() -> None:
    storage_text = (WORKSPACE_ROOT / "src/ait/local_content_storage.py").read_text(encoding="utf-8")

    assert '"""Compatibility facade for local-content pack/runtime helpers."""' in storage_text
    assert "from . import local_content_pack_runtime as _pack_runtime" in storage_text
    assert "globals()[_name] = getattr(_pack_runtime, _name)" in storage_text
    assert "def create_pack(" not in storage_text
    assert "def gc_content(" not in storage_text
    assert "def storage_stats(" not in storage_text


def test_store_content_ops_uses_local_content_pack_runtime_narrow_seam() -> None:
    ops_text = (WORKSPACE_ROOT / "src/ait/store_content_ops.py").read_text(encoding="utf-8")

    assert "from . import local_content, local_content_pack_runtime, local_control" in ops_text
    assert "local_content_pack_runtime.storage_stats(ctx)" in ops_text
    assert "local_content_pack_runtime.create_pack(ctx, max_members=max_members, repack=repack)" in ops_text
    assert "local_content_pack_runtime.gc_content(" in ops_text
    assert "local_content.storage_stats(ctx)" not in ops_text
    assert "local_content.create_pack(ctx, max_members=max_members, repack=repack)" not in ops_text
    assert "local_content.gc_content(" not in ops_text
