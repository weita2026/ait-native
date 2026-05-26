from __future__ import annotations

from pathlib import Path

import pytest

from ait_server import server_content, server_store
from ait_server.server_paths import ServerContext
from tests.postgres_fake import fake_postgres_context


def _index_names(conn) -> set[str]:
    rows = conn.execute("select name from sqlite_master where type = 'index'").fetchall()
    return {str(row["name"]) for row in rows if row.get("name")}


def test_repository_namespace_prefix_guard_rejects_duplicate_create_and_update(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    repo_a = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="ACC")
    assert repo_a["id_namespace_prefix"] == "ACC"

    with pytest.raises(ValueError, match="already in use"):
        server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="ACC")

    repo_b = server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BET")
    assert repo_b["id_namespace_prefix"] == "BET"

    with pytest.raises(ValueError, match="already in use"):
        server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="ACC")


def test_repository_namespace_prefix_guard_creates_unique_index_when_clean(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="AAA")
    server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="BBB")

    conn = server_content.connect(ctx)
    try:
        assert server_content.REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX in _index_names(conn)
    finally:
        conn.close()


def test_repository_namespace_prefix_guard_ignores_empty_prefix_duplicates(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    repo_a = server_store.ensure_repository(ctx, "repo-a", "main", id_namespace_prefix="")
    repo_b = server_store.ensure_repository(ctx, "repo-b", "main", id_namespace_prefix="")
    assert repo_a["id_namespace_prefix"] == ""
    assert repo_b["id_namespace_prefix"] == ""

    conn = server_content.connect(ctx)
    try:
        assert server_content.REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX in _index_names(conn)
    finally:
        conn.close()

    assert server_content.audit_repository_namespace_prefix_duplicates(ctx) == []


def test_repository_namespace_prefix_duplicate_audit_reports_legacy_collisions(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    ctx.root.mkdir(parents=True, exist_ok=True)
    conn = server_content.connect(ctx)
    conn.executescript(
        """
        create table repositories (
            repo_name text primary key,
            repo_id text,
            default_line text not null,
            id_namespace_prefix text not null default 'AIT',
            policy_json text not null default '{}',
            created_at text not null,
            updated_at text not null
        );
        create table lines (
            repo_name text not null,
            repo_id text,
            line_name text not null,
            status text not null default 'active',
            archived_at text,
            created_at text not null,
            updated_at text not null,
            primary key (repo_name, line_name)
        );
        insert into repositories(repo_name, repo_id, default_line, id_namespace_prefix, policy_json, created_at, updated_at)
        values
            ('repo-empty-a', 'RP-EMPTY-A', 'main', '', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
            ('repo-empty-b', 'RP-EMPTY-B', 'main', '', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
            ('repo-a', 'RP-A', 'main', 'DUP', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
            ('repo-b', 'RP-B', 'main', 'DUP', '{}', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z');
        insert into lines(repo_name, repo_id, line_name, created_at, updated_at)
        values
            ('repo-empty-a', 'RP-EMPTY-A', 'main', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
            ('repo-empty-b', 'RP-EMPTY-B', 'main', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
            ('repo-a', 'RP-A', 'main', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z'),
            ('repo-b', 'RP-B', 'main', '2026-04-26T00:00:00Z', '2026-04-26T00:00:00Z');
        """
    )
    conn.commit()
    conn.close()

    duplicates = server_content.audit_repository_namespace_prefix_duplicates(ctx)
    assert duplicates == [
        {
            "id_namespace_prefix": "DUP",
            "repo_count": 2,
            "repo_names": ["repo-a", "repo-b"],
        }
    ]

    conn = server_content.connect(ctx)
    try:
        assert server_content.REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX not in _index_names(conn)
    finally:
        conn.close()
