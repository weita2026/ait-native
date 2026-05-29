from __future__ import annotations

from pathlib import Path

from ait import store
from ait import store_workspace_replay


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]

STORE_WORKSPACE_REPLAY_EXPORTS = (
    "revert_snapshot",
    "replay_snapshot",
    "revert_change",
    "replay_change",
)


def test_store_workspace_replay_helpers_match_store_facade() -> None:
    for name in STORE_WORKSPACE_REPLAY_EXPORTS:
        assert getattr(store_workspace_replay, name) is getattr(store, name), name


def test_store_workspace_replay_is_extracted_from_store_facade() -> None:
    store_text = (WORKSPACE_ROOT / "src/ait/store.py").read_text(encoding="utf-8")
    replay_text = (WORKSPACE_ROOT / "src/ait/store_workspace_replay.py").read_text(encoding="utf-8")

    assert "from .store_workspace_replay import (" in store_text
    assert "def revert_snapshot(" not in store_text
    assert "def replay_snapshot(" not in store_text
    assert "def revert_change(" not in store_text
    assert "def replay_change(" not in store_text
    assert "from .store import (" not in replay_text
    assert "local_content_snapshots" in replay_text
