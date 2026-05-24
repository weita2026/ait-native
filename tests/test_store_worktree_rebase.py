from __future__ import annotations

from pathlib import Path

from ait import store_worktree_rebase
from ait.repo_paths import RepoContext


def test_compute_worktree_rebase_plan_resolves_conflicts_and_same_results(monkeypatch) -> None:
    snapshot_files = {
        "base": {
            "shared.txt": {"blob_id": "base-shared", "mode": "0o644", "size_bytes": 10},
            "remove.txt": {"blob_id": "remove-me", "mode": "0o644", "size_bytes": 9},
            "same.txt": {"blob_id": "same-base", "mode": "0o644", "size_bytes": 8},
        },
        "head": {
            "shared.txt": {"blob_id": "head-shared", "mode": "0o644", "size_bytes": 11},
            "same.txt": {"blob_id": "same-head", "mode": "0o644", "size_bytes": 12},
            "feature-only.txt": {"blob_id": "feature-only", "mode": "0o644", "size_bytes": 5},
        },
        "target": {
            "shared.txt": {"blob_id": "target-shared", "mode": "0o644", "size_bytes": 13},
            "remove.txt": {"blob_id": "remove-me", "mode": "0o644", "size_bytes": 9},
            "same.txt": {"blob_id": "same-head", "mode": "0o644", "size_bytes": 12},
        },
    }

    monkeypatch.setattr(
        store_worktree_rebase,
        "_snapshot_file_map_for_id",
        lambda _ctx, snapshot_id: snapshot_files[str(snapshot_id)],
    )

    plan = store_worktree_rebase._compute_worktree_rebase_plan(
        RepoContext(
            root=Path("/tmp/repo"),
            ait_dir=Path("/tmp/repo/.ait"),
            content_db_path=Path("/tmp/repo/.ait/content.db"),
            control_db_path=Path("/tmp/repo/.ait/control.db"),
            config_path=Path("/tmp/repo/.ait/config.json"),
        ),
        line_name="feature",
        old_base_snapshot_id="base",
        old_head_snapshot_id="head",
        new_base_snapshot_id="target",
        onto_line_name="main",
    )

    assert plan["feature_delta_count"] == 4
    assert plan["conflict_count"] == 1
    assert plan["conflict_paths"] == ["shared.txt"]
    assert plan["apply_write_paths"] == ["feature-only.txt"]
    assert plan["apply_remove_paths"] == ["remove.txt"]
    assert plan["would_fast_forward"] is False

    by_path = {row["path"]: row for row in plan["files"]}
    assert by_path["feature-only.txt"]["resolution"] == "feature"
    assert by_path["feature-only.txt"]["apply_status"] == "write"
    assert by_path["remove.txt"]["resolution"] == "feature"
    assert by_path["remove.txt"]["apply_status"] == "remove"
    assert by_path["same.txt"]["resolution"] == "same_result"
    assert by_path["same.txt"]["apply_status"] == "unchanged"
    assert by_path["shared.txt"]["resolution"] == "conflict"
    assert by_path["shared.txt"]["apply_status"] == "conflict"
