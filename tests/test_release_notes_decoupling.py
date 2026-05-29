from __future__ import annotations

from pathlib import Path

import ait.release_notes as release_notes


WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_release_note_helpers_live_outside_release_ops() -> None:
    release_ops_text = (WORKSPACE_ROOT / "src/ait/release_ops.py").read_text(encoding="utf-8")
    notes_text = (WORKSPACE_ROOT / "src/ait/release_notes.py").read_text(encoding="utf-8")

    assert callable(release_notes.collect_release_note_tasks)
    assert callable(release_notes.apply_release_notes_to_readme)
    assert "from .release_notes import (" in release_ops_text
    assert "def _collect_release_note_tasks(" not in release_ops_text
    assert "def _render_release_notes(" not in release_ops_text
    assert "def apply_release_notes_to_readme(" in notes_text
    assert "from .release_ops import" not in notes_text
