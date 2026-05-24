from __future__ import annotations

from pathlib import Path

from ait_native.treepacks import (
    build_tree_pack_members,
    read_tree_pack_index,
    read_tree_pack_tree,
    summarize_tree_pack_archives,
    write_tree_pack_archive,
)


def test_tree_pack_round_trip_and_summary(tmp_path: Path):
    tree_rows = [
        {"tree_id": "TRE-CHILD", "entry_count": 1},
        {"tree_id": "TRE-ROOT", "entry_count": 2},
    ]
    tree_entry_rows = [
        {
            "tree_id": "TRE-ROOT",
            "entry_name": "README.md",
            "entry_type": "blob",
            "target_id": "BLB-README",
            "size_bytes": 5,
            "mode": "0o644",
        },
        {
            "tree_id": "TRE-ROOT",
            "entry_name": "nested",
            "entry_type": "tree",
            "target_id": "TRE-CHILD",
            "size_bytes": None,
            "mode": "tree",
        },
        {
            "tree_id": "TRE-CHILD",
            "entry_name": "main.py",
            "entry_type": "blob",
            "target_id": "BLB-MAIN",
            "size_bytes": 11,
            "mode": "0o755",
        },
    ]
    pack_path = tmp_path / "tree-packs" / "TPK-TEST.zip"

    members = build_tree_pack_members(tree_rows, tree_entry_rows)
    stats = write_tree_pack_archive(pack_path, "TPK-TEST", "2026-04-16T00:00:00+00:00", members)

    pack_index = read_tree_pack_index(pack_path)
    assert stats["tree_count"] == 2
    assert pack_index["pack_id"] == "TPK-TEST"
    assert pack_index["tree_count"] == 2
    assert {entry["tree_id"] for entry in pack_index["trees"]} == {"TRE-CHILD", "TRE-ROOT"}

    root_rows = read_tree_pack_tree(pack_path, "TRE-ROOT")
    assert [row["entry_name"] for row in root_rows] == ["README.md", "nested"]
    assert root_rows[0]["target_id"] == "BLB-README"
    assert root_rows[1]["entry_type"] == "tree"

    summary = summarize_tree_pack_archives(
        tmp_path,
        [{"pack_id": "TPK-TEST", "pack_path": str(pack_path.relative_to(tmp_path))}],
    )
    assert summary["pack_count"] == 1
    assert summary["indexed_tree_count"] == 2
    assert summary["indexed_entry_count"] == 3
    assert summary["index_error_count"] == 0
    assert summary["archive_bytes"] == pack_path.stat().st_size
