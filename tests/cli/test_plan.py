from __future__ import annotations

from ait_server.server_control import connect

from ._shared import *  # noqa: F401,F403

def test_plan_help_frames_discovery_and_dispatch_story():
    plan_help = runner.invoke(app, ["plan", "--help"])
    assert plan_help.exit_code == 0, plan_help.stdout
    assert "show" in plan_help.stdout
    assert "items" in plan_help.stdout
    assert "candidates" in plan_help.stdout
    assert "inspect" in plan_help.stdout

    show_help = runner.invoke(app, ["plan", "show", "--help"])
    assert show_help.exit_code == 0, show_help.stdout
    assert "Inspect one synced plan and optionally one specific revision." in " ".join(show_help.stdout.split())

    items_help = runner.invoke(app, ["plan", "items", "--help"])
    assert items_help.exit_code == 0, items_help.stdout
    assert "List stable plan item refs from the current or selected plan revision." in " ".join(items_help.stdout.split())

    candidates_help = runner.invoke(app, ["plan", "candidates", "--help"])
    assert candidates_help.exit_code == 0, candidates_help.stdout
    assert "List currently taskable plan items across open plans before execution." in " ".join(candidates_help.stdout.split())

    inspect_help = runner.invoke(app, ["plan", "inspect", "--help"])
    assert inspect_help.exit_code == 0, inspect_help.stdout
    assert "Inspect one plan's DAG state, linked tasks, and unpublished local-vs-remote lineage." in " ".join(inspect_help.stdout.split())


def test_local_only_plan_sync_task_start_snapshot_and_land_local_e2e(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-only-e2e"
    repo.mkdir()
    readme = repo / "README.md"
    app_file = repo / "app.py"
    readme.write_text("base\n", encoding="utf-8")
    app_file.write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    init_out = runner.invoke(app, ["init", "--name", "housekeeper-local-only-e2e", "--json"])
    assert init_out.exit_code == 0, init_out.stdout
    assert json.loads(init_out.stdout)["default_line"] == "main"

    main_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "main seed", "--json"])
    assert main_snap_out.exit_code == 0, main_snap_out.stdout
    assert json.loads(main_snap_out.stdout)["line_name"] == "main"
    assert (
        runner.invoke(
            app,
            ["config", "set", "--task-auto-worktree", "on", "--json"],
            catch_exceptions=False,
        ).exit_code
        == 0
    )

    plan_file = _write_plan_artifact(
        repo,
        "docs/sprints/local_only_e2e.md",
        "# Local-Only E2E\n\n## Local Core [plan-ref: local-core/e2e]\n\n- [ ] finish local-only e2e [ref: local-core/e2e-finish]\n",
    )

    sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
    assert sync_out.exit_code == 0, sync_out.stdout
    synced = json.loads(sync_out.stdout)
    assert synced["mode"] == "local"
    assert synced["summary"]["created_count"] == 1
    assert "line_sync" not in synced
    assert "root_main_sync" not in synced
    assert "remote_main_sync" not in synced
    plan_id = synced["results"][0]["plan_id"]

    synced_main_out = runner.invoke(app, ["line", "show", "main", "--json"])
    assert synced_main_out.exit_code == 0, synced_main_out.stdout
    synced_main_snapshot_id = json.loads(synced_main_out.stdout)["head_snapshot_id"]

    start_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--local",
            "--title",
            "Local-only E2E",
            "--intent",
            "exercise init -> plan sync -> task start --local -> snapshot -> land-local",
            "--change-title",
            "Land local-only E2E",
            "--base-line",
            "main",
            "--plan",
            plan_id,
            "--plan-item-ref",
            "local-core/e2e-finish",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert start_out.exit_code == 0, start_out.stdout
    started = json.loads(start_out.stdout)
    assert started["publication_state"] == "local_draft"
    assert started["change"]["publication_state"] == "local_draft"
    task_id = started["task_id"]
    change_id = started["change"]["change_id"]
    assert "worktree" in started
    worktree_path = Path(started["worktree"]["path"])
    monkeypatch.chdir(worktree_path)

    (worktree_path / "app.py").write_text("print('local-only e2e')\n", encoding="utf-8")
    feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "local-only e2e", "--json"])
    assert feature_snap_out.exit_code == 0, feature_snap_out.stdout
    feature_snapshot = json.loads(feature_snap_out.stdout)

    land_out = runner.invoke(app, ["workflow", "land-local", change_id, "--target", "main", "--json"])
    assert land_out.exit_code == 0, land_out.stdout
    landed = json.loads(land_out.stdout)
    assert landed["change_id"] == change_id
    assert landed["task_id"] == task_id
    assert landed["target_line"] == "main"
    assert landed["previous_target_head_snapshot_id"] == synced_main_snapshot_id
    assert landed["landed_snapshot_id"] == feature_snapshot["snapshot_id"]
    assert landed["change_status"] == "landed"
    assert landed["task_status"] == "completed"

    monkeypatch.chdir(repo)
    main_show_out = runner.invoke(app, ["line", "show", "main", "--json"])
    assert main_show_out.exit_code == 0, main_show_out.stdout
    assert json.loads(main_show_out.stdout)["head_snapshot_id"] == feature_snapshot["snapshot_id"]

    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    assert json.loads(status_out.stdout)["clean"] is True
    assert app_file.read_text(encoding="utf-8") == "print('local-only e2e')\n"


def test_plan_publish_command_is_not_public():
    help_out = runner.invoke(app, ["plan", "--help"])
    assert help_out.exit_code == 0, help_out.stdout
    assert "publish" not in help_out.stdout

    publish_out = runner.invoke(app, ["plan", "publish", "PL-DOES-NOT-EXIST"])
    assert publish_out.exit_code != 0
    assert "No such command" in (publish_out.stdout or publish_out.stderr or publish_out.output)


def test_plan_create_and_revise_commands_are_not_public():
    for subcommand in ("create", "revise"):
        result = runner.invoke(app, ["plan", subcommand])
        assert result.exit_code != 0
        assert "No such command" in (result.stdout or result.stderr or result.output)


def test_plan_sync_default_line_option_is_not_public():
    result = runner.invoke(app, ["plan", "sync", "README.md", "--default-line", "main"])
    assert result.exit_code != 0
    assert "No such option" in (result.stdout or result.stderr or result.output)


def test_plan_public_surface_omits_legacy_line_alignment_contract():
    plan_help = runner.invoke(app, ["plan", "--help"])
    assert plan_help.exit_code == 0, plan_help.stdout

    sync_help = runner.invoke(app, ["plan", "sync", "--help"])
    assert sync_help.exit_code == 0, sync_help.stdout

    for forbidden in ("line_sync", "root_main_sync", "remote_main_sync", "--default-line"):
        assert forbidden not in plan_help.stdout
        assert forbidden not in sync_help.stdout


def test_plan_sync_help_keeps_shared_lineage_remote_boundary_explicit():
    sync_help = runner.invoke(app, ["plan", "sync", "--help"], catch_exceptions=False)
    assert sync_help.exit_code == 0, sync_help.stdout
    normalized = " ".join(sync_help.stdout.split())
    assert "Use `--remote origin` when this sync should update shared Markdown lineage." in normalized
    assert "`--remote origin`" in normalized
    assert "shared-lineage Markdown sync boundary;" in normalized
    assert "boundary;" in normalized
    assert "omitting `--remote`" in normalized
    assert "keeps the sync" in normalized
    assert "local-only." in normalized


def test_plan_sync_defaults_to_local_store_and_noops_by_artifact_hash(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-local-default"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    plan_file = _write_plan_artifact(
        repo,
        "docs/sprints/local_default.md",
        "# Local Plan Default\n\n## Keep Plan Local First [plan-ref: local-plan/default]\n\n- [ ] sync locally by default [ref: local-plan/default-sync]\n",
    )

    sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
    assert sync_out.exit_code == 0, sync_out.stdout
    sync_payload = json.loads(sync_out.stdout)
    assert sync_payload["summary"]["created_count"] == 1
    assert sync_payload["results"][0]["action"] == "created"
    assert "line_sync" not in sync_payload
    assert "root_main_sync" not in sync_payload
    assert "remote_main_sync" not in sync_payload
    plan_id = sync_payload["results"][0]["plan_id"]

    show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    plan = json.loads(show_out.stdout)
    assert plan["publication_state"] == "local_draft"
    assert plan["head_revision"]["artifact_blob_id"].startswith("BLB-")

    noop_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
    assert noop_out.exit_code == 0, noop_out.stdout
    noop_payload = json.loads(noop_out.stdout)
    assert noop_payload["summary"]["unchanged_count"] == 1
    assert noop_payload["results"][0]["action"] == "unchanged"

    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["untracked_paths"] == []


def test_plan_sync_local_preserves_structured_plan_lineage_across_markdown_move(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-structured-move"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    old_path = "docs/sprints/runtime_sync.md"
    new_path = "docs/archive/runtime_sync_renamed.md"
    markdown = (
        "# Runtime Stability\n\n"
        "## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n"
        "- [ ] Move init to startup only [ref: runtime/startup-only-init]\n"
    )
    plan_file = _write_plan_artifact(repo, old_path, markdown)

    first_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
    assert first_sync_out.exit_code == 0, first_sync_out.stdout
    first_payload = json.loads(first_sync_out.stdout)
    plan_id = str(first_payload["results"][0]["plan_id"])

    moved_path = repo / new_path
    moved_path.parent.mkdir(parents=True, exist_ok=True)
    (repo / old_path).rename(moved_path)

    moved_sync_out = runner.invoke(app, ["plan", "sync", new_path, "--json"])
    assert moved_sync_out.exit_code == 0, moved_sync_out.stdout
    moved_payload = json.loads(moved_sync_out.stdout)
    assert moved_payload["summary"]["updated_count"] == 1
    assert moved_payload["results"][0]["plan_id"] == plan_id
    assert moved_payload["results"][0]["continuity_match"] == {
        "match_kind": "artifact_selector_move",
        "previous_artifact_path": old_path,
        "new_artifact_path": new_path,
        "artifact_selector": "runtime-stability/tasks",
    }

    show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["head_revision"]["artifact_path"] == new_path
    assert shown["head_revision"]["revision_number"] == 2


def test_plan_sync_local_preserves_generic_markdown_lineage_for_exact_blob_move(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-generic-move"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    old_path = "docs/notes/workflow_bootstrap.md"
    new_path = "docs/archive/workflow_bootstrap.md"
    markdown = "# Workflow Bootstrap\n\nThis coordination note is tracked by plan sync.\n"
    artifact_file = _write_plan_artifact(repo, old_path, markdown)

    first_sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--json"])
    assert first_sync_out.exit_code == 0, first_sync_out.stdout
    first_payload = json.loads(first_sync_out.stdout)
    plan_id = str(first_payload["results"][0]["plan_id"])

    moved_path = repo / new_path
    moved_path.parent.mkdir(parents=True, exist_ok=True)
    (repo / old_path).rename(moved_path)

    moved_sync_out = runner.invoke(app, ["plan", "sync", new_path, "--json"])
    assert moved_sync_out.exit_code == 0, moved_sync_out.stdout
    moved_payload = json.loads(moved_sync_out.stdout)
    assert moved_payload["summary"]["updated_count"] == 1
    assert moved_payload["results"][0]["plan_id"] == plan_id
    continuity = moved_payload["results"][0]["continuity_match"]
    assert continuity["match_kind"] == "exact_blob_move"
    assert continuity["previous_artifact_path"] == old_path
    assert continuity["new_artifact_path"] == new_path
    assert continuity["artifact_blob_id"].startswith("BLB-")

    show_out = runner.invoke(app, ["plan", "show", plan_id, "--json"])
    assert show_out.exit_code == 0, show_out.stdout
    shown = json.loads(show_out.stdout)
    assert shown["head_revision"]["artifact_path"] == new_path
    assert shown["head_revision"]["artifact_selector"] is None
    assert shown["head_revision"]["revision_number"] == 2


def test_plan_sync_local_blocks_structured_copy_while_old_markdown_path_still_exists(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-structured-copy-block"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    old_path = "docs/sprints/runtime_sync.md"
    new_path = "docs/archive/runtime_sync_copy.md"
    markdown = (
        "# Runtime Stability\n\n"
        "## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n"
        "- [ ] Move init to startup only [ref: runtime/startup-only-init]\n"
    )
    plan_file = _write_plan_artifact(repo, old_path, markdown)

    assert runner.invoke(app, ["plan", "sync", plan_file, "--json"]).exit_code == 0
    copied_path = repo / new_path
    copied_path.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(repo / old_path, copied_path)

    blocked_out = runner.invoke(app, ["plan", "sync", new_path, "--json"])
    assert blocked_out.exit_code == 1
    payload = json.loads(blocked_out.stdout)
    assert payload["status"] == "failed"
    assert "rename/move continuity" in payload["error"]["message"]

    list_out = runner.invoke(app, ["plan", "list", "--json"])
    assert list_out.exit_code == 0, list_out.stdout
    assert len(json.loads(list_out.stdout)) == 1


def test_plan_sync_local_rejects_ambiguous_generic_exact_blob_move_matches(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-generic-ambiguous"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    shared_markdown = "# Workflow Bootstrap\n\nThis coordination note is tracked by plan sync.\n"
    plan_a = _write_plan_artifact(repo, "docs/notes/workflow_a.md", shared_markdown)
    plan_b = _write_plan_artifact(
        repo,
        "docs/notes/workflow_b.md",
        "# Workflow Bootstrap B\n\nThis second note starts distinct.\n",
    )

    assert runner.invoke(app, ["plan", "sync", plan_a, "--json"]).exit_code == 0
    assert runner.invoke(app, ["plan", "sync", plan_b, "--json"]).exit_code == 0
    _write_plan_artifact(repo, "docs/notes/workflow_b.md", shared_markdown)
    assert runner.invoke(app, ["plan", "sync", "docs/notes/workflow_b.md", "--json"]).exit_code == 0

    (repo / "docs/notes/workflow_a.md").unlink()
    (repo / "docs/notes/workflow_b.md").unlink()
    candidate_path = _write_plan_artifact(repo, "docs/notes/workflow_c.md", shared_markdown)

    blocked_out = runner.invoke(app, ["plan", "sync", candidate_path, "--json"])
    assert blocked_out.exit_code == 1
    payload = json.loads(blocked_out.stdout)
    assert payload["status"] == "failed"
    assert "ambiguous" in payload["error"]["message"].lower()


def test_plan_sync_remote_preserves_structured_plan_lineage_across_markdown_move(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-remote-structured-move"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-remote-structured-move") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        old_path = "docs/sprints/runtime_sync.md"
        new_path = "docs/archive/runtime_sync_renamed.md"
        markdown = (
            "# Runtime Stability\n\n"
            "## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n"
            "- [ ] Move init to startup only [ref: runtime/startup-only-init]\n"
        )
        plan_file = _write_plan_artifact(repo, old_path, markdown)

        first_sync_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert first_sync_out.exit_code == 0, first_sync_out.stdout
        first_payload = json.loads(first_sync_out.stdout)
        plan_id = str(first_payload["results"][0]["plan_id"])

        moved_path = repo / new_path
        moved_path.parent.mkdir(parents=True, exist_ok=True)
        (repo / old_path).rename(moved_path)

        moved_sync_out = runner.invoke(app, ["plan", "sync", new_path, "--remote", "origin", "--json"])
        assert moved_sync_out.exit_code == 0, moved_sync_out.stdout
        moved_payload = json.loads(moved_sync_out.stdout)
        assert moved_payload["summary"]["updated_count"] == 1
        assert moved_payload["summary"]["published_count"] == 1
        assert moved_payload["results"][0]["plan_id"] == plan_id
        assert moved_payload["results"][0]["continuity_match"]["match_kind"] == "artifact_selector_move"

        remote_show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert remote_show_out.exit_code == 0, remote_show_out.stdout
        remote_plan = json.loads(remote_show_out.stdout)
        assert remote_plan["head_revision"]["artifact_path"] == new_path
        assert remote_plan["head_revision"]["revision_number"] == 2


def test_plan_sync_remote_adopts_structured_move_continuity_from_existing_remote_plan(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-remote-adopted-move"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-remote-adopted-move") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        old_path = "docs/sprints/runtime_sync.md"
        new_path = "docs/archive/runtime_sync_renamed.md"
        markdown = (
            "# Runtime Stability\n\n"
            "## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n"
            "- [ ] Move init to startup only [ref: runtime/startup-only-init]\n"
        )
        remote_plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            markdown,
            "runtime-stability/tasks",
            artifact_path=old_path,
            title="Stabilize Runtime Execution Tasks",
            summary="Seed remote-only lineage",
        )
        _write_plan_artifact(repo, new_path, markdown)

        sync_out = runner.invoke(app, ["plan", "sync", new_path, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        payload = json.loads(sync_out.stdout)
        assert payload["summary"]["adopted_count"] == 1
        assert payload["summary"]["updated_count"] == 1
        assert payload["summary"]["published_count"] == 1
        assert payload["results"][0]["plan_id"] == remote_plan["plan_id"]
        assert payload["results"][0]["continuity_match"]["match_kind"] == "artifact_selector_move"
        assert payload["adoptions"][0]["plan_id"] == remote_plan["plan_id"]

        remote_show_out = runner.invoke(app, ["plan", "show", remote_plan["plan_id"], "--remote", "origin", "--json"])
        assert remote_show_out.exit_code == 0, remote_show_out.stdout
        shown = json.loads(remote_show_out.stdout)
        assert shown["head_revision"]["artifact_path"] == new_path
        assert shown["head_revision"]["revision_number"] == 2


def test_plan_sync_sprint_markdown_remote_publishes_without_advancing_default_line(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-non-sprint-remote"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-non-sprint-remote") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        artifact_file = _write_plan_artifact(
            repo,
            "docs/sprints/workflow_bootstrap.md",
            "# Workflow Bootstrap\n\nThis coordination note stays lineage-only.\n",
        )

        sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        payload = json.loads(sync_out.stdout)
        assert payload["summary"]["created_count"] == 1
        assert payload["summary"]["published_count"] == 1
        assert "line_sync" not in payload
        assert "root_main_sync" not in payload
        assert "remote_main_sync" not in payload

        main_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_out.exit_code == 0, main_out.stdout
        local_main_head = json.loads(main_out.stdout)["head_snapshot_id"]
        assert local_main_head == seed_snapshot_id

        remote_main = remote_client_module.get_remote_line(base_url, "housekeeper", "main")
        assert remote_main["head_snapshot_id"] == seed_snapshot_id

        status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["clean"] is True
        assert status["untracked_paths"] == []


def test_plan_sync_remote_publishes_plan_revision_without_new_snapshot_when_main_is_already_aligned(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-already-aligned"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    artifact_body = "# Workflow Bootstrap\n\nThis coordination note is already on main.\n"

    with running_server(tmp_path / "server-data-plan-sync-already-aligned") as base_url:
        monkeypatch.chdir(repo)
        artifact_file = _write_plan_artifact(repo, "docs/sprints/workflow_bootstrap.md", artifact_body)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0

        sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        payload = json.loads(sync_out.stdout)
        assert payload["summary"]["created_count"] == 1
        assert payload["summary"]["published_count"] == 1
        assert "line_sync" not in payload
        assert "root_main_sync" not in payload
        assert "remote_main_sync" not in payload

        main_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_out.exit_code == 0, main_out.stdout
        assert json.loads(main_out.stdout)["head_snapshot_id"] == seed_snapshot_id
        assert remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"] == seed_snapshot_id


def test_plan_sync_remote_does_not_repoint_default_line_to_equivalent_snapshot(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-reuse-equivalent-snapshot"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    original_artifact = "# Workflow Bootstrap\n\nOriginal content on main.\n"
    equivalent_artifact = "# Workflow Bootstrap Refresh\n\nEquivalent snapshot already exists elsewhere.\n"

    with running_server(tmp_path / "server-data-plan-sync-reuse-equivalent-snapshot") as base_url:
        monkeypatch.chdir(repo)
        artifact_file = _write_plan_artifact(repo, "docs/sprints/workflow_bootstrap.md", original_artifact)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0

        seed_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"])
        assert seed_out.exit_code == 0, seed_out.stdout
        seed_snapshot_id = json.loads(seed_out.stdout)["snapshot_id"]
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        first_sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert first_sync_out.exit_code == 0, first_sync_out.stdout
        first_sync = json.loads(first_sync_out.stdout)
        assert first_sync["summary"]["created_count"] == 1
        assert "line_sync" not in first_sync
        assert "root_main_sync" not in first_sync
        assert "remote_main_sync" not in first_sync

        line_out = runner.invoke(app, ["line", "create", "feature/equivalent-plan-sync", "--switch", "--restore", "--json"])
        assert line_out.exit_code == 0, line_out.stdout
        _write_plan_artifact(repo, artifact_file, equivalent_artifact)
        equivalent_out = runner.invoke(app, ["snapshot", "create", "--message", "equivalent candidate", "--json"])
        assert equivalent_out.exit_code == 0, equivalent_out.stdout
        equivalent_snapshot_id = json.loads(equivalent_out.stdout)["snapshot_id"]

        assert runner.invoke(app, ["line", "switch", "main"]).exit_code == 0
        assert runner.invoke(app, ["workspace", "restore", "--line", "main", "--force"]).exit_code == 0
        _write_plan_artifact(repo, artifact_file, equivalent_artifact)

        snapshot_list_before = runner.invoke(app, ["snapshot", "list", "--json"])
        assert snapshot_list_before.exit_code == 0, snapshot_list_before.stdout
        count_before = len(json.loads(snapshot_list_before.stdout))

        sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--remote", "origin", "--json"])
        assert sync_out.exit_code == 0, sync_out.stdout
        payload = json.loads(sync_out.stdout)
        assert payload["summary"]["updated_count"] == 1
        assert payload["summary"]["published_count"] == 1
        assert "line_sync" not in payload
        assert "root_main_sync" not in payload
        assert "remote_main_sync" not in payload

        main_out = runner.invoke(app, ["line", "show", "main", "--json"])
        assert main_out.exit_code == 0, main_out.stdout
        assert json.loads(main_out.stdout)["head_snapshot_id"] == seed_snapshot_id
        assert equivalent_snapshot_id != seed_snapshot_id
        assert remote_client_module.get_remote_line(base_url, "housekeeper", "main")["head_snapshot_id"] == seed_snapshot_id

        snapshot_list_after = runner.invoke(app, ["snapshot", "list", "--json"])
        assert snapshot_list_after.exit_code == 0, snapshot_list_after.stdout
        assert len(json.loads(snapshot_list_after.stdout)) == count_before

        status_out = runner.invoke(app, ["workspace", "status", "--json"])
        assert status_out.exit_code == 0, status_out.stdout
        status = json.loads(status_out.stdout)
        assert status["clean"] is True
        assert status["modified_paths"] == []


def test_plan_sync_docs_sprints_readme_is_forbidden(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-sprint-readme-lineage-only"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0

    artifact_file = _write_plan_artifact(
        repo,
        "docs/sprints/README.md",
        "# Sprint Docs\n\nThis index stays lineage-only.\n",
    )

    sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--json"])
    assert sync_out.exit_code != 0
    assert "docs/sprints/README.md is reserved" in (sync_out.stdout + sync_out.stderr)

    status_out = runner.invoke(app, ["workspace", "status", "--json"])
    assert status_out.exit_code == 0, status_out.stdout
    status = json.loads(status_out.stdout)
    assert status["clean"] is True
    assert status["changed_paths"] == []


def test_plan_sync_ait_dag_md_is_forbidden_with_runtime_helper_guidance(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-ait-dag-reserved"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0

    artifact_file = _write_plan_artifact(
        repo,
        "ait-dag.md",
        "# ait-dag\n\nRuntime helper only.\n",
    )

    sync_out = runner.invoke(app, ["plan", "sync", artifact_file, "--json"])
    assert sync_out.exit_code != 0
    output = sync_out.stdout + sync_out.stderr
    assert "ait-dag.md is reserved" in output
    assert "compact-DAG runtime helper" in output
    assert "authoring_workspace_context" in output
    assert "docs/sprints/<card>.md" in output


def test_plan_sync_directory_honors_aitignore_but_explicit_file_can_sync(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-aitignore"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / ".aitignore").write_text("docs/sprints/ignored.md\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
    assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
    tracked_file = _write_plan_artifact(
        repo,
        "docs/sprints/tracked.md",
        "# Tracked Plan\n\n## Track Visible Plan [plan-ref: visible/root]\n\n- [ ] keep visible [ref: visible/keep]\n",
    )
    ignored_file = _write_plan_artifact(
        repo,
        "docs/sprints/ignored.md",
        "# Ignored Plan\n\n## Track Explicit Plan [plan-ref: ignored/root]\n\n- [ ] sync only when explicit [ref: ignored/explicit]\n",
    )
    _write_plan_artifact(
        repo,
        ".pytest_cache/README.md",
        "# Cache Note\n\nThis built-in ignored directory must not be synced by directory scans.\n",
    )

    sync_out = runner.invoke(app, ["plan", "sync", "docs", "--json"])
    assert sync_out.exit_code == 0, sync_out.stdout
    sync_payload = json.loads(sync_out.stdout)
    paths = [row["artifact_path"] for row in sync_payload["results"]]
    assert sync_payload["summary"]["created_count"] == len(paths) == 3
    assert tracked_file in paths
    assert ignored_file not in paths
    assert "docs/milestone.md" in paths
    assert "docs/plan.md" in paths

    explicit_out = runner.invoke(app, ["plan", "sync", ignored_file, "--json"])
    assert explicit_out.exit_code == 0, explicit_out.stdout
    explicit_payload = json.loads(explicit_out.stdout)
    assert explicit_payload["summary"]["created_count"] == 1
    assert explicit_payload["results"][0]["artifact_path"] == ignored_file


def test_task_create_links_remote_plan_item_from_plan_id(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-linked-task"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-linked-task") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--task-tracking", "on", "--json"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/runtime_plan_link.md",
            "# Runtime Stability\n\n## Stabilize Runtime Execution Tasks [plan-ref: runtime-stability/tasks]\n\n- [ ] Move init to startup only [ref: runtime/startup-only-init]\n",
        )

        create_out = runner.invoke(
            app,
            ["plan", "sync", plan_file, "--remote", "origin", "--json"],
            catch_exceptions=False,
        )
        assert create_out.exit_code == 0, create_out.stdout
        plan_id = json.loads(create_out.stdout)["results"][0]["plan_id"]

        task_out = runner.invoke(
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
                "Deliver Move init to startup only from Stabilize Runtime Execution Tasks.",
                "--risk",
                "medium",
                "--plan",
                plan_id,
                "--plan-item-ref",
                "runtime/startup-only-init",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        linked_task = json.loads(task_out.stdout)
        assert linked_task["title"] == "Move init to startup only"
        assert linked_task["intent"] == "Deliver Move init to startup only from Stabilize Runtime Execution Tasks."
        assert linked_task["plan_id"] == plan_id
        assert linked_task["plan_item_ref"] == "runtime/startup-only-init"
        assert linked_task["tracking"]["session_id"].startswith("AITS-")


def test_plan_sync_remote_auto_uploads_sibling_task_graph_artifact(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-artifact"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-artifact") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/remote_sync.md",
            "# Remote Sync\n\n"
            "## Bundle DAG Artifacts [plan-ref: remote-sync/root]\n\n"
            "- [ ] Upload graph JSON [ref: remote-sync/graph-json]\n",
        )

        initial_sync = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert initial_sync.exit_code == 0, initial_sync.stdout
        initial_payload = json.loads(initial_sync.stdout)
        plan_id = initial_payload["results"][0]["plan_id"]
        plan_revision_id = initial_payload["publish_results"][0]["published_head_revision_id"]

        graph_path = repo / "docs/sprints/remote_sync.task_graph.json"
        graph_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "graph_id": "remote-sync/task-graph",
                    "repo_name": "housekeeper",
                    "source_plan": {
                        "artifact_path": plan_file,
                        "plan_id": plan_id,
                        "plan_ref": "remote-sync/root",
                        "plan_revision_id": plan_revision_id,
                    },
                    "dispatch_artifacts": {
                        "source_markdown": plan_file,
                        "parallel_execution_markdown": plan_file,
                        "task_graph_json": "docs/sprints/remote_sync.task_graph.json",
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
                            "title": "Upload graph JSON",
                            "plan_item_ref": "remote-sync/graph-json",
                            "depends_on": [],
                            "progress_weight": 1,
                            "task_template": {"title": "Upload graph JSON", "risk_tier": "low"},
                        }
                    ],
                    "edges": [],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )

        artifact_sync = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"], catch_exceptions=False)
        assert artifact_sync.exit_code == 0, artifact_sync.stdout
        payload = json.loads(artifact_sync.stdout)
        assert payload["summary"]["unchanged_count"] == 1
        assert payload["summary"]["artifact_count"] == 1
        assert payload["artifact_results"][0]["artifact_path"] == "docs/sprints/remote_sync.task_graph.json"
        assert payload["artifact_results"][0]["plan_revision_id"] == plan_revision_id

        show_out = runner.invoke(app, ["plan", "show", plan_id, "--remote", "origin", "--json"])
        assert show_out.exit_code == 0, show_out.stdout
        head_revision = json.loads(show_out.stdout)["head_revision"]
        stored_artifacts = head_revision["artifacts"]
        assert stored_artifacts[0]["artifact_path"] == "docs/sprints/remote_sync.task_graph.json"
        assert stored_artifacts[0]["role"] == "task_graph_json"
        assert stored_artifacts[0]["metadata"]["source_plan"]["plan_revision_id"] == plan_revision_id


def test_plan_sync_remote_rejects_stale_auto_discovered_task_graph_artifact(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-sync-stale-artifact"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-sync-stale-artifact") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/stale_remote_sync.md",
            "# Stale Remote Sync\n\n"
            "## Reject Stale DAG Artifact [plan-ref: stale-remote-sync/root]\n\n"
            "- [ ] reject stale graph JSON [ref: stale-remote-sync/graph-json]\n",
        )
        initial_sync = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert initial_sync.exit_code == 0, initial_sync.stdout
        plan_id = json.loads(initial_sync.stdout)["results"][0]["plan_id"]

        graph_path = repo / "docs/sprints/stale_remote_sync.task_graph.json"
        graph_path.write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "graph_id": "stale-remote-sync/task-graph",
                    "repo_name": "housekeeper",
                    "source_plan": {
                        "artifact_path": plan_file,
                        "plan_id": plan_id,
                        "plan_ref": "stale-remote-sync/root",
                        "plan_revision_id": "PR-stale",
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
                    "nodes": [],
                    "edges": [],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )

        stale_sync = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"], catch_exceptions=False)
        assert stale_sync.exit_code == 1
        payload = json.loads(stale_sync.stdout)
        assert payload["status"] == "failed"
        assert payload["error"]["stage"] == "sync"
        assert "is stale for plan" in payload["error"]["message"]


def test_plan_candidates_and_inspect_report_taskable_items_and_local_unpublished_heads(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-candidates"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-candidates") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/plan_dispatch.md",
            "# Plan Dispatch\n\n"
            "## Collapse Plan Dispatch Discovery [plan-ref: plan-dispatch/usability]\n\n"
            "- [ ] Add candidates view [ref: plan-dispatch/candidates]\n"
            "- [ ] Add inspect view [ref: plan-dispatch/inspect]\n",
        )

        create_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert create_out.exit_code == 0, create_out.stdout
        plan_id = json.loads(create_out.stdout)["results"][0]["plan_id"]

        publish_out = runner.invoke(app, ["plan", "sync", plan_file, "--remote", "origin", "--json"])
        assert publish_out.exit_code == 0, publish_out.stdout
        published = json.loads(publish_out.stdout)["publish_results"][0]

        task_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--remote",
                "origin",
                "--title",
                "Add candidates view",
                "--intent",
                "Track the candidates slice from the published plan revision.",
                "--risk",
                "medium",
                "--plan",
                plan_id,
                "--revision",
                published["published_head_revision_id"],
                "--plan-item-ref",
                "plan-dispatch/candidates",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        linked_task = json.loads(task_out.stdout)
        ctx = ServerContext.from_env()
        closed = server_store_module.close_task(ctx, linked_task["task_id"], "completed")
        assert closed["status"] == "completed"

        _write_plan_artifact(
            repo,
            plan_file,
            "# Plan Dispatch\n\n"
            "## Collapse Plan Dispatch Discovery [plan-ref: plan-dispatch/usability]\n\n"
            "- [ ] Add candidates view [ref: plan-dispatch/candidates]\n"
            "- [ ] Add inspect view [ref: plan-dispatch/inspect]\n"
            "- [ ] Add workflow help [ref: plan-dispatch/workflow-help]\n",
        )
        revise_out = runner.invoke(app, ["plan", "sync", plan_file, "--json"])
        assert revise_out.exit_code == 0, revise_out.stdout
        assert json.loads(revise_out.stdout)["summary"]["updated_count"] == 1

        candidates_out = runner.invoke(app, ["plan", "candidates", "--remote", "origin", "--json"])
        assert candidates_out.exit_code == 0, candidates_out.stdout
        candidates_payload = json.loads(candidates_out.stdout)
        assert candidates_payload["summary"]["candidate_plan_count"] == 1
        assert candidates_payload["summary"]["taskable_item_count"] == 1
        assert candidates_payload["summary"]["linked_task_count"] == 1
        assert candidates_payload["summary"]["local_unpublished_head_count"] == 1
        candidate = candidates_payload["candidates"][0]
        assert candidate["plan_id"] == plan_id
        assert candidate["local_unpublished_head"] is True
        assert candidate["open_item_count"] == 2
        assert candidate["taskable_item_count"] == 1
        assert candidate["taskable_items"][0]["plan_item_ref"] == "plan-dispatch/inspect"
        linked_open_items = [item for item in candidate["open_items"] if item["plan_item_ref"] == "plan-dispatch/candidates"]
        assert linked_open_items[0]["taskable_blocker"] == "linked_task_exists"
        assert linked_open_items[0]["linked_tasks"][0]["task_id"] == linked_task["task_id"]

        inspect_out = runner.invoke(app, ["plan", "inspect", plan_id, "--remote", "origin", "--json"])
        assert inspect_out.exit_code == 0, inspect_out.stdout
        inspected = json.loads(inspect_out.stdout)["plan"]
        assert inspected["plan_id"] == plan_id
        assert inspected["local_unpublished_head"] is True
        assert inspected["linked_task_count"] == 1
        assert inspected["items"][0]["linked_tasks"][0]["task_id"] == linked_task["task_id"]
        assert inspected["items"][1]["taskable"] is True


def test_plan_session_create_append_promote_and_close(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-session-runtime"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-session-runtime") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/planning_session_runtime.md",
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
            [
                "plan",
                "session",
                "create",
                plan["plan_id"],
                "--title",
                "Initial planning session",
                "--mode",
                "connected_local",
                "--preferred-agent",
                "local",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        session = json.loads(session_out.stdout)
        assert session["planning_session_id"].startswith("AITPS-")
        assert session["plan_id"] == plan["plan_id"]
        assert session["status"] == "active"
        assert session["artifact_status"] == "not_promoted"

        list_out = runner.invoke(
            app,
            ["plan", "session", "list", plan["plan_id"], "--json"],
            catch_exceptions=False,
        )
        assert list_out.exit_code == 0, list_out.stdout
        listed = json.loads(list_out.stdout)
        assert [row["planning_session_id"] for row in listed] == [session["planning_session_id"]]

        append_out = runner.invoke(
            app,
            [
                "plan",
                "session",
                "append",
                session["planning_session_id"],
                "--type",
                "plan.message",
                "--text",
                "Summarize the runtime rollout options.",
                "--field",
                "role=user",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert append_out.exit_code == 0, append_out.stdout
        appended = json.loads(append_out.stdout)
        assert appended["sequence"] == 1
        assert appended["payload"]["role"] == "user"

        events_out = runner.invoke(
            app,
            ["plan", "session", "events", session["planning_session_id"], "--json"],
            catch_exceptions=False,
        )
        assert events_out.exit_code == 0, events_out.stdout
        events = json.loads(events_out.stdout)
        assert len(events) == 1
        assert events[0]["event_type"] == "plan.message"

        promote_out = runner.invoke(
            app,
            [
                "plan",
                "session",
                "promote",
                session["planning_session_id"],
                "--summary",
                "Promote planning-session results into the plan head",
                *_plan_file_args(
                    _write_plan_artifact(
                        repo,
                        plan_file,
                        "# Planning Session Runtime\n\n## Runtime MVP [plan-ref: planning-session/runtime]\n\n- [x] Add planning session runtime [ref: planning-session/runtime-mvp]\n",
                    ),
                    "planning-session/runtime",
                ),
                "--json",
            ],
            catch_exceptions=False,
        )
        assert promote_out.exit_code == 0, promote_out.stdout
        promoted = json.loads(promote_out.stdout)
        assert promoted["planning_session"]["artifact_status"] == "promoted"
        assert promoted["promoted_revision"]["source_kind"] == "planning_session_promotion"
        assert promoted["promoted_revision"]["source_session_id"] == session["planning_session_id"]
        assert promoted["promoted_revision"]["revision_number"] == 2

        close_out = runner.invoke(
            app,
            ["plan", "session", "close", session["planning_session_id"], "--json"],
            catch_exceptions=False,
        )
        assert close_out.exit_code == 0, close_out.stdout
        closed = json.loads(close_out.stdout)
        assert closed["status"] == "closed"

        append_fail_out = runner.invoke(
            app,
            [
                "plan",
                "session",
                "append",
                session["planning_session_id"],
                "--type",
                "plan.message",
                "--text",
                "This should fail after close.",
            ],
            catch_exceptions=False,
        )
        assert append_fail_out.exit_code == 2
        assert "cannot accept new events" in (append_fail_out.output or append_fail_out.stdout)


def test_plan_session_create_resume_if_active_returns_existing_session(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-session-resume"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-session-resume") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/planning_session_resume.md",
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

        first_out = runner.invoke(
            app,
            ["plan", "session", "create", plan["plan_id"], "--title", "Initial planning session", "--json"],
            catch_exceptions=False,
        )
        assert first_out.exit_code == 0, first_out.stdout
        first = json.loads(first_out.stdout)

        second_out = runner.invoke(
            app,
            ["plan", "session", "create", plan["plan_id"], "--title", "Second request", "--json"],
            catch_exceptions=False,
        )
        assert second_out.exit_code == 0, second_out.stdout
        second = json.loads(second_out.stdout)
        assert second["planning_session_id"] == first["planning_session_id"]


def test_plan_session_join_bootstraps_or_reuses_relay_session(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-session-join"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-session-join") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/planning_session_join.md",
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
            ["plan", "session", "create", plan["plan_id"], "--title", "Initial planning session", "--json"],
            catch_exceptions=False,
        )
        assert session_out.exit_code == 0, session_out.stdout
        planning_session = json.loads(session_out.stdout)

        join_out = runner.invoke(
            app,
            [
                "plan",
                "session",
                "join",
                planning_session["planning_session_id"],
                "--surface",
                "editor",
                "--title",
                "Editor relay",
                "--model-name",
                "gpt-5.4-codex",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert join_out.exit_code == 0, join_out.stdout
        joined = json.loads(join_out.stdout)
        relay = joined["session"]
        assert joined["planning_session"]["planning_session_id"] == planning_session["planning_session_id"]
        assert relay["session_kind"] == "planning_session_relay"
        assert relay["title"] == "Editor relay"
        assert relay["metadata"]["planning_session_id"] == planning_session["planning_session_id"]
        assert relay["metadata"]["plan_id"] == plan["plan_id"]
        assert relay["metadata"]["surface"] == "editor"
        assert relay["model_name"] == "gpt-5.4-codex"

        resumed_out = runner.invoke(
            app,
            ["plan", "session", "join", planning_session["planning_session_id"], "--surface", "editor", "--json"],
            catch_exceptions=False,
        )
        assert resumed_out.exit_code == 0, resumed_out.stdout
        resumed = json.loads(resumed_out.stdout)
        assert resumed["session"]["session_id"] == relay["session_id"]

        fresh_out = runner.invoke(
            app,
            [
                "plan",
                "session",
                "join",
                planning_session["planning_session_id"],
                "--surface",
                "editor",
                "--no-resume-if-active",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert fresh_out.exit_code == 0, fresh_out.stdout
        fresh = json.loads(fresh_out.stdout)
        assert fresh["session"]["session_id"] != relay["session_id"]

        close_out = runner.invoke(
            app,
            ["plan", "session", "close", planning_session["planning_session_id"], "--json"],
            catch_exceptions=False,
        )
        assert close_out.exit_code == 0, close_out.stdout

        join_fail_out = runner.invoke(
            app,
            ["plan", "session", "join", planning_session["planning_session_id"], "--surface", "editor"],
            catch_exceptions=False,
        )
        assert join_fail_out.exit_code == 2
        assert "cannot accept relay joins" in (join_fail_out.output or join_fail_out.stdout)


def test_task_create_can_pin_plan_lineage_and_land_change_in_strict_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-first-strict-land"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-first-strict-land") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "strict", "--json"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/strict_land.md",
            "# Strict Plan Land\n\n## Bootstrap Durable Plan Storage [plan-ref: strict-land/bootstrap]\n\n- [ ] bootstrap native workflow [ref: milestone-1/bootstrap-native-workflow]\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "strict-land/bootstrap",
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
                "Land a plan-linked change",
                "--risk",
                "medium",
                "--plan",
                plan["plan_id"],
                "--plan-item-ref",
                "milestone-1/bootstrap-native-workflow",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert task_out.exit_code == 0, task_out.stdout
        task = json.loads(task_out.stdout)
        assert task["plan_id"] == plan["plan_id"]
        assert task["origin_plan_revision_id"] == plan["head_revision"]["plan_revision_id"]
        assert task["plan_item_ref"] == "milestone-1/bootstrap-native-workflow"
        assert task["plan_linked_at"]

        worktree_path = Path(task["worktree"]["path"])
        monkeypatch.chdir(worktree_path)
        (worktree_path / "app.py").write_text("print('plan-linked')\n", encoding="utf-8")
        feature_snap_out = runner.invoke(app, ["snapshot", "create", "--message", "plan linked work", "--json"])
        assert feature_snap_out.exit_code == 0, feature_snap_out.stdout

        change_out = runner.invoke(
            app,
            [
                "change",
                "create",
                "--task",
                task["task_id"],
                "--title",
                "Plan-linked landing change",
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

        patchset_out = runner.invoke(
            app,
            ["patchset", "publish", "--change", change["change_id"], "--summary", "plan linked patchset", "--json"],
            catch_exceptions=False,
        )
        assert patchset_out.exit_code == 0, patchset_out.stdout
        patchset = json.loads(patchset_out.stdout)

        attest_out = runner.invoke(app, ["attest", "put", patchset["patchset_id"], "--tests", "pass", "--json"])
        assert attest_out.exit_code == 0, attest_out.stdout

        review_out = runner.invoke(
            app,
            ["review", "approve", change["change_id"], "--patchset", patchset["patchset_id"], "--reviewer", "reviewer@example.com", "--json"],
            catch_exceptions=False,
        )
        assert review_out.exit_code == 0, review_out.stdout
        code_review_out = runner.invoke(
            app,
            [
                "review",
                "code",
                "submit",
                change["change_id"],
                "--patchset",
                patchset["patchset_id"],
                "--verdict",
                "pass",
                "--reviewer",
                "reviewer@example.com",
                "--message",
                "Reviewed files: app.py; Findings: no blocking findings; Risks: low; Tests: plan linked patchset attestation passed; Recommendation: safe to land.",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert code_review_out.exit_code == 0, code_review_out.stdout

        land_out = runner.invoke(
            app,
            ["land", "submit", change["change_id"], "--patchset", patchset["patchset_id"], "--target", "main", "--mode", "direct", "--json"],
            catch_exceptions=False,
        )
        assert land_out.exit_code == 0, land_out.stdout

        complete_out = runner.invoke(app, ["task", "complete", task["task_id"], "--json"])
        assert complete_out.exit_code == 0, complete_out.stdout
        completed = json.loads(complete_out.stdout)
        assert completed["status"] == "completed"


def test_task_start_can_pin_plan_lineage_in_strict_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-first-task-start"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-first-task-start") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "strict", "--json"]).exit_code == 0
        plan_file = _write_plan_artifact(
            repo,
            "docs/sprints/task_start_strict.md",
            "# Strict Task Start\n\n## Bootstrap Durable Plan Storage [plan-ref: strict-task-start/bootstrap]\n\n- [ ] bootstrap native workflow [ref: milestone-1/bootstrap-native-workflow]\n",
        )

        plan = _create_remote_plan_from_artifact(
            base_url,
            "housekeeper",
            (repo / plan_file).read_text(encoding="utf-8"),
            "strict-task-start/bootstrap",
            artifact_path=plan_file,
            title="Bootstrap durable plan storage",
            summary="Seed the first plan revision",
        )

        start_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Bootstrap native workflow",
                "--intent",
                "Open the task and first change from one plan-linked command",
                "--base-line",
                "main",
                "--risk",
                "medium",
                "--plan",
                plan["plan_id"],
                "--revision",
                plan["head_revision"]["plan_revision_id"],
                "--plan-item-ref",
                "milestone-1/bootstrap-native-workflow",
                "--json",
            ],
            catch_exceptions=False,
        )
        assert start_out.exit_code == 0, start_out.stdout
        payload = json.loads(start_out.stdout)
        assert payload["task_id"].startswith("RAITT-")
        assert payload["plan_id"] == plan["plan_id"]
        assert payload["origin_plan_revision_id"] == plan["head_revision"]["plan_revision_id"]
        assert payload["plan_item_ref"] == "milestone-1/bootstrap-native-workflow"
        assert payload["change"]["task_id"] == payload["task_id"]


def test_task_create_rejects_plan_linkage_for_local_drafts(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-local-plan-linkage"
    repo.mkdir()
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Local plan-linked task",
            "--intent",
            "Confirm plan linkage is remote-only for now",
            "--plan",
            "AITPL-0001",
            "--plan-item-ref",
            "milestone-1/bootstrap-native-workflow",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 2
    output = task_out.output or task_out.stdout
    assert "Unknown local plan" in output


def test_task_create_requires_plan_binding_in_required_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-first-required-create"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-first-required-create") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required", "--json"]).exit_code == 0

        missing_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--task-only",
                "--title",
                "Missing plan linkage",
                "--intent",
                "required mode should block execution tasks without a plan item",
            ],
            catch_exceptions=False,
        )
        assert missing_out.exit_code == 2
        assert "Required plan/task binding requires `--plan`" in (missing_out.output or missing_out.stdout)

def test_task_start_requires_plan_binding_in_required_mode(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-plan-first-required-start"
    repo.mkdir()
    (repo / "README.md").write_text("base\n", encoding="utf-8")

    with running_server(tmp_path / "server-data-plan-first-required-start") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper"]).exit_code == 0
        assert runner.invoke(app, ["remote", "add", "origin", base_url, "--repo-name", "housekeeper", "--default"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"]).exit_code == 0
        assert runner.invoke(app, ["snapshot", "create", "--message", "main seed"]).exit_code == 0
        assert runner.invoke(app, ["push", "--line", "main"]).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--plan-task-binding-mode", "required", "--json"]).exit_code == 0

        missing_out = runner.invoke(
            app,
            [
                "task",
                "start",
                "--title",
                "Missing plan linkage",
                "--intent",
                "required mode should block task start without a plan item",
                "--base-line",
                "main",
            ],
            catch_exceptions=False,
        )
        assert missing_out.exit_code == 2
        assert "Required plan/task binding requires `--plan`" in (missing_out.output or missing_out.stdout)

def test_server_store_task_can_link_parent_plan_revision(tmp_path: Path):
    server_data = tmp_path / "server-data-direct-plan-store"
    ctx = fake_postgres_context(server_data)
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main")
    artifact = _plan_artifact_payload(
        "# Direct Plan Store\n\n## Bootstrap Durable Plan Storage [plan-ref: direct-plan-store/bootstrap]\n\n- store plan records\n",
        "direct-plan-store/bootstrap",
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
        "Keep task linkage pointed at a durable plan revision",
        "medium",
        plan_id=plan["plan_id"],
    )

    assert task["plan_id"] == plan["plan_id"]
    assert task["origin_plan_revision_id"] == plan["head_revision"]["plan_revision_id"]
    assert task["tracking"]["session_id"]
    sessions = [
        row
        for row in server_store_module.list_sessions(ctx, "repo-a")
        if row["task_id"] == task["task_id"] and row["session_kind"] == "task_run"
    ]
    assert len(sessions) == 1
    assert sessions[0]["session_id"] == task["tracking"]["session_id"]


def test_server_store_create_task_accepts_explicit_plan_revision(tmp_path: Path):
    server_data = tmp_path / "server-data-direct-plan-revision"
    ctx = fake_postgres_context(server_data)
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main")
    create_artifact = _plan_artifact_payload(
        "# Direct Plan Revision\n\n## Bootstrap Durable Plan Storage [plan-ref: direct-plan-revision/bootstrap]\n\n- store plan records\n",
        "direct-plan-revision/bootstrap",
    )
    revise_artifact = _plan_artifact_payload(
        "# Direct Plan Revision\n\n## Bootstrap Durable Plan Storage [plan-ref: direct-plan-revision/bootstrap]\n\n- store plan records\n- keep task lineage pinned\n",
        "direct-plan-revision/bootstrap",
    )

    plan = server_store_module.create_plan(
        ctx,
        "repo-a",
        "Bootstrap durable plan storage",
        create_artifact["artifact_path"],
        create_artifact["artifact_selector"],
        create_artifact["artifact_heading"],
        create_artifact["items"],
        summary="seed",
    )
    revised = server_store_module.revise_plan(
        ctx,
        plan["plan_id"],
        revise_artifact["artifact_path"],
        revise_artifact["artifact_selector"],
        revise_artifact["artifact_heading"],
        revise_artifact["items"],
        summary="revise",
    )
    task = server_store_module.create_task(
        ctx,
        "repo-a",
        "Plan-linked work",
        "Keep task linkage pointed at an explicit plan revision",
        "medium",
        plan_id=plan["plan_id"],
        origin_plan_revision_id=plan["head_revision"]["plan_revision_id"],
    )

    assert revised["head_revision"]["plan_revision_id"] != plan["head_revision"]["plan_revision_id"]
    assert task["plan_id"] == plan["plan_id"]
    assert task["origin_plan_revision_id"] == plan["head_revision"]["plan_revision_id"]
    assert task["tracking"]["session_id"]
    sessions = [
        row
        for row in server_store_module.list_sessions(ctx, "repo-a")
        if row["task_id"] == task["task_id"] and row["session_kind"] == "task_run"
    ]
    assert len(sessions) == 1
    assert sessions[0]["metadata"]["objective"] == "Keep task linkage pointed at an explicit plan revision"


def test_server_store_close_task_closes_server_tracking_session(tmp_path: Path):
    server_data = tmp_path / "server-data-direct-task-close"
    ctx = fake_postgres_context(server_data)
    server_store_module.initialize(ctx)
    connect_server_content(ctx)
    server_store_module.ensure_repository(ctx, "repo-a", "main")
    task = server_store_module.create_task(
        ctx,
        "repo-a",
        "Close tracked task",
        "Ensure task_run sessions follow task closure",
        "medium",
    )
    change = server_store_module.create_change(ctx, "repo-a", task["task_id"], "Land it", "main", "medium")
    with connect(ctx) as conn:
        conn.execute("update changes set status = 'landed' where change_id = ?", (change["change_id"],))
        conn.commit()

    closed = server_store_module.close_task(ctx, task["task_id"], "completed")

    assert closed["status"] == "completed"
    sessions = [
        row
        for row in server_store_module.list_sessions(ctx, "repo-a")
        if row["task_id"] == task["task_id"] and row["session_kind"] == "task_run"
    ]
    assert len(sessions) == 1
    assert sessions[0]["status"] == "completed"
