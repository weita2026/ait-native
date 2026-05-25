from __future__ import annotations

import pytest
import ait.cli.task_dag_telegram_watch as watch_cli_module
from typing import Any

from ._shared import *  # noqa: F401,F403
from ait_native.store import create_snapshot as local_create_snapshot
from ait_native.store import get_snapshot as local_get_snapshot


def _assert_plan_sync_lineage_only(payload: dict[str, object]) -> None:
    assert "line_sync" not in payload
    assert "root_main_sync" not in payload
    assert "remote_main_sync" not in payload


def test_plan_sync_command_prefix_stays_active_root_guard_bypass():
    prefixes = cli_module.ACTIVE_ROOT_WORKTREE_GUARD_BYPASS_PREFIXES

    assert any("plan sync docs/sprints/example.md --remote origin".startswith(prefix) for prefix in prefixes)
    assert not any("snapshot create --message blocked".startswith(prefix) for prefix in prefixes)


def test_init_bootstraps_agents_and_next_step_guidance(tmp_path: Path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir()
    monkeypatch.chdir(repo)

    result = runner.invoke(app, ["init", "--name", "repo", "--json"])
    assert result.exit_code == 0, result.stdout

    payload = json.loads(result.stdout)
    assert payload["workflow_mode"]["value"] == "solo_local"
    assert payload["bootstrap_files"] == [
        {"path": "AGENTS.md"},
        {"path": "ait-native.md"},
        {"path": "docs/plan.md"},
        {"path": "docs/milestone.md"},
    ]
    assert payload["bootstrap_guide"] == {"path": "ait-native.md"}
    assert payload["bootstrap_directories"] == [{"path": "docs/sprints"}]
    assert payload["forbidden_bootstrap_paths"] == [{"path": "docs/sprints/README.md"}]
    assert payload["next_steps"]["workflow_guides"] == [
        "ait workflow guide inventory",
        "ait workflow guide land",
    ]
    assert 'ait task start --local --title "Describe the work" --intent "Explain the outcome" --base-line main' in payload["next_steps"]["solo_local"]
    assert f"ait remote add origin <url> --repo-name {payload['repo_name']} --default" in payload["next_steps"]["optional_solo_remote"]
    assert 'ait task start --title "Describe the work" --intent "Explain the outcome" --base-line main' in payload["next_steps"]["optional_solo_remote"]

    agents_text = (repo / "AGENTS.md").read_text(encoding="utf-8")
    assert "solo_local" in agents_text
    assert "docs/sprints/" in agents_text
    assert "docs/sprints/README.md" in agents_text
    assert "ait queue summary" in agents_text
    assert "ait task audit <task-id>" in agents_text
    assert "ait workflow land <change-id>" in agents_text
    assert "ait-native.md" in agents_text
    native_text = (repo / "ait-native.md").read_text(encoding="utf-8")
    assert "default mode after `ait init`: `solo_local`" in native_text
    assert "ait config set --workflow-mode solo_remote" in native_text
    assert "ait config set --workflow-mode team_remote" not in native_text
    assert "It does not migrate existing plan / task / change lineage." in native_text
    assert 'ait task start --local --title "Describe the work" --intent "Explain the outcome" --base-line main' in native_text
    assert f"ait remote add origin <url> --repo-name {payload['repo_name']} --default" in native_text
    assert "ait workflow guide inventory" in native_text
    assert "ait workflow guide land" in native_text
    assert "docs/ait.md" not in native_text
    assert "docs/ait_team_remote.md" not in native_text
    assert (repo / "docs" / "plan.md").exists()
    assert (repo / "docs" / "milestone.md").exists()
    assert (repo / "docs" / "sprints").is_dir()
    assert not (repo / "docs" / "sprints" / "README.md").exists()


def test_init_keeps_docs_sprints_readme_forbidden_as_bootstrap_surface(tmp_path: Path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir()
    monkeypatch.chdir(repo)

    result = runner.invoke(app, ["init", "--name", "repo", "--json"])
    assert result.exit_code == 0, result.stdout

    payload = json.loads(result.stdout)
    assert payload["bootstrap_files"] == [
        {"path": "AGENTS.md"},
        {"path": "ait-native.md"},
        {"path": "docs/plan.md"},
        {"path": "docs/milestone.md"},
    ]
    assert payload["forbidden_bootstrap_paths"] == [{"path": "docs/sprints/README.md"}]
    assert not any(row["path"] == "docs/sprints/README.md" for row in payload["bootstrap_files"])
    assert not (repo / "docs" / "sprints" / "README.md").exists()


def test_init_refuses_existing_ait_directory_and_help_no_longer_advertises_force(tmp_path: Path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir()
    monkeypatch.chdir(repo)

    help_out = runner.invoke(app, ["init", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "--force" not in help_out.stdout

    first_out = runner.invoke(app, ["init", "--name", "repo"], catch_exceptions=False)
    assert first_out.exit_code == 0, first_out.stdout

    second_out = runner.invoke(app, ["init", "--name", "repo"])
    assert second_out.exit_code == 1
    assert isinstance(second_out.exception, FileExistsError)
    assert ".ait already exists" in str(second_out.exception)


def test_status_uses_ait_repo_root_from_other_working_directory(tmp_path: Path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir()
    other = tmp_path / "other"
    other.mkdir()
    monkeypatch.chdir(repo)

    init_result = runner.invoke(app, ["init"])
    assert init_result.exit_code == 0, init_result.stdout

    monkeypatch.chdir(other)
    monkeypatch.setenv("AIT_REPO_ROOT", str(repo))
    result = runner.invoke(app, ["status", "--json"])

    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["repo_name"] == "repo"


def test_status_reports_workspace_dirty_summary_in_json_and_default_output(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-status-dirty"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("hello\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-status-dirty"]).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert snap_out.exit_code == 0, snap_out.stdout

    readme.write_text("hello dirty\n", encoding="utf-8")
    (repo / "notes.txt").write_text("new note\n", encoding="utf-8")

    status_json_out = runner.invoke(app, ["status", "--json"])
    assert status_json_out.exit_code == 0, status_json_out.stdout
    status = json.loads(status_json_out.stdout)
    assert status["workspace_status"] == "dirty"
    assert status["workspace_dirty"] is True
    assert status["workspace_changed_count"] == 1
    assert status["workspace_modified_count"] == 0
    assert status["workspace_untracked_count"] == 1
    assert status["workspace_changed_paths_sample"] == ["notes.txt"]

    status_out = runner.invoke(app, ["status"])
    assert status_out.exit_code == 0, status_out.stdout
    output = status_out.output or status_out.stdout
    assert "ait status" in output
    assert "workspace" in output
    assert "dirty (1 changed)" in output


def test_status_uses_cached_worktree_hygiene_by_default(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-status-cached-worktrees"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    observed: dict[str, object] = {}
    store_worktrees_module = import_module("ait.store_worktrees")

    def fake_list_worktrees(ctx, *, refresh_status: bool = True):
        observed["refresh_status"] = refresh_status
        return [{"name": "missingcase", "workspace_status": "missing"}]

    def fake_worktree_doctor_from_rows(rows):
        observed["rows"] = rows
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

    monkeypatch.setattr(store_worktrees_module, "list_worktrees", fake_list_worktrees)
    monkeypatch.setattr(store_worktrees_module, "worktree_doctor_from_rows", fake_worktree_doctor_from_rows)

    status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    payload = json.loads(status_out.stdout)
    assert payload["worktree_hygiene"]["stale_count"] == 1
    assert observed["refresh_status"] is False
    assert observed["rows"] == [{"name": "missingcase", "workspace_status": "missing"}]


def test_status_reuses_one_cached_worktree_inventory_for_hygiene(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-status-shared-worktree-inventory"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    call_count = 0
    store_worktrees_module = import_module("ait.store_worktrees")
    original_list_worktrees = store_worktrees_module.list_worktrees

    def counted_list_worktrees(ctx, *, refresh_status=True):
        nonlocal call_count
        call_count += 1
        return original_list_worktrees(ctx, refresh_status=refresh_status)

    monkeypatch.setattr(store_worktrees_module, "list_worktrees", counted_list_worktrees)

    status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    payload = json.loads(status_out.stdout)
    assert payload["worktree_hygiene"]["total_count"] == 0
    assert call_count == 1


def test_status_avoids_full_storage_stats_in_default_path(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-status-lightweight-storage"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0

    local_content_module = import_module("ait.local_content")
    monkeypatch.setattr(
        local_content_module,
        "storage_stats",
        lambda ctx: (_ for _ in ()).throw(AssertionError("status should use lightweight storage counts")),
    )

    status_out = runner.invoke(app, ["status", "--json"], catch_exceptions=False)
    assert status_out.exit_code == 0, status_out.stdout
    payload = json.loads(status_out.stdout)
    assert payload["snapshot_count"] >= 1
    assert payload["pack_count"] >= 0
    assert payload["packed_blob_count"] >= 0


def test_line_and_worktree_help_frame_local_movement_and_isolated_workspace_story():
    line_help = runner.invoke(app, ["line", "--help"])
    assert line_help.exit_code == 0, line_help.stdout
    assert "list" in line_help.stdout
    assert "create" in line_help.stdout
    assert "switch" in line_help.stdout
    assert "show" in line_help.stdout
    assert "archive" in line_help.stdout
    assert "cleanup-candidates" in line_help.stdout
    assert "cleanup" in line_help.stdout

    line_list_help = runner.invoke(app, ["line", "list", "--help"])
    assert line_list_help.exit_code == 0, line_list_help.stdout
    assert "List local or remote lines and their current head snapshots." in " ".join(line_list_help.stdout.split())

    line_create_help = runner.invoke(app, ["line", "create", "--help"])
    assert line_create_help.exit_code == 0, line_create_help.stdout
    assert "Create a new line from the current head or an explicit snapshot." in " ".join(line_create_help.stdout.split())

    line_archive_help = runner.invoke(app, ["line", "archive", "--help"])
    assert line_archive_help.exit_code == 0, line_archive_help.stdout
    assert "Archive a local line or close a shared remote line." in " ".join(line_archive_help.stdout.split())

    line_cleanup_help = runner.invoke(app, ["line", "cleanup", "--help"])
    assert line_cleanup_help.exit_code == 0, line_cleanup_help.stdout
    assert "Archive line cleanup candidates after an explicit confirmation." in " ".join(line_cleanup_help.stdout.split())

    worktree_help = runner.invoke(app, ["worktree", "--help"])
    assert worktree_help.exit_code == 0, worktree_help.stdout
    for command in ["show", "path", "open", "exec", "doctor", "cleanup-candidates", "cleanup", "prune-stale", "list", "sync", "recreate", "restore-owned-head", "rebase", "remove"]:
        assert command in worktree_help.stdout
    assert "add" not in worktree_help.stdout
    assert "bind" not in worktree_help.stdout
    assert "promote" not in worktree_help.stdout
    assert "extract" not in worktree_help.stdout.lower()

    worktree_add_help = runner.invoke(app, ["worktree", "add", "--help"])
    assert worktree_add_help.exit_code != 0
    assert "No such command 'add'" in (worktree_add_help.output or worktree_add_help.stdout)

    worktree_bind_help = runner.invoke(app, ["worktree", "bind", "--help"])
    assert worktree_bind_help.exit_code != 0
    assert "No such command 'bind'" in (worktree_bind_help.output or worktree_bind_help.stdout)

    worktree_promote_help = runner.invoke(app, ["worktree", "promote", "--help"])
    assert worktree_promote_help.exit_code != 0
    assert "No such command 'promote'" in (worktree_promote_help.output or worktree_promote_help.stdout)

    worktree_path_help = runner.invoke(app, ["worktree", "path", "--help"])
    assert worktree_path_help.exit_code == 0, worktree_path_help.stdout
    assert "Print a worktree path or shell-open helpers for entering it." in " ".join(worktree_path_help.stdout.split())

    worktree_sync_help = runner.invoke(app, ["worktree", "sync", "--help"])
    assert worktree_sync_help.exit_code == 0, worktree_sync_help.stdout
    assert "Restore one worktree, or all live worktrees, to their intended line heads." in " ".join(worktree_sync_help.stdout.split())

    worktree_remove_help = runner.invoke(app, ["worktree", "remove", "--help"])
    assert worktree_remove_help.exit_code == 0, worktree_remove_help.stdout
    assert "Remove one or more worktree registrations and optionally delete their paths." in " ".join(worktree_remove_help.stdout.split())

    workspace_restore_help = runner.invoke(app, ["workspace", "restore", "--help"])
    assert workspace_restore_help.exit_code == 0, workspace_restore_help.stdout
    assert "--path" in workspace_restore_help.stdout
    assert "selected workspace paths" in " ".join(workspace_restore_help.stdout.split())


def test_history_returns_snapshot_rows_with_line_heads(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-history"
    repo.mkdir()
    readme = repo / "README.md"
    app_file = repo / "app.py"
    readme.write_text("base\n", encoding="utf-8")
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-history"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/history"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/history"]).exit_code == 0
    app_file.write_text("print('feature work')\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout
    feature_snapshot = json.loads(feature_out.stdout)

    history_out = runner.invoke(app, ["history", "--json"])
    assert history_out.exit_code == 0, history_out.stdout
    history_rows = json.loads(history_out.stdout)

    assert history_rows[0]["snapshot_id"] == feature_snapshot["snapshot_id"]
    assert history_rows[0]["head_lines"] == ["feature/history"]
    assert history_rows[0]["is_current_head"] is True
    assert history_rows[0]["marker"] == "@"
    assert history_rows[1]["snapshot_id"] == main_snapshot["snapshot_id"]
    assert history_rows[1]["head_lines"] == ["main"]
    assert history_rows[1]["is_head"] is True
    assert history_rows[1]["marker"] == "*"


def test_history_can_filter_to_selected_line_ancestry(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-history-line"
    repo.mkdir()
    readme = repo / "README.md"
    app_file = repo / "app.py"
    readme.write_text("base\n", encoding="utf-8")
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-history-line"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/history"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/history"]).exit_code == 0
    readme.write_text("feature work one\n", encoding="utf-8")
    first_feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature one", "--json"])
    assert first_feature_out.exit_code == 0, first_feature_out.stdout
    first_feature_snapshot = json.loads(first_feature_out.stdout)

    readme.write_text("feature work two\n", encoding="utf-8")
    second_feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature two", "--json"])
    assert second_feature_out.exit_code == 0, second_feature_out.stdout
    second_feature_snapshot = json.loads(second_feature_out.stdout)

    history_out = runner.invoke(app, ["history", "--line", "feature/history", "--json"])
    assert history_out.exit_code == 0, history_out.stdout
    history_rows = json.loads(history_out.stdout)

    assert [row["snapshot_id"] for row in history_rows] == [
        second_feature_snapshot["snapshot_id"],
        first_feature_snapshot["snapshot_id"],
        main_snapshot["snapshot_id"],
    ]
    assert history_rows[0]["graph"] == "@"
    assert history_rows[0]["is_selected_line_head"] is True
    assert history_rows[1]["graph"] == "|"
    assert history_rows[2]["graph"] == "|"


def test_workspace_restore_can_materialize_snapshot_without_switching_current_line(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-restore-snapshot"
    repo.mkdir()
    app_file = repo / "app.py"
    notes = repo / "notes.txt"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-restore-snapshot"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/restore"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/restore"]).exit_code == 0
    app_file.write_text("print('feature')\n", encoding="utf-8")
    notes.write_text("feature note\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout

    restore_out = runner.invoke(
        app,
        ["workspace", "restore", "--snapshot", main_snapshot["snapshot_id"], "--json"])
    assert restore_out.exit_code == 0, restore_out.stdout
    payload = json.loads(restore_out.stdout)
    assert payload["target_snapshot_id"] == main_snapshot["snapshot_id"]
    assert payload["current_line"] == "feature/restore"
    assert payload["applied"] is True
    assert payload["plan"]["remove_count"] == 1
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"
    assert not notes.exists()


def test_workspace_restore_selected_paths_requires_explicit_snapshot_or_line(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-restore-selected-paths-explicit"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-restore-selected-paths-explicit"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0

    restore_out = runner.invoke(app, ["workspace", "restore", "--path", "app.py"], catch_exceptions=False)
    assert restore_out.exit_code != 0
    output = restore_out.output or restore_out.stdout or restore_out.stderr or ""
    assert "Selected-path restore requires --snapshot or --line" in output


def test_workspace_restore_selected_paths_from_snapshot_can_restore_multiple_paths(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-restore-selected-paths-snapshot"
    repo.mkdir()
    app_file = repo / "app.py"
    notes = repo / "notes.txt"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-restore-selected-paths-snapshot"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/restore"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/restore"]).exit_code == 0
    app_file.write_text("print('feature')\n", encoding="utf-8")
    notes.write_text("feature note\n", encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"]).exit_code == 0

    restore_out = runner.invoke(
        app,
        [
            "workspace",
            "restore",
            "--snapshot",
            main_snapshot["snapshot_id"],
            "--path",
            "app.py",
            "--path",
            "notes.txt",
            "--json",
        ],
    )
    assert restore_out.exit_code == 0, restore_out.stdout
    payload = json.loads(restore_out.stdout)
    assert payload["target_snapshot_id"] == main_snapshot["snapshot_id"]
    assert payload["current_line"] == "feature/restore"
    assert payload["line_name"] == "feature/restore"
    assert payload["applied"] is True
    assert payload["plan"]["requested_paths"] == ["app.py", "notes.txt"]
    assert payload["plan"]["write_count"] == 1
    assert payload["plan"]["remove_count"] == 1
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"
    assert not notes.exists()


def test_workspace_restore_selected_paths_from_line_keeps_current_line_and_dirty_outside_paths(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-restore-selected-paths-line"
    repo.mkdir()
    app_file = repo / "app.py"
    notes = repo / "notes.txt"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-restore-selected-paths-line"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0

    assert runner.invoke(app, ["line", "create", "feature/restore"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/restore"]).exit_code == 0
    app_file.write_text("print('feature')\n", encoding="utf-8")
    notes.write_text("feature note\n", encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"]).exit_code == 0

    notes.write_text("feature note, still dirty\n", encoding="utf-8")
    restore_out = runner.invoke(
        app,
        ["workspace", "restore", "--line", "main", "--path", "app.py", "--json"],
        catch_exceptions=False,
    )
    assert restore_out.exit_code == 0, restore_out.stdout
    payload = json.loads(restore_out.stdout)
    assert payload["current_line"] == "feature/restore"
    assert payload["line_name"] == "main"
    assert payload["workspace_dirty"] is True
    assert payload["would_overwrite_selected_changes"] is False
    assert payload["dirty_selected_paths"] == []
    assert payload["dirty_outside_paths"] == ["notes.txt"]
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"
    assert notes.read_text(encoding="utf-8") == "feature note, still dirty\n"


def test_workspace_restore_selected_paths_rejects_dirty_selected_paths_without_force(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-restore-selected-paths-dirty"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-restore-selected-paths-dirty"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/restore"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/restore"]).exit_code == 0
    app_file.write_text("print('feature')\n", encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"]).exit_code == 0

    app_file.write_text("print('dirty selected change')\n", encoding="utf-8")
    restore_out = runner.invoke(
        app,
        ["workspace", "restore", "--snapshot", main_snapshot["snapshot_id"], "--path", "app.py"],
        catch_exceptions=False,
    )
    assert restore_out.exit_code != 0
    output = restore_out.output or restore_out.stdout or restore_out.stderr or ""
    assert "Selected paths have unsaved changes relative to" in output
    assert app_file.read_text(encoding="utf-8") == "print('dirty selected change')\n"


def test_workspace_status_reports_dirty_paths_relative_to_current_line_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workspace-status"
    repo.mkdir()
    app_file = repo / "app.py"
    notes = repo / "notes.txt"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-workspace-status"]).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    app_file.write_text("print('dirty change')\n", encoding="utf-8")
    notes.write_text("new note\n", encoding="utf-8")

    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    payload = json.loads(status_out.stdout)
    assert payload["repo_name"] == "housekeeper-workspace-status"
    assert payload["current_line"] == "main"
    assert payload["baseline_source"] == "current_line_head"
    assert payload["baseline_line_name"] == "main"
    assert payload["baseline_snapshot_id"] == snapshot["snapshot_id"]
    assert payload["clean"] is False
    assert payload["changed_count"] == 2
    assert "app.py" in payload["modified_paths"]
    assert "notes.txt" in payload["untracked_paths"]
    assert sorted(payload["changed_paths"]) == ["app.py", "notes.txt"]
    assert payload["phase_timings_ms"]["workspace_scan"] >= 0
    assert payload["phase_timings_ms"]["ignore_filtering"] >= 0
    assert payload["phase_timings_ms"]["hashing"] >= 0
    assert payload["phase_timings_ms"]["compare_manifest"] >= 0
    assert payload["phase_timings_ms"]["total"] >= 0


def test_snapshot_create_json_exposes_phase_timings(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-snapshot-phase-timings"
    repo.mkdir()
    app_file = repo / "app.py"
    notes = repo / "notes.txt"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    notes.write_text("seed note\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-snapshot-phase-timings"]).exit_code == 0

    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert snap_out.exit_code == 0, snap_out.stdout
    payload = json.loads(snap_out.stdout)

    assert payload["phase_timings_ms"]["workspace_scan"] >= 0
    assert payload["phase_timings_ms"]["ignore_filtering"] >= 0
    assert payload["phase_timings_ms"]["workspace_projection_filter"] >= 0
    assert payload["phase_timings_ms"]["hashing"] >= 0
    assert payload["phase_timings_ms"]["tree_record_stage"] >= 0
    assert payload["phase_timings_ms"]["pack_archive_write"]["blob_pack_write"] >= 0
    assert payload["phase_timings_ms"]["pack_archive_write"]["tree_pack_write"] >= 0
    assert payload["phase_timings_ms"]["pack_archive_write"]["total"] >= 0
    assert payload["phase_timings_ms"]["metadata_commit"] >= 0
    assert payload["phase_timings_ms"]["total"] >= 0


def test_workspace_status_default_output_is_human_readable(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workspace-status-human"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-workspace-status-human"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0

    app_file.write_text("print('dirty change')\n", encoding="utf-8")

    status_out = runner.invoke(app, ["workspace", "status"])
    assert status_out.exit_code == 0, status_out.stdout
    output = status_out.output or status_out.stdout
    assert "ait workspace status (dirty)" in output
    assert "current line" in output
    assert "baseline" in output
    assert "changed files" in output
    assert "changed paths" in output
    assert "modified" in output
    assert "app.py" in output


def test_task_worktree_keeps_non_sprint_docs_out_of_auto_created_worktree_and_preserves_parent_docs(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-worktree-sprints-only"
    repo.mkdir()
    readme = repo / "README.md"
    docs_dir = repo / "docs"
    sprints_dir = docs_dir / "sprints"
    readme.write_text("base\n", encoding="utf-8")
    docs_dir.mkdir()
    sprints_dir.mkdir()
    (docs_dir / "plan.md").write_text("top-level docs stay in root only\n", encoding="utf-8")
    (sprints_dir / "card.md").write_text("# Sprint Card\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout

    config_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--plan-task-binding-mode",
            "advisory",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert config_out.exit_code == 0, config_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Sprint-focused worktree",
            "--intent",
            "keep docs markdown out of the auto-created worktree",
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

    assert not (worktree_path / "README.md").exists()
    assert not (worktree_path / "docs" / "sprints" / "card.md").exists()
    assert not (worktree_path / "docs" / "plan.md").exists()

    monkeypatch.chdir(worktree_path)
    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["changed_count"] == 0

    (worktree_path / "feature.txt").write_text("feature only\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature side", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout
    feature_snapshot = json.loads(feature_out.stdout)

    worktree_ctx = RepoContext.discover(worktree_path)
    stored_snapshot = local_get_snapshot(worktree_ctx, feature_snapshot["snapshot_id"])
    snapshot_paths = {row["path"] for row in stored_snapshot["files"]}
    assert "README.md" not in snapshot_paths
    assert "docs/sprints/card.md" not in snapshot_paths
    assert "docs/plan.md" not in snapshot_paths
    assert "feature.txt" in snapshot_paths


def test_workspace_status_can_compare_against_explicit_line_or_snapshot(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-workspace-status-targets"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-workspace-status-targets"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/status"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/status"]).exit_code == 0
    app_file.write_text("print('feature')\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature seed", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout

    line_status_out = runner.invoke(app, ["workspace", "status", "--line", "main", "--json"])
    assert line_status_out.exit_code == 0, line_status_out.stdout
    line_payload = json.loads(line_status_out.stdout)
    assert line_payload["baseline_source"] == "line_head"
    assert line_payload["baseline_line_name"] == "main"
    assert line_payload["baseline_snapshot_id"] == main_snapshot["snapshot_id"]
    assert line_payload["clean"] is False
    assert line_payload["modified_paths"] == ["app.py"]

    snapshot_status_out = runner.invoke(app, ["workspace", "status", "--snapshot", main_snapshot["snapshot_id"], "--json"])
    assert snapshot_status_out.exit_code == 0, snapshot_status_out.stdout
    snapshot_payload = json.loads(snapshot_status_out.stdout)
    assert snapshot_payload["baseline_source"] == "snapshot"
    assert snapshot_payload["baseline_line_name"] is None
    assert snapshot_payload["baseline_snapshot_id"] == main_snapshot["snapshot_id"]
    assert snapshot_payload["modified_paths"] == ["app.py"]


def test_worktree_add_materializes_isolated_workspace_for_line(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/ux"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/ux"]).exit_code == 0
    app_file.write_text("print('feature ux')\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature seed", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout
    feature_snapshot = json.loads(feature_out.stdout)

    restore_main = runner.invoke(app, ["line", "switch", "main", "--restore", "--json"])
    assert restore_main.exit_code == 0, restore_main.stdout
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"

    add_out = _invoke_internal_worktree_add(["worktree", "add", "ux", "--line", "feature/ux", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    payload = json.loads(add_out.stdout)
    worktree_path = Path(payload["path"])
    assert payload["line_name"] == "feature/ux"
    assert payload["head_snapshot_id"] == feature_snapshot["snapshot_id"]
    assert worktree_path.is_dir()
    assert (worktree_path / ".ait").is_dir()
    assert (worktree_path / ".ait-worktree.json").exists()
    assert (worktree_path / "app.py").read_text(encoding="utf-8") == "print('feature ux')\n"
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"

    monkeypatch.chdir(worktree_path)
    config_out = runner.invoke(app, ["config", "show", "--json"])
    assert config_out.exit_code == 0, config_out.stdout
    config_payload = json.loads(config_out.stdout)
    assert config_payload["is_worktree"] is True
    assert config_payload["worktree_name"] == "ux"
    assert config_payload["current_line"] == "feature/ux"
    assert Path(config_payload["repo_root"]) == repo
    assert Path(config_payload["workspace_root"]) == worktree_path

    status_out = runner.invoke(app, ["status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status_payload = json.loads(status_out.stdout)
    assert status_payload["current_line"] == "feature/ux"
    assert status_payload["head_snapshot_id"] == feature_snapshot["snapshot_id"]
    assert status_payload["workspace_dirty"] is False

    monkeypatch.chdir(repo)
    root_status_out = runner.invoke(app, ["status", "--json"])
    assert root_status_out.exit_code == 0, root_status_out.stdout
    root_status = json.loads(root_status_out.stdout)
    assert root_status["current_line"] == "main"
    assert root_status["head_snapshot_id"] == main_snapshot["snapshot_id"]
    assert root_status["workspace_dirty"] is False


def test_worktree_add_defaults_to_current_line_and_show_reports_status(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-default-line"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-default-line"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/defaulted"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/defaulted"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "defaulted", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    added = json.loads(add_out.stdout)
    assert added["line_name"] == "feature/defaulted"

    show_out = runner.invoke(app, ["worktree", "show", "defaulted", "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["name"] == "defaulted"
    assert shown["current_line"] == "feature/defaulted"
    assert shown["workspace_status"] == "clean"
    assert shown["changed_count"] == 0
    assert shown["creation_kind"] == "manual_add"
    assert shown["cleanup_policy"] == "manual_only"
    assert shown["cleanup_class"] == "protected"
    assert shown["cleanup_candidate"] is False
    assert shown["protected_reason"] == "cleanup policy manual_only"
    assert shown["last_used_at"]


def test_worktree_show_backfills_cleanup_metadata_for_legacy_registration(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-legacy-cleanup-metadata"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-legacy-cleanup-metadata"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "legacy", "--json"])
    assert add_out.exit_code == 0, add_out.stdout

    metadata_path = repo / ".ait" / "worktrees" / "legacy.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    created_at = metadata["created_at"]
    metadata.pop("creation_kind", None)
    metadata.pop("cleanup_policy", None)
    metadata.pop("last_used_at", None)
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

    show_out = runner.invoke(app, ["worktree", "show", "legacy", "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["creation_kind"] == "manual_add"
    assert shown["cleanup_policy"] == "manual_only"
    assert shown["last_used_at"] == created_at
    assert shown["cleanup_class"] == "protected"
    assert shown["protected_reason"] == "cleanup policy manual_only"


def test_worktree_show_normalizes_retired_creation_kinds_to_manual_add(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-retired-kind"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-retired-kind"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "legacy", "--json"])
    assert add_out.exit_code == 0, add_out.stdout

    metadata_path = repo / ".ait" / "worktrees" / "legacy.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["creation_kind"] = "retired_kind"
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

    show_out = runner.invoke(app, ["worktree", "show", "legacy", "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["creation_kind"] == "manual_add"
    assert shown["cleanup_policy"] == "manual_only"


def test_internal_worktree_promote_creates_new_line_and_keeps_dirty_state(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-promote"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-promote"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/base"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "promoted", "--line", "feature/base", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    added = json.loads(add_out.stdout)
    worktree_path = Path(added["path"])

    monkeypatch.chdir(worktree_path)
    (worktree_path / "README.md").write_text("dirty promoted\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    promote_out = _invoke_internal_worktree_promote(["worktree", "promote", "promoted", "--line", "feature/promoted", "--json"])
    assert promote_out.exit_code == 0, promote_out.stdout
    promoted = json.loads(promote_out.stdout)
    assert promoted["previous_line_name"] == "feature/base"
    assert promoted["line_name"] == "feature/promoted"
    assert promoted["current_line"] == "feature/promoted"
    assert promoted["created_line"]["line_name"] == "feature/promoted"
    assert promoted["created_line"]["head_snapshot_id"] == added["head_snapshot_id"]

    line_out = runner.invoke(app, ["line", "show", "feature/promoted", "--json"])
    assert line_out.exit_code == 0, line_out.stdout
    line_payload = json.loads(line_out.stdout)
    assert line_payload["head_snapshot_id"] == added["head_snapshot_id"]

    monkeypatch.chdir(worktree_path)
    config_out = runner.invoke(app, ["config", "show", "--json"])
    assert config_out.exit_code == 0, config_out.stdout
    config_payload = json.loads(config_out.stdout)
    assert config_payload["current_line"] == "feature/promoted"


def test_internal_worktree_promote_can_target_current_worktree_without_name(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-promote-current"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-promote-current"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/from-current"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "currentpromote", "--line", "feature/from-current", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    monkeypatch.chdir(worktree_path)
    promote_out = _invoke_internal_worktree_promote(["worktree", "promote", "--line", "feature/current-promoted", "--json"])
    assert promote_out.exit_code == 0, promote_out.stdout
    promoted = json.loads(promote_out.stdout)
    assert promoted["name"] == "currentpromote"
    assert promoted["current_line"] == "feature/current-promoted"


def test_worktree_path_and_open_commands_report_navigable_location(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-path"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-path"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "nav", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    added = json.loads(add_out.stdout)
    worktree_path = Path(added["path"])
    navigable_path = str(added.get("open_path") or added.get("alias_path") or added["path"])

    path_out = runner.invoke(app, ["worktree", "path", "nav"])
    assert path_out.exit_code == 0, path_out.stdout
    assert (path_out.output or path_out.stdout).strip() == navigable_path

    shell_out = runner.invoke(app, ["worktree", "path", "nav", "--shell"])
    assert shell_out.exit_code == 0, shell_out.stdout
    shell_command = (shell_out.output or shell_out.stdout).strip()
    assert shell_command.startswith(f"cd {shlex.quote(navigable_path)} &&")
    assert "export AIT_WORKTREE_NAME=nav" in shell_command
    assert "export AIT_WORKTREE_LINE=main" in shell_command
    assert f"export PYTHONPATH={shlex.quote(str(worktree_path / 'src'))}${{PYTHONPATH:+:$PYTHONPATH}}" in shell_command

    open_out = runner.invoke(app, ["worktree", "open", "nav", "--json"])
    assert open_out.exit_code == 0, open_out.stdout
    opened = json.loads(open_out.stdout)
    assert opened["path"] == str(worktree_path)
    assert opened["open_path"] == navigable_path
    assert opened["cd_command"] == f"cd {shlex.quote(navigable_path)}"
    assert opened["shell_command"] == shell_command
    assert opened["src_path"] == str(worktree_path / "src")
    assert opened["venv_path"] is None
    assert opened["workspace_status"] == "clean"

    monkeypatch.chdir(worktree_path)
    current_out = runner.invoke(app, ["worktree", "path", "--json"])
    assert current_out.exit_code == 0, current_out.stdout
    current_payload = json.loads(current_out.stdout)
    assert current_payload["name"] == "nav"
    assert current_payload["path"] == str(worktree_path)


def test_worktree_path_and_exec_refresh_last_used_at(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-last-used"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-last-used"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "usagecase", "--json"])
    assert add_out.exit_code == 0, add_out.stdout

    metadata_path = repo / ".ait" / "worktrees" / "usagecase.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["last_used_at"] = "2020-01-01T00:00:00+00:00"
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

    path_out = runner.invoke(app, ["worktree", "path", "usagecase", "--json"])
    assert path_out.exit_code == 0, path_out.stdout
    path_payload = json.loads(path_out.stdout)
    assert path_payload["name"] == "usagecase"

    show_after_path = runner.invoke(app, ["worktree", "show", "usagecase", "--json"])
    assert show_after_path.exit_code == 0, show_after_path.stdout
    assert json.loads(show_after_path.stdout)["last_used_at"] != "2020-01-01T00:00:00+00:00"

    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["last_used_at"] = "2020-01-01T00:00:00+00:00"
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

    exec_out = runner.invoke(
        app,
        ["worktree", "exec", "usagecase", "--json", "--", sys.executable, "-c", "print('touched')"])
    assert exec_out.exit_code == 0, exec_out.stdout
    assert "touched" in json.loads(exec_out.stdout)["stdout"]

    show_after_exec = runner.invoke(app, ["worktree", "show", "usagecase", "--json"])
    assert show_after_exec.exit_code == 0, show_after_exec.stdout
    assert json.loads(show_after_exec.stdout)["last_used_at"] != "2020-01-01T00:00:00+00:00"


def test_worktree_exec_runs_command_in_target_worktree_and_returns_json(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-exec"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-exec"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "execme", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    exec_out = runner.invoke(
        app,
        [
            "worktree",
            "exec",
            "execme",
            "--json",
            "--",
            sys.executable,
            "-c",
            "import os; print(os.getcwd()); print(os.environ.get('AIT_WORKTREE_NAME')); print(os.environ.get('AIT_WORKTREE_LINE')); print(os.environ.get('PYTHONPATH'))",
        ])
    assert exec_out.exit_code == 0, exec_out.stdout
    payload = json.loads(exec_out.stdout)
    lines = payload["stdout"].splitlines()
    assert payload["path"] == str(worktree_path)
    assert payload["returncode"] == 0
    assert lines[0] == str(worktree_path)
    assert lines[1] == "execme"
    assert lines[2] == "main"
    assert lines[3].split(os.pathsep)[0] == str(worktree_path / "src")


def test_worktree_exec_propagates_subprocess_exit_code(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-exec-fail"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-exec-fail"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "failme", "--json"])
    assert add_out.exit_code == 0, add_out.stdout

    exec_out = runner.invoke(
        app,
        ["worktree", "exec", "failme", "--", sys.executable, "-c", "import sys; print('boom'); sys.exit(7)"])
    assert exec_out.exit_code == 7
    output = exec_out.output or exec_out.stdout
    assert "boom" in output


def test_worktree_add_links_shared_venv_when_present(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-venv"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    (repo / ".venv" / "bin").mkdir(parents=True)
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-venv"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "venvcase", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    payload = json.loads(add_out.stdout)
    worktree_path = Path(payload["path"])
    worktree_venv = worktree_path / ".venv"
    assert payload["venv_path"] == str(worktree_venv)
    assert worktree_venv.is_symlink()
    assert worktree_venv.resolve() == (repo / ".venv").resolve()


def test_worktree_show_repairs_shared_venv_link_for_existing_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-venv-repair"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-venv-repair"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "repairme", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])
    worktree_venv = worktree_path / ".venv"
    assert not worktree_venv.exists()

    (repo / ".venv" / "bin").mkdir(parents=True)
    show_out = runner.invoke(app, ["worktree", "show", "repairme", "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    payload = json.loads(show_out.stdout)
    assert payload["venv_path"] == str(worktree_venv)
    assert worktree_venv.is_symlink()
    assert worktree_venv.resolve() == (repo / ".venv").resolve()


def test_worktree_remove_cleans_managed_alias(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-remove-managed-alias"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-remove-managed-alias"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    worktree_path = tmp_path / "ram-root" / "housekeeper-worktree-remove-managed-alias" / "aliascase"
    alias_path = repo / ".ait" / "worktree-links" / "aliascase"
    add_out = _invoke_internal_worktree_add(
        ["worktree", "add", "aliascase", "--path", str(worktree_path), "--json"]
    )
    assert add_out.exit_code == 0, add_out.stdout

    metadata_path = repo / ".ait" / "worktrees" / "aliascase.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["alias_path"] = str(alias_path)
    metadata["root_source"] = "linux_tmp"
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")
    alias_path.parent.mkdir(parents=True, exist_ok=True)
    alias_path.symlink_to(worktree_path, target_is_directory=True)

    show_out = runner.invoke(app, ["worktree", "show", "aliascase", "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["alias_path"] == str(alias_path)
    assert shown["open_path"] == str(alias_path)

    path_out = runner.invoke(app, ["worktree", "path", "aliascase", "--json"])
    assert path_out.exit_code == 0, path_out.stdout
    path_payload = json.loads(path_out.stdout)
    assert path_payload["path"] == str(worktree_path)
    assert path_payload["open_path"] == str(alias_path)
    assert path_payload["alias_path"] == str(alias_path)
    assert path_payload["cd_command"] == f"cd {shlex.quote(str(alias_path))}"

    remove_out = runner.invoke(
        app,
        ["worktree", "remove", "aliascase", "--delete-path", "--json"],
        catch_exceptions=False,
    )
    assert remove_out.exit_code == 0, remove_out.stdout
    removed = json.loads(remove_out.stdout)
    assert removed["alias_path"] == str(alias_path)
    assert not alias_path.exists()
    assert not alias_path.is_symlink()


def test_worktree_open_shell_command_bootstraps_checkout_runtime(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-shell-runtime"
    repo.mkdir()
    monkeypatch.chdir(repo)

    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "src" / "ait_native").mkdir(parents=True)
    (repo / "src" / "ait_native" / "__init__.py").write_text("", encoding="utf-8")
    (repo / "src" / "ait_native" / "probe.py").write_text("ORIGIN = 'checkout'\n", encoding="utf-8")

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-shell-runtime"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "shellcase", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    shell_out = runner.invoke(app, ["worktree", "open", "shellcase", "--shell"])
    assert shell_out.exit_code == 0, shell_out.stdout
    shell_command = (shell_out.output or shell_out.stdout).strip()
    python_code = "import ait_native.probe as probe; print(probe.ORIGIN); print(probe.__file__)"
    completed = subprocess.run(
        ["zsh", "-lc", f"{shell_command} && {shlex.quote(sys.executable)} -c {shlex.quote(python_code)}"],
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    )
    assert completed.returncode == 0, completed.stderr
    lines = completed.stdout.splitlines()
    assert lines[0] == "checkout"
    assert lines[1].startswith(str(worktree_path / "src" / "ait_native"))


def test_worktree_cleanup_candidates_reports_candidates_and_protected_rows(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-cleanup-candidates"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-cleanup-candidates"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    assert _invoke_internal_worktree_add(["worktree", "add", "protectedcase", "--json"]).exit_code == 0
    assert _invoke_internal_worktree_add(["worktree", "add", "helpercase", "--json"]).exit_code == 0

    helper_metadata_path = repo / ".ait" / "worktrees" / "helpercase.json"
    helper_metadata = json.loads(helper_metadata_path.read_text(encoding="utf-8"))
    helper_metadata["creation_kind"] = "bootstrap_helper"
    helper_metadata["cleanup_policy"] = "after_idle"
    helper_metadata["last_used_at"] = "2020-01-01T00:00:00+00:00"
    helper_metadata_path.write_text(json.dumps(helper_metadata), encoding="utf-8")

    cleanup_out = runner.invoke(
        app,
        ["worktree", "cleanup-candidates", "--older-than", "7d", "--include-protected", "--json"])
    assert cleanup_out.exit_code == 0, cleanup_out.stdout
    payload = json.loads(cleanup_out.stdout)
    assert payload["older_than"] == "7d"
    assert payload["candidate_count"] == 1
    assert payload["protected_count"] >= 1
    assert payload["stale_count"] == 0

    candidate = payload["candidates"][0]
    assert candidate["name"] == "helpercase"
    assert candidate["cleanup_class"] == "safe_cleanup_candidate"
    assert candidate["cleanup_candidate"] is True
    assert candidate["cleanup_policy"] == "after_idle"
    assert "idle for 7d" in candidate["cleanup_reason"]

    protected_rows = {row["name"]: row for row in payload["protected"]}
    assert protected_rows["protectedcase"]["cleanup_class"] == "protected"
    assert protected_rows["protectedcase"]["protected_reason"] == "cleanup policy manual_only"


def test_worktree_cleanup_candidates_do_not_protect_clean_completed_worktrees_for_active_sessions(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-cleanup-active-session"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-cleanup-active-session"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "sessioncase", "--json"])
    assert add_out.exit_code == 0, add_out.stdout

    metadata_path = repo / ".ait" / "worktrees" / "sessioncase.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["creation_kind"] = "task_auto_created"
    metadata["cleanup_policy"] = "after_remote_land"
    metadata["bound_task_id"] = "AITT-1234"
    metadata["bound_change_id"] = "AITC-5678"
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

    store_worktrees_module = import_module("ait.store_worktrees")
    monkeypatch.setattr(
        store_worktrees_module.local_control,
        "list_workflow_sessions",
        lambda _ctx, *, status=None: [
            {"session_id": "AITS-SESSIONCASE", "status": status or "active", "worktree_name": "sessioncase"}
        ],
    )
    monkeypatch.setattr(
        store_worktrees_module,
        "_workflow_statuses_for_worktree",
        lambda *_args, **_kwargs: ("completed", "landed"),
    )

    cleanup_out = runner.invoke(
        app,
        ["worktree", "cleanup-candidates", "--older-than", "7d", "--json"])
    assert cleanup_out.exit_code == 0, cleanup_out.stdout
    payload = json.loads(cleanup_out.stdout)
    assert payload["candidate_count"] == 1

    candidate = payload["candidates"][0]
    assert candidate["name"] == "sessioncase"
    assert candidate["cleanup_class"] == "safe_auto_remove"
    assert candidate["cleanup_candidate"] is True
    assert candidate["protected_reason"] is None
    assert candidate["binding_summary"]["active_session_count"] == 1
    assert candidate["binding_summary"]["active_session_ids"] == ["AITS-SESSIONCASE"]


def test_worktree_cleanup_forces_dirty_canceled_task_bound_worktrees(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-cleanup-canceled-task"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-cleanup-canceled-task"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "canceledcase", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    metadata_path = repo / ".ait" / "worktrees" / "canceledcase.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["cleanup_policy"] = "never"
    metadata["bound_task_id"] = "AITT-CANCELED"
    metadata["bound_change_id"] = "AITC-DRAFT"
    metadata_path.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    (worktree_path / "app.py").write_text("print('dirty canceled task')\n", encoding="utf-8")

    store_worktrees_module = import_module("ait.store_worktrees")
    monkeypatch.setattr(
        store_worktrees_module,
        "_workflow_statuses_for_worktree",
        lambda *_args, **_kwargs: ("canceled", "draft"),
    )

    candidate_out = runner.invoke(
        app,
        ["worktree", "cleanup-candidates", "--include-protected", "--json"],
        catch_exceptions=False,
    )
    assert candidate_out.exit_code == 0, candidate_out.stdout
    candidate_payload = json.loads(candidate_out.stdout)
    assert candidate_payload["candidate_count"] == 1
    assert candidate_payload["protected_count"] == 0
    candidate = candidate_payload["candidates"][0]
    assert candidate["name"] == "canceledcase"
    assert candidate["workspace_status"] == "dirty"
    assert candidate["cleanup_class"] == "safe_auto_remove"
    assert candidate["cleanup_policy"] == "never"
    assert candidate["cleanup_candidate"] is True
    assert candidate["protected_reason"] is None
    assert candidate["force_remove_dirty"] is True
    assert candidate["cleanup"]["force_remove_dirty"] is True
    assert candidate["binding_summary"]["task_status"] == "canceled"
    assert candidate["binding_summary"]["change_status"] == "draft"

    dry_run_out = runner.invoke(app, ["worktree", "cleanup", "--dry-run", "--json"], catch_exceptions=False)
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run = json.loads(dry_run_out.stdout)
    assert dry_run["planned_count"] == 1
    assert dry_run["planned_rows"][0]["name"] == "canceledcase"
    assert dry_run["planned_rows"][0]["force"] is True

    apply_out = runner.invoke(app, ["worktree", "cleanup", "--yes", "--json"], catch_exceptions=False)
    assert apply_out.exit_code == 0, apply_out.stdout
    applied = json.loads(apply_out.stdout)
    assert applied["removed_count"] == 1
    assert applied["removed_rows"][0]["name"] == "canceledcase"
    assert applied["removed_rows"][0]["workspace_status"] == "dirty"
    assert not metadata_path.exists()
    assert not worktree_path.exists()


def test_worktree_add_accepts_explicit_helper_kind_defaults(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-helper-kind"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-helper-kind"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "scratchcase", "--kind", "scratch", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    payload = json.loads(add_out.stdout)
    assert payload["creation_kind"] == "scratch"
    assert payload["cleanup_policy"] == "after_idle"


def test_worktree_cleanup_can_explicitly_remove_manual_only_candidates(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-cleanup-apply"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-cleanup-apply"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "manualcase", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    added = json.loads(add_out.stdout)
    worktree_path = Path(added["path"])

    metadata_path = repo / ".ait" / "worktrees" / "manualcase.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["last_used_at"] = "2020-01-01T00:00:00+00:00"
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

    blocked_out = runner.invoke(
        app,
        ["worktree", "cleanup-candidates", "--older-than", "7d", "--json"])
    assert blocked_out.exit_code == 0, blocked_out.stdout
    blocked = json.loads(blocked_out.stdout)
    assert blocked["candidate_count"] == 0
    assert blocked["protected_count"] == 1

    candidate_out = runner.invoke(
        app,
        [
            "worktree",
            "cleanup-candidates",
            "--older-than",
            "7d",
            "--allow-manual-only",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert candidate_out.exit_code == 0, candidate_out.stdout
    candidate_payload = json.loads(candidate_out.stdout)
    assert candidate_payload["candidate_count"] == 1
    assert candidate_payload["candidates"][0]["name"] == "manualcase"

    dry_run_out = runner.invoke(
        app,
        [
            "worktree",
            "cleanup",
            "--older-than",
            "7d",
            "--allow-manual-only",
            "--dry-run",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run = json.loads(dry_run_out.stdout)
    assert dry_run["planned_count"] == 1
    assert dry_run["planned_rows"][0]["name"] == "manualcase"

    apply_out = runner.invoke(
        app,
        [
            "worktree",
            "cleanup",
            "--older-than",
            "7d",
            "--allow-manual-only",
            "--yes",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert apply_out.exit_code == 0, apply_out.stdout
    applied = json.loads(apply_out.stdout)
    assert applied["removed_count"] == 1
    assert applied["removed_rows"][0]["name"] == "manualcase"
    assert not metadata_path.exists()
    assert not worktree_path.exists()


def test_worktree_doctor_reports_missing_and_detached_registrations(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-doctor"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-doctor"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    missing_out = _invoke_internal_worktree_add(["worktree", "add", "missingcase", "--json"])
    assert missing_out.exit_code == 0, missing_out.stdout
    missing_path = Path(json.loads(missing_out.stdout)["path"])

    detached_out = _invoke_internal_worktree_add(["worktree", "add", "detachedcase", "--json"])
    assert detached_out.exit_code == 0, detached_out.stdout
    detached_path = Path(json.loads(detached_out.stdout)["path"])

    shutil.rmtree(missing_path)
    (detached_path / ".ait-worktree.json").unlink()

    doctor_out = runner.invoke(app, ["worktree", "doctor", "--json"])
    assert doctor_out.exit_code == 0, doctor_out.stdout
    doctor = json.loads(doctor_out.stdout)
    assert doctor["healthy"] is False
    assert doctor["missing_count"] == 1
    assert doctor["detached_count"] == 1
    assert doctor["stale_count"] == 2
    statuses = {row["name"]: row["workspace_status"] for row in doctor["stale_rows"]}
    assert statuses == {"missingcase": "missing", "detachedcase": "detached"}


def test_worktree_prune_stale_unregisters_only_stale_entries(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-prune"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-prune"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    live_out = _invoke_internal_worktree_add(["worktree", "add", "live", "--json"])
    assert live_out.exit_code == 0, live_out.stdout
    live_path = Path(json.loads(live_out.stdout)["path"])

    missing_out = _invoke_internal_worktree_add(["worktree", "add", "missingcase", "--json"])
    assert missing_out.exit_code == 0, missing_out.stdout
    missing_path = Path(json.loads(missing_out.stdout)["path"])

    detached_out = _invoke_internal_worktree_add(["worktree", "add", "detachedcase", "--json"])
    assert detached_out.exit_code == 0, detached_out.stdout
    detached_path = Path(json.loads(detached_out.stdout)["path"])

    shutil.rmtree(missing_path)
    (detached_path / ".ait-worktree.json").unlink()

    dry_run_out = runner.invoke(app, ["worktree", "prune-stale", "--dry-run", "--json"])
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run = json.loads(dry_run_out.stdout)
    assert dry_run["dry_run"] is True
    assert dry_run["pruned_count"] == 2
    assert dry_run["remaining_count"] == 3
    assert live_path.exists()
    assert detached_path.exists()

    prune_out = runner.invoke(app, ["worktree", "prune-stale", "--json"])
    assert prune_out.exit_code == 0, prune_out.stdout
    pruned = json.loads(prune_out.stdout)
    assert pruned["dry_run"] is False
    assert pruned["pruned_count"] == 2
    assert pruned["remaining_count"] == 1
    assert detached_path.exists()

    list_out = runner.invoke(app, ["worktree", "list", "--json"])
    assert list_out.exit_code == 0, list_out.stdout
    rows = json.loads(list_out.stdout)
    assert [row["name"] for row in rows] == ["live"]


def test_worktree_remove_all_stale_can_preview_and_prune_registry_entries(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-remove-all-stale"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-remove-all-stale"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    live_out = _invoke_internal_worktree_add(["worktree", "add", "live", "--json"])
    assert live_out.exit_code == 0, live_out.stdout

    stale_out = _invoke_internal_worktree_add(["worktree", "add", "stale", "--json"])
    assert stale_out.exit_code == 0, stale_out.stdout
    stale_path = Path(json.loads(stale_out.stdout)["path"])
    shutil.rmtree(stale_path)

    dry_run_out = runner.invoke(app, ["worktree", "remove", "--all-stale", "--dry-run", "--json"])
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run = json.loads(dry_run_out.stdout)
    assert dry_run["dry_run"] is True
    assert dry_run["pruned_count"] == 1
    assert dry_run["pruned_rows"][0]["name"] == "stale"

    remove_out = runner.invoke(app, ["worktree", "remove", "--all-stale", "--json"])
    assert remove_out.exit_code == 0, remove_out.stdout
    removed = json.loads(remove_out.stdout)
    assert removed["dry_run"] is False
    assert removed["pruned_count"] == 1
    assert removed["pruned_rows"][0]["name"] == "stale"

    list_out = runner.invoke(app, ["worktree", "list", "--json"])
    assert list_out.exit_code == 0, list_out.stdout
    rows = json.loads(list_out.stdout)
    assert [row["name"] for row in rows] == ["live"]


def test_worktree_remove_all_stale_rejects_delete_path_and_force(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-remove-all-stale-guards"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-remove-all-stale-guards"]).exit_code == 0

    delete_path_out = runner.invoke(app, ["worktree", "remove", "--all-stale", "--delete-path"])
    assert delete_path_out.exit_code != 0
    assert "--delete-path cannot be combined with --all-stale" in (delete_path_out.output or delete_path_out.stdout or "")

    force_out = runner.invoke(app, ["worktree", "remove", "--all-stale", "--force"])
    assert force_out.exit_code != 0
    assert "--force cannot be combined with --all-stale" in (force_out.output or force_out.stdout or "")


def test_worktree_sync_all_updates_live_worktrees_and_skips_stale_entries(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-sync-all"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-sync-all"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    assert runner.invoke(app, ["line", "create", "feature/live"]).exit_code == 0
    live_out = _invoke_internal_worktree_add(["worktree", "add", "live", "--line", "feature/live", "--json"])
    assert live_out.exit_code == 0, live_out.stdout
    live_path = Path(json.loads(live_out.stdout)["path"])

    assert runner.invoke(app, ["line", "create", "feature/stale"]).exit_code == 0
    stale_out = _invoke_internal_worktree_add(["worktree", "add", "stale", "--line", "feature/stale", "--json"])
    assert stale_out.exit_code == 0, stale_out.stdout
    stale_path = Path(json.loads(stale_out.stdout)["path"])

    assert runner.invoke(app, ["line", "switch", "feature/live", "--restore"]).exit_code == 0
    app_file.write_text("print('feature live updated')\n", encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "feature live update"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "main", "--restore"]).exit_code == 0

    shutil.rmtree(stale_path)

    sync_out = runner.invoke(app, ["worktree", "sync", "--all", "--json"])
    assert sync_out.exit_code == 0, sync_out.stdout
    payload = json.loads(sync_out.stdout)
    assert payload["ok"] is True
    assert payload["synced_count"] == 1
    assert payload["skipped_count"] == 1
    assert payload["error_count"] == 0
    assert payload["synced_rows"][0]["name"] == "live"
    assert payload["skipped_rows"][0]["name"] == "stale"
    assert payload["skipped_rows"][0]["workspace_status"] == "missing"
    assert (live_path / "app.py").read_text(encoding="utf-8") == "print('feature live updated')\n"


def test_worktree_sync_all_reports_dirty_errors_but_continues_other_live_worktrees(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-sync-all-errors"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-sync-all-errors"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0

    assert runner.invoke(app, ["line", "create", "feature/clean"]).exit_code == 0
    clean_out = _invoke_internal_worktree_add(["worktree", "add", "clean", "--line", "feature/clean", "--json"])
    assert clean_out.exit_code == 0, clean_out.stdout
    clean_path = Path(json.loads(clean_out.stdout)["path"])

    assert runner.invoke(app, ["line", "create", "feature/dirty"]).exit_code == 0
    dirty_out = _invoke_internal_worktree_add(["worktree", "add", "dirty", "--line", "feature/dirty", "--json"])
    assert dirty_out.exit_code == 0, dirty_out.stdout
    dirty_path = Path(json.loads(dirty_out.stdout)["path"])

    assert runner.invoke(app, ["line", "switch", "feature/clean", "--restore"]).exit_code == 0
    app_file.write_text("print('clean line updated')\n", encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "clean line update"]).exit_code == 0

    assert runner.invoke(app, ["line", "switch", "feature/dirty", "--restore"]).exit_code == 0
    app_file.write_text("print('dirty line head updated')\n", encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "dirty line update"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "main", "--restore"]).exit_code == 0

    (dirty_path / "app.py").write_text("print('local dirty conflict')\n", encoding="utf-8")

    sync_out = runner.invoke(app, ["worktree", "sync", "--all", "--json"])
    assert sync_out.exit_code == 2
    payload = json.loads(sync_out.stdout)
    assert payload["ok"] is False
    assert payload["synced_count"] == 1
    assert payload["error_count"] == 1
    assert payload["synced_rows"][0]["name"] == "clean"
    assert payload["error_rows"][0]["name"] == "dirty"
    assert payload["error_rows"][0]["workspace_status"] == "dirty"
    assert "unsaved changes" in payload["error_rows"][0]["error"]
    assert (clean_path / "app.py").read_text(encoding="utf-8") == "print('clean line updated')\n"
    assert (dirty_path / "app.py").read_text(encoding="utf-8") == "print('local dirty conflict')\n"


def test_worktree_rebase_dry_run_reports_plan_and_retarget_metadata(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-rebase-dry-run"
    repo.mkdir()
    readme = repo / "README.md"
    readme.write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-rebase-dry-run"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    seed = json.loads(seed_out.stdout)
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"]).exit_code == 0
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Dry run rebase",
            "--intent",
            "preview retarget state before applying",
            "--base-line",
            "main",
            "--json",
        ])
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]

    readme.write_text("base\nmain advanced\n", encoding="utf-8")
    repo_ctx = RepoContext.discover(repo)
    local_create_snapshot(repo_ctx, "main advance")

    show_out = runner.invoke(app, ["worktree", "show", worktree["name"], "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["fork_snapshot_id"] == seed["snapshot_id"]
    assert shown["forked_from_line"] == "main"
    assert shown["target_base_line"] == "main"
    assert shown["needs_retarget"] is True
    assert shown["base_behind_count"] == 1
    assert shown["rebase_state"] == "idle"

    dry_run_out = runner.invoke(
        app,
        ["worktree", "rebase", worktree["name"], "--onto", "main", "--dry-run", "--json"],
        catch_exceptions=False,
    )
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run = json.loads(dry_run_out.stdout)
    assert dry_run["rebase"]["dry_run"] is True
    assert dry_run["rebase"]["old_base_snapshot_id"] == seed["snapshot_id"]
    assert dry_run["rebase"]["new_base_snapshot_id"] == json.loads(
        runner.invoke(app, ["line", "show", "main", "--json"]).stdout
    )["head_snapshot_id"]
    assert dry_run["rebase"]["would_fast_forward"] is True
    assert dry_run["rebase"]["conflict_count"] == 0


def test_worktree_rebase_applies_single_sided_changes_and_updates_fork_point(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-rebase-apply"
    repo.mkdir()
    app_file = repo / "app.py"
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-rebase-apply"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    seed = json.loads(seed_out.stdout)
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"]).exit_code == 0
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Apply rebase",
            "--intent",
            "carry feature-only work onto a newer base",
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
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature side", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout

    monkeypatch.chdir(repo)
    app_file.write_text("print('main advanced')\n", encoding="utf-8")
    repo_ctx = RepoContext.discover(repo)
    main_advance = local_create_snapshot(repo_ctx, "main advance")

    rebase_out = runner.invoke(app, ["worktree", "rebase", worktree["name"], "--onto", "main", "--json"])
    assert rebase_out.exit_code == 0, rebase_out.stdout
    rebased = json.loads(rebase_out.stdout)
    assert rebased["rebase"]["status"] == "applied"
    assert rebased["fork_snapshot_id"] == main_advance["snapshot_id"]
    assert rebased["target_base_line"] == "main"
    assert rebased["needs_retarget"] is False

    assert (worktree_path / "app.py").read_text(encoding="utf-8") == "print('main advanced')\n"
    assert (worktree_path / "feature.txt").read_text(encoding="utf-8") == "feature only\n"


def test_worktree_rebase_conflict_can_continue_or_abort(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-rebase-conflict"
    repo.mkdir()
    app_file = repo / "app.py"
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-rebase-conflict"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"]).exit_code == 0
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Conflict rebase",
            "--intent",
            "pause in-place for resolution and support continue or abort",
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
    (worktree_path / "app.py").write_text("print('feature side')\n", encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "feature conflict", "--json"]).exit_code == 0

    monkeypatch.chdir(repo)
    app_file.write_text("print('main side')\n", encoding="utf-8")
    repo_ctx = RepoContext.discover(repo)
    main_advance = local_create_snapshot(repo_ctx, "main conflict")

    conflict_out = runner.invoke(app, ["worktree", "rebase", worktree["name"], "--onto", "main", "--json"])
    assert conflict_out.exit_code == 0, conflict_out.stdout
    conflicted = json.loads(conflict_out.stdout)
    assert conflicted["rebase"]["status"] == "conflicted"
    assert conflicted["rebase_state"] == "conflicted"
    assert conflicted["rebase_conflict_paths"] == ["app.py"]
    conflict_text = (worktree_path / "app.py").read_text(encoding="utf-8")
    assert "<<<<<<< feature/" in conflict_text
    assert ">>>>>>> main" in conflict_text

    (worktree_path / "app.py").write_text("print('resolved')\n", encoding="utf-8")
    continue_out = runner.invoke(app, ["worktree", "rebase", worktree["name"], "--continue", "--json"])
    assert continue_out.exit_code == 0, continue_out.stdout
    continued = json.loads(continue_out.stdout)
    assert continued["rebase"]["status"] == "continued"
    assert continued["fork_snapshot_id"] == main_advance["snapshot_id"]
    assert continued["rebase_state"] == "idle"
    assert (worktree_path / "app.py").read_text(encoding="utf-8") == "print('resolved')\n"

    monkeypatch.chdir(worktree_path)
    (worktree_path / "app.py").write_text("print('feature side two')\n", encoding="utf-8")
    assert runner.invoke(app, ["snapshot", "create", "--message", "feature conflict two", "--json"]).exit_code == 0

    monkeypatch.chdir(repo)
    app_file.write_text("print('main side two')\n", encoding="utf-8")
    local_create_snapshot(repo_ctx, "main conflict two")

    second_conflict_out = runner.invoke(app, ["worktree", "rebase", worktree["name"], "--onto", "main", "--json"])
    assert second_conflict_out.exit_code == 0, second_conflict_out.stdout
    assert json.loads(second_conflict_out.stdout)["rebase"]["status"] == "conflicted"

    abort_out = runner.invoke(app, ["worktree", "rebase", worktree["name"], "--abort", "--json"])
    assert abort_out.exit_code == 0, abort_out.stdout
    aborted = json.loads(abort_out.stdout)
    assert aborted["rebase"]["status"] == "aborted"
    assert aborted["rebase_state"] == "idle"
    assert (worktree_path / "app.py").read_text(encoding="utf-8") == "print('feature side two')\n"


def test_worktree_dirty_is_isolated_from_primary_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-dirty"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-dirty"]).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert snap_out.exit_code == 0, snap_out.stdout

    assert runner.invoke(app, ["line", "create", "feature/isolation"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "isolation", "--line", "feature/isolation", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    monkeypatch.chdir(worktree_path)
    (worktree_path / "app.py").write_text("print('changed in worktree')\n", encoding="utf-8")
    dirty_out = runner.invoke(app, ["status", "--json"])
    assert dirty_out.exit_code == 0, dirty_out.stdout
    dirty_status = json.loads(dirty_out.stdout)
    assert dirty_status["workspace_dirty"] is True
    assert "app.py" in dirty_status["workspace_changed_paths_sample"]

    monkeypatch.chdir(repo)
    clean_out = runner.invoke(app, ["status", "--json"])
    assert clean_out.exit_code == 0, clean_out.stdout
    clean_status = json.loads(clean_out.stdout)
    assert clean_status["workspace_dirty"] is False
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"


def test_stateful_workspace_commands_fail_while_same_workspace_lock_is_held(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-command-lock"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-command-lock"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
    ctx = cli_module.RepoContext.discover(repo)

    with cli_module._workspace_command_lock(ctx, "test holder"):
        for argv, attempted_command in (
            (["snapshot", "create", "--message", "next"], "snapshot create"),
            (["line", "switch", "main"], "line switch"),
            (["workspace", "restore", "--snapshot", seed_snapshot_id, "--force"], "workspace restore"),
        ):
            blocked = runner.invoke(app, argv, catch_exceptions=False)
            assert blocked.exit_code != 0
            output = blocked.output or blocked.stdout or blocked.stderr or ""
            assert "Workspace command lock is busy" in output
            assert "test holder" in output
            assert attempted_command in output
            assert repo.name in output


def test_stateful_workspace_lock_is_scoped_per_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-command-lock-worktree"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-command-lock-worktree"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    assert runner.invoke(app, ["line", "create", "feature/wt"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "lockscope", "--line", "feature/wt", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    main_ctx = cli_module.RepoContext.discover(repo)
    worktree_ctx = cli_module.RepoContext.discover(worktree_path)
    assert cli_module._workspace_command_lock_path(main_ctx) != cli_module._workspace_command_lock_path(worktree_ctx)

    with cli_module._workspace_command_lock(main_ctx, "main workspace holder"):
        monkeypatch.chdir(worktree_path)
        snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "worktree seed", "--json"])
        assert snapshot_out.exit_code == 0, snapshot_out.stdout
        payload = json.loads(snapshot_out.stdout)
        assert payload["line_name"] == "feature/wt"


def test_worktree_list_and_remove_update_registry(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-registry"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-registry"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/listing"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "listing", "--line", "feature/listing", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    list_out = runner.invoke(app, ["worktree", "list", "--json"])
    assert list_out.exit_code == 0, list_out.stdout
    rows = json.loads(list_out.stdout)
    assert rows[0]["name"] == "listing"
    assert rows[0]["current_line"] == "feature/listing"
    assert rows[0]["exists"] is True
    assert rows[0]["workspace_status"] == "clean"
    assert rows[0]["changed_count"] == 0

    remove_out = runner.invoke(app, ["worktree", "remove", "listing", "--delete-path", "--json"])
    assert remove_out.exit_code == 0, remove_out.stdout
    removed = json.loads(remove_out.stdout)
    assert removed["name"] == "listing"
    assert removed["deleted_path"] is True
    assert not worktree_path.exists()

    final_list_out = runner.invoke(app, ["worktree", "list", "--json"])
    assert final_list_out.exit_code == 0, final_list_out.stdout
    assert json.loads(final_list_out.stdout) == []


def test_worktree_list_can_use_cached_status_and_refresh_on_demand(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-list-cache"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-list-cache"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0

    add_out = _invoke_internal_worktree_add(["worktree", "add", "listing", "--json"])
    assert add_out.exit_code == 0, add_out.stdout

    metadata_path = repo / ".ait" / "worktrees" / "listing.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata.pop("workspace_status_cache", None)
    metadata_path.write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    cached_miss_out = runner.invoke(app, ["worktree", "list", "--json"])
    assert cached_miss_out.exit_code == 0, cached_miss_out.stdout
    cached_miss_rows = json.loads(cached_miss_out.stdout)
    assert cached_miss_rows[0]["workspace_status"] == "unknown"
    assert cached_miss_rows[0]["status_source"] == "unverified"

    refresh_out = runner.invoke(app, ["worktree", "list", "--refresh", "--json"])
    assert refresh_out.exit_code == 0, refresh_out.stdout
    refreshed_rows = json.loads(refresh_out.stdout)
    assert refreshed_rows[0]["workspace_status"] == "clean"
    assert refreshed_rows[0]["status_source"] == "verified"
    assert refreshed_rows[0]["changed_count"] == 0

    cached_out = runner.invoke(app, ["worktree", "list", "--json"])
    assert cached_out.exit_code == 0, cached_out.stdout
    cached_rows = json.loads(cached_out.stdout)
    assert cached_rows[0]["workspace_status"] == "clean"
    assert cached_rows[0]["status_source"] == "cached"
    assert cached_rows[0]["changed_count"] == 0


def test_worktree_remove_rejects_dirty_workspace_without_force(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-remove-guard"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-remove-guard"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/remove-guard"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "guarded", "--line", "feature/remove-guard", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree_path = Path(json.loads(add_out.stdout)["path"])

    monkeypatch.chdir(worktree_path)
    (worktree_path / "app.py").write_text("print('dirty remove')\n", encoding="utf-8")

    monkeypatch.chdir(repo)
    blocked_out = runner.invoke(app, ["worktree", "remove", "guarded", "--delete-path"])
    assert blocked_out.exit_code != 0
    blocked_text = blocked_out.output or blocked_out.stdout or blocked_out.stderr or ""
    assert "unsaved changes" in blocked_text
    assert "--force" in blocked_text
    assert worktree_path.exists()

    remove_out = runner.invoke(app, ["worktree", "remove", "guarded", "--delete-path", "--force", "--json"])
    assert remove_out.exit_code == 0, remove_out.stdout
    removed = json.loads(remove_out.stdout)
    assert removed["workspace_status"] == "dirty"
    assert not worktree_path.exists()


def test_worktree_remove_can_preview_and_remove_multiple_targets(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-remove-batch"
    repo.mkdir()
    readme = repo / "README.md"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-worktree-remove-batch"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/remove-one"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/remove-two"]).exit_code == 0

    first_out = _invoke_internal_worktree_add(["worktree", "add", "removeone", "--line", "feature/remove-one", "--json"])
    assert first_out.exit_code == 0, first_out.stdout
    first_path = Path(json.loads(first_out.stdout)["path"])

    second_out = _invoke_internal_worktree_add(["worktree", "add", "removetwo", "--line", "feature/remove-two", "--json"])
    assert second_out.exit_code == 0, second_out.stdout
    second_path = Path(json.loads(second_out.stdout)["path"])

    dry_run_out = runner.invoke(
        app,
        ["worktree", "remove", "removeone", "removetwo", "--delete-path", "--dry-run", "--json"])
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run = json.loads(dry_run_out.stdout)
    assert dry_run["dry_run"] is True
    assert dry_run["planned_count"] == 2
    assert [row["name"] for row in dry_run["planned_rows"]] == ["removeone", "removetwo"]
    assert first_path.exists()
    assert second_path.exists()

    remove_out = runner.invoke(
        app,
        ["worktree", "remove", "removeone", "removetwo", "--delete-path", "--json"],
        catch_exceptions=False,
    )
    assert remove_out.exit_code == 0, remove_out.stdout
    removed = json.loads(remove_out.stdout)
    assert removed["removed_count"] == 2
    assert [row["name"] for row in removed["removed_rows"]] == ["removeone", "removetwo"]
    assert not first_path.exists()
    assert not second_path.exists()


def test_workspace_restore_reports_dirty_workspace_and_supports_force(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-restore-dirty"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-restore-dirty"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout

    app_file.write_text("print('unsaved change')\n", encoding="utf-8")

    blocked_out = runner.invoke(app, ["workspace", "restore"])
    assert blocked_out.exit_code != 0
    blocked_text = blocked_out.output or blocked_out.stdout or blocked_out.stderr or ""
    assert "unsaved changes" in blocked_text

    dry_run_out = runner.invoke(app, ["workspace", "restore", "--dry-run", "--json"])
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run = json.loads(dry_run_out.stdout)
    assert dry_run["workspace_dirty"] is True
    assert dry_run["would_overwrite_workspace_changes"] is True
    assert dry_run["applied"] is False
    assert "app.py" in dry_run["dirty_workspace"]["modified_paths"]

    force_out = runner.invoke(app, ["workspace", "restore", "--force", "--json"])
    assert force_out.exit_code == 0, force_out.stdout
    forced = json.loads(force_out.stdout)
    assert forced["applied"] is True
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"


def test_line_switch_restore_materializes_target_line_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-switch-restore"
    repo.mkdir()
    app_file = repo / "app.py"
    notes = repo / "notes.txt"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-line-switch-restore"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/restore"]).exit_code == 0
    assert runner.invoke(app, ["line", "switch", "feature/restore"]).exit_code == 0
    app_file.write_text("print('feature')\n", encoding="utf-8")
    notes.write_text("feature note\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout

    switch_out = runner.invoke(app, ["line", "switch", "main", "--restore", "--json"])
    assert switch_out.exit_code == 0, switch_out.stdout
    payload = json.loads(switch_out.stdout)
    assert payload["current_line_before"] == "feature/restore"
    assert payload["current_line"] == "main"
    assert payload["line_name"] == "main"
    assert payload["line_head_snapshot_id"] == main_snapshot["snapshot_id"]
    assert payload["applied"] is True
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"
    assert not notes.exists()

    status_out = runner.invoke(app, ["status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["current_line"] == "main"
    assert status["head_snapshot_id"] == main_snapshot["snapshot_id"]


def test_line_create_switch_updates_current_line_without_restoring_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-create-switch"
    repo.mkdir()
    app_file = repo / "app.py"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-line-create-switch"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    app_file.write_text("print('unsaved change')\n", encoding="utf-8")

    create_out = runner.invoke(app, ["line", "create", "feature/switch", "--switch", "--json"])
    assert create_out.exit_code == 0, create_out.stdout
    payload = json.loads(create_out.stdout)
    assert payload["line_name"] == "feature/switch"
    assert payload["head_snapshot_id"] == main_snapshot["snapshot_id"]
    assert payload["current_line_before"] == "main"
    assert payload["current_line"] == "feature/switch"
    assert payload["switched"] is True
    assert payload["restored"] is False
    assert app_file.read_text(encoding="utf-8") == "print('unsaved change')\n"

    status_out = runner.invoke(app, ["status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["current_line"] == "feature/switch"
    assert status["workspace_dirty"] is True


def test_line_create_switch_restore_materializes_created_line_head(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-create-switch-restore"
    repo.mkdir()
    app_file = repo / "app.py"
    notes = repo / "notes.txt"
    monkeypatch.chdir(repo)

    app_file.write_text("print('base')\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-line-create-switch-restore"]).exit_code == 0
    main_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    main_snapshot = json.loads(main_out.stdout)

    assert runner.invoke(app, ["line", "create", "feature/source", "--switch"]).exit_code == 0
    app_file.write_text("print('feature')\n", encoding="utf-8")
    notes.write_text("feature note\n", encoding="utf-8")
    feature_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
    assert feature_out.exit_code == 0, feature_out.stdout
    feature_snapshot = json.loads(feature_out.stdout)

    create_out = runner.invoke(
        app,
        [
            "line",
            "create",
            "feature/from-main",
            "--from-snapshot",
            main_snapshot["snapshot_id"],
            "--switch",
            "--restore",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert create_out.exit_code == 0, create_out.stdout
    payload = json.loads(create_out.stdout)
    assert payload["current_line_before"] == "feature/source"
    assert payload["current_line"] == "feature/from-main"
    assert payload["line_name"] == "feature/from-main"
    assert payload["line_head_snapshot_id"] == main_snapshot["snapshot_id"]
    assert payload["applied"] is True
    assert payload["switched"] is True
    assert payload["restored"] is True
    assert payload["created_line"]["line_name"] == "feature/from-main"
    assert payload["created_line"]["head_snapshot_id"] == main_snapshot["snapshot_id"]
    assert app_file.read_text(encoding="utf-8") == "print('base')\n"
    assert not notes.exists()

    status_out = runner.invoke(app, ["status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["current_line"] == "feature/from-main"
    assert status["head_snapshot_id"] == main_snapshot["snapshot_id"]
    assert status["workspace_dirty"] is False

    line_out = runner.invoke(app, ["line", "show", "feature/source", "--json"])
    assert line_out.exit_code == 0, line_out.stdout
    source_line = json.loads(line_out.stdout)
    assert source_line["head_snapshot_id"] == feature_snapshot["snapshot_id"]


def test_line_create_restore_requires_switch(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-create-restore-validation"
    repo.mkdir()
    monkeypatch.chdir(repo)
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    assert runner.invoke(app, ["init", "--name", "housekeeper-line-create-restore-validation"]).exit_code == 0

    out = runner.invoke(app, ["line", "create", "feature/invalid", "--restore"])
    assert out.exit_code != 0
    text = out.output or out.stdout or out.stderr or ""
    assert "--restore requires --switch" in text


def test_line_switch_restore_preserves_files_ignored_by_target_aitignore(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-switch-restore-aitignore"
    repo.mkdir()
    readme = repo / "README.md"
    ignore_file = repo / ".aitignore"
    env_dir = repo / "local-secrets"
    env_path = env_dir / ".env"
    monkeypatch.chdir(repo)

    readme.write_text("base\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-line-switch-restore-aitignore"]).exit_code == 0
    main_seed_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_seed_out.exit_code == 0, main_seed_out.stdout

    assert runner.invoke(app, ["line", "create", "feature/restore-ignore"]).exit_code == 0

    ignore_file.write_text("local-secrets/.env\n", encoding="utf-8")
    main_ignore_out = runner.invoke(app, ["snapshot", "create", "--message", "main adds .aitignore", "--json"])
    assert main_ignore_out.exit_code == 0, main_ignore_out.stdout
    main_snapshot = json.loads(main_ignore_out.stdout)

    switch_feature_out = runner.invoke(app, ["line", "switch", "feature/restore-ignore", "--restore", "--json"])
    assert switch_feature_out.exit_code == 0, switch_feature_out.stdout
    assert not ignore_file.exists()

    env_dir.mkdir()
    env_path.write_text("BOT_TOKEN=secret\n", encoding="utf-8")

    switch_main_out = runner.invoke(app, ["line", "switch", "main", "--restore", "--json"])
    assert switch_main_out.exit_code == 0, switch_main_out.stdout
    payload = json.loads(switch_main_out.stdout)
    assert payload["current_line_before"] == "feature/restore-ignore"
    assert payload["current_line"] == "main"
    assert payload["line_head_snapshot_id"] == main_snapshot["snapshot_id"]
    assert payload["dirty_workspace"]["clean"] is True
    assert env_path.exists()
    assert env_path.read_text(encoding="utf-8") == "BOT_TOKEN=secret\n"
    assert ignore_file.read_text(encoding="utf-8") == "local-secrets/.env\n"

    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["changed_paths"] == []


def test_server_rejects_line_update_when_snapshot_belongs_to_other_repo(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "README.md").write_text("hello\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-cross-repo-line") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        snapshot = json.loads(snap_out.stdout)
        push_out = runner.invoke(app, ["push", "--json"])
        assert push_out.exit_code == 0, push_out.stdout

        headers = {"Accept": "application/json", "Content-Type": "application/json"}
        create_req = urllib.request.Request(
            f"{base_url}/v1/native/repositories",
            data=json.dumps({"repo_name": "other-repo", "default_line": "main"}).encode("utf-8"),
            headers=headers,
            method="POST",
        )
        with urllib.request.urlopen(create_req, timeout=5) as resp:
            assert resp.status == 200

        line_req = urllib.request.Request(
            f"{base_url}/v1/native/repositories/other-repo/lines/main",
            data=json.dumps({"head_snapshot_id": snapshot["snapshot_id"]}).encode("utf-8"),
            headers=headers,
            method="PUT",
        )
        try:
            urllib.request.urlopen(line_req, timeout=5)
            raise AssertionError("expected foreign snapshot line update to fail")
        except urllib.error.HTTPError as exc:
            assert exc.code == 404
            payload = exc.read().decode("utf-8", errors="replace")
            assert snapshot["snapshot_id"] in payload
            assert "other-repo" in payload
            assert "housekeeper" in payload


def test_task_start_auto_worktree_prints_switch_hint_and_dirty_workspace_warning(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-auto-worktree-warning"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert snapshot_out.exit_code == 0, snapshot_out.stdout
    binding_out = runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    )
    assert binding_out.exit_code == 0, binding_out.stdout

    (repo / "notes.txt").write_text("dirty note\n", encoding="utf-8")

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap isolated workflow with warning",
            "--intent",
            "print the worktree switch warning and keep existing edits in the source workspace",
            "--base-line",
            "main",
            "--risk",
            "medium",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    rendered_output = " ".join(start_out.stdout.split())
    assert "Your current shell has not been switched automatically." in start_out.stdout
    assert "Continue in the task worktree with:" in start_out.stdout
    assert "task worktree:" in start_out.stdout
    assert "Existing workspace changes were not copied into the new task worktree." in start_out.stdout
    assert "Reconcile or discard them in the source workspace before continuing formal task work." in rendered_output
    assert "notes.txt" in start_out.stdout

    worktree_list_out = runner.invoke(app, ["worktree", "list", "--json"])
    assert worktree_list_out.exit_code == 0, worktree_list_out.stdout
    worktrees = json.loads(worktree_list_out.stdout)
    first_worktree = next(row for row in worktrees if row.get("bound_task_id"))
    remove_out = runner.invoke(
        app,
        ["worktree", "remove", first_worktree["name"], "--delete-path", "--json"],
        catch_exceptions=False,
    )
    assert remove_out.exit_code == 0, remove_out.stdout

    start_json_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Bootstrap isolated workflow with warning json",
            "--intent",
            "record warning details in JSON output",
            "--base-line",
            "main",
            "--risk",
            "medium",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_json_out.exit_code == 0, start_json_out.stdout
    payload = json.loads(start_json_out.stdout)
    guidance = payload["worktree_guidance"]
    assert guidance["switch_required"] is True
    assert guidance["source_workspace_changed_count"] == 1
    assert guidance["source_workspace_changed_paths"] == ["notes.txt"]
    assert "Current workspace had 1 changed path(s): notes.txt" == guidance["source_workspace_summary"]
    assert "Existing workspace changes were not copied into the new task worktree." in guidance["dirty_source_warning"]
    assert "Reconcile or discard them in the source workspace before continuing formal task work." in guidance["dirty_source_warning"]


def test_workflow_land_local_moves_target_line_and_completes_task(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-local"
    repo.mkdir()
    readme = repo / "README.md"
    app_file = repo / "app.py"
    readme.write_text("base\n", encoding="utf-8")
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"]).exit_code == 0
    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    assert (
        runner.invoke(
            app,
            ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )
    main_snapshot = json.loads(main_snap_out.stdout)

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Local parser fix",
            "--intent",
            "land local-only work onto main",
            "--change-title",
            "Fix parser locally",
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
    bound_worktree_path = Path(started["worktree"]["path"])
    assert bound_worktree_path.is_dir()

    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(bound_worktree_path)
        (bound_worktree_path / "app.py").write_text("print('feature')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
        land_out = runner.invoke(app, ["workflow", "land-local", change_id, "--json"])
    assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
    feature_snapshot = json.loads(feature_snap_out.stdout)
    assert land_out.exit_code == 0, land_out.stdout
    landed = json.loads(land_out.stdout)
    assert landed["change_id"] == change_id
    assert landed["task_id"] == task_id
    assert landed["target_line"] == "main"
    assert landed["previous_target_head_snapshot_id"] == main_snapshot["snapshot_id"]
    assert landed["landed_snapshot_id"] == feature_snapshot["snapshot_id"]
    assert landed["change_status"] == "landed"
    assert landed["task_status"] == "completed"
    assert landed["workspace_action"] == "restored"
    assert landed["repo_root_restore"]["status"] == "restored"
    assert landed["bound_worktree_cleanup"]["status"] == "removed"
    assert landed["bound_worktree_cleanup"]["task_id"] == task_id
    assert not bound_worktree_path.exists()

    main_show_out = runner.invoke(app, ["line", "show", "main", "--json"])
    assert main_show_out.exit_code == 0, main_show_out.stdout
    assert json.loads(main_show_out.stdout)["head_snapshot_id"] == feature_snapshot["snapshot_id"]

    change_show_out = runner.invoke(app, ["change", "show", change_id, "--local", "--json"])
    assert change_show_out.exit_code == 0, change_show_out.stdout
    shown_change = json.loads(change_show_out.stdout)
    assert shown_change["status"] == "landed"
    assert shown_change["target_line"] == "main"
    assert shown_change["landed_snapshot_id"] == feature_snapshot["snapshot_id"]
    assert shown_change["landed_at"]

    task_show_out = runner.invoke(app, ["task", "show", task_id, "--local", "--json"])
    assert task_show_out.exit_code == 0, task_show_out.stdout
    assert json.loads(task_show_out.stdout)["status"] == "completed"

    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["current_line"] == "main"
    assert app_file.read_text(encoding="utf-8") == "print('feature')\n"


def test_workflow_land_local_reports_telegram_graph_notification_summary(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-local-telegram"
    repo.mkdir()
    app_file = repo / "app.py"
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    captured: dict[str, Any] = {}

    def fake_trigger(ctx, *, repo_name, event_type, entity_id):
        captured["repo_name"] = repo_name
        captured["event_type"] = event_type
        captured["entity_id"] = entity_id
        captured["repo_root"] = str(ctx.repo_root)
        return {
            "enabled": True,
            "checked": 1,
            "sent": 1,
            "errors": 0,
            "event_type": event_type,
            "entity_id": entity_id,
        }

    monkeypatch.setattr(watch_cli_module, "trigger_local_task_dag_telegram_notifications", fake_trigger)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Local parser fix",
            "--intent",
            "land local-only work onto main and report telegram graph notifications",
            "--change-title",
            "Fix parser locally",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    started = json.loads(start_out.stdout)
    change_id = started["change"]["change_id"]
    bound_worktree_path = Path(started["worktree"]["path"])

    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(bound_worktree_path)
        (bound_worktree_path / "app.py").write_text("print('feature')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"]).exit_code == 0
        land_out = runner.invoke(app, ["workflow", "land-local", change_id, "--json"])

    assert land_out.exit_code == 0, land_out.stdout
    landed = json.loads(land_out.stdout)
    assert landed["telegram_graph_notifications"] == {
        "enabled": True,
        "checked": 1,
        "sent": 1,
        "errors": 0,
        "event_type": "change.local_landed",
        "entity_id": change_id,
    }
    assert captured == {
        "repo_name": "housekeeper",
        "event_type": "change.local_landed",
        "entity_id": change_id,
        "repo_root": str(repo),
    }


def test_workflow_land_local_requires_rebase_when_target_line_advanced(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-local-stale-target"
    repo.mkdir()
    app_file = repo / "app.py"
    notes = repo / "NOTES.txt"
    app_file.write_text("print('base')\n", encoding="utf-8")
    notes.write_text("tracked base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
    assert (
        runner.invoke(
            app,
            ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Rebase before local land",
            "--intent",
            "block stale local land when main advanced after the task forked",
            "--change-title",
            "Require rebase before local land",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    started = json.loads(start_out.stdout)
    change_id = started["change"]["change_id"]
    bound_worktree_name = started["worktree"]["name"]
    bound_worktree_path = Path(started["worktree"]["path"])

    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(bound_worktree_path)
        (bound_worktree_path / "app.py").write_text("print('feature')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "feature work", "--json"])
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout

    notes.write_text("tracked base\nmain advanced\n", encoding="utf-8")
    repo_ctx = RepoContext.discover(repo)
    main_advance_snapshot = local_create_snapshot(repo_ctx, "main advance")

    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(bound_worktree_path)
        blocked_out = runner.invoke(app, ["workflow", "land-local", change_id, "--json"])
    assert blocked_out.exit_code != 0
    blocked_output = blocked_out.output or blocked_out.stderr or ""
    assert "does not" in blocked_output
    assert "descend from that head" in blocked_output
    assert "ait worktree rebase --onto main" in blocked_output

    main_show_out = runner.invoke(app, ["line", "show", "main", "--json"])
    assert main_show_out.exit_code == 0, main_show_out.stdout
    assert json.loads(main_show_out.stdout)["head_snapshot_id"] == main_advance_snapshot["snapshot_id"]

    rebase_out = runner.invoke(app, ["worktree", "rebase", bound_worktree_name, "--onto", "main", "--json"])
    assert rebase_out.exit_code == 0, rebase_out.stdout
    assert json.loads(rebase_out.stdout)["rebase"]["status"] == "applied"

    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(bound_worktree_path)
        land_out = runner.invoke(app, ["workflow", "land-local", change_id, "--json"])
    assert land_out.exit_code == 0, land_out.stdout
    landed = json.loads(land_out.stdout)
    assert landed["previous_target_head_snapshot_id"] == main_advance_snapshot["snapshot_id"]
    assert landed["change_status"] == "landed"
    assert landed["task_status"] == "completed"
    assert landed["workspace_action"] == "restored"
    assert not bound_worktree_path.exists()
    assert app_file.read_text(encoding="utf-8") == "print('feature')\n"
    assert notes.read_text(encoding="utf-8") == "tracked base\nmain advanced\n"


def test_workflow_land_local_restores_repo_root_and_removes_bound_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-local-bound-worktree"
    repo.mkdir()
    readme = repo / "README.md"
    app_file = repo / "app.py"
    notes = repo / "NOTES.md"
    readme.write_text("base\n", encoding="utf-8")
    app_file.write_text("print('base')\n", encoding="utf-8")
    notes.write_text("tracked base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    assert (
        runner.invoke(
            app,
            ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Bound local land",
            "--intent",
            "restore the repo root and auto-clean the bound worktree after local land",
            "--change-title",
            "Finish bound local change",
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
    bound_worktree_path = Path(started["worktree"]["path"])
    assert bound_worktree_path.is_dir()

    notes.write_text("tracked base\nlocal dirty note\n", encoding="utf-8")
    untracked_path = repo / "docs" / "benchmarks" / "runs" / "land-local-preserve" / "evidence" / "worker_session.jsonl"
    untracked_path.parent.mkdir(parents=True, exist_ok=True)
    untracked_path.write_text("{\"status\":\"keep\"}\n", encoding="utf-8")

    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(bound_worktree_path)
        (bound_worktree_path / "app.py").write_text("print('landed change')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "landed change"]).exit_code == 0
        land_out = runner.invoke(app, ["workflow", "land-local", change_id, "--json"])

    assert land_out.exit_code == 0, land_out.stdout
    landed = json.loads(land_out.stdout)
    workspace_restore = landed["repo_root_restore"]

    assert landed["change_id"] == change_id
    assert landed["task_id"] == task_id
    assert landed["workspace_action"] == "restored"
    assert workspace_restore["status"] == "restored"
    assert workspace_restore["unrelated_paths"] == [
        "docs/benchmarks/runs/land-local-preserve/evidence/worker_session.jsonl",
    ]
    assert workspace_restore["preserved_unrelated_paths"] == workspace_restore["unrelated_paths"]
    assert workspace_restore["remaining_paths"] == workspace_restore["unrelated_paths"]
    assert "app.py" in workspace_restore["landed_diff_paths"]
    assert landed["bound_worktree_cleanup"]["status"] == "removed"
    assert landed["bound_worktree_cleanup"]["task_id"] == task_id
    assert landed["bound_worktree_cleanup"]["worktree"]["name"] == started["worktree"]["name"]
    assert not bound_worktree_path.exists()
    assert app_file.read_text(encoding="utf-8") == "print('landed change')\n"
    assert notes.read_text(encoding="utf-8") == "tracked base\nlocal dirty note\n"
    assert untracked_path.read_text(encoding="utf-8") == "{\"status\":\"keep\"}\n"

    config_out = runner.invoke(app, ["config", "show", "--json"])
    assert config_out.exit_code == 0, config_out.stdout
    config_payload = json.loads(config_out.stdout)
    assert config_payload["worktree_name"] is None

    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["current_line"] == "main"
    assert status["clean"] is False
    assert status["modified_paths"] == []
    assert status["untracked_paths"] == ["docs/benchmarks/runs/land-local-preserve/evidence/worker_session.jsonl"]


def test_workflow_land_local_does_not_materialize_non_sprint_root_markdown(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-local-root-markdown"
    repo.mkdir()
    app_file = repo / "app.py"
    readme_zh = repo / "README_ZH.md"
    app_file.write_text("print('base')\n", encoding="utf-8")
    readme_zh.write_text("base release docs\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"]).exit_code == 0
    assert (
        runner.invoke(
            app,
            ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Exclude root markdown from local land",
            "--intent",
            "keep non-sprint root markdown out of landed snapshots",
            "--change-title",
            "Land tracked code only",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    started = json.loads(start_out.stdout)
    change_id = started["change"]["change_id"]
    bound_worktree_path = Path(started["worktree"]["path"])

    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(bound_worktree_path)
        (bound_worktree_path / "app.py").write_text("print('landed')\n", encoding="utf-8")
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "land tracked code", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        feature_snapshot_id = json.loads(snap_out.stdout)["snapshot_id"]
        land_out = runner.invoke(app, ["workflow", "land-local", change_id, "--target", "main", "--json"])

    assert land_out.exit_code == 0, land_out.stdout
    landed = json.loads(land_out.stdout)
    assert landed["landed_snapshot_id"] == feature_snapshot_id
    assert "README_ZH.md" not in landed["repo_root_restore"]["landed_diff_paths"]
    assert app_file.read_text(encoding="utf-8") == "print('landed')\n"
    assert readme_zh.read_text(encoding="utf-8") == "base release docs\n"

    main_out = runner.invoke(app, ["line", "show", "main", "--json"])
    assert main_out.exit_code == 0, main_out.stdout
    snapshot_id = json.loads(main_out.stdout)["head_snapshot_id"]
    snapshot_show_out = runner.invoke(app, ["snapshot", "show", snapshot_id, "--json"])
    assert snapshot_show_out.exit_code == 0, snapshot_show_out.stdout
    snapshot_paths = [row["path"] for row in json.loads(snapshot_show_out.stdout)["files"]]
    assert "app.py" in snapshot_paths
    assert "README_ZH.md" not in snapshot_paths


def test_workflow_land_local_requires_clean_workspace(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-land-local-dirty"
    repo.mkdir()
    readme = repo / "README.md"
    app_file = repo / "app.py"
    readme.write_text("base\n", encoding="utf-8")
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert (
        runner.invoke(
            app,
            ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )
    assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Dirty local land",
            "--intent",
            "reject unsnapshotted local work",
            "--change-title",
            "Dirty local change",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    started = json.loads(start_out.stdout)
    change_id = started["change"]["change_id"]
    bound_worktree_path = Path(started["worktree"]["path"])

    (bound_worktree_path / "app.py").write_text("print('dirty')\n", encoding="utf-8")
    with monkeypatch.context() as worktree_context:
        worktree_context.chdir(bound_worktree_path)
        land_out = runner.invoke(app, ["workflow", "land-local", change_id])
    assert land_out.exit_code != 0
    output = land_out.output or land_out.stderr or ""
    assert "Workspace is dirty" in output
    assert "ait snapshot create" in output


def test_workspace_group_help_lists_restore_and_status_roles():
    help_out = runner.invoke(app, ["workspace", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "Restore local workspace content from snapshots or line heads" in " ".join(help_out.stdout.split())
    assert "Inspect workspace drift against line or snapshot." in help_out.stdout


def test_workspace_restore_help_describes_materialize_reset_role():
    help_out = runner.invoke(app, ["workspace", "restore", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    help_text = " ".join(help_out.stdout.split())
    assert "Restore workspace files, or selected paths, from a line head or snapshot" in help_text
    assert "materialize a known revision" in help_text


def test_workspace_status_help_describes_drift_inspection_role():
    help_out = runner.invoke(app, ["workspace", "status", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "Inspect workspace drift against the current line head" in help_out.stdout
    assert "snapshotting, publishing, or restoring." in help_out.stdout


def test_patchset_group_help_lists_published_history_role():
    help_out = runner.invoke(app, ["patchset", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "List published patchsets for a change." in help_out.stdout


def test_patchset_list_help_describes_review_history_role():
    help_out = runner.invoke(app, ["patchset", "list", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "List published patchsets for one change" in help_out.stdout
    assert "active candidate." in help_out.stdout


def test_task_start_tracking_session_inherits_auto_created_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-tracking-auto-worktree"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    config_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--plan-task-binding-mode",
            "advisory",
            "--task-tracking",
            "on",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert config_out.exit_code == 0, config_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Track auto worktree",
            "--intent",
            "bind the tracking session to the auto-created task worktree",
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
    worktree = payload["worktree"]
    tracking = payload["tracking"]
    expected_worktree_name = payload["task_id"].lower()
    feature_line_name = f"feature/{payload['task_id'].lower()}"
    assert worktree["name"] == expected_worktree_name
    assert tracking["worktree_name"] == worktree["name"]
    assert tracking["workspace_root"] == worktree["path"]
    assert worktree["registered_line_name"] == feature_line_name
    assert worktree["current_line"] == feature_line_name
    assert worktree["creation_kind"] == "task_auto_created"
    assert worktree["cleanup_policy"] == "after_remote_land"
    assert worktree["forked_from_line"] == "main"
    assert worktree["target_base_line"] == "main"
    assert worktree["fork_snapshot_id"] == json.loads(seed_out.stdout)["snapshot_id"]
    assert worktree["needs_retarget"] is False

    session_out = runner.invoke(
        app,
        ["session", "show", tracking["session_id"], "--local", "--json"],
        catch_exceptions=False,
    )
    assert session_out.exit_code == 0, session_out.stdout
    session = json.loads(session_out.stdout)
    assert session["worktree_name"] == worktree["name"]
    assert session["line_name"] == feature_line_name
    assert session["metadata"]["repo_root"] == str(repo)
    assert session["metadata"]["workspace_root"] == worktree["path"]


def test_task_start_keeps_repo_root_guard_but_allows_dirty_task_bootstrap(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-root-guard"
    repo.mkdir()
    readme = repo / "README.md"
    app_file = repo / "app.py"
    readme.write_text("base\n", encoding="utf-8")
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    config_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--plan-task-binding-mode",
            "advisory",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert config_out.exit_code == 0, config_out.stdout

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Guard repo root",
            "--intent",
            "keep later stateful commands inside the bound task worktree",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree = payload["worktree"]
    guidance = payload["worktree_guidance"]
    assert "repo root" in guidance["root_guard_warning"]

    config_show_out = runner.invoke(app, ["config", "show", "--json"])
    assert config_show_out.exit_code == 0, config_show_out.stdout
    config_payload = json.loads(config_show_out.stdout)
    assert config_payload["is_worktree"] is False
    assert config_payload["worktree_name"] == worktree["name"]

    blocked_snapshot_out = runner.invoke(app, ["snapshot", "create", "--message", "blocked"])
    assert blocked_snapshot_out.exit_code == 2
    blocked_snapshot_output = blocked_snapshot_out.output or blocked_snapshot_out.stdout
    assert "Repo root is pinned to bound worktree" in blocked_snapshot_output
    assert worktree["name"] in blocked_snapshot_output
    assert "ait snapshot create" in blocked_snapshot_output

    second_task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Allow fresh bootstrap",
            "--intent",
            "allow a clean repo root to open a new bound worktree even while another one remains active",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert second_task_out.exit_code == 0, second_task_out.stdout
    second_payload = json.loads(second_task_out.stdout)
    second_worktree = second_payload["worktree"]
    assert second_worktree["name"] != worktree["name"]
    assert Path(second_worktree["path"]).exists()

    config_show_out = runner.invoke(app, ["config", "show", "--json"])
    assert config_show_out.exit_code == 0, config_show_out.stdout
    config_payload = json.loads(config_show_out.stdout)
    assert config_payload["worktree_name"] == second_worktree["name"]

    readme.write_text("base\ndirty root\n", encoding="utf-8")
    app_file.write_text("print('dirty root')\n", encoding="utf-8")
    third_task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Allow dirty tracked bootstrap",
            "--intent",
            "allow pinned repo-root task bootstrap to continue when root has tracked changes",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert third_task_out.exit_code == 0, third_task_out.stdout
    third_payload = json.loads(third_task_out.stdout)
    third_worktree = third_payload["worktree"]
    third_guidance = third_payload["worktree_guidance"]
    assert third_worktree["name"] not in {worktree["name"], second_worktree["name"]}
    assert Path(third_worktree["path"]).exists()
    assert third_guidance["source_workspace_changed_count"] == 1
    assert third_guidance["source_workspace_changed_paths"] == ["app.py"]
    assert "Existing workspace changes were not copied into the new task worktree." in third_guidance["dirty_source_warning"]
    assert not (Path(third_worktree["path"]) / "README.md").exists()
    assert (Path(third_worktree["path"]) / "app.py").read_text(encoding="utf-8") == "print('base')\n"

    config_show_out = runner.invoke(app, ["config", "show", "--json"])
    assert config_show_out.exit_code == 0, config_show_out.stdout
    config_payload = json.loads(config_show_out.stdout)
    assert config_payload["worktree_name"] == third_worktree["name"]

    blocked_snapshot_after_dirty_out = runner.invoke(
        app,
        ["snapshot", "create", "--message", "still blocked from repo root"],
        catch_exceptions=False,
    )
    assert blocked_snapshot_after_dirty_out.exit_code == 2
    blocked_snapshot_after_dirty_output = blocked_snapshot_after_dirty_out.output or blocked_snapshot_after_dirty_out.stdout
    assert "Repo root is pinned to bound worktree" in blocked_snapshot_after_dirty_output
    assert third_worktree["name"] in blocked_snapshot_after_dirty_output

    worktree_path = Path(third_worktree["path"])
    monkeypatch.chdir(worktree_path)
    worktree_app = worktree_path / "app.py"
    worktree_app.write_text("print('inside worktree')\n", encoding="utf-8")
    worktree_snapshot_out = runner.invoke(
        app,
        ["snapshot", "create", "--message", "worktree checkpoint", "--json"],
        catch_exceptions=False,
    )
    assert worktree_snapshot_out.exit_code == 0, worktree_snapshot_out.stdout
    assert json.loads(worktree_snapshot_out.stdout)["snapshot_id"]


def test_task_auto_worktree_strict_authoring_blocks_unbound_worktree_commands(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-bound-authoring-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    advisory_out = runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    )
    assert advisory_out.exit_code == 0, advisory_out.stdout

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Strict task-bound authoring",
            "--intent",
            "create one task before strict task worktree mode turns on",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)

    other_task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Other task",
            "--intent",
            "prove a bound worktree cannot create changes for another task",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert other_task_out.exit_code == 0, other_task_out.stdout
    other_task = json.loads(other_task_out.stdout)

    worktree_out = _invoke_internal_worktree_add(["worktree", "add", "manual-scratch", "--line", "main", "--json"])
    assert worktree_out.exit_code == 0, worktree_out.stdout
    scratch = json.loads(worktree_out.stdout)
    scratch_path = Path(scratch["path"])

    monkeypatch.chdir(scratch_path)

    blocked_snapshot_out = runner.invoke(
        app,
        ["snapshot", "create", "--message", "blocked from scratch"],
        catch_exceptions=False,
    )
    assert blocked_snapshot_out.exit_code == 2
    blocked_snapshot_output = blocked_snapshot_out.output or blocked_snapshot_out.stdout
    assert "requires a task-bound worktree" in blocked_snapshot_output
    assert "manual-scratch" in blocked_snapshot_output

    blocked_change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            task["task_id"],
            "--title",
            "Blocked from scratch worktree",
            "--base-line",
            "main",
        ],
        catch_exceptions=False,
    )
    assert blocked_change_out.exit_code == 2
    blocked_change_output = blocked_change_out.output or blocked_change_out.stdout
    assert "requires a task-bound worktree" in blocked_change_output
    assert "manual-scratch" in blocked_change_output

    bind_out = _invoke_internal_worktree_bind(["worktree", "bind", "manual-scratch", "--task", task["task_id"], "--json"])
    assert bind_out.exit_code == 0, bind_out.stdout
    bound = json.loads(bind_out.stdout)
    assert bound["bound_task_id"] == task["task_id"]
    assert bound["bound_change_id"] is None

    allowed_change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            task["task_id"],
            "--title",
            "Allowed from bound manual worktree",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert allowed_change_out.exit_code == 0, allowed_change_out.stdout
    allowed_change = json.loads(allowed_change_out.stdout)
    assert allowed_change["task_id"] == task["task_id"]

    repo_ctx = RepoContext.discover(repo)
    published_task = mark_local_task_published(
        repo_ctx,
        task["task_id"],
        remote_name="origin",
        published_task_id="AITT-9001",
    )
    alias_change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            published_task["published_task_id"],
            "--title",
            "Allowed from same-lineage task alias",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert alias_change_out.exit_code == 0, alias_change_out.stdout
    alias_change = json.loads(alias_change_out.stdout)
    assert alias_change["task_id"] == task["task_id"]

    blocked_other_task_change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            other_task["task_id"],
            "--title",
            "Blocked from another task worktree",
            "--base-line",
            "main",
        ],
        catch_exceptions=False,
    )
    assert blocked_other_task_change_out.exit_code == 2
    blocked_other_task_change_output = (
        blocked_other_task_change_out.output or blocked_other_task_change_out.stdout
    )
    assert f"is bound to task `{task['task_id']}`" in blocked_other_task_change_output
    assert other_task["task_id"] in blocked_other_task_change_output


def test_internal_worktree_bind_blocks_cross_task_rebind_but_allows_same_lineage_alias(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-bind-cross-task-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    advisory_out = runner.invoke(
        app,
        ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
        catch_exceptions=False,
    )
    assert advisory_out.exit_code == 0, advisory_out.stdout

    first_task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "First bound task",
            "--intent",
            "own the manual worktree before alias and rebind checks",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert first_task_out.exit_code == 0, first_task_out.stdout
    first_task = json.loads(first_task_out.stdout)

    second_task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Second task",
            "--intent",
            "try to steal the already bound worktree",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert second_task_out.exit_code == 0, second_task_out.stdout
    second_task = json.loads(second_task_out.stdout)

    worktree_out = _invoke_internal_worktree_add(["worktree", "add", "manual-scratch", "--line", "main", "--json"])
    assert worktree_out.exit_code == 0, worktree_out.stdout
    scratch = json.loads(worktree_out.stdout)
    scratch_path = Path(scratch["path"])

    bind_out = _invoke_internal_worktree_bind(["worktree", "bind", "manual-scratch", "--task", first_task["task_id"], "--json"])
    assert bind_out.exit_code == 0, bind_out.stdout

    monkeypatch.chdir(scratch_path)
    first_change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            first_task["task_id"],
            "--title",
            "Same-lineage alias bind remains valid",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert first_change_out.exit_code == 0, first_change_out.stdout
    first_change = json.loads(first_change_out.stdout)

    repo_ctx = RepoContext.discover(repo)
    published_task = mark_local_task_published(
        repo_ctx,
        first_task["task_id"],
        remote_name="origin",
        published_task_id="AITT-9001",
    )
    published_change = mark_local_change_published(
        repo_ctx,
        first_change["change_id"],
        remote_name="origin",
        published_change_id="AITC-9001",
    )

    alias_bind_out = _invoke_internal_worktree_bind(
        [
            "worktree",
            "bind",
            "manual-scratch",
            "--task",
            published_task["published_task_id"],
            "--change",
            published_change["published_change_id"],
            "--json",
        ]
    )
    assert alias_bind_out.exit_code == 0, alias_bind_out.stdout
    alias_bound = json.loads(alias_bind_out.stdout)
    assert alias_bound["bound_task_id"] == published_task["published_task_id"]
    assert alias_bound["bound_change_id"] == published_change["published_change_id"]

    with pytest.raises(ValueError, match="already bound to task"):
        _invoke_internal_worktree_bind(["worktree", "bind", "manual-scratch", "--task", second_task["task_id"]])


def test_task_auto_created_bound_worktree_blocks_cross_task_change_create_and_allows_same_lineage_alias(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-auto-worktree-cross-task-change-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    config_out = runner.invoke(
        app,
        [
            "config",
            "set",
            "--plan-task-binding-mode",
            "advisory",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert config_out.exit_code == 0, config_out.stdout

    first_task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "First auto-created task",
            "--intent",
            "verify wrong-task change creation is rejected from auto-created worktrees",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert first_task_out.exit_code == 0, first_task_out.stdout
    first_task = json.loads(first_task_out.stdout)

    second_task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Second auto-created task",
            "--intent",
            "act as the wrong target for cross-task change creation attempts",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert second_task_out.exit_code == 0, second_task_out.stdout
    second_task = json.loads(second_task_out.stdout)

    first_worktree_path = Path(first_task["worktree"]["path"])
    monkeypatch.chdir(first_worktree_path)

    repo_ctx = RepoContext.discover(repo)
    published_first_task = mark_local_task_published(
        repo_ctx,
        first_task["task_id"],
        remote_name="origin",
        published_task_id="AITT-9101",
    )
    alias_change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            published_first_task["published_task_id"],
            "--title",
            "Allowed auto-created alias change",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert alias_change_out.exit_code == 0, alias_change_out.stdout
    alias_change = json.loads(alias_change_out.stdout)
    assert alias_change["task_id"] == first_task["task_id"]

    blocked_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            second_task["task_id"],
            "--title",
            "Blocked from other auto-created task worktree",
            "--base-line",
            "main",
        ],
        catch_exceptions=False,
    )
    assert blocked_out.exit_code == 2
    blocked_output = blocked_out.output or blocked_out.stdout
    assert f"is bound to task `{first_task['task_id']}`" in blocked_output
    assert second_task["task_id"] in blocked_output


def test_task_bound_worktree_blocks_remote_change_creation_for_other_task_but_allows_same_lineage_alias(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-cross-task-change-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-remote-cross-task-change-guard") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        config_out = runner.invoke(
            app,
            [
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert config_out.exit_code == 0, config_out.stdout
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        first_task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "First local task",
                "--intent",
                "bind an auto-created worktree before remote change creation checks",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert first_task_out.exit_code == 0, first_task_out.stdout
        first_task = json.loads(first_task_out.stdout)

        second_task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--local",
                "--title",
                "Second local task",
                "--intent",
                "act as the wrong remote task target for the bound worktree",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert second_task_out.exit_code == 0, second_task_out.stdout
        second_task = json.loads(second_task_out.stdout)

        remote_first_task = remote_client_module.create_task(
            base_url,
            "housekeeper",
            "First remote task",
            "mirror the first local task",
            "medium",
            task_id="AITT-9201",
        )
        remote_second_task = remote_client_module.create_task(
            base_url,
            "housekeeper",
            "Second remote task",
            "mirror the second local task",
            "medium",
            task_id="AITT-9202",
        )

        repo_ctx = RepoContext.discover(repo)
        published_first_task = mark_local_task_published(
            repo_ctx,
            first_task["task_id"],
            remote_name="origin",
            published_task_id=remote_first_task["task_id"],
        )
        mark_local_task_published(
            repo_ctx,
            second_task["task_id"],
            remote_name="origin",
            published_task_id=remote_second_task["task_id"],
        )

        first_worktree_path = Path(first_task["worktree"]["path"])
        monkeypatch.chdir(first_worktree_path)

        allowed_remote_change_out = runner.invoke(
            app,
            [
                "change",
                "create",
                "--remote",
                "origin",
                "--task",
                published_first_task["published_task_id"],
                "--title",
                "Allowed remote same-lineage alias change",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert allowed_remote_change_out.exit_code == 0, allowed_remote_change_out.stdout
        allowed_remote_change = json.loads(allowed_remote_change_out.stdout)
        assert allowed_remote_change["task_id"] == remote_first_task["task_id"]

        blocked_remote_change_out = runner.invoke(
            app,
            [
                "change",
                "create",
                "--remote",
                "origin",
                "--task",
                remote_second_task["task_id"],
                "--title",
                "Blocked remote wrong-task change",
                "--base-line",
                "main",
                "--risk",
                "medium",
            ],
            catch_exceptions=False,
        )
        assert blocked_remote_change_out.exit_code == 2
        blocked_remote_change_output = (
            blocked_remote_change_out.output or blocked_remote_change_out.stdout
        )
        assert f"is bound to task `{first_task['task_id']}`" in blocked_remote_change_output
        assert remote_second_task["task_id"] in blocked_remote_change_output


def test_repo_root_change_create_guides_operator_to_matching_bound_task_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-root-change-create-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    _set_plan_task_binding_advisory()
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Root change guard task",
            "--intent",
            "prove repo root change create guidance points at the matching task worktree",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)

    add_out = _invoke_internal_worktree_add(["worktree", "add", "root-change-guard", "--line", "main", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    worktree = json.loads(add_out.stdout)

    bind_out = _invoke_internal_worktree_bind(["worktree", "bind", "root-change-guard", "--task", task["task_id"], "--json"])
    assert bind_out.exit_code == 0, bind_out.stdout

    blocked_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            task["task_id"],
            "--title",
            "Blocked from repo root",
            "--base-line",
            "main",
        ],
        catch_exceptions=False,
    )
    assert blocked_out.exit_code == 2
    blocked_output = blocked_out.output or blocked_out.stdout
    assert "Repo root is pinned to bound worktree" in blocked_output
    assert "Continue in that task workspace" in blocked_output
    assert task["task_id"] in blocked_output


def test_repo_root_patchset_publish_guides_operator_to_matching_bound_task_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-root-patchset-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-root-patchset-guard") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        remote_task = remote_client_module.create_task(
            base_url,
            "housekeeper",
            "Root patchset guard task",
            "bind a manual worktree for repo-root patchset guidance",
            "medium",
            task_id="AITT-9301",
        )
        add_out = _invoke_internal_worktree_add(["worktree", "add", "root-patchset-guard", "--line", "main", "--json"])
        assert add_out.exit_code == 0, add_out.stdout
        worktree = json.loads(add_out.stdout)
        bind_out = _invoke_internal_worktree_bind(
            ["worktree", "bind", "root-patchset-guard", "--task", remote_task["task_id"], "--json"]
        )
        assert bind_out.exit_code == 0, bind_out.stdout
        remote_change = remote_client_module.create_change(
            base_url,
            "housekeeper",
            remote_task["task_id"],
            "Remote patchset guard change",
            "main",
            "medium",
            change_id="AITC-9301",
        )

        blocked_out = runner.invoke(
            app,
            [
                "patchset",
                "publish",
                "--change",
                remote_change["change_id"],
                "--summary",
                "blocked from repo root",
                "--remote",
                "origin",
            ],
            catch_exceptions=False,
        )
        assert blocked_out.exit_code == 2
        blocked_output = blocked_out.output or blocked_out.stdout
        assert "requires a task-bound worktree" in blocked_output
        assert "ait worktree recreate" in blocked_output


def test_repo_root_land_submit_guides_operator_to_matching_bound_task_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-root-land-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-root-land-guard") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        remote_task = remote_client_module.create_task(
            base_url,
            "housekeeper",
            "Root land guard task",
            "bind a manual worktree for repo-root land guidance",
            "medium",
            task_id="AITT-9302",
        )
        add_out = _invoke_internal_worktree_add(["worktree", "add", "root-land-guard", "--line", "main", "--json"])
        assert add_out.exit_code == 0, add_out.stdout
        worktree = json.loads(add_out.stdout)
        bind_out = _invoke_internal_worktree_bind(
            ["worktree", "bind", "root-land-guard", "--task", remote_task["task_id"], "--json"]
        )
        assert bind_out.exit_code == 0, bind_out.stdout
        remote_change = remote_client_module.create_change(
            base_url,
            "housekeeper",
            remote_task["task_id"],
            "Remote land guard change",
            "main",
            "medium",
            change_id="AITC-9302",
        )

        blocked_out = runner.invoke(
            app,
            [
                "land",
                "submit",
                remote_change["change_id"],
                "--target",
                "main",
                "--mode",
                "direct",
                "--remote",
                "origin",
            ],
            catch_exceptions=False,
        )
        assert blocked_out.exit_code == 2
        blocked_output = blocked_out.output or blocked_out.stdout
        assert "requires a task-bound worktree" in blocked_output
        assert remote_task["task_id"] in blocked_output
        assert worktree["name"] in blocked_output


def test_repo_root_remote_plan_sync_bypasses_active_root_worktree_guard(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-root-plan-sync-guard"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    artifact_file = _write_plan_artifact(repo, "docs/sprints/root_plan_sync_guard.md", "# Root Plan Sync Guard\n\nInitial lineage.\n")
    updated_artifact = "# Root Plan Sync Guard\n\nUpdated lineage from repo root.\n"

    with running_server(tmp_path / "server-data-root-plan-sync-guard") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        _set_plan_task_binding_advisory()
        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Root plan sync guard task",
                "--intent",
                "pin repo root to a task worktree while public repo-root remote plan sync stays allowed",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        started = json.loads(start_out.stdout)
        worktree = started["worktree"]
        task_id = started["task_id"]
        remote_task = remote_client_module.create_task(
            base_url,
            "housekeeper",
            "Root plan sync guard task",
            "mirror the local root-pinning task so remote plan sync provenance resolves",
            "medium",
            task_id=task_id,
        )
        mark_local_task_published(
            RepoContext.discover(repo),
            task_id,
            remote_name="origin",
            published_task_id=remote_task["task_id"],
        )

        blocked_snapshot_out = runner.invoke(
            app,
            ["snapshot", "create", "--message", "blocked from repo root", "--json"],
            catch_exceptions=False,
        )
        assert blocked_snapshot_out.exit_code == 2
        blocked_output = blocked_snapshot_out.output or blocked_snapshot_out.stdout
        assert "Repo root is pinned to bound worktree" in blocked_output
        assert task_id in blocked_output
        assert worktree["name"] in blocked_output

        _write_plan_artifact(repo, artifact_file, updated_artifact)
        sync_out = runner.invoke(
            app,
            ["plan", "sync", artifact_file, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["summary"]["processed_count"] == 1
        assert sync_payload["summary"]["published_count"] == 1
        _assert_plan_sync_lineage_only(sync_payload)
        assert (repo / artifact_file).read_text(encoding="utf-8") == updated_artifact


def test_repo_root_remote_plan_sync_falls_back_when_active_root_binding_is_local_only(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-root-plan-sync-local-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    artifact_file = _write_plan_artifact(repo, "docs/sprints/root_plan_sync_local_only.md", "# Root Plan Sync Local Only\n\nInitial lineage.\n")
    updated_artifact = "# Root Plan Sync Local Only\n\nUpdated lineage from repo root without a published task.\n"

    with running_server(tmp_path / "server-data-root-plan-sync-local-only") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        _set_plan_task_binding_advisory()
        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Root plan sync local-only guard task",
                "--intent",
                "pin repo root to a local-only task worktree while remote plan sync still records generic lineage provenance",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout

        _write_plan_artifact(repo, artifact_file, updated_artifact)
        sync_out = runner.invoke(
            app,
            ["plan", "sync", artifact_file, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["summary"]["processed_count"] == 1
        assert sync_payload["summary"]["published_count"] == 1
        _assert_plan_sync_lineage_only(sync_payload)

        plan_id = sync_payload["results"][0]["plan_id"]
        remote_plan = remote_client_module.get_plan(base_url, plan_id)
        source_session_id = remote_plan["head_revision"]["source_session_id"]
        sessions = remote_client_module.list_sessions(base_url, "housekeeper")
        matching_session = next(row for row in sessions if row["session_id"] == source_session_id)
        assert matching_session["task_id"] is None
        assert matching_session["session_kind"] == "agent_run"
        assert (matching_session.get("metadata") or {}).get("tracking_policy") == "workflow_boundary_session"

        events = remote_client_module.list_session_events(base_url, source_session_id, repo_name="housekeeper")
        boundary_events = [row for row in events if row["event_type"] == "workflow.boundary"]
        assert len(boundary_events) == 1
        assert boundary_events[0]["payload"]["boundary_kind"] == "plan_sync"
        assert boundary_events[0]["payload"]["session_resolution"] == "workspace_boundary_session"
        assert "plan_sync" in str(boundary_events[0]["payload"]["workflow_context"])


def test_worktree_remove_clears_active_root_worktree_binding(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-worktree-remove-root-guard"
    repo.mkdir()
    readme = repo / "README.md"
    readme.write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    _set_plan_task_binding_advisory()
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Guard repo root cleanup",
            "--intent",
            "clear the active root worktree binding when the bound worktree is removed",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    start_payload = json.loads(start_out.stdout)
    worktree = start_payload["worktree"]
    task_id = start_payload["task_id"]

    remove_out = runner.invoke(
        app,
        ["worktree", "remove", worktree["name"], "--delete-path", "--json"],
        catch_exceptions=False,
    )
    assert remove_out.exit_code == 0, remove_out.stdout
    assert not Path(worktree["path"]).exists()

    config_show_out = runner.invoke(app, ["config", "show", "--json"])
    assert config_show_out.exit_code == 0, config_show_out.stdout
    assert json.loads(config_show_out.stdout)["worktree_name"] is None

    readme.write_text("base\nroot again\n", encoding="utf-8")
    root_snapshot_out = runner.invoke(
        app,
        ["snapshot", "create", "--message", "root after cleanup", "--json"],
        catch_exceptions=False,
    )
    assert root_snapshot_out.exit_code == 2
    root_snapshot_output = root_snapshot_out.output or root_snapshot_out.stdout
    assert "read-only deployment workspace" in root_snapshot_output
    assert "ait snapshot create" in root_snapshot_output

    blocked_change_out = runner.invoke(
        app,
        ["change", "create", "--task", task_id, "--title", "blocked from root", "--base-line", "main", "--json"],
        catch_exceptions=False,
    )
    assert blocked_change_out.exit_code == 2
    blocked_change_output = blocked_change_out.output or blocked_change_out.stdout
    assert "read-only deployment workspace" in blocked_change_output
    assert "ait change create" in blocked_change_output


def test_task_start_validates_local_base_line_before_creating_task(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-preflight"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Should fail early",
            "--intent",
            "missing base line should not create a task",
            "--base-line",
            "missing",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code != 0
    assert "missing" in start_out.output.lower()

    task_list_out = runner.invoke(app, ["task", "list", "--local", "--json"])
    assert task_list_out.exit_code == 0, task_list_out.stdout
    assert json.loads(task_list_out.stdout) == []


def test_task_create_allows_authored_markdown_workspace_bootstrap_in_advisory_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-create-md-advisory"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0
    (repo / "docs").mkdir(exist_ok=True)
    (repo / "docs" / "workflow.md").write_text("# Workflow\n\n- [ ] sync me\n", encoding="utf-8")

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Document workflow",
            "--intent",
            "advisory mode still requires markdown drift to be synced first",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree_path = Path(task["worktree"]["path"])
    assert worktree_path.exists()
    assert not (worktree_path / "docs" / "workflow.md").exists()


def test_task_create_allows_authored_markdown_workspace_bootstrap_even_with_other_dirty_paths(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-create-md-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0
    (repo / "docs").mkdir(exist_ok=True)
    (repo / "docs" / "workflow.md").write_text("# Workflow\n\n- [ ] sync me\n", encoding="utf-8")
    (repo / "README.md").write_text("base\ncode drift too\n", encoding="utf-8")

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Document workflow",
            "--intent",
            "should be plan synced instead",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree_path = Path(task["worktree"]["path"])
    assert worktree_path.exists()
    assert not (worktree_path / "docs" / "workflow.md").exists()
    assert not (worktree_path / "README.md").exists()

    task_list_out = runner.invoke(app, ["task", "list", "--local", "--json"])
    assert task_list_out.exit_code == 0, task_list_out.stdout
    rows = json.loads(task_list_out.stdout)
    assert len(rows) == 1
    assert rows[0]["task_id"] == task["task_id"]


def test_task_start_allows_authored_markdown_workspace_bootstrap(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-start-md-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0
    (repo / "docs").mkdir(exist_ok=True)
    (repo / "docs" / "workflow.md").write_text("# Workflow\n\n- [ ] sync me\n", encoding="utf-8")

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Document workflow",
            "--intent",
            "should be plan synced instead",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree_path = Path(payload["worktree"]["path"])
    assert worktree_path.exists()
    assert not (worktree_path / "docs" / "workflow.md").exists()
    assert payload["change"]["task_id"] == payload["task_id"]

    task_list_out = runner.invoke(app, ["task", "list", "--local", "--json"])
    assert task_list_out.exit_code == 0, task_list_out.stdout
    rows = json.loads(task_list_out.stdout)
    assert len(rows) == 1
    assert rows[0]["task_id"] == payload["task_id"]


def test_change_create_rejects_authored_markdown_workspace_dispatch(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-change-create-md-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0
    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Code workflow",
            "--intent",
            "create a task before markdown drift appears",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree_path = Path(task["worktree"]["path"])
    monkeypatch.chdir(worktree_path)

    (worktree_path / "docs").mkdir(exist_ok=True)
    (worktree_path / "docs" / "workflow.md").write_text("# Workflow\n\n- [ ] sync me\n", encoding="utf-8")
    change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            task["task_id"],
            "--title",
            "Blocked until markdown sync",
            "--base-line",
            "main",
        ],
        catch_exceptions=False,
    )
    assert change_out.exit_code == 2
    output = change_out.output or change_out.stdout
    assert "Markdown drift is present" in output
    assert "ait plan sync" in output
    assert "repository's normal default" in output
    assert "scope" in output
    assert "docs/workflow.md" in output


def test_change_create_rejects_authored_markdown_workspace_dispatch_explains_solo_remote_default_scope(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-change-create-md-solo-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(
        app,
        ["remote", "add", "origin", "http://example.test:8088", "--repo-name", "housekeeper", "--default"],
        catch_exceptions=False,
    ).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote", "--json"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Code workflow",
            "--intent",
            "create the task worktree before markdown drift appears",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree_path = Path(task["worktree"]["path"])
    monkeypatch.chdir(worktree_path)

    (worktree_path / "docs").mkdir(exist_ok=True)
    (worktree_path / "docs" / "workflow.md").write_text("# Workflow\n\n- [ ] sync me\n", encoding="utf-8")

    change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            task["task_id"],
            "--title",
            "Blocked until markdown sync",
            "--base-line",
            "main",
        ],
        catch_exceptions=False,
    )
    assert change_out.exit_code == 2
    output = change_out.output or change_out.stdout
    assert "ait plan sync" in output
    assert "shared plan state" in output
    assert "without another" in output
    assert "`--remote`" in output
    assert "docs/workflow.md" in output


def test_change_create_rejects_dirty_sprint_task_graph_workspace_dispatch(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-change-create-task-graph"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0
    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Code workflow",
            "--intent",
            "create a task before task-graph drift appears",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    worktree_path = Path(task["worktree"]["path"])
    monkeypatch.chdir(worktree_path)

    graph_path = worktree_path / "docs" / "sprints" / "demo.task_graph.json"
    graph_path.parent.mkdir(parents=True, exist_ok=True)
    graph_path.write_text("{\"schema_version\": 1}\n", encoding="utf-8")

    change_out = runner.invoke(
        app,
        [
            "change",
            "create",
            "--local",
            "--task",
            task["task_id"],
            "--title",
            "Blocked until sprint task-graph sync",
            "--base-line",
            "main",
        ],
        catch_exceptions=False,
    )
    assert change_out.exit_code == 2
    output = change_out.output or change_out.stdout
    assert "planning-only sprint artifacts are dirty" in output
    assert "ait plan sync" in output
    assert "docs/sprints/demo.task_graph.json" in output
    assert "task_graph.json" in output


def test_snapshot_create_rejects_dirty_sprint_task_graph_in_execution_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-snapshot-task-graph"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory", "--json"], catch_exceptions=False).exit_code == 0
    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Code workflow",
            "--intent",
            "open the task worktree before task-graph drift appears",
            "--base-line",
            "main",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    payload = json.loads(start_out.stdout)
    worktree_path = Path(payload["worktree"]["path"])
    monkeypatch.chdir(worktree_path)

    graph_path = worktree_path / "docs" / "sprints" / "demo.task_graph.json"
    graph_path.parent.mkdir(parents=True, exist_ok=True)
    graph_path.write_text("{\"schema_version\": 1}\n", encoding="utf-8")

    snapshot_out = runner.invoke(
        app,
        ["snapshot", "create", "--message", "blocked by task graph"],
        catch_exceptions=False,
    )
    assert snapshot_out.exit_code == 2
    output = snapshot_out.output or snapshot_out.stdout
    assert "planning-only sprint artifacts" in output
    assert "ait plan sync" in output
    assert "docs/sprints/demo.task_graph.json" in output


def test_task_create_allows_reconciled_markdown_drift_when_only_non_markdown_paths_remain(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-create-reconciled-markdown-drift"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    (repo / "docs").mkdir(exist_ok=True)
    (repo / "docs" / "workflow.md").write_text(
        "# Workflow\n\n## Dispatchable Note [plan-ref: docs/workflow]\n\n- [ ] sync me [ref: docs/workflow-sync]\n",
        encoding="utf-8",
    )
    (repo / "notes.txt").write_text("keep this local drift\n", encoding="utf-8")

    sync_out = runner.invoke(
        app,
        ["plan", "sync", "docs/workflow.md", "--local", "--json"],
        catch_exceptions=False,
    )
    assert sync_out.exit_code == 0, sync_out.stdout
    sync_payload = json.loads(sync_out.stdout)
    assert sync_payload["summary"]["created_count"] == 1
    _assert_plan_sync_lineage_only(sync_payload)
    plan_id = sync_payload["results"][0]["plan_id"]

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Document workflow",
            "--intent",
            "reconciled markdown drift should not block task dispatch",
            "--plan",
            plan_id,
            "--plan-item-ref",
            "docs/workflow-sync",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    assert task["title"] == "Document workflow"


def test_session_create_and_turn_use_bound_task_worktree(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-session-bound-task-worktree"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-session-bound-task-worktree") as base_url:
        monkeypatch.chdir(repo)
        monkeypatch.setenv("AIT_CHAT_APPEND_TURN_ANALYSIS", "false")
        monkeypatch.setenv("TERM_PROGRAM", "vscode")
        captured: dict[str, object] = {}

        def fake_generate(config, *, session, events, chat_id, chat_title, checkpoint=None, surface="telegram", actor_identity=None):
            captured["repo_root"] = str(config.repo_root)
            captured["session_worktree_name"] = session.get("worktree_name")
            return AiReplyResult(
                text="Bound worktree reply.",
                model="gpt-5.4-codex",
                response_id="turn_cli_session_bound_worktree",
                source="codex",
            )

        monkeypatch.setattr(server_app_module, "generate_session_reply", fake_generate)

        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(
            app,
            [
                "config",
                "set",
                "--plan-task-binding-mode",
                "advisory",
                "--task-tracking",
                "on",
                "--json",
            ],
            catch_exceptions=False,
        ).exit_code == 0

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Tracked bound worktree task",
                "--intent",
                "run session turns inside the bound task worktree",
                "--base-line",
                "main",
                "--risk",
                "low",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        bound_worktree = task["worktree"]
        tracking = task["tracking"]
        expected_worktree_name = task["task_id"].lower()
        feature_line_name = f"feature/{task['task_id'].lower()}"
        assert bound_worktree["name"] == expected_worktree_name
        assert tracking["worktree_name"] == bound_worktree["name"]
        assert bound_worktree["registered_line_name"] == feature_line_name
        assert bound_worktree["current_line"] == feature_line_name

        tracked_session_out = runner.invoke(
            app,
            ["session", "show", tracking["session_id"], "--json"],
            catch_exceptions=False,
        )
        assert tracked_session_out.exit_code == 0, tracked_session_out.stdout
        tracked_session = json.loads(tracked_session_out.stdout)
        assert tracked_session["worktree_name"] == bound_worktree["name"]
        assert tracked_session["line_name"] == feature_line_name
        assert tracked_session["metadata"]["workspace_root"] == bound_worktree["path"]

        explicit_session_out = runner.invoke(
            app,
            [
                "session",
                "create",
                "--task",
                task["task_id"],
                "--kind",
                "agent_run",
                "--title",
                "Bound worktree agent session",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert explicit_session_out.exit_code == 0, explicit_session_out.stdout
        explicit_session = json.loads(explicit_session_out.stdout)
        assert explicit_session["worktree_name"] == bound_worktree["name"]
        assert explicit_session["line_name"] == feature_line_name
        assert explicit_session["metadata"]["workspace_root"] == bound_worktree["path"]

        turn_out = runner.invoke(
            app,
            ["session", "turn", explicit_session["session_id"], "--text", "Continue inside the task worktree.", "--json"],
            catch_exceptions=False,
        )
        assert turn_out.exit_code == 0, turn_out.stdout
        payload = json.loads(turn_out.stdout)
        assert payload["ok"] is True
        assert payload["reply_text"] == "Bound worktree reply."
        assert captured["repo_root"] == bound_worktree["path"]
        assert captured["session_worktree_name"] == bound_worktree["name"]


def test_line_archive_hides_local_line_and_blocks_new_snapshots(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-archive"
    repo.mkdir()
    readme = repo / "README.md"
    readme.write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-line-archive"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout

    create_out = runner.invoke(app, ["line", "create", "feature/archive-me", "--json"])
    assert create_out.exit_code == 0, create_out.stdout

    archive_out = runner.invoke(app, ["line", "archive", "feature/archive-me", "--json"])
    assert archive_out.exit_code == 0, archive_out.stdout
    archived_line = json.loads(archive_out.stdout)
    assert archived_line["status"] == "archived"

    list_out = runner.invoke(app, ["line", "list", "--json"])
    assert list_out.exit_code == 0, list_out.stdout
    visible = json.loads(list_out.stdout)
    assert [row["line_name"] for row in visible] == ["main"]

    all_out = runner.invoke(app, ["line", "list", "--all", "--json"])
    assert all_out.exit_code == 0, all_out.stdout
    all_lines = json.loads(all_out.stdout)
    archived = next(row for row in all_lines if row["line_name"] == "feature/archive-me")
    assert archived["status"] == "archived"

    assert runner.invoke(app, ["line", "switch", "feature/archive-me"]).exit_code == 0
    readme.write_text("base\narchived\n", encoding="utf-8")
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "should fail"])
    assert snap_out.exit_code != 0
    output = snap_out.output or snap_out.stdout or ""
    assert "archived" in output
    assert "Current line" in output or "cannot create" in output


def test_line_cleanup_candidates_and_apply_archive_idle_review_lines(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-cleanup"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-line-cleanup"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "review-base/example", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "review/example", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "wip/example", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "feature/keep-me", "--json"]).exit_code == 0

    conn = sqlite3.connect(repo / ".ait" / "content.db")
    try:
        conn.execute(
            "update lines set updated_at = ? where line_name in (?, ?, ?, ?)",
            (
                "2020-01-01T00:00:00+00:00",
                "review-base/example",
                "review/example",
                "wip/example",
                "feature/keep-me",
            ),
        )
        conn.commit()
    finally:
        conn.close()

    candidates_out = runner.invoke(
        app,
        ["line", "cleanup-candidates", "--older-than", "7d", "--include-protected", "--json"],
        catch_exceptions=False,
    )
    assert candidates_out.exit_code == 0, candidates_out.stdout
    candidates = json.loads(candidates_out.stdout)
    assert candidates["candidate_count"] == 3
    candidate_names = {row["line_name"] for row in candidates["candidates"]}
    assert candidate_names == {"review-base/example", "review/example", "wip/example"}
    protected = {row["line_name"]: row for row in candidates["protected"]}
    assert protected["feature/keep-me"]["protected_reason"] == "line lifecycle is manual_only"

    dry_run_out = runner.invoke(
        app,
        ["line", "cleanup", "--older-than", "7d", "--dry-run", "--json"],
        catch_exceptions=False,
    )
    assert dry_run_out.exit_code == 0, dry_run_out.stdout
    dry_run = json.loads(dry_run_out.stdout)
    assert dry_run["planned_count"] == 3

    apply_out = runner.invoke(
        app,
        ["line", "cleanup", "--older-than", "7d", "--yes", "--json"],
        catch_exceptions=False,
    )
    assert apply_out.exit_code == 0, apply_out.stdout
    applied = json.loads(apply_out.stdout)
    assert applied["archived_count"] == 3

    all_out = runner.invoke(app, ["line", "list", "--all", "--json"])
    assert all_out.exit_code == 0, all_out.stdout
    all_lines = {row["line_name"]: row for row in json.loads(all_out.stdout)}
    assert all_lines["review-base/example"]["status"] == "archived"
    assert all_lines["review/example"]["status"] == "archived"
    assert all_lines["wip/example"]["status"] == "archived"
    assert all_lines["feature/keep-me"]["status"] == "active"


def test_line_cleanup_candidates_precompute_usage_indexes_once(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-cleanup-usage-indexes"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-line-cleanup-usage-indexes"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "review-base/example", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "review/example", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "wip/example", "--json"]).exit_code == 0

    conn = sqlite3.connect(repo / ".ait" / "content.db")
    try:
        conn.execute(
            "update lines set updated_at = ? where line_name in (?, ?, ?)",
            (
                "2020-01-01T00:00:00+00:00",
                "review-base/example",
                "review/example",
                "wip/example",
            ),
        )
        conn.commit()
    finally:
        conn.close()

    call_counts = {"worktrees": 0, "sessions": 0, "changes": 0}

    store_module = import_module("ait.store")
    store_worktrees_module = import_module("ait.store_worktrees")
    original_list_worktrees = store_worktrees_module.list_worktrees
    original_list_sessions = store_module.local_control.list_workflow_sessions
    original_list_changes = store_module.local_control.list_workflow_changes

    def counted_list_worktrees(ctx, *, refresh_status=True):
        call_counts["worktrees"] += 1
        return original_list_worktrees(ctx, refresh_status=refresh_status)

    def counted_list_sessions(ctx, status=None):
        call_counts["sessions"] += 1
        return original_list_sessions(ctx, status=status)

    def counted_list_changes(ctx):
        call_counts["changes"] += 1
        return original_list_changes(ctx)

    monkeypatch.setattr(store_worktrees_module, "list_worktrees", counted_list_worktrees)
    monkeypatch.setattr(store_module.local_control, "list_workflow_sessions", counted_list_sessions)
    monkeypatch.setattr(store_module.local_control, "list_workflow_changes", counted_list_changes)

    candidates_out = runner.invoke(
        app,
        ["line", "cleanup-candidates", "--older-than", "7d", "--json"],
        catch_exceptions=False,
    )
    assert candidates_out.exit_code == 0, candidates_out.stdout
    payload = json.loads(candidates_out.stdout)
    assert payload["candidate_count"] == 3
    assert call_counts == {"worktrees": 1, "sessions": 1, "changes": 1}


def test_line_cleanup_candidates_hoist_current_line_default_line_and_now(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-line-cleanup-hoist"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "review-base/example", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "review/example", "--json"]).exit_code == 0
    assert runner.invoke(app, ["line", "create", "wip/example", "--json"]).exit_code == 0

    conn = sqlite3.connect(repo / ".ait" / "content.db")
    try:
        conn.execute(
            "update lines set updated_at = ? where line_name in (?, ?, ?)",
            (
                "2020-01-01T00:00:00+00:00",
                "review-base/example",
                "review/example",
                "wip/example",
            ),
        )
        conn.commit()
    finally:
        conn.close()

    store_line_cleanup_module = import_module("ait.store_line_cleanup")
    counts = {"current_line": 0, "default_line": 0, "now": 0}
    original_current_line = store_line_cleanup_module.current_line
    original_get_meta = store_line_cleanup_module.local_control.get_meta
    original_utc_now = store_line_cleanup_module.utc_now

    def counted_current_line(ctx):
        counts["current_line"] += 1
        return original_current_line(ctx)

    def counted_get_meta(ctx, key):
        if key == "default_line":
            counts["default_line"] += 1
        return original_get_meta(ctx, key)

    def counted_utc_now():
        counts["now"] += 1
        return original_utc_now()

    monkeypatch.setattr(store_line_cleanup_module, "current_line", counted_current_line)
    monkeypatch.setattr(store_line_cleanup_module.local_control, "get_meta", counted_get_meta)
    monkeypatch.setattr(store_line_cleanup_module, "utc_now", counted_utc_now)

    candidates_out = runner.invoke(
        app,
        ["line", "cleanup-candidates", "--older-than", "7d", "--json"],
        catch_exceptions=False,
    )
    assert candidates_out.exit_code == 0, candidates_out.stdout
    payload = json.loads(candidates_out.stdout)
    assert payload["candidate_count"] == 3
    assert counts == {"current_line": 1, "default_line": 1, "now": 1}


def test_status_json_reports_worktree_and_line_hygiene(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-status-hygiene"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-status-hygiene"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    add_out = _invoke_internal_worktree_add(["worktree", "add", "manualcase", "--json"])
    assert add_out.exit_code == 0, add_out.stdout
    assert runner.invoke(app, ["line", "create", "review-base/example", "--json"]).exit_code == 0

    metadata_path = repo / ".ait" / "worktrees" / "manualcase.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    metadata["last_used_at"] = "2020-01-01T00:00:00+00:00"
    metadata_path.write_text(json.dumps(metadata), encoding="utf-8")

    conn = sqlite3.connect(repo / ".ait" / "content.db")
    try:
        conn.execute(
            "update lines set updated_at = ? where line_name = ?",
            ("2020-01-01T00:00:00+00:00", "review-base/example"),
        )
        conn.commit()
    finally:
        conn.close()

    status_out = runner.invoke(app, ["status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    payload = json.loads(status_out.stdout)
    assert payload["worktree_hygiene"]["manual_review_candidate_count"] == 1
    assert payload["line_hygiene"]["candidate_count"] == 1


def test_task_audit_flags_workflow_stale_when_target_line_already_contains_patchset_revision(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-task-audit-stale"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-task-audit-stale") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "advisory"]).exit_code == 0

        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        task_out = runner.invoke(
            app,
            ["task", "start", "--task-only", "--title", "Audit stale workflow", "--intent", "detect target-line merges without landing records", "--risk", "medium", "--json"])
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        worktree_path = Path(task["worktree"]["path"])

        monkeypatch.chdir(worktree_path)
        assert runner.invoke(app, ["line", "create", "feature/task-audit-stale"]).exit_code == 0
        assert runner.invoke(app, ["line", "switch", "feature/task-audit-stale"]).exit_code == 0
        (worktree_path / "app.py").write_text("print('stale')\n", encoding="utf-8")
        assert runner.invoke(app, ["snapshot", "create", "--message", "feature work"]).exit_code == 0

        change_out = runner.invoke(
            app,
            ["change", "create", "--task", task["task_id"], "--title", "Merged without land", "--base-line", "main", "--risk", "medium", "--json"],
            catch_exceptions=False,
        )
        assert change_out.exit_code == 0, change_out.stdout
        change = json.loads(change_out.stdout)

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "stale workflow patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        monkeypatch.chdir(repo)
        update_remote_line(base_url, "housekeeper", "main", patchset["revision_snapshot_id"])

        audit_out = runner.invoke(app, ["task", "audit", task["task_id"], "--json"])
        assert audit_out.exit_code == 0, audit_out.stdout
        audit = json.loads(audit_out.stdout)
        assert audit["workflow"]["state"] != "ready_to_complete"
        assert audit["summary"]["ready_to_complete"] is False
        assert audit["summary"]["effectively_complete_on_target"] is True
        assert audit["summary"]["stale_workflow_records"] is True
        assert audit["summary"]["verdict"] == "workflow_stale_on_target"
        assert audit["recommended_action"]["code"] == "repair_workflow_state"
        assert audit["summary"]["effective_on_target_change_count"] == 1
        assert audit["changes"][0]["target_state"] == "merged_on_target"


def test_plan_sync_leaves_current_line_unchanged_when_unrelated_workspace_paths_are_dirty(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-unrelated-dirty"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-unrelated-dirty") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        seed_snapshot_id = json.loads(snap_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        artifact_file = _write_plan_artifact(
            repo,
            "docs/sprints/workflow_bootstrap.md",
            "# Workflow Bootstrap\n\nThis coordination note does not expose plan refs yet.\n",
        )
        (repo / "README.md").write_text("base\nunrelated dirty change\n", encoding="utf-8")

        sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        sync_payload = json.loads(sync_out.stdout)
        assert sync_payload["summary"]["created_count"] == 1
        _assert_plan_sync_lineage_only(sync_payload)

        status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["clean"] is True
        assert status["changed_paths"] == []
        assert status["modified_paths"] == []
        assert status["untracked_paths"] == []
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_snapshot_id


def test_plan_sync_allowed_from_repo_root_when_root_main_is_readonly(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-root-readonly"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    artifact_file = _write_plan_artifact(
        repo,
        "docs/sprints/root_readonly.md",
        "# Root Readonly\n\nThis authored Markdown should still sync from repo root.\n",
    )
    main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
    assert main_line_out.exit_code == 0, main_line_out.stdout
    seed_snapshot_id = json.loads(main_line_out.stdout)["head_snapshot_id"]
    sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--json"])
    assert sync_out.exit_code == 0, sync_out.stdout
    sync_payload = json.loads(sync_out.stdout)
    assert sync_payload["summary"]["created_count"] == 1
    _assert_plan_sync_lineage_only(sync_payload)

    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["modified_paths"] == []
    assert status["untracked_paths"] == []
    updated_main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
    assert updated_main_line_out.exit_code == 0, updated_main_line_out.stdout
    assert json.loads(updated_main_line_out.stdout)["head_snapshot_id"] == seed_snapshot_id


def test_plan_sync_local_from_worktree_is_rejected(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-worktree-local-rejected"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
    assert seed_out.exit_code == 0, seed_out.stdout
    seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
    assert runner.invoke(app, ["line", "create", "feature/worktree-local-plan-sync", "--json"]).exit_code == 0
    worktree_out = _invoke_internal_worktree_add(
        ["worktree", "add", "worktree-local-plan-sync", "--line", "feature/worktree-local-plan-sync", "--json"]
    )
    assert worktree_out.exit_code == 0, worktree_out.stdout
    worktree_path = Path(json.loads(worktree_out.stdout)["path"])

    artifact_file = "docs/workflow_bootstrap_local.md"
    monkeypatch.chdir(worktree_path)
    _write_plan_artifact(worktree_path, artifact_file, "# Workflow Bootstrap\n\nThis should be rejected from any worktree.\n")
    blocked_out = runner.invoke(app, ["plan", "sync", artifact_file, "--local", "--json"])
    assert blocked_out.exit_code == 2
    blocked_output = blocked_out.output or blocked_out.stdout
    assert "worktree" in blocked_output.lower()
    assert "repo root" in blocked_output.lower()
    assert "plan sync" in blocked_output.lower()
    assert not (repo / artifact_file).exists()

    monkeypatch.chdir(repo)
    root_status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert root_status_out.exit_code == 0, root_status_out.stdout
    assert json.loads(root_status_out.stdout)["clean"] is True
    list_out = runner.invoke(app, ["plan", "list", "--json"])
    assert list_out.exit_code == 0, list_out.stdout
    assert json.loads(list_out.stdout) == []
    main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
    assert main_line_out.exit_code == 0, main_line_out.stdout
    assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_snapshot_id


def test_plan_sync_remote_from_worktree_is_rejected_without_restoring_repo_root_main_markdown(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-worktree-root-main"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    original_artifact = "# Worktree Mirror\n\nOriginal main copy.\n"
    updated_artifact = "# Worktree Mirror\n\nUpdated from task worktree.\n"

    with running_server(tmp_path / "server-data-plan-sync-worktree-root-main") as base_url:
        monkeypatch.chdir(repo)
        artifact_file = _write_plan_artifact(repo, "docs/sprints/worktree_mirror.md", original_artifact)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert snap_out.exit_code == 0, snap_out.stdout
        seed_snapshot_id = json.loads(snap_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        line_out = runner.invoke(app, ["line", "create", "feature/worktree-mirror", "--json"])
        assert line_out.exit_code == 0, line_out.stdout
        worktree_out = _invoke_internal_worktree_add(["worktree", "add", "worktree-mirror", "--line", "feature/worktree-mirror", "--json"])
        assert worktree_out.exit_code == 0, worktree_out.stdout
        worktree_path = Path(json.loads(worktree_out.stdout)["path"])

        monkeypatch.chdir(worktree_path)
        _write_plan_artifact(worktree_path, artifact_file, updated_artifact)
        blocked_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert blocked_out.exit_code == 2
        blocked_output = blocked_out.output or blocked_out.stdout
        assert "worktree" in blocked_output.lower()
        assert "repo root" in blocked_output.lower()
        assert "plan sync" in blocked_output.lower()
        assert "authoring_workspace_context" in blocked_output
        assert "read-only planning/runtime context" in blocked_output
        assert "repo-root source Markdown" in blocked_output
        assert Path(repo / artifact_file).read_text(encoding="utf-8") == original_artifact

        monkeypatch.chdir(repo)
        root_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert root_status_out.exit_code == 0, root_status_out.stdout
        assert json.loads(root_status_out.stdout)["clean"] is True
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_snapshot_id
        assert remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"] == seed_snapshot_id

        monkeypatch.chdir(worktree_path)
        worktree_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert worktree_status_out.exit_code == 0, worktree_status_out.stdout
        worktree_status = json.loads(worktree_status_out.stdout)
        assert worktree_status["clean"] is True
        assert worktree_status["modified_paths"] == []


def test_plan_sync_remote_from_task_bound_worktree_is_rejected_and_preserves_repo_root_tracked_paths(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-task-bound-root-preserve"
    repo.mkdir()
    readme = repo / "README.md"
    tracked_root = repo / "tracked.txt"
    readme.write_text("base\n", encoding="utf-8")
    tracked_root.write_text("keep me tracked\n", encoding="utf-8")

    original_artifact = "# Task-Bound Mirror\n\nOriginal main copy.\n"
    updated_artifact = "# Task-Bound Mirror\n\nUpdated from task worktree.\n"

    with running_server(tmp_path / "server-data-plan-sync-task-bound-root-preserve") as base_url:
        monkeypatch.chdir(repo)
        artifact_file = _write_plan_artifact(repo, "docs/sprints/task_bound_mirror.md", original_artifact)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(
            app,
            ["config", "set", "--plan-task-binding-mode", "advisory", "--json"],
            catch_exceptions=False,
        ).exit_code == 0
        push_out = runner.invoke(app, ["push", "--line", "main"])
        assert push_out.exit_code == 0, push_out.stdout
        seed_main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert seed_main_line_out.exit_code == 0, seed_main_line_out.stdout
        seed_snapshot_id = json.loads(seed_main_line_out.stdout)["head_snapshot_id"]

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--local",
                "--title",
                "Plan sync from bound task worktree",
                "--intent",
                "keep remote plan sync working while repo root is pinned to the active task worktree",
                "--base-line",
                "main",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        started = json.loads(task_out.stdout)
        remote_task = remote_client_module.create_task(
            base_url,
            "housekeeper",
            "Plan sync from bound task worktree",
            "mirror the bound local task so remote plan sync provenance resolves",
            "medium",
            task_id=started["task_id"],
        )
        repo_ctx = RepoContext.discover(repo)
        mark_local_task_published(
            repo_ctx,
            started["task_id"],
            remote_name="origin",
            published_task_id=remote_task["task_id"],
        )
        worktree_path = Path(started["worktree"]["path"])

        tracked_root.unlink()
        monkeypatch.chdir(repo)
        pre_sync_root_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert pre_sync_root_status_out.exit_code == 0, pre_sync_root_status_out.stdout
        pre_sync_root_status = json.loads(pre_sync_root_status_out.stdout)
        assert pre_sync_root_status["clean"] is False
        assert pre_sync_root_status["missing_paths"] == ["tracked.txt"]

        monkeypatch.chdir(worktree_path)
        _write_plan_artifact(worktree_path, artifact_file, updated_artifact)
        blocked_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert blocked_out.exit_code == 2
        blocked_output = blocked_out.output or blocked_out.stdout
        assert "worktree" in blocked_output.lower()
        assert "repo root" in blocked_output.lower()
        assert "plan sync" in blocked_output.lower()
        assert Path(repo / artifact_file).read_text(encoding="utf-8") == original_artifact

        monkeypatch.chdir(repo)
        root_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert root_status_out.exit_code == 0, root_status_out.stdout
        root_status = json.loads(root_status_out.stdout)
        assert root_status["clean"] is False
        assert root_status["missing_paths"] == ["tracked.txt"]
        assert not tracked_root.exists()
        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_snapshot_id
        assert remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"] == seed_snapshot_id

        monkeypatch.chdir(worktree_path)
        worktree_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert worktree_status_out.exit_code == 0, worktree_status_out.stdout
        worktree_status = json.loads(worktree_status_out.stdout)
        assert worktree_status["clean"] is True
        assert worktree_status["modified_paths"] == []


def test_plan_sync_remote_from_worktree_is_rejected(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-worktree-lineage-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-worktree-lineage-only") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["line", "create", "feature/worktree-lineage-only", "--json"]).exit_code == 0
        worktree_out = _invoke_internal_worktree_add(["worktree", "add", "worktree-lineage-only", "--line", "feature/worktree-lineage-only", "--json"])
        assert worktree_out.exit_code == 0, worktree_out.stdout
        worktree_path = Path(json.loads(worktree_out.stdout)["path"])

        artifact_file = "docs/workflow_bootstrap.md"
        monkeypatch.chdir(worktree_path)
        _write_plan_artifact(worktree_path, artifact_file, "# Workflow Bootstrap\n\nThis stays lineage-only.\n")
        blocked_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert blocked_out.exit_code == 2
        blocked_output = blocked_out.output or blocked_out.stdout
        assert "worktree" in blocked_output.lower()
        assert "repo root" in blocked_output.lower()
        assert "plan sync" in blocked_output.lower()
        assert not (repo / artifact_file).exists()

        monkeypatch.chdir(repo)
        root_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert root_status_out.exit_code == 0, root_status_out.stdout
        assert json.loads(root_status_out.stdout)["clean"] is True

        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        assert json.loads(main_line_out.stdout)["head_snapshot_id"] == seed_snapshot_id
        remote_main = remote_client_module.get_remote_line(base_url, "housekeeper", "main")
        assert remote_main["head_snapshot_id"] == seed_snapshot_id
        local_plan_list_out = runner.invoke(app, ["plan", "list", "--json"], catch_exceptions=False)
        assert local_plan_list_out.exit_code == 0, local_plan_list_out.stdout
        assert json.loads(local_plan_list_out.stdout) == []
        remote_plan_list_out = runner.invoke(app, ["plan", "list", "--remote", "origin", "--json"], catch_exceptions=False)
        assert remote_plan_list_out.exit_code == 0, remote_plan_list_out.stdout
        assert json.loads(remote_plan_list_out.stdout) == []
        monkeypatch.chdir(worktree_path)
        worktree_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert worktree_status_out.exit_code == 0, worktree_status_out.stdout
        worktree_status = json.loads(worktree_status_out.stdout)
        assert worktree_status["clean"] is True
        assert worktree_status["changed_paths"] == []


def test_plan_sync_remote_from_worktree_rejects_archived_markdown_prune(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-worktree-root-delete"
    repo.mkdir()
    (repo / "seed.txt").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-worktree-root-delete") as base_url:
        monkeypatch.chdir(repo)
        artifact_file = _write_plan_artifact(repo, "docs/sprints/workflow.md", "# Workflow Entry\n\nRoot-level tracked Markdown.\n")
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        create_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert create_out.exit_code == 0, create_out.stdout
        plan_id = json.loads(create_out.stdout)["results"][0]["plan_id"]

        repo_ctx = RepoContext.discover(repo)
        local_closed = cli_module.close_local_plan(repo_ctx, plan_id, "archived")
        assert local_closed["status"] == "archived"
        remote_closed = remote_client_module.update_plan_status(base_url, plan_id, "archived")
        assert remote_closed["status"] == "archived"

        line_out = runner.invoke(app, ["line", "create", "feature/worktree-root-delete", "--json"])
        assert line_out.exit_code == 0, line_out.stdout
        worktree_out = _invoke_internal_worktree_add(["worktree", "add", "worktree-root-delete", "--line", "feature/worktree-root-delete", "--json"])
        assert worktree_out.exit_code == 0, worktree_out.stdout
        worktree_path = Path(json.loads(worktree_out.stdout)["path"])

        monkeypatch.chdir(worktree_path)
        blocked_out = runner.invoke(app, ["plan", "sync", ".", "--remote", "origin", "--json"])
        assert blocked_out.exit_code == 2
        blocked_output = blocked_out.output or blocked_out.stdout
        assert "worktree" in blocked_output.lower()
        assert "repo root" in blocked_output.lower()
        assert "plan sync" in blocked_output.lower()

        worktree_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert worktree_status_out.exit_code == 0, worktree_status_out.stdout
        worktree_status = json.loads(worktree_status_out.stdout)
        assert worktree_status["clean"] is True
        assert worktree_status["missing_paths"] == []

        monkeypatch.chdir(repo)
        root_status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert root_status_out.exit_code == 0, root_status_out.stdout
        assert json.loads(root_status_out.stdout)["clean"] is True
        assert Path(repo / artifact_file).exists()

        main_line_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_line_out.exit_code == 0, main_line_out.stdout
        local_main_head = json.loads(main_line_out.stdout)["head_snapshot_id"]
        remote_main_head = remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"]
        assert remote_main_head == local_main_head


def test_workspace_status_treats_synced_sprint_task_graph_artifact_as_root_planning_only(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-root-task-graph-planning-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-root-task-graph-planning-only") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/root_task_graph_planning_only.md",
            "# Root Task Graph Planning Only\n\n"
            "## Sync the paired task graph [plan-ref: root-task-graph/root]\n\n"
            "- [ ] Keep the task graph on the plan-sync path. [ref: root-task-graph/pair]\n",
        )

        initial_sync = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"], catch_exceptions=False)
        assert initial_sync.exit_code == 0, initial_sync.stdout
        initial_payload = json.loads(initial_sync.stdout)
        plan_id = initial_payload["results"][0]["plan_id"]
        plan_revision_id = initial_payload["publish_results"][0]["published_head_revision_id"]

        graph_path = repo / "docs/sprints/root_task_graph_planning_only.task_graph.json"
        graph_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "graph_id": "root-task-graph/task-graph",
                    "repo_name": "housekeeper",
                    "source_plan": {
                        "artifact_path": plan_file,
                        "plan_id": plan_id,
                        "plan_ref": "root-task-graph/root",
                        "plan_revision_id": plan_revision_id,
                    },
                    "dispatch_artifacts": {
                        "source_markdown": plan_file,
                        "parallel_execution_markdown": plan_file,
                        "task_graph_json": "docs/sprints/root_task_graph_planning_only.task_graph.json",
                    },
                    "execution_policy": {
                        "mode": "guarded_full_dag_convergence",
                        "validate_source_plan_revision": True,
                        "default_mode": "local_execution_dag_with_selective_promotion",
                        "dispatch_model": "compact_packet",
                        "worker_execution_mode": "worker_only_compact_packet",
                        "max_total_sessions": 1,
                        "max_worker_sessions": 1,
                        "max_batch_sessions": 1,
                    },
                    "nodes": [
                        {
                            "node_id": "A",
                            "node_kind": "task",
                            "title": "Sync the paired task graph",
                            "plan_item_ref": "root-task-graph/pair",
                            "depends_on": [],
                            "progress_weight": 1,
                            "task_template": {"title": "Sync the paired task graph", "risk_tier": "low"},
                        }
                    ],
                    "edges": [],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )

        artifact_sync = runner.invoke(
            app,
            [
                "plan",
                "sync",
                plan_file,
                "--remote",
                "origin",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert artifact_sync.exit_code == 0, artifact_sync.stdout
        payload = json.loads(artifact_sync.stdout)
        assert payload["summary"]["artifact_count"] == 1

        status_out = runner.invoke(app, ["workspace", "status", "--json"], catch_exceptions=False)
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["clean"] is True
        assert status["changed_paths"] == []
        assert status["modified_paths"] == []
        assert status["untracked_paths"] == []
