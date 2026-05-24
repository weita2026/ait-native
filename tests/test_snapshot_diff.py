from __future__ import annotations

from pathlib import Path

from ait import snapshot_diff
from ait import local_content
from ait import store


def test_diff_snapshot_file_maps_tracks_added_modified_deleted_mode_changed():
    old_files = {
        "a.txt": {"path": "a.txt", "blob_id": "A", "size_bytes": 3, "mode": "0o644"},
        "b.txt": {"path": "b.txt", "blob_id": "B", "size_bytes": 1, "mode": "0o644"},
        "c.txt": {"path": "c.txt", "blob_id": "C", "size_bytes": 1, "mode": "0o644"},
    }
    new_files = {
        "a.txt": {"path": "a.txt", "blob_id": "A", "size_bytes": 3, "mode": "0o644"},
        "b.txt": {"path": "b.txt", "blob_id": "B", "size_bytes": 1, "mode": "0o755"},
        "d.txt": {"path": "d.txt", "blob_id": "D", "size_bytes": 2, "mode": "0o644"},
        "c.txt": {"path": "c.txt", "blob_id": "C2", "size_bytes": 1, "mode": "0o644"},
    }

    result = snapshot_diff.diff_snapshot_file_maps(old_files, new_files, old_snapshot_id="S1", new_snapshot_id="S2")

    assert result["added"] == ["d.txt"]
    assert result["deleted"] == []
    assert result["modified"] == ["c.txt"]
    assert result["mode_changed"] == ["b.txt"]
    assert result["summary"]["files_changed"] == 3

    by_path = {item["path"]: item for item in result["files"]}
    assert by_path["d.txt"]["status"] == "added"
    assert by_path["d.txt"]["old"]["blob_id"] is None
    assert by_path["d.txt"]["new"]["blob_id"] == "D"
    assert by_path["b.txt"]["status"] == "mode_changed"
    assert by_path["c.txt"]["status"] == "modified"
    assert by_path["c.txt"]["old"]["blob_id"] == "C"
    assert by_path["c.txt"]["new"]["blob_id"] == "C2"


def test_snapshot_diff_includes_text_diff_for_small_text_blobs(tmp_path: Path):
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    (repo / "a.txt").write_text("hello\nfoo\n", encoding="utf-8")
    old_snapshot = local_content.create_snapshot(ctx, "repo", "main", "first")

    (repo / "a.txt").write_text("hello\nbar\n", encoding="utf-8")
    new_snapshot = local_content.create_snapshot(ctx, "repo", "main", "second")

    result = snapshot_diff.snapshot_diff(
        ctx,
        old_snapshot["snapshot_id"],
        new_snapshot["snapshot_id"],
        include_text=True,
        max_bytes=1_000_000,
    )

    assert result["summary"]["files_changed"] == 1
    assert result["summary"]["insertions"] >= 1
    assert result["summary"]["deletions"] >= 1
    file_row = next(f for f in result["files"] if f["path"] == "a.txt")
    assert file_row["status"] == "modified"
    assert file_row["diff"]["status"] == "text"
    assert "+bar" in (file_row["diff"]["text"] or "")


def test_diff_snapshot_file_maps_reports_exact_rename_hints_without_rewriting_core_delta_lists():
    old_files = {
        "docs/old-name.md": {"path": "docs/old-name.md", "blob_id": "BLB-1", "size_bytes": 12, "mode": "0o644"},
    }
    new_files = {
        "guides/new-name.md": {"path": "guides/new-name.md", "blob_id": "BLB-1", "size_bytes": 12, "mode": "0o644"},
    }

    result = snapshot_diff.diff_snapshot_file_maps(old_files, new_files, old_snapshot_id="S1", new_snapshot_id="S2")

    assert result["added"] == ["guides/new-name.md"]
    assert result["deleted"] == ["docs/old-name.md"]
    assert result["modified"] == []
    assert result["mode_changed"] == []
    assert result["summary"]["files_changed"] == 2
    assert result["rename_hints"] == [
        {
            "match_kind": "exact_blob_id",
            "blob_id": "BLB-1",
            "old_path": "docs/old-name.md",
            "new_path": "guides/new-name.md",
            "old_parent_path": "docs",
            "new_parent_path": "guides",
            "size_bytes": 12,
        }
    ]
    assert result["directory_move_hints"] == []


def test_diff_snapshot_file_maps_skips_ambiguous_same_blob_rename_pairing():
    old_files = {
        "src/alpha.py": {"path": "src/alpha.py", "blob_id": "BLB-same", "size_bytes": 3, "mode": "0o644"},
        "src/beta.py": {"path": "src/beta.py", "blob_id": "BLB-same", "size_bytes": 3, "mode": "0o644"},
    }
    new_files = {
        "pkg/alpha.py": {"path": "pkg/alpha.py", "blob_id": "BLB-same", "size_bytes": 3, "mode": "0o644"},
    }

    result = snapshot_diff.diff_snapshot_file_maps(old_files, new_files)

    assert result["rename_hints"] == []
    assert result["directory_move_hints"] == []


def test_diff_snapshot_file_maps_groups_unambiguous_directory_moves_from_exact_rename_hints():
    old_files = {
        "src/a.py": {"path": "src/a.py", "blob_id": "BLB-a", "size_bytes": 10, "mode": "0o644"},
        "src/b.py": {"path": "src/b.py", "blob_id": "BLB-b", "size_bytes": 20, "mode": "0o644"},
    }
    new_files = {
        "pkg/a.py": {"path": "pkg/a.py", "blob_id": "BLB-a", "size_bytes": 10, "mode": "0o644"},
        "pkg/b.py": {"path": "pkg/b.py", "blob_id": "BLB-b", "size_bytes": 20, "mode": "0o644"},
    }

    result = snapshot_diff.diff_snapshot_file_maps(old_files, new_files)

    assert [hint["old_path"] for hint in result["rename_hints"]] == ["src/a.py", "src/b.py"]
    assert result["directory_move_hints"] == [
        {
            "match_kind": "exact_blob_id_group",
            "old_parent_path": "src",
            "new_parent_path": "pkg",
            "rename_count": 2,
            "renames": [
                {"old_path": "src/a.py", "new_path": "pkg/a.py", "blob_id": "BLB-a"},
                {"old_path": "src/b.py", "new_path": "pkg/b.py", "blob_id": "BLB-b"},
            ],
        }
    ]
