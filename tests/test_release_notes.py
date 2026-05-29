from __future__ import annotations

from ait.release_notes import apply_release_notes_to_readme


def test_apply_release_notes_to_readme_replaces_existing_block() -> None:
    record = {"version": "0.2.0"}
    notes = {
        "mode": "delta",
        "previous_version": "0.1.0",
        "task_count": 1,
        "shown_task_count": 1,
        "omitted_task_count": 0,
        "tasks": [{"task_id": "LT-1301", "title": "Extract release note helpers"}],
    }
    readme_text = """# Demo

Release body.

<!-- ait-release-notes:start -->
Old notes
<!-- ait-release-notes:end -->
"""

    updated = apply_release_notes_to_readme(readme_text, record=record, notes=notes)

    assert updated.count("<!-- ait-release-notes:start -->") == 1
    assert updated.count("<!-- ait-release-notes:end -->") == 1
    assert "Old notes" not in updated
    assert "Tasks landed since `v0.1.0` (1 task):" in updated
    assert "- `LT-1301` Extract release note helpers" in updated

