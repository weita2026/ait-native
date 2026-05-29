from __future__ import annotations

import pytest

from ait import local_content as local_content_module
from ait import store as store_module
from ait.repo_paths import RepoContext
from ait_protocol.common import connect_sqlite

from ._shared import *  # noqa: F401,F403


def _assert_plan_sync_lineage_only(payload: dict[str, object]) -> None:
    assert "line_sync" not in payload
    assert "root_main_sync" not in payload
    assert "remote_main_sync" not in payload


def test_init_remote_snapshot_flow(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "README.md").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    result = runner.invoke(app, ["init", "--name", "housekeeper", "--json"])
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["repo_name"] == "housekeeper"

    result = runner.invoke(app, ["line", "create", "feature/native-bootstrap", "--json"])
    assert result.exit_code == 0, result.stdout

    result = runner.invoke(app, ["line", "switch", "feature/native-bootstrap", "--json"])
    assert result.exit_code == 0, result.stdout

    (repo / "README.md").write_text("hello native\n", encoding="utf-8")
    result = runner.invoke(app, ["snapshot", "create", "--message", "initial native snapshot", "--json"])
    assert result.exit_code == 0, result.stdout
    snapshot = json.loads(result.stdout)
    assert snapshot["line_name"] == "feature/native-bootstrap"

    result = runner.invoke(app, ["status", "--json"])
    assert result.exit_code == 0, result.stdout
    status = json.loads(result.stdout)
    assert status["head_snapshot_id"] == snapshot["snapshot_id"]


def test_remote_actor_identity_uses_ait_repo_root_from_other_working_directory(tmp_path: Path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir()
    other = tmp_path / "other"
    other.mkdir()
    monkeypatch.chdir(repo)

    init_result = runner.invoke(app, ["init"])
    assert init_result.exit_code == 0, init_result.stdout

    config_result = runner.invoke(app, ["config", "set", "--user-email", "operator@example.com"])
    assert config_result.exit_code == 0, config_result.stdout

    with running_server(tmp_path / "server-data") as base_url:
        remote_add_result = runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--default"],
            catch_exceptions=False,
        )
        assert remote_add_result.exit_code == 0, remote_add_result.stdout
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        monkeypatch.chdir(other)
        monkeypatch.setenv("AIT_REPO_ROOT", str(repo))
        assert remote_client_module._config_actor_identity() == "operator@example.com"

        whoami_result = runner.invoke(app, ["auth", "whoami", "--json"])
        assert whoami_result.exit_code == 0, whoami_result.stdout
        payload = json.loads(whoami_result.stdout)
        assert payload["identity"] == "operator@example.com"
        assert payload["repo_name"] == "repo"


def test_patchset_rerun_ci_surfaces_stale_runtime_guidance(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    monkeypatch.setattr(
        cli_module,
        "remote_run_patchset_ci",
        lambda *args, **kwargs: (_ for _ in ()).throw(
            RemoteError("POST http://example.invalid/v1/native/patchsets/P-1:runCi failed: 405 Method Not Allowed")
        ),
    )
    monkeypatch.setattr(
        cli_module,
        "remote_get_server_health",
        lambda base_url: {
            "ok": True,
            "runtime_root": "/srv/ait",
            "ci_capabilities": {"patchset_run_ci_route": False},
            "ci_readiness": {"runtime_generation": "native_ci_runtime_v1"},
        },
    )

    result = runner.invoke(app, ["patchset", "rerun-ci", "P-1"])
    assert result.exit_code != 0
    output = (result.output or result.stderr or "").lower()
    assert "patchset_run_ci_route" in output
    assert "restart/update" in output
    assert "repo ci-capabilities" in output


def test_repo_run_ci_surfaces_stale_runtime_guidance(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    monkeypatch.setattr(
        cli_module,
        "remote_run_repo_ci",
        lambda *args, **kwargs: (_ for _ in ()).throw(
            RemoteError("POST http://example.invalid/v1/native/admin/repositories/housekeeper:runCi failed: 404 Not Found")
        ),
    )
    monkeypatch.setattr(
        cli_module,
        "remote_get_server_health",
        lambda base_url: {
            "ok": True,
            "runtime_root": "/srv/ait",
            "ci_capabilities": {"repo_run_ci_route": False},
            "ci_readiness": {"runtime_generation": "native_ci_runtime_v1"},
        },
    )

    result = runner.invoke(app, ["repo", "run-ci", "--plane", "nightly"])
    assert result.exit_code != 0
    output = (result.output or result.stderr or "").lower()
    assert "repo_run_ci_route" in output
    assert "restart/update" in output
    assert "repo ci-capabilities" in output


def test_repo_ci_capabilities_reads_healthz(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    monkeypatch.setattr(
        cli_module,
        "remote_get_server_health",
        lambda base_url: {
            "ok": True,
            "runtime_root": "/srv/ait",
            "ci_capabilities": {
                "patchset_run_ci_route": True,
                "repo_run_ci_route": True,
                "patchset_ci_status_route": True,
                "repo_ci_runs_route": True,
                "supported_repo_planes": ["nightly", "release"],
                "supported_task_batch_selectors": ["recent_remote_landed"],
            },
            "ci_readiness": {
                "runtime_generation": "native_ci_runtime_v1",
                "stale_runtime_hint": "restart/update before retrying stale routes",
            },
        },
    )

    result = runner.invoke(app, ["repo", "ci-capabilities", "--json"])
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["repo_name"] == "housekeeper"
    assert payload["ci_capabilities"]["repo_run_ci_route"] is True
    assert payload["ci_readiness"]["runtime_generation"] == "native_ci_runtime_v1"


def test_queue_summary_aggregates_remote_local_and_workspace_state(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-queue-summary"
    repo.mkdir()
    readme = repo / "README.md"
    readme.write_text("base\n", encoding="utf-8")
    app_file = repo / "app.py"
    app_file.write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-queue-summary") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        remote_task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Stabilize queue summary", "--intent", "exercise read-model aggregation", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert remote_task_out.exit_code == 0, remote_task_out.stdout

        local_task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--local", "--title", "Draft local follow-up", "--intent", "keep unpublished work visible", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert local_task_out.exit_code == 0, local_task_out.stdout
        local_task = json.loads(local_task_out.stdout)
        _bind_task_worktree(local_task["task_id"], monkeypatch, name="queue-summary-local")

        local_change_out = runner.invoke(
            app,
            ["change", "create", "--local", "--task", local_task["task_id"], "--title", "Draft queue polish", "--base-line", "main", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert local_change_out.exit_code == 0, local_change_out.stdout
        monkeypatch.chdir(repo)

        app_file.write_text("print('queue summary dirty')\n", encoding="utf-8")

        summary_out = runner.invoke(app, ["queue", "summary", "--json"])
        assert summary_out.exit_code == 0, summary_out.stdout
        payload = json.loads(summary_out.stdout)

        assert payload["summary"]["shared_task_count"] == 1
        assert payload["summary"]["reviewer_inbox_count"] == 0
        assert payload["summary"]["local_draft_task_count"] == 1
        assert payload["summary"]["local_draft_change_count"] == 1
        assert payload["summary"]["workspace_dirty"] is True
        assert payload["summary"]["workspace_changed_count"] == 1
        assert payload["remote"]["configured"] is True
        assert payload["remote"]["task_queue"]["items"][0]["task"]["title"] == "Stabilize queue summary"
        assert payload["remote"]["task_queue"]["items"][0]["next_action"]["code"] == "create_change"




def test_queue_summary_without_default_remote_explains_per_read_override(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-queue-nondefault-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.test:8088", "--repo-name", "housekeeper"],
        catch_exceptions=False,
    ).exit_code == 0

    summary_out = runner.invoke(app, ["queue", "summary", "--json"], catch_exceptions=False)
    assert summary_out.exit_code == 0, summary_out.stdout
    payload = json.loads(summary_out.stdout)

    assert payload["remote"]["configured"] is False
    assert payload["remote"]["available_remotes"] == ["origin"]
    assert payload["remote"]["error"] == "No default remote configured. Set one first, or pass --remote <name> for this queue read."

def test_queue_summary_falls_back_to_local_state_without_remote(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-queue-local-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    _set_plan_task_binding_advisory()
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert snap_out.exit_code == 0, snap_out.stdout

    local_task_out = runner.invoke(
        app,
        ["task", "start", "--task-only", "--local", "--title", "Local-only planning", "--intent", "queue summary should still work", "--risk", "medium", "--json"],
        catch_exceptions=False,
    )
    assert local_task_out.exit_code == 0, local_task_out.stdout

    summary_out = runner.invoke(app, ["queue", "summary", "--json"])
    assert summary_out.exit_code == 0, summary_out.stdout
    payload = json.loads(summary_out.stdout)

    assert payload["remote"]["configured"] is False
    assert payload["summary"]["shared_task_count"] == 0
    assert payload["summary"]["local_draft_task_count"] == 1
    assert payload["summary"]["local_draft_change_count"] == 0
    assert payload["summary"]["workspace_dirty"] is False


def test_queue_summary_uses_cached_worktree_health(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-queue-worktree-cache"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    observed: dict[str, object] = {}

    def fake_worktree_doctor(ctx, *, refresh_status: bool = True):
        observed["refresh_status"] = refresh_status
        return {
            "total_count": 4,
            "current_count": 1,
            "clean_count": 2,
            "dirty_count": 1,
            "missing_count": 1,
            "detached_count": 0,
            "protected_count": 0,
            "safe_auto_remove_count": 0,
            "safe_cleanup_candidate_count": 0,
            "manual_review_candidate_count": 0,
            "healthy": False,
            "stale_count": 1,
            "stale_rows": [],
            "cleanup_candidate_rows": [],
            "manual_review_rows": [],
            "rows": [],
        }

    monkeypatch.setattr(cli_module, "local_worktree_doctor", fake_worktree_doctor)

    summary_out = runner.invoke(app, ["queue", "summary", "--json"])
    assert summary_out.exit_code == 0, summary_out.stdout
    payload = json.loads(summary_out.stdout)

    assert observed["refresh_status"] is False
    assert payload["summary"]["dirty_worktree_count"] == 1
    assert payload["summary"]["stale_worktree_count"] == 1


def test_queue_summary_ignores_completed_local_history_when_reporting_drafts(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-queue-local-history"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    _set_plan_task_binding_advisory()
    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert main_snap_out.exit_code == 0, main_snap_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Local-only queue cleanup",
            "--intent",
            "ensure completed local history does not look like a draft queue item",
            "--change-title",
            "Land completed local-only work",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    started = json.loads(start_out.stdout)
    task_id = started["task_id"]
    change_id = started["change"]["change_id"]
    workspace = _bind_task_worktree(task_id, monkeypatch, name="queue-local-history")

    assert runner.invoke(app, ["line", "create", "feature/local-history", "--switch"]).exit_code == 0
    (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
    feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature", "--json"])
    assert feature_snap_out.exit_code == 0, feature_snap_out.stdout

    land_out = runner.invoke(app, ["workflow", "land-local", change_id, "--target", "main", "--json"])
    assert land_out.exit_code == 0, land_out.stdout
    landed = json.loads(land_out.stdout)
    assert landed["task_status"] == "completed"
    assert landed["change_status"] == "landed"

    summary_out = runner.invoke(app, ["queue", "summary", "--json"])
    assert summary_out.exit_code == 0, summary_out.stdout
    payload = json.loads(summary_out.stdout)

    assert payload["summary"]["local_draft_task_count"] == 0
    assert payload["summary"]["local_draft_change_count"] == 0
    assert payload["local"]["tasks"] == []
    assert payload["local"]["changes"] == []
    assert any(row["task_id"] == task_id and row["status"] == "completed" for row in payload["local"]["all_tasks"])
    assert any(row["change_id"] == change_id and row["status"] == "landed" for row in payload["local"]["all_changes"])

    summary_text_out = runner.invoke(app, ["queue", "summary"])
    assert summary_text_out.exit_code == 0, summary_text_out.stdout
    output = summary_text_out.output or summary_text_out.stdout
    assert "Local-only queue cleanup" not in output
    assert "Land completed local-only work" not in output
    assert "No active shared tasks, local drafts, or workspace changes detected." in output


def test_auth_remote_and_ref_help_frame_shared_identity_and_pointer_story():
    remote_help = runner.invoke(app, ["remote", "--help"])
    assert remote_help.exit_code == 0, remote_help.stdout
    assert "add" in remote_help.stdout
    assert "list" in remote_help.stdout

    remote_add_help = runner.invoke(app, ["remote", "add", "--help"])
    assert remote_add_help.exit_code == 0, remote_add_help.stdout
    assert "Register one shared ait-native remote and optionally make it the default." in " ".join(remote_add_help.stdout.split())

    ref_help = runner.invoke(app, ["ref", "--help"])
    assert ref_help.exit_code == 0, ref_help.stdout
    assert "list" in ref_help.stdout
    assert "show" in ref_help.stdout
    assert "move" in ref_help.stdout

    ref_move_help = runner.invoke(app, ["ref", "move", "--help"])
    assert ref_move_help.exit_code == 0, ref_move_help.stdout
    assert "Move one starter line ref to a different snapshot." in " ".join(ref_move_help.stdout.split())

    auth_help = runner.invoke(app, ["auth", "--help"])
    assert auth_help.exit_code == 0, auth_help.stdout
    assert "whoami" in auth_help.stdout
    assert "grant" in auth_help.stdout
    assert "bindings" in auth_help.stdout

    whoami_help = runner.invoke(app, ["auth", "whoami", "--help"])
    assert whoami_help.exit_code == 0, whoami_help.stdout
    assert "Inspect the current actor identity and effective roles for one repo." in " ".join(whoami_help.stdout.split())

    bindings_help = runner.invoke(app, ["auth", "bindings", "--help"])
    assert bindings_help.exit_code == 0, bindings_help.stdout
    assert "List the shared role bindings recorded for one remote repo." in " ".join(bindings_help.stdout.split())


def test_task_and_change_default_scope_config_controls_authoring(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-scope-authoring"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-scope-authoring") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        set_local = runner.invoke(app, ["config", "set", "--workflow-default-scope", "local", "--json"])
        assert set_local.exit_code == 0, set_local.stdout
        _set_plan_task_binding_advisory()

        local_start = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Local default task",
                "--intent",
                "create a local task and change through config scope",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert local_start.exit_code == 0, local_start.stdout
        local_payload = json.loads(local_start.stdout)
        assert local_payload["publication_state"] == "local_draft"
        assert local_payload["change"]["publication_state"] == "local_draft"

        local_show = runner.invoke(app, ["task", "show", local_payload["task_id"], "--json"])
        assert local_show.exit_code == 0, local_show.stdout
        assert json.loads(local_show.stdout)["publication_state"] == "local_draft"

        remote_override = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--remote",
                "origin",
                "--title",
                "Remote override task",
                "--intent",
                "explicit remote flag wins over local config",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert remote_override.exit_code == 0, remote_override.stdout
        remote_task = json.loads(remote_override.stdout)
        assert "publication_state" not in remote_task
        assert remote_task["title"] == "Remote override task"

        set_remote = runner.invoke(
            app,
            ["config", "set", "--task-default-scope", "remote", "--change-default-scope", "remote", "--json"],
            catch_exceptions=False,
        )
        assert set_remote.exit_code == 0, set_remote.stdout

        remote_task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--title",
                "Remote config task",
                "--intent",
                "task default scope selects remote",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert remote_task_out.exit_code == 0, remote_task_out.stdout
        remote_task = json.loads(remote_task_out.stdout)
        assert "publication_state" not in remote_task

        _bind_task_worktree(remote_task["task_id"], monkeypatch, name="scope-remote-config")
        remote_change_out = runner.invoke(
            app,
            [
                "change",
                "create",
                "--task",
                remote_task["task_id"],
                "--title",
                "Remote config change",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert remote_change_out.exit_code == 0, remote_change_out.stdout
        remote_change = json.loads(remote_change_out.stdout)
        monkeypatch.chdir(repo)
        assert remote_change["task_id"] == remote_task["task_id"]
        assert "publication_state" not in remote_change

        local_override = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--local",
                "--title",
                "Local override task",
                "--intent",
                "explicit local flag wins over remote config",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert local_override.exit_code == 0, local_override.stdout
        assert json.loads(local_override.stdout)["publication_state"] == "local_draft"


def test_task_and_change_show_hint_when_default_remote_misses_local_record(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workflow-show-scope-hint"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-workflow-show-scope-hint") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--local", "--title", "Scope hint task", "--intent", "exercise show hint", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        _bind_task_worktree(task["task_id"], monkeypatch, name="scope-hint-local")

        change_out = runner.invoke(
            app,
            ["change", "create", "--local", "--task", task["task_id"], "--title", "Scope hint change", "--base-line", "main", "--risk", "low", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)
        monkeypatch.chdir(repo)

        task_show_out = runner.invoke(app, ["task", "show", task["task_id"]])
        assert task_show_out.exit_code != 0
        assert f"Task {task['task_id']} was not found on" in task_show_out.output
        assert "--local" in task_show_out.output

        change_show_out = runner.invoke(app, ["change", "show", change["change_id"]])
        assert change_show_out.exit_code != 0
        assert f"Change {change['change_id']} was not found on" in change_show_out.output
        assert "--local" in change_show_out.output


def test_config_set_id_namespace_prefix_can_be_empty_and_cleared(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-id-namespace-config"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    show_out = runner.invoke(app, ["config", "show", "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["id_namespace_prefix"] == {
        "value": "",
        "source": "repo_config",
    }

    set_out = runner.invoke(
        app,
        ["config", "set", "--id-namespace-prefix", "", "--json"],
        catch_exceptions=False,
    )
    assert set_out.exit_code == 0, set_out.stdout
    updated = json.loads(set_out.stdout)
    assert updated["id_namespace_prefix"] == {
        "value": "",
        "source": "repo_config",
    }

    config_path = repo / ".ait" / "config.json"
    saved = json.loads(config_path.read_text(encoding="utf-8"))
    assert saved["id_namespace_prefix"] == ""

    clear_out = runner.invoke(
        app,
        ["config", "set", "--clear-id-namespace-prefix", "--json"],
        catch_exceptions=False,
    )
    assert clear_out.exit_code == 0, clear_out.stdout
    cleared = json.loads(clear_out.stdout)
    assert cleared["id_namespace_prefix"] == {
        "value": "AIT",
        "source": "default",
    }

    saved = json.loads(config_path.read_text(encoding="utf-8"))
    assert "id_namespace_prefix" not in saved


def test_worktree_show_and_sync_refresh_target_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-sync"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-sync"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/sync"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/sync"]).exit_code == 0
    app_file.write_text("print('feature sync')\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature seed", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout

    assert runner.invoke(app, ["line", "switch", "main", "--restore"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "syncme", "--line", "feature/sync", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    monkeypatch.chdir(worktree_path)
    (worktree_path / "app.py").write_text("print('dirty sync')\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    show_out = runner.invoke(app, ["worktree", "show", "syncme", "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["workspace_status"] == "dirty"
    assert shown["changed_count"] == 1
    assert shown["modified_paths"] == ["app.py"]

    blocked_sync = runner.invoke(app, ["worktree", "sync", "syncme"])
    assert blocked_sync.exit_code != 0
    blocked_text = blocked_sync.output or blocked_sync.stdout or blocked_sync.stderr or ""
    assert "unsaved changes" in blocked_text

    sync_out = runner.invoke(app, ["worktree", "sync", "syncme", "--force", "--json"])
    assert sync_out.exit_code == 0, sync_out.stdout
    synced = json.loads(sync_out.stdout)
    assert synced["workspace_status"] == "clean"
    assert synced["restore"]["applied"] is True
    assert synced["restore"]["line_name"] == "feature/sync"
    assert (worktree_path / "app.py").read_text(encoding="utf-8") == "print('feature sync')\n"


def test_push_then_pull_between_native_repos(tmp_path: Path, monkeypatch):
    repo1 = tmp_path / "repo1"
    repo1.mkdir()
    (repo1 / "README.md").write_text("one\n", encoding="utf-8")
    (repo1 / "app.py").write_text("print('one')\n", encoding="utf-8")

    repo2 = tmp_path / "repo2"
    repo2.mkdir()

    with running_server(tmp_path / "server-data") as base_url:
        monkeypatch.chdir(repo1)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        snapshot = json.loads(snap_out.stdout)
        push_out = runner.invoke(app, ["push", "--json"])
        assert push_out.exit_code == 0, push_out.stdout
        pushed = json.loads(push_out.stdout)
        assert pushed["head_snapshot_id"] == snapshot["snapshot_id"]
        assert pushed["checked_snapshots"] == 1
        assert pushed["uploaded_snapshots"] == 1
        assert pushed["skipped_snapshots"] == 0
        assert pushed["pushed_snapshots"] == pushed["uploaded_snapshots"]
        assert pushed["remote_repository"]["repo_name"] == "housekeeper"
        assert pushed["remote_line"]["line_name"] == "main"
        assert pushed["remote_line"]["head_snapshot_id"] == snapshot["snapshot_id"]

        second_push_out = runner.invoke(app, ["push", "--json"])
        assert second_push_out.exit_code == 0, second_push_out.stdout
        second_push = json.loads(second_push_out.stdout)
        assert second_push["head_snapshot_id"] == snapshot["snapshot_id"]
        assert second_push["checked_snapshots"] == 1
        assert second_push["uploaded_snapshots"] == 0
        assert second_push["skipped_snapshots"] == 1
        assert second_push["pushed_snapshots"] == second_push["uploaded_snapshots"]

        headers = {"Accept": "application/json", "Content-Type": "application/json"}
        exists_req = urllib.request.Request(
            f"{base_url}/v1/native/repositories/housekeeper/snapshots:exists",
            data=json.dumps({"snapshot_ids": [snapshot["snapshot_id"], "SNP-MISSING"]}).encode("utf-8"),
            headers=headers,
            method="POST",
        )
        with urllib.request.urlopen(exists_req, timeout=5) as resp:
            assert resp.status == 200
            exists_payload = json.loads(resp.read().decode("utf-8"))
        assert exists_payload["repo_name"] == "housekeeper"
        assert exists_payload["checked_snapshots"] == 2
        assert exists_payload["present"] == [snapshot["snapshot_id"]]
        assert exists_payload["missing"] == ["SNP-MISSING"]

        repo_show_out = runner.invoke(app, ["repo", "show", "--json"])
        assert repo_show_out.exit_code == 0, repo_show_out.stdout
        remote_repo = json.loads(repo_show_out.stdout)
        assert remote_repo["repo_name"] == "housekeeper"
        assert remote_repo["default_line"] == "main"
        assert remote_repo["policy"]["policy_id"] == "prototype"
        assert remote_repo["policy"]["defaults"]["require_tests"] is True
        assert remote_repo["policy"]["defaults"]["require_lint"] is False

        monkeypatch.chdir(repo2)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        pull_out = runner.invoke(app, ["pull", "--line", "main", "--json"])
        assert pull_out.exit_code == 0, pull_out.stdout
        pulled = json.loads(pull_out.stdout)
        assert pulled["head_snapshot_id"] == snapshot["snapshot_id"]

        status_out = runner.invoke(app, ["status", "--json"])
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["head_snapshot_id"] == snapshot["snapshot_id"]

        show_out = runner.invoke(app, ["snapshot", "show", snapshot["snapshot_id"], "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        body = json.loads(show_out.stdout)
        assert [f["path"] for f in body["files"]] == ["app.py"]


def test_fetch_line_imports_remote_snapshots_without_moving_local_line_heads(tmp_path: Path, monkeypatch):
    repo1 = tmp_path / "repo1"
    repo1.mkdir()
    (repo1 / "app.py").write_text("print('seed')\n", encoding="utf-8")

    repo2 = tmp_path / "repo2"
    repo2.mkdir()
    (repo2 / "app.py").write_text("print('local')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-fetch-line") as base_url:
        monkeypatch.chdir(repo1)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert seed_out.exit_code == 0, seed_out.stdout
        assert runner.invoke(app, ["line", "create", "feature/fetch-only"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/fetch-only"], catch_exceptions=False).exit_code == 0
        (repo1 / "app.py").write_text("print('remote feature')\n", encoding="utf-8")
        feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature", "--json"], catch_exceptions=False)
        assert feature_out.exit_code == 0, feature_out.stdout
        feature_snapshot = json.loads(feature_out.stdout)
        push_out = runner.invoke(app, ["push", "--line", "feature/fetch-only", "--json"], catch_exceptions=False)
        assert push_out.exit_code == 0, push_out.stdout

        monkeypatch.chdir(repo2)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        local_out = runner.invoke(app, ["snapshot", "create", "--message", "local", "--json"], catch_exceptions=False)
        assert local_out.exit_code == 0, local_out.stdout
        local_snapshot = json.loads(local_out.stdout)

        fetch_out = runner.invoke(app, ["fetch", "--line", "feature/fetch-only", "--json"], catch_exceptions=False)
        assert fetch_out.exit_code == 0, fetch_out.stdout
        fetched = json.loads(fetch_out.stdout)
        assert fetched["mode"] == "line"
        assert fetched["line"] == "feature/fetch-only"
        assert fetched["head_snapshot_id"] == feature_snapshot["snapshot_id"]
        assert fetched["remote_line"]["line_name"] == "feature/fetch-only"
        assert fetched["local_line_present"] is False
        assert fetched["line_head_updated"] is False
        assert fetched["workspace_restored"] is False
        assert fetched["imported_snapshots"] >= 1

        status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["current_line"] == "main"
        assert status["head_snapshot_id"] == local_snapshot["snapshot_id"]
        assert (repo2 / "app.py").read_text(encoding="utf-8") == "print('local')\n"

        show_out = runner.invoke(app, ["snapshot", "show", feature_snapshot["snapshot_id"], "--json"], catch_exceptions=False)
        assert show_out.exit_code == 0, show_out.stdout
        shown = json.loads(show_out.stdout)
        assert shown["snapshot_id"] == feature_snapshot["snapshot_id"]
        assert shown["files"][0]["path"] == "app.py"


def test_fetch_snapshot_imports_remote_snapshot_without_restoring_workspace(tmp_path: Path, monkeypatch):
    repo1 = tmp_path / "repo1"
    repo1.mkdir()
    (repo1 / "app.py").write_text("print('remote seed')\n", encoding="utf-8")

    repo2 = tmp_path / "repo2"
    repo2.mkdir()
    (repo2 / "app.py").write_text("print('local seed')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-fetch-snapshot") as base_url:
        monkeypatch.chdir(repo1)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        remote_out = runner.invoke(app, ["snapshot", "create", "--message", "remote", "--json"], catch_exceptions=False)
        assert remote_out.exit_code == 0, remote_out.stdout
        remote_snapshot = json.loads(remote_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main", "--json"], catch_exceptions=False).exit_code == 0

        monkeypatch.chdir(repo2)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        local_out = runner.invoke(app, ["snapshot", "create", "--message", "local", "--json"], catch_exceptions=False)
        assert local_out.exit_code == 0, local_out.stdout
        local_snapshot = json.loads(local_out.stdout)

        fetch_out = runner.invoke(app, ["fetch", "--snapshot", remote_snapshot["snapshot_id"], "--json"], catch_exceptions=False)
        assert fetch_out.exit_code == 0, fetch_out.stdout
        fetched = json.loads(fetch_out.stdout)
        assert fetched["mode"] == "snapshot"
        assert fetched["snapshot_id"] == remote_snapshot["snapshot_id"]
        assert fetched["remote_snapshot"]["snapshot_id"] == remote_snapshot["snapshot_id"]
        assert fetched["line_head_updated"] is False
        assert fetched["workspace_restored"] is False
        assert fetched["imported_snapshots"] == 1

        status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["current_line"] == "main"
        assert status["head_snapshot_id"] == local_snapshot["snapshot_id"]
        assert (repo2 / "app.py").read_text(encoding="utf-8") == "print('local seed')\n"

        show_out = runner.invoke(app, ["snapshot", "show", remote_snapshot["snapshot_id"], "--json"], catch_exceptions=False)
        assert show_out.exit_code == 0, show_out.stdout
        shown = json.loads(show_out.stdout)
        assert shown["snapshot_id"] == remote_snapshot["snapshot_id"]


def test_push_creates_remote_line_even_without_snapshots(tmp_path: Path, monkeypatch):
    repo1 = tmp_path / "repo1"
    repo1.mkdir()
    (repo1 / "README.md").write_text("one\n", encoding="utf-8")

    repo2 = tmp_path / "repo2"
    repo2.mkdir()

    with running_server(tmp_path / "server-data-empty-line") as base_url:
        monkeypatch.chdir(repo1)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["line", "create", "feature/empty"]).exit_code == 0

        push_out = runner.invoke(app, ["push", "--line", "feature/empty", "--json"])
        assert push_out.exit_code == 0, push_out.stdout
        pushed = json.loads(push_out.stdout)
        assert pushed["head_snapshot_id"] is None
        assert pushed["pushed_snapshots"] == 0
        assert pushed["checked_snapshots"] == 0
        assert pushed["uploaded_snapshots"] == 0
        assert pushed["skipped_snapshots"] == 0
        assert pushed["remote_line"]["line_name"] == "feature/empty"
        assert pushed["remote_line"]["head_snapshot_id"] is None

        monkeypatch.chdir(repo2)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        pull_out = runner.invoke(app, ["pull", "--line", "feature/empty", "--json"])
        assert pull_out.exit_code == 0, pull_out.stdout
        pulled = json.loads(pull_out.stdout)
        assert pulled["line"] == "feature/empty"
        assert pulled["head_snapshot_id"] is None

        line_out = runner.invoke(app, ["line", "list", "--json"])
        assert line_out.exit_code == 0, line_out.stdout
        lines = json.loads(line_out.stdout)
        feature_lines = [row for row in lines if row["line_name"] == "feature/empty"]
        assert len(feature_lines) == 1
        assert feature_lines[0]["head_snapshot_id"] is None


def test_push_uses_batch_snapshot_existence_check(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-batch-push"
    repo.mkdir()
    app_file = repo / "app.py"
    app_file.write_text("print('one')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
    first_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert first_out.exit_code == 0, first_out.stdout
    first_snapshot = json.loads(first_out.stdout)
    app_file.write_text("print('one')\nprint('two')\n", encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "second", "--json"])
    assert second_out.exit_code == 0, second_out.stdout
    second_snapshot = json.loads(second_out.stdout)

    monkeypatch.setattr(
        cli_module,
        "ensure_repository",
        lambda base_url, repo_name, default_line, policy=None, **kwargs: {
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy or {},
            **({"id_namespace_prefix": kwargs["id_namespace_prefix"]} if "id_namespace_prefix" in kwargs else {}),
        },
    )
    monkeypatch.setattr(
        cli_module,
        "remote_get_repository",
        lambda base_url, repo_name: {"repo_name": repo_name, "default_line": "main"},
    )
    monkeypatch.setattr(
        cli_module,
        "get_remote_line",
        lambda base_url, repo_name, line_name: {"repo_name": repo_name, "line_name": line_name, "head_snapshot_id": None},
    )
    batch_calls: list[list[str]] = []

    def _batch_exists(base_url, repo_name, snapshot_ids):
        batch_calls.append(list(snapshot_ids))
        return {
            "repo_name": repo_name,
            "checked_snapshots": len(snapshot_ids),
            "present": [first_snapshot["snapshot_id"]],
            "missing": [second_snapshot["snapshot_id"]],
        }

    monkeypatch.setattr(cli_module, "get_remote_snapshots_existence", _batch_exists)
    monkeypatch.setattr(
        cli_module,
        "get_remote_snapshot",
        lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("single snapshot GET should not run when batch exists is available")),
    )
    uploaded: list[str] = []

    def _put_snapshot(base_url, repo_name, snapshot_id, bundle, *, storage_ingest_mode=None):
        uploaded.append(snapshot_id)
        return {
            "snapshot_id": bundle["snapshot_id"],
            "repo_name": bundle["repo_name"],
            "line_name": bundle["line_name"],
            "parent_snapshot_id": bundle["parent_snapshot_id"],
            "message": bundle["message"],
            "file_count": bundle["file_count"],
            "total_bytes": bundle["total_bytes"],
        }

    monkeypatch.setattr(cli_module, "put_remote_snapshot", _put_snapshot)
    monkeypatch.setattr(
        cli_module,
        "update_remote_line",
        lambda base_url, repo_name, line_name, head_snapshot_id, expected_head_snapshot_id=None: {
            "repo_name": repo_name,
            "line_name": line_name,
            "head_snapshot_id": head_snapshot_id,
        },
    )

    push_out = runner.invoke(app, ["push", "--json"])
    assert push_out.exit_code == 0, push_out.stdout
    pushed = json.loads(push_out.stdout)
    assert batch_calls == [[first_snapshot["snapshot_id"], second_snapshot["snapshot_id"]]]
    assert uploaded == [second_snapshot["snapshot_id"]]
    assert pushed["checked_snapshots"] == 2
    assert pushed["uploaded_snapshots"] == 1
    assert pushed["skipped_snapshots"] == 1
    assert pushed["pushed_snapshots"] == pushed["uploaded_snapshots"]


def test_push_fails_if_remote_repository_cannot_be_verified(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "README.md").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert snap_out.exit_code == 0, snap_out.stdout

    monkeypatch.setattr(
        cli_module,
        "ensure_repository",
        lambda base_url, repo_name, default_line, policy=None, **kwargs: {
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy or {},
            **({"id_namespace_prefix": kwargs["id_namespace_prefix"]} if "id_namespace_prefix" in kwargs else {}),
        },
    )

    def _raise_remote_get(*args, **kwargs):
        raise RemoteError("GET http://example.invalid/v1/native/repositories/housekeeper failed: 404 Unknown repository")

    monkeypatch.setattr(cli_module, "remote_get_repository", _raise_remote_get)

    push_out = runner.invoke(app, ["push"])
    assert push_out.exit_code != 0
    output = push_out.output or push_out.stderr or ""
    assert "could not be verified" in output
    assert "ensure/create" in output


def test_push_fails_if_uploaded_snapshot_cannot_be_verified(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "app.py").write_text("print('hello')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    monkeypatch.setattr(
        cli_module,
        "ensure_repository",
        lambda base_url, repo_name, default_line, policy=None, **kwargs: {
            "repo_name": repo_name,
            "default_line": default_line,
            "policy": policy or {},
            **({"id_namespace_prefix": kwargs["id_namespace_prefix"]} if "id_namespace_prefix" in kwargs else {}),
        },
    )
    monkeypatch.setattr(
        cli_module,
        "remote_get_repository",
        lambda base_url, repo_name: {"repo_name": repo_name, "default_line": "main"},
    )
    monkeypatch.setattr(
        cli_module,
        "get_remote_line",
        lambda base_url, repo_name, line_name: {"repo_name": repo_name, "line_name": line_name, "head_snapshot_id": None},
    )
    monkeypatch.setattr(
        cli_module,
        "get_remote_snapshots_existence",
        lambda *args, **kwargs: (_ for _ in ()).throw(RemoteError("POST snapshot exists failed: 404 Not Found")),
    )
    monkeypatch.setattr(
        cli_module,
        "get_remote_snapshot",
        lambda *args, **kwargs: (_ for _ in ()).throw(RemoteError("GET snapshot failed: 404 Unknown snapshot")),
    )
    monkeypatch.setattr(
        cli_module,
        "put_remote_snapshot",
        lambda *args, **kwargs: {
            "snapshot_id": "SNP-OTHER",
            "repo_name": "housekeeper",
            "line_name": "main",
            "parent_snapshot_id": None,
            "message": "seed",
            "file_count": 1,
            "total_bytes": 6,
        },
    )
    monkeypatch.setattr(
        cli_module,
        "update_remote_line",
        lambda *args, **kwargs: (_ for _ in ()).throw(AssertionError("line update should not run after snapshot verification fails")),
    )

    push_out = runner.invoke(app, ["push"])
    assert push_out.exit_code != 0
    output = push_out.output or push_out.stderr or ""
    assert "unexpected snapshot" in output
    assert "SNP-OTHER" in output
    assert snapshot["snapshot_id"] in output


def test_pull_fails_if_remote_line_repository_does_not_match_request(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    monkeypatch.setattr(
        cli_module,
        "get_remote_line",
        lambda base_url, repo_name, line_name: {"repo_name": "other-repo", "line_name": line_name, "head_snapshot_id": None},
    )

    pull_out = runner.invoke(app, ["pull", "--line", "main"])
    assert pull_out.exit_code != 0
    output = pull_out.output or pull_out.stderr or ""
    assert "unexpected repository" in output
    assert "other-repo" in output


def test_pull_fails_if_remote_line_name_does_not_match_request(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    monkeypatch.setattr(
        cli_module,
        "get_remote_line",
        lambda base_url, repo_name, line_name: {"repo_name": repo_name, "line_name": "feature/other", "head_snapshot_id": None},
    )

    pull_out = runner.invoke(app, ["pull", "--line", "main"])
    assert pull_out.exit_code != 0
    output = pull_out.output or pull_out.stderr or ""
    assert "unexpected line" in output
    assert "feature/other" in output


def test_pull_fails_if_remote_snapshot_bundle_snapshot_id_does_not_match_request(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    monkeypatch.setattr(
        cli_module,
        "get_remote_line",
        lambda base_url, repo_name, line_name: {"repo_name": repo_name, "line_name": line_name, "head_snapshot_id": "SNP-REMOTE"},
    )
    monkeypatch.setattr(
        cli_module,
        "get_remote_snapshot",
        lambda base_url, repo_name, snapshot_id: {
            "snapshot_id": "SNP-OTHER",
            "repo_name": repo_name,
            "line_name": "main",
            "parent_snapshot_id": None,
            "files": [],
        },
    )

    pull_out = runner.invoke(app, ["pull", "--line", "main"])
    assert pull_out.exit_code != 0
    output = pull_out.output or pull_out.stderr or ""
    assert "unexpected snapshot" in output
    assert "SNP-OTHER" in output


def test_pull_fails_if_remote_snapshot_bundle_repository_does_not_match_request(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.invalid", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

    monkeypatch.setattr(
        cli_module,
        "get_remote_line",
        lambda base_url, repo_name, line_name: {"repo_name": repo_name, "line_name": line_name, "head_snapshot_id": "SNP-REMOTE"},
    )
    monkeypatch.setattr(
        cli_module,
        "get_remote_snapshot",
        lambda base_url, repo_name, snapshot_id: {
            "snapshot_id": snapshot_id,
            "repo_name": "other-repo",
            "line_name": "main",
            "parent_snapshot_id": None,
            "files": [],
        },
    )

    pull_out = runner.invoke(app, ["pull", "--line", "main"])
    assert pull_out.exit_code != 0
    output = pull_out.output or pull_out.stderr or ""
    assert "unexpected repository" in output
    assert "other-repo" in output


def test_line_list_remote_lists_shared_lines(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-list-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-line-list-remote") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["line", "create", "review-base/example", "--from-snapshot", seed_snapshot_id]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "review-base/example"]).exit_code == 0

        remote_lines_out = runner.invoke(app, ["line", "list", "--remote", "origin", "--json"])
        assert remote_lines_out.exit_code == 0, remote_lines_out.stdout
        line_names = {row["line_name"] for row in json.loads(remote_lines_out.stdout)}
        assert {"main", "review-base/example"} <= line_names


def test_patchset_publish_from_main_does_not_advance_remote_base(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-main-review-base-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-main-review-base-guard") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        base_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        (repo / "app.py").write_text("print('main-line work')\n", encoding="utf-8")
        revision_out = runner.invoke(app, ["snapshot", "create", "--message", "main line work", "--json"])
        assert revision_out.exit_code == 0, revision_out.stdout
        revision_snapshot_id = json.loads(revision_out.stdout)["snapshot_id"]

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--remote", "origin", "--title", "Main review guard", "--intent", "publish from main without moving remote main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        _bind_task_worktree(task["task_id"], monkeypatch, name="main-review-guard", line_name="main")
        change_out = runner.invoke(
            app,
            ["change", "create", "--remote", "origin", "--task", task["task_id"], "--title", "Main review guard", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "main review guard", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["base_snapshot_id"] == base_snapshot_id
        assert patchset["revision_snapshot_id"] == revision_snapshot_id
        assert patchset["publish_context"]["line_updated"] is False

        remote_lines_out = runner.invoke(app, ["line", "list", "--remote", "origin", "--json"])
        assert remote_lines_out.exit_code == 0, remote_lines_out.stdout
        remote_main = next(row for row in json.loads(remote_lines_out.stdout) if row["line_name"] == "main")
        assert remote_main["head_snapshot_id"] == base_snapshot_id


def test_task_start_creates_remote_task_and_initial_change(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-task-start-remote") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Bootstrap remote workflow",
                "--intent",
                "create the first review unit in one command",
                "--change-title",
                "Bootstrap remote change",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        assert payload["task_id"].startswith("RT-")
        assert payload["title"] == "Bootstrap remote workflow"
        assert payload["change"]["change_id"].startswith("RC-")
        assert payload["change"]["task_id"] == payload["task_id"]
        assert payload["change"]["title"] == "Bootstrap remote change"
        assert payload["change"]["base_line"] == "main"

        task_show_out = runner.invoke(app, ["task", "show", payload["task_id"], "--json"])
        assert task_show_out.exit_code == 0, task_show_out.stdout
        shown_task = json.loads(task_show_out.stdout)
        assert shown_task["intent"] == "create the first review unit in one command"

        change_show_out = runner.invoke(app, ["change", "show", payload["change"]["change_id"], "--json"])
        assert change_show_out.exit_code == 0, change_show_out.stdout
        shown_change = json.loads(change_show_out.stdout)
        assert shown_change["task_id"] == payload["task_id"]
        assert shown_change["title"] == "Bootstrap remote change"


def test_local_plan_can_be_revised_synced_to_remote_without_losing_revision_history(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-plan-draft"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-local-plan-draft") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/local_publish.md",
            "# Local Plan Publishing\n\n## Stabilize Local Publishing [plan-ref: local-plan-publish/stabilize]\n\n- [ ] create local draft plans [ref: local-plan-publish/create-drafts]\n",
        )

        create_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert create_out.exit_code == 0, create_out.stdout
        create_payload = json.loads(create_out.stdout)
        assert create_payload["summary"]["created_count"] == 1
        plan_id = create_payload["results"][0]["plan_id"]
        plan_show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
        assert plan_show_out.exit_code == 0, plan_show_out.stdout
        plan = json.loads(plan_show_out.stdout)
        assert plan["publication_state"] == "local_draft"
        assert plan["head_revision"]["revision_number"] == 1

        _write_plan_artifact(
            repo,
            plan_file,
            "# Local Plan Publishing\n\n## Stabilize Local Publishing [plan-ref: local-plan-publish/stabilize]\n\n- [x] create local draft plans [ref: local-plan-publish/create-drafts]\n- [ ] sync revisions to remote [ref: local-plan-publish/sync-revisions]\n",
        )
        revise_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert revise_out.exit_code == 0, revise_out.stdout
        revise_payload = json.loads(revise_out.stdout)
        assert revise_payload["summary"]["updated_count"] == 1
        revised_show_out = runner.invoke(app, ["plan", "show", plan["plan_id"], "--json"])
        assert revised_show_out.exit_code == 0, revised_show_out.stdout
        revised = json.loads(revised_show_out.stdout)
        assert revised["head_revision"]["revision_number"] == 2
        assert revised["head_revision"]["publication_state"] == "local_draft"

        list_out = runner.invoke(app, ["plan", "list", "--json"])
        assert list_out.exit_code == 0, list_out.stdout
        listed = json.loads(list_out.stdout)
        assert [row["plan_id"] for row in listed] == [plan["plan_id"]]
        assert listed[0]["head_revision_number"] == 2

        publish_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert publish_out.exit_code == 0, publish_out.stdout
        publish_payload = json.loads(publish_out.stdout)
        assert publish_payload["summary"]["unchanged_count"] == 1
        assert publish_payload["summary"]["published_count"] == 1
        published = publish_payload["publish_results"][0]
        assert published["plan_id"] == plan["plan_id"]
        assert published["publication_state"] == "published"
        assert published["published_head_revision_id"]
        assert published["head_publication_state"] == "published"
        second_remote_head = published["published_head_revision_id"]

        remote_show_out = runner.invoke(app, ["plan", "show", plan["plan_id"], "--remote", "origin", "--json"])
        assert remote_show_out.exit_code == 0, remote_show_out.stdout
        remote_plan = json.loads(remote_show_out.stdout)
        assert remote_plan["title"] == "Stabilize Local Publishing"
        assert remote_plan["status"] == "draft"
        assert remote_plan["head_revision"]["revision_number"] == 2

        republish_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert republish_out.exit_code == 0, republish_out.stdout
        republish_payload = json.loads(republish_out.stdout)
        assert republish_payload["summary"]["published_count"] == 0
        assert republish_payload["publish_results"] == []

        _write_plan_artifact(
            repo,
            plan_file,
            "# Local Plan Publishing\n\n## Stabilize Local Publishing [plan-ref: local-plan-publish/stabilize]\n\n- [x] create local draft plans [ref: local-plan-publish/create-drafts]\n- [x] sync revisions to remote [ref: local-plan-publish/sync-revisions]\n- [ ] sync follow-up revision [ref: local-plan-publish/sync-follow-up]\n",
        )
        revise_again_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert revise_again_out.exit_code == 0, revise_again_out.stdout
        revise_again_payload = json.loads(revise_again_out.stdout)
        assert revise_again_payload["summary"]["updated_count"] == 1
        revised_again_show_out = runner.invoke(app, ["plan", "show", plan["plan_id"], "--json"])
        assert revised_again_show_out.exit_code == 0, revised_again_show_out.stdout
        revised_again = json.loads(revised_again_show_out.stdout)
        assert revised_again["head_revision"]["revision_number"] == 3
        assert revised_again["head_revision"]["publication_state"] == "local_draft"

        publish_again_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert publish_again_out.exit_code == 0, publish_again_out.stdout
        publish_again_payload = json.loads(publish_again_out.stdout)
        assert publish_again_payload["summary"]["published_count"] == 1
        republished = publish_again_payload["publish_results"][0]
        assert republished["status"] == "draft"
        assert republished["published_head_revision_id"] != second_remote_head
        assert republished["head_publication_state"] == "published"

        remote_revisions_out = runner.invoke(app, ["plan", "revisions", plan["plan_id"], "--remote", "origin", "--json"])
        assert remote_revisions_out.exit_code == 0, remote_revisions_out.stdout
        remote_revisions = json.loads(remote_revisions_out.stdout)
        assert [row["revision_number"] for row in remote_revisions] == [3, 2, 1]

        remote_show_after_out = runner.invoke(app, ["plan", "show", plan["plan_id"], "--remote", "origin", "--json"])
        assert remote_show_after_out.exit_code == 0, remote_show_after_out.stdout
        remote_after = json.loads(remote_show_after_out.stdout)
        assert remote_after["status"] == "draft"
        assert remote_after["head_revision"]["revision_number"] == 3


def test_plan_sync_remote_reconcile_republishes_current_local_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-reconcile"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-reconcile") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/reconcile.md",
            "# Reconcile Plan\n\n## Keep Shared Plan In Sync [plan-ref: reconcile/root]\n\n- [ ] seed local publish [ref: reconcile/seed]\n",
        )

        local_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert local_sync_out.exit_code == 0, local_sync_out.stdout
        plan_id = json.loads(local_sync_out.stdout)["results"][0]["plan_id"]

        publish_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert publish_out.exit_code == 0, publish_out.stdout

        _write_plan_artifact(
            repo,
            plan_file,
            "# Reconcile Plan\n\n## Keep Shared Plan In Sync [plan-ref: reconcile/root]\n\n- [x] seed local publish [ref: reconcile/seed]\n- [ ] remote-only advance [ref: reconcile/remote-only]\n",
        )
        remote_only = _revise_remote_plan_from_artifact(
            base_url,
            plan_id,
            (repo / plan_file).read_text(encoding="utf-8"),
            "reconcile/root",
            artifact_path=plan_file,
            summary="remote-only advance",
            expected_head_revision_id=json.loads(publish_out.stdout)["publish_results"][0]["published_head_revision_id"],
        )
        remote_head_before = remote_only["head_revision"]["plan_revision_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Reconcile Plan\n\n## Keep Shared Plan In Sync [plan-ref: reconcile/root]\n\n- [x] seed local publish [ref: reconcile/seed]\n- [ ] local reconcile publish [ref: reconcile/local]\n",
        )
        revise_local_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert revise_local_out.exit_code == 0, revise_local_out.stdout
        assert json.loads(revise_local_out.stdout)["summary"]["updated_count"] == 1

        fail_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert fail_out.exit_code != 0
        normalized_fail_output = " ".join(fail_out.output.split())
        assert "retry the shared publish with `--rebase`" in normalized_fail_output
        assert "legacy `--reconcile` retry path" in normalized_fail_output

        reconcile_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, "--remote", "origin", "--reconcile", "--json"],
            catch_exceptions=False,
        )
        assert reconcile_out.exit_code == 0, reconcile_out.stdout
        reconcile_payload = json.loads(reconcile_out.stdout)
        assert reconcile_payload["summary"]["published_count"] == 1
        published = reconcile_payload["publish_results"][0]
        assert published["publish_action"] == "reconciled"
        assert published["reconciled"] is True
        assert published["published_revision_count"] == 1
        assert published["reconcile_mode"] == "publish_head"
        assert published["reconcile_remote_head_revision_id"] == remote_head_before

        remote_show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert remote_show_out.exit_code == 0, remote_show_out.stdout
        remote_plan = json.loads(remote_show_out.stdout)
        assert remote_plan["head_revision"]["revision_number"] == 3
        assert [item["plan_item_ref"] for item in remote_plan["head_revision"]["items"]] == ["reconcile/seed", "reconcile/local"]

        local_show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
        assert local_show_out.exit_code == 0, local_show_out.stdout
        local_plan = json.loads(local_show_out.stdout)
        assert local_plan["published_head_revision_id"] == remote_plan["head_revision"]["plan_revision_id"]
        assert local_plan["head_revision"]["publication_state"] == "published"


def test_plan_sync_remote_rebase_republishes_current_local_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-rebase"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-rebase") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/rebase.md",
            "# Rebase Plan\n\n## Keep Shared Plan In Sync [plan-ref: rebase/root]\n\n- [ ] seed local publish [ref: rebase/seed]\n",
        )

        local_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert local_sync_out.exit_code == 0, local_sync_out.stdout
        plan_id = json.loads(local_sync_out.stdout)["results"][0]["plan_id"]

        publish_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert publish_out.exit_code == 0, publish_out.stdout

        _write_plan_artifact(
            repo,
            plan_file,
            "# Rebase Plan\n\n## Keep Shared Plan In Sync [plan-ref: rebase/root]\n\n- [x] seed local publish [ref: rebase/seed]\n- [ ] remote-only advance [ref: rebase/remote-only]\n",
        )
        remote_only = _revise_remote_plan_from_artifact(
            base_url,
            plan_id,
            (repo / plan_file).read_text(encoding="utf-8"),
            "rebase/root",
            artifact_path=plan_file,
            summary="remote-only advance",
            expected_head_revision_id=json.loads(publish_out.stdout)["publish_results"][0]["published_head_revision_id"],
        )
        remote_head_before = remote_only["head_revision"]["plan_revision_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Rebase Plan\n\n## Keep Shared Plan In Sync [plan-ref: rebase/root]\n\n- [x] seed local publish [ref: rebase/seed]\n- [ ] local rebase publish [ref: rebase/local]\n",
        )
        revise_local_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert revise_local_out.exit_code == 0, revise_local_out.stdout
        assert json.loads(revise_local_out.stdout)["summary"]["updated_count"] == 1

        fail_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert fail_out.exit_code != 0
        normalized_fail_output = " ".join(fail_out.output.split())
        assert "retry the shared publish with `--rebase`" in normalized_fail_output
        assert "legacy `--reconcile` retry path" in normalized_fail_output

        rebase_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, "--remote", "origin", "--rebase", "--json"],
            catch_exceptions=False,
        )
        assert rebase_out.exit_code == 0, rebase_out.stdout
        rebase_payload = json.loads(rebase_out.stdout)
        assert rebase_payload["summary"]["published_count"] == 1
        published = rebase_payload["publish_results"][0]
        assert published["publish_action"] == "rebased"
        assert published["rebased"] is True
        assert published["published_revision_count"] == 1
        assert published["rebase_mode"] == "publish_head"
        assert published["rebase_remote_head_revision_id"] == remote_head_before
        assert published["reconciled"] is False

        remote_show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert remote_show_out.exit_code == 0, remote_show_out.stdout
        remote_plan = json.loads(remote_show_out.stdout)
        assert remote_plan["head_revision"]["revision_number"] == 3
        assert [item["plan_item_ref"] for item in remote_plan["head_revision"]["items"]] == ["rebase/seed", "rebase/local"]

        local_show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
        assert local_show_out.exit_code == 0, local_show_out.stdout
        local_plan = json.loads(local_show_out.stdout)
        assert local_plan["published_head_revision_id"] == remote_plan["head_revision"]["plan_revision_id"]
        assert local_plan["head_revision"]["publication_state"] == "published"


def test_task_publish_help_describes_local_to_remote_promotion():
    help_out = runner.invoke(app, ["task", "publish", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "Promote a local draft task into shared remote workflow state." in help_out.stdout


def test_change_publish_help_describes_local_to_remote_promotion():
    help_out = runner.invoke(app, ["change", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "Promote a local draft change into shared remote workflow state" in help_out.stdout


def test_land_submit_help_describes_guarded_remote_integration_role():
    help_out = runner.invoke(app, ["land", "submit", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "Submit the guarded remote integration step after review," in help_out.stdout
    assert "attestation, and" in help_out.stdout
    assert "policy gates pass." in help_out.stdout


def test_remote_session_create_allows_unpublished_line_metadata(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-session-unpublished-line"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session-unpublished-line") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        assert runner.invoke(app, ["line", "create", "feature/session-before-push"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/session-before-push"]).exit_code == 0

        session_out = runner.invoke(
            app,
            [
                "session",
                "create",
                "--kind",
                "agent_run",
                "--title",
                "Remote unpublished line session",
                "--objective",
                "Capture local line metadata before push",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)
        assert session["line_name"] == "feature/session-before-push"
        assert session["metadata"]["objective"] == "Capture local line metadata before push"


def test_remote_session_ids_are_globally_unique_across_repositories(tmp_path: Path, monkeypatch):
    repo_a = tmp_path / "housekeeper-remote-session-a"
    repo_a.mkdir()
    (repo_a / "README.md").write_text("repo a\n", encoding="utf-8")

    repo_b = tmp_path / "housekeeper-remote-session-b"
    repo_b.mkdir()
    (repo_b / "README.md").write_text("repo b\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session-global-ids") as base_url:
        monkeypatch.chdir(repo_a)
        assert runner.invoke(app, ["init", "--name", "housekeeper-a"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-a", "--default"]).exit_code == 0
        assert runner.invoke(
            app,
            ["config", "set", "--workflow-mode", "solo_remote", "--id-namespace-prefix", "AAA"],
            catch_exceptions=False,
        ).exit_code == 0
        snap_a = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert snap_a.exit_code == 0, snap_a.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        session_a_out = runner.invoke(
            app,
            ["session", "create", "--kind", "agent_run", "--title", "Repo A session", "--json"],
            catch_exceptions=False,
        )
        assert session_a_out.exit_code == 0, session_a_out.stdout
        session_a = json.loads(session_a_out.stdout)

        monkeypatch.chdir(repo_b)
        assert runner.invoke(app, ["init", "--name", "housekeeper-b"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-b", "--default"]).exit_code == 0
        assert runner.invoke(
            app,
            ["config", "set", "--workflow-mode", "solo_remote", "--id-namespace-prefix", "BBB"],
            catch_exceptions=False,
        ).exit_code == 0
        snap_b = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert snap_b.exit_code == 0, snap_b.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        session_b_out = runner.invoke(
            app,
            ["session", "create", "--kind", "agent_run", "--title", "Repo B session", "--json"],
            catch_exceptions=False,
        )
        assert session_b_out.exit_code == 0, session_b_out.stdout
        session_b = json.loads(session_b_out.stdout)

        assert session_a["session_id"] != session_b["session_id"]


def test_remote_session_analyze_recognizes_implicit_ait_command_paths(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-session-analysis"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session-analysis") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        session_out = runner.invoke(
            app,
            ["session", "create", "--kind", "agent_run", "--title", "Analyze remote command usage", "--json"],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)

        append_out = runner.invoke(
            app,
            ["session", "append", session["session_id"], "--type", "tool.result", "--field", "command=policy eval", "--json"],
            catch_exceptions=False,
        )
        assert append_out.exit_code == 0, append_out.stdout

        analyze_out = runner.invoke(
            app,
            ["session", "analyze", session["session_id"], "--json"],
            catch_exceptions=False,
        )
        assert analyze_out.exit_code == 0, analyze_out.stdout
        analysis = json.loads(analyze_out.stdout)

        assert analysis["ait_command_count"] == 1
        assert analysis["unscoped_ait_command_count"] == 1
        assert analysis["command_paths"] == [{"command_path": "policy eval", "count": 1, "example": "policy eval"}]


def test_remote_session_autolog_tracks_commands_on_default_remote(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-session-autolog"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session-autolog-remote") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        session_out = runner.invoke(
            app,
            ["session", "create", "--kind", "agent_run", "--title", "Autolog remote command usage", "--json"],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)

        prompt_out = runner.invoke(
            app,
            ["session", "append", session["session_id"], "--type", "session.message", "--text", "What is my current workspace state?", "--json"],
            catch_exceptions=False,
        )
        assert prompt_out.exit_code == 0, prompt_out.stdout

        monkeypatch.setenv("AIT_SESSION_ID", session["session_id"])
        monkeypatch.delenv("AIT_SESSION_LOCAL", raising=False)
        monkeypatch.delenv("AIT_SESSION_REMOTE", raising=False)
        monkeypatch.setenv("AIT_SESSION_AUTOLOG", "1")

        for argv in (
            ["status", "--json"],
            ["config", "show", "--json"],
        ):
            result = runner.invoke(app, argv, catch_exceptions=False)
            assert result.exit_code == 0, result.stdout

        monkeypatch.setenv("AIT_SESSION_AUTOLOG", "0")

        reply_out = runner.invoke(
            app,
            ["session", "append", session["session_id"], "--type", "assistant.reply", "--text", "Status and config are available.", "--json"],
            catch_exceptions=False,
        )
        assert reply_out.exit_code == 0, reply_out.stdout

        events_out = runner.invoke(app, ["session", "events", session["session_id"], "--json"])
        assert events_out.exit_code == 0, events_out.stdout
        events = json.loads(events_out.stdout)
        started = [row for row in events if row["payload"].get("command_phase") == "started"]
        finished = [row for row in events if row["payload"].get("command_phase") == "finished"]
        assert len(started) == 2
        assert len(finished) == 2
        assert all(row["payload"]["capture_mode"] == "auto" for row in started)

        analyze_out = runner.invoke(
            app,
            ["session", "analyze", session["session_id"], "--json"],
            catch_exceptions=False,
        )
        assert analyze_out.exit_code == 0, analyze_out.stdout
        analysis = json.loads(analyze_out.stdout)

        assert analysis["ait_command_count"] == 2
        assert any(row == {"capture_mode": "auto", "count": 2} for row in analysis["capture_modes"])
        assert len(analysis["conversation_turns"]) == 1
        assert analysis["conversation_turns"][0]["ait_command_count"] == 2
        assert {row["command_path"] for row in analysis["command_paths"]} == {"config show", "status"}


def test_task_canceled_rejects_remote_completion_while_change_is_open(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-task-open-change"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-task-open-change") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Stabilize push", "--intent", "exercise task completion guard", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        _bind_task_worktree(task["task_id"], monkeypatch, name="remote-open-change")

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Open review", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        close_out = runner.invoke(app, ["task", "complete", task["task_id"]])
        assert close_out.exit_code != 0
        output = close_out.output or close_out.stderr or ""
        assert "cannot be completed while changes are still" in output
        assert "open:" in output
        assert change["change_id"] in output


def test_change_close_archives_remote_draft_and_unblocks_task_completion(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-change-close"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-change-close") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Close stale change", "--intent", "archive stale draft changes", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        _bind_task_worktree(task["task_id"], monkeypatch, name="remote-change-close")

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Stale draft", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        close_out = runner.invoke(app, ["change", "close", change["change_id"], "--json"])
        assert close_out.exit_code == 0, close_out.stdout
        closed = json.loads(close_out.stdout)
        assert closed["status"] == "archived"

        complete_out = runner.invoke(app, ["task", "complete", task["task_id"], "--json"])
        assert complete_out.exit_code == 0, complete_out.stdout
        assert json.loads(complete_out.stdout)["status"] == "completed"


def test_change_close_rejects_patchset_publish_for_archived_remote_change(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-change-close-publish"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-change-close-publish") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Archive before publish", "--intent", "prevent archived changes from publishing patchsets", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="archived-change-publish")

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Archived draft", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        assert runner.invoke(app, ["change", "close", change["change_id"]]).exit_code == 0

        assert runner.invoke(app, ["line", "create", "feature/archived-change"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/archived-change"]).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout

        publish_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "should fail"],
            catch_exceptions=False,
        )
        assert publish_out.exit_code != 0
        output = publish_out.output or publish_out.stderr or ""
        assert "archived" in output
        assert "publish patchsets" in output


def test_line_archive_archives_remote_line(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-line-archive"
    repo.mkdir()
    readme = repo / "README.md"
    readme.write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-line-archive") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        assert runner.invoke(app, ["line", "create", "feature/remote-archive"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/remote-archive"]).exit_code == 0
        readme.write_text("base\nremote\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "remote archive"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "feature/remote-archive"]).exit_code == 0

        archive_out = runner.invoke(
            app,
            ["line", "archive", "feature/remote-archive", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert archive_out.exit_code == 0, archive_out.stdout
        archived_line = json.loads(archive_out.stdout)
        assert archived_line["status"] == "archived"

        remote_line_json = urllib.request.urlopen(f"{base_url}/v1/native/repositories/housekeeper/lines/feature%2Fremote-archive").read().decode("utf-8")
        remote_line = json.loads(remote_line_json)
        assert remote_line["status"] == "archived"


def test_task_complete_marks_remote_task_completed_and_cancel_is_blocked_after_land(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-task-complete"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-task-complete") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        main_snapshot = json.loads(main_snap_out.stdout)
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Finish lifecycle", "--intent", "land one change and close the task", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="remote-task-complete")

        assert runner.invoke(app, ["line", "create", "feature/task-close"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/task-close"]).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
        feature_snapshot = json.loads(feature_snap_out.stdout)

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Land one change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "closeable task patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["base_snapshot_id"] == main_snapshot["snapshot_id"]
        assert patchset["revision_snapshot_id"] == feature_snapshot["snapshot_id"]

        attest_out = runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"])
        assert attest_out.exit_code == 0, attest_out.stdout

        review_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        )
        assert review_out.exit_code == 0, review_out.stdout
        _submit_passing_code_review_summary(change["change_id"], patchset["patchset_id"])

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout
        land = json.loads(land_out.stdout)
        assert land["local_sync"]["status"] == "synced"
        assert land["local_sync"]["line"] == "main"
        assert land["local_sync"]["head_snapshot_id"] == patchset["revision_snapshot_id"]
        assert land["local_sync"]["workspace_restore"]["status"] == "restored"

        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == patchset["revision_snapshot_id"]

        cancel_out = runner.invoke(app, ["task", "canceled", task["task_id"]])
        assert cancel_out.exit_code != 0

        complete_out = runner.invoke(app, ["task", "complete", task["task_id"], "--json"])
        assert complete_out.exit_code == 0, complete_out.stdout
        completed = json.loads(complete_out.stdout)
        assert completed["status"] == "completed"

        show_out = runner.invoke(app, ["task", "show", task["task_id"], "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        assert json.loads(show_out.stdout)["status"] == "completed"

        new_change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Should fail after complete", "--base-line", "main", "--risk", "medium"],
            catch_exceptions=False,
        )
        assert new_change_out.exit_code != 0
        output = new_change_out.output or new_change_out.stderr or ""
        assert "cannot accept new changes" in output


def test_task_audit_falls_back_to_local_records_when_remote_task_is_missing(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-audit-local-fallback"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-task-audit-local-fallback") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert main_snap_out.exit_code == 0, main_snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--local", "--title", "Audit missing remote task", "--intent", "infer closure from local line evidence", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        change_out = runner.invoke(
            app,
            ["change", "create", "--local", "--task", task["task_id"], "--title", "Missing remote change", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        ctx = RepoContext.discover(repo)
        mark_local_task_published(ctx, task["task_id"])
        mark_local_change_published(ctx, change["change_id"])

        line_name = f"feature/{task['task_id'].lower()}-local-fallback"
        assert runner.invoke(app, ["line", "create", line_name]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", line_name]).exit_code == 0
        (workspace / "app.py").write_text("print('merged locally')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
        feature_snapshot = json.loads(feature_snap_out.stdout)
        assert runner.invoke(app, ["push", "--line", line_name]).exit_code == 0
        update_remote_line(base_url, "housekeeper", "main", feature_snapshot["snapshot_id"])

        audit_out = runner.invoke(app, ["task", "audit", task["task_id"], "--json"])
        assert audit_out.exit_code == 0, audit_out.stdout
        audit = json.loads(audit_out.stdout)
        assert audit["audit_source"]["mode"] == "local_fallback"
        assert audit["audit_source"]["remote_task_missing"] is True
        assert audit["workflow"]["state"] == "workflow_missing_on_target"
        assert audit["summary"]["effectively_complete_on_target"] is True
        assert audit["summary"]["missing_remote_change_count"] == 1
        assert audit["summary"]["verdict"] == "workflow_missing_on_target"
        assert audit["recommended_action"]["code"] == "reconcile_workflow_records"
        assert audit["changes"][0]["target_state"] == "merged_on_target_missing_remote"
        bound_line_name = f"feature/{task['task_id'].lower()}"
        assert audit["changes"][0]["preferred_line"]["line_name"] in {bound_line_name, line_name}


def test_local_draft_publish_rejects_remote_reassigned_ids_and_keeps_local_ids(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-publish-mismatch"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://127.0.0.1:8088", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed main"]).exit_code == 0

    task_start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Mismatch guard",
            "--change-title",
            "Mismatch guard",
            "--intent",
            "reject remote reassigned ids",
            "--base-line",
            "main",
            "--risk",
            "medium",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_start_out.exit_code == 0, task_start_out.stdout
    started = json.loads(task_start_out.stdout)
    task = started
    change = started["change"]
    assert task["task_id"].startswith("LT-")
    assert change["change_id"].startswith("LC-")

    remote_calls: dict[str, dict[str, object]] = {}

    def fake_create_task(*args, **kwargs):
        remote_calls["task"] = {"args": args, "kwargs": kwargs}
        return {"task_id": "RT-0042", "title": task["title"]}

    def fake_create_change(*args, **kwargs):
        remote_calls["change"] = {"args": args, "kwargs": kwargs}
        return {"change_id": "RC-0042", "title": change["title"]}

    monkeypatch.setattr(
        cli_module,
        "remote_create_task",
        fake_create_task,
    )
    task_publish_out = runner.invoke(app, ["task", "publish", task["task_id"], "--json"])
    assert task_publish_out.exit_code == 0, task_publish_out.stdout
    published_task = json.loads(task_publish_out.stdout)
    assert published_task["publication_state"] == "published"
    assert published_task["published_task_id"] == "RT-0042"
    assert remote_calls["task"]["kwargs"]["task_id"] is None

    monkeypatch.setattr(
        cli_module,
        "remote_create_change",
        fake_create_change,
    )

    change_publish_out = runner.invoke(app, ["change", "publish", change["change_id"], "--json"])
    assert change_publish_out.exit_code == 0, change_publish_out.stdout
    published_change = json.loads(change_publish_out.stdout)
    assert published_change["publication_state"] == "published"
    assert published_change["published_change_id"] == "RC-0042"
    assert remote_calls["change"]["kwargs"]["change_id"] is None
    assert remote_calls["change"]["args"][2] == "RT-0042"


def test_local_draft_publish_rejects_unexpected_remote_id_prefix(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-publish-prefix-mismatch"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://127.0.0.1:8088", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed main"]).exit_code == 0

    task_start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Mismatch guard",
            "--change-title",
            "Mismatch guard",
            "--intent",
            "reject unexpected id prefix",
            "--base-line",
            "main",
            "--risk",
            "medium",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_start_out.exit_code == 0, task_start_out.stdout
    started = json.loads(task_start_out.stdout)
    task = started
    change = started["change"]

    monkeypatch.setattr(
        cli_module,
        "remote_list_tasks",
        lambda *args, **kwargs: [],
    )
    monkeypatch.setattr(
        cli_module,
        "remote_create_task",
        lambda *args, **kwargs: {"task_id": "ZZZT-9999", "title": "unexpected prefix"},
    )
    task_publish_out = runner.invoke(app, ["task", "publish", task["task_id"]])
    assert task_publish_out.exit_code != 0
    assert "ZZZT-9999" in task_publish_out.output
    assert "namespace prefix" in task_publish_out.output

    monkeypatch.setattr(
        cli_module,
        "remote_create_task",
        lambda *args, **kwargs: {"task_id": task["task_id"], "title": task["title"]},
    )
    task_publish_ok = runner.invoke(app, ["task", "publish", task["task_id"], "--json"])
    assert task_publish_ok.exit_code == 0, task_publish_ok.stdout

    monkeypatch.setattr(
        cli_module,
        "remote_list_changes",
        lambda *args, **kwargs: [],
    )
    monkeypatch.setattr(
        cli_module,
        "remote_create_change",
        lambda *args, **kwargs: {"change_id": "ZZZC-9999", "title": "unexpected prefix"},
    )
    change_publish_out = runner.invoke(app, ["change", "publish", change["change_id"]])
    assert change_publish_out.exit_code != 0
    assert "ZZZC-9999" in change_publish_out.output
    assert "namespace prefix" in change_publish_out.output


def test_push_syncs_team_policy_profile_to_remote_repo(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-team"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-policy") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper", "--policy-profile", "team"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push"]).exit_code == 0

        repo_show_out = runner.invoke(app, ["repo", "show", "--json"])
        assert repo_show_out.exit_code == 0, repo_show_out.stdout
        remote_repo = json.loads(repo_show_out.stdout)
        assert remote_repo["policy"]["policy_id"] == "team"
        assert remote_repo["policy"]["defaults"]["require_tests"] is True
        assert remote_repo["policy"]["defaults"]["require_lint"] is True
        assert remote_repo["policy"]["defaults"]["require_security_scan"] is False
        assert remote_repo["policy"]["defaults"]["require_ai_provenance"] is False
        assert remote_repo["policy"]["class_overrides"][0]["when"]["content_class"] == "docs_only"


def test_plan_cli_remote_sync_and_show_flow(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-first"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plans") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/durable_plan_storage.md",
            "# Durable Plan Storage\n\n## Bootstrap Durable Plan Storage [plan-ref: durable-plan-storage/bootstrap]\n\n- [ ] add durable plans [ref: durable-plan-storage/add-durable-plans]\n",
        )

        create_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert create_out.exit_code == 0, create_out.stdout
        create_payload = json.loads(create_out.stdout)
        assert create_payload["summary"]["created_count"] == 1
        assert create_payload["summary"]["published_count"] == 1
        plan_id = create_payload["results"][0]["plan_id"]
        seed_revision_id = create_payload["publish_results"][0]["published_head_revision_id"]

        list_out = runner.invoke(app, ["plan", "list", "--remote", "origin", "--json"])
        assert list_out.exit_code == 0, list_out.stdout
        rows = json.loads(list_out.stdout)
        assert rows[0]["plan_id"] == plan_id
        assert rows[0]["head_revision_number"] == 1

        _write_plan_artifact(
            repo,
            plan_file,
            "# Durable Plan Storage\n\n## Bootstrap Durable Plan Storage [plan-ref: durable-plan-storage/bootstrap]\n\n- [ ] add durable plans [ref: durable-plan-storage/add-durable-plans]\n- [ ] add revisions [ref: durable-plan-storage/add-revisions]\n",
        )
        revise_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert revise_out.exit_code == 0, revise_out.stdout
        revised = json.loads(revise_out.stdout)
        assert revised["summary"]["updated_count"] == 1
        assert revised["results"][0]["plan_id"] == plan_id
        revised_revision_id = revised["publish_results"][0]["published_head_revision_id"]

        revisions_out = runner.invoke(app, ["plan", "revisions", plan_id, "--remote", "origin", "--json"])
        assert revisions_out.exit_code == 0, revisions_out.stdout
        revisions = json.loads(revisions_out.stdout)
        assert [row["revision_number"] for row in revisions] == [2, 1]

        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        shown = json.loads(show_out.stdout)
        assert shown["head_revision"]["revision_number"] == 2
        assert shown["head_revision"]["artifact_heading"] == "Bootstrap Durable Plan Storage"
        assert [item["plan_item_ref"] for item in shown["head_revision"]["items"]] == [
            "durable-plan-storage/add-durable-plans",
            "durable-plan-storage/add-revisions",
        ]

        revision_show_out = runner.invoke(
            app,
            ["plan", "show", plan_id, "--remote", "origin", "--revision", seed_revision_id, "--json"],
            catch_exceptions=False,
        )
        assert revision_show_out.exit_code == 0, revision_show_out.stdout
        revision_payload = json.loads(revision_show_out.stdout)
        assert revision_payload["revision"]["revision_number"] == 1
        assert revision_payload["revision"]["artifact_heading"] == "Bootstrap Durable Plan Storage"

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--remote",
                "origin",
                "--title",
                "Plan-linked durable work",
                "--intent",
                "Promote the plan into a tracked execution task",
                "--risk",
                "medium",
                "--plan",
                plan_id,
                "--plan-item-ref",
                "durable-plan-storage/add-durable-plans",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        linked_task = json.loads(task_out.stdout)
        assert linked_task["task_id"].startswith("RT-")
        assert linked_task["plan_id"] == plan_id
        assert linked_task["origin_plan_revision_id"] == revised_revision_id
        assert linked_task["plan_item_ref"] == "durable-plan-storage/add-durable-plans"


def test_plan_sync_can_infer_single_plan_ref_and_title_from_file(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-create-infer"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-create-infer") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/inferred_plan.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n",
        )

        create_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert create_out.exit_code == 0, create_out.stdout
        plan_id = json.loads(create_out.stdout)["results"][0]["plan_id"]
        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        plan = json.loads(show_out.stdout)
        assert plan["title"] == "Stabilize Runtime Execution Tasks"
        assert plan["head_revision"]["artifact_selector"] == "runtime-stability/tasks"
        assert plan["head_revision"]["artifact_heading"] == "Stabilize Runtime Execution Tasks"


def test_remote_task_start_rejects_unpublished_local_plan_lineage(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-task-start-unpublished-local-plan"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-task-start-unpublished-local-plan") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/remote_task_start_local_revision.md",
            "# Remote Task Start Local Revision\n\n"
            "## Remote revision mapping [plan-ref: remote-task-start-local-revision/root]\n\n"
            "- [ ] map published local revisions [ref: remote-task-start-local-revision/task]\n",
        )
        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        plan_id = sync_payload["results"][0]["plan_id"]
        local_revision_id = sync_payload["results"][0]["plan_revision_id"]

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--remote",
                "origin",
                "--title",
                "Remote revision mapping task",
                "--intent",
                "exercise published local plan revision mapping during remote task start",
                "--risk",
                "medium",
                "--plan",
                plan_id,
                "--revision",
                local_revision_id,
                "--plan-item-ref",
                "remote-task-start-local-revision/task",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code != 0
        output = task_out.output or task_out.stdout or task_out.stderr
        assert f"Unknown plan: {plan_id}" in output


def test_remote_task_start_rejects_published_local_plan_revision_id(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-task-start-local-plan-revision"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-task-start-local-plan-revision") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/remote_task_start_local_revision.md",
            "# Remote Task Start Local Revision\n\n"
            "## Remote revision mapping [plan-ref: remote-task-start-local-revision/root]\n\n"
            "- [ ] map published local revisions [ref: remote-task-start-local-revision/task]\n",
        )
        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        plan_id = sync_payload["results"][0]["plan_id"]
        local_revision_id = sync_payload["results"][0]["plan_revision_id"]
        published_revision_id = sync_payload["publish_results"][0]["published_head_revision_id"]
        assert local_revision_id != published_revision_id

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--remote",
                "origin",
                "--title",
                "Remote revision mapping task",
                "--intent",
                "reject published local plan revision ids during remote task start",
                "--risk",
                "medium",
                "--plan",
                plan_id,
                "--revision",
                local_revision_id,
                "--plan-item-ref",
                "remote-task-start-local-revision/task",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code != 0
        output = task_out.output or task_out.stdout or task_out.stderr
        assert f"Unknown plan revision: {local_revision_id}" in output


def test_task_create_rejects_reusing_linked_plan_item_ref_after_completion(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-linked-duplicate-ref"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-linked-duplicate-ref") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/runtime_duplicate_ref.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "runtime-stability/tasks",
            artifact_path=plan_file,
        )

        first_task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--remote",
                "origin",
                "--title",
                "Move init to startup only",
                "--intent",
                "Create the first runtime task from the execution plan",
                "--risk",
                "medium",
                "--plan",
                plan["plan_id"],
                "--plan-item-ref",
                "runtime/startup-only-init",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert first_task_out.exit_code == 0, first_task_out.stdout
        first_task = json.loads(first_task_out.stdout)

        ctx = ServerContext.from_env()
        closed = server_store_module.close_task(ctx, first_task["task_id"], "completed")
        assert closed["status"] == "completed"

        second_task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--remote",
                "origin",
                "--title",
                "Reopen runtime work incorrectly",
                "--intent",
                "This should fail because the plan item ref already has task lineage",
                "--risk",
                "medium",
                "--plan",
                plan["plan_id"],
                "--plan-item-ref",
                "runtime/startup-only-init",
            ],
            catch_exceptions=False,
        )
        assert second_task_out.exit_code == 2
        output = second_task_out.output or second_task_out.stdout
        assert "already linked to task" in output
        assert first_task["task_id"] in output
        assert "older" in output
        assert "dispatched ref" in output


def test_plan_sync_remote_creates_updates_and_noops_by_artifact_path(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/runtime_sync.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n",
        )

        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["mode"] == "local_publish"
        assert sync_payload["summary"]["created_count"] == 1
        assert sync_payload["summary"]["updated_count"] == 0
        assert sync_payload["summary"]["unchanged_count"] == 0
        assert sync_payload["summary"]["adopted_count"] == 0
        assert sync_payload["summary"]["published_count"] == 1
        assert sync_payload["results"][0]["action"] == "created"
        plan_id = sync_payload["results"][0]["plan_id"]

        local_show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
        assert local_show_out.exit_code == 0, local_show_out.stdout
        local_shown = json.loads(local_show_out.stdout)
        assert local_shown["publication_state"] == "published"
        assert local_shown["head_revision"]["publication_state"] == "published"
        assert local_shown["published_head_revision_id"] == sync_payload["publish_results"][0]["published_head_revision_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Runtime Stability\n\n## Stabilize Runtime Execution Work [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n- [x] Add busy timeout defaults [ref: runtime/busy-timeout]\n",
        )
        sync_update_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_update_out.exit_code == 0, sync_update_out.stdout
        update_payload = json.loads(sync_update_out.stdout)
        assert update_payload["summary"]["created_count"] == 0
        assert update_payload["summary"]["updated_count"] == 1
        assert update_payload["summary"]["published_count"] == 1
        assert update_payload["results"][0]["action"] == "updated"
        assert update_payload["results"][0]["plan_id"] == plan_id

        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        shown = json.loads(show_out.stdout)
        assert shown["title"] == "Stabilize Runtime Execution Work"
        assert shown["head_revision"]["revision_number"] == 2
        assert shown["head_revision"]["artifact_selector"] == "runtime-stability/tasks"
        assert [item["plan_item_ref"] for item in shown["head_revision"]["items"]] == [
            "runtime/startup-only-init",
            "runtime/busy-timeout",
        ]

        sync_noop_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_noop_out.exit_code == 0, sync_noop_out.stdout
        noop_payload = json.loads(sync_noop_out.stdout)
        assert noop_payload["summary"]["updated_count"] == 0


def test_plan_sync_remote_records_boundary_events_on_the_bound_task_session(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-session-boundary"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-session-boundary") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Track plan sync boundary", "--intent", "reuse the server-guaranteed task session for remote plan sync provenance", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        tracking_session_id = task["tracking"]["session_id"]
        assert "worktree" in task

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/session_boundary.md",
            "# Session Boundary\n\n## Sync Through Task Session [plan-ref: session-boundary/root]\n\n- [ ] persist remote sync boundaries [ref: session-boundary/persist]\n",
        )

        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        plan_id = sync_payload["results"][0]["plan_id"]

        remote_plan = remote_client_module.get_plan(base_url, plan_id)
        assert remote_plan["head_revision"]["source_session_id"] == tracking_session_id

        events = remote_client_module.list_session_events(base_url, tracking_session_id, repo_name="housekeeper")
        boundary_events = [row for row in events if row["event_type"] == "workflow.boundary"]
        assert len(boundary_events) == 1
        assert boundary_events[0]["payload"]["boundary_kind"] == "plan_sync"
        assert "ait plan sync" in boundary_events[0]["payload"]["text"]
        assert boundary_events[0]["payload"]["workflow_context"]["signals"][0]["kind"] == "plan_sync"
        assert boundary_events[0]["payload"]["workflow_context"]["attachment_hints"]["plan_id"] == plan_id

        noop_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert noop_out.exit_code == 0, noop_out.stdout
        noop_payload = json.loads(noop_out.stdout)

        events_after_noop = remote_client_module.list_session_events(base_url, tracking_session_id, repo_name="housekeeper")
        noop_boundary_events = [row for row in events_after_noop if row["event_type"] == "workflow.boundary"]
        assert len(noop_boundary_events) == 2
        assert noop_boundary_events[-1]["payload"]["boundary_kind"] == "plan_sync"
        assert noop_payload["summary"]["unchanged_count"] == 1
        assert noop_payload["summary"]["published_count"] == 0
        assert noop_payload["results"][0]["action"] == "unchanged"

        show_again_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_again_out.exit_code == 0, show_again_out.stdout
        assert json.loads(show_again_out.stdout)["head_revision"]["revision_number"] == 1


def test_plan_sync_remote_noop_single_file_skips_unrelated_remote_plan_fetches(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-noop-fast-path"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-noop-fast-path") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/noop_fast_path.md",
            "# No-op Fast Path\n\n## Keep no-op sync targeted [plan-ref: noop-fast-path/root]\n\n- [ ] avoid unrelated fetches [ref: noop-fast-path/fetch]\n",
        )
        initial_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert initial_sync_out.exit_code == 0, initial_sync_out.stdout

        for index in range(40):
            ref = f"unrelated-{index}/root"
            remote_client_module.create_plan(
                base_url,
                "housekeeper",
                f"Unrelated {index}",
                f"docs/sprints/unrelated_{index}.md",
                ref,
                f"Unrelated {index}",
                [{"plan_item_ref": f"unrelated-{index}/item", "text": f"Unrelated item {index}", "checkbox_state": "open"}],
                artifact_body=(
                    f"# Unrelated {index}\n\n"
                    f"## Unrelated {index} [plan-ref: {ref}]\n\n"
                    f"- [ ] Unrelated item {index} [ref: unrelated-{index}/item]\n"
                ),
            )

        counts = {"list_plans": 0, "get_plan": 0, "list_plan_revisions": 0}
        observed = {"artifact_path": None, "row_count": None}
        original_list_plans = cli_module.remote_list_plans
        original_get_plan = cli_module.remote_get_plan
        original_list_plan_revisions = cli_module.remote_list_plan_revisions

        def counting_list_plans(*args, **kwargs):
            counts["list_plans"] += 1
            observed["artifact_path"] = kwargs.get("artifact_path")
            rows = original_list_plans(*args, **kwargs)
            observed["row_count"] = len(rows)
            return rows

        def counting_get_plan(*args, **kwargs):
            counts["get_plan"] += 1
            return original_get_plan(*args, **kwargs)

        def counting_list_plan_revisions(*args, **kwargs):
            counts["list_plan_revisions"] += 1
            return original_list_plan_revisions(*args, **kwargs)

        monkeypatch.setattr(cli_module, "remote_list_plans", counting_list_plans)
        monkeypatch.setattr(cli_module, "remote_get_plan", counting_get_plan)
        monkeypatch.setattr(cli_module, "remote_list_plan_revisions", counting_list_plan_revisions)

        noop_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert noop_out.exit_code == 0, noop_out.stdout
        noop_payload = json.loads(noop_out.stdout)
        assert noop_payload["summary"]["unchanged_count"] == 1
        assert noop_payload["summary"]["published_count"] == 0
        assert noop_payload["results"][0]["action"] == "unchanged"
        assert counts["list_plans"] == 1
        assert observed["artifact_path"] == plan_file
        assert observed["row_count"] == 1
        assert counts["get_plan"] == 0
        assert counts["list_plan_revisions"] == 0


def test_plan_sync_remote_falls_back_when_bound_task_session_is_completed(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-completed-session"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-completed-session") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-auto-worktree", "on"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Completed session fallback", "--intent", "remote plan sync should not append to a completed task_run session", "--risk", "medium", "--json"])
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        tracking_session_id = task["tracking"]["session_id"]
        assert "worktree" in task

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/completed_session.md",
            "# Completed Session\n\n## Avoid completed task session reuse [plan-ref: completed-session/root]\n\n- [ ] keep plan sync working [ref: completed-session/keep-working]\n",
        )

        first_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert first_sync_out.exit_code == 0, first_sync_out.stdout

        complete_out = runner.invoke(app, ["task", "complete", task["task_id"], "--json"])
        assert complete_out.exit_code == 0, complete_out.stdout
        closed_session = remote_client_module.get_session(base_url, tracking_session_id, repo_name="housekeeper")
        assert closed_session["status"] == "completed"

        noop_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert noop_out.exit_code == 0, noop_out.stdout
        noop_payload = json.loads(noop_out.stdout)
        assert noop_payload["summary"]["unchanged_count"] == 1
        assert noop_payload["summary"]["published_count"] == 0

        original_events = remote_client_module.list_session_events(base_url, tracking_session_id, repo_name="housekeeper")
        original_boundary_events = [row for row in original_events if row["event_type"] == "workflow.boundary"]
        assert len(original_boundary_events) == 1

        sessions = remote_client_module.list_sessions(base_url, "housekeeper")
        boundary_session_ids = []
        for session in sessions:
            if session["session_id"] == tracking_session_id:
                continue
            events = remote_client_module.list_session_events(base_url, session["session_id"], repo_name="housekeeper")
            if any(
                row["event_type"] == "workflow.boundary" and row["payload"].get("boundary_kind") == "plan_sync"
                for row in events
            ):
                boundary_session_ids.append(session["session_id"])
        assert len(boundary_session_ids) == 1


def test_plan_sync_remote_adopts_existing_remote_plan_before_publishing(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-adopt-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-adopt-remote") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/remote_adoption.md",
            "# Remote Adoption\n\n## Adopt Existing Remote Plan [plan-ref: remote-adoption/root]\n\n- [ ] Adopt before publishing [ref: remote-adoption/adopt]\n",
        )
        remote_plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "remote-adoption/root",
            artifact_path=plan_file,
            title="Adopt Existing Remote Plan",
        )
        plan_id = remote_plan["plan_id"]

        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["mode"] == "local_publish"
        assert sync_payload["summary"]["adopted_count"] == 1
        assert sync_payload["summary"]["unchanged_count"] == 1
        assert sync_payload["summary"]["published_count"] == 0
        assert sync_payload["adoptions"][0]["plan_id"] == plan_id
        assert sync_payload["results"][0]["plan_id"] == plan_id

        local_show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
        assert local_show_out.exit_code == 0, local_show_out.stdout
        local_plan = json.loads(local_show_out.stdout)
        assert local_plan["publication_state"] == "published"
        assert local_plan["head_revision"]["publication_state"] == "published"
        assert local_plan["published_head_revision_id"] == remote_plan["head_revision"]["plan_revision_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Remote Adoption\n\n## Adopt Existing Remote Plan [plan-ref: remote-adoption/root]\n\n- [ ] Adopt before publishing [ref: remote-adoption/adopt]\n- [ ] Publish after adoption [ref: remote-adoption/publish]\n",
        )
        update_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert update_out.exit_code == 0, update_out.stdout
        update_payload = json.loads(update_out.stdout)
        assert update_payload["summary"]["adopted_count"] == 0
        assert update_payload["summary"]["updated_count"] == 1
        assert update_payload["summary"]["published_count"] == 1

        remote_show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert remote_show_out.exit_code == 0, remote_show_out.stdout
        remote_shown = json.loads(remote_show_out.stdout)
        assert remote_shown["head_revision"]["revision_number"] == 2
        assert [item["plan_item_ref"] for item in remote_shown["head_revision"]["items"]] == [
            "remote-adoption/adopt",
            "remote-adoption/publish",
        ]


def test_plan_sync_remote_adoption_materializes_markdown_history_for_blame_and_repairs_missing_local_blob(
    tmp_path: Path,
    monkeypatch,
):
    repo = tmp_path / "housekeeper-plan-sync-adopt-remote-blame"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-adopt-remote-blame") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/remote_adoption_blame.md",
            "# Remote Adoption Blame\n\n## Adopt Existing Remote Plan [plan-ref: remote-adoption-blame/root]\n\n- [ ] Adopt before publishing [ref: remote-adoption-blame/adopt]\n",
        )
        remote_plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "remote-adoption-blame/root",
            artifact_path=plan_file,
            title="Adopt Existing Remote Plan For Markdown Blame",
        )
        plan_id = remote_plan["plan_id"]
        first_remote_revision_id = remote_plan["head_revision"]["plan_revision_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Remote Adoption Blame\n\n## Adopt Existing Remote Plan [plan-ref: remote-adoption-blame/root]\n\n- [x] Adopt before publishing [ref: remote-adoption-blame/adopt]\n- [ ] Publish after adoption [ref: remote-adoption-blame/publish]\n",
        )
        second_remote_revision = _revise_remote_plan_from_artifact(
            base_url,
            plan_id,
            (repo / plan_file).read_text(encoding="utf-8"),
            "remote-adoption-blame/root",
            artifact_path=plan_file,
            expected_head_revision_id=first_remote_revision_id,
        )
        second_remote_revision_id = second_remote_revision["head_revision"]["plan_revision_id"]

        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["summary"]["adopted_count"] == 1
        assert sync_payload["results"][0]["plan_id"] == plan_id

        local_show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
        assert local_show_out.exit_code == 0, local_show_out.stdout
        local_plan = json.loads(local_show_out.stdout)
        ctx = RepoContext.discover(repo)
        local_revisions = store_module.list_local_plan_revisions(ctx, plan_id)
        second_local_revision = next(
            row for row in local_revisions if row["published_plan_revision_id"] == second_remote_revision_id
        )

        initial_blame_out = runner.invoke(app, ["blame", plan_file, "--line", "6", "--json"], catch_exceptions=False)
        assert initial_blame_out.exit_code == 0, initial_blame_out.stdout
        initial_blame = json.loads(initial_blame_out.stdout)
        assert initial_blame["target"]["kind"] == "markdown_plan"
        assert initial_blame["target"]["plan_id"] == plan_id
        assert initial_blame["target"]["resolved_plan_revision_id"] == local_plan["head_revision"]["plan_revision_id"]
        assert initial_blame["lines"][0]["plan_id"] == plan_id
        assert initial_blame["lines"][0]["plan_revision_id"] == second_local_revision["plan_revision_id"]
        assert initial_blame["lines"][0]["start_line"] == 6

        first_local_revision = next(
            row for row in local_revisions if row["published_plan_revision_id"] == first_remote_revision_id
        )
        first_local_blob_id = str(first_local_revision["artifact_blob_id"])

        with connect_sqlite(ctx.content_db_path) as conn:
            conn.execute("delete from blobs where blob_id = ?", (first_local_blob_id,))
            conn.commit()

        with pytest.raises(KeyError, match="Unknown blob"):
            local_content_module._read_blob_bytes(ctx, first_local_blob_id)
        pre_repair_blame_out = runner.invoke(app, ["blame", plan_file, "--line", "6", "--json"], catch_exceptions=False)
        assert pre_repair_blame_out.exit_code == 0, pre_repair_blame_out.stdout
        pre_repair_blame = json.loads(pre_repair_blame_out.stdout)
        assert pre_repair_blame["target"]["kind"] == "markdown_plan"
        assert pre_repair_blame["target"]["plan_id"] == plan_id
        assert pre_repair_blame["lines"][0]["plan_revision_id"] == second_local_revision["plan_revision_id"]

        repair_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert repair_sync_out.exit_code == 0, repair_sync_out.stdout
        repair_payload = json.loads(repair_sync_out.stdout)
        assert repair_payload["summary"]["adopted_count"] == 0
        assert repair_payload["summary"]["unchanged_count"] == 1
        assert repair_payload["summary"]["published_count"] == 0

        assert local_content_module._read_blob_bytes(ctx, first_local_blob_id).decode("utf-8").startswith(
            "# Remote Adoption Blame"
        )
        repaired_blame_out = runner.invoke(app, ["blame", plan_file, "--line", "6", "--json"], catch_exceptions=False)
        assert repaired_blame_out.exit_code == 0, repaired_blame_out.stdout


def test_plan_sync_remote_replaces_equivalent_unpublished_local_duplicate(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-replace-local-duplicate"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-replace-local-duplicate") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/duplicate_adoption.md",
            "# Duplicate Adoption\n\n## Adopt Remote Duplicate [plan-ref: duplicate-adoption/root]\n\n- [ ] keep one lineage [ref: duplicate-adoption/one-lineage]\n",
        )
        local_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert local_sync_out.exit_code == 0, local_sync_out.stdout
        local_plan_id = json.loads(local_sync_out.stdout)["results"][0]["plan_id"]

        remote_plan_id = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "duplicate-adoption/root",
            artifact_path=plan_file,
            title="Adopt Remote Duplicate",
        )["plan_id"]
        assert remote_plan_id != local_plan_id

        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["summary"]["adopted_count"] == 1
        assert sync_payload["summary"]["unchanged_count"] == 1
        assert sync_payload["summary"]["published_count"] == 0
        assert sync_payload["adoptions"][0]["plan_id"] == remote_plan_id
        assert sync_payload["adoptions"][0]["replaced_local_plan_id"] == local_plan_id

        archived_local_out = runner.invoke(app, ["plan", "show", local_plan_id, "--json"])
        assert archived_local_out.exit_code == 0, archived_local_out.stdout
        assert json.loads(archived_local_out.stdout)["status"] == "archived"

        adopted_local_out = runner.invoke(app, ["plan", "show", remote_plan_id, "--json"])
        assert adopted_local_out.exit_code == 0, adopted_local_out.stdout
        adopted_local = json.loads(adopted_local_out.stdout)
        assert adopted_local["publication_state"] == "published"
        assert adopted_local["head_revision"]["publication_state"] == "published"


def test_plan_sync_remote_fast_forwards_equivalent_local_unpublished_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-fast-forward-local-head"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-fast-forward-local-head") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/fast_forward.md",
            "# Fast Forward\n\n## Fast Forward Local Head [plan-ref: fast-forward/root]\n\n- [ ] publish v1 [ref: fast-forward/v1]\n",
        )
        initial_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert initial_sync_out.exit_code == 0, initial_sync_out.stdout
        plan_id = json.loads(initial_sync_out.stdout)["results"][0]["plan_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Fast Forward\n\n## Fast Forward Local Head [plan-ref: fast-forward/root]\n\n- [x] publish v1 [ref: fast-forward/v1]\n- [ ] map remote v2 [ref: fast-forward/v2]\n",
        )
        local_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert local_sync_out.exit_code == 0, local_sync_out.stdout
        assert json.loads(local_sync_out.stdout)["results"][0]["action"] == "updated"

        remote_head_id = _revise_remote_plan_from_artifact(
            base_url,
            plan_id,
            (repo / plan_file).read_text(encoding="utf-8"),
            "fast-forward/root",
            artifact_path=plan_file,
            expected_head_revision_id=json.loads(initial_sync_out.stdout)["publish_results"][0]["published_head_revision_id"],
        )["head_revision"]["plan_revision_id"]

        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["summary"]["unchanged_count"] == 1
        assert sync_payload["summary"]["published_count"] == 1

        local_show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
        assert local_show_out.exit_code == 0, local_show_out.stdout
        local_plan = json.loads(local_show_out.stdout)
        assert local_plan["published_head_revision_id"] == remote_head_id
        assert local_plan["head_revision"]["publication_state"] == "published"
        assert local_plan["head_revision"]["published_plan_revision_id"] == remote_head_id


def test_plan_sync_remote_adopts_remote_head_that_matches_later_local_draft(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-mid-chain-remote-head"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-mid-chain-remote-head") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/mid_chain.md",
            "# Mid Chain\n\n## Mid Chain Publish [plan-ref: mid-chain/root]\n\n- [ ] publish seed [ref: mid-chain/seed]\n",
        )
        initial_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert initial_sync_out.exit_code == 0, initial_sync_out.stdout
        plan_id = json.loads(initial_sync_out.stdout)["results"][0]["plan_id"]

        skipped_markdown = (
            "# Mid Chain\n\n## Mid Chain Publish [plan-ref: mid-chain/root]\n\n"
            "- [ ] publish seed [ref: mid-chain/seed]\n"
            "- [ ] skipped local draft [ref: mid-chain/skipped]\n"
        )
        _write_plan_artifact(repo, plan_file, skipped_markdown)
        skipped_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert skipped_sync_out.exit_code == 0, skipped_sync_out.stdout
        assert json.loads(skipped_sync_out.stdout)["results"][0]["action"] == "updated"

        remote_matching_markdown = (
            "# Mid Chain\n\n## Mid Chain Publish [plan-ref: mid-chain/root]\n\n"
            "- [x] publish seed [ref: mid-chain/seed]\n"
            "- [ ] remote matching draft [ref: mid-chain/remote]\n"
        )
        _write_plan_artifact(repo, plan_file, remote_matching_markdown)
        matching_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert matching_sync_out.exit_code == 0, matching_sync_out.stdout
        assert json.loads(matching_sync_out.stdout)["results"][0]["action"] == "updated"

        remote_matching_revision_id = _revise_remote_plan_from_artifact(
            base_url,
            plan_id,
            (repo / plan_file).read_text(encoding="utf-8"),
            "mid-chain/root",
            artifact_path=plan_file,
            expected_head_revision_id=json.loads(initial_sync_out.stdout)["publish_results"][0]["published_head_revision_id"],
        )["head_revision"]["plan_revision_id"]

        adopt_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert adopt_out.exit_code == 0, adopt_out.stdout
        adopt_payload = json.loads(adopt_out.stdout)
        assert adopt_payload["summary"]["unchanged_count"] == 1
        assert adopt_payload["summary"]["published_count"] == 1

        local_show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
        assert local_show_out.exit_code == 0, local_show_out.stdout
        local_plan = json.loads(local_show_out.stdout)
        assert local_plan["published_head_revision_id"] == remote_matching_revision_id
        assert local_plan["head_revision"]["published_plan_revision_id"] == remote_matching_revision_id

        next_markdown = (
            "# Mid Chain\n\n## Mid Chain Publish [plan-ref: mid-chain/root]\n\n"
            "- [x] publish seed [ref: mid-chain/seed]\n"
            "- [x] remote matching draft [ref: mid-chain/remote]\n"
            "- [ ] publish after adoption [ref: mid-chain/after]\n"
        )
        _write_plan_artifact(repo, plan_file, next_markdown)
        next_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert next_sync_out.exit_code == 0, next_sync_out.stdout

        remote_revisions_out = runner.invoke(app, ["plan", "revisions", plan_id, "--remote", "origin", "--json"])
        assert remote_revisions_out.exit_code == 0, remote_revisions_out.stdout
        remote_revisions = json.loads(remote_revisions_out.stdout)
        assert len(remote_revisions) == 3
        assert cli_module._artifact_blob_id(skipped_markdown) not in {
            revision.get("artifact_blob_id") for revision in remote_revisions
        }


def test_plan_sync_remote_rejects_head_that_advances_after_listing(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-head-conflict"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-head-conflict") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/runtime_sync.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n",
        )
        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        plan_id = json.loads(sync_out.stdout)["results"][0]["plan_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Runtime Stability\n\n## Stabilize Runtime Execution Work [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n- [ ] Add conflict guard [ref: runtime/conflict-guard]\n",
        )

        original_remote_revise_plan = cli_module.remote_revise_plan
        injected = {"done": False}

        def racing_revise(base_url_arg, plan_id_arg, artifact_path, artifact_selector, artifact_heading, items, **kwargs):
            if not injected["done"]:
                injected["done"] = True
                original_remote_revise_plan(
                    base_url_arg,
                    plan_id_arg,
                    artifact_path,
                    artifact_selector,
                    artifact_heading,
                    items,
                    title="Concurrent Runtime Update",
                    artifact_body="# Concurrent Runtime Update\n",
                )
            return original_remote_revise_plan(
                base_url_arg,
                plan_id_arg,
                artifact_path,
                artifact_selector,
                artifact_heading,
                items,
                **kwargs,
            )

        monkeypatch.setattr(cli_module, "remote_revise_plan", racing_revise)
        conflict_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin"])
        assert conflict_out.exit_code != 0
        assert "head advanced" in conflict_out.output

        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        assert json.loads(show_out.stdout)["head_revision"]["revision_number"] == 2


def test_plan_sync_remote_tracks_multi_root_markdown_by_path_and_selector(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-multi-root"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-multi-root") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/runtime_multi_root_sync.md",
            "# Runtime Stability\n\n## Stabilize Runtime Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n\n## Harden Review Flow [plan-ref: review-flow/root]\n\n- [ ] Add inbox grouping [ref: review-flow/inbox-grouping]\n",
        )

        runtime_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, "--plan-ref", "runtime-stability/tasks", "--remote", "origin", "--json"])
        assert runtime_sync_out.exit_code == 0, runtime_sync_out.stdout
        runtime_payload = json.loads(runtime_sync_out.stdout)
        assert runtime_payload["summary"]["created_count"] == 1
        assert runtime_payload["summary"]["published_count"] == 1
        runtime_plan_id = runtime_payload["results"][0]["plan_id"]
        assert runtime_payload["results"][0]["artifact_selector"] == "runtime-stability/tasks"

        review_sync_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, "--plan-ref", "review-flow/root", "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert review_sync_out.exit_code == 0, review_sync_out.stdout
        review_payload = json.loads(review_sync_out.stdout)
        assert review_payload["summary"]["created_count"] == 1
        assert review_payload["summary"]["published_count"] == 1
        review_plan_id = review_payload["results"][0]["plan_id"]
        assert review_payload["results"][0]["artifact_selector"] == "review-flow/root"
        assert review_plan_id != runtime_plan_id

        runtime_show_out = runner.invoke(app, ["plan", "show", runtime_plan_id, "--remote", "origin", "--json"])
        assert runtime_show_out.exit_code == 0, runtime_show_out.stdout
        assert json.loads(runtime_show_out.stdout)["head_revision"]["artifact_selector"] == "runtime-stability/tasks"

        review_show_out = runner.invoke(app, ["plan", "show", review_plan_id, "--remote", "origin", "--json"])
        assert review_show_out.exit_code == 0, review_show_out.stdout
        assert json.loads(review_show_out.stdout)["head_revision"]["artifact_selector"] == "review-flow/root"


def test_plan_sync_remote_missing_multi_root_file_adopts_and_archives_all_remote_plan_refs(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-multi-root-prune"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-multi-root-prune") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/runtime_multi_root_prune.md",
            "# Runtime Stability\n\n## Stabilize Runtime Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n\n## Harden Review Flow [plan-ref: review-flow/root]\n\n- [ ] Add inbox grouping [ref: review-flow/inbox-grouping]\n",
        )

        runtime_plan_id = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "runtime-stability/tasks",
            artifact_path=plan_file,
        )["plan_id"]

        review_plan_id = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "review-flow/root",
            artifact_path=plan_file,
        )["plan_id"]

        (repo / plan_file).unlink()

        prune_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert prune_out.exit_code == 0, prune_out.stdout
        prune_payload = json.loads(prune_out.stdout)
        assert prune_payload["summary"]["adopted_count"] == 2
        assert prune_payload["summary"]["pruned_count"] == 2
        assert prune_payload["summary"]["published_count"] == 2
        _assert_plan_sync_lineage_only(prune_payload)
        pruned_selectors = {row["artifact_selector"] for row in prune_payload["results"] if row["action"] == "pruned"}
        assert pruned_selectors == {"runtime-stability/tasks", "review-flow/root"}

        runtime_show_out = runner.invoke(app, ["plan", "show", runtime_plan_id, "--remote", "origin", "--json"])
        assert runtime_show_out.exit_code == 0, runtime_show_out.stdout
        assert json.loads(runtime_show_out.stdout)["status"] == "archived"

        review_show_out = runner.invoke(app, ["plan", "show", review_plan_id, "--remote", "origin", "--json"])
        assert review_show_out.exit_code == 0, review_show_out.stdout
        assert json.loads(review_show_out.stdout)["status"] == "archived"


def test_plan_sync_remote_does_not_push_remote_main(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-partial-success"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-partial-success") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/partial_success.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n",
        )
        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        plan_id = json.loads(sync_out.stdout)["results"][0]["plan_id"]
        seed_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Runtime Stability\n\n## Stabilize Runtime Execution Work [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n- [ ] Add conflict guard [ref: runtime/conflict-guard]\n",
        )

        original_update_remote_line = cli_module.update_remote_line
        injected = {"done": False}

        def failing_update_line(base_url_arg, repo_name_arg, line_name_arg, head_snapshot_id, **kwargs):
            if line_name_arg == "main" and not injected["done"]:
                injected["done"] = True
                raise remote_client_module.RemoteError("simulated remote main push failure")
            return original_update_remote_line(base_url_arg, repo_name_arg, line_name_arg, head_snapshot_id, **kwargs)

        monkeypatch.setattr(cli_module, "update_remote_line", failing_update_line)
        success_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert success_out.exit_code == 0, success_out.stdout
        success_payload = json.loads(success_out.stdout)
        assert success_payload["status"] == "ok"
        assert success_payload["summary"]["published_count"] == 1
        _assert_plan_sync_lineage_only(success_payload)
        assert injected["done"] is False

        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        assert json.loads(show_out.stdout)["head_revision"]["revision_number"] == 2
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_main_head
        assert remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"] == seed_main_head


def test_plan_sync_remote_does_not_use_remote_main_head_cas(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-main-cas"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-main-cas") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/main_cas.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n",
        )
        sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        seed_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]

        _write_plan_artifact(
            repo,
            plan_file,
            "# Runtime Stability\n\n## Stabilize Runtime Execution Work [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n- [ ] Add conflict guard [ref: runtime/conflict-guard]\n",
        )

        original_update_remote_line = cli_module.update_remote_line
        injected = {"done": False}

        def racing_update_line(base_url_arg, repo_name_arg, line_name_arg, head_snapshot_id, **kwargs):
            if line_name_arg == "main" and not injected["done"] and kwargs.get("expected_head_snapshot_id") is not None:
                injected["done"] = True
                original_update_remote_line(base_url_arg, repo_name_arg, line_name_arg, head_snapshot_id)
            return original_update_remote_line(base_url_arg, repo_name_arg, line_name_arg, head_snapshot_id, **kwargs)

        monkeypatch.setattr(cli_module, "update_remote_line", racing_update_line)
        success_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert success_out.exit_code == 0, success_out.stdout
        success_payload = json.loads(success_out.stdout)
        assert success_payload["status"] == "ok"
        assert success_payload["summary"]["published_count"] == 1
        _assert_plan_sync_lineage_only(success_payload)
        assert injected["done"] is False
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_main_head
        assert remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"] == seed_main_head


def test_plan_sync_remote_can_track_generic_markdown_artifacts_without_plan_ref(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-generic-markdown"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-generic-markdown") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        artifact_file = _write_plan_artifact(
            repo,
            "docs/sprints/workflow_bootstrap.md",
            "# Workflow Bootstrap\n\nThis coordination note does not expose plan refs yet.\n",
        )
        seed_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]

        sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["summary"]["created_count"] == 1
        assert sync_payload["summary"]["published_count"] == 1
        assert sync_payload["results"][0]["action"] == "created"
        _assert_plan_sync_lineage_only(sync_payload)
        plan_id = sync_payload["results"][0]["plan_id"]
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        local_main_head = json.loads(main_line_out.stdout)["head_snapshot_id"]
        remote_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]
        assert local_main_head == seed_main_head
        assert remote_main_head == seed_main_head
        status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["clean"] is True
        assert status["modified_paths"] == []
        assert status["untracked_paths"] == []

        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        shown = json.loads(show_out.stdout)
        assert shown["title"] == "Workflow Bootstrap"
        assert shown["head_revision"]["artifact_selector"] is None
        assert shown["head_revision"]["artifact_heading"] == "Workflow Bootstrap"
        assert shown["head_revision"]["items"] == []

        _write_plan_artifact(
            repo,
            artifact_file,
            "# Workflow Bootstrap Refresh\n\nThis coordination note now has updated artifact-only content.\n",
        )
        sync_update_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert sync_update_out.exit_code == 0, sync_update_out.stdout
        update_payload = json.loads(sync_update_out.stdout)
        assert update_payload["summary"]["updated_count"] == 1
        assert update_payload["summary"]["published_count"] == 1
        assert update_payload["results"][0]["action"] == "updated"
        assert update_payload["results"][0]["plan_id"] == plan_id
        _assert_plan_sync_lineage_only(update_payload)
        status_updated_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert status_updated_out.exit_code == 0, status_updated_out.stdout
        status_updated = json.loads(status_updated_out.stdout)
        assert status_updated["clean"] is True
        assert status_updated["modified_paths"] == []
        assert status_updated["untracked_paths"] == []

        show_updated_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_updated_out.exit_code == 0, show_updated_out.stdout
        shown_updated = json.loads(show_updated_out.stdout)
        assert shown_updated["title"] == "Workflow Bootstrap Refresh"
        assert shown_updated["head_revision"]["artifact_selector"] is None
        assert shown_updated["head_revision"]["artifact_heading"] == "Workflow Bootstrap Refresh"
        assert shown_updated["head_revision"]["items"] == []
        assert shown_updated["head_revision"]["revision_number"] == 2

        sync_noop_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert sync_noop_out.exit_code == 0, sync_noop_out.stdout
        noop_payload = json.loads(sync_noop_out.stdout)
        assert noop_payload["summary"]["unchanged_count"] == 1
        assert noop_payload["summary"]["published_count"] == 0
        assert noop_payload["results"][0]["action"] == "unchanged"
        _assert_plan_sync_lineage_only(noop_payload)

def test_plan_sync_remote_preserves_unrelated_dirty_paths_without_advancing_default_line(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-remote-unrelated-dirty"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-remote-unrelated-dirty") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        artifact_file = _write_plan_artifact(
            repo,
            "docs/sprints/workflow_bootstrap.md",
            "# Workflow Bootstrap\n\nThis coordination note does not expose plan refs yet.\n",
        )
        seed_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]
        (repo / "README.md").write_text("base\nunrelated dirty change\n", encoding="utf-8")

        sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["summary"]["created_count"] == 1
        assert sync_payload["summary"]["published_count"] == 1
        _assert_plan_sync_lineage_only(sync_payload)

        assert (repo / "README.md").read_text(encoding="utf-8") == "base\nunrelated dirty change\n"
        status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["clean"] is True
        assert status["changed_paths"] == []
        assert status["modified_paths"] == []
        assert status["untracked_paths"] == []

        remote_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_main_head
        assert remote_main_head == seed_main_head

        plan_id = sync_payload["results"][0]["plan_id"]
        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        assert json.loads(show_out.stdout)["title"] == "Workflow Bootstrap"


def test_plan_sync_remote_from_worktree_is_rejected_without_advancing_main(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-no-sync-line-restore"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    original_artifact = "# Workflow Bootstrap\n\nOriginal plan artifact content.\n"
    updated_artifact = "# Workflow Bootstrap Refresh\n\nUpdated plan artifact content already synced to remote.\n"

    with running_server(tmp_path / "server-data-plan-sync-no-sync-line-restore") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        artifact_file = _write_plan_artifact(repo, "docs/sprints/workflow_bootstrap.md", original_artifact)
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        seed_main_head = json.loads(snap_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        line_out = runner.invoke(app, ["line", "create", "feature/worktree-no-sync-line", "--json"])
        assert line_out.exit_code == 0, line_out.stdout
        worktree_out = _invoke_internal_worktree_add(["worktree", "add", "worktree-no-sync-line", "--line", "feature/worktree-no-sync-line", "--json"])
        assert worktree_out.exit_code == 0, worktree_out.stdout
        worktree_path = Path(json.loads(worktree_out.stdout)["path"])

        monkeypatch.chdir(worktree_path)
        _write_plan_artifact(worktree_path, artifact_file, updated_artifact)
        (worktree_path / "README.md").write_text("base\nunrelated dirty change\n", encoding="utf-8")

        blocked_out = runner.invoke(
            app,
            ["plan", "sync", artifact_file, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert blocked_out.exit_code == 2
        blocked_output = blocked_out.output or blocked_out.stdout
        assert "worktree" in blocked_output.lower()
        assert "repo root" in blocked_output.lower()
        assert "plan sync" in blocked_output.lower()
        assert (worktree_path / artifact_file).read_text(encoding="utf-8") == updated_artifact
        assert (repo / artifact_file).read_text(encoding="utf-8") == original_artifact

        worktree_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert worktree_status_out.exit_code == 0, worktree_status_out.stdout
        worktree_status = json.loads(worktree_status_out.stdout)
        assert worktree_status["clean"] is True
        assert worktree_status["changed_paths"] == []
        assert worktree_status["modified_paths"] == []
        assert worktree_status["untracked_paths"] == []

        remote_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]
        monkeypatch.chdir(repo)
        root_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert root_status_out.exit_code == 0, root_status_out.stdout
        assert json.loads(root_status_out.stdout)["clean"] is True
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_main_head
        assert remote_main_head == seed_main_head

        local_plan_list_out = runner.invoke(app, ["plan", "list", "--json"], catch_exceptions=False)
        assert local_plan_list_out.exit_code == 0, local_plan_list_out.stdout
        assert json.loads(local_plan_list_out.stdout) == []
        remote_plan_list_out = runner.invoke(app, ["plan", "list", "--remote", "origin", "--json"], catch_exceptions=False)
        assert remote_plan_list_out.exit_code == 0, remote_plan_list_out.stdout
        assert json.loads(remote_plan_list_out.stdout) == []


def test_plan_sync_remote_from_worktree_is_rejected_and_preserves_repo_root_untracked_paths(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-root-main-untracked"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    original_artifact = "# Workflow Bootstrap\n\nOriginal plan artifact content.\n"
    updated_artifact = "# Workflow Bootstrap Refresh\n\nUpdated plan artifact content already synced to remote.\n"

    with running_server(tmp_path / "server-data-plan-sync-root-main-untracked") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        artifact_file = _write_plan_artifact(repo, "docs/sprints/workflow_bootstrap.md", original_artifact)
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        seed_main_head = json.loads(snap_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        line_out = runner.invoke(app, ["line", "create", "feature/root-main-untracked", "--json"])
        assert line_out.exit_code == 0, line_out.stdout
        worktree_out = _invoke_internal_worktree_add(["worktree", "add", "root-main-untracked", "--line", "feature/root-main-untracked", "--json"])
        assert worktree_out.exit_code == 0, worktree_out.stdout
        worktree_path = Path(json.loads(worktree_out.stdout)["path"])

        (repo / "notes.txt").write_text("keep me\n", encoding="utf-8")

        monkeypatch.chdir(worktree_path)
        _write_plan_artifact(worktree_path, artifact_file, updated_artifact)

        blocked_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert blocked_out.exit_code == 2
        blocked_output = blocked_out.output or blocked_out.stdout
        assert "worktree" in blocked_output.lower()
        assert "repo root" in blocked_output.lower()
        assert "plan sync" in blocked_output.lower()

        assert (repo / "notes.txt").read_text(encoding="utf-8") == "keep me\n"
        assert (repo / artifact_file).read_text(encoding="utf-8") == original_artifact
        monkeypatch.chdir(repo)
        repo_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert repo_status_out.exit_code == 0, repo_status_out.stdout
        repo_status = json.loads(repo_status_out.stdout)
        assert repo_status["clean"] is False
        assert repo_status["untracked_paths"] == ["notes.txt"]
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_main_head
        assert remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"] == seed_main_head

        monkeypatch.chdir(worktree_path)
        worktree_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert worktree_status_out.exit_code == 0, worktree_status_out.stdout
        worktree_status = json.loads(worktree_status_out.stdout)
        assert worktree_status["clean"] is True
        assert worktree_status["modified_paths"] == []
        assert worktree_status["untracked_paths"] == []


def test_plan_sync_remote_archives_deleted_artifacts_under_directory_without_explicit_prune(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-prune"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-prune") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        _write_plan_artifact(
            repo,
            "docs/sprints/runtime_sync.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n",
        )
        _write_plan_artifact(
            repo,
            "docs/sprints/review_sync.md",
            "# Review Flow\n\n## Harden Review Flow [plan-ref: review-flow/root]\n\n- [ ] Add inbox grouping [ref: review-flow/inbox-grouping]\n",
        )

        create_out = runner.invoke(app, ["plan", "sync", "docs/sprints", "--remote", "origin", "--json"])
        assert create_out.exit_code == 0, create_out.stdout
        create_payload = json.loads(create_out.stdout)
        assert create_payload["summary"]["created_count"] == 2
        assert create_payload["summary"]["published_count"] == 2
        plan_ids_by_path = {row["artifact_path"]: row["plan_id"] for row in create_payload["results"]}

        (repo / "docs/sprints/review_sync.md").unlink()

        prune_out = runner.invoke(app, ["plan", "sync", "docs/sprints", "--remote", "origin", "--json"])
        assert prune_out.exit_code == 0, prune_out.stdout
        prune_payload = json.loads(prune_out.stdout)
        assert prune_payload["summary"]["pruned_count"] == 1
        assert prune_payload["summary"]["unchanged_count"] == 1
        assert prune_payload["summary"]["published_count"] == 1
        _assert_plan_sync_lineage_only(prune_payload)
        pruned_rows = [row for row in prune_payload["results"] if row["action"] == "pruned"]
        assert len(pruned_rows) == 1
        assert pruned_rows[0]["artifact_path"] == "docs/sprints/review_sync.md"
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        local_main_head = json.loads(main_line_out.stdout)["head_snapshot_id"]
        remote_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]
        assert remote_main_head == local_main_head

        show_pruned_out = runner.invoke(
            app,
            ["plan", "show", plan_ids_by_path["docs/sprints/review_sync.md"], "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert show_pruned_out.exit_code == 0, show_pruned_out.stdout
        assert json.loads(show_pruned_out.stdout)["status"] == "archived"


def test_plan_sync_remote_missing_file_target_adopts_remote_plan_before_archiving(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-prune-adopt-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-prune-adopt-remote") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/prune_remote_adoption.md",
            "# Remote Prune Adoption\n\n## Archive Missing Remote Plan [plan-ref: remote-prune/root]\n\n- [ ] Archive after delete [ref: remote-prune/archive]\n",
        )
        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "remote-prune/root",
            artifact_path=plan_file,
            title="Archive Missing Remote Plan",
        )
        plan_id = plan["plan_id"]

        (repo / plan_file).unlink()
        prune_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert prune_out.exit_code == 0, prune_out.stdout
        prune_payload = json.loads(prune_out.stdout)
        assert prune_payload["summary"]["adopted_count"] == 1
        assert prune_payload["summary"]["pruned_count"] == 1
        assert prune_payload["summary"]["published_count"] == 1
        assert prune_payload["results"][0]["plan_id"] == plan_id
        _assert_plan_sync_lineage_only(prune_payload)

        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        assert json.loads(show_out.stdout)["status"] == "archived"


def test_plan_sync_remote_directory_scope_archives_deleted_root_markdown_without_advancing_main(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-root-delete"
    repo.mkdir()
    (repo / "seed.txt").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-root-delete") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        artifact_file = _write_plan_artifact(
            repo,
            "workflow.md",
            "# Workflow Entry\n\nThis root-level workflow file is tracked by plan sync.\n",
        )
        create_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert create_out.exit_code == 0, create_out.stdout
        create_payload = json.loads(create_out.stdout)
        plan_id = create_payload["results"][0]["plan_id"]
        seed_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]

        repo_ctx = RepoContext.discover()
        local_closed = cli_module.close_local_plan(repo_ctx, plan_id, "archived")
        assert local_closed["status"] == "archived"
        remote_closed = remote_client_module.update_plan_status(base_url, plan_id, "archived")
        assert remote_closed["status"] == "archived"

        (repo / artifact_file).unlink()

        delete_out = runner.invoke(app, ["plan", "sync", ".", "--remote", "origin", "--json"])
        assert delete_out.exit_code == 0, delete_out.stdout
        delete_payload = json.loads(delete_out.stdout)
        assert delete_payload["summary"]["pruned_count"] == 0
        _assert_plan_sync_lineage_only(delete_payload)

        status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert status_out.exit_code == 0, status_out.stdout
        assert json.loads(status_out.stdout)["clean"] is True

        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        local_main_head = json.loads(main_line_out.stdout)["head_snapshot_id"]
        remote_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]
        assert local_main_head == seed_main_head
        assert remote_main_head == seed_main_head
        assert not (repo / artifact_file).exists()


def test_task_create_requires_plan_item_ref_in_strict_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-first-strict-binding"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-first-strict-binding") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "strict", "--json"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/strict_binding.md",
            "# Strict Binding\n\n## Bootstrap Durable Plan Storage [plan-ref: strict-binding/bootstrap]\n\n- add durable plans\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "strict-binding/bootstrap",
            artifact_path=plan_file,
            title="Bootstrap durable plan storage",
            summary="Seed the first plan revision",
        )

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--remote",
                "origin",
                "--title",
                "Plan-linked durable work",
                "--intent",
                "Promote the plan into a tracked execution task",
                "--risk",
                "medium",
                "--plan",
                plan["plan_id"],
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 2
        output = task_out.output or task_out.stdout
        assert "Strict plan/task binding requires `--plan-item-ref`" in output


def test_plan_close_command_is_not_available(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-close"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-close") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        close_out = runner.invoke(app, ["plan", "close"], catch_exceptions=False)
        assert close_out.exit_code != 0
        output = close_out.output or close_out.stdout
        assert "No such command 'close'" in output


def test_plan_items_lists_explicit_markdown_refs(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-items"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-items") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/runtime_stability.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n### Runtime Stability\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n- [x] Add busy timeout defaults [ref: runtime/busy-timeout]\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "runtime-stability/tasks",
            artifact_path=plan_file,
            title="Stabilize runtime execution tasks",
            summary="Seed a plan with explicit item refs",
        )

        items_out = runner.invoke(app, ["plan", "items", plan["plan_id"], "--remote", "origin", "--json"])
        assert items_out.exit_code == 0, items_out.stdout
        payload = json.loads(items_out.stdout)
        assert payload["plan_id"] == plan["plan_id"]
        assert payload["plan_revision_id"] == plan["head_revision"]["plan_revision_id"]
        assert payload["item_count"] == 2
        assert payload["items"][0]["plan_item_ref"] == "runtime/startup-only-init"
        assert payload["items"][0]["checkbox_state"] == "open"
        assert payload["items"][0]["heading_path"] == ["Stabilize Runtime Execution Tasks", "Runtime Stability"]
        assert payload["items"][1]["plan_item_ref"] == "runtime/busy-timeout"
        assert payload["items"][1]["checkbox_state"] == "done"


def test_plan_session_ids_follow_empty_namespace_prefix(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-session-empty-prefix"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-session-empty-prefix") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(
            app,
            ["config", "set", "--id-namespace-prefix", "", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/planning_session_empty_prefix.md",
            "# Planning Session Runtime\n\n## Runtime MVP [plan-ref: planning-session/runtime]\n\n- [ ] Add planning session runtime [ref: planning-session/runtime-mvp]\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "planning-session/runtime",
            artifact_path=plan_file,
            title="Plan-first planning session runtime",
            summary="Seed plan planning-session work",
        )

        session_out = runner.invoke(
            app,
            ["plan", "session", "create", plan["plan_id"], "--json"],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)
        assert session["planning_session_id"].startswith("PS-")
        assert not session["planning_session_id"].startswith("AITPS-")


def test_remote_workflow_ids_follow_empty_namespace_prefix(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-empty-id-namespace"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-empty-id-namespace") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        config_out = runner.invoke(
            app,
            [
                "config",
                "set",
                "--id-namespace-prefix",
                "",
                "--plan-task-binding-mode",
                "strict",
                "--task-tracking",
                "on",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert config_out.exit_code == 0, config_out.stdout
        config_payload = json.loads(config_out.stdout)
        assert config_payload["id_namespace_prefix"] == {
            "value": "",
            "source": "repo_config",
        }
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/empty_namespace.md",
            "# Empty Namespace\n\n## Remove Default Namespace Prefix [plan-ref: id-namespace/remove-default-prefix-plan]\n\n- [ ] emit workflow ids without the default namespace prefix [ref: id-namespace/remove-default-prefix]\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "id-namespace/remove-default-prefix-plan",
            artifact_path=plan_file,
            title="Remove default namespace prefix",
            summary="Seed namespace-aware plan execution",
        )
        assert plan["plan_id"].startswith("PL-")
        assert not plan["plan_id"].startswith("AITPL-")
        assert plan["head_revision"]["plan_revision_id"].startswith("PR-")
        assert not plan["head_revision"]["plan_revision_id"].startswith("AITPR-")

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--title",
                "Wire empty namespace through remote workflow",
                "--intent",
                "Verify remote ids stay namespace-aware when the prefix is blank",
                "--risk",
                "medium",
                "--plan",
                plan["plan_id"],
                "--plan-item-ref",
                "id-namespace/remove-default-prefix",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        assert task["task_id"].startswith("RT-")
        assert not task["task_id"].startswith("AITT-")
        assert task["tracking"]["session_id"].startswith("S-")
        assert not task["tracking"]["session_id"].startswith("AITS-")
        workspace = _bind_task_worktree(task["task_id"], monkeypatch)

        assert runner.invoke(app, ["line", "create", "feature/empty-id-namespace"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/empty-id-namespace"]).exit_code == 0
        (workspace / "app.py").write_text("print('empty namespace')\n", encoding="utf-8")
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "empty namespace work", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout

        change_out = runner.invoke(
            app,
            [
                "change",
                "create",
                "--task",
                task["task_id"],
                "--title",
                "Emit change ids without the default namespace prefix",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)
        assert change["change_id"].startswith("RC-")
        assert not change["change_id"].startswith("AITC-")

        patchset_out = runner.invoke(
            app,
            [
                "patchset",
                "publish",
                "--change",
                change["change_id"],
                "--summary",
                "empty namespace patchset",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)
        assert patchset["patchset_id"].startswith("RP-")
        assert not patchset["patchset_id"].startswith("AITP-")


def test_task_create_rejects_unknown_plan_item_ref(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-first-unknown-plan-item"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-first-unknown-plan-item") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/unknown_plan_item.md",
            "# Unknown Plan Item\n\n## Bootstrap Durable Plan Storage [plan-ref: unknown-plan-item/bootstrap]\n\n- [ ] bootstrap native workflow [ref: milestone-1/bootstrap-native-workflow]\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "unknown-plan-item/bootstrap",
            artifact_path=plan_file,
            title="Bootstrap durable plan storage",
            summary="Seed the first plan revision",
        )

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--title",
                "Bootstrap native workflow",
                "--intent",
                "Reject unknown plan item refs",
                "--risk",
                "medium",
                "--plan",
                plan["plan_id"],
                "--plan-item-ref",
                "milestone-1/unknown-item",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 2
        output = task_out.output or task_out.stdout
        assert "Known refs:" in output
        assert "milestone-1/bootstrap-native-workflow" in output


def test_task_create_rejects_dag_plan_without_plan_item_ref(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-dag-plan-item-required"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-dag-plan-item-required") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/dag_plan.md",
            "# DAG Plan\n\n## Bootstrap Durable Plan Storage [plan-ref: dag-plan/bootstrap]\n\n- [ ] bootstrap native workflow [ref: milestone-1/bootstrap-native-workflow]\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "dag-plan/bootstrap",
            artifact_path=plan_file,
            title="Bootstrap durable plan storage",
            summary="Seed the first plan revision",
        )
        ctx = ServerContext.from_env()
        plan_revision_id = plan["head_revision"]["plan_revision_id"]
        server_store_module.put_plan_revision_artifacts(
            ctx,
            plan["plan_id"],
            plan_revision_id,
            [
                _task_graph_artifact_payload(
                    repo_name="housekeeper",
                    plan_id=plan["plan_id"],
                    plan_revision_id=plan_revision_id,
                    plan_ref="dag-plan/bootstrap",
                    plan_item_ref="milestone-1/bootstrap-native-workflow",
                    artifact_path="docs/sprints/dag_plan.task_graph.json",
                    graph_id="dag-plan/task-graph",
                )
            ],
        )

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--title",
                "Bootstrap native workflow",
                "--intent",
                "Reject umbrella DAG task creation",
                "--risk",
                "medium",
                "--remote",
                "origin",
                "--plan",
                plan["plan_id"],
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 2
        output = task_out.output or task_out.stdout
        assert "plan/task binding requires `--plan-item-ref`" in output
        assert "--plan-item-ref" in output
        assert "task is linked to a plan" in output


def test_server_rejects_invalid_author_mode_for_attestation(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-invalid-author-mode"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-invalid-author-mode") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        _set_solo_remote_advisory()

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Invalid author mode", "--intent", "verify enum validation", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        workspace = _bind_task_worktree(task["task_id"], monkeypatch, name="invalid-author-mode")

        assert runner.invoke(app, ["line", "create", "feature/invalid-author-mode"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/invalid-author-mode"]).exit_code == 0
        (workspace / "app.py").write_text("print('feature')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature work"]).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Invalid author mode", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "invalid author mode patchset", "--json"],
            catch_exceptions=False,
        )
        patchset = json.loads(patchset_out.stdout)

        req = urllib.request.Request(
            f"{base_url}/v1/native/patchsets/{patchset['patchset_id']}/attestation",
            data=json.dumps(
                {
                    "author_mode": "robot_overlord",
                    "evaluation_summary": {"tests": "pass"},
                    "provenance_summary": {},
                    "detail": {},
                }
            ).encode("utf-8"),
            headers={"Content-Type": "application/json", "Accept": "application/json"},
            method="PUT",
        )
        try:
            urllib.request.urlopen(req, timeout=5)
            assert False, "expected invalid author_mode to be rejected"
        except urllib.error.HTTPError as exc:
            assert exc.code == 422


def test_server_store_task_persists_plan_item_ref_and_linked_timestamp(tmp_path: Path):
    server_data = tmp_path / "server-data-direct-plan-item"
    ctx = fake_postgres_context(server_data)
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main")
    artifact = _plan_artifact_payload(
        "# Direct Plan Store\n\n## Bootstrap Durable Plan Storage [plan-ref: direct-plan-item/bootstrap]\n\n- store plan records [ref: milestone-1/bootstrap-native-workflow]\n",
        "direct-plan-item/bootstrap",
    )

    plan = server_store_module.create_plan(
        ctx,
        "repo-a",
        "Bootstrap durable plan storage",
        artifact["artifact_path"],
        artifact["artifact_selector"],
        artifact["artifact_heading"],
        artifact["items"],
        summary="seed",
    )
    task = server_store_module.create_task(
        ctx,
        "repo-a",
        "Derive plan work",
        "Persist immutable plan item lineage on the task",
        "medium",
        plan_id=plan["plan_id"],
        plan_item_ref="milestone-1/bootstrap-native-workflow",
    )

    assert task["plan_id"] == plan["plan_id"]
    assert task["origin_plan_revision_id"] == plan["head_revision"]["plan_revision_id"]
    assert task["plan_item_ref"] == "milestone-1/bootstrap-native-workflow"
    assert task["plan_linked_at"]


def test_server_store_rejects_reusing_plan_item_ref_for_new_task(tmp_path: Path):
    server_data = tmp_path / "server-data-direct-duplicate-plan-item"
    ctx = fake_postgres_context(server_data)
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main")
    artifact = _plan_artifact_payload(
        "# Direct Plan Store\n\n## Bootstrap Durable Plan Storage [plan-ref: direct-plan-item/bootstrap]\n\n- store plan records [ref: milestone-1/bootstrap-native-workflow]\n",
        "direct-plan-item/bootstrap",
    )

    plan = server_store_module.create_plan(
        ctx,
        "repo-a",
        "Bootstrap durable plan storage",
        artifact["artifact_path"],
        artifact["artifact_selector"],
        artifact["artifact_heading"],
        artifact["items"],
        summary="seed",
    )
    task = server_store_module.create_task(
        ctx,
        "repo-a",
        "Derive plan work",
        "Persist immutable plan item lineage on the task",
        "medium",
        plan_id=plan["plan_id"],
        plan_item_ref="milestone-1/bootstrap-native-workflow",
    )
    closed = server_store_module.close_task(ctx, task["task_id"], "completed")
    assert closed["status"] == "completed"

    with pytest.raises(ValueError, match="already linked to task"):
        server_store_module.create_task(
            ctx,
            "repo-a",
            "Incorrect follow-up task",
            "This should be rejected because the plan item ref already has task history",
            "medium",
            plan_id=plan["plan_id"],
            plan_item_ref="milestone-1/bootstrap-native-workflow",
        )


def test_doctor_postgres_reports_preflight_issues(tmp_path: Path, monkeypatch):
    server_data = tmp_path / "server-data"
    monkeypatch.setenv("AIT_NATIVE_SERVER_DATA", str(server_data))
    monkeypatch.setenv("AIT_NATIVE_SERVER_DB_BACKEND", "postgres")
    monkeypatch.delenv("AIT_NATIVE_SERVER_POSTGRES_DSN", raising=False)
    result = runner.invoke(app, ["doctor", "postgres", "--json"])
    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["backend"] == "postgres"
    assert payload["server_data_root"] == str(server_data.resolve())
    assert payload["ready"] is False
    assert payload["postgres_dsn_configured"] is False
    assert "AIT_NATIVE_SERVER_POSTGRES_DSN is not configured." in payload["issues"]
