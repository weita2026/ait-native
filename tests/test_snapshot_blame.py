from __future__ import annotations

from pathlib import Path

import pytest

from ait import local_control
from ait import snapshot_blame
from ait import store
from ait.repo_paths import RepoContext


def test_snapshot_blame_tracks_line_ownership_and_ignores_mode_only_changes(tmp_path: Path):
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    target = repo / "story.txt"
    target.write_text("one\ntwo\n", encoding="utf-8")
    first = store.create_snapshot(ctx, "first")

    target.write_text("one\ntwo changed\nthree\n", encoding="utf-8")
    second = store.create_snapshot(ctx, "second")

    target.chmod(0o755)
    third = store.create_snapshot(ctx, "mode only")

    result = snapshot_blame.compute_snapshot_blame(
        ctx,
        "story.txt",
        target={"kind": "snapshot", "resolved_snapshot_id": third["snapshot_id"]},
    )

    assert result["resolved_snapshot_id"] == third["snapshot_id"]
    assert result["range"] == {"start": 1, "end": 3}
    assert [row["snapshot_id"] for row in result["lines"]] == [
        first["snapshot_id"],
        second["snapshot_id"],
        second["snapshot_id"],
    ]
    assert result["hunks"] == [
        {
            "path": "story.txt",
            "start_line": 1,
            "end_line": 1,
            "snapshot_id": first["snapshot_id"],
            "parent_snapshot_id": first["parent_snapshot_id"],
            "line_name": first["line_name"],
            "message": first["message"],
            "created_at": first["created_at"],
            "task_id": None,
            "change_id": None,
            "patchset_id": None,
            "land_id": None,
            "submission_id": None,
            "session_id": None,
            "checkpoint_id": None,
            "author_mode": None,
            "model_name": None,
            "worktree_name": None,
            "provenance_confidence": snapshot_blame.PROVENANCE_CONFIDENCE_UNKNOWN,
        },
        {
            "path": "story.txt",
            "start_line": 2,
            "end_line": 3,
            "snapshot_id": second["snapshot_id"],
            "parent_snapshot_id": second["parent_snapshot_id"],
            "line_name": second["line_name"],
            "message": second["message"],
            "created_at": second["created_at"],
            "task_id": None,
            "change_id": None,
            "patchset_id": None,
            "land_id": None,
            "submission_id": None,
            "session_id": None,
            "checkpoint_id": None,
            "author_mode": None,
            "model_name": None,
            "worktree_name": None,
            "provenance_confidence": snapshot_blame.PROVENANCE_CONFIDENCE_UNKNOWN,
        },
    ]


def test_snapshot_blame_scoped_restore_refuses_multi_owner_and_applies_single_owner(tmp_path: Path):
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    target = repo / "story.txt"
    target.write_text("one\ntwo\n", encoding="utf-8")
    store.create_snapshot(ctx, "first")

    target.write_text("one\ntwo changed\nthree\n", encoding="utf-8")
    final_snapshot = store.create_snapshot(ctx, "second")

    target.write_text("scratch\nlocal dirty\nthree dirty\n", encoding="utf-8")

    with pytest.raises(ValueError, match="spans multiple owning snapshots"):
        snapshot_blame.preview_scoped_restore(
            ctx,
            "story.txt",
            target={"kind": "snapshot", "resolved_snapshot_id": final_snapshot["snapshot_id"]},
            start_line=1,
            end_line=2,
        )

    preview = snapshot_blame.preview_scoped_restore(
        ctx,
        "story.txt",
        target={"kind": "snapshot", "resolved_snapshot_id": final_snapshot["snapshot_id"]},
        start_line=2,
        end_line=3,
    )
    assert preview["source_snapshot_id"] == final_snapshot["snapshot_id"]
    assert preview["would_overwrite_selected_local_edits"] is True
    assert preview["applied"] is False

    applied = snapshot_blame.apply_scoped_restore(
        ctx,
        "story.txt",
        target={"kind": "snapshot", "resolved_snapshot_id": final_snapshot["snapshot_id"]},
        start_line=2,
        end_line=3,
    )
    assert applied["applied"] is True
    assert target.read_text(encoding="utf-8") == "scratch\ntwo changed\nthree\n"


def test_snapshot_create_records_direct_workflow_snapshot_provenance_in_bound_worktree(tmp_path: Path):
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    target = repo / "story.txt"
    target.write_text("base\n", encoding="utf-8")
    seed = store.create_snapshot(ctx, "seed")

    store.create_line(ctx, "feature/t-1", from_snapshot=seed["snapshot_id"])
    local_control.create_workflow_task(
        ctx,
        "T-1",
        "repo",
        "Implement blame",
        "capture direct snapshot provenance",
        "medium",
    )
    local_control.create_workflow_change(
        ctx,
        "C-1",
        "T-1",
        "repo",
        "Implement blame change",
        "main",
        "medium",
        "assisted",
    )
    worktree_path = tmp_path / "wt"
    store.add_worktree(ctx, "task-t-1", line_name="feature/t-1", path=str(worktree_path))
    store.bind_worktree(
        ctx,
        "task-t-1",
        task_id="T-1",
        change_id="C-1",
        auto_created_for_task=True,
        fork_snapshot_id=seed["snapshot_id"],
        forked_from_line="main",
        target_base_line="main",
    )

    local_control.create_workflow_session(
        ctx,
        "S-1",
        "repo",
        "agent_run",
        task_id="T-1",
        change_id="C-1",
        line_name="feature/t-1",
        worktree_name="task-t-1",
        model_name="gpt-test",
        metadata={"author_mode": "ai_with_human_review"},
    )
    local_control.create_workflow_checkpoint(ctx, "CP-1", "S-1", "ready to snapshot")

    worktree_ctx = RepoContext.discover(worktree_path)
    worktree_target = worktree_path / "story.txt"
    worktree_target.write_text("base\nfeature\n", encoding="utf-8")
    snapshot = store.create_snapshot(worktree_ctx, "feature snapshot")

    provenance = local_control.get_workflow_snapshot_provenance(ctx, snapshot["snapshot_id"])
    assert provenance["task_id"] == "T-1"
    assert provenance["change_id"] == "C-1"
    assert provenance["session_id"] == "S-1"
    assert provenance["checkpoint_id"] == "CP-1"
    assert provenance["worktree_name"] == "task-t-1"
    assert provenance["line_name"] == "feature/t-1"
    assert provenance["author_mode"] == "ai_with_human_review"
    assert provenance["model_name"] == "gpt-test"


def test_snapshot_blame_current_line_skips_remote_overlay_for_direct_provenance(tmp_path: Path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    target = repo / "story.txt"
    target.write_text("base\n", encoding="utf-8")
    seed = store.create_snapshot(ctx, "seed")

    store.create_line(ctx, "feature/t-3", from_snapshot=seed["snapshot_id"])
    local_control.create_workflow_task(
        ctx,
        "T-3",
        "repo",
        "Fast current-line blame",
        "keep direct provenance lookups local",
        "medium",
    )
    local_control.create_workflow_change(
        ctx,
        "C-3",
        "T-3",
        "repo",
        "Fast current-line blame",
        "main",
        "medium",
        "assisted",
    )
    worktree_path = tmp_path / "wt"
    store.add_worktree(ctx, "task-t-3", line_name="feature/t-3", path=str(worktree_path))
    store.bind_worktree(
        ctx,
        "task-t-3",
        task_id="T-3",
        change_id="C-3",
        auto_created_for_task=True,
        fork_snapshot_id=seed["snapshot_id"],
        forked_from_line="main",
        target_base_line="main",
    )

    worktree_ctx = RepoContext.discover(worktree_path)
    worktree_target = worktree_path / "story.txt"
    worktree_target.write_text("base\nfeature\n", encoding="utf-8")
    snapshot = store.create_snapshot(worktree_ctx, "feature snapshot")

    def _unexpected_remote_overlay(*args, **kwargs):
        raise AssertionError("current-line blame should not need remote overlay for direct provenance")

    monkeypatch.setattr(snapshot_blame, "_remote_change_overlay", _unexpected_remote_overlay)

    result = snapshot_blame.compute_snapshot_blame(
        ctx,
        "story.txt",
        target={"kind": "current_line", "line_name": "feature/t-3", "resolved_snapshot_id": snapshot["snapshot_id"]},
        line=2,
    )

    line_row = result["lines"][0]
    assert line_row["snapshot_id"] == snapshot["snapshot_id"]
    assert line_row["task_id"] == "T-3"
    assert line_row["change_id"] == "C-3"
    assert line_row["provenance_confidence"] == snapshot_blame.PROVENANCE_CONFIDENCE_DIRECT
    assert line_row["patchset_id"] is None
    assert line_row["submission_id"] is None


def test_snapshot_blame_falls_back_to_landed_change_context_without_direct_provenance(tmp_path: Path):
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    target = repo / "story.txt"
    target.write_text("landed\n", encoding="utf-8")
    landed_snapshot = store.create_snapshot(ctx, "landed snapshot")

    local_control.create_workflow_task(
        ctx,
        "T-2",
        "repo",
        "Backfill landed blame context",
        "infer snapshot ownership from landed change state",
        "medium",
    )
    local_control.create_workflow_change(
        ctx,
        "C-2",
        "T-2",
        "repo",
        "Backfill landed blame context",
        "main",
        "medium",
        "assisted",
    )
    local_control.land_workflow_change(
        ctx,
        "C-2",
        target_line="main",
        landed_snapshot_id=landed_snapshot["snapshot_id"],
    )

    result = snapshot_blame.compute_snapshot_blame(
        ctx,
        "story.txt",
        target={"kind": "snapshot", "resolved_snapshot_id": landed_snapshot["snapshot_id"]},
    )

    line_row = result["lines"][0]
    assert line_row["task_id"] == "T-2"
    assert line_row["change_id"] == "C-2"
    assert line_row["patchset_id"] is None
    assert line_row["submission_id"] is None
    assert line_row["provenance_confidence"] == snapshot_blame.PROVENANCE_CONFIDENCE_LAND


def test_snapshot_blame_reuses_blob_reads_for_unchanged_file_snapshots(tmp_path: Path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    story = repo / "story.txt"
    other = repo / "other.txt"

    story.write_text("stable\n", encoding="utf-8")
    first = store.create_snapshot(ctx, "first")

    other.write_text("one\n", encoding="utf-8")
    store.create_snapshot(ctx, "second")

    other.write_text("two\n", encoding="utf-8")
    third = store.create_snapshot(ctx, "third")

    calls: list[str] = []
    original = snapshot_blame.local_content._blob_bytes_by_id

    def counting_blob_bytes(ctx_arg, conn, blob_id, *, seen_blob_ids=None):
        calls.append(str(blob_id))
        return original(ctx_arg, conn, blob_id, seen_blob_ids=seen_blob_ids)

    monkeypatch.setattr(snapshot_blame.local_content, "_blob_bytes_by_id", counting_blob_bytes)

    result = snapshot_blame.compute_snapshot_blame(
        ctx,
        "story.txt",
        target={"kind": "snapshot", "resolved_snapshot_id": third["snapshot_id"]},
        line=1,
    )

    assert result["lines"][0]["snapshot_id"] == first["snapshot_id"]
    assert len(calls) == 1
