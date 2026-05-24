from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_workspace_restore


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_WORKSPACE_RESTORE_EXPORTS = (
    "restore_workspace",
    "restore_workspace_paths",
)


def test_store_workspace_restore_helpers_match_store_facade() -> None:
    for name in STORE_WORKSPACE_RESTORE_EXPORTS:
        assert getattr(store_workspace_restore, name) is getattr(store, name), name


def test_store_workspace_restore_is_extracted_from_store_facade() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    restore_text = (WORKSPACE_ROOT / "src/ait/store_workspace_restore.py").read_text(encoding="utf-8")

    assert "from .store_workspace_restore import (" in store_text
    assert "def restore_workspace(" not in store_text
    assert "def restore_workspace_paths(" not in store_text
    assert "from .store import (" not in restore_text
