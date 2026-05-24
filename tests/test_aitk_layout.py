from __future__ import annotations

from ait.aitk_layout import layout_active_columns


def _row(snapshot_id: str, parent_snapshot_id: str | list[str] | None = None, *, labels: list[str] | None = None):
    row = {"snapshot_id": snapshot_id}
    if parent_snapshot_id is not None:
        row["parent_snapshot_id"] = parent_snapshot_id
    if labels is not None:
        row["head_lines"] = labels
    return row


def test_layout_active_columns_linear_history_uses_single_column():
    history_rows = [
        _row("SNP-C", "SNP-B"),
        _row("SNP-B", "SNP-A"),
        _row("SNP-A", None),
    ]

    layout = layout_active_columns(history_rows)

    assert [row["column"] for row in layout] == [0, 0, 0]
    assert [row["active_columns"] for row in layout] == [[0], [0], [0]]
    assert layout[0]["segments"] == [
        {"to_snapshot_id": "SNP-B", "from_column": 0, "to_column": 0, "kind": "parent"}
    ]


def test_layout_active_columns_branch_diverge_and_rejoin_limits_columns():
    history_rows = [
        _row("feature-top", "shared-base"),
        _row("shared-base", "main-base"),
        _row("main-top", "main-base"),
        _row("main-base", "root"),
        _row("root", None),
    ]

    layout = layout_active_columns(history_rows)

    assert layout[0]["column"] == 0
    assert layout[2]["column"] == 1
    assert max(row["column"] for row in layout) <= 1
    assert [row["snapshot_id"] for row in layout] == [r["snapshot_id"] for r in history_rows]


def test_layout_active_columns_attaches_head_labels_to_metadata():
    history_rows = [_row("HEAD", "BASE", labels=["feature/headline"])]
    layout = layout_active_columns(history_rows)

    assert layout[0]["labels"] == ["feature/headline"]


def test_layout_active_columns_supports_multiple_parent_snapshot_list():
    history_rows = [
        _row("merge", ["first-parent", "second-parent"]),
        _row("first-parent", "base"),
        _row("second-parent", "base"),
        _row("base", None),
    ]

    layout = layout_active_columns(history_rows)

    assert len(layout[0]["segments"]) == 2
    merge_segments = {
        (segment["from_column"], segment["to_column"], segment["to_snapshot_id"]) for segment in layout[0]["segments"]
    }
    assert (0, 0, "first-parent") in merge_segments
    assert (0, 1, "second-parent") in merge_segments
    assert layout[0]["column"] == 0
    assert layout[1]["column"] == 0
    assert layout[2]["column"] == 1
