from __future__ import annotations

from ._shared import *  # noqa: F401,F403


def _write_local_plan(repo: Path, filename: str, *, plan_ref: str, item_ref: str, title: str) -> tuple[Path, str]:
    plan_dir = repo / "docs" / "sprints"
    plan_dir.mkdir(parents=True, exist_ok=True)
    plan_file = plan_dir / filename
    plan_file.write_text(
        f"# {title}\n\n## Workflow [plan-ref: {plan_ref}]\n\n- [ ] {title} [ref: {item_ref}]\n",
        encoding="utf-8",
    )
    sync_out = runner.invoke(app, ["plan", "sync", str(plan_file), "--json"], catch_exceptions=False)
    assert sync_out.exit_code == 0, sync_out.stdout
    synced = json.loads(sync_out.stdout)
    return plan_file, synced["results"][0]["plan_id"]


def test_snapshot_diff_command_emits_json_with_text_diff(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-snapshot-diff"
    repo.mkdir()
    readme = repo / "app.py"
    monkeypatch.chdir(repo)

    readme.write_text("hello\n", encoding="utf-8")
    assert runner.invoke(app, ["init", "--name", "housekeeper-snapshot-diff"], catch_exceptions=False).exit_code == 0
    first_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert first_out.exit_code == 0, first_out.stdout
    first_snapshot = json.loads(first_out.stdout)

    readme.write_text("hello aitk\n", encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "update readme", "--json"], catch_exceptions=False)
    assert second_out.exit_code == 0, second_out.stdout
    second_snapshot = json.loads(second_out.stdout)

    diff_out = runner.invoke(
        app,
        [
            "snapshot",
            "diff",
            first_snapshot["snapshot_id"],
            second_snapshot["snapshot_id"],
            "--include-text",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert diff_out.exit_code == 0, diff_out.stdout
    payload = json.loads(diff_out.stdout)
    assert payload["old_snapshot_id"] == first_snapshot["snapshot_id"]
    assert payload["new_snapshot_id"] == second_snapshot["snapshot_id"]
    assert payload["modified"] == ["app.py"]
    assert payload["summary"]["files_changed"] == 1
    assert payload["summary"]["insertions"] == 1
    assert payload["summary"]["deletions"] == 1
    assert payload["files"][0]["diff"]["status"] == "text"
    assert "-hello" in payload["files"][0]["diff"]["text"]
    assert "+hello aitk" in payload["files"][0]["diff"]["text"]


def test_gc_validate_returns_condensed_storage_validation_view(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-gc-validate"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-gc-validate"], catch_exceptions=False).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout

    validate_out = runner.invoke(app, ["gc", "validate", "--json"], catch_exceptions=False)
    assert validate_out.exit_code == 0, validate_out.stdout
    payload = json.loads(validate_out.stdout)
    assert payload["state"] == "packed_full_only"
    assert payload["recommended_action"] == "repack"
    assert payload["next_actions"] == ["repack"]
    assert payload["packed_blob_count"] >= 1
    assert payload["pack_count"] == 1


def test_gc_stats_reports_first_wave_schema_cleanup_audit(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-gc-audit"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-gc-audit"], catch_exceptions=False).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)

    content_db = repo / ".ait" / "content.db"
    conn = sqlite3.connect(content_db)
    conn.row_factory = sqlite3.Row
    conn.execute("create table repositories (repo_name text primary key)")
    conn.execute("create table repository_groups (group_id text primary key)")
    conn.execute("create table repository_group_memberships (repo_name text primary key)")
    snapshot_row = conn.execute(
        "select root_tree_id from snapshots where snapshot_id = ?",
        (snapshot["snapshot_id"],),
    ).fetchone()
    assert snapshot_row is not None
    root_tree_id = snapshot_row["root_tree_id"]
    conn.execute(
        "update snapshots set manifest_path = ? where snapshot_id = ?",
        (f".ait/objects/tree-packs/TPK-STALE.zip#trees/{root_tree_id}.json", snapshot["snapshot_id"]),
    )
    conn.commit()
    conn.close()

    stats_out = runner.invoke(app, ["gc", "stats", "--json"], catch_exceptions=False)
    assert stats_out.exit_code == 0, stats_out.stdout
    payload = json.loads(stats_out.stdout)
    audit = payload["schema_cleanup_summary"]
    assert audit["schema_version"] == 3
    assert audit["legacy_local_server_catalog_tables_present"] == [
        "repository_group_memberships",
        "repository_groups",
        "repositories",
    ]
    assert audit["legacy_local_server_catalog_table_count"] == 3
    assert audit["legacy_pack_metadata_columns_present"] == []
    assert audit["legacy_pack_metadata_column_count"] == 0
    assert audit["stale_manifest_count"] == 1
    assert audit["first_wave_schema_cleanup_applied"] is False


def test_repo_validate_returns_condensed_storage_validation_view(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-repo-validate"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")

    with running_server(tmp_path / "server-repo-validate") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-repo-validate"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-repo-validate", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        assert runner.invoke(app, ["config", "set", "--workflow-mode", "solo_remote"], catch_exceptions=False).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        validate_out = runner.invoke(app, ["repo", "validate", "--json"], catch_exceptions=False)
        assert validate_out.exit_code == 0, validate_out.stdout
        payload = json.loads(validate_out.stdout)
        assert payload["state"] == "packed_full_only"
        assert payload["recommended_action"] == "repack"
        assert payload["next_actions"] == ["repack"]
        assert payload["packed_blob_count"] >= 1
        assert payload["pack_count"] == 1


def test_gc_optimize_can_pack_repack_and_gc_local_storage(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-gc-optimize"
    repo.mkdir()
    readme = repo / "app.py"
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper-gc-optimize"], catch_exceptions=False).exit_code == 0
    base_text = "".join(f"line {i:02d} keep same text for compression\n" for i in range(20))
    updated_lines = [f"line {i:02d} keep same text for compression\n" for i in range(20)]
    updated_lines[10] = "line 10 changed text for compression\n"
    updated_lines.append("line 20 keep same text for compression\n")
    updated_text = "".join(updated_lines)

    readme.write_text(base_text, encoding="utf-8")
    first_out = runner.invoke(app, ["snapshot", "create", "--message", "base", "--json"], catch_exceptions=False)
    assert first_out.exit_code == 0, first_out.stdout

    first_pack = runner.invoke(app, ["gc", "pack", "--json"], catch_exceptions=False)
    assert first_pack.exit_code == 0, first_pack.stdout

    readme.write_text(updated_text, encoding="utf-8")
    second_out = runner.invoke(app, ["snapshot", "create", "--message", "update", "--json"], catch_exceptions=False)
    assert second_out.exit_code == 0, second_out.stdout

    optimize_out = runner.invoke(app, ["gc", "optimize", "--json"], catch_exceptions=False)
    assert optimize_out.exit_code == 0, optimize_out.stdout
    payload = json.loads(optimize_out.stdout)
    assert payload["repo_name"] == "housekeeper-gc-optimize"
    assert payload["executed_step_count"] == 2
    assert [step["action"] for step in payload["steps"]] == ["repack", "gc"]
    assert payload["initial_storage"]["validation_summary"]["state"] == "partially_optimized"
    assert payload["initial_storage"]["validation_summary"]["recommended_action"] == "repack"
    assert payload["final_storage"]["pack_count"] == 1
    assert payload["final_storage"]["packed_delta_blob_count"] >= 1
    assert payload["final_storage"]["validation_summary"]["state"] == "delta_optimized"
    assert payload["final_storage"]["validation_summary"]["recommended_action"] == "none"


def test_put_snapshot_fails_if_bundle_repository_does_not_match_request(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper"
    repo.mkdir()
    (repo / "app.py").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
    assert snap_out.exit_code == 0, snap_out.stdout
    snapshot = json.loads(snap_out.stdout)
    bundle = export_snapshot_bundle(RepoContext.discover(repo), snapshot["snapshot_id"])

    with running_server(tmp_path / "server-data-bundle-mismatch") as base_url:
        headers = {"Accept": "application/json", "Content-Type": "application/json"}
        create_req = urllib.request.Request(
            f"{base_url}/v1/native/repositories",
            data=json.dumps({"repo_name": "other-repo", "default_line": "main"}).encode("utf-8"),
            headers=headers,
            method="POST",
        )
        with urllib.request.urlopen(create_req, timeout=5) as resp:
            assert resp.status == 200

        push_req = urllib.request.Request(
            f"{base_url}/v1/native/repositories/other-repo/snapshots/{snapshot['snapshot_id']}",
            data=json.dumps(bundle).encode("utf-8"),
            headers=headers,
            method="PUT",
        )
        try:
            urllib.request.urlopen(push_req, timeout=5)
            raise AssertionError("expected remote snapshot upload to fail")
        except urllib.error.HTTPError as exc:
            assert exc.code == 400
            payload = exc.read().decode("utf-8", errors="replace")
            assert "repository mismatch" in payload
            assert "housekeeper" in payload
            assert "other-repo" in payload


def test_snapshot_create_help_describes_immutable_revision_role():
    help_out = runner.invoke(app, ["snapshot", "create", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Freeze the current workspace line head as an immutable snapshot" in help_out.stdout
    assert "revision." in help_out.stdout


def test_snapshot_group_help_lists_inventory_and_compare_roles():
    help_out = runner.invoke(app, ["snapshot", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "List immutable snapshots on the current line." in help_out.stdout
    assert "Inspect one immutable snapshot." in help_out.stdout
    assert "Compare two immutable snapshots." in help_out.stdout


def test_snapshot_list_help_describes_revision_history_role():
    help_out = runner.invoke(app, ["snapshot", "list", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "List immutable snapshots on the current line" in help_out.stdout
    assert "show or diff." in help_out.stdout


def test_snapshot_show_help_describes_manifest_inspection_role():
    help_out = runner.invoke(app, ["snapshot", "show", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Inspect one immutable snapshot" in help_out.stdout
    assert "parent, and" in help_out.stdout
    assert "message metadata." in help_out.stdout


def test_snapshot_diff_help_describes_revision_comparison_role():
    help_out = runner.invoke(app, ["snapshot", "diff", "--help"], catch_exceptions=False)
    assert help_out.exit_code == 0, help_out.stdout
    assert "Compare two immutable snapshots" in help_out.stdout
    assert "changed between" in help_out.stdout
    assert "revisions." in help_out.stdout


def test_local_draft_commands_recreate_missing_workflow_tables_for_existing_repo(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-existing-control-db"
    repo.mkdir()
    (repo / "app.py").write_text("print('base')\n", encoding="utf-8")
    monkeypatch.chdir(repo)

    assert runner.invoke(app, ["init", "--name", "housekeeper"], catch_exceptions=False).exit_code == 0
    _, plan_id = _write_local_plan(
        repo,
        "workflow_schema_backfill.md",
        plan_ref="workflow-schema-backfill",
        item_ref="workflow-schema-backfill/recreate-missing-workflow-tables",
        title="Workflow schema backfill",
    )

    control_db = repo / ".ait" / "control.db"
    conn = sqlite3.connect(control_db)
    conn.execute("drop table workflow_changes")
    conn.execute("drop table workflow_tasks")
    conn.commit()
    conn.close()

    task_out = runner.invoke(
        app,
        [
            "task",
            "start",
            "--task-only",
            "--local",
            "--title",
            "Backfill missing schema",
            "--intent",
            "ensure workflow tables are recreated",
            "--risk",
            "medium",
            "--plan",
            plan_id,
            "--plan-item-ref",
            "workflow-schema-backfill/recreate-missing-workflow-tables",
            "--json",
        ],
        catch_exceptions=False,
    )
    assert task_out.exit_code == 0, task_out.stdout
    task = json.loads(task_out.stdout)
    monkeypatch.chdir(Path(task["worktree"]["path"]))

    task_list_out = runner.invoke(app, ["task", "list", "--local", "--json"], catch_exceptions=False)
    assert task_list_out.exit_code == 0, task_list_out.stdout
    assert [row["task_id"] for row in json.loads(task_list_out.stdout)] == [task["task_id"]]

    change_out = runner.invoke(
        app,
        ["change", "create", "--local", "--task", task["task_id"], "--title", "Backfill change schema", "--base-line", "main", "--risk", "medium", "--json"],
        catch_exceptions=False,
    )
    assert change_out.exit_code == 0, change_out.stdout
    change = json.loads(change_out.stdout)

    change_list_out = runner.invoke(app, ["change", "list", "--local", "--json"], catch_exceptions=False)
    assert change_list_out.exit_code == 0, change_list_out.stdout
    assert [row["change_id"] for row in json.loads(change_list_out.stdout)] == [change["change_id"]]


def test_server_store_export_snapshot_falls_back_for_legacy_content_export(monkeypatch):
    def legacy_export_snapshot(_ctx, _repo_name, snapshot_id):
        return {
            "snapshot_id": snapshot_id,
            "repo_name": "housekeeper",
            "files": [
                {
                    "path": "app.py",
                    "blob_id": "BLB-1",
                    "size_bytes": 5,
                    "mode": "0o644",
                    "sha256": "abc",
                    "content_b64": "YmFzZQo=",
                },
                {
                    "path": "notes.txt",
                    "blob_id": "BLB-2",
                    "size_bytes": 6,
                    "mode": "0o644",
                    "sha256": "def",
                    "content_b64": "bm90ZXMK",
                }
            ],
        }

    monkeypatch.setattr(server_store_module, "export_content_snapshot", legacy_export_snapshot)

    full_bundle = server_store_module.export_snapshot(object(), "housekeeper", "SNP-LEGACY")
    assert full_bundle["content_included"] is True
    assert full_bundle["files"][0]["content_b64"] == "YmFzZQo="

    metadata_bundle = server_store_module.export_snapshot(object(), "housekeeper", "SNP-LEGACY", include_content=False)
    assert metadata_bundle["content_included"] is False
    assert metadata_bundle["files"][0]["path"] == "app.py"
    assert "content_b64" not in metadata_bundle["files"][0]
    assert "content_b64" not in metadata_bundle["files"][1]

    path_bundle = server_store_module.export_snapshot(
        object(),
        "housekeeper",
        "SNP-LEGACY",
        include_content=False,
        path="notes.txt",
    )
    assert path_bundle["content_included"] is False
    assert [row["path"] for row in path_bundle["files"]] == ["notes.txt"]
    assert "content_b64" not in path_bundle["files"][0]


def test_remote_snapshot_metadata_export_skips_blob_reads_and_can_filter_one_path(tmp_path: Path, monkeypatch):
    repo = tmp_path / "housekeeper-remote-snapshot-metadata"
    repo.mkdir()
    (repo / "app.py").write_text("print('hello')\n", encoding="utf-8")
    (repo / "notes.txt").write_text("notes\n", encoding="utf-8")

    with running_server(tmp_path / "server-remote-snapshot-metadata") as base_url:
        monkeypatch.chdir(repo)
        assert runner.invoke(app, ["init", "--name", "housekeeper-remote-snapshot-metadata"], catch_exceptions=False).exit_code == 0
        assert runner.invoke(
            app,
            ["remote", "add", "origin", base_url, "--repo-name", "housekeeper-remote-snapshot-metadata", "--default"],
            catch_exceptions=False,
        ).exit_code == 0
        snap_out = runner.invoke(app, ["snapshot", "create", "--message", "seed", "--json"], catch_exceptions=False)
        assert snap_out.exit_code == 0, snap_out.stdout
        snapshot_id = str(json.loads(snap_out.stdout)["snapshot_id"])
        assert runner.invoke(app, ["push", "--line", "main"], catch_exceptions=False).exit_code == 0

        full_bundle = remote_client_module.get_remote_snapshot(
            base_url,
            "housekeeper-remote-snapshot-metadata",
            snapshot_id,
        )
        assert full_bundle["content_included"] is True
        assert [row["path"] for row in full_bundle["files"]] == ["app.py", "notes.txt"]
        assert all("content_b64" in row for row in full_bundle["files"])

        import ait_server.server_content as server_content_module

        def fail_blob_read(*_args, **_kwargs):
            raise AssertionError("_blob_bytes_by_row should not run for metadata-only snapshot export")

        monkeypatch.setattr(server_content_module, "_blob_bytes_by_row", fail_blob_read)

        metadata_bundle = remote_client_module.get_remote_snapshot(
            base_url,
            "housekeeper-remote-snapshot-metadata",
            snapshot_id,
            include_content=False,
        )
        assert metadata_bundle["content_included"] is False
        assert [row["path"] for row in metadata_bundle["files"]] == ["app.py", "notes.txt"]
        assert all("content_b64" not in row for row in metadata_bundle["files"])

        path_bundle = remote_client_module.get_remote_snapshot(
            base_url,
            "housekeeper-remote-snapshot-metadata",
            snapshot_id,
            include_content=False,
            path="notes.txt",
        )
        assert path_bundle["content_included"] is False
        assert [row["path"] for row in path_bundle["files"]] == ["notes.txt"]
        assert "content_b64" not in path_bundle["files"][0]

        metadata_map = {row["path"]: row["blob_id"] for row in metadata_bundle["files"]}
        assert path_bundle["files"][0]["blob_id"] == metadata_map["notes.txt"]
