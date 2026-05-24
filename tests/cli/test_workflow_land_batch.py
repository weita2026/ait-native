from __future__ import annotations

import base64
import pytest

import ait_native.local_control as local_control
from ait import snapshot_diff as snapshot_diff_module
from ait_native.store import create_snapshot, mark_local_plan_published

from ._shared import *  # noqa: F401,F403


def _work_file(root: Path) -> Path:
    return root / "work.txt"


def _land_local_with_rebase_recovery(
    worktree_root: Path,
    *,
    change_id: str,
    task_id: str,
    desired_text: str,
) -> dict:
    land_out = runner.invoke(
        app,
        ["workflow", "land-local", change_id, "--target", "main", "--json"],
        catch_exceptions=False,
    )
    if land_out.exit_code == 0:
        return json.loads(land_out.stdout)

    stderr = getattr(land_out, "stderr", "") or land_out.stdout
    assert "Run `ait worktree rebase --onto main`" in stderr, stderr

    rebase_out = runner.invoke(
        app,
        ["worktree", "rebase", "--onto", "main", "--json"],
        catch_exceptions=False,
    )
    assert rebase_out.exit_code == 0, rebase_out.stdout
    rebase_payload = json.loads(rebase_out.stdout)
    rebase = rebase_payload.get("rebase") if isinstance(rebase_payload.get("rebase"), dict) else {}
    if str(rebase.get("status") or "") == "conflicted":
        work_file = _work_file(worktree_root)
        work_file.write_text(desired_text, encoding="utf-8")
        continue_out = runner.invoke(
            app,
            ["worktree", "rebase", "--continue", "--json"],
            catch_exceptions=False,
        )
        assert continue_out.exit_code == 0, continue_out.stdout

    snapshot_retry_out = runner.invoke(
        app,
        ["snapshot", "create", "--message", f"{task_id} completion after rebase", "--json"],
        catch_exceptions=False,
    )
    assert snapshot_retry_out.exit_code == 0, snapshot_retry_out.stdout

    retried_land_out = runner.invoke(
        app,
        ["workflow", "land-local", change_id, "--target", "main", "--json"],
        catch_exceptions=False,
    )
    assert retried_land_out.exit_code == 0, retried_land_out.stdout
    return json.loads(retried_land_out.stdout)


def _complete_local_only_batch_task(
    repo: Path,
    monkeypatch,
    *,
    title: str,
    intent: str,
    change_title: str,
    readme_append: str,
    plan_id: str | None = None,
    plan_revision_id: str | None = None,
    plan_item_ref: str | None = None,
) -> dict:
    monkeypatch.chdir(repo)
    args = [
        "task",
        "start",
        "--local",
        "--title",
        title,
        "--intent",
        intent,
        "--risk",
        "medium",
        "--change-title",
        change_title,
    ]
    if plan_id is not None:
        args.extend(["--plan", plan_id])
    if plan_revision_id is not None:
        args.extend(["--revision", plan_revision_id])
    if plan_item_ref is not None:
        args.extend(["--plan-item-ref", plan_item_ref])
    args.append("--json")
    task_out = runner.invoke(
        app,
        args,
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task_payload = json.loads(task_out.stdout)
    change_payload = task_payload["change"]
    worktree_root = Path(task_payload["worktree_guidance"]["target_workspace_root"])
    worktree_name = str(task_payload["worktree"]["name"])

    monkeypatch.chdir(worktree_root)
    work_file = _work_file(worktree_root)
    work_file.write_text(work_file.read_text(encoding="utf-8") + readme_append, encoding="utf-8")
    snapshot_out = runner.invoke(
        app,
        ["snapshot", "create", "--message", f"{title} completion", "--json"],
        catch_exceptions=False,
    )
    assert snapshot_out.exit_code == 0, snapshot_out.stdout

    land_payload = _land_local_with_rebase_recovery(
        worktree_root,
        change_id=str(change_payload["change_id"]),
        task_id=str(task_payload["task_id"]),
        desired_text=work_file.read_text(encoding="utf-8"),
    )
    assert land_payload["task_status"] == "completed"
    cleanup = land_payload["bound_worktree_cleanup"]
    assert cleanup["status"] in {"removed", "skipped"}
    if cleanup["status"] == "skipped":
        assert cleanup["reason"] in {"current_worktree", "worktree_not_clean"}

    monkeypatch.chdir(repo)
    restore_out = runner.invoke(
        app,
        ["workspace", "restore", "--line", "main", "--force", "--json"],
        catch_exceptions=False,
    )
    assert restore_out.exit_code == 0, restore_out.stdout
    return task_payload


def _land_and_complete_local_task_from_existing_worktree(
    repo: Path,
    monkeypatch,
    *,
    task_payload: dict,
    readme_text: str,
    restore_repo_root: bool = True,
) -> None:
    worktree_root = Path(task_payload["worktree_guidance"]["target_workspace_root"])
    worktree_name = str(task_payload["worktree"]["name"])
    change_id = str(task_payload["change"]["change_id"])
    task_id = str(task_payload["task_id"])

    monkeypatch.chdir(worktree_root)
    work_file = _work_file(worktree_root)
    work_file.write_text(readme_text, encoding="utf-8")
    snapshot_out = runner.invoke(
        app,
        ["snapshot", "create", "--message", f"{task_id} completion", "--json"],
        catch_exceptions=False,
    )
    assert snapshot_out.exit_code == 0, snapshot_out.stdout

    land_payload = _land_local_with_rebase_recovery(
        worktree_root,
        change_id=change_id,
        task_id=task_id,
        desired_text=readme_text,
    )
    assert land_payload["task_status"] == "completed"
    cleanup = land_payload["bound_worktree_cleanup"]
    assert cleanup["status"] in {"removed", "skipped"}
    if cleanup["status"] == "skipped":
        assert cleanup["reason"] in {"current_worktree", "worktree_not_clean"}

    monkeypatch.chdir(repo)
    if restore_repo_root:
        restore_out = runner.invoke(
            app,
            ["workspace", "restore", "--line", "main", "--force", "--json"],
            catch_exceptions=False,
        )
        assert restore_out.exit_code == 0, restore_out.stdout



def test_workflow_land_help_describes_completed_local_later_promotion_surface():
    help_out = runner.invoke(app, ["workflow", "land", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    output = " ".join(help_out.stdout.split())
    assert "later-promotion surface" in output
    assert "solo_local" in output
    assert "completed local slice" in output


def test_completed_local_publish_commands_redirect_to_batch_workflow_land(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-completed-local-publish-guidance"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert seed_out.exit_code == 0, seed_out.stdout

    task_payload = _complete_local_only_batch_task(
        repo,
        monkeypatch,
        title="Complete local slice",
        intent="exercise later-promotion guidance for already-landed local work",
        change_title="Land locally before any remote promotion",
        readme_append="completed locally\n",
    )

    task_publish_out = runner.invoke(app, ["task", "publish", task_payload["task_id"]], catch_exceptions=False)
    assert task_publish_out.exit_code != 0
    task_output = " ".join(((task_publish_out.output or "") + (getattr(task_publish_out, "stderr", "") or "")).split())
    assert "workflow land" in task_output
    assert "--all-completed-local" in task_output
    assert "--remote <name>" in task_output
    assert task_payload["task_id"] in task_output

    change_publish_out = runner.invoke(app, ["change", "publish", task_payload["change"]["change_id"]], catch_exceptions=False)
    assert change_publish_out.exit_code != 0
    change_output = " ".join(((change_publish_out.output or "") + (getattr(change_publish_out, "stderr", "") or "")).split())
    assert "workflow land" in change_output
    assert "--all-completed-local" in change_output
    assert "--remote <name>" in change_output
    assert task_payload["change"]["change_id"] in change_output


def test_workflow_land_all_completed_local_preview_is_read_only_until_apply(tmp_path: Path, monkeypatch):
    repo = tmp_path / "batch-remote-land-preview"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-preview"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-preview"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_payload = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Preview a completed local slice",
            intent="Verify that workflow land batch preview remains read-only until --apply.",
            change_title="Preview the completed local slice",
            readme_append="completed locally\n",
        )

        preview_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert preview_out.exit_code == 0, preview_out.stdout
        payload = json.loads(preview_out.stdout)

        assert payload["status"] == "ready"
        assert payload["mode"] == "all_completed_local"
        assert payload["total_items"] == 1
        assert payload["ready_items"] == 1
        assert payload["completed_items"] == 0
        assert payload["blocked_items"] == 0
        item = payload["items"][0]
        assert item["status"] == "ready"
        assert item["task_id"] == task_payload["task_id"]
        assert item["change_id"] == task_payload["change"]["change_id"]
        assert item["remote_task_id"] == ""
        assert item["remote_change_id"] == task_payload["change"]["change_id"]
        state = item["state"]
        assert state["routing"]["kind"] == "completed_local"

        local_task_out = runner.invoke(
            app,
            ["task", "show", task_payload["task_id"], "--local", "--json"],
            catch_exceptions=False,
        )
        assert local_task_out.exit_code == 0, local_task_out.stdout
        local_task = json.loads(local_task_out.stdout)
        assert local_task["publication_state"] == "local_draft"
        assert local_task["published_task_id"] is None

        local_change_out = runner.invoke(
            app,
            ["change", "show", task_payload["change"]["change_id"], "--local", "--json"],
            catch_exceptions=False,
        )
        assert local_change_out.exit_code == 0, local_change_out.stdout
        local_change = json.loads(local_change_out.stdout)
        assert local_change["publication_state"] == "local_draft"
        assert local_change["published_change_id"] is None

        assert remote_client_module.list_tasks(base_url, repo_name) == []
        assert remote_client_module.list_changes(base_url, repo_name) == []


def test_workflow_land_all_completed_local_skips_completed_tasks_reclassified_as_canceled(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-canceled-completed-slice"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-canceled-completed-slice"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-canceled-completed-slice"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_payload = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Canceled completed local slice",
            intent="Verify that later-promotion-excluded completed local landed slices do not re-enter repo-wide later-promotion.",
            change_title="Exclude the completed local slice from later promotion",
            readme_append="completed then excluded\n",
        )

        canceled_out = runner.invoke(
            app,
            ["task", "canceled", task_payload["task_id"], "--local", "--exclude-later-promotion", "--json"],
            catch_exceptions=False,
        )
        assert canceled_out.exit_code == 0, canceled_out.stdout
        canceled = json.loads(canceled_out.stdout)
        assert canceled["status"] == "later_promotion_excluded"

        with pytest.raises(
            ValueError,
            match="No completed local tasks are available for batch remote land.",
        ):
            cli_module._workflow_batch_local_change_entries(repo_ctx, remote_name="origin")


def test_workflow_land_routes_local_change_ids_without_remote_sequence_collision(tmp_path: Path, monkeypatch):
    repo = tmp_path / "single-local-change-routing"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-single-local-change-routing"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "single-local-change-routing"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        seed_snapshot_id = str(seed_snapshot["snapshot_id"])
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        server_ctx = fake_postgres_context(server_data)
        server_store_module.ensure_repository(server_ctx, repo_name, "main")
        remote_task = server_store_module.create_task(
            server_ctx,
            repo_name,
            "Existing remote sequence collision",
            "Keep a remote RT/RC row at the same numeric sequence.",
            "medium",
            task_id="RT-0001",
        )
        remote_change = server_store_module.create_change(
            server_ctx,
            repo_name,
            str(remote_task["task_id"]),
            "Existing remote RC collision",
            "main",
            "medium",
            change_id="RC-0001",
            fork_snapshot_id=seed_snapshot_id,
            forked_from_line="main",
        )

        local_task = local_control.create_workflow_task(
            repo_ctx,
            "LT-0001",
            repo_name,
            "Promote a collided local slice",
            "Route LC/LT ids through completed-local later-promotion without colliding with RT/RC lookups.",
            "medium",
            status="active",
        )
        local_change = local_control.create_workflow_change(
            repo_ctx,
            "LC-0001",
            str(local_task["task_id"]),
            repo_name,
            "Promote the collided local slice",
            "main",
            "medium",
            "assisted",
            fork_snapshot_id=seed_snapshot_id,
            forked_from_line="main",
            status="draft",
        )
        _work_file(repo).write_text("base\nlocal promotion\n", encoding="utf-8")
        landed_snapshot = create_snapshot(repo_ctx, "land local collided slice")
        local_control.land_workflow_change(
            repo_ctx,
            str(local_change["change_id"]),
            target_line="main",
            landed_snapshot_id=str(landed_snapshot["snapshot_id"]),
        )
        local_control.close_workflow_task(repo_ctx, str(local_task["task_id"]), "completed")

        preview_out = runner.invoke(
            app,
            ["workflow", "land", "LC-0001", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert preview_out.exit_code == 0, preview_out.stdout
        preview_payload = json.loads(preview_out.stdout)
        assert preview_payload["routing"]["kind"] == "completed_local"
        assert preview_payload["routing"]["local_change_id"] == "LC-0001"
        assert preview_payload["change"]["change_id"] == "LC-0001"

        local_change_out = runner.invoke(app, ["change", "show", "LC-0001", "--local", "--json"], catch_exceptions=False)
        assert local_change_out.exit_code == 0, local_change_out.stdout
        local_change_after_preview = json.loads(local_change_out.stdout)
        assert local_change_after_preview["publication_state"] == "local_draft"
        assert local_change_after_preview["published_change_id"] is None

        remote_change_out = runner.invoke(app, ["change", "show", "RC-0001", "--remote", "origin", "--json"], catch_exceptions=False)
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change_before_apply = json.loads(remote_change_out.stdout)
        assert remote_change_before_apply["change_id"] == "RC-0001"
        assert remote_change_before_apply["task_id"] == "RT-0001"

        apply_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "LC-0001",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert apply_out.exit_code == 0, apply_out.stdout
        apply_payload = json.loads(apply_out.stdout)
        assert apply_payload["routing"]["kind"] == "completed_local"
        assert apply_payload["change"]["change_id"] == "LC-0001"
        assert apply_payload["task"]["task_id"] == "LT-0001"
        assert apply_payload["change"]["status"] == "landed"
        assert apply_payload["task"]["status"] == "completed"

        remote_local_change_out = runner.invoke(
            app,
            ["change", "show", "LC-0001", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_local_change_out.exit_code == 0, remote_local_change_out.stdout
        remote_local_change = json.loads(remote_local_change_out.stdout)
        assert remote_local_change["change_id"] == "LC-0001"
        assert remote_local_change["status"] == "landed"
        assert remote_local_change["task_id"] == "LT-0001"

        remote_collision_out = runner.invoke(
            app,
            ["change", "show", "RC-0001", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_collision_out.exit_code == 0, remote_collision_out.stdout
        remote_collision = json.loads(remote_collision_out.stdout)
        assert remote_collision["change_id"] == "RC-0001"
        assert remote_collision["task_id"] == "RT-0001"
        assert remote_collision["status"] == str(remote_change.get("status") or "draft")


def test_workflow_land_all_completed_local_promotes_completed_local_slice_through_native_remote_land(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "batch-remote-land"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "batch-remote-land", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_payload = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote completed local slice",
            intent="Finish work locally and then promote it through the native remote land workflow",
            change_title="Land the completed local slice remotely",
            readme_append="completed locally\n",
        )

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.output
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["mode"] == "all_completed_local"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        item = payload["items"][0]
        assert item["status"] == "completed"
        assert item["task_id"] == task_payload["task_id"]
        assert item["change_id"] == task_payload["change"]["change_id"]
        assert item["remote_task_id"]
        assert item["remote_change_id"]

        local_task_out = runner.invoke(
            app,
            ["task", "show", task_payload["task_id"], "--local", "--json"],
            catch_exceptions=False,
        )
        assert local_task_out.exit_code == 0, local_task_out.stdout
        local_task = json.loads(local_task_out.stdout)
        assert local_task["publication_state"] == "published"
        assert local_task["published_remote_name"] == "origin"
        assert local_task["published_task_id"] == item["remote_task_id"]

        local_change_out = runner.invoke(
            app,
            ["change", "show", task_payload["change"]["change_id"], "--local", "--json"],
            catch_exceptions=False,
        )
        assert local_change_out.exit_code == 0, local_change_out.stdout
        local_change = json.loads(local_change_out.stdout)
        assert local_change["publication_state"] == "published"
        assert local_change["published_remote_name"] == "origin"
        assert local_change["published_change_id"] == item["remote_change_id"]

        remote_task_out = runner.invoke(
            app,
            ["task", "show", item["remote_task_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_task_out.exit_code == 0, remote_task_out.stdout
        remote_task = json.loads(remote_task_out.stdout)
        assert remote_task["status"] == "completed"

        remote_change_out = runner.invoke(
            app,
            ["change", "show", item["remote_change_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["status"] == "landed"


def test_workflow_land_graph_run_session_rejects_worker_session_and_surfaces_owner(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "graph-run-batch"
    repo.mkdir()
    server_data = tmp_path / "server-data-graph-run-batch"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "graph-run-batch"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "graph-run-batch", "--default"],
            catch_exceptions=False,
        ).exit_code == 0

        ctx = fake_postgres_context(server_data)
        server_store_module.ensure_repository(ctx, "graph-run-batch", "main")
        server_store_module.create_session(
            ctx,
            "graph-run-batch",
            "task_graph_run",
            title="owning graph run",
            metadata={"graph_run_id": "GR-1"},
            session_id="S-GRAPH-001",
        )
        server_store_module.create_session(
            ctx,
            "graph-run-batch",
            "agent_run",
            title="worker run",
            metadata={"graph_run_id": "GR-1", "graph_run_session_id": "S-GRAPH-001"},
            session_id="S-WORKER-001",
        )

        out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--graph-run-session",
                "S-WORKER-001",
                "--remote",
                "origin",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert out.exit_code != 0
        assert "S-GRAPH-001" in out.output
        assert "task_graph_run" in out.output


def test_workflow_land_all_completed_local_skips_ineligible_local_draft_rows_and_promotes_eligible_rows(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-skip-ineligible"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-skip-ineligible"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "batch-remote-land-skip-ineligible"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "batch-remote-land-skip-ineligible", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land.md",
            "# Batch Remote Land\n\n## Promotion [plan-ref: batch-remote-land/root]\n\n- [ ] promote the planned row [ref: batch-remote-land/planned]\n",
        )
        plan_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert plan_sync_out.exit_code == 0, plan_sync_out.stdout
        plan_sync_payload = json.loads(plan_sync_out.stdout)
        plan_id = str(plan_sync_payload["results"][0]["plan_id"])

        skipped_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Skipped legacy residue",
            intent="Leave one completed local-only residue without durable plan linkage",
            change_title="Leave legacy residue local-only",
            readme_append="legacy residue\n",
        )
        planned_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote planned local slice",
            intent="Complete one planned local slice and later promote it through native remote land",
            change_title="Land the planned local slice remotely",
            readme_append="planned slice\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land/planned",
        )

        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.output
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["mode"] == "all_completed_local"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        assert payload["blocked_items"] == 0
        assert len(payload["skipped_change_ids"]) == 1
        skipped_row = payload["skipped_change_ids"][0]
        assert skipped_row["task_id"] == skipped_task["task_id"]
        assert skipped_row["change_id"] == skipped_task["change"]["change_id"]
        assert skipped_row["reason"] == "missing_durable_plan_linkage"
        assert "durable plan linkage" in skipped_row["detail"]

        item = payload["items"][0]
        assert item["status"] == "completed"
        assert item["task_id"] == planned_task["task_id"]
        assert item["change_id"] == planned_task["change"]["change_id"]
        assert item["remote_task_id"]
        assert item["remote_change_id"]

        planned_local_task_out = runner.invoke(
            app,
            ["task", "show", planned_task["task_id"], "--local", "--json"],
            catch_exceptions=False,
        )
        assert planned_local_task_out.exit_code == 0, planned_local_task_out.stdout
        planned_local_task = json.loads(planned_local_task_out.stdout)
        assert planned_local_task["publication_state"] == "published"
        assert planned_local_task["published_remote_name"] == "origin"

        skipped_local_task_out = runner.invoke(
            app,
            ["task", "show", skipped_task["task_id"], "--local", "--json"],
            catch_exceptions=False,
        )
        assert skipped_local_task_out.exit_code == 0, skipped_local_task_out.stdout
        skipped_local_task = json.loads(skipped_local_task_out.stdout)
        assert skipped_local_task["publication_state"] == "local_draft"
        assert skipped_local_task["published_task_id"] is None

        remote_change_out = runner.invoke(
            app,
            ["change", "show", item["remote_change_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["status"] == "landed"


def test_workflow_land_all_completed_local_prefers_default_line_and_skips_other_target_lines(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-mixed-target-lines"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-mixed-target-lines"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "batch-remote-land-mixed-target-lines"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "batch-remote-land-mixed-target-lines", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        main_task = local_control.create_workflow_task(
            repo_ctx,
            "T-MAIN",
            "batch-remote-land-mixed-target-lines",
            "Main row",
            "Promote a default-line row",
            "medium",
            status="active",
        )
        main_change = local_control.create_workflow_change(
            repo_ctx,
            "C-MAIN",
            main_task["task_id"],
            "batch-remote-land-mixed-target-lines",
            "Main change",
            "main",
            "medium",
            "assisted",
        )
        _work_file(repo).write_text("base\nmain\n", encoding="utf-8")
        main_snapshot = create_snapshot(repo_ctx, "main row")
        local_control.land_workflow_change(
            repo_ctx,
            main_change["change_id"],
            target_line="main",
            landed_snapshot_id=str(main_snapshot["snapshot_id"]),
        )
        local_control.close_workflow_task(repo_ctx, main_task["task_id"], "completed")

        feature_task = local_control.create_workflow_task(
            repo_ctx,
            "T-FEATURE",
            "batch-remote-land-mixed-target-lines",
            "Feature row",
            "Leave a feature-line completed row",
            "medium",
            status="active",
        )
        feature_change = local_control.create_workflow_change(
            repo_ctx,
            "C-FEATURE",
            feature_task["task_id"],
            "batch-remote-land-mixed-target-lines",
            "Feature change",
            "feature/parent",
            "medium",
            "assisted",
        )
        _work_file(repo).write_text("base\nfeature\n", encoding="utf-8")
        feature_snapshot = create_snapshot(repo_ctx, "feature row")
        local_control.land_workflow_change(
            repo_ctx,
            feature_change["change_id"],
            target_line="feature/parent",
            landed_snapshot_id=str(feature_snapshot["snapshot_id"]),
        )
        local_control.close_workflow_task(repo_ctx, feature_task["task_id"], "completed")

        payload = cli_module._workflow_batch_local_change_entries(repo_ctx, remote_name="origin")

        assert payload["target_line"] == "main"
        assert [entry["change_id"] for entry in payload["entries"]] == [main_change["change_id"]]
        assert len(payload["skipped_change_ids"]) == 1
        skipped_row = payload["skipped_change_ids"][0]
        assert skipped_row["task_id"] == feature_task["task_id"]
        assert skipped_row["change_id"] == feature_change["change_id"]
        assert skipped_row["reason"] == "batch_target_line_mismatch"
        assert "feature/parent" in skipped_row["detail"]
        assert "`main`" in skipped_row["detail"]


def test_workflow_land_all_completed_local_skips_published_done_rows_before_target_selection(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-published-done-residue"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-published-done-residue"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "batch-remote-land-published-done-residue"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "batch-remote-land-published-done-residue", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        done_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Already promoted main row",
            intent="Promote this main row once so a rerun sees already-done published residue",
            change_title="Already promoted main row",
            readme_append="main row\n",
        )
        first_batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert first_batch_out.exit_code == 0, first_batch_out.stdout
        first_payload = json.loads(first_batch_out.stdout)
        assert first_payload["completed_items"] == 1

        feature_task = local_control.create_workflow_task(
            repo_ctx,
            "T-FEATURE",
            "batch-remote-land-published-done-residue",
            "Feature row",
            "Leave a feature-line row after main is already published and done",
            "medium",
            status="active",
        )
        feature_change = local_control.create_workflow_change(
            repo_ctx,
            "C-FEATURE",
            feature_task["task_id"],
            "batch-remote-land-published-done-residue",
            "Feature change",
            "feature/parent",
            "medium",
            "assisted",
        )
        _work_file(repo).write_text("base\nfeature\n", encoding="utf-8")
        feature_snapshot = create_snapshot(repo_ctx, "feature row")
        local_control.land_workflow_change(
            repo_ctx,
            feature_change["change_id"],
            target_line="feature/parent",
            landed_snapshot_id=str(feature_snapshot["snapshot_id"]),
        )
        local_control.close_workflow_task(repo_ctx, feature_task["task_id"], "completed")

        payload = cli_module._workflow_batch_local_change_entries(repo_ctx, remote_name="origin")

        assert payload["target_line"] == "feature/parent"
        assert [entry["change_id"] for entry in payload["entries"]] == [feature_change["change_id"]]
        assert len(payload["skipped_change_ids"]) == 1
        skipped_row = payload["skipped_change_ids"][0]
        assert skipped_row["task_id"] == done_task["task_id"]
        assert skipped_row["change_id"] == done_task["change"]["change_id"]
        assert skipped_row["reason"] == "published_remote_done_residue"
        assert "completed" in skipped_row["detail"]
        assert "landed" in skipped_row["detail"]


def test_workflow_land_single_completed_local_lookup_skips_remote_task_index_for_published_done_rows(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "single-remote-land-published-done-residue"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-single-remote-land-published-done-residue"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "single-remote-land-published-done-residue"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "single-remote-land-published-done-residue", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        done_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Already promoted single row",
            intent="Verify specific completed-local lookups do not build the remote task index once the row is already published and done.",
            change_title="Already promoted single row",
            readme_append="single row\n",
        )
        first_batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert first_batch_out.exit_code == 0, first_batch_out.stdout

        monkeypatch.setattr(
            cli_module,
            "remote_list_tasks",
            lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("specific completed-local lookup should not build the remote task index")),
        )

        with pytest.raises(ValueError, match="is completed and remote change .* is landed"):
            cli_module._workflow_batch_local_change_entries(
                repo_ctx,
                remote_name="origin",
                local_change_id=done_task["change"]["change_id"],
            )


def test_workflow_land_all_completed_local_ignores_tasks_reclassified_as_canceled(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-canceled-completed-slice"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-canceled-completed-slice"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "batch-remote-land-canceled-completed-slice"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "batch-remote-land-canceled-completed-slice", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_payload = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Canceled completed slice",
            intent="Verify reclassified local slices stop participating in completed-local batch land.",
            change_title="Canceled completed slice",
            readme_append="completed locally\n",
        )

        canceled_out = runner.invoke(
            app,
            ["task", "canceled", task_payload["task_id"], "--local", "--exclude-later-promotion", "--json"],
            catch_exceptions=False,
        )
        assert canceled_out.exit_code == 0, canceled_out.stdout
        canceled = json.loads(canceled_out.stdout)
        assert canceled["status"] == "later_promotion_excluded"

        with pytest.raises(
            ValueError,
            match="No completed local tasks are available for batch remote land",
        ):
            cli_module._workflow_batch_local_change_entries(repo_ctx, remote_name="origin")


def test_workflow_land_all_completed_local_skips_published_rows_missing_remote_records(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-skip-missing-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-skip-missing-remote"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "batch-remote-land-skip-missing-remote"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "batch-remote-land-skip-missing-remote", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land_missing_remote.md",
            "# Batch Remote Land Missing Remote\n\n## Promotion [plan-ref: batch-remote-land-missing-remote/root]\n\n- [ ] promote the planned row [ref: batch-remote-land-missing-remote/planned]\n",
        )
        plan_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert plan_sync_out.exit_code == 0, plan_sync_out.stdout
        plan_sync_payload = json.loads(plan_sync_out.stdout)
        plan_id = str(plan_sync_payload["results"][0]["plan_id"])

        phantom_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Published residue without remote record",
            intent="Leave one completed row marked published even though origin no longer has the remote record",
            change_title="Published residue without remote record",
            readme_append="phantom published residue\n",
        )
        repo_ctx = RepoContext.discover(repo)
        mark_local_task_published(
            repo_ctx,
            phantom_task["task_id"],
            remote_name=None,
            published_task_id=phantom_task["task_id"],
        )
        mark_local_change_published(
            repo_ctx,
            phantom_task["change"]["change_id"],
            remote_name=None,
            published_change_id=phantom_task["change"]["change_id"],
            allow_landed=True,
        )

        planned_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote planned local slice after phantom residue",
            intent="Complete one planned local slice and still allow batch remote land to proceed after skipping phantom published residue",
            change_title="Land the planned local slice remotely",
            readme_append="planned slice\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land-missing-remote/planned",
        )

        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        assert len(payload["skipped_change_ids"]) == 1
        skipped_row = payload["skipped_change_ids"][0]
        assert skipped_row["task_id"] == phantom_task["task_id"]
        assert skipped_row["change_id"] == phantom_task["change"]["change_id"]
        assert skipped_row["reason"] == "missing_published_remote_task"
        assert "Unknown task" in skipped_row["detail"]

        item = payload["items"][0]
        assert item["task_id"] == planned_task["task_id"]
        assert item["change_id"] == planned_task["change"]["change_id"]
        assert item["status"] == "completed"

        remote_change_out = runner.invoke(
            app,
            ["change", "show", item["remote_change_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["status"] == "landed"


def test_workflow_land_all_completed_local_skips_rows_when_published_remote_task_is_closed(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-skip-closed-remote-task"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-skip-closed-remote-task"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-skip-closed-remote-task"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land_skip_closed_remote_task.md",
            "# Batch Remote Land Skip Closed Remote Task\n\n## Promotion [plan-ref: batch-remote-land-skip-closed-remote-task/root]\n\n- [ ] promote the planned row [ref: batch-remote-land-skip-closed-remote-task/planned]\n",
        )
        plan_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert plan_sync_out.exit_code == 0, plan_sync_out.stdout
        plan_sync_payload = json.loads(plan_sync_out.stdout)
        plan_id = str(plan_sync_payload["results"][0]["plan_id"])

        residue_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Published residue under closed remote task",
            intent="Leave one completed local row whose remote task was already completed elsewhere",
            change_title="Published residue under closed remote task",
            readme_append="closed remote residue\n",
        )
        repo_ctx = RepoContext.discover(repo)
        mark_local_task_published(
            repo_ctx,
            residue_task["task_id"],
            remote_name=None,
            published_task_id=residue_task["task_id"],
        )
        remote_task = remote_client_module.create_task(
            base_url,
            repo_name,
            "Published residue under closed remote task",
            "Simulate a remote task that was already completed before local batch promotion retries.",
            "medium",
            task_id=residue_task["task_id"],
        )
        assert remote_task["task_id"] == residue_task["task_id"]
        remote_client_module.close_task(base_url, residue_task["task_id"], repo_name=repo_name)

        planned_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote planned local slice after closed remote residue",
            intent="Complete one planned local slice and still allow batch remote land to proceed after skipping the closed-remote-task residue",
            change_title="Land the planned local slice remotely",
            readme_append="planned slice\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land-skip-closed-remote-task/planned",
        )

        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0

        blocked_remote_change_calls: list[str] = []
        original_remote_create_change = cli_module.remote_create_change

        def _guard_remote_create_change(base_url_arg, repo_name_arg, task_id_arg, *args, **kwargs):
            if task_id_arg == residue_task["task_id"]:
                blocked_remote_change_calls.append(str(task_id_arg))
                raise AssertionError("closed remote-task residue should be skipped before remote_create_change")
            return original_remote_create_change(base_url_arg, repo_name_arg, task_id_arg, *args, **kwargs)

        monkeypatch.setattr(cli_module, "remote_create_change", _guard_remote_create_change)

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        assert len(payload["skipped_change_ids"]) == 1
        skipped_row = payload["skipped_change_ids"][0]
        assert skipped_row["task_id"] == residue_task["task_id"]
        assert skipped_row["change_id"] == residue_task["change"]["change_id"]
        assert skipped_row["reason"] == "remote_task_closed_for_new_changes"
        assert "completed and cannot accept new changes" in skipped_row["detail"]
        assert blocked_remote_change_calls == []

        item = payload["items"][0]
        assert item["task_id"] == planned_task["task_id"]
        assert item["change_id"] == planned_task["change"]["change_id"]
        assert item["status"] == "completed"

        remote_change_out = runner.invoke(
            app,
            ["change", "show", item["remote_change_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["status"] == "landed"


def test_workflow_land_all_completed_local_recovers_missing_published_remote_plan(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-missing-remote-plan"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-missing-remote-plan"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-missing-remote-plan"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land_missing_remote_plan.md",
            "# Batch Remote Land Missing Remote Plan\n\n## Promotion [plan-ref: batch-remote-land-missing-remote-plan/root]\n\n- [ ] promote the planned row [ref: batch-remote-land-missing-remote-plan/planned]\n",
        )
        plan_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, "--json"],
            catch_exceptions=False,
        )
        assert plan_sync_out.exit_code == 0, plan_sync_out.stdout
        plan_sync_payload = json.loads(plan_sync_out.stdout)
        plan_id = str(plan_sync_payload["results"][0]["plan_id"])
        plan_revision_id = str(plan_sync_payload["results"][0]["plan_revision_id"])

        mark_local_plan_published(
            repo_ctx,
            plan_id,
            remote_name="origin",
            published_plan_id=plan_id,
            published_head_revision_id="PR-REMOTE-MISSING-1",
            revision_mappings=[(plan_revision_id, "PR-REMOTE-MISSING-1")],
        )

        planned_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote planned local slice after missing remote plan recovery",
            intent="Complete one planned local slice and let batch remote land recover the missing remote plan container first",
            change_title="Land the planned local slice remotely",
            readme_append="planned slice\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land-missing-remote-plan/planned",
        )

        missing_remote_plan_out = runner.invoke(
            app,
            ["plan", "show", plan_id, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert missing_remote_plan_out.exit_code != 0

        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        assert payload["skipped_change_ids"] == []

        item = payload["items"][0]
        assert item["task_id"] == planned_task["task_id"]
        assert item["change_id"] == planned_task["change"]["change_id"]
        assert item["status"] == "completed"

        remote_plan_out = runner.invoke(
            app,
            ["plan", "show", plan_id, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_plan_out.exit_code == 0, remote_plan_out.stdout
        remote_plan = json.loads(remote_plan_out.stdout)
        assert remote_plan["head_revision"]["revision_number"] == 1

        local_plan_out = runner.invoke(app, ["plan", "show", plan_id, "--json"], catch_exceptions=False)
        assert local_plan_out.exit_code == 0, local_plan_out.stdout
        local_plan = json.loads(local_plan_out.stdout)
        assert local_plan["published_head_revision_id"] != "PR-REMOTE-MISSING-1"

        remote_change_out = runner.invoke(
            app,
            ["change", "show", item["remote_change_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["status"] == "landed"


def test_workflow_land_all_completed_local_publishes_required_origin_plan_revision_only(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-origin-plan-revision"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-origin-plan-revision"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-origin-plan-revision"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land_origin_plan_revision.md",
            "# Batch Remote Land Origin Plan Revision\n\n"
            "## Promotion [plan-ref: batch-origin-plan-revision/root]\n\n"
            "- [ ] seed the remote plan [ref: batch-origin-plan-revision/seed]\n",
        )
        initial_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert initial_sync_out.exit_code == 0, initial_sync_out.stdout
        plan_id = str(json.loads(initial_sync_out.stdout)["results"][0]["plan_id"])

        _write_plan_artifact(
            repo,
            plan_file,
            "# Batch Remote Land Origin Plan Revision\n\n"
            "## Promotion [plan-ref: batch-origin-plan-revision/root]\n\n"
            "- [ ] seed the remote plan [ref: batch-origin-plan-revision/seed]\n"
            "- [ ] promote the required origin revision [ref: batch-origin-plan-revision/required-origin]\n",
        )
        target_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"], catch_exceptions=False)
        assert target_sync_out.exit_code == 0, target_sync_out.stdout
        target_revision_id = str(json.loads(target_sync_out.stdout)["results"][0]["plan_revision_id"])

        _write_plan_artifact(
            repo,
            plan_file,
            "# Batch Remote Land Origin Plan Revision\n\n"
            "## Promotion [plan-ref: batch-origin-plan-revision/root]\n\n"
            "- [ ] seed the remote plan [ref: batch-origin-plan-revision/seed]\n"
            "- [ ] promote the required origin revision [ref: batch-origin-plan-revision/required-origin]\n"
            "- [ ] keep this newer local head out of the current task publish [ref: batch-origin-plan-revision/newer-local-head]\n",
        )
        newer_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"], catch_exceptions=False)
        assert newer_sync_out.exit_code == 0, newer_sync_out.stdout
        newer_revision_id = str(json.loads(newer_sync_out.stdout)["results"][0]["plan_revision_id"])
        assert newer_revision_id != target_revision_id

        planned_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote task bound to unpublished origin revision",
            intent="Complete a local task whose required origin sprint revision is unpublished while a newer local head exists.",
            change_title="Land the origin-revision task remotely",
            readme_append="origin revision slice\n",
            plan_id=plan_id,
            plan_revision_id=target_revision_id,
            plan_item_ref="batch-origin-plan-revision/required-origin",
        )

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        item = payload["items"][0]
        assert item["task_id"] == planned_task["task_id"]
        assert item["status"] == "completed"

        local_revisions_out = runner.invoke(app, ["plan", "revisions", plan_id, "--json"], catch_exceptions=False)
        assert local_revisions_out.exit_code == 0, local_revisions_out.stdout
        local_revisions = {
            str(row["plan_revision_id"]): row
            for row in json.loads(local_revisions_out.stdout)
        }
        published_target_revision_id = local_revisions[target_revision_id]["published_plan_revision_id"]
        assert published_target_revision_id
        assert local_revisions[target_revision_id]["publication_state"] == "published"
        assert local_revisions[newer_revision_id]["publication_state"] == "local_draft"
        assert local_revisions[newer_revision_id]["published_plan_revision_id"] is None

        remote_plan_out = runner.invoke(
            app,
            ["plan", "show", plan_id, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_plan_out.exit_code == 0, remote_plan_out.stdout
        remote_plan = json.loads(remote_plan_out.stdout)
        assert remote_plan["head_revision"]["revision_number"] == 2
        assert remote_plan["head_revision"]["plan_revision_id"] == published_target_revision_id

        remote_task_out = runner.invoke(
            app,
            ["task", "show", item["remote_task_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_task_out.exit_code == 0, remote_task_out.stdout
        remote_task = json.loads(remote_task_out.stdout)
        assert remote_task["origin_plan_revision_id"] == published_target_revision_id
        assert remote_task["plan_item_ref"] == "batch-origin-plan-revision/required-origin"


def test_workflow_land_all_completed_local_auto_publishes_rebound_unpublished_local_plan_lineage(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-rebound-local-plan-lineage"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-rebound-local-plan-lineage"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-rebound-local-plan-lineage"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/rebound_local_plan_lineage.md",
            "# Rebound Local Plan Lineage\n\n## Preserve the older local plan [plan-ref: rebound-local-plan-lineage/root]\n\n- [ ] promote the older planned slice [ref: rebound-local-plan-lineage/promote-old]\n",
        )
        plan_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, "--json"],
            catch_exceptions=False,
        )
        assert plan_sync_out.exit_code == 0, plan_sync_out.stdout
        plan_sync_payload = json.loads(plan_sync_out.stdout)
        old_plan_id = str(plan_sync_payload["results"][0]["plan_id"])

        planned_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote completed local slice after plan rebinding",
            intent="Finish one planned local slice and keep its original plan lineage even after the sprint file is rebound",
            change_title="Land the older planned local slice remotely",
            readme_append="planned slice\n",
            plan_id=old_plan_id,
            plan_item_ref="rebound-local-plan-lineage/promote-old",
        )

        _write_plan_artifact(
            repo,
            plan_file,
            "Removed. Historical publication is no longer a supported workflow path.\n",
        )
        rebind_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert rebind_out.exit_code == 0, rebind_out.stdout
        rebind_payload = json.loads(rebind_out.stdout)
        rebound_plan_id = str(rebind_payload["results"][0]["plan_id"])
        assert rebound_plan_id != old_plan_id

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["completed_items"] == 1
        assert payload["skipped_change_ids"] == []

        item = payload["items"][0]
        assert item["task_id"] == planned_task["task_id"]
        assert item["change_id"] == planned_task["change"]["change_id"]
        assert item["status"] == "completed"

        local_plan_out = runner.invoke(app, ["plan", "show", old_plan_id, "--json"], catch_exceptions=False)
        assert local_plan_out.exit_code == 0, local_plan_out.stdout
        local_plan = json.loads(local_plan_out.stdout)
        assert local_plan["publication_state"] == "published"
        assert local_plan["published_remote_name"] == "origin"

        remote_plan_out = runner.invoke(
            app,
            ["plan", "show", old_plan_id, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_plan_out.exit_code == 0, remote_plan_out.stdout
        remote_plan = json.loads(remote_plan_out.stdout)
        assert remote_plan["plan_id"] == old_plan_id

        remote_task_out = runner.invoke(
            app,
            ["task", "show", item["remote_task_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_task_out.exit_code == 0, remote_task_out.stdout
        remote_task = json.loads(remote_task_out.stdout)
        assert remote_task["plan_id"] == old_plan_id
        assert remote_task["plan_item_ref"] == "rebound-local-plan-lineage/promote-old"


def test_workflow_land_all_completed_local_skips_stale_plan_item_ref_residue_when_remote_task_already_owns_ref(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-stale-plan-item-ref"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-stale-plan-item-ref"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-stale-plan-item-ref"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land_stale_plan_item_ref.md",
            "# Batch Remote Land Stale Plan Item Ref\n\n## Promotion [plan-ref: batch-remote-land-stale-plan-item-ref/root]\n\n- [ ] legacy local row [ref: batch-remote-land-stale-plan-item-ref/legacy]\n- [ ] promote the planned row [ref: batch-remote-land-stale-plan-item-ref/planned]\n",
        )
        plan_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert plan_sync_out.exit_code == 0, plan_sync_out.stdout
        plan_sync_payload = json.loads(plan_sync_out.stdout)
        plan_id = str(plan_sync_payload["results"][0]["plan_id"])
        published_revision_id = str(plan_sync_payload["publish_results"][0]["published_head_revision_id"])

        stale_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Legacy local row on stale plan item ref",
            intent="Leave one completed local row on an older execution slice whose plan_item_ref is already owned by a newer remote task.",
            change_title="Leave stale plan-item-ref residue local-only",
            readme_append="stale plan item residue\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land-stale-plan-item-ref/legacy",
        )

        remote_task = remote_client_module.create_task(
            base_url,
            repo_name,
            "Remote owner for stale plan-item ref",
            "Simulate a newer remote task that already owns the same plan item ref.",
            "medium",
            task_id="AITT-0999",
            plan_id=plan_id,
            origin_plan_revision_id=published_revision_id,
            plan_item_ref="batch-remote-land-stale-plan-item-ref/legacy",
        )
        assert remote_task["status"] == "active"
        assert remote_task["task_id"] == "AITT-0999"

        planned_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote planned local slice after stale plan-item ref residue",
            intent="Complete one planned local slice and still allow batch remote land to proceed after skipping stale plan-item-ref residue.",
            change_title="Land the planned local slice remotely",
            readme_append="planned slice\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land-stale-plan-item-ref/planned",
        )

        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        assert len(payload["skipped_change_ids"]) == 1
        skipped_row = payload["skipped_change_ids"][0]
        assert skipped_row["task_id"] == stale_task["task_id"]
        assert skipped_row["change_id"] == stale_task["change"]["change_id"]
        assert skipped_row["reason"] == "stale_plan_item_ref_residue"
        assert remote_task["task_id"] in skipped_row["detail"]
        assert "already linked" in skipped_row["detail"]


def test_workflow_land_all_completed_local_skips_duplicate_local_plan_item_ref_residue_within_same_batch(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-duplicate-local-plan-item-ref"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-duplicate-local-plan-item-ref"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-duplicate-local-plan-item-ref"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land_duplicate_local_plan_item_ref.md",
            "# Batch Remote Land Duplicate Local Plan Item Ref\n\n## Promotion [plan-ref: batch-remote-land-duplicate-local-plan-item-ref/root]\n\n- [ ] shared execution slice [ref: batch-remote-land-duplicate-local-plan-item-ref/shared]\n",
        )
        plan_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert plan_sync_out.exit_code == 0, plan_sync_out.stdout
        plan_sync_payload = json.loads(plan_sync_out.stdout)
        plan_id = str(plan_sync_payload["results"][0]["plan_id"])

        first_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="First local completed row on shared plan item ref",
            intent="Keep the earliest local completed row eligible for remote promotion.",
            change_title="Promote the earliest local row",
            readme_append="first duplicate\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land-duplicate-local-plan-item-ref/shared",
        )

        duplicate_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Later local completed row on shared plan item ref",
            intent="Leave a later local completed row on the same execution slice so batch remote land must skip it.",
            change_title="Do not publish the duplicate local row",
            readme_append="second duplicate\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land-duplicate-local-plan-item-ref/shared",
        )

        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        assert len(payload["skipped_change_ids"]) == 1
        skipped_row = payload["skipped_change_ids"][0]
        assert skipped_row["task_id"] == duplicate_task["task_id"]
        assert skipped_row["change_id"] == duplicate_task["change"]["change_id"]
        assert skipped_row["reason"] == "duplicate_local_plan_item_ref_residue"
        assert first_task["task_id"] in skipped_row["detail"]
        assert first_task["change"]["change_id"] in skipped_row["detail"]

        completed_item = payload["items"][0]
        assert completed_item["task_id"] == first_task["task_id"]
        assert completed_item["change_id"] == first_task["change"]["change_id"]

        remote_tasks = remote_client_module.list_tasks(base_url, repo_name)
        matching_remote_tasks = [
            row
            for row in remote_tasks
            if row.get("plan_id") == plan_id
            and row.get("plan_item_ref") == "batch-remote-land-duplicate-local-plan-item-ref/shared"
        ]
        assert len(matching_remote_tasks) == 1

        assert completed_item["status"] == "completed"

        remote_change_out = runner.invoke(
            app,
            ["change", "show", completed_item["remote_change_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["status"] == "landed"


def test_workflow_land_all_completed_local_skips_execution_only_task_dag_residue_and_later_promotes_final_local_output(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-local-final-dag"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-local-final-dag"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-local-final-dag"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land_local_final_dag.md",
            "# Batch Remote Land Local Final DAG\n\n"
            "## Promotion [plan-ref: batch-remote-land-local-final-dag/root]\n\n"
            "- [ ] Keep execution-only node local-only [ref: batch-remote-land-local-final-dag/execution]\n"
            "- [ ] Later-promote final converged output [ref: batch-remote-land-local-final-dag/final]\n",
        )
        initial_sync = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert initial_sync.exit_code == 0, initial_sync.stdout
        initial_payload = json.loads(initial_sync.stdout)
        plan_id = str(initial_payload["results"][0]["plan_id"])
        local_revision_id = str(initial_payload["results"][0]["plan_revision_id"])
        remote_plan_out = runner.invoke(
            app,
            ["plan", "show", plan_id, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_plan_out.exit_code == 0, remote_plan_out.stdout
        published_revision_id = str(json.loads(remote_plan_out.stdout)["head_revision"]["plan_revision_id"])

        graph_path = repo / "docs/sprints/batch_remote_land_local_final_dag.task_graph.json"
        graph_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "graph_id": "batch-remote-land-local-final-dag/demo",
                    "repo_name": repo_name,
                    "source_plan": {
                        "artifact_path": plan_file,
                        "plan_id": plan_id,
                        "plan_ref": "batch-remote-land-local-final-dag/root",
                        "plan_revision_id": published_revision_id,
                    },
                    "dispatch_artifacts": {
                        "source_markdown": plan_file,
                        "parallel_execution_markdown": plan_file,
                        "task_graph_json": "docs/sprints/batch_remote_land_local_final_dag.task_graph.json",
                    },
                    "execution_policy": {
                        "mode": "guarded_full_dag_convergence",
                        "validate_source_plan_revision": True,
                        "default_mode": "local_execution_dag_with_selective_promotion",
                        "dispatch_model": "compact_packet",
                        "worker_execution_mode": "worker_only_compact_packet",
                        "change_strategy": "local_first_final_local_land",
                        "max_total_sessions": 1,
                        "max_worker_sessions": 1,
                        "max_batch_sessions": 1,
                    },
                    "nodes": [
                        {
                            "node_id": "A",
                            "node_kind": "task",
                            "title": "Execution-only node",
                            "plan_item_ref": "batch-remote-land-local-final-dag/execution",
                            "workflow_boundary": "execution_only",
                            "depends_on": [],
                            "progress_weight": 1,
                            "task_template": {"title": "Execution-only node", "risk_tier": "low"},
                        },
                        {
                            "node_id": "B",
                            "node_kind": "task",
                            "title": "Final node",
                            "plan_item_ref": "batch-remote-land-local-final-dag/final",
                            "workflow_boundary": "reviewable_output",
                            "converged_output": True,
                            "depends_on": ["A"],
                            "progress_weight": 1,
                            "task_template": {"title": "Final node", "change_title": "Later-promote final node", "risk_tier": "medium"},
                        },
                    ],
                    "edges": [{"from": "A", "to": "B", "edge_kind": "depends_on"}],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )
        artifact_sync = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert artifact_sync.exit_code == 0, artifact_sync.stdout
        artifact_payload = json.loads(artifact_sync.stdout)
        assert artifact_payload["artifact_results"][0]["artifact_path"] == "docs/sprints/batch_remote_land_local_final_dag.task_graph.json"
        assert artifact_payload["artifact_results"][0]["plan_revision_id"] == published_revision_id

        execution_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Execution-only residue",
            intent="Leave one execution-only DAG node landed locally so repo-wide later-promotion must skip it.",
            change_title="Keep execution-only DAG output local-only",
            readme_append="execution-only residue\n",
            plan_id=plan_id,
            plan_revision_id=local_revision_id,
            plan_item_ref="batch-remote-land-local-final-dag/execution",
        )
        final_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Final converged output",
            intent="Leave the final converged DAG output landed locally so repo-wide later-promotion can publish it.",
            change_title="Later-promote the final DAG output",
            readme_append="final dag output\n",
            plan_id=plan_id,
            plan_revision_id=local_revision_id,
            plan_item_ref="batch-remote-land-local-final-dag/final",
        )

        local_session = local_control.create_workflow_session(
            repo_ctx,
            "S-LOCAL-DAG-FINAL-1",
            repo_name,
            "agent_run",
            task_id=final_task["task_id"],
            change_id=final_task["change"]["change_id"],
            title="Task DAG final local lineage",
            line_name="main",
            metadata={
                "session_policy": "task_dag_node_bootstrap",
                "graph_id": "batch-remote-land-local-final-dag/demo",
                "graph_run_id": "graph-run-local-final-1",
                "graph_run_session_id": "S-GRAPH-LOCAL-FINAL-1",
                "plan_id": plan_id,
                "plan_revision_id": local_revision_id,
                "plan_item_ref": "batch-remote-land-local-final-dag/final",
                "node_id": "B",
                "workflow_boundary": "reviewable_output",
                "single_path_dag": True,
                "dag_shared_boundary_node": True,
                "remote_workflow_allowed": True,
                "final_remote_disposition_default": False,
            },
        )
        assert local_session["session_id"] == "S-LOCAL-DAG-FINAL-1"

        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["total_items"] == 1
        assert payload["completed_items"] == 1
        assert len(payload["skipped_change_ids"]) == 1
        skipped_row = payload["skipped_change_ids"][0]
        assert skipped_row["task_id"] == execution_task["task_id"]
        assert skipped_row["change_id"] == execution_task["change"]["change_id"]
        assert skipped_row["reason"] == "task_dag_execution_only_residue"

        completed_item = payload["items"][0]
        assert completed_item["task_id"] == final_task["task_id"]
        assert completed_item["change_id"] == final_task["change"]["change_id"]
        assert completed_item["status"] == "completed"

        remote_change_out = runner.invoke(
            app,
            ["change", "show", completed_item["remote_change_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["status"] == "landed"

        remote_task_out = runner.invoke(
            app,
            ["task", "show", completed_item["remote_task_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_task_out.exit_code == 0, remote_task_out.stdout
        remote_task = json.loads(remote_task_out.stdout)
        assert remote_task["status"] == "completed"

        remote_sessions = remote_client_module.list_sessions(base_url, repo_name)
        dag_sessions = [
            row
            for row in remote_sessions
            if row.get("task_id") == completed_item["remote_task_id"]
            and row.get("change_id") == completed_item["remote_change_id"]
            and bool((row.get("metadata") or {}).get("single_path_dag"))
            and bool((row.get("metadata") or {}).get("dag_shared_boundary_node"))
        ]
        assert dag_sessions
        assert dag_sessions[-1]["metadata"]["graph_run_session_id"] == "S-GRAPH-LOCAL-FINAL-1"


def test_workflow_land_all_completed_local_keeps_authoritative_patchset_resume_when_repo_root_has_unrelated_drift(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-authoritative-resume"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-authoritative-resume"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-authoritative-resume"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/batch_remote_land_authoritative_resume.md",
            "# Batch Remote Land Authoritative Resume\n\n## Promotion [plan-ref: batch-remote-land-authoritative-resume/root]\n\n- [ ] promote the planned row [ref: batch-remote-land-authoritative-resume/planned]\n",
        )
        plan_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, *_plan_sync_remote_args("--json")],
            catch_exceptions=False,
        )
        assert plan_sync_out.exit_code == 0, plan_sync_out.stdout
        plan_sync_payload = json.loads(plan_sync_out.stdout)
        plan_id = str(plan_sync_payload["results"][0]["plan_id"])

        planned_task = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Promote planned local slice through partial remote resume",
            intent="Leave one completed local slice whose remote batch promotion stops after creating the authoritative patchset but before review is recorded.",
            change_title="Stop after authoritative patchset creation",
            readme_append="planned slice\n",
            plan_id=plan_id,
            plan_item_ref="batch-remote-land-authoritative-resume/planned",
        )

        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required"], catch_exceptions=False).exit_code == 0

        _work_file(repo).write_text("base\nunrelated repo root drift\n", encoding="utf-8")

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "blocked"
        assert payload["completed_items"] == 0
        assert payload["blocked_items"] == 1

        item = payload["items"][0]
        assert item["task_id"] == planned_task["task_id"]
        assert item["change_id"] == planned_task["change"]["change_id"]
        assert item["status"] == "blocked"
        assert item["current_step"] in {"record_review", "record_code_review_summary"}
        state = item["state"]
        next_action = state["next_action"]
        assert next_action["code"] in {"record_review", "record_code_review_summary"}
        assert next_action["code"] != "snapshot_create"
        assert state["steps"][0]["status"] == "done"
        assert "does not require repo-root authoring state" in state["steps"][0]["detail"]
        assert state["patchset"]["patchset_id"]
        assert state["attestation"]["attestation_id"]


def test_workflow_land_all_completed_local_refreshes_stale_authoritative_patchset_from_repo_root(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-stale-authoritative-patchset"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-stale-authoritative-patchset"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "batch-remote-land-stale-authoritative-patchset"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_payload = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Refresh a stale authoritative batch patchset",
            intent="Partially promote one completed local slice, advance remote main, then rerun batch land from repo root.",
            change_title="Refresh the authoritative patchset from landed local lineage",
            readme_append="completed locally\n",
        )

        first_batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert first_batch_out.exit_code == 0, first_batch_out.stdout
        first_payload = json.loads(first_batch_out.stdout)
        assert first_payload["status"] == "blocked"
        assert first_payload["completed_items"] == 0
        assert first_payload["blocked_items"] == 1

        first_item = first_payload["items"][0]
        assert first_item["task_id"] == task_payload["task_id"]
        assert first_item["change_id"] == task_payload["change"]["change_id"]
        assert first_item["current_step"] == "record_review"
        remote_change_id = str(first_item["remote_change_id"])
        first_patchset_id = str(first_item["state"]["patchset"]["patchset_id"])

        remote_change_out = runner.invoke(
            app,
            ["change", "show", remote_change_id, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["current_patchset_number"] == 1
        assert remote_change["current_patchset_id"] == first_patchset_id

        _work_file(repo).write_text("base\nadvanced remote main\n", encoding="utf-8")
        advanced_snapshot = create_snapshot(repo_ctx, "advance remote main")
        assert advanced_snapshot["snapshot_id"]
        push_out = runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False)
        assert push_out.exit_code == 0, push_out.stdout

        remote_line = remote_client_module.get_remote_line(base_url, repo_name, "main")
        assert str(remote_line["head_snapshot_id"]) == str(advanced_snapshot["snapshot_id"])

        second_batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert second_batch_out.exit_code == 0, second_batch_out.stdout
        assert "read-only deployment workspace" not in second_batch_out.stdout
        second_payload = json.loads(second_batch_out.stdout)
        assert second_payload["status"] == "completed"
        assert second_payload["completed_items"] == 1
        assert second_payload["blocked_items"] == 0

        remote_change_out = runner.invoke(
            app,
            ["change", "show", remote_change_id, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        assert remote_change["status"] == "landed"
        assert remote_change["current_patchset_number"] == 2

        patchset_out = runner.invoke(
            app,
            ["patchset", "show", remote_change["current_patchset_id"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["base_snapshot_id"] == advanced_snapshot["snapshot_id"]


def test_workflow_land_completed_local_refresh_replays_only_landed_delta_when_remote_main_advances(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "single-local-change-refresh-delta-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-single-local-change-refresh-delta-only"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        repo_name = "single-local-change-refresh-delta-only"
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        task_payload = _complete_local_only_batch_task(
            repo,
            monkeypatch,
            title="Refresh one completed local slice against advanced remote main",
            intent="Advance remote main after partial promotion and ensure LC later-promotion replays only its landed delta.",
            change_title="Later-promote one completed local slice after remote main advances",
            readme_append="completed locally\n",
        )
        local_change_id = str(task_payload["change"]["change_id"])

        local_change_out = runner.invoke(
            app,
            ["change", "show", local_change_id, "--local", "--json"],
            catch_exceptions=False,
        )
        assert local_change_out.exit_code == 0, local_change_out.stdout
        local_change = json.loads(local_change_out.stdout)
        landed_snapshot_id = str(local_change["landed_snapshot_id"])
        assert local_change["pre_land_target_snapshot_id"] == str(seed_snapshot["snapshot_id"])
        landed_snapshot = cli_module.get_snapshot(repo_ctx, landed_snapshot_id)
        parent_snapshot_id = str(landed_snapshot["parent_snapshot_id"])

        local_delta = snapshot_diff_module.snapshot_diff(repo_ctx, parent_snapshot_id, landed_snapshot_id)
        assert local_delta["added"] == []
        assert local_delta["deleted"] == []
        assert local_delta["mode_changed"] == []
        assert local_delta["modified"] == ["work.txt"]

        remote_writer = tmp_path / "single-local-change-refresh-delta-only-remote-writer"
        remote_writer.mkdir()
        monkeypatch.chdir(remote_writer)
        assert runner.invoke(app, ["init", "--name", repo_name], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", repo_name, "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        pull_out = runner.invoke(app, ["pull", "--line", "main", "--json"], catch_exceptions=False)
        assert pull_out.exit_code == 0, pull_out.stdout
        (remote_writer / "README.md").write_text("base\nadvanced remote main\n", encoding="utf-8")
        remote_writer_ctx = RepoContext.discover(remote_writer)
        advanced_snapshot = create_snapshot(remote_writer_ctx, "advance remote main with unrelated readme change")
        assert advanced_snapshot["snapshot_id"]
        push_out = runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False)
        assert push_out.exit_code == 0, push_out.stdout
        monkeypatch.chdir(repo)

        apply_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                local_change_id,
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert apply_out.exit_code == 0, apply_out.stdout
        apply_payload = json.loads(apply_out.stdout)
        assert apply_payload["apply_status"] == "done"
        assert apply_payload["change"]["change_id"] == local_change_id
        assert apply_payload["change"]["status"] == "landed"

        remote_change = remote_client_module.get_change(base_url, local_change_id, repo_name=repo_name)
        patchset = remote_client_module.get_patchset(
            base_url,
            str(remote_change["current_patchset_id"]),
            repo_name=repo_name,
            change_ref=local_change_id,
        )
        assert patchset["base_snapshot_id"] == str(advanced_snapshot["snapshot_id"])

        base_bundle = remote_client_module.get_remote_snapshot(
            base_url,
            repo_name,
            str(patchset["base_snapshot_id"]),
        )
        revision_bundle = remote_client_module.get_remote_snapshot(
            base_url,
            repo_name,
            str(patchset["revision_snapshot_id"]),
        )
        patchset_delta = snapshot_diff_module.diff_snapshot_file_maps(
            base_bundle["files"],
            revision_bundle["files"],
            old_snapshot_id=str(patchset["base_snapshot_id"]),
            new_snapshot_id=str(patchset["revision_snapshot_id"]),
        )
        assert patchset_delta["deleted"] == []
        assert patchset_delta["mode_changed"] == []
        assert sorted([*(patchset_delta["added"]), *(patchset_delta["modified"])]) == ["work.txt"]

        remote_files = {row["path"]: row for row in revision_bundle["files"]}
        assert base64.b64decode(remote_files["work.txt"]["content_b64"]).decode("utf-8") == "base\ncompleted locally\n"


def test_workflow_land_all_completed_local_converges_remote_main_across_stale_local_segments(
    tmp_path: Path, monkeypatch
):
    repo = tmp_path / "batch-remote-land-stale-segments"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _work_file(repo).write_text("base\n", encoding="utf-8")
    server_data = tmp_path / "server-data-batch-remote-land-stale-segments"

    with running_server(server_data) as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "batch-remote-land-stale-segments"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "batch-remote-land-stale-segments", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_local"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"], catch_exceptions=False).exit_code == 0
        repo_ctx = RepoContext.discover(repo)
        seed_snapshot = create_snapshot(repo_ctx, "seed")
        assert seed_snapshot["snapshot_id"]

        task_one_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Land first stale segment",
                "--intent",
                "Create the first completed local segment before remote promotion",
                "--risk",
                "medium",
                "--change-title",
                "Remote land the first local segment",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_one_out.exit_code == 0, task_one_out.stdout
        task_one = json.loads(task_one_out.stdout)

        task_two_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Land second stale segment",
                "--intent",
                "Create a sibling completed local segment from the same base before remote promotion",
                "--risk",
                "medium",
                "--change-title",
                "Remote land the second local segment",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_two_out.exit_code == 0, task_two_out.stdout
        task_two = json.loads(task_two_out.stdout)

        _land_and_complete_local_task_from_existing_worktree(
            repo,
            monkeypatch,
            task_payload=task_one,
            readme_text="base\none\n",
            restore_repo_root=False,
        )
        _land_and_complete_local_task_from_existing_worktree(
            repo,
            monkeypatch,
            task_payload=task_two,
            readme_text="base\ntwo\n",
            restore_repo_root=False,
        )

        local_main = cli_module.get_line(repo_ctx, "main")
        local_head_snapshot_id = str(local_main["head_snapshot_id"])
        local_bundle = export_snapshot_bundle(repo_ctx, local_head_snapshot_id)
        local_work_file = next(file for file in local_bundle["files"] if file["path"] == "work.txt")
        assert local_work_file["content_b64"] == "YmFzZQp0d28K"

        batch_out = runner.invoke(
            app,
            [
                "workflow",
                "land",
                "--all-completed-local",
                "--remote",
                "origin",
                "--tests",
                "pass",
                "--lint",
                "pass",
                "--security",
                "pass",
                "--license",
                "pass",
                "--author-mode",
                "human_only",
                "--reviewer",
                "reviewer@example.com",
                "--apply",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert batch_out.exit_code == 0, batch_out.stdout
        payload = json.loads(batch_out.stdout)

        assert payload["status"] == "completed"
        assert payload["mode"] == "all_completed_local"
        assert payload["total_items"] == 2
        assert payload["completed_items"] == 2
        assert payload["blocked_items"] == 0
        assert payload["skipped_change_ids"] == []

        remote_line = remote_client_module.get_remote_line(base_url, "batch-remote-land-stale-segments", "main")
        remote_head_snapshot_id = str(remote_line["head_snapshot_id"])
        remote_bundle = remote_client_module.get_remote_snapshot(
            base_url,
            "batch-remote-land-stale-segments",
            remote_head_snapshot_id,
        )

        assert remote_head_snapshot_id == local_head_snapshot_id
        assert [
            {
                "path": file_row["path"],
                "sha256": file_row["sha256"],
                "mode": file_row["mode"],
                "size_bytes": file_row["size_bytes"],
            }
            for file_row in remote_bundle["files"]
        ] == [
            {
                "path": file_row["path"],
                "sha256": file_row["sha256"],
                "mode": file_row["mode"],
                "size_bytes": file_row["size_bytes"],
            }
            for file_row in local_bundle["files"]
        ]
