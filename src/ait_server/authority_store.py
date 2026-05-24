from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import Any

from ait_protocol.common import generate_namespaced_workflow_id, utc_now

from .server_content import get_repository
from .server_control import connect, record_event
from .server_paths import ServerContext

AUTHORITY_ROOT_PLAN_PATH = "docs/plan.md"
AUTHORITY_MILESTONE_PATH = "docs/milestone.md"
AUTHORITY_LEGAL_LAYER_PATHS = [
    "docs/product_plan.md",
    "docs/market_strategy.md",
    "docs/engineering_plan.md",
    "docs/financial_plan.md",
    "docs/legal_plan.md",
]
AUTHORITY_NODE_KINDS = {"layer1", "milestone", "layer2", "layer3"}
AUTHORITY_CONNECTION_MODES = {"connected", "detached"}
_SLUG_RE = re.compile(r"[^a-z0-9]+")


def _repo_root() -> Path:
    configured = os.environ.get("AIT_REPO_ROOT")
    if configured:
        return Path(configured).expanduser().resolve()
    return Path(__file__).resolve().parents[2]


def _repo_namespace_prefix(ctx: ServerContext, repo_name: str) -> str:
    repo = get_repository(ctx, repo_name)
    value = str(repo.get("id_namespace_prefix") or "").strip()
    return value or "AIT"


def _workflow_namespace_token(value: str | None) -> str:
    token = re.sub(r"[^A-Za-z0-9]+", "", str(value or "").strip()).upper()
    return token or "AIT"


def _slugify(value: str) -> str:
    text = _SLUG_RE.sub("-", str(value or "").strip().lower()).strip("-")
    return text or "node"


def _normalize_text(value: str | None, *, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} must not be empty")
    return text


def _normalize_optional_text(value: str | None) -> str | None:
    text = str(value or "").strip()
    return text or None


def _actor_label(actor_identity: str | None = None, actor_type: str | None = None) -> str:
    identity = _normalize_optional_text(actor_identity) or "system"
    kind = _normalize_optional_text(actor_type)
    return f"{identity} ({kind})" if kind else identity


def _fetchone(conn, sql: str, params: tuple[Any, ...], *, missing: str) -> dict[str, Any]:
    row = conn.execute(sql, params).fetchone()
    if row is None:
        raise KeyError(missing)
    return dict(row)


def _authority_map(conn, authority_map_id: str) -> dict[str, Any]:
    return _fetchone(
        conn,
        "select * from authority_maps where authority_map_id = ?",
        (authority_map_id,),
        missing=f"Unknown authority map: {authority_map_id}",
    )


def _repo_id_for_repo_name(ctx: ServerContext, repo_name: str) -> str | None:
    if repo_name == "*":
        return None
    try:
        repository = get_repository(ctx, repo_name)
    except KeyError:
        return None
    repo_id = str(repository.get("repo_id") or "").strip()
    return repo_id or None


def _authority_map_row(conn, repo_name: str, *, repo_id: str | None = None):
    if repo_id:
        row = conn.execute("select * from authority_maps where repo_id = ?", (repo_id,)).fetchone()
        if row is not None:
            return row
    return conn.execute("select * from authority_maps where repo_name = ?", (repo_name,)).fetchone()


def _authority_node(conn, authority_node_id: str) -> dict[str, Any]:
    return _fetchone(
        conn,
        "select * from authority_nodes where authority_node_id = ?",
        (authority_node_id,),
        missing=f"Unknown authority node: {authority_node_id}",
    )


def _sibling_nodes(conn, authority_map_id: str, parent_node_id: str | None) -> list[dict[str, Any]]:
    if parent_node_id is None:
        rows = conn.execute(
            """
            select *
            from authority_nodes
            where authority_map_id = ? and parent_node_id is null
            order by sort_index asc, document_path asc
            """,
            (authority_map_id,),
        ).fetchall()
    else:
        rows = conn.execute(
            """
            select *
            from authority_nodes
            where authority_map_id = ? and parent_node_id = ?
            order by sort_index asc, document_path asc
            """,
            (authority_map_id, parent_node_id),
        ).fetchall()
    return [dict(row) for row in rows]


def _normalize_sibling_order(conn, authority_map_id: str, parent_node_id: str | None) -> None:
    now = utc_now()
    for index, row in enumerate(_sibling_nodes(conn, authority_map_id, parent_node_id), start=1):
        if int(row.get("sort_index") or 0) == index:
            continue
        conn.execute(
            "update authority_nodes set sort_index = ?, updated_at = ? where authority_node_id = ?",
            (index, now, row["authority_node_id"]),
        )


def _record_mutation(
    conn,
    authority_map_id: str,
    authority_node_id: str | None,
    mutation_kind: str,
    payload: dict[str, Any],
    *,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    authority_map = _authority_map(conn, authority_map_id)
    mutation_id = generate_namespaced_workflow_id("AMU", _workflow_namespace_token(authority_map.get("repo_name")))
    actor_label = _actor_label(actor_identity, actor_type)
    created_at = utc_now()
    conn.execute(
        """
        insert into authority_mutations(
            mutation_id, authority_map_id, authority_node_id, mutation_kind, payload_json, actor_label, created_at
        ) values (?, ?, ?, ?, ?, ?, ?)
        """,
        (
            mutation_id,
            authority_map_id,
            authority_node_id,
            mutation_kind,
            json.dumps(payload, sort_keys=True),
            actor_label,
            created_at,
        ),
    )
    record_event(
        conn,
        "authority.updated",
        "authority_map",
        authority_map_id,
        {"action": mutation_kind, **payload},
        actor_identity=actor_identity or "system",
        actor_type=actor_type or "system_worker",
    )
    return {
        "mutation_id": mutation_id,
        "authority_map_id": authority_map_id,
        "authority_node_id": authority_node_id,
        "mutation_kind": mutation_kind,
        "payload_json": json.dumps(payload, sort_keys=True),
        "actor_label": actor_label,
        "created_at": created_at,
    }


def ensure_authority_map(ctx: ServerContext, repo_name: str) -> dict[str, Any]:
    with connect(ctx) as conn:
        repo_id = _repo_id_for_repo_name(ctx, repo_name)
        existing = _authority_map_row(conn, repo_name, repo_id=repo_id)
        if existing is not None:
            return dict(existing)

        namespace = _repo_namespace_prefix(ctx, repo_name)
        authority_map_id = generate_namespaced_workflow_id("AM", namespace)
        created_at = utc_now()
        conn.execute(
            """
            insert into authority_maps(
                authority_map_id, repo_name, repo_id, root_document_path, milestone_document_path, schema_version, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                authority_map_id,
                repo_name,
                repo_id,
                AUTHORITY_ROOT_PLAN_PATH,
                AUTHORITY_MILESTONE_PATH,
                1,
                created_at,
                created_at,
            ),
        )
        create_authority_node(
            ctx,
            authority_map_id,
            node_kind="layer1",
            parent_node_id=None,
            title="Plan",
            document_path=AUTHORITY_ROOT_PLAN_PATH,
            connection_mode="connected",
            actor_identity="system",
            actor_type="system_worker",
            _conn=conn,
            _commit=False,
        )
        create_authority_node(
            ctx,
            authority_map_id,
            node_kind="milestone",
            parent_node_id=None,
            title="Milestone Index",
            document_path=AUTHORITY_MILESTONE_PATH,
            connection_mode="detached",
            actor_identity="system",
            actor_type="system_worker",
            _conn=conn,
            _commit=False,
        )
        conn.commit()
        out = _authority_map(conn, authority_map_id)
    return out


def get_authority_map(ctx: ServerContext, authority_map_id: str) -> dict[str, Any]:
    with connect(ctx) as conn:
        out = _authority_map(conn, authority_map_id)
    return out


def get_authority_node(ctx: ServerContext, authority_node_id: str) -> dict[str, Any]:
    with connect(ctx) as conn:
        out = _authority_node(conn, authority_node_id)
    return out


def list_authority_nodes(
    ctx: ServerContext,
    authority_map_id: str,
    *,
    parent_node_id: str | None,
) -> list[dict[str, Any]]:
    with connect(ctx) as conn:
        rows = _sibling_nodes(conn, authority_map_id, parent_node_id)
    return rows


def list_authority_mutations(ctx: ServerContext, authority_map_id: str, *, limit: int = 50) -> list[dict[str, Any]]:
    with connect(ctx) as conn:
        rows = [
            dict(row)
            for row in conn.execute(
                """
                select *
                from authority_mutations
                where authority_map_id = ?
                order by created_at desc, mutation_id desc
                limit ?
                """,
                (authority_map_id, max(int(limit), 1)),
            ).fetchall()
        ]
    return rows


def create_authority_node(
    ctx: ServerContext,
    authority_map_id: str,
    *,
    node_kind: str,
    parent_node_id: str | None,
    title: str,
    document_path: str,
    connection_mode: str = "connected",
    actor_identity: str | None = None,
    actor_type: str | None = None,
    _conn=None,
    _commit: bool = True,
) -> dict[str, Any]:
    normalized_kind = _normalize_text(node_kind, field="node_kind")
    if normalized_kind not in AUTHORITY_NODE_KINDS:
        raise ValueError(f"Unsupported authority node_kind: {normalized_kind}")
    normalized_path = _normalize_text(document_path, field="document_path")
    normalized_title = _normalize_text(title, field="title")
    normalized_mode = _normalize_text(connection_mode, field="connection_mode")
    if normalized_mode not in AUTHORITY_CONNECTION_MODES:
        raise ValueError(f"Unsupported connection_mode: {normalized_mode}")

    conn = _conn or connect(ctx)
    owns_connection = _conn is None
    try:
        authority_map = _authority_map(conn, authority_map_id)
        parent = _authority_node(conn, parent_node_id) if parent_node_id is not None else None
        if normalized_kind == "layer1" and parent is not None:
            raise ValueError("layer1 nodes cannot have parents")
        if normalized_kind == "milestone" and parent is not None and parent["node_kind"] != "layer1":
            raise ValueError("milestone nodes must be roots or children of a layer1 node")
        if normalized_kind == "layer2" and (parent is None or parent["node_kind"] != "layer1"):
            raise ValueError("layer2 nodes must be children of a layer1 node")
        if normalized_kind == "layer3" and (parent is None or parent["node_kind"] != "layer2"):
            raise ValueError("layer3 nodes must be children of a layer2 node")
        duplicate = conn.execute(
            "select authority_node_id from authority_nodes where authority_map_id = ? and document_path = ?",
            (authority_map_id, normalized_path),
        ).fetchone()
        if duplicate is not None:
            raise ValueError(f"Authority document already exists: {normalized_path}")

        namespace = _repo_namespace_prefix(ctx, authority_map["repo_name"])
        authority_node_id = generate_namespaced_workflow_id("AN", namespace)
        slug = _slugify(normalized_title)
        siblings = _sibling_nodes(conn, authority_map_id, parent_node_id)
        sort_index = len(siblings) + 1
        created_at = utc_now()
        conn.execute(
            """
            insert into authority_nodes(
                authority_node_id, authority_map_id, node_kind, parent_node_id, document_path, title, slug,
                sort_index, connection_mode, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                authority_node_id,
                authority_map_id,
                normalized_kind,
                parent_node_id,
                normalized_path,
                normalized_title,
                slug,
                sort_index,
                normalized_mode,
                created_at,
                created_at,
            ),
        )
        conn.execute(
            "update authority_maps set updated_at = ? where authority_map_id = ?",
            (created_at, authority_map_id),
        )
        _record_mutation(
            conn,
            authority_map_id,
            authority_node_id,
            "create_node",
            {"document_path": normalized_path, "node_kind": normalized_kind, "parent_node_id": parent_node_id},
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        if _commit:
            conn.commit()
        return _authority_node(conn, authority_node_id)
    finally:
        if owns_connection:
            conn.close()


def update_authority_node(
    ctx: ServerContext,
    authority_node_id: str,
    *,
    title: str | None = None,
    document_path: str | None = None,
    connection_mode: str | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    with connect(ctx) as conn:
        row = _authority_node(conn, authority_node_id)
        updates: list[str] = []
        params: list[Any] = []
        payload: dict[str, Any] = {"authority_node_id": authority_node_id}
        if title is not None:
            normalized_title = _normalize_text(title, field="title")
            updates.extend(["title = ?", "slug = ?"])
            params.extend([normalized_title, _slugify(normalized_title)])
            payload["title"] = normalized_title
        if document_path is not None:
            normalized_path = _normalize_text(document_path, field="document_path")
            updates.append("document_path = ?")
            params.append(normalized_path)
            payload["document_path"] = normalized_path
        if connection_mode is not None:
            normalized_mode = _normalize_text(connection_mode, field="connection_mode")
            if normalized_mode not in AUTHORITY_CONNECTION_MODES:
                raise ValueError(f"Unsupported connection_mode: {normalized_mode}")
            updates.append("connection_mode = ?")
            params.append(normalized_mode)
            payload["connection_mode"] = normalized_mode
        if not updates:
            return row
        updates.append("updated_at = ?")
        params.append(utc_now())
        params.append(authority_node_id)
        conn.execute(f"update authority_nodes set {', '.join(updates)} where authority_node_id = ?", tuple(params))
        _record_mutation(
            conn,
            row["authority_map_id"],
            authority_node_id,
            "update_node",
            payload,
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
        out = _authority_node(conn, authority_node_id)
    return out


def reorder_authority_node(
    ctx: ServerContext,
    authority_node_id: str,
    position: int,
    *,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    with connect(ctx) as conn:
        row = _authority_node(conn, authority_node_id)
        siblings = _sibling_nodes(conn, row["authority_map_id"], row.get("parent_node_id"))
        if position < 1 or position > len(siblings):
            raise ValueError(f"Invalid position: {position}")
        ordered = [item for item in siblings if item["authority_node_id"] != authority_node_id]
        ordered.insert(position - 1, row)
        now = utc_now()
        for index, item in enumerate(ordered, start=1):
            conn.execute(
                "update authority_nodes set sort_index = ?, updated_at = ? where authority_node_id = ?",
                (index, now, item["authority_node_id"]),
            )
        _record_mutation(
            conn,
            row["authority_map_id"],
            authority_node_id,
            "reorder_node",
            {"authority_node_id": authority_node_id, "position": position},
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
        out = _authority_node(conn, authority_node_id)
    return out


def delete_authority_node(
    ctx: ServerContext,
    authority_node_id: str,
    *,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> None:
    with connect(ctx) as conn:
        row = _authority_node(conn, authority_node_id)
        if row["node_kind"] in {"layer1", "milestone"}:
            raise ValueError("Protected authority roots cannot be deleted")
        authority_map_id = row["authority_map_id"]
        parent_node_id = row.get("parent_node_id")
        conn.execute("delete from authority_nodes where authority_node_id = ?", (authority_node_id,))
        _normalize_sibling_order(conn, authority_map_id, parent_node_id)
        _record_mutation(
            conn,
            authority_map_id,
            None,
            "delete_node",
            {"authority_node_id": authority_node_id, "document_path": row["document_path"]},
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()


def list_authority_graph(ctx: ServerContext, repo_name: str) -> dict[str, Any] | None:
    with connect(ctx) as conn:
        repo_id = _repo_id_for_repo_name(ctx, repo_name)
        map_row = _authority_map_row(conn, repo_name, repo_id=repo_id)
        if map_row is None:
            return None
        authority_map_id = map_row["authority_map_id"]
        nodes = [
            dict(row)
            for row in conn.execute(
                """
                select *
                from authority_nodes
                where authority_map_id = ?
                order by
                    case node_kind
                        when 'layer1' then 0
                        when 'milestone' then 1
                        when 'layer2' then 2
                        else 3
                    end asc,
                    sort_index asc,
                    document_path asc
                """,
                (authority_map_id,),
            ).fetchall()
        ]
    return {"authority_map": dict(map_row), "nodes": nodes}


def replace_authority_graph(
    ctx: ServerContext,
    repo_name: str,
    *,
    nodes: list[dict[str, Any]],
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    authority_map = ensure_authority_map(ctx, repo_name)
    with connect(ctx) as conn:
        authority_map_id = authority_map["authority_map_id"]
        conn.execute("delete from authority_nodes where authority_map_id = ?", (authority_map_id,))
        path_to_id: dict[str, str] = {}
        created_ids: list[str] = []
        pending = [dict(node) for node in nodes]
        order_index = {str(node.get("document_path") or ""): index for index, node in enumerate(pending)}
        created_at = utc_now()
        while pending:
            progressed = False
            for node in sorted(
                pending,
                key=lambda item: (
                    0 if str(item.get("node_kind") or "") == "layer1" else 1,
                    int(item.get("sort_index") or 1),
                    order_index.get(str(item.get("document_path") or ""), 0),
                ),
            ):
                parent_path = _normalize_optional_text(node.get("parent_document_path"))
                if parent_path is not None and parent_path not in path_to_id:
                    continue
                created = create_authority_node(
                    ctx,
                    authority_map_id,
                    node_kind=str(node["node_kind"]),
                    parent_node_id=path_to_id.get(parent_path) if parent_path is not None else None,
                    title=str(node["title"]),
                    document_path=str(node["document_path"]),
                    connection_mode=str(node.get("connection_mode") or "connected"),
                    actor_identity=actor_identity,
                    actor_type=actor_type,
                    _conn=conn,
                    _commit=False,
                )
                path_to_id[created["document_path"]] = created["authority_node_id"]
                created_ids.append(created["authority_node_id"])
                pending.remove(node)
                progressed = True
            if not progressed:
                unresolved = ", ".join(str(node.get("document_path") or "") for node in pending)
                raise ValueError(f"Unable to resolve authority node parents for: {unresolved}")
        for node in nodes:
            authority_node_id = path_to_id[str(node["document_path"])]
            conn.execute(
                """
                update authority_nodes
                set sort_index = ?, updated_at = ?
                where authority_node_id = ?
                """,
                (
                    int(node.get("sort_index") or 1),
                    created_at,
                    authority_node_id,
                ),
            )
        _normalize_sibling_order(conn, authority_map_id, None)
        for authority_node_id in created_ids:
            row = _authority_node(conn, authority_node_id)
            _normalize_sibling_order(conn, authority_map_id, row.get("parent_node_id"))
        _record_mutation(
            conn,
            authority_map_id,
            None,
            "replace_graph",
            {"node_count": len(nodes)},
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
    return list_authority_graph(ctx, repo_name) or {}


def _relative_markdown_link(source_path: str, target_path: str) -> str:
    source_parent = Path(source_path).parent
    return Path(os.path.relpath(target_path, start=source_parent)).as_posix()


def _authority_doc_template(*, title: str, path: str, parent_path: str | None, node_kind: str) -> str:
    if node_kind == "layer2":
        authority = f"Authority: legal layer under [plan.md]({_relative_markdown_link(path, AUTHORITY_ROOT_PLAN_PATH)})."
        scope = f"Scope: legal-layer governance placeholder for {title}."
    else:
        authority = (
            f"Authority: command layer under [plan.md]({_relative_markdown_link(path, AUTHORITY_ROOT_PLAN_PATH)}) "
            f"and [{Path(parent_path or AUTHORITY_ROOT_PLAN_PATH).name}]({_relative_markdown_link(path, parent_path or AUTHORITY_ROOT_PLAN_PATH)})."
        )
        scope = f"Scope: command-layer placeholder for {title}."
    return f"# {title}\n\n{authority}\nStatus: draft.\n{scope}\n"


def _update_strategy_index(*_args, **_kwargs) -> None:
    return None


def _bootstrap_docs(repo_root: Path, repo_name: str) -> None:
    docs_dir = repo_root / "docs"
    docs_dir.mkdir(parents=True, exist_ok=True)
    plan_path = repo_root / AUTHORITY_ROOT_PLAN_PATH
    milestone_path = repo_root / AUTHORITY_MILESTONE_PATH
    if not plan_path.exists():
        plan_path.write_text(
            f"# {repo_name} Plan\n\nAuthority: constitutional layer for repository planning.\nStatus: draft.\nScope: repository plan root.\n",
            encoding="utf-8",
        )
    if not milestone_path.exists():
        milestone_path.write_text(
            f"# {repo_name} Milestone Index\n\nAuthority: command layer under [plan.md]({_relative_markdown_link(AUTHORITY_MILESTONE_PATH, AUTHORITY_ROOT_PLAN_PATH)}).\nStatus: draft.\nScope: milestone routing index.\n",
            encoding="utf-8",
        )


def seed_blank_authority_graph(
    ctx: ServerContext,
    repo_name: str,
    *,
    repo_root: Path | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    resolved_root = repo_root or _repo_root()
    _bootstrap_docs(resolved_root, repo_name)
    return replace_authority_graph(
        ctx,
        repo_name,
        nodes=[
            {
                "node_kind": "layer1",
                "document_path": AUTHORITY_ROOT_PLAN_PATH,
                "title": "Plan",
                "sort_index": 1,
                "connection_mode": "connected",
            },
            {
                "node_kind": "milestone",
                "document_path": AUTHORITY_MILESTONE_PATH,
                "title": "Milestone Index",
                "sort_index": 2,
                "connection_mode": "detached",
            },
        ],
        actor_identity=actor_identity,
        actor_type=actor_type,
    )


def _next_document_path(graph: dict[str, Any], *, title: str, parent_document_path: str, node_kind: str) -> str:
    used = {str(row.get("document_path") or "") for row in graph.get("nodes", [])}
    slug = _slugify(title)
    if node_kind == "layer2":
        base = f"docs/{slug}"
    else:
        base = f"docs/{_slugify(Path(parent_document_path).stem)}_{slug}"
    candidate = f"{base}.md"
    suffix = 2
    while candidate in used:
        candidate = f"{base}_{suffix}.md"
        suffix += 1
    return candidate


def create_authority_child(
    ctx: ServerContext,
    repo_name: str,
    *,
    parent_document_path: str,
    title: str,
    repo_root: Path | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    graph = list_authority_graph(ctx, repo_name)
    if graph is None:
        graph = seed_blank_authority_graph(ctx, repo_name, repo_root=repo_root, actor_identity=actor_identity, actor_type=actor_type)
    parent = next((row for row in graph["nodes"] if row["document_path"] == parent_document_path), None)
    if parent is None:
        raise KeyError(f"Unknown authority document: {parent_document_path}")
    if parent["node_kind"] == "layer1":
        node_kind = "layer2"
        connection_mode = "connected"
    elif parent["node_kind"] == "layer2":
        node_kind = "layer3"
        connection_mode = "connected"
    else:
        raise ValueError("Children can only be created under Layer 1 or Layer 2 nodes")
    document_path = _next_document_path(graph, title=title, parent_document_path=parent_document_path, node_kind=node_kind)
    created = create_authority_node(
        ctx,
        graph["authority_map"]["authority_map_id"],
        node_kind=node_kind,
        parent_node_id=parent["authority_node_id"],
        title=title,
        document_path=document_path,
        connection_mode=connection_mode,
        actor_identity=actor_identity,
        actor_type=actor_type,
    )
    resolved_root = repo_root or _repo_root()
    target = resolved_root / document_path
    target.parent.mkdir(parents=True, exist_ok=True)
    if not target.exists():
        target.write_text(
            _authority_doc_template(
                title=created["title"],
                path=document_path,
                parent_path=parent_document_path,
                node_kind=node_kind,
            ),
            encoding="utf-8",
        )
    updated = list_authority_graph(ctx, repo_name) or graph
    _update_strategy_index(resolved_root, repo_name, updated)
    return updated


def delete_authority_document(
    ctx: ServerContext,
    repo_name: str,
    *,
    document_path: str,
    repo_root: Path | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    graph = list_authority_graph(ctx, repo_name)
    if graph is None:
        raise KeyError(f"Authority map is not initialized for {repo_name}")
    node = next((row for row in graph["nodes"] if row["document_path"] == document_path), None)
    if node is None:
        raise KeyError(f"Unknown authority document: {document_path}")
    descendants = [row for row in graph["nodes"] if row.get("parent_node_id") == node["authority_node_id"]]
    delete_authority_node(ctx, node["authority_node_id"], actor_identity=actor_identity, actor_type=actor_type)
    resolved_root = repo_root or _repo_root()
    for row in [node, *descendants]:
        target = resolved_root / row["document_path"]
        if target.exists():
            target.unlink()
    updated = list_authority_graph(ctx, repo_name) or graph
    _update_strategy_index(resolved_root, repo_name, updated)
    return updated


def reorder_authority_document(
    ctx: ServerContext,
    repo_name: str,
    *,
    document_path: str,
    position: int,
    repo_root: Path | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    graph = list_authority_graph(ctx, repo_name)
    if graph is None:
        raise KeyError(f"Authority map is not initialized for {repo_name}")
    node = next((row for row in graph["nodes"] if row["document_path"] == document_path), None)
    if node is None:
        raise KeyError(f"Unknown authority document: {document_path}")
    reorder_authority_node(ctx, node["authority_node_id"], position, actor_identity=actor_identity, actor_type=actor_type)
    updated = list_authority_graph(ctx, repo_name) or graph
    _update_strategy_index(repo_root or _repo_root(), repo_name, updated)
    return updated


__all__ = [
    "AUTHORITY_MILESTONE_PATH",
    "AUTHORITY_ROOT_PLAN_PATH",
    "create_authority_child",
    "create_authority_node",
    "delete_authority_document",
    "delete_authority_node",
    "ensure_authority_map",
    "get_authority_map",
    "get_authority_node",
    "list_authority_graph",
    "list_authority_mutations",
    "list_authority_nodes",
    "reorder_authority_document",
    "reorder_authority_node",
    "replace_authority_graph",
    "seed_blank_authority_graph",
    "update_authority_node",
]
