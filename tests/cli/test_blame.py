from __future__ import annotations

import json
from pathlib import Path

from ait_protocol.common import connect_sqlite

from ait import local_content as local_content_module
from ait import snapshot_blame as snapshot_blame_module
from ait import local_workflow_plans as local_plan_module
from ait.repo_paths import RepoContext
from ait.store import create_local_plan

from ._shared import (
    _set_plan_task_binding_advisory,
    _set_solo_remote_advisory,
    app,
    runner,
    running_server,
)


def test_blame_json_and_scoped_restore_apply(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-local"
    repo.mkdir()
    story = repo / "story.txt"
    story.write_text("one\ntwo\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    first_out = runner.invoke(app, ["snapshot", "create", "--message", "first", "--json"])
    assert first_out.exit_code == 0, first_out.stdout

    story.write_text("one\ntwo changed\nthree\n", encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "second", "--json"])
    assert second_out.exit_code == 0, second_out.stdout
    second_snapshot = json.loads(second_out.stdout)

    story.write_text("scratch\nlocal dirty\nthree dirty\n", encoding="utf-8")

    preview_out = runner.invoke(
        app,
        ["blame", "story.txt", "--start", "2", "--end", "3", "--restore", "--dry-run", "--json"],
        catch_exceptions=False,
    )
    assert preview_out.exit_code == 0, preview_out.stdout
    preview = json.loads(preview_out.stdout)
    assert preview["resolved_snapshot_id"] == second_snapshot["snapshot_id"]
    assert preview["restore"]["source_snapshot_id"] == second_snapshot["snapshot_id"]
    assert preview["restore"]["would_overwrite_selected_local_edits"] is True
    assert preview["restore"]["applied"] is False

    apply_out = runner.invoke(
        app,
        ["blame", "story.txt", "--start", "2", "--end", "3", "--restore", "--json"],
        catch_exceptions=False,
    )
    assert apply_out.exit_code == 0, apply_out.stdout
    applied = json.loads(apply_out.stdout)
    assert applied["restore"]["applied"] is True
    assert story.read_text(encoding="utf-8") == "scratch\ntwo changed\nthree\n"


def test_blame_surfaces_direct_snapshot_task_and_change_provenance(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-provenance"
    repo.mkdir()
    notes = repo / "notes.txt"
    notes.write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    _set_plan_task_binding_advisory()
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout
    assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on"]).exit_code == 0

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Implement blame",
            "--intent",
            "record direct snapshot provenance in the bound worktree",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree_path = Path(payload["worktree"].get("open_path") or payload["worktree"]["path"])

    monkeypatch.chdir(worktree_path)
    notes = worktree_path / "notes.txt"
    notes.write_text("base\nfeature line\n", encoding="utf-8")
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout

    blame_out = runner.invoke(app, ["blame", "notes.txt", "--line", "2", "--json"], catch_exceptions=False)
    assert blame_out.exit_code == 0, blame_out.stdout
    blame = json.loads(blame_out.stdout)
    line_row = blame["lines"][0]
    assert line_row["task_id"] == payload["task_id"]
    assert line_row["change_id"] == payload["change"]["change_id"]
    assert line_row["provenance_confidence"] == "direct_snapshot_binding"


def test_blame_patchset_target_resolves_revision_snapshot(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-patchset"
    repo.mkdir()
    notes = repo / "notes.txt"
    notes.write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-blame-patchset") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_out.exit_code == 0, main_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on"]).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Remote blame patchset",
                "--intent",
                "resolve a patchset review candidate to its revision snapshot",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        worktree_path = Path(payload["worktree"].get("open_path") or payload["worktree"]["path"])

        monkeypatch.chdir(worktree_path)
        (worktree_path / "notes.txt").write_text("base\nfeature line\n", encoding="utf-8")
        revision_out = runner.invoke(app, ["snapshot", "create", "--message", "feature", "--json"], catch_exceptions=False)
        assert revision_out.exit_code == 0, revision_out.stdout
        revision_snapshot = json.loads(revision_out.stdout)

        patchset_out = runner.invoke(
            app,
            [
                "patchset",
                "publish",
                "--change",
                payload["change"]["change_id"],
                "--summary",
                "reviewable blame patchset",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        blame_out = runner.invoke(
            app,
            ["blame", "notes.txt", "--patchset", patchset["patchset_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert blame_out.exit_code == 0, blame_out.stdout
        blame = json.loads(blame_out.stdout)
        assert blame["target"]["kind"] == "patchset"
        assert blame["target"]["patchset_id"] == patchset["patchset_id"]
        assert blame["target"]["revision_snapshot_id"] == revision_snapshot["snapshot_id"]
        assert blame["target"]["resolved_snapshot_id"] == revision_snapshot["snapshot_id"]
        assert blame["lines"][1]["patchset_id"] == patchset["patchset_id"]


def test_blame_surfaces_landed_submission_overlay_for_snapshot_owner(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-landed"
    repo.mkdir()
    notes = repo / "notes.txt"
    notes.write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-blame-landed") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_out.exit_code == 0, main_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on"]).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Remote blame landed overlay",
                "--intent",
                "surface landed workflow overlays for blamed snapshots",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        worktree_path = Path(payload["worktree"].get("open_path") or payload["worktree"]["path"])

        monkeypatch.chdir(worktree_path)
        (worktree_path / "notes.txt").write_text("base\nfeature line\n", encoding="utf-8")
        revision_out = runner.invoke(app, ["snapshot", "create", "--message", "feature", "--json"], catch_exceptions=False)
        assert revision_out.exit_code == 0, revision_out.stdout
        revision_snapshot = json.loads(revision_out.stdout)

        patchset_out = runner.invoke(
            app,
            [
                "patchset",
                "publish",
                "--change",
                payload["change"]["change_id"],
                "--summary",
                "landed blame patchset",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        assert runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"]).exit_code == 0
        assert runner.invoke(
            app,
            ["review", "approve", payload["change"]["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                payload["change"]["change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--reviewer",
                "codex",
                "--message",
                "Reviewed files: notes.txt; Findings: no blocking findings; Risks: low; Tests: pytest; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["policy", "eval", patchset["patchset_id"], "--json"], catch_exceptions=False).exit_code == 0

        land_out = runner.invoke(
            app,
            ["land", "submit", payload["change"]["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        land = json.loads(land_out.stdout)

        blame_out = runner.invoke(
            app,
            ["blame", "notes.txt", "--snapshot", revision_snapshot["snapshot_id"], "--line", "2", "--json"],
            catch_exceptions=False,
        )
        assert blame_out.exit_code == 0, blame_out.stdout
        blame = json.loads(blame_out.stdout)
        line_row = blame["lines"][0]
        assert line_row["patchset_id"] == patchset["patchset_id"]
        assert line_row["submission_id"] == land["submission_id"]
        assert line_row["provenance_confidence"] == "direct_snapshot_binding"


def test_blame_routes_lineage_only_markdown_to_plan_revisions(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-markdown"
    repo.mkdir()
    (repo / "README.txt").write_text("seed\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    plan_file = repo / "docs" / "sprints" / "markdown_blame.md"
    plan_file.parent.mkdir(parents=True, exist_ok=True)
    plan_file.write_text("# Markdown Blame\n\nalpha\n", encoding="utf-8")

    first_sync_out = runner.invoke(app, ["plan", "sync", "docs/sprints/markdown_blame.md", "--json"], catch_exceptions=False)
    assert first_sync_out.exit_code == 0, first_sync_out.stdout
    first_sync = json.loads(first_sync_out.stdout)
    plan_id = first_sync["results"][0]["plan_id"]

    plan_file.write_text("# Markdown Blame\n\nalpha updated\nbeta\n", encoding="utf-8")
    second_sync_out = runner.invoke(app, ["plan", "sync", "docs/sprints/markdown_blame.md", "--json"], catch_exceptions=False)
    assert second_sync_out.exit_code == 0, second_sync_out.stdout
    second_sync = json.loads(second_sync_out.stdout)
    head_revision_id = second_sync["results"][0]["plan_revision_id"]

    blame_out = runner.invoke(app, ["blame", "docs/sprints/markdown_blame.md", "--line", "4", "--json"], catch_exceptions=False)
    assert blame_out.exit_code == 0, blame_out.stdout
    blame = json.loads(blame_out.stdout)

    assert blame["target"]["kind"] == "markdown_plan"
    assert blame["target"]["plan_id"] == plan_id
    assert blame["target"]["resolved_plan_revision_id"] == head_revision_id
    line_row = blame["lines"][0]
    assert line_row["plan_id"] == plan_id
    assert line_row["plan_revision_id"] == head_revision_id
    assert line_row["provenance_confidence"] == "direct_plan_revision_binding"


def test_blame_lineage_only_markdown_refuses_unsynced_local_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-markdown-unsynced"
    repo.mkdir()
    (repo / "README.txt").write_text("seed\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    plan_file = repo / "docs" / "sprints" / "markdown_unsynced.md"
    plan_file.parent.mkdir(parents=True, exist_ok=True)
    plan_file.write_text("# Markdown Unsynced\n\nalpha\n", encoding="utf-8")

    sync_out = runner.invoke(app, ["plan", "sync", "docs/sprints/markdown_unsynced.md", "--json"], catch_exceptions=False)
    assert sync_out.exit_code == 0, sync_out.stdout

    plan_file.write_text("# Markdown Unsynced\n\nalpha drifted locally\n", encoding="utf-8")

    blame_out = runner.invoke(app, ["blame", "docs/sprints/markdown_unsynced.md", "--line", "3", "--json"], catch_exceptions=False)
    assert blame_out.exit_code != 0
    assert "has unsynced local edits relative to local plan head" in blame_out.output
    assert "ait plan sync" in blame_out.output
    assert "docs/sprints/markdown_unsynced.md" in blame_out.output


def test_blame_lineage_only_markdown_accepts_plan_ref_or_plan_id_for_ambiguous_path(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-markdown-selector"
    repo.mkdir()
    (repo / "README.txt").write_text("seed\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    plan_file = repo / "docs" / "shared.md"
    plan_file.parent.mkdir(parents=True, exist_ok=True)
    plan_file.write_text("# Shared Plan\n\nalpha\n", encoding="utf-8")

    ctx = RepoContext.discover(repo)
    artifact_path = "docs/shared.md"
    artifact_blob_id = local_content_module.ensure_blob_bytes(
        ctx,
        plan_file.read_bytes(),
        path_hint=artifact_path,
    )
    generic_plan = create_local_plan(
        ctx,
        "Generic shared plan",
        artifact_path,
        None,
        "Generic shared plan",
        [],
        artifact_blob_id=artifact_blob_id,
    )
    scoped_plan = create_local_plan(
        ctx,
        "Scoped shared plan",
        artifact_path,
        "shared/root",
        "Scoped shared plan",
        [],
        artifact_blob_id=artifact_blob_id,
    )

    ambiguous_out = runner.invoke(app, ["blame", artifact_path, "--line", "3", "--json"])
    assert ambiguous_out.exit_code != 0
    assert "Multiple current plans track lineage-only Markdown path" in ambiguous_out.output
    assert artifact_path in ambiguous_out.output
    assert "--plan-ref" in ambiguous_out.output
    assert "--plan-id" in ambiguous_out.output

    by_ref_out = runner.invoke(
        app,
        ["blame", artifact_path, "--line", "3", "--plan-ref", "shared/root", "--json"],
        catch_exceptions=False,
    )
    assert by_ref_out.exit_code == 0, by_ref_out.stdout
    by_ref = json.loads(by_ref_out.stdout)
    assert by_ref["target"]["plan_id"] == scoped_plan["plan_id"]
    assert by_ref["target"]["plan_ref"] == "shared/root"
    assert by_ref["lines"][0]["plan_revision_id"] == scoped_plan["head_revision"]["plan_revision_id"]

    by_id_out = runner.invoke(
        app,
        ["blame", artifact_path, "--line", "3", "--plan-id", generic_plan["plan_id"], "--json"],
        catch_exceptions=False,
    )
    assert by_id_out.exit_code == 0, by_id_out.stdout
    by_id = json.loads(by_id_out.stdout)
    assert by_id["target"]["plan_id"] == generic_plan["plan_id"]
    assert by_id["target"]["plan_ref"] is None
    assert by_id["lines"][0]["plan_revision_id"] == generic_plan["head_revision"]["plan_revision_id"]


def test_blame_lineage_only_markdown_repairs_missing_local_blob_from_published_remote_revision(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-markdown-repair"
    repo.mkdir()
    (repo / "README.txt").write_text("seed\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-blame-markdown-repair") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = repo / "docs" / "sprints" / "markdown_repair.md"
        plan_file.parent.mkdir(parents=True, exist_ok=True)
        plan_file.write_text("# Markdown Repair\n\nalpha\n", encoding="utf-8")

        first_sync_out = runner.invoke(
            app,
            ["plan", "sync", "docs/sprints/markdown_repair.md", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert first_sync_out.exit_code == 0, first_sync_out.stdout
        first_sync = json.loads(first_sync_out.stdout)
        plan_id = first_sync["results"][0]["plan_id"]

        plan_file.write_text("# Markdown Repair\n\nalpha updated\nbeta\n", encoding="utf-8")
        second_sync_out = runner.invoke(
            app,
            ["plan", "sync", "docs/sprints/markdown_repair.md", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert second_sync_out.exit_code == 0, second_sync_out.stdout
        second_sync = json.loads(second_sync_out.stdout)
        head_revision_id = second_sync["results"][0]["plan_revision_id"]

        ctx = RepoContext.discover(repo)
        local_revisions = local_plan_module.list_workflow_plan_revisions(ctx, plan_id)
        first_local_revision = next(row for row in local_revisions if int(row["revision_number"]) == 1)
        first_local_blob_id = str(first_local_revision["artifact_blob_id"])

        with connect_sqlite(ctx.content_db_path) as conn:
            conn.execute("delete from blobs where blob_id = ?", (first_local_blob_id,))
            conn.commit()

        try:
            local_content_module._read_blob_bytes(ctx, first_local_blob_id)
        except KeyError:
            pass
        else:
            raise AssertionError("expected first published Markdown revision blob to be absent before blame repair")

        blame_out = runner.invoke(
            app,
            ["blame", "docs/sprints/markdown_repair.md", "--line", "4", "--json"],
            catch_exceptions=False,
        )
        assert blame_out.exit_code == 0, blame_out.stdout
        blame = json.loads(blame_out.stdout)
        assert blame["target"]["kind"] == "markdown_plan"
        assert blame["target"]["plan_id"] == plan_id
        assert blame["target"]["resolved_plan_revision_id"] == head_revision_id
        assert blame["lines"][0]["plan_revision_id"] == head_revision_id
        assert local_content_module._read_blob_bytes(ctx, first_local_blob_id).decode("utf-8").startswith(
            "# Markdown Repair"
        )


def test_blame_lineage_only_markdown_skips_unreadable_remote_history_revision(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blame-markdown-degraded"
    repo.mkdir()
    (repo / "README.txt").write_text("seed\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-blame-markdown-degraded") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = repo / "docs" / "sprints" / "markdown_degraded.md"
        plan_file.parent.mkdir(parents=True, exist_ok=True)
        plan_file.write_text("# Markdown Degraded\n\nalpha\n", encoding="utf-8")

        first_sync_out = runner.invoke(
            app,
            ["plan", "sync", "docs/sprints/markdown_degraded.md", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert first_sync_out.exit_code == 0, first_sync_out.stdout
        first_sync = json.loads(first_sync_out.stdout)
        plan_id = first_sync["results"][0]["plan_id"]
        first_local_revision_id = first_sync["results"][0]["plan_revision_id"]

        plan_file.write_text("# Markdown Degraded\n\nalpha revised\n", encoding="utf-8")
        second_sync_out = runner.invoke(
            app,
            ["plan", "sync", "docs/sprints/markdown_degraded.md", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert second_sync_out.exit_code == 0, second_sync_out.stdout
        second_sync = json.loads(second_sync_out.stdout)
        second_local_revision_id = second_sync["results"][0]["plan_revision_id"]

        ctx = RepoContext.discover(repo)
        local_revisions = local_plan_module.list_workflow_plan_revisions(ctx, plan_id)
        first_local_revision = next(row for row in local_revisions if row["plan_revision_id"] == first_local_revision_id)
        first_local_blob_id = str(first_local_revision["artifact_blob_id"])

        with connect_sqlite(ctx.content_db_path) as conn:
            conn.execute("delete from blobs where blob_id = ?", (first_local_blob_id,))
            conn.commit()

        try:
            local_content_module._read_blob_bytes(ctx, first_local_blob_id)
        except KeyError:
            pass
        else:
            raise AssertionError("expected first published Markdown revision blob to be absent before degraded blame")

        original_remote_get_plan_revision = snapshot_blame_module.remote_get_plan_revision

        def _remote_revision_without_body(base_url_value: str, remote_plan_id: str, remote_revision_id: str) -> dict:
            payload = dict(original_remote_get_plan_revision(base_url_value, remote_plan_id, remote_revision_id))
            payload.pop("artifact_body", None)
            return payload

        monkeypatch.setattr(snapshot_blame_module, "remote_get_plan_revision", _remote_revision_without_body)

        blame_out = runner.invoke(
            app,
            ["blame", "docs/sprints/markdown_degraded.md", "--line", "3", "--json"],
            catch_exceptions=False,
        )
        assert blame_out.exit_code == 0, blame_out.stdout
        blame = json.loads(blame_out.stdout)
        assert blame["target"]["kind"] == "markdown_plan"
        assert blame["target"]["plan_id"] == plan_id
        assert blame["target"]["resolved_plan_revision_id"] == second_local_revision_id
        assert blame["lines"][0]["plan_revision_id"] == second_local_revision_id
        assert blame["warnings"] == [
            {
                "kind": "missing_markdown_revision_body",
                "plan_revision_id": first_local_revision_id,
                "message": (
                    f"Skipped unreadable historical plan revision {first_local_revision_id} "
                    "while attributing lineage-only Markdown path docs/sprints/markdown_degraded.md."
                ),
            }
        ]
        try:
            local_content_module._read_blob_bytes(ctx, first_local_blob_id)
        except KeyError:
            pass
        else:
            raise AssertionError("expected degraded blame to leave the unreadable historical blob absent")
