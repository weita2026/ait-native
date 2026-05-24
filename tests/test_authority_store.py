from pathlib import Path

import pytest

from ait_server import server_control, server_db, server_store
from ait_server.authority_store import (
    create_authority_node,
    delete_authority_node,
    ensure_authority_map,
    list_authority_mutations,
    list_authority_nodes,
    list_authority_graph,
    reorder_authority_node,
    update_authority_node,
)
from ait_server.server_paths import ServerContext
from tests.postgres_fake import FakePsycopg, fake_postgres_context


def _authority_columns(conn, table_name: str) -> set[str]:
    return {row["name"] for row in conn.execute(f"pragma table_info({table_name})")}


def _index_names(conn) -> set[str]:
    return {row["name"] for row in conn.execute("select name from sqlite_master where type = 'index'")}


def test_authority_schema_controls_for_sqlite_control_plane(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        for table_name in ("authority_maps", "authority_nodes", "authority_mutations"):
            rows = conn.execute(
                "select name from sqlite_master where type='table' and name = ?",
                (table_name,),
            ).fetchall()
            assert rows, f"Missing table: {table_name}"
        authority_map_columns = _authority_columns(conn, "authority_maps")
        authority_node_columns = _authority_columns(conn, "authority_nodes")
        authority_mutation_columns = _authority_columns(conn, "authority_mutations")
        role_binding_columns = _authority_columns(conn, "role_bindings")
    finally:
        conn.close()

    assert {"authority_map_id", "repo_name", "root_document_path", "milestone_document_path", "schema_version"} <= authority_map_columns
    assert "repo_id" in authority_map_columns
    assert "repo_id" in role_binding_columns
    assert {"authority_node_id", "authority_map_id", "node_kind", "document_path", "connection_mode"} <= authority_node_columns
    assert {"mutation_id", "authority_map_id", "authority_node_id", "mutation_kind", "payload_json", "actor_label"} <= authority_mutation_columns


def test_authority_schema_contract_sql_file_matches_runtime_contract():
    sql = Path("sql/ait_native_postgres_control_schema.sql").read_text(encoding="utf-8")
    for token in (
        "create table if not exists authority_maps",
        "create table if not exists authority_nodes",
        "create table if not exists authority_mutations",
        "create table if not exists role_bindings",
        "idx_authority_maps_repo",
        "idx_authority_maps_repo_id",
        "idx_authority_nodes_map_parent_sort",
        "idx_authority_mutations_map_created",
        "idx_role_bindings_repo_id_actor",
    ):
        assert token in server_control.SCHEMA_POSTGRES
        assert token in sql


def test_authority_map_and_role_bindings_use_repo_id_for_lookup(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    repo = server_store.ensure_repository(ctx, "repo-a", "main")
    repo_id = repo["repo_id"]

    conn = server_control.connect(ctx)
    now = "2026-04-26T00:00:00Z"
    try:
        conn.execute(
            """
            insert into authority_maps(
                authority_map_id, repo_name, root_document_path, milestone_document_path,
                schema_version, created_at, updated_at
            ) values ('legacy-auth-map', 'repo-a', 'docs/plan.md', 'docs/milestone.md', 1, ?, ?)
            """,
            (now, now),
        )
        conn.execute(
            "insert into role_bindings(repo_name, actor_identity, role, created_at) values (?, ?, ?, ?)",
            ("repo-a", "actor@example.com", "repo_contributor", now),
        )
        conn.execute(
            "insert into role_bindings(repo_name, actor_identity, role, created_at) values (?, ?, ?, ?)",
            ("*", "global@example.com", "repo_owner", now),
        )
        conn.commit()
    finally:
        conn.close()

    server_control.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        authority_row = conn.execute(
            "select repo_id from authority_maps where authority_map_id = 'legacy-auth-map'"
        ).fetchone()
        assert authority_row is not None
        assert authority_row["repo_id"] == repo_id

        actor_role_row = conn.execute(
            "select repo_name, repo_id from role_bindings where actor_identity = 'actor@example.com'"
        ).fetchone()
        assert actor_role_row is not None
        assert actor_role_row["repo_name"] == "repo-a"
        assert actor_role_row["repo_id"] == repo_id

        wildcard_binding = conn.execute(
            "select repo_name, repo_id from role_bindings where actor_identity = 'global@example.com' and repo_name = '*'"
        ).fetchone()
        assert wildcard_binding is not None
        assert wildcard_binding["repo_id"] is None
    finally:
        conn.close()

    conn = server_control.connect(ctx)
    try:
        conn.execute("update authority_maps set repo_name = 'repo-a-legacy' where authority_map_id = 'legacy-auth-map'")
        conn.execute(
            "update role_bindings set repo_name = 'repo-a-legacy' "
            "where actor_identity = 'actor@example.com' and role = 'repo_contributor'"
        )
        conn.commit()
    finally:
        conn.close()

    authority_map = ensure_authority_map(ctx, "repo-a")
    assert authority_map["authority_map_id"] == "legacy-auth-map"
    assert authority_map["repo_id"] == repo_id

    graph = list_authority_graph(ctx, "repo-a")
    assert graph is not None
    assert graph["authority_map"]["authority_map_id"] == "legacy-auth-map"

    bindings = server_control.list_role_bindings(ctx, "repo-a")
    assert {row["role"] for row in bindings if row["actor_identity"] == "actor@example.com"} == {"repo_contributor"}
    assert {row["role"] for row in bindings if row["actor_identity"] == "global@example.com"} == {"repo_owner"}
    assert len(bindings) >= 2

    conn = server_control.connect(ctx)
    try:
        assert server_control.resolve_bound_roles(conn, "repo-a", "actor@example.com", repo_id=repo_id) == {"repo_contributor"}
        assert server_control.resolve_bound_roles(conn, "repo-a", "global@example.com", repo_id=repo_id) == {"repo_owner"}
    finally:
        conn.close()


def test_server_control_initialize_drops_historical_publication_storage(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_control.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        conn.execute("alter table tasks add column historical_publication_id text")
        conn.execute("alter table changes add column historical_publication_id text")
        conn.execute(
            """
            create table if not exists historical_publications (
                publication_id text primary key,
                repo_id text,
                repo_name text not null,
                created_at text not null
            )
            """
        )
        conn.execute(
            """
            create table if not exists historical_publication_items (
                publication_id text not null,
                item_kind text not null,
                item_id text not null
            )
            """
        )
        conn.execute("create index if not exists idx_historical_publication_items_repo on historical_publication_items(publication_id)")
        conn.execute("create index if not exists idx_historical_publications_repo_created on historical_publications(repo_name, created_at desc)")
        conn.execute("create unique index if not exists uq_historical_publications_repo_id_seq on historical_publications(repo_id, publication_id)")
        conn.execute(
            "create unique index if not exists uq_historical_publications_repo_id_idempotency on historical_publications(repo_id, created_at)"
        )
        conn.commit()
    finally:
        conn.close()

    server_control.initialize(ctx)

    conn = server_control.connect(ctx)
    try:
        assert "historical_publication_id" not in _authority_columns(conn, "tasks")
        assert "historical_publication_id" not in _authority_columns(conn, "changes")
        tables = {row["name"] for row in conn.execute("select name from sqlite_master where type = 'table'").fetchall()}
        assert "historical_publications" not in tables
        assert "historical_publication_items" not in tables
        index_names = _index_names(conn)
        assert "idx_historical_publication_items_repo" not in index_names
        assert "idx_historical_publications_repo_created" not in index_names
        assert "uq_historical_publications_repo_id_seq" not in index_names
        assert "uq_historical_publications_repo_id_idempotency" not in index_names
    finally:
        conn.close()


def test_authority_map_crud_and_reorder_helpers(tmp_path: Path):
    ctx = fake_postgres_context(tmp_path / "server-data")
    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main")

    authority_map = ensure_authority_map(ctx, "repo-a")
    assert authority_map["repo_name"] == "repo-a"
    root_nodes = list_authority_nodes(ctx, authority_map["authority_map_id"], parent_node_id=None)
    assert len(root_nodes) >= 2
    layer1 = next(node for node in root_nodes if node["node_kind"] == "layer1")
    layer2_a = create_authority_node(
        ctx,
        authority_map["authority_map_id"],
        node_kind="layer2",
        parent_node_id=layer1["authority_node_id"],
        title="product plan",
        document_path="docs/product_plan.md",
    )
    layer2_b = create_authority_node(
        ctx,
        authority_map["authority_map_id"],
        node_kind="layer2",
        parent_node_id=layer1["authority_node_id"],
        title="market strategy",
        document_path="docs/market_strategy.md",
    )
    layer3 = create_authority_node(
        ctx,
        authority_map["authority_map_id"],
        node_kind="layer3",
        parent_node_id=layer2_a["authority_node_id"],
        title="sample layer3",
        document_path="docs/sample-layer3.md",
    )
    root_children = list_authority_nodes(ctx, authority_map["authority_map_id"], parent_node_id=layer1["authority_node_id"])
    assert [node["document_path"] for node in root_children] == [
        "docs/product_plan.md",
        "docs/market_strategy.md",
    ]

    updated_layer2_b = reorder_authority_node(ctx, layer2_b["authority_node_id"], 1)
    assert updated_layer2_b["sort_index"] == 1
    reordered_children = list_authority_nodes(ctx, authority_map["authority_map_id"], parent_node_id=layer1["authority_node_id"])
    assert reordered_children[0]["authority_node_id"] == layer2_b["authority_node_id"]
    assert reordered_children[1]["authority_node_id"] == layer2_a["authority_node_id"]

    updated_layer2_b = update_authority_node(
        ctx,
        layer2_b["authority_node_id"],
        title="market strategy revised",
    )
    assert updated_layer2_b["title"] == "market strategy revised"
    assert updated_layer2_b["slug"] == "market-strategy-revised"

    delete_authority_node(ctx, layer2_a["authority_node_id"])
    remaining = list_authority_nodes(ctx, authority_map["authority_map_id"], parent_node_id=layer1["authority_node_id"])
    assert len(remaining) == 1
    assert remaining[0]["authority_node_id"] == layer2_b["authority_node_id"]
    descendants = list_authority_nodes(ctx, authority_map["authority_map_id"], parent_node_id=layer2_a["authority_node_id"])
    assert descendants == []
    mutations = list_authority_mutations(ctx, authority_map["authority_map_id"], limit=5)
    mutation_kinds = {mutation["mutation_kind"] for mutation in mutations}
    assert {"create_node", "reorder_node", "delete_node", "update_node"} <= mutation_kinds


def test_authority_create_node_releases_connection_after_validation_error(tmp_path: Path, monkeypatch):
    class CountingPsycopg:
        def __init__(self):
            self.inner = FakePsycopg()
            self.connect_calls = 0

        def connect(self, dsn: str):
            self.connect_calls += 1
            return self.inner.connect(dsn)

    fake = CountingPsycopg()
    monkeypatch.setattr(server_db, "_load_psycopg", lambda: fake)
    monkeypatch.setattr(server_db, "postgres_support_installed", lambda: True)
    ctx = ServerContext.create(
        tmp_path / "server-data",
        backend="postgres",
        postgres_dsn=f"fake-postgres:///{tmp_path / 'fake-runtime'}",
        content_schema="ait_native_content_authority_test",
        control_schema="ait_native_control_authority_test",
    )

    server_store.initialize(ctx)
    server_store.ensure_repository(ctx, "repo-a", "main")
    authority_map = ensure_authority_map(ctx, "repo-a")
    baseline_connect_calls = fake.connect_calls

    with pytest.raises(ValueError, match="layer2 nodes must be children of a layer1 node"):
        create_authority_node(
            ctx,
            authority_map["authority_map_id"],
            node_kind="layer2",
            parent_node_id=None,
            title="invalid child",
            document_path="docs/invalid-child.md",
        )

    conn = server_control.connect(ctx)
    try:
        assert fake.connect_calls == baseline_connect_calls
    finally:
        conn.close()
