from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_stash


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_STASH_EXPORTS = (
    "create_stash",
    "list_stashes",
    "get_stash",
    "apply_stash",
    "drop_stash",
)


def test_store_stash_helpers_match_store_facade() -> None:
    for name in STORE_STASH_EXPORTS:
        assert getattr(store_stash, name) is getattr(store, name), name


def test_store_stash_is_extracted_from_store_facade() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    stash_text = (WORKSPACE_ROOT / "src/ait/store_stash.py").read_text(encoding="utf-8")

    assert "from .store_stash import (" in store_text
    assert "def create_stash(" not in store_text
    assert "def list_stashes(" not in store_text
    assert "def get_stash(" not in store_text
    assert "def apply_stash(" not in store_text
    assert "def drop_stash(" not in store_text
    assert "from .store import (" not in stash_text
    assert "local_content_snapshots" in stash_text
    assert "local_content.workspace_delta(" not in stash_text
    assert "local_content.create_snapshot(" not in stash_text
    assert "local_content.restore_workspace(" not in stash_text
