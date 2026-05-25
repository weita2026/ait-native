from __future__ import annotations

import base64
from pathlib import Path
import sqlite3

import pytest

from ait import local_content
from ait import local_content_bundle as local_content_bundle_helpers
from ait import local_content_workspace as local_content_workspace_helpers
from ait import snapshot_blame
from ait import store


def test_local_content_workspace_helpers_match_facade(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / ".aitignore").write_text("generated/\n", encoding="utf-8")
    (repo / "generated").mkdir()
    (repo / "generated" / "artifact.txt").write_text("ignored", encoding="utf-8")

    helper_diag = local_content_workspace_helpers.workspace_runtime_root_hygiene(
        repo,
        runtime_root=repo / ".ait-server",
    )

    assert helper_diag["state"] == "warn"
    assert helper_diag["inside_repo"] is True
    assert helper_diag["snapshot_ignored"] is True
    assert local_content_workspace_helpers.workspace_path_is_ignored(repo, "generated/artifact.txt") is True
    assert local_content.workspace_path_is_ignored(repo, "generated/artifact.txt") is True
    assert local_content.workspace_runtime_root_hygiene(repo, runtime_root=repo / ".ait-server") == helper_diag
    assert local_content.workspace_ignore_policy(repo) == local_content_workspace_helpers.workspace_ignore_policy(repo)


def test_local_content_workspace_internal_helpers_match_facade(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / ".aitignore").write_text("generated/\n", encoding="utf-8")
    (repo / "generated").mkdir()
    (repo / "generated" / "artifact.txt").write_text("ignored", encoding="utf-8")
    (repo / "docs").mkdir()
    (repo / "docs" / "plan.md").write_text("plan\n", encoding="utf-8")

    state = local_content_workspace_helpers._workspace_state(repo)
    snapshot_rules = local_content_workspace_helpers._snapshot_workspace_ignore_rules(
        "SNP-EXAMPLE",
        {".aitignore": {"blob_id": "BLB-EXAMPLE"}},
        lambda blob_id: "generated/\n" if blob_id == "BLB-EXAMPLE" else "",
    )

    assert "generated/artifact.txt" not in state
    assert state["docs/plan.md"]["path"] == "docs/plan.md"
    assert snapshot_rules[0].pattern == "generated"
    assert local_content_workspace_helpers._normalize_workspace_restore_path(r"docs\plan.md") == "docs/plan.md"
    assert local_content._workspace_state is local_content_workspace_helpers._workspace_state
    assert local_content._snapshot_workspace_ignore_rules is local_content_workspace_helpers._snapshot_workspace_ignore_rules
    assert local_content._normalize_workspace_restore_path is local_content_workspace_helpers._normalize_workspace_restore_path


def test_workspace_visible_files_prunes_builtin_operational_dirs_during_walk(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo = tmp_path / "repo"
    (repo / "docs").mkdir(parents=True, exist_ok=True)
    (repo / "nested").mkdir(parents=True, exist_ok=True)
    (repo / ".ait").mkdir(parents=True, exist_ok=True)
    (repo / "__pycache__").mkdir(parents=True, exist_ok=True)
    (repo / "root.txt").write_text("root\n", encoding="utf-8")
    (repo / "docs" / "plan.md").write_text("plan\n", encoding="utf-8")
    (repo / "nested" / "keep.txt").write_text("keep\n", encoding="utf-8")
    (repo / ".ait" / "internal.txt").write_text("ignore\n", encoding="utf-8")
    (repo / "__pycache__" / "cache.pyc").write_bytes(b"cache")

    observed: dict[str, list[str]] = {}

    def fake_walk(_root: Path, topdown: bool = False, followlinks: bool = True):
        assert _root == repo
        assert topdown is True
        assert followlinks is False
        dirnames = [".ait", "__pycache__", "docs", "nested"]
        filenames = ["root.txt", ".DS_Store"]
        yield repo, dirnames, filenames
        observed["pruned_dirnames"] = list(dirnames)
        if "docs" in dirnames:
            yield repo / "docs", [], ["plan.md"]
        if "nested" in dirnames:
            yield repo / "nested", [], ["keep.txt"]
        if ".ait" in dirnames or "__pycache__" in dirnames:
            raise AssertionError("built-in operational dirs should be pruned before traversal continues")

    monkeypatch.setattr(local_content_workspace_helpers.os, "walk", fake_walk)

    visible = local_content_workspace_helpers._workspace_visible_files(repo)

    assert observed["pruned_dirnames"] == ["docs", "nested"]
    assert [path.relative_to(repo).as_posix() for path in visible] == [
        "root.txt",
        "docs/plan.md",
        "nested/keep.txt",
    ]


def test_workspace_restore_path_normalization_rejects_escape_routes() -> None:
    with pytest.raises(ValueError):
        local_content_workspace_helpers._normalize_workspace_restore_path("../secrets.txt")

    with pytest.raises(ValueError):
        local_content_workspace_helpers._normalize_workspace_restore_path("/abs/path.txt")


def test_local_content_bundle_helpers_round_trip_between_repos(tmp_path: Path) -> None:
    source_repo = tmp_path / "source"
    source_repo.mkdir(parents=True, exist_ok=True)
    source_ctx = store.init_repo(source_repo, "repo", "main")
    (source_repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")

    snapshot = local_content.create_snapshot(source_ctx, "repo", "main", "first snapshot")
    bundle = local_content_bundle_helpers.export_snapshot_bundle(source_ctx, snapshot["snapshot_id"], "repo")

    target_repo = tmp_path / "target"
    target_repo.mkdir(parents=True, exist_ok=True)
    target_ctx = store.init_repo(target_repo, "repo", "main")
    imported = local_content_bundle_helpers.import_snapshot_bundle(target_ctx, bundle)

    assert imported["snapshot_id"] == snapshot["snapshot_id"]
    assert imported["root_tree_id"] == snapshot["root_tree_id"]
    assert local_content.collect_snapshot_chain(target_ctx, snapshot["snapshot_id"]) == [
        snapshot["snapshot_id"]
    ]
    assert local_content.ensure_snapshot_chain(target_ctx, [bundle])[0]["snapshot_id"] == snapshot["snapshot_id"]


def test_local_content_bundle_export_derives_missing_blob_pack_entry_name(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "seed")

    conn = sqlite3.connect(ctx.content_db_path)
    blob_columns = {row[1] for row in conn.execute("pragma table_info(blobs)").fetchall()}
    conn.close()

    assert "pack_entry_name" not in blob_columns
    bundle = local_content_bundle_helpers.export_snapshot_bundle(ctx, snapshot["snapshot_id"], "repo")
    assert bundle["snapshot_id"] == snapshot["snapshot_id"]
    restored = base64.b64decode(bundle["files"][0]["content_b64"]).decode("utf-8")
    assert restored == "alpha\n"


def test_local_writers_stop_persisting_deterministic_pack_entry_names(tmp_path: Path) -> None:
    source_repo = tmp_path / "source"
    source_repo.mkdir(parents=True, exist_ok=True)
    (source_repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    source_ctx = store.init_repo(source_repo, "repo", "main")

    first = local_content.create_snapshot(source_ctx, "repo", "main", "seed")
    (source_repo / "alpha.txt").write_text("alpha updated\n", encoding="utf-8")
    second = local_content.create_snapshot(source_ctx, "repo", "main", "update")

    conn = sqlite3.connect(source_ctx.content_db_path)
    source_blob_columns = {row[1] for row in conn.execute("pragma table_info(blobs)").fetchall()}
    source_tree_columns = {row[1] for row in conn.execute("pragma table_info(trees)").fetchall()}
    conn.close()

    assert first["snapshot_id"] != second["snapshot_id"]
    assert "pack_entry_name" not in source_blob_columns
    assert "tree_pack_entry_name" not in source_tree_columns

    repack = local_content.create_pack(source_ctx, repack=True)
    assert repack["created"] is True

    bundle = local_content_bundle_helpers.export_snapshot_bundle(source_ctx, second["snapshot_id"], "repo")

    target_repo = tmp_path / "target"
    target_repo.mkdir(parents=True, exist_ok=True)
    target_ctx = store.init_repo(target_repo, "repo", "main")
    imported = local_content_bundle_helpers.import_snapshot_bundle(target_ctx, bundle)

    conn = sqlite3.connect(target_ctx.content_db_path)
    target_blob_columns = {row[1] for row in conn.execute("pragma table_info(blobs)").fetchall()}
    target_tree_columns = {row[1] for row in conn.execute("pragma table_info(trees)").fetchall()}
    conn.close()

    assert imported["root_tree_id"]
    assert "pack_entry_name" not in target_blob_columns
    assert "tree_pack_entry_name" not in target_tree_columns


def test_local_writers_stop_persisting_deterministic_packed_timestamps(tmp_path: Path) -> None:
    source_repo = tmp_path / "source-packed-at"
    source_repo.mkdir(parents=True, exist_ok=True)
    (source_repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    source_ctx = store.init_repo(source_repo, "repo", "main")

    snapshot = local_content.create_snapshot(source_ctx, "repo", "main", "seed")

    conn = sqlite3.connect(source_ctx.content_db_path)
    source_blob_columns = {row[1] for row in conn.execute("pragma table_info(blobs)").fetchall()}
    source_tree_columns = {row[1] for row in conn.execute("pragma table_info(trees)").fetchall()}
    conn.close()

    assert "packed_at" not in source_blob_columns
    assert "tree_packed_at" not in source_tree_columns

    repack = local_content.create_pack(source_ctx, repack=True)
    assert repack["created"] is True

    bundle = local_content_bundle_helpers.export_snapshot_bundle(source_ctx, snapshot["snapshot_id"], "repo")

    target_repo = tmp_path / "target-packed-at"
    target_repo.mkdir(parents=True, exist_ok=True)
    target_ctx = store.init_repo(target_repo, "repo", "main")
    imported = local_content_bundle_helpers.import_snapshot_bundle(target_ctx, bundle)

    conn = sqlite3.connect(target_ctx.content_db_path)
    target_blob_columns = {row[1] for row in conn.execute("pragma table_info(blobs)").fetchall()}
    target_tree_columns = {row[1] for row in conn.execute("pragma table_info(trees)").fetchall()}
    conn.close()

    assert imported["root_tree_id"]
    assert "packed_at" not in target_blob_columns
    assert "tree_packed_at" not in target_tree_columns


def test_local_reduced_schema_omits_tree_entry_blob_size_column_for_new_and_imported_snapshots(tmp_path: Path) -> None:
    source_repo = tmp_path / "source-tree-entry-size"
    source_repo.mkdir(parents=True, exist_ok=True)
    (source_repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    source_ctx = store.init_repo(source_repo, "repo", "main")

    snapshot = local_content.create_snapshot(source_ctx, "repo", "main", "seed")

    conn = sqlite3.connect(source_ctx.content_db_path)
    source_columns = {row[1] for row in conn.execute("pragma table_info(tree_entries)").fetchall()}
    conn.close()

    assert "size_bytes" not in source_columns

    bundle = local_content_bundle_helpers.export_snapshot_bundle(source_ctx, snapshot["snapshot_id"], "repo")

    target_repo = tmp_path / "target-tree-entry-size"
    target_repo.mkdir(parents=True, exist_ok=True)
    target_ctx = store.init_repo(target_repo, "repo", "main")
    imported = local_content_bundle_helpers.import_snapshot_bundle(target_ctx, bundle)

    conn = sqlite3.connect(target_ctx.content_db_path)
    target_columns = {row[1] for row in conn.execute("pragma table_info(tree_entries)").fetchall()}
    conn.close()

    assert imported["root_tree_id"]
    assert "size_bytes" not in target_columns


def test_local_reads_derive_tree_entry_blob_sizes_without_stored_column(tmp_path: Path) -> None:
    repo = tmp_path / "repo-tree-size"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "seed")
    root_tree_id = snapshot["root_tree_id"]

    conn = sqlite3.connect(ctx.content_db_path)
    conn.row_factory = sqlite3.Row
    columns = {row["name"] for row in conn.execute("pragma table_info(tree_entries)").fetchall()}
    view_row = conn.execute(
        "select size_bytes from snapshot_files where snapshot_id = ? and path = 'alpha.txt'",
        (snapshot["snapshot_id"],),
    ).fetchone()
    tree_pack_rows = local_content._tree_pack_entry_rows(conn, [root_tree_id])
    blame_row = snapshot_blame._tree_entry_row(conn, root_tree_id, "alpha.txt", cache={})
    conn.close()

    assert "size_bytes" not in columns
    assert view_row is not None
    assert view_row["size_bytes"] == len("alpha\n")
    assert blame_row is not None
    assert blame_row["size_bytes"] == len("alpha\n")
    assert next(row for row in tree_pack_rows if row["entry_name"] == "alpha.txt")["size_bytes"] == len("alpha\n")
    assert local_content.get_snapshot(ctx, snapshot["snapshot_id"])["files"][0]["size_bytes"] == len("alpha\n")


def test_local_manifest_path_and_storage_stats_derive_missing_tree_pack_entry_name(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "seed")
    root_tree_id = snapshot["root_tree_id"]

    conn = sqlite3.connect(ctx.content_db_path)
    conn.row_factory = sqlite3.Row
    tree_columns = {row["name"] for row in conn.execute("pragma table_info(trees)").fetchall()}
    pack_row = conn.execute(
        """
        select tp.pack_path
        from trees t
        join tree_packs tp on tp.pack_id = t.tree_pack_id
        where t.tree_id = ?
        """,
        (root_tree_id,),
    ).fetchone()
    derived_manifest_path = local_content._manifest_path_for_tree(conn, root_tree_id)
    conn.close()

    assert pack_row is not None
    assert "tree_pack_entry_name" not in tree_columns
    assert derived_manifest_path == f"{pack_row['pack_path']}#trees/{root_tree_id}.json"
    assert local_content.storage_stats(ctx)["schema_cleanup_summary"]["stale_manifest_count"] == 0


def test_local_content_operations_skip_schema_migration_after_init(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    def fail_initialize_schema(*_args, **_kwargs):
        raise AssertionError("content schema bootstrap should only run during `ait init`")

    def fail_snapshot_metadata_migration(*_args, **_kwargs):
        raise AssertionError("snapshot metadata migration should only run during `ait init`")

    monkeypatch.setattr(local_content, "_initialize_schema", fail_initialize_schema)
    monkeypatch.setattr(local_content, "_migrate_snapshot_metadata", fail_snapshot_metadata_migration)

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "seed")
    assert local_content.get_line(ctx, "main")["head_snapshot_id"] == snapshot["snapshot_id"]
    assert local_content.list_lines(ctx)[0]["line_name"] == "main"
    assert local_content.snapshot_exists(ctx, snapshot["snapshot_id"]) is True
    assert local_content.get_snapshot(ctx, snapshot["snapshot_id"])["snapshot_id"] == snapshot["snapshot_id"]
    assert local_content.list_snapshots(ctx)[0]["snapshot_id"] == snapshot["snapshot_id"]
    assert local_content.workspace_delta(ctx, snapshot["snapshot_id"])["clean"] is True
    assert local_content.storage_stats(ctx)["snapshot_count"] >= 1
    assert local_content.ensure_blob_bytes(ctx, b"blob\n", path_hint="blob.txt").startswith("BLB-")
    assert (
        local_content_bundle_helpers.export_snapshot_bundle(ctx, snapshot["snapshot_id"], "repo")["snapshot_id"]
        == snapshot["snapshot_id"]
    )
    assert local_content_bundle_helpers.collect_snapshot_chain(ctx, snapshot["snapshot_id"]) == [snapshot["snapshot_id"]]


def test_local_server_catalog_cleanup_does_not_run_during_normal_content_operations(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    conn = sqlite3.connect(ctx.content_db_path)
    conn.execute("create table repositories (repo_name text primary key)")
    conn.execute("create table repository_groups (group_id text primary key)")
    conn.execute("create table repository_group_memberships (repo_name text primary key)")
    conn.commit()
    conn.close()

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "seed")
    assert snapshot["snapshot_id"].startswith("SNP-")

    conn = sqlite3.connect(ctx.content_db_path)
    rows = conn.execute(
        "select name from sqlite_master where type = 'table' and name in ('repositories', 'repository_groups', 'repository_group_memberships') order by name"
    ).fetchall()
    conn.close()
    assert [row[0] for row in rows] == [
        "repositories",
        "repository_group_memberships",
        "repository_groups",
    ]


def test_snapshot_file_map_uses_tree_traversal_for_tree_backed_snapshots(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "nested").mkdir(parents=True, exist_ok=True)
    (repo / "alpha.txt").write_text("alpha\n", encoding="utf-8")
    (repo / "nested" / "beta.txt").write_text("beta\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "seed")

    def fail_via_view(*_args, **_kwargs):
        raise AssertionError("tree-backed snapshots should not hit the snapshot_files view")

    monkeypatch.setattr(local_content, "_snapshot_file_map_via_view", fail_via_view)

    conn = local_content.connect_sqlite(ctx.content_db_path)
    try:
        file_map = local_content._snapshot_file_map(conn, snapshot["snapshot_id"])
    finally:
        conn.close()

    assert sorted(file_map) == ["alpha.txt", "nested/beta.txt"]
    assert file_map["nested/beta.txt"]["path"] == "nested/beta.txt"
    assert file_map["nested/beta.txt"]["blob_id"].startswith("BLB-")
    assert file_map["nested/beta.txt"]["sha256"]


def test_snapshot_file_map_falls_back_to_view_when_tree_metadata_is_missing(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    ctx = store.init_repo(repo, "repo", "main")
    conn = local_content.connect_sqlite(ctx.content_db_path)
    try:
        sentinel = {
            "legacy.txt": {
                "path": "legacy.txt",
                "blob_id": "BLB-LEGACY",
                "sha256": "deadbeef",
                "size_bytes": 7,
                "mode": "0o644",
            }
        }

        monkeypatch.setattr(
            local_content,
            "_snapshot_row",
            lambda _conn, _snapshot_id: {"snapshot_id": "SNP-LEGACY", "root_tree_id": ""},
        )
        monkeypatch.setattr(local_content, "_snapshot_file_map_via_view", lambda _conn, _snapshot_id: sentinel)

        assert local_content._snapshot_file_map(conn, "SNP-LEGACY") == sentinel
    finally:
        conn.close()


def test_workspace_status_ignores_tracked_non_sprint_markdown_lineage_only(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    artifact_path = "docs/lineage_only_projection_test.md"
    seed = local_content.create_snapshot(ctx, "repo", "main", "seed")
    (repo / "docs").mkdir(exist_ok=True)
    (repo / artifact_path).write_text("# Plan\n", encoding="utf-8")
    store.create_local_plan(ctx, "Plan", artifact_path, None, "Plan", [])

    status = local_content.workspace_delta(ctx, seed["snapshot_id"])

    assert status["clean"] is True
    assert status["changed_paths"] == []


def test_restore_workspace_preserves_tracked_non_sprint_markdown_lineage_only(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    artifact_path = "docs/lineage_only_projection_test.md"
    seed = local_content.create_snapshot(ctx, "repo", "main", "seed")
    (repo / "docs").mkdir(exist_ok=True)
    plan_path = repo / artifact_path
    plan_path.write_text("# Plan\n", encoding="utf-8")
    store.create_local_plan(ctx, "Plan", artifact_path, None, "Plan", [])

    restored = local_content.restore_workspace(ctx, seed["snapshot_id"], baseline_snapshot_id=seed["snapshot_id"])

    assert restored["plan"]["remove_count"] == 0
    assert plan_path.exists()
    assert plan_path.read_text(encoding="utf-8") == "# Plan\n"


def test_create_snapshot_excludes_tracked_non_sprint_markdown_lineage_only(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    artifact_path = "docs/lineage_only_projection_test.md"
    local_content.create_snapshot(ctx, "repo", "main", "seed")
    (repo / "docs").mkdir(exist_ok=True)
    (repo / artifact_path).write_text("# Plan\n", encoding="utf-8")
    store.create_local_plan(ctx, "Plan", artifact_path, None, "Plan", [])

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "exclude lineage-only markdown")

    snapshot_paths = [row["path"] for row in snapshot["files"]]
    assert artifact_path not in snapshot_paths


def test_create_snapshot_excludes_root_markdown_unconditionally(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "seed.txt").write_text("base\n", encoding="utf-8")
    (repo / "README_ZH.md").write_text("release docs\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "exclude root markdown")

    snapshot_paths = [row["path"] for row in snapshot["files"]]
    assert "seed.txt" in snapshot_paths
    assert "README_ZH.md" not in snapshot_paths


def test_docs_sprints_readme_stays_lineage_only(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    seed = local_content.create_snapshot(ctx, "repo", "main", "seed")
    (repo / "docs" / "sprints").mkdir(parents=True, exist_ok=True)
    readme_path = repo / "docs" / "sprints" / "README.md"
    readme_path.write_text("# Sprint Docs\n", encoding="utf-8")
    store.create_local_plan(ctx, "Sprint README", "docs/sprints/README.md", None, "Sprint README", [])

    status = local_content.workspace_delta(ctx, seed["snapshot_id"])
    snapshot = local_content.create_snapshot(ctx, "repo", "main", "exclude sprint readme")

    snapshot_paths = [row["path"] for row in snapshot["files"]]

    assert status["clean"] is True
    assert readme_path.exists()
    assert "docs/sprints/README.md" not in snapshot_paths


def test_store_init_repo_leaves_docs_sprints_readme_unmaterialized(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)

    store.init_repo(repo, "repo", "main")

    assert not (repo / "docs" / "sprints" / "README.md").exists()


def test_workspace_status_ignores_tracked_sprint_card_lineage_only(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    artifact_path = "docs/sprints/card.md"
    seed = local_content.create_snapshot(ctx, "repo", "main", "seed")
    (repo / "docs" / "sprints").mkdir(parents=True, exist_ok=True)
    (repo / artifact_path).write_text("# Sprint Card\n", encoding="utf-8")
    store.create_local_plan(ctx, "Sprint Card", artifact_path, None, "Sprint Card", [])

    status = local_content.workspace_delta(ctx, seed["snapshot_id"])

    assert status["clean"] is True
    assert status["changed_paths"] == []


def test_restore_workspace_preserves_tracked_sprint_card_lineage_only(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    artifact_path = "docs/sprints/card.md"
    seed = local_content.create_snapshot(ctx, "repo", "main", "seed")
    (repo / "docs" / "sprints").mkdir(parents=True, exist_ok=True)
    plan_path = repo / artifact_path
    plan_path.write_text("# Sprint Card\n", encoding="utf-8")
    store.create_local_plan(ctx, "Sprint Card", artifact_path, None, "Sprint Card", [])

    restored = local_content.restore_workspace(ctx, seed["snapshot_id"], baseline_snapshot_id=seed["snapshot_id"])

    assert restored["plan"]["remove_count"] == 0
    assert plan_path.exists()
    assert plan_path.read_text(encoding="utf-8") == "# Sprint Card\n"


def test_create_snapshot_excludes_tracked_sprint_card_lineage_only(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    artifact_path = "docs/sprints/card.md"
    local_content.create_snapshot(ctx, "repo", "main", "seed")
    (repo / "docs" / "sprints").mkdir(parents=True, exist_ok=True)
    (repo / artifact_path).write_text("# Sprint Card\n", encoding="utf-8")
    store.create_local_plan(ctx, "Sprint Card", artifact_path, None, "Sprint Card", [])

    snapshot = local_content.create_snapshot(ctx, "repo", "main", "exclude sprint card")

    snapshot_paths = [row["path"] for row in snapshot["files"]]
    assert artifact_path not in snapshot_paths


def test_workspace_status_ignores_root_markdown_unconditionally(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "seed.txt").write_text("base\n", encoding="utf-8")
    readme_zh = repo / "README_ZH.md"
    readme_zh.write_text("release docs\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    seed = local_content.create_snapshot(ctx, "repo", "main", "seed")
    readme_zh.write_text("release docs updated\n", encoding="utf-8")

    status = local_content.workspace_delta(ctx, seed["snapshot_id"])

    assert status["clean"] is True
    assert status["changed_paths"] == []


def test_workspace_status_ignores_newly_ignored_tracked_file(tmp_path: Path) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    (repo / "pyproject.toml").write_text("[project]\nname='ait-native'\nversion='0.9.0'\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    seed = local_content.create_snapshot(ctx, "repo", "main", "seed")
    (repo / ".aitignore").write_text("pyproject.toml\n", encoding="utf-8")

    status = local_content.workspace_delta(ctx, seed["snapshot_id"])

    assert "pyproject.toml" not in status["changed_paths"]
    assert status["changed_paths"] == [".aitignore"]


def _count_target_reads(monkeypatch: pytest.MonkeyPatch, *targets: Path) -> dict[str, int]:
    original_read_bytes = Path.read_bytes
    resolved_targets = {target.resolve(): f"{target.parent.name}/{target.name}" for target in targets}
    counts: dict[str, int] = {label: 0 for label in resolved_targets.values()}

    def counting_read_bytes(self: Path) -> bytes:
        try:
            resolved = self.resolve()
        except OSError:
            resolved = self
        label = resolved_targets.get(resolved)
        if label is not None:
            counts[label] += 1
        return original_read_bytes(self)

    monkeypatch.setattr(Path, "read_bytes", counting_read_bytes)
    return counts


def test_workspace_state_reuses_cached_digest_for_unchanged_paths(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    payload = repo / "payload.bin"
    payload.write_bytes(b"alpha" * 512)
    ctx = store.init_repo(repo, "repo", "main")

    first = local_content_workspace_helpers._workspace_state(ctx.root)
    assert first["payload.bin"]["sha256"]

    read_counts = _count_target_reads(monkeypatch, payload)
    second = local_content_workspace_helpers._workspace_state(ctx.root)

    assert second["payload.bin"]["sha256"] == first["payload.bin"]["sha256"]
    assert read_counts["repo/payload.bin"] == 0


def test_workspace_delta_only_rehashes_changed_paths_when_cache_is_warm(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    alpha = repo / "alpha.txt"
    beta = repo / "beta.txt"
    alpha.write_text("alpha-1\n", encoding="utf-8")
    beta.write_text("beta-1\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    seed = local_content.create_snapshot(ctx, "repo", "main", "seed")
    alpha.write_text("alpha-2\n", encoding="utf-8")

    read_counts = _count_target_reads(monkeypatch, alpha, beta)
    status = local_content.workspace_delta(ctx, seed["snapshot_id"])

    assert status["modified_paths"] == ["alpha.txt"]
    assert read_counts["repo/alpha.txt"] == 1
    assert read_counts["repo/beta.txt"] == 0


def test_create_snapshot_reuses_cached_digests_and_skips_blob_repacking_for_noop_runs(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    repo = tmp_path / "repo"
    repo.mkdir(parents=True, exist_ok=True)
    alpha = repo / "alpha.txt"
    beta = repo / "beta.txt"
    alpha.write_text("alpha-1\n", encoding="utf-8")
    beta.write_text("beta-1\n", encoding="utf-8")
    ctx = store.init_repo(repo, "repo", "main")

    first = local_content.create_snapshot(ctx, "repo", "main", "seed")
    assert first["phase_timings_ms"]["hashing_cache"]["rehashed_paths"] == 2

    read_counts = _count_target_reads(monkeypatch, alpha, beta)
    second = local_content.create_snapshot(ctx, "repo", "main", "noop follow-up")

    assert second["phase_timings_ms"]["hashing_cache"]["reused_paths"] == 2
    assert second["phase_timings_ms"]["hashing_cache"]["rehashed_paths"] == 0
    assert second["phase_timings_ms"]["pack_archive_write"]["blob_pack_write"] == 0.0
    assert read_counts["repo/alpha.txt"] == 0
    assert read_counts["repo/beta.txt"] == 0
