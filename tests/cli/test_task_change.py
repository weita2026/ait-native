from __future__ import annotations

import importlib

import ait_native.local_content as native_local_content
import ait_native.local_control as native_local_control
from ._shared import *  # noqa: F401,F403
from ait_native.store import set_line_head as local_set_line_head
from ait_native.store import create_snapshot as local_create_snapshot

task_cli_module = importlib.import_module("ait.cli.commands.task")

def test_queue_summary_all_changes_expands_open_shared_change_inventory(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-queue-all-changes"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-queue-all-changes") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Audit all queue changes", "--intent", "exercise all-changes inventory", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task_payload = json.loads(task_out.stdout)
        _bind_task_worktree(task_payload["task_id"], monkeypatch, name="queue-all-changes")

        change_out = runner.invoke(
            app,
            [
                "change",
                "create",
                "--task",
                task_payload["task_id"],
                "--title",
                "Prepare shared queue inventory",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change_payload = json.loads(change_out.stdout)
        monkeypatch.chdir(repo)

        summary_out = runner.invoke(app, ["queue", "summary", "--all-changes", "--json"], catch_exceptions=False)
        assert summary_out.exit_code == 0, summary_out.stdout
        payload = json.loads(summary_out.stdout)

        assert payload["query"]["all_changes"] is True
        assert payload["summary"]["open_shared_change_count"] == 1
        assert payload["remote"]["changes"][0]["change_id"] == change_payload["change_id"]
        assert payload["remote"]["changes"][0]["status"] == "draft"
        assert payload["remote"]["changes"][0]["reason"] == "No published patchset exists yet."

        summary_text_out = runner.invoke(app, ["queue", "summary", "--all-changes"], catch_exceptions=False)
        assert summary_text_out.exit_code == 0, summary_text_out.stdout
        output = summary_text_out.output or summary_text_out.stdout
        assert "shared changes" in output
        assert change_payload["change_id"] in output
        assert "No published" in output
        assert "exists yet." in output


def test_queue_summary_all_changes_reports_missing_attestation_without_task_queue_focus(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-queue-missing-attestation"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    app_file = repo / "app.py"
    app_file.write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-queue-missing-attestation") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Review missing attestation", "--intent", "exercise all-changes fallback reasoning", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task_payload = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task_payload["task_id"], monkeypatch, name="queue-missing-attestation")

        assert runner.invoke(app, ["line", "create", "feature/missing-attestation"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/missing-attestation"], catch_exceptions=False).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout

        change_out = runner.invoke(
            app,
            [
                "change",
                "create",
                "--task",
                task_payload["task_id"],
                "--title",
                "Publish without attestation",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change_payload = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change_payload["change_id"], "--summary", "missing attestation patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout

        summary_out = runner.invoke(
            app,
            ["queue", "summary", "--all-changes", "--status", "completed", "--json"],
            catch_exceptions=False,
        )
        assert summary_out.exit_code == 0, summary_out.stdout
        payload = json.loads(summary_out.stdout)

        assert payload["summary"]["shared_task_count"] == 0
        assert payload["summary"]["open_shared_change_count"] == 1
        assert payload["remote"]["changes"][0]["change_id"] == change_payload["change_id"]
        assert payload["remote"]["changes"][0]["ready_to_land"] is False
        assert payload["remote"]["changes"][0]["reason"] == "Attestation is missing for the current patchset."

        summary_text_out = runner.invoke(
            app,
            ["queue", "summary", "--all-changes", "--status", "completed"],
            catch_exceptions=False,
        )
        assert summary_text_out.exit_code == 0, summary_text_out.stdout
        output = summary_text_out.output or summary_text_out.stdout
        assert "shared changes" in output
        assert change_payload["change_id"] in output


def test_task_and_change_create_remain_mode_safe_across_three_workflow_modes(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-create-mode-safety"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-create-mode-safety") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0

        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        local_task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Local execution node", "--intent", "keep node local first", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert local_task_out.exit_code == 0, local_task_out.stdout
        local_task = json.loads(local_task_out.stdout)
        local_task_show = runner.invoke(app, ["task", "show", local_task["task_id"], "--json"], catch_exceptions=False)
        assert local_task_show.exit_code == 0, local_task_show.stdout
        assert json.loads(local_task_show.stdout)["publication_state"] == "local_draft"
        _bind_task_worktree(local_task["task_id"], monkeypatch, name="mode-safe-local")
        local_change_out = runner.invoke(
            app,
            ["change", "create", "--task", local_task["task_id"], "--title", "Promote local slice", "--base-line", "main", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert local_change_out.exit_code == 0, local_change_out.stdout
        local_change = json.loads(local_change_out.stdout)
        monkeypatch.chdir(repo)
        local_change_show = runner.invoke(app, ["change", "show", local_change["change_id"], "--json"], catch_exceptions=False)
        assert local_change_show.exit_code == 0, local_change_show.stdout
        shown_local_change = json.loads(local_change_show.stdout)
        assert shown_local_change["publication_state"] == "local_draft"
        assert shown_local_change["forked_from_line"] == "main"

        _set_solo_remote_advisory()
        remote_task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Remote solo task", "--intent", "use remote backed solo defaults", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert remote_task_out.exit_code == 0, remote_task_out.stdout
        remote_task = json.loads(remote_task_out.stdout)
        remote_task_show = runner.invoke(app, ["task", "show", remote_task["task_id"], "--json"], catch_exceptions=False)
        assert remote_task_show.exit_code == 0, remote_task_show.stdout
        assert json.loads(remote_task_show.stdout)["task_id"] == remote_task["task_id"]
        _bind_task_worktree(remote_task["task_id"], monkeypatch, name="mode-safe-remote")
        remote_change_out = runner.invoke(
            app,
            ["change", "create", "--task", remote_task["task_id"], "--title", "Remote solo change", "--base-line", "main", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        monkeypatch.chdir(repo)
        remote_change_show = runner.invoke(app, ["change", "show", remote_change["change_id"], "--json"], catch_exceptions=False)
        assert remote_change_show.exit_code == 0, remote_change_show.stdout
        shown_remote_change = json.loads(remote_change_show.stdout)
        assert shown_remote_change["change_id"] == remote_change["change_id"]
        assert shown_remote_change["forked_from_line"] == "main"

        assert runner.invoke(app, ["config", "set", "--workflow-mode", "team_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        team_task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Team remote task", "--intent", "exercise required remote mode", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert team_task_out.exit_code == 0, team_task_out.stdout
        team_task = json.loads(team_task_out.stdout)
        team_task_show = runner.invoke(app, ["task", "show", team_task["task_id"], "--json"], catch_exceptions=False)
        assert team_task_show.exit_code == 0, team_task_show.stdout
        assert json.loads(team_task_show.stdout)["task_id"] == team_task["task_id"]
        _bind_task_worktree(team_task["task_id"], monkeypatch, name="mode-safe-team")
        team_change_out = runner.invoke(
            app,
            ["change", "create", "--task", team_task["task_id"], "--title", "Team remote change", "--base-line", "main", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert team_change_out.exit_code == 0, team_change_out.stdout
        team_change = json.loads(team_change_out.stdout)
        monkeypatch.chdir(repo)
        team_change_show = runner.invoke(app, ["change", "show", team_change["change_id"], "--json"], catch_exceptions=False)
        assert team_change_show.exit_code == 0, team_change_show.stdout
        assert json.loads(team_change_show.stdout)["change_id"] == team_change["change_id"]


def test_task_start_task_only_keeps_single_public_entrypoint_without_opening_change(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-task-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--task-only",
            "--title",
            "Task-only start",
            "--intent",
            "keep task start as the only taught entrypoint",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)

    assert payload["task_id"]
    assert "change" not in payload
    assert payload["worktree"]["bound_change_id"] is None
    assert payload["worktree"]["bound_task_id"] == payload["task_id"]

    task_show_out = runner.invoke(app, ["task", "show", payload["task_id"], "--local", "--json"], catch_exceptions=False)
    assert task_show_out.exit_code == 0, task_show_out.stdout
    assert json.loads(task_show_out.stdout)["status"] == "active"


def test_task_reads_and_change_close_remain_mode_safe_across_three_workflow_modes(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-read-close-mode-safety"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-read-close-mode-safety") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0

        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        local_task = json.loads(
            runner.invoke(
                app,
                ["task", "start", "--task-only", "--title", "Local read task", "--intent", "inspect local inventory", "--risk", "low", "--json"],
                catch_exceptions=False,
            ).stdout
        )
        _bind_task_worktree(local_task["task_id"], monkeypatch, name="read-close-local")
        local_change = json.loads(
            runner.invoke(
                app,
                ["change", "create", "--task", local_task["task_id"], "--title", "Local close change", "--base-line", "main", "--risk", "low", "--json"],
                catch_exceptions=False,
            ).stdout
        )
        monkeypatch.chdir(repo)
        local_tasks = json.loads(runner.invoke(app, ["task", "list", "--json"], catch_exceptions=False).stdout)
        assert any(row["task_id"] == local_task["task_id"] for row in local_tasks)
        local_changes = json.loads(runner.invoke(app, ["change", "list", "--json"], catch_exceptions=False).stdout)
        assert any(row["change_id"] == local_change["change_id"] for row in local_changes)
        local_audit = json.loads(runner.invoke(app, ["task", "audit", local_task["task_id"], "--json"], catch_exceptions=False).stdout)
        assert local_audit["task"]["task_id"] == local_task["task_id"]
        local_close = json.loads(runner.invoke(app, ["change", "close", local_change["change_id"], "--json"], catch_exceptions=False).stdout)
        assert local_close["status"] == "archived"

        _set_solo_remote_advisory()
        remote_task = json.loads(
            runner.invoke(
                app,
                ["task", "start", "--task-only", "--title", "Remote read task", "--intent", "inspect remote inventory", "--risk", "low", "--json"],
                catch_exceptions=False,
            ).stdout
        )
        _bind_task_worktree(remote_task["task_id"], monkeypatch, name="read-close-remote")
        remote_change = json.loads(
            runner.invoke(
                app,
                ["change", "create", "--task", remote_task["task_id"], "--title", "Remote close change", "--base-line", "main", "--risk", "low", "--json"],
                catch_exceptions=False,
            ).stdout
        )
        monkeypatch.chdir(repo)
        remote_tasks = json.loads(runner.invoke(app, ["task", "list", "--json"], catch_exceptions=False).stdout)
        assert any(row["task_id"] == remote_task["task_id"] for row in remote_tasks)
        remote_changes = json.loads(runner.invoke(app, ["change", "list", "--json"], catch_exceptions=False).stdout)
        assert any(row["change_id"] == remote_change["change_id"] for row in remote_changes)
        remote_audit = json.loads(runner.invoke(app, ["task", "audit", remote_task["task_id"], "--json"], catch_exceptions=False).stdout)
        assert remote_audit["task"]["task_id"] == remote_task["task_id"]
        remote_close = json.loads(runner.invoke(app, ["change", "close", remote_change["change_id"], "--json"], catch_exceptions=False).stdout)
        assert remote_close["status"] == "archived"

        assert runner.invoke(app, ["config", "set", "--workflow-mode", "team_remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        team_task = json.loads(
            runner.invoke(
                app,
                ["task", "start", "--task-only", "--title", "Team read task", "--intent", "inspect team inventory", "--risk", "low", "--json"],
                catch_exceptions=False,
            ).stdout
        )
        _bind_task_worktree(team_task["task_id"], monkeypatch, name="read-close-team")
        team_change = json.loads(
            runner.invoke(
                app,
                ["change", "create", "--task", team_task["task_id"], "--title", "Team close change", "--base-line", "main", "--risk", "low", "--json"],
                catch_exceptions=False,
            ).stdout
        )
        monkeypatch.chdir(repo)
        team_tasks = json.loads(runner.invoke(app, ["task", "list", "--json"], catch_exceptions=False).stdout)
        assert any(row["task_id"] == team_task["task_id"] for row in team_tasks)
        team_changes = json.loads(runner.invoke(app, ["change", "list", "--json"], catch_exceptions=False).stdout)
        assert any(row["change_id"] == team_change["change_id"] for row in team_changes)
        team_audit = json.loads(runner.invoke(app, ["task", "audit", team_task["task_id"], "--json"], catch_exceptions=False).stdout)
        assert team_audit["task"]["task_id"] == team_task["task_id"]
        team_close = json.loads(runner.invoke(app, ["change", "close", team_change["change_id"], "--json"], catch_exceptions=False).stdout)
        assert team_close["status"] == "archived"


def test_change_close_archives_blocked_remote_change_and_clears_reviewer_inbox(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-blocked-change-close"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-blocked-change-close") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Archive blocked remote change", "--intent", "remove abandoned blocked change from inbox", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="blocked-change-close")

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Blocked remote change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        assert runner.invoke(app, ["line", "create", "feature/blocked-change-close"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/blocked-change-close"], catch_exceptions=False).exit_code == 0
        (workspace / "app.py").write_text("print('blocked')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "blocked work", "--json"], catch_exceptions=False)
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "blocked remote change", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        review_out = runner.invoke(
            app,
            [
                "review",
                "request-changes",
                change["change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--reviewer",
                "reviewer@example.com",
                "--message",
                "needs changes",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert review_out.exit_code == 0, review_out.stdout

        show_out = runner.invoke(app, ["change", "show", change["change_id"], "--json"], catch_exceptions=False)
        assert show_out.exit_code == 0, show_out.stdout
        assert json.loads(show_out.stdout)["status"] == "blocked"

        inbox_before = json.loads(runner.invoke(app, ["queue", "summary", "--all-changes", "--json"], catch_exceptions=False).stdout)
        before_items = ((inbox_before.get("remote") or {}).get("reviewer_inbox") or {}).get("items") or []
        assert any(item.get("change_id") == change["change_id"] for item in before_items)

        close_out = runner.invoke(app, ["change", "close", change["change_id"], "--json"], catch_exceptions=False)
        assert close_out.exit_code == 0, close_out.stdout
        assert json.loads(close_out.stdout)["status"] == "archived"

        inbox_after = json.loads(runner.invoke(app, ["queue", "summary", "--all-changes", "--json"], catch_exceptions=False).stdout)
        after_items = ((inbox_after.get("remote") or {}).get("reviewer_inbox") or {}).get("items") or []
        assert all(item.get("change_id") != change["change_id"] for item in after_items)


def test_native_task_change_and_patchset_publish(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-2") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        main_snapshot = json.loads(main_snap_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Adopt native ait", "--intent", "bootstrap housekeeper", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        assert task["task_id"].startswith("RAITT-")
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="native-task-change")

        assert runner.invoke(app, ["line", "create", "feature/bootstrap"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/bootstrap"], catch_exceptions=False).exit_code == 0
        (workspace / "notes.txt").write_text("native patchset\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
        feature_snapshot = json.loads(feature_snap_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Bootstrap native workflow", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)
        assert change["change_id"].startswith("RAITC-")

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "initial reviewable native patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["revision_snapshot_id"] == feature_snapshot["snapshot_id"]
        assert patchset["base_snapshot_id"] == main_snapshot["snapshot_id"]
        assert patchset["patchset_number"] == 1
        assert patchset["patchset_id"].startswith("RAITP-")

        patchset_show = runner.invoke(app, ["patchset", "show", patchset["patchset_id"], "--json"], catch_exceptions=False)
        assert patchset_show.exit_code == 0, patchset_show.stdout
        shown = json.loads(patchset_show.stdout)
        assert shown["diff_stats"]["files_changed"] >= 1


def test_patchset_publish_refuses_when_bound_worktree_needs_retarget(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-bound-worktree-retarget"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-bound-worktree-retarget") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Remote rebase guard",
                "--intent",
                "refuse publishing from a stale task worktree",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        change = payload["change"]
        worktree = payload["worktree"]
        worktree_path = Path(worktree["path"])

        monkeypatch.chdir(worktree_path)
        (worktree_path / "feature.txt").write_text("feature only\n", encoding="utf-8")
        feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature side", "--json"], catch_exceptions=False)
        assert feature_out.exit_code == 0, feature_out.stdout

        monkeypatch.chdir(repo)
        (repo / "README.md").write_text("base\nmain advanced\n", encoding="utf-8")
        repo_ctx = RepoContext.discover(repo)
        local_create_snapshot(repo_ctx, "main advance")

        monkeypatch.chdir(worktree_path)
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        publish_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "should be blocked"],
            catch_exceptions=False,
        )
        assert publish_out.exit_code != 0
        output = publish_out.output or publish_out.stdout or ""
        assert "worktree rebase --onto main" in output
        assert "before publishing" in output


def test_task_show_surfaces_bound_worktree_rebase_advisory_when_stale(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-show-retarget"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-task-show-retarget") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Remote task show rebase advisory",
                "--intent",
                "surface stale worktree guidance in task show",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        worktree = payload["worktree"]
        worktree_path = Path(worktree["path"])

        monkeypatch.chdir(worktree_path)
        (worktree_path / "feature.txt").write_text("feature only\n", encoding="utf-8")
        feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature side", "--json"], catch_exceptions=False)
        assert feature_out.exit_code == 0, feature_out.stdout

        monkeypatch.chdir(repo)
        (repo / "README.md").write_text("base\nmain advanced\n", encoding="utf-8")
        repo_ctx = RepoContext.discover(repo)
        local_create_snapshot(repo_ctx, "main advance")

        task_show_out = runner.invoke(app, ["task", "show", payload["task_id"], "--json"], catch_exceptions=False)
        assert task_show_out.exit_code == 0, task_show_out.stdout
        shown = json.loads(task_show_out.stdout)

        assert shown["worktree"]["name"] == worktree["name"]
        assert shown["worktree"]["needs_retarget"] is True
        advisory = shown["worktree_advisory"]
        assert advisory["code"] == "needs_retarget"
        assert advisory["status"] == "stale"
        assert advisory["target_base_line"] == "main"
        assert advisory["command"] == "ait worktree rebase --onto main"
        assert advisory["fork_snapshot_id"] == shown["worktree"]["retarget"]["fork_snapshot_id"]
        assert advisory["target_base_snapshot_id"] == shown["worktree"]["retarget"]["target_base_snapshot_id"]
        assert "still forks from" in advisory["detail"]


def test_patchset_publish_rejects_foreign_snapshot_lineage_for_bound_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-patchset-scope-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-patchset-scope-guard") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Patchset snapshot scope guard",
                "--intent",
                "reject foreign lineage before publishing",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        change = payload["change"]
        worktree = payload["worktree"]
        feature_line_name = str(worktree["current_line"])
        worktree_path = Path(worktree["path"])

        monkeypatch.chdir(worktree_path)
        worktree_ctx = RepoContext.discover(worktree_path)
        (worktree_path / "feature.txt").write_text("foreign lineage\n", encoding="utf-8")
        foreign_snapshot = native_local_content.create_snapshot(
            worktree_ctx,
            "housekeeper",
            feature_line_name,
            "foreign lineage",
            parent_snapshot_id=str(worktree["head_snapshot_id"]),
        )
        local_set_line_head(worktree_ctx, feature_line_name, str(foreign_snapshot["snapshot_id"]))

        publish_out = runner.invoke(
            app,
            [
                "patchset",
                "publish",
                "--change",
                change["change_id"],
                "--summary",
                "should be blocked by snapshot scope guard",
            ],
            catch_exceptions=False,
        )
        assert publish_out.exit_code != 0
        output = publish_out.output or publish_out.stdout
        assert "not owned by bound task" in output
        assert "Restore or reopen the correct task worktree" in output


def test_worktree_restore_owned_head_rewinds_to_last_clean_owned_snapshot(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-restore-owned-head"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    _set_plan_task_binding_advisory()

    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Restore owned head",
            "--intent",
            "rewind a contaminated task worktree to the last owned snapshot",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]
    worktree_name = str(worktree["name"])
    worktree_path = Path(worktree["path"])

    worktree_ctx = RepoContext.discover(worktree_path)
    monkeypatch.chdir(worktree_path)
    (worktree_path / "feature.txt").write_text("owned\n", encoding="utf-8")
    owned_snapshot = local_create_snapshot(worktree_ctx, "owned feature snapshot")
    (worktree_path / "feature.txt").write_text("foreign\n", encoding="utf-8")
    foreign_snapshot = local_create_snapshot(worktree_ctx, "foreign feature snapshot")
    native_local_control.record_workflow_snapshot_provenance(
        worktree_ctx,
        foreign_snapshot["snapshot_id"],
        task_id="LT-foreign",
        change_id="LC-foreign",
        worktree_name="lt-foreign",
        line_name=str(worktree["current_line"]),
        created_at=foreign_snapshot.get("created_at"),
    )

    monkeypatch.chdir(repo)
    restore_out = runner.invoke(
        app,
        ["worktree", "restore-owned-head", worktree_name, "--json"],
        catch_exceptions=False,
    )
    assert restore_out.exit_code == 0, restore_out.stdout
    restored = json.loads(restore_out.stdout)

    assert restored["head_snapshot_id"] == owned_snapshot["snapshot_id"]
    assert restored["workspace_status"] == "clean"
    details = restored["restore_owned_head"]
    assert details["foreign_detected"] is True
    assert details["current_head_snapshot_id_before"] == foreign_snapshot["snapshot_id"]
    assert details["restored_snapshot_id"] == owned_snapshot["snapshot_id"]
    assert details["dropped_snapshots"][0]["snapshot_id"] == foreign_snapshot["snapshot_id"]
    assert details["dropped_snapshots"][0]["owner_task_id"] == "LT-foreign"
    assert (worktree_path / "feature.txt").read_text(encoding="utf-8") == "owned\n"


def test_worktree_restore_owned_head_refuses_dirty_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-restore-owned-head-dirty"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    _set_plan_task_binding_advisory()

    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Restore owned head dirty guard",
            "--intent",
            "fail closed when the contaminated worktree also has unsaved changes",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]
    worktree_name = str(worktree["name"])
    worktree_path = Path(worktree["path"])

    worktree_ctx = RepoContext.discover(worktree_path)
    monkeypatch.chdir(worktree_path)
    (worktree_path / "feature.txt").write_text("owned\n", encoding="utf-8")
    _ = local_create_snapshot(worktree_ctx, "owned feature snapshot")
    (worktree_path / "feature.txt").write_text("foreign\n", encoding="utf-8")
    foreign_snapshot = local_create_snapshot(worktree_ctx, "foreign feature snapshot")
    native_local_control.record_workflow_snapshot_provenance(
        worktree_ctx,
        foreign_snapshot["snapshot_id"],
        task_id="LT-foreign",
        change_id="LC-foreign",
        worktree_name="lt-foreign",
        line_name=str(worktree["current_line"]),
        created_at=foreign_snapshot.get("created_at"),
    )
    (worktree_path / "feature.txt").write_text("dirty drift\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    restore_out = runner.invoke(
        app,
        ["worktree", "restore-owned-head", worktree_name],
        catch_exceptions=False,
    )
    assert restore_out.exit_code != 0
    output = restore_out.output or restore_out.stdout
    assert "unsaved changes relative to current" in output
    assert "head" in output
    assert foreign_snapshot["snapshot_id"] in output


def test_local_task_show_surfaces_bound_worktree_rebase_advisory_when_stale(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-task-show-retarget"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    _set_plan_task_binding_advisory()

    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Local task show rebase advisory",
            "--intent",
            "surface stale worktree guidance in local task show",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]
    worktree_path = Path(worktree["path"])

    monkeypatch.chdir(worktree_path)
    (worktree_path / "feature.txt").write_text("feature only\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature side", "--json"], catch_exceptions=False)
    assert feature_out.exit_code == 0, feature_out.stdout

    monkeypatch.chdir(repo)
    (repo / "README.md").write_text("base\nmain advanced\n", encoding="utf-8")
    repo_ctx = RepoContext.discover(repo)
    local_create_snapshot(repo_ctx, "main advance")

    task_show_out = runner.invoke(app, ["task", "show", payload["task_id"], "--local", "--json"], catch_exceptions=False)
    assert task_show_out.exit_code == 0, task_show_out.stdout
    shown = json.loads(task_show_out.stdout)

    assert shown["worktree"]["name"] == worktree["name"]
    assert shown["worktree"]["needs_retarget"] is True
    advisory = shown["worktree_advisory"]
    assert advisory["code"] == "needs_retarget"
    assert advisory["status"] == "stale"
    assert advisory["target_base_line"] == "main"
    assert advisory["command"] == "ait worktree rebase --onto main"
    assert advisory["fork_snapshot_id"] == shown["worktree"]["retarget"]["fork_snapshot_id"]
    assert advisory["target_base_snapshot_id"] == shown["worktree"]["retarget"]["target_base_snapshot_id"]
    assert "still forks from" in advisory["detail"]


def test_patchset_publish_rejects_empty_diff_without_allow_empty(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-empty-patchset"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-empty-patchset") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--remote", "origin", "--title", "Empty patchset guard", "--intent", "prevent accidental empty review patchsets", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        _bind_task_worktree(task["task_id"], monkeypatch, name="empty-patchset")
        change_out = runner.invoke(
            app,
            ["change", "create", "--remote", "origin", "--task", task["task_id"], "--title", "Empty patchset guard", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        rejected = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "should be rejected"],
            catch_exceptions=False,
        )
        assert rejected.exit_code != 0
        assert "Refusing to publish empty patchset" in rejected.output

        allowed = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "intentional empty patchset", "--allow-empty", "--json"],
            catch_exceptions=False,
        )
        assert allowed.exit_code == 0, allowed.stdout
        patchset = json.loads(allowed.stdout)
        assert patchset["base_snapshot_id"] == patchset["revision_snapshot_id"]


def test_workflow_publish_creates_review_base_change_and_patchset(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-publish"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-publish") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert seed_out.exit_code == 0, seed_out.stdout
        base_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0
        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--remote", "origin", "--title", "Publish helper", "--intent", "collapse review publish commands", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="workflow-publish")

        (workspace / "app.py").write_text("print('workflow helper')\n", encoding="utf-8")
        publish_out = runner.invoke(
            app,
            ["workflow", "publish", "--task", task["task_id"], "--summary", "workflow helper patchset", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert publish_out.exit_code == 0, publish_out.stdout
        payload = json.loads(publish_out.stdout)
        assert payload["snapshot"]["snapshot_id"] != base_snapshot_id
        assert payload["change"]["base_line"].startswith("review-base/")
        assert payload["patchset"]["base_snapshot_id"] == base_snapshot_id
        assert payload["patchset"]["revision_snapshot_id"] == payload["snapshot"]["snapshot_id"]
        assert payload["patchset"]["diff_stats"]["files_changed"] >= 1
        assert payload["patchset"]["publish_context"]["line_updated"] is False

        status_out = runner.invoke(app, ["workspace", "status", "--json"], catch_exceptions=False)
        assert status_out.exit_code == 0, status_out.stdout
        assert json.loads(status_out.stdout)["clean"] is True

        remote_lines_out = runner.invoke(app, ["line", "list", "--remote", "origin", "--json"], catch_exceptions=False)
        assert remote_lines_out.exit_code == 0, remote_lines_out.stdout
        remote_main = next(row for row in json.loads(remote_lines_out.stdout) if row["line_name"] == "main")
        assert remote_main["head_snapshot_id"] == base_snapshot_id


def test_local_draft_task_and_change_can_be_published_without_id_renaming(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-draft"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-local-draft") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        main_snapshot = json.loads(main_snap_out.stdout)

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--local", "--title", "Stabilize bootstrap", "--intent", "keep local draft ids stable", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        assert task["task_id"].startswith("LAITT-")
        assert len(task["task_id"]) == len("LAITT-0001")
        assert task["task_seq"] == 1
        assert task["publication_state"] == "local_draft"

        task_list_out = runner.invoke(app, ["task", "list", "--local", "--json"], catch_exceptions=False)
        assert task_list_out.exit_code == 0, task_list_out.stdout
        task_rows = json.loads(task_list_out.stdout)
        assert [row["task_id"] for row in task_rows] == [task["task_id"]]

        task_show_out = runner.invoke(app, ["task", "show", task["task_id"], "--local", "--json"], catch_exceptions=False)
        assert task_show_out.exit_code == 0, task_show_out.stdout
        shown_task = json.loads(task_show_out.stdout)
        assert shown_task["intent"] == "keep local draft ids stable"
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="local-draft")

        assert runner.invoke(app, ["line", "create", "feature/local-draft"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/local-draft"], catch_exceptions=False).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout

        change_out = runner.invoke(
            app,
            ["change", "create", "--local", "--task", task["task_id"], "--title", "Fix bootstrap reliability", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)
        assert change["change_id"].startswith("LAITC-")
        assert len(change["change_id"]) == len("LAITC-0001")
        assert change["change_seq"] == 1
        assert change["publication_state"] == "local_draft"
        assert change["current_patchset_number"] == 0

        change_list_out = runner.invoke(app, ["change", "list", "--local", "--json"], catch_exceptions=False)
        assert change_list_out.exit_code == 0, change_list_out.stdout
        change_rows = json.loads(change_list_out.stdout)
        assert [row["change_id"] for row in change_rows] == [change["change_id"]]

        change_show_out = runner.invoke(app, ["change", "show", change["change_id"], "--local", "--json"], catch_exceptions=False)
        assert change_show_out.exit_code == 0, change_show_out.stdout
        shown_change = json.loads(change_show_out.stdout)
        assert shown_change["task_id"] == task["task_id"]
        assert shown_change["base_line"] == "main"
        assert shown_change["fork_snapshot_id"] == main_snapshot["snapshot_id"]
        assert shown_change["forked_from_line"] == "main"

        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        patchset_fail_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "should fail before publish"],
            catch_exceptions=False,
        )
        assert patchset_fail_out.exit_code != 0
        assert f"Local change {change['change_id']}" in patchset_fail_out.output

        change_publish_fail = runner.invoke(app, ["change", "publish", change["change_id"]], catch_exceptions=False)
        assert change_publish_fail.exit_code != 0
        assert f"Local task {task['task_id']} must be published" in change_publish_fail.output

        task_publish_out = runner.invoke(app, ["task", "publish", task["task_id"], "--json"], catch_exceptions=False)
        assert task_publish_out.exit_code == 0, task_publish_out.stdout
        published_task = json.loads(task_publish_out.stdout)
        assert published_task["task_id"] == task["task_id"]
        assert published_task["publication_state"] == "published"

        task_republish_out = runner.invoke(app, ["task", "publish", task["task_id"], "--json"], catch_exceptions=False)
        assert task_republish_out.exit_code == 0, task_republish_out.stdout
        assert json.loads(task_republish_out.stdout)["task_id"] == task["task_id"]

        remote_task_out = runner.invoke(app, ["task", "show", task["task_id"], "--json"], catch_exceptions=False)
        assert remote_task_out.exit_code == 0, remote_task_out.stdout
        assert json.loads(remote_task_out.stdout)["task_id"] == task["task_id"]

        change_publish_out = runner.invoke(app, ["change", "publish", change["change_id"], "--json"], catch_exceptions=False)
        assert change_publish_out.exit_code == 0, change_publish_out.stdout
        published_change = json.loads(change_publish_out.stdout)
        assert published_change["change_id"] == change["change_id"]
        assert published_change["publication_state"] == "published"

        change_republish_out = runner.invoke(app, ["change", "publish", change["change_id"], "--json"], catch_exceptions=False)
        assert change_republish_out.exit_code == 0, change_republish_out.stdout
        assert json.loads(change_republish_out.stdout)["change_id"] == change["change_id"]

        remote_change_out = runner.invoke(app, ["change", "show", change["change_id"], "--json"], catch_exceptions=False)
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["change_id"] == change["change_id"]
        assert remote_change["fork_snapshot_id"] == main_snapshot["snapshot_id"]
        assert remote_change["forked_from_line"] == "main"

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "first published patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["change_id"] == change["change_id"]


def test_change_publish_refuses_when_local_base_advanced_after_local_main_land(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-change-publish-local-base-advanced"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    with running_server(tmp_path / "server-data-change-publish-local-base-advanced") as base_url:
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Guard stale local promotion",
                "--intent",
                "refuse change publish after local main advanced",
                "--change-title",
                "Guard stale local promotion",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        started = json.loads(start_out.stdout)
        task = started
        change = started["change"]
        worktree_path = Path(started["worktree"]["path"])

        monkeypatch.chdir(worktree_path)
        (worktree_path / "feature.txt").write_text("feature only\n", encoding="utf-8")
        feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"], catch_exceptions=False)
        assert feature_out.exit_code == 0, feature_out.stdout

        monkeypatch.chdir(repo)
        (repo / "README.md").write_text("base\nmain advanced\n", encoding="utf-8")
        repo_ctx = RepoContext.discover(repo)
        local_create_snapshot(repo_ctx, "main advance")

        task_publish_out = runner.invoke(app, ["task", "publish", task["task_id"], "--json"], catch_exceptions=False)
        assert task_publish_out.exit_code == 0, task_publish_out.stdout

        change_publish_out = runner.invoke(app, ["change", "publish", change["change_id"]], catch_exceptions=False)
        assert change_publish_out.exit_code != 0
        output = change_publish_out.output or change_publish_out.stdout or ""
        assert change["change_id"] in output
        assert "worktree rebase --onto main" in output
        assert "before publishing change" in output


def test_task_start_help_describes_public_task_entrypoint():
    help_out = runner.invoke(app, ["task", "start", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    normalized = " ".join(help_out.stdout.split())
    assert "Start a task and optionally open its first change." in normalized
    assert "omitting `--local` and `--remote` usually starts remote-backed lineage." in normalized
    assert "omitting this usually already follows the" in normalized
    assert "remote-backed default." in normalized
    assert "--task-only" in help_out.stdout


def test_task_help_omits_task_create_from_the_public_entry_surface():
    help_out = runner.invoke(app, ["task", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Create a task without opening a change yet." not in help_out.stdout
    assert "Start a task and optionally open its first change." in help_out.stdout
    assert "Close as abandoned or excluded." in help_out.stdout
    assert "Close or cancel a task." not in help_out.stdout


def test_removed_task_create_command_is_no_longer_available():
    removed_command = "cr" + "eate"
    help_out = runner.invoke(app, ["task", removed_command, "--help"], catch_exceptions=False)
    assert help_out.exit_code != 0
    output = help_out.output or help_out.stderr or ""
    assert "No such command" in output


def test_change_create_help_describes_review_boundary_entrypoint():
    help_out = runner.invoke(app, ["change", "create", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    normalized = " ".join(help_out.stdout.split())
    assert "Open a change for an existing task when work reaches a review or shared boundary." in normalized
    assert "omitting `--local` and `--remote` usually creates remote-backed lineage." in normalized
    assert "omitting this usually already follows the" in normalized
    assert "remote-backed default." in normalized


def test_task_canceled_help_describes_abandonment_role():
    help_out = runner.invoke(app, ["task", "canceled", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    normalized = " ".join(help_out.stdout.split())
    assert "Close a task explicitly as abandoned work" in normalized
    assert "stops participating in later promotion." in normalized
    assert "--exclude-later-promotion" in normalized


def test_task_close_command_is_no_longer_available():
    close_out = runner.invoke(app, ["task", "close", "--help"], catch_exceptions=False)
    assert close_out.exit_code != 0
    output = close_out.output or close_out.stderr or ""
    assert "No such command" in output


def test_task_canceled_defaults_to_abandoned_status(monkeypatch):
    call_log = {}

    def fake_task_close_action(task_id: str, status: str, local: bool, remote: str | None, json_output: bool) -> None:
        call_log["task_id"] = task_id
        call_log["status"] = status
        call_log["local"] = local
        call_log["remote"] = remote
        call_log["json_output"] = json_output

    monkeypatch.setattr(task_cli_module, "_task_close_action", fake_task_close_action)

    canceled_out = runner.invoke(app, ["task", "canceled", "T-0123", "--local", "--json"], catch_exceptions=False)
    assert canceled_out.exit_code == 0, canceled_out.stdout
    assert call_log == {
        "task_id": "T-0123",
        "status": "abandoned",
        "local": True,
        "remote": None,
        "json_output": True,
    }


def test_task_canceled_dispatches_later_promotion_excluded_status(monkeypatch):
    call_log = {}

    def fake_task_close_action(task_id: str, status: str, local: bool, remote: str | None, json_output: bool) -> None:
        call_log["task_id"] = task_id
        call_log["status"] = status
        call_log["local"] = local
        call_log["remote"] = remote
        call_log["json_output"] = json_output

    monkeypatch.setattr(task_cli_module, "_task_close_action", fake_task_close_action)

    canceled_out = runner.invoke(
        app,
        ["task", "canceled", "T-0123", "--local", "--exclude-later-promotion", "--json"],
        catch_exceptions=False,
    )
    assert canceled_out.exit_code == 0, canceled_out.stdout
    assert call_log == {
        "task_id": "T-0123",
        "status": "later_promotion_excluded",
        "local": True,
        "remote": None,
        "json_output": True,
    }


def test_queue_group_help_lists_inventory_role():
    help_out = runner.invoke(app, ["queue", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Use helper inventory reads for the shared queue and change status" in help_out.stdout


def test_queue_summary_help_describes_shared_inventory_role():
    help_out = runner.invoke(app, ["queue", "summary", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Summarize the shared queue and optionally the non-landed" in help_out.stdout
    assert "change inventory" in help_out.stdout
    assert "one helper read." in help_out.stdout


def test_workflow_group_help_lists_helper_orchestrator_role():
    help_out = runner.invoke(app, ["workflow", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Use helper/orchestrator entrypoints for common workflow bursts" in help_out.stdout
    assert "guide" in help_out.stdout
    assert "Show workflow helper guides." in help_out.stdout
    assert "land" in help_out.stdout
    assert "Show the landing helper/orchestrator." in help_out.stdout
    assert "land-local" in help_out.stdout
    assert "Run the local-only landing helper." in help_out.stdout


def test_task_audit_help_describes_helper_readiness_view():
    help_out = runner.invoke(app, ["task", "audit", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Summarize one task's readiness against a target line in one" in help_out.stdout
    assert "helper read-model" in help_out.stdout
    assert "view." in help_out.stdout


def test_task_complete_help_describes_successful_closeout_role():
    help_out = runner.invoke(app, ["task", "complete", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Mark a task complete after landed work or intentional local-only" in help_out.stdout
    assert "finish." in help_out.stdout


def test_patchset_publish_help_describes_formal_review_artifact_role():
    help_out = runner.invoke(app, ["patchset", "publish", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Publish the current base/revision snapshot pair for a change" in help_out.stdout
    assert "formal" in help_out.stdout
    assert "review patchset." in help_out.stdout


def test_patchset_group_help_describes_core_gate_role():
    help_out = runner.invoke(app, ["patchset", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Run core patchset publication and inspection commands" in help_out.stdout


def test_attest_put_help_describes_patchset_evidence_role():
    help_out = runner.invoke(app, ["attest", "put", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    normalized_help = " ".join(help_out.stdout.split())
    assert "Backfill or override patchset evidence" in normalized_help
    assert "automatic CI/provenance capture is not enough." in normalized_help


def test_attest_group_help_describes_core_gate_role():
    help_out = runner.invoke(app, ["attest", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Run core patchset evidence and provenance commands" in help_out.stdout


def test_review_task_approve_help_describes_human_gate_role():
    help_out = runner.invoke(app, ["review", "task", "approve", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Approve the task/outcome result for the selected or current" in help_out.stdout
    assert "patchset." in help_out.stdout


def test_review_group_help_lists_interaction_roles():
    help_out = runner.invoke(app, ["review", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Inspect review state for a change." in help_out.stdout
    assert "Record AI code review evidence for a patchset." in help_out.stdout
    assert "Record task/outcome review decisions." in help_out.stdout
    assert "Record preserved team patchset review decisions." in help_out.stdout


def test_review_team_request_help_describes_reviewer_group_request_role():
    help_out = runner.invoke(app, ["review", "team", "request", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Request team review from one or more reviewer groups" in help_out.stdout
    assert "selected or" in help_out.stdout
    assert "current patchset." in help_out.stdout


def test_review_task_request_changes_help_describes_blocking_feedback_role():
    help_out = runner.invoke(app, ["review", "task", "request-changes", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Request task/outcome changes for the selected or current" in help_out.stdout
    assert "patchset." in help_out.stdout


def test_review_task_comment_help_describes_non_blocking_feedback_role():
    help_out = runner.invoke(app, ["review", "task", "comment", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Record non-blocking task/outcome review commentary." in help_out.stdout


def test_review_task_defer_help_describes_deferred_decision_role():
    help_out = runner.invoke(app, ["review", "task", "defer", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Record that task/outcome review is intentionally deferred." in help_out.stdout


def test_hidden_root_review_aliases_do_not_appear_in_review_help():
    help_out = runner.invoke(app, ["review", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "approve          Record team approval for a patchset." not in help_out.stdout
    assert "request          Request review from reviewer groups." not in help_out.stdout
    assert "code-summary" not in help_out.stdout


def test_hidden_root_review_alias_help_still_resolves_for_compatibility():
    help_out = runner.invoke(app, ["review", "approve", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Legacy alias for `ait review team approve`." in help_out.stdout


def test_policy_eval_help_describes_readiness_gate_role():
    help_out = runner.invoke(app, ["policy", "eval", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Evaluate whether a patchset currently satisfies landing" in help_out.stdout
    assert "policy requirements." in help_out.stdout


def test_policy_group_help_describes_core_gate_role():
    help_out = runner.invoke(app, ["policy", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Run core readiness-gate evaluation and waiver commands" in help_out.stdout


def test_patchset_show_help_describes_inspection_role():
    help_out = runner.invoke(app, ["patchset", "show", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Inspect one published patchset" in help_out.stdout
    assert "base/revision snapshots and" in help_out.stdout
    assert "evaluation state." in help_out.stdout


def test_review_show_help_describes_review_state_inspection_role():
    help_out = runner.invoke(app, ["review", "show", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Inspect the current approval, blocking, comment, and review-request" in help_out.stdout
    assert "state for" in help_out.stdout
    assert "a change." in help_out.stdout


def test_attest_show_help_describes_evidence_inspection_role():
    help_out = runner.invoke(app, ["attest", "show", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Inspect recorded tests, checks, and provenance evidence" in help_out.stdout
    assert "for one patchset." in help_out.stdout


def test_policy_show_help_describes_readiness_inspection_role():
    help_out = runner.invoke(app, ["policy", "show", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Inspect the last evaluated policy decision" in help_out.stdout
    assert "per-check readiness breakdown" in help_out.stdout
    assert "for a patchset." in help_out.stdout


def test_policy_waive_help_describes_exception_path_role():
    help_out = runner.invoke(app, ["policy", "waive", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Create an explicit policy exception for a specific patchset" in help_out.stdout
    assert "documented waiver." in help_out.stdout


def test_land_group_help_describes_core_gate_role():
    help_out = runner.invoke(app, ["land", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Run core guarded remote landing commands" in help_out.stdout


def test_workflow_land_help_describes_helper_orchestrator_role():
    help_out = runner.invoke(app, ["workflow", "land", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Show or apply the landing helper/orchestrator view for one" in help_out.stdout
    assert "change or" in help_out.stdout
    assert "patchset." in help_out.stdout


def test_workflow_land_local_help_describes_local_helper_role():
    help_out = runner.invoke(app, ["workflow", "land-local", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Run the local-only landing helper for one change onto a" in help_out.stdout
    assert "local target" in help_out.stdout
    assert "line." in help_out.stdout


def test_task_start_creates_local_draft_task_and_change_with_tracking(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-local"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
    set_out = runner.invoke(app, ["config", "set", "--task-tracking", "on", "--json"], catch_exceptions=False)
    assert set_out.exit_code == 0, set_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap local workflow",
            "--intent",
            "create draft task and change together",
            "--base-line",
            "main",
            "--risk",
            "low",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    assert payload["task_id"].startswith("LAITT-")
    assert len(payload["task_id"]) == len("LAITT-0001")
    assert payload["task_seq"] == 1
    assert payload["publication_state"] == "local_draft"
    assert payload["tracking"]["session_scope"] == "local"
    assert payload["change"]["change_id"].startswith("LAITC-")
    assert len(payload["change"]["change_id"]) == len("LAITC-0001")
    assert payload["change"]["change_seq"] == 1
    assert payload["change"]["publication_state"] == "local_draft"
    assert payload["change"]["task_id"] == payload["task_id"]
    assert payload["change"]["title"] == "Bootstrap local workflow"
    assert payload["change"]["forked_from_line"] == "main"

    config_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert config_out.exit_code == 0, config_out.stdout
    config_data = json.loads(config_out.stdout)
    assert config_data["tracked_session"]["task_id"] == payload["task_id"]
    assert config_data["tracked_session"]["session_id"] == payload["tracking"]["session_id"]

    task_show_out = runner.invoke(app, ["task", "show", payload["task_id"], "--local", "--json"], catch_exceptions=False)
    assert task_show_out.exit_code == 0, task_show_out.stdout
    shown_task = json.loads(task_show_out.stdout)
    assert shown_task["intent"] == "create draft task and change together"

    change_show_out = runner.invoke(app, ["change", "show", payload["change"]["change_id"], "--local", "--json"], catch_exceptions=False)
    assert change_show_out.exit_code == 0, change_show_out.stdout
    shown_change = json.loads(change_show_out.stdout)
    assert shown_change["task_id"] == payload["task_id"]


def test_task_tracking_does_not_auto_log_without_explicit_env_for_local_tasks(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-tracking-local"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
    set_out = runner.invoke(app, ["config", "set", "--task-tracking", "on", "--json"], catch_exceptions=False)
    assert set_out.exit_code == 0, set_out.stdout

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Track local task", "--intent", "verify config-backed autolog", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    session_id = task["tracking"]["session_id"]

    config_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert config_out.exit_code == 0, config_out.stdout
    config_data = json.loads(config_out.stdout)
    assert config_data["task_tracking"] == "on"
    assert config_data["tracked_session"]["task_id"] == task["task_id"]
    assert config_data["tracked_session"]["session_id"] == session_id

    for argv in (
        ["status", "--json"],
        ["line", "show", "main", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")

    events_out = runner.invoke(app, ["session", "events", session_id, "--local", "--json"], catch_exceptions=False)
    assert events_out.exit_code == 0, events_out.stdout
    events = json.loads(events_out.stdout)
    started = [row for row in events if row["payload"].get("command_phase") == "started"]
    finished = [row for row in events if row["payload"].get("command_phase") == "finished"]
    assert started == []
    assert finished == []

    analyze_out = runner.invoke(app, ["session", "analyze", session_id, "--local", "--json"], catch_exceptions=False)
    assert analyze_out.exit_code == 0, analyze_out.stdout
    analysis = json.loads(analyze_out.stdout)
    assert analysis["ait_command_count"] == 0
    assert analysis["capture_modes"] == []
    assert analysis["command_paths"] == []


def test_task_complete_with_tracking_writes_retrospective_and_clears_binding(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-tracking-complete"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--task-tracking", "on"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Complete tracked task", "--intent", "capture retrospective", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    session_id = task["tracking"]["session_id"]
    monkeypatch.setenv("AIT_SESSION_ID", session_id)
    monkeypatch.setenv("AIT_SESSION_LOCAL", "1")
    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "1")

    for argv in (
        ["status", "--json"],
        ["status", "--json"],
        ["task", "show", task["task_id"], "--local", "--json"],
        ["task", "show", task["task_id"], "--local", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")
    complete_out = runner.invoke(app, ["task", "complete", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert complete_out.exit_code == 0, complete_out.stdout
    completed = json.loads(complete_out.stdout)
    assert completed["status"] == "completed"
    assert completed["tracking"]["session_id"] == session_id
    assert completed["tracking"]["session_status"] == "completed"
    assert completed["tracking"]["checkpoint_id"] is not None
    assert completed["retrospective"]["merge_opportunity_count"] >= 1
    assert completed["retrospective"]["avoidable_command_count"] >= 1
    assert completed["improvement_plan"]

    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")

    config_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert config_out.exit_code == 0, config_out.stdout
    config_data = json.loads(config_out.stdout)
    assert config_data["task_tracking"] == "on"
    assert config_data["tracked_session"] is None

    session_out = runner.invoke(app, ["session", "show", session_id, "--local", "--json"], catch_exceptions=False)
    assert session_out.exit_code == 0, session_out.stdout
    assert json.loads(session_out.stdout)["status"] == "completed"

    events_out = runner.invoke(app, ["session", "events", session_id, "--local", "--json"], catch_exceptions=False)
    assert events_out.exit_code == 0, events_out.stdout
    events = json.loads(events_out.stdout)
    retrospective_events = [row for row in events if row["event_type"] == "task.retrospective"]
    assert len(retrospective_events) == 1
    assert retrospective_events[0]["payload"]["task_id"] == task["task_id"]
    assert retrospective_events[0]["payload"]["improvement_plan"]

    checkpoints_out = runner.invoke(app, ["session", "checkpoints", session_id, "--local", "--json"], catch_exceptions=False)
    assert checkpoints_out.exit_code == 0, checkpoints_out.stdout
    checkpoints = json.loads(checkpoints_out.stdout)
    assert checkpoints


def test_task_canceled_with_tracking_writes_retrospective_and_clears_binding(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-tracking-canceled"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--task-tracking", "on"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Cancel tracked task", "--intent", "capture canceled retrospective", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    session_id = task["tracking"]["session_id"]
    monkeypatch.setenv("AIT_SESSION_ID", session_id)
    monkeypatch.setenv("AIT_SESSION_LOCAL", "1")
    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "1")

    for argv in (
        ["status", "--json"],
        ["task", "show", task["task_id"], "--local", "--json"],
    ):
        result = runner.invoke(app, argv, catch_exceptions=False)
        assert result.exit_code == 0, result.stdout

    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")
    canceled_out = runner.invoke(app, ["task", "canceled", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert canceled_out.exit_code == 0, canceled_out.stdout
    canceled = json.loads(canceled_out.stdout)
    assert canceled["status"] == "abandoned"
    assert canceled["tracking"]["session_id"] == session_id
    assert canceled["tracking"]["session_status"] == "canceled"
    assert canceled["tracking"]["checkpoint_id"] is not None
    assert canceled["retrospective"]["ait_command_count"] >= 1
    assert canceled["improvement_plan"]

    config_out = runner.invoke(app, ["config", "show", "--json"], catch_exceptions=False)
    assert config_out.exit_code == 0, config_out.stdout
    config_data = json.loads(config_out.stdout)
    assert config_data["tracked_session"] is None

    session_out = runner.invoke(app, ["session", "show", session_id, "--local", "--json"], catch_exceptions=False)
    assert session_out.exit_code == 0, session_out.stdout
    assert json.loads(session_out.stdout)["status"] == "canceled"


def test_task_tracking_off_hard_disables_env_autolog(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-tracking-off"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--task-tracking", "off"], catch_exceptions=False).exit_code == 0

    session_out = runner.invoke(
        app,
        ["session", "create", "--local", "--kind", "agent_run", "--title", "Manual session", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)

    monkeypatch.setenv("AIT_SESSION_ID", session["session_id"])
    monkeypatch.setenv("AIT_SESSION_LOCAL", "1")
    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "1")

    status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout

    monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")

    events_out = runner.invoke(app, ["session", "events", session["session_id"], "--local", "--json"], catch_exceptions=False)
    assert events_out.exit_code == 0, events_out.stdout
    assert json.loads(events_out.stdout) == []


def test_task_complete_marks_local_draft_task_completed_and_blocks_new_local_changes(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-task-complete"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Document bootstrap", "--intent", "finish draft planning", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    _bind_task_worktree(task["task_id"], monkeypatch, name="local-task-complete")

    complete_out = runner.invoke(app, ["task", "complete", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert complete_out.exit_code == 0, complete_out.stdout
    completed = json.loads(complete_out.stdout)
    assert completed["status"] == "completed"

    show_out = runner.invoke(app, ["task", "show", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    assert json.loads(show_out.stdout)["status"] == "completed"

    change_out = runner.invoke(
        app,
        ["change", "create", "--local", "--task", task["task_id"], "--title", "Should fail", "--base-line", "main", "--risk", "low"],
        catch_exceptions=False,
    )
    assert change_out.exit_code != 0
    output = change_out.output or change_out.stderr or ""
    assert "cannot accept" in output


def test_task_restart_rejects_completed_local_task(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-task-restart-completed"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Complete then reject restart", "--intent", "prove restart is limited to canceled lineage", "--risk", "low", "--json"],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    _bind_task_worktree(task["task_id"], monkeypatch, name="local-task-restart-completed")

    complete_out = runner.invoke(app, ["task", "complete", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert complete_out.exit_code == 0, complete_out.stdout

    monkeypatch.chdir(repo)
    restart_out = runner.invoke(app, ["task", "restart", task["task_id"], "--local"], catch_exceptions=False)
    assert restart_out.exit_code != 0
    output = restart_out.output or restart_out.stderr or ""
    assert "completed" in output
    assert "task canceled lineage" in output


def test_task_restart_reopens_local_abandoned_task_and_unique_archived_change(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-task-restart"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Restart local canceled lineage",
            "--intent",
            "restore one archived local change when a task was canceled by mistake",
            "--risk",
            "low",
            "--change-title",
            "Restart local canceled lineage",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    change = task["change"]
    _bind_task_worktree(task["task_id"], monkeypatch, name="local-task-restart")

    close_change_out = runner.invoke(app, ["change", "close", change["change_id"], "--local", "--json"], catch_exceptions=False)
    assert close_change_out.exit_code == 0, close_change_out.stdout
    assert json.loads(close_change_out.stdout)["status"] == "archived"

    canceled_out = runner.invoke(app, ["task", "canceled", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert canceled_out.exit_code == 0, canceled_out.stdout
    assert json.loads(canceled_out.stdout)["status"] == "abandoned"

    restart_out = runner.invoke(app, ["task", "restart", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert restart_out.exit_code == 0, restart_out.stdout
    restarted = json.loads(restart_out.stdout)
    assert restarted["status"] == "active"
    assert restarted["change"]["change_id"] == change["change_id"]
    assert restarted["change"]["status"] == "draft"

    task_show_out = runner.invoke(app, ["task", "show", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert task_show_out.exit_code == 0, task_show_out.stdout
    assert json.loads(task_show_out.stdout)["status"] == "active"

    change_show_out = runner.invoke(app, ["change", "show", change["change_id"], "--local", "--json"], catch_exceptions=False)
    assert change_show_out.exit_code == 0, change_show_out.stdout
    assert json.loads(change_show_out.stdout)["status"] == "draft"


def test_task_canceled_can_reclassify_completed_local_landed_slice(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-task-cancel-completed-landed"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Complete then cancel local slice",
            "--intent",
            "verify completed local landed slices can be excluded from later promotion",
            "--risk",
            "low",
            "--change-title",
            "Complete then cancel local slice",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree_path = Path(task["worktree"]["path"])

    with monkeypatch.context() as worktree_context:
        (worktree_path / "README.md").write_text("base\ncompleted locally\n", encoding="utf-8")
        worktree_context.chdir(worktree_path)
        local_create_snapshot(RepoContext.discover(worktree_path), "complete local slice")
        land_out = runner.invoke(
            app,
            ["workflow", "land-local", task["change"]["change_id"], "--target", "main", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout

    monkeypatch.chdir(repo)
    canceled_out = runner.invoke(
        app,
        ["task", "canceled", task["task_id"], "--local", "--exclude-later-promotion", "--json"],
        catch_exceptions=False,
    )
    assert canceled_out.exit_code == 0, canceled_out.stdout
    canceled = json.loads(canceled_out.stdout)
    assert canceled["status"] == "later_promotion_excluded"

    show_out = runner.invoke(app, ["task", "show", task["task_id"], "--local", "--json"], catch_exceptions=False)
    assert show_out.exit_code == 0, show_out.stdout
    assert json.loads(show_out.stdout)["status"] == "later_promotion_excluded"


def test_task_canceled_rejects_local_published_task_mirror(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-task-published-close"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-local-task-published-close") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--local", "--title", "Publish task", "--intent", "verify local mirror guard", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        task_worktree_path = Path(task["worktree"]["path"])

        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()
        with monkeypatch.context() as worktree_context:
            worktree_context.chdir(task_worktree_path)
            assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        publish_out = runner.invoke(app, ["task", "publish", task["task_id"], "--json"], catch_exceptions=False)
        assert publish_out.exit_code == 0, publish_out.stdout

    close_out = runner.invoke(app, ["task", "complete", task["task_id"], "--local"], catch_exceptions=False)
    assert close_out.exit_code != 0
    output = close_out.output or close_out.stderr or ""
    assert "close the" in output
    assert "remote task instead" in output


def test_change_close_archives_local_draft_and_blocks_publish(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-change-close"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-local-change-close") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--local", "--title", "Archive local change", "--intent", "close stale local draft changes", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        _bind_task_worktree(task["task_id"], monkeypatch, name="local-change-close")

        change_out = runner.invoke(
            app,
            ["change", "create", "--local", "--task", task["task_id"], "--title", "Local stale draft", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        close_out = runner.invoke(app, ["change", "close", change["change_id"], "--local", "--json"], catch_exceptions=False)
        assert close_out.exit_code == 0, close_out.stdout
        assert json.loads(close_out.stdout)["status"] == "archived"

        publish_task_out = runner.invoke(app, ["task", "publish", task["task_id"], "--json"], catch_exceptions=False)
        assert publish_task_out.exit_code == 0, publish_task_out.stdout

        publish_change_out = runner.invoke(app, ["change", "publish", change["change_id"]], catch_exceptions=False)
        assert publish_change_out.exit_code != 0
        output = publish_change_out.output or publish_change_out.stderr or ""
        assert "archived" in output
        assert "cannot be published" in output


def test_task_restart_reopens_remote_canceled_task_and_archived_change(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-task-restart"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-task-restart") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"], catch_exceptions=False)
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Restart remote canceled lineage",
                "--intent",
                "restore one archived shared change when a shared task was canceled by mistake",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        change = task["change"]
        _bind_task_worktree(task["task_id"], monkeypatch, name="remote-task-restart")

        close_change_out = runner.invoke(app, ["change", "close", change["change_id"], "--json"], catch_exceptions=False)
        assert close_change_out.exit_code == 0, close_change_out.stdout
        assert json.loads(close_change_out.stdout)["status"] == "archived"

        remote_client_module.close_task(base_url, task["task_id"], "canceled", repo_name="housekeeper")

        restart_out = runner.invoke(app, ["task", "restart", task["task_id"], "--json"], catch_exceptions=False)
        assert restart_out.exit_code == 0, restart_out.stdout
        restarted = json.loads(restart_out.stdout)
        assert restarted["status"] == "active"
        assert restarted["change"]["change_id"] == change["change_id"]
        assert restarted["change"]["status"] == "draft"

        task_show_out = runner.invoke(app, ["task", "show", task["task_id"], "--json"], catch_exceptions=False)
        assert task_show_out.exit_code == 0, task_show_out.stdout
        assert json.loads(task_show_out.stdout)["status"] == "active"

        change_show_out = runner.invoke(app, ["change", "show", change["change_id"], "--json"], catch_exceptions=False)
        assert change_show_out.exit_code == 0, change_show_out.stdout
        assert json.loads(change_show_out.stdout)["status"] == "draft"


def test_task_audit_read_endpoint_returns_404_for_unknown_task(tmp_path: Path):
    with running_server(tmp_path / "server-data-task-audit-404") as base_url:
        req = urllib.request.Request(f"{base_url}/v1/native/read/tasks/AITT-404/audit", headers={"Accept": "application/json"})
        try:
            urllib.request.urlopen(req, timeout=5)
        except urllib.error.HTTPError as exc:
            assert exc.code == 404
            payload = json.loads(exc.read().decode("utf-8"))
            assert "Unknown task" in payload["detail"]
        else:
            raise AssertionError("expected task audit read endpoint to return 404 for unknown task")


def test_attestation_marks_ai_provenance_partial_without_session_and_checkpoint(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-provenance-partial"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-provenance-partial") as base_url:
        monkeypatch.chdir(repo)
        monkeypatch.delenv("AIT_SESSION_ID", raising=False)
        monkeypatch.delenv("AIT_CHECKPOINT_ID", raising=False)
        assert runner.invoke(
            app,
            ["init", "--name", "housekeeper", "--default-author-mode", "human_with_ai_assist"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Partial provenance", "--intent", "verify missing ai evidence", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="provenance-partial")

        assert runner.invoke(app, ["line", "create", "feature/provenance-partial"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/provenance-partial"], catch_exceptions=False).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature work"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Partial provenance", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "partial provenance patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        attest_out = runner.invoke(
            app,
            ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--model", "gpt-5.4-codex", "--json"],
            catch_exceptions=False,
        )
        assert attest_out.exit_code == 0, attest_out.stdout
        attestation = json.loads(attest_out.stdout)
        assert attestation["provenance_summary"]["evidence_readiness"] == "partial"
        assert set(attestation["provenance_summary"]["missing_fields"]) == {"session_id", "checkpoint_id"}
        assert attestation["detail"]["minimum_evidence"]["policy_readable"] is False


def test_attest_put_can_resolve_latest_patchset_from_change(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-attest-change-default"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-attest-change-default") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"], catch_exceptions=False).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Attest from change", "--intent", "resolve patchset automatically", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="attest-change-default")

        assert runner.invoke(app, ["line", "create", "feature/attest-change-default"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/attest-change-default"], catch_exceptions=False).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature work"], catch_exceptions=False).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Attest from change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "patchset for inferred attestation", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        attest_out = runner.invoke(app, ["attest", "put", "--change", change["change_id"], "--tests", "pass", "--json"], catch_exceptions=False)
        assert attest_out.exit_code == 0, attest_out.stdout
        attestation = json.loads(attest_out.stdout)
        assert attestation["patchset_id"] == patchset["patchset_id"]
        assert attestation["evaluation_summary"]["tests"] == "pass"


def test_attest_put_requires_patchset_id_or_change(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-attest-missing-target"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0

    result = runner.invoke(app, ["attest", "put", "--tests", "pass"], catch_exceptions=False)
    assert result.exit_code == 2
    assert "Provide PATCHSET_ID or --change" in result.output
