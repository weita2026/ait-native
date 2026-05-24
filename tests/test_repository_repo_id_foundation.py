from __future__ import annotations

from pathlib import Path

from ait_server import read_models, server_content, server_store
from ait_server.server_paths import ServerContext
from tests.postgres_fake import fake_postgres_context


def _index_names(conn) -> set[str]:
    rows = conn.execute("select name from sqlite_master where type = 'index'").fetchall()
    return {str(row["name"]) for row in rows if row.get("name")}


def test_repository_repo_id_is_created_and_exposed_in_read_surfaces(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    repo = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    assert repo["repo_id"].startswith("REPO-")

    fetched = server_store.get_repository(ctx, "repo-a")
    assert fetched["repo_id"] == repo["repo_id"]

    index_payload = read_models.repository_index(ctx)
    assert index_payload["repositories"] == [
        {
            "repo_name": "repo-a",
            "repo_id": repo["repo_id"],
            "default_line": "main",
            "created_at": repo["created_at"],
            "updated_at": repo["updated_at"],
            "line_count": 1,
        }
    ]


def test_repository_repo_id_backfills_legacy_rows_and_adds_unique_index(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    ctx.root.mkdir(parents=True, exist_ok=True)
    conn = server_content.connect(ctx)
    conn.executescript(
        """
        create table repositories (
            repo_name text primary key,
            default_line text not null,
            id_namespace_prefix text not null default 'AIT',
            policy_json text not null default '{}',
            created_at text not null,
            updated_at text not null
        );
        create table lines (
            repo_name text not null,
            line_name text not null,
            status text not null default 'active',
            archived_at text,
            created_at text not null,
            updated_at text not null,
            primary key (repo_name, line_name)
        );
        insert into repositories(repo_name, default_line, id_namespace_prefix, policy_json, created_at, updated_at)
        values
            ('repo-a', 'main', 'AAA', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
            ('repo-b', 'main', 'BBB', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z');
        insert into lines(repo_name, line_name, created_at, updated_at)
        values
            ('repo-a', 'main', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
            ('repo-b', 'main', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z');
        """
    )
    conn.commit()
    conn.close()

    repo_a = server_store.get_repository(ctx, "repo-a")
    repo_b = server_store.get_repository(ctx, "repo-b")
    assert repo_a["repo_id"].startswith("REPO-")
    assert repo_b["repo_id"].startswith("REPO-")
    assert repo_a["repo_id"] != repo_b["repo_id"]

    conn = server_content.connect(ctx)
    try:
        rows = conn.execute("select repo_name, repo_id from repositories order by repo_name asc").fetchall()
        assert [str(row["repo_id"]) for row in rows] == [repo_a["repo_id"], repo_b["repo_id"]]
        assert server_content.REPOSITORY_ID_UNIQUE_INDEX in _index_names(conn)
    finally:
        conn.close()
